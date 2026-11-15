//! Modo remoto: la PC principal ofrece un servicio en la red local (WebSocket) y
//! otras PCs con Relevo se conectan a él con usuario y contraseña.
//!
//! - La PC remota recibe el estado completo varias veces por segundo y lo dibuja
//!   con la misma interfaz.
//! - Sus órdenes llegan al núcleo de la PC principal con su nombre y rol, y el
//!   núcleo aplica los permisos y el **mando**.
//! - Los videos de la PC remota se **suben** a la principal (a `media/IMPORTADOS`, o
//!   para reemplazar el video de un spot). Si la red se corta a mitad de una subida,
//!   al reconectar **continúa desde donde quedó**.
//! - Si la red se corta, la emisión sigue en la principal; la remota se reconecta sola.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::archivos;
use crate::biblioteca::{Ficha, TipoTrabajo};
use crate::controlador::{
    Controlador, Destino, Importacion, Instantanea, MapaMiniaturas, Miniatura, Orden, Origen, PedidoSubida, Ranura,
};
use crate::pauta::formato;
use crate::usuarios::Usuarios;

/// Versión del protocolo (si cambia, hay que actualizar ambas PCs).
pub const VERSION_PROTOCOLO: u32 = 9;
const TROZO: usize = 1024 * 1024;
const INTERVALO_ESTADO: Duration = Duration::from_millis(150);
/// Las subidas a medias se guardan este tiempo por si la PC remota vuelve.
const VIDA_PARTES: Duration = Duration::from_secs(2 * 24 * 3600);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t")]
enum DeCliente {
    Login {
        usuario: String,
        clave: String,
        version: u32,
    },
    Orden {
        orden: Orden,
    },
    Miniatura {
        clip: String,
    },
    /// Empieza (o retoma) una subida. `huella` identifica el archivo (nombre, tamaño y fecha).
    SubirInicio {
        id: u32,
        nombre: String,
        tamano: u64,
        huella: String,
        duracion: Option<Duration>,
        destino: Destino,
        /// Si viene, el archivo reemplaza el video de este clip.
        reemplazar: Option<String>,
    },
    SubirFin {
        id: u32,
    },
    /// Pide un reporte de emisión (fechas `AAAA-MM-DD`, hora `HH:MM`).
    Reporte {
        id: u32,
        desde: String,
        hasta: String,
        hoy: String,
        ahora: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t")]
enum DeServidor {
    Bienvenida {
        sesion: u64,
        nombre: String,
        rol: crate::controlador::Rol,
    },
    Rechazo {
        motivo: String,
    },
    Estado {
        estado: Box<Instantanea>,
    },
    Medios {
        medios: Vec<amcp::Medio>,
    },
    Fichas {
        fichas: HashMap<String, Ficha>,
    },
    Miniatura {
        clip: String,
        png: Option<String>,
    },
    /// Respuesta a `SubirInicio`: desde qué byte hay que enviar (lo anterior ya llegó).
    SubirDesde {
        id: u32,
        desde: u64,
    },
    Subida {
        id: u32,
        progreso: f32,
        error: Option<String>,
        terminada: bool,
    },
    Reporte {
        id: u32,
        reporte: Result<crate::reportes::Reporte, String>,
    },
}

fn texto<T: Serialize>(m: &T) -> Message {
    Message::text(serde_json::to_string(m).unwrap_or_default())
}

/// Huella corta y estable (FNV-1a) para nombrar la subida a medias de un archivo.
fn hash(t: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in t.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

// ======================================================================
// Servidor (PC principal)
// ======================================================================

/// Abre el servicio de red en `puerto`. Devuelve un error si el puerto está ocupado.
pub async fn servir(puerto: u16, ctl: Controlador, usuarios: Arc<Usuarios>) -> std::io::Result<()> {
    let escucha = TcpListener::bind(("0.0.0.0", puerto)).await?;
    tracing::info!("modo remoto: escuchando en el puerto {puerto}");
    limpiar_partes_viejas(&ctl.ver().carpeta_datos.join("subidas"));
    let siguiente = Arc::new(AtomicU64::new(1));
    tokio::spawn(async move {
        loop {
            let Ok((flujo, dir)) = escucha.accept().await else { continue };
            flujo.set_nodelay(true).ok();
            let (ctl, usuarios, siguiente) = (ctl.clone(), usuarios.clone(), siguiente.clone());
            tokio::spawn(async move {
                if let Err(e) = atender(flujo, ctl, usuarios, siguiente).await {
                    tracing::info!("conexión remota {dir} terminada: {e}");
                }
            });
        }
    });
    Ok(())
}

fn limpiar_partes_viejas(carpeta: &Path) {
    let Ok(l) = std::fs::read_dir(carpeta) else { return };
    for e in l.flatten() {
        let viejo = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
        if viejo.is_some_and(|v| v > VIDA_PARTES) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

struct SubidaEnCurso {
    parte: PathBuf,
    nombre: String,
    archivo: tokio::fs::File,
    tamano: u64,
    recibido: u64,
    ultimo_aviso: f32,
    duracion: Option<Duration>,
    destino: Destino,
    reemplazar: Option<String>,
}

async fn atender(
    flujo: TcpStream,
    ctl: Controlador,
    usuarios: Arc<Usuarios>,
    siguiente: Arc<AtomicU64>,
) -> Result<(), String> {
    let ws = tokio_tungstenite::accept_async(flujo).await.map_err(|e| e.to_string())?;
    let (mut tx, mut rx) = ws.split();

    // 1) Inicio de sesión (con 15 s de plazo).
    let primero = tokio::time::timeout(Duration::from_secs(15), rx.next())
        .await
        .map_err(|_| "no inició sesión a tiempo".to_string())?;
    let Some(Ok(Message::Text(t))) = primero else { return Err("conexión sin inicio de sesión".into()) };
    let Ok(DeCliente::Login { usuario, clave, version }) = serde_json::from_str::<DeCliente>(&t) else {
        let m = DeServidor::Rechazo {
            motivo: "La versión de Relevo de esta PC no coincide con la de la PC principal.".into(),
        };
        let _ = tx.send(texto(&m)).await;
        return Err("mensaje inicial no válido".into());
    };
    if version != VERSION_PROTOCOLO {
        let m = DeServidor::Rechazo {
            motivo: "La versión de Relevo de esta PC no coincide con la de la PC principal.".into(),
        };
        let _ = tx.send(texto(&m)).await;
        return Err("versión distinta".into());
    }
    let usuarios_c = usuarios.clone();
    let (u, c) = (usuario.clone(), clave);
    let rol = tokio::task::spawn_blocking(move || usuarios_c.verificar(&u, &c)).await.ok().flatten();
    let Some(rol) = rol else {
        tokio::time::sleep(Duration::from_millis(800)).await; // frena los intentos a ciegas
        let _ = tx.send(texto(&DeServidor::Rechazo { motivo: "Usuario o contraseña incorrectos.".into() })).await;
        return Err(format!("contraseña incorrecta para {usuario}"));
    };
    let sesion = siguiente.fetch_add(1, Ordering::SeqCst);
    let origen = Origen { id: sesion, nombre: usuario.trim().to_string(), rol };
    tx.send(texto(&DeServidor::Bienvenida { sesion, nombre: origen.nombre.clone(), rol }))
        .await
        .map_err(|e| e.to_string())?;
    ctl.enviar_como(origen.clone(), Orden::SesionAbierta);

    // 2) Un canal para todo lo que sale hacia esta PC.
    let (tx_salida, mut rx_salida) = mpsc::unbounded_channel::<Message>();
    let escritor = tokio::spawn(async move {
        while let Some(m) = rx_salida.recv().await {
            if tx.send(m).await.is_err() {
                return;
            }
        }
    });

    // 3) Estado periódico (solo si cambió); medios y fichas aparte, cuando cambian.
    let estado_tarea = {
        let (ctl, tx_salida) = (ctl.clone(), tx_salida.clone());
        tokio::spawn(async move {
            let mut ultimo = String::new();
            let mut medios_enviados: Option<Arc<Vec<amcp::Medio>>> = None;
            let mut fichas_enviadas: Option<Arc<HashMap<String, Ficha>>> = None;
            loop {
                let mut e = ctl.ver();
                if medios_enviados.as_ref().is_none_or(|m| !Arc::ptr_eq(m, &e.medios)) {
                    medios_enviados = Some(e.medios.clone());
                    if tx_salida.send(texto(&DeServidor::Medios { medios: e.medios.to_vec() })).is_err() {
                        return;
                    }
                }
                if fichas_enviadas.as_ref().is_none_or(|f| !Arc::ptr_eq(f, &e.fichas)) {
                    fichas_enviadas = Some(e.fichas.clone());
                    if tx_salida.send(texto(&DeServidor::Fichas { fichas: (*e.fichas).clone() })).is_err() {
                        return;
                    }
                }
                e.medios = Arc::new(Vec::new()); // ya se mandaron aparte
                e.fichas = Arc::new(HashMap::new());
                let t = serde_json::to_string(&DeServidor::Estado { estado: Box::new(e) }).unwrap_or_default();
                if t != ultimo {
                    if tx_salida.send(Message::text(t.clone())).is_err() {
                        return;
                    }
                    ultimo = t;
                }
                tokio::time::sleep(INTERVALO_ESTADO).await;
            }
        })
    };

    // 4) Lo que llega de la PC remota.
    let mut subidas: HashMap<u32, SubidaEnCurso> = HashMap::new();
    let resultado = loop {
        let Some(m) = rx.next().await else { break Ok(()) };
        let m = match m {
            Ok(m) => m,
            Err(e) => break Err(e.to_string()),
        };
        match m {
            Message::Text(t) => match serde_json::from_str::<DeCliente>(&t) {
                Ok(DeCliente::Orden { orden }) if !orden.es_interna() => ctl.enviar_como(origen.clone(), orden),
                Ok(DeCliente::Miniatura { clip }) => {
                    let (caspar, tx_s) = (ctl.caspar().cloned(), tx_salida.clone());
                    tokio::spawn(async move {
                        let png = match caspar {
                            Some(c) => c.miniatura(&clip).await.ok(),
                            None => None,
                        };
                        let png =
                            png.filter(|p| !p.is_empty()).map(|p| base64::engine::general_purpose::STANDARD.encode(p));
                        let _ = tx_s.send(texto(&DeServidor::Miniatura { clip, png }));
                    });
                }
                Ok(DeCliente::SubirInicio { id, nombre, tamano, huella, duracion, destino, reemplazar }) => {
                    let falta_rol = if reemplazar.is_some() {
                        origen.rol < crate::controlador::Rol::Editor
                    } else {
                        origen.rol < crate::controlador::Rol::Operador
                    };
                    let r = if falta_rol {
                        Err("Tu usuario no puede subir videos para reemplazar (hace falta rol Editor).".to_string())
                    } else {
                        abrir_subida(&ctl, &nombre, tamano, &huella, duracion, destino, reemplazar).await
                    };
                    match r {
                        Ok(s) => {
                            let desde = s.recibido;
                            subidas.insert(id, s);
                            let _ = tx_salida.send(texto(&DeServidor::SubirDesde { id, desde }));
                        }
                        Err(e) => {
                            let _ = tx_salida.send(texto(&DeServidor::Subida {
                                id,
                                progreso: 0.0,
                                error: Some(e),
                                terminada: true,
                            }));
                        }
                    }
                }
                Ok(DeCliente::Reporte { id, desde, hasta, hoy, ahora }) => {
                    let e = ctl.ver();
                    let tx_s = tx_salida.clone();
                    tokio::task::spawn_blocking(move || {
                        let fechas = (
                            formato::leer_fecha(&desde),
                            formato::leer_fecha(&hasta),
                            formato::leer_fecha(&hoy),
                            formato::leer_hora(&ahora),
                        );
                        let reporte = match fechas {
                            (Some(d), Some(h), Some(hoy), Some(ahora)) => {
                                Ok(crate::reportes::calcular(&e.carpeta_datos, &e.pautas, d, h, hoy, ahora))
                            }
                            _ => Err("fechas no válidas".to_string()),
                        };
                        let _ = tx_s.send(texto(&DeServidor::Reporte { id, reporte }));
                    });
                }
                Ok(DeCliente::SubirFin { id }) => {
                    if let Some(s) = subidas.remove(&id) {
                        let r = terminar_subida(s, &ctl, &origen).await;
                        let error = r.err();
                        let _ =
                            tx_salida.send(texto(&DeServidor::Subida { id, progreso: 1.0, error, terminada: true }));
                    }
                }
                _ => {}
            },
            Message::Binary(b) if b.len() >= 4 => {
                let id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                if let Some(s) = subidas.get_mut(&id) {
                    if let Err(e) = s.archivo.write_all(&b[4..]).await {
                        let _ = tx_salida.send(texto(&DeServidor::Subida {
                            id,
                            progreso: 0.0,
                            error: Some(e.to_string()),
                            terminada: true,
                        }));
                        subidas.remove(&id);
                        continue;
                    }
                    s.recibido += (b.len() - 4) as u64;
                    let p = s.recibido as f32 / s.tamano.max(1) as f32;
                    if p - s.ultimo_aviso >= 0.02 {
                        s.ultimo_aviso = p;
                        let _ = tx_salida.send(texto(&DeServidor::Subida {
                            id,
                            progreso: p,
                            error: None,
                            terminada: false,
                        }));
                    }
                }
            }
            Message::Close(_) => break Ok(()),
            _ => {}
        }
    };

    // 5) Limpieza. Las subidas a medias se conservan (en datos/subidas) para continuarlas.
    for (_, mut s) in subidas {
        let _ = s.archivo.flush().await;
    }
    ctl.enviar_como(origen, Orden::SesionCerrada);
    estado_tarea.abort();
    escritor.abort();
    resultado
}

/// Abre (o retoma) el archivo donde se va guardando una subida.
async fn abrir_subida(
    ctl: &Controlador,
    nombre: &str,
    tamano: u64,
    huella: &str,
    duracion: Option<Duration>,
    destino: Destino,
    reemplazar: Option<String>,
) -> Result<SubidaEnCurso, String> {
    let e = ctl.ver();
    if e.carpeta_medios.is_none() {
        return Err("La PC principal todavía no conoce la carpeta de CasparCG.".into());
    }
    // Solo el nombre del archivo: nunca rutas que salgan de la carpeta.
    let nombre = Path::new(nombre).file_name().ok_or("nombre de archivo no válido")?.to_string_lossy().into_owned();
    if !archivos::es_video(Path::new(&nombre)) {
        return Err("Solo se aceptan videos.".into());
    }
    let carpeta = e.carpeta_datos.join("subidas");
    tokio::fs::create_dir_all(&carpeta).await.map_err(|e| e.to_string())?;
    let parte = carpeta.join(format!("{}.parte", hash(&format!("{huella}|{tamano}"))));
    let mut archivo = tokio::fs::OpenOptions::new()
        .create(true)
        .append(false)
        .write(true)
        .truncate(false)
        .open(&parte)
        .await
        .map_err(|e| e.to_string())?;
    let mut recibido = archivo.metadata().await.map_err(|e| e.to_string())?.len();
    if recibido > tamano {
        archivo.set_len(0).await.map_err(|e| e.to_string())?;
        recibido = 0;
    }
    archivo.seek(std::io::SeekFrom::Start(recibido)).await.map_err(|e| e.to_string())?;
    if recibido > 0 {
        tracing::info!("subida de «{nombre}»: continúa desde {recibido} de {tamano} bytes");
    }
    Ok(SubidaEnCurso { parte, nombre, archivo, tamano, recibido, ultimo_aviso: 0.0, duracion, destino, reemplazar })
}

async fn terminar_subida(mut s: SubidaEnCurso, ctl: &Controlador, origen: &Origen) -> Result<(), String> {
    s.archivo.flush().await.map_err(|e| e.to_string())?;
    s.archivo.sync_all().await.map_err(|e| e.to_string())?;
    drop(s.archivo);
    if s.recibido != s.tamano {
        let _ = tokio::fs::remove_file(&s.parte).await;
        return Err(format!("llegaron {} de {} bytes", s.recibido, s.tamano));
    }
    let e = ctl.ver();
    let medios = e.carpeta_medios.ok_or("La PC principal no conoce la carpeta de CasparCG.")?;
    match s.reemplazar {
        // Para reemplazar: queda fuera de la carpeta de CasparCG hasta que el núcleo lo ponga en su lugar.
        Some(clip) => {
            let destino = e.carpeta_datos.join("temporal").join(format!("subido-{}", s.nombre));
            let parte = s.parte.clone();
            let d = destino.clone();
            tokio::task::spawn_blocking(move || crate::biblioteca::mover(&parte, &d))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            ctl.enviar_como(origen.clone(), Orden::Trabajo { clip, tipo: TipoTrabajo::Reemplazar { desde: destino } });
        }
        None => {
            let carpeta = medios.join(archivos::CARPETA_IMPORTADOS);
            let destino = nombre_libre(&carpeta, &s.nombre, s.tamano);
            let parte = s.parte.clone();
            let d = destino.clone();
            tokio::task::spawn_blocking(move || crate::biblioteca::mover(&parte, &d))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            ctl.enviar_como(
                origen.clone(),
                Orden::AgregarArchivo { ruta: destino, duracion: s.duracion, destino: s.destino },
            );
        }
    }
    Ok(())
}

/// `carpeta/nombre`, o `nombre (2)`, `(3)`… si ya hay otro archivo distinto con ese nombre.
fn nombre_libre(carpeta: &Path, nombre: &str, tamano: u64) -> PathBuf {
    let mut destino = carpeta.join(nombre);
    let base = Path::new(nombre).file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let ext = Path::new(nombre).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut n = 2;
    while let Ok(m) = std::fs::metadata(&destino) {
        if m.len() == tamano {
            break; // el mismo video: se sobrescribe
        }
        destino = carpeta.join(format!("{base} ({n}){ext}"));
        n += 1;
    }
    destino
}

// ======================================================================
// Cliente (PC remota)
// ======================================================================

/// Estado de la conexión con la PC principal.
#[derive(Debug, Clone, PartialEq)]
pub enum Conexion {
    Conectando,
    Conectada {
        nombre: String,
        rol: crate::controlador::Rol,
    },
    /// Usuario o contraseña incorrectos, o versión distinta: no se reintenta.
    Rechazada(String),
    /// Se cortó; se reintenta solo.
    Caida(String),
}

pub struct Ajustes {
    pub servidor: String,
    pub puerto: u16,
    pub usuario: String,
    pub clave: String,
}

/// Lo que comparten las subidas de una PC remota.
#[derive(Clone)]
struct Subidor {
    rt: tokio::runtime::Handle,
    tx_trozo: mpsc::Sender<Message>,
    /// Subidas pendientes (clave estable → pedido). Se borran al terminar.
    pendientes: Arc<Mutex<HashMap<u64, PedidoSubida>>>,
    /// Intento en curso (id del protocolo) → clave de la subida.
    intentos: Arc<Mutex<HashMap<u32, u64>>>,
    /// Respuestas `SubirDesde` que espera cada intento.
    esperas: Arc<Mutex<HashMap<u32, oneshot::Sender<u64>>>>,
    /// Barra de progreso de cada subida.
    vista: Arc<Mutex<Vec<Importacion>>>,
    siguiente_intento: Arc<AtomicU64>,
    /// Cambia cada vez que se corta la conexión: los intentos viejos se detienen.
    generacion: Arc<AtomicU64>,
    conectado: Arc<AtomicBool>,
    al_cambiar: Arc<dyn Fn() + Send + Sync>,
}

impl Subidor {
    /// Lanza (o relanza tras un corte) la subida `clave`.
    fn lanzar(&self, clave: u64) {
        let Some(p) = self.pendientes.lock().expect("pendientes").get(&clave).cloned() else { return };
        let id = self.siguiente_intento.fetch_add(1, Ordering::SeqCst) as u32;
        self.intentos.lock().expect("intentos").insert(id, clave);
        let (rx, generacion) = {
            let (tx, rx) = oneshot::channel();
            self.esperas.lock().expect("esperas").insert(id, tx);
            (rx, self.generacion.load(Ordering::SeqCst))
        };
        let yo = self.clone();
        self.rt.spawn(async move {
            match yo.subir(id, &p, rx, generacion).await {
                // Se olvida cuando la principal confirma (mensaje `Subida` terminada).
                Ok(()) => {}
                Err(e) if yo.generacion.load(Ordering::SeqCst) != generacion => {
                    tracing::info!("subida interrumpida por la red ({e}); continuará al reconectar");
                    yo.intentos.lock().expect("intentos").remove(&id);
                }
                Err(e) => {
                    // Si la principal ya la rechazó, este intento ya se olvidó.
                    if yo.intentos.lock().expect("intentos").remove(&id).is_some() {
                        tracing::warn!("no se pudo subir {}: {e}", p.ruta.display());
                        yo.olvidar(clave);
                    }
                }
            }
        });
    }

    async fn subir(
        &self,
        id: u32,
        p: &PedidoSubida,
        rx: oneshot::Receiver<u64>,
        generacion: u64,
    ) -> Result<(), String> {
        let mut f = tokio::fs::File::open(&p.ruta).await.map_err(|e| e.to_string())?;
        let meta = f.metadata().await.map_err(|e| e.to_string())?;
        let tamano = meta.len();
        let nombre = p.ruta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let fecha = crate::biblioteca::segundos(meta.modified().ok());
        let inicio = DeCliente::SubirInicio {
            id,
            nombre: nombre.clone(),
            tamano,
            huella: format!("{nombre}|{fecha}"),
            duracion: p.duracion,
            destino: p.destino,
            reemplazar: p.reemplazar.clone(),
        };
        self.tx_trozo.send(texto(&inicio)).await.map_err(|e| e.to_string())?;
        let desde = tokio::time::timeout(Duration::from_secs(20), rx)
            .await
            .map_err(|_| "la PC principal no respondió".to_string())?
            .map_err(|_| "la subida se canceló".to_string())?;
        f.seek(std::io::SeekFrom::Start(desde)).await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; TROZO];
        loop {
            if self.generacion.load(Ordering::SeqCst) != generacion {
                return Err("se cortó la conexión".into());
            }
            let n = f.read(&mut buf).await.map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            let mut trozo = Vec::with_capacity(n + 4);
            trozo.extend_from_slice(&id.to_le_bytes());
            trozo.extend_from_slice(&buf[..n]);
            self.tx_trozo.send(Message::binary(trozo)).await.map_err(|e| e.to_string())?;
        }
        if self.generacion.load(Ordering::SeqCst) != generacion {
            return Err("se cortó la conexión".into());
        }
        self.tx_trozo.send(texto(&DeCliente::SubirFin { id })).await.map_err(|e| e.to_string())
    }

    fn olvidar(&self, clave: u64) {
        self.pendientes.lock().expect("pendientes").remove(&clave);
        self.vista.lock().expect("vista").retain(|s| s.id != clave);
        (self.al_cambiar)();
    }

    /// Respuesta del servidor a un intento.
    fn desde(&self, id: u32, desde: u64) {
        if let Some(tx) = self.esperas.lock().expect("esperas").remove(&id) {
            let _ = tx.send(desde);
        }
    }

    fn avance(&self, id: u32, progreso: f32, error: Option<String>, terminada: bool) {
        let Some(clave) = self.intentos.lock().expect("intentos").get(&id).copied() else { return };
        if terminada {
            self.intentos.lock().expect("intentos").remove(&id);
            // Si esperaba la respuesta de inicio, deja de esperar.
            self.esperas.lock().expect("esperas").remove(&id);
            if let Some(e) = error {
                tracing::warn!("la subida falló: {e}");
            }
            self.olvidar(clave);
        } else if let Some(x) = self.vista.lock().expect("vista").iter_mut().find(|x| x.id == clave) {
            x.progreso = progreso;
        }
    }
}

/// Se conecta a la PC principal y devuelve un [`Controlador`] que se usa igual que el local.
pub fn conectar(
    rt: &tokio::runtime::Handle,
    aj: Ajustes,
    al_cambiar: Arc<dyn Fn() + Send + Sync>,
) -> (Controlador, Arc<Mutex<Conexion>>) {
    let direccion = format!("{}:{}", aj.servidor.trim(), aj.puerto);
    let estado = Arc::new(Mutex::new(Instantanea::vacia(&direccion)));
    let conexion = Arc::new(Mutex::new(Conexion::Conectando));
    let miniaturas: MapaMiniaturas = Arc::new(Mutex::new(HashMap::new()));
    let sesion = Arc::new(AtomicU64::new(u64::MAX));

    let (tx_texto, rx_texto) = mpsc::unbounded_channel::<String>();
    // Las subidas van por su propio canal (con límite y en orden: inicio, trozos, fin).
    let (tx_trozo, rx_trozo) = mpsc::channel::<Message>(8);

    let subidor = Subidor {
        rt: rt.clone(),
        tx_trozo,
        pendientes: Arc::default(),
        intentos: Arc::default(),
        esperas: Arc::default(),
        vista: Arc::default(),
        siguiente_intento: Arc::new(AtomicU64::new(1)),
        generacion: Arc::new(AtomicU64::new(0)),
        conectado: Arc::new(AtomicBool::new(false)),
        al_cambiar: al_cambiar.clone(),
    };

    let enviar = {
        let tx = tx_texto.clone();
        Arc::new(move |o: Orden| {
            let _ = tx.send(serde_json::to_string(&DeCliente::Orden { orden: o }).unwrap_or_default());
        })
    };
    let pedir = {
        let tx = tx_texto.clone();
        Arc::new(move |clip: String| {
            let _ = tx.send(serde_json::to_string(&DeCliente::Miniatura { clip }).unwrap_or_default());
        })
    };
    let reportes: Arc<Mutex<HashMap<u32, Ranura>>> = Arc::default();
    let pedir_reporte = {
        let (tx, reportes, conectado) = (tx_texto.clone(), reportes.clone(), subidor.conectado.clone());
        let siguiente = Arc::new(AtomicU64::new(1));
        Arc::new(move |desde: time::Date, hasta: time::Date, hoy: time::Date, ahora: time::Time, r: Ranura| {
            if !conectado.load(Ordering::SeqCst) {
                *r.lock().expect("ranura") = Some(Err("No hay conexión con la PC principal".into()));
                return;
            }
            let id = siguiente.fetch_add(1, Ordering::SeqCst) as u32;
            reportes.lock().expect("reportes").insert(id, r);
            let m = DeCliente::Reporte {
                id,
                desde: formato::texto_fecha(desde),
                hasta: formato::texto_fecha(hasta),
                hoy: formato::texto_fecha(hoy),
                ahora: formato::texto_hora(ahora),
            };
            let _ = tx.send(serde_json::to_string(&m).unwrap_or_default());
        })
    };
    let siguiente_clave = Arc::new(AtomicU64::new(1));
    let subir = {
        let s = subidor.clone();
        Arc::new(move |p: PedidoSubida| {
            let clave = siguiente_clave.fetch_add(1, Ordering::SeqCst);
            let nombre = p.ruta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let flecha = if p.reemplazar.is_some() { "↑ reemplazo:" } else { "↑" };
            s.vista.lock().expect("vista").push(Importacion {
                id: clave,
                nombre: format!("{flecha} {nombre}"),
                progreso: 0.0,
            });
            s.pendientes.lock().expect("pendientes").insert(clave, p);
            // Sin conexión, espera: se lanza al reconectar.
            if s.conectado.load(Ordering::SeqCst) {
                s.lanzar(clave);
            }
            (s.al_cambiar)();
        })
    };

    let ctl = Controlador {
        enviar,
        estado: estado.clone(),
        miniaturas: miniaturas.clone(),
        pedir_miniatura: pedir,
        subir: Some(subir),
        sesion: sesion.clone(),
        tx_local: None,
        caspar: None,
        pedir_reporte: Some(pedir_reporte),
        salida: None,
    };

    let tarea = Tarea { aj, estado, conexion: conexion.clone(), miniaturas, sesion, subidor, reportes, al_cambiar };
    rt.spawn(tarea.correr(rx_texto, rx_trozo));
    (ctl, conexion)
}

struct Tarea {
    aj: Ajustes,
    estado: Arc<Mutex<Instantanea>>,
    conexion: Arc<Mutex<Conexion>>,
    miniaturas: MapaMiniaturas,
    sesion: Arc<AtomicU64>,
    subidor: Subidor,
    /// Reportes pedidos que esperan respuesta.
    reportes: Arc<Mutex<HashMap<u32, Ranura>>>,
    al_cambiar: Arc<dyn Fn() + Send + Sync>,
}

impl Tarea {
    fn poner(&self, c: Conexion) {
        *self.conexion.lock().expect("conexion") = c;
        (self.al_cambiar)();
    }

    /// Las barras de subida de esta PC se muestran junto a las copias de la principal.
    fn con_subidas(&self, e: &mut Instantanea) {
        e.importaciones.retain(|x| !x.nombre.starts_with('↑'));
        e.importaciones.extend(self.subidor.vista.lock().expect("vista").iter().cloned());
    }

    async fn correr(self, mut rx_texto: mpsc::UnboundedReceiver<String>, mut rx_trozo: mpsc::Receiver<Message>) {
        let url = format!("ws://{}:{}/", self.aj.servidor.trim(), self.aj.puerto);
        let mut medios: Arc<Vec<amcp::Medio>> = Arc::new(Vec::new());
        let mut fichas: Arc<HashMap<String, Ficha>> = Arc::new(HashMap::new());
        loop {
            self.poner(Conexion::Conectando);
            let ws = match tokio::time::timeout(Duration::from_secs(5), tokio_tungstenite::connect_async(&url)).await {
                Ok(Ok((ws, _))) => ws,
                Ok(Err(e)) => {
                    self.poner(Conexion::Caida(format!("No se pudo conectar a {url}: {e}")));
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    continue;
                }
                Err(_) => {
                    self.poner(Conexion::Caida(format!("La PC principal ({url}) no responde")));
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    continue;
                }
            };
            let (mut tx, mut rx) = ws.split();
            let login = DeCliente::Login {
                usuario: self.aj.usuario.clone(),
                clave: self.aj.clave.clone(),
                version: VERSION_PROTOCOLO,
            };
            if tx.send(texto(&login)).await.is_err() {
                continue;
            }
            // Lo que se pidió mientras no había conexión se descarta (podría ser viejo).
            while rx_texto.try_recv().is_ok() {}

            let motivo = loop {
                tokio::select! {
                    m = rx.next() => {
                        let Some(Ok(m)) = m else { break "Se cortó la conexión con la PC principal".to_string() };
                        let Message::Text(t) = m else { continue };
                        match serde_json::from_str::<DeServidor>(&t) {
                            Ok(DeServidor::Bienvenida { sesion, nombre, rol }) => {
                                self.sesion.store(sesion, Ordering::SeqCst);
                                self.poner(Conexion::Conectada { nombre, rol });
                                // Subidas pendientes (nuevas o cortadas): continúan.
                                self.subidor.conectado.store(true, Ordering::SeqCst);
                                let claves: Vec<u64> = self.subidor.pendientes.lock().expect("pendientes").keys().copied().collect();
                                for c in claves {
                                    self.subidor.lanzar(c);
                                }
                            }
                            Ok(DeServidor::Rechazo { motivo }) => {
                                self.poner(Conexion::Rechazada(motivo));
                                return;
                            }
                            Ok(DeServidor::Medios { medios: m }) => {
                                medios = Arc::new(m);
                                self.estado.lock().expect("estado").medios = medios.clone();
                                (self.al_cambiar)();
                            }
                            Ok(DeServidor::Fichas { fichas: f }) => {
                                fichas = Arc::new(f);
                                self.estado.lock().expect("estado").fichas = fichas.clone();
                                (self.al_cambiar)();
                            }
                            Ok(DeServidor::Estado { estado }) => {
                                let mut e = *estado;
                                e.medios = medios.clone();
                                e.fichas = fichas.clone();
                                self.con_subidas(&mut e);
                                *self.estado.lock().expect("estado") = e;
                                (self.al_cambiar)();
                            }
                            Ok(DeServidor::Miniatura { clip, png }) => {
                                let m = match png.and_then(|p| base64::engine::general_purpose::STANDARD.decode(p).ok()) {
                                    Some(b) => Miniatura::Lista(Arc::new(b)),
                                    None => Miniatura::NoDisponible,
                                };
                                self.miniaturas.lock().expect("miniaturas").insert(clip.to_uppercase(), m);
                                (self.al_cambiar)();
                            }
                            Ok(DeServidor::SubirDesde { id, desde }) => self.subidor.desde(id, desde),
                            Ok(DeServidor::Reporte { id, reporte }) => {
                                if let Some(r) = self.reportes.lock().expect("reportes").remove(&id) {
                                    *r.lock().expect("ranura") = Some(reporte);
                                }
                                (self.al_cambiar)();
                            }
                            Ok(DeServidor::Subida { id, progreso, error, terminada }) => {
                                self.subidor.avance(id, progreso, error, terminada);
                                let mut e = self.estado.lock().expect("estado");
                                self.con_subidas(&mut e);
                                drop(e);
                                (self.al_cambiar)();
                            }
                            Err(e) => tracing::debug!("mensaje no reconocido: {e}"),
                        }
                    }
                    Some(t) = rx_texto.recv() => {
                        if tx.send(Message::text(t)).await.is_err() {
                            break "Se cortó la conexión con la PC principal".to_string();
                        }
                    }
                    Some(m) = rx_trozo.recv() => {
                        if tx.send(m).await.is_err() {
                            break "Se cortó la conexión durante una subida".to_string();
                        }
                    }
                }
            };
            // Corte: las subidas en curso se detienen y continuarán al reconectar.
            self.subidor.conectado.store(false, Ordering::SeqCst);
            self.subidor.generacion.fetch_add(1, Ordering::SeqCst);
            self.subidor.esperas.lock().expect("esperas").clear();
            for (_, r) in self.reportes.lock().expect("reportes").drain() {
                *r.lock().expect("ranura") = Some(Err("Se cortó la conexión con la PC principal".into()));
            }
            while rx_trozo.try_recv().is_ok() {}
            self.estado.lock().expect("estado").conectado = false;
            self.poner(Conexion::Caida(motivo));
            tokio::time::sleep(Duration::from_secs(2)).await;
            while rx_trozo.try_recv().is_ok() {}
        }
    }
}

#[cfg(test)]
pub(crate) mod pruebas {
    use super::*;
    use crate::controlador::{self, Rol};
    use tokio::io::{AsyncBufReadExt, BufReader};

    /// CasparCG mínimo para las pruebas.
    pub(crate) async fn caspar_falso(medios: PathBuf) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let puerto = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (f, _) = l.accept().await.unwrap();
                let medios = medios.clone();
                tokio::spawn(async move {
                    let (r, mut w) = f.into_split();
                    let mut r = BufReader::new(r);
                    let mut linea = String::new();
                    while r.read_line(&mut linea).await.unwrap_or(0) > 0 {
                        let cmd = linea.trim().to_string();
                        linea.clear();
                        let resp = if cmd == "VERSION" {
                            "201 VERSION OK\r\n2.3.3\r\n".to_string()
                        } else if cmd == "INFO PATHS" {
                            format!("201 INFO OK\r\n<paths><media-path>{}</media-path></paths>\r\n", medios.display())
                        } else if cmd == "CLS" {
                            "200 CLS OK\r\n\"SPOT\" MOVIE 1 20260101000000 300 1/30\r\n\r\n".to_string()
                        } else {
                            format!("202 {} OK\r\n", cmd.split_whitespace().next().unwrap_or(""))
                        };
                        if w.write_all(resp.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        puerto
    }

    /// Opciones del núcleo para las pruebas.
    pub(crate) fn opciones(base: &std::path::Path) -> controlador::Opciones {
        controlador::Opciones {
            canal: 1,
            capa: 20,
            zona: time::UtcOffset::UTC,
            osc_puerto: 0,
            carpeta_medios: None,
            carpeta_datos: base.join("datos"),
            orden: Default::default(),
            externos: Default::default(),
            ffmpeg: None,
            estandar: Default::default(),
            tope_corte: None,
            relleno: Vec::new(),
            relleno_clientes: false,
            recuperar_min: 0,
            carpeta_respaldos: base.join("respaldos"),
            respaldos_a_guardar: 30,
            respaldar_tambien: Vec::new(),
            transicion: Default::default(),
            iptv: None,
        }
    }

    #[track_caller]
    fn esperar<F: Fn() -> bool>(f: F) -> impl std::future::Future<Output = ()> {
        let donde = std::panic::Location::caller();
        async move {
            for _ in 0..100 {
                if f() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("la condición de la línea {} no se cumplió a tiempo", donde.line());
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn principal_y_remota() {
        let base = std::env::temp_dir().join(format!("relevo-remoto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let medios = base.join("media");
        std::fs::create_dir_all(&medios).unwrap();

        // PC principal.
        let pc = caspar_falso(medios.clone()).await;
        let rt = tokio::runtime::Handle::current();
        let principal = controlador::iniciar(
            &rt,
            amcp::Cliente::new("127.0.0.1", pc),
            controlador::Opciones {
                canal: 1,
                capa: 20,
                zona: time::UtcOffset::UTC,
                osc_puerto: 0,
                carpeta_medios: None,
                carpeta_datos: base.join("datos"),
                orden: Default::default(),
                externos: Default::default(),
                ffmpeg: None,
                estandar: Default::default(),
                tope_corte: None,
                relleno: Vec::new(),
                relleno_clientes: false,
                recuperar_min: 0,
                carpeta_respaldos: base.join("respaldos"),
                respaldos_a_guardar: 30,
                respaldar_tambien: Vec::new(),
                transicion: Default::default(),
                iptv: None,
            },
            Arc::new(|| {}),
        );
        let usuarios = Arc::new(Usuarios::cargar(&base.join("datos")).unwrap());
        usuarios.guardar_usuario("juan", "1234", Rol::Operador).unwrap();
        let usuarios_ref = usuarios.clone();
        let puerto = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        servir(puerto, principal.clone(), usuarios).await.unwrap();

        // Contraseña equivocada: rechazada.
        let (_, con_mal) = conectar(
            &rt,
            Ajustes { servidor: "127.0.0.1".into(), puerto, usuario: "juan".into(), clave: "x".into() },
            Arc::new(|| {}),
        );
        esperar(|| matches!(*con_mal.lock().unwrap(), Conexion::Rechazada(_))).await;

        // PC remota.
        let (remota, con) = conectar(
            &rt,
            Ajustes { servidor: "127.0.0.1".into(), puerto, usuario: "juan".into(), clave: "1234".into() },
            Arc::new(|| {}),
        );
        esperar(|| matches!(*con.lock().unwrap(), Conexion::Conectada { .. })).await;
        esperar(|| remota.ver().conectado && !remota.ver().medios.is_empty()).await;
        esperar(|| principal.ver().sesiones.len() == 1).await;

        // La principal empieza a operar: toma el mando.
        principal.enviar(Orden::Agregar {
            clip: "SPOT".into(),
            duracion: Duration::from_secs(10),
            destino: Destino::Final,
        });
        esperar(|| principal.ver().mando.as_ref().is_some_and(|(id, _)| *id == 0)).await;
        esperar(|| remota.ver().lista.len() == 1).await;
        assert!(!remota.tengo_mando(&remota.ver()));

        // La remota no puede operar sin el mando…
        remota.enviar(Orden::LimpiarTanda);
        esperar(|| remota.ver().aviso.as_ref().is_some_and(|a| a.para == Some(remota.sesion()))).await;
        assert_eq!(principal.ver().lista.len(), 1);
        // …pero Volver a TV siempre funciona.
        principal.enviar(Orden::PlayPausa(None));
        esperar(|| remota.ver().fase == controlador::FaseVista::AlAire).await;
        remota.enviar(Orden::VolverATv);
        esperar(|| principal.ver().fase == controlador::FaseVista::Tv).await;

        // Un operador no puede quitar el mando a la fuerza; cuando la principal lo suelta, sí lo toma.
        remota.enviar(Orden::TomarMando { forzar: true });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(principal.ver().mando.as_ref().map(|m| m.0), Some(0));
        principal.enviar(Orden::SoltarMando);
        esperar(|| remota.ver().mando.is_none()).await;
        remota.enviar(Orden::TomarMando { forzar: false });
        esperar(|| remota.tengo_mando(&remota.ver()) && remota.ver().mando.is_some()).await;

        // Subir un video desde la remota.
        let video = base.join("en-la-remota").join("Nuevo cliente.mp4");
        std::fs::create_dir_all(video.parent().unwrap()).unwrap();
        std::fs::write(&video, vec![5u8; 3 * TROZO + 123]).unwrap();
        remota.agregar_archivo(video, Some(Duration::from_secs(15)), Destino::Final);
        let subido = medios.join("IMPORTADOS").join("Nuevo cliente.mp4");
        esperar(|| subido.exists()).await;
        assert_eq!(std::fs::metadata(&subido).unwrap().len(), 3 * TROZO as u64 + 123);
        esperar(|| principal.ver().lista.len() == 2).await;

        // Una subida cortada continúa desde donde quedó: se simula que ya llegó la
        // primera parte (con bytes distintos, para comprobar que no se vuelve a enviar).
        let video2 = base.join("en-la-remota").join("Grande.mp4");
        std::fs::write(&video2, vec![7u8; 2 * TROZO + 10]).unwrap();
        let fecha = crate::biblioteca::segundos(std::fs::metadata(&video2).unwrap().modified().ok());
        let parte = base
            .join("datos")
            .join("subidas")
            .join(format!("{}.parte", hash(&format!("Grande.mp4|{fecha}|{}", 2 * TROZO + 10))));
        std::fs::create_dir_all(parte.parent().unwrap()).unwrap();
        std::fs::write(&parte, vec![9u8; TROZO]).unwrap();
        remota.agregar_archivo(video2, None, Destino::Final);
        let subido2 = medios.join("IMPORTADOS").join("Grande.mp4");
        esperar(|| subido2.exists()).await;
        let datos = std::fs::read(&subido2).unwrap();
        assert_eq!(datos.len(), 2 * TROZO + 10);
        assert_eq!(datos[0], 9, "la primera parte no se volvió a enviar");
        assert_eq!(datos[TROZO], 7);
        assert!(!parte.exists());

        // Reemplazar el video de un spot desde otra PC: hace falta rol Editor.
        let spot = medios.join("SPOT.mp4");
        std::fs::write(&spot, b"version vieja").unwrap();
        principal.enviar(Orden::RecargarMedios);
        let nuevo = base.join("en-la-remota").join("Spot nuevo.mp4");
        std::fs::write(&nuevo, b"version nueva").unwrap();
        remota.reemplazar_con("SPOT".into(), nuevo.clone());
        esperar(|| remota.ver().importaciones.is_empty()).await;
        assert_eq!(std::fs::read(&spot).unwrap(), b"version vieja", "un operador no puede reemplazar");

        usuarios_ref.guardar_usuario("ana", "abcd", Rol::Editor).unwrap();
        let (editora, con2) = conectar(
            &rt,
            Ajustes { servidor: "127.0.0.1".into(), puerto, usuario: "ana".into(), clave: "abcd".into() },
            Arc::new(|| {}),
        );
        esperar(|| matches!(*con2.lock().unwrap(), Conexion::Conectada { .. })).await;
        editora.reemplazar_con("SPOT".into(), nuevo);
        // SPOT está en la tanda: el cambio espera a que salga de ella.
        esperar(|| !principal.ver().biblioteca.trabajos.is_empty()).await;
        esperar(|| {
            principal
                .ver()
                .biblioteca
                .trabajos
                .first()
                .is_some_and(|t| t.estado == crate::biblioteca::EstadoTrabajo::Esperando)
        })
        .await;
        assert_eq!(std::fs::read(&spot).unwrap(), b"version vieja");
        remota.enviar(Orden::LimpiarTanda);
        esperar(|| std::fs::read(&spot).unwrap_or_default() == b"version nueva").await;
        esperar(|| principal.ver().biblioteca.papelera.len() == 1).await;
        let en_papelera = principal.ver().biblioteca.papelera[0].clone();
        assert_eq!(en_papelera.ruta_original, "SPOT.mp4");

        // Y se puede restaurar la versión anterior (sin pisar la actual).
        editora.enviar(Orden::Restaurar(en_papelera.id));
        esperar(|| medios.join("SPOT (restaurado).mp4").exists()).await;

        // Los reportes se calculan en la principal aunque se pidan desde la remota.
        let hoy = time::OffsetDateTime::now_utc().date();
        let r = editora.reporte(hoy, hoy, hoy, time::Time::MIDNIGHT);
        esperar(|| r.lock().unwrap().is_some()).await;
        let rep = r.lock().unwrap().clone().unwrap().unwrap();
        assert!(!rep.salidas.is_empty(), "el SPOT que salió al aire quedó en el registro");
        assert!(rep.salidas.iter().all(|s| !s.salida.completo), "se cortó con Volver a TV");

        // Al desconectarse la remota, el mando queda libre.
        drop(remota);
    }
}
