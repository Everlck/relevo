//! Actualización de Relevo desde las versiones publicadas en GitHub (Releases).
//!
//! Al abrir Relevo (y cada 6 horas) se pregunta a GitHub cuál es la última versión del
//! repositorio. Si es más nueva, la barra de abajo lo avisa; al pulsar «Actualizar» se
//! baja su instalador (`Relevo-X.Y.Z-Instalador.exe`), se cierra Relevo y el instalador
//! se ejecuta en silencio: reemplaza el programa, conserva `config.toml` y los datos, y
//! vuelve a abrir Relevo.
//!
//! Nunca se actualiza sola, y la interfaz no deja hacerlo con publicidad al aire.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::ActualizacionCfg;

/// Versión de este programa (sale de Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Cada cuánto se vuelve a preguntar.
pub const CADA: Duration = Duration::from_secs(6 * 3600);

/// Una versión publicada.
#[derive(Debug, Clone, PartialEq)]
pub struct Publicada {
    /// "1.0.1" (sin la «v» del tag).
    pub numero: String,
    /// Lo que se escribió en la Release (novedades).
    pub notas: String,
    /// Página de la Release.
    pub pagina: String,
    /// Instalador: dirección de descarga directa, dirección de la API (para repositorios
    /// privados, con clave) y tamaño en bytes.
    pub descarga: String,
    pub descarga_api: String,
    pub tamano: u64,
    pub nombre: String,
}

#[derive(Debug, Clone, Default)]
pub struct Estado {
    /// Última versión publicada (si ya se consultó y hay alguna con instalador).
    pub ultima: Option<Publicada>,
    pub consultando: bool,
    pub revisado: Option<Instant>,
    /// Descargando: progreso 0..1 (None = sin tamaño todavía).
    pub descargando: Option<Option<f32>>,
    pub detalle: String,
    /// Instalador ya descargado, listo para ejecutar.
    pub listo: Option<PathBuf>,
    pub error: Option<String>,
}

impl Estado {
    pub fn hay_nueva(&self) -> bool {
        self.ultima.as_ref().is_some_and(|u| crate::ffmpeg::es_mas_nueva(&u.numero, VERSION))
    }
}

pub type Compartido = Arc<Mutex<Estado>>;

/// ¿Es una copia compilada a mano (cargo run)? Entonces no se ofrece actualizar.
pub fn es_desarrollo() -> bool {
    std::env::current_exe().ok().is_some_and(|e| e.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target")))
}

fn agente() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// Pregunta a GitHub (en segundo plano) por la última versión.
pub fn consultar(estado: Compartido, cfg: ActualizacionCfg, repintar: egui::Context) {
    {
        let mut e = estado.lock().expect("actualizacion");
        if e.consultando || e.descargando.is_some() {
            return;
        }
        e.consultando = true;
        e.error = None;
    }
    std::thread::spawn(move || {
        let r = ultima(&cfg);
        let mut e = estado.lock().expect("actualizacion");
        e.consultando = false;
        e.revisado = Some(Instant::now());
        match r {
            Ok(u) => {
                if let Some(p) = &u {
                    tracing::info!("Relevo: última versión publicada {} (esta es {VERSION})", p.numero);
                }
                e.ultima = u;
            }
            Err(m) => {
                tracing::info!("Relevo: no se pudo revisar si hay versión nueva: {m}");
                e.error = Some(m);
            }
        }
        drop(e);
        repintar.request_repaint();
    });
}

fn ultima(cfg: &ActualizacionCfg) -> Result<Option<Publicada>, String> {
    let repo = cfg.repo.trim();
    if repo.split('/').count() != 2 {
        return Err(format!("repositorio no válido: «{repo}» (debe ser usuario/nombre)"));
    }
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let mut pedido = agente()
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", &format!("Relevo/{VERSION}"));
    let token = cfg.token.trim();
    if !token.is_empty() {
        pedido = pedido.header("Authorization", &format!("Bearer {token}"));
    }
    let mut r = pedido.call().map_err(|e| format!("sin conexión con GitHub ({e})"))?;
    let codigo = r.status().as_u16();
    let texto = r.body_mut().with_config().limit(2 * 1024 * 1024).read_to_string().unwrap_or_default();
    match codigo {
        200 => interpretar(&texto),
        // Todavía no hay ninguna versión publicada (o el repositorio es privado sin clave).
        404 => Ok(None),
        403 | 429 => Err("GitHub pidió esperar (demasiadas consultas); se vuelve a probar más tarde".into()),
        c => Err(format!("GitHub respondió {c}")),
    }
}

/// Lee la respuesta de `releases/latest`. `None` si la versión no trae instalador.
pub fn interpretar(json: &str) -> Result<Option<Publicada>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("respuesta no válida: {e}"))?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return Ok(None);
    }
    let tag = v["tag_name"].as_str().unwrap_or("");
    let numero = tag.trim().trim_start_matches(['v', 'V']).to_string();
    if numero.is_empty() || !numero.starts_with(|c: char| c.is_ascii_digit()) {
        return Ok(None);
    }
    let Some(a) = v["assets"].as_array().into_iter().flatten().find(|a| {
        let n = a["name"].as_str().unwrap_or("").to_lowercase();
        n.ends_with(".exe") && n.contains("instalador")
    }) else {
        return Ok(None);
    };
    Ok(Some(Publicada {
        numero,
        notas: v["body"].as_str().unwrap_or("").trim().to_string(),
        pagina: v["html_url"].as_str().unwrap_or("").to_string(),
        descarga: a["browser_download_url"].as_str().unwrap_or("").to_string(),
        descarga_api: a["url"].as_str().unwrap_or("").to_string(),
        tamano: a["size"].as_u64().unwrap_or(0),
        nombre: a["name"].as_str().unwrap_or("Relevo-Instalador.exe").to_string(),
    }))
}

/// Baja el instalador de la última versión (en segundo plano).
pub fn descargar(estado: Compartido, cfg: ActualizacionCfg, repintar: egui::Context) {
    let publicada = {
        let mut e = estado.lock().expect("actualizacion");
        let Some(p) = e.ultima.clone() else { return };
        if e.descargando.is_some() {
            return;
        }
        e.descargando = Some(None);
        e.error = None;
        e.listo = None;
        p
    };
    std::thread::spawn(move || {
        let destino = std::env::temp_dir().join(&publicada.nombre);
        let est = estado.clone();
        let rep = repintar.clone();
        let inicio = Instant::now();
        let avance = move |bajado: u64, total: u64| {
            let seg = inicio.elapsed().as_secs_f64().max(0.1);
            let mb = |b: u64| b as f64 / 1_048_576.0;
            let mut e = est.lock().expect("actualizacion");
            e.descargando = Some((total > 0).then(|| (bajado as f32 / total as f32).min(1.0)));
            e.detalle = format!("{:.0} de {:.0} MB · {:.1} MB/s", mb(bajado), mb(total), mb(bajado) / seg);
            drop(e);
            rep.request_repaint();
        };
        let r = bajar(&publicada, cfg.token.trim(), &destino, avance);
        let mut e = estado.lock().expect("actualizacion");
        e.descargando = None;
        match r {
            Ok(()) => e.listo = Some(destino),
            Err(m) => {
                let _ = std::fs::remove_file(&destino);
                e.error = Some(m);
            }
        }
        drop(e);
        repintar.request_repaint();
    });
}

fn bajar(p: &Publicada, token: &str, destino: &Path, mut avance: impl FnMut(u64, u64)) -> Result<(), String> {
    // Repositorio privado: el archivo se pide a la API con la clave.
    let pedido = if token.is_empty() || p.descarga_api.is_empty() {
        agente().get(&p.descarga)
    } else {
        agente()
            .get(&p.descarga_api)
            .header("Accept", "application/octet-stream")
            .header("Authorization", &format!("Bearer {token}"))
    };
    let r = pedido.header("User-Agent", &format!("Relevo/{VERSION}")).call().map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("GitHub respondió {} al bajar el instalador", r.status().as_u16()));
    }
    let (_, cuerpo) = r.into_parts();
    let mut lector = cuerpo.into_reader();
    let mut f = std::fs::File::create(destino).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut leido = 0u64;
    let mut ultimo = Instant::now();
    loop {
        let n = lector.read(&mut buf).map_err(|e| format!("se cortó la descarga: {e}"))?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        leido += n as u64;
        if ultimo.elapsed() > Duration::from_millis(150) {
            ultimo = Instant::now();
            avance(leido, p.tamano);
        }
    }
    f.flush().map_err(|e| e.to_string())?;
    // Debe medir exactamente lo que dice GitHub (si no, quedó cortado).
    if p.tamano > 0 && leido != p.tamano {
        return Err(format!("la descarga quedó incompleta ({leido} de {} bytes)", p.tamano));
    }
    avance(leido, p.tamano);
    Ok(())
}

/// Ejecuta el instalador en silencio. Relevo tiene que cerrarse justo después: el
/// instalador espera a que se cierre, reemplaza el programa y lo vuelve a abrir.
pub fn instalar(instalador: &Path) -> Result<(), String> {
    std::process::Command::new(instalador)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS"])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("no se pudo abrir el instalador: {e}"))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const RESPUESTA: &str = r#"{
        "tag_name": "v1.2.0", "draft": false, "prerelease": false,
        "html_url": "https://github.com/Everlck/relevo/releases/tag/v1.2.0",
        "body": "- Arreglado el monitor\n- Nuevo atajo",
        "assets": [
            {"name": "notas.txt", "browser_download_url": "https://x/notas.txt", "url": "https://api/1", "size": 10},
            {"name": "Relevo-1.2.0-Instalador.exe",
             "browser_download_url": "https://github.com/Everlck/relevo/releases/download/v1.2.0/Relevo-1.2.0-Instalador.exe",
             "url": "https://api.github.com/repos/Everlck/relevo/releases/assets/7", "size": 52000000}
        ]
    }"#;

    #[test]
    fn lee_la_ultima_version_publicada() {
        let p = interpretar(RESPUESTA).unwrap().unwrap();
        assert_eq!(p.numero, "1.2.0");
        assert_eq!(p.nombre, "Relevo-1.2.0-Instalador.exe");
        assert_eq!(p.tamano, 52_000_000);
        assert!(p.descarga.ends_with("Relevo-1.2.0-Instalador.exe"));
        assert!(p.notas.contains("monitor"));
        let mut e = Estado { ultima: Some(p), ..Default::default() };
        // 1.2.0 es más nueva que la 1.0.0 de este programa.
        assert!(e.hay_nueva() || VERSION != "1.0.0");
        e.ultima.as_mut().unwrap().numero = VERSION.to_string();
        assert!(!e.hay_nueva(), "la misma versión no es nueva");
    }

    #[test]
    fn sin_instalador_o_borrador_no_hay_version() {
        let sin = RESPUESTA.replace("Relevo-1.2.0-Instalador.exe", "Relevo.zip");
        assert_eq!(interpretar(&sin).unwrap(), None);
        let borrador = RESPUESTA.replace("\"draft\": false", "\"draft\": true");
        assert_eq!(interpretar(&borrador).unwrap(), None);
        assert!(interpretar("no es json").is_err());
    }
}
