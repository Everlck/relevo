//! Relevo · App de escritorio para controlar la publicidad con CasparCG.
//!
//! Uso: `relevo [ruta/a/config.toml]` (por defecto `config.toml` en la carpeta actual o, si
//! no hay, junto al programa).

// En Windows, la versión final no abre una consola negra junto a la ventana
// (el registro va a `datos/relevo.log`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod actualizacion;
mod ajustes;
mod arranque;
mod atajos;
mod config;
mod deteccion;
mod dialogo;
mod diseno;
mod editor_pauta;
mod explorador;
mod exportar;
mod ffmpeg;
mod inicio;
mod miniaturas;
mod monitor;
mod navegacion;
mod plantillas_horario;
mod senal;
mod tandas;
mod tema;
mod usuarios_ui;
mod ventana;
mod vista_previa;

use std::path::PathBuf;

use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    // La zona horaria se lee antes de crear hilos (requisito de la librería `time`).
    let zona = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);

    arranque::heredar_caspar_propio();
    let ruta = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(ruta_config_por_defecto);
    // Tandas guardadas y datos (pautas, emisiones, usuarios) viven junto al archivo de configuración.
    let base = ruta
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // Registro: en la consola (si hay) y en datos/relevo.log.
    let filtro = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    match abrir_log(&base.join("datos")) {
        Some(archivo) => {
            use tracing_subscriber::fmt::writer::MakeWriterExt;
            let archivo = std::sync::Arc::new(archivo);
            tracing_subscriber::fmt()
                .with_env_filter(filtro)
                .with_ansi(false)
                .with_writer(std::io::stderr.and(archivo))
                .init();
        }
        None => tracing_subscriber::fmt().with_env_filter(filtro).init(),
    }

    let cfg = config::Config::cargar(&ruta)?;

    // Al reiniciar desde Ajustes: dar tiempo a que la ventana anterior se cierre
    // y suelte sus puertos.
    if std::env::var_os("RELEVO_REINICIO").is_some() {
        std::env::remove_var("RELEVO_REINICIO");
        std::thread::sleep(std::time::Duration::from_millis(2500));
    }

    // Los trabajos de red y disco corren en un runtime de tokio aparte de la ventana.
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(3).enable_all().build()?;

    let titulo = if cfg.remoto.es_remoto() {
        concat!("Relevo ", env!("CARGO_PKG_VERSION"), " · Control de publicidad (remoto)")
    } else {
        concat!("Relevo ", env!("CARGO_PKG_VERSION"), " · Control de publicidad")
    };
    let opciones = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(titulo)
            .with_inner_size([1480.0, 880.0])
            .with_min_inner_size([1000.0, 620.0])
            .with_maximized(true)
            .with_icon(std::sync::Arc::new(tema::icono_ventana()))
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "Relevo",
        opciones,
        Box::new(move |cc| {
            tema::aplicar(&cc.egui_ctx);
            Ok(Box::new(inicio::Aplicacion::new(cfg, ruta, base, zona, rt, cc.egui_ctx.clone())))
        }),
    )
    .map_err(|e| anyhow::anyhow!("no se pudo abrir la ventana: {e}"))
}

/// `config.toml` de la carpeta actual; si no hay, el que está junto al programa
/// (así funciona igual con doble clic o desde el acceso directo del instalador).
fn ruta_config_por_defecto() -> PathBuf {
    let aqui = PathBuf::from("config.toml");
    if aqui.exists() {
        return aqui;
    }
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|c| c.join("config.toml")))
        .filter(|c| c.exists())
        .unwrap_or(aqui)
}

/// Archivo de registro (se empieza de nuevo si pasa de 5 MB).
fn abrir_log(carpeta: &std::path::Path) -> Option<std::fs::File> {
    std::fs::create_dir_all(carpeta).ok()?;
    let ruta = carpeta.join("relevo.log");
    let grande = std::fs::metadata(&ruta).is_ok_and(|m| m.len() > 5 * 1024 * 1024);
    std::fs::OpenOptions::new().create(true).append(!grande).write(true).truncate(grande).open(ruta).ok()
}
