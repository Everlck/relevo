//! Monitor de **programa**: muestra en la app el spot que está al aire, en el mismo
//! punto en que lo está reproduciendo CasparCG.
//!
//! ffmpeg decodifica el mismo archivo (sin sonido, en pequeño) desde el segundo que
//! informa CasparCG; si se adelanta o atrasa más de medio segundo, o cambia el spot,
//! vuelve a empezar desde el punto correcto.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ANCHO: usize = 384;
const ALTO: usize = 216;
const FPS: f64 = 15.0;
/// Diferencia con CasparCG a partir de la cual se vuelve a sincronizar.
const DESFASE_MAX: f64 = 0.6;

struct Compartido {
    cuadro: Mutex<Option<(Vec<u8>, f64)>>,
    detener: AtomicBool,
}

/// Lo que se está mostrando.
struct Fuente {
    archivo: PathBuf,
    /// Segundo del archivo en el que arrancó la decodificación, y cuándo.
    desde: f64,
    inicio: Instant,
}

pub struct Monitor {
    ffmpeg: Option<PathBuf>,
    fuente: Option<Fuente>,
    hijo: Option<Child>,
    compartido: Option<Arc<Compartido>>,
    textura: Option<egui::TextureHandle>,
    /// Último segundo mostrado.
    mostrado: f64,
    /// En pausa: segundo del cuadro pedido (si se salta a otro punto, se pide otro).
    pedido_en_pausa: f64,
    /// En pausa: la imagen queda quieta.
    en_pausa: bool,
}

impl Monitor {
    pub fn new(ffmpeg: Option<PathBuf>) -> Self {
        Monitor {
            ffmpeg,
            fuente: None,
            hijo: None,
            compartido: None,
            textura: None,
            mostrado: 0.0,
            pedido_en_pausa: -1.0,
            en_pausa: false,
        }
    }

    /// Cambia el ffmpeg (tras descargarlo o actualizarlo).
    pub fn usar_ffmpeg(&mut self, f: Option<PathBuf>) {
        if self.ffmpeg != f {
            self.parar();
            self.fuente = None;
            self.ffmpeg = f;
        }
    }

    pub fn hay_ffmpeg(&self) -> bool {
        self.ffmpeg.is_some()
    }

    /// Pone al día el monitor: `archivo` y `segundo` de lo que está al aire, o `None` si
    /// no hay nada (o no se puede ver desde esta PC). `pausado` congela la imagen.
    pub fn seguir(&mut self, ctx: &egui::Context, al_aire: Option<(&Path, f64)>, pausado: bool) {
        let Some((archivo, segundo)) = al_aire else {
            self.parar();
            self.fuente = None;
            self.textura = None;
            return;
        };
        if self.ffmpeg.is_none() {
            return;
        }
        // Al cambiar de spot se deja el último cuadro hasta que llegue el primero del
        // nuevo (así no parpadea en negro).
        let otro = self.fuente.as_ref().is_none_or(|f| f.archivo != archivo);
        if pausado {
            // Se queda el último cuadro (o, si no había, se saca uno de ese segundo).
            if !self.en_pausa || otro {
                self.en_pausa = true;
                self.parar();
                self.pedido_en_pausa = self.mostrado;
                if otro || self.textura.is_none() {
                    self.pedido_en_pausa = segundo;
                    self.arrancar(archivo, segundo, true);
                }
            } else if (segundo - self.pedido_en_pausa).abs() > 0.5 {
                // Se saltó a otro punto con el spot en pausa: el cuadro de ahí.
                self.pedido_en_pausa = segundo;
                self.arrancar(archivo, segundo, true);
            }
            self.tomar_cuadro(ctx);
            ctx.request_repaint_after(Duration::from_millis(200));
            return;
        }
        if self.en_pausa {
            self.en_pausa = false;
            self.parar();
        }
        let esperado = self.fuente.as_ref().map(|f| f.desde + f.inicio.elapsed().as_secs_f64());
        let desfase = esperado.map(|e| (e - segundo).abs()).unwrap_or(f64::MAX);
        if otro || self.hijo.is_none() || desfase > DESFASE_MAX {
            self.arrancar(archivo, segundo, false);
        }
        self.tomar_cuadro(ctx);
        ctx.request_repaint_after(Duration::from_millis(1000 / FPS as u64));
    }

    fn tomar_cuadro(&mut self, ctx: &egui::Context) {
        let Some(c) = &self.compartido else { return };
        if let Some((pixeles, seg)) = c.cuadro.lock().expect("cuadro").take() {
            self.mostrado = seg;
            let img = egui::ColorImage::from_rgba_unmultiplied([ANCHO, ALTO], &pixeles);
            match &mut self.textura {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.textura = Some(ctx.load_texture("monitor-programa", img, egui::TextureOptions::LINEAR)),
            }
        }
    }

    /// `un_cuadro`: solo la imagen de ese segundo (para una pausa).
    fn arrancar(&mut self, archivo: &Path, desde: f64, un_cuadro: bool) {
        self.parar();
        let Some(ffmpeg) = self.ffmpeg.clone() else { return };
        let filtro = format!(
            "scale={ANCHO}:{ALTO}:force_original_aspect_ratio=decrease:flags=fast_bilinear,pad={ANCHO}:{ALTO}:(ow-iw)/2:(oh-ih)/2,fps={FPS}"
        );
        let mut cmd = nucleo::biblioteca::comando(&ffmpeg, true);
        cmd.args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{:.2}", desde.max(0.0)), "-i"])
            .arg(archivo)
            .args(["-an", "-vf", &filtro]);
        if un_cuadro {
            cmd.args(["-frames:v", "1"]);
        }
        cmd.args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"]).stdout(Stdio::piped()).stderr(Stdio::null());
        let Ok(mut hijo) = cmd.spawn() else { return };
        let mut salida = hijo.stdout.take().expect("stdout");
        let c = Arc::new(Compartido { cuadro: Mutex::new(None), detener: AtomicBool::new(false) });
        let cc = c.clone();
        let inicio = Instant::now();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; ANCHO * ALTO * 4];
            let mut n: u64 = 0;
            while salida.read_exact(&mut buf).is_ok() {
                if cc.detener.load(Ordering::SeqCst) {
                    return;
                }
                let t = n as f64 / FPS;
                std::thread::sleep(Duration::from_secs_f64(t).saturating_sub(inicio.elapsed()));
                *cc.cuadro.lock().expect("cuadro") = Some((buf.clone(), desde + t));
                n += 1;
            }
        });
        self.hijo = Some(hijo);
        self.compartido = Some(c);
        self.fuente = Some(Fuente { archivo: archivo.to_path_buf(), desde, inicio });
    }

    fn parar(&mut self) {
        if let Some(c) = self.compartido.take() {
            c.detener.store(true, Ordering::SeqCst);
        }
        if let Some(mut h) = self.hijo.take() {
            let _ = h.kill();
            let _ = h.wait();
        }
    }

    pub fn textura(&self) -> Option<&egui::TextureHandle> {
        self.textura.as_ref()
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.parar();
    }
}
