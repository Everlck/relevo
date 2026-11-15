use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex};
use tokio::time::timeout;

use crate::citar;
use crate::error::{Error, Resultado};
use crate::medio::Medio;
use crate::respuesta::{leer_respuesta, Respuesta};
use crate::rutas::Rutas;

const ESPERA_CONEXION: Duration = Duration::from_secs(3);
const ESPERA_RESPUESTA: Duration = Duration::from_secs(5);
const ESPERA_RESTOS: Duration = Duration::from_millis(5);
const ESPERA_EXTRA: Duration = Duration::from_millis(150);

struct Conexion {
    lector: BufReader<OwnedReadHalf>,
    escritor: OwnedWriteHalf,
}

struct Interno {
    direccion: String,
    conexion: Mutex<Option<Conexion>>,
    conectado: watch::Sender<bool>,
}

/// Cliente de CasparCG. Es barato de clonar y se puede usar desde varias tareas:
/// los comandos se envían de uno en uno, en orden.
///
/// No hace falta "conectar" a mano: el primer comando conecta, y si la conexión
/// se pierde, el siguiente comando vuelve a conectar.
#[derive(Clone)]
pub struct Cliente {
    interno: Arc<Interno>,
}

/// Opciones al reproducir un clip.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpcionesReproduccion {
    /// Repetir el clip sin fin.
    pub bucle: bool,
    /// Transición de entrada: tipo (`MIX`, `WIPE`, `PUSH`, `SLIDE`) y cuadros.
    pub transicion: Option<(&'static str, u32)>,
    /// Empezar en este cuadro (`SEEK`); 0 = desde el principio.
    pub desde_cuadro: u64,
}

fn agregar_transicion(cmd: &mut String, op: &OpcionesReproduccion) {
    if let Some((tipo, cuadros)) = op.transicion {
        if cuadros > 0 {
            cmd.push_str(&format!(" {tipo} {cuadros}"));
        }
    }
}

impl Cliente {
    pub fn new(host: &str, puerto: u16) -> Self {
        let (conectado, _) = watch::channel(false);
        Cliente {
            interno: Arc::new(Interno { direccion: format!("{host}:{puerto}"), conexion: Mutex::new(None), conectado }),
        }
    }

    pub fn direccion(&self) -> &str {
        &self.interno.direccion
    }

    /// Estado de la conexión. Cambia a `true`/`false` a medida que se conecta o se pierde.
    pub fn estado_conexion(&self) -> watch::Receiver<bool> {
        self.interno.conectado.subscribe()
    }

    pub fn esta_conectado(&self) -> bool {
        *self.interno.conectado.borrow()
    }

    /// Lanza una tarea que comprueba la conexión cada `intervalo` (con `VERSION`).
    /// Así el estado se mantiene al día y se reconecta solo, aunque nadie envíe comandos.
    pub fn vigilar(&self, intervalo: Duration) -> tokio::task::JoinHandle<()> {
        let cliente = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = cliente.version().await {
                    tracing::debug!("vigilancia de CasparCG: {e}");
                }
                tokio::time::sleep(intervalo).await;
            }
        })
    }

    fn marcar(&self, conectado: bool) {
        self.interno.conectado.send_if_modified(|actual| {
            if *actual != conectado {
                *actual = conectado;
                if conectado {
                    tracing::info!("conectado a CasparCG en {}", self.interno.direccion);
                } else {
                    tracing::warn!("se perdió la conexión con CasparCG en {}", self.interno.direccion);
                }
                true
            } else {
                false
            }
        });
    }

    async fn conectar(&self) -> Resultado<Conexion> {
        let flujo = timeout(ESPERA_CONEXION, TcpStream::connect(&self.interno.direccion))
            .await
            .map_err(|_| Error::Desconectado(format!("tiempo agotado conectando a {}", self.interno.direccion)))?
            .map_err(|e| Error::Desconectado(format!("{}: {e}", self.interno.direccion)))?;
        flujo.set_nodelay(true).ok();
        let (lectura, escritor) = flujo.into_split();
        Ok(Conexion { lector: BufReader::new(lectura), escritor })
    }

    /// Envía una línea AMCP tal cual y devuelve la respuesta, sea cual sea su código.
    pub async fn enviar(&self, comando: &str) -> Resultado<Respuesta> {
        self.enviar_interno(comando, false).await
    }

    /// Como `enviar`, pero además recoge las líneas que lleguen justo después de la
    /// respuesta. Algunas respuestas de `INFO` traen XML en varias líneas aunque
    /// el código sea 201.
    pub async fn enviar_largo(&self, comando: &str) -> Resultado<Respuesta> {
        self.enviar_interno(comando, true).await
    }

    async fn enviar_interno(&self, comando: &str, extra: bool) -> Resultado<Respuesta> {
        let mut guardia = self.interno.conexion.lock().await;

        // Hasta dos intentos: si una conexión vieja resultó estar muerta,
        // se reintenta una vez con una conexión nueva.
        for intento in 0..2 {
            let reutilizada = guardia.is_some();
            if guardia.is_none() {
                match self.conectar().await {
                    Ok(c) => *guardia = Some(c),
                    Err(e) => {
                        self.marcar(false);
                        return Err(e);
                    }
                }
            }
            let conexion = guardia.as_mut().expect("conexión recién abierta");

            match ejecutar(conexion, comando, extra).await {
                Ok(respuesta) => {
                    self.marcar(true);
                    return Ok(respuesta);
                }
                Err(e) => {
                    *guardia = None;
                    // Solo se reintenta si el servidor cerró una conexión antigua:
                    // en ese caso el comando nunca llegó y repetirlo es seguro.
                    let reintentar = reutilizada && intento == 0 && matches!(e, Error::Desconectado(_) | Error::Red(_));
                    if !reintentar {
                        if e.rompe_conexion() {
                            self.marcar(false);
                        }
                        return Err(e);
                    }
                    tracing::debug!("conexión antigua cerrada, reintentando: {e}");
                }
            }
        }
        unreachable!("el bucle siempre devuelve")
    }

    /// Como `enviar`, pero convierte los códigos 4xx/5xx en error.
    pub async fn comando(&self, comando: &str) -> Resultado<Respuesta> {
        let r = self.enviar(comando).await?;
        if r.es_correcta() {
            Ok(r)
        } else {
            Err(Error::Servidor { codigo: r.codigo, mensaje: describir_codigo(r.codigo, &r.cabecera) })
        }
    }

    // ---------- Comandos con tipos ----------

    /// Versión de CasparCG Server.
    pub async fn version(&self) -> Resultado<String> {
        let r = self.comando("VERSION").await?;
        Ok(r.datos.into_iter().next().unwrap_or_default())
    }

    /// Lista de archivos en la carpeta de medios de CasparCG.
    pub async fn cls(&self) -> Resultado<Vec<Medio>> {
        let r = self.comando("CLS").await?;
        Ok(r.datos.iter().filter_map(|l| Medio::desde_linea_cls(l)).collect())
    }

    /// Reproduce un clip de inmediato en `canal-capa`.
    pub async fn play(&self, canal: u16, capa: u16, clip: &str, op: OpcionesReproduccion) -> Resultado<()> {
        let mut cmd = format!("PLAY {canal}-{capa} {}", citar(clip));
        if op.bucle {
            cmd.push_str(" LOOP");
        }
        agregar_transicion(&mut cmd, &op);
        if op.desde_cuadro > 0 {
            cmd.push_str(&format!(" SEEK {}", op.desde_cuadro));
        }
        self.comando(&cmd).await.map(|_| ())
    }

    /// Salta a ese cuadro del clip que está en la capa (`CALL … SEEK`).
    pub async fn ir_a_cuadro(&self, canal: u16, capa: u16, cuadro: u64) -> Resultado<()> {
        self.comando(&format!("CALL {canal}-{capa} SEEK {cuadro}")).await.map(|_| ())
    }

    /// Precarga un clip "detrás" del actual. Con `auto`, empieza solo cuando termina el actual.
    pub async fn loadbg(
        &self,
        canal: u16,
        capa: u16,
        clip: &str,
        auto: bool,
        op: OpcionesReproduccion,
    ) -> Resultado<()> {
        let mut cmd = format!("LOADBG {canal}-{capa} {}", citar(clip));
        if op.bucle {
            cmd.push_str(" LOOP");
        }
        agregar_transicion(&mut cmd, &op);
        if auto {
            cmd.push_str(" AUTO");
        }
        self.comando(&cmd).await.map(|_| ())
    }

    /// Pausa la capa.
    pub async fn pause(&self, canal: u16, capa: u16) -> Resultado<()> {
        self.comando(&format!("PAUSE {canal}-{capa}")).await.map(|_| ())
    }

    /// Reanuda la capa pausada.
    pub async fn resume(&self, canal: u16, capa: u16) -> Resultado<()> {
        self.comando(&format!("RESUME {canal}-{capa}")).await.map(|_| ())
    }

    /// Pasa a primer plano lo precargado (`PLAY` sin clip).
    pub async fn play_precargado(&self, canal: u16, capa: u16) -> Resultado<()> {
        self.comando(&format!("PLAY {canal}-{capa}")).await.map(|_| ())
    }

    /// Detiene lo que suena en la capa.
    pub async fn stop(&self, canal: u16, capa: u16) -> Resultado<()> {
        self.comando(&format!("STOP {canal}-{capa}")).await.map(|_| ())
    }

    /// Vacía la capa por completo (también lo precargado).
    pub async fn clear(&self, canal: u16, capa: u16) -> Resultado<()> {
        self.comando(&format!("CLEAR {canal}-{capa}")).await.map(|_| ())
    }

    /// Carpetas de CasparCG (`INFO PATHS`): sobre todo, dónde está la carpeta de medios.
    pub async fn rutas(&self) -> Resultado<Rutas> {
        let r = self.enviar_largo("INFO PATHS").await?;
        if !r.es_correcta() {
            return Err(Error::Servidor { codigo: r.codigo, mensaje: describir_codigo(r.codigo, &r.cabecera) });
        }
        Rutas::desde_xml(&r.datos.join("\n")).ok_or_else(|| Error::Protocolo("INFO PATHS sin <media-path>".into()))
    }

    /// Miniatura PNG de un clip (`THUMBNAIL RETRIEVE`).
    pub async fn miniatura(&self, clip: &str) -> Resultado<Vec<u8>> {
        use base64::Engine;
        let r = self.comando(&format!("THUMBNAIL RETRIEVE {}", citar(clip))).await?;
        let texto: String = r.datos.concat();
        base64::engine::general_purpose::STANDARD
            .decode(texto.trim())
            .map_err(|e| Error::Protocolo(format!("miniatura en base64 no válida: {e}")))
    }

    /// Activa o desactiva el bucle del clip que ya se está reproduciendo.
    pub async fn bucle_actual(&self, canal: u16, capa: u16, activo: bool) -> Resultado<()> {
        let v = if activo { 1 } else { 0 };
        self.comando(&format!("CALL {canal}-{capa} LOOP {v}")).await.map(|_| ())
    }

    /// Volumen de la capa, de 0.0 (silencio) a 1.0 (normal).
    pub async fn volumen(&self, canal: u16, capa: u16, volumen: f32) -> Resultado<()> {
        let v = volumen.clamp(0.0, 2.0);
        self.comando(&format!("MIXER {canal}-{capa} VOLUME {v:.3}")).await.map(|_| ())
    }
}

async fn ejecutar(conexion: &mut Conexion, comando: &str, extra: bool) -> Resultado<Respuesta> {
    descartar_restos(&mut conexion.lector).await?;

    let linea = format!("{}\r\n", comando.trim_end());
    tracing::debug!("AMCP → {}", comando.trim_end());
    conexion.escritor.write_all(linea.as_bytes()).await?;
    conexion.escritor.flush().await?;

    let mut respuesta =
        timeout(ESPERA_RESPUESTA, leer_respuesta(&mut conexion.lector)).await.map_err(|_| Error::SinRespuesta)??;
    if extra && respuesta.es_correcta() {
        loop {
            let mut linea = String::new();
            match timeout(ESPERA_EXTRA, conexion.lector.read_line(&mut linea)).await {
                Ok(Ok(n)) if n > 0 => respuesta.datos.push(linea.trim_end_matches(['\r', '\n']).to_string()),
                Ok(Ok(_)) => return Err(Error::Desconectado("CasparCG cerró la conexión".into())),
                Ok(Err(e)) => return Err(e.into()),
                Err(_) => break,
            }
        }
    }
    tracing::debug!("AMCP ← {}", respuesta.cabecera);
    Ok(respuesta)
}

/// Algunas versiones de CasparCG mandan líneas extra tras un error.
/// Antes de cada comando se descarta lo que haya quedado pendiente,
/// para no confundir esa línea con la respuesta del comando siguiente.
async fn descartar_restos(lector: &mut BufReader<OwnedReadHalf>) -> Resultado<()> {
    loop {
        match timeout(ESPERA_RESTOS, lector.fill_buf()).await {
            Err(_) => return Ok(()), // no hay nada pendiente
            Ok(Err(e)) => return Err(e.into()),
            Ok(Ok([])) => return Err(Error::Desconectado("CasparCG cerró la conexión".into())),
            Ok(Ok(buf)) => {
                let n = buf.len();
                tracing::debug!("descartando {n} bytes sobrantes de CasparCG");
                lector.consume(n);
            }
        }
    }
}

fn describir_codigo(codigo: u16, cabecera: &str) -> String {
    let explicacion = match codigo {
        400 => "comando no reconocido",
        401 => "canal de video no válido",
        402 => "falta un parámetro",
        403 => "parámetro no válido",
        404 => "no se encontró el archivo",
        500 => "falló la operación",
        501 => "error interno de CasparCG",
        502 => "no se pudo leer el archivo",
        503 => "acceso denegado",
        _ => "error",
    };
    format!("{explicacion} — {cabecera}")
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    /// CasparCG simulado: responde a cada línea con lo que diga `responder`.
    /// Devuelve la dirección y un receptor con los comandos recibidos.
    async fn simulador<F>(responder: F) -> (String, tokio::sync::mpsc::UnboundedReceiver<String>)
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        let escucha = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = escucha.local_addr().unwrap().to_string();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let responder = Arc::new(responder);
        tokio::spawn(async move {
            loop {
                let (flujo, _) = escucha.accept().await.unwrap();
                let tx = tx.clone();
                let responder = responder.clone();
                tokio::spawn(async move {
                    let (l, mut e) = flujo.into_split();
                    let mut lector = BufReader::new(l);
                    let mut linea = String::new();
                    while lector.read_line(&mut linea).await.unwrap_or(0) > 0 {
                        let cmd = linea.trim_end().to_string();
                        linea.clear();
                        let resp = responder(&cmd);
                        tx.send(cmd).ok();
                        if resp == "CERRAR" {
                            return;
                        }
                        e.write_all(resp.as_bytes()).await.unwrap();
                    }
                });
            }
        });
        (dir, rx)
    }

    fn cliente(dir: &str) -> Cliente {
        let (host, puerto) = dir.rsplit_once(':').unwrap();
        Cliente::new(host, puerto.parse().unwrap())
    }

    #[tokio::test]
    async fn version_y_estado() {
        let (dir, _rx) = simulador(|_| "201 VERSION OK\r\n2.3.3 LTS\r\n".into()).await;
        let c = cliente(&dir);
        assert!(!c.esta_conectado());
        assert_eq!(c.version().await.unwrap(), "2.3.3 LTS");
        assert!(c.esta_conectado());
    }

    #[tokio::test]
    async fn play_con_bucle_envia_comando_correcto() {
        let (dir, mut rx) = simulador(|_| "202 PLAY OK\r\n".into()).await;
        let c = cliente(&dir);
        c.play(1, 20, "Clientes\\spot 1", OpcionesReproduccion { bucle: true, transicion: None, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(rx.recv().await.unwrap(), "PLAY 1-20 \"Clientes/spot 1\" LOOP");
    }

    #[tokio::test]
    async fn transiciones() {
        let (dir, mut rx) = simulador(|_| "202 OK\r\n".into()).await;
        let c = cliente(&dir);
        let op = OpcionesReproduccion { bucle: false, transicion: Some(("MIX", 15)), ..Default::default() };
        c.play(1, 20, "SPOT", op).await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), "PLAY 1-20 \"SPOT\" MIX 15");
        c.loadbg(1, 20, "OTRO", true, op).await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), "LOADBG 1-20 \"OTRO\" MIX 15 AUTO");
        // 0 cuadros = corte.
        let corte = OpcionesReproduccion { bucle: false, transicion: Some(("MIX", 0)), ..Default::default() };
        c.play(1, 20, "SPOT", corte).await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), "PLAY 1-20 \"SPOT\"");
        c.play_precargado(1, 20).await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), "PLAY 1-20");
        // Empezar más adelante y saltar dentro del clip.
        let desde = OpcionesReproduccion { desde_cuadro: 450, ..Default::default() };
        c.play(1, 20, "SPOT", desde).await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), "PLAY 1-20 \"SPOT\" SEEK 450");
        c.ir_a_cuadro(1, 20, 90).await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), "CALL 1-20 SEEK 90");
    }

    #[tokio::test]
    async fn error_del_servidor() {
        let (dir, _rx) = simulador(|_| "404 PLAY FAILED\r\n".into()).await;
        let c = cliente(&dir);
        let e = c.play(1, 20, "NO_EXISTE", Default::default()).await.unwrap_err();
        assert!(matches!(e, Error::Servidor { codigo: 404, .. }));
        // Un 404 no rompe la conexión.
        assert!(c.esta_conectado());
    }

    #[tokio::test]
    async fn linea_extra_tras_error_no_desincroniza() {
        // Simula versiones que devuelven "400 ERROR" + el comando repetido.
        let (dir, _rx) = simulador(|cmd| {
            if cmd.starts_with("MALO") {
                format!("400 ERROR\r\n{cmd}\r\n")
            } else {
                "201 VERSION OK\r\n2.3.3\r\n".into()
            }
        })
        .await;
        let c = cliente(&dir);
        assert!(c.comando("MALO").await.is_err());
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(c.version().await.unwrap(), "2.3.3");
    }

    #[tokio::test]
    async fn info_paths_en_varias_lineas() {
        let (dir, _rx) = simulador(|cmd| {
            if cmd == "INFO PATHS" {
                "201 INFO PATHS OK\r\n<?xml version=\"1.0\"?>\r\n<paths>\r\n<media-path>media/</media-path>\r\n<initial-path>C:\\caspar\\</initial-path>\r\n</paths>\r\n".into()
            } else {
                "201 VERSION OK\r\n2.3.3\r\n".into()
            }
        })
        .await;
        let c = cliente(&dir);
        let r = c.rutas().await.unwrap();
        assert_eq!(r.carpeta_medios(), "C:\\caspar\\media");
        // Lo que sigue no queda desincronizado.
        assert_eq!(c.version().await.unwrap(), "2.3.3");
    }

    #[tokio::test]
    async fn miniatura_base64() {
        let (dir, _rx) = simulador(|_| "201 THUMBNAIL RETRIEVE OK\r\naG9sYQ==\r\n".into()).await;
        let c = cliente(&dir);
        assert_eq!(c.miniatura("SPOT").await.unwrap(), b"hola");
    }

    #[tokio::test]
    async fn cls_devuelve_medios() {
        let (dir, _rx) = simulador(|_| {
            "200 CLS OK\r\n\"SPOT1\" MOVIE 10 20240101000000 750 1/25\r\n\"LOGO\" STILL 5 20240101000000 0 0/1\r\n\r\n"
                .into()
        })
        .await;
        let c = cliente(&dir);
        let medios = c.cls().await.unwrap();
        assert_eq!(medios.len(), 2);
        assert_eq!(medios[0].nombre, "SPOT1");
    }

    #[tokio::test]
    async fn reconecta_si_el_servidor_cierra() {
        // El simulador cierra la conexión tras cada respuesta "CERRAR"; la primera
        // conexión responde VERSION y luego se cierra con el siguiente comando.
        let contador = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c2 = contador.clone();
        let (dir, _rx) = simulador(move |_| {
            let n = c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 1 {
                "CERRAR".into()
            } else {
                "201 VERSION OK\r\nok\r\n".into()
            }
        })
        .await;
        let c = cliente(&dir);
        assert_eq!(c.version().await.unwrap(), "ok");
        // El segundo comando encuentra la conexión cerrada y reintenta con una nueva.
        assert_eq!(c.version().await.unwrap(), "ok");
        assert!(c.esta_conectado());
    }

    #[tokio::test]
    async fn sin_servidor_queda_desconectado() {
        // Puerto cerrado: se reserva y se suelta para obtener uno libre.
        let escucha = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = escucha.local_addr().unwrap().to_string();
        drop(escucha);
        let c = cliente(&dir);
        assert!(matches!(c.version().await, Err(Error::Desconectado(_))));
        assert!(!c.esta_conectado());
    }

    #[tokio::test]
    async fn sin_respuesta_da_error_de_tiempo() {
        let escucha = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = escucha.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (mut f, _) = escucha.accept().await.unwrap();
            let mut buf = [0u8; 64];
            // Lee pero nunca responde.
            loop {
                if f.read(&mut buf).await.unwrap_or(0) == 0 {
                    break;
                }
            }
        });
        // Tarda lo que dure ESPERA_RESPUESTA (5 s).
        let c = cliente(&dir);
        assert!(matches!(c.version().await, Err(Error::SinRespuesta)));
        assert!(!c.esta_conectado());
    }
}
