//! Ventana principal: barra de "al aire" arriba y tres columnas
//! (Archivos de la PC · Tanda · Pauta de hoy / Registro).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Align, Align2, Color32, CornerRadius, FontId, Layout, RichText, Sense, Stroke, StrokeKind, Vec2};
use nucleo::archivos;
use nucleo::controlador::{FaseVista, ModoTanda, PaseVista, TipoEvento};
use nucleo::pauta::EstadoPase;
use nucleo::remoto::Conexion;
use nucleo::usuarios::Usuarios;
use nucleo::{Bucle, Controlador, Destino, Instantanea, Miniatura, Orden};

use crate::ajustes::{self, VentanaAjustes};
use crate::arranque;
use crate::config::Config;
use crate::dialogo::{self, Dialogo, Tono};
use crate::diseno::{self, Arrastre, Diseno};
use crate::explorador::{Busqueda, Entrada, Explorador, Listado};
use crate::navegacion::{AccionNav, Navegacion};
use crate::tandas::Tandas;
use crate::tema::{self, *};
use crate::usuarios_ui::VentanaUsuarios;
use crate::vista_previa::{AccionVista, VistaPrevia};
use egui_extras::{Size, StripBuilder};

#[path = "al_aire_ui.rs"]
mod al_aire_ui;
#[path = "biblioteca_ui.rs"]
mod biblioteca_ui;
#[path = "deteccion_ui.rs"]
mod deteccion_ui;
#[path = "senal_ui.rs"]
mod senal_ui;
use biblioteca_ui::{Confirmar, Filtro};

#[path = "compacta_ui.rs"]
mod compacta_ui;

#[path = "reportes_ui.rs"]
mod reportes_ui;
use reportes_ui::EstadoReportes;
#[path = "consolas_ui.rs"]
mod consolas_ui;
#[path = "publicidades_ui.rs"]
mod publicidades_ui;
use consolas_ui::EstadoCaspar;
use publicidades_ui::EstadoPublicidades;

/// Pantalla principal que se ve debajo de "Al aire".
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pantalla {
    Emision,
    Biblioteca,
    Reportes,
    CasparCG,
    Publicidades,
}

/// Cuánto se ve un aviso (los errores, un poco más).
const DURACION_AVISO: Duration = Duration::from_millis(2500);
const DURACION_ERROR: Duration = Duration::from_secs(5);

/// El núcleo cambió algo desde la última vez que la ventana copió su estado.
pub static CAMBIO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Lo que cambia entre la PC principal y una remota.
pub struct Extras {
    /// Puesta en marcha automática (solo en la principal).
    pub arranque: Option<arranque::Compartido>,
    /// Usuarios del modo remoto (solo en la principal).
    pub usuarios: Option<Arc<Usuarios>>,
    /// Conexión con la principal (solo en una remota).
    pub conexion: Option<Arc<Mutex<Conexion>>>,
    /// Puerto del servicio de red, si está abierto.
    pub puerto_red: Option<u16>,
    /// Puerto de la API, si está abierta.
    pub puerto_api: Option<u16>,
    pub problema_red: Option<String>,
    pub ffmpeg: Option<PathBuf>,
    /// Carpeta de datos de esta PC (preferencias locales).
    pub carpeta_local: PathBuf,
    /// config.toml (lo edita ⚙ Ajustes).
    pub ruta_config: PathBuf,
}

pub struct Ventana {
    ctl: Controlador,
    cfg: Config,
    zona: time::UtcOffset,
    explorador: Explorador,
    tandas: Tandas,
    tanda_sel: Option<u64>,
    /// Varios spots elegidos (Ctrl+clic, Mayús+clic). `tanda_sel` es el último clic.
    tanda_multi: Vec<u64>,
    /// Al guardar: solo estos spots (los elegidos), o toda la tanda.
    guardar_ids: Option<Vec<u64>>,
    /// Marco para elegir varios spots arrastrando el mouse.
    marco_tanda: Option<MarcoTanda>,
    /// Borde de arriba del contenido de la lista (cambia al desplazarse).
    tanda_arriba: f32,
    /// Dónde quedó cada fila de la tanda en el último cuadro.
    tanda_rects: HashMap<u64, egui::Rect>,
    /// Ya se detuvo la publicidad al cerrar (no repetirlo).
    detenido_al_cerrar: bool,
    /// Cómo se cierra Relevo: `None` = aún no; `Some(true)` = el usuario lo cerró
    /// (se cierra CasparCG si toca); `Some(false)` = lo cierra Relevo solo (reinicio o
    /// actualización: CasparCG se deja).
    cierre: Option<bool>,
    /// Se pregunta si cerrar también CasparCG.
    preguntar_cierre: bool,
    /// Varios videos elegidos en Archivos (Ctrl+clic, Mayús+clic, marco).
    archivos_multi: Vec<PathBuf>,
    /// Videos de la lista de Archivos, en el orden en que se ven (con su duración).
    archivos_visibles: Vec<(PathBuf, Option<Duration>)>,
    /// Marco para elegir videos con el mouse.
    marco_archivos: Option<MarcoArchivos>,
    archivos_arriba: f32,
    /// Carpeta (y si es búsqueda) de la lista de Archivos del último cuadro.
    archivos_en: (Option<PathBuf>, bool),
    /// Cuándo se marcó la carpeta de la que se vino (la marca se desvanece).
    resalte_desde: f64,
    volumen: f32,
    aviso_visto: u64,
    /// Mensaje flotante: texto, desde cuándo y si es un error (rojo) o solo información.
    aviso: Option<(String, Instant, bool)>,
    /// Texturas de miniaturas ya decodificadas (None = CasparCG no tiene).
    texturas: HashMap<String, Option<egui::TextureHandle>>,
    /// Nombre de clip de cada archivo (si está en la carpeta de CasparCG).
    clips: HashMap<PathBuf, Option<String>>,
    clips_de_carpeta: Option<PathBuf>,
    /// Ventanita para guardar la tanda: `Some(nombre)` mientras está abierta.
    guardando: Option<String>,
    /// Pantalla Publicidades (la ficha que se está editando).
    publicidades: EstadoPublicidades,
    arranque: Option<arranque::Compartido>,
    arranque_visto: usize,
    usuarios: Option<Arc<Usuarios>>,
    conexion: Option<Arc<Mutex<Conexion>>>,
    puerto_red: Option<u16>,
    puerto_api: Option<u16>,
    ventana_usuarios: VentanaUsuarios,
    vista: VistaPrevia,
    monitor: crate::monitor::Monitor,
    /// Cuadro quieto del spot que sale al dar Play (en el PREVIEW).
    monitor_cue: crate::monitor::Monitor,
    /// ffmpeg: ruta, versión, descarga y actualización.
    ffmpeg: crate::ffmpeg::Compartido,
    /// Actualización de Relevo (versiones publicadas en GitHub).
    act: crate::actualizacion::Compartido,
    /// Se pidió actualizar: al terminar la descarga (y sin publicidad al aire) se instala.
    act_instalar: bool,
    /// Versión nueva que ya se avisó (para no repetir el aviso).
    act_avisada: Option<String>,
    ffmpeg_visto: u64,
    navegacion: Navegacion,
    /// Panel de carpetas (Acceso rápido / Este equipo) visible.
    arbol_visible: bool,
    /// Spot al aire y su archivo en esta PC (para el monitor).
    monitor_clip: Option<(String, Option<PathBuf>)>,
    /// En el explorador: mostrar la lista de CasparCG (de la PC principal) en vez de los archivos de esta PC.
    vista_caspar: bool,
    /// Carpeta abierta dentro de la lista de CasparCG ("" = raíz).
    carpeta_caspar: String,
    /// Última carpeta guardada (para volver a ella al abrir Relevo).
    ultima_carpeta: UltimaCarpeta,
    /// Escribiendo la ruta a mano (en vez de las migas de pan).
    editando_ruta: bool,
    /// Carpeta resaltada a la que ya se desplazó la lista.
    resalte_visto: Option<PathBuf>,
    busqueda_caspar: String,
    caspar_sel: Option<String>,
    /// La lista de la pauta ya se desplazó hasta el pase actual.
    pauta_desplazada: bool,
    /// Carpeta abierta mientras no se conocía la de CasparCG.
    carpeta_provisional: Option<PathBuf>,
    inicio: Instant,
    /// Problema al abrir el servicio de red (se muestra una vez).
    aviso_inicial: Option<String>,
    pantalla: Pantalla,
    bib_sel: Option<String>,
    bib_busqueda: String,
    bib_filtro: Filtro,
    confirmar: Option<Confirmar>,
    /// Último cambio de archivos de la biblioteca ya visto (para recargar miniaturas).
    cambios_vistos: u64,
    reportes: EstadoReportes,
    /// Pestaña CasparCG (procesos y consolas).
    caspar: EstadoCaspar,
    /// Escala de los tiempos y botones de Al aire (sigue al alto del monitor).
    escala_aire: f32,
    /// Vista compacta (botones grandes).
    compacta: bool,
    /// Tamaño de las secciones (se cambia arrastrando las separaciones).
    diseno: Diseno,
    /// Ventana ⚙ Ajustes.
    ajustes: VentanaAjustes,
    /// Carpeta de datos de esta PC.
    carpeta_datos_local: PathBuf,
    /// Ventana «Detección de corte».
    det: deteccion_ui::DetUi,
    /// Detector del corte del canal (si está activado).
    detector: Option<crate::deteccion::Detector>,
    /// Último corte detectado (para el aviso y la espera entre cortes).
    corte_detectado: Option<Instant>,
    /// Cuándo vio la detección que el canal volvió, con publicidad al aire.
    programa_detectado: Option<Instant>,
    /// Monitores PREVIEW y PROGRAMA (señales en vivo).
    sen: senal_ui::Senales,
    /// Miniaturas hechas con ffmpeg.
    miniaturas: crate::miniaturas::Miniaturas,
    /// La vista previa va en una ventanita flotante (si no, dentro del detalle).
    vista_flotante: bool,
    /// Copia del estado del núcleo (se renueva cuando cambia).
    foto: Option<Arc<Instantanea>>,
    foto_en: Instant,
    /// Dónde está el archivo de cada clip (para no buscar en el disco en cada cuadro).
    cache_rutas: std::cell::RefCell<HashMap<String, (Instant, Option<PathBuf>)>>,
    /// Vúmetros (con su caída suave).
    vu: Vec<al_aire_ui::CanalVu>,
    /// Para animar el cambio de pantalla: la última que se vio y desde cuándo.
    pantalla_vista: (Pantalla, bool),
    cambio_pantalla: f64,
    /// Cuándo apareció cada spot de la tanda (para resaltar los que llegan).
    tanda_llegada: HashMap<u64, f64>,
    tanda_cargada: bool,
    // Se mantiene vivo mientras dure la ventana.
    _rt: tokio::runtime::Runtime,
}

impl Ventana {
    pub fn new(
        ctl: Controlador,
        cfg: Config,
        zona: time::UtcOffset,
        rt: tokio::runtime::Runtime,
        tandas: Tandas,
        extras: Extras,
        ctx: egui::Context,
    ) -> Self {
        let Extras {
            arranque,
            usuarios,
            conexion,
            puerto_red,
            puerto_api,
            problema_red,
            ffmpeg,
            carpeta_local,
            ruta_config,
        } = extras;
        dialogo::poner_animaciones(&ctx, cfg.interfaz.animaciones);
        crate::tema::poner_hora_12(cfg.interfaz.hora_12);
        let ajustes = VentanaAjustes::new(&cfg, ruta_config);
        let es_remoto = ctl.es_remoto();
        let ruta_horarios = tandas
            .carpeta()
            .parent()
            .map(|c| c.join("datos").join("horarios.json"))
            .unwrap_or_else(|| "horarios.json".into());
        let publicidades = EstadoPublicidades::new(
            time::OffsetDateTime::now_utc().date(),
            cfg.pauta.tolerancia_por_defecto,
            crate::plantillas_horario::Plantillas::abrir(ruta_horarios),
        );
        let monitor = crate::monitor::Monitor::new(ffmpeg.clone());
        let monitor_cue = crate::monitor::Monitor::new(ffmpeg.clone());
        let estado_ffmpeg: crate::ffmpeg::Compartido = Arc::new(Mutex::new(crate::ffmpeg::Estado {
            version: ffmpeg.as_deref().and_then(crate::ffmpeg::version),
            ruta: ffmpeg.clone(),
            ..Default::default()
        }));
        crate::ffmpeg::al_iniciar(estado_ffmpeg.clone(), cfg.archivos.actualizar_ffmpeg, ctx.clone());
        let miniaturas = crate::miniaturas::Miniaturas::new(ffmpeg.clone(), &carpeta_local, ctx.clone());
        let vista = VistaPrevia::new(ffmpeg, &cfg.vista_previa, &carpeta_local);
        let mut explorador = Explorador::new(rt.handle().clone(), ctx);
        // Donde se quedó el explorador la última vez.
        let ultima = UltimaCarpeta::leer(&carpeta_local);
        if let Some(c) = ultima.carpeta.clone().filter(|c| c.is_dir()) {
            explorador.abrir(c);
        }
        Ventana {
            ctl,
            ajustes,
            vu: Vec::new(),
            carpeta_datos_local: carpeta_local.clone(),
            det: deteccion_ui::DetUi::new(&cfg.deteccion),
            detector: None,
            corte_detectado: None,
            programa_detectado: None,
            sen: Default::default(),
            miniaturas,
            vista_flotante: false,
            foto: None,
            foto_en: Instant::now(),
            cache_rutas: Default::default(),
            cfg,
            zona,
            explorador,
            tandas,
            tanda_sel: None,
            tanda_multi: Vec::new(),
            guardar_ids: None,
            marco_tanda: None,
            tanda_arriba: 0.0,
            tanda_rects: HashMap::new(),
            detenido_al_cerrar: false,
            cierre: None,
            preguntar_cierre: false,
            archivos_multi: Vec::new(),
            archivos_visibles: Vec::new(),
            marco_archivos: None,
            archivos_arriba: 0.0,
            archivos_en: (None, false),
            resalte_desde: 0.0,
            volumen: 1.0,
            aviso_visto: 0,
            aviso: None,
            texturas: HashMap::new(),
            clips: HashMap::new(),
            clips_de_carpeta: None,
            guardando: None,
            publicidades,
            arranque,
            usuarios,
            conexion,
            puerto_red,
            puerto_api,
            ventana_usuarios: VentanaUsuarios::new(),
            vista,
            monitor,
            monitor_cue,
            monitor_clip: None,
            ffmpeg: estado_ffmpeg,
            act: Default::default(),
            act_instalar: false,
            act_avisada: None,
            ffmpeg_visto: 0,
            navegacion: Navegacion::new(&carpeta_local),
            diseno: Diseno::new(&carpeta_local),
            pantalla_vista: (Pantalla::Emision, false),
            cambio_pantalla: 0.0,
            tanda_llegada: HashMap::new(),
            tanda_cargada: false,
            arbol_visible: true,
            vista_caspar: es_remoto,
            carpeta_caspar: ultima.carpeta_caspar.clone(),
            ultima_carpeta: ultima,
            editando_ruta: false,
            resalte_visto: None,
            busqueda_caspar: String::new(),
            caspar_sel: None,
            arranque_visto: 0,
            pauta_desplazada: false,
            carpeta_provisional: None,
            inicio: Instant::now(),
            pantalla: Pantalla::Emision,
            bib_sel: None,
            bib_busqueda: String::new(),
            bib_filtro: Filtro::Todos,
            confirmar: None,
            cambios_vistos: 0,
            compacta: false,
            reportes: EstadoReportes::new(time::OffsetDateTime::now_utc().to_offset(zona).date()),
            caspar: EstadoCaspar::new(),
            escala_aire: 1.0,
            aviso_inicial: problema_red,
            _rt: rt,
        }
    }

    fn hora(&self, formato: &[time::format_description::BorrowedFormatItem<'_>]) -> String {
        time::OffsetDateTime::now_utc().to_offset(self.zona).format(formato).unwrap_or_default()
    }

    fn hoy(&self) -> time::Date {
        time::OffsetDateTime::now_utc().to_offset(self.zona).date()
    }

    /// Clip de CasparCG y duración del video seleccionado en Archivos (si está en CasparCG).
    fn video_seleccionado(&mut self, e: &Instantanea) -> Option<(String, Duration)> {
        let ruta = self.explorador.seleccion.clone()?;
        let clip = self.clip_de(&ruta, e)?;
        let m = e.medio(&clip)?;
        Some((m.nombre.clone(), Duration::from_secs_f64(m.duracion_seg.max(0.0))))
    }

    /// Muestra lo que hizo la puesta en marcha automática.
    fn revisar_arranque(&mut self) {
        if let Some(p) = self.aviso_inicial.take() {
            self.avisar_error(p);
        }
        let Some(arranque) = self.arranque.clone() else { return };
        let (nuevos, problema) = {
            let a = arranque.lock().expect("arranque");
            let nuevos: Vec<String> = a.mensajes[self.arranque_visto.min(a.mensajes.len())..].to_vec();
            self.arranque_visto = a.mensajes.len();
            (nuevos, a.problema.clone())
        };
        if let Some(ultimo) = nuevos.last() {
            if problema.as_deref() == Some(ultimo.as_str()) {
                self.avisar_error(ultimo.clone());
            } else {
                self.avisar_local(ultimo.clone());
            }
        }
    }

    /// ⏹: detiene la tanda (Play retoma el spot donde quedó); otra vez, la reinicia.
    /// La selección se quita para que Play no empiece desde un spot seleccionado antes.
    pub(super) fn detener(&mut self) {
        self.tanda_sel = None;
        self.ctl.enviar(Orden::VolverATv);
    }

    fn avisar_error(&mut self, texto: impl Into<String>) {
        self.aviso = Some((texto.into(), Instant::now(), true));
    }

    fn avisar_local(&mut self, texto: impl Into<String>) {
        self.aviso = Some((texto.into(), Instant::now(), false));
    }

    /// Nombre de clip de CasparCG para un archivo (con caché).
    fn clip_de(&mut self, ruta: &Path, e: &Instantanea) -> Option<String> {
        if self.clips_de_carpeta != e.carpeta_medios {
            self.clips.clear();
            self.clips_de_carpeta = e.carpeta_medios.clone();
        }
        let medios = e.carpeta_medios.as_ref()?;
        self.clips.entry(ruta.to_path_buf()).or_insert_with(|| archivos::clip_de(ruta, medios)).clone()
    }

    /// Textura de la miniatura de un clip (la pide a CasparCG la primera vez).
    fn textura(&mut self, ctx: &egui::Context, clip: &str) -> Option<egui::TextureHandle> {
        let clave = clip.to_uppercase();
        if let Some(t) = self.texturas.get(&clave) {
            return t.clone();
        }
        match self.ctl.miniatura(clip) {
            Miniatura::Pendiente => None,
            Miniatura::NoDisponible => {
                self.texturas.insert(clave, None);
                None
            }
            Miniatura::Lista(png) => {
                let t = image::load_from_memory(&png).ok().map(|img| {
                    let rgba = img.to_rgba8();
                    let tam = [rgba.width() as usize, rgba.height() as usize];
                    let imagen = egui::ColorImage::from_rgba_unmultiplied(tam, rgba.as_raw());
                    ctx.load_texture(format!("mini-{clave}"), imagen, egui::TextureOptions::LINEAR)
                });
                self.texturas.insert(clave, t.clone());
                t
            }
        }
    }

    /// Miniatura de un archivo de video, hecha con ffmpeg.
    fn textura_de_archivo(&mut self, ctx: &egui::Context, archivo: &Path) -> Option<egui::TextureHandle> {
        let clave = format!("archivo:{}", archivo.display());
        if let Some(t) = self.texturas.get(&clave) {
            return t.clone();
        }
        match self.miniaturas.pedir(archivo) {
            crate::miniaturas::Estado::Pendiente => None,
            // No se guarda: puede aparecer ffmpeg más tarde.
            crate::miniaturas::Estado::NoDisponible => None,
            crate::miniaturas::Estado::Lista(png) => {
                let t = image::load_from_memory(&png).ok().map(|img| {
                    let rgba = img.to_rgba8();
                    let tam = [rgba.width() as usize, rgba.height() as usize];
                    let imagen = egui::ColorImage::from_rgba_unmultiplied(tam, rgba.as_raw());
                    ctx.load_texture(&clave, imagen, egui::TextureOptions::LINEAR)
                });
                // Para no llenar la memoria de video con carpetas enormes.
                if self.texturas.len() > 600 {
                    self.texturas.clear();
                }
                self.texturas.insert(clave, t.clone());
                t
            }
        }
    }

    /// Dibuja la miniatura real si existe; si no, la provisional.
    fn pintar_miniatura(&mut self, ui: &egui::Ui, rect: egui::Rect, clip: Option<&str>, semilla: &str, resaltar: bool) {
        // Primero la hecha con ffmpeg a partir del archivo (sirve para cualquier
        // carpeta); si no se puede, la de CasparCG; si tampoco, un color.
        let archivo = {
            let p = Path::new(semilla);
            if p.is_absolute() && archivos::es_video(p) {
                Some(p.to_path_buf())
            } else {
                let foto = self.foto.clone();
                clip.and_then(|c| foto.as_deref().and_then(|e| self.ruta_de_clip(c, e)))
            }
        };
        let tex = archivo
            .and_then(|a| self.textura_de_archivo(ui.ctx(), &a))
            .or_else(|| clip.and_then(|c| self.textura(ui.ctx(), c)));
        match tex {
            Some(t) => {
                ui.painter().rect_filled(rect, 5.0, Color32::BLACK);
                egui::Image::new(&t).corner_radius(5).fit_to_exact_size(rect.size()).paint_at(ui, rect);
                if resaltar {
                    ui.painter().rect_filled(rect, 5.0, Color32::from_white_alpha(18));
                }
            }
            None => miniatura(ui.painter(), rect, semilla, resaltar),
        }
    }

    fn agregar_archivo(&mut self, ruta: &Path, duracion: Option<Duration>, destino: Destino) {
        // Si no es MP4/MOV, ffmpeg (si está) da la duración.
        let duracion =
            duracion.or_else(|| self.vista.ffmpeg().and_then(|f| crate::vista_previa::duracion_con_ffmpeg(f, ruta)));
        self.ctl.agregar_archivo(ruta.to_path_buf(), duracion, destino);
    }

    /// Ruta del archivo de un clip, si se puede ver desde esta PC.
    fn ruta_de_clip(&self, clip: &str, e: &Instantanea) -> Option<PathBuf> {
        // Se recuerda unos segundos: esto se pregunta en cada cuadro y mira el disco.
        if let Some((cuando, r)) = self.cache_rutas.borrow().get(clip) {
            if cuando.elapsed() < Duration::from_secs(3) {
                return r.clone();
            }
        }
        let r = self.buscar_ruta_de_clip(clip, e);
        let mut cache = self.cache_rutas.borrow_mut();
        if cache.len() > 500 {
            cache.clear();
        }
        cache.insert(clip.to_string(), (Instant::now(), r.clone()));
        r
    }

    fn buscar_ruta_de_clip(&self, clip: &str, e: &Instantanea) -> Option<PathBuf> {
        let p = PathBuf::from(clip);
        if (p.is_absolute() || clip.as_bytes().get(1) == Some(&b':')) && p.is_file() {
            return Some(p);
        }
        let medios = e.carpeta_medios.as_ref().filter(|m| m.is_dir())?;
        // La revisión de la biblioteca sabe el archivo exacto (con mayúsculas y extensión).
        if let Some(f) = e.ficha(clip) {
            let p = medios.join(&f.ruta);
            if p.is_file() {
                return Some(p);
            }
        }
        let (carpeta, nombre) = partir(clip);
        let dir = medios.join(carpeta.trim_end_matches('/'));
        std::fs::read_dir(dir).ok()?.flatten().map(|f| f.path()).find(|f| {
            archivos::es_video(f) && f.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(nombre))
        })
    }

    /// Vista previa dentro de la columna de archivos.
    fn reproductor(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        // En la PC principal, la vista previa se calla mientras hay publicidad al aire,
        // por si su sonido sale por la misma tarjeta que va a la mezcladora.
        let silenciar = (!self.ctl.es_remoto() && self.cfg.vista_previa.silenciar_al_aire && e.fase != FaseVista::Tv)
            .then_some("Sin sonido mientras hay publicidad al aire");
        match self.vista.mostrar_en(ui, silenciar) {
            AccionVista::Nada => {}
            AccionVista::Cerrar => self.vista_flotante = false,
            AccionVista::Agregar(destino) => {
                if let Some((ruta, dur)) = self.vista.video() {
                    let (ruta, dur) = (ruta.to_path_buf(), dur);
                    // (Si está en la carpeta de CasparCG, el núcleo lo usa por su nombre de clip.)
                    self.agregar_archivo(&ruta, dur, destino);
                }
            }
        }
    }

    fn abrir_vista_previa(&mut self, ruta: PathBuf, duracion: Option<Duration>) {
        let titulo = ruta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.vista.abrir(ruta, titulo, duracion);
        // Por defecto, en la ventanita flotante; el detalle la pone en su miniatura.
        self.vista_flotante = true;
    }

    /// Vista previa en una ventanita dentro de la app, que se puede mover y agrandar
    /// (no tapa de forma fija la lista de archivos).
    fn ventana_vista_previa(&mut self, ctx: &egui::Context, e: &Instantanea) {
        if !self.vista.activa() || !self.vista_flotante || self.pantalla != Pantalla::Emision || self.compacta {
            return;
        }
        let pantalla = ctx.content_rect();
        egui::Window::new("vista-previa-flotante")
            .title_bar(false)
            .resizable([true, false])
            .default_width(420.0)
            .min_width(300.0)
            .max_width(760.0)
            // Por defecto sobre la tanda, para no tapar la lista de archivos.
            .default_pos(egui::pos2(pantalla.center().x - 210.0, pantalla.bottom() - 480.0))
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, BORDE))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(egui::Margin::same(6))
                    .shadow(egui::Shadow {
                        offset: [0, 10],
                        blur: 30,
                        spread: 0,
                        color: Color32::from_black_alpha(170),
                    }),
            )
            .show(ctx, |ui| {
                self.vista.ajustar(ui.available_width(), 600.0);
                self.reproductor(ui, e);
            });
    }
}

impl eframe::App for Ventana {
    /// Al cerrar: si está activado, la publicidad que esté saliendo se detiene (vuelve
    /// la señal de TV). Se espera un momento a que CasparCG lo haga.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.detener_al_cerrar();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Se pidió cerrar la ventana: primero se detiene lo que esté al aire.
        if ui.input(|i| i.viewport().close_requested()) {
            self.al_pedir_cierre(ui.ctx());
        }
        self.ui_ventana(ui);
        self.dialogo_cierre(ui.ctx());
    }
}

impl Ventana {
    /// ¿Al cerrar, toca cerrar también CasparCG? (Solo si lo abrió Relevo, en esta PC.)
    fn caspar_para_cerrar(&self) -> bool {
        !self.ctl.es_remoto()
            && arranque::es_local(&self.cfg.casparcg.host)
            && arranque::caspar_es_propio()
            && !self.cfg.casparcg.cerrar_al_salir.trim().eq_ignore_ascii_case("no")
    }

    fn al_pedir_cierre(&mut self, ctx: &egui::Context) {
        match self.cierre {
            // Reinicio o actualización: CasparCG sigue.
            Some(false) => self.detener_al_cerrar(),
            Some(true) => {}
            None => {
                let preguntar = self.cfg.casparcg.cerrar_al_salir.trim().eq_ignore_ascii_case("preguntar");
                if self.caspar_para_cerrar() && preguntar {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.preguntar_cierre = true;
                } else {
                    let cerrar = self.caspar_para_cerrar();
                    self.cerrar_todo(cerrar);
                }
            }
        }
    }

    /// Detiene lo que esté al aire y, si `caspar`, cierra CasparCG y su scanner.
    fn cerrar_todo(&mut self, caspar: bool) {
        self.cierre = Some(true);
        self.detener_al_cerrar();
        if caspar {
            let carpeta = self.arranque.as_ref().and_then(|a| a.lock().ok().and_then(|a| a.carpeta.clone()));
            let hechos = arranque::cerrar_lo_propio(carpeta.as_deref());
            if !hechos.is_empty() {
                tracing::info!("Relevo se cierra: se cerró {}", hechos.join(" y "));
            }
        }
    }

    /// «¿Cerrar también CasparCG?»
    fn dialogo_cierre(&mut self, ctx: &egui::Context) {
        if !self.preguntar_cierre {
            return;
        }
        let mut elegido: Option<Option<bool>> = None;
        egui::Modal::new(egui::Id::unique("dialogo_cierre")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new("¿Cerrar también CasparCG?").size(18.0).strong());
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Relevo abrió CasparCG. Si lo cierras, su salida de video se apaga; si lo dejas abierto, al volver a abrir Relevo se conecta sin reiniciarlo.",
                )
                .color(TENUE),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Cerrar todo").color(AIRE))).clicked() {
                    elegido = Some(Some(true));
                }
                if ui.button("Dejar CasparCG abierto").clicked() {
                    elegido = Some(Some(false));
                }
                if ui.button("Cancelar").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    elegido = Some(None);
                }
            });
        });
        if let Some(e) = elegido {
            self.preguntar_cierre = false;
            if let Some(caspar) = e {
                self.cerrar_todo(caspar);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn detener_al_cerrar(&mut self) {
        if self.detenido_al_cerrar || !self.cfg.casparcg.detener_al_cerrar || self.ctl.es_remoto() {
            return;
        }
        self.detenido_al_cerrar = true;
        let e = self.ctl.ver();
        if e.fase == FaseVista::Tv || !self.ctl.tengo_mando(&e) {
            return;
        }
        tracing::info!("Relevo se cierra: se detiene la publicidad al aire");
        self.ctl.enviar(Orden::VolverATv);
        let hasta = Instant::now() + Duration::from_secs(3);
        while Instant::now() < hasta {
            std::thread::sleep(Duration::from_millis(50));
            if self.ctl.ver().fase == FaseVista::Tv {
                // Un poco más para que el comando llegue a CasparCG.
                std::thread::sleep(Duration::from_millis(200));
                break;
            }
        }
    }

    fn ui_ventana(&mut self, ui: &mut egui::Ui) {
        ajustar_escala(ui.ctx(), self.cfg.interfaz.escala);
        // El estado del núcleo se copia solo cuando cambió (no en cada cuadro).
        if self.foto.is_none()
            || CAMBIO.swap(false, std::sync::atomic::Ordering::Relaxed)
            || self.foto_en.elapsed() > Duration::from_millis(400)
        {
            self.foto = Some(Arc::new(self.ctl.ver()));
            self.foto_en = Instant::now();
        }
        let e = self.foto.clone().expect("recién copiada");

        // La primera vez, el explorador abre la carpeta de CasparCG. Si en unos
        // segundos no se conoce, abre la carpeta de videos del usuario.
        if self.explorador.carpeta.is_none() {
            let medios = e.carpeta_medios.clone().filter(|p| p.is_dir());
            let esperar_mas = self.inicio.elapsed() < Duration::from_secs(3);
            let inicio = medios.or_else(|| {
                if esperar_mas {
                    return None;
                }
                dirs::video_dir().filter(|d| d.is_dir()).or_else(dirs::home_dir)
            });
            if let Some(c) = inicio {
                self.carpeta_provisional = e.carpeta_medios.as_ref().is_none_or(|m| *m != c).then(|| c.clone());
                self.explorador.abrir(c);
            }
        } else if let (Some(prov), Some(m)) = (&self.carpeta_provisional, &e.carpeta_medios) {
            // Se abrió otra carpeta mientras CasparCG arrancaba: ahora que se conoce la
            // de medios, se cambia a ella (si el operador no se movió de la provisional).
            if self.explorador.carpeta.as_ref() == Some(prov) && m.is_dir() {
                self.explorador.abrir(m.clone());
            }
            self.carpeta_provisional = None;
        }

        self.recordar_carpeta();

        // Avisos nuevos del núcleo.
        if let Some(a) = &e.aviso {
            if a.id > self.aviso_visto {
                self.aviso_visto = a.id;
                if a.para.is_none_or(|p| p == self.ctl.sesion()) {
                    self.aviso = Some((a.texto.clone(), Instant::now(), true));
                }
            }
        }
        // Una remota que pierde la conexión lo muestra siempre.
        if let Some(c) = &self.conexion {
            if let Conexion::Caida(m) = &*c.lock().expect("conexion") {
                if self.aviso.as_ref().is_none_or(|(t, _, _)| t != m) {
                    self.aviso = Some((format!("{m}. Reintentando…"), Instant::now(), true));
                }
            }
        }

        // Cambiaron archivos de la biblioteca (nivelado, reemplazo…): miniaturas nuevas.
        if e.biblioteca.cambios != self.cambios_vistos {
            self.cambios_vistos = e.biblioteca.cambios;
            self.ctl.olvidar_miniaturas();
            self.texturas.clear();
            self.clips.clear();
        }

        // ffmpeg nuevo (descargado o actualizado): lo usan la vista previa, el monitor y el núcleo.
        let (cambios, ruta) = {
            let f = self.ffmpeg.lock().expect("ffmpeg");
            (f.cambios, f.ruta.clone())
        };
        if cambios != self.ffmpeg_visto {
            self.ffmpeg_visto = cambios;
            self.vista.usar_ffmpeg(ruta.clone());
            self.monitor.usar_ffmpeg(ruta.clone());
            self.monitor_cue.usar_ffmpeg(ruta.clone());
            self.miniaturas.usar_ffmpeg(ruta.clone());
            if let (Some(r), false) = (ruta, self.ctl.es_remoto()) {
                self.ctl.enviar(Orden::UsarFfmpeg(r));
            }
        }

        self.revisar_arranque();
        self.atajos(ui, &e);
        self.archivos_soltados(ui, &e);

        if self.pantalla_vista != (self.pantalla, self.compacta) {
            self.pantalla_vista = (self.pantalla, self.compacta);
            self.cambio_pantalla = ui.ctx().input(|i| i.time);
        }
        let entrada_pantalla = dialogo::desde(ui.ctx(), self.cambio_pantalla, 0.28);

        egui::Panel::top("cabecera")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin { left: 16, right: 16, top: 7, bottom: 7 }))
            .show_separator_line(false)
            .show(ui, |ui| {
                self.cabecera(ui, &e);
                // Línea fina que separa la cabecera del resto.
                let r = ui.max_rect();
                let y = r.bottom() + 7.0;
                ui.painter().hline((r.left() - 16.0)..=(r.right() + 16.0), y - 0.5, Stroke::new(1.0, BORDE));
            });

        egui::Panel::bottom("estado")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(16, 5)))
            .show_separator_line(false)
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().hline((r.left() - 16.0)..=(r.right() + 16.0), r.top() - 4.5, Stroke::new(1.0, BORDE));
                self.barra_estado(ui, &e);
            });

        if self.compacta {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(FONDO).inner_margin(egui::Margin::same(16)))
                .show(ui, |ui| {
                    ui.multiply_opacity(entrada_pantalla);
                    self.pantalla_compacta(ui, &e)
                });
            self.mostrar_aviso(ui);
            let espera = if e.fase == FaseVista::AlAire { 50 } else { 500 };
            ui.ctx().request_repaint_after(Duration::from_millis(espera));
            return;
        }

        egui::Panel::top("al_aire")
            .frame(egui::Frame::new().fill(FONDO).inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 12,
                bottom: 0,
            }))
            .show_separator_line(false)
            .show(ui, |ui| {
                self.al_aire(ui, &e);
                if self.pantalla == Pantalla::Emision {
                    // Separación de abajo: cambia el alto del monitor.
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), diseno::GROSOR), Sense::hover());
                    let r = diseno::divisor(ui, "al_aire", rect, false);
                    if r.dragged() {
                        let max = (ui.ctx().content_rect().height() * 0.45).max(120.0);
                        let min = al_aire_ui::ALTO_DATOS * 0.55;
                        self.diseno.m.monitor = (self.diseno.m.monitor + r.drag_delta().y).clamp(min, max.max(min));
                    }
                    if r.double_clicked() {
                        self.diseno.m.monitor = diseno::Medidas::default().monitor;
                    }
                    if r.drag_stopped() || r.double_clicked() {
                        self.diseno.guardar();
                    }
                }
            });

        let margen_arriba = if self.pantalla == Pantalla::Emision { 0 } else { 12 };
        let marco_central = egui::Frame::new().fill(FONDO).inner_margin(egui::Margin {
            left: 12,
            right: 12,
            top: margen_arriba,
            bottom: 12,
        });
        egui::CentralPanel::default().frame(marco_central).show(ui, |ui| {
            // Al cambiar de pantalla, lo nuevo aparece con un fundido corto.
            ui.multiply_opacity(entrada_pantalla);
            if self.pantalla == Pantalla::Biblioteca {
                self.pantalla_biblioteca(ui, &e);
                return;
            }
            if self.pantalla == Pantalla::Reportes {
                self.pantalla_reportes(ui, &e);
                return;
            }
            if self.pantalla == Pantalla::CasparCG {
                self.pantalla_caspar(ui, &e);
                return;
            }
            if self.pantalla == Pantalla::Publicidades {
                self.pantalla_publicidades(ui, &e);
                return;
            }
            self.columnas_emision(ui, &e);
        });

        self.ventana_guardar(ui, &e);
        if let Some(u) = self.usuarios.clone() {
            self.ventana_usuarios.mostrar(ui.ctx(), &u, self.puerto_red);
        }
        self.seguir_deteccion(ui.ctx(), &e);
        self.seguir_senales(ui.ctx());
        self.seguir_actualizacion(ui.ctx(), &e);
        self.ventana_deteccion(ui.ctx());
        self.avisos_visuales(ui.ctx(), &e);
        self.ventana_vista_previa(ui.ctx(), &e);
        self.ventana_ajustes(ui.ctx(), &e);
        self.mostrar_aviso(ui);
        diseno::etiqueta_arrastre(ui.ctx());
        if diseno::arrastrando(ui.ctx()).is_some() {
            ui.ctx().request_repaint();
        }

        // Refresco: fluido mientras hay algo al aire; si no, solo cuando cambia el
        // segundo del reloj (lo demás se redibuja cuando el núcleo avisa de un cambio).
        let espera = if e.fase == FaseVista::AlAire || !e.importaciones.is_empty() {
            50
        } else {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_millis() as u64)
                .unwrap_or(0);
            1005 - ms.min(1000)
        };
        ui.ctx().request_repaint_after(Duration::from_millis(espera));
    }
}

// ======================================================================
// Teclado y archivos soltados
// ======================================================================

impl Ventana {
    fn atajos(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let escribiendo = ui.ctx().egui_wants_keyboard_input();
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        // Esc funciona siempre, incluso escribiendo en el buscador.
        // (Con una ventanita abierta, Esc solo la cierra.)
        let ventanita = self.guardando.is_some() || self.confirmar.is_some() || self.ajustes.abierta;
        if esc && self.cfg.interfaz.boton_volver_a_tv && !ventanita {
            self.ctl.enviar(Orden::VolverATv);
        }
        if escribiendo || ventanita {
            return;
        }
        // Los atajos (configurables en Ajustes) se "consumen" antes de dibujar, para
        // que Espacio o Enter no activen además el botón que tenga el foco.
        let a = self.cfg.atajos.clone();
        let pulso = |ui: &mut egui::Ui, t: &str| ui.input_mut(|i| crate::atajos::pulsado(i, t));
        // Primero los que llevan Ctrl/Mayús/Alt (si no, "Ctrl+→" contaría también como "→").
        let mut orden: Vec<(&str, u8)> = vec![
            (&a.play_pausa, 0),
            (&a.siguiente, 1),
            (&a.anterior, 2),
            (&a.detener, 3),
            (&a.anadir, 4),
            (&a.quitar, 5),
            (&a.subir, 6),
            (&a.bajar, 7),
            (&a.emision, 8),
            (&a.biblioteca, 9),
            (&a.reportes, 10),
            (&a.compacta, 11),
            (&a.casparcg, 12),
            (&a.publicidades, 13),
        ];
        orden.sort_by_key(|(t, _)| std::cmp::Reverse(t.matches('+').count()));
        let al_aire = e.fase != FaseVista::Tv;
        let en_tanda = self.pantalla == Pantalla::Emision;
        for (texto, accion) in orden {
            if !pulso(ui, texto) {
                continue;
            }
            match accion {
                0 => self.ctl.enviar(Orden::PlayPausa(self.tanda_sel)),
                1 if al_aire => self.ctl.enviar(Orden::Siguiente),
                2 if al_aire => self.ctl.enviar(Orden::Anterior),
                3 if al_aire || e.retomar.is_some() => self.detener(),
                4 if en_tanda => self.agregar_elegidos(Destino::Final),
                5 if en_tanda => self.quitar_elegidos(e),
                6 if en_tanda => self.mover_elegidos(e, Movida::Subir),
                7 if en_tanda => self.mover_elegidos(e, Movida::Bajar),
                8 => self.pantalla = Pantalla::Emision,
                9 => self.pantalla = Pantalla::Biblioteca,
                10 => self.pantalla = Pantalla::Reportes,
                11 => self.compacta = !self.compacta,
                12 if !self.ctl.es_remoto() => self.pantalla = Pantalla::CasparCG,
                13 => self.pantalla = Pantalla::Publicidades,
                _ => {}
            }
        }
    }

    /// Enciende, apaga o reinicia el detector de corte según la configuración, y
    /// atiende lo que detecta.
    fn seguir_deteccion(&mut self, ctx: &egui::Context, e: &Instantanea) {
        use crate::deteccion::{Detector, Evento};
        let cfg = &self.cfg.deteccion;
        // Fuente «IPTV»: la misma dirección que reproduce CasparCG.
        let fuente = crate::senal::Fuente::nueva(&cfg.fuente, &cfg.origen, cfg.monitor, &self.cfg.iptv.direccion);
        // Con su ventana abierta funciona aunque esté apagada (para probar y ajustar).
        let debe = (cfg.activa || self.det.abierta) && !self.ctl.es_remoto() && self.vista.ffmpeg().is_some();
        match fuente.filter(|_| debe) {
            None => self.detector = None,
            Some(f) => {
                if !self.detector.as_mut().is_some_and(|d| d.actualizar(cfg, &f)) {
                    self.detector = None; // primero se suelta el anterior
                    let ffmpeg = self.vista.ffmpeg().expect("comprobado").to_path_buf();
                    self.detector =
                        Some(Detector::iniciar(cfg.clone(), f, ffmpeg, &self.carpeta_datos_local, ctx.clone()));
                }
            }
        }
        if !self.cfg.deteccion.activa {
            self.corte_detectado = None; // al apagarla se quita su cartel
        }
        let Some(evento) = self.detector.as_ref().and_then(|d| d.tomar_evento()) else { return };
        let hora = self.hora(time::macros::format_description!("[hour]:[minute]:[second]"));
        if !self.cfg.deteccion.activa {
            // Apagada: solo se anota en su ventana, sin avisar ni iniciar nada.
            let t = if evento == Evento::Corte { "Corte (prueba: detección apagada)" } else { "Volvió el programa" };
            self.det.anotar(hora, t);
            return;
        }
        match evento {
            Evento::Programa => {
                if e.fase == FaseVista::Tv {
                    self.det.anotar(hora, "Volvió el programa");
                    return;
                }
                // Con publicidad al aire: el canal ya volvió de su corte.
                let volver = self.cfg.deteccion.al_volver == "volver" && self.cfg.en_transmision();
                if volver && self.ctl.tengo_mando(e) {
                    self.det.anotar(hora, "▶ Volvió el programa: se volvió al canal");
                    self.ctl.enviar(Orden::VolverATv);
                    self.avisar_local("▶ Volvió el programa del canal: se cortó la tanda");
                } else {
                    self.det.anotar(hora, "▶ Volvió el programa (con publicidad al aire)");
                    self.programa_detectado = Some(Instant::now());
                }
            }
            Evento::Corte => {
                let espera = Duration::from_secs_f64(self.cfg.deteccion.espera_min.max(0.0) * 60.0);
                let hace_poco = self.corte_detectado.is_some_and(|t| t.elapsed() < espera);
                if e.fase != FaseVista::Tv || hace_poco {
                    // Ya hay publicidad al aire, o acaba de haber un corte.
                    self.det.anotar(hora, "Corte (no se avisó: muy seguido o ya al aire)");
                    return;
                }
                self.det.anotar(hora.clone(), "✂ Corte detectado");
                self.corte_detectado = Some(Instant::now());
                let iniciar = self.cfg.deteccion.accion == "iniciar";
                if iniciar && !e.lista.is_empty() && self.ctl.tengo_mando(e) {
                    self.ctl.enviar(Orden::PlayPausa(None));
                    self.avisar_local("✂ Corte detectado: se inició la tanda");
                } else if iniciar && e.lista.is_empty() {
                    self.avisar_local("✂ Corte detectado, pero la tanda está vacía");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                    egui::UserAttentionType::Informational,
                ));
            }
        }
    }

    /// Avisos que no suenan (la PC va conectada a la mezcladora): un marco que destella
    /// alrededor de toda la ventana y un cartel arriba. No mueve nada de sitio.
    fn avisos_visuales(&mut self, ctx: &egui::Context, e: &Instantanea) {
        let ahora = ctx.input(|i| i.time);
        let pulso = ((ahora * 5.0).sin() * 0.5 + 0.5) as f32;
        let mut aviso: Option<(String, Color32)> = None;

        // 1) Se perdió CasparCG con publicidad al aire.
        if !e.conectado && e.fase != FaseVista::Tv {
            aviso = Some(("⚠ SIN CONEXIÓN CON CASPARCG".into(), AIRE));
        }
        // 2) El canal volvió de su corte y la publicidad sigue al aire.
        if e.fase == FaseVista::Tv {
            self.programa_detectado = None;
        }
        if aviso.is_none() && self.programa_detectado.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
            aviso = Some(("▶ VOLVIÓ EL PROGRAMA DEL CANAL".into(), TV));
        }
        // Se cortó el canal por IPTV mientras está al aire.
        if aviso.is_none() && e.fase == FaseVista::Tv && matches!(e.iptv.estado, nucleo::EstadoIptv::Caida(_)) {
            aviso = Some(("📺 SE CORTÓ LA SEÑAL IPTV — reintentando".into(), AIRE));
        }
        // 3) Falta poco para volver a la señal de TV.
        let n = self.cfg.interfaz.aviso_fin_seg as f64;
        if aviso.is_none() && n > 0.0 && e.fase == FaseVista::AlAire && e.bucle == Bucle::No {
            if let Some(i) = e.actual.and_then(|id| e.lista.iter().position(|x| x.id == id)) {
                let resta: f64 =
                    e.lista[i..].iter().map(|x| x.duracion.as_secs_f64()).sum::<f64>() - e.transcurrido.as_secs_f64();
                if resta > 0.0 && resta <= n {
                    aviso = Some((format!("📺 VUELVE LA SEÑAL DE TV EN {}", resta.ceil() as u64), AMBAR));
                }
            }
        }
        // 3) El detector vio el corte del canal y la tanda no salió sola.
        if aviso.is_none()
            && e.fase == FaseVista::Tv
            && self.corte_detectado.is_some_and(|t| t.elapsed() < Duration::from_secs(25))
        {
            let tecla = crate::atajos::bonito(&self.cfg.atajos.play_pausa);
            aviso = Some((format!("✂ CORTE DETECTADO — pulsa {tecla} para iniciar la tanda"), ACENTO));
        }
        let Some((texto, color)) = aviso else { return };
        let pantalla = ctx.content_rect();
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("aviso-visual")));
        let fuerza = if dialogo::animaciones() { 0.35 + 0.65 * pulso } else { 1.0 };
        p.rect_stroke(pantalla, 0.0, Stroke::new(5.0, color.gamma_multiply(fuerza)), StrokeKind::Inside);
        let galley = p.layout_no_wrap(texto, FontId::new(15.0, tema::negrita()), Color32::WHITE);
        let caja = egui::Rect::from_center_size(
            egui::pos2(pantalla.center().x, pantalla.top() + 26.0),
            galley.size() + Vec2::new(28.0, 12.0),
        );
        p.rect_filled(caja, 8.0, dialogo::mezclar(Color32::from_rgb(0x14, 0x18, 0x20), color, 0.55));
        p.rect_stroke(caja, 8.0, Stroke::new(1.5, color), StrokeKind::Inside);
        p.galley(caja.min + Vec2::new(14.0, 6.0), galley, Color32::WHITE);
        ctx.request_repaint_after(Duration::from_millis(40));
    }

    /// Modo de trabajo: Publicidad (CasparCG solo pone los spots; el canal va por la
    /// mezcladora) o Transmisión (CasparCG muestra el canal por IPTV y la publicidad
    /// entra encima).
    pub(super) fn selector_modo(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let posible = self.cfg.transmision_posible();
        let transmision = self.cfg.en_transmision();
        let libre = e.fase == FaseVista::Tv;
        let mut nuevo: Option<&str> = None;
        let k = self.escala_aire;
        egui::Frame::new()
            .fill(Color32::from_rgb(0x12, 0x15, 0x1C))
            .stroke(Stroke::new(1.0, BORDE))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::same(2))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                // Publicidad a la izquierda y Transmisión a la derecha, aunque la fila se
                // dibuje de derecha a izquierda.
                let mut botones =
                    vec![("publicidad", "🎞 Publicidad", !transmision), ("transmision", "📺 Transmisión", transmision)];
                if ui.layout().prefer_right_to_left() {
                    botones.reverse();
                }
                for (clave, texto, activo) in botones {
                    let color = if activo { Color32::WHITE } else { TENUE };
                    let b = egui::Button::new(RichText::new(texto).size(11.5 * k).color(color))
                        .fill(if activo { ACENTO_OSCURO } else { Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(6))
                        .min_size(Vec2::new(0.0, 22.0 * k));
                    let habil = libre && !activo;
                    let r = ui.add_enabled(habil, b);
                    let r = if !libre && !activo {
                        r.on_disabled_hover_text("Espera a que termine la tanda para cambiar de modo")
                    } else if clave == "publicidad" {
                        r.on_hover_text("CasparCG solo pone la publicidad; el canal va por la mezcladora")
                    } else if posible {
                        r.on_hover_text("CasparCG muestra el canal por IPTV y la publicidad entra encima")
                    } else {
                        r.on_hover_text("Primero permite el modo Transmisión en ⚙ Ajustes → Señal por IPTV")
                    };
                    if r.clicked() {
                        nuevo = Some(clave);
                    }
                }
            });
        let Some(modo) = nuevo else { return };
        if modo == "transmision" && !posible {
            self.ajustes.abrir_iptv(&self.cfg);
            return;
        }
        self.cfg.emision.modo = modo.to_string();
        let m = modo.to_string();
        self.ajustes.guardar_con(|c| c.emision.modo = m.clone());
        self.ctl.enviar(Orden::Iptv(self.cfg.iptv_para_nucleo()));
        self.avisar_local(if modo == "transmision" {
            "📺 Modo Transmisión: CasparCG muestra el canal por IPTV"
        } else {
            "🎞 Modo Publicidad: CasparCG solo pone la publicidad"
        });
    }

    /// Estado de la señal por IPTV en la barra de abajo (con un clic: reconectar o configurar).
    fn chip_iptv(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        use nucleo::EstadoIptv as E;
        let (texto, color) = match &e.iptv.estado {
            E::Apagada => return,
            E::Conectando => ("📺 IPTV: conectando…".to_string(), TENUE),
            E::EnVivo => ("📺 IPTV: en vivo".to_string(), TV),
            E::SinVigilancia => ("📺 IPTV: enviado (sin OSC)".to_string(), AMBAR),
            E::Caida(_) => ("📺 IPTV: CORTADA".to_string(), AIRE),
        };
        let r = ui
            .add(egui::Button::new(RichText::new(texto).size(12.0).color(color)).frame(false))
            .on_hover_text(format!("Señal de TV por IPTV (capa {})\n{}", e.iptv.capa, e.iptv.direccion));
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(260.0);
            let corta: String = e.iptv.direccion.chars().take(46).collect();
            ui.label(RichText::new(corta).size(11.5).color(TENUE));
            match &e.iptv.estado {
                E::Caida(m) => {
                    ui.label(RichText::new(m).size(11.5).color(AIRE));
                }
                E::SinVigilancia => {
                    ui.label(
                        RichText::new("CasparCG no manda OSC: no se puede comprobar si llega.").size(11.5).color(AMBAR),
                    );
                }
                _ => {}
            }
            if e.iptv.reconexiones > 0 {
                ui.label(RichText::new(format!("Reconexiones: {}", e.iptv.reconexiones)).size(11.5).color(TENUE));
            }
            if e.iptv.silenciado {
                ui.label(RichText::new("🔇 Sonido del canal bajado (publicidad al aire)").size(11.5).color(TENUE));
            }
            ui.separator();
            if ui.button("⟳ Reconectar ahora").clicked() {
                self.ctl.enviar(Orden::ReconectarIptv);
                ui.close();
            }
            if ui.button("⚙ Configurar…").clicked() {
                self.ajustes.abrir_iptv(&self.cfg);
                ui.close();
            }
        });
    }

    /// Videos arrastrados desde el Explorador de Windows a la ventana: van a la tanda.
    fn archivos_soltados(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let soltados: Vec<PathBuf> = ui.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect()
        });
        let mut ignorados = 0;
        for ruta in soltados {
            if self.pantalla == Pantalla::Biblioteca {
                // En la Biblioteca, el video soltado reemplaza al spot elegido.
                if archivos::es_video(&ruta) {
                    self.soltado_en_biblioteca(ruta, e);
                } else {
                    ignorados += 1;
                }
                break;
            }
            if archivos::es_video(&ruta) {
                let dur = archivos::duracion_video(&ruta);
                self.agregar_archivo(&ruta, dur, Destino::Final);
            } else {
                ignorados += 1;
            }
        }
        if ignorados > 0 {
            self.avisar_local(format!("Se ignoraron {ignorados} archivos que no son video"));
        }
    }
}

// ======================================================================
// Cabecera
// ======================================================================

impl Ventana {
    fn cabecera(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        ui.horizontal(|ui| {
            // Marca: cuadrado rojo con el triángulo de "reproducir" y el nombre.
            let (logo, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
            ui.painter().rect_filled(logo, 7.0, Color32::from_rgb(0xD9, 0x3A, 0x40));
            let c = logo.center();
            ui.painter().add(egui::Shape::convex_polygon(
                vec![c + Vec2::new(-4.5, -6.5), c + Vec2::new(7.0, 0.0), c + Vec2::new(-4.5, 6.5)],
                Color32::WHITE,
                Stroke::NONE,
            ));
            ui.label(RichText::new("Relevo").size(19.0).family(tema::negrita()).color(TEXTO));
            ui.add_space(14.0);
            // Pestañas.
            let graves = self.problemas_graves(e);
            let mut pestanas = vec![
                (Pantalla::Emision, "Emisión".to_string()),
                (Pantalla::Publicidades, "Publicidades".to_string()),
                (Pantalla::Biblioteca, "Biblioteca".to_string()),
                (Pantalla::Reportes, "Reportes".to_string()),
            ];
            // CasparCG solo se maneja desde la PC donde está.
            if !self.ctl.es_remoto() {
                pestanas.push((Pantalla::CasparCG, "CasparCG".to_string()));
            }
            for (p, t) in pestanas {
                let activa = self.pantalla == p;
                // Pestañas de texto: la activa en negrita con una raya de color debajo.
                let mut texto = RichText::new(&t).size(14.5).color(if activa { TEXTO } else { TENUE });
                if activa {
                    texto = texto.family(tema::negrita());
                }
                let b = egui::Button::new(texto)
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE)
                    .min_size(Vec2::new(0.0, 32.0));
                let r = ui.add(b);
                let k = ui.ctx().animate_bool_with_time(r.id.with("raya"), activa, 0.18);
                if k > 0.0 {
                    let ancho = (r.rect.width() - 16.0) * k;
                    let raya = egui::Rect::from_center_size(
                        egui::pos2(r.rect.center().x, r.rect.bottom() + 6.0),
                        Vec2::new(ancho, 3.0),
                    );
                    ui.painter().rect_filled(raya, 2.0, ACENTO);
                } else if r.hovered() {
                    ui.painter().rect_filled(r.rect, 6.0, Color32::from_white_alpha(8));
                }
                if p == Pantalla::Biblioteca && graves > 0 {
                    let c = egui::pos2(r.rect.right() - 4.0, r.rect.top() + 4.0);
                    ui.painter().circle_filled(c, 8.0, AIRE);
                    ui.painter().text(
                        c,
                        Align2::CENTER_CENTER,
                        graves.min(99).to_string(),
                        FontId::proportional(10.0),
                        Color32::WHITE,
                    );
                }
                let r = if p == Pantalla::Biblioteca {
                    r.on_hover_text("Revisión de los videos, volumen, conversión, reemplazo y papelera")
                } else {
                    r
                };
                if p == Pantalla::Reportes && e.alertas.iter().any(|a| a.grave) {
                    let n = e.alertas.iter().filter(|a| a.grave).count();
                    let c = egui::pos2(r.rect.right() - 4.0, r.rect.top() + 4.0);
                    ui.painter().circle_filled(c, 8.0, AMBAR);
                    ui.painter().text(
                        c,
                        Align2::CENTER_CENTER,
                        n.min(99).to_string(),
                        FontId::proportional(10.0),
                        Color32::BLACK,
                    );
                }
                if r.clicked() && !activa {
                    if p == Pantalla::Reportes {
                        self.al_abrir_reportes();
                    }
                    self.pantalla = p;
                    // La vista previa abierta queda en la pantalla donde se abrió.
                    self.vista.cerrar();
                }
            }
            ui.add_space(16.0);

            let (texto, fondo, color) = match e.fase {
                FaseVista::AlAire => ("⏺  PUBLICIDAD AL AIRE", AIRE_OSCURO, AIRE),
                FaseVista::Pausado => ("⏸  PAUSADO", AMBAR_OSCURO, AMBAR),
                FaseVista::Tv => ("■  SEÑAL DE TV", TV_OSCURO, TV),
            };
            pastilla(ui, texto, fondo, color, 14.0);
            ui.add_space(10.0);
            self.indicador_mando(ui, e);

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Botones "fantasma": sin borde, se marcan al pasar el mouse.
                let w = &mut ui.visuals_mut().widgets;
                w.inactive.weak_bg_fill = Color32::TRANSPARENT;
                w.inactive.bg_stroke = Stroke::NONE;
                w.hovered.bg_stroke = Stroke::NONE;
                let reloj = self.hora(time::macros::format_description!("[hour]:[minute]:[second]"));
                ui.label(RichText::new(reloj).font(FontId::monospace(24.0)).color(TEXTO));
                let hoy = self.hoy();
                const DIAS: [&str; 7] = ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"];
                const MESES: [&str; 12] =
                    ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic"];
                let fecha = format!(
                    "{} {} {}",
                    DIAS[hoy.weekday().number_days_from_monday() as usize],
                    hoy.day(),
                    MESES[hoy.month() as usize - 1]
                );
                ui.label(RichText::new(fecha).size(12.5).color(TENUE));
                ui.add_space(8.0);
                if ui
                    .add(egui::Button::new(RichText::new("⚙ Ajustes").size(13.0)).selected(self.ajustes.abierta))
                    .on_hover_text("CasparCG, salida de video, red, respaldos…")
                    .clicked()
                {
                    self.ajustes.abrir(&self.cfg);
                }
                ui.add_space(6.0);
                let t = if self.compacta { "⛶ Vista completa" } else { "⛶ Compacta" };
                if ui
                    .add(egui::Button::new(RichText::new(t).size(13.0)).selected(self.compacta))
                    .on_hover_text("Vista compacta: lo justo para operar el corte, con botones grandes")
                    .clicked()
                {
                    self.compacta = !self.compacta;
                }
                ui.add_space(6.0);
                if let Some(c) = &self.conexion {
                    match &*c.lock().expect("conexion") {
                        Conexion::Conectada { nombre, rol } => {
                            pastilla(ui, &format!("↔  {} · {}", nombre, rol.nombre()), ACENTO_OSCURO, ACENTO, 13.0)
                        }
                        _ => pastilla(ui, "↔  Sin conexión con la PC principal", AIRE_OSCURO, AIRE, 13.0),
                    }
                    ui.add_space(6.0);
                }
                if self.usuarios.is_some() {
                    let aviso = self.usuarios.as_ref().is_some_and(|u| u.hay_clave_inicial());
                    let texto = if aviso { "⚠ Usuarios" } else { "👥 Usuarios" };
                    let b = egui::Button::new(RichText::new(texto).size(13.0).color(if aviso { AMBAR } else { TEXTO }));
                    let r = ui.add(b);
                    let r = if aviso {
                        r.on_hover_text("El usuario admin tiene la contraseña inicial: cámbiala")
                    } else {
                        r
                    };
                    if r.clicked() {
                        self.ventana_usuarios.abierta = true;
                    }
                    ui.add_space(6.0);
                }
                if e.conectado {
                    let v = e.version.clone().unwrap_or_default();
                    pastilla(ui, &format!("✔  CasparCG {v}"), TV_OSCURO, TV, 13.0);
                } else {
                    pastilla(ui, "✖  CasparCG desconectado", AIRE_OSCURO, AIRE, 13.0);
                }
            });
        });
    }

    /// Quién tiene el mando de la emisión, y botones para pedirlo o soltarlo.
    fn indicador_mando(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        // Sin PCs remotas conectadas (y en la principal), el mando no hace falta mostrarlo.
        if !self.ctl.es_remoto() && e.sesiones.is_empty() && e.mando.as_ref().is_none_or(|(id, _)| *id == 0) {
            return;
        }
        let mio = e.mando.as_ref().is_some_and(|(id, _)| *id == self.ctl.sesion());
        match &e.mando {
            Some((_, nombre)) if mio => {
                pastilla(ui, "🎮  Tienes el mando", TV_OSCURO, TV, 13.0);
                if ui.small_button("Soltar").on_hover_text("Deja que otra PC tome el mando").clicked() {
                    self.ctl.enviar(Orden::SoltarMando);
                }
                let _ = nombre;
            }
            Some((_, nombre)) => {
                pastilla(ui, &format!("🎮  Mando: {nombre}"), AMBAR_OSCURO, AMBAR, 13.0);
                if ui.small_button("Pedir").on_hover_text("Funciona si quien lo tiene lo suelta").clicked() {
                    self.ctl.enviar(Orden::TomarMando { forzar: false });
                }
                if ui.small_button(RichText::new("Quitar").color(AIRE)).on_hover_text("Solo administradores").clicked()
                {
                    self.ctl.enviar(Orden::TomarMando { forzar: true });
                }
            }
            None => {
                pastilla(ui, "🎮  Mando libre", TARJETA, TENUE, 13.0);
                if ui.small_button("Tomar").clicked() {
                    self.ctl.enviar(Orden::TomarMando { forzar: false });
                }
            }
        }
    }

    fn barra_estado(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        ui.horizontal(|ui| {
            let c = &self.cfg.casparcg;
            let pequeño = |t: String| RichText::new(t).size(12.0).color(TENUE);
            ui.label(pequeño(format!("CasparCG {}  ·  canal {} capa {}", e.direccion, c.canal, c.capa_publicidad)));
            ui.separator();
            match &e.carpeta_medios {
                Some(m) => ui.label(pequeño(format!("Medios: {}", m.display()))),
                None => ui.label(RichText::new("Carpeta de medios desconocida").size(12.0).color(AMBAR)),
            };
            ui.separator();
            if !self.ctl.es_remoto() {
                // Detección de corte: estado y un clic para encenderla o apagarla.
                use crate::deteccion::Estado;
                let (texto, color) = match self.detector.as_ref().map(|d| d.vista()) {
                    None => ("✂ Detección: apagada".to_string(), MUY_TENUE),
                    Some(v) => match v.estado {
                        Estado::Conectando => ("✂ Detección: conectando…".into(), TENUE),
                        Estado::SinAprender => ("✂ Detección: falta aprender el logo".into(), AMBAR),
                        Estado::Programa => (format!("✂ Programa ({:.2})", v.puntaje), TV),
                        Estado::Corte => (format!("✂ CORTE ({:.2})", v.puntaje), AIRE),
                        Estado::Error(_) => ("✂ Detección: sin señal".into(), AIRE),
                    },
                };
                let r = ui
                    .add(egui::Button::new(RichText::new(texto).size(12.0).color(color)).frame(false))
                    .on_hover_text("Detección del corte del canal");
                egui::Popup::menu(&r).show(|ui| {
                    ui.set_min_width(230.0);
                    let activa = self.cfg.deteccion.activa;
                    let t = if activa { "Apagar la detección" } else { "Encender la detección" };
                    if ui.button(t).clicked() {
                        self.cfg.deteccion.activa = !activa;
                        ui.close();
                    }
                    if ui.button("⚙ Configurar…").clicked() {
                        self.abrir_deteccion();
                        ui.close();
                    }
                    if let Some(crate::deteccion::Vista { estado: Estado::Error(e), .. }) =
                        self.detector.as_ref().map(|d| d.vista())
                    {
                        ui.separator();
                        ui.label(RichText::new(e).size(11.5).color(AIRE));
                    }
                });
                ui.separator();
            }
            if e.iptv.estado != nucleo::EstadoIptv::Apagada {
                self.chip_iptv(ui, e);
                ui.separator();
            }
            if e.osc_activo {
                ui.label(RichText::new("⏱ Tiempo exacto (OSC)").size(12.0).color(TV));
            } else {
                ui.label(pequeño("⏱ Tiempo estimado".into())).on_hover_text(
                    "CasparCG no está enviando OSC a esta PC; el fin de cada spot se calcula con su duración.",
                );
            }
            // ffmpeg.
            ui.separator();
            self.estado_ffmpeg(ui);
            ui.separator();
            self.estado_relevo(ui, e);
            // Respaldo.
            ui.separator();
            let r = &e.respaldo;
            let (t, color) = if r.haciendo {
                ("💾 Respaldando…".to_string(), TENUE)
            } else if r.error.is_some() {
                ("💾 Respaldo falló".to_string(), AIRE)
            } else {
                match &r.ultimo {
                    Some(n) => (format!("💾 Respaldo: {}", fecha_respaldo(n)), TENUE),
                    None => ("💾 Sin respaldo".to_string(), AMBAR),
                }
            };
            let boton = ui.add(egui::Button::new(RichText::new(t).size(12.0).color(color)).frame(false));
            let ayuda = match &r.error {
                Some(err) => format!("Error: {err}\nClic: opciones de respaldo"),
                None => format!("Respaldo diario automático en {}\nClic: opciones", r.carpeta),
            };
            let boton = boton.on_hover_text(ayuda);
            egui::Popup::menu(&boton).show(|ui| {
                if ui.button("💾 Respaldar ahora").clicked() {
                    self.ctl.enviar(Orden::Respaldar);
                    ui.close();
                }
                if !self.ctl.es_remoto() && ui.button("🗁 Abrir la carpeta de respaldos").clicked() {
                    let c = PathBuf::from(&r.carpeta);
                    let _ = std::fs::create_dir_all(&c);
                    crate::exportar::abrir(&c);
                    ui.close();
                }
                ui.label(
                    RichText::new("Para restaurar: cierra Relevo y descomprime el respaldo en su carpeta.")
                        .size(11.0)
                        .color(MUY_TENUE),
                );
            });
            if let Some(p) = self.puerto_api {
                ui.separator();
                ui.label(pequeño(format!("API: puerto {p}"))).on_hover_text(format!(
                    "Botoneras y otros programas: abre http://<IP de esta PC>:{p}/api para ver las acciones"
                ));
            }
            if let Some(p) = self.puerto_red {
                ui.separator();
                let n = e.sesiones.len();
                let t =
                    if n == 0 { format!("Red: puerto {p}") } else { format!("Red: puerto {p} · {n} PC conectada(s)") };
                let nombres: Vec<String> = e.sesiones.iter().map(|(_, n)| n.clone()).collect();
                ui.label(pequeño(t)).on_hover_text(if nombres.is_empty() {
                    "Ninguna PC remota conectada".into()
                } else {
                    nombres.join(", ")
                });
            }
            let (reinicio, carpeta) = match &self.arranque {
                Some(a) => {
                    let a = a.lock().expect("arranque");
                    (a.reinicio_pendiente, a.carpeta.clone())
                }
                None => (false, None),
            };
            if let (true, Some(carpeta)) = (reinicio, carpeta) {
                ui.separator();
                let puede = e.fase == FaseVista::Tv;
                let b = egui::Button::new(
                    RichText::new("⟳ Reiniciar CasparCG para aplicar los cambios").size(12.0).color(AMBAR),
                )
                .small();
                let r = ui
                    .add_enabled(puede, b)
                    .on_disabled_hover_text("Espera a que termine la tanda (solo se reinicia en señal de TV)");
                if r.clicked() {
                    match arranque::reiniciar_caspar(&carpeta, &self.cfg.casparcg.como_abrir(&self.carpeta_datos_local))
                    {
                        Ok(()) => {
                            if let Some(a) = &self.arranque {
                                a.lock().expect("arranque").reinicio_pendiente = false;
                            }
                            self.avisar_local("CasparCG se está reiniciando…");
                        }
                        Err(err) => self.avisar_error(format!("No se pudo reiniciar CasparCG: {err}")),
                    }
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Los atajos solo si hay sitio (no se encima con lo de la izquierda).
                let at = &self.cfg.atajos;
                let atajos = format!(
                    "{}: play/pausa · {}: siguiente · {}: añadir · {}: quitar",
                    crate::atajos::bonito(&at.play_pausa),
                    crate::atajos::bonito(&at.siguiente),
                    crate::atajos::bonito(&at.anadir),
                    crate::atajos::bonito(&at.quitar)
                );
                if ui.available_width() > 470.0 {
                    ui.label(pequeño(atajos));
                } else if ui.available_width() > 40.0 {
                    ui.label(pequeño("⌨".into())).on_hover_text(atajos);
                }
            });
        });
    }
}

// ======================================================================
// Barra de AL AIRE (progreso + transporte)
// ======================================================================

impl Ventana {
    /// Monitor con la imagen de lo que está al aire.
    fn ventana_ajustes(&mut self, ctx: &egui::Context, e: &Instantanea) {
        match self.ajustes.mostrar(ctx, e.fase != FaseVista::Tv) {
            ajustes::Resultado::Nada => {}
            ajustes::Resultado::Guardado { cfg, reiniciar_app, salida_cambiada } => {
                let t_vieja = self.cfg.transiciones.transicion();
                let iptv_vieja = self.cfg.iptv_para_nucleo();
                self.cfg = *cfg;
                crate::tema::poner_hora_12(self.cfg.interfaz.hora_12);
                let t_nueva = self.cfg.transiciones.transicion();
                if t_nueva != t_vieja {
                    self.ctl.enviar(Orden::Transicion(t_nueva));
                }
                let iptv_nueva = self.cfg.iptv_para_nucleo();
                if iptv_nueva != iptv_vieja && !self.ctl.es_remoto() {
                    self.ctl.enviar(Orden::Iptv(iptv_nueva));
                }
                dialogo::poner_animaciones(ctx, self.cfg.interfaz.animaciones);
                if salida_cambiada {
                    self.aplicar_salida(e.conectado);
                }
                if !reiniciar_app {
                    self.avisar_local("Ajustes guardados");
                }
            }
            ajustes::Resultado::ReiniciarApp => {
                self.cierre = Some(false);
                reiniciar_app(ctx)
            }
        }
    }

    /// Escribe la salida de pantalla elegida en casparcg.config (en segundo plano:
    /// leer los monitores tarda un poco). Si CasparCG está abierto, pide reiniciarlo.
    fn aplicar_salida(&mut self, conectado: bool) {
        let Some(estado) = self.arranque.clone() else { return };
        let Some(carpeta) = estado.lock().expect("arranque").carpeta.clone() else {
            self.avisar_error("No se encontró la carpeta de CasparCG: indícala en Ajustes → CasparCG");
            return;
        };
        let modo = self.cfg.casparcg.pantalla.clone();
        let canal = self.cfg.casparcg.canal;
        std::thread::spawn(move || {
            let salida = arranque::decidir_salida(&modo, &arranque::monitores());
            match arranque::aplicar_pantalla(&carpeta, canal, &salida) {
                Ok(true) => {
                    let mut a = estado.lock().expect("arranque");
                    if conectado {
                        a.reinicio_pendiente = true;
                    }
                    a.mensajes.push(format!("Salida de CasparCG: {}", arranque::describir_salida(&salida)));
                }
                Ok(false) => {}
                Err(e) => estado.lock().expect("arranque").mensajes.push(format!("Salida de pantalla: {e}")),
            }
        });
    }

    fn monitor_programa(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let alto = self.alto_monitor();
        self.monitor_programa_de(ui, e, alto);
    }

    /// Las tres columnas de Emisión (archivos, tanda, pauta), con separaciones que
    /// se arrastran para cambiar el ancho.
    fn columnas_emision(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let total = ui.available_rect_before_wrap();
        let [a, t, p] = self.diseno.columnas(total.width());
        let g = diseno::GROSOR;
        let r_arch = egui::Rect::from_min_size(total.min, Vec2::new(a, total.height()));
        let r_div1 = egui::Rect::from_min_size(egui::pos2(r_arch.right(), total.top()), Vec2::new(g, total.height()));
        let r_tanda = egui::Rect::from_min_size(egui::pos2(r_div1.right(), total.top()), Vec2::new(t, total.height()));
        let r_div2 = egui::Rect::from_min_size(egui::pos2(r_tanda.right(), total.top()), Vec2::new(g, total.height()));
        let r_pauta = egui::Rect::from_min_size(egui::pos2(r_div2.right(), total.top()), Vec2::new(p, total.height()));

        let columna = |ui: &mut egui::Ui, rect: egui::Rect, id: &str, f: &mut dyn FnMut(&mut egui::Ui)| {
            ui.scope_builder(
                egui::UiBuilder::new().id_salt(id).max_rect(rect).layout(Layout::top_down(Align::Min)),
                |ui| {
                    ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                    tema::marco_columna().show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        f(ui);
                    });
                },
            );
        };
        columna(ui, r_arch, "col_archivos", &mut |ui| self.columna_archivos(ui, e));
        columna(ui, r_tanda, "col_tanda", &mut |ui| self.columna_tanda(ui, e));
        columna(ui, r_pauta, "col_pauta", &mut |ui| self.columna_pauta(ui, e));

        let util = (total.width() - 2.0 * g).max(1.0);
        let r = diseno::divisor(ui, "col1", r_div1, true);
        if r.dragged() {
            let nuevo = (a + r.drag_delta().x) / util;
            self.diseno.m.archivos = nuevo.clamp(0.1, 0.8);
        }
        let r2 = diseno::divisor(ui, "col2", r_div2, true);
        if r2.dragged() {
            let nuevo = (p - r2.drag_delta().x) / util;
            self.diseno.m.pauta = nuevo.clamp(0.1, 0.6);
        }
        let normal = diseno::Medidas::default();
        if r.double_clicked() {
            self.diseno.m.archivos = normal.archivos;
        }
        if r2.double_clicked() {
            self.diseno.m.pauta = normal.pauta;
        }
        if r.drag_stopped() || r2.drag_stopped() || r.double_clicked() || r2.double_clicked() {
            // Se guarda lo que de verdad se ve (con los mínimos aplicados).
            let [a, _, p] = self.diseno.columnas(total.width());
            self.diseno.m.archivos = a / util;
            self.diseno.m.pauta = p / util;
            self.diseno.guardar();
        }
        ui.allocate_rect(total, Sense::hover());
    }

    fn monitor_programa_de(&mut self, ui: &mut egui::Ui, e: &Instantanea, alto: f32) {
        let tam = Vec2::new(alto * 16.0 / 9.0, alto);
        let (rect, r) = ui.allocate_exact_size(tam, Sense::hover());
        let actual = e.actual.and_then(|id| e.item(id)).cloned();
        // Archivo del spot al aire (se busca una vez por spot).
        let archivo = match &actual {
            Some(item) => {
                if self.monitor_clip.as_ref().map(|(c, _)| c) != Some(&item.clip) {
                    let ruta = self.ruta_de_clip(&item.clip, e);
                    self.monitor_clip = Some((item.clip.clone(), ruta));
                }
                self.monitor_clip.as_ref().and_then(|(_, r)| r.clone())
            }
            None => {
                self.monitor_clip = None;
                None
            }
        };
        let segundo = e.transcurrido.as_secs_f64();
        self.monitor.seguir(ui.ctx(), archivo.as_deref().map(|a| (a, segundo)), e.fase == FaseVista::Pausado);

        // En modo Transmisión, con el canal al aire, se ve el canal en vivo.
        let canal_en_vivo = actual.is_none() && self.programa_en_vivo(ui, rect);
        let p = ui.painter();
        if !canal_en_vivo {
            p.rect_filled(rect, 6.0, Color32::BLACK);
        }
        match (&actual, self.monitor.textura()) {
            (None, _) if canal_en_vivo => {}
            (None, _) => {
                p.rect_filled(rect, 6.0, TV_OSCURO);
                p.text(rect.center(), Align2::CENTER_CENTER, "📺  Señal de TV", FontId::proportional(16.0), TV);
            }
            (Some(_), Some(t)) => {
                egui::Image::new(t).corner_radius(6).fit_to_exact_size(tam).paint_at(ui, rect);
            }
            (Some(item), None) => {
                // Sin el archivo en esta PC (o sin ffmpeg): la miniatura del spot.
                let clip = item.clip.clone();
                self.pintar_miniatura(ui, rect, Some(&clip), &clip, false);
            }
        }
        let p = ui.painter();
        // Etiquetas.
        let (texto, color) = match e.fase {
            FaseVista::AlAire => ("⏺ AL AIRE", AIRE),
            FaseVista::Pausado => ("⏸ PAUSA", AMBAR),
            // En Transmisión hay dos monitores (PROGRAMA y PREVIEW); en Publicidad, uno
            // solo que muestra el canal.
            FaseVista::Tv if self.cfg.en_transmision() => ("PROGRAMA", TENUE),
            FaseVista::Tv => ("📺 CANAL", TV),
        };
        let et = egui::Rect::from_min_size(rect.left_top() + Vec2::new(6.0, 6.0), Vec2::new(78.0, 18.0));
        p.rect_filled(et, 4.0, Color32::from_black_alpha(170));
        p.text(et.center(), Align2::CENTER_CENTER, texto, FontId::proportional(11.0), color);
        let borde = if e.fase == FaseVista::AlAire { AIRE } else { BORDE };
        p.rect_stroke(rect, 6.0, Stroke::new(1.0, borde), StrokeKind::Inside);
        if !self.cfg.en_transmision() {
            // Modo Publicidad: aquí se elige qué señal del canal se ve.
            self.boton_senal_canal(ui, rect);
        }
        if actual.is_some() && archivo.is_none() {
            r.on_hover_text(if self.ctl.es_remoto() {
                "La imagen en vivo solo se ve en la PC principal (aquí, la miniatura del spot)"
            } else {
                "No se encontró el archivo del spot en esta PC"
            });
        } else if actual.is_some() && !self.monitor.hay_ffmpeg() {
            r.on_hover_text("Para ver la imagen en vivo hace falta ffmpeg");
        } else if actual.is_some() {
            r.on_hover_text("Imagen del spot al aire, al mismo tiempo que CasparCG (sin sonido)");
        }
    }
}

// ======================================================================
// Columna TANDA (centro)
// ======================================================================

impl Ventana {
    /// Duración del corte (tope) y relleno, en modo AUTO.
    fn duracion_corte(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let c = &e.corte;
        let puede = self.ctl.tengo_mando(e);
        ui.add_enabled_ui(puede, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Duración del corte").size(12.5));
                let texto = match c.tope {
                    Some(s) => format!("{}:{:02}", s / 60, s % 60),
                    None => "Sin tope".into(),
                };
                let mut nuevo: Option<Option<u32>> = None;
                egui::ComboBox::from_id_salt("tope_corte").width(90.0).selected_text(texto).show_ui(ui, |ui| {
                    for (t, v) in [
                        ("Sin tope", None),
                        ("0:30", Some(30)),
                        ("1:00", Some(60)),
                        ("1:30", Some(90)),
                        ("2:00", Some(120)),
                        ("2:30", Some(150)),
                        ("3:00", Some(180)),
                        ("4:00", Some(240)),
                        ("5:00", Some(300)),
                    ] {
                        if ui.selectable_label(c.tope == v, t).clicked() {
                            nuevo = Some(v);
                        }
                    }
                });
                if let Some(s) = c.tope {
                    // Ajuste fino de 5 en 5 segundos.
                    if ui.small_button("−").on_hover_text("5 s menos").clicked() && s > 5 {
                        nuevo = Some(Some(s - 5));
                    }
                    if ui.small_button("+").on_hover_text("5 s más").clicked() {
                        nuevo = Some(Some(s + 5));
                    }
                }
                let hay_relleno = !c.clips_relleno.is_empty();
                let mut relleno = c.relleno;
                let r = ui
                    .add_enabled(c.tope.is_some() && hay_relleno, egui::Checkbox::new(&mut relleno, "Rellenar"))
                    .on_hover_text(if hay_relleno {
                        format!(
                            "Completa el tiempo que sobra con: {}",
                            c.clips_relleno.iter().map(|x| nombre_corto(x)).collect::<Vec<_>>().join(", ")
                        )
                    } else {
                        "No hay clips de relleno: indica sus carpetas en config.toml ([pauta] relleno)".to_string()
                    })
                    .on_disabled_hover_text(if hay_relleno {
                        "Elige primero una duración de corte"
                    } else {
                        "No hay clips de relleno: indica sus carpetas en config.toml ([pauta] relleno)"
                    });
                if r.changed() {
                    self.ctl.enviar(Orden::Corte { tope: c.tope, relleno, bucle: c.bucle });
                }
                if let Some(t) = nuevo {
                    self.ctl.enviar(Orden::Corte { tope: t, relleno: c.relleno, bucle: c.bucle });
                }
                let mut bucle = c.bucle;
                if ui
                    .checkbox(&mut bucle, "Repetir")
                    .on_hover_text(
                        "Si la tanda (con el relleno) termina y el corte sigue, vuelve a empezar hasta que \
                         vuelvas a TV. Las repeticiones no cuentan como pases del cliente.",
                    )
                    .changed()
                {
                    self.ctl.enviar(Orden::Corte { tope: c.tope, relleno: c.relleno, bucle });
                }
            });
        });
        if !c.fuera.is_empty() {
            ui.label(
                RichText::new(format!("⚠ No entran en este corte (quedan para el siguiente): {}", c.fuera.join(", ")))
                    .size(12.0)
                    .color(AMBAR),
            );
        }
    }

    fn columna_tanda(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        self.cabecera_tanda(ui, e);
        ui.add_space(4.0);

        if e.modo == ModoTanda::Auto {
            let vigentes = e.pases_hoy.iter().filter(|p| p.estado == EstadoPase::Vigente).count();
            tema::marco_tarjeta().fill(ACENTO_OSCURO).stroke(Stroke::new(1.0, ACENTO)).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⟳").size(18.0).color(ACENTO));
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Modo AUTO: la tanda se arma sola con la pauta")
                                .strong()
                                .family(crate::tema::negrita()),
                        );
                        let pendientes = e.pases_hoy.iter().filter(|p| p.estado == EstadoPase::Pendiente).count();
                        let t = if e.fase == FaseVista::Tv && pendientes > 0 {
                            format!(
                                "{vigentes} pases en ventana y {pendientes} pendientes. Pulsa Play (Espacio) cuando empiece el corte."
                            )
                        } else if e.fase == FaseVista::Tv {
                            format!("{vigentes} pases en ventana ahora. Pulsa Play (Espacio) cuando empiece el corte.")
                        } else {
                            "Al terminar esta tanda se preparará la siguiente.".to_string()
                        };
                        ui.label(RichText::new(t).size(12.0).color(TENUE));
                    });
                });
                ui.add_space(2.0);
                self.duracion_corte(ui, e);
            });
            ui.add_space(4.0);
        }

        // Copias en curso hacia la carpeta de CasparCG.
        for imp in &e.importaciones {
            tema::marco_tarjeta().fill(AMBAR_OSCURO).stroke(Stroke::new(1.0, AMBAR)).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(AMBAR));
                    ui.add(
                        egui::Label::new(RichText::new(format!("Copiando «{}» a CasparCG", imp.nombre)).size(13.0))
                            .truncate(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{:.0}%", imp.progreso * 100.0))
                                .font(FontId::monospace(13.0))
                                .color(AMBAR),
                        );
                    });
                });
                ui.add(egui::ProgressBar::new(imp.progreso).desired_height(5.0).fill(AMBAR));
            });
            ui.add_space(4.0);
        }

        let alto_pie = 64.0;
        let alto_lista = (ui.available_height() - alto_pie).max(80.0);

        let bloqueado = !self.ctl.tengo_mando(e);
        if bloqueado {
            if let Some((_, quien)) = &e.mando {
                ui.label(
                    RichText::new(format!("🎮 La tanda la maneja {quien}. Pide el mando arriba para operar."))
                        .size(12.0)
                        .color(AMBAR),
                );
            }
        }
        ui.add_enabled_ui(!bloqueado, |ui| {
        let mut filas: Vec<egui::Rect> = Vec::new();
        let arrastre = diseno::arrastrando(ui.ctx());
        let mut fondo: Option<egui::Response> = None;
        let salida = egui::ScrollArea::vertical()
            .id_salt("tanda")
            .auto_shrink([false, false])
            .scroll_source(egui::scroll_area::ScrollSource { drag: egui::scroll_area::DragScroll::Never, ..egui::scroll_area::ScrollSource::ALL })
            .max_height(alto_lista)
            .show(ui, |ui| {
                // El fondo de la lista (lo que no es una fila): arrastrar ahí dibuja un
                // marco para elegir varios; clic ahí deja de elegir.
                let vista = vista_de_lista(ui);
                fondo = Some(ui.interact(vista, egui::Id::unique("fondo_tanda"), Sense::click_and_drag()));
                self.tanda_arriba = ui.min_rect().top();
                // Mientras se elige con el marco cerca del borde, la lista se desplaza sola.
                if self.marco_tanda.is_some() {
                    if let Some(pos) = ui.ctx().pointer_hover_pos() {
                        if pos.y < vista.top() + 30.0 {
                            ui.scroll_with_delta(Vec2::new(0.0, 8.0));
                        } else if pos.y > vista.bottom() - 30.0 {
                            ui.scroll_with_delta(Vec2::new(0.0, -8.0));
                        }
                    }
                }
                // Mientras se arrastra algo cerca del borde, la lista se desplaza sola.
                if arrastre.is_some() {
                    if let Some(pos) = ui.ctx().pointer_hover_pos() {
                        let vis = ui.clip_rect();
                        if vis.x_range().contains(pos.x) {
                            if pos.y < vis.top() + 36.0 && pos.y > vis.top() - 20.0 {
                                ui.scroll_with_delta(Vec2::new(0.0, 8.0));
                            } else if pos.y > vis.bottom() - 36.0 && pos.y < vis.bottom() + 20.0 {
                                ui.scroll_with_delta(Vec2::new(0.0, -8.0));
                            }
                        }
                    }
                }
                if e.lista.is_empty() && e.importaciones.is_empty() && e.modo == ModoTanda::Auto {
                    let proximo = e.pases_hoy.iter().find(|p| p.estado == EstadoPase::Programado);
                    let detalle = match proximo {
                        Some(p) => format!("Próximo pase: {} · {}", crate::tema::hora_txt(&p.hora), p.nombre),
                        None if e.pautas.is_empty() => "Todavía no hay pautas: regístralas en la pestaña Publicidades (F5).".into(),
                        None => "No quedan pases para hoy.".into(),
                    };
                    mensaje_vacio(ui, "🕓", "Ningún spot en ventana ahora", &detalle);
                    return;
                }
                if e.lista.is_empty() && e.importaciones.is_empty() {
                    mensaje_vacio(
                        ui,
                        "☰",
                        "La tanda está vacía",
                        "Arrastra aquí un video de la izquierda (o doble clic / Enter),\no arrástralos desde el Explorador de Windows.",
                    );
                    return;
                }
                for (i, item) in e.lista.iter().enumerate() {
                    filas.push(self.fila_tanda(ui, e, i, item));
                }
                if self.tanda_llegada.len() > e.lista.len() + 50 {
                    self.tanda_llegada.retain(|id, _| e.lista.iter().any(|i| i.id == *id));
                }
            });
        self.tanda_cargada = true;
        if !bloqueado {
            if let Some(f) = &fondo {
                self.elegir_con_marco(ui, e, f, &filas);
            }
            self.soltar_en_tanda(ui, e, salida.inner_rect, &filas);
            self.teclas_tanda(ui, e, salida.inner_rect);
        }
        self.tanda_rects = e.lista.iter().map(|i| i.id).zip(filas.iter().copied()).collect();

        // Pie: el resumen y, con spots elegidos, sus acciones (mismo lugar, mismo alto).
        ui.add_space(4.0);
        ui.separator();
        self.pie_tanda(ui, e);
        });
    }

    /// Lo que se suelta sobre la lista de la tanda: videos de la izquierda (se añaden
    /// donde cae) o un spot de la misma tanda (cambia de lugar).
    fn soltar_en_tanda(&mut self, ui: &mut egui::Ui, e: &Instantanea, zona: egui::Rect, filas: &[egui::Rect]) {
        let ctx = ui.ctx().clone();
        let Some(arrastre) = diseno::arrastrando(&ctx) else { return };
        let Some(pos) = ctx.pointer_latest_pos() else { return };
        if !zona.contains(pos) {
            return;
        }
        let k = diseno::indice_de_insercion(filas, pos.y);
        // Si es un spot y cae en su mismo lugar, no se marca nada.
        let origen = match &*arrastre {
            Arrastre::Spot { id, .. } => e.lista.iter().position(|i| i.id == *id),
            _ => None,
        };
        let sin_cambio = origen.is_some_and(|o| diseno::delta_mover(o, k).is_none());

        if !sin_cambio {
            let y = if filas.is_empty() {
                zona.top() + 4.0
            } else if k < filas.len() {
                filas[k].top() - 2.0
            } else {
                filas[filas.len() - 1].bottom() + 2.0
            };
            let y = y.clamp(zona.top() + 2.0, zona.bottom() - 2.0);
            let capa = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("linea_soltar"));
            let p = ctx.layer_painter(capa).with_clip_rect(zona.expand(4.0));
            p.line_segment(
                [egui::pos2(zona.left() + 8.0, y), egui::pos2(zona.right() - 8.0, y)],
                Stroke::new(3.0, ACENTO),
            );
            p.circle_filled(egui::pos2(zona.left() + 8.0, y), 5.0, ACENTO);
        }

        if !ctx.input(|i| i.pointer.any_released()) {
            return;
        }
        let Some(arrastre) = egui::DragAndDrop::take_payload::<Arrastre>(&ctx) else { return };
        let destino = if k >= e.lista.len() { Destino::Final } else { Destino::Posicion(k) };
        match &*arrastre {
            Arrastre::Archivo { ruta, duracion, .. } => self.agregar_archivo(ruta, *duracion, destino),
            Arrastre::Archivos { lista, .. } => {
                // En su orden, uno detrás del otro desde donde se soltaron.
                for (j, (ruta, duracion)) in lista.iter().enumerate() {
                    let d = match destino {
                        Destino::Posicion(k) => Destino::Posicion(k + j),
                        otro => otro,
                    };
                    self.agregar_archivo(ruta, *duracion, d);
                }
                self.archivos_multi.clear();
            }
            Arrastre::Clip { clip, duracion } => {
                self.ctl.enviar(Orden::Agregar { clip: clip.clone(), duracion: *duracion, destino })
            }
            Arrastre::Spot { id, .. } => {
                let elegidos = self.elegidos(e);
                if elegidos.len() > 1 && elegidos.contains(id) {
                    // Varios elegidos: se mueven todos juntos.
                    self.ctl.enviar(Orden::MoverA(elegidos, k));
                } else {
                    if let Some(d) = origen.and_then(|o| diseno::delta_mover(o, k)) {
                        self.ctl.enviar(Orden::Mover(*id, d));
                    }
                    self.tanda_sel = Some(*id);
                    self.tanda_multi.clear();
                }
            }
        }
    }

    /// Con la señal de TV: el spot que sale al dar Play (donde se detuvo la tanda, el
    /// seleccionado o el primero).
    pub(super) fn en_cola(&self, e: &Instantanea) -> Option<u64> {
        if e.fase != FaseVista::Tv {
            return None;
        }
        e.retomar.or(self.tanda_sel.filter(|s| e.item(*s).is_some())).or(e.lista.first().map(|i| i.id))
    }

    /// Dibuja un spot de la tanda y devuelve su rectángulo (para soltar cosas entre filas).
    fn fila_tanda(&mut self, ui: &mut egui::Ui, e: &Instantanea, i: usize, item: &nucleo::ItemTanda) -> egui::Rect {
        let es_actual = e.actual == Some(item.id) && e.fase != FaseVista::Tv;
        let lista = self.ultima_carpeta.tanda_lista;
        // Con la tanda detenida a mitad: dónde quedó (Play lo reinicia) y cuál sigue.
        let quedo = e.fase == FaseVista::Tv && e.retomar == Some(item.id);
        let pos_quedo = e.retomar.and_then(|r| e.lista.iter().position(|x| x.id == r));
        let es_proximo =
            e.proximo == Some(item.id) || (e.fase == FaseVista::Tv && pos_quedo.is_some_and(|p| p + 1 == i));
        let seleccionado = self.esta_elegido(item.id);

        let (fondo, borde) = if es_actual && seleccionado {
            (AIRE_OSCURO, ACENTO)
        } else if es_actual {
            (AIRE_OSCURO, AIRE)
        } else if quedo {
            (AMBAR_OSCURO, AMBAR)
        } else if seleccionado {
            (ACENTO_OSCURO, ACENTO)
        } else if es_proximo {
            (TARJETA, Color32::from_rgb(0x3A, 0x55, 0x80))
        } else {
            (TARJETA, BORDE)
        };

        // La fila entera se puede agarrar y arrastrar. Se registra *debajo* de sus
        // botones (UiBuilder::sense) para que los botones reciban sus clics.
        // Un spot recién llegado se resalta un momento.
        let ahora = ui.ctx().input(|i| i.time);
        let llegada = *self.tanda_llegada.entry(item.id).or_insert(if self.tanda_cargada { ahora } else { -10.0 });
        let resalte = 1.0 - dialogo::desde(ui.ctx(), llegada, 1.1);
        let respuesta = ui
            .scope_builder(egui::UiBuilder::new().sense(Sense::click_and_drag()), |ui| {
                egui::Frame::new()
                    .fill(fondo)
                    .stroke(Stroke::new(1.0, borde))
                    .corner_radius(CornerRadius::same(if lista { 5 } else { 7 }))
                    .inner_margin(if lista { egui::Margin::symmetric(8, 3) } else { egui::Margin::symmetric(8, 6) })
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let tam_num = if lista { 13.0 } else { 15.0 };
                            ui.label(
                                RichText::new(format!("{:>2}", i + 1)).font(FontId::monospace(tam_num)).color(TENUE),
                            );
                            if !lista {
                                let (rect, _) = ui.allocate_exact_size(Vec2::new(80.0, 45.0), Sense::hover());
                                self.pintar_miniatura(ui, rect, Some(&item.clip), &item.clip, false);
                                if es_actual {
                                    ui.painter().rect_stroke(rect, 5.0, Stroke::new(2.0, AIRE), StrokeKind::Inside);
                                }
                            }

                            let (carpeta, nombre) = partir(&item.clip);
                            // Deja sitio a la derecha para la duración y los botones.
                            let ancho_nombre = (ui.available_width() - 110.0).max(60.0);
                            if lista {
                                // Una línea: [estado] NOMBRE  carpeta · pase.
                                ui.allocate_ui_with_layout(
                                    Vec2::new(ancho_nombre, 20.0),
                                    Layout::left_to_right(Align::Center),
                                    |ui| {
                                        ui.set_width(ancho_nombre);
                                        if es_actual {
                                            pastilla(ui, "AL AIRE", AIRE, Color32::WHITE, 9.5);
                                        } else if quedo {
                                            pastilla(ui, "QUEDÓ AQUÍ", AMBAR, Color32::BLACK, 9.5);
                                        } else if es_proximo {
                                            pastilla(ui, "SIGUE", ACENTO_OSCURO, ACENTO, 9.5);
                                        }
                                        let pase =
                                            item.pase.as_ref().and_then(|c| e.pases_hoy.iter().find(|p| &p.clave == c));
                                        let mut texto = egui::text::LayoutJob::default();
                                        texto.append(
                                            nombre,
                                            0.0,
                                            egui::TextFormat::simple(
                                                FontId::new(14.0, crate::tema::negrita()),
                                                if es_actual { Color32::WHITE } else { TEXTO },
                                            ),
                                        );
                                        let mut sub = carpeta.trim_end_matches('/').to_string();
                                        if let Some(p) = pase {
                                            sub = format!(
                                                "🕓 {} {}  {sub}",
                                                crate::tema::hora_txt(&p.hora),
                                                "★".repeat(p.prioridad as usize)
                                            );
                                        }
                                        if !sub.trim().is_empty() {
                                            texto.append(
                                                &sub,
                                                8.0,
                                                egui::TextFormat::simple(FontId::proportional(11.5), TENUE),
                                            );
                                        }
                                        texto.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
                                        ui.add(
                                            egui::Label::new(texto).truncate().selectable(false).sense(Sense::hover()),
                                        );
                                    },
                                );
                            } else {
                                ui.vertical(|ui| {
                                    ui.set_width(ancho_nombre);
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.horizontal(|ui| {
                                        if es_actual {
                                            pastilla(ui, "AL AIRE", AIRE, Color32::WHITE, 10.0);
                                        } else if quedo {
                                            pastilla(ui, "QUEDÓ AQUÍ", AMBAR, Color32::BLACK, 10.0);
                                        } else if es_proximo {
                                            pastilla(ui, "SIGUE", ACENTO_OSCURO, ACENTO, 10.0);
                                        }
                                        // Los textos no se quedan el clic: así elige la fila.
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(nombre)
                                                    .size(16.0)
                                                    .strong()
                                                    .family(crate::tema::negrita()),
                                            )
                                            .truncate()
                                            .selectable(false)
                                            .sense(Sense::hover()),
                                        );
                                    });
                                    // Debajo: carpeta y, si viene de la pauta, su hora.
                                    let pase =
                                        item.pase.as_ref().and_then(|c| e.pases_hoy.iter().find(|p| &p.clave == c));
                                    let mut sub = carpeta.trim_end_matches('/').to_string();
                                    if let Some(p) = pase {
                                        let estrellas = "★".repeat(p.prioridad as usize);
                                        sub = format!(
                                            "🕓 pase {} {estrellas}  ·  {} ",
                                            crate::tema::hora_txt(&p.hora),
                                            sub
                                        );
                                    }
                                    if !sub.trim().is_empty() {
                                        let color = if pase.is_some_and(|p| p.prioridad > 0) { AMBAR } else { TENUE };
                                        ui.add(
                                            egui::Label::new(RichText::new(sub).size(12.0).color(color))
                                                .truncate()
                                                .selectable(false)
                                                .sense(Sense::hover()),
                                        );
                                    }
                                });
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                // Quitar: solo al pasar el mouse (las demás acciones están
                                // abajo y en el clic derecho).
                                let encima = ui
                                    .ctx()
                                    .pointer_hover_pos()
                                    .is_some_and(|p| self.tanda_rects.get(&item.id).is_some_and(|r| r.contains(p)));
                                if encima && !es_actual {
                                    if ui.add(boton_chico("✖")).on_hover_text("Quitar de la tanda (Supr)").clicked() {
                                        self.ctl.enviar(Orden::Quitar(item.id));
                                    }
                                } else {
                                    ui.add_space(24.0);
                                }
                                let texto = if item.duracion.is_zero() {
                                    "--:--".to_string()
                                } else {
                                    mmss(item.duracion.as_secs_f64())
                                };
                                ui.label(RichText::new(texto).font(FontId::monospace(if lista { 13.5 } else { 15.0 })));
                            });
                        });

                        // Mini barra de progreso en el spot al aire.
                        if es_actual && !item.duracion.is_zero() {
                            let frac = (e.transcurrido.as_secs_f32() / item.duracion.as_secs_f32()).clamp(0.0, 1.0);
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), Sense::hover());
                            ui.painter().rect_filled(rect, 2.0, Color32::from_black_alpha(90));
                            let mut lleno = rect;
                            lleno.set_width(rect.width() * frac);
                            ui.painter().rect_filled(lleno, 2.0, AIRE);
                        }
                    });
            })
            .response;

        let rect = respuesta.rect;
        // El número es un asa para elegir: arrastrar sobre los números elige varios
        // (como los encabezados de fila de una hoja de cálculo).
        let asa = egui::Rect::from_min_size(rect.min, Vec2::new(30.0, rect.height()));
        let r_asa = ui.interact(asa, egui::Id::unique(("asa_tanda", item.id)), Sense::click_and_drag());
        if r_asa.hovered() && self.marco_tanda.is_none() {
            ui.painter().rect_filled(asa.shrink(2.0), 5.0, Color32::from_white_alpha(10));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Cell);
        }
        let r_asa = r_asa.on_hover_text("Clic: elegir · Arrastrar sobre los números: elegir varios");
        if r_asa.drag_started() {
            self.empezar_marco(ui, e);
        }
        if r_asa.clicked() {
            let mods = ui.input(|i| i.modifiers);
            self.clic_en_spot(e, item.id, mods);
        }
        if resalte > 0.0 {
            ui.painter().rect_filled(rect, 7.0, ACENTO.gamma_multiply(0.35 * resalte));
            ui.painter().rect_stroke(rect, 7.0, Stroke::new(2.0, ACENTO.gamma_multiply(resalte)), StrokeKind::Inside);
        }
        if respuesta.drag_started() {
            // Si se arrastra uno de varios elegidos, van todos.
            let varios = self.tanda_multi.len() > 1 && self.tanda_multi.contains(&item.id);
            let nombre = if varios {
                format!("{} spots", self.elegidos(e).len())
            } else {
                self.tanda_multi.clear();
                self.tanda_sel = Some(item.id);
                partir(&item.clip).1.to_string()
            };
            egui::DragAndDrop::set_payload(ui.ctx(), Arrastre::Spot { id: item.id, nombre });
        }
        // Lo que se está arrastrando se ve apagado en su lugar.
        if let Some(a) = diseno::arrastrando(ui.ctx()) {
            if let Arrastre::Spot { id, .. } = &*a {
                let varios = self.tanda_multi.len() > 1 && self.tanda_multi.contains(id);
                if *id == item.id || (varios && self.tanda_multi.contains(&item.id)) {
                    ui.painter().rect_filled(rect, 7.0, Color32::from_black_alpha(140));
                }
            }
        }
        if respuesta.clicked() {
            let mods = ui.input(|i| i.modifiers);
            self.clic_en_spot(e, item.id, mods);
        }
        if respuesta.double_clicked() {
            self.ctl.enviar(Orden::Reproducir(item.id));
        }
        // Clic derecho sobre uno que no estaba elegido: queda elegido solo él.
        if respuesta.secondary_clicked() && !self.esta_elegido(item.id) {
            self.tanda_multi.clear();
            self.tanda_sel = Some(item.id);
        }
        respuesta.context_menu(|ui| self.menu_spot(ui, e, item));
        ui.add_space(if lista { 2.0 } else { 4.0 });
        rect
    }

    /// Los spots elegidos, en el orden de la tanda (uno solo si no hay varios).
    fn elegidos(&self, e: &Instantanea) -> Vec<u64> {
        if !self.tanda_multi.is_empty() {
            e.lista.iter().filter(|i| self.tanda_multi.contains(&i.id)).map(|i| i.id).collect()
        } else {
            self.tanda_sel.filter(|s| e.item(*s).is_some()).into_iter().collect()
        }
    }

    fn esta_elegido(&self, id: u64) -> bool {
        if !self.tanda_multi.is_empty() {
            self.tanda_multi.contains(&id)
        } else {
            self.tanda_sel == Some(id)
        }
    }

    /// Clic: elige solo ese. Ctrl+clic: lo suma o lo quita. Mayús+clic: desde el último
    /// elegido hasta ese.
    fn clic_en_spot(&mut self, e: &Instantanea, id: u64, mods: egui::Modifiers) {
        let pos = |x: u64| e.lista.iter().position(|i| i.id == x);
        if mods.shift {
            if let (Some(a), Some(b)) = (self.tanda_sel.and_then(pos), pos(id)) {
                let (desde, hasta) = (a.min(b), a.max(b));
                self.tanda_multi = e.lista[desde..=hasta].iter().map(|i| i.id).collect();
                return;
            }
        }
        if mods.command {
            // Ctrl+clic entra en modo «elegir» (aunque sea uno solo).
            if self.tanda_multi.is_empty() {
                self.tanda_multi = self.tanda_sel.filter(|s| e.item(*s).is_some() && *s != id).into_iter().collect();
            }
            if let Some(k) = self.tanda_multi.iter().position(|x| *x == id) {
                self.tanda_multi.remove(k);
                self.tanda_sel = self.tanda_multi.last().copied();
            } else {
                self.tanda_multi.push(id);
                self.tanda_sel = Some(id);
            }
            return;
        }
        self.tanda_multi.clear();
        self.tanda_sel = Some(id);
    }

    fn quitar_elegidos(&mut self, e: &Instantanea) {
        let ids = self.elegidos(e);
        match ids.len() {
            0 => {}
            1 => self.ctl.enviar(Orden::Quitar(ids[0])),
            _ => self.ctl.enviar(Orden::QuitarVarios(ids)),
        }
        self.tanda_multi.clear();
    }

    fn mover_elegidos(&mut self, e: &Instantanea, m: Movida) {
        let ids = self.elegidos(e);
        let posiciones: Vec<usize> =
            e.lista.iter().enumerate().filter(|(_, i)| ids.contains(&i.id)).map(|(k, _)| k).collect();
        let (Some(&primero), Some(&ultimo)) = (posiciones.first(), posiciones.last()) else { return };
        let destino = match m {
            Movida::Subir => primero.saturating_sub(1),
            Movida::Bajar => (ultimo + 2).min(e.lista.len()),
            Movida::Principio => 0,
            Movida::Final => e.lista.len(),
            Movida::Siguiente => {
                // Detrás del que está al aire (o del que quedó, que sale al dar Play).
                let ancla = if e.fase != FaseVista::Tv { e.actual } else { e.retomar };
                ancla.and_then(|a| e.lista.iter().position(|i| i.id == a)).map(|p| p + 1).unwrap_or(0)
            }
        };
        self.ctl.enviar(Orden::MoverA(ids, destino));
    }

    /// Título, MANUAL / AUTO y el menú de tandas guardadas.
    fn cabecera_tanda(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        ui.horizontal(|ui| {
            tema::titulo(ui, "TANDA");
            ui.add_space(8.0);
            // MANUAL | AUTO como un solo control.
            let auto = e.modo == ModoTanda::Auto;
            egui::Frame::new()
                .fill(Color32::from_rgb(0x0E, 0x11, 0x17))
                .stroke(Stroke::new(1.0, BORDE))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::same(2))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (texto, modo, ayuda) in [
                        ("MANUAL", ModoTanda::Manual, "Tú armas la tanda"),
                        ("AUTO", ModoTanda::Auto, "La tanda se arma sola con la pauta del día; tú solo das Play"),
                    ] {
                        let activo = (modo == ModoTanda::Auto) == auto;
                        let b = egui::Button::new(
                            RichText::new(texto).size(12.5).strong().family(crate::tema::negrita()).color(if activo {
                                TEXTO
                            } else {
                                TENUE
                            }),
                        )
                        .fill(if activo { ACENTO_OSCURO } else { Color32::TRANSPARENT })
                        .stroke(if activo { Stroke::new(1.0, ACENTO) } else { Stroke::NONE })
                        .corner_radius(CornerRadius::same(6))
                        .min_size(Vec2::new(64.0, 24.0));
                        if ui.add(b).on_hover_text(ayuda).clicked() && !activo {
                            self.ctl.enviar(Orden::Modo(modo));
                        }
                    }
                });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let nombres = self.tandas.nombres();
                let elegidos = self.elegidos(e);
                ui.menu_button("📁 Tandas ⏷", |ui| {
                    ui.set_min_width(220.0);
                    if ui.add_enabled(!e.lista.is_empty(), egui::Button::new("💾 Guardar esta tanda…")).clicked() {
                        self.guardando = Some(String::new());
                        self.guardar_ids = None;
                        ui.close();
                    }
                    if elegidos.len() > 1 && ui.button(format!("💾 Guardar los {} elegidos…", elegidos.len())).clicked()
                    {
                        self.guardando = Some(String::new());
                        self.guardar_ids = Some(elegidos.clone());
                        ui.close();
                    }
                    ui.separator();
                    ui.label(RichText::new("ABRIR (reemplaza la tanda actual)").size(11.0).color(MUY_TENUE));
                    if nombres.is_empty() {
                        ui.label(RichText::new("No hay tandas guardadas").color(TENUE));
                    }
                    for n in &nombres {
                        ui.horizontal(|ui| {
                            if ui.button(format!("📄 {n}")).clicked() {
                                match self.tandas.abrir(n) {
                                    Ok(spots) => self.ctl.enviar(Orden::CargarTanda(spots)),
                                    Err(err) => self.avisar_error(format!("No se pudo abrir «{n}»: {err}")),
                                }
                                ui.close();
                            }
                            if ui.small_button("🗑").on_hover_text("Borrar esta tanda guardada").clicked() {
                                if let Err(err) = self.tandas.borrar(n) {
                                    self.avisar_error(format!("No se pudo borrar «{n}»: {err}"));
                                }
                            }
                        });
                    }
                })
                .response
                .on_hover_text("Guardar la tanda o abrir una guardada");
                ui.add_space(4.0);
                // Miniaturas | Lista (se dibujan al revés: de derecha a izquierda).
                let lista = self.ultima_carpeta.tanda_lista;
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x0E, 0x11, 0x17))
                    .stroke(Stroke::new(1.0, BORDE))
                    .corner_radius(CornerRadius::same(7))
                    .inner_margin(egui::Margin::same(2))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (icono, es_lista, ayuda) in
                            [("☰", true, "Lista: una línea por spot"), ("⊞", false, "Con miniaturas")]
                        {
                            let activo = lista == es_lista;
                            let b = egui::Button::new(RichText::new(icono).size(13.0).color(if activo {
                                TEXTO
                            } else {
                                TENUE
                            }))
                            .fill(if activo { ACENTO_OSCURO } else { Color32::TRANSPARENT })
                            .stroke(if activo { Stroke::new(1.0, ACENTO) } else { Stroke::NONE })
                            .corner_radius(CornerRadius::same(5))
                            .min_size(Vec2::new(28.0, 22.0));
                            if ui.add(b).on_hover_text(ayuda).clicked() && !activo {
                                self.ultima_carpeta.tanda_lista = es_lista;
                                self.ultima_carpeta.guardar(&self.carpeta_datos_local);
                            }
                        }
                    });
            });
        });
        ui.add_space(4.0);
    }

    /// Empieza a elegir con el marco desde donde se apretó el mouse.
    fn empezar_marco(&mut self, ui: &egui::Ui, e: &Instantanea) {
        let (origen, mods) = ui.input(|i| (i.pointer.press_origin(), i.modifiers));
        let Some(origen) = origen else { return };
        // Con Ctrl, se suma a lo que ya estaba elegido.
        let base = if mods.command { self.elegidos(e) } else { Vec::new() };
        self.marco_tanda = Some(MarcoTanda { x0: origen.x, y0: origen.y - self.tanda_arriba, base });
    }

    /// Marco de elección: los spots que toca quedan elegidos.
    fn elegir_con_marco(&mut self, ui: &egui::Ui, e: &Instantanea, fondo: &egui::Response, filas: &[egui::Rect]) {
        if fondo.drag_started() {
            self.empezar_marco(ui, e);
        }
        if fondo.clicked() && !ui.input(|i| i.modifiers.command) {
            self.tanda_multi.clear();
            self.tanda_sel = None;
        }
        let Some(m) = &self.marco_tanda else { return };
        let (abajo, pos) = ui.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
        let Some(pos) = pos.filter(|_| abajo) else {
            self.marco_tanda = None;
            return;
        };
        let inicio = egui::pos2(m.x0, m.y0 + self.tanda_arriba);
        let marco = egui::Rect::from_two_pos(inicio, pos);
        let mut ids: Vec<u64> = m.base.clone();
        let mut ultimo = None;
        for (item, r) in e.lista.iter().zip(filas) {
            if r.max.y >= marco.min.y && r.min.y <= marco.max.y {
                if !ids.contains(&item.id) {
                    ids.push(item.id);
                }
                // El último tocado es el más cercano al puntero.
                if ultimo.is_none() || (pos.y >= inicio.y) {
                    ultimo = Some(item.id);
                }
                if pos.y < inicio.y && ultimo.is_some() && ids.len() == m.base.len() + 1 {
                    ultimo = Some(item.id);
                }
            }
        }
        // En el orden de la tanda.
        let en_orden: Vec<u64> = e.lista.iter().filter(|i| ids.contains(&i.id)).map(|i| i.id).collect();
        self.tanda_sel = ultimo.or(en_orden.first().copied()).or(self.tanda_sel);
        self.tanda_multi = en_orden;
        // El marco, encima de la lista.
        let capa = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("marco_tanda"));
        let p = ui.ctx().layer_painter(capa).with_clip_rect(fondo.rect);
        let dibujo = if marco.width() < 8.0 { marco.expand2(Vec2::new(14.0, 0.0)) } else { marco };
        p.rect_filled(dibujo, 3.0, ACENTO.gamma_multiply(0.12));
        p.rect_stroke(dibujo, 3.0, Stroke::new(1.0, ACENTO), StrokeKind::Inside);
        ui.ctx().request_repaint();
    }

    /// Pie de la tanda: resumen; con spots elegidos, sus acciones en el mismo lugar.
    fn pie_tanda(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let ids = self.elegidos(e);
        // Las acciones de lo elegido solo salen en modo «elegir» (Ctrl, Mayús, los
        // números o el marco); un clic normal solo marca el spot.
        let eligiendo = !self.tanda_multi.is_empty() && !ids.is_empty();
        let alto = 30.0;
        ui.horizontal(|ui| {
            ui.set_min_height(alto);
            let b = |t: &str| egui::Button::new(RichText::new(t).size(14.0)).min_size(Vec2::new(30.0, 28.0));
            if !eligiendo {
                let total: f64 = e.lista.iter().map(|i| i.duracion.as_secs_f64()).sum();
                ui.label(RichText::new(format!("{} spots", e.lista.len())).color(TENUE));
                ui.label(RichText::new("·").color(MUY_TENUE));
                ui.label(RichText::new(mmss(total)).font(FontId::monospace(17.0)).strong());
                // Con la tanda al aire: cuánto falta.
                if e.fase != FaseVista::Tv {
                    if let Some(pos) = e.actual.and_then(|a| e.lista.iter().position(|i| i.id == a)) {
                        let resto: f64 = e.lista[pos..].iter().map(|i| i.duracion.as_secs_f64()).sum::<f64>()
                            - e.transcurrido.as_secs_f64();
                        ui.label(RichText::new(format!("· quedan {}", mmss(resto.max(0.0)))).color(AIRE));
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(!e.lista.is_empty(), egui::Button::new("🗑 Vaciar").min_size(Vec2::new(0.0, 28.0)))
                        .on_hover_text("Quita todos los spots (el que está al aire se queda)")
                        .clicked()
                    {
                        self.ctl.enviar(Orden::LimpiarTanda);
                        self.tanda_sel = None;
                        self.tanda_multi.clear();
                    }
                    // Un spot marcado (clic normal): ponerlo al aire desde ahí.
                    if let [id] = ids.as_slice() {
                        if ui
                            .add(egui::Button::new("⏵ Desde aquí").min_size(Vec2::new(0.0, 28.0)))
                            .on_hover_text("Pone al aire el spot marcado y sigue la tanda desde ahí")
                            .clicked()
                        {
                            self.ctl.enviar(Orden::Reproducir(*id));
                        }
                    } else if !e.lista.is_empty() {
                        ui.label(
                            RichText::new("Ctrl+clic o arrastra sobre los números para elegir varios")
                                .size(11.5)
                                .color(MUY_TENUE),
                        );
                    }
                });
                return;
            }
            // Con spots elegidos.
            let total: f64 = e.lista.iter().filter(|i| ids.contains(&i.id)).map(|i| i.duracion.as_secs_f64()).sum();
            let texto = if ids.len() == 1 { "1 elegido".to_string() } else { format!("{} elegidos", ids.len()) };
            tema::chip_de(1.15, ui, &format!("{texto} · {}", mmss(total)), TEXTO, ACENTO_OSCURO);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                if ui.add(b("×")).on_hover_text("Dejar de elegir (Esc)").clicked() {
                    self.tanda_multi.clear();
                    self.tanda_sel = None;
                }
                let quitar = egui::Button::new(RichText::new("✖ Quitar").size(13.0))
                    .fill(AIRE_OSCURO)
                    .min_size(Vec2::new(0.0, 28.0));
                if ui.add(quitar).on_hover_text("Quitar de la tanda (Supr)").clicked() {
                    self.quitar_elegidos(e);
                }
                ui.add_space(4.0);
                let mas = ui.add(b("…")).on_hover_text("Más opciones");
                egui::Popup::menu(&mas).show(|ui| {
                    ui.set_min_width(200.0);
                    if ui.button("⏫ Al principio").clicked() {
                        self.mover_elegidos(e, Movida::Principio);
                    }
                    if ui.button("⏬ Al final").clicked() {
                        self.mover_elegidos(e, Movida::Final);
                    }
                    ui.separator();
                    if ui.button("🗐 Duplicar (Ctrl+D)").clicked() {
                        self.ctl.enviar(Orden::Duplicar(ids.clone()));
                    }
                    if ui.button("💾 Guardar como tanda…").clicked() {
                        self.guardando = Some(String::new());
                        self.guardar_ids = Some(ids.clone());
                    }
                    ui.separator();
                    if ui.button("☑ Elegir todos (Ctrl+A)").clicked() {
                        self.tanda_multi = e.lista.iter().map(|i| i.id).collect();
                    }
                });
                if ui.add(b("⏷")).on_hover_text("Bajar (Ctrl+↓)").clicked() {
                    self.mover_elegidos(e, Movida::Bajar);
                }
                if ui.add(b("⏶")).on_hover_text("Subir (Ctrl+↑)").clicked() {
                    self.mover_elegidos(e, Movida::Subir);
                }
                if ui
                    .add(b("⏭"))
                    .on_hover_text("Que salga(n) a continuación (justo después del que está al aire)")
                    .clicked()
                {
                    self.mover_elegidos(e, Movida::Siguiente);
                }
                if ids.len() == 1
                    && ui.add(b("⏵")).on_hover_text("Poner al aire ahora (y seguir la tanda desde ahí)").clicked()
                {
                    self.ctl.enviar(Orden::Reproducir(ids[0]));
                }
            });
        });
    }

    /// Ctrl+A (elegir todos), Ctrl+D (duplicar) y Esc (dejar de elegir), con el puntero
    /// sobre la tanda.
    fn teclas_tanda(&mut self, ui: &egui::Ui, e: &Instantanea, zona: egui::Rect) {
        let encima = ui.ctx().pointer_hover_pos().is_some_and(|p| zona.contains(p));
        if !encima || ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let (todo, duplicar, soltar) = ui.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::A),
                i.modifiers.command && i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::Escape) && !self.tanda_multi.is_empty(),
            )
        });
        if todo && !e.lista.is_empty() {
            self.tanda_multi = e.lista.iter().map(|i| i.id).collect();
            if self.tanda_sel.is_none() {
                self.tanda_sel = e.lista.first().map(|i| i.id);
            }
        }
        if duplicar {
            let ids = self.elegidos(e);
            if !ids.is_empty() {
                self.ctl.enviar(Orden::Duplicar(ids));
            }
        }
        if soltar {
            self.tanda_multi.clear();
        }
    }

    /// Clic derecho sobre un spot (o sobre varios elegidos).
    fn menu_spot(&mut self, ui: &mut egui::Ui, e: &Instantanea, item: &nucleo::ItemTanda) {
        let ids = self.elegidos(e);
        let varios = ids.len() > 1;
        let n = ids.len();
        let titulo = if varios { format!("{n} spots elegidos") } else { partir(&item.clip).1.to_string() };
        ui.label(RichText::new(titulo).size(12.0).color(TENUE));
        ui.separator();
        let mut cerrar = false;
        if !varios && ui.button("⏵ Poner al aire ahora").clicked() {
            self.ctl.enviar(Orden::Reproducir(item.id));
            cerrar = true;
        }
        let t = if varios { "⏭ Que salgan a continuación" } else { "⏭ Que salga a continuación" };
        if ui.button(t).on_hover_text("Justo después del que está al aire").clicked() {
            self.mover_elegidos(e, Movida::Siguiente);
            cerrar = true;
        }
        ui.separator();
        for (t, m) in [
            ("⏶ Subir", Movida::Subir),
            ("⏷ Bajar", Movida::Bajar),
            ("⏫ Al principio", Movida::Principio),
            ("⏬ Al final", Movida::Final),
        ] {
            if ui.button(t).clicked() {
                self.mover_elegidos(e, m);
                cerrar = true;
            }
        }
        ui.separator();
        if ui.button(if varios { format!("🗐 Duplicar los {n}") } else { "🗐 Duplicar".into() }).clicked() {
            self.ctl.enviar(Orden::Duplicar(ids.clone()));
            cerrar = true;
        }
        if varios && ui.button("💾 Guardar los elegidos como tanda…").clicked() {
            self.guardando = Some(String::new());
            self.guardar_ids = Some(ids.clone());
            cerrar = true;
        }
        if !varios {
            if let Some(ruta) = self.ruta_de_clip(&item.clip, e) {
                if ui.button("👁 Vista previa").clicked() {
                    self.abrir_vista_previa(ruta.clone(), Some(item.duracion).filter(|d| !d.is_zero()));
                    cerrar = true;
                }
                if !self.ctl.es_remoto() && ui.button("📂 Mostrar en Archivos").clicked() {
                    if let Some(carpeta) = ruta.parent() {
                        self.vista_caspar = false;
                        self.explorador.abrir(carpeta.to_path_buf());
                        self.explorador.seleccion = Some(ruta.clone());
                    }
                    cerrar = true;
                }
            }
        }
        ui.separator();
        if ui.button("☑ Elegir todos").on_hover_text("Ctrl+A").clicked() {
            self.tanda_multi = e.lista.iter().map(|i| i.id).collect();
            cerrar = true;
        }
        if varios && ui.button("Dejar de elegir").clicked() {
            self.tanda_multi.clear();
            cerrar = true;
        }
        ui.separator();
        let quitar = if varios { format!("✖ Quitar los {n} de la tanda") } else { "✖ Quitar de la tanda".into() };
        if ui.button(RichText::new(quitar).color(AIRE)).on_hover_text("Supr").clicked() {
            self.quitar_elegidos(e);
            cerrar = true;
        }
        if cerrar {
            ui.close();
        }
    }

    fn ventana_guardar(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let Some(mut nombre) = self.guardando.take() else {
            return;
        };
        let mut guardar = false;
        let mut cancelar = false;
        let spots_a_guardar: Vec<&nucleo::ItemTanda> = match &self.guardar_ids {
            Some(ids) => e.lista.iter().filter(|i| ids.contains(&i.id)).collect(),
            None => e.lista.iter().collect(),
        };
        let sub = format!(
            "{} spots · {}{}",
            spots_a_guardar.len(),
            mmss(spots_a_guardar.iter().map(|i| i.duracion.as_secs_f64()).sum()),
            if self.guardar_ids.is_some() { " (los elegidos)" } else { "" }
        );
        let modal = Dialogo::new("relevo-guardar-tanda", "💾", "Guardar tanda").subtitulo(sub).ancho(380.0).mostrar(
            ui.ctx(),
            |ui| {
                ui.label(RichText::new("Nombre").size(12.5).color(TENUE));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut nombre)
                        .hint_text("Nombre, por ejemplo: Mañana lunes")
                        .desired_width(f32::INFINITY),
                );
                if nombre.is_empty() && !r.has_focus() {
                    r.request_focus();
                }
                if (r.has_focus() || r.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    guardar = true;
                }
                let limpio = crate::tandas::limpiar_nombre(&nombre);
                if self.tandas.nombres().iter().any(|n| n.eq_ignore_ascii_case(&limpio)) {
                    ui.label(
                        RichText::new("Ya existe una tanda con ese nombre: se reemplazará").size(12.0).color(AMBAR),
                    );
                }
                let (si, no) = dialogo::pie(ui, "Guardar", Tono::Normal, !limpio.is_empty());
                guardar |= si;
                cancelar |= no;
            },
        );
        if modal.cerrar {
            cancelar = true;
        }
        if guardar {
            let spots: Vec<(String, Duration)> = spots_a_guardar.iter().map(|i| (i.clip.clone(), i.duracion)).collect();
            match self.tandas.guardar(&nombre, &spots) {
                Ok(()) => {
                    self.avisar_local(format!(
                        "Tanda «{}» guardada en {}",
                        nombre.trim(),
                        self.tandas.carpeta().display()
                    ));
                    return;
                }
                Err(err) => self.avisar_error(format!("No se pudo guardar: {err}")),
            }
        }
        if !cancelar {
            self.guardando = Some(nombre);
        }
    }
}

// ======================================================================
// Columna ARCHIVOS (izquierda): explorador de la PC
// ======================================================================

impl Ventana {
    fn columna_archivos(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let remoto = self.ctl.es_remoto();
        let zona = ui.max_rect();
        self.navegar_con_mouse_y_teclado(ui, zona);
        // Cabecera: título, cómo se ven los videos, panel de carpetas y recargar.
        ui.horizontal(|ui| {
            tema::titulo(ui, "ARCHIVOS");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if ui.button("↻").on_hover_text("Volver a leer carpetas y unidades").clicked() {
                    self.explorador.recargar();
                    self.navegacion.recargar();
                    self.ctl.enviar(Orden::RecargarMedios);
                    self.ctl.olvidar_miniaturas();
                    self.texturas.clear();
                }
                let t =
                    if self.arbol_visible { "Ocultar el panel de carpetas" } else { "Mostrar el panel de carpetas" };
                if ui.add(egui::Button::new("🗀").selected(self.arbol_visible)).on_hover_text(t).clicked() {
                    self.arbol_visible = !self.arbol_visible;
                }
                ui.add_space(4.0);
                // Miniaturas | Lista.
                let lista = self.ultima_carpeta.vista_lista;
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x0E, 0x11, 0x17))
                    .stroke(Stroke::new(1.0, BORDE))
                    .corner_radius(CornerRadius::same(7))
                    .inner_margin(egui::Margin::same(2))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        // (right_to_left: se dibujan al revés)
                        for (icono, es_lista, ayuda) in
                            [("☰", true, "Lista: solo nombres, más videos a la vista"), ("⊞", false, "Miniaturas")]
                        {
                            let activo = lista == es_lista;
                            let b = egui::Button::new(RichText::new(icono).size(13.0).color(if activo {
                                TEXTO
                            } else {
                                TENUE
                            }))
                            .fill(if activo { ACENTO_OSCURO } else { Color32::TRANSPARENT })
                            .stroke(if activo { Stroke::new(1.0, ACENTO) } else { Stroke::NONE })
                            .corner_radius(CornerRadius::same(5))
                            .min_size(Vec2::new(28.0, 22.0));
                            if ui.add(b).on_hover_text(ayuda).clicked() && !activo {
                                self.ultima_carpeta.vista_lista = es_lista;
                                self.ultima_carpeta.guardar(&self.carpeta_datos_local);
                            }
                        }
                    });
            });
        });

        // Ruta y búsqueda a todo el ancho (también por encima del panel de carpetas).
        let datos = if self.vista_caspar {
            None
        } else {
            self.barra_ruta(ui);
            self.barra_busqueda(ui);
            ui.add_space(2.0);
            Some(self.entradas_archivos())
        };

        let alto_pie = if datos.is_some() { 40.0 } else { 0.0 };
        let alto_medio = (ui.available_height() - alto_pie).max(120.0);
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), alto_medio),
            Layout::left_to_right(Align::Min),
            |ui| {
                ui.set_height(alto_medio);
                if self.arbol_visible {
                    let disponible = ui.available_width();
                    let ancho = match self.diseno.m.arbol {
                        Some(a) => a.clamp(120.0, (disponible - 200.0).max(120.0)),
                        None => (disponible * 0.36).clamp(150.0, 230.0),
                    };
                    ui.allocate_ui_with_layout(Vec2::new(ancho, alto_medio), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(ancho);
                        egui::ScrollArea::vertical().id_salt("arbol").auto_shrink([false, false]).show(ui, |ui| {
                            let medios = if remoto { None } else { e.carpeta_medios.as_deref() };
                            let actual = self.explorador.carpeta.clone();
                            match self.navegacion.mostrar(ui, actual.as_deref(), medios, remoto, self.vista_caspar) {
                                Some(AccionNav::Abrir(r)) => {
                                    self.vista_caspar = false;
                                    self.explorador.abrir(r);
                                }
                                Some(AccionNav::CasparRemoto) => self.vista_caspar = true,
                                None => {}
                            }
                        });
                    });
                    // Separación: ancho del panel de carpetas.
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(diseno::GROSOR, alto_medio), Sense::hover());
                    let r = diseno::divisor(ui, "arbol", rect, true);
                    if r.dragged() {
                        self.diseno.m.arbol = Some((ancho + r.drag_delta().x).max(120.0));
                    }
                    if r.double_clicked() {
                        self.diseno.m.arbol = None;
                    }
                    if r.drag_stopped() || r.double_clicked() {
                        self.diseno.guardar();
                    }
                }
                ui.vertical(|ui| match &datos {
                    None => self.lista_caspar(ui, e),
                    Some(d) => self.lista_archivos(ui, e, d),
                });
            },
        );
        if let Some(d) = &datos {
            ui.separator();
            self.pie_archivos(ui, d);
        }
    }

    /// Buscador (en esta carpeta y sus subcarpetas) y orden.
    fn barra_busqueda(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let ancho_orden = 150.0;
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.explorador.texto_busqueda)
                    .hint_text("🔍  Buscar videos aquí y en subcarpetas…")
                    .desired_width((ui.available_width() - ancho_orden - 8.0).max(80.0)),
            );
            if r.changed() {
                self.explorador.busqueda_cambio();
            }
            // Esc borra la búsqueda.
            if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.explorador.texto_busqueda.clear();
                self.explorador.busqueda_cambio();
            }
            let ya = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            self.explorador.tal_vez_buscar(ya);
            let antes = self.ultima_carpeta.orden;
            egui::ComboBox::from_id_salt("orden_archivos")
                .width(ancho_orden - 8.0)
                .selected_text(RichText::new(format!("Orden: {}", antes.nombre())).size(12.5))
                .show_ui(ui, |ui| {
                    for o in crate::explorador::Orden::TODOS {
                        ui.selectable_value(&mut self.ultima_carpeta.orden, o, o.nombre());
                    }
                })
                .response
                .on_hover_text("Orden de los videos (las carpetas van primero)");
            if self.ultima_carpeta.orden != antes {
                self.ultima_carpeta.guardar(&self.carpeta_datos_local);
            }
        });
    }

    /// Lo que hay que mostrar: la carpeta abierta o lo que se buscó, y su resumen.
    fn entradas_archivos(&mut self) -> DatosArchivos {
        let (entradas, estado): (Option<std::sync::Arc<Vec<Entrada>>>, Option<String>) =
            match self.explorador.busqueda() {
                Busqueda::Buscando { encontrados } => (Some(encontrados), Some("Buscando…".into())),
                Busqueda::Lista { encontrados, cortada } => {
                    let n = encontrados.len();
                    let t = if cortada {
                        format!("{n} resultados (se muestran los primeros)")
                    } else {
                        format!("{n} resultados")
                    };
                    (Some(encontrados), Some(t))
                }
                Busqueda::Inactiva => match self.explorador.listado() {
                    Listado::Cargando => (None, Some("Cargando…".into())),
                    Listado::Error(err) => (None, Some(err)),
                    Listado::Listo(v) => (Some(v), None),
                },
            };
        let buscando = !matches!(self.explorador.busqueda(), Busqueda::Inactiva);
        let entradas = entradas.map(|v| crate::explorador::ordenar(&v, self.ultima_carpeta.orden));
        // Resumen: cuántas carpetas y videos hay, y cuánto duran.
        let estado = estado.or_else(|| {
            let v = entradas.as_ref()?;
            let carpetas = v.iter().filter(|x| x.es_carpeta).count();
            let videos: Vec<&Entrada> = v.iter().filter(|x| !x.es_carpeta).collect();
            if v.is_empty() {
                return None;
            }
            let mut partes = Vec::new();
            if carpetas > 0 {
                partes.push(format!("{carpetas} carpeta{}", if carpetas == 1 { "" } else { "s" }));
            }
            if !videos.is_empty() {
                partes.push(format!("{} video{}", videos.len(), if videos.len() == 1 { "" } else { "s" }));
                let total: f64 = videos.iter().filter_map(|x| x.duracion).map(|d| d.as_secs_f64()).sum();
                if total > 0.0 {
                    partes.push(format!("{} en total", duracion_larga(total)));
                }
            }
            Some(partes.join(" · "))
        });
        DatosArchivos { entradas, estado, buscando }
    }

    /// La lista de carpetas y videos.
    fn lista_archivos(&mut self, ui: &mut egui::Ui, e: &Instantanea, d: &DatosArchivos) {
        // Al cambiar de carpeta (o de búsqueda) no queda nada elegido de la anterior.
        let ahora_en = (self.explorador.carpeta.clone(), d.buscando);
        if self.archivos_en != ahora_en {
            self.archivos_en = ahora_en;
            self.archivos_multi.clear();
            self.explorador.seleccion = None;
            self.marco_archivos = None;
        }
        let lista = self.ultima_carpeta.vista_lista;
        let mut filas: Vec<(PathBuf, egui::Rect)> = Vec::new();
        let mut fondo: Option<egui::Response> = None;
        let alto_lista = ui.available_height().max(80.0);
        let salida = egui::ScrollArea::vertical()
            .id_salt("archivos")
            .auto_shrink([false, false])
            .scroll_source(egui::scroll_area::ScrollSource {
                drag: egui::scroll_area::DragScroll::Never,
                ..egui::scroll_area::ScrollSource::ALL
            })
            .max_height(alto_lista)
            .show(ui, |ui| {
                // Fondo: arrastrar desde un espacio vacío dibuja un marco para elegir.
                let vista = vista_de_lista(ui);
                fondo = Some(ui.interact(vista, egui::Id::unique("fondo_archivos"), Sense::click_and_drag()));
                self.archivos_arriba = ui.min_rect().top();
                if self.marco_archivos.is_some() {
                    if let Some(pos) = ui.ctx().pointer_hover_pos() {
                        if pos.y < vista.top() + 30.0 {
                            ui.scroll_with_delta(Vec2::new(0.0, 8.0));
                        } else if pos.y > vista.bottom() - 30.0 {
                            ui.scroll_with_delta(Vec2::new(0.0, -8.0));
                        }
                    }
                }
                let Some(entradas) = &d.entradas else { return };
                if entradas.is_empty() {
                    let (titulo, detalle) = if d.buscando {
                        ("Sin resultados", "Prueba con otra palabra o busca desde una carpeta más arriba.")
                    } else {
                        ("No hay videos aquí", "Solo se muestran carpetas y videos (mp4, mov, mxf, avi…).")
                    };
                    mensaje_vacio(ui, "🗁", titulo, detalle);
                    return;
                }
                if lista {
                    ui.spacing_mut().item_spacing.y = 1.0;
                }
                for entrada in entradas.iter() {
                    if entrada.es_carpeta {
                        self.fila_carpeta(ui, entrada, lista);
                    } else {
                        let r = self.fila_video(ui, e, entrada, d.buscando, lista);
                        filas.push((entrada.ruta.clone(), r));
                    }
                }
            });
        self.archivos_visibles = d
            .entradas
            .iter()
            .flat_map(|v| v.iter())
            .filter(|x| !x.es_carpeta)
            .map(|x| (x.ruta.clone(), x.duracion))
            .collect();
        if let Some(f) = &fondo {
            self.elegir_archivos_con_marco(ui, f, &filas);
        }
        self.teclas_archivos(ui, salida.inner_rect);
    }

    /// Pie de Archivos: el resumen o, con videos elegidos, sus acciones.
    fn pie_archivos(&mut self, ui: &mut egui::Ui, d: &DatosArchivos) {
        let elegidos = self.archivos_elegidos();
        // Solo en modo «elegir» (Ctrl, Mayús, casillas o marco); un clic normal solo marca.
        let eligiendo = !self.archivos_multi.is_empty() && !elegidos.is_empty();
        ui.horizontal(|ui| {
            ui.set_min_height(30.0);
            if !eligiendo {
                if let Some(t) = &d.estado {
                    if t.starts_with("Buscando") || t.starts_with("Cargando") {
                        ui.add(egui::Spinner::new().size(12.0));
                    }
                    ui.add(egui::Label::new(RichText::new(t).size(12.5).color(TENUE)).truncate());
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new("Arrastra a la tanda · Ctrl+clic: elegir varios").size(11.5).color(MUY_TENUE),
                    );
                });
                return;
            }
            let total: f64 = elegidos.iter().filter_map(|(_, d)| *d).map(|d| d.as_secs_f64()).sum();
            let texto = if elegidos.len() == 1 { "1 video".to_string() } else { format!("{} videos", elegidos.len()) };
            let texto = if total > 0.0 { format!("{texto} · {}", mmss(total)) } else { texto };
            tema::chip_de(1.15, ui, &texto, TEXTO, ACENTO_OSCURO);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let b = |t: &str| egui::Button::new(RichText::new(t).size(13.5)).min_size(Vec2::new(30.0, 28.0));
                if ui.add(b("×")).on_hover_text("Dejar de elegir (Esc)").clicked() {
                    self.archivos_multi.clear();
                    self.explorador.seleccion = None;
                }
                if ui
                    .add(b("⏭"))
                    .on_hover_text("Que salga(n) a continuación (justo después del que está al aire)")
                    .clicked()
                {
                    self.agregar_elegidos(Destino::Siguiente);
                }
                let anadir = egui::Button::new(RichText::new("+ Añadir a la tanda").size(13.0))
                    .fill(ACENTO_OSCURO)
                    .stroke(Stroke::new(1.0, ACENTO))
                    .min_size(Vec2::new(0.0, 28.0));
                if ui.add(anadir).on_hover_text("Al final de la tanda (Enter)").clicked() {
                    self.agregar_elegidos(Destino::Final);
                }
            });
        });
    }

    /// Los videos elegidos en Archivos, en el orden en que se ven (con su duración).
    fn archivos_elegidos(&self) -> Vec<(PathBuf, Option<Duration>)> {
        if !self.archivos_multi.is_empty() {
            self.archivos_visibles.iter().filter(|(r, _)| self.archivos_multi.contains(r)).cloned().collect()
        } else {
            match &self.explorador.seleccion {
                Some(s) => {
                    let d = self.archivos_visibles.iter().find(|(r, _)| r == s).and_then(|(_, d)| *d);
                    vec![(s.clone(), d.or_else(|| archivos::duracion_video(s)))]
                }
                None => Vec::new(),
            }
        }
    }

    fn archivo_elegido(&self, ruta: &Path) -> bool {
        if !self.archivos_multi.is_empty() {
            self.archivos_multi.iter().any(|r| r == ruta)
        } else {
            self.explorador.seleccion.as_deref() == Some(ruta)
        }
    }

    /// Añade a la tanda los videos elegidos, en su orden.
    fn agregar_elegidos(&mut self, destino: Destino) {
        let elegidos = self.archivos_elegidos();
        // «Siguiente» pone cada uno justo después del que está al aire: al revés, así
        // quedan en su orden.
        let orden: Vec<_> =
            if matches!(destino, Destino::Siguiente) { elegidos.into_iter().rev().collect() } else { elegidos };
        for (ruta, duracion) in orden {
            self.agregar_archivo(&ruta, duracion, destino);
        }
    }

    /// Clic: elige ese. Ctrl+clic: lo suma o lo quita. Mayús+clic: desde el último.
    fn clic_en_archivo(&mut self, ruta: &Path, mods: egui::Modifiers) {
        let pos = |r: &Path| self.archivos_visibles.iter().position(|(x, _)| x == r);
        if mods.shift {
            if let (Some(a), Some(b)) = (self.explorador.seleccion.as_deref().and_then(pos), pos(ruta)) {
                let (desde, hasta) = (a.min(b), a.max(b));
                self.archivos_multi = self.archivos_visibles[desde..=hasta].iter().map(|(r, _)| r.clone()).collect();
                return;
            }
        }
        if mods.command {
            // Ctrl+clic (o la casilla) entra en modo «elegir», aunque sea uno solo.
            if self.archivos_multi.is_empty() {
                self.archivos_multi = self
                    .explorador
                    .seleccion
                    .clone()
                    .filter(|s| s != ruta && self.archivos_visibles.iter().any(|(r, _)| r == s))
                    .into_iter()
                    .collect();
            }
            if let Some(k) = self.archivos_multi.iter().position(|x| x == ruta) {
                self.archivos_multi.remove(k);
                self.explorador.seleccion = self.archivos_multi.last().cloned();
            } else {
                self.archivos_multi.push(ruta.to_path_buf());
                self.explorador.seleccion = Some(ruta.to_path_buf());
            }
            return;
        }
        self.archivos_multi.clear();
        self.explorador.seleccion = Some(ruta.to_path_buf());
    }

    fn empezar_marco_archivos(&mut self, ui: &egui::Ui) {
        let (origen, mods) = ui.input(|i| (i.pointer.press_origin(), i.modifiers));
        let Some(origen) = origen else { return };
        let base: Vec<PathBuf> =
            if mods.command { self.archivos_elegidos().into_iter().map(|(r, _)| r).collect() } else { Vec::new() };
        self.marco_archivos = Some(MarcoArchivos { y0: origen.y - self.archivos_arriba, x0: origen.x, base });
    }

    /// Marco de elección en Archivos: los videos que toca quedan elegidos.
    fn elegir_archivos_con_marco(&mut self, ui: &egui::Ui, fondo: &egui::Response, filas: &[(PathBuf, egui::Rect)]) {
        if fondo.drag_started() {
            self.empezar_marco_archivos(ui);
        }
        if fondo.clicked() && !ui.input(|i| i.modifiers.command) {
            self.archivos_multi.clear();
            self.explorador.seleccion = None;
        }
        let Some(m) = &self.marco_archivos else { return };
        let (abajo, pos) = ui.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
        let Some(pos) = pos.filter(|_| abajo) else {
            self.marco_archivos = None;
            return;
        };
        let inicio = egui::pos2(m.x0, m.y0 + self.archivos_arriba);
        let marco = egui::Rect::from_two_pos(inicio, pos);
        let mut elegidos = m.base.clone();
        let mut ultimo = None;
        for (ruta, r) in filas {
            if r.max.y >= marco.min.y && r.min.y <= marco.max.y {
                if !elegidos.contains(ruta) {
                    elegidos.push(ruta.clone());
                }
                if ultimo.is_none() || pos.y >= inicio.y {
                    ultimo = Some(ruta.clone());
                }
            }
        }
        let en_orden: Vec<PathBuf> =
            self.archivos_visibles.iter().filter(|(r, _)| elegidos.contains(r)).map(|(r, _)| r.clone()).collect();
        self.explorador.seleccion = ultimo.or_else(|| en_orden.first().cloned());
        self.archivos_multi = en_orden;
        let capa = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("marco_archivos"));
        let p = ui.ctx().layer_painter(capa).with_clip_rect(fondo.rect);
        let dibujo = if marco.width() < 8.0 { marco.expand2(Vec2::new(12.0, 0.0)) } else { marco };
        p.rect_filled(dibujo, 3.0, ACENTO.gamma_multiply(0.12));
        p.rect_stroke(dibujo, 3.0, Stroke::new(1.0, ACENTO), StrokeKind::Inside);
        ui.ctx().request_repaint();
    }

    /// Ctrl+A (elegir todos los videos) y Esc (dejar de elegir), con el puntero encima.
    fn teclas_archivos(&mut self, ui: &egui::Ui, zona: egui::Rect) {
        let encima = ui.ctx().pointer_hover_pos().is_some_and(|p| zona.contains(p));
        if !encima || ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let (todo, soltar) =
            ui.input(|i| (i.modifiers.command && i.key_pressed(egui::Key::A), i.key_pressed(egui::Key::Escape)));
        if todo && !self.archivos_visibles.is_empty() {
            self.archivos_multi = self.archivos_visibles.iter().map(|(r, _)| r.clone()).collect();
            if self.explorador.seleccion.is_none() {
                self.explorador.seleccion = self.archivos_multi.first().cloned();
            }
        }
        if soltar {
            self.archivos_multi.clear();
        }
    }

    /// Atrás, adelante, subir y la ruta como migas de pan (clic en una: ir ahí; ✎ o
    /// Ctrl+L: escribir la ruta).
    fn barra_ruta(&mut self, ui: &mut egui::Ui) {
        let tam = Vec2::new(28.0, 28.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui
                .add_enabled(self.explorador.puede_atras(), egui::Button::new("⬅").min_size(tam))
                .on_hover_text("Atrás (Alt+←, botón de atrás del mouse)")
                .clicked()
            {
                self.explorador.volver_atras();
            }
            if ui
                .add_enabled(self.explorador.puede_adelante(), egui::Button::new("➡").min_size(tam))
                .on_hover_text("Adelante (Alt+→, botón de adelante del mouse)")
                .clicked()
            {
                self.explorador.ir_adelante();
            }
            let hay_padre = self.explorador.carpeta.as_ref().and_then(|c| c.parent()).is_some();
            if ui
                .add_enabled(hay_padre, egui::Button::new("⬆").min_size(tam))
                .on_hover_text("Subir una carpeta (Alt+↑ o Retroceso)")
                .clicked()
            {
                self.explorador.subir();
            }
            ui.add_space(2.0);
            let Some(actual) = self.explorador.carpeta.clone() else { return };
            // A la derecha: anclar y escribir la ruta.
            let reservado = 2.0 * (tam.x + 4.0) + 12.0;
            let ancho = (ui.available_width() - reservado).max(60.0);
            if self.editando_ruta {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.explorador.texto_ruta)
                        .font(FontId::proportional(13.0))
                        .desired_width(ancho),
                );
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if r.lost_focus() {
                    self.editando_ruta = false;
                    if enter {
                        let ruta = PathBuf::from(self.explorador.texto_ruta.trim());
                        if ruta.is_dir() {
                            self.explorador.abrir(ruta);
                        } else {
                            self.avisar_error(format!("No existe la carpeta {}", ruta.display()));
                        }
                    }
                    if esc || !enter {
                        self.explorador.texto_ruta = actual.display().to_string();
                    }
                }
            } else {
                self.migas_de_pan(ui, &actual, ancho);
            }
            if ui
                .add(egui::Button::new("✏").min_size(tam))
                .on_hover_text("Escribir la ruta (Ctrl+L o doble clic en la ruta)")
                .clicked()
            {
                self.editando_ruta = !self.editando_ruta;
            }
            let anclada = self.navegacion.anclada(&actual);
            let b = egui::Button::new(RichText::new("📌").color(if anclada { ACENTO } else { TENUE }))
                .selected(anclada)
                .min_size(tam);
            let ayuda = if anclada { "Quitar de acceso rápido" } else { "Anclar esta carpeta a acceso rápido" };
            if ui.add(b).on_hover_text(ayuda).clicked() {
                if anclada {
                    self.navegacion.desanclar(&actual);
                } else {
                    self.navegacion.anclar(&actual);
                }
            }
        });
    }

    /// Migas de pan de la carpeta abierta. Si no caben, se muestran las últimas
    /// (las de antes, en un menú «…»).
    fn migas_de_pan(&mut self, ui: &mut egui::Ui, actual: &Path, ancho: f32) {
        let migas = crate::explorador::migas(actual);
        let fuente = FontId::proportional(13.0);
        let medir = |t: &str| ui.painter().layout_no_wrap(t.to_string(), fuente.clone(), TEXTO).size().x + 14.0;
        let sep = 14.0;
        // Desde la última hacia atrás, las que quepan.
        let mut usado = 30.0; // sitio para «…»
        let mut desde = migas.len();
        while desde > 0 {
            let w = medir(&migas[desde - 1].0).min(160.0) + sep;
            if usado + w > ancho && desde < migas.len() {
                break;
            }
            usado += w;
            desde -= 1;
        }
        let mut ir: Option<PathBuf> = None;
        let (rect, fondo) = ui.allocate_exact_size(Vec2::new(ancho, 28.0), Sense::click());
        ui.painter().rect_filled(rect, 6.0, Color32::from_rgb(0x0E, 0x11, 0x17));
        ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, BORDE), StrokeKind::Inside);
        // Doble clic en el espacio libre: escribir la ruta (como en Windows).
        if fondo.double_clicked() {
            self.editando_ruta = true;
        }
        let mut hijo = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(4.0, 2.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        hijo.set_clip_rect(rect.intersect(ui.clip_rect()));
        hijo.spacing_mut().item_spacing.x = 0.0;
        if desde > 0 {
            let r =
                hijo.add(egui::Button::new(RichText::new("…").size(13.0)).frame(false).min_size(Vec2::new(24.0, 22.0)));
            egui::Popup::menu(&r).show(|ui| {
                for (nombre, ruta) in migas[..desde].iter().rev() {
                    if ui.button(format!("🗀 {nombre}")).clicked() {
                        ir = Some(ruta.clone());
                    }
                }
            });
            hijo.label(RichText::new(" › ").size(13.0).color(MUY_TENUE));
        }
        let ultima = migas.len().saturating_sub(1);
        for (i, (nombre, ruta)) in migas.iter().enumerate().skip(desde) {
            let es_ultima = i == ultima;
            let texto = RichText::new(recortar(nombre, 24)).size(13.0).color(if es_ultima { TEXTO } else { TENUE });
            let texto = if es_ultima { texto.family(tema::negrita()) } else { texto };
            let r = hijo.add(egui::Button::new(texto).frame(false).min_size(Vec2::new(0.0, 22.0)));
            let r = if nombre.chars().count() > 24 { r.on_hover_text(nombre) } else { r };
            if r.clicked() && !es_ultima {
                ir = Some(ruta.clone());
            }
            if !es_ultima {
                hijo.label(RichText::new(" › ").size(13.0).color(MUY_TENUE));
            }
        }
        if let Some(r) = ir {
            self.explorador.abrir(r);
        }
    }

    /// Botones de atrás/adelante del mouse y atajos de teclado sobre la sección Archivos.
    fn navegar_con_mouse_y_teclado(&mut self, ui: &egui::Ui, zona: egui::Rect) {
        let encima = ui.ctx().pointer_hover_pos().is_some_and(|p| zona.contains(p));
        let escribiendo = ui.ctx().egui_wants_keyboard_input();
        let (atras, adelante, subir, editar) = ui.input(|i| {
            let mouse_atras = encima && i.pointer.button_pressed(egui::PointerButton::Extra1);
            let mouse_adelante = encima && i.pointer.button_pressed(egui::PointerButton::Extra2);
            let alt = i.modifiers.alt;
            let teclas = !escribiendo;
            (
                mouse_atras || (teclas && alt && i.key_pressed(egui::Key::ArrowLeft)),
                mouse_adelante || (teclas && alt && i.key_pressed(egui::Key::ArrowRight)),
                teclas
                    && ((alt && i.key_pressed(egui::Key::ArrowUp))
                        || (encima && !i.modifiers.any() && i.key_pressed(egui::Key::Backspace))),
                teclas && i.modifiers.command && i.key_pressed(egui::Key::L),
            )
        });
        if self.vista_caspar {
            // En la lista de CasparCG: atrás = subir una carpeta.
            if atras || subir {
                let mut partes: Vec<&str> = self.carpeta_caspar.split('/').filter(|p| !p.is_empty()).collect();
                partes.pop();
                self.carpeta_caspar = partes.iter().map(|p| format!("{p}/")).collect();
            }
            return;
        }
        if atras {
            self.explorador.volver_atras();
        }
        if adelante {
            self.explorador.ir_adelante();
        }
        if subir {
            self.explorador.subir();
        }
        if editar {
            self.editando_ruta = true;
        }
    }

    /// Lista de videos de CasparCG por carpetas (en una PC remota, los de la PC principal).
    fn lista_caspar(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        // Migas de pan.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if ui.add(egui::Button::new("🎬 medios").selected(self.carpeta_caspar.is_empty()).small()).clicked() {
                self.carpeta_caspar.clear();
            }
            let mut acumulado = String::new();
            let partes: Vec<String> =
                self.carpeta_caspar.split('/').filter(|p| !p.is_empty()).map(String::from).collect();
            for parte in partes {
                ui.label(RichText::new("›").color(MUY_TENUE));
                acumulado.push_str(&parte);
                acumulado.push('/');
                let ultima = acumulado == self.carpeta_caspar;
                if ui.add(egui::Button::new(&parte).selected(ultima).small()).clicked() {
                    self.carpeta_caspar = acumulado.clone();
                }
            }
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.busqueda_caspar)
                .hint_text("🔍  Buscar en CasparCG…")
                .desired_width(ui.available_width()),
        );
        ui.add_space(4.0);

        let videos: Vec<&amcp::Medio> = e.medios.iter().filter(|m| m.tipo == amcp::TipoMedio::Movie).collect();
        let filtro = self.busqueda_caspar.trim().to_lowercase();
        let (subcarpetas, clips) = if filtro.is_empty() {
            contenido_de(&videos, &self.carpeta_caspar)
        } else {
            (Vec::new(), videos.iter().copied().filter(|m| m.nombre.to_lowercase().contains(&filtro)).collect())
        };

        let alto_lista = ui.available_height().max(80.0);
        egui::ScrollArea::vertical().id_salt("lista_caspar").auto_shrink([false, false]).max_height(alto_lista).show(
            ui,
            |ui| {
                if videos.is_empty() {
                    let t =
                        if e.conectado { "La carpeta de CasparCG está vacía" } else { "Sin conexión con CasparCG" };
                    mensaje_vacio(ui, "🗁", t, "");
                    return;
                }
                for (nombre, n) in &subcarpetas {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 5.0, TARJETA_HOVER);
                    }
                    let p = ui.painter();
                    p.text(
                        rect.left_center() + Vec2::new(10.0, 0.0),
                        Align2::LEFT_CENTER,
                        "🗀",
                        FontId::proportional(17.0),
                        AMBAR,
                    );
                    p.text(
                        rect.left_center() + Vec2::new(36.0, 0.0),
                        Align2::LEFT_CENTER,
                        recortar(nombre, 40),
                        FontId::proportional(14.0),
                        TEXTO,
                    );
                    p.text(
                        rect.right_center() - Vec2::new(10.0, 0.0),
                        Align2::RIGHT_CENTER,
                        n.to_string(),
                        FontId::proportional(12.0),
                        TENUE,
                    );
                    if resp.clicked() {
                        self.carpeta_caspar.push_str(nombre);
                        self.carpeta_caspar.push('/');
                    }
                }
                for m in clips {
                    let sel = self.caspar_sel.as_deref() == Some(m.nombre.as_str());
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), 50.0), Sense::click_and_drag());
                    if resp.drag_started() {
                        self.caspar_sel = Some(m.nombre.clone());
                        egui::DragAndDrop::set_payload(
                            ui.ctx(),
                            Arrastre::Clip {
                                clip: m.nombre.clone(),
                                duracion: Duration::from_secs_f64(m.duracion_seg.max(0.0)),
                            },
                        );
                    }
                    let fondo = if sel {
                        ACENTO_OSCURO
                    } else if resp.hovered() {
                        TARJETA_HOVER
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, 6.0, fondo);
                    let mini = egui::Rect::from_min_size(rect.min + Vec2::new(6.0, 5.0), Vec2::new(71.0, 40.0));
                    self.pintar_miniatura(ui, mini, Some(&m.nombre), &m.nombre, resp.hovered());
                    let (carpeta, nombre) = partir(&m.nombre);
                    let p = ui.painter();
                    let x = mini.right() + 10.0;
                    p.text(
                        egui::pos2(x, rect.top() + 15.0),
                        Align2::LEFT_CENTER,
                        recortar(nombre, 34),
                        FontId::proportional(14.0),
                        TEXTO,
                    );
                    let sub = if filtro.is_empty() { String::new() } else { carpeta.trim_end_matches('/').to_string() };
                    p.text(
                        egui::pos2(x, rect.bottom() - 13.0),
                        Align2::LEFT_CENTER,
                        recortar(&sub, 40),
                        FontId::proportional(11.5),
                        TENUE,
                    );
                    p.text(
                        egui::pos2(rect.right() - 8.0, rect.top() + 15.0),
                        Align2::RIGHT_CENTER,
                        mmss(m.duracion_seg),
                        FontId::monospace(14.0),
                        TEXTO,
                    );
                    let dur = Duration::from_secs_f64(m.duracion_seg.max(0.0));
                    if resp.clicked() {
                        self.caspar_sel = Some(m.nombre.clone());
                    }
                    if resp.double_clicked() {
                        self.ctl.enviar(Orden::Agregar {
                            clip: m.nombre.clone(),
                            duracion: dur,
                            destino: Destino::Final,
                        });
                    }
                    resp.context_menu(|ui| {
                        if ui.button("+ Añadir a la tanda").clicked() {
                            self.ctl.enviar(Orden::Agregar {
                                clip: m.nombre.clone(),
                                duracion: dur,
                                destino: Destino::Final,
                            });
                            ui.close();
                        }
                        if ui.button("⏭ Poner como siguiente").clicked() {
                            self.ctl.enviar(Orden::Agregar {
                                clip: m.nombre.clone(),
                                duracion: dur,
                                destino: Destino::Siguiente,
                            });
                            ui.close();
                        }
                        if ui.button(RichText::new("⏵ Al aire ahora").color(AIRE)).clicked() {
                            self.ctl.enviar(Orden::Agregar {
                                clip: m.nombre.clone(),
                                duracion: dur,
                                destino: Destino::AlAire,
                            });
                            ui.close();
                        }
                        if ui.button("🕓 Programar este spot").clicked() {
                            let hoy = self.hoy();
                            self.publicidades.nueva_con_video(hoy, &m.nombre, dur);
                            self.pantalla = Pantalla::Publicidades;
                            ui.close();
                        }
                    });
                }
            },
        );
    }

    /// Guarda la carpeta del explorador cuando cambia (y la de la lista de CasparCG).
    fn recordar_carpeta(&mut self) {
        let ahora = UltimaCarpeta {
            carpeta: self.explorador.carpeta.clone(),
            carpeta_caspar: self.carpeta_caspar.clone(),
            orden: self.ultima_carpeta.orden,
            vista_lista: self.ultima_carpeta.vista_lista,
            tanda_lista: self.ultima_carpeta.tanda_lista,
        };
        // La carpeta provisional (mientras arranca CasparCG) no se guarda.
        if ahora.carpeta.is_none() || self.carpeta_provisional.is_some() || ahora == self.ultima_carpeta {
            return;
        }
        ahora.guardar(&self.carpeta_datos_local);
        self.ultima_carpeta = ahora;
    }

    /// Etiqueta de un video del explorador: dónde está y qué pasará al añadirlo.
    fn insignia(&self, en_caspar: bool, e: &Instantanea) -> (&'static str, Color32) {
        if en_caspar {
            ("CasparCG", TV)
        } else if self.ctl.es_remoto() {
            ("se subirá", AMBAR)
        } else if self.cfg.archivos.externos == nucleo::ModoExternos::Directo && e.rutas_directas == Some(true) {
            ("sin copiar", ACENTO)
        } else if self.cfg.archivos.externos == nucleo::ModoExternos::Directo && e.rutas_directas.is_none() {
            ("fuera de CasparCG", AMBAR)
        } else {
            ("se copiará", AMBAR)
        }
    }

    fn fila_carpeta(&mut self, ui: &mut egui::Ui, entrada: &Entrada, lista: bool) {
        let alto = if lista { 26.0 } else { 30.0 };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), alto), Sense::click());
        // La carpeta de la que se vino (al subir o volver) queda marcada y a la vista.
        // Es solo una pista de dónde estabas: se desvanece en un momento (no está elegida).
        let resaltada = self.explorador.resaltar.as_deref() == Some(entrada.ruta.as_path());
        let mut desvanece = 0.0;
        if resaltada {
            let ahora = ui.input(|i| i.time);
            if self.resalte_visto.as_deref() != Some(entrada.ruta.as_path()) {
                self.resalte_visto = Some(entrada.ruta.clone());
                self.resalte_desde = ahora;
                resp.scroll_to_me(Some(Align::Center));
            }
            desvanece = 1.0 - dialogo::desde(ui.ctx(), self.resalte_desde, 1.2);
        }
        if desvanece > 0.0 && !resp.hovered() {
            ui.painter().rect_stroke(rect, 5.0, Stroke::new(1.0, ACENTO.gamma_multiply(desvanece)), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 5.0, TARJETA_HOVER);
        }
        if resp.hovered() {
            ui.painter().text(
                rect.right_center() - Vec2::new(10.0, 0.0),
                Align2::RIGHT_CENTER,
                "›",
                FontId::proportional(18.0),
                TENUE,
            );
        }
        let p = ui.painter();
        let (tam_icono, tam_texto) = if lista { (15.0, 13.5) } else { (17.0, 14.0) };
        p.text(
            rect.left_center() + Vec2::new(10.0, 0.0),
            Align2::LEFT_CENTER,
            "🗀",
            FontId::proportional(tam_icono),
            AMBAR,
        );
        p.text(
            rect.left_center() + Vec2::new(34.0, 0.0),
            Align2::LEFT_CENTER,
            recortar(&entrada.nombre, 48),
            FontId::proportional(tam_texto),
            TEXTO,
        );
        if resp.clicked() {
            self.explorador.abrir(entrada.ruta.clone());
        }
        let anclada = self.navegacion.anclada(&entrada.ruta);
        resp.context_menu(|ui| {
            if ui.button("🗁 Abrir").clicked() {
                self.explorador.abrir(entrada.ruta.clone());
                ui.close();
            }
            let t = if anclada { "📌 Quitar de acceso rápido" } else { "📌 Anclar a acceso rápido" };
            if ui.button(t).clicked() {
                if anclada {
                    self.navegacion.desanclar(&entrada.ruta);
                } else {
                    self.navegacion.anclar(&entrada.ruta);
                }
                ui.close();
            }
        });
    }

    fn fila_video(
        &mut self,
        ui: &mut egui::Ui,
        e: &Instantanea,
        entrada: &Entrada,
        mostrar_carpeta: bool,
        lista: bool,
    ) -> egui::Rect {
        let clip = self.clip_de(&entrada.ruta, e);
        let medio = clip.as_deref().and_then(|c| e.medio(c));
        let duracion = medio.map(|m| m.duracion_seg).or(entrada.duracion.map(|d| d.as_secs_f64()));
        let seleccionado = self.archivo_elegido(&entrada.ruta);
        let hay_varios = self.archivos_multi.len() > 1;
        let eligiendo = !self.archivos_multi.is_empty();

        let alto = if lista { 26.0 } else { 50.0 };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), alto), Sense::click_and_drag());
        if resp.drag_started() {
            // Si se arrastra uno de varios elegidos, van todos (en su orden).
            if hay_varios && seleccionado {
                let lista_arr = self.archivos_elegidos();
                let nombre = format!("{} videos", lista_arr.len());
                egui::DragAndDrop::set_payload(ui.ctx(), Arrastre::Archivos { lista: lista_arr, nombre });
            } else {
                self.archivos_multi.clear();
                self.explorador.seleccion = Some(entrada.ruta.clone());
                egui::DragAndDrop::set_payload(
                    ui.ctx(),
                    Arrastre::Archivo {
                        ruta: entrada.ruta.clone(),
                        duracion: entrada.duracion,
                        nombre: entrada.nombre.clone(),
                    },
                );
            }
        }
        let encima = resp.hovered();
        let fondo = if seleccionado {
            ACENTO_OSCURO
        } else if encima {
            TARJETA_HOVER
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, if lista { 4.0 } else { 6.0 }, fondo);
        if seleccionado {
            ui.painter().rect_stroke(rect, if lista { 4.0 } else { 6.0 }, Stroke::new(1.0, ACENTO), StrokeKind::Inside);
        }

        // Casilla a la izquierda: clic para sumar o quitar; arrastrar por las casillas
        // elige varios. Se ve al pasar el mouse o si hay varios elegidos.
        let casilla = egui::Rect::from_min_size(rect.min, Vec2::new(24.0, rect.height()));
        let r_casilla =
            ui.interact(casilla, egui::Id::unique(("casilla_archivo", &entrada.ruta)), Sense::click_and_drag());
        if eligiendo || r_casilla.hovered() {
            let c = casilla.center();
            let caja = egui::Rect::from_center_size(c, Vec2::splat(13.0));
            let marcado = seleccionado;
            ui.painter().rect_stroke(
                caja,
                3.0,
                Stroke::new(1.2, if marcado { ACENTO } else { TENUE }),
                StrokeKind::Inside,
            );
            if marcado {
                ui.painter().rect_filled(caja.shrink(1.0), 2.0, ACENTO);
                ui.painter().text(c, Align2::CENTER_CENTER, "✔", FontId::proportional(10.0), Color32::WHITE);
            }
        }
        let r_casilla = r_casilla.on_hover_text("Clic: elegir o quitar · Arrastrar por aquí: elegir varios");
        if r_casilla.clicked() {
            // La casilla siempre suma o quita (como Ctrl+clic).
            let mut mods = ui.input(|i| i.modifiers);
            mods.command = !mods.shift;
            mods.ctrl = mods.command;
            self.clic_en_archivo(&entrada.ruta, mods);
        }
        if r_casilla.drag_started() {
            self.empezar_marco_archivos(ui);
        }

        let p = ui.painter();
        let derecha = rect.right() - 8.0;
        let texto_dur = duracion.map(mmss).unwrap_or_else(|| "--:--".into());
        let (insignia, color) = self.insignia(clip.is_some(), e);
        if lista {
            // Una línea: ▶ nombre ······ etiqueta · duración.
            let x = casilla.right() + 2.0;
            p.text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, "🎞", FontId::proportional(12.0), TENUE);
            p.text(
                egui::pos2(derecha, rect.center().y),
                Align2::RIGHT_CENTER,
                &texto_dur,
                FontId::monospace(13.0),
                TEXTO,
            );
            // Si la columna es angosta, la etiqueta queda como un punto de color.
            let angosta = rect.width() < 360.0;
            let ancho_insignia = if angosta { 12.0 } else { 78.0 };
            if angosta {
                p.circle_filled(egui::pos2(derecha - 56.0, rect.center().y), 3.5, color);
            } else {
                p.text(
                    egui::pos2(derecha - 52.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    insignia,
                    FontId::proportional(10.5),
                    color,
                );
            }
            let ancho_texto = (derecha - 52.0 - ancho_insignia - (x + 22.0)).max(40.0);
            let nombre = if mostrar_carpeta {
                let carpeta = entrada.ruta.parent().and_then(|c| c.file_name()).map(|c| c.to_string_lossy());
                match carpeta {
                    Some(c) => format!("{} — {c}", entrada.nombre),
                    None => entrada.nombre.clone(),
                }
            } else {
                entrada.nombre.clone()
            };
            p.text(
                egui::pos2(x + 22.0, rect.center().y),
                Align2::LEFT_CENTER,
                recortar_ancho(ui, &nombre, &FontId::proportional(13.5), ancho_texto),
                FontId::proportional(13.5),
                if seleccionado { Color32::WHITE } else { TEXTO },
            );
        } else {
            let mini = egui::Rect::from_min_size(rect.min + Vec2::new(26.0, 5.0), Vec2::new(71.0, 40.0));
            let clave = entrada.ruta.to_string_lossy().into_owned();
            self.pintar_miniatura(ui, mini, clip.as_deref(), &clave, encima);
            let p = ui.painter();
            let x = mini.right() + 10.0;
            p.text(
                egui::pos2(derecha, rect.top() + 15.0),
                Align2::RIGHT_CENTER,
                &texto_dur,
                FontId::monospace(14.0),
                TEXTO,
            );
            p.text(
                egui::pos2(derecha, rect.bottom() - 13.0),
                Align2::RIGHT_CENTER,
                insignia,
                FontId::proportional(11.0),
                color,
            );
            let ancho_texto = (derecha - 70.0 - x).max(40.0);
            let max_car = ((ancho_texto / 7.4) as usize).max(8);
            p.text(
                egui::pos2(x, rect.top() + 15.0),
                Align2::LEFT_CENTER,
                recortar_ancho(ui, &entrada.nombre, &FontId::proportional(14.0), ancho_texto),
                FontId::proportional(14.0),
                if seleccionado { Color32::WHITE } else { TEXTO },
            );
            let sub = if mostrar_carpeta {
                entrada.ruta.parent().map(|c| c.display().to_string()).unwrap_or_default()
            } else {
                tamano_legible(entrada.tamano)
            };
            p.text(
                egui::pos2(x, rect.bottom() - 13.0),
                Align2::LEFT_CENTER,
                recortar_inicio(&sub, max_car + 4),
                FontId::proportional(11.5),
                TENUE,
            );
        }

        let resp = resp.on_hover_text(format!(
            "{}\n{} · {}\nArrástralo a la tanda · doble clic: añadir al final",
            entrada.ruta.display(),
            texto_dur,
            tamano_legible(entrada.tamano)
        ));
        if resp.clicked() {
            let mods = ui.input(|i| i.modifiers);
            self.clic_en_archivo(&entrada.ruta, mods);
        }
        if resp.double_clicked() {
            self.agregar_archivo(&entrada.ruta, entrada.duracion, Destino::Final);
        }
        // Clic derecho sobre uno que no estaba elegido: queda elegido solo él.
        if resp.secondary_clicked() && !seleccionado {
            self.archivos_multi.clear();
            self.explorador.seleccion = Some(entrada.ruta.clone());
        }
        let varios = self.archivos_multi.len() > 1 && self.archivos_multi.contains(&entrada.ruta);
        let rect_fila = rect;
        resp.context_menu(|ui| {
            if varios {
                let n = self.archivos_multi.len();
                ui.label(RichText::new(format!("{n} videos elegidos")).size(12.0).color(TENUE));
                ui.separator();
                if ui.button(format!("+ Añadir los {n} a la tanda")).clicked() {
                    self.agregar_elegidos(Destino::Final);
                    ui.close();
                }
                if ui.button("⏭ Que salgan a continuación").clicked() {
                    self.agregar_elegidos(Destino::Siguiente);
                    ui.close();
                }
                ui.separator();
                if ui.button("Dejar de elegir").clicked() {
                    self.archivos_multi.clear();
                    ui.close();
                }
                return;
            }
            if ui.button("+ Añadir a la tanda").clicked() {
                self.agregar_archivo(&entrada.ruta, entrada.duracion, Destino::Final);
                ui.close();
            }
            if ui.button("⏭ Poner como siguiente").clicked() {
                self.agregar_archivo(&entrada.ruta, entrada.duracion, Destino::Siguiente);
                ui.close();
            }
            if ui.button(RichText::new("⏵ Al aire ahora").color(AIRE)).clicked() {
                self.agregar_archivo(&entrada.ruta, entrada.duracion, Destino::AlAire);
                ui.close();
            }
            if ui.button("👁 Vista previa").clicked() {
                self.abrir_vista_previa(entrada.ruta.clone(), entrada.duracion);
                ui.close();
            }
            if let (Some(c), Some(m)) = (clip.as_deref(), medio) {
                if ui.button("🕓 Programar este spot").clicked() {
                    let hoy = self.hoy();
                    self.publicidades.nueva_con_video(hoy, c, Duration::from_secs_f64(m.duracion_seg.max(0.0)));
                    self.pantalla = Pantalla::Publicidades;
                    ui.close();
                }
            }
            if mostrar_carpeta {
                if let Some(c) = entrada.ruta.parent() {
                    if ui.button("🗀 Ir a su carpeta").clicked() {
                        self.explorador.abrir(c.to_path_buf());
                        ui.close();
                    }
                }
            }
        });
        rect_fila
    }
}

// ======================================================================
// Columna PAUTA DE HOY + REGISTRO
// ======================================================================

impl Ventana {
    fn columna_pauta(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        ui.horizontal(|ui| {
            tema::titulo(ui, "PAUTA DE HOY");
            let hoy = self.hora(time::macros::format_description!("[day]/[month]"));
            ui.label(RichText::new(hoy).size(12.0).color(TENUE));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(egui::Button::new("🕓 Publicidades").fill(ACENTO_OSCURO))
                    .on_hover_text("Registrar publicidades y sus horarios (F5)")
                    .clicked()
                {
                    self.pantalla = Pantalla::Publicidades;
                }
            });
        });
        ui.add_space(4.0);

        // Alertas (contratos por vencer, videos que faltan): las primeras dos.
        if !e.alertas.is_empty() {
            let grave = e.alertas.iter().any(|a| a.grave);
            tema::marco_tarjeta().stroke(Stroke::new(1.0, if grave { AIRE } else { AMBAR })).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                for a in e.alertas.iter().take(2) {
                    let c = if a.grave { AIRE } else { AMBAR };
                    ui.label(RichText::new(format!("⚠ {}", a.texto)).size(12.0).color(c));
                }
                if e.alertas.len() > 2 {
                    let t = format!("Ver las {} alertas en Reportes", e.alertas.len());
                    if ui.link(RichText::new(t).size(12.0)).clicked() {
                        self.al_abrir_reportes();
                        self.pantalla = Pantalla::Reportes;
                    }
                }
            });
            ui.add_space(4.0);
        }

        let total = e.pases_hoy.len();
        let emitidos = e.pases_hoy.iter().filter(|p| p.estado == EstadoPase::Emitido).count();
        let perdidos = e.pases_hoy.iter().filter(|p| p.estado == EstadoPase::Perdido).count();
        let pendientes = e.pases_hoy.iter().filter(|p| p.estado == EstadoPase::Pendiente).count();
        if total > 0 {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{emitidos}/{total} emitidos")).size(12.0).color(TV));
                if pendientes > 0 {
                    ui.label(RichText::new(format!("· {pendientes} pendientes")).size(12.0).color(AMBAR))
                        .on_hover_text(
                            "No salieron en su horario (o se cortaron): en AUTO entran solos en el próximo corte",
                        );
                }
                if perdidos > 0 {
                    ui.label(RichText::new(format!("· ⚠ {perdidos} perdidos")).size(12.0).color(AIRE));
                }
            });
        }

        let alto_pauta = (ui.available_height() * 0.55).max(140.0);
        egui::ScrollArea::vertical().id_salt("pauta_hoy").auto_shrink([false, false]).max_height(alto_pauta).show(
            ui,
            |ui| {
                if e.pases_hoy.is_empty() {
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("🕓").size(30.0).color(MUY_TENUE));
                        let t = if e.pautas.is_empty() { "Todavía no hay pautas" } else { "Hoy no hay pases" };
                        ui.label(RichText::new(t).color(TENUE));
                        ui.label(RichText::new("Regístralas en Publicidades (F5).").size(12.0).color(MUY_TENUE));
                    });
                    return;
                }
                // El próximo o el que está en ventana queda a la vista.
                let foco =
                    e.pases_hoy.iter().position(|p| matches!(p.estado, EstadoPase::Vigente | EstadoPase::Programado));
                for (i, p) in e.pases_hoy.iter().enumerate() {
                    let r = self.fila_pase(ui, p);
                    if Some(i) == foco && !self.pauta_desplazada {
                        r.scroll_to_me(Some(Align::Center));
                        self.pauta_desplazada = true;
                    }
                }
            },
        );

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            tema::titulo(ui, "REGISTRO DE EMISIÓN");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let emitidos = e.registro.iter().filter(|r| r.tipo == TipoEvento::AlAire).count();
                ui.label(RichText::new(format!("{emitidos} al aire")).size(12.0).color(TENUE));
            });
        });
        ui.add_space(2.0);

        egui::ScrollArea::vertical().id_salt("registro").auto_shrink([false, false]).show(ui, |ui| {
            if e.registro.is_empty() {
                ui.add_space(10.0);
                ui.label(RichText::new("Todavía no hay eventos.").color(MUY_TENUE));
            }
            for ev in &e.registro {
                let color = match ev.tipo {
                    TipoEvento::AlAire => AIRE,
                    TipoEvento::Tv => TV,
                    TipoEvento::Aviso => AMBAR,
                    TipoEvento::Info => ACENTO,
                };
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new(&ev.hora).font(FontId::monospace(13.0)).color(TENUE));
                    let (r, _) = ui.allocate_exact_size(Vec2::new(8.0, 16.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 3.5, color);
                    ui.add(egui::Label::new(RichText::new(&ev.texto).size(13.0)).wrap());
                });
            }
        });
    }

    fn fila_pase(&mut self, ui: &mut egui::Ui, p: &PaseVista) -> egui::Response {
        let (color, etiqueta) = match p.estado {
            EstadoPase::Emitido => (TV, "✔"),
            EstadoPase::Vigente => (AMBAR, "AHORA"),
            EstadoPase::Pendiente => (AMBAR, "PENDIENTE"),
            EstadoPase::Perdido => (AIRE, "PERDIDO"),
            EstadoPase::Programado => (MUY_TENUE, ""),
        };
        let fondo = match p.estado {
            EstadoPase::Vigente => AMBAR_OSCURO,
            _ => Color32::TRANSPARENT,
        };
        let texto_color = if p.estado == EstadoPase::Emitido { TENUE } else { TEXTO };
        let r = egui::Frame::new()
            .fill(fondo)
            .corner_radius(CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(6, 3))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(crate::tema::hora_txt(&p.hora)).font(FontId::monospace(14.0)).color(texto_color),
                    );
                    if p.prioridad > 0 {
                        ui.label(RichText::new("★".repeat(p.prioridad as usize)).color(AMBAR));
                    }
                    let ancho = (ui.available_width() - 110.0).max(40.0);
                    ui.allocate_ui_with_layout(Vec2::new(ancho, 18.0), Layout::left_to_right(Align::Center), |ui| {
                        ui.set_width(ancho);
                        ui.add(egui::Label::new(RichText::new(&p.nombre).size(13.5).color(texto_color)).truncate());
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let puede =
                            matches!(p.estado, EstadoPase::Vigente | EstadoPase::Perdido | EstadoPase::Pendiente)
                                && !p.en_tanda;
                        if puede && ui.small_button("+").on_hover_text("Añadir a la tanda").clicked() {
                            self.ctl.enviar(Orden::AgregarPase(p.clave.clone()));
                        }
                        if p.en_tanda && p.estado != EstadoPase::Emitido {
                            ui.label(RichText::new("☰").color(ACENTO)).on_hover_text("Ya está en la tanda");
                        }
                        if !etiqueta.is_empty() {
                            ui.label(
                                RichText::new(etiqueta).size(11.0).strong().family(crate::tema::negrita()).color(color),
                            );
                        }
                        if p.veces_hoy > 0 {
                            ui.label(RichText::new(format!("×{}", p.veces_hoy)).size(11.5).color(TENUE)).on_hover_text(
                                format!("Su video salió {} veces hoy (con relleno y repeticiones)", p.veces_hoy),
                            );
                        }
                    });
                });
            })
            .response;
        let extra = if p.estado == EstadoPase::Pendiente {
            "\nNo salió en su horario (o se cortó): en AUTO entra solo en el próximo corte"
        } else {
            ""
        };
        r.on_hover_text(format!("{} · {}{extra}", p.clip, mmss(p.duracion.as_secs_f64())))
    }

    fn mostrar_aviso(&mut self, ui: &mut egui::Ui) {
        let Some((texto, desde, error)) = &self.aviso else {
            return;
        };
        let pasado = desde.elapsed();
        let duracion = if *error { DURACION_ERROR } else { DURACION_AVISO };
        if pasado > duracion {
            self.aviso = None;
            return;
        }
        let texto = texto.clone();
        let error = *error;
        // Entra deslizándose desde abajo y se desvanece al final.
        let (entrada, salida) = if dialogo::animaciones() {
            let entrada = dialogo::suave(pasado.as_secs_f32() / 0.2);
            let resta = (duracion - pasado).as_secs_f32();
            (entrada, (resta / 0.3).clamp(0.0, 1.0))
        } else {
            (1.0, 1.0)
        };
        let opacidad = entrada * salida;
        let (fondo, borde, icono, color_icono) = if error {
            (Color32::from_rgb(0x2E, 0x13, 0x16), AIRE, "⚠", AMBAR)
        } else {
            (Color32::from_rgb(0x10, 0x1F, 0x36), ACENTO, "ℹ", ACENTO)
        };
        let mut cerrar = false;
        let mut encima = false;
        let frac_restante = 1.0 - pasado.as_secs_f32() / duracion.as_secs_f32();
        // Abajo a la derecha, encima de la barra de estado.
        egui::Area::new(egui::Id::unique("relevo-aviso"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -40.0 + (1.0 - entrada) * 24.0))
            .order(egui::Order::Foreground)
            .fade_in(false)
            .show(ui.ctx(), |ui| {
                ui.multiply_opacity(opacidad);
                let r = egui::Frame::new()
                    .fill(fondo)
                    .stroke(Stroke::new(1.0, borde))
                    .corner_radius(CornerRadius::same(8))
                    .shadow(egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(120) })
                    .inner_margin(egui::Margin { left: 10, right: 8, top: 6, bottom: 5 })
                    .show(ui, |ui| {
                        ui.set_max_width(340.0);
                        let fila = ui
                            .horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.label(RichText::new(icono).size(13.0).color(color_icono));
                                ui.add(egui::Label::new(RichText::new(texto).size(12.5).color(TEXTO)).wrap());
                                let b = egui::Button::new(RichText::new("✖").size(10.5).color(TENUE)).frame(false);
                                if ui.add(b).on_hover_text("Cerrar").clicked() {
                                    cerrar = true;
                                }
                            })
                            .response
                            .rect
                            .width();
                        // Tiempo que le queda al aviso.
                        let (barra, _) = ui.allocate_exact_size(Vec2::new(fila, 2.0), Sense::hover());
                        let mut lleno = barra;
                        lleno.set_width(barra.width() * frac_restante.clamp(0.0, 1.0));
                        ui.painter().rect_filled(lleno, 1.0, borde.gamma_multiply(0.7));
                    });
                encima = r.response.contains_pointer();
            });
        if cerrar {
            self.aviso = None;
        } else if encima {
            // Mientras el mouse está encima, el aviso no se va.
            if let Some((_, desde, _)) = &mut self.aviso {
                *desde = Instant::now() - Duration::from_millis(400);
            }
        }
        // Fluido al entrar y al salir; en medio basta con mover la barrita del tiempo.
        let animando = pasado < Duration::from_millis(300) || duracion - pasado < Duration::from_millis(400);
        ui.ctx().request_repaint_after(Duration::from_millis(if animando { 16 } else { 120 }));
    }
}

// ======================================================================
// Piezas reutilizables
// ======================================================================

fn pastilla(ui: &mut egui::Ui, texto: &str, fondo: Color32, color: Color32, tam: f32) {
    egui::Frame::new()
        .fill(fondo)
        .corner_radius(CornerRadius::same(99))
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(texto).size(tam).strong().family(crate::tema::negrita()).color(color));
        });
}

/// Lo que muestra la lista de Archivos (carpeta abierta o búsqueda) y su resumen.
struct DatosArchivos {
    entradas: Option<Vec<Entrada>>,
    estado: Option<String>,
    buscando: bool,
}

/// Marco de elección en Archivos.
struct MarcoArchivos {
    x0: f32,
    y0: f32,
    base: Vec<PathBuf>,
}

/// Marco de elección en la tanda (coordenadas del contenido, así sigue al desplazar).
struct MarcoTanda {
    x0: f32,
    y0: f32,
    /// Lo que ya estaba elegido (con Ctrl) y se suma.
    base: Vec<u64>,
}

/// A dónde mover los spots elegidos de la tanda.
#[derive(Clone, Copy)]
enum Movida {
    Subir,
    Bajar,
    Principio,
    Final,
    /// Justo después del que está al aire.
    Siguiente,
}

/// Carpeta abierta en el explorador al cerrar Relevo (`datos/ultima_carpeta.json`).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct UltimaCarpeta {
    carpeta: Option<PathBuf>,
    /// Carpeta dentro de la lista de CasparCG (modo remoto).
    carpeta_caspar: String,
    /// Orden de la lista de archivos.
    orden: crate::explorador::Orden,
    /// Archivos como lista (sin miniaturas).
    vista_lista: bool,
    /// Tanda como lista (sin miniaturas).
    tanda_lista: bool,
}

impl UltimaCarpeta {
    fn archivo(datos: &Path) -> PathBuf {
        datos.join("ultima_carpeta.json")
    }

    fn leer(datos: &Path) -> Self {
        std::fs::read_to_string(Self::archivo(datos))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn guardar(&self, datos: &Path) {
        let _ = std::fs::create_dir_all(datos);
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::archivo(datos), t);
        }
    }
}

fn boton_icono(icono: &str, tam: Vec2, k: f32) -> egui::Button<'static> {
    egui::Button::new(RichText::new(icono.to_string()).size(22.0 * k))
        .min_size(tam)
        .corner_radius(CornerRadius::same(8))
}

fn boton_chico(icono: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(icono.to_string()).size(12.0)).min_size(Vec2::new(24.0, 22.0)).frame(false)
}

fn alternador(texto: &str, activo: bool, k: f32) -> egui::Button<'static> {
    let (fondo, borde, color) = if activo { (ACENTO_OSCURO, ACENTO, TEXTO) } else { (TARJETA, BORDE, TENUE) };
    egui::Button::new(RichText::new(texto.to_string()).size(14.0 * k).color(color))
        .fill(fondo)
        .stroke(Stroke::new(1.0, borde))
        .min_size(Vec2::new(0.0, 36.0 * k))
}

fn barra_progreso(ui: &mut egui::Ui, frac: f32, fase: FaseVista, alto: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), alto), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 5.0, Color32::from_rgb(0x0C, 0x0E, 0x13));
    p.rect_stroke(rect, 5.0, Stroke::new(1.0, BORDE), StrokeKind::Inside);
    if frac > 0.0 {
        let mut lleno = rect.shrink(2.0);
        lleno.set_width(lleno.width() * frac);
        let color = if fase == FaseVista::Pausado { AMBAR } else { AIRE };
        p.rect_filled(lleno, 4.0, color);
    }
    // Marcas cada 10 %.
    for i in 1..10 {
        let x = rect.left() + rect.width() * i as f32 / 10.0;
        p.line_segment(
            [egui::pos2(x, rect.bottom() - 4.0), egui::pos2(x, rect.bottom() - 2.0)],
            Stroke::new(1.0, Color32::from_white_alpha(30)),
        );
    }
}

/// Miniatura provisional: un fondo de color según el nombre y un ícono de video.
/// (Las miniaturas reales se pedirán a CasparCG en la etapa 2.)
fn miniatura(p: &egui::Painter, rect: egui::Rect, nombre: &str, resaltar: bool) {
    let base = tema::color_de(nombre);
    p.rect_filled(rect, 6.0, base);
    // Degradado simple: franja más oscura abajo.
    let mut abajo = rect;
    abajo.set_top(rect.center().y);
    p.rect_filled(abajo, CornerRadius { nw: 0, ne: 0, sw: 6, se: 6 }, Color32::from_black_alpha(60));
    let alfa = if resaltar { 230 } else { 150 };
    p.circle_filled(rect.center(), rect.height() * 0.2, Color32::from_black_alpha(110));
    p.text(
        rect.center() + Vec2::new(2.0, 0.0),
        Align2::CENTER_CENTER,
        "⏵",
        FontId::proportional(rect.height() * 0.26),
        Color32::from_white_alpha(alfa),
    );
    p.rect_stroke(rect, 6.0, Stroke::new(1.0, Color32::from_white_alpha(18)), StrokeKind::Inside);
}

fn mensaje_vacio(ui: &mut egui::Ui, icono: &str, titulo: &str, detalle: &str) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(icono).size(40.0).color(MUY_TENUE));
        ui.label(RichText::new(titulo).size(17.0).color(TENUE));
        ui.label(RichText::new(detalle).size(13.0).color(MUY_TENUE));
    });
}

fn dato(ui: &mut egui::Ui, etiqueta: &str, valor: &str) {
    ui.label(RichText::new(etiqueta).size(12.0).color(TENUE));
    ui.label(RichText::new(valor).size(13.0).strong().family(crate::tema::negrita()));
}

impl Ventana {
    /// Revisa cada tanto si hay versión nueva de Relevo y, si se pidió actualizar,
    /// instala en cuanto está bajada y no hay publicidad al aire.
    fn seguir_actualizacion(&mut self, ctx: &egui::Context, e: &Instantanea) {
        use crate::actualizacion as act;
        if !self.cfg.actualizacion.activa || act::es_desarrollo() {
            return;
        }
        let a = self.act.lock().expect("actualizacion").clone();
        let toca = a.revisado.is_none_or(|t| t.elapsed() > act::CADA);
        if toca && !a.consultando && a.descargando.is_none() {
            act::consultar(self.act.clone(), self.cfg.actualizacion.clone(), ctx.clone());
        }
        if a.hay_nueva() {
            let n = a.ultima.as_ref().map(|u| u.numero.clone());
            if n != self.act_avisada {
                self.act_avisada = n.clone();
                self.avisar_local(format!("⬆ Hay una versión nueva de Relevo: {}", n.unwrap_or_default()));
            }
        }
        if !self.act_instalar {
            return;
        }
        if let Some(err) = a.error.clone().filter(|_| a.descargando.is_none() && a.listo.is_none()) {
            self.act_instalar = false;
            self.avisar_error(format!("No se pudo actualizar Relevo: {err}"));
            return;
        }
        // Nunca con publicidad al aire (en la PC principal).
        let libre = self.ctl.es_remoto() || e.fase == FaseVista::Tv;
        if let (Some(instalador), true) = (a.listo.clone(), libre) {
            self.act_instalar = false;
            match act::instalar(&instalador) {
                Ok(()) => {
                    tracing::info!("actualizando Relevo con {}", instalador.display());
                    self.cierre = Some(false);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Err(m) => self.avisar_error(m),
            }
        }
    }

    /// Relevo en la barra de abajo: versión, aviso de versión nueva y actualización.
    fn estado_relevo(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        use crate::actualizacion as act;
        let a = self.act.lock().expect("actualizacion").clone();
        let nueva = a.ultima.clone().filter(|_| a.hay_nueva());
        let (texto, color) = match (&a.descargando, &nueva) {
            (Some(Some(p)), _) => (format!("⬇ Relevo {:.0} % · {}", p * 100.0, a.detalle), ACENTO),
            (Some(None), _) => ("⬇ Bajando Relevo…".to_string(), ACENTO),
            _ if self.act_instalar && a.listo.is_some() => ("⬆ Se instala al terminar la tanda".to_string(), AMBAR),
            (None, Some(n)) => (format!("⬆ Relevo {} disponible", n.numero), ACENTO),
            (None, None) => (format!("Relevo {}", act::VERSION), MUY_TENUE),
        };
        let r = ui.add(egui::Button::new(RichText::new(texto).size(12.0).color(color)).frame(false));
        if a.descargando.is_some() || a.consultando {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        let r = r.on_hover_text("Versión de Relevo. Clic: buscar o instalar una versión nueva");
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(300.0);
            ui.label(RichText::new(format!("Relevo {}", act::VERSION)).strong().family(crate::tema::negrita()));
            if act::es_desarrollo() {
                ui.label(RichText::new("Copia compilada a mano: no se actualiza sola.").size(11.5).color(TENUE));
                return;
            }
            if let Some(err) = &a.error {
                ui.label(RichText::new(err).size(11.5).color(AIRE));
            }
            match &nueva {
                Some(n) => {
                    ui.label(
                        RichText::new(format!("Hay una versión nueva: {}", n.numero))
                            .size(13.0)
                            .color(ACENTO)
                            .family(crate::tema::negrita()),
                    );
                    if !n.notas.is_empty() {
                        let notas: String = n.notas.lines().take(12).collect::<Vec<_>>().join("\n");
                        egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                            ui.label(RichText::new(notas).size(11.5).color(TENUE));
                        });
                    }
                    ui.separator();
                    let al_aire = !self.ctl.es_remoto() && e.fase != FaseVista::Tv;
                    let ocupado = a.descargando.is_some() || self.act_instalar;
                    let mb = n.tamano as f64 / 1_048_576.0;
                    let b = egui::Button::new(format!("⬆ Actualizar ahora ({mb:.0} MB)"));
                    let r = ui
                        .add_enabled(!ocupado, b)
                        .on_hover_text("Baja el instalador y lo ejecuta: Relevo se cierra y se vuelve a abrir solo. Se conservan config.toml y los datos.");
                    if r.clicked() {
                        self.act_instalar = true;
                        if a.listo.is_none() {
                            act::descargar(self.act.clone(), self.cfg.actualizacion.clone(), ui.ctx().clone());
                        }
                        if al_aire {
                            self.avisar_local("La actualización se instalará cuando termine la tanda");
                        }
                        ui.close();
                    }
                    if al_aire {
                        ui.label(
                            RichText::new("Con publicidad al aire no se instala: se hará al terminar la tanda.")
                                .size(11.0)
                                .color(AMBAR),
                        );
                    }
                    if !n.pagina.is_empty() && ui.button("🌐 Ver la versión en GitHub").clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&n.pagina));
                        ui.close();
                    }
                }
                None => {
                    let t = if a.consultando {
                        "Revisando…"
                    } else if a.revisado.is_some() && a.error.is_none() {
                        "✔ Al día"
                    } else {
                        ""
                    };
                    if !t.is_empty() {
                        ui.label(RichText::new(t).size(12.0).color(TV));
                    }
                }
            }
            if ui
                .add_enabled(!a.consultando && a.descargando.is_none(), egui::Button::new("🔍 Buscar actualización"))
                .clicked()
            {
                act::consultar(self.act.clone(), self.cfg.actualizacion.clone(), ui.ctx().clone());
            }
        });
    }

    /// ffmpeg en la barra de abajo: versión, descarga y actualización.
    fn estado_ffmpeg(&mut self, ui: &mut egui::Ui) {
        let f = self.ffmpeg.lock().expect("ffmpeg").clone();
        let (texto, color) = match (&f.descargando, &f.ruta) {
            (Some(Some(p)), _) => (format!("⬇ ffmpeg {:.0} % · {}", p * 100.0, f.detalle_descarga), ACENTO),
            (Some(None), _) => ("⬇ Descargando ffmpeg…".to_string(), ACENTO),
            (None, None) => ("⚠ Falta ffmpeg".to_string(), AMBAR),
            (None, Some(_)) if f.hay_actualizacion() => {
                (format!("🎞 ffmpeg: hay versión {}", f.ultima.clone().unwrap_or_default()), ACENTO)
            }
            (None, Some(_)) => (format!("🎞 ffmpeg {}", f.version.clone().unwrap_or_default()), TENUE),
        };
        let r = ui.add(egui::Button::new(RichText::new(texto).size(12.0).color(color)).frame(false));
        if f.descargando.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        let ayuda = match (&f.error, &f.ruta) {
            (Some(e), _) => format!("{e}\nClic: opciones"),
            (None, Some(r)) => format!("{}\nClic: buscar actualización", r.display()),
            (None, None) => {
                "ffmpeg hace falta para la vista previa, el monitor y la Biblioteca. Clic: descargarlo".into()
            }
        };
        let r = r.on_hover_text(ayuda);
        egui::Popup::menu(&r).show(|ui| {
            match &f.ruta {
                Some(ruta) => {
                    ui.label(
                        RichText::new(format!("ffmpeg {}", f.version.clone().unwrap_or_default()))
                            .strong()
                            .family(crate::tema::negrita()),
                    );
                    ui.label(RichText::new(ruta.display().to_string()).size(11.0).color(TENUE));
                }
                None => {
                    ui.label(
                        RichText::new("No se encontró ffmpeg en esta PC")
                            .strong()
                            .family(crate::tema::negrita())
                            .color(AMBAR),
                    );
                }
            }
            if let Some(err) = &f.error {
                ui.label(RichText::new(err).size(11.5).color(AIRE));
            }
            ui.separator();
            let ocupado = f.descargando.is_some();
            if f.ruta.is_none() {
                if ui.add_enabled(!ocupado, egui::Button::new("⬇ Descargar ffmpeg (~35 MB)")).clicked() {
                    crate::ffmpeg::descargar(self.ffmpeg.clone(), ui.ctx().clone());
                    ui.close();
                }
            } else {
                if f.hay_actualizacion() {
                    let t = format!("⬇ Actualizar a la versión {}", f.ultima.clone().unwrap_or_default());
                    if ui.add_enabled(!ocupado, egui::Button::new(t)).clicked() {
                        crate::ffmpeg::descargar(self.ffmpeg.clone(), ui.ctx().clone());
                        ui.close();
                    }
                } else if let Some(u) = &f.ultima {
                    ui.label(RichText::new(format!("✔ Al día (última versión: {u})")).size(12.0).color(TV));
                }
                if ui.add_enabled(!f.consultando && !ocupado, egui::Button::new("🔍 Buscar actualización")).clicked()
                {
                    crate::ffmpeg::consultar(self.ffmpeg.clone(), ui.ctx().clone());
                }
                if f.consultando {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(RichText::new("Consultando…").size(12.0).color(TENUE));
                    });
                }
            }
            ui.label(
                RichText::new(
                    "Se descarga de gyan.dev (versión oficial para Windows) y queda en\n%LOCALAPPDATA%\\Relevo\\ffmpeg",
                )
                .size(11.0)
                .color(MUY_TENUE),
            );
        });
    }
}

/// `respaldo-2026-11-11-1950.zip` → `11/11 19:50`; `respaldo-2026-11-11.zip` → `11/11`.
fn fecha_respaldo(nombre: &str) -> String {
    let n = nombre.trim_start_matches("respaldo-").trim_end_matches(".zip");
    let p: Vec<&str> = n.split('-').collect();
    match p.as_slice() {
        [_, m, d, hm] if hm.len() == 4 => format!("{d}/{m} {}:{}", &hm[..2], &hm[2..]),
        [_, m, d] => format!("{d}/{m}"),
        _ => n.to_string(),
    }
}

/// Separa `"CARPETA/SUB/NOMBRE"` en `("CARPETA/SUB/", "NOMBRE")`.
fn partir(clip: &str) -> (&str, &str) {
    match clip.rfind('/') {
        Some(i) => (&clip[..=i], &clip[i + 1..]),
        None => ("", clip),
    }
}

fn nombre_corto(clip: &str) -> &str {
    partir(clip).1
}

/// `1:02:03` o `4:05` (para totales que pueden pasar de la hora).
fn duracion_larga(seg: f64) -> String {
    let s = seg.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        mmss(seg)
    }
}

/// Tamaño de todo lo que se dibuja. Automático: la interfaz está pensada para unos
/// 1480 × 860 puntos; si la ventana es más chica (pantalla de 1366, o 1080 con la escala
/// de Windows al 125 %), se achica lo justo para que quepa (hasta un 70 %).
fn ajustar_escala(ctx: &egui::Context, escala: u32) {
    const ANCHO: f32 = 1480.0;
    const ALTO: f32 = 860.0;
    // Tamaño real de la ventana en píxeles (de este mismo cuadro) y la escala de Windows.
    // Así el cálculo no depende del zoom que ya está puesto y no se realimenta.
    let (fisico, nativo) = ctx.input(|i| {
        let r = i.viewport_rect();
        (r.size() * i.pixels_per_point, i.viewport().native_pixels_per_point.unwrap_or(i.pixels_per_point))
    });
    let clave = egui::Id::unique("relevo-escala");
    let anterior: Option<(u32, u32, u32)> = ctx.data(|d| d.get_temp(clave));
    let ahora = (fisico.x.round() as u32, fisico.y.round() as u32, escala);
    // Solo se recalcula cuando cambia el tamaño de la ventana o el ajuste.
    if anterior == Some(ahora) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(clave, ahora));
    let objetivo = if escala > 0 {
        (escala as f32 / 100.0).clamp(0.5, 2.0)
    } else {
        let (w, h) = (fisico.x / nativo, fisico.y / nativo);
        if w < 100.0 || h < 100.0 {
            return; // minimizada
        }
        // En pasos de 5 % (sin saltitos por un píxel de diferencia).
        let z = (w / ANCHO).min(h / ALTO).clamp(0.7, 1.0);
        (z * 20.0).floor() / 20.0
    };
    if (objetivo - ctx.zoom_factor()).abs() > 0.001 {
        ctx.set_zoom_factor(objetivo);
    }
}

/// Lo visible de una lista con barra vertical. (Su `clip_rect` no se recorta a lo
/// ancho: abarcaría las columnas vecinas y les quitaría los clics.)
fn vista_de_lista(ui: &egui::Ui) -> egui::Rect {
    let ancho = ui.max_rect();
    let mut v = ui.clip_rect();
    v.min.x = v.min.x.max(ancho.min.x);
    v.max.x = v.max.x.min(ancho.max.x);
    v
}

/// El texto recortado con «…» para que quepa en `ancho` puntos con esa letra.
fn recortar_ancho(ui: &egui::Ui, texto: &str, fuente: &FontId, ancho: f32) -> String {
    let medir = |t: &str| ui.fonts_mut(|f| f.layout_no_wrap(t.to_string(), fuente.clone(), Color32::WHITE).size().x);
    if medir(texto) <= ancho {
        return texto.to_string();
    }
    let letras: Vec<char> = texto.chars().collect();
    let (mut lo, mut hi) = (0usize, letras.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let t: String = letras[..mid].iter().collect::<String>() + "…";
        if medir(&t) <= ancho {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    letras[..lo].iter().collect::<String>().trim_end().to_string() + "…"
}

fn recortar(texto: &str, max: usize) -> String {
    if texto.chars().count() <= max {
        texto.to_string()
    } else {
        let corto: String = texto.chars().take(max - 1).collect();
        format!("{corto}…")
    }
}

/// Recorta por el inicio (útil para rutas largas): `…\\Clientes\\Farmacia`.
fn recortar_inicio(texto: &str, max: usize) -> String {
    let n = texto.chars().count();
    if n <= max {
        texto.to_string()
    } else {
        let fin: String = texto.chars().skip(n - (max - 1)).collect();
        format!("…{fin}")
    }
}

fn tamano_legible(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1_073_741_824.0 {
        format!("{:.1} GB", b / 1_073_741_824.0)
    } else if b >= 1_048_576.0 {
        format!("{:.1} MB", b / 1_048_576.0)
    } else {
        format!("{:.0} KB", (b / 1024.0).max(1.0))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn utilidades_de_texto() {
        assert_eq!(partir("A/B/C"), ("A/B/", "C"));
        assert_eq!(partir("C"), ("", "C"));
        assert_eq!(recortar("abcdef", 4), "abc…");
        assert_eq!(recortar_inicio("C:/videos/spots", 7), "…/spots");
        assert_eq!(tamano_legible(9_437_184), "9.0 MB");
        assert_eq!(mmss(90.4), "1:30");
    }
}

/// Subcarpetas (con cantidad de videos) y videos directos de `carpeta` dentro de la lista de CasparCG.
fn contenido_de<'a>(videos: &[&'a amcp::Medio], carpeta: &str) -> (Vec<(String, usize)>, Vec<&'a amcp::Medio>) {
    let mut sub: BTreeMap<String, usize> = BTreeMap::new();
    let mut clips = Vec::new();
    for m in videos {
        let Some(resto) = m.nombre.strip_prefix(carpeta) else { continue };
        match resto.find('/') {
            Some(i) => *sub.entry(resto[..i].to_string()).or_default() += 1,
            None => clips.push(*m),
        }
    }
    (sub.into_iter().collect(), clips)
}

#[cfg(test)]
mod pruebas_ultima {
    use super::*;

    #[test]
    fn recuerda_la_ultima_carpeta() {
        let dir = std::env::temp_dir().join(format!("relevo-ultima-{}", std::process::id()));
        assert_eq!(UltimaCarpeta::leer(&dir), UltimaCarpeta::default());
        let u = UltimaCarpeta {
            carpeta: Some(PathBuf::from("/videos/clientes")),
            carpeta_caspar: "PROMOS/".into(),
            orden: crate::explorador::Orden::Recientes,
            vista_lista: true,
            tanda_lista: true,
        };
        u.guardar(&dir);
        assert_eq!(UltimaCarpeta::leer(&dir), u);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod pruebas_caspar {
    use super::*;

    #[test]
    fn carpetas_virtuales_de_casparcg() {
        let ms: Vec<amcp::Medio> = ["CLIENTES/A/SPOT1", "CLIENTES/A/SPOT2", "CLIENTES/B", "ID"]
            .iter()
            .map(|n| amcp::Medio::desde_linea_cls(&format!("\"{n}\" MOVIE 1 20261110120000 300 1/30")).unwrap())
            .collect();
        let refs: Vec<&amcp::Medio> = ms.iter().collect();
        let (sub, clips) = contenido_de(&refs, "");
        assert_eq!(sub, vec![("CLIENTES".to_string(), 3)]);
        assert_eq!(clips.len(), 1);
        let (sub, clips) = contenido_de(&refs, "CLIENTES/");
        assert_eq!(sub, vec![("A".to_string(), 2)]);
        assert_eq!(clips[0].nombre, "CLIENTES/B");
    }
}

/// Cierra Relevo y lo vuelve a abrir (para aplicar los ajustes).
fn reiniciar_app(ctx: &egui::Context) {
    match std::env::current_exe() {
        Ok(exe) => {
            let r = std::process::Command::new(exe)
                .args(std::env::args_os().skip(1))
                .env("RELEVO_REINICIO", "1")
                .env(arranque::VAR_CASPAR_PROPIO, if arranque::caspar_es_propio() { "1" } else { "0" })
                .spawn();
            match r {
                Ok(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(e) => tracing::error!("no se pudo reiniciar Relevo: {e}"),
            }
        }
        Err(e) => tracing::error!("no se pudo reiniciar Relevo: {e}"),
    }
}
