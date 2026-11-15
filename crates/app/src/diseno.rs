//! Tamaño de las secciones de la pantalla de Emisión (se cambia arrastrando las
//! separaciones) y arrastrar y soltar videos dentro de la ventana.
//!
//! Los tamaños se guardan por PC en `datos/diseno.json`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, Color32, CursorIcon, Rect, Sense, Stroke, Vec2};
use serde::{Deserialize, Serialize};

/// Ancho de la franja que se puede agarrar entre dos secciones.
pub const GROSOR: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Medidas {
    /// Parte del ancho para la columna de archivos (0–1).
    pub archivos: f32,
    /// Parte del ancho para la columna de la pauta (0–1). La tanda se queda con el resto.
    pub pauta: f32,
    /// Alto del monitor de "Al aire", en puntos.
    pub monitor: f32,
    /// Ancho del panel de carpetas, en puntos.
    pub arbol: Option<f32>,
}

impl Default for Medidas {
    fn default() -> Self {
        Medidas { archivos: 0.37, pauta: 0.22, monitor: 170.0, arbol: None }
    }
}

pub struct Diseno {
    pub m: Medidas,
    archivo: PathBuf,
}

impl Diseno {
    pub fn new(carpeta_local: &Path) -> Self {
        let archivo = carpeta_local.join("diseno.json");
        let m = std::fs::read_to_string(&archivo).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Diseno { m, archivo }
    }

    pub fn guardar(&self) {
        if let Ok(t) = serde_json::to_string_pretty(&self.m) {
            if let Some(c) = self.archivo.parent() {
                let _ = std::fs::create_dir_all(c);
            }
            if let Err(e) = std::fs::write(&self.archivo, t) {
                tracing::warn!("No se pudo guardar {}: {e}", self.archivo.display());
            }
        }
    }

    /// Anchos de las tres columnas (archivos, tanda, pauta) para un ancho total,
    /// dejando sitio para las dos separaciones.
    pub fn columnas(&self, total: f32) -> [f32; 3] {
        anchos_columnas(total, self.m.archivos, self.m.pauta)
    }
}

/// Mínimos de cada columna (archivos, tanda, pauta).
const MINIMOS: [f32; 3] = [320.0, 320.0, 230.0];

pub fn anchos_columnas(total: f32, archivos: f32, pauta: f32) -> [f32; 3] {
    let util = (total - 2.0 * GROSOR).max(0.0);
    let mut a = util * archivos.clamp(0.05, 0.9);
    let mut p = util * pauta.clamp(0.05, 0.9);
    // Si no entran los mínimos, se reparte en proporción a ellos.
    let suma_min: f32 = MINIMOS.iter().sum();
    if util <= suma_min {
        let f = util / suma_min;
        return [MINIMOS[0] * f, MINIMOS[1] * f, MINIMOS[2] * f];
    }
    a = a.max(MINIMOS[0]);
    p = p.max(MINIMOS[2]);
    let mut t = util - a - p;
    if t < MINIMOS[1] {
        // Falta para la tanda: se le quita a la más ancha de las otras dos.
        let falta = MINIMOS[1] - t;
        let de_a = (a - MINIMOS[0]).max(0.0);
        let de_p = (p - MINIMOS[2]).max(0.0);
        let total_sobra = (de_a + de_p).max(1.0);
        a -= falta * de_a / total_sobra;
        p -= falta * de_p / total_sobra;
        t = util - a - p;
    }
    [a, t, p]
}

/// Franja para cambiar el tamaño arrastrando. `vertical` = la línea es vertical
/// (cambia anchos). Devuelve la respuesta: `drag_delta()` dice cuánto se movió,
/// `drag_stopped()` cuándo guardar y `double_clicked()` para volver al tamaño normal.
pub fn divisor(ui: &mut egui::Ui, id: &str, rect: Rect, vertical: bool) -> egui::Response {
    let resp = ui.interact(rect, ui.scope_id().with(("divisor", id)), Sense::click_and_drag());
    let activo = resp.hovered() || resp.dragged();
    if activo {
        ui.ctx().set_cursor_icon(if vertical { CursorIcon::ResizeColumn } else { CursorIcon::ResizeRow });
    }
    let color = if resp.dragged() {
        crate::tema::ACENTO
    } else if resp.hovered() {
        Color32::from_rgb(0x4A, 0x5A, 0x78)
    } else {
        Color32::TRANSPARENT
    };
    if color != Color32::TRANSPARENT {
        let c = rect.center();
        let (a, b) = if vertical {
            (egui::pos2(c.x, rect.top() + 4.0), egui::pos2(c.x, rect.bottom() - 4.0))
        } else {
            (egui::pos2(rect.left() + 4.0, c.y), egui::pos2(rect.right() - 4.0, c.y))
        };
        ui.painter().line_segment([a, b], Stroke::new(3.0, color));
    } else {
        // Tres puntitos para que se note que se puede agarrar.
        let c = rect.center();
        for k in [-1.0, 0.0, 1.0] {
            let d = if vertical { Vec2::new(0.0, k * 6.0) } else { Vec2::new(k * 6.0, 0.0) };
            ui.painter().circle_filled(c + d, 1.3, Color32::from_rgb(0x3A, 0x44, 0x58));
        }
    }
    resp.on_hover_text("Arrastra para cambiar el tamaño · doble clic: tamaño normal")
}

/// Lo que se arrastra dentro de la ventana.
#[derive(Debug, Clone)]
pub enum Arrastre {
    /// Un archivo de esta PC (lista de archivos).
    Archivo { ruta: PathBuf, duracion: Option<Duration>, nombre: String },
    /// Un clip de la lista de CasparCG.
    Clip { clip: String, duracion: Duration },
    /// Un spot de la tanda (para cambiar el orden).
    Spot { id: u64, nombre: String },
    /// Varios archivos de esta PC elegidos juntos (van en ese orden).
    Archivos { lista: Vec<(PathBuf, Option<Duration>)>, nombre: String },
}

impl Arrastre {
    pub fn nombre(&self) -> &str {
        match self {
            Arrastre::Archivo { nombre, .. } | Arrastre::Spot { nombre, .. } | Arrastre::Archivos { nombre, .. } => {
                nombre
            }
            Arrastre::Clip { clip, .. } => clip.rsplit('/').next().unwrap_or(clip),
        }
    }
}

/// Lo que se arrastra ahora, si hay algo.
pub fn arrastrando(ctx: &egui::Context) -> Option<std::sync::Arc<Arrastre>> {
    egui::DragAndDrop::payload::<Arrastre>(ctx)
}

/// Etiqueta que sigue al puntero mientras se arrastra.
pub fn etiqueta_arrastre(ctx: &egui::Context) {
    let Some(a) = arrastrando(ctx) else { return };
    let Some(pos) = ctx.pointer_hover_pos() else { return };
    let texto = match &*a {
        Arrastre::Spot { .. } => format!("☰  {}", a.nombre()),
        _ => format!("+  {}", a.nombre()),
    };
    egui::Area::new(egui::Id::unique("etiqueta_arrastre"))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .fixed_pos(pos + Vec2::new(14.0, 10.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgb(0x1E, 0x2A, 0x44))
                .stroke(Stroke::new(1.0, crate::tema::ACENTO))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(texto).size(13.0).color(Color32::WHITE))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
        });
}

/// Dónde cae algo soltado sobre una lista: el índice antes del cual se inserta,
/// según la altura del puntero y los rectángulos de las filas.
pub fn indice_de_insercion(filas: &[Rect], y: f32) -> usize {
    filas.iter().position(|r| y < r.center().y).unwrap_or(filas.len())
}

/// Movimiento (para `Orden::Mover`) que deja el spot de la posición `de` delante
/// del que estaba en `antes_de`. `None` si queda donde estaba.
pub fn delta_mover(de: usize, antes_de: usize) -> Option<i32> {
    let destino = if antes_de > de { antes_de - 1 } else { antes_de };
    (destino != de).then(|| destino as i32 - de as i32)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn columnas_respetan_minimos_y_suman_el_total() {
        for total in [700.0, 1100.0, 1600.0, 2400.0] {
            for (a, p) in [(0.37, 0.22), (0.8, 0.1), (0.1, 0.8), (0.5, 0.5)] {
                let [x, y, z] = anchos_columnas(total, a, p);
                assert!((x + y + z + 2.0 * GROSOR - total).abs() < 0.5, "{total} {a} {p}");
                if total > 1000.0 {
                    assert!(x >= MINIMOS[0] - 0.5 && y >= MINIMOS[1] - 0.5 && z >= MINIMOS[2] - 0.5);
                }
            }
        }
    }

    #[test]
    fn soltar_entre_filas() {
        let filas: Vec<Rect> =
            (0..3).map(|i| Rect::from_min_size(egui::pos2(0.0, i as f32 * 60.0), Vec2::new(100.0, 56.0))).collect();
        assert_eq!(indice_de_insercion(&filas, 5.0), 0);
        assert_eq!(indice_de_insercion(&filas, 40.0), 1);
        assert_eq!(indice_de_insercion(&filas, 150.0), 3);
        // Arrastrar el primero debajo del tercero: baja dos.
        assert_eq!(delta_mover(0, 3), Some(2));
        // Soltarlo en su mismo lugar (arriba o abajo de sí mismo): nada.
        assert_eq!(delta_mover(1, 1), None);
        assert_eq!(delta_mover(1, 2), None);
        // El tercero arriba de todo.
        assert_eq!(delta_mover(2, 0), Some(-2));
    }
}
