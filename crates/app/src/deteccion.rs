//! Detección del corte del canal.
//!
//! Se mira una señal (capturadora, dirección de video o la pantalla) con ffmpeg y se
//! vigila la zona donde el canal pone su logo. Mientras hay programa, el logo está;
//! cuando el canal va a comerciales, desaparece. Primero se "aprende" el logo (unos
//! segundos con el programa al aire) y después cada cuadro se compara con él.
//!
//! Se comparan los *bordes* de la imagen, no los colores: el logo suele ser
//! semitransparente y lo de atrás cambia, pero sus bordes quedan siempre en el mismo sitio.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::DeteccionCfg;
use crate::senal::{self, EstadoSenal, Fuente, Lector};

/// Tamaño al que se reduce la señal para analizarla (la mitad de la que se lee).
pub const ANCHO: usize = senal::ANCHO / 2;
pub const ALTO: usize = senal::ALTO / 2;
const CUADROS_POR_SEG: f64 = 5.0;
/// Cuadros que se promedian al aprender el logo (8 segundos).
const CUADROS_APRENDER: usize = 40;
/// Segundos seguidos con logo para dar por terminado el corte del canal.
const SEG_VUELTA: f64 = 1.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evento {
    /// El canal fue a comerciales.
    Corte,
    /// El canal volvió con su programa.
    Programa,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Estado {
    Conectando,
    /// Hay señal pero falta aprender el logo.
    SinAprender,
    Programa,
    Corte,
    Error(String),
}

/// Lo que ve la interfaz.
#[derive(Clone)]
pub struct Vista {
    /// Último cuadro, en color (`senal::ANCHO` × `senal::ALTO`, RGB).
    pub cuadro: Option<Arc<Vec<u8>>>,
    /// Cambia con cada cuadro nuevo.
    pub serie: u64,
    /// Parecido con el logo aprendido (0–1).
    pub puntaje: f32,
    pub estado: Estado,
    /// Avance del aprendizaje (0–1), si se está aprendiendo.
    pub aprendiendo: Option<f32>,
    /// Parecido de los últimos segundos (5 por segundo), para el gráfico.
    pub historia: Vec<f32>,
    /// El logo aprendido, como imagen en grises (ancho, alto, puntos).
    pub logo: Option<Arc<(usize, usize, Vec<u8>)>>,
    /// Dirección del video encontrada en la página web (fuente "web").
    pub flujo: Option<String>,
}

/// Muestras que se guardan para el gráfico (90 segundos).
const HISTORIA: usize = 450;

/// Logo aprendido (mapa de bordes promedio de la zona).
#[derive(Clone, Serialize, Deserialize)]
struct Referencia {
    zona: [f64; 4],
    ancho: usize,
    alto: usize,
    valores: Vec<f32>,
}

struct Interno {
    vista: Vista,
    evento: Option<Evento>,
    pedir_aprender: bool,
}

pub struct Detector {
    interno: Arc<Mutex<Interno>>,
    parar: Arc<AtomicBool>,
    /// Ajustes en uso; la zona, el umbral y los tiempos se pueden cambiar en marcha.
    ajustes: Arc<Mutex<DeteccionCfg>>,
    pub cfg: DeteccionCfg,
    pub fuente: Fuente,
}

impl Detector {
    /// Mira la señal de `fuente` (compartida con el preview si es la misma).
    pub fn iniciar(
        cfg: DeteccionCfg,
        fuente: Fuente,
        ffmpeg: PathBuf,
        carpeta_datos: &Path,
        ctx: egui::Context,
    ) -> Self {
        let interno = Arc::new(Mutex::new(Interno {
            vista: Vista {
                cuadro: None,
                serie: 0,
                puntaje: 0.0,
                estado: Estado::Conectando,
                aprendiendo: None,
                historia: Vec::new(),
                logo: None,
                flujo: None,
            },
            evento: None,
            pedir_aprender: false,
        }));
        let parar = Arc::new(AtomicBool::new(false));
        let ajustes = Arc::new(Mutex::new(cfg.clone()));
        let (i, p, c) = (interno.clone(), parar.clone(), ajustes.clone());
        let archivo_ref = carpeta_datos.join("logo_corte.json");
        let lector = senal::lector(&fuente, ffmpeg);
        std::thread::Builder::new()
            .name("deteccion".into())
            .spawn(move || trabajar(c, lector, archivo_ref, i, p, ctx))
            .expect("hilo de detección");
        Detector { interno, parar, ajustes, cfg, fuente }
    }

    /// Aplica ajustes nuevos sin cortar la señal. Devuelve `false` si cambió la fuente
    /// (hay que crear otro detector).
    pub fn actualizar(&mut self, cfg: &DeteccionCfg, fuente: &Fuente) -> bool {
        if self.fuente != *fuente {
            return false;
        }
        if self.cfg != *cfg {
            self.cfg = cfg.clone();
            *self.ajustes.lock().expect("ajustes") = cfg.clone();
        }
        true
    }

    pub fn vista(&self) -> Vista {
        self.interno.lock().expect("detección").vista.clone()
    }

    /// Aprende el logo con lo que se ve ahora (debe estar el programa al aire).
    pub fn aprender(&self) {
        self.interno.lock().expect("detección").pedir_aprender = true;
    }

    pub fn tomar_evento(&self) -> Option<Evento> {
        self.interno.lock().expect("detección").evento.take()
    }
}

impl Drop for Detector {
    fn drop(&mut self) {
        // El hilo suelta su lector; si nadie más lo usa, se cierra su ffmpeg.
        self.parar.store(true, Ordering::SeqCst);
    }
}

/// Argumentos de entrada de ffmpeg según la fuente.
#[cfg(test)]
pub fn entrada(cfg: &DeteccionCfg) -> Result<Vec<String>, String> {
    let f = Fuente { tipo: cfg.fuente.clone(), origen: cfg.origen.trim().to_string(), monitor: cfg.monitor };
    entrada_con(&f, None, None)
}

/// `flujo`: dirección del video ya encontrada (fuente "web"). `monitor`: posición y
/// tamaño del monitor elegido (fuente "pantalla"); `None` = todo el escritorio.
pub fn entrada_con(
    fuente: &Fuente,
    flujo: Option<&str>,
    monitor: Option<(i32, i32, i32, i32)>,
) -> Result<Vec<String>, String> {
    let origen = fuente.origen.trim();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match fuente.tipo.as_str() {
        "web" => {
            let flujo = flujo.ok_or("Falta buscar el video de la página")?;
            if !flujo.starts_with("http://") && !flujo.starts_with("https://") {
                // udp://, rtp://, rtmp://…: sin cabeceras de navegador.
                return Ok(s(&["-i", flujo]));
            }
            // Muchos sitios solo entregan el video si se dice de qué página viene.
            let cabecera = format!("Referer: {origen}\r\n");
            Ok(s(&["-user_agent", NAVEGADOR, "-headers", &cabecera, "-i", flujo]))
        }
        "pantalla" if monitor.is_some() => {
            let (x, y, w, h) = monitor.expect("comprobado");
            let tam = format!("{w}x{h}");
            if cfg!(windows) {
                Ok(s(&[
                    "-f",
                    "gdigrab",
                    "-framerate",
                    "10",
                    "-offset_x",
                    &x.to_string(),
                    "-offset_y",
                    &y.to_string(),
                    "-video_size",
                    &tam,
                    "-i",
                    "desktop",
                ]))
            } else {
                let d = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
                Ok(s(&["-f", "x11grab", "-framerate", "10", "-video_size", &tam, "-i", &format!("{d}+{x},{y}")]))
            }
        }
        "capturadora" => {
            if origen.is_empty() {
                return Err("Elige la capturadora en la lista".into());
            }
            Ok(s(&["-f", "dshow", "-rtbufsize", "64M", "-i", &format!("video={origen}")]))
        }
        "url" => {
            if origen.is_empty() {
                return Err("Falta la dirección o el archivo".into());
            }
            Ok(s(&["-i", origen]))
        }
        "pantalla" => {
            if cfg!(windows) {
                Ok(s(&["-f", "gdigrab", "-framerate", "10", "-i", "desktop"]))
            } else {
                let d = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
                Ok(s(&["-f", "x11grab", "-framerate", "10", "-i", &d]))
            }
        }
        "archivo" => Ok(s(&["-re", "-stream_loop", "-1", "-i", origen])),
        otra => Err(format!("Fuente desconocida: {otra}")),
    }
}

const NAVEGADOR: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

/// ¿Es ya la dirección de un video (y no la de una página)?
pub fn es_flujo(url: &str) -> bool {
    let u = url.trim().to_lowercase();
    let sin_consulta = u.split(['?', '#']).next().unwrap_or("");
    ["rtmp://", "rtmps://", "rtsp://", "udp://", "srt://", "rtp://"].iter().any(|p| u.starts_with(p))
        || [".m3u8", ".mpd", ".mp4", ".ts", ".flv", ".mkv", ".webm"].iter().any(|e| sin_consulta.ends_with(e))
}

/// Direcciones de video que aparecen en el texto de una página (primero las .m3u8).
pub fn flujos_en(html: &str) -> Vec<String> {
    // Las páginas suelen escribirlas "escapadas": https:\/\/sitio\/video.m3u8
    let texto = html.replace("\\/", "/").replace("\\u0026", "&").replace("&amp;", "&");
    let fin = |c: char| c == '"' || c == '\'' || c == '<' || c == '>' || c == '\\' || c.is_whitespace();
    let mut v: Vec<String> = Vec::new();
    for ext in [".m3u8", ".mpd", ".mp4"] {
        let mut desde = 0;
        while let Some(i) = texto[desde..].find(ext) {
            let pos = desde + i;
            desde = pos + ext.len();
            let inicio = texto[..pos].rfind(fin).map(|k| k + 1).unwrap_or(0);
            let Some(http) = texto[inicio..pos].rfind("http") else { continue };
            let final_ = texto[pos..].find(fin).map(|k| pos + k).unwrap_or(texto.len());
            let url = texto[inicio + http..final_].trim_end_matches([',', ';', ')']).to_string();
            if (url.starts_with("http://") || url.starts_with("https://")) && !v.contains(&url) {
                v.push(url);
            }
        }
    }
    v
}

/// Páginas incrustadas (`<iframe src="…">`), donde suele estar el reproductor.
fn marcos_en(html: &str, base: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut desde = 0;
    while let Some(i) = html[desde..].find("<iframe") {
        let pos = desde + i;
        desde = pos + 7;
        let fin = html[pos..].find('>').map(|k| pos + k).unwrap_or(html.len());
        let etiqueta = &html[pos..fin];
        let Some(s) = etiqueta.find("src=") else { continue };
        let resto = etiqueta[s + 4..].trim_start_matches(['"', '\'']);
        let url = resto.split(['"', '\'', ' ']).next().unwrap_or("");
        let url = if url.starts_with("//") {
            format!("https:{url}")
        } else if url.starts_with('/') {
            let raiz = base.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
            format!("{raiz}{url}")
        } else {
            url.to_string()
        };
        if url.starts_with("http") {
            v.push(url);
        }
    }
    v
}

fn bajar_pagina(url: &str, de: &str) -> Result<String, String> {
    let agente: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(Duration::from_secs(15)))
        .user_agent(NAVEGADOR)
        .build()
        .into();
    let mut r = agente.get(url).header("Referer", de).call().map_err(|e| format!("no se pudo abrir {url}: {e}"))?;
    r.body_mut().with_config().limit(4 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())
}

/// Busca la dirección del video de una página web: si ya es un video, la devuelve;
/// si no, prueba con yt-dlp (si está instalado) y después lee la página y sus
/// reproductores incrustados buscando un .m3u8, .mpd o .mp4.
pub fn buscar_flujo(pagina: &str) -> Result<String, String> {
    let pagina = pagina.trim();
    if pagina.is_empty() {
        return Err("Escribe la dirección de la página".into());
    }
    if es_flujo(pagina) {
        return Ok(pagina.to_string());
    }
    // yt-dlp conoce cientos de sitios (YouTube, Facebook, Twitch…).
    let mut cmd = Command::new("yt-dlp");
    cmd.args(["-g", "--no-warnings", "-f", "best[height<=480]/best", pagina]).stdin(Stdio::null());
    crate::vista_previa::sin_ventana(&mut cmd);
    if let Ok(o) = cmd.output() {
        if let Some(l) = String::from_utf8_lossy(&o.stdout).lines().find(|l| l.starts_with("http")) {
            return Ok(l.trim().to_string());
        }
    }
    let html = bajar_pagina(pagina, pagina)?;
    if let Some(u) = flujos_en(&html).into_iter().next() {
        return Ok(u);
    }
    for marco in marcos_en(&html, pagina).into_iter().take(4) {
        if let Ok(h) = bajar_pagina(&marco, pagina) {
            if let Some(u) = flujos_en(&h).into_iter().next() {
                return Ok(u);
            }
        }
    }
    Err("No encontré el video dentro de la página. Prueba con «Pantalla» (la página abierta en un monitor) \
         o pega la dirección del video (.m3u8)."
        .into())
}

/// Capturadoras de video de la PC (Windows, DirectShow).
pub fn capturadoras(ffmpeg: &Path) -> Vec<String> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"]).stdin(Stdio::null());
    crate::vista_previa::sin_ventana(&mut cmd);
    match cmd.output() {
        Ok(o) => leer_capturadoras(&String::from_utf8_lossy(&o.stderr)),
        Err(_) => Vec::new(),
    }
}

/// Líneas como `[dshow @ 000…] "USB Video" (video)`.
pub fn leer_capturadoras(texto: &str) -> Vec<String> {
    texto
        .lines()
        .filter(|l| l.contains("(video)"))
        .filter_map(|l| {
            let a = l.find('"')?;
            let b = l[a + 1..].find('"')?;
            Some(l[a + 1..a + 1 + b].to_string())
        })
        .collect()
}

/// Zona del logo en píxeles del cuadro reducido: (x, y, ancho, alto).
pub fn zona_px(cfg: &DeteccionCfg) -> (usize, usize, usize, usize) {
    let w = ((cfg.ancho.clamp(0.03, 1.0)) * ANCHO as f64).round().max(8.0) as usize;
    let h = ((cfg.alto.clamp(0.03, 1.0)) * ALTO as f64).round().max(8.0) as usize;
    let w = w.min(ANCHO);
    let h = h.min(ALTO);
    let x = ((cfg.x.clamp(0.0, 1.0)) * ANCHO as f64).round() as usize;
    let y = ((cfg.y.clamp(0.0, 1.0)) * ALTO as f64).round() as usize;
    (x.min(ANCHO - w), y.min(ALTO - h), w, h)
}

/// Mapa de bordes de una zona del cuadro (cuánto cambia el brillo alrededor de cada punto).
pub fn bordes(cuadro: &[u8], zona: (usize, usize, usize, usize)) -> Vec<f32> {
    let (x0, y0, w, h) = zona;
    let px = |x: usize, y: usize| cuadro[(y0 + y) * ANCHO + x0 + x] as f32;
    let mut v = vec![0.0f32; w * h];
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            v[y * w + x] = (px(x + 1, y) - px(x - 1, y)).abs() + (px(x, y + 1) - px(x, y - 1)).abs();
        }
    }
    v
}

/// Parecido entre dos mapas (correlación de Pearson, de −1 a 1).
pub fn parecido(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let n = a.len() as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let (mut sab, mut saa, mut sbb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        let (dx, dy) = (x - ma, y - mb);
        sab += dx * dy;
        saa += dx * dx;
        sbb += dy * dy;
    }
    if saa <= 1e-6 || sbb <= 1e-6 {
        return 0.0;
    }
    sab / (saa.sqrt() * sbb.sqrt())
}

/// Decide cuándo empezó o terminó el corte, a partir del parecido cuadro a cuadro.
pub struct Seguimiento {
    pub en_programa: bool,
    sin_logo: f64,
    con_logo: f64,
}

impl Default for Seguimiento {
    fn default() -> Self {
        Seguimiento { en_programa: true, sin_logo: 0.0, con_logo: 0.0 }
    }
}

impl Seguimiento {
    pub fn paso(&mut self, puntaje: f32, dt: f64, umbral: f64, segundos: f64) -> Option<Evento> {
        if (puntaje as f64) < umbral {
            self.sin_logo += dt;
            self.con_logo = 0.0;
        } else {
            self.con_logo += dt;
            self.sin_logo = 0.0;
        }
        if self.en_programa && self.sin_logo + 1e-6 >= segundos.max(0.2) {
            self.en_programa = false;
            return Some(Evento::Corte);
        }
        if !self.en_programa && self.con_logo + 1e-6 >= SEG_VUELTA {
            self.en_programa = true;
            return Some(Evento::Programa);
        }
        None
    }
}

/// El logo aprendido como imagen (para mostrarlo).
fn imagen_de(r: &Referencia) -> Arc<(usize, usize, Vec<u8>)> {
    let max = r.valores.iter().cloned().fold(1.0f32, f32::max);
    Arc::new((r.ancho, r.alto, r.valores.iter().map(|v| (v / max * 255.0) as u8).collect()))
}

fn trabajar(
    ajustes: Arc<Mutex<DeteccionCfg>>,
    lector: Arc<Lector>,
    archivo_ref: PathBuf,
    interno: Arc<Mutex<Interno>>,
    parar: Arc<AtomicBool>,
    ctx: egui::Context,
) {
    let cfg = ajustes.lock().expect("ajustes").clone();
    let zona_de = |c: &DeteccionCfg| [c.x, c.y, c.ancho, c.alto];
    let misma_zona = |a: &[f64; 4], b: &[f64; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    let mut zona_cfg = zona_de(&cfg);
    let mut zona = zona_px(&cfg);
    // El logo guardado solo vale si la zona es la misma.
    let mut referencia: Option<Referencia> = std::fs::read_to_string(&archivo_ref)
        .ok()
        .and_then(|t| serde_json::from_str::<Referencia>(&t).ok())
        .filter(|r| misma_zona(&r.zona, &zona_cfg) && r.valores.len() == zona.2 * zona.3);
    {
        let mut i = interno.lock().expect("detección");
        i.vista.logo = referencia.as_ref().map(imagen_de);
    }
    ctx.request_repaint();

    // Para afinar: RELEVO_DET_LOG=1 escribe el parecido de cada segundo.
    let registrar = std::env::var_os("RELEVO_DET_LOG").is_some();
    let rx = lector.suscribir();
    let mut seguimiento = Seguimiento::default();
    let mut suave: Option<f32> = None;
    let mut aprendiendo: Option<(usize, Vec<f32>)> = None;
    let mut cuadros = 0u64;
    // La señal llega a 10 cuadros por segundo; se analiza uno de cada dos (5 por segundo).
    let saltar = (senal::FPS / CUADROS_POR_SEG).round().max(1.0) as u64;
    let mut leidos = 0u64;
    while !parar.load(Ordering::SeqCst) {
        let (serie, rgb) = match rx.recv_timeout(Duration::from_millis(400)) {
            Ok(c) => c,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Sin cuadros: el estado de la señal (conectando o el error).
                let estado = match lector.estado() {
                    EstadoSenal::Error(e) => Some(Estado::Error(e)),
                    EstadoSenal::Conectando => Some(Estado::Conectando),
                    EstadoSenal::Activa => None,
                };
                let flujo = lector.flujo();
                let mut i = interno.lock().expect("detección");
                if let Some(e) = estado {
                    if i.vista.estado != e {
                        i.vista.estado = e;
                        ctx.request_repaint();
                    }
                }
                i.vista.flujo = flujo;
                continue;
            }
            Err(_) => return,
        };
        leidos += 1;
        if !leidos.is_multiple_of(saltar) {
            continue;
        }
        cuadros += 1;
        let buf = senal::a_grises_mitad(&rgb);
        // Ajustes en marcha: si se movió la zona, el logo aprendido ya no vale.
        let ahora_cfg = ajustes.lock().expect("ajustes").clone();
        let mut logo_cambio = false;
        if !misma_zona(&zona_de(&ahora_cfg), &zona_cfg) {
            zona_cfg = zona_de(&ahora_cfg);
            zona = zona_px(&ahora_cfg);
            referencia = None;
            aprendiendo = None;
            suave = None;
            logo_cambio = true;
        }
        let mapa = bordes(&buf, zona);
        if std::mem::take(&mut interno.lock().expect("detección").pedir_aprender) {
            aprendiendo = Some((0, vec![0.0; mapa.len()]));
        }
        let mut evento = None;
        let mut avance = None;
        if let Some((n, suma)) = &mut aprendiendo {
            for (s, v) in suma.iter_mut().zip(&mapa) {
                *s += v;
            }
            *n += 1;
            avance = Some(*n as f32 / CUADROS_APRENDER as f32);
            if *n >= CUADROS_APRENDER {
                let valores: Vec<f32> = suma.iter().map(|s| s / *n as f32).collect();
                let r = Referencia { zona: zona_cfg, ancho: zona.2, alto: zona.3, valores };
                if let Ok(t) = serde_json::to_string(&r) {
                    if let Some(c) = archivo_ref.parent() {
                        let _ = std::fs::create_dir_all(c);
                    }
                    let _ = std::fs::write(&archivo_ref, t);
                }
                referencia = Some(r);
                aprendiendo = None;
                avance = None;
                seguimiento = Seguimiento::default();
                suave = None;
                logo_cambio = true;
            }
        }
        let (puntaje, estado) = match (&referencia, aprendiendo.is_some()) {
            (Some(r), false) => {
                let p = parecido(&mapa, &r.valores).max(0.0);
                // Se suaviza un poco para que un cuadro raro no dispare nada.
                let s = suave.map(|a| a * 0.6 + p * 0.4).unwrap_or(p);
                suave = Some(s);
                evento = seguimiento.paso(s, 1.0 / CUADROS_POR_SEG, ahora_cfg.umbral, ahora_cfg.segundos);
                (s, if seguimiento.en_programa { Estado::Programa } else { Estado::Corte })
            }
            _ => (0.0, Estado::SinAprender),
        };
        if registrar && cuadros.is_multiple_of(5) {
            eprintln!("DET t={:.0}s parecido={puntaje:.2} {estado:?}", cuadros as f64 / CUADROS_POR_SEG);
        }
        let flujo = lector.flujo();
        {
            let mut i = interno.lock().expect("detección");
            i.vista.cuadro = Some(rgb);
            i.vista.serie = serie;
            i.vista.puntaje = puntaje;
            i.vista.flujo = flujo;
            if estado != Estado::SinAprender {
                if i.vista.historia.len() >= HISTORIA {
                    i.vista.historia.remove(0);
                }
                i.vista.historia.push(puntaje);
            }
            i.vista.estado = estado;
            i.vista.aprendiendo = avance;
            if logo_cambio {
                i.vista.logo = referencia.as_ref().map(imagen_de);
                i.vista.historia.clear();
            }
            if evento.is_some() {
                i.evento = evento;
            }
        }
        if evento.is_some() {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Cuadro con ruido de fondo y, si `logo`, un recuadro fijo dentro de la zona.
    fn cuadro(semilla: u32, logo: bool) -> Vec<u8> {
        let mut v = vec![0u8; ANCHO * ALTO];
        let mut s = semilla.wrapping_mul(2654435761).wrapping_add(1);
        for p in v.iter_mut() {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            *p = 90 + ((s >> 24) % 50) as u8;
        }
        if logo {
            for y in 20..34 {
                for x in 270..300 {
                    let borde = y == 20 || y == 33 || x == 270 || x == 299 || (x + y) % 7 == 0;
                    if borde {
                        v[y * ANCHO + x] = 245;
                    }
                }
            }
        }
        v
    }

    #[test]
    fn distingue_programa_de_corte() {
        let cfg = DeteccionCfg::default();
        let zona = zona_px(&cfg);
        // Aprender con 20 cuadros distintos que llevan el logo.
        let mut suma = vec![0.0f32; zona.2 * zona.3];
        for k in 0..20 {
            for (s, v) in suma.iter_mut().zip(bordes(&cuadro(k, true), zona)) {
                *s += v;
            }
        }
        let referencia: Vec<f32> = suma.iter().map(|s| s / 20.0).collect();
        let con = parecido(&bordes(&cuadro(99, true), zona), &referencia);
        let sin = parecido(&bordes(&cuadro(98, false), zona), &referencia);
        assert!(con > 0.5, "con logo: {con}");
        assert!(sin < 0.2, "sin logo: {sin}");
    }

    #[test]
    fn el_corte_necesita_varios_segundos_seguidos() {
        let mut s = Seguimiento::default();
        // Un parpadeo de 1 s sin logo no es un corte.
        for _ in 0..5 {
            assert_eq!(s.paso(0.05, 0.2, 0.35, 2.0), None);
        }
        assert_eq!(s.paso(0.8, 0.2, 0.35, 2.0), None);
        // 2 s seguidos sí.
        let mut evento = None;
        for _ in 0..10 {
            evento = evento.or(s.paso(0.05, 0.2, 0.35, 2.0));
        }
        assert_eq!(evento, Some(Evento::Corte));
        // Sigue el corte: no se repite.
        assert_eq!(s.paso(0.05, 0.2, 0.35, 2.0), None);
        // Vuelve el logo 1,5 s: terminó.
        let mut evento = None;
        for _ in 0..8 {
            evento = evento.or(s.paso(0.8, 0.2, 0.35, 2.0));
        }
        assert_eq!(evento, Some(Evento::Programa));
    }

    #[test]
    fn lista_de_capturadoras() {
        let t = "[dshow @ 0000] \"USB Video\" (video)\n[dshow @ 0000]   Alternative name \"@device_pnp\"\n[dshow @ 0000] \"Micrófono (USB Audio)\" (audio)\n[dshow @ 0000] \"Cam Link 4K\" (video)\n";
        assert_eq!(leer_capturadoras(t), vec!["USB Video".to_string(), "Cam Link 4K".to_string()]);
    }

    #[test]
    fn entradas_de_ffmpeg() {
        let mut c = DeteccionCfg { origen: "USB Video".into(), ..Default::default() };
        assert_eq!(entrada(&c).unwrap().last().unwrap(), "video=USB Video");
        c.origen.clear();
        assert!(entrada(&c).is_err());
        c.fuente = "url".into();
        c.origen = "http://x/y.m3u8".into();
        assert_eq!(entrada(&c).unwrap(), vec!["-i", "http://x/y.m3u8"]);
        let (x, y, w, h) = zona_px(&DeteccionCfg { x: 0.99, y: 0.99, ..Default::default() });
        assert!(x + w <= ANCHO && y + h <= ALTO);
    }

    #[test]
    fn encuentra_el_video_de_una_pagina() {
        let html = r#"<html><script>var p = {"file":"https:\/\/cdn.canal.pe\/vivo\/master.m3u8?token=ab&amp;x=1","img":"a.jpg"};</script>
            <video src='https://otro.pe/v/clip.mp4'></video><a href="/nota.html">x</a></html>"#;
        let v = flujos_en(html);
        assert_eq!(v[0], "https://cdn.canal.pe/vivo/master.m3u8?token=ab&x=1");
        assert_eq!(v[1], "https://otro.pe/v/clip.mp4");
        assert!(flujos_en("<p>nada</p>").is_empty());
        assert!(es_flujo("https://x.pe/vivo/a.m3u8?t=1") && es_flujo("rtmp://x/live"));
        assert!(!es_flujo("https://www.canal.pe/en-vivo"));
        let marcos = marcos_en(
            r#"<iframe width="1" src="//player.pe/e/1"></iframe><iframe src="/local"></iframe>"#,
            "https://canal.pe/vivo",
        );
        assert_eq!(marcos, vec!["https://player.pe/e/1".to_string(), "https://canal.pe/local".to_string()]);
    }

    #[test]
    fn pantalla_de_un_monitor_y_web() {
        let c = Fuente::nueva("pantalla", "", 2, "").unwrap();
        let a = entrada_con(&c, None, Some((1920, 0, 1920, 1080))).unwrap().join(" ");
        assert!(a.contains("1920x1080") && a.contains("1920"), "{a}");
        let w = Fuente::nueva("web", "https://canal.pe/vivo", 0, "").unwrap();
        assert!(entrada_con(&w, None, None).is_err());
        let a = entrada_con(&w, Some("https://cdn/x.m3u8"), None).unwrap();
        assert_eq!(a.last().unwrap(), "https://cdn/x.m3u8");
        assert!(a.iter().any(|x| x.contains("Referer: https://canal.pe/vivo")));
        // IPTV por udp: sin cabeceras de navegador.
        let i = Fuente::nueva("iptv", "", 0, "udp://@239.1.1.1:1234").unwrap();
        assert_eq!(entrada_con(&i, Some("udp://@239.1.1.1:1234"), None).unwrap(), vec!["-i", "udp://@239.1.1.1:1234"]);
    }
}
