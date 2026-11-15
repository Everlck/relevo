//! Puesta en marcha automática: encontrar CasparCG en la PC, dejar configurado
//! el OSC en `casparcg.config` y abrir `scanner.exe` y `casparcg.exe` si no
//! están abiertos. Solo actúa cuando CasparCG está en esta misma PC.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Lo que la ventana muestra sobre el arranque.
#[derive(Debug, Clone, Default)]
pub struct EstadoArranque {
    pub carpeta: Option<PathBuf>,
    /// Mensajes de lo que se hizo (para el registro).
    pub mensajes: Vec<String>,
    /// Se añadió el OSC con CasparCG ya abierto: hay que reiniciarlo para que lo use.
    pub reinicio_pendiente: bool,
    /// Algo salió mal (se muestra como aviso).
    pub problema: Option<String>,
}

pub type Compartido = Arc<Mutex<EstadoArranque>>;

/// Datos que necesita el arranque.
#[derive(Clone)]
pub struct Ajustes {
    pub host: String,
    pub puerto: u16,
    pub osc_puerto: u16,
    pub carpeta_programa: String,
    pub iniciar: bool,
    /// Canal de salida (para la salida de pantalla).
    pub canal: u16,
    /// Salida de pantalla de CasparCG: "auto", "no", "ventana" o el número de monitor.
    pub pantalla: String,
    /// Cómo se abren CasparCG y el scanner (con o sin sus ventanas de consola).
    pub como: ComoAbrir,
}

/// Cómo abrir CasparCG y su media scanner.
#[derive(Clone, Debug, Default)]
pub struct ComoAbrir {
    /// Sin ventanas de consola: lo que escriben se ve en la pestaña CasparCG de Relevo.
    pub ocultas: bool,
    /// Donde se guarda lo que escriben (con `ocultas`): `datos/consolas`.
    pub carpeta_consolas: PathBuf,
}

pub fn es_local(host: &str) -> bool {
    matches!(host.trim(), "127.0.0.1" | "localhost" | "::1" | "")
}

/// Corre en segundo plano al abrir la app.
pub fn arrancar(aj: Ajustes, estado: Compartido, avisar: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let log = |m: String| {
            tracing::info!("{m}");
            estado.lock().expect("arranque").mensajes.push(m);
            avisar();
        };
        if !aj.iniciar {
            return;
        }
        if !es_local(&aj.host) {
            log(format!("CasparCG está en otra PC ({}): no se abre ni se configura desde aquí.", aj.host));
            return;
        }
        let carpeta = if aj.carpeta_programa.trim().is_empty() {
            buscar_carpeta()
        } else {
            Some(PathBuf::from(aj.carpeta_programa.trim())).filter(|p| p.join(EXE_CASPAR).is_file())
        };
        let Some(carpeta) = carpeta else {
            let m = if aj.carpeta_programa.trim().is_empty() {
                "No encontré la carpeta de CasparCG. Indícala en config.toml (carpeta_programa).".to_string()
            } else {
                format!("No hay {EXE_CASPAR} en {}", aj.carpeta_programa)
            };
            estado.lock().expect("arranque").problema = Some(m.clone());
            log(m);
            return;
        };
        estado.lock().expect("arranque").carpeta = Some(carpeta.clone());
        log(format!("CasparCG encontrado en {}", carpeta.display()));

        let caspar_abierto = puerto_abierto(&aj.host, aj.puerto);

        // 1) OSC en casparcg.config.
        if aj.osc_puerto != 0 {
            match configurar_osc(&carpeta, aj.osc_puerto) {
                Ok(true) if caspar_abierto => {
                    estado.lock().expect("arranque").reinicio_pendiente = true;
                    log("Se activó el tiempo exacto (OSC) en casparcg.config: reinicia CasparCG cuando no haya nada al aire.".into());
                }
                Ok(true) => log("Se activó el tiempo exacto (OSC) en casparcg.config.".into()),
                Ok(false) => {}
                Err(e) => log(format!("No se pudo configurar el OSC: {e}")),
            }
        }

        // 2) Salida de pantalla (pantalla completa en el segundo monitor, si hay).
        let modo = aj.pantalla.trim().to_lowercase();
        if modo != "no" {
            let mons = monitores();
            let salida = decidir_salida(&modo, &mons);
            match aplicar_pantalla(&carpeta, aj.canal, &salida) {
                Ok(true) => {
                    let que = describir_salida(&salida);
                    if caspar_abierto {
                        estado.lock().expect("arranque").reinicio_pendiente = true;
                        log(format!("Salida de CasparCG: {que}. Reinicia CasparCG cuando no haya nada al aire."));
                    } else {
                        log(format!("Salida de CasparCG: {que}."));
                    }
                }
                Ok(false) => {}
                Err(e) => log(format!("No se pudo configurar la salida de pantalla: {e}")),
            }
        }

        // 3) Abrir lo que falte.
        // El media scanner es el que responde a CLS (la lista de videos). En 2.3 es
        // scanner.exe; desde 2.4 viene como casparcg-scanner-vX.Y.Z-win32-x64.exe.
        match buscar_scanner(&carpeta) {
            Some(scanner) => {
                let nombre = scanner.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if !proceso_abierto(&nombre) {
                    match abrir(&scanner, &aj.como, false) {
                        Ok(()) => log(format!("Se abrió el media scanner de CasparCG ({nombre}).")),
                        Err(e) => log(format!("No se pudo abrir {nombre}: {e}")),
                    }
                }
            }
            None => log(format!(
                "No encontré el media scanner de CasparCG en {}: sin él, CasparCG no puede listar los videos.",
                carpeta.display()
            )),
        }
        if !caspar_abierto && !proceso_abierto(EXE_CASPAR) {
            match abrir(&carpeta.join(EXE_CASPAR), &aj.como, true) {
                Ok(()) => log("Se abrió CasparCG Server.".into()),
                Err(e) => {
                    let m = format!("No se pudo abrir CasparCG: {e}");
                    estado.lock().expect("arranque").problema = Some(m.clone());
                    log(m);
                }
            }
        }
    });
}

/// Cierra y vuelve a abrir CasparCG (para que lea el casparcg.config nuevo).
pub fn reiniciar_caspar(carpeta: &Path, como: &ComoAbrir) -> Result<(), String> {
    cerrar_proceso(EXE_CASPAR)?;
    std::thread::sleep(Duration::from_secs(2));
    abrir(&carpeta.join(EXE_CASPAR), como, true).map_err(|e| e.to_string())
}

/// Abre CasparCG si no está abierto.
pub fn abrir_caspar(carpeta: &Path, como: &ComoAbrir) -> Result<(), String> {
    if proceso_abierto(EXE_CASPAR) {
        return Err("CasparCG ya está abierto".into());
    }
    abrir(&carpeta.join(EXE_CASPAR), como, true).map_err(|e| e.to_string())
}

/// Vigila en segundo plano la ventana de video de CasparCG (su salida de pantalla): si
/// se minimiza (por ejemplo con Windows + D, «Mostrar escritorio»), la vuelve a mostrar
/// sin quitarle el foco a lo que se esté usando. Minimizada, CasparCG la deja en negro.
pub fn vigilar_salida() {
    std::thread::spawn(|| {
        let mut pids: Vec<u32> = Vec::new();
        let mut revisado: Option<Instant> = None;
        loop {
            // La lista de procesos se renueva cada 10 s (tasklist tarda un poco).
            if revisado.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
                pids = procesos(EXE_CASPAR);
                revisado = Some(Instant::now());
            }
            if !pids.is_empty() {
                let n = restaurar_salida(&pids);
                if n > 0 {
                    tracing::info!("La salida de video de CasparCG estaba minimizada: se volvió a mostrar");
                }
            }
            std::thread::sleep(Duration::from_millis(700));
        }
    });
}

/// Vuelve a mostrar las ventanas minimizadas de CasparCG (no sus consolas). Devuelve
/// cuántas había.
pub fn restaurar_salida(pids: &[u32]) -> usize {
    #[cfg(windows)]
    {
        ventanas_win::restaurar(pids)
    }
    #[cfg(not(windows))]
    {
        let _ = pids;
        0
    }
}

#[cfg(windows)]
mod ventanas_win {
    use std::ffi::c_void;
    type Hwnd = *mut c_void;
    const SW_SHOWNOACTIVATE: i32 = 4;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(cada: unsafe extern "system" fn(Hwnd, isize) -> i32, dato: isize) -> i32;
        fn GetWindowThreadProcessId(ventana: Hwnd, pid: *mut u32) -> u32;
        fn IsIconic(ventana: Hwnd) -> i32;
        fn ShowWindow(ventana: Hwnd, como: i32) -> i32;
        fn GetClassNameW(ventana: Hwnd, nombre: *mut u16, largo: i32) -> i32;
    }

    struct Busqueda<'a> {
        pids: &'a [u32],
        encontradas: Vec<Hwnd>,
    }

    unsafe extern "system" fn cada(ventana: Hwnd, dato: isize) -> i32 {
        // SAFETY: `dato` es el puntero a la `Busqueda` de `restaurar`, vivo mientras dura
        // EnumWindows (que llama aquí en el mismo hilo).
        let b = unsafe { &mut *(dato as *mut Busqueda) };
        if unsafe { IsIconic(ventana) } == 0 {
            return 1;
        }
        let mut clase = [0u16; 64];
        let n = unsafe { GetClassNameW(ventana, clase.as_mut_ptr(), clase.len() as i32) };
        let clase = String::from_utf16_lossy(&clase[..n.max(0) as usize]);
        // Las consolas (cmd) se dejan como estén.
        if clase == "ConsoleWindowClass" || clase.starts_with("CASCADIA") || clase == "PseudoConsoleWindow" {
            return 1;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(ventana, &mut pid) };
        if b.pids.contains(&pid) {
            b.encontradas.push(ventana);
        }
        1
    }

    pub fn restaurar(pids: &[u32]) -> usize {
        let mut b = Busqueda { pids, encontradas: Vec::new() };
        // SAFETY: ver `cada`.
        unsafe { EnumWindows(cada, &mut b as *mut Busqueda as isize) };
        for v in &b.encontradas {
            unsafe { ShowWindow(*v, SW_SHOWNOACTIVATE) };
        }
        b.encontradas.len()
    }
}

/// Cierra CasparCG (todas sus copias abiertas).
pub fn cerrar_caspar() -> Result<(), String> {
    CASPAR_PROPIO.store(false, Ordering::Relaxed);
    cerrar_proceso(EXE_CASPAR)
}

/// Abre el media scanner si no está abierto; con `reiniciar`, lo cierra antes.
pub fn abrir_scanner(carpeta: &Path, como: &ComoAbrir, reiniciar: bool) -> Result<(), String> {
    let scanner = buscar_scanner(carpeta).ok_or("no está el media scanner en la carpeta de CasparCG")?;
    let nombre = scanner.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if reiniciar {
        cerrar_proceso(&nombre)?;
        std::thread::sleep(Duration::from_secs(1));
    } else if proceso_abierto(&nombre) {
        return Err("el media scanner ya está abierto".into());
    }
    abrir(&scanner, como, false).map_err(|e| e.to_string())
}

/// Cuántas copias hay abiertas de CasparCG y del scanner (para la pestaña CasparCG).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Procesos {
    pub caspar: Vec<u32>,
    pub scanner: Vec<u32>,
    pub nombre_scanner: Option<String>,
}

pub fn procesos_abiertos(carpeta: Option<&Path>) -> Procesos {
    let nombre_scanner =
        carpeta.and_then(buscar_scanner).and_then(|s| s.file_name().map(|n| n.to_string_lossy().into_owned()));
    Procesos {
        caspar: procesos(EXE_CASPAR),
        scanner: nombre_scanner.as_deref().map(procesos).unwrap_or_default(),
        nombre_scanner,
    }
}

/// Archivo donde queda lo que escribe un programa abierto sin consola.
pub fn archivo_consola(carpeta_consolas: &Path, exe: &Path) -> PathBuf {
    let base = exe.file_stem().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let base = if base.contains("scanner") { "scanner".to_string() } else { base };
    carpeta_consolas.join(format!("{base}.log"))
}

/// Carpeta donde CasparCG guarda sus registros (`<log-path>` de casparcg.config;
/// por defecto `log`), relativa a la carpeta del programa si no es absoluta.
pub fn carpeta_logs(carpeta: &Path) -> PathBuf {
    let xml = std::fs::read_to_string(carpeta.join("casparcg.config")).unwrap_or_default();
    let ruta = ruta_logs(&xml);
    let p = PathBuf::from(&ruta);
    if p.is_absolute() {
        p
    } else {
        carpeta.join(p)
    }
}

/// El valor de `<log-path>` (sin comentarios XML); `log` si no está.
pub fn ruta_logs(xml: &str) -> String {
    let mut limpio = String::new();
    let mut resto = xml;
    while let Some(i) = resto.find("<!--") {
        limpio.push_str(&resto[..i]);
        resto = match resto[i..].find("-->") {
            Some(f) => &resto[i + f + 3..],
            None => "",
        };
    }
    limpio.push_str(resto);
    limpio
        .find("<log-path>")
        .and_then(|i| {
            let desde = i + "<log-path>".len();
            limpio[desde..].find("</log-path>").map(|f| limpio[desde..desde + f].trim().to_string())
        })
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "log".into())
}

const EXE_CASPAR: &str = if cfg!(windows) { "casparcg.exe" } else { "casparcg" };
const EXE_SCANNER: &str = if cfg!(windows) { "scanner.exe" } else { "scanner" };

/// El media scanner en la carpeta de CasparCG: `scanner.exe` o, desde CasparCG 2.4,
/// `casparcg-scanner-v1.4.0-win32-x64.exe` (se toma la versión más nueva si hay varias).
pub fn buscar_scanner(carpeta: &Path) -> Option<PathBuf> {
    let fijo = carpeta.join(EXE_SCANNER);
    if fijo.is_file() {
        return Some(fijo);
    }
    let mut v: Vec<PathBuf> = std::fs::read_dir(carpeta)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            n.contains("scanner") && (n.ends_with(".exe") || !cfg!(windows) && !n.contains('.'))
        })
        .collect();
    v.sort();
    v.pop()
}

fn puerto_abierto(host: &str, puerto: u16) -> bool {
    let host = if host.trim().is_empty() { "127.0.0.1" } else { host.trim() };
    (host, puerto)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .is_some_and(|a| TcpStream::connect_timeout(&a, Duration::from_millis(400)).is_ok())
}

/// Carpetas típicas donde se descomprime CasparCG.
fn buscar_carpeta() -> Option<PathBuf> {
    let mut raices: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for letra in ['C', 'D', 'E'] {
            raices.push(PathBuf::from(format!("{letra}:\\")));
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(p) = std::env::var_os(var) {
                raices.push(PathBuf::from(p));
            }
        }
    }
    if let Some(casa) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from) {
        for sub in ["Desktop", "Downloads", "Documents", ""] {
            raices.push(casa.join(sub));
        }
    }
    buscar_en(&raices)
}

/// Busca `casparcg.exe` en las raíces y en sus subcarpetas cuyo nombre contenga
/// "caspar" (hasta dos niveles, porque el zip suele traer una carpeta dentro de otra).
pub fn buscar_en(raices: &[PathBuf]) -> Option<PathBuf> {
    for raiz in raices {
        if raiz.join(EXE_CASPAR).is_file() {
            return Some(raiz.clone());
        }
        let Ok(lector) = std::fs::read_dir(raiz) else { continue };
        let mut candidatas: Vec<PathBuf> = lector
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| e.file_name().to_string_lossy().to_lowercase().contains("caspar"))
            .map(|e| e.path())
            .collect();
        candidatas.sort();
        for c in candidatas {
            if c.join(EXE_CASPAR).is_file() {
                return Some(c);
            }
            if let Ok(sub) = std::fs::read_dir(&c) {
                for e in sub.flatten() {
                    if e.path().join(EXE_CASPAR).is_file() {
                        return Some(e.path());
                    }
                }
            }
        }
    }
    None
}

// ----------------------------------------------------------------------
// Monitores y salida de pantalla de CasparCG
// ----------------------------------------------------------------------

/// Un monitor de la PC. `numero` es el de Windows (\\.\DISPLAY2 → 2), que es el
/// `<device>` del screen consumer de CasparCG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub numero: u32,
    pub x: i32,
    pub y: i32,
    pub ancho: i32,
    pub alto: i32,
    pub principal: bool,
}

impl Monitor {
    pub fn describir(&self) -> String {
        format!(
            "Monitor {} · {}×{}{}",
            self.numero,
            self.ancho,
            self.alto,
            if self.principal { " (principal)" } else { "" }
        )
    }
}

/// Monitores conectados (en Windows, con PowerShell; tarda ~1 s, llamar en segundo plano).
/// Para pruebas se puede indicar con la variable `RELEVO_MONITORES` (mismo formato).
pub fn monitores() -> Vec<Monitor> {
    if let Ok(t) = std::env::var("RELEVO_MONITORES") {
        return parsear_monitores(&t.replace(';', "\n"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // Con SetProcessDPIAware los tamaños salen en píxeles reales (no escalados).
        // Se pasa con -EncodedCommand para que las comillas no se pierdan.
        let script = concat!(
            "Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public class PpvDpi { [DllImport(\"user32.dll\")] public static extern bool SetProcessDPIAware(); }';",
            "[PpvDpi]::SetProcessDPIAware() | Out-Null;",
            "Add-Type -AssemblyName System.Windows.Forms;",
            "foreach ($s in [System.Windows.Forms.Screen]::AllScreens) { '{0}|{1}|{2}|{3}|{4}|{5}' -f $s.DeviceName,$s.Bounds.X,$s.Bounds.Y,$s.Bounds.Width,$s.Bounds.Height,$s.Primary }"
        );
        let salida = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand"])
            .arg(base64_utf16(script))
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        match salida {
            Ok(o) => return parsear_monitores(&String::from_utf8_lossy(&o.stdout)),
            Err(e) => tracing::warn!("no se pudo leer la lista de monitores: {e}"),
        }
    }
    Vec::new()
}

/// Texto en UTF-16LE y base64 (lo que espera `powershell -EncodedCommand`).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn base64_utf16(texto: &str) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = texto.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut s = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(ABC[(n >> 18) as usize & 63] as char);
        s.push(ABC[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { ABC[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { ABC[n as usize & 63] as char } else { '=' });
    }
    s
}

/// Líneas `\\.\DISPLAY2|1920|0|1920|1080|False`.
pub fn parsear_monitores(texto: &str) -> Vec<Monitor> {
    let mut v: Vec<Monitor> = texto
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.trim().split('|').collect();
            if p.len() < 6 {
                return None;
            }
            let numero = p[0].trim_start_matches(|c: char| !c.is_ascii_digit()).parse().ok()?;
            Some(Monitor {
                numero,
                x: p[1].trim().parse().ok()?,
                y: p[2].trim().parse().ok()?,
                ancho: p[3].trim().parse().ok()?,
                alto: p[4].trim().parse().ok()?,
                principal: p[5].trim().eq_ignore_ascii_case("true"),
            })
        })
        .collect();
    v.sort_by_key(|m| m.numero);
    v
}

/// Qué hacer con la salida de pantalla de CasparCG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SalidaPantalla {
    /// No cambiar casparcg.config.
    NoTocar,
    /// Una ventana normal en el monitor principal.
    Ventana,
    /// A pantalla completa (sin bordes) en ese monitor.
    Completa(Monitor),
}

/// `modo`: "auto" (pantalla completa en el primer monitor que no es el principal, si hay
/// dos o más), "no", "ventana" o el número de monitor.
pub fn decidir_salida(modo: &str, mons: &[Monitor]) -> SalidaPantalla {
    match modo.trim().to_lowercase().as_str() {
        "no" | "" => SalidaPantalla::NoTocar,
        "ventana" => SalidaPantalla::Ventana,
        "auto" => match mons.iter().find(|m| !m.principal).filter(|_| mons.len() >= 2) {
            Some(m) => SalidaPantalla::Completa(m.clone()),
            None => SalidaPantalla::NoTocar,
        },
        n => match n.parse::<u32>().ok().and_then(|n| mons.iter().find(|m| m.numero == n)) {
            Some(m) => SalidaPantalla::Completa(m.clone()),
            // Monitor que no está conectado ahora: no se toca nada.
            None => SalidaPantalla::NoTocar,
        },
    }
}

pub fn describir_salida(s: &SalidaPantalla) -> String {
    match s {
        SalidaPantalla::NoTocar => "sin cambios".into(),
        SalidaPantalla::Ventana => "en una ventana".into(),
        SalidaPantalla::Completa(m) => {
            format!("a pantalla completa en el monitor {} ({}×{})", m.numero, m.ancho, m.alto)
        }
    }
}

/// Cambia la salida de pantalla en `casparcg.config` (con la copia de seguridad de
/// siempre). Devuelve `true` si cambió algo.
pub fn aplicar_pantalla(carpeta: &Path, canal: u16, salida: &SalidaPantalla) -> Result<bool, String> {
    if *salida == SalidaPantalla::NoTocar {
        return Ok(false);
    }
    let ruta = carpeta.join("casparcg.config");
    let texto = std::fs::read_to_string(&ruta).map_err(|e| format!("no se pudo leer {}: {e}", ruta.display()))?;
    let nuevo = configurar_pantalla(&texto, canal, salida)?;
    let Some(nuevo) = nuevo else { return Ok(false) };
    guardar_respaldo(carpeta, &texto)?;
    std::fs::write(&ruta, nuevo).map_err(|e| format!("no se pudo escribir {}: {e}", ruta.display()))?;
    Ok(true)
}

/// Devuelve el XML con el `<screen>` del canal `canal` (1 = el primero) ajustado, o
/// `None` si ya estaba así. Si el canal no tiene `<screen>`, se agrega.
///
/// CasparCG 2.3: con `windowed=false` y `borderless=true` la ventana se pone sin bordes
/// en la posición y del tamaño del monitor `<device>` (pantalla completa sin cambiar la
/// resolución). `<device>` empieza en 1.
pub fn configurar_pantalla(xml: &str, canal: u16, salida: &SalidaPantalla) -> Result<Option<String>, String> {
    let valores: Vec<(&str, Option<String>)> = match salida {
        SalidaPantalla::NoTocar => return Ok(None),
        SalidaPantalla::Ventana => vec![
            ("device", Some("1".into())),
            ("windowed", Some("true".into())),
            ("borderless", Some("false".into())),
            ("always-on-top", Some("false".into())),
        ],
        SalidaPantalla::Completa(m) => vec![
            ("device", Some(m.numero.to_string())),
            ("windowed", Some("false".into())),
            ("borderless", Some("true".into())),
            ("always-on-top", Some("true".into())),
        ],
    };
    let comentarios = rangos_comentario(xml);
    let fuera = |pos: usize| !comentarios.iter().any(|(a, b)| pos >= *a && pos < *b);
    let buscar_desde = |patron: &str, desde: usize, hasta: usize| {
        xml[desde..hasta].match_indices(patron).map(|(i, _)| i + desde).find(|i| fuera(*i))
    };
    // Etiqueta de apertura `<nombre …>` o `<nombre …/>` (con o sin espacios ni
    // atributos: `<screen />` es lo mismo que `<screen/>`), fuera de comentarios.
    let etiquetas = |nombre: &str, desde: usize, hasta: usize| -> Vec<Etiqueta> {
        let abre = format!("<{nombre}");
        let mut v = Vec::new();
        for (i, _) in xml[desde..hasta].match_indices(&abre) {
            let i = i + desde;
            let sig = xml[i + abre.len()..].chars().next();
            if !matches!(sig, Some(' ' | '\t' | '\r' | '\n' | '/' | '>')) || !fuera(i) {
                continue;
            }
            let Some(f) = xml[i..hasta].find('>') else { continue };
            let fin = i + f + 1;
            v.push(Etiqueta { ini: i, fin, sola: xml[i..fin - 1].trim_end().ends_with('/') });
        }
        v
    };

    // <channels> … el canal pedido.
    let ini_canales =
        etiquetas("channels", 0, xml.len()).first().map(|e| e.ini).ok_or("casparcg.config no tiene <channels>")?;
    let fin_canales = buscar_desde("</channels>", ini_canales, xml.len()).ok_or("falta </channels>")?;
    let mut canales = Vec::new();
    for e in etiquetas("channel", ini_canales, fin_canales) {
        if e.sola || canales.last().is_some_and(|&(_, f)| e.ini < f) {
            continue;
        }
        let f = buscar_desde("</channel>", e.fin, fin_canales).ok_or("falta </channel>")?;
        canales.push((e.ini, f));
    }
    let Some(&(ini_canal, fin_canal)) = canales.get(canal.max(1) as usize - 1) else {
        return Err(format!("casparcg.config no tiene el canal {canal}"));
    };

    // <consumers> del canal.
    let (ini_cons, fin_cons) = match etiquetas("consumers", ini_canal, fin_canal).first() {
        Some(e) if !e.sola => {
            let f = buscar_desde("</consumers>", e.fin, fin_canal).ok_or("falta </consumers>")?;
            (e.fin, f)
        }
        Some(e) => {
            let sangria = sangria_de(xml, e.ini);
            let mut s = String::from(xml);
            s.replace_range(e.ini..e.fin, &format!("<consumers>\n{sangria}</consumers>"));
            return configurar_pantalla(&s, canal, salida).map(|r| r.or(Some(s)));
        }
        None => {
            // El canal no tiene <consumers>: se agrega antes de </channel>.
            let sangria = sangria_de(xml, ini_canal);
            let mut s = String::from(xml);
            s.insert_str(fin_canal, &format!("\n{sangria}    <consumers/>\n{sangria}"));
            return configurar_pantalla(&s, canal, salida).map(|r| r.or(Some(s)));
        }
    };

    // <screen> dentro de consumers (o se agrega uno).
    let pantallas = etiquetas("screen", ini_cons, fin_cons);
    let Some(primera) = pantallas.first() else {
        // No hay salida de pantalla: se agrega al principio de <consumers>.
        let sangria = format!("{}    ", sangria_de(xml, fin_cons));
        let bloque = armar_screen("", &valores, &sangria);
        let mut s = String::from(xml);
        s.insert_str(ini_cons, &format!("\n{sangria}{bloque}"));
        return Ok(Some(s));
    };
    let (bloque_ini, bloque_fin, interior) = if primera.sola {
        (primera.ini, primera.fin, String::new())
    } else {
        let f = buscar_desde("</screen>", primera.fin, fin_cons).ok_or("falta </screen>")?;
        (primera.ini, f + "</screen>".len(), xml[primera.fin..f].to_string())
    };
    // Otras salidas de pantalla vacías en el mismo canal (`<screen />`): son una
    // copia sin configurar (una versión anterior agregaba otra en vez de usar esa) y
    // abren una segunda ventana. Se quitan con su línea.
    let sobrantes: Vec<(usize, usize)> = pantallas[1..]
        .iter()
        .filter(|e| e.sola && e.ini >= bloque_fin)
        .map(|e| {
            let ini = xml[..e.ini].rfind('\n').filter(|&n| xml[n..e.ini].trim().is_empty()).unwrap_or(e.ini);
            (ini, e.fin)
        })
        .collect();
    let sangria = sangria_de(xml, bloque_ini);
    let bloque = armar_screen(&interior, &valores, &sangria);
    if xml[bloque_ini..bloque_fin] == bloque && sobrantes.is_empty() {
        return Ok(None);
    }
    let mut s = String::from(xml);
    for (a, b) in sobrantes.iter().rev() {
        s.replace_range(*a..*b, "");
    }
    s.replace_range(bloque_ini..bloque_fin, &bloque);
    Ok(Some(s))
}

/// Una etiqueta de apertura en el XML: dónde empieza, dónde termina (después de `>`)
/// y si se cierra sola (`<x/>`, `<x />`).
struct Etiqueta {
    ini: usize,
    fin: usize,
    sola: bool,
}

/// Espacios al comienzo de la línea donde está `pos`.
fn sangria_de(xml: &str, pos: usize) -> String {
    let inicio = xml[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    xml[inicio..pos].chars().take_while(|c| c.is_whitespace()).collect()
}

/// `<screen>` con los hijos de `interior` y los `valores` puestos (los demás se conservan).
fn armar_screen(interior: &str, valores: &[(&str, Option<String>)], sangria: &str) -> String {
    let mut dentro = interior.to_string();
    let hijo = format!("{sangria}    ");
    for (etiqueta, valor) in valores {
        let abre = format!("<{etiqueta}>");
        let cierra = format!("</{etiqueta}>");
        let vacio = format!("<{etiqueta}/>");
        let existente = if let Some(i) = dentro.find(&abre) {
            dentro[i..].find(&cierra).map(|f| (i, i + f + cierra.len()))
        } else {
            dentro.find(&vacio).map(|i| (i, i + vacio.len()))
        };
        match (existente, valor) {
            (Some((a, b)), Some(v)) => dentro.replace_range(a..b, &format!("{abre}{v}{cierra}")),
            (Some((a, b)), None) => {
                // Se quita con su línea.
                let ini = dentro[..a].rfind('\n').unwrap_or(a);
                dentro.replace_range(ini..b, "");
            }
            (None, Some(v)) => {
                let fin = dentro.trim_end().len();
                dentro.insert_str(fin, &format!("\n{hijo}{abre}{v}{cierra}"));
            }
            (None, None) => {}
        }
    }
    let dentro = dentro.trim_end();
    format!("<screen>{dentro}\n{sangria}</screen>")
}

/// Guarda la configuración original de CasparCG antes del primer cambio, en
/// `casparcg.config.antes-de-relevo`. Si ya hay una copia (también con el nombre de
/// antes, `antes-de-p-pv`), no se toca: esa es la original.
fn guardar_respaldo(carpeta: &Path, texto: &str) -> Result<(), String> {
    let respaldo = carpeta.join("casparcg.config.antes-de-relevo");
    if respaldo.exists() || carpeta.join("casparcg.config.antes-de-p-pv").exists() {
        return Ok(());
    }
    std::fs::write(&respaldo, texto).map_err(|e| format!("no se pudo hacer la copia de seguridad: {e}"))
}

/// Añade el cliente OSC a `casparcg.config` si falta. Devuelve `true` si cambió algo.
/// Antes del primer cambio guarda una copia `casparcg.config.antes-de-relevo`.
pub fn configurar_osc(carpeta: &Path, puerto: u16) -> Result<bool, String> {
    let ruta = carpeta.join("casparcg.config");
    let texto = std::fs::read_to_string(&ruta).map_err(|e| format!("no se pudo leer {}: {e}", ruta.display()))?;
    let Some(nuevo) = agregar_osc(&texto, puerto) else { return Ok(false) };
    guardar_respaldo(carpeta, &texto)?;
    std::fs::write(&ruta, nuevo).map_err(|e| format!("no se pudo escribir {}: {e}", ruta.display()))?;
    Ok(true)
}

/// Devuelve el XML con el cliente OSC `127.0.0.1:puerto` añadido, o `None` si ya estaba.
/// Ignora lo que esté dentro de comentarios `<!-- -->`.
pub fn agregar_osc(xml: &str, puerto: u16) -> Option<String> {
    let comentarios = rangos_comentario(xml);
    let fuera = |pos: usize| !comentarios.iter().any(|(a, b)| pos >= *a && pos < *b);
    let buscar = |patron: &str| xml.match_indices(patron).map(|(i, _)| i).find(|i| fuera(*i));

    // ¿Ya hay un cliente con ese puerto?
    let mut desde = 0;
    while let Some(i) = xml[desde..].find("<predefined-client>").map(|i| i + desde) {
        let fin = xml[i..].find("</predefined-client>").map(|f| f + i).unwrap_or(xml.len());
        if fuera(i) && xml[i..fin].contains(&format!("<port>{puerto}</port>")) {
            return None;
        }
        desde = fin.min(xml.len().saturating_sub(1)).max(i + 1);
    }

    let cliente = format!(
        "\n            <predefined-client>\n                <address>127.0.0.1</address>\n                <port>{puerto}</port>\n            </predefined-client>"
    );
    if let Some(i) = buscar("<predefined-clients>") {
        let p = i + "<predefined-clients>".len();
        return Some(format!("{}{}{}", &xml[..p], cliente, &xml[p..]));
    }
    if let Some(i) = buscar("<predefined-clients/>") {
        let bloque = format!("<predefined-clients>{cliente}\n        </predefined-clients>");
        return Some(format!("{}{}{}", &xml[..i], bloque, &xml[i + "<predefined-clients/>".len()..]));
    }
    let bloque_clientes = format!("\n        <predefined-clients>{cliente}\n        </predefined-clients>");
    if let Some(i) = buscar("<osc>") {
        let p = i + "<osc>".len();
        return Some(format!("{}{}{}", &xml[..p], bloque_clientes, &xml[p..]));
    }
    let bloque_osc = format!("    <osc>{bloque_clientes}\n    </osc>\n");
    let i = buscar("</configuration>")?;
    Some(format!("{}{}{}", &xml[..i], bloque_osc, &xml[i..]))
}

fn rangos_comentario(xml: &str) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut desde = 0;
    while let Some(a) = xml[desde..].find("<!--").map(|a| a + desde) {
        let b = xml[a..].find("-->").map(|b| b + a + 3).unwrap_or(xml.len());
        v.push((a, b));
        desde = b;
    }
    v
}

/// Abre un programa de CasparCG.
///
/// - Consolas visibles: en su propia ventana de consola, como siempre.
/// - Consolas ocultas: sin ventana; lo que escribe va a `datos/consolas/<nombre>.log`
///   (se ve en la pestaña CasparCG). A CasparCG (`entrada = true`) se le da una
///   entrada que nunca se cierra: si su entrada se cierra, en Windows se queda dando
///   vueltas y gasta un núcleo entero. Así sigue bien aunque se cierre Relevo.
fn abrir(exe: &Path, como: &ComoAbrir, entrada: bool) -> std::io::Result<()> {
    let r = abrir_sin_marcar(exe, como, entrada);
    if r.is_ok() {
        if entrada {
            CASPAR_PROPIO.store(true, Ordering::Relaxed);
        } else {
            SCANNER_PROPIO.store(true, Ordering::Relaxed);
        }
    }
    r
}

use std::sync::atomic::{AtomicBool, Ordering};

/// CasparCG / el scanner los abrió este Relevo (o el Relevo que se reinició en este).
static CASPAR_PROPIO: AtomicBool = AtomicBool::new(false);
static SCANNER_PROPIO: AtomicBool = AtomicBool::new(false);

/// Variable con la que un Relevo que se reinicia le pasa al nuevo que CasparCG es suyo.
pub const VAR_CASPAR_PROPIO: &str = "RELEVO_CASPAR_PROPIO";

/// Al arrancar: si lo reabrió un Relevo que había abierto CasparCG, también es nuestro.
pub fn heredar_caspar_propio() {
    if std::env::var(VAR_CASPAR_PROPIO).is_ok_and(|v| v == "1") {
        CASPAR_PROPIO.store(true, Ordering::Relaxed);
        SCANNER_PROPIO.store(true, Ordering::Relaxed);
    }
}

/// ¿Este Relevo abrió CasparCG (y sigue abierto)?
pub fn caspar_es_propio() -> bool {
    CASPAR_PROPIO.load(Ordering::Relaxed)
}

/// Cierra CasparCG y su scanner, si los abrió Relevo. Devuelve qué cerró.
pub fn cerrar_lo_propio(carpeta: Option<&Path>) -> Vec<String> {
    let mut hechos = Vec::new();
    if CASPAR_PROPIO.swap(false, Ordering::Relaxed) && proceso_abierto(EXE_CASPAR) && cerrar_proceso(EXE_CASPAR).is_ok()
    {
        hechos.push("CasparCG".to_string());
    }
    if SCANNER_PROPIO.swap(false, Ordering::Relaxed) {
        if let Some(scanner) = carpeta.and_then(buscar_scanner) {
            let nombre = scanner.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if proceso_abierto(&nombre) && cerrar_proceso(&nombre).is_ok() {
                hechos.push(nombre);
            }
        }
    }
    hechos
}

fn abrir_sin_marcar(exe: &Path, como: &ComoAbrir, entrada: bool) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(exe);
    if let Some(c) = exe.parent() {
        cmd.current_dir(c);
    }
    if !como.ocultas {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
            cmd.creation_flags(CREATE_NEW_CONSOLE);
        }
        return cmd.spawn().map(|_| ());
    }
    // Sin consola: la salida a un archivo (los dos con «append», para no pisarse).
    std::fs::create_dir_all(&como.carpeta_consolas)?;
    let archivo = archivo_consola(&como.carpeta_consolas, exe);
    let ahora = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let hora = ahora
        .format(time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]:[second]"))
        .unwrap_or_default();
    std::fs::write(&archivo, format!("=== Relevo abrió {} a las {hora} (sin consola) ===\n", exe.display()))?;
    let abrir_anexar = || std::fs::OpenOptions::new().append(true).open(&archivo);
    cmd.stdout(abrir_anexar()?).stderr(abrir_anexar()?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        if entrada {
            let (lector, escritor) = entrada_que_no_se_cierra()?;
            cmd.stdin(lector);
            let r = cmd.spawn().map(|_| ());
            // CasparCG heredó su propia copia del otro extremo: el tubo sigue abierto.
            drop(escritor);
            return r;
        }
    }
    let _ = entrada;
    cmd.stdin(std::process::Stdio::null());
    cmd.spawn().map(|_| ())
}

/// Un tubo para la entrada de CasparCG que no se cierra nunca: el otro extremo se marca
/// para que CasparCG también lo herede, así que mientras CasparCG viva, el tubo vive.
#[cfg(windows)]
fn entrada_que_no_se_cierra() -> std::io::Result<(std::process::Stdio, std::os::windows::io::OwnedHandle)> {
    use std::os::windows::io::{AsRawHandle, OwnedHandle};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetHandleInformation(h: *mut core::ffi::c_void, mascara: u32, banderas: u32) -> i32;
    }
    const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;
    let (lector, escritor) = std::io::pipe()?;
    let escritor = OwnedHandle::from(escritor);
    // SAFETY: el handle es válido (es nuestro) y solo se marca como heredable.
    let ok = unsafe { SetHandleInformation(escritor.as_raw_handle(), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((std::process::Stdio::from(lector), escritor))
}

/// Números de proceso (PID) de los programas con ese nombre que están abiertos.
pub fn procesos(nombre: &str) -> Vec<u32> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // En CSV, tasklist no corta los nombres largos (en tabla los corta a 25 letras).
        std::process::Command::new("tasklist")
            .args(["/FO", "CSV", "/NH", "/FI", &format!("IMAGENAME eq {nombre}")])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|o| parsear_tasklist(&String::from_utf8_lossy(&o.stdout), nombre))
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        // pgrep -x solo compara los primeros 15 caracteres del nombre.
        let args: [&str; 2] = if nombre.len() <= 15 { ["-x", nombre] } else { ["-f", nombre] };
        std::process::Command::new("pgrep")
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.trim().parse().ok()).collect())
            .unwrap_or_default()
    }
}

/// Lee la salida de `tasklist /FO CSV /NH`: `"nombre.exe","1234","Console","1","12.345 KB"`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn parsear_tasklist(texto: &str, nombre: &str) -> Vec<u32> {
    texto
        .lines()
        .filter_map(|l| {
            let mut campos = l.split("\",\"").map(|c| c.trim().trim_matches('"'));
            let n = campos.next()?;
            let pid = campos.next()?.parse().ok()?;
            n.eq_ignore_ascii_case(nombre).then_some(pid)
        })
        .collect()
}

fn proceso_abierto(nombre: &str) -> bool {
    !procesos(nombre).is_empty()
}

#[cfg(windows)]
fn cerrar_proceso(nombre: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("taskkill")
        .args(["/IM", nombre, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| e.to_string())
        .map(|_| ())
}

#[cfg(not(windows))]
fn cerrar_proceso(nombre: &str) -> Result<(), String> {
    std::process::Command::new("pkill").args(["-x", nombre]).status().map_err(|e| e.to_string()).map(|_| ())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const SIN_OSC: &str = "<configuration>\n    <paths/>\n</configuration>\n";

    #[test]
    fn lee_la_carpeta_de_registros() {
        assert_eq!(ruta_logs("<configuration><log-path>log/</log-path></configuration>"), "log/");
        assert_eq!(ruta_logs("<!-- <log-path>x</log-path> --><paths/>"), "log");
        assert_eq!(ruta_logs("<log-path> D:\\logs </log-path>"), "D:\\logs");
    }

    #[test]
    fn agrega_bloque_osc_completo() {
        let r = agregar_osc(SIN_OSC, 6250).unwrap();
        assert!(r.contains("<osc>"));
        assert!(r.contains("<port>6250</port>"));
        assert!(r.find("<osc>").unwrap() < r.find("</configuration>").unwrap());
        // La segunda vez ya no cambia nada.
        assert!(agregar_osc(&r, 6250).is_none());
    }

    #[test]
    fn respeta_osc_y_clientes_existentes() {
        let xml = "<configuration><osc><default-port>6250</default-port><predefined-clients><predefined-client><address>10.0.0.5</address><port>7000</port></predefined-client></predefined-clients></osc></configuration>";
        let r = agregar_osc(xml, 6250).unwrap();
        assert!(r.contains("<port>7000</port>"));
        assert!(r.contains("<port>6250</port>"));
        assert_eq!(r.matches("<predefined-clients>").count(), 1);

        let xml2 = "<configuration><osc><default-port>6250</default-port></osc></configuration>";
        let r2 = agregar_osc(xml2, 6250).unwrap();
        assert_eq!(r2.matches("<osc>").count(), 1);
        assert!(r2.contains("<predefined-clients>"));
    }

    #[test]
    fn ignora_ejemplos_comentados() {
        // El casparcg.config de ejemplo trae el bloque OSC dentro de un comentario.
        let xml = "<configuration>\n<!--\n<osc>\n  <predefined-clients>\n    <predefined-client><address>127.0.0.1</address><port>6250</port></predefined-client>\n  </predefined-clients>\n</osc>\n-->\n</configuration>";
        let r = agregar_osc(xml, 6250).unwrap();
        let fin_comentario = r.find("-->").unwrap();
        let osc_real = r.rfind("<osc>").unwrap();
        assert!(osc_real > fin_comentario, "el bloque nuevo debe ir fuera del comentario");
    }

    #[test]
    fn encuentra_casparcg_dentro_de_subcarpetas() {
        let d = std::env::temp_dir().join(format!("relevo-arranque-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let destino = d.join("CasparCG_Server_2.3.3").join("server");
        std::fs::create_dir_all(&destino).unwrap();
        std::fs::create_dir_all(d.join("Otra cosa")).unwrap();
        std::fs::write(destino.join(EXE_CASPAR), b"").unwrap();
        assert_eq!(buscar_en(std::slice::from_ref(&d)), Some(destino));
        assert_eq!(buscar_en(&[d.join("Otra cosa")]), None);
    }

    #[test]
    fn configurar_osc_con_respaldo() {
        let d = std::env::temp_dir().join(format!("relevo-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("casparcg.config"), SIN_OSC).unwrap();
        assert_eq!(configurar_osc(&d, 6250), Ok(true));
        assert_eq!(std::fs::read_to_string(d.join("casparcg.config.antes-de-relevo")).unwrap(), SIN_OSC);
        assert_eq!(configurar_osc(&d, 6250), Ok(false));
    }

    #[test]
    fn hosts_locales() {
        assert!(es_local("127.0.0.1"));
        assert!(es_local("localhost"));
        assert!(!es_local("192.168.1.20"));
    }

    const CONFIG_CASPAR: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<configuration>
    <paths>
        <media-path>media/</media-path>
    </paths>
    <channels>
        <channel>
            <video-mode>1080i5994</video-mode>
            <consumers>
                <screen>
                    <device>1</device>
                    <windowed>true</windowed>
                    <name>Programa</name>
                </screen>
                <system-audio/>
            </consumers>
        </channel>
        <channel>
            <video-mode>720p5994</video-mode>
            <consumers/>
        </channel>
    </channels>
<!--
    <screen>
        <device>[0..]</device>
    </screen>
-->
</configuration>
"#;

    fn dos_monitores() -> Vec<Monitor> {
        parsear_monitores("\\\\.\\DISPLAY1|0|0|1920|1080|True\r\n\\\\.\\DISPLAY2|1920|0|1920|1080|False\r\n")
    }

    #[test]
    fn lee_monitores_y_elige_el_segundo() {
        let m = dos_monitores();
        assert_eq!(m.len(), 2);
        assert_eq!(m[1], Monitor { numero: 2, x: 1920, y: 0, ancho: 1920, alto: 1080, principal: false });
        assert_eq!(decidir_salida("auto", &m), SalidaPantalla::Completa(m[1].clone()));
        // Con un solo monitor, "auto" no toca nada.
        assert_eq!(decidir_salida("auto", &m[..1]), SalidaPantalla::NoTocar);
        assert_eq!(decidir_salida("1", &m), SalidaPantalla::Completa(m[0].clone()));
        assert_eq!(decidir_salida("3", &m), SalidaPantalla::NoTocar);
        assert_eq!(decidir_salida("ventana", &m), SalidaPantalla::Ventana);
        assert_eq!(decidir_salida("no", &m), SalidaPantalla::NoTocar);
    }

    #[test]
    fn pone_la_salida_a_pantalla_completa_en_el_monitor_2() {
        let m = dos_monitores();
        let r = configurar_pantalla(CONFIG_CASPAR, 1, &SalidaPantalla::Completa(m[1].clone())).unwrap().unwrap();
        assert!(r.contains("<device>2</device>"), "{r}");
        assert!(r.contains("<windowed>false</windowed>"));
        assert!(r.contains("<borderless>true</borderless>"));
        assert!(r.contains("<always-on-top>true</always-on-top>"));
        // Lo demás se conserva: nombre, audio, el otro canal y el comentario.
        assert!(r.contains("<name>Programa</name>"));
        assert!(r.contains("<system-audio/>"));
        assert!(r.contains("<device>[0..]</device>"));
        assert_eq!(r.matches("<screen>").count(), 2);
        // Aplicarlo otra vez no cambia nada.
        assert_eq!(configurar_pantalla(&r, 1, &SalidaPantalla::Completa(m[1].clone())).unwrap(), None);
        // Volver a ventana.
        let v = configurar_pantalla(&r, 1, &SalidaPantalla::Ventana).unwrap().unwrap();
        assert!(v.contains("<device>1</device>") && v.contains("<windowed>true</windowed>"));
        assert!(v.contains("<borderless>false</borderless>"));
    }

    #[test]
    fn agrega_la_salida_si_el_canal_no_tiene() {
        let m = dos_monitores();
        let r = configurar_pantalla(CONFIG_CASPAR, 2, &SalidaPantalla::Completa(m[1].clone())).unwrap().unwrap();
        let canal2 = &r[r.find("720p5994").unwrap()..];
        assert!(canal2.contains("<consumers>") && canal2.contains("<device>2</device>"), "{r}");
        assert!(configurar_pantalla(CONFIG_CASPAR, 3, &SalidaPantalla::Ventana).is_err());
    }

    #[test]
    fn canal_sin_consumers() {
        let xml = "<configuration>\n    <channels>\n        <channel><video-mode>1080i5994</video-mode></channel>\n    </channels>\n</configuration>\n";
        let m = dos_monitores();
        let r = configurar_pantalla(xml, 1, &SalidaPantalla::Completa(m[1].clone())).unwrap().unwrap();
        assert!(r.contains("<consumers>") && r.contains("</consumers>") && r.contains("<device>2</device>"), "{r}");
        assert_eq!(configurar_pantalla(&r, 1, &SalidaPantalla::Completa(m[1].clone())).unwrap(), None);
    }

    #[test]
    fn reconoce_screen_con_espacio_y_no_duplica_la_salida() {
        // Así viene casparcg.config de CasparCG 2.5: `<screen />`, con espacio.
        let xml = "<configuration>\n    <channels>\n        <channel>\n            <video-mode>720p5000</video-mode>\n            <consumers>\n                <screen />\n                <system-audio />\n            </consumers>\n        </channel>\n    </channels>\n</configuration>\n";
        let m = dos_monitores();
        let r = configurar_pantalla(xml, 1, &SalidaPantalla::Completa(m[1].clone())).unwrap().unwrap();
        assert_eq!(r.matches("<screen").count(), 1, "una sola salida de pantalla:\n{r}");
        assert!(r.contains("<device>2</device>") && r.contains("<system-audio />"), "{r}");
        assert_eq!(configurar_pantalla(&r, 1, &SalidaPantalla::Completa(m[1].clone())).unwrap(), None);
    }

    #[test]
    fn quita_la_salida_vacia_que_quedo_duplicada() {
        // Lo que dejaba la versión anterior: la salida configurada y, además, la vacía.
        let xml = "<configuration>\n    <channels>\n        <channel>\n            <consumers>\n                <screen>\n                    <device>2</device>\n                </screen>\n                <screen />\n                <system-audio />\n            </consumers>\n        </channel>\n    </channels>\n</configuration>\n";
        let m = dos_monitores();
        let r = configurar_pantalla(xml, 1, &SalidaPantalla::Completa(m[1].clone())).unwrap().unwrap();
        assert_eq!(r.matches("<screen").count(), 1, "{r}");
        assert!(r.contains("                <system-audio />"), "se conserva lo demás:\n{r}");
    }

    #[test]
    fn lee_tasklist_con_nombres_largos() {
        let t = "\"casparcg-scanner-v1.4.0-win32-x64.exe\",\"4120\",\"Console\",\"1\",\"45.120 KB\"\r\n\"casparcg.exe\",\"880\",\"Console\",\"1\",\"300.000 KB\"\r\n\"casparcg.exe\",\"912\",\"Console\",\"1\",\"290.000 KB\"\r\n";
        assert_eq!(parsear_tasklist(t, "casparcg-scanner-v1.4.0-win32-x64.exe"), vec![4120]);
        assert_eq!(parsear_tasklist(t, "CasparCG.exe"), vec![880, 912]);
        // Sin procesos, tasklist escribe un aviso (en el idioma de Windows).
        assert!(parsear_tasklist("INFORMACIÓN: no hay tareas ejecutándose.", "casparcg.exe").is_empty());
    }

    #[test]
    fn sin_consola_la_salida_va_a_un_archivo() {
        let d = std::env::temp_dir().join(format!("relevo-consolas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let como = ComoAbrir { ocultas: true, carpeta_consolas: d.clone() };
        assert_eq!(archivo_consola(&d, Path::new("x/casparcg-scanner-v1.4.0-win32-x64.exe")), d.join("scanner.log"));
        let exe = Path::new(if cfg!(windows) { "C:/Windows/System32/whoami.exe" } else { "/bin/echo" });
        if exe.exists() {
            abrir(exe, &como, false).unwrap();
            std::thread::sleep(Duration::from_millis(500));
            let t = std::fs::read_to_string(archivo_consola(&d, exe)).unwrap();
            assert!(t.starts_with("=== Relevo abrió"), "{t}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn base64_para_powershell() {
        // "ab" en UTF-16LE = 61 00 62 00
        assert_eq!(base64_utf16("ab"), "YQBiAA==");
        assert_eq!(base64_utf16("a"), "YQA=");
    }

    #[test]
    fn aplicar_pantalla_con_respaldo() {
        let d = std::env::temp_dir().join(format!("relevo-pantalla-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("casparcg.config"), CONFIG_CASPAR).unwrap();
        let s = SalidaPantalla::Completa(dos_monitores()[1].clone());
        assert!(aplicar_pantalla(&d, 1, &s).unwrap());
        assert!(!aplicar_pantalla(&d, 1, &s).unwrap());
        assert_eq!(std::fs::read_to_string(d.join("casparcg.config.antes-de-relevo")).unwrap(), CONFIG_CASPAR);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn encuentra_el_scanner_de_casparcg_2_5() {
        let d = std::env::temp_dir().join(format!("relevo-scanner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(buscar_scanner(&d), None);
        std::fs::write(d.join("casparcg-scanner-v1.3.0-win32-x64.exe"), b"").unwrap();
        std::fs::write(d.join("casparcg-scanner-v1.4.0-win32-x64.exe"), b"").unwrap();
        std::fs::write(d.join("casparcg.exe"), b"").unwrap();
        let s = buscar_scanner(&d).unwrap();
        assert_eq!(s.file_name().unwrap(), "casparcg-scanner-v1.4.0-win32-x64.exe");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn respeta_la_copia_original_con_el_nombre_de_antes() {
        let d = std::env::temp_dir().join(format!("relevo-respaldo-viejo-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("casparcg.config.antes-de-p-pv"), "original").unwrap();
        std::fs::write(d.join("casparcg.config"), SIN_OSC).unwrap();
        assert!(configurar_osc(&d, 6250).unwrap());
        // No se crea otra copia: la original es la que ya estaba.
        assert!(!d.join("casparcg.config.antes-de-relevo").exists());
        assert_eq!(std::fs::read_to_string(d.join("casparcg.config.antes-de-p-pv")).unwrap(), "original");
        let _ = std::fs::remove_dir_all(&d);
    }
}
