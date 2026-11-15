//! Biblioteca de spots: revisión de cada video, nivelación de volumen, conversión al
//! formato del canal, reemplazo de versiones y papelera.
//!
//! Todo usa ffmpeg (el mismo de la vista previa) y corre en segundo plano en la PC
//! principal, con prioridad baja para no molestar a CasparCG.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::archivos;

/// Días que un video queda en la papelera antes de borrarse del disco.
pub const DIAS_PAPELERA: u64 = 30;
/// Por debajo de este espacio libre se avisa.
pub const DISCO_BAJO: u64 = 5 * 1024 * 1024 * 1024;

/// Lo que se sabe de un video tras revisarlo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ficha {
    /// Ruta dentro de la carpeta de medios, con extensión (p. ej. `CLIENTES/Ferreteria.mp4`).
    pub ruta: String,
    pub tamano: u64,
    /// Fecha de modificación (segundos Unix) cuando se revisó.
    pub modificado: u64,
    pub duracion: f64,
    pub ancho: u32,
    pub alto: u32,
    pub fps: f64,
    pub codec: String,
    pub audio: bool,
    pub canales: u8,
    /// Volumen integrado (LUFS). `None` si no tiene audio.
    pub lufs: Option<f32>,
    /// Pico verdadero (dBTP).
    pub pico: Option<f32>,
    /// ffmpeg no pudo leerlo.
    pub error: Option<String>,
}

/// Formato y volumen que usa el canal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Estandar {
    pub ancho: u32,
    pub alto: u32,
    pub fps: f64,
    /// Volumen objetivo (LUFS). -23 es la norma EBU R128; -24 la ATSC A/85.
    pub lufs: f32,
    /// Diferencia de volumen aceptada (± dB).
    pub tolerancia: f32,
}

impl Default for Estandar {
    fn default() -> Self {
        Estandar { ancho: 1920, alto: 1080, fps: 29.97, lufs: -23.0, tolerancia: 2.0 }
    }
}

/// Algo que conviene corregir en un video.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Problema {
    Ilegible(String),
    SinAudio,
    Silencio,
    /// Más fuerte que el canal (dB de más).
    Fuerte(f32),
    /// Más bajo que el canal (dB de menos).
    Bajo(f32),
    Resolucion(u32, u32),
    Fps(f64),
}

impl Problema {
    pub fn describir(&self) -> String {
        match self {
            Problema::Ilegible(e) => format!("No se puede leer: {e}"),
            Problema::SinAudio => "No tiene audio".into(),
            Problema::Silencio => "El audio está en silencio".into(),
            Problema::Fuerte(d) => format!("Suena {d:.0} dB más fuerte que el canal"),
            Problema::Bajo(d) => format!("Suena {d:.0} dB más bajo que el canal"),
            Problema::Resolucion(a, b) => format!("Resolución {a}×{b}, distinta a la del canal"),
            Problema::Fps(f) => format!("{f:.2} cuadros/s, distinto al canal"),
        }
    }

    /// Se arregla nivelando el volumen (sin tocar la imagen).
    pub fn es_de_volumen(&self) -> bool {
        matches!(self, Problema::Fuerte(_) | Problema::Bajo(_))
    }

    /// Grave: el spot puede salir mal al aire.
    pub fn es_grave(&self) -> bool {
        matches!(self, Problema::Ilegible(_) | Problema::SinAudio | Problema::Silencio)
            || matches!(self, Problema::Fuerte(d) if *d >= 6.0)
    }
}

/// Lo que habría que corregir en un video para que cumpla el estándar del canal.
pub fn problemas(f: &Ficha, est: &Estandar) -> Vec<Problema> {
    let mut v = Vec::new();
    if let Some(e) = &f.error {
        v.push(Problema::Ilegible(e.clone()));
        return v;
    }
    if !f.audio {
        v.push(Problema::SinAudio);
    } else if let Some(l) = f.lufs {
        if l <= -60.0 {
            v.push(Problema::Silencio);
        } else if l - est.lufs > est.tolerancia {
            v.push(Problema::Fuerte(l - est.lufs));
        } else if est.lufs - l > est.tolerancia {
            v.push(Problema::Bajo(est.lufs - l));
        }
    }
    if f.ancho > 0 && (f.ancho != est.ancho || f.alto != est.alto) {
        v.push(Problema::Resolucion(f.ancho, f.alto));
    }
    // 29.97 y 30 (o 59.94 y 60) se consideran iguales: CasparCG los adapta sin saltos visibles.
    if f.fps > 0.0 && est.fps > 0.0 && (f.fps - est.fps).abs() > 0.1 && (f.fps - est.fps * 2.0).abs() > 0.1 {
        v.push(Problema::Fps(f.fps));
    }
    v
}

// ======================================================================
// Revisión con ffmpeg
// ======================================================================

/// Prepara un comando de ffmpeg sin ventana y, si `baja` es true, con prioridad baja.
pub fn comando(ffmpeg: &Path, baja: bool) -> Command {
    let mut cmd = Command::new(ffmpeg);
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW y, para los trabajos pesados, BELOW_NORMAL_PRIORITY_CLASS.
        cmd.creation_flags(0x0800_0000 | if baja { 0x0000_4000 } else { 0 });
    }
    let _ = baja;
    cmd
}

/// Revisa un video: formato, audio y volumen. Tarda lo que tarda decodificar su audio
/// (un spot de 30 s, menos de un segundo).
pub fn analizar(ffmpeg: &Path, archivo: &Path) -> Ficha {
    let meta = std::fs::metadata(archivo).ok();
    let mut cmd = comando(ffmpeg, true);
    cmd.args(["-hide_banner", "-nostats", "-i"])
        .arg(archivo)
        .args(["-vn", "-sn", "-dn", "-af", "ebur128=peak=true:framelog=quiet", "-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut f = match cmd.output() {
        Ok(salida) => leer_salida(&String::from_utf8_lossy(&salida.stderr)),
        Err(e) => Ficha { error: Some(format!("no se pudo ejecutar ffmpeg: {e}")), ..Default::default() },
    };
    if let Some(m) = meta {
        f.tamano = m.len();
        f.modificado = segundos(m.modified().ok());
    }
    f
}

pub fn segundos(t: Option<SystemTime>) -> u64 {
    t.and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// Interpreta lo que ffmpeg escribe al revisar un video.
pub fn leer_salida(texto: &str) -> Ficha {
    let mut f = Ficha::default();
    // Solo la parte de la entrada (antes de "Stream mapping"/"Output #0").
    let entrada = texto.split("Stream mapping:").next().unwrap_or(texto);
    let entrada = entrada.split("Output #0").next().unwrap_or(entrada);
    if !entrada.contains("Input #0") {
        let ultima = texto.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("formato desconocido");
        f.error = Some(ultima.trim().to_string());
        return f;
    }
    for linea in entrada.lines() {
        let l = linea.trim();
        if let Some(i) = l.find("Duration: ") {
            f.duracion = hms(l[i + 10..].split(',').next().unwrap_or("")).unwrap_or(0.0);
        }
        if !l.starts_with("Stream #0:") {
            continue;
        }
        if let Some(i) = l.find(": Video: ") {
            if f.ancho > 0 || l.contains("attached pic") {
                continue; // la primera pista de video (y no una carátula)
            }
            let resto = &l[i + 9..];
            f.codec = resto.split([' ', ',']).next().unwrap_or("").to_string();
            for parte in resto.split(", ") {
                let p = parte.trim();
                if f.ancho == 0 {
                    if let Some((a, b)) = p.split_whitespace().next().and_then(|t| t.split_once('x')) {
                        if let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) {
                            f.ancho = a;
                            f.alto = b;
                        }
                    }
                }
                if let Some(n) = p.strip_suffix(" fps") {
                    f.fps = n.trim().parse().unwrap_or(0.0);
                }
            }
        } else if let Some(i) = l.find(": Audio: ") {
            if f.audio {
                continue;
            }
            f.audio = true;
            let resto = &l[i + 9..];
            f.canales = resto
                .split(", ")
                .find_map(|p| match p.trim() {
                    "mono" => Some(1),
                    "stereo" => Some(2),
                    "5.1" | "5.1(side)" => Some(6),
                    "7.1" => Some(8),
                    otro => otro.strip_suffix(" channels").and_then(|n| n.parse().ok()),
                })
                .unwrap_or(2);
        }
    }
    if f.ancho == 0 && f.error.is_none() {
        f.error = Some("no tiene imagen".into());
    }
    // Resumen del medidor de volumen.
    if let Some(i) = texto.rfind("Summary:") {
        let resumen = &texto[i..];
        f.lufs = valor_tras(resumen, "I:");
        f.pico = valor_tras(resumen, "Peak:");
    }
    f
}

fn valor_tras(texto: &str, etiqueta: &str) -> Option<f32> {
    let i = texto.find(etiqueta)? + etiqueta.len();
    let v = texto[i..].split_whitespace().next()?;
    if v == "-inf" {
        return Some(-99.0);
    }
    v.parse().ok()
}

fn hms(t: &str) -> Option<f64> {
    let mut p = t.trim().split(':');
    let (h, m, s): (f64, f64, f64) = (p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?);
    Some(h * 3600.0 + m * 60.0 + s)
}

// ======================================================================
// Trabajos: nivelar volumen, convertir, reemplazar
// ======================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TipoTrabajo {
    /// Ajusta el volumen al del canal. La imagen no se toca.
    Nivelar,
    /// Lleva el video al formato del canal (resolución, cuadros/s, H.264 + AAC) y nivela el volumen.
    Convertir,
    /// Pone otro archivo en lugar de este video (el spot conserva su nombre y su pauta).
    Reemplazar { desde: PathBuf },
}

impl TipoTrabajo {
    pub fn nombre(&self) -> &'static str {
        match self {
            TipoTrabajo::Nivelar => "Nivelar volumen",
            TipoTrabajo::Convertir => "Convertir al formato del canal",
            TipoTrabajo::Reemplazar { .. } => "Reemplazar video",
        }
    }
}

/// Ganancia (dB) para llegar al volumen del canal y si hace falta limitar los picos.
pub fn ganancia(f: &Ficha, est: &Estandar) -> Option<(f32, bool)> {
    let l = f.lufs.filter(|l| *l > -60.0)?;
    let g = est.lufs - l;
    let pico_nuevo = f.pico.unwrap_or(0.0) + g;
    Some((g, pico_nuevo > -1.0))
}

/// Filtro de audio para nivelar (o `None` si no hay audio que nivelar).
fn filtro_audio(f: &Ficha, est: &Estandar) -> Option<String> {
    let (g, limitar) = ganancia(f, est)?;
    let mut s = format!("volume={g:.2}dB");
    if limitar {
        // Limita a -1 dBFS para que subir el volumen no sature.
        s.push_str(",alimiter=limit=0.891:level=false");
    }
    Some(s)
}

/// Argumentos de ffmpeg para un trabajo (sin la entrada ni la salida).
pub fn argumentos(tipo: &TipoTrabajo, f: &Ficha, est: &Estandar, copiar_video: bool) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let audio = filtro_audio(f, est);
    let s = |t: &str| t.to_string();
    match tipo {
        TipoTrabajo::Nivelar if copiar_video => {
            a.extend([s("-map"), s("0:v:0"), s("-map"), s("0:a:0"), s("-c:v"), s("copy")]);
        }
        TipoTrabajo::Nivelar | TipoTrabajo::Convertir => {
            let (w, h) = (est.ancho, est.alto);
            let fps = if est.fps > 0.0 { format!(",fps={}", fps_texto(est.fps)) } else { String::new() };
            let vf = format!(
                "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1{fps}"
            );
            a.extend([s("-map"), s("0:v:0"), s("-map"), s("0:a:0?"), s("-vf"), vf]);
            a.extend([
                s("-c:v"),
                s("libx264"),
                s("-preset"),
                s("medium"),
                s("-crf"),
                s("18"),
                s("-pix_fmt"),
                s("yuv420p"),
            ]);
        }
        TipoTrabajo::Reemplazar { .. } => return a,
    }
    if f.audio {
        if let Some(af) = audio {
            a.extend([s("-af"), af]);
        }
        a.extend([s("-c:a"), s("aac"), s("-b:a"), s("192k"), s("-ar"), s("48000")]);
        if f.canales > 2 || matches!(tipo, TipoTrabajo::Convertir) {
            a.extend([s("-ac"), s("2")]);
        }
    }
    a.extend([s("-movflags"), s("+faststart")]);
    a
}

/// 29.97 → "30000/1001", 59.94 → "60000/1001"; el resto tal cual.
fn fps_texto(fps: f64) -> String {
    for (n, base) in [(23.976, 24000), (29.97, 30000), (59.94, 60000)] {
        if (fps - n).abs() < 0.01 {
            return format!("{base}/1001");
        }
    }
    format!("{fps}")
}

/// Hace un trabajo y deja el resultado en `salida` (un .mp4). `avance` recibe 0..1.
#[allow(clippy::too_many_arguments)]
pub fn hacer(
    ffmpeg: &Path,
    tipo: &TipoTrabajo,
    original: &Path,
    ficha: &Ficha,
    est: &Estandar,
    salida: &Path,
    cancelar: &AtomicBool,
    mut avance: impl FnMut(f32),
) -> Result<(), String> {
    if let TipoTrabajo::Reemplazar { desde } = tipo {
        return archivos::copiar_con_progreso(desde, salida, &mut avance).map_err(|e| e.to_string());
    }
    let contenedor_ok = matches!(
        original.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref(),
        Some("mp4" | "mov" | "m4v")
    );
    let intentos: Vec<bool> = match tipo {
        // Nivelar: primero copiando la imagen (rápido); si el códec no entra en MP4, recodificando.
        TipoTrabajo::Nivelar if contenedor_ok || ficha.codec == "h264" || ficha.codec == "hevc" => vec![true, false],
        _ => vec![false],
    };
    let mut ultimo = String::new();
    for copiar in intentos {
        let args = argumentos(tipo, ficha, est, copiar);
        match correr_ffmpeg(ffmpeg, original, &args, salida, ficha.duracion, cancelar, &mut avance) {
            Ok(()) => return Ok(()),
            Err(e) => {
                let _ = std::fs::remove_file(salida);
                if cancelar.load(Ordering::SeqCst) {
                    return Err("cancelado".into());
                }
                ultimo = e;
            }
        }
    }
    Err(ultimo)
}

fn correr_ffmpeg(
    ffmpeg: &Path,
    entrada: &Path,
    args: &[String],
    salida: &Path,
    duracion: f64,
    cancelar: &AtomicBool,
    avance: &mut impl FnMut(f32),
) -> Result<(), String> {
    let mut cmd = comando(ffmpeg, true);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostats", "-y", "-i"])
        .arg(entrada)
        .args(args)
        .args(["-f", "mp4", "-progress", "pipe:1"])
        .arg(salida)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut hijo = cmd.spawn().map_err(|e| format!("no se pudo ejecutar ffmpeg: {e}"))?;
    let mut err = hijo.stderr.take().expect("stderr");
    let lector_err = std::thread::spawn(move || {
        let mut t = String::new();
        let _ = err.read_to_string(&mut t);
        t
    });
    let salida_prog = BufReader::new(hijo.stdout.take().expect("stdout"));
    for linea in salida_prog.lines() {
        let Ok(l) = linea else { break };
        if cancelar.load(Ordering::SeqCst) {
            let _ = hijo.kill();
            break;
        }
        if let Some(us) = l.strip_prefix("out_time_us=").or_else(|| l.strip_prefix("out_time_ms=")) {
            if let (Ok(us), true) = (us.trim().parse::<f64>(), duracion > 0.0) {
                avance(((us / 1_000_000.0) / duracion).clamp(0.0, 1.0) as f32);
            }
        }
    }
    let estado = hijo.wait().map_err(|e| e.to_string())?;
    let errores = lector_err.join().unwrap_or_default();
    if cancelar.load(Ordering::SeqCst) {
        return Err("cancelado".into());
    }
    if !estado.success() {
        let e = errores.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg falló").trim().to_string();
        return Err(e);
    }
    avance(1.0);
    Ok(())
}

/// Cómo va un trabajo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EstadoTrabajo {
    EnCola,
    Haciendo,
    /// Terminado, esperando a que el spot salga de la tanda para cambiar el archivo.
    Esperando,
    Error(String),
}

/// Un trabajo de la biblioteca, tal como lo ve la interfaz.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trabajo {
    pub id: u64,
    pub clip: String,
    pub tipo: TipoTrabajo,
    pub estado: EstadoTrabajo,
    pub progreso: f32,
    /// Quién lo pidió.
    pub quien: String,
}

/// Estado de la biblioteca para la interfaz (sin las fichas, que van aparte).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VistaBiblioteca {
    pub trabajos: Vec<Trabajo>,
    /// Revisión en curso: (revisados, total).
    pub revision: Option<(usize, usize)>,
    pub papelera: Vec<EnPapelera>,
    pub disco_libre: Option<u64>,
    pub estandar: Estandar,
    /// Hay ffmpeg en la PC principal.
    pub ffmpeg: bool,
    /// Crece cada vez que cambia un archivo de la carpeta de medios (para recargar miniaturas).
    pub cambios: u64,
}

// ======================================================================
// Archivos de la carpeta de medios
// ======================================================================

/// Clave de un clip para buscarlo sin distinguir mayúsculas.
pub fn clave(clip: &str) -> String {
    clip.replace('\\', "/").to_uppercase()
}

/// Todos los videos de la carpeta de medios: clave del clip → archivo.
pub fn indice(medios: &Path) -> HashMap<String, PathBuf> {
    let mut mapa = HashMap::new();
    let mut pendientes = vec![medios.to_path_buf()];
    while let Some(dir) = pendientes.pop() {
        let Ok(lectura) = std::fs::read_dir(&dir) else { continue };
        for e in lectura.flatten() {
            let p = e.path();
            let Ok(tipo) = e.file_type() else { continue };
            if tipo.is_dir() {
                pendientes.push(p);
            } else if archivos::es_video(&p) {
                if let Some(c) = archivos::clip_de(&p, medios) {
                    mapa.insert(clave(&c), p);
                }
            }
        }
    }
    mapa
}

/// Ruta de un archivo dentro de la carpeta de medios, con '/'.
pub fn relativa(archivo: &Path, medios: &Path) -> String {
    archivo.strip_prefix(medios).unwrap_or(archivo).to_string_lossy().replace('\\', "/")
}

/// Mueve un archivo (si es otra unidad, lo copia y borra el original).
pub fn mover(de: &Path, a: &Path) -> io::Result<()> {
    if let Some(c) = a.parent() {
        std::fs::create_dir_all(c)?;
    }
    if std::fs::rename(de, a).is_ok() {
        return Ok(());
    }
    std::fs::copy(de, a)?;
    std::fs::remove_file(de)
}

// ======================================================================
// Papelera
// ======================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnPapelera {
    pub id: u64,
    /// Dónde estaba, dentro de la carpeta de medios (con extensión).
    pub ruta_original: String,
    /// Nombre del archivo dentro de la papelera.
    pub archivo: String,
    /// Cuándo se tiró (segundos Unix).
    pub fecha: u64,
    pub motivo: String,
    pub tamano: u64,
}

/// Papelera en `datos/papelera`. Los videos se guardan 30 días.
pub struct Papelera {
    carpeta: PathBuf,
    pub entradas: Vec<EnPapelera>,
}

impl Papelera {
    pub fn abrir(carpeta_datos: &Path) -> Self {
        let carpeta = carpeta_datos.join("papelera");
        let entradas = std::fs::read_to_string(carpeta.join("papelera.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Papelera { carpeta, entradas }
    }

    fn guardar(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.carpeta)?;
        let t = serde_json::to_string_pretty(&self.entradas).map_err(io::Error::other)?;
        let tmp = self.carpeta.join("papelera.json.tmp");
        std::fs::write(&tmp, t)?;
        std::fs::rename(tmp, self.carpeta.join("papelera.json"))
    }

    /// Tira un archivo de la carpeta de medios a la papelera.
    pub fn tirar(&mut self, archivo: &Path, medios: &Path, motivo: &str, ahora: u64) -> io::Result<EnPapelera> {
        let id = self.entradas.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        let nombre = archivo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let destino = format!("{id:05} {nombre}");
        let tamano = std::fs::metadata(archivo).map(|m| m.len()).unwrap_or(0);
        mover(archivo, &self.carpeta.join(&destino))?;
        let e = EnPapelera {
            id,
            ruta_original: relativa(archivo, medios),
            archivo: destino,
            fecha: ahora,
            motivo: motivo.to_string(),
            tamano,
        };
        self.entradas.push(e.clone());
        self.guardar()?;
        Ok(e)
    }

    /// Devuelve un archivo a su lugar. Si ya hay otro video con ese nombre, no lo pisa.
    pub fn restaurar(&mut self, id: u64, medios: &Path) -> Result<PathBuf, String> {
        let i = self.entradas.iter().position(|e| e.id == id).ok_or("ya no está en la papelera")?;
        let e = self.entradas[i].clone();
        let mut destino = medios.join(&e.ruta_original);
        if let Some(clip) = archivos::clip_de(&destino, medios) {
            let ya = indice_de_carpeta(destino.parent().unwrap_or(medios), medios)
                .into_iter()
                .any(|(c, _)| c == clave(&clip));
            if ya {
                let base = destino.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                let ext = destino.extension().map(|x| format!(".{}", x.to_string_lossy())).unwrap_or_default();
                destino.set_file_name(format!("{base} (restaurado){ext}"));
            }
        }
        mover(&self.carpeta.join(&e.archivo), &destino).map_err(|x| x.to_string())?;
        self.entradas.remove(i);
        self.guardar().map_err(|x| x.to_string())?;
        Ok(destino)
    }

    /// Borra del disco para siempre.
    pub fn eliminar(&mut self, id: u64) -> io::Result<()> {
        if let Some(i) = self.entradas.iter().position(|e| e.id == id) {
            let e = self.entradas.remove(i);
            let _ = std::fs::remove_file(self.carpeta.join(&e.archivo));
            self.guardar()?;
        }
        Ok(())
    }

    /// Borra lo que lleva más de [`DIAS_PAPELERA`] días. Devuelve cuántos borró.
    pub fn purgar(&mut self, ahora: u64) -> usize {
        let limite = ahora.saturating_sub(DIAS_PAPELERA * 24 * 3600);
        let viejos: Vec<u64> = self.entradas.iter().filter(|e| e.fecha < limite).map(|e| e.id).collect();
        for id in &viejos {
            let _ = self.eliminar(*id);
        }
        viejos.len()
    }
}

fn indice_de_carpeta(dir: &Path, medios: &Path) -> Vec<(String, PathBuf)> {
    std::fs::read_dir(dir)
        .map(|l| {
            l.flatten()
                .map(|e| e.path())
                .filter(|p| archivos::es_video(p))
                .filter_map(|p| archivos::clip_de(&p, medios).map(|c| (clave(&c), p)))
                .collect()
        })
        .unwrap_or_default()
}

/// Revisiones guardadas (`datos/biblioteca.json`), para no repetirlas cada vez que se abre la app.
pub fn cargar_fichas(carpeta_datos: &Path) -> HashMap<String, Ficha> {
    std::fs::read_to_string(carpeta_datos.join("biblioteca.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn guardar_fichas(carpeta_datos: &Path, fichas: &HashMap<String, Ficha>) {
    let _ = std::fs::create_dir_all(carpeta_datos);
    if let Ok(t) = serde_json::to_string(fichas) {
        let tmp = carpeta_datos.join("biblioteca.json.tmp");
        if std::fs::write(&tmp, t).is_ok() {
            let _ = std::fs::rename(tmp, carpeta_datos.join("biblioteca.json"));
        }
    }
}

/// La ficha sigue valiendo si el archivo no cambió.
pub fn vigente(f: &Ficha, archivo: &Path) -> bool {
    std::fs::metadata(archivo)
        .map(|m| m.len() == f.tamano && segundos(m.modified().ok()) == f.modificado)
        .unwrap_or(false)
}

/// Espacio libre en la unidad de una carpeta.
pub fn espacio_libre(carpeta: &Path) -> Option<u64> {
    fs4::available_space(carpeta).ok()
}

pub fn ahora() -> u64 {
    segundos(Some(SystemTime::now()))
}

/// Duración legible para los avisos.
pub fn tiempo(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const SALIDA: &str = r#"Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'spot.mp4':
  Duration: 00:00:25.00, start: 0.000000, bitrate: 833 kb/s
  Stream #0:0[0x1](und): Video: h264 (Constrained Baseline) (avc1 / 0x31637661), yuv420p(progressive), 1280x720 [SAR 1:1 DAR 16:9], 757 kb/s, 29.97 fps, 29.97 tbr, 30k tbn (default)
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, stereo, fltp, 69 kb/s (default)
Stream mapping:
  Stream #0:1 -> #0:0 (aac (native) -> pcm_s16le (native))
Output #0, null, to 'pipe:':
  Stream #0:0(und): Audio: pcm_s16le, 44100 Hz, mono, s16, 705 kb/s (default)
[Parsed_ebur128_0 @ 0x55fc67e5c5c0] Summary:

  Integrated loudness:
    I:         -14.2 LUFS
    Threshold: -24.2 LUFS

  Loudness range:
    LRA:         3.1 LU

  True peak:
    Peak:       -0.4 dBFS
"#;

    #[test]
    fn lee_formato_y_volumen() {
        let f = leer_salida(SALIDA);
        assert_eq!((f.ancho, f.alto), (1280, 720));
        assert!((f.fps - 29.97).abs() < 0.001);
        assert_eq!(f.codec, "h264");
        assert!(f.audio);
        assert_eq!(f.canales, 2);
        assert_eq!(f.lufs, Some(-14.2));
        assert_eq!(f.pico, Some(-0.4));
        assert!((f.duracion - 25.0).abs() < 0.01);
        assert!(f.error.is_none());
    }

    #[test]
    fn archivo_que_no_es_video() {
        let f = leer_salida("spot.mp4: Invalid data found when processing input\n");
        assert!(f.error.unwrap().contains("Invalid data"));
    }

    #[test]
    fn problemas_frente_al_canal() {
        let est = Estandar::default();
        let f = leer_salida(SALIDA);
        let p = problemas(&f, &est);
        assert!(p.contains(&Problema::Resolucion(1280, 720)));
        assert!(p.iter().any(|x| matches!(x, Problema::Fuerte(d) if (*d - 8.8).abs() < 0.01)));
        assert!(p.iter().any(|x| x.es_grave()));

        let bien = Ficha {
            ancho: 1920,
            alto: 1080,
            fps: 30.0,
            audio: true,
            canales: 2,
            lufs: Some(-22.0),
            ..Default::default()
        };
        assert!(problemas(&bien, &est).is_empty(), "30 y 29.97 cuentan igual; 1 dB está dentro de lo aceptado");

        let mudo = Ficha { ancho: 1920, alto: 1080, audio: false, ..Default::default() };
        assert_eq!(problemas(&mudo, &est), vec![Problema::SinAudio]);
    }

    #[test]
    fn nivelar_baja_el_volumen_y_limita_si_sube() {
        let est = Estandar::default();
        let fuerte = leer_salida(SALIDA);
        let (g, limitar) = ganancia(&fuerte, &est).unwrap();
        assert!((g + 8.8).abs() < 0.01);
        assert!(!limitar, "al bajar no hace falta limitar");
        let args = argumentos(&TipoTrabajo::Nivelar, &fuerte, &est, true);
        assert!(args.windows(2).any(|w| w == ["-c:v", "copy"]));
        assert!(args.iter().any(|a| a == "volume=-8.80dB"));

        let bajo = Ficha { lufs: Some(-35.0), pico: Some(-8.0), audio: true, canales: 2, ..Default::default() };
        let (g, limitar) = ganancia(&bajo, &est).unwrap();
        assert!((g - 12.0).abs() < 0.01);
        assert!(limitar, "subir 12 dB llevaría el pico a +4 dB");
    }

    #[test]
    fn convertir_lleva_al_formato_del_canal() {
        let est = Estandar::default();
        let f = leer_salida(SALIDA);
        let args = argumentos(&TipoTrabajo::Convertir, &f, &est, false);
        let vf = args.iter().skip_while(|a| *a != "-vf").nth(1).unwrap();
        assert!(vf.contains("scale=1920:1080") && vf.contains("fps=30000/1001"), "{vf}");
        assert!(args.iter().any(|a| a == "libx264"));
        assert!(args.windows(2).any(|w| w == ["-ac", "2"]));
    }

    #[test]
    fn papelera_tira_restaura_y_purga() {
        let base = std::env::temp_dir().join(format!("relevo-papelera-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let medios = base.join("media");
        std::fs::create_dir_all(medios.join("CLIENTES")).unwrap();
        let spot = medios.join("CLIENTES").join("Ferreteria.mp4");
        std::fs::write(&spot, b"video").unwrap();

        let mut p = Papelera::abrir(&base.join("datos"));
        let e = p.tirar(&spot, &medios, "borrado", 1_000).unwrap();
        assert!(!spot.exists());
        assert_eq!(e.ruta_original, "CLIENTES/Ferreteria.mp4");

        // Se vuelve a abrir (se guardó en disco).
        let mut p = Papelera::abrir(&base.join("datos"));
        assert_eq!(p.entradas.len(), 1);

        // Si mientras tanto hay otro video con el mismo nombre, no se pisa.
        std::fs::write(&spot, b"nuevo").unwrap();
        let r = p.restaurar(e.id, &medios).unwrap();
        assert_eq!(r.file_name().unwrap(), "Ferreteria (restaurado).mp4");
        assert_eq!(std::fs::read(&spot).unwrap(), b"nuevo");

        // Purga a los 30 días.
        p.tirar(&spot, &medios, "x", 1_000).unwrap();
        assert_eq!(p.purgar(1_000 + 29 * 86_400), 0);
        assert_eq!(p.purgar(1_000 + 31 * 86_400), 1);
        assert!(p.entradas.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn indice_de_la_carpeta_de_medios() {
        let base = std::env::temp_dir().join(format!("relevo-indice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("IDS")).unwrap();
        std::fs::write(base.join("IDS").join("Canal.mov"), b"x").unwrap();
        std::fs::write(base.join("Nota.txt"), b"x").unwrap();
        let i = indice(&base);
        assert_eq!(i.len(), 1);
        assert!(i.contains_key("IDS/CANAL"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn revisa_y_nivela_un_video_real() {
        // Solo si hay ffmpeg en esta PC.
        let ffmpeg = PathBuf::from(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
        if Command::new(&ffmpeg).arg("-version").output().is_err() {
            return;
        }
        let base = std::env::temp_dir().join(format!("relevo-nivelar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let spot = base.join("fuerte.mp4");
        let ok = Command::new(&ffmpeg)
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=4"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4,volume=7"])
            .args(["-c:v", "libx264", "-c:a", "aac", "-shortest"])
            .arg(&spot)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return;
        }
        let f = analizar(&ffmpeg, &spot);
        assert!(f.audio && f.lufs.unwrap() > -10.0, "{f:?}");
        let est = Estandar::default();
        let salida = base.join("nivelado.mp4");
        let cancelar = AtomicBool::new(false);
        let mut ultimo = 0.0;
        hacer(&ffmpeg, &TipoTrabajo::Nivelar, &spot, &f, &est, &salida, &cancelar, |p| ultimo = p).unwrap();
        assert_eq!(ultimo, 1.0);
        let n = analizar(&ffmpeg, &salida);
        assert!((n.lufs.unwrap() - est.lufs).abs() < 1.5, "quedó en {:?}", n.lufs);
        assert_eq!((n.ancho, n.alto), (320, 180), "nivelar no toca la imagen");
        let _ = std::fs::remove_dir_all(&base);
    }
}
