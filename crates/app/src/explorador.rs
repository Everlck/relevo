//! Explorador de archivos de la PC: carpetas, videos y búsqueda.
//! Todo lo que toca el disco se hace en segundo plano para no congelar la ventana.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nucleo::archivos;

const MAX_RESULTADOS: usize = 400;
const MAX_PROFUNDIDAD: usize = 8;

#[derive(Debug, Clone)]
pub struct Entrada {
    pub ruta: PathBuf,
    pub nombre: String,
    pub es_carpeta: bool,
    pub tamano: u64,
    pub duracion: Option<Duration>,
    /// Fecha de modificación (para ordenar por «más recientes»).
    pub modificado: Option<std::time::SystemTime>,
}

/// Cómo se ordena la lista (las carpetas siempre van primero).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Orden {
    #[default]
    Nombre,
    Recientes,
    Duracion,
    Tamano,
}

impl Orden {
    pub const TODOS: [Orden; 4] = [Orden::Nombre, Orden::Recientes, Orden::Duracion, Orden::Tamano];

    pub fn nombre(self) -> &'static str {
        match self {
            Orden::Nombre => "Nombre",
            Orden::Recientes => "Más recientes",
            Orden::Duracion => "Más largos",
            Orden::Tamano => "Más pesados",
        }
    }
}

/// La lista en ese orden: carpetas primero (por nombre) y luego los videos.
pub fn ordenar(entradas: &[Entrada], orden: Orden) -> Vec<Entrada> {
    let mut v = entradas.to_vec();
    let nombre = |e: &Entrada| e.nombre.to_lowercase();
    v.sort_by(|a, b| {
        b.es_carpeta.cmp(&a.es_carpeta).then_with(|| {
            let por_nombre = nombre(a).cmp(&nombre(b));
            if a.es_carpeta {
                return por_nombre;
            }
            match orden {
                Orden::Nombre => por_nombre,
                Orden::Recientes => b.modificado.cmp(&a.modificado).then(por_nombre),
                Orden::Duracion => b.duracion.cmp(&a.duracion).then(por_nombre),
                Orden::Tamano => b.tamano.cmp(&a.tamano).then(por_nombre),
            }
        })
    });
    v
}

/// Las carpetas de una ruta, de la raíz hasta ella (para las migas de pan):
/// `C:\Videos\Clientes` → `C:`, `Videos`, `Clientes`.
pub fn migas(ruta: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = ruta
        .ancestors()
        .map(|a| {
            let nombre = a.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| {
                // La raíz: «C:» en Windows, «/» en Linux.
                let t = a.display().to_string();
                let t = t.trim_end_matches(['\\', '/']).to_string();
                if t.is_empty() {
                    "/".into()
                } else {
                    t
                }
            });
            (nombre, a.to_path_buf())
        })
        .filter(|(n, _)| !n.is_empty())
        .collect();
    v.reverse();
    v
}

#[derive(Debug, Clone)]
pub enum Listado {
    Cargando,
    Listo(Arc<Vec<Entrada>>),
    Error(String),
}

#[derive(Debug, Clone)]
pub enum Busqueda {
    Inactiva,
    Buscando { encontrados: Arc<Vec<Entrada>> },
    Lista { encontrados: Arc<Vec<Entrada>>, cortada: bool },
}

pub struct Explorador {
    pub carpeta: Option<PathBuf>,
    pub texto_ruta: String,
    pub texto_busqueda: String,
    pub seleccion: Option<PathBuf>,
    atras: Vec<PathBuf>,
    adelante: Vec<PathBuf>,
    /// Carpeta de la que se vino al subir o volver (se resalta en la lista).
    pub resaltar: Option<PathBuf>,
    listado: Arc<Mutex<Listado>>,
    busqueda: Arc<Mutex<Busqueda>>,
    /// Sube con cada listado/búsqueda nueva: las tareas viejas se dan por canceladas.
    generacion: Arc<AtomicU64>,
    busqueda_pendiente: Option<Instant>,
    busqueda_hecha: String,
    rt: tokio::runtime::Handle,
    repintar: egui::Context,
}

impl Explorador {
    pub fn new(rt: tokio::runtime::Handle, repintar: egui::Context) -> Self {
        Explorador {
            carpeta: None,
            texto_ruta: String::new(),
            texto_busqueda: String::new(),
            seleccion: None,
            atras: Vec::new(),
            adelante: Vec::new(),
            resaltar: None,
            listado: Arc::new(Mutex::new(Listado::Cargando)),
            busqueda: Arc::new(Mutex::new(Busqueda::Inactiva)),
            generacion: Arc::new(AtomicU64::new(0)),
            busqueda_pendiente: None,
            busqueda_hecha: String::new(),
            rt,
            repintar,
        }
    }

    pub fn listado(&self) -> Listado {
        self.listado.lock().expect("listado").clone()
    }

    pub fn busqueda(&self) -> Busqueda {
        self.busqueda.lock().expect("busqueda").clone()
    }

    /// Abre una carpeta (guardando la anterior para el botón "atrás").
    pub fn abrir(&mut self, carpeta: PathBuf) {
        if let Some(actual) = self.carpeta.take() {
            if actual != carpeta {
                self.atras.push(actual);
                if self.atras.len() > 50 {
                    self.atras.remove(0);
                }
                self.adelante.clear();
            }
        }
        self.resaltar = None;
        self.ir(carpeta);
    }

    pub fn puede_atras(&self) -> bool {
        !self.atras.is_empty()
    }

    pub fn puede_adelante(&self) -> bool {
        !self.adelante.is_empty()
    }

    pub fn volver_atras(&mut self) {
        if let Some(c) = self.atras.pop() {
            if let Some(actual) = self.carpeta.clone() {
                self.adelante.push(actual.clone());
                self.resaltar = Some(actual);
            }
            self.ir(c);
        }
    }

    pub fn ir_adelante(&mut self) {
        if let Some(c) = self.adelante.pop() {
            if let Some(actual) = self.carpeta.clone() {
                self.atras.push(actual.clone());
                self.resaltar = Some(actual);
            }
            self.ir(c);
        }
    }

    pub fn subir(&mut self) {
        if let Some(actual) = self.carpeta.clone() {
            if let Some(padre) = actual.parent().map(Path::to_path_buf) {
                self.abrir(padre);
                self.resaltar = Some(actual);
            }
        }
    }

    pub fn recargar(&mut self) {
        if let Some(c) = self.carpeta.clone() {
            self.ir(c);
        }
    }

    fn ir(&mut self, carpeta: PathBuf) {
        self.texto_ruta = carpeta.display().to_string();
        self.carpeta = Some(carpeta.clone());
        self.texto_busqueda.clear();
        self.busqueda_hecha.clear();
        self.busqueda_pendiente = None;
        *self.busqueda.lock().expect("busqueda") = Busqueda::Inactiva;
        *self.listado.lock().expect("listado") = Listado::Cargando;

        let gen = self.generacion.fetch_add(1, Ordering::SeqCst) + 1;
        let (destino, generacion, ctx) = (self.listado.clone(), self.generacion.clone(), self.repintar.clone());
        self.rt.spawn_blocking(move || {
            let r = listar(&carpeta);
            if generacion.load(Ordering::SeqCst) == gen {
                *destino.lock().expect("listado") = match r {
                    Ok(v) => Listado::Listo(Arc::new(v)),
                    Err(e) => Listado::Error(e),
                };
                ctx.request_repaint();
            }
        });
    }

    /// Avisar que el texto de búsqueda cambió (la búsqueda arranca tras una pausa).
    pub fn busqueda_cambio(&mut self) {
        self.busqueda_pendiente = Some(Instant::now());
        if self.texto_busqueda.trim().is_empty() {
            self.generacion.fetch_add(1, Ordering::SeqCst);
            self.busqueda_hecha.clear();
            *self.busqueda.lock().expect("busqueda") = Busqueda::Inactiva;
        }
    }

    /// Lanza la búsqueda pendiente si ya pasó la pausa (o si `ya` es verdadero).
    pub fn tal_vez_buscar(&mut self, ya: bool) {
        let texto = self.texto_busqueda.trim().to_lowercase();
        let Some(desde) = self.busqueda_pendiente else {
            return;
        };
        if texto.is_empty() || texto == self.busqueda_hecha {
            self.busqueda_pendiente = None;
            return;
        }
        if !ya && desde.elapsed() < Duration::from_millis(350) {
            self.repintar.request_repaint_after(Duration::from_millis(120));
            return;
        }
        let Some(raiz) = self.carpeta.clone() else {
            return;
        };
        self.busqueda_pendiente = None;
        self.busqueda_hecha = texto.clone();

        let gen = self.generacion.fetch_add(1, Ordering::SeqCst) + 1;
        let (destino, generacion, ctx) = (self.busqueda.clone(), self.generacion.clone(), self.repintar.clone());
        *destino.lock().expect("busqueda") = Busqueda::Buscando { encontrados: Arc::new(Vec::new()) };
        self.rt.spawn_blocking(move || {
            let mut encontrados = Vec::new();
            let mut cortada = false;
            let vigente = || generacion.load(Ordering::SeqCst) == gen;
            buscar(&raiz, &texto, 0, &mut encontrados, &mut cortada, &vigente, &mut |parcial| {
                if vigente() {
                    *destino.lock().expect("busqueda") = Busqueda::Buscando { encontrados: Arc::new(parcial.to_vec()) };
                    ctx.request_repaint();
                }
            });
            if vigente() {
                encontrados.sort_by_key(|e| e.nombre.to_lowercase());
                *destino.lock().expect("busqueda") = Busqueda::Lista { encontrados: Arc::new(encontrados), cortada };
                ctx.request_repaint();
            }
        });
    }
}

/// Ubicaciones rápidas: carpeta de CasparCG, carpetas del usuario y unidades.
/// Carpetas y videos de una carpeta (carpetas primero, por nombre).
pub fn listar(carpeta: &Path) -> Result<Vec<Entrada>, String> {
    let lector = std::fs::read_dir(carpeta).map_err(|e| format!("No se puede abrir {}: {e}", carpeta.display()))?;
    let mut v = Vec::new();
    for e in lector.flatten() {
        let nombre = e.file_name().to_string_lossy().into_owned();
        if oculto(&nombre) {
            continue;
        }
        let ruta = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            v.push(Entrada {
                ruta,
                nombre,
                es_carpeta: true,
                tamano: 0,
                duracion: None,
                modificado: meta.modified().ok(),
            });
        } else if archivos::es_video(&ruta) {
            let duracion = archivos::duracion_video(&ruta);
            let modificado = meta.modified().ok();
            v.push(Entrada { ruta, nombre, es_carpeta: false, tamano: meta.len(), duracion, modificado });
        }
    }
    v.sort_by(|a, b| {
        b.es_carpeta.cmp(&a.es_carpeta).then_with(|| a.nombre.to_lowercase().cmp(&b.nombre.to_lowercase()))
    });
    Ok(v)
}

fn oculto(nombre: &str) -> bool {
    nombre.starts_with('.') || nombre.starts_with('$') || nombre.ends_with(".copiando")
}

fn saltar_en_busqueda(nombre: &str) -> bool {
    const SISTEMA: &[&str] = &[
        "Windows",
        "Program Files",
        "Program Files (x86)",
        "ProgramData",
        "AppData",
        "System Volume Information",
        "node_modules",
        "target",
    ];
    oculto(nombre) || SISTEMA.iter().any(|s| s.eq_ignore_ascii_case(nombre))
}

fn buscar(
    carpeta: &Path,
    texto: &str,
    nivel: usize,
    salida: &mut Vec<Entrada>,
    cortada: &mut bool,
    vigente: &dyn Fn() -> bool,
    avance: &mut dyn FnMut(&[Entrada]),
) {
    if nivel > MAX_PROFUNDIDAD || *cortada || !vigente() {
        return;
    }
    let Ok(lector) = std::fs::read_dir(carpeta) else {
        return;
    };
    let mut subcarpetas = Vec::new();
    let antes = salida.len();
    for e in lector.flatten() {
        let nombre = e.file_name().to_string_lossy().into_owned();
        let Ok(tipo) = e.file_type() else { continue };
        if tipo.is_dir() {
            if !saltar_en_busqueda(&nombre) {
                subcarpetas.push(e.path());
            }
            continue;
        }
        let ruta = e.path();
        if archivos::es_video(&ruta) && nombre.to_lowercase().contains(texto) {
            let meta = e.metadata().ok();
            salida.push(Entrada {
                duracion: archivos::duracion_video(&ruta),
                ruta,
                nombre,
                es_carpeta: false,
                tamano: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                modificado: meta.and_then(|m| m.modified().ok()),
            });
            if salida.len() >= MAX_RESULTADOS {
                *cortada = true;
                return;
            }
        }
    }
    if salida.len() > antes {
        avance(salida);
    }
    for s in subcarpetas {
        buscar(&s, texto, nivel + 1, salida, cortada, vigente, avance);
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn lista_carpetas_y_solo_videos() {
        let d = std::env::temp_dir().join(format!("relevo-explorador-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Clientes/Farmacia")).unwrap();
        std::fs::create_dir_all(d.join(".oculta")).unwrap();
        std::fs::write(d.join("spot B.mp4"), b"x").unwrap();
        std::fs::write(d.join("Spot a.MOV"), b"x").unwrap();
        std::fs::write(d.join("notas.txt"), b"x").unwrap();
        std::fs::write(d.join("Clientes/Farmacia/promo farmacia.mp4"), b"x").unwrap();

        let v = listar(&d).unwrap();
        let nombres: Vec<&str> = v.iter().map(|e| e.nombre.as_str()).collect();
        assert_eq!(nombres, vec!["Clientes", "Spot a.MOV", "spot B.mp4"]);

        let mut r = Vec::new();
        let mut cortada = false;
        buscar(&d, "farmacia", 0, &mut r, &mut cortada, &|| true, &mut |_| {});
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].nombre, "promo farmacia.mp4");
        assert!(!cortada);
    }

    fn video(nombre: &str, seg: u64, tam: u64, hace: u64) -> Entrada {
        Entrada {
            ruta: PathBuf::from(nombre),
            nombre: nombre.into(),
            es_carpeta: false,
            tamano: tam,
            duracion: Some(Duration::from_secs(seg)),
            modificado: Some(std::time::UNIX_EPOCH + Duration::from_secs(1_000_000 - hace)),
        }
    }

    #[test]
    fn ordena_con_las_carpetas_primero() {
        let carpeta = Entrada {
            ruta: "Zeta".into(),
            nombre: "Zeta".into(),
            es_carpeta: true,
            tamano: 0,
            duracion: None,
            modificado: None,
        };
        let v = vec![video("b", 10, 5, 50), carpeta, video("a", 30, 1, 100), video("c", 20, 9, 1)];
        let n = |o| ordenar(&v, o).iter().map(|e| e.nombre.clone()).collect::<Vec<_>>().join(",");
        assert_eq!(n(Orden::Nombre), "Zeta,a,b,c");
        assert_eq!(n(Orden::Recientes), "Zeta,c,b,a");
        assert_eq!(n(Orden::Duracion), "Zeta,a,c,b");
        assert_eq!(n(Orden::Tamano), "Zeta,c,b,a");
    }

    #[test]
    fn migas_de_una_ruta() {
        let m = migas(Path::new("/tmp/caspar/media"));
        let nombres: Vec<&str> = m.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(nombres, ["/", "tmp", "caspar", "media"]);
        assert_eq!(m[2].1, PathBuf::from("/tmp/caspar"));
    }

    #[test]
    fn atras_adelante_y_subir() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut x = Explorador::new(rt.handle().clone(), egui::Context::default());
        let (a, b) = (PathBuf::from("/tmp"), PathBuf::from("/tmp/uno"));
        x.abrir(a.clone());
        x.abrir(b.clone());
        x.volver_atras();
        assert_eq!(x.carpeta.as_ref(), Some(&a));
        assert_eq!(x.resaltar.as_ref(), Some(&b));
        assert!(x.puede_adelante());
        x.ir_adelante();
        assert_eq!(x.carpeta.as_ref(), Some(&b));
        x.subir();
        assert_eq!(x.carpeta.as_ref(), Some(&a));
        assert_eq!(x.resaltar.as_ref(), Some(&b));
        // Abrir otra carpeta borra el «adelante».
        x.volver_atras();
        x.abrir(PathBuf::from("/"));
        assert!(!x.puede_adelante());
    }

    #[test]
    fn carpeta_que_no_existe() {
        assert!(listar(Path::new("/no/existe/relevo")).is_err());
    }
}
