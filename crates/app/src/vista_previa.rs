//! Vista previa de videos dentro de la ventana principal, con sonido.
//!
//! ffmpeg decodifica el video a cuadros pequeños y el audio a muestras; el audio
//! suena por la salida elegida (cpal) y marca el reloj al que avanzan los cuadros.
//! No pasa por CasparCG: no cambia lo que está al aire.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use egui::{Align, Color32, Layout, RichText, Sense, Vec2};

use crate::tema::*;

const ANCHO: usize = 640;
const ALTO: usize = 360;
const FPS: f64 = 25.0;

pub fn sin_ventana(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let _ = cmd;
}

/// Duración de cualquier video, preguntándole a ffmpeg (para formatos que no son MP4/MOV).
pub fn duracion_con_ffmpeg(ffmpeg: &Path, video: &Path) -> Option<Duration> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-i"]).arg(video).stdout(Stdio::null()).stderr(Stdio::piped());
    sin_ventana(&mut cmd);
    let salida = cmd.output().ok()?;
    let texto = String::from_utf8_lossy(&salida.stderr);
    let i = texto.find("Duration: ")? + "Duration: ".len();
    let hms = texto[i..].split(',').next()?.trim();
    let mut p = hms.split(':');
    let (h, m, s): (f64, f64, f64) = (p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?);
    Some(Duration::from_secs_f64(h * 3600.0 + m * 60.0 + s))
}

struct Cuadro {
    pixeles: Vec<u8>,
    segundo: f64,
}

/// Lo que comparten la interfaz, el hilo del video, el del audio y la tarjeta de sonido.
struct Compartido {
    cuadro: Mutex<Option<Cuadro>>,
    terminado: AtomicBool,
    detener: AtomicBool,
    error: Mutex<Option<String>>,
    /// Muestras de audio esperando a sonar (intercaladas por canal).
    cola: Mutex<VecDeque<f32>>,
    /// Cuadros de audio (muestras por canal) que ya sonaron: es el reloj de la reproducción.
    sonados: AtomicU64,
    /// El audio está sonando y manda en el reloj. Si no hay audio o se acabó, manda el reloj normal.
    audio_vivo: AtomicBool,
    /// Volumen (f32 en bits) aplicado en la tarjeta de sonido.
    volumen: AtomicU32,
}

impl Compartido {
    fn volumen(&self) -> f32 {
        f32::from_bits(self.volumen.load(Ordering::Relaxed))
    }
}

pub enum AccionVista {
    Nada,
    Cerrar,
    Agregar(nucleo::Destino),
}

/// Salida de sonido que se puede elegir.
#[derive(Clone)]
pub struct Salida {
    pub id: String,
    pub nombre: String,
}

/// Salidas de sonido de la PC. La primera es la predeterminada de Windows.
pub fn salidas_de_audio() -> Vec<Salida> {
    let host = cpal::default_host();
    let mut v = vec![Salida { id: String::new(), nombre: "Predeterminada de Windows".into() }];
    if let Ok(dispositivos) = host.output_devices() {
        for d in dispositivos {
            let (Ok(id), Ok(desc)) = (d.id(), d.description()) else { continue };
            v.push(Salida { id: id.to_string(), nombre: desc.name().to_string() });
        }
    }
    v
}

fn dispositivo(id: &str) -> Option<cpal::Device> {
    let host = cpal::default_host();
    if !id.is_empty() {
        if let Ok(i) = id.parse() {
            if let Some(d) = host.device_by_id(&i) {
                return Some(d);
            }
        }
        tracing::warn!("no se encontró la salida de audio «{id}»; se usa la predeterminada");
    }
    host.default_output_device()
}

/// Abre la tarjeta de sonido. Devuelve el flujo (debe seguir vivo mientras suena),
/// la frecuencia y los canales que pide.
fn abrir_audio(id: &str, c: Arc<Compartido>) -> Option<(cpal::Stream, u32, u16)> {
    let disp = dispositivo(id)?;
    let conf = disp.default_output_config().ok()?;
    let formato = conf.sample_format();
    let config: cpal::StreamConfig = conf.into();
    let (tasa, canales) = (config.sample_rate, config.channels);
    let flujo = match formato {
        cpal::SampleFormat::F32 => flujo::<f32>(&disp, config, c),
        cpal::SampleFormat::I16 => flujo::<i16>(&disp, config, c),
        cpal::SampleFormat::U16 => flujo::<u16>(&disp, config, c),
        cpal::SampleFormat::I32 => flujo::<i32>(&disp, config, c),
        cpal::SampleFormat::F64 => flujo::<f64>(&disp, config, c),
        otro => {
            tracing::warn!("formato de audio no soportado: {otro}");
            None
        }
    }?;
    flujo.play().ok()?;
    Some((flujo, tasa, canales))
}

fn flujo<T>(disp: &cpal::Device, config: cpal::StreamConfig, c: Arc<Compartido>) -> Option<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let canales = config.channels.max(1) as usize;
    disp.build_output_stream(
        config,
        move |datos: &mut [T], _: &cpal::OutputCallbackInfo| {
            let vol = c.volumen();
            let mut cola = c.cola.lock().expect("cola");
            let mut reales = 0usize;
            for m in datos.iter_mut() {
                match cola.pop_front() {
                    Some(v) => {
                        *m = T::from_sample((v * vol).clamp(-1.0, 1.0));
                        reales += 1;
                    }
                    None => *m = T::from_sample(0.0f32),
                }
            }
            c.sonados.fetch_add((reales / canales) as u64, Ordering::SeqCst);
        },
        |e| tracing::warn!("audio de la vista previa: {e}"),
        None,
    )
    .map_err(|e| tracing::warn!("no se pudo abrir el audio: {e}"))
    .ok()
}

pub struct VistaPrevia {
    ffmpeg: Option<PathBuf>,
    video: Option<PathBuf>,
    titulo: String,
    duracion: Option<Duration>,
    reproduciendo: bool,
    posicion: f64,
    hijos: Vec<Child>,
    compartido: Option<Arc<Compartido>>,
    audio: Option<cpal::Stream>,
    textura: Option<egui::TextureHandle>,
    arrastrando: Option<f64>,
    /// Con sonido.
    pub sonido: bool,
    pub volumen: f32,
    /// Id de la salida de sonido ("" = la predeterminada).
    pub salida: String,
    salidas: Option<Vec<Salida>>,
    /// Silenciada porque hay publicidad al aire (para que no se escuche en la emisión).
    silenciada: bool,
    /// Donde se recuerdan el sonido, el volumen y la salida elegidos.
    archivo_pref: Option<PathBuf>,
    /// Alto del video en la columna (lo decide `ajustar`).
    alto_video: f32,
}

/// Lo que se recuerda de la vista previa entre usos.
#[derive(serde::Serialize, serde::Deserialize)]
struct Preferencias {
    sonido: bool,
    volumen: f32,
    salida: String,
}

impl VistaPrevia {
    pub fn new(ffmpeg: Option<PathBuf>, cfg: &crate::config::VistaPreviaCfg, carpeta_local: &Path) -> Self {
        let archivo_pref = carpeta_local.join("vista_previa.json");
        let pref: Option<Preferencias> =
            std::fs::read_to_string(&archivo_pref).ok().and_then(|t| serde_json::from_str(&t).ok());
        let (sonido, volumen, salida) = match pref {
            Some(p) => (p.sonido, p.volumen.clamp(0.0, 1.0), p.salida),
            None => (cfg.sonido, 0.8, cfg.salida_audio.clone()),
        };
        VistaPrevia {
            ffmpeg,
            video: None,
            titulo: String::new(),
            duracion: None,
            reproduciendo: false,
            posicion: 0.0,
            hijos: Vec::new(),
            compartido: None,
            audio: None,
            textura: None,
            arrastrando: None,
            sonido,
            volumen,
            salida,
            salidas: None,
            silenciada: false,
            archivo_pref: Some(archivo_pref),
            alto_video: 200.0,
        }
    }

    pub fn ffmpeg(&self) -> Option<&Path> {
        self.ffmpeg.as_deref()
    }

    /// Cambia el ffmpeg (por ejemplo, tras descargarlo o actualizarlo).
    pub fn usar_ffmpeg(&mut self, f: Option<PathBuf>) {
        if self.ffmpeg != f {
            self.cerrar();
            self.ffmpeg = f;
        }
    }

    pub fn activa(&self) -> bool {
        self.video.is_some()
    }

    pub fn video(&self) -> Option<(&Path, Option<Duration>)> {
        self.video.as_deref().map(|v| (v, self.duracion))
    }

    /// Carga un video en el reproductor y empieza a reproducirlo.
    pub fn abrir(&mut self, video: PathBuf, titulo: String, duracion: Option<Duration>) {
        self.detener();
        self.duracion = duracion.or_else(|| self.ffmpeg.as_ref().and_then(|f| duracion_con_ffmpeg(f, &video)));
        self.video = Some(video);
        self.titulo = titulo;
        self.posicion = 0.0;
        self.textura = None;
        self.arrancar(0.0);
    }

    pub fn cerrar(&mut self) {
        self.detener();
        self.video = None;
        self.textura = None;
    }

    fn arrancar(&mut self, desde: f64) {
        self.detener();
        let (Some(ffmpeg), Some(video)) = (self.ffmpeg.clone(), self.video.clone()) else { return };
        let c = Arc::new(Compartido {
            cuadro: Mutex::new(None),
            terminado: AtomicBool::new(false),
            detener: AtomicBool::new(false),
            error: Mutex::new(None),
            cola: Mutex::new(VecDeque::new()),
            sonados: AtomicU64::new(0),
            audio_vivo: AtomicBool::new(false),
            volumen: AtomicU32::new(self.volumen_efectivo().to_bits()),
        });

        // Video.
        let filtro = format!(
            "scale={ANCHO}:{ALTO}:force_original_aspect_ratio=decrease:flags=fast_bilinear,pad={ANCHO}:{ALTO}:(ow-iw)/2:(oh-ih)/2,fps={FPS}"
        );
        let mut cmd = Command::new(&ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{desde:.2}"), "-i"])
            .arg(&video)
            .args(["-an", "-vf", &filtro, "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        sin_ventana(&mut cmd);
        let mut hijo = match cmd.spawn() {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("no se pudo iniciar ffmpeg: {e}");
                return;
            }
        };
        let salida_video = hijo.stdout.take().expect("stdout");
        let errores = hijo.stderr.take().expect("stderr");
        self.hijos.push(hijo);

        // Audio: si hay sonido y la tarjeta abre, ffmpeg entrega las muestras ya en
        // su frecuencia y canales.
        if self.sonido {
            if let Some((flujo, tasa, canales)) = abrir_audio(&self.salida, c.clone()) {
                let mut cmd = Command::new(&ffmpeg);
                cmd.args(["-hide_banner", "-loglevel", "error", "-ss", &format!("{desde:.2}"), "-i"])
                    .arg(&video)
                    .args(["-vn", "-ac", &canales.to_string(), "-ar", &tasa.to_string(), "-f", "f32le", "-"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .stdin(Stdio::null());
                sin_ventana(&mut cmd);
                if let Ok(mut h) = cmd.spawn() {
                    let salida_audio = h.stdout.take().expect("stdout");
                    self.hijos.push(h);
                    c.audio_vivo.store(true, Ordering::SeqCst);
                    let ca = c.clone();
                    std::thread::spawn(move || hilo_audio(salida_audio, ca, tasa, canales));
                    self.audio = Some(flujo);
                    let cv = c.clone();
                    std::thread::spawn(move || hilo_video(salida_video, errores, cv, desde, Some(tasa)));
                    self.compartido = Some(c);
                    self.reproduciendo = true;
                    return;
                }
            }
        }
        let cv = c.clone();
        std::thread::spawn(move || hilo_video(salida_video, errores, cv, desde, None));
        self.compartido = Some(c);
        self.reproduciendo = true;
    }

    fn detener(&mut self) {
        if let Some(c) = &self.compartido {
            c.detener.store(true, Ordering::SeqCst);
        }
        for mut h in self.hijos.drain(..) {
            let _ = h.kill();
            let _ = h.wait();
        }
        self.audio = None;
        self.compartido = None;
        self.reproduciendo = false;
    }

    fn guardar_pref(&self) {
        let Some(a) = &self.archivo_pref else { return };
        let p = Preferencias { sonido: self.sonido, volumen: self.volumen, salida: self.salida.clone() };
        if let Some(c) = a.parent() {
            let _ = std::fs::create_dir_all(c);
        }
        if let Ok(t) = serde_json::to_string_pretty(&p) {
            let _ = std::fs::write(a, t);
        }
    }

    fn volumen_efectivo(&self) -> f32 {
        if self.silenciada {
            0.0
        } else {
            self.volumen
        }
    }

    /// Pasa el último cuadro decodificado a la textura. Se llama en cada cuadro de la interfaz.
    fn actualizar(&mut self, ctx: &egui::Context) -> Option<String> {
        let c = self.compartido.clone()?;
        if let Some(cuadro) = c.cuadro.lock().expect("cuadro").take() {
            self.posicion = cuadro.segundo;
            let img = egui::ColorImage::from_rgba_unmultiplied([ANCHO, ALTO], &cuadro.pixeles);
            match &mut self.textura {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => self.textura = Some(ctx.load_texture("vista-previa", img, egui::TextureOptions::LINEAR)),
            }
        }
        c.volumen.store(self.volumen_efectivo().to_bits(), Ordering::Relaxed);
        if c.terminado.load(Ordering::SeqCst) {
            self.reproduciendo = false;
            // Que el audio que queda termine solo; los procesos ya acabaron.
        } else {
            ctx.request_repaint_after(Duration::from_millis(15));
        }
        let e = c.error.lock().expect("error").clone();
        e
    }

    /// Decide el tamaño del video para este ancho, si el reproductor puede usar hasta
    /// `disponible` de alto. Devuelve el alto total que ocupará.
    pub fn ajustar(&mut self, ancho: f32, disponible: f32) -> f32 {
        let (_, alto_video) = tam_video(ancho - 24.0, disponible - CONTROLES);
        self.alto_video = alto_video;
        alto_video + CONTROLES
    }

    /// Dibuja el reproductor dentro de la columna. `silenciar` trae el motivo si el
    /// sonido debe callarse (hay publicidad al aire).
    pub fn mostrar_en(&mut self, ui: &mut egui::Ui, silenciar: Option<&str>) -> AccionVista {
        let mut accion = AccionVista::Nada;
        self.silenciada = silenciar.is_some();
        let error = self.actualizar(ui.ctx());

        crate::tema::marco_tarjeta().stroke(egui::Stroke::new(1.0, ACENTO.gamma_multiply(0.6))).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            // Cabecera: qué es, el nombre y cerrar.
            ui.horizontal(|ui| {
                crate::tema::chip(ui, "▶ VISTA PREVIA", Color32::WHITE, ACENTO_OSCURO);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let cerrar = egui::Button::new(RichText::new("✖").size(13.0).color(TENUE))
                        .frame(false)
                        .min_size(Vec2::splat(24.0));
                    if ui.add(cerrar).on_hover_text("Cerrar la vista previa").clicked() {
                        accion = AccionVista::Cerrar;
                    }
                    ui.add(
                        egui::Label::new(
                            RichText::new(&self.titulo).size(13.5).strong().family(crate::tema::negrita()),
                        )
                        .truncate(),
                    );
                });
            });
            ui.add_space(2.0);

            // Imagen, a todo el ancho sobre negro.
            let (_, h) = tam_video(ui.available_width(), self.alto_video);
            let (fila, r_video) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
            let (w, h) = tam_video(fila.width(), h);
            let rect = egui::Rect::from_center_size(fila.center(), Vec2::new(w, h));
            let pintor = ui.painter();
            pintor.rect_filled(fila, 6.0, Color32::BLACK);
            let texto_centro = |t: &str, color| {
                pintor.text(rect.center(), egui::Align2::CENTER_CENTER, t, egui::FontId::proportional(13.0), color)
            };
            if self.ffmpeg.is_none() {
                texto_centro("No hay ffmpeg todavía.\nSe descarga solo (mira 🎞 ffmpeg en la barra de abajo).", TENUE);
            } else if let Some(e) = &error {
                texto_centro(&format!("No se pudo leer el video:\n{e}"), AIRE);
            } else if let Some(t) = &self.textura {
                egui::Image::new(t).fit_to_exact_size(Vec2::new(w, h)).paint_at(ui, rect);
            } else {
                texto_centro("Cargando…", TENUE);
            }
            // Pausado: ▶ grande encima; clic en el video = reproducir/pausar.
            let total = self.duracion.map(|d| d.as_secs_f64()).unwrap_or(0.0);
            let alternar = |yo: &mut Self| {
                if yo.reproduciendo {
                    yo.detener();
                } else {
                    let desde = if total > 0.0 && yo.posicion >= total - 0.2 { 0.0 } else { yo.posicion };
                    yo.arrancar(desde);
                }
            };
            if !self.reproduciendo && self.textura.is_some() && error.is_none() {
                pintor.circle_filled(rect.center(), 24.0, Color32::from_black_alpha(170));
                pintor.text(
                    rect.center() + Vec2::new(2.0, 0.0),
                    egui::Align2::CENTER_CENTER,
                    "⏵",
                    egui::FontId::proportional(24.0),
                    Color32::WHITE,
                );
            }
            if let Some(motivo) = silenciar {
                if self.sonido {
                    let r = egui::Rect::from_min_size(
                        fila.left_top() + Vec2::new(0.0, h - 22.0),
                        Vec2::new(fila.width(), 22.0),
                    );
                    pintor.rect_filled(r, 4.0, Color32::from_black_alpha(170));
                    pintor.text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("🔇 {motivo}"),
                        egui::FontId::proportional(11.5),
                        AMBAR,
                    );
                }
            }
            if r_video.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && self.ffmpeg.is_some() {
                alternar(self);
            }

            // Barra de tiempo.
            ui.add_space(2.0);
            if total > 0.0 {
                let pos = self.arrastrando.unwrap_or(self.posicion).min(total);
                let (nuevo, soltado) = crate::tema::barra_tiempo(ui, pos, total);
                if let Some(s) = nuevo {
                    if soltado {
                        self.arrastrando = None;
                        self.posicion = s;
                        self.arrancar(s);
                    } else {
                        self.arrastrando = Some(s);
                    }
                }
            }

            // Controles.
            ui.horizontal(|ui| {
                let salto = |ui: &mut egui::Ui, t: &str, ayuda: &str| {
                    ui.add(
                        egui::Button::new(RichText::new(t).size(12.5).color(TEXTO))
                            .frame(false)
                            .min_size(Vec2::new(30.0, 30.0)),
                    )
                    .on_hover_text(ayuda)
                    .clicked()
                };
                if salto(ui, "⏮", "Desde el inicio") {
                    self.posicion = 0.0;
                    self.arrancar(0.0);
                }
                if salto(ui, "⏪ 5", "5 segundos atrás") {
                    let p = (self.posicion - 5.0).max(0.0);
                    self.posicion = p;
                    self.arrancar(p);
                }
                let icono = if self.reproduciendo { "⏸" } else { "⏵" };
                if crate::tema::boton_redondo(ui, icono, 34.0).on_hover_text("Reproducir / pausar").clicked() {
                    alternar(self);
                }
                if salto(ui, "5 ⏩", "5 segundos adelante") {
                    let p =
                        if total > 0.0 { (self.posicion + 5.0).min(total - 0.1).max(0.0) } else { self.posicion + 5.0 };
                    self.posicion = p;
                    self.arrancar(p);
                }
                ui.add_space(4.0);
                let tiempo = format!(
                    "{} / {}",
                    mmss(self.arrastrando.unwrap_or(self.posicion)),
                    if total > 0.0 { mmss(total) } else { "--:--".into() }
                );
                ui.label(RichText::new(tiempo).monospace().size(12.5).color(TEXTO));

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Salida de sonido.
                    let r = ui
                        .add(
                            egui::Button::new(RichText::new("⚙").color(TENUE)).frame(false).min_size(Vec2::splat(26.0)),
                        )
                        .on_hover_text("Salida de sonido");
                    egui::Popup::menu(&r).show(|ui| {
                        ui.label(RichText::new("Salida de sonido de la vista previa").size(12.0).color(TENUE));
                        let salidas = self.salidas.get_or_insert_with(salidas_de_audio).clone();
                        let mut cambio = false;
                        for s in &salidas {
                            if ui.radio(self.salida == s.id, &s.nombre).clicked() {
                                self.salida = s.id.clone();
                                cambio = true;
                            }
                        }
                        ui.separator();
                        ui.label(
                            RichText::new("Usa una salida que NO vaya a la mezcladora\n(por ejemplo, los audífonos).")
                                .size(11.0)
                                .color(MUY_TENUE),
                        );
                        if cambio {
                            self.guardar_pref();
                            if self.reproduciendo {
                                let p = self.posicion;
                                self.arrancar(p);
                            }
                        }
                    });
                    if self.sonido {
                        ui.spacing_mut().slider_width = 64.0;
                        ui.visuals_mut().widgets.inactive.bg_fill = BORDE;
                        let r = ui.add(egui::Slider::new(&mut self.volumen, 0.0..=1.0).show_value(false));
                        if r.drag_stopped() || (r.changed() && !r.dragged()) {
                            self.guardar_pref();
                        }
                    }
                    let icono = if !self.sonido { "🔇" } else { "🔊" };
                    let ayuda = if self.sonido { "Quitar el sonido" } else { "Escuchar el sonido" };
                    if ui
                        .add(
                            egui::Button::new(RichText::new(icono).size(14.0)).frame(false).min_size(Vec2::splat(26.0)),
                        )
                        .on_hover_text(ayuda)
                        .clicked()
                    {
                        self.sonido = !self.sonido;
                        self.guardar_pref();
                        if self.reproduciendo {
                            let p = self.posicion;
                            self.arrancar(p);
                        }
                    }
                });
            });

            // Usar el video.
            ui.add_space(4.0);
            if let Some(destino) = crate::tema::acciones_tanda(ui) {
                accion = AccionVista::Agregar(destino);
            }
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("La vista previa no sale al aire").size(10.5).color(MUY_TENUE));
            });
        });
        if matches!(accion, AccionVista::Cerrar) {
            self.cerrar();
        }
        accion
    }
}

/// Alto del reproductor sin contar el video (título, barra, botones y márgenes).
const CONTROLES: f32 = 170.0;

/// Tamaño del video para un ancho dado (16:9, entre 90 y 270 px de alto).
fn tam_video(ancho: f32, alto_max: f32) -> (f32, f32) {
    let alto = (ancho * 9.0 / 16.0).min(alto_max.clamp(90.0, 270.0));
    (alto * 16.0 / 9.0, alto)
}

/// Lee los cuadros de ffmpeg y los entrega al ritmo del reloj (el del audio si suena).
fn hilo_video(
    mut salida: impl Read,
    mut errores: impl Read + Send + 'static,
    c: Arc<Compartido>,
    desde: f64,
    tasa_audio: Option<u32>,
) {
    let mut buf = vec![0u8; ANCHO * ALTO * 4];
    let mut n: u64 = 0;
    // Reloj normal, para cuando no hay audio o se acabó: arranca donde quedó el del audio.
    let inicio = Instant::now();
    let mut base = (Instant::now(), 0.0f64);
    let reloj = |base: &mut (Instant, f64)| -> f64 {
        if let Some(tasa) = tasa_audio {
            if c.audio_vivo.load(Ordering::SeqCst) {
                // El audio nunca va por delante del tiempo real (algunas salidas
                // "consumen" más rápido de lo que suenan).
                let t = (c.sonados.load(Ordering::SeqCst) as f64 / tasa as f64).min(inicio.elapsed().as_secs_f64());
                *base = (Instant::now(), t);
                return t;
            }
        }
        base.1 + base.0.elapsed().as_secs_f64()
    };
    while salida.read_exact(&mut buf).is_ok() {
        if c.detener.load(Ordering::SeqCst) {
            return;
        }
        let t = n as f64 / FPS;
        loop {
            let ahora = reloj(&mut base);
            if ahora >= t || c.detener.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_secs_f64((t - ahora).min(0.01)));
        }
        *c.cuadro.lock().expect("cuadro") = Some(Cuadro { pixeles: buf.clone(), segundo: desde + t });
        n += 1;
    }
    let mut e = String::new();
    let _ = errores.read_to_string(&mut e);
    if !e.trim().is_empty() && n == 0 {
        *c.error.lock().expect("error") = Some(e.lines().last().unwrap_or_default().to_string());
    }
    c.terminado.store(true, Ordering::SeqCst);
}

/// Lee las muestras de audio de ffmpeg y las deja en la cola de la tarjeta de sonido,
/// con medio segundo de reserva como mucho.
fn hilo_audio(mut salida: impl Read, c: Arc<Compartido>, tasa: u32, canales: u16) {
    let maximo = (tasa as usize * canales as usize) / 2;
    let mut buf = vec![0u8; 4096 * 4];
    let mut alguna = false;
    loop {
        if c.detener.load(Ordering::SeqCst) {
            return;
        }
        // Bloques enteros de muestras (read_exact no corta una muestra a la mitad).
        if salida.read_exact(&mut buf).is_err() {
            break;
        }
        let n = buf.len();
        alguna = true;
        let muestras = buf[..n].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        c.cola.lock().expect("cola").extend(muestras);
        while c.cola.lock().expect("cola").len() > maximo {
            if c.detener.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    // Se espera a que suene lo que quedó; luego manda el reloj normal.
    while alguna && !c.cola.lock().expect("cola").is_empty() && !c.detener.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(10));
    }
    c.audio_vivo.store(false, Ordering::SeqCst);
}

impl Drop for VistaPrevia {
    fn drop(&mut self) {
        self.detener();
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn duracion_desde_la_salida_de_ffmpeg() {
        // Solo se prueba si hay ffmpeg en esta PC.
        let Some(f) = crate::ffmpeg::buscar("", &[]) else { return };
        let d = std::env::temp_dir().join(format!("relevo-vp-{}.mp4", std::process::id()));
        let ok = Command::new(&f)
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=160x90:rate=25:duration=3"])
            .arg(&d)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return;
        }
        let dur = duracion_con_ffmpeg(&f, &d).unwrap();
        assert!((dur.as_secs_f64() - 3.0).abs() < 0.1);
    }
}
