use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub casparcg: CasparCg,
    pub pauta: PautaCfg,
    pub remoto: RemotoCfg,
    pub archivos: ArchivosCfg,
    pub vista_previa: VistaPreviaCfg,
    pub biblioteca: BibliotecaCfg,
    pub respaldo: RespaldoCfg,
    pub api: ApiCfg,
    pub interfaz: InterfazCfg,
    pub transiciones: TransicionesCfg,
    pub atajos: AtajosCfg,
    pub deteccion: DeteccionCfg,
    pub iptv: IptvCfg,
    pub preview: PreviewCfg,
    pub emision: EmisionCfg,
    pub actualizacion: ActualizacionCfg,
}

/// Actualización de Relevo desde GitHub (Releases).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct ActualizacionCfg {
    /// Revisar al abrir (y cada 6 horas) si hay una versión nueva. Nunca se instala sola.
    pub activa: bool,
    /// Repositorio de GitHub: "usuario/nombre".
    pub repo: String,
    /// Solo si el repositorio es privado: clave de GitHub de solo lectura.
    pub token: String,
}

impl Default for ActualizacionCfg {
    fn default() -> Self {
        ActualizacionCfg { activa: true, repo: "Everlck/relevo".into(), token: String::new() }
    }
}

/// Monitor PREVIEW (arriba a la derecha): una señal que se mira dentro de Relevo,
/// sin sonido y sin salir al aire.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct PreviewCfg {
    /// "ninguna", "capturadora", "web", "iptv", "pantalla" o "archivo".
    pub fuente: String,
    pub origen: String,
    /// Con "pantalla": número de monitor (0 = todo el escritorio).
    pub monitor: u32,
}

impl Default for PreviewCfg {
    fn default() -> Self {
        PreviewCfg { fuente: "ninguna".into(), origen: String::new(), monitor: 0 }
    }
}

/// Modo de trabajo.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct EmisionCfg {
    /// "publicidad": CasparCG solo pone la publicidad (el canal va por la mezcladora).
    /// "transmision": CasparCG muestra el canal por IPTV y la publicidad entra encima.
    pub modo: String,
}

impl Default for EmisionCfg {
    fn default() -> Self {
        EmisionCfg { modo: "publicidad".into() }
    }
}

/// Configuración de ejemplo (la que se sube a GitHub). Si falta `config.toml`, se
/// crea a partir de ella. El `config.toml` de cada PC no se sube: lleva sus claves y
/// direcciones.
pub const EJEMPLO: &str = include_str!("../../../config.ejemplo.toml");

impl Config {
    /// ¿Se puede usar el modo Transmisión? (IPTV permitida y con dirección.)
    pub fn transmision_posible(&self) -> bool {
        self.iptv.activa && nucleo::iptv::es_direccion(&self.iptv.direccion)
    }

    /// ¿Está en modo Transmisión?
    pub fn en_transmision(&self) -> bool {
        self.emision.modo == "transmision" && self.transmision_posible()
    }

    /// Lo que se le pasa al núcleo: el canal por IPTV solo en modo Transmisión.
    pub fn iptv_para_nucleo(&self) -> Option<nucleo::Iptv> {
        self.en_transmision()
            .then(|| nucleo::Iptv { direccion: self.iptv.direccion.trim().to_string(), capa: self.iptv.capa })
            .filter(|i| i.capa < self.casparcg.capa_publicidad)
    }
}

/// Señal de TV por IPTV: CasparCG reproduce el canal en una capa debajo de la
/// publicidad (en vez de recibirlo por la mezcladora).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct IptvCfg {
    pub activa: bool,
    /// Dirección del canal (http(s)://….m3u8, udp://@239.x.x.x:1234, rtp://…).
    pub direccion: String,
    /// Lista de canales .m3u (dirección o archivo), para elegir el canal por nombre.
    pub lista: String,
    /// Capa de CasparCG para el canal (debajo de la de publicidad).
    pub capa: u16,
}

impl Default for IptvCfg {
    fn default() -> Self {
        IptvCfg { activa: false, direccion: String::new(), lista: String::new(), capa: 10 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct InterfazCfg {
    /// Animaciones de transición (apagarlas en PCs lentas).
    pub animaciones: bool,
    /// Mostrar el botón «Volver a TV» (y atender la tecla Esc).
    pub boton_volver_a_tv: bool,
    /// Aviso visual (sin sonido) cuando faltan estos segundos para volver a la señal
    /// de TV. 0 = sin aviso.
    pub aviso_fin_seg: u32,
    /// Tamaño de la interfaz en %: 0 = automático (se achica si la pantalla es chica o
    /// está al 125 %); 70 a 130 = fijo.
    pub escala: u32,
    /// Mostrar las horas en formato de 12 h (`8:30 pm`) en vez de 24 h (`20:30`).
    pub hora_12: bool,
}

impl Default for InterfazCfg {
    fn default() -> Self {
        InterfazCfg { animaciones: true, boton_volver_a_tv: false, aviso_fin_seg: 10, escala: 0, hora_12: false }
    }
}

/// Atajos de teclado ("Space", "Ctrl+ArrowRight", "F5"…; vacío = sin atajo).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AtajosCfg {
    pub play_pausa: String,
    pub siguiente: String,
    pub anterior: String,
    pub detener: String,
    pub anadir: String,
    pub quitar: String,
    pub subir: String,
    pub bajar: String,
    pub emision: String,
    pub biblioteca: String,
    pub reportes: String,
    pub compacta: String,
    pub casparcg: String,
    pub publicidades: String,
}

impl Default for AtajosCfg {
    fn default() -> Self {
        AtajosCfg {
            play_pausa: "Space".into(),
            siguiente: "Ctrl+ArrowRight".into(),
            anterior: "Ctrl+ArrowLeft".into(),
            detener: "Ctrl+Backspace".into(),
            anadir: "Enter".into(),
            quitar: "Delete".into(),
            subir: "Ctrl+ArrowUp".into(),
            bajar: "Ctrl+ArrowDown".into(),
            emision: "F1".into(),
            biblioteca: "F2".into(),
            reportes: "F3".into(),
            compacta: "F11".into(),
            casparcg: "F4".into(),
            publicidades: "F5".into(),
        }
    }
}

impl AtajosCfg {
    /// (clave en config.toml, nombre para el operador, valor).
    pub fn lista(&mut self) -> Vec<(&'static str, &'static str, &mut String)> {
        vec![
            ("play_pausa", "Reproducir / pausar la tanda", &mut self.play_pausa),
            ("siguiente", "Spot siguiente", &mut self.siguiente),
            ("anterior", "Spot anterior", &mut self.anterior),
            ("detener", "Detener la tanda", &mut self.detener),
            ("anadir", "Añadir el video elegido a la tanda", &mut self.anadir),
            ("quitar", "Quitar de la tanda el spot elegido", &mut self.quitar),
            ("subir", "Subir el spot elegido", &mut self.subir),
            ("bajar", "Bajar el spot elegido", &mut self.bajar),
            ("emision", "Ir a Emisión", &mut self.emision),
            ("biblioteca", "Ir a Biblioteca", &mut self.biblioteca),
            ("reportes", "Ir a Reportes", &mut self.reportes),
            ("casparcg", "Ir a CasparCG", &mut self.casparcg),
            ("publicidades", "Ir a Publicidades", &mut self.publicidades),
            ("compacta", "Vista compacta / completa", &mut self.compacta),
        ]
    }
}

/// Detección del corte del canal: se vigila su logo en una señal (capturadora,
/// dirección de video o la pantalla); cuando desaparece, el canal fue a comerciales.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct DeteccionCfg {
    pub activa: bool,
    /// "capturadora", "url", "pantalla" o "archivo" (para pruebas).
    pub fuente: String,
    /// Nombre de la capturadora, dirección de la página o del video, o ruta del archivo.
    pub origen: String,
    /// Con fuente "pantalla": número de monitor (0 = todo el escritorio).
    pub monitor: u32,
    /// Zona del logo, en partes de la imagen (0–1): izquierda, arriba, ancho, alto.
    pub x: f64,
    pub y: f64,
    pub ancho: f64,
    pub alto: f64,
    /// Parecido mínimo con el logo aprendido (0–1) para decir "hay logo".
    pub umbral: f64,
    /// Segundos seguidos sin logo para dar por empezado el corte.
    pub segundos: f64,
    /// "avisar" (solo aviso en pantalla) o "iniciar" (lanza la tanda sola).
    pub accion: String,
    /// Minutos mínimos entre un corte detectado y el siguiente.
    pub espera_min: f64,
    /// Cuando vuelve el logo con publicidad al aire: "avisar" (cartel verde) o
    /// "volver" (vuelve al canal; solo en modo Transmisión).
    pub al_volver: String,
}

impl Default for DeteccionCfg {
    fn default() -> Self {
        DeteccionCfg {
            activa: false,
            fuente: "capturadora".into(),
            origen: String::new(),
            monitor: 0,
            x: 0.80,
            y: 0.04,
            ancho: 0.17,
            alto: 0.17,
            umbral: 0.35,
            segundos: 2.0,
            accion: "avisar".into(),
            espera_min: 3.0,
            al_volver: "avisar".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct TransicionesCfg {
    /// "corte", "fundido", "barrido", "empuje" o "deslizar".
    pub tipo: String,
    /// Duración en segundos.
    pub segundos: f32,
}

impl Default for TransicionesCfg {
    fn default() -> Self {
        TransicionesCfg { tipo: "fundido".into(), segundos: 0.5 }
    }
}

impl TransicionesCfg {
    pub fn transicion(&self) -> nucleo::Transicion {
        nucleo::Transicion {
            tipo: nucleo::TipoTransicion::desde_texto(&self.tipo).unwrap_or_default(),
            segundos: self.segundos.clamp(0.0, 5.0),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RemotoCfg {
    /// "principal" = esta PC controla CasparCG; "remoto" = se conecta a la PC principal.
    pub modo: String,
    /// En la principal: aceptar conexiones de otras PCs.
    pub servir: bool,
    pub puerto: u16,
    /// En una remota: IP de la PC principal (se puede escribir al conectar).
    pub servidor: String,
    /// En una remota: último usuario usado.
    pub usuario: String,
}

impl Default for RemotoCfg {
    fn default() -> Self {
        RemotoCfg {
            modo: "principal".into(),
            servir: true,
            puerto: 7800,
            servidor: String::new(),
            usuario: String::new(),
        }
    }
}

impl RemotoCfg {
    pub fn es_remoto(&self) -> bool {
        let m = self.modo.trim();
        m.eq_ignore_ascii_case("remoto") || m.eq_ignore_ascii_case("remota")
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ArchivosCfg {
    /// Videos fuera de la carpeta de CasparCG: "directo" (sin copiar, si CasparCG lo acepta) o "copiar".
    pub externos: nucleo::ModoExternos,
    /// Ruta de ffmpeg para la vista previa. Vacía = buscarlo solo.
    pub ffmpeg: String,
    /// Revisar una vez al día si hay una versión nueva de ffmpeg (si falta, se descarga siempre).
    pub actualizar_ffmpeg: bool,
}

impl Default for ArchivosCfg {
    fn default() -> Self {
        ArchivosCfg { externos: nucleo::ModoExternos::Directo, ffmpeg: String::new(), actualizar_ffmpeg: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RespaldoCfg {
    /// Carpeta de los respaldos diarios. Vacía = `respaldos` junto a config.toml.
    /// Mejor en otro disco o en una memoria USB.
    pub carpeta: String,
    /// Cuántos respaldos guardar (los más viejos se borran).
    pub guardar: usize,
}

impl Default for RespaldoCfg {
    fn default() -> Self {
        RespaldoCfg { carpeta: String::new(), guardar: 30 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ApiCfg {
    /// Activar la API (para botoneras tipo Stream Deck u otros programas).
    pub activa: bool,
    pub puerto: u16,
    /// Clave que deben enviar las peticiones (`?clave=` o cabecera `X-Clave`). Obligatoria.
    pub clave: String,
}

impl Default for ApiCfg {
    fn default() -> Self {
        ApiCfg { activa: false, puerto: 7801, clave: String::new() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BibliotecaCfg {
    /// Resolución del canal, por ejemplo "1920x1080".
    pub resolucion: String,
    /// Cuadros por segundo del canal (29.97, 25, 59.94…).
    pub fps: f64,
    /// Volumen objetivo en LUFS (-23 EBU R128; -24 ATSC).
    pub lufs: f32,
    /// Diferencia de volumen aceptada, en dB.
    pub tolerancia: f32,
}

impl Default for BibliotecaCfg {
    fn default() -> Self {
        let e = nucleo::biblioteca::Estandar::default();
        BibliotecaCfg {
            resolucion: format!("{}x{}", e.ancho, e.alto),
            fps: e.fps,
            lufs: e.lufs,
            tolerancia: e.tolerancia,
        }
    }
}

impl BibliotecaCfg {
    pub fn estandar(&self) -> nucleo::biblioteca::Estandar {
        let d = nucleo::biblioteca::Estandar::default();
        let (ancho, alto) = self
            .resolucion
            .trim()
            .split_once(['x', 'X', '×'])
            .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)))
            .unwrap_or((d.ancho, d.alto));
        nucleo::biblioteca::Estandar {
            ancho,
            alto,
            fps: self.fps,
            lufs: self.lufs,
            tolerancia: self.tolerancia.max(0.5),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VistaPreviaCfg {
    /// Con sonido al abrir la vista previa.
    pub sonido: bool,
    /// Salida de sonido (vacía = la predeterminada de Windows). Se elige con ⚙ en el reproductor.
    pub salida_audio: String,
    /// En la PC principal, callar la vista previa mientras hay publicidad al aire.
    pub silenciar_al_aire: bool,
}

impl Default for VistaPreviaCfg {
    fn default() -> Self {
        VistaPreviaCfg { sonido: true, salida_audio: String::new(), silenciar_al_aire: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PautaCfg {
    /// Orden de los spots normales en la tanda automática: "hora" o "cercania".
    pub orden: nucleo::pauta::Orden,
    /// Tolerancia que trae una pauta nueva, en minutos.
    pub tolerancia_por_defecto: u32,
    /// Duración del corte en AUTO, "m:ss" ("0:00" = sin tope). La que elijas en la app se recuerda.
    pub duracion_corte: String,
    /// Carpetas (o clips) de CasparCG con piezas para rellenar el corte.
    pub relleno: Vec<String>,
    /// Rellenar primero con spots de otros clientes (el que menos salió hoy).
    pub relleno_clientes: bool,
    /// Minutos, después de cerrarse su ventana, en que un pase que no salió (o se cortó)
    /// entra solo en el próximo corte. 0 = no se recupera.
    pub recuperar_min: u32,
}

impl PautaCfg {
    /// `duracion_corte` en segundos (None = sin tope).
    pub fn tope(&self) -> Option<u32> {
        let t = self.duracion_corte.trim();
        let s = match t.split_once(':') {
            Some((m, s)) => m.trim().parse::<u32>().ok()? * 60 + s.trim().parse::<u32>().ok()?,
            None => t.parse().ok()?,
        };
        (s > 0).then_some(s)
    }
}

impl Default for PautaCfg {
    fn default() -> Self {
        PautaCfg {
            orden: nucleo::pauta::Orden::Hora,
            tolerancia_por_defecto: 15,
            duracion_corte: "0:00".into(),
            relleno: vec!["IDS".into(), "PROMOS".into()],
            relleno_clientes: true,
            recuperar_min: 180,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CasparCg {
    pub host: String,
    pub puerto: u16,
    pub canal: u16,
    pub capa_publicidad: u16,
    /// Carpeta de medios de CasparCG. Vacía = preguntarle a CasparCG (INFO PATHS).
    pub carpeta_medios: String,
    /// Puerto UDP donde CasparCG envía el OSC (0 = no usar OSC).
    pub osc_puerto: u16,
    /// Carpeta donde está casparcg.exe. Vacía = buscarla sola.
    pub carpeta_programa: String,
    /// Abrir CasparCG y su scanner al iniciar la app, y configurar el OSC.
    pub iniciar_automaticamente: bool,
    /// Salida de pantalla de CasparCG: "auto" (pantalla completa en el segundo monitor,
    /// si hay), "ventana", "no" (no tocar casparcg.config) o el número de monitor.
    pub pantalla: String,
    /// Al abrir CasparCG y su scanner desde Relevo: "visibles" (cada uno en su ventana
    /// de consola) u "ocultas" (sin ventanas; se ven en la pestaña CasparCG).
    pub consolas: String,
    /// Si la ventana de video de CasparCG se minimiza (Windows + D), volver a mostrarla.
    pub salida_siempre_visible: bool,
    /// Al cerrar Relevo, detener la publicidad que esté saliendo en CasparCG.
    pub detener_al_cerrar: bool,
    /// Al cerrar Relevo, cerrar también CasparCG y su scanner (solo si los abrió
    /// Relevo): "si", "preguntar" o "no".
    pub cerrar_al_salir: String,
}

impl CasparCg {
    /// Cómo abrir CasparCG y el scanner. `datos` es la carpeta de datos de esta PC.
    pub fn como_abrir(&self, datos: &std::path::Path) -> crate::arranque::ComoAbrir {
        crate::arranque::ComoAbrir {
            ocultas: self.consolas.trim().eq_ignore_ascii_case("ocultas"),
            carpeta_consolas: datos.join("consolas"),
        }
    }
}

impl Default for CasparCg {
    fn default() -> Self {
        CasparCg {
            host: "127.0.0.1".into(),
            puerto: 5250,
            canal: 1,
            capa_publicidad: 20,
            carpeta_medios: String::new(),
            osc_puerto: 6250,
            carpeta_programa: String::new(),
            iniciar_automaticamente: true,
            pantalla: "auto".into(),
            consolas: "visibles".into(),
            salida_siempre_visible: true,
            detener_al_cerrar: true,
            cerrar_al_salir: "si".into(),
        }
    }
}

impl Config {
    /// Lee el archivo de configuración. Si no existe, usa los valores por defecto.
    pub fn cargar(ruta: &Path) -> anyhow::Result<Config> {
        if !ruta.exists() {
            // Primera vez (PC nueva o recién descargado de GitHub): se crea a partir del
            // ejemplo, con todos sus comentarios, para poder editarlo después.
            if let Err(e) = std::fs::write(ruta, EJEMPLO) {
                tracing::warn!("no existe {} y no se pudo crear ({e}); se usan valores por defecto", ruta.display());
                return Ok(Config::default());
            }
            tracing::info!("se creó {} a partir del ejemplo", ruta.display());
        }
        let texto = std::fs::read_to_string(ruta)?;
        toml::from_str(&texto).map_err(|e| anyhow::anyhow!("error en {}: {e}", ruta.display()))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn config_parcial_completa_con_defectos() {
        let c: Config = toml::from_str("[casparcg]\nhost = \"192.168.1.20\"\n").unwrap();
        assert_eq!(c.casparcg.host, "192.168.1.20");
        assert_eq!(c.casparcg.puerto, 5250);
        assert_eq!(c.casparcg.osc_puerto, 6250);
        assert!(c.casparcg.carpeta_medios.is_empty());
    }

    #[test]
    fn config_del_repositorio_es_valida() {
        let c: Config = toml::from_str(EJEMPLO).unwrap();
        assert_eq!(c.casparcg.capa_publicidad, 20);
        assert!(c.casparcg.iniciar_automaticamente);
        assert_eq!(c.pauta.orden, nucleo::pauta::Orden::Hora);
        assert_eq!(c.pauta.tolerancia_por_defecto, 15);
        assert!(!c.remoto.es_remoto());
        assert_eq!(c.remoto.puerto, 7800);
        assert_eq!(c.archivos.externos, nucleo::ModoExternos::Directo);
    }
}
