//! ffmpeg: dónde está, qué versión es, y descargarlo o actualizarlo por internet.
//!
//! Relevo usa ffmpeg para la vista previa, el monitor y la Biblioteca. Lo busca en
//! varios lugares (junto al programa, la carpeta que deja el instalador, winget,
//! `C:\ffmpeg`, el PATH…). Si no lo encuentra, lo descarga solo; y una vez al día
//! revisa si hay una versión nueva (para actualizar basta un clic).
//!
//! Lo descargado queda en `%LOCALAPPDATA%\Relevo\ffmpeg\ffmpeg.exe`, así lo comparten
//! todas las copias de Relevo de la PC.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Versión estable más reciente (texto, por ejemplo `7.1.1`).
const URL_VERSION: &str = "https://www.gyan.dev/ffmpeg/builds/release-version";
/// Última versión publicada en GitHub (redirige a `.../releases/tag/<versión>`).
const URL_ULTIMA_GITHUB: &str = "https://github.com/GyanD/codexffmpeg/releases/latest";
/// ffmpeg para Windows (64 bits), versión estable "essentials" de gyan.dev, en la copia
/// de GitHub (mucho más rápida que gyan.dev). El `.7z` pesa ~35 MB; el `.zip`, ~90 MB.
const URL_GITHUB: &str = "https://github.com/GyanD/codexffmpeg/releases/download/{v}/ffmpeg-{v}-essentials_build.{ext}";
/// Lo mismo desde gyan.dev (si GitHub no responde).
const URL_GYAN: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.{ext}";
/// Último recurso: compilación diaria de BtbN.
const URL_BTBN: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";

/// Direcciones a probar, en orden: primero las más rápidas y livianas.
fn direcciones(version: Option<&str>) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(ver) = version {
        v.push(URL_GITHUB.replace("{v}", ver).replace("{ext}", "7z"));
    }
    v.push(URL_GYAN.replace("{ext}", "7z"));
    if let Some(ver) = version {
        v.push(URL_GITHUB.replace("{v}", ver).replace("{ext}", "zip"));
    }
    v.push(URL_GYAN.replace("{ext}", "zip"));
    v.push(URL_BTBN.to_string());
    v
}

fn exe() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// Carpeta donde Relevo guarda el ffmpeg que descarga.
pub fn carpeta_propia() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("Relevo").join("ffmpeg")
}

/// Lugares donde puede estar ffmpeg, en orden de preferencia.
pub fn candidatos(configurado: &str, extra: &[PathBuf]) -> Vec<PathBuf> {
    let e = exe();
    let mut v: Vec<PathBuf> = Vec::new();
    if !configurado.trim().is_empty() {
        v.push(PathBuf::from(configurado.trim()));
    }
    // El que descarga y actualiza Relevo.
    v.push(carpeta_propia().join(e));
    // Junto al programa (instalador) y, compilando el proyecto, el que deja empaquetar.ps1.
    if let Ok(propio) = std::env::current_exe() {
        if let Some(c) = propio.parent() {
            v.push(c.join(e));
            for a in c.ancestors().take(5) {
                v.push(a.join("instalador").join("ffmpeg").join(e));
            }
        }
    }
    v.push(PathBuf::from(".").join(e));
    v.push(PathBuf::from("instalador").join("ffmpeg").join(e));
    for c in extra {
        v.push(c.join(e));
    }
    if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        if let Some(l) = &local {
            // winget deja un enlace aquí (y el PATH no se actualiza hasta reiniciar la terminal).
            v.push(l.join("Microsoft").join("WinGet").join("Links").join(e));
            // …y el programa en su carpeta de paquetes.
            if let Ok(l) = std::fs::read_dir(l.join("Microsoft").join("WinGet").join("Packages")) {
                for p in l.flatten().filter(|p| p.file_name().to_string_lossy().to_lowercase().contains("ffmpeg")) {
                    if let Ok(dentro) = std::fs::read_dir(p.path()) {
                        for d in dentro.flatten() {
                            v.push(d.path().join("bin").join(e));
                        }
                    }
                }
            }
        }
        for base in ["C:\\ffmpeg", "C:\\Program Files\\ffmpeg", "C:\\ProgramData\\chocolatey"] {
            v.push(PathBuf::from(base).join("bin").join(e));
        }
        if let Some(h) = dirs::home_dir() {
            v.push(h.join("scoop").join("shims").join(e));
        }
    }
    v.push(PathBuf::from(e)); // PATH
    v
}

/// El primer ffmpeg que funciona.
pub fn buscar(configurado: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    candidatos(configurado, extra).into_iter().find(|c| version(c).is_some())
}

/// Versión de un ffmpeg (`7.1.1`, `N-117000-g…`…), o None si no funciona.
pub fn version(ffmpeg: &Path) -> Option<String> {
    let mut cmd = nucleo::biblioteca::comando(ffmpeg, false);
    cmd.arg("-version").stdout(Stdio::piped()).stderr(Stdio::null());
    let salida = cmd.output().ok()?;
    if !salida.status.success() {
        return None;
    }
    let t = String::from_utf8_lossy(&salida.stdout);
    let v = t.split_whitespace().nth(2)?;
    Some(limpiar_version(v))
}

/// `7.1.1-essentials_build-www.gyan.dev` → `7.1.1`.
fn limpiar_version(v: &str) -> String {
    v.split(['-', '_']).next().unwrap_or(v).trim_start_matches('n').to_string()
}

/// ¿`nueva` es más reciente que `actual`? (compara números: 7.10 > 7.9)
pub fn es_mas_nueva(nueva: &str, actual: &str) -> bool {
    let num = |v: &str| -> Option<Vec<u32>> { v.split('.').map(|p| p.parse().ok()).collect() };
    match (num(nueva), num(actual)) {
        (Some(a), Some(b)) => a > b,
        // Una versión de desarrollo (N-…) no se compara: no se ofrece actualizar.
        _ => false,
    }
}

// ======================================================================
// Estado compartido con la interfaz
// ======================================================================

#[derive(Debug, Clone, Default)]
pub struct Estado {
    pub ruta: Option<PathBuf>,
    pub version: Option<String>,
    /// Última versión publicada (si ya se consultó).
    pub ultima: Option<String>,
    /// Descargando: progreso 0..1 (None = no se sabe el tamaño todavía).
    pub descargando: Option<Option<f32>>,
    /// Durante la descarga: "12 de 34 MB · 9.4 MB/s · faltan 3 s".
    pub detalle_descarga: String,
    pub consultando: bool,
    pub error: Option<String>,
    /// Sube cada vez que cambia el ffmpeg (para avisar a la vista previa y al núcleo).
    pub cambios: u64,
}

impl Estado {
    pub fn hay_actualizacion(&self) -> bool {
        match (&self.ultima, &self.version) {
            (Some(u), Some(v)) => es_mas_nueva(u, v),
            _ => false,
        }
    }
}

pub type Compartido = Arc<Mutex<Estado>>;

/// "45 s", "3 min".
fn duracion_corta(seg: f64) -> String {
    if seg < 60.0 {
        format!("{:.0} s", seg.max(1.0))
    } else {
        format!("{:.0} min", (seg / 60.0).ceil())
    }
}

/// Lo que se recuerda entre usos (cuándo se revisó por última vez).
fn archivo_revision() -> PathBuf {
    carpeta_propia().join("revision.txt")
}

fn hoy() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

/// Al abrir Relevo: si falta ffmpeg lo descarga; si hay, revisa (una vez al día) si hay versión nueva.
pub fn al_iniciar(estado: Compartido, auto: bool, repintar: egui::Context) {
    if !cfg!(windows) {
        return; // las descargas son de ffmpeg para Windows
    }
    let falta = estado.lock().expect("ffmpeg").ruta.is_none();
    if falta {
        descargar(estado, repintar);
        return;
    }
    if !auto {
        return;
    }
    let ya = std::fs::read_to_string(archivo_revision()).ok().and_then(|t| t.trim().parse::<u64>().ok());
    if ya != Some(hoy()) {
        consultar(estado, repintar);
    }
}

/// Pregunta por internet cuál es la última versión.
pub fn consultar(estado: Compartido, repintar: egui::Context) {
    {
        let mut e = estado.lock().expect("ffmpeg");
        if e.consultando {
            return;
        }
        e.consultando = true;
        e.error = None;
    }
    std::thread::spawn(move || {
        let r = ultima_version();
        let mut e = estado.lock().expect("ffmpeg");
        e.consultando = false;
        match r {
            Ok(v) => {
                e.ultima = Some(v);
                let _ = std::fs::create_dir_all(carpeta_propia());
                let _ = std::fs::write(archivo_revision(), hoy().to_string());
            }
            Err(m) => e.error = Some(format!("No se pudo consultar la versión: {m}")),
        }
        drop(e);
        repintar.request_repaint();
    });
}

fn agente() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .build()
        .into()
}

/// Última versión: primero GitHub (responde rápido), si no gyan.dev.
pub fn ultima_version() -> Result<String, String> {
    match version_github() {
        Ok(v) => Ok(v),
        Err(e) => {
            tracing::info!("versión de ffmpeg en GitHub: {e}; se pregunta a gyan.dev");
            version_gyan()
        }
    }
}

fn version_github() -> Result<String, String> {
    let r = agente()
        .get(URL_ULTIMA_GITHUB)
        .config()
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|e| e.to_string())?;
    let destino = r.headers().get("location").and_then(|v| v.to_str().ok()).ok_or("sin redirección")?;
    let v = limpiar_version(destino.rsplit('/').next().unwrap_or(""));
    if v.is_empty() || v.len() > 20 || !v.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(format!("respuesta no válida: {destino}"));
    }
    Ok(v)
}

fn version_gyan() -> Result<String, String> {
    let mut r = agente().get(URL_VERSION).call().map_err(|e| e.to_string())?;
    let t = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v = limpiar_version(t.trim());
    if v.is_empty() || v.len() > 20 {
        return Err("respuesta no válida".into());
    }
    Ok(v)
}

/// Descarga (o actualiza) ffmpeg en segundo plano.
pub fn descargar(estado: Compartido, repintar: egui::Context) {
    {
        let mut e = estado.lock().expect("ffmpeg");
        if e.descargando.is_some() {
            return;
        }
        e.descargando = Some(None);
        e.error = None;
    }
    std::thread::spawn(move || {
        let est = estado.clone();
        let rep = repintar.clone();
        let inicio = std::time::Instant::now();
        let mut ultimo = std::time::Instant::now() - Duration::from_secs(1);
        let avance = move |bajado: u64, total: Option<u64>| {
            let listo = total.is_some_and(|t| bajado >= t);
            if ultimo.elapsed() < Duration::from_millis(150) && !listo {
                return;
            }
            ultimo = std::time::Instant::now();
            let seg = inicio.elapsed().as_secs_f64().max(0.1);
            let vel = bajado as f64 / seg;
            let mb = |b: u64| b as f64 / 1_048_576.0;
            let mut texto = match total {
                Some(t) => format!("{:.0} de {:.0} MB", mb(bajado), mb(t)),
                None => format!("{:.0} MB", mb(bajado)),
            };
            if bajado > 0 {
                texto.push_str(&format!(" · {:.1} MB/s", vel / 1_048_576.0));
                if let Some(t) = total {
                    let falta = (t.saturating_sub(bajado)) as f64 / vel.max(1.0);
                    texto.push_str(&format!(" · faltan {}", duracion_corta(falta)));
                }
            }
            let mut e = est.lock().expect("ffmpeg");
            e.descargando = Some(total.map(|t| (bajado as f32 / t.max(1) as f32).min(1.0)));
            e.detalle_descarga = texto;
            drop(e);
            rep.request_repaint();
        };
        // La versión publicada dice qué archivo bajar de GitHub.
        let ultima = estado.lock().expect("ffmpeg").ultima.clone().or_else(|| ultima_version().ok());
        let urls = direcciones(ultima.as_deref());
        let urls: Vec<&str> = urls.iter().map(String::as_str).collect();
        let r = descargar_en(&carpeta_propia(), &urls, avance);
        let mut e = estado.lock().expect("ffmpeg");
        e.descargando = None;
        match r {
            Ok(ruta) => {
                e.version = version(&ruta);
                if ultima.is_some() {
                    e.ultima = ultima;
                }
                e.ruta = Some(ruta);
                e.cambios += 1;
                tracing::info!("ffmpeg listo: {:?} {:?}", e.ruta, e.version);
            }
            Err(m) => e.error = Some(m),
        }
        drop(e);
        repintar.request_repaint();
    });
}

/// Baja el zip (probando las direcciones en orden), saca `ffmpeg(.exe)` y lo deja en `carpeta`.
/// El ffmpeg anterior, si está en uso, se aparta (`ffmpeg-anterior.exe`) y se borra la próxima vez.
pub fn descargar_en(
    carpeta: &Path,
    urls: &[&str],
    mut avance: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(carpeta).map_err(|e| e.to_string())?;
    let zip = carpeta.join("descarga.tmp");
    let mut errores = Vec::new();
    let mut bajado = false;
    for url in urls {
        match bajar(url, &zip, &mut avance) {
            Ok(()) => {
                bajado = true;
                break;
            }
            Err(e) => {
                tracing::warn!("no se pudo descargar ffmpeg de {url}: {e}");
                errores.push(e);
            }
        }
    }
    if !bajado {
        let _ = std::fs::remove_file(&zip);
        return Err(format!("No se pudo descargar ffmpeg ({}). Revisa la conexión a internet.", errores.join("; ")));
    }
    let nuevo = carpeta.join(format!("{}.nuevo", exe()));
    let r = extraer(&zip, &nuevo);
    let _ = std::fs::remove_file(&zip);
    r?;
    // Reemplazar: el que está en uso se aparta (Windows deja renombrar un .exe abierto).
    let destino = carpeta.join(exe());
    let anterior = carpeta.join(if cfg!(windows) { "ffmpeg-anterior.exe" } else { "ffmpeg-anterior" });
    let _ = std::fs::remove_file(&anterior);
    if destino.exists() {
        std::fs::rename(&destino, &anterior).map_err(|e| format!("no se pudo reemplazar el ffmpeg anterior: {e}"))?;
    }
    std::fs::rename(&nuevo, &destino).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&destino, std::fs::Permissions::from_mode(0o755));
    }
    let _ = std::fs::remove_file(&anterior);
    Ok(destino)
}

fn bajar(url: &str, destino: &Path, avance: &mut impl FnMut(u64, Option<u64>)) -> Result<(), String> {
    let r = agente().get(url).call().map_err(|e| e.to_string())?;
    let total: Option<u64> =
        r.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
    let (_, cuerpo) = r.into_parts();
    let mut lector = cuerpo.into_reader();
    let mut f = std::fs::File::create(destino).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut leido: u64 = 0;
    avance(0, total);
    loop {
        let n = lector.read(&mut buf).map_err(|e| format!("se cortó la descarga: {e}"))?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        leido += n as u64;
        avance(leido, total);
    }
    if total.is_some_and(|t| t != leido) {
        return Err("la descarga quedó incompleta".into());
    }
    Ok(())
}

/// Saca `…/bin/ffmpeg(.exe)` del archivo bajado (`.7z` o `.zip`, según su contenido).
fn extraer(archivo: &Path, destino: &Path) -> Result<(), String> {
    let mut firma = [0u8; 6];
    {
        let mut f = std::fs::File::open(archivo).map_err(|e| e.to_string())?;
        let _ = f.read(&mut firma);
    }
    if firma == [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C] {
        extraer_7z(archivo, destino, exe())
    } else {
        extraer_zip(archivo, destino, exe())
    }
}

fn es_el_buscado(nombre: &str, buscado: &str) -> bool {
    let nombre = nombre.replace('\\', "/");
    nombre.ends_with(&format!("/bin/{buscado}")) || nombre == buscado
}

fn extraer_7z(archivo: &Path, destino: &Path, buscado: &str) -> Result<(), String> {
    use sevenz_rust2::{ArchiveReader, Password};
    let mut lector = ArchiveReader::open(archivo, Password::empty())
        .map_err(|e| format!("el archivo descargado no es un 7z válido: {e}"))?;
    let mut hecho = false;
    let mut error = None;
    lector
        .for_each_entries(|ent, datos| {
            if !ent.is_directory() && es_el_buscado(ent.name(), buscado) {
                match std::fs::File::create(destino).and_then(|mut out| std::io::copy(datos, &mut out)) {
                    Ok(_) => hecho = true,
                    Err(e) => error = Some(e.to_string()),
                }
                return Ok(false); // listo: no hace falta seguir
            }
            // Las demás entradas se saltan (hay que leerlas: el 7z es "sólido").
            std::io::copy(datos, &mut std::io::sink())?;
            Ok(true)
        })
        .map_err(|e| format!("no se pudo abrir el 7z: {e}"))?;
    if let Some(e) = error {
        return Err(e);
    }
    if !hecho {
        return Err(format!("el 7z descargado no trae {buscado}"));
    }
    Ok(())
}

fn extraer_zip(zip: &Path, destino: &Path, buscado: &str) -> Result<(), String> {
    let f = std::fs::File::open(zip).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| format!("el archivo descargado no es un zip válido: {e}"))?;
    for i in 0..z.len() {
        let mut ent = z.by_index(i).map_err(|e| e.to_string())?;
        let nombre = ent.name().map_err(|e| e.to_string())?.to_string();
        if es_el_buscado(&nombre, buscado) {
            let mut out = std::fs::File::create(destino).map_err(|e| e.to_string())?;
            std::io::copy(&mut ent, &mut out).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err(format!("el zip descargado no trae {buscado}"))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn versiones() {
        assert_eq!(limpiar_version("7.1.1-essentials_build-www.gyan.dev"), "7.1.1");
        assert_eq!(limpiar_version("n7.1"), "7.1");
        assert!(es_mas_nueva("7.1.2", "7.1.1"));
        assert!(es_mas_nueva("7.10", "7.9"));
        assert!(!es_mas_nueva("7.1", "7.1"));
        assert!(!es_mas_nueva("6.1.1", "7.0"));
        assert!(!es_mas_nueva("7.1", "N"), "una versión de desarrollo no se compara");
    }

    #[test]
    fn direcciones_de_descarga() {
        let d = direcciones(Some("8.1.1"));
        // Primero el .7z de GitHub (el más liviano y rápido).
        assert_eq!(
            d[0],
            "https://github.com/GyanD/codexffmpeg/releases/download/8.1.1/ffmpeg-8.1.1-essentials_build.7z"
        );
        assert_eq!(d[1], "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.7z");
        assert_eq!(
            d[2],
            "https://github.com/GyanD/codexffmpeg/releases/download/8.1.1/ffmpeg-8.1.1-essentials_build.zip"
        );
        assert_eq!(d.len(), 5);
        assert_eq!(direcciones(None).len(), 3);
    }

    #[test]
    fn busca_en_la_carpeta_del_instalador() {
        let c = candidatos("", &[PathBuf::from("/opt/caspar")]);
        assert!(c.iter().any(|p| p.ends_with(PathBuf::from("instalador").join("ffmpeg").join(exe()))));
        assert!(c.iter().any(|p| p.starts_with("/opt/caspar")));
        assert_eq!(c[0], carpeta_propia().join(exe()), "primero el que descarga Relevo");
    }

    /// Descarga de un servidor local: un zip con `algo/bin/ffmpeg` dentro; si la primera
    /// dirección falla, usa la siguiente; el ffmpeg anterior se reemplaza.
    #[test]
    fn descarga_y_reemplaza() {
        // Zip de prueba.
        let mut datos = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut datos));
            let op = zip::write::SimpleFileOptions::default();
            z.start_file("ffmpeg-7.1.1/LICENSE", op).unwrap();
            z.write_all(b"licencia").unwrap();
            z.start_file(format!("ffmpeg-7.1.1/bin/{}", exe()), op).unwrap();
            z.write_all(b"ffmpeg nuevo").unwrap();
            z.finish().unwrap();
        }
        let largo = datos.len() as u64;
        let escucha = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let puerto = escucha.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for c in escucha.incoming().take(2) {
                let mut c = c.unwrap();
                let mut b = [0u8; 2048];
                let n = c.read(&mut b).unwrap();
                let pedido = String::from_utf8_lossy(&b[..n]).to_string();
                if pedido.starts_with("GET /malo") {
                    let _ = c.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                } else {
                    let cab =
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", datos.len());
                    let _ = c.write_all(cab.as_bytes());
                    let _ = c.write_all(&datos);
                }
            }
        });
        let carpeta = std::env::temp_dir().join(format!("relevo-ffmpeg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&carpeta);
        std::fs::create_dir_all(&carpeta).unwrap();
        std::fs::write(carpeta.join(exe()), b"ffmpeg viejo").unwrap();
        let malo = format!("http://127.0.0.1:{puerto}/malo.zip");
        let bueno = format!("http://127.0.0.1:{puerto}/ffmpeg.zip");
        let mut ultimo = None;
        let r = descargar_en(&carpeta, &[&malo, &bueno], |b, t| ultimo = Some((b, t))).unwrap();
        assert_eq!(std::fs::read(&r).unwrap(), b"ffmpeg nuevo");
        assert_eq!(ultimo, Some((largo, Some(largo))));
        assert!(!carpeta.join("descarga.tmp").exists());
        let _ = std::fs::remove_dir_all(&carpeta);
    }

    /// Saca ffmpeg de un `.7z` real de gyan.dev. Solo corre si se indica el archivo:
    /// `RELEVO_7Z=ruta/ffmpeg-essentials_build.7z cargo test -p app extrae_de_un_7z`.
    #[test]
    fn extrae_de_un_7z_real() {
        let Ok(ruta) = std::env::var("RELEVO_7Z") else { return };
        let destino = std::env::temp_dir().join(format!("relevo-7z-{}", std::process::id()));
        let t = std::time::Instant::now();
        extraer_7z(Path::new(&ruta), &destino, "ffmpeg.exe").unwrap();
        let tam = std::fs::metadata(&destino).unwrap().len();
        eprintln!("ffmpeg extraído: {tam} bytes en {:?}", t.elapsed());
        assert!(tam > 10_000_000);
        let mut cab = [0u8; 2];
        std::fs::File::open(&destino).unwrap().read_exact(&mut cab).unwrap();
        assert_eq!(&cab, b"MZ", "debe ser un .exe de Windows");
        let _ = std::fs::remove_file(&destino);
    }
}
