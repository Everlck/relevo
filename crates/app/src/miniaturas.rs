//! Miniaturas de los videos hechas con ffmpeg (un cuadro del video), para los
//! archivos de cualquier carpeta. Se hacen en segundo plano, de a una, y se guardan
//! en `datos/miniaturas` para no repetir el trabajo.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};

/// Ancho de la miniatura (el alto sale de la proporción del video).
const ANCHO: u32 = 240;

#[derive(Clone)]
pub enum Estado {
    Pendiente,
    Lista(Arc<Vec<u8>>),
    NoDisponible,
}

#[derive(Default)]
struct Interno {
    hechas: HashMap<PathBuf, Estado>,
    cola: VecDeque<PathBuf>,
    ffmpeg: Option<PathBuf>,
}

pub struct Miniaturas {
    interno: Arc<(Mutex<Interno>, Condvar)>,
}

impl Miniaturas {
    pub fn new(ffmpeg: Option<PathBuf>, carpeta_local: &Path, ctx: egui::Context) -> Self {
        let interno = Arc::new((Mutex::new(Interno { ffmpeg, ..Default::default() }), Condvar::new()));
        let cache = carpeta_local.join("miniaturas");
        let i = interno.clone();
        std::thread::Builder::new()
            .name("miniaturas".into())
            .spawn(move || trabajar(i, cache, ctx))
            .expect("hilo de miniaturas");
        Miniaturas { interno }
    }

    pub fn usar_ffmpeg(&self, f: Option<PathBuf>) {
        let mut i = self.interno.0.lock().expect("miniaturas");
        i.ffmpeg = f;
        // Lo que no se pudo hacer sin ffmpeg se vuelve a intentar.
        i.hechas.retain(|_, e| !matches!(e, Estado::NoDisponible));
    }

    /// Miniatura de un archivo de video. La primera vez la encarga y devuelve `Pendiente`.
    pub fn pedir(&self, archivo: &Path) -> Estado {
        let mut i = self.interno.0.lock().expect("miniaturas");
        if let Some(e) = i.hechas.get(archivo) {
            return e.clone();
        }
        if i.ffmpeg.is_none() {
            return Estado::NoDisponible;
        }
        i.hechas.insert(archivo.to_path_buf(), Estado::Pendiente);
        i.cola.push_back(archivo.to_path_buf());
        self.interno.1.notify_one();
        Estado::Pendiente
    }
}

fn trabajar(interno: Arc<(Mutex<Interno>, Condvar)>, cache: PathBuf, ctx: egui::Context) {
    loop {
        let (archivo, ffmpeg) = {
            let mut i = interno.0.lock().expect("miniaturas");
            loop {
                // Lo último pedido primero: es lo que se está viendo ahora.
                if let Some(a) = i.cola.pop_back() {
                    break (a, i.ffmpeg.clone());
                }
                i = interno.1.wait(i).expect("miniaturas");
            }
        };
        let resultado = match hacer(&archivo, ffmpeg.as_deref(), &cache) {
            Some(png) => Estado::Lista(Arc::new(png)),
            None => Estado::NoDisponible,
        };
        interno.0.lock().expect("miniaturas").hechas.insert(archivo, resultado);
        ctx.request_repaint();
    }
}

/// Nombre del archivo guardado: cambia si el video cambia (tamaño o fecha).
fn nombre_cache(archivo: &Path) -> Option<String> {
    let m = std::fs::metadata(archivo).ok()?;
    let fecha = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
    let texto = format!("{}|{}|{:?}", archivo.display(), m.len(), fecha);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in texto.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    Some(format!("{h:016x}.png"))
}

fn hacer(archivo: &Path, ffmpeg: Option<&Path>, cache: &Path) -> Option<Vec<u8>> {
    let guardado = cache.join(nombre_cache(archivo)?);
    if let Ok(png) = std::fs::read(&guardado) {
        if !png.is_empty() {
            return Some(png);
        }
    }
    let ffmpeg = ffmpeg?;
    // Un cuadro a 1 s del inicio (evita el negro del primer cuadro); si el video es
    // más corto, el primero.
    let png = ["1", "0"].iter().find_map(|desde| {
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error", "-ss", desde, "-i"])
            .arg(archivo)
            .args(["-frames:v", "1", "-an", "-vf", &format!("scale={ANCHO}:-2:flags=fast_bilinear")])
            .args(["-f", "image2pipe", "-vcodec", "png", "-"])
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        crate::vista_previa::sin_ventana(&mut cmd);
        let salida = cmd.output().ok()?;
        (salida.stdout.len() > 100).then_some(salida.stdout)
    })?;
    let _ = std::fs::create_dir_all(cache);
    let _ = std::fs::write(&guardado, &png);
    Some(png)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_nombre_cambia_si_cambia_el_video() {
        let d = std::env::temp_dir().join(format!("relevo-mini-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let v = d.join("a.mp4");
        std::fs::write(&v, b"uno").unwrap();
        let a = nombre_cache(&v).unwrap();
        assert_eq!(a, nombre_cache(&v).unwrap());
        std::fs::write(&v, b"otro contenido").unwrap();
        assert_ne!(a, nombre_cache(&v).unwrap());
        assert!(nombre_cache(&d.join("no-existe.mp4")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
