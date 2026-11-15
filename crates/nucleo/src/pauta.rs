//! Pauta: qué spot debe salir, qué días y a qué horas (con tolerancia y prioridad).
//!
//! - Una [`Pauta`] es la programación de un spot.
//! - Cada pauta genera **pases**: una hora concreta de un día concreto.
//! - Un pase está *vigente* desde `hora - antes` hasta `hora + después` (cada
//!   publicidad tiene sus minutos; el canal no siempre corta a la hora).
//! - La tanda sugerida son los pases vigentes que aún no salieron: primero los
//!   prioritarios y después por hora (o por cercanía a la hora actual).
//!
//! Todo es lógica pura: la fecha y la hora llegan como parámetros.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::{Date, Time};

/// Qué días toca una pauta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Dias {
    Todos,
    /// Lunes a domingo.
    Semana {
        dias: [bool; 7],
    },
    /// Un día sí y N-1 no, contando desde la fecha de inicio (2 = interdiario).
    CadaN {
        n: u32,
    },
    /// Solo estas fechas.
    Fechas {
        #[serde(with = "formato::fechas")]
        fechas: Vec<Date>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pauta {
    pub id: u64,
    /// Nombre para el operador (cliente o descripción).
    pub nombre: String,
    /// Clip de CasparCG.
    pub clip: String,
    pub duracion_seg: f64,
    #[serde(with = "formato::fecha")]
    pub inicio: Date,
    /// `None` = sin fecha de fin.
    #[serde(with = "formato::fecha_opcional")]
    pub fin: Option<Date>,
    pub dias: Dias,
    #[serde(with = "formato::horas")]
    pub horas: Vec<Time>,
    /// Minutos **antes** de cada hora en que el pase ya puede salir (y, si no se
    /// indica `despues_min`, también los minutos después).
    pub tolerancia_min: u32,
    /// Minutos **después** de cada hora en que el pase todavía está en su ventana.
    /// `None` = los mismos que `tolerancia_min` (archivos viejos).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub despues_min: Option<u32>,
    /// 0 = normal; 1, 2, 3 = prioritaria (más alto, antes).
    pub prioridad: u8,
    pub activa: bool,
    /// Cliente (para agrupar y para los reportes). Vacío: se usa `nombre`.
    #[serde(default)]
    pub cliente: String,
    /// Notas libres (contrato, contacto…).
    #[serde(default)]
    pub notas: String,
}

impl Pauta {
    /// El cliente, o el nombre si no se indicó.
    pub fn cliente_o_nombre(&self) -> &str {
        if self.cliente.trim().is_empty() {
            &self.nombre
        } else {
            self.cliente.trim()
        }
    }

    /// ¿Esta pauta tiene pases en `fecha`?
    pub fn toca(&self, fecha: Date) -> bool {
        if !self.activa || fecha < self.inicio || self.fin.is_some_and(|f| fecha > f) {
            return false;
        }
        match &self.dias {
            Dias::Todos => true,
            Dias::Semana { dias } => dias[fecha.weekday().number_days_from_monday() as usize],
            Dias::CadaN { n } => *n > 0 && (fecha - self.inicio).whole_days() % *n as i64 == 0,
            Dias::Fechas { fechas } => fechas.contains(&fecha),
        }
    }

    pub fn duracion(&self) -> Duration {
        Duration::from_secs_f64(self.duracion_seg.max(0.0))
    }

    /// Minutos después de la hora en que el pase sigue en su ventana.
    pub fn despues(&self) -> u32 {
        self.despues_min.unwrap_or(self.tolerancia_min)
    }

    /// Descripción corta de los días, para mostrar.
    pub fn describir_dias(&self) -> String {
        const N: [&str; 7] = ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"];
        match &self.dias {
            Dias::Todos => "todos los días".into(),
            Dias::Semana { dias } => {
                if dias[..5].iter().all(|d| *d) && !dias[5] && !dias[6] {
                    return "lunes a viernes".into();
                }
                let v: Vec<&str> = (0..7).filter(|i| dias[*i]).map(|i| N[i]).collect();
                if v.is_empty() {
                    "ningún día".into()
                } else {
                    v.join(", ")
                }
            }
            Dias::CadaN { n: 2 } => "interdiario".into(),
            Dias::CadaN { n } => format!("cada {n} días"),
            Dias::Fechas { fechas } => format!("{} fechas", fechas.len()),
        }
    }
}

/// Estado de un pase respecto a la hora actual.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EstadoPase {
    /// Todavía no abre su ventana.
    Programado,
    /// Dentro de la ventana: debe salir en el próximo corte.
    Vigente,
    Emitido,
    /// Se cerró la ventana sin que saliera (y ya no se recupera).
    Perdido,
    /// Se cerró la ventana sin que saliera (o se cortó), pero todavía se recupera:
    /// entra solo en el próximo corte.
    Pendiente,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pase {
    /// Identificador único del pase: `pauta@AAAA-MM-DD@HH:MM`.
    pub clave: String,
    pub pauta_id: u64,
    pub nombre: String,
    pub clip: String,
    pub duracion: Duration,
    pub hora: Time,
    pub prioridad: u8,
    /// Minutos antes de la hora en que abre la ventana.
    pub tolerancia_min: u32,
    /// Minutos después de la hora en que cierra la ventana.
    pub despues_min: u32,
}

impl Pase {
    fn minuto(&self) -> i32 {
        self.hora.hour() as i32 * 60 + self.hora.minute() as i32
    }

    pub fn estado(&self, ahora: Time, emitidos: &HashSet<String>) -> EstadoPase {
        if emitidos.contains(&self.clave) {
            return EstadoPase::Emitido;
        }
        let ahora = ahora.hour() as i32 * 60 + ahora.minute() as i32;
        if ahora < self.minuto() - self.tolerancia_min as i32 {
            EstadoPase::Programado
        } else if ahora <= self.minuto() + self.despues_min as i32 {
            EstadoPase::Vigente
        } else {
            EstadoPase::Perdido
        }
    }
}

impl Pase {
    /// Como `estado`, pero un pase cuya ventana se cerró hace menos de `recuperar_min`
    /// minutos queda **Pendiente** (se recupera en el próximo corte).
    pub fn estado_con(&self, ahora: Time, emitidos: &HashSet<String>, recuperar_min: u32) -> EstadoPase {
        let e = self.estado(ahora, emitidos);
        if e == EstadoPase::Perdido && recuperar_min > 0 {
            let ahora_m = ahora.hour() as i32 * 60 + ahora.minute() as i32;
            if ahora_m <= self.minuto() + self.despues_min as i32 + recuperar_min as i32 {
                return EstadoPase::Pendiente;
            }
        }
        e
    }
}

/// Minutos antes de su hora en que una tanda nueva ya «toca» y va antes que lo
/// pendiente de una tanda anterior.
pub const CERCA_MIN: i32 = 5;

/// Cómo se ordenan los pases normales (no prioritarios) dentro de la tanda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orden {
    /// La hora programada más temprana primero (la que vence antes).
    #[default]
    Hora,
    /// La hora más cercana a la hora actual primero.
    Cercania,
}

pub fn clave(pauta_id: u64, fecha: Date, hora: Time) -> String {
    format!("{pauta_id}@{}@{:02}:{:02}", formato::texto_fecha(fecha), hora.hour(), hora.minute())
}

/// Todos los pases de un día, ordenados por hora.
pub fn pases_del_dia(pautas: &[Pauta], fecha: Date) -> Vec<Pase> {
    let mut v: Vec<Pase> = pautas
        .iter()
        .filter(|p| p.toca(fecha))
        .flat_map(|p| {
            p.horas.iter().map(move |h| Pase {
                clave: clave(p.id, fecha, *h),
                pauta_id: p.id,
                nombre: p.nombre.clone(),
                clip: p.clip.clone(),
                duracion: p.duracion(),
                hora: *h,
                prioridad: p.prioridad,
                tolerancia_min: p.tolerancia_min,
                despues_min: p.despues(),
            })
        })
        .collect();
    v.sort_by(|a, b| a.hora.cmp(&b.hora).then(b.prioridad.cmp(&a.prioridad)).then(a.nombre.cmp(&b.nombre)));
    v
}

/// Tanda sugerida para un corte que empieza `ahora` (sin recuperar pendientes).
pub fn tanda_sugerida(pases: &[Pase], ahora: Time, emitidos: &HashSet<String>, orden: Orden) -> Vec<Pase> {
    tanda_del_corte(pases, ahora, emitidos, orden, 0)
}

/// Tanda para un corte que empieza `ahora`, con lo que está en su ventana y lo que
/// quedó pendiente (no salió o se cortó, hasta `recuperar_min` después de su ventana).
///
/// Orden:
/// 1. Los **prioritarios** en su ventana (★★ antes que ★).
/// 2. Si la tanda nueva ya toca (llegó su hora o faltan menos de `CERCA_MIN` min):
///    sus spots y **después** lo pendiente. Si todavía no toca: primero lo pendiente.
/// 3. Lo pendiente va en el mismo orden en que estaba (prioridad y hora).
///
/// Un mismo spot no se repite en la tanda: se queda su pase más antiguo.
pub fn tanda_del_corte(
    pases: &[Pase],
    ahora: Time,
    emitidos: &HashSet<String>,
    orden: Orden,
    recuperar_min: u32,
) -> Vec<Pase> {
    let ahora_min = ahora.hour() as i32 * 60 + ahora.minute() as i32;
    let mut candidatos: Vec<(&Pase, EstadoPase)> = pases
        .iter()
        .map(|p| (p, p.estado_con(ahora, emitidos, recuperar_min)))
        .filter(|(_, e)| matches!(e, EstadoPase::Vigente | EstadoPase::Pendiente))
        .collect();
    candidatos.sort_by_key(|(p, _)| p.minuto());
    let mut vistos = HashSet::new();
    candidatos.retain(|(p, _)| vistos.insert(p.pauta_id));

    let por_orden = |a: &&Pase, b: &&Pase| match orden {
        Orden::Hora => a.minuto().cmp(&b.minuto()),
        Orden::Cercania => {
            (a.minuto() - ahora_min).abs().cmp(&(b.minuto() - ahora_min).abs()).then(a.minuto().cmp(&b.minuto()))
        }
    };
    let mut pendientes: Vec<&Pase> =
        candidatos.iter().filter(|(_, e)| *e == EstadoPase::Pendiente).map(|(p, _)| *p).collect();
    let nuevos: Vec<&Pase> = candidatos.iter().filter(|(_, e)| *e == EstadoPase::Vigente).map(|(p, _)| *p).collect();
    let mut prioritarios: Vec<&Pase> = nuevos.iter().filter(|p| p.prioridad > 0).copied().collect();
    let mut normales: Vec<&Pase> = nuevos.iter().filter(|p| p.prioridad == 0).copied().collect();
    prioritarios.sort_by(|a, b| b.prioridad.cmp(&a.prioridad).then_with(|| por_orden(a, b)));
    normales.sort_by(por_orden);
    pendientes.sort_by(|a, b| b.prioridad.cmp(&a.prioridad).then(a.minuto().cmp(&b.minuto())));

    let toca = nuevos.iter().any(|p| ahora_min >= p.minuto() - CERCA_MIN);
    let mut v = prioritarios;
    if toca {
        v.extend(normales);
        v.extend(pendientes);
    } else {
        v.extend(pendientes);
        v.extend(normales);
    }
    v.into_iter().cloned().collect()
}

// ======================================================================
// Duración del corte y relleno
// ======================================================================

/// Un spot de la tanda armada para un corte.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnCorte {
    pub nombre: String,
    pub clip: String,
    pub duracion: Duration,
    /// Clave del pase que cumple (None = relleno).
    pub pase: Option<String>,
}

/// Ajusta la tanda sugerida a la duración del corte.
///
/// - Los pases van en el orden dado (los prioritarios ya vienen primero). Si uno no
///   entra, se salta y se prueba con los siguientes (más cortos); los que no entran
///   quedan para el próximo corte.
/// - Con `relleno`, el tiempo que sobra se completa con esos clips (IDs, promos…),
///   sin repetir uno seguido y sin pasarse del tope.
///
/// Devuelve la tanda y los pases que no entraron.
pub fn ajustar_al_corte(
    sugerida: &[Pase],
    tope: Option<Duration>,
    relleno: &[(String, Duration)],
) -> (Vec<EnCorte>, Vec<Pase>) {
    ajustar_al_corte_con(sugerida, tope, &[], relleno)
}

/// Como `ajustar_al_corte`, pero el tiempo que sobra se llena **primero con spots de
/// otros clientes** (`clientes`, en el orden dado: el que menos salió hoy primero; cada
/// uno una sola vez por tanda) y después con `relleno` (IDs, promos…).
pub fn ajustar_al_corte_con(
    sugerida: &[Pase],
    tope: Option<Duration>,
    clientes: &[(String, Duration)],
    relleno: &[(String, Duration)],
) -> (Vec<EnCorte>, Vec<Pase>) {
    // Las duraciones reales traen centésimas (10,01 s): medio segundo de holgura.
    let tope = tope.map(|t| t + Duration::from_millis(500));
    let mut tanda = Vec::new();
    let mut fuera = Vec::new();
    let mut total = Duration::ZERO;
    for p in sugerida {
        if tope.is_none_or(|t| total + p.duracion <= t) {
            total += p.duracion;
            tanda.push(EnCorte {
                nombre: p.nombre.clone(),
                clip: p.clip.clone(),
                duracion: p.duracion,
                pase: Some(p.clave.clone()),
            });
        } else {
            fuera.push(p.clone());
        }
    }
    if let Some(t) = tope {
        // Spots de otros clientes, sin repetir ninguno de la tanda.
        for (clip, d) in clientes {
            if d.is_zero() || total + *d > t || tanda.iter().any(|x: &EnCorte| x.clip == *clip) {
                continue;
            }
            total += *d;
            tanda.push(EnCorte { nombre: "Relleno".into(), clip: clip.clone(), duracion: *d, pase: None });
        }
        // El más largo que entre (llena mejor) sin repetir el anterior; si hay varios
        // iguales, se van turnando.
        let mut utiles: Vec<&(String, Duration)> = relleno.iter().filter(|(_, d)| !d.is_zero()).collect();
        utiles.sort_by_key(|x| std::cmp::Reverse(x.1));
        let mut usados: HashMap<&str, u32> = HashMap::new();
        let mut ultimo: Option<&str> = None;
        while tanda.len() < 200 {
            let resto = t.saturating_sub(total);
            let elegido = utiles
                .iter()
                .filter(|(c, d)| *d <= resto && (ultimo != Some(c.as_str()) || utiles.len() == 1))
                .min_by_key(|(c, d)| (std::cmp::Reverse(*d), usados.get(c.as_str()).copied().unwrap_or(0)));
            let Some((clip, d)) = elegido else { break };
            total += *d;
            tanda.push(EnCorte { nombre: "Relleno".into(), clip: clip.clone(), duracion: *d, pase: None });
            *usados.entry(clip.as_str()).or_default() += 1;
            ultimo = Some(clip.as_str());
        }
    }
    (tanda, fuera)
}

/// Un corte del día simulado.
#[derive(Debug, Clone, PartialEq)]
pub struct CorteSimulado {
    pub hora: Time,
    pub tanda: Vec<EnCorte>,
    pub total: Duration,
    /// Pases en ventana que no entraron en este corte.
    pub no_entran: Vec<String>,
}

/// Simula un día: supone un corte a la hora de cada pase y arma la tanda como lo haría
/// el modo AUTO. Devuelve los cortes y los pases que se perderían (no entraron en
/// ningún corte dentro de su ventana).
pub fn simular_dia(
    pautas: &[Pauta],
    fecha: Date,
    orden: Orden,
    tope: Option<Duration>,
    recuperar_min: u32,
    clientes: &[(String, Duration)],
    relleno: &[(String, Duration)],
) -> (Vec<CorteSimulado>, Vec<Pase>) {
    let pases = pases_del_dia(pautas, fecha);
    let mut horas: Vec<Time> = pases.iter().map(|p| p.hora).collect();
    horas.dedup();
    let mut emitidos = HashSet::new();
    let mut cortes = Vec::new();
    for h in horas {
        let sugerida = tanda_del_corte(&pases, h, &emitidos, orden, recuperar_min);
        if sugerida.is_empty() {
            continue;
        }
        let (tanda, fuera) = ajustar_al_corte_con(&sugerida, tope, clientes, relleno);
        for t in &tanda {
            if let Some(c) = &t.pase {
                emitidos.insert(c.clone());
            }
        }
        // Lo que no salió y ya era su último corte posible también se pierde (lo dice abajo).
        let total = tanda.iter().map(|t| t.duracion).sum();
        cortes.push(CorteSimulado {
            hora: h,
            tanda,
            total,
            no_entran: fuera.iter().map(|p| p.nombre.clone()).collect(),
        });
    }
    let perdidos = pases.into_iter().filter(|p| !emitidos.contains(&p.clave)).collect();
    (cortes, perdidos)
}

// ======================================================================
// Guardado en disco (JSON legibles)
// ======================================================================

#[derive(Serialize, Deserialize)]
struct ArchivoPautas {
    version: u32,
    pautas: Vec<Pauta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Emision {
    pub clave: String,
    pub clip: String,
    /// Hora real en que salió, `HH:MM:SS`.
    pub hora: String,
}

/// Carpeta de datos: `pautas.json` y `emitidos/AAAA-MM-DD.json`.
#[derive(Debug, Clone)]
pub struct Almacen {
    carpeta: PathBuf,
}

impl Almacen {
    pub fn new(carpeta: PathBuf) -> Self {
        Almacen { carpeta }
    }

    pub fn carpeta(&self) -> &Path {
        &self.carpeta
    }

    fn ruta_pautas(&self) -> PathBuf {
        self.carpeta.join("pautas.json")
    }

    fn ruta_emitidos(&self, fecha: Date) -> PathBuf {
        self.carpeta.join("emitidos").join(format!("{}.json", formato::texto_fecha(fecha)))
    }

    pub fn cargar_pautas(&self) -> Result<Vec<Pauta>, String> {
        match std::fs::read_to_string(self.ruta_pautas()) {
            Ok(t) => serde_json::from_str::<ArchivoPautas>(&t)
                .map(|a| a.pautas)
                .map_err(|e| format!("pautas.json dañado: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn guardar_pautas(&self, pautas: &[Pauta]) -> Result<(), String> {
        let a = ArchivoPautas { version: 1, pautas: pautas.to_vec() };
        escribir(&self.ruta_pautas(), &serde_json::to_string_pretty(&a).map_err(|e| e.to_string())?)
    }

    pub fn cargar_emitidos(&self, fecha: Date) -> Vec<Emision> {
        std::fs::read_to_string(self.ruta_emitidos(fecha))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn guardar_emitidos(&self, fecha: Date, emisiones: &[Emision]) -> Result<(), String> {
        escribir(&self.ruta_emitidos(fecha), &serde_json::to_string_pretty(emisiones).map_err(|e| e.to_string())?)
    }
}

/// Escribe a un temporal y lo renombra: si se corta la luz, no queda un archivo a medias.
fn escribir(ruta: &Path, texto: &str) -> Result<(), String> {
    if let Some(p) = ruta.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let tmp = ruta.with_extension("tmp");
    std::fs::write(&tmp, texto).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, ruta).map_err(|e| e.to_string())
}

// ======================================================================
// Formatos de fecha y hora (texto legible en los JSON)
// ======================================================================

pub mod formato {
    use serde::{Deserialize, Deserializer, Serializer};
    use time::{Date, Month, Time};

    pub fn texto_fecha(f: Date) -> String {
        format!("{:04}-{:02}-{:02}", f.year(), f.month() as u8, f.day())
    }

    /// `AAAA-MM-DD`.
    pub fn leer_fecha(t: &str) -> Option<Date> {
        let mut p = t.trim().split('-');
        let a: i32 = p.next()?.parse().ok()?;
        let m: u8 = p.next()?.parse().ok()?;
        let d: u8 = p.next()?.parse().ok()?;
        Date::from_calendar_date(a, Month::try_from(m).ok()?, d).ok()
    }

    /// `DD/MM/AAAA` (como lo escribe el operador).
    pub fn leer_fecha_dma(t: &str) -> Option<Date> {
        let mut p = t.trim().split(['/', '-', '.']);
        let d: u8 = p.next()?.trim().parse().ok()?;
        let m: u8 = p.next()?.trim().parse().ok()?;
        let a: i32 = p.next()?.trim().parse().ok()?;
        let a = if a < 100 { 2000 + a } else { a };
        Date::from_calendar_date(a, Month::try_from(m).ok()?, d).ok()
    }

    pub fn texto_fecha_dma(f: Date) -> String {
        format!("{:02}/{:02}/{:04}", f.day(), f.month() as u8, f.year())
    }

    /// Lee una hora escrita como sea: `08:00`, `8:00`, `0800`, `8.30`, `8`, `20h`,
    /// `8:00 PM`, `8pm`, `10:pm`, `8 p.m.`, `8:30a`.
    pub fn leer_hora(t: &str) -> Option<Time> {
        let t = normalizar(t);
        let (t, sufijo) = if let Some(r) = t.strip_suffix("am").or_else(|| t.strip_suffix('a')) {
            (r, Some(false))
        } else if let Some(r) = t.strip_suffix("pm").or_else(|| t.strip_suffix('p')) {
            (r, Some(true))
        } else {
            (t.strip_suffix("hrs").or_else(|| t.strip_suffix("hs")).or_else(|| t.strip_suffix('h')).unwrap_or(&t), None)
        };
        let t = t.trim().trim_end_matches(':').trim();
        if t.is_empty() || !t.chars().all(|c| c.is_ascii_digit() || c == ':') {
            return None;
        }
        let (h, m): (u8, u8) = match t.split_once(':') {
            Some((h, m)) => (h.trim().parse().ok()?, m.trim().parse().ok()?),
            None if t.len() == 4 => (t[..2].parse().ok()?, t[2..].parse().ok()?),
            None if t.len() == 3 => (t[..1].parse().ok()?, t[1..].parse().ok()?),
            None => (t.parse().ok()?, 0),
        };
        let h = match sufijo {
            Some(pm) => {
                if h == 0 || h > 12 {
                    return None;
                }
                match (pm, h) {
                    (false, 12) => 0,
                    (true, 12) => 12,
                    (true, h) => h + 12,
                    (false, h) => h,
                }
            }
            None => h,
        };
        Time::from_hms(h, m, 0).ok()
    }

    /// Minúsculas, sin espacios y con `a.m.`/`p.m.` → `am`/`pm`; `.` → `:`.
    fn normalizar(t: &str) -> String {
        let t = t.trim().to_lowercase().replace("a.m.", "am").replace("p.m.", "pm").replace("a. m.", "am");
        let t = t.replace("p. m.", "pm").replace("a.m", "am").replace("p.m", "pm");
        t.chars().filter(|c| !c.is_whitespace()).map(|c| if c == '.' { ':' } else { c }).collect()
    }

    /// Lee varias horas de un texto pegado: separadas por comas, `;`, espacios,
    /// saltos de línea, «y», `/` o `|`. `8:00 PM` (con espacio) cuenta como una sola.
    /// Ignora `...`. Devuelve las horas o el primer trozo que no se entendió.
    pub fn leer_horas(texto: &str) -> Result<Vec<Time>, String> {
        let t = texto.to_lowercase().replace('…', " ").replace("...", " ");
        let t = t.replace("a. m.", "am").replace("p. m.", "pm").replace("a.m.", "am").replace("p.m.", "pm");
        let mut trozos: Vec<String> = Vec::new();
        for p in t.split(|c: char| c == ',' || c == ';' || c == '/' || c == '|' || c.is_whitespace()) {
            let p = p.trim();
            if p.is_empty() || p == "y" || p == "-" {
                continue;
            }
            // Un sufijo suelto se pega a la hora anterior: «8:00 pm», «20 h».
            if matches!(p, "am" | "pm" | "a" | "p" | "h" | "hs" | "hrs" | "a.m" | "p.m") {
                if let Some(u) = trozos.last_mut() {
                    u.push_str(p);
                    continue;
                }
            }
            trozos.push(p.to_string());
        }
        let mut v = Vec::new();
        for p in trozos {
            v.push(leer_hora(&p).ok_or(p)?);
        }
        Ok(v)
    }

    pub fn texto_hora(h: Time) -> String {
        format!("{:02}:{:02}", h.hour(), h.minute())
    }

    pub mod fecha {
        use super::*;
        pub fn serialize<S: Serializer>(f: &Date, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&texto_fecha(*f))
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Date, D::Error> {
            let t = String::deserialize(d)?;
            leer_fecha(&t).ok_or_else(|| serde::de::Error::custom(format!("fecha no válida: {t}")))
        }
    }

    pub mod fecha_opcional {
        use super::*;
        pub fn serialize<S: Serializer>(f: &Option<Date>, s: S) -> Result<S::Ok, S::Error> {
            match f {
                Some(f) => s.serialize_str(&texto_fecha(*f)),
                None => s.serialize_none(),
            }
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Date>, D::Error> {
            let t = Option::<String>::deserialize(d)?;
            t.filter(|t| !t.is_empty())
                .map(|t| leer_fecha(&t).ok_or_else(|| serde::de::Error::custom(format!("fecha no válida: {t}"))))
                .transpose()
        }
    }

    pub mod fechas {
        use super::*;
        pub fn serialize<S: Serializer>(v: &[Date], s: S) -> Result<S::Ok, S::Error> {
            s.collect_seq(v.iter().map(|f| texto_fecha(*f)))
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Date>, D::Error> {
            Vec::<String>::deserialize(d)?
                .iter()
                .map(|t| leer_fecha(t).ok_or_else(|| serde::de::Error::custom(format!("fecha no válida: {t}"))))
                .collect()
        }
    }

    pub mod horas {
        use super::*;
        pub fn serialize<S: Serializer>(v: &[Time], s: S) -> Result<S::Ok, S::Error> {
            s.collect_seq(v.iter().map(|h| texto_hora(*h)))
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Time>, D::Error> {
            Vec::<String>::deserialize(d)?
                .iter()
                .map(|t| leer_hora(t).ok_or_else(|| serde::de::Error::custom(format!("hora no válida: {t}"))))
                .collect()
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::formato::{leer_fecha_dma, leer_hora};
    use super::*;
    use time::macros::{date, time};

    fn pauta(id: u64, nombre: &str, horas: &[Time], prioridad: u8) -> Pauta {
        Pauta {
            id,
            nombre: nombre.into(),
            clip: format!("CLIENTES/{nombre}"),
            duracion_seg: 30.0,
            inicio: date!(2026 - 11 - 01),
            fin: None,
            dias: Dias::Todos,
            horas: horas.to_vec(),
            tolerancia_min: 15,
            despues_min: None,
            prioridad,
            activa: true,
            cliente: String::new(),
            notas: String::new(),
        }
    }

    fn nombres(v: &[Pase]) -> Vec<&str> {
        v.iter().map(|p| p.nombre.as_str()).collect()
    }

    const HOY: Date = date!(2026 - 11 - 11); // miércoles

    #[test]
    fn ejemplo_10_15_y_10_30() {
        let pautas = [pauta(1, "A", &[time!(10:15)], 0), pauta(2, "B", &[time!(10:30)], 0)];
        let pases = pases_del_dia(&pautas, HOY);
        let vacio = HashSet::new();
        // A las 10:20 están las dos en ventana: primero la de las 10:15.
        assert_eq!(nombres(&tanda_sugerida(&pases, time!(10:20), &vacio, Orden::Hora)), ["A", "B"]);
        // Con orden por cercanía, a las 10:25 la de las 10:30 está más cerca.
        assert_eq!(nombres(&tanda_sugerida(&pases, time!(10:25), &vacio, Orden::Cercania)), ["B", "A"]);
        assert_eq!(nombres(&tanda_sugerida(&pases, time!(10:25), &vacio, Orden::Hora)), ["A", "B"]);
    }

    #[test]
    fn prioritaria_sale_primero_aunque_sea_mas_tarde() {
        let pautas =
            [pauta(1, "A", &[time!(10:15)], 0), pauta(2, "B", &[time!(10:30)], 0), pauta(3, "C", &[time!(10:30)], 1)];
        let pases = pases_del_dia(&pautas, HOY);
        assert_eq!(nombres(&tanda_sugerida(&pases, time!(10:20), &HashSet::new(), Orden::Hora)), ["C", "A", "B"]);
    }

    #[test]
    fn lo_que_no_salio_queda_pendiente_un_tiempo() {
        let pases = pases_del_dia(&[pauta(1, "A", &[time!(10:00)], 0)], HOY);
        let v = HashSet::new();
        // Ventana hasta las 10:15; se recupera hasta 60 min después (11:15).
        assert_eq!(pases[0].estado_con(time!(10:16), &v, 60), EstadoPase::Pendiente);
        assert_eq!(pases[0].estado_con(time!(11:15), &v, 60), EstadoPase::Pendiente);
        assert_eq!(pases[0].estado_con(time!(11:16), &v, 60), EstadoPase::Perdido);
        assert_eq!(pases[0].estado_con(time!(10:16), &v, 0), EstadoPase::Perdido);
    }

    #[test]
    fn tanda_nueva_que_ya_toca_va_antes_que_lo_pendiente() {
        // Tanda de las 10:00 (A, B) que no terminó; a las 10:45 toca la de las 10:45 (C, D★).
        let pautas = [
            pauta(1, "A", &[time!(10:00)], 0),
            pauta(2, "B", &[time!(10:00)], 0),
            pauta(3, "C", &[time!(10:45)], 0),
            pauta(4, "D", &[time!(10:45)], 1),
        ];
        let pases = pases_del_dia(&pautas, HOY);
        let emitidos: HashSet<String> = [pases[0].clave.clone()].into(); // A salió; B no
        let t = tanda_del_corte(&pases, time!(10:45), &emitidos, Orden::Hora, 120);
        assert_eq!(nombres(&t), ["D", "C", "B"]);
        // A las 10:35 la nueva todavía no toca (faltan 10 min): primero lo pendiente.
        let t = tanda_del_corte(&pases, time!(10:35), &emitidos, Orden::Hora, 120);
        assert_eq!(nombres(&t), ["D", "B", "C"]);
        // Sin recuperar, B ya no entra.
        let t = tanda_del_corte(&pases, time!(10:45), &emitidos, Orden::Hora, 0);
        assert_eq!(nombres(&t), ["D", "C"]);
    }

    #[test]
    fn la_tanda_detenida_sigue_donde_quedo() {
        // 4 spots de las 10:00; salieron 1 y 2, el 3 se cortó (no cuenta): sigue 3, 4.
        let pautas: Vec<Pauta> = ["S1", "S2", "S3", "S4"]
            .iter()
            .enumerate()
            .map(|(i, n)| pauta(i as u64 + 1, n, &[time!(10:00)], 0))
            .collect();
        let pases = pases_del_dia(&pautas, HOY);
        let emitidos: HashSet<String> = [pases[0].clave.clone(), pases[1].clave.clone()].into();
        assert_eq!(nombres(&tanda_del_corte(&pases, time!(10:05), &emitidos, Orden::Hora, 60)), ["S3", "S4"]);
        // Igual si ya se cerró su ventana (pendientes).
        assert_eq!(nombres(&tanda_del_corte(&pases, time!(10:40), &emitidos, Orden::Hora, 60)), ["S3", "S4"]);
    }

    #[test]
    fn relleno_con_spots_de_otros_clientes_primero() {
        let pases = pases_del_dia(&[pauta(1, "A", &[time!(10:00)], 0)], HOY);
        let s = |x| Duration::from_secs(x);
        let clientes =
            [("CLIENTES/A".to_string(), s(30)), ("CLIENTES/X".to_string(), s(20)), ("CLIENTES/Y".to_string(), s(20))];
        let relleno = [("IDS/ID".to_string(), s(5))];
        let (t, _) = ajustar_al_corte_con(&pases, Some(s(60)), &clientes, &relleno);
        let clips: Vec<&str> = t.iter().map(|x| x.clip.as_str()).collect();
        // A (su pase), X (el primero de la lista que entra; A ya está), y 2 IDs para los 10 s.
        assert_eq!(clips, ["CLIENTES/A", "CLIENTES/X", "IDS/ID", "IDS/ID"]);
    }

    #[test]
    fn leer_horas_pegadas() {
        use super::formato::leer_horas;
        let v = leer_horas("6:00, 8:30, 10:30, 10:45, 11:30 ... 8:00PM, 10:pm").unwrap();
        assert_eq!(
            v,
            vec![time!(6:00), time!(8:30), time!(10:30), time!(10:45), time!(11:30), time!(20:00), time!(22:00)]
        );
        let v = leer_horas("8:00 pm y 9 p.m.\n7 am; 20 h").unwrap();
        assert_eq!(v, vec![time!(20:00), time!(21:00), time!(7:00), time!(20:00)]);
        assert_eq!(leer_horas("8:00, nueve"), Err("nueve".to_string()));
    }

    #[test]
    fn ventana_antes_y_despues() {
        let mut p = pauta(1, "A", &[time!(10:30)], 0);
        p.tolerancia_min = 5;
        p.despues_min = Some(30);
        let pases = pases_del_dia(&[p], date!(2026 - 11 - 02));
        let e = HashSet::new();
        assert_eq!(pases[0].estado(time!(10:24), &e), EstadoPase::Programado);
        assert_eq!(pases[0].estado(time!(10:25), &e), EstadoPase::Vigente);
        assert_eq!(pases[0].estado(time!(11:00), &e), EstadoPase::Vigente);
        assert_eq!(pases[0].estado(time!(11:01), &e), EstadoPase::Perdido);
        assert_eq!(pases[0].estado_con(time!(11:30), &e, 60), EstadoPase::Pendiente);
    }

    #[test]
    fn ventana_de_tolerancia() {
        let pases = pases_del_dia(&[pauta(1, "A", &[time!(10:30)], 0)], HOY);
        let v = HashSet::new();
        assert_eq!(pases[0].estado(time!(10:14), &v), EstadoPase::Programado);
        assert_eq!(pases[0].estado(time!(10:15), &v), EstadoPase::Vigente);
        assert_eq!(pases[0].estado(time!(10:45), &v), EstadoPase::Vigente);
        assert_eq!(pases[0].estado(time!(10:46), &v), EstadoPase::Perdido);
    }

    #[test]
    fn emitidos_no_se_repiten() {
        let pautas = [pauta(1, "A", &[time!(10:15)], 0), pauta(2, "B", &[time!(10:30)], 0)];
        let pases = pases_del_dia(&pautas, HOY);
        let emitidos: HashSet<String> = [pases[0].clave.clone()].into();
        assert_eq!(nombres(&tanda_sugerida(&pases, time!(10:20), &emitidos, Orden::Hora)), ["B"]);
        assert_eq!(pases[0].estado(time!(10:20), &emitidos), EstadoPase::Emitido);
    }

    #[test]
    fn mismo_spot_una_sola_vez_por_tanda() {
        let pases = pases_del_dia(&[pauta(1, "A", &[time!(10:00), time!(10:20)], 0)], HOY);
        let t = tanda_sugerida(&pases, time!(10:10), &HashSet::new(), Orden::Hora);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].hora, time!(10:00));
    }

    #[test]
    fn dias_de_la_semana_cada_n_y_fechas() {
        let mut p = pauta(1, "A", &[time!(10:00)], 0);
        p.dias = Dias::Semana { dias: [true, true, true, true, true, false, false] };
        assert!(p.toca(HOY));
        assert!(!p.toca(date!(2026 - 11 - 14))); // sábado
        assert_eq!(p.describir_dias(), "lunes a viernes");

        p.dias = Dias::CadaN { n: 2 };
        assert!(p.toca(date!(2026 - 11 - 01)));
        assert!(!p.toca(date!(2026 - 11 - 02)));
        assert!(p.toca(date!(2026 - 11 - 03)));
        assert_eq!(p.describir_dias(), "interdiario");

        p.dias = Dias::Fechas { fechas: vec![date!(2026 - 11 - 20)] };
        assert!(!p.toca(HOY));
        assert!(p.toca(date!(2026 - 11 - 20)));
    }

    #[test]
    fn inicio_fin_e_inactiva() {
        let mut p = pauta(1, "A", &[time!(10:00)], 0);
        assert!(!p.toca(date!(2026 - 10 - 31)));
        p.fin = Some(date!(2026 - 11 - 10));
        assert!(!p.toca(HOY));
        p.fin = None;
        p.activa = false;
        assert!(!p.toca(HOY));
        assert!(pases_del_dia(&[p], HOY).is_empty());
    }

    #[test]
    fn leer_horas_y_fechas_escritas_a_mano() {
        assert_eq!(leer_hora("9:30"), Some(time!(9:30)));
        assert_eq!(leer_hora("0930"), Some(time!(9:30)));
        assert_eq!(leer_hora("21.15"), Some(time!(21:15)));
        assert_eq!(leer_hora("8"), Some(time!(8:00)));
        assert_eq!(leer_hora("25:00"), None);
        assert_eq!(leer_hora("8:00PM"), Some(time!(20:00)));
        assert_eq!(leer_hora("10:pm"), Some(time!(22:00)));
        assert_eq!(leer_hora("8 p.m."), Some(time!(20:00)));
        assert_eq!(leer_hora("12am"), Some(time!(0:00)));
        assert_eq!(leer_hora("12:30 pm"), Some(time!(12:30)));
        assert_eq!(leer_hora("6:15a"), Some(time!(6:15)));
        assert_eq!(leer_hora("20h"), Some(time!(20:00)));
        assert_eq!(leer_hora("830"), Some(time!(8:30)));
        assert_eq!(leer_hora("13pm"), None);
        assert_eq!(leer_hora("hola"), None);
        assert_eq!(leer_fecha_dma("15/11/2026"), Some(date!(2026 - 11 - 15)));
        assert_eq!(leer_fecha_dma("1-2-26"), Some(date!(2026 - 02 - 01)));
        assert_eq!(leer_fecha_dma("31/02/2026"), None);
    }

    #[test]
    fn guardar_y_cargar() {
        let d = std::env::temp_dir().join(format!("relevo-pauta-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let a = Almacen::new(d);
        assert!(a.cargar_pautas().unwrap().is_empty());
        let mut p = pauta(7, "Ferretería X", &[time!(8:00), time!(13:30)], 1);
        p.fin = Some(date!(2026 - 12 - 31));
        p.dias = Dias::Fechas { fechas: vec![HOY] };
        a.guardar_pautas(&[p.clone()]).unwrap();
        assert_eq!(a.cargar_pautas().unwrap(), vec![p]);

        let e = vec![Emision { clave: "7@2026-11-11@08:00".into(), clip: "X".into(), hora: "08:03:10".into() }];
        a.guardar_emitidos(HOY, &e).unwrap();
        assert_eq!(a.cargar_emitidos(HOY)[0].clave, "7@2026-11-11@08:00");
        assert!(a.cargar_emitidos(date!(2026 - 11 - 12)).is_empty());
    }

    fn pase_de(nombre: &str, dur: u64, prioridad: u8) -> Pase {
        Pase {
            clave: format!("{nombre}@x"),
            pauta_id: 1,
            nombre: nombre.into(),
            clip: nombre.to_uppercase(),
            duracion: Duration::from_secs(dur),
            hora: time!(10:00),
            prioridad,
            tolerancia_min: 15,
            despues_min: 15,
        }
    }

    #[test]
    fn tope_del_corte_con_prioridades_y_relleno() {
        // Ya ordenada: prioritario primero.
        let sugerida = vec![pase_de("C", 30, 1), pase_de("A", 45, 0), pase_de("B", 20, 0)];
        // Sin tope: todo.
        let (t, f) = ajustar_al_corte(&sugerida, None, &[]);
        assert_eq!(t.len(), 3);
        assert!(f.is_empty());
        // Tope 60 s: C (30) entra, A (45) no, B (20) sí → 50 s.
        let (t, f) = ajustar_al_corte(&sugerida, Some(Duration::from_secs(60)), &[]);
        assert_eq!(t.iter().map(|x| x.nombre.as_str()).collect::<Vec<_>>(), ["C", "B"]);
        assert_eq!(f[0].nombre, "A");
        // Con relleno de 5 s y 10 s: completa los 10 s que sobran sin pasarse.
        let relleno =
            vec![("IDS/ID".to_string(), Duration::from_secs(5)), ("PROMOS/P".to_string(), Duration::from_secs(10))];
        let (t, _) = ajustar_al_corte(&sugerida, Some(Duration::from_secs(60)), &relleno);
        let total: Duration = t.iter().map(|x| x.duracion).sum();
        assert_eq!(total, Duration::from_secs(60));
        assert!(t.iter().skip(2).all(|x| x.pase.is_none()));
        // Nunca dos rellenos iguales seguidos (si hay más de uno).
        let rellenos: Vec<&str> = t.iter().filter(|x| x.pase.is_none()).map(|x| x.clip.as_str()).collect();
        assert!(rellenos.windows(2).all(|w| w[0] != w[1]), "{rellenos:?}");
    }

    #[test]
    fn simula_un_dia() {
        let mut a = pauta(1, "A", &[time!(10:00), time!(10:20)], 0);
        a.duracion_seg = 40.0;
        let mut b = pauta(2, "B", &[time!(10:05)], 0);
        b.duracion_seg = 40.0;
        let fecha = date!(2026 - 11 - 11);
        // Sin tope: a las 10:00 salen A y B (B está en ventana); a las 10:20, A otra vez.
        let (cortes, perdidos) = simular_dia(&[a.clone(), b.clone()], fecha, Orden::Hora, None, 0, &[], &[]);
        assert_eq!(cortes.len(), 2);
        assert_eq!(cortes[0].tanda.len(), 2);
        assert!(perdidos.is_empty());
        // Con tope de 40 s: a las 10:00 solo A; B pasa al corte de las 10:05.
        let (cortes, perdidos) = simular_dia(&[a, b], fecha, Orden::Hora, Some(Duration::from_secs(40)), 0, &[], &[]);
        assert_eq!(cortes[0].tanda.len(), 1);
        assert_eq!(cortes[0].no_entran, vec!["B".to_string()]);
        assert_eq!(cortes[1].hora, time!(10:05));
        assert!(perdidos.is_empty());
    }
}
