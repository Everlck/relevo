//! Controlador: ejecuta las órdenes de la interfaz contra CasparCG en segundo plano.
//!
//! Hay un solo "cerebro" (el núcleo) en la PC principal. Las órdenes llegan con su
//! [`Origen`] (esta PC o una PC remota) y el núcleo decide si se permiten según el
//! rol del usuario y quién tiene el **mando** de la emisión.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::archivos;
use crate::biblioteca::{Estandar, Ficha, TipoTrabajo, VistaBiblioteca};
use crate::osc::{self, TiempoOsc};
use crate::pauta::{self, Almacen, Emision, EstadoPase, Pauta};
use crate::reportes::{self, Alerta, Reporte, Salida};
use crate::reproductor::{Accion, Bucle, Fase, ItemTanda, Reproductor, TRANSPARENTE};

#[path = "controlador_biblioteca.rs"]
mod biblio;

const MAX_REGISTRO: usize = 300;
const INTERVALO_TICK: Duration = Duration::from_millis(100);
const OSC_VIGENTE: Duration = Duration::from_secs(2);
/// Capa de CasparCG que se usa solo para comprobar si acepta rutas completas.
const CAPA_PRUEBA: u16 = 199;

/// Dónde se pone un spot que se añade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Destino {
    /// Al final de la tanda.
    Final,
    /// Justo después del que está al aire.
    Siguiente,
    /// Inmediatamente al aire.
    AlAire,
    /// En esa posición de la tanda (0 = arriba). Si es mayor que la tanda, al final.
    Posicion(usize),
}

/// MANUAL: el operador arma la tanda. AUTO: la tanda se arma sola con la pauta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModoTanda {
    Manual,
    Auto,
}

/// Qué hacer con los videos que están fuera de la carpeta de CasparCG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModoExternos {
    /// Reproducirlos desde donde están (si CasparCG lo acepta; si no, se copian).
    #[default]
    Directo,
    /// Copiarlos siempre a `media/IMPORTADOS`.
    Copiar,
}

/// Permisos de un usuario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Rol {
    /// Emite (con el mando) y ve todo.
    Operador,
    /// Además edita pautas y sube videos.
    Editor,
    /// Además gestiona usuarios y puede quitar el mando a otro.
    Admin,
}

impl Rol {
    pub fn nombre(self) -> &'static str {
        match self {
            Rol::Operador => "Operador",
            Rol::Editor => "Editor",
            Rol::Admin => "Administrador",
        }
    }
}

/// Quién envía una orden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origen {
    /// 0 = esta PC (la principal); otro número = una sesión remota.
    pub id: u64,
    pub nombre: String,
    pub rol: Rol,
}

impl Origen {
    pub fn local() -> Self {
        Origen { id: 0, nombre: "PC principal".into(), rol: Rol::Admin }
    }
}

/// Un pase de hoy, tal como lo muestra la interfaz.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaseVista {
    pub clave: String,
    pub hora: String,
    pub nombre: String,
    pub clip: String,
    pub duracion: Duration,
    pub prioridad: u8,
    pub estado: EstadoPase,
    /// Ya está en la tanda actual.
    pub en_tanda: bool,
    /// Cuántas veces salió hoy su video (contando relleno y repeticiones).
    #[serde(default)]
    pub veces_hoy: u32,
}

/// Lo que la interfaz (de esta PC o de una remota) le puede pedir al núcleo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Orden {
    /// Un clip que ya está en la carpeta de medios de CasparCG.
    Agregar {
        clip: String,
        duracion: Duration,
        destino: Destino,
    },
    /// Un archivo de la PC principal. Si está fuera de la carpeta de medios, se
    /// reproduce directo o se copia (según la configuración).
    AgregarArchivo {
        ruta: PathBuf,
        duracion: Option<Duration>,
        destino: Destino,
    },
    /// Reemplaza la tanda (por ejemplo, al abrir una tanda guardada).
    CargarTanda(Vec<(String, Duration)>),
    Quitar(u64),
    Mover(u64, i32),
    /// Quita varios spots de la tanda.
    QuitarVarios(Vec<u64>),
    /// Mueve varios spots juntos antes de la posición dada (largo de la lista: al final).
    MoverA(Vec<u64>, usize),
    /// Duplica varios spots (las copias van detrás del último).
    Duplicar(Vec<u64>),
    LimpiarTanda,
    Reproducir(u64),
    /// Play/Pausa. Si está detenido, arranca desde el spot indicado (o el primero).
    PlayPausa(Option<u64>),
    Siguiente,
    Anterior,
    /// Ir a ese punto del spot al aire (o en pausa).
    Buscar(Duration),
    /// Poner al aire un spot de la tanda desde ese punto (o saltar, si ya es el actual).
    ReproducirDesde(u64, Duration),
    /// Siempre permitido, tenga quien tenga el mando.
    VolverATv,
    Bucle(Bucle),
    Aleatorio(bool),
    Volumen(f32),
    /// Transición entre spots (y al entrar y salir de la señal de TV).
    Transicion(crate::Transicion),
    RecargarMedios,
    /// Cambia entre tanda manual y automática (por pauta).
    Modo(ModoTanda),
    /// Duración del corte en AUTO (segundos; None = sin tope) y si se completa con relleno.
    Corte {
        tope: Option<u32>,
        relleno: bool,
        /// Si al terminar la tanda (con relleno incluido) el corte sigue: repetirla.
        bucle: bool,
    },
    /// Crea (id 0) o modifica una pauta.
    GuardarPauta(Pauta),
    BorrarPauta(u64),
    /// Añade a la tanda un pase de hoy (por ejemplo, uno perdido que se quiere recuperar).
    AgregarPase(String),
    /// Pide el mando de la emisión. `forzar` (solo administradores) se lo quita a quien lo tenga.
    TomarMando {
        forzar: bool,
    },
    SoltarMando,
    /// Biblioteca (rol Editor; no necesitan el mando). Vuelve a revisar un clip (o todos).
    Revisar(Option<String>),
    /// Nivelar, convertir o reemplazar el archivo de un clip.
    Trabajo {
        clip: String,
        tipo: TipoTrabajo,
    },
    CancelarTrabajo(u64),
    /// Hacer un respaldo ahora.
    Respaldar,
    /// Usar este ffmpeg (tras descargarlo o actualizarlo). Solo desde la PC principal.
    UsarFfmpeg(PathBuf),
    /// Manda el archivo de un clip a la papelera.
    Borrar(String),
    Restaurar(u64),
    /// Borra para siempre uno (o todos, con `None`) de la papelera.
    EliminarDePapelera(Option<u64>),
    /// Activa (con dirección y capa), cambia o desactiva (`None`) la señal de TV por
    /// IPTV. Solo desde la PC principal (se guarda en su config.toml).
    Iptv(Option<crate::Iptv>),
    /// Vuelve a pedir el canal por IPTV ahora.
    ReconectarIptv,
    /// Con la tanda detenida: desde qué spot sigue al dar Play (`None`: desde el principio).
    Retomar(Option<u64>),
    /// Internas del servicio de red (no las puede enviar una PC remota).
    SesionAbierta,
    SesionCerrada,
}

impl Orden {
    /// Órdenes que cambian lo que sale al aire o la tanda: necesitan el mando.
    fn necesita_mando(&self) -> bool {
        matches!(
            self,
            Orden::Agregar { .. }
                | Orden::AgregarArchivo { .. }
                | Orden::CargarTanda(_)
                | Orden::Quitar(_)
                | Orden::Mover(..)
                | Orden::QuitarVarios(_)
                | Orden::MoverA(..)
                | Orden::Duplicar(_)
                | Orden::LimpiarTanda
                | Orden::Reproducir(_)
                | Orden::PlayPausa(_)
                | Orden::Siguiente
                | Orden::Anterior
                | Orden::Buscar(_)
                | Orden::ReproducirDesde(..)
                | Orden::Bucle(_)
                | Orden::Aleatorio(_)
                | Orden::Volumen(_)
                | Orden::Transicion(_)
                | Orden::Modo(_)
                | Orden::Corte { .. }
                | Orden::AgregarPase(_)
                | Orden::ReconectarIptv
                | Orden::Retomar(_)
        )
    }

    /// Órdenes que una PC remota no puede enviar.
    pub fn es_interna(&self) -> bool {
        matches!(self, Orden::SesionAbierta | Orden::SesionCerrada | Orden::UsarFfmpeg(_) | Orden::Iptv(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TipoEvento {
    AlAire,
    Tv,
    Aviso,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evento {
    pub hora: String,
    pub tipo: TipoEvento,
    pub texto: String,
}

/// Un video que se está copiando (o subiendo por la red) a la carpeta de medios.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Importacion {
    pub id: u64,
    pub nombre: String,
    pub progreso: f32,
}

/// Un aviso para la interfaz.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Aviso {
    /// Crece con cada aviso, para saber si es nuevo.
    pub id: u64,
    pub texto: String,
    /// `Some(sesión)` si es solo para quien envió la orden; `None` = para todos.
    pub para: Option<u64>,
}

/// Foto del estado que la interfaz dibuja en cada cuadro.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instantanea {
    pub conectado: bool,
    pub direccion: String,
    pub version: Option<String>,
    pub carpeta_medios: Option<PathBuf>,
    pub medios: Arc<Vec<amcp::Medio>>,
    pub lista: Vec<ItemTanda>,
    pub actual: Option<u64>,
    pub proximo: Option<u64>,
    pub fase: FaseVista,
    pub transcurrido: Duration,
    pub bucle: Bucle,
    pub aleatorio: bool,
    pub volumen: f32,
    pub registro: VecDeque<Evento>,
    pub importaciones: Vec<Importacion>,
    /// `true` si en los últimos segundos llegó el tiempo exacto por OSC.
    pub osc_activo: bool,
    pub aviso: Option<Aviso>,
    pub modo: ModoTanda,
    pub pautas: Arc<Vec<Pauta>>,
    pub pases_hoy: Vec<PaseVista>,
    /// Carpeta donde se guardan pautas y emisiones.
    pub carpeta_datos: PathBuf,
    /// Quién tiene el mando de la emisión: (sesión, nombre).
    pub mando: Option<(u64, String)>,
    /// PCs remotas conectadas: (sesión, nombre).
    pub sesiones: Vec<(u64, String)>,
    /// `Some(true)` si CasparCG reproduce videos desde cualquier carpeta.
    pub rutas_directas: Option<bool>,
    /// Revisión de cada video (clave del clip en mayúsculas → ficha). Por la red va aparte.
    pub fichas: Arc<HashMap<String, Ficha>>,
    pub biblioteca: VistaBiblioteca,
    /// Contratos por vencer, videos que faltan…
    pub alertas: Vec<Alerta>,
    pub corte: VistaCorte,
    pub respaldo: VistaRespaldo,
    /// Transición entre spots.
    #[serde(default)]
    pub transicion: crate::Transicion,
    /// Señal de TV por IPTV.
    #[serde(default)]
    pub iptv: crate::VistaIptv,
    /// Spot donde quedó detenida la tanda (Play lo vuelve a empezar).
    #[serde(default)]
    pub retomar: Option<u64>,
}

/// Estado de los respaldos.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VistaRespaldo {
    /// Nombre del último respaldo.
    pub ultimo: Option<String>,
    pub carpeta: String,
    pub haciendo: bool,
    pub error: Option<String>,
}

/// Duración del corte en AUTO.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VistaCorte {
    /// Segundos (None = sin tope).
    pub tope: Option<u32>,
    pub relleno: bool,
    /// Repetir la tanda si el corte sigue.
    #[serde(default)]
    pub bucle: bool,
    /// Pases en ventana que no entraron en la tanda (quedan para el próximo corte).
    pub fuera: Vec<String>,
    /// Clips de relleno disponibles.
    pub clips_relleno: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FaseVista {
    Tv,
    AlAire,
    Pausado,
}

impl Instantanea {
    pub fn vacia(direccion: &str) -> Self {
        Instantanea {
            conectado: false,
            direccion: direccion.to_string(),
            version: None,
            carpeta_medios: None,
            medios: Arc::new(Vec::new()),
            lista: Vec::new(),
            actual: None,
            proximo: None,
            fase: FaseVista::Tv,
            transcurrido: Duration::ZERO,
            bucle: Bucle::No,
            aleatorio: false,
            volumen: 1.0,
            registro: VecDeque::new(),
            importaciones: Vec::new(),
            osc_activo: false,
            aviso: None,
            modo: ModoTanda::Manual,
            pautas: Arc::new(Vec::new()),
            pases_hoy: Vec::new(),
            carpeta_datos: PathBuf::new(),
            mando: None,
            sesiones: Vec::new(),
            rutas_directas: None,
            fichas: Arc::new(HashMap::new()),
            biblioteca: VistaBiblioteca::default(),
            alertas: Vec::new(),
            corte: VistaCorte::default(),
            respaldo: VistaRespaldo::default(),
            transicion: crate::Transicion::default(),
            iptv: crate::VistaIptv::default(),
            retomar: None,
        }
    }

    /// Revisión de un clip, si ya se hizo.
    pub fn ficha(&self, clip: &str) -> Option<&Ficha> {
        self.fichas.get(&crate::biblioteca::clave(clip))
    }

    pub fn item(&self, id: u64) -> Option<&ItemTanda> {
        self.lista.iter().find(|i| i.id == id)
    }

    /// Busca un clip en la carpeta de medios sin distinguir mayúsculas.
    pub fn medio(&self, clip: &str) -> Option<&amcp::Medio> {
        buscar_medio(&self.medios, clip)
    }
}

fn buscar_medio<'a>(medios: &'a [amcp::Medio], clip: &str) -> Option<&'a amcp::Medio> {
    medios.iter().find(|m| m.nombre.eq_ignore_ascii_case(clip))
}

/// Estado de la miniatura de un clip.
#[derive(Debug, Clone)]
pub enum Miniatura {
    Pendiente,
    /// PNG tal como lo envía CasparCG.
    Lista(Arc<Vec<u8>>),
    NoDisponible,
}

pub type MapaMiniaturas = Arc<Mutex<HashMap<String, Miniatura>>>;

/// Un archivo de una PC remota que hay que subir a la principal.
#[derive(Debug, Clone)]
pub struct PedidoSubida {
    pub ruta: PathBuf,
    pub duracion: Option<Duration>,
    pub destino: Destino,
    /// Si viene, el archivo reemplaza el video de este clip (en vez de ir a la tanda).
    pub reemplazar: Option<String>,
}

/// Función que sube un archivo de una PC remota a la principal.
pub type FnSubir = Arc<dyn Fn(PedidoSubida) + Send + Sync>;

/// Dónde queda un reporte pedido (se llena en segundo plano).
pub type Ranura = Arc<Mutex<Option<Result<Reporte, String>>>>;

/// Pide un reporte a la PC principal: (desde, hasta, hoy, ahora, dónde dejarlo).
pub type FnReporte = Arc<dyn Fn(time::Date, time::Date, time::Date, time::Time, Ranura) + Send + Sync>;

/// Mango para usar el núcleo desde la interfaz. Se puede clonar.
///
/// Es igual en la PC principal y en una PC remota: lo que cambia es a dónde van
/// las órdenes (al núcleo de esta PC o, por la red, al de la PC principal).
#[derive(Clone)]
pub struct Controlador {
    pub(crate) enviar: Arc<dyn Fn(Orden) + Send + Sync>,
    pub(crate) estado: Arc<Mutex<Instantanea>>,
    pub(crate) miniaturas: MapaMiniaturas,
    pub(crate) pedir_miniatura: Arc<dyn Fn(String) + Send + Sync>,
    /// Solo en modo remoto: sube un archivo de esta PC a la principal.
    pub(crate) subir: Option<FnSubir>,
    pub(crate) sesion: Arc<AtomicU64>,
    /// Solo en la PC principal: para que el servicio de red envíe órdenes en nombre de otros.
    pub(crate) tx_local: Option<mpsc::UnboundedSender<(Origen, Orden)>>,
    pub(crate) caspar: Option<amcp::Cliente>,
    /// Solo en modo remoto: los reportes se piden a la principal.
    pub(crate) pedir_reporte: Option<FnReporte>,
    /// Solo en la PC principal: niveles de audio y cuadros por segundo de la salida.
    pub(crate) salida: Option<osc::SalidaCompartida>,
}

impl Controlador {
    pub fn enviar(&self, orden: Orden) {
        (self.enviar)(orden)
    }

    /// Envía una orden en nombre de otro usuario (lo usa el servicio de red).
    pub fn enviar_como(&self, origen: Origen, orden: Orden) {
        if let Some(tx) = &self.tx_local {
            let _ = tx.send((origen, orden));
        }
    }

    /// Copia del estado actual.
    pub fn ver(&self) -> Instantanea {
        self.estado.lock().expect("estado envenenado").clone()
    }

    /// Número de sesión de esta PC (0 en la principal).
    pub fn sesion(&self) -> u64 {
        self.sesion.load(Ordering::Relaxed)
    }

    pub fn es_remoto(&self) -> bool {
        self.subir.is_some()
    }

    /// `true` si esta PC tiene el mando de la emisión (o nadie lo tiene).
    pub fn tengo_mando(&self, e: &Instantanea) -> bool {
        e.mando.as_ref().is_none_or(|(id, _)| *id == self.sesion())
    }

    /// Niveles de audio de la salida de CasparCG (solo en la PC principal, con OSC).
    pub fn salida(&self) -> Option<osc::EstadoSalida> {
        self.salida.as_ref().map(|s| s.lock().expect("salida").clone())
    }

    /// Cliente de CasparCG (solo en la PC principal).
    pub fn caspar(&self) -> Option<&amcp::Cliente> {
        self.caspar.as_ref()
    }

    /// Añade un archivo de esta PC a la tanda. En la PC principal lo usa directo
    /// (o lo copia); en una remota lo sube primero a la principal.
    pub fn agregar_archivo(&self, ruta: PathBuf, duracion: Option<Duration>, destino: Destino) {
        match &self.subir {
            Some(subir) => subir(PedidoSubida { ruta, duracion, destino, reemplazar: None }),
            None => self.enviar(Orden::AgregarArchivo { ruta, duracion, destino }),
        }
    }

    /// Pone el archivo `ruta` (de esta PC) en lugar del video del clip. El spot conserva
    /// su nombre y su pauta; la versión anterior va a la papelera.
    pub fn reemplazar_con(&self, clip: String, ruta: PathBuf) {
        match &self.subir {
            Some(subir) => {
                subir(PedidoSubida { ruta, duracion: None, destino: Destino::Final, reemplazar: Some(clip) })
            }
            None => self.enviar(Orden::Trabajo { clip, tipo: TipoTrabajo::Reemplazar { desde: ruta } }),
        }
    }

    /// Reporte de emisión de `desde` a `hasta`. Se calcula en segundo plano (en la PC
    /// principal, aunque se pida desde una remota) y queda en la ranura devuelta.
    pub fn reporte(&self, desde: time::Date, hasta: time::Date, hoy: time::Date, ahora: time::Time) -> Ranura {
        let ranura: Ranura = Arc::new(Mutex::new(None));
        match &self.pedir_reporte {
            Some(pedir) => pedir(desde, hasta, hoy, ahora, ranura.clone()),
            None => {
                let e = self.ver();
                let r = ranura.clone();
                std::thread::spawn(move || {
                    let rep = reportes::calcular(&e.carpeta_datos, &e.pautas, desde, hasta, hoy, ahora);
                    *r.lock().expect("ranura") = Some(Ok(rep));
                });
            }
        }
        ranura
    }

    /// Miniatura de un clip. La primera vez se pide en segundo plano.
    pub fn miniatura(&self, clip: &str) -> Miniatura {
        let clave = clip.to_uppercase();
        {
            let mut mapa = self.miniaturas.lock().expect("miniaturas envenenadas");
            if let Some(m) = mapa.get(&clave) {
                return m.clone();
            }
            mapa.insert(clave, Miniatura::Pendiente);
        }
        (self.pedir_miniatura)(clip.to_string());
        Miniatura::Pendiente
    }

    /// Olvida las miniaturas (por ejemplo, tras recargar la carpeta de medios).
    pub fn olvidar_miniaturas(&self) {
        self.miniaturas.lock().expect("miniaturas envenenadas").clear();
    }
}

pub struct Opciones {
    pub canal: u16,
    pub capa: u16,
    /// Zona horaria local, para la hora del registro.
    pub zona: time::UtcOffset,
    /// Puerto UDP donde escuchar el OSC de CasparCG (0 = no escuchar).
    pub osc_puerto: u16,
    /// Carpeta de medios de CasparCG. Si es `None`, se pregunta a CasparCG.
    pub carpeta_medios: Option<PathBuf>,
    /// Carpeta de datos de la app (pautas y registro de emisiones).
    pub carpeta_datos: PathBuf,
    /// Orden de los pases normales en la tanda automática.
    pub orden: pauta::Orden,
    pub externos: ModoExternos,
    /// ffmpeg, para revisar, nivelar y convertir videos.
    pub ffmpeg: Option<PathBuf>,
    /// Formato y volumen del canal.
    pub estandar: Estandar,
    /// Duración del corte por defecto (segundos).
    pub tope_corte: Option<u32>,
    /// Carpetas o clips de relleno (por ejemplo "IDS", "PROMOS").
    pub relleno: Vec<String>,
    /// El relleno usa primero spots de otros clientes (el que menos salió hoy).
    pub relleno_clientes: bool,
    /// Minutos después de cerrarse su ventana en que un pase que no salió (o se cortó)
    /// se recupera solo en el próximo corte (0 = no se recupera).
    pub recuperar_min: u32,
    /// Dónde se guardan los respaldos diarios.
    pub carpeta_respaldos: PathBuf,
    /// Cuántos respaldos se guardan.
    pub respaldos_a_guardar: usize,
    /// Archivos que también van en el respaldo (config.toml).
    pub respaldar_tambien: Vec<PathBuf>,
    /// Transición por defecto (la elegida en la pantalla queda en datos/transicion.json).
    pub transicion: crate::Transicion,
    /// Señal de TV por IPTV (None = desactivada).
    pub iptv: Option<crate::Iptv>,
}

/// Arranca el núcleo (en la PC principal) en el runtime de tokio indicado.
/// `al_cambiar` se llama cada vez que hay algo nuevo que dibujar.
pub fn iniciar(
    rt: &tokio::runtime::Handle,
    caspar: amcp::Cliente,
    opciones: Opciones,
    al_cambiar: Arc<dyn Fn() + Send + Sync>,
) -> Controlador {
    let (tx, rx) = mpsc::unbounded_channel::<(Origen, Orden)>();
    let (tx_interno, rx_interno) = mpsc::unbounded_channel();
    let mut inicial = Instantanea::vacia(caspar.direccion());
    inicial.carpeta_medios = opciones.carpeta_medios.clone();
    inicial.carpeta_datos = opciones.carpeta_datos.clone();
    let estado = Arc::new(Mutex::new(inicial));

    let salida: osc::SalidaCompartida = Default::default();
    if opciones.osc_puerto != 0 {
        let (tx_osc, mut rx_osc) = mpsc::unbounded_channel();
        rt.spawn(osc::escuchar(opciones.osc_puerto, opciones.canal, opciones.capa, tx_osc, salida.clone()));
        let tx_i = tx_interno.clone();
        rt.spawn(async move {
            while let Some(t) = rx_osc.recv().await {
                if tx_i.send(Interno::Osc(t)).is_err() {
                    return;
                }
            }
        });
    }

    let almacen = Almacen::new(opciones.carpeta_datos.clone());
    // Duración del corte: la última elegida, o la de la configuración.
    let texto_corte = std::fs::read_to_string(opciones.carpeta_datos.join("corte.json")).ok();
    // [tope, relleno, bucle] (las versiones anteriores guardaban solo [tope, relleno]).
    let guardado: Option<(Option<u32>, bool, bool)> = texto_corte.as_deref().and_then(|t| {
        serde_json::from_str(t)
            .ok()
            .or_else(|| serde_json::from_str::<(Option<u32>, bool)>(t).ok().map(|(a, b)| (a, b, false)))
    });
    let (tope, relleno, bucle) =
        guardado.unwrap_or((opciones.tope_corte.filter(|t| *t > 0), !opciones.relleno.is_empty(), false));
    let corte = VistaCorte { tope, relleno, bucle, ..Default::default() };
    let transicion: crate::Transicion = std::fs::read_to_string(opciones.carpeta_datos.join("transicion.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(opciones.transicion);
    let bib = biblio::Biblioteca::new(opciones.ffmpeg.clone(), opciones.estandar.clone(), &opciones.carpeta_datos);
    let (fecha, _) = ahora_local(opciones.zona);
    let nucleo = Nucleo {
        almacen,
        pautas: Vec::new(),
        emisiones: Vec::new(),
        emitidos: HashSet::new(),
        fecha,
        modo: ModoTanda::Manual,
        ultima_sugerencia: Vec::new(),
        ultimo_marcado: None,
        conteo_hoy: HashMap::new(),
        pases_salidos: HashSet::new(),
        bucle_puesto: false,
        pases_vista: Vec::new(),
        ticks: 0,
        cls_reintento: None,
        cls_avisado: false,
        publicado: Instant::now(),
        relleno_pub: (0, Vec::new()),
        caspar: caspar.clone(),
        carpeta_fija: opciones.carpeta_medios.clone(),
        carpeta_medios: opciones.carpeta_medios.clone(),
        op: opciones,
        rep: Reproductor::new(),
        estado: estado.clone(),
        al_cambiar: al_cambiar.clone(),
        transicion,
        salida: salida.clone(),
        volumen: 1.0,
        registro: VecDeque::new(),
        aviso: None,
        siguiente_aviso: 1,
        medios: Arc::new(Vec::new()),
        version: None,
        importaciones: Vec::new(),
        siguiente_importacion: 1,
        ultimo_osc: None,
        tx_interno,
        mando: None,
        sesiones: Vec::new(),
        rutas_directas: None,
        origen: Origen::local(),
        bib,
        en_curso: None,
        alertas: Vec::new(),
        corte,
        respaldo: VistaRespaldo::default(),
        iptv: None,
        iptv_vista: crate::VistaIptv::default(),
        iptv_vigilancia: Default::default(),
    };
    rt.spawn(nucleo.correr(rx, rx_interno));

    let miniaturas: MapaMiniaturas = Arc::new(Mutex::new(HashMap::new()));
    let pedir = {
        let (caspar, mapa, aviso, rt) = (caspar.clone(), miniaturas.clone(), al_cambiar.clone(), rt.clone());
        Arc::new(move |clip: String| {
            let (caspar, mapa, aviso) = (caspar.clone(), mapa.clone(), aviso.clone());
            rt.spawn(async move {
                let m = match caspar.miniatura(&clip).await {
                    Ok(png) if !png.is_empty() => Miniatura::Lista(Arc::new(png)),
                    Ok(_) => Miniatura::NoDisponible,
                    Err(e) => {
                        tracing::debug!("sin miniatura para {clip}: {e}");
                        Miniatura::NoDisponible
                    }
                };
                mapa.lock().expect("miniaturas envenenadas").insert(clip.to_uppercase(), m);
                aviso();
            });
        })
    };
    let tx_enviar = tx.clone();
    Controlador {
        enviar: Arc::new(move |o| {
            let _ = tx_enviar.send((Origen::local(), o));
        }),
        estado,
        miniaturas,
        pedir_miniatura: pedir,
        subir: None,
        sesion: Arc::new(AtomicU64::new(0)),
        tx_local: Some(tx),
        caspar: Some(caspar),
        pedir_reporte: None,
        salida: Some(salida),
    }
}

/// Avisos que llegan al núcleo desde tareas en segundo plano.
enum Interno {
    Osc(TiempoOsc),
    Progreso { id: u64, progreso: f32 },
    Importado { id: u64, resultado: Result<Importado, String>, destino: Destino },
    Indice { indice: HashMap<String, PathBuf>, por_revisar: Vec<String>, libre: Option<u64> },
    Revisado { clave: String, ficha: Ficha },
    TrabajoAvance { id: u64, progreso: f32 },
    TrabajoHecho { id: u64, resultado: Result<PathBuf, String> },
    Respaldo(Result<PathBuf, String>),
}

struct Importado {
    clip: String,
    duracion: Duration,
    medios: Option<Vec<amcp::Medio>>,
    copiado: bool,
}

struct Nucleo {
    almacen: Almacen,
    pautas: Vec<Pauta>,
    emisiones: Vec<Emision>,
    emitidos: HashSet<String>,
    /// Día al que corresponden `emisiones`.
    fecha: time::Date,
    modo: ModoTanda,
    /// Claves de la última tanda sugerida (para no pisar los cambios del operador).
    ultima_sugerencia: Vec<String>,
    /// Último spot de la tanda cuyo pase ya se anotó como emitido.
    ultimo_marcado: Option<u64>,
    /// Cuántas veces salió hoy cada video.
    conteo_hoy: HashMap<String, u32>,
    /// Pases que ya tuvieron su salida hoy (una nueva salida del mismo pase, por el
    /// bucle, es solo relleno: no cuenta ni se recupera si se corta).
    pases_salidos: HashSet<String>,
    /// El bucle de la tanda lo puso el modo AUTO («Repetir si el corte sigue»).
    bucle_puesto: bool,
    pases_vista: Vec<PaseVista>,
    ticks: u32,
    /// Última vez que se publicó el estado.
    publicado: Instant,
    /// Clips de relleno ya calculados para una lista de videos (clave: su dirección).
    relleno_pub: (usize, Vec<String>),
    /// CLS falló: cuándo reintentar y cuántas veces se intentó.
    cls_reintento: Option<(Instant, u32)>,
    /// Ya se avisó que el media scanner no responde.
    cls_avisado: bool,
    caspar: amcp::Cliente,
    op: Opciones,
    rep: Reproductor,
    estado: Arc<Mutex<Instantanea>>,
    al_cambiar: Arc<dyn Fn() + Send + Sync>,
    volumen: f32,
    registro: VecDeque<Evento>,
    aviso: Option<Aviso>,
    siguiente_aviso: u64,
    medios: Arc<Vec<amcp::Medio>>,
    version: Option<String>,
    /// Carpeta indicada en la configuración (tiene prioridad).
    carpeta_fija: Option<PathBuf>,
    carpeta_medios: Option<PathBuf>,
    importaciones: Vec<Importacion>,
    siguiente_importacion: u64,
    ultimo_osc: Option<Instant>,
    tx_interno: mpsc::UnboundedSender<Interno>,
    /// Quién tiene el mando: (sesión, nombre).
    mando: Option<(u64, String)>,
    sesiones: Vec<(u64, String)>,
    rutas_directas: Option<bool>,
    /// Origen de la orden que se está atendiendo (para dirigirle los avisos).
    origen: Origen,
    bib: biblio::Biblioteca,
    /// Spot al aire que se anotará en el registro cuando termine.
    en_curso: Option<EnCurso>,
    alertas: Vec<Alerta>,
    corte: VistaCorte,
    respaldo: VistaRespaldo,
    transicion: crate::Transicion,
    salida: osc::SalidaCompartida,
    iptv: Option<crate::Iptv>,
    iptv_vista: crate::VistaIptv,
    iptv_vigilancia: crate::iptv::Vigilancia,
}

/// Lo que se va sabiendo de la salida al aire actual.
struct EnCurso {
    id: u64,
    clip: String,
    pase: Option<String>,
    duracion: f64,
    fecha: time::Date,
    hora: String,
    emitido: f64,
    modo: ModoTanda,
    quien: String,
}

impl Nucleo {
    async fn correr(
        mut self,
        mut rx: mpsc::UnboundedReceiver<(Origen, Orden)>,
        mut rx_interno: mpsc::UnboundedReceiver<Interno>,
    ) {
        let _vigia = self.caspar.vigilar(Duration::from_secs(3));
        let mut conexion = self.caspar.estado_conexion();
        let mut reloj = tokio::time::interval(INTERVALO_TICK);
        reloj.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        match self.almacen.cargar_pautas() {
            Ok(p) => self.pautas = p,
            Err(e) => self.avisar(format!("No se pudo leer la pauta: {e}")),
        }
        self.cargar_emitidos();
        self.actualizar_pauta(true);
        self.purgar_papelera();
        self.respaldo.carpeta = self.op.carpeta_respaldos.display().to_string();
        self.respaldo.ultimo = crate::respaldo::lista(&self.op.carpeta_respaldos)
            .first()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        self.respaldar_si_falta();
        if let Some(i) = self.op.iptv.clone() {
            self.poner_iptv(Some(i)).await;
        }
        self.publicar();

        loop {
            tokio::select! {
                orden = rx.recv() => {
                    let Some((origen, orden)) = orden else { return };
                    self.origen = origen;
                    if let Err(motivo) = self.permitir(&orden) {
                        self.avisar(motivo);
                    } else {
                        self.atender(orden).await;
                    }
                    self.origen = Origen::local();
                    self.publicar();
                }
                Some(interno) = rx_interno.recv() => {
                    // CasparCG manda el tiempo por OSC en cada cuadro (hasta 60 por
                    // segundo): eso solo no justifica publicar más de 20 veces por segundo.
                    let solo_tiempo = matches!(interno, Interno::Osc(_));
                    let antes = (self.rep.actual(), self.rep.fase());
                    self.interno(interno).await;
                    let igual = antes == (self.rep.actual(), self.rep.fase());
                    if !(solo_tiempo && igual && self.publicado.elapsed() < Duration::from_millis(50)) {
                        self.publicar();
                    }
                }
                _ = reloj.tick() => {
                    let adelanto = self.transicion.adelanto(self.fps());
                    self.rep.poner_adelanto(adelanto);
                    let res = self.rep.tick(Instant::now());
                    self.anotar_empezados(&res);
                    if res.termino {
                        self.anotar(TipoEvento::Tv, "Fin de la tanda: vuelve la señal de TV".into());
                    }
                    self.ejecutar(res.acciones).await;
                    // La pauta se revisa una vez por segundo (o al terminar una tanda).
                    self.ticks = self.ticks.wrapping_add(1);
                    let revisar = self.ticks.is_multiple_of(10) || res.termino;
                    if revisar && self.iptv.is_some() {
                        self.vigilar_iptv().await;
                    }
                    if revisar {
                        self.actualizar_pauta(false);
                        // Trabajos terminados que esperaban a que su spot saliera de la tanda.
                        if self.bib.trabajos.iter().any(|t| t.estado == crate::biblioteca::EstadoTrabajo::Esperando) {
                            self.aplicar_trabajos().await;
                        }
                    }
                    if self.ticks.is_multiple_of(600) {
                        self.revisar_disco();
                    }
                    // CLS falló (el media scanner aún no estaba listo): se reintenta.
                    // (No mientras hay publicidad al aire: cada intento puede tardar segundos.)
                    if self.cls_reintento.is_some_and(|(t, _)| Instant::now() >= t)
                        && *conexion.borrow()
                        && self.rep.fase() == Fase::Detenido
                    {
                        self.recargar_medios().await;
                        self.indexar();
                        self.publicar();
                    }
                    if self.ticks.is_multiple_of(36_000) {
                        self.purgar_papelera();
                        self.respaldar_si_falta();
                    }
                    let trabajando = self.bib.trabajos.iter().any(|t| t.estado == crate::biblioteca::EstadoTrabajo::Haciendo);
                    if revisar || trabajando || self.rep.fase() != Fase::Detenido || !self.importaciones.is_empty() {
                        self.publicar();
                    }
                }
                cambio = conexion.changed() => {
                    if cambio.is_err() { return; }
                    let conectado = *conexion.borrow_and_update();
                    if conectado {
                        self.version = self.caspar.version().await.ok();
                        self.anotar(TipoEvento::Info, format!("Conectado a CasparCG {}", self.version.clone().unwrap_or_default()));
                        self.conocer_carpeta_medios().await;
                        self.recargar_medios().await;
                        self.indexar();
                        // CasparCG pudo reiniciarse: el canal (y su volumen) se piden otra vez
                        // (salvo que se acabe de pedir).
                        let reciente = self.iptv_vigilancia.pedido.is_some_and(|t| t.elapsed() < Duration::from_secs(5));
                        if self.iptv.is_some() && !reciente {
                            self.pedir_iptv().await;
                        }
                    } else {
                        self.version = None;
                        self.avisar(format!("Se perdió la conexión con CasparCG ({})", self.caspar.direccion()));
                    }
                    self.publicar();
                }
            }
            if self.iptv.is_some() {
                self.ajustar_sonido_iptv().await;
            }
        }
    }

    async fn atender(&mut self, orden: Orden) {
        let ahora = Instant::now();
        let acciones = match orden {
            Orden::Agregar { clip, duracion, destino } => self.agregar(&clip, duracion, destino),
            Orden::AgregarArchivo { ruta, duracion, destino } => {
                self.agregar_archivo(ruta, duracion, destino).await;
                Vec::new()
            }
            Orden::CargarTanda(items) => {
                let items: Vec<(String, Duration, Option<String>)> =
                    items.into_iter().map(|(c, d)| (c, d, None)).collect();
                let acc = self.rep.reemplazar(&items);
                self.anotar(TipoEvento::Info, format!("Tanda cargada ({} spots)", items.len()));
                acc
            }
            Orden::Quitar(id) => {
                let (ok, acc) = self.rep.quitar(id);
                if !ok {
                    self.avisar("No se puede quitar el spot que está al aire".into());
                }
                acc
            }
            Orden::Mover(id, delta) => self.rep.mover(id, delta),
            Orden::QuitarVarios(ids) => {
                let (n, acc) = self.rep.quitar_varios(&ids);
                if n < ids.len() {
                    self.avisar("El spot que está al aire no se quita".into());
                }
                acc
            }
            Orden::MoverA(ids, destino) => self.rep.mover_a(&ids, destino),
            Orden::Duplicar(ids) => self.rep.duplicar(&ids),
            Orden::LimpiarTanda => self.rep.limpiar_lista(),
            Orden::Reproducir(id) => self.rep.reproducir(id, ahora),
            Orden::PlayPausa(sel) => self.rep.play_pausa(sel, ahora),
            Orden::Siguiente => self.rep.siguiente(ahora),
            Orden::Anterior => self.rep.anterior(ahora),
            Orden::Buscar(pos) => self.rep.buscar(pos, ahora),
            // Desde la señal de TV, no: clic en la línea solo salta con la tanda al aire.
            Orden::ReproducirDesde(..) if self.rep.fase() == Fase::Detenido => Vec::new(),
            Orden::ReproducirDesde(id, pos) => self.rep.reproducir_desde(id, pos, ahora),
            Orden::VolverATv if self.rep.fase() == Fase::Detenido && self.rep.retomar().is_some() => {
                // ⏹ otra vez con la tanda detenida a mitad: se reinicia la tanda.
                self.rep.poner_retomar(None);
                self.anotar(TipoEvento::Tv, "Tanda reiniciada: el próximo Play empieza desde el primer spot".into());
                Vec::new()
            }
            Orden::VolverATv => {
                // Siempre se envía, aunque el estado crea que ya está en TV.
                let acc = self.rep.volver_a_tv();
                let texto = match self.rep.retomar().and_then(|id| self.rep.item(id)) {
                    Some(i) => format!("Tanda detenida: vuelve la señal de TV (queda en «{}»)", i.clip),
                    None => "Volver a TV".into(),
                };
                self.anotar(TipoEvento::Tv, texto);
                acc
            }
            Orden::Retomar(id) => {
                self.rep.poner_retomar(id);
                Vec::new()
            }
            Orden::Bucle(b) => self.rep.cambiar_bucle(b, ahora),
            Orden::Aleatorio(a) => self.rep.cambiar_aleatorio(a),
            Orden::Volumen(v) => {
                let v = v.clamp(0.0, 1.0);
                match self.caspar.volumen(self.op.canal, self.op.capa, v).await {
                    Ok(()) => self.volumen = v,
                    Err(e) => self.avisar(format!("No se pudo cambiar el volumen: {e}")),
                }
                Vec::new()
            }
            Orden::RecargarMedios => {
                self.conocer_carpeta_medios().await;
                self.recargar_medios().await;
                self.indexar();
                Vec::new()
            }
            Orden::Transicion(t) => {
                let t = crate::Transicion { segundos: t.segundos.clamp(0.0, 5.0), ..t };
                if t != self.transicion {
                    self.transicion = t;
                    let ruta = self.op.carpeta_datos.join("transicion.json");
                    if let Ok(texto) = serde_json::to_string_pretty(&t) {
                        let _ = std::fs::write(ruta, texto);
                    }
                    self.anotar(TipoEvento::Info, format!("Transición: {}", t.describir()));
                    // Lo precargado se vuelve a cargar con la transición nueva.
                    self.rep.reprecargar()
                } else {
                    Vec::new()
                }
            }
            Orden::Corte { tope, relleno, bucle } => {
                let tope = tope.filter(|t| *t > 0);
                let mut acciones = Vec::new();
                if bucle != self.corte.bucle {
                    self.corte.bucle = bucle;
                    self.guardar_corte();
                    self.anotar(
                        TipoEvento::Info,
                        if bucle { "Repetir la tanda si el corte sigue" } else { "Sin repetir la tanda" }.into(),
                    );
                    acciones = self.aplicar_bucle_auto(ahora);
                }
                if tope != self.corte.tope || relleno != self.corte.relleno {
                    self.corte.tope = tope;
                    self.corte.relleno = relleno;
                    self.guardar_corte();
                    let t = match tope {
                        Some(s) => format!(
                            "Duración del corte: {}:{:02}{}",
                            s / 60,
                            s % 60,
                            if relleno { " con relleno" } else { "" }
                        ),
                        None => "Duración del corte: sin tope".into(),
                    };
                    self.anotar(TipoEvento::Info, t);
                    self.actualizar_pauta(true);
                }
                acciones
            }
            Orden::Revisar(clip) => {
                self.pedir_revision(clip);
                Vec::new()
            }
            Orden::Trabajo { clip, tipo } => {
                self.pedir_trabajo(clip, tipo);
                Vec::new()
            }
            Orden::UsarFfmpeg(f) => {
                let nuevo = self.bib.ffmpeg.as_ref() != Some(&f);
                self.bib.ffmpeg = Some(f);
                if nuevo {
                    self.anotar(TipoEvento::Info, "ffmpeg listo: se revisan los videos de la Biblioteca".into());
                    self.indexar();
                }
                Vec::new()
            }
            Orden::Iptv(i) => {
                self.poner_iptv(i).await;
                Vec::new()
            }
            Orden::ReconectarIptv => {
                if self.iptv.is_some() {
                    self.anotar(TipoEvento::Info, "IPTV: se vuelve a pedir el canal".into());
                    self.pedir_iptv().await;
                }
                Vec::new()
            }
            Orden::Respaldar => {
                let ahora = time::OffsetDateTime::now_utc().to_offset(self.op.zona);
                let nombre = format!(
                    "respaldo-{}-{:02}{:02}.zip",
                    pauta::formato::texto_fecha(ahora.date()),
                    ahora.hour(),
                    ahora.minute()
                );
                self.respaldar(nombre);
                Vec::new()
            }
            Orden::CancelarTrabajo(id) => {
                self.cancelar_trabajo(id);
                Vec::new()
            }
            Orden::Borrar(clip) => {
                self.borrar_clip(clip).await;
                Vec::new()
            }
            Orden::Restaurar(id) => {
                self.restaurar(id).await;
                Vec::new()
            }
            Orden::EliminarDePapelera(id) => {
                self.eliminar_de_papelera(id);
                Vec::new()
            }
            Orden::Modo(m) => {
                if m != self.modo {
                    self.modo = m;
                    let t = match m {
                        ModoTanda::Auto => "Modo AUTO: la tanda se arma con la pauta",
                        ModoTanda::Manual => "Modo MANUAL",
                    };
                    self.anotar(TipoEvento::Info, t.into());
                    self.ultima_sugerencia.clear();
                    self.actualizar_pauta(true);
                    self.aplicar_bucle_auto(ahora)
                } else {
                    Vec::new()
                }
            }
            Orden::GuardarPauta(mut p) => {
                let nueva = p.id == 0;
                if nueva {
                    p.id = self.pautas.iter().map(|x| x.id).max().unwrap_or(0) + 1;
                }
                let nombre = p.nombre.clone();
                match self.pautas.iter_mut().find(|x| x.id == p.id) {
                    Some(x) => *x = p,
                    None => self.pautas.push(p),
                }
                self.guardar_pautas();
                self.anotar(
                    TipoEvento::Info,
                    format!("Pauta {}: {nombre}", if nueva { "creada" } else { "modificada" }),
                );
                self.actualizar_pauta(true);
                Vec::new()
            }
            Orden::BorrarPauta(id) => {
                if let Some(i) = self.pautas.iter().position(|x| x.id == id) {
                    let p = self.pautas.remove(i);
                    self.guardar_pautas();
                    self.anotar(TipoEvento::Info, format!("Pauta borrada: {}", p.nombre));
                    self.actualizar_pauta(true);
                }
                Vec::new()
            }
            Orden::TomarMando { .. } | Orden::SoltarMando => Vec::new(), // resueltas en `permitir`
            Orden::SesionAbierta => {
                let o = self.origen.clone();
                self.sesiones.retain(|(id, _)| *id != o.id);
                self.sesiones.push((o.id, o.nombre.clone()));
                self.anotar(TipoEvento::Info, format!("Se conectó {} ({})", o.nombre, o.rol.nombre()));
                Vec::new()
            }
            Orden::SesionCerrada => {
                let o = self.origen.clone();
                self.sesiones.retain(|(id, _)| *id != o.id);
                if self.mando.as_ref().is_some_and(|(id, _)| *id == o.id) {
                    self.mando = None;
                    self.anotar(TipoEvento::Info, format!("{} se desconectó: el mando queda libre", o.nombre));
                } else {
                    self.anotar(TipoEvento::Info, format!("Se desconectó {}", o.nombre));
                }
                Vec::new()
            }
            Orden::AgregarPase(clave) => match self.pases_vista.iter().find(|p| p.clave == clave).cloned() {
                Some(p) => self.rep.agregar_pase(&p.clip, p.duracion, Some(p.clave)).1,
                None => Vec::new(),
            },
        };
        self.ejecutar(acciones).await;
    }

    fn agregar(&mut self, clip: &str, duracion: Duration, destino: Destino) -> Vec<Accion> {
        match destino {
            Destino::Final => self.rep.agregar(clip, duracion).1,
            Destino::Siguiente => self.rep.insertar_siguiente(clip, duracion).1,
            Destino::Posicion(pos) => self.rep.insertar_en(pos, clip, duracion).1,
            Destino::AlAire => {
                let (id, _) = self.rep.insertar_siguiente(clip, duracion);
                // El PLAY reemplaza cualquier precarga.
                self.rep.reproducir(id, Instant::now())
            }
        }
    }

    /// Añade un archivo de la PC. Si está en la carpeta de medios se usa tal cual;
    /// si no, se copia a `medios/IMPORTADOS` en segundo plano y luego se añade.
    async fn agregar_archivo(&mut self, ruta: PathBuf, duracion: Option<Duration>, destino: Destino) {
        let medios = self.carpeta_medios.clone();

        if let Some(clip) = medios.as_ref().and_then(|m| archivos::clip_de(&ruta, m)) {
            if buscar_medio(&self.medios, &clip).is_none() {
                self.recargar_medios().await;
            }
            let (clip, dur) = match buscar_medio(&self.medios, &clip) {
                Some(m) => (m.nombre.clone(), Duration::from_secs_f64(m.duracion_seg.max(0.0))),
                None => (clip, duracion.unwrap_or_default()),
            };
            let acc = self.agregar(&clip, dur, destino);
            self.ejecutar(acc).await;
            return;
        }

        // Está fuera de la carpeta de CasparCG: si CasparCG acepta rutas completas,
        // se reproduce desde donde está, sin copiarlo.
        if self.op.externos == ModoExternos::Directo && self.acepta_rutas_completas(&ruta).await {
            let clip = ruta.to_string_lossy().replace('\\', "/");
            let nombre = ruta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.anotar(TipoEvento::Info, format!("Sin copiar: «{nombre}» se reproduce desde su carpeta"));
            let acc = self.agregar(&clip, duracion.unwrap_or_default(), destino);
            self.ejecutar(acc).await;
            return;
        }

        let Some(medios) = medios else {
            self.avisar(
                "Todavía no se conoce la carpeta de medios de CasparCG. Conéctate a CasparCG o indícala en config.toml (carpeta_medios)."
                    .into(),
            );
            return;
        };

        // Hay que copiarlo.
        let id = self.siguiente_importacion;
        self.siguiente_importacion += 1;
        let nombre = ruta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.importaciones.push(Importacion { id, nombre: nombre.clone(), progreso: 0.0 });
        self.anotar(TipoEvento::Info, format!("Copiando «{nombre}» a la carpeta de CasparCG"));

        let tx = self.tx_interno.clone();
        let caspar = self.caspar.clone();
        tokio::spawn(async move {
            let resultado = importar(id, ruta, medios, duracion, caspar, tx.clone()).await;
            let _ = tx.send(Interno::Importado { id, resultado, destino });
        });
    }

    /// Comprueba una vez si CasparCG puede abrir un archivo por su ruta completa:
    /// lo precarga en una capa de prueba que no sale al aire y la vacía enseguida.
    async fn acepta_rutas_completas(&mut self, ejemplo: &std::path::Path) -> bool {
        if let Some(r) = self.rutas_directas {
            return r;
        }
        let clip = ejemplo.to_string_lossy().replace('\\', "/");
        let canal = self.op.canal;
        let r = match self.caspar.loadbg(canal, CAPA_PRUEBA, &clip, false, Default::default()).await {
            Ok(()) => true,
            Err(amcp::Error::Servidor { .. }) => false,
            Err(e) => {
                // Sin conexión no se puede saber: se decide la próxima vez.
                tracing::warn!("no se pudo comprobar rutas completas: {e}");
                return false;
            }
        };
        let _ = self.caspar.clear(canal, CAPA_PRUEBA).await;
        self.rutas_directas = Some(r);
        let t = if r {
            "CasparCG reproduce videos desde cualquier carpeta: ya no hace falta copiarlos"
        } else {
            "CasparCG solo reproduce su carpeta de medios: los videos de fuera se copiarán"
        };
        self.anotar(TipoEvento::Info, t.into());
        r
    }

    async fn interno(&mut self, ev: Interno) {
        match ev {
            Interno::Osc(t) => {
                self.ultimo_osc = Some(Instant::now());
                let res = self.rep.sincronizar(Instant::now(), t.transcurrido, t.total, t.archivo.as_deref());
                if !res.empezo.is_empty() || !res.acciones.is_empty() {
                    self.anotar_empezados(&res);
                    self.ejecutar(res.acciones).await;
                }
            }
            Interno::Progreso { id, progreso } => {
                if let Some(i) = self.importaciones.iter_mut().find(|i| i.id == id) {
                    i.progreso = progreso;
                }
            }
            Interno::Indice { indice, por_revisar, libre } => self.indice_listo(indice, por_revisar, libre),
            Interno::Respaldo(r) => {
                self.respaldo.haciendo = false;
                match r {
                    Ok(p) => {
                        let n = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        self.anotar(TipoEvento::Info, format!("Respaldo guardado: {n}"));
                        self.respaldo.ultimo = Some(n);
                        self.respaldo.error = None;
                        crate::respaldo::podar(&self.op.carpeta_respaldos, self.op.respaldos_a_guardar);
                    }
                    Err(e) => {
                        let prev = std::mem::replace(&mut self.origen, Origen::local());
                        self.avisar(format!("No se pudo hacer el respaldo: {e}"));
                        self.origen = prev;
                        self.respaldo.error = Some(e);
                    }
                }
            }
            Interno::Revisado { clave, ficha } => self.revisado(clave, ficha),
            Interno::TrabajoAvance { id, progreso } => self.trabajo_avance(id, progreso),
            Interno::TrabajoHecho { id, resultado } => self.trabajo_hecho(id, resultado).await,
            Interno::Importado { id, resultado, destino } => {
                let nombre =
                    self.importaciones.iter().find(|i| i.id == id).map(|i| i.nombre.clone()).unwrap_or_default();
                self.importaciones.retain(|i| i.id != id);
                match resultado {
                    Ok(imp) => {
                        if let Some(m) = imp.medios {
                            self.medios = Arc::new(ordenar(m));
                        }
                        let texto = if imp.copiado { "Copiado" } else { "Ya estaba en CasparCG" };
                        self.anotar(TipoEvento::Info, format!("{texto}: {}", imp.clip));
                        let acc = self.agregar(&imp.clip, imp.duracion, destino);
                        self.ejecutar(acc).await;
                    }
                    Err(e) => self.avisar(format!("No se pudo copiar «{nombre}»: {e}")),
                }
            }
        }
    }

    fn anotar_empezados(&mut self, res: &crate::reproductor::TickResultado) {
        for id in &res.empezo {
            if let Some(item) = self.rep.item(*id) {
                let t = format!("Al aire: {}", item.clip);
                self.anotar(TipoEvento::AlAire, t);
            }
        }
    }

    /// Cuadros por segundo del canal (según CasparCG; si aún no se sabe, 29,97).
    fn fps(&self) -> f64 {
        self.salida.lock().expect("salida").fps.unwrap_or(29.97)
    }

    async fn ejecutar(&mut self, acciones: Vec<Accion>) {
        let (canal, capa) = (self.op.canal, self.op.capa);
        let trans = self.transicion.para_amcp(self.fps());
        for accion in acciones {
            let resultado = match &accion {
                Accion::Play { clip, bucle, desde } => {
                    let desde_cuadro = a_cuadros(*desde, self.fps());
                    let op = amcp::OpcionesReproduccion { bucle: *bucle, transicion: trans, desde_cuadro };
                    let r = self.caspar.play(canal, capa, clip, op).await;
                    if r.is_ok() {
                        let texto = if desde.is_zero() {
                            format!("Al aire: {clip}")
                        } else {
                            {
                                let t = desde.as_secs();
                                format!("Al aire: {clip} (desde {}:{:02})", t / 60, t % 60)
                            }
                        };
                        self.anotar(TipoEvento::AlAire, texto);
                    }
                    r
                }
                Accion::Buscar(pos) => self.caspar.ir_a_cuadro(canal, capa, a_cuadros(*pos, self.fps())).await,
                Accion::Precargar { clip } => {
                    let op = amcp::OpcionesReproduccion { bucle: false, transicion: trans, ..Default::default() };
                    self.caspar.loadbg(canal, capa, clip, true, op).await
                }
                Accion::Pausa => self.caspar.pause(canal, capa).await,
                Accion::Reanudar => self.caspar.resume(canal, capa).await,
                Accion::BucleActual(b) => self.caspar.bucle_actual(canal, capa, *b).await,
                // Con transición, la vuelta a TV es un fundido al transparente (lo
                // precargado se descarta igual que con CLEAR).
                Accion::Limpiar if trans.is_some() => {
                    let op = amcp::OpcionesReproduccion { bucle: false, transicion: trans, ..Default::default() };
                    match self.caspar.loadbg(canal, capa, TRANSPARENTE, false, op).await {
                        Ok(()) => self.caspar.play_precargado(canal, capa).await,
                        Err(e) => Err(e),
                    }
                }
                Accion::Limpiar => self.caspar.clear(canal, capa).await,
            };
            if let Err(e) = resultado {
                match accion {
                    Accion::Play { clip, .. } => {
                        self.rep.forzar_detenido();
                        self.avisar(format!("No salió al aire «{clip}»: {e}"));
                        return;
                    }
                    otra => self.avisar(format!("CasparCG no pudo {}: {e}", otra.describir())),
                }
            }
        }
    }

    // ---------- Señal de TV por IPTV ----------

    async fn poner_iptv(&mut self, nueva: Option<crate::Iptv>) {
        let nueva = nueva.filter(|i| !i.direccion.trim().is_empty() && i.capa > 0 && i.capa != self.op.capa);
        if nueva == self.iptv {
            return;
        }
        // La capa vieja se vacía (si cambió de capa o se desactivó).
        if let Some(vieja) = self.iptv.take() {
            if nueva.as_ref().map(|n| n.capa) != Some(vieja.capa) && self.caspar.esta_conectado() {
                let _ = self.caspar.clear(self.op.canal, vieja.capa).await;
            }
        }
        self.iptv = nueva;
        self.iptv_vigilancia = Default::default();
        match self.iptv.clone() {
            None => {
                osc::reiniciar_iptv(&mut self.salida.lock().expect("salida"), 0);
                self.iptv_vista = crate::VistaIptv::default();
                self.anotar(TipoEvento::Info, "Modo Publicidad: CasparCG ya no reproduce el canal".into());
            }
            Some(i) => {
                self.iptv_vista = crate::VistaIptv {
                    estado: crate::EstadoIptv::Conectando,
                    direccion: i.direccion.clone(),
                    capa: i.capa,
                    reconexiones: 0,
                    silenciado: false,
                };
                self.anotar(TipoEvento::Info, format!("Modo Transmisión: canal por IPTV desde {}", i.direccion));
                if self.caspar.esta_conectado() {
                    self.pedir_iptv().await;
                }
            }
        }
    }

    /// Pide a CasparCG que reproduzca el canal en su capa.
    async fn pedir_iptv(&mut self) {
        let Some(i) = self.iptv.clone() else { return };
        osc::reiniciar_iptv(&mut self.salida.lock().expect("salida"), i.capa);
        self.iptv_vigilancia.pedido(Instant::now());
        let op = amcp::OpcionesReproduccion { bucle: false, transicion: None, ..Default::default() };
        match self.caspar.play(self.op.canal, i.capa, i.direccion.trim(), op).await {
            Ok(()) => {
                if !matches!(self.iptv_vista.estado, crate::EstadoIptv::Caida(_)) {
                    self.iptv_vista.estado = crate::EstadoIptv::Conectando;
                }
            }
            Err(e) => {
                tracing::warn!("IPTV: CasparCG no abrió el canal: {e}");
                self.iptv_vista.estado = crate::EstadoIptv::Caida(format!("CasparCG no abrió el canal: {e}"));
            }
        }
        // El volumen de la capa se vuelve a poner como corresponda.
        self.iptv_vista.silenciado = !self.iptv_vista.silenciado;
        self.ajustar_sonido_iptv().await;
    }

    /// Una vez por segundo: ¿sigue llegando el canal?
    async fn vigilar_iptv(&mut self) {
        if self.iptv.is_none() || !self.caspar.esta_conectado() {
            return;
        }
        let (osc_ult, vida) = {
            let s = self.salida.lock().expect("salida");
            (s.ultimo, s.iptv_vida)
        };
        let antes = self.iptv_vista.estado.clone();
        let (estado, paso) = self.iptv_vigilancia.revisar(Instant::now(), osc_ult, vida);
        self.iptv_vista.estado = estado.clone();
        self.iptv_vista.reconexiones = self.iptv_vigilancia.reconexiones;
        if estado != antes {
            match &estado {
                crate::EstadoIptv::EnVivo => {
                    self.anotar(TipoEvento::Tv, "IPTV: el canal llega".into());
                }
                crate::EstadoIptv::Caida(m) if !matches!(antes, crate::EstadoIptv::Caida(_)) => {
                    self.avisar(format!("IPTV: {m}"));
                }
                _ => {}
            }
        }
        if paso == crate::iptv::Paso::Pedir {
            self.pedir_iptv().await;
        }
    }

    /// Con publicidad al aire, el sonido del canal se baja a cero (con la duración
    /// de la transición); al volver a la señal de TV, se sube.
    async fn ajustar_sonido_iptv(&mut self) {
        let Some(capa) = self.iptv.as_ref().map(|i| i.capa) else { return };
        let silenciar = self.rep.fase() != Fase::Detenido;
        if silenciar == self.iptv_vista.silenciado || !self.caspar.esta_conectado() {
            return;
        }
        let cuadros = self.transicion.para_amcp(self.fps()).map(|(_, c)| c).unwrap_or(0);
        let v = if silenciar { 0.0 } else { 1.0 };
        let cmd = format!("MIXER {}-{capa} VOLUME {v} {cuadros}", self.op.canal);
        match self.caspar.comando(&cmd).await {
            Ok(_) => self.iptv_vista.silenciado = silenciar,
            Err(e) => {
                tracing::warn!("IPTV: no se pudo cambiar el volumen del canal: {e}");
                // Se reintenta en la próxima vuelta; se marca para no insistir sin pausa.
                self.iptv_vista.silenciado = silenciar;
                self.avisar(format!(
                    "No se pudo {} el sonido del canal: {e}",
                    if silenciar { "bajar" } else { "subir" }
                ));
            }
        }
    }

    async fn conocer_carpeta_medios(&mut self) {
        if self.carpeta_fija.is_some() {
            return;
        }
        match self.caspar.rutas().await {
            Ok(r) => {
                let carpeta = PathBuf::from(r.carpeta_medios());
                if self.carpeta_medios.as_ref() != Some(&carpeta) {
                    self.anotar(TipoEvento::Info, format!("Carpeta de medios: {}", carpeta.display()));
                }
                self.carpeta_medios = Some(carpeta);
            }
            Err(e) => tracing::warn!("no se pudo saber la carpeta de medios: {e}"),
        }
    }

    /// Pide a CasparCG la lista de videos (CLS). Al abrir CasparCG, su media scanner
    /// tarda unos segundos en estar listo y CLS falla (501): mientras tanto se leen los
    /// videos del disco y se reintenta solo; se avisa únicamente si sigue fallando.
    async fn recargar_medios(&mut self) {
        // Si CLS viene fallando, no se le vuelve a preguntar en cada cambio (cada intento
        // puede tardar segundos): se usa el disco hasta el próximo reintento programado.
        if self.cls_reintento.is_some_and(|(t, n)| n >= 2 && Instant::now() < t) {
            self.usar_lista_del_disco().await;
            return;
        }
        match self.caspar.cls().await {
            Ok(lista) => {
                self.medios = Arc::new(ordenar(lista));
                if self.cls_reintento.take().is_some() && self.cls_avisado {
                    self.anotar(TipoEvento::Info, "El media scanner de CasparCG ya responde".into());
                }
                self.cls_avisado = false;
            }
            Err(e) => {
                let intentos = self.cls_reintento.map(|r| r.1).unwrap_or(0) + 1;
                let espera = Duration::from_secs(if intentos < 10 { 3 } else { 60 });
                self.cls_reintento = Some((Instant::now() + espera, intentos));
                tracing::warn!("CLS falló (intento {intentos}): {e}");
                self.usar_lista_del_disco().await;
                if intentos == 10 && !self.cls_avisado {
                    self.cls_avisado = true;
                    self.avisar(format!(
                        "El media scanner de CasparCG no responde ({e}). Revisa que scanner.exe esté abierto; \
                         mientras tanto los videos se leen directamente de la carpeta de medios."
                    ));
                }
            }
        }
    }

    async fn usar_lista_del_disco(&mut self) {
        let Some(carpeta) = self.carpeta_medios.clone() else { return };
        let lista =
            tokio::task::spawn_blocking(move || archivos::listar_medios_disco(&carpeta)).await.unwrap_or_default();
        self.medios = Arc::new(ordenar(lista));
    }

    fn hora(&self) -> String {
        let formato = time::macros::format_description!("[hour]:[minute]:[second]");
        time::OffsetDateTime::now_utc().to_offset(self.op.zona).format(formato).unwrap_or_default()
    }

    fn anotar(&mut self, tipo: TipoEvento, texto: String) {
        tracing::info!("{texto}");
        let hora = self.hora();
        self.registro.push_front(Evento { hora, tipo, texto });
        self.registro.truncate(MAX_REGISTRO);
    }

    /// Aviso para quien envió la orden actual (o para todos si viene de CasparCG o del reloj).
    fn avisar(&mut self, texto: String) {
        self.anotar(TipoEvento::Aviso, texto.clone());
        let para = if self.origen.id == 0 { None } else { Some(self.origen.id) };
        self.aviso = Some(Aviso { id: self.siguiente_aviso, texto, para });
        self.siguiente_aviso += 1;
    }

    /// Comprueba rol y mando antes de atender una orden. Resuelve aquí las del mando.
    fn permitir(&mut self, orden: &Orden) -> Result<(), String> {
        let o = self.origen.clone();
        match orden {
            Orden::TomarMando { forzar } => {
                match &self.mando {
                    Some((id, _)) if *id == o.id => {}
                    Some((_, nombre)) if !*forzar => {
                        return Err(format!("El mando lo tiene {nombre}. Pídele que lo suelte."));
                    }
                    Some(_) if o.rol < Rol::Admin => {
                        return Err("Solo un administrador puede quitar el mando a otro.".into());
                    }
                    anterior => {
                        let quien = anterior.as_ref().map(|(_, n)| n.clone());
                        self.mando = Some((o.id, o.nombre.clone()));
                        let t = match quien {
                            Some(q) => format!("{} tomó el mando (se lo quitó a {q})", o.nombre),
                            None => format!("{} tomó el mando", o.nombre),
                        };
                        self.anotar(TipoEvento::Info, t);
                    }
                }
                Ok(())
            }
            Orden::SoltarMando => {
                if self.mando.as_ref().is_some_and(|(id, _)| *id == o.id) {
                    self.mando = None;
                    self.anotar(TipoEvento::Info, format!("{} soltó el mando", o.nombre));
                }
                Ok(())
            }
            Orden::GuardarPauta(_) | Orden::BorrarPauta(_) if o.rol < Rol::Editor => {
                Err("Tu usuario no puede modificar la pauta (hace falta rol Editor).".into())
            }
            Orden::Revisar(_)
            | Orden::Trabajo { .. }
            | Orden::CancelarTrabajo(_)
            | Orden::Borrar(_)
            | Orden::Restaurar(_)
            | Orden::EliminarDePapelera(_)
                if o.rol < Rol::Editor =>
            {
                Err("Tu usuario no puede cambiar la biblioteca (hace falta rol Editor).".into())
            }
            Orden::AgregarArchivo { ruta, .. }
                if o.id != 0 && self.carpeta_medios.as_ref().is_none_or(|m| !ruta.starts_with(m)) =>
            {
                Err("Ese archivo no está en la PC principal.".into())
            }
            ord if ord.necesita_mando() => match &self.mando {
                Some((id, _)) if *id == o.id => Ok(()),
                Some((_, nombre)) => Err(format!("No tienes el mando: lo tiene {nombre}.")),
                None => {
                    // Libre: lo toma quien empieza a operar.
                    self.mando = Some((o.id, o.nombre.clone()));
                    self.anotar(TipoEvento::Info, format!("{} tomó el mando", o.nombre));
                    Ok(())
                }
            },
            _ => Ok(()),
        }
    }

    fn publicar(&mut self) {
        self.marcar_emision();
        self.seguir_registro();
        let ahora = Instant::now();
        let fase = match self.rep.fase() {
            Fase::Detenido => FaseVista::Tv,
            Fase::Reproduciendo { .. } => FaseVista::AlAire,
            Fase::Pausado { .. } => FaseVista::Pausado,
        };
        // Solo se escribe (y se avisa a la pantalla) lo que cambió: así, con todo quieto,
        // la ventana no se redibuja ni se copian listas enteras varias veces por segundo.
        self.actualizar_relleno_pub();
        let biblioteca = self.bib.vista();
        let mut cambio = false;
        {
            let mut e = self.estado.lock().expect("estado envenenado");
            macro_rules! pon {
                ($campo:ident, $valor:expr) => {{
                    let v = $valor;
                    if e.$campo != v {
                        e.$campo = v;
                        cambio = true;
                    }
                }};
            }
            macro_rules! copia {
                ($campo:ident, $origen:expr) => {{
                    let o = $origen;
                    if e.$campo != *o {
                        e.$campo = o.clone();
                        cambio = true;
                    }
                }};
            }
            pon!(conectado, self.caspar.esta_conectado());
            copia!(version, &self.version);
            copia!(carpeta_medios, &self.carpeta_medios);
            if !Arc::ptr_eq(&e.medios, &self.medios) {
                e.medios = self.medios.clone();
                cambio = true;
            }
            if e.lista.as_slice() != self.rep.lista() {
                e.lista = self.rep.lista().to_vec();
                cambio = true;
            }
            pon!(actual, self.rep.actual());
            pon!(proximo, self.rep.proximo());
            pon!(fase, fase);
            pon!(transcurrido, self.rep.transcurrido(ahora));
            pon!(bucle, self.rep.bucle());
            pon!(aleatorio, self.rep.aleatorio());
            pon!(volumen, self.volumen);
            copia!(registro, &self.registro);
            copia!(importaciones, &self.importaciones);
            pon!(osc_activo, self.ultimo_osc.is_some_and(|t| t.elapsed() < OSC_VIGENTE));
            copia!(aviso, &self.aviso);
            pon!(modo, self.modo);
            if *e.pautas != self.pautas {
                e.pautas = Arc::new(self.pautas.clone());
                cambio = true;
            }
            copia!(pases_hoy, &self.pases_vista);
            copia!(mando, &self.mando);
            copia!(sesiones, &self.sesiones);
            pon!(rutas_directas, self.rutas_directas);
            if !Arc::ptr_eq(&e.fichas, &self.bib.fichas_pub) {
                e.fichas = self.bib.fichas_pub.clone();
                cambio = true;
            }
            pon!(biblioteca, biblioteca);
            copia!(alertas, &self.alertas);
            if e.corte.tope != self.corte.tope
                || e.corte.relleno != self.corte.relleno
                || e.corte.fuera != self.corte.fuera
                || e.corte.clips_relleno != self.relleno_pub.1
            {
                let mut c = self.corte.clone();
                c.clips_relleno = self.relleno_pub.1.clone();
                e.corte = c;
                cambio = true;
            }
            copia!(respaldo, &self.respaldo);
            pon!(transicion, self.transicion);
            copia!(iptv, &self.iptv_vista);
            pon!(retomar, self.rep.retomar());
        }
        self.publicado = Instant::now();
        if cambio {
            (self.al_cambiar)();
        }
    }

    /// Nombres de los clips de relleno (se recalcula solo cuando cambia la lista de videos).
    fn actualizar_relleno_pub(&mut self) {
        let clave = Arc::as_ptr(&self.medios) as usize;
        if self.relleno_pub.0 != clave {
            let v = self.clips_relleno().into_iter().map(|(n, _)| n).collect();
            self.relleno_pub = (clave, v);
        }
    }

    // ---------- Pauta ----------

    fn guardar_pautas(&mut self) {
        if let Err(e) = self.almacen.guardar_pautas(&self.pautas) {
            self.avisar(format!("No se pudo guardar la pauta: {e}"));
        }
    }

    fn cargar_emitidos(&mut self) {
        self.emisiones = self.almacen.cargar_emitidos(self.fecha);
        self.emitidos = self.emisiones.iter().map(|e| e.clave.clone()).collect();
        // Cuántas veces salió cada video hoy, y qué pases ya tuvieron su salida.
        self.conteo_hoy.clear();
        self.pases_salidos = self.emitidos.clone();
        for s in reportes::leer_dia(&self.op.carpeta_datos, self.fecha) {
            *self.conteo_hoy.entry(s.clip.clone()).or_default() += 1;
        }
    }

    /// En AUTO con «Repetir si el corte sigue», la tanda va en bucle; al quitarlo (o
    /// pasar a MANUAL) se quita el bucle que se puso.
    fn aplicar_bucle_auto(&mut self, ahora: Instant) -> Vec<Accion> {
        let quiere = self.modo == ModoTanda::Auto && self.corte.bucle;
        if quiere {
            self.bucle_puesto = true;
            self.rep.cambiar_bucle(Bucle::Lista, ahora)
        } else if self.bucle_puesto {
            self.bucle_puesto = false;
            self.rep.cambiar_bucle(Bucle::No, ahora)
        } else {
            Vec::new()
        }
    }

    /// Videos de otros clientes para rellenar: los de las pautas activas hoy que están
    /// en CasparCG, el que menos salió hoy primero.
    fn clientes_relleno(&self, fecha: time::Date) -> Vec<(String, Duration)> {
        if !self.op.relleno_clientes {
            return Vec::new();
        }
        let mut v: Vec<(String, Duration)> = Vec::new();
        for p in self.pautas.iter().filter(|p| p.toca(fecha)) {
            if v.iter().any(|(c, _)| c.eq_ignore_ascii_case(&p.clip)) {
                continue;
            }
            let Some(m) = buscar_medio(&self.medios, &p.clip) else { continue };
            let d = if m.duracion_seg > 0.0 { m.duracion_seg } else { p.duracion_seg };
            if d > 0.0 {
                v.push((p.clip.clone(), Duration::from_secs_f64(d)));
            }
        }
        v.sort_by_key(|(c, _)| (self.conteo_hoy.get(c).copied().unwrap_or(0), c.clone()));
        v
    }

    /// Recalcula los pases de hoy y, en modo AUTO con la tanda detenida, arma la tanda.
    fn actualizar_pauta(&mut self, forzar: bool) {
        let (fecha, hora) = ahora_local(self.op.zona);
        let clips: Vec<String> = self.medios.iter().map(|m| m.nombre.clone()).collect();
        let medios = (!clips.is_empty()).then_some(clips.as_slice());
        self.alertas = reportes::alertas(&self.pautas, medios, fecha);
        if fecha != self.fecha {
            self.fecha = fecha;
            self.cargar_emitidos();
        }
        let pases = pauta::pases_del_dia(&self.pautas, fecha);
        let en_tanda: HashSet<String> = self.rep.lista().iter().filter_map(|i| i.pase.clone()).collect();
        self.pases_vista = pases
            .iter()
            .map(|p| PaseVista {
                clave: p.clave.clone(),
                hora: pauta::formato::texto_hora(p.hora),
                nombre: p.nombre.clone(),
                clip: p.clip.clone(),
                duracion: p.duracion,
                prioridad: p.prioridad,
                estado: p.estado_con(hora, &self.emitidos, self.op.recuperar_min),
                en_tanda: en_tanda.contains(&p.clave),
                veces_hoy: self.conteo_hoy.get(&p.clip).copied().unwrap_or(0),
            })
            .collect();

        if self.modo == ModoTanda::Auto && self.rep.fase() == Fase::Detenido {
            let sugerida = pauta::tanda_del_corte(&pases, hora, &self.emitidos, self.op.orden, self.op.recuperar_min);
            // Ajustada a la duración del corte (y con relleno, si está activo: primero
            // spots de otros clientes y luego IDs, promos…).
            let rellenar = self.corte.relleno && !sugerida.is_empty();
            let relleno = if rellenar { self.clips_relleno() } else { Vec::new() };
            let clientes = if rellenar { self.clientes_relleno(fecha) } else { Vec::new() };
            let tope = self.corte.tope.map(|s| Duration::from_secs(s as u64));
            let (tanda, fuera) = pauta::ajustar_al_corte_con(&sugerida, tope, &clientes, &relleno);
            self.corte.fuera =
                fuera.iter().map(|p| format!("{} ({})", p.nombre, pauta::formato::texto_hora(p.hora))).collect();
            let claves: Vec<String> =
                tanda.iter().map(|t| t.pase.clone().unwrap_or_else(|| format!("relleno:{}", t.clip))).collect();
            if forzar || claves != self.ultima_sugerencia {
                let items: Vec<(String, Duration, Option<String>)> =
                    tanda.iter().map(|t| (t.clip.clone(), t.duracion, t.pase.clone())).collect();
                // Con la tanda detenida, reemplazar no envía nada a CasparCG.
                let _ = self.rep.reemplazar(&items);
                self.ultima_sugerencia = claves;
                for v in &mut self.pases_vista {
                    v.en_tanda = self.ultima_sugerencia.contains(&v.clave);
                }
            }
        }
    }

    /// Respaldo del día, si todavía no se hizo.
    fn respaldar_si_falta(&mut self) {
        let (fecha, _) = ahora_local(self.op.zona);
        let dia = pauta::formato::texto_fecha(fecha);
        if !crate::respaldo::hay_de(&self.op.carpeta_respaldos, &dia) {
            self.respaldar(format!("respaldo-{dia}.zip"));
        }
    }

    fn respaldar(&mut self, nombre: String) {
        if self.respaldo.haciendo {
            return;
        }
        self.respaldo.haciendo = true;
        let (datos, extra, destino) =
            (self.op.carpeta_datos.clone(), self.op.respaldar_tambien.clone(), self.op.carpeta_respaldos.clone());
        let tx = self.tx_interno.clone();
        tokio::task::spawn_blocking(move || {
            let r = crate::respaldo::respaldar(&datos, &extra, &destino, &nombre);
            let _ = tx.send(Interno::Respaldo(r));
        });
    }

    /// Clips de relleno disponibles: los de las carpetas (o clips) indicados en la configuración.
    fn clips_relleno(&self) -> Vec<(String, Duration)> {
        let fuentes: Vec<String> = self.op.relleno.iter().map(|r| r.trim().trim_matches('/').to_uppercase()).collect();
        self.medios
            .iter()
            .filter(|m| m.tipo == amcp::TipoMedio::Movie && m.duracion_seg > 0.0)
            .filter(|m| {
                let n = m.nombre.to_uppercase();
                fuentes.iter().any(|f| !f.is_empty() && (n == *f || n.starts_with(&format!("{f}/"))))
            })
            .map(|m| (m.nombre.clone(), Duration::from_secs_f64(m.duracion_seg)))
            .collect()
    }

    fn guardar_corte(&self) {
        let ruta = self.op.carpeta_datos.join("corte.json");
        let _ = std::fs::create_dir_all(&self.op.carpeta_datos);
        if let Ok(t) = serde_json::to_string_pretty(&(self.corte.tope, self.corte.relleno, self.corte.bucle)) {
            let _ = std::fs::write(ruta, t);
        }
    }

    /// Anota en el registro cada salida al aire cuando termina (completa o cortada).
    fn seguir_registro(&mut self) {
        let ahora = Instant::now();
        let actual = if self.rep.fase() == Fase::Detenido { None } else { self.rep.actual() };
        let transcurrido = self.rep.transcurrido(ahora).as_secs_f64();
        // ¿Sigue la misma salida? (En bucle de un spot, volver a 0 es una salida nueva.)
        if let Some(c) = &mut self.en_curso {
            if Some(c.id) == actual && transcurrido + 1.0 >= c.emitido {
                c.emitido = c.emitido.max(transcurrido);
                return;
            }
        }
        if let Some(c) = self.en_curso.take() {
            let emitido = c.emitido.min(c.duracion.max(c.emitido));
            let completo = c.duracion <= 0.0 || emitido >= c.duracion - 1.0;
            // Un pase que se cortó no cuenta: vuelve a quedar por salir (se recupera).
            if !completo {
                if let Some(clave) = &c.pase {
                    self.desmarcar(clave, &c.clip);
                }
            }
            let s = Salida {
                hora: c.hora,
                cliente: reportes::cliente_de(&c.clip, c.pase.as_deref(), &self.pautas),
                clip: c.clip,
                pase: c.pase,
                duracion: c.duracion,
                emitido,
                completo,
                modo: if c.modo == ModoTanda::Auto { "AUTO".into() } else { "MANUAL".into() },
                quien: c.quien,
            };
            if let Err(e) = reportes::anotar(&self.op.carpeta_datos, c.fecha, &s) {
                tracing::warn!("no se pudo anotar en el registro: {e}");
            }
        }
        if let Some(id) = actual {
            if let Some(item) = self.rep.item(id).cloned() {
                let (fecha, _) = ahora_local(self.op.zona);
                *self.conteo_hoy.entry(item.clip.clone()).or_default() += 1;
                // Si su pase ya tuvo su salida (el bucle lo repite), esta es solo relleno.
                let pase = item.pase.filter(|c| self.pases_salidos.insert(c.clone()));
                self.en_curso = Some(EnCurso {
                    id,
                    clip: item.clip,
                    pase,
                    duracion: item.duracion.as_secs_f64(),
                    fecha,
                    hora: self.hora(),
                    emitido: transcurrido,
                    modo: self.modo,
                    quien: self.mando.as_ref().map(|(_, n)| n.clone()).unwrap_or_else(|| "PC principal".into()),
                });
            }
        }
    }

    /// Un pase que salió cortado vuelve a quedar por salir.
    fn desmarcar(&mut self, clave: &str, clip: &str) {
        self.pases_salidos.remove(clave);
        if !self.emitidos.remove(clave) {
            return;
        }
        self.emisiones.retain(|e| e.clave != clave);
        if let Err(e) = self.almacen.guardar_emitidos(self.fecha, &self.emisiones) {
            self.avisar(format!("No se pudo guardar el registro de emisiones: {e}"));
        }
        if self.op.recuperar_min > 0 {
            self.anotar(TipoEvento::Info, format!("Se cortó {clip}: queda pendiente para el próximo corte"));
        }
        if let Some(v) = self.pases_vista.iter_mut().find(|v| v.clave == clave) {
            v.estado = EstadoPase::Vigente;
        }
        // Que el modo AUTO vuelva a armar la tanda con él.
        self.ultima_sugerencia.clear();
    }

    /// Cuando sale al aire un spot que viene de la pauta, su pase queda emitido.
    fn marcar_emision(&mut self) {
        if self.rep.fase() == Fase::Detenido {
            return;
        }
        let Some(id) = self.rep.actual() else { return };
        if self.ultimo_marcado == Some(id) {
            return;
        }
        self.ultimo_marcado = Some(id);
        let Some(item) = self.rep.item(id).cloned() else { return };
        let Some(clave) = item.pase else { return };
        if !self.emitidos.insert(clave.clone()) {
            return;
        }
        self.emisiones.push(Emision { clave: clave.clone(), clip: item.clip.clone(), hora: self.hora() });
        if let Err(e) = self.almacen.guardar_emitidos(self.fecha, &self.emisiones) {
            self.avisar(format!("No se pudo guardar el registro de emisiones: {e}"));
        }
        let hora_pase = clave.rsplit('@').next().unwrap_or_default().to_string();
        self.anotar(TipoEvento::Info, format!("Pase de las {hora_pase} cumplido: {}", item.clip));
        if let Some(v) = self.pases_vista.iter_mut().find(|v| v.clave == clave) {
            v.estado = EstadoPase::Emitido;
        }
    }
}

fn ahora_local(zona: time::UtcOffset) -> (time::Date, time::Time) {
    let t = time::OffsetDateTime::now_utc().to_offset(zona);
    (t.date(), t.time())
}

fn ordenar(mut lista: Vec<amcp::Medio>) -> Vec<amcp::Medio> {
    lista.sort_by_key(|m| m.nombre.to_lowercase());
    lista
}

/// Copia `ruta` a la carpeta de medios y espera a que CasparCG la vea.
async fn importar(
    id: u64,
    ruta: PathBuf,
    medios: PathBuf,
    duracion_local: Option<Duration>,
    caspar: amcp::Cliente,
    tx: mpsc::UnboundedSender<Interno>,
) -> Result<Importado, String> {
    let tx_p = tx.clone();
    let medios_c = medios.clone();
    let (destino, copiado) = tokio::task::spawn_blocking(move || -> std::io::Result<(PathBuf, bool)> {
        let (destino, ya_existe) = archivos::destino_importacion(&ruta, &medios_c)?;
        if !ya_existe {
            let mut ultimo = 0.0f32;
            archivos::copiar_con_progreso(&ruta, &destino, |p| {
                if p - ultimo >= 0.01 || p >= 1.0 {
                    ultimo = p;
                    let _ = tx_p.send(Interno::Progreso { id, progreso: p });
                }
            })?;
        }
        Ok((destino, !ya_existe))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let clip = archivos::clip_de(&destino, &medios).ok_or("el destino quedó fuera de la carpeta de medios")?;

    // CasparCG tarda un poco en ver archivos nuevos (su escáner de medios).
    let mut ultima_lista = None;
    for _ in 0..12 {
        if let Ok(lista) = caspar.cls().await {
            if let Some(m) = buscar_medio(&lista, &clip) {
                let (nombre, dur) = (m.nombre.clone(), Duration::from_secs_f64(m.duracion_seg.max(0.0)));
                return Ok(Importado { clip: nombre, duracion: dur, medios: Some(lista), copiado });
            }
            ultima_lista = Some(lista);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    // No apareció todavía: se añade igual con la duración local (OSC la corregirá).
    Ok(Importado { clip, duracion: duracion_local.unwrap_or_default(), medios: ultima_lista, copiado })
}

/// Un punto del spot en cuadros del canal (como lo cuenta `SEEK`).
fn a_cuadros(t: Duration, fps: f64) -> u64 {
    (t.as_secs_f64() * fps).round() as u64
}
