//! Panel de navegación al estilo del Explorador de Windows: **Acceso rápido**
//! (carpetas ancladas) y **Este equipo** (unidades y carpetas en árbol).
//!
//! Las carpetas ancladas se guardan en `datos/accesos.json` de esta PC.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use egui::{Color32, RichText, Sense, Vec2};
use serde::{Deserialize, Serialize};

use crate::tema::*;

/// Una carpeta anclada en Acceso rápido.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anclada {
    pub nombre: String,
    pub ruta: PathBuf,
}

/// Qué pidió el operador en el panel.
pub enum AccionNav {
    Abrir(PathBuf),
    /// La lista de CasparCG de la PC principal (en una PC remota).
    CasparRemoto,
}

pub struct Navegacion {
    archivo: PathBuf,
    pub ancladas: Vec<Anclada>,
    /// Subcarpetas ya leídas de cada carpeta abierta en el árbol.
    hijos: HashMap<PathBuf, Vec<PathBuf>>,
    unidades: Vec<(String, PathBuf)>,
}

/// Icono y nombre de las carpetas conocidas de Windows.
fn conocidas() -> Vec<(&'static str, &'static str, Option<PathBuf>)> {
    vec![
        ("🖵", "Escritorio", dirs::desktop_dir()),
        ("⬇", "Descargas", dirs::download_dir()),
        ("🗐", "Documentos", dirs::document_dir()),
        ("🖼", "Imágenes", dirs::picture_dir()),
        ("🎵", "Música", dirs::audio_dir()),
        ("🎞", "Videos", dirs::video_dir()),
    ]
}

fn icono_de(ruta: &Path) -> &'static str {
    conocidas().into_iter().find(|(_, _, r)| r.as_deref() == Some(ruta)).map(|(i, _, _)| i).unwrap_or("🗀")
}

fn nombre_de(ruta: &Path) -> String {
    ruta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| ruta.display().to_string())
}

/// Unidades de la PC ("Disco local (C:)"…).
fn unidades() -> Vec<(String, PathBuf)> {
    if cfg!(windows) {
        (b'A'..=b'Z')
            .map(|l| (l as char, PathBuf::from(format!("{}:\\", l as char))))
            .filter(|(_, r)| r.is_dir())
            .map(|(l, r)| (format!("Disco local ({l}:)"), r))
            .collect()
    } else {
        let mut v = vec![("Sistema (/)".to_string(), PathBuf::from("/"))];
        if let Some(h) = dirs::home_dir() {
            v.push(("Carpeta personal".into(), h));
        }
        v
    }
}

/// Subcarpetas visibles de una carpeta, por nombre.
fn subcarpetas(dir: &Path) -> Vec<PathBuf> {
    let Ok(l) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = l
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            !(n.starts_with('.') || n.starts_with('$') || n == "System Volume Information")
        })
        .map(|e| e.path())
        .take(500)
        .collect();
    v.sort_by_key(|p| nombre_de(p).to_lowercase());
    v
}

impl Navegacion {
    pub fn new(carpeta_local: &Path) -> Self {
        let archivo = carpeta_local.join("accesos.json");
        let ancladas: Vec<Anclada> =
            match std::fs::read_to_string(&archivo).ok().and_then(|t| serde_json::from_str(&t).ok()) {
                Some(v) => v,
                None => conocidas()
                    .into_iter()
                    .filter_map(|(_, n, r)| r.filter(|r| r.is_dir()).map(|r| Anclada { nombre: n.into(), ruta: r }))
                    .collect(),
            };
        Navegacion { archivo, ancladas, hijos: HashMap::new(), unidades: unidades() }
    }

    fn guardar(&self) {
        if let Some(c) = self.archivo.parent() {
            let _ = std::fs::create_dir_all(c);
        }
        if let Ok(t) = serde_json::to_string_pretty(&self.ancladas) {
            let _ = std::fs::write(&self.archivo, t);
        }
    }

    pub fn anclada(&self, ruta: &Path) -> bool {
        self.ancladas.iter().any(|a| a.ruta == ruta)
    }

    pub fn anclar(&mut self, ruta: &Path) {
        if !self.anclada(ruta) {
            let nombre = conocidas()
                .into_iter()
                .find(|(_, _, r)| r.as_deref() == Some(ruta))
                .map(|(_, n, _)| n.to_string())
                .unwrap_or_else(|| nombre_de(ruta));
            self.ancladas.push(Anclada { nombre, ruta: ruta.to_path_buf() });
            self.guardar();
        }
    }

    pub fn desanclar(&mut self, ruta: &Path) {
        self.ancladas.retain(|a| a.ruta != ruta);
        self.guardar();
    }

    fn mover(&mut self, i: usize, delta: isize) {
        let j = i as isize + delta;
        if j >= 0 && (j as usize) < self.ancladas.len() {
            self.ancladas.swap(i, j as usize);
            self.guardar();
        }
    }

    /// Vuelve a leer unidades y subcarpetas (por ejemplo, tras conectar un USB).
    pub fn recargar(&mut self) {
        self.hijos.clear();
        self.unidades = unidades();
    }

    /// Dibuja el panel. `actual` es la carpeta abierta (para resaltarla).
    pub fn mostrar(
        &mut self,
        ui: &mut egui::Ui,
        actual: Option<&Path>,
        medios: Option<&Path>,
        remoto: bool,
        en_caspar: bool,
    ) -> Option<AccionNav> {
        let mut accion = None;
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.spacing_mut().indent = 14.0;

        // ---- Acceso rápido ----
        let id = ui.make_persistent_id("nav-rapido");
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                ui.label(
                    RichText::new("★ Acceso rápido").size(13.0).strong().family(crate::tema::negrita()).color(TENUE),
                );
            })
            .body(|ui| {
                if remoto {
                    if fila(ui, "🎬", "CasparCG (principal)", en_caspar, Some(TV), false)
                        .on_hover_text("Videos de la carpeta de CasparCG en la PC principal")
                        .clicked()
                    {
                        accion = Some(AccionNav::CasparRemoto);
                    }
                } else if let Some(m) = medios {
                    let sel = !en_caspar && actual == Some(m);
                    if fila(ui, "🎬", "CasparCG", sel, Some(TV), false).on_hover_text(m.display().to_string()).clicked()
                    {
                        accion = Some(AccionNav::Abrir(m.to_path_buf()));
                    }
                }
                let mut quitar = None;
                let mut mover = None;
                let n = self.ancladas.len();
                for (i, a) in self.ancladas.iter().enumerate() {
                    let sel = !en_caspar && actual == Some(a.ruta.as_path());
                    let existe = existe_carpeta(&a.ruta);
                    let r = fila(ui, icono_de(&a.ruta), &a.nombre, sel, (!existe).then_some(MUY_TENUE), true)
                        .on_hover_text(a.ruta.display().to_string());
                    if r.clicked() && existe {
                        accion = Some(AccionNav::Abrir(a.ruta.clone()));
                    }
                    r.context_menu(|ui| {
                        if ui.button("📌 Quitar de acceso rápido").clicked() {
                            quitar = Some(a.ruta.clone());
                            ui.close();
                        }
                        if i > 0 && ui.button("⏶ Subir").clicked() {
                            mover = Some((i, -1));
                            ui.close();
                        }
                        if i + 1 < n && ui.button("⏷ Bajar").clicked() {
                            mover = Some((i, 1));
                            ui.close();
                        }
                    });
                }
                if let Some(q) = quitar {
                    self.desanclar(&q);
                }
                if let Some((i, d)) = mover {
                    self.mover(i, d);
                }
            });

        ui.add_space(6.0);

        // ---- Este equipo ----
        let id = ui.make_persistent_id("nav-equipo");
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                ui.label(
                    RichText::new("💻 Este equipo").size(13.0).strong().family(crate::tema::negrita()).color(TENUE),
                );
            })
            .body(|ui| {
                for (nombre, ruta) in self.unidades.clone() {
                    self.nodo(ui, &ruta, &nombre, "🖴", actual, en_caspar, &mut accion);
                }
            });
        accion
    }

    /// Una carpeta del árbol, con sus subcarpetas (se leen al desplegarla).
    #[allow(clippy::too_many_arguments)]
    fn nodo(
        &mut self,
        ui: &mut egui::Ui,
        ruta: &Path,
        nombre: &str,
        icono: &str,
        actual: Option<&Path>,
        en_caspar: bool,
        accion: &mut Option<AccionNav>,
    ) {
        let id = ui.make_persistent_id(("nav", ruta));
        // Las ramas solo se despliegan cuando el usuario las abre (▸).
        let estado = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        let sel = !en_caspar && actual == Some(ruta);
        estado
            .show_header(ui, |ui| {
                let r = fila(ui, icono, nombre, sel, None, false).on_hover_text(ruta.display().to_string());
                if r.clicked() {
                    *accion = Some(AccionNav::Abrir(ruta.to_path_buf()));
                }
                let anclada = self.anclada(ruta);
                r.context_menu(|ui| {
                    let t = if anclada { "📌 Quitar de acceso rápido" } else { "📌 Anclar a acceso rápido" };
                    if ui.button(t).clicked() {
                        if anclada {
                            self.desanclar(ruta);
                        } else {
                            self.anclar(ruta);
                        }
                        ui.close();
                    }
                });
            })
            .body(|ui| {
                let hijos = self.hijos.entry(ruta.to_path_buf()).or_insert_with(|| subcarpetas(ruta)).clone();
                if hijos.is_empty() {
                    ui.label(RichText::new("(sin carpetas)").size(11.0).color(MUY_TENUE));
                }
                for h in hijos {
                    let n = nombre_de(&h);
                    self.nodo(ui, &h, &n, "🗀", actual, en_caspar, accion);
                }
            });
    }
}

/// Una fila del panel: icono + nombre, resaltada si está elegida.
fn fila(ui: &mut egui::Ui, icono: &str, nombre: &str, sel: bool, color: Option<Color32>, pin: bool) -> egui::Response {
    let ancho = ui.available_width().max(60.0);
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ancho, 22.0), Sense::click());
    let p = ui.painter();
    if sel {
        p.rect_filled(rect, 4.0, ACENTO_OSCURO);
    } else if r.hovered() {
        p.rect_filled(rect, 4.0, TARJETA_HOVER);
    }
    let c = color.unwrap_or(TEXTO);
    let color_icono = if icono == "🗀" { AMBAR } else { color.unwrap_or(ACENTO) };
    p.text(
        rect.left_center() + Vec2::new(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        icono,
        egui::FontId::proportional(13.0),
        color_icono,
    );
    let espacio_pin = if pin { 16.0 } else { 0.0 };
    let max = ((rect.width() - 26.0 - espacio_pin) / 7.0).max(4.0) as usize;
    let texto = if nombre.chars().count() > max {
        format!("{}…", nombre.chars().take(max.saturating_sub(1)).collect::<String>())
    } else {
        nombre.to_string()
    };
    p.text(
        rect.left_center() + Vec2::new(23.0, 0.0),
        egui::Align2::LEFT_CENTER,
        texto,
        egui::FontId::proportional(13.0),
        c,
    );
    if pin {
        p.text(
            rect.right_center() - Vec2::new(4.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            "📌",
            egui::FontId::proportional(10.0),
            MUY_TENUE,
        );
    }
    r
}

/// ¿Existe la carpeta? Se recuerda unos segundos: se pregunta en cada cuadro y, en una
/// unidad de red o un USB lento, mirar el disco puede tardar.
fn existe_carpeta(ruta: &std::path::Path) -> bool {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};
    thread_local! {
        static CACHE: RefCell<HashMap<PathBuf, (Instant, bool)>> = RefCell::new(HashMap::new());
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((cuando, si)) = c.get(ruta) {
            if cuando.elapsed() < Duration::from_secs(5) {
                return *si;
            }
        }
        let si = ruta.is_dir();
        c.insert(ruta.to_path_buf(), (Instant::now(), si));
        si
    })
}
