//! Lector de una señal de video (capturadora, página web, IPTV, pantalla o archivo)
//! con ffmpeg, en color y sin sonido.
//!
//! Lo usan a la vez el monitor PREVIEW, el monitor PROGRAMA (en modo Transmisión) y la
//! detección de corte. Si piden la misma fuente, se comparte **un solo ffmpeg**: una
//! capturadora, por ejemplo, no se puede abrir dos veces a la vez en Windows.
//!
//! El lector vive mientras alguien lo use (un `Arc<Lector>`); al soltarse el último,
//! se cierra su ffmpeg.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use crate::deteccion::{buscar_flujo, entrada_con};

/// Tamaño y cuadros por segundo de la señal leída.
pub const ANCHO: usize = 640;
pub const ALTO: usize = 360;
pub const FPS: f64 = 10.0;

/// De dónde viene la señal. Dos lectores con la misma `Fuente` son el mismo.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fuente {
    /// "capturadora", "web", "pantalla", "archivo" o "url".
    pub tipo: String,
    /// Nombre de la capturadora, dirección (página, video o IPTV) o ruta del archivo.
    pub origen: String,
    /// Con "pantalla": número de monitor (0 = todo el escritorio).
    pub monitor: u32,
}

impl Fuente {
    /// `tipo` "iptv" se convierte en la dirección del canal (como una página web).
    pub fn nueva(tipo: &str, origen: &str, monitor: u32, iptv: &str) -> Option<Fuente> {
        let (tipo, origen) = match tipo {
            "" | "ninguna" => return None,
            "iptv" => ("web", iptv.trim()),
            "url" => ("web", origen.trim()),
            t => (t, origen.trim()),
        };
        if origen.is_empty() && tipo != "pantalla" {
            return None;
        }
        Some(Fuente {
            tipo: tipo.to_string(),
            origen: origen.to_string(),
            monitor: if tipo == "pantalla" { monitor } else { 0 },
        })
    }

    /// Texto corto para mostrar.
    pub fn describir(&self) -> String {
        match self.tipo.as_str() {
            "capturadora" => format!("Capturadora: {}", self.origen),
            "pantalla" if self.monitor > 0 => format!("Pantalla: monitor {}", self.monitor),
            "pantalla" => "Pantalla: todo el escritorio".into(),
            "archivo" => {
                let n = std::path::Path::new(&self.origen).file_name().map(|n| n.to_string_lossy().into_owned());
                format!("Archivo: {}", n.unwrap_or_else(|| self.origen.clone()))
            }
            _ => self.origen.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EstadoSenal {
    Conectando,
    /// Llegan cuadros.
    Activa,
    Error(String),
}

/// Un cuadro en color (RGB, ANCHO × ALTO). `serie` crece con cada cuadro.
pub type Cuadro = (u64, Arc<Vec<u8>>);

struct Compartido {
    ultimo: Mutex<Option<Cuadro>>,
    estado: Mutex<EstadoSenal>,
    /// Dirección del video encontrada dentro de la página web.
    flujo: Mutex<Option<String>>,
    suscriptores: Mutex<Vec<SyncSender<Cuadro>>>,
    parar: AtomicBool,
    hijo: Mutex<Option<Child>>,
    /// Instante (ms desde `inicio`) del último cuadro.
    ultimo_ms: AtomicU64,
    inicio: Instant,
}

pub struct Lector {
    pub fuente: Fuente,
    c: Arc<Compartido>,
}

impl Lector {
    pub fn ultimo(&self) -> Option<Cuadro> {
        self.c.ultimo.lock().expect("ultimo").clone()
    }

    pub fn estado(&self) -> EstadoSenal {
        self.c.estado.lock().expect("estado").clone()
    }

    pub fn flujo(&self) -> Option<String> {
        self.c.flujo.lock().expect("flujo").clone()
    }

    /// Recibe cada cuadro nuevo (si el que recibe se atrasa, se saltan cuadros).
    pub fn suscribir(&self) -> Receiver<Cuadro> {
        let (tx, rx) = mpsc::sync_channel(3);
        self.c.suscriptores.lock().expect("suscriptores").push(tx);
        rx
    }
}

impl Drop for Lector {
    fn drop(&mut self) {
        self.c.parar.store(true, Ordering::SeqCst);
        if let Some(mut h) = self.c.hijo.lock().expect("hijo").take() {
            let _ = h.kill();
            let _ = h.wait();
        }
    }
}

type Registro = Mutex<Vec<(Fuente, Weak<Lector>)>>;
static LECTORES: Registro = Mutex::new(Vec::new());

/// El lector de esa fuente: el que ya existe o uno nuevo.
pub fn lector(fuente: &Fuente, ffmpeg: PathBuf) -> Arc<Lector> {
    let mut r = LECTORES.lock().expect("lectores");
    r.retain(|(_, w)| w.strong_count() > 0);
    if let Some(l) = r.iter().find(|(f, _)| f == fuente).and_then(|(_, w)| w.upgrade()) {
        return l;
    }
    let c = Arc::new(Compartido {
        ultimo: Mutex::new(None),
        estado: Mutex::new(EstadoSenal::Conectando),
        flujo: Mutex::new(None),
        suscriptores: Mutex::new(Vec::new()),
        parar: AtomicBool::new(false),
        hijo: Mutex::new(None),
        ultimo_ms: AtomicU64::new(0),
        inicio: Instant::now(),
    });
    let l = Arc::new(Lector { fuente: fuente.clone(), c: c.clone() });
    let f = fuente.clone();
    std::thread::Builder::new().name("senal".into()).spawn(move || trabajar(f, ffmpeg, c)).expect("hilo de señal");
    r.push((fuente.clone(), Arc::downgrade(&l)));
    l
}

fn trabajar(fuente: Fuente, ffmpeg: PathBuf, c: Arc<Compartido>) {
    let parar = || c.parar.load(Ordering::SeqCst);
    let poner = |e: EstadoSenal| *c.estado.lock().expect("estado") = e;
    let dormir = |seg: u64| {
        let hasta = Instant::now() + Duration::from_secs(seg);
        while Instant::now() < hasta && !parar() {
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let mut serie = 0u64;
    while !parar() {
        poner(EstadoSenal::Conectando);
        // Página web: primero hay que encontrar su video (en cada conexión, porque
        // esas direcciones suelen caducar).
        let flujo = if fuente.tipo == "web" {
            match buscar_flujo(&fuente.origen) {
                Ok(f) => {
                    *c.flujo.lock().expect("flujo") = Some(f.clone());
                    Some(f)
                }
                Err(e) => {
                    poner(EstadoSenal::Error(e));
                    dormir(20);
                    continue;
                }
            }
        } else {
            None
        };
        let monitor = if fuente.tipo == "pantalla" && fuente.monitor > 0 {
            crate::arranque::monitores()
                .iter()
                .find(|m| m.numero == fuente.monitor)
                .map(|m| (m.x, m.y, m.ancho, m.alto))
        } else {
            None
        };
        let args = match entrada_con(&fuente, flujo.as_deref(), monitor) {
            Ok(a) => a,
            Err(e) => {
                poner(EstadoSenal::Error(e));
                return;
            }
        };
        let mut cmd = Command::new(&ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error"])
            .args(&args)
            .args(["-an", "-vf", &format!("fps={FPS},scale={ANCHO}:{ALTO},format=rgb24")])
            .args(["-f", "rawvideo", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::vista_previa::sin_ventana(&mut cmd);
        let mut proceso = match cmd.spawn() {
            Ok(p) => p,
            Err(e) => {
                poner(EstadoSenal::Error(format!("No se pudo abrir ffmpeg: {e}")));
                return;
            }
        };
        let mut salida = proceso.stdout.take().expect("stdout");
        let mut errores = proceso.stderr.take().expect("stderr");
        // ffmpeg escribe sus errores mientras corre: se leen aparte para que no se llene.
        let texto_err = Arc::new(Mutex::new(String::new()));
        {
            let t = texto_err.clone();
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = errores.read_to_string(&mut s);
                *t.lock().expect("errores") = s;
            });
        }
        *c.hijo.lock().expect("hijo") = Some(proceso);
        if parar() {
            break;
        }

        let mut cuadros = 0u64;
        loop {
            let mut buf = vec![0u8; ANCHO * ALTO * 3];
            if parar() || salida.read_exact(&mut buf).is_err() {
                break;
            }
            cuadros += 1;
            serie += 1;
            if cuadros == 1 {
                poner(EstadoSenal::Activa);
            }
            let cuadro: Cuadro = (serie, Arc::new(buf));
            *c.ultimo.lock().expect("ultimo") = Some(cuadro.clone());
            c.ultimo_ms.store(c.inicio.elapsed().as_millis().max(1) as u64, Ordering::Relaxed);
            c.suscriptores.lock().expect("suscriptores").retain(|tx| match tx.try_send(cuadro.clone()) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => true,
                Err(mpsc::TrySendError::Disconnected(_)) => false,
            });
        }
        if let Some(mut h) = c.hijo.lock().expect("hijo").take() {
            let _ = h.kill();
            let _ = h.wait();
        }
        if parar() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
        let texto = texto_err.lock().expect("errores").clone();
        let motivo = motivo_de(&texto);
        let mensaje = if cuadros == 0 {
            format!("No llega la señal. {motivo}")
        } else {
            format!("Se cortó la señal; reintentando… {motivo}")
        };
        tracing::warn!("señal {}: {mensaje}", fuente.describir());
        poner(EstadoSenal::Error(mensaje.trim().to_string()));
        dormir(4);
    }
}

/// La última línea útil de ffmpeg, sin su prefijo "[algo @ 0x…]".
pub fn motivo_de(texto: &str) -> String {
    texto
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.contains("Nothing was written") && !l.starts_with("Conversion failed"))
        .map(|l| match l.split_once("] ") {
            Some((antes, resto)) if antes.starts_with('[') => resto,
            _ => l,
        })
        .unwrap_or("")
        .chars()
        .take(140)
        .collect()
}

/// Reduce un cuadro en color a grises de la mitad de tamaño (promedio de 2×2 puntos).
pub fn a_grises_mitad(rgb: &[u8]) -> Vec<u8> {
    let (w, h) = (ANCHO / 2, ALTO / 2);
    let mut v = vec![0u8; w * h];
    let luma = |i: usize| (rgb[i] as u32 * 77 + rgb[i + 1] as u32 * 150 + rgb[i + 2] as u32 * 29) >> 8;
    for y in 0..h {
        for x in 0..w {
            let a = ((2 * y) * ANCHO + 2 * x) * 3;
            let b = ((2 * y + 1) * ANCHO + 2 * x) * 3;
            let s = luma(a) + luma(a + 3) + luma(b) + luma(b + 3);
            v[y * w + x] = (s / 4) as u8;
        }
    }
    v
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn fuentes() {
        assert_eq!(Fuente::nueva("ninguna", "x", 0, ""), None);
        assert_eq!(Fuente::nueva("capturadora", "", 0, ""), None);
        let f = Fuente::nueva("iptv", "", 3, " udp://@239.1.1.1:1234 ").unwrap();
        assert_eq!((f.tipo.as_str(), f.origen.as_str(), f.monitor), ("web", "udp://@239.1.1.1:1234", 0));
        // La misma fuente pedida de dos formas es la misma.
        assert_eq!(Fuente::nueva("url", "udp://@239.1.1.1:1234", 0, ""), Some(f));
        let p = Fuente::nueva("pantalla", "", 2, "").unwrap();
        assert_eq!(p.describir(), "Pantalla: monitor 2");
    }

    #[test]
    fn grises_a_la_mitad() {
        let mut rgb = vec![0u8; ANCHO * ALTO * 3];
        // Un punto blanco de 2×2 arriba a la izquierda.
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let i = (y * ANCHO + x) * 3;
            rgb[i..i + 3].copy_from_slice(&[255, 255, 255]);
        }
        let g = a_grises_mitad(&rgb);
        assert_eq!(g.len(), (ANCHO / 2) * (ALTO / 2));
        assert!(g[0] >= 254 && g[1] == 0, "{} {}", g[0], g[1]);
    }

    #[test]
    fn el_mismo_lector_se_comparte() {
        let f = Fuente::nueva("archivo", "/no/existe.mp4", 0, "").unwrap();
        let a = lector(&f, PathBuf::from("ffmpeg-que-no-existe"));
        let b = lector(&f, PathBuf::from("ffmpeg-que-no-existe"));
        assert!(Arc::ptr_eq(&a, &b));
        drop((a, b));
        // Soltado el último, se abre otro nuevo.
        let c = lector(&f, PathBuf::from("ffmpeg-que-no-existe"));
        std::thread::sleep(Duration::from_millis(100));
        assert!(matches!(c.estado(), EstadoSenal::Error(_)));
    }
}
