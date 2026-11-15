//! Ventanas emergentes con un mismo diseño (cabecera con ícono, cerrar, fondo
//! oscurecido) y las animaciones de la app.

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Align, Color32, CornerRadius, Id, Layout, RichText, Stroke, Vec2};

use crate::tema::*;

/// Animaciones encendidas (se cambia en Ajustes → Interfaz).
static ANIMACIONES: AtomicBool = AtomicBool::new(true);

pub fn animaciones() -> bool {
    ANIMACIONES.load(Ordering::Relaxed)
}

pub fn poner_animaciones(ctx: &egui::Context, si: bool) {
    ANIMACIONES.store(si, Ordering::Relaxed);
    ctx.all_styles_mut(|s| s.animation_time = if si { 0.16 } else { 0.0 });
}

/// Curva suave de salida (rápido al principio, lento al final).
pub fn suave(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Avance (0 → 1, ya suavizado) de algo que aparece: empieza la primera vez que se
/// llama (o si no se llamó en el cuadro anterior) y dura `seg`.
pub fn aparicion(ctx: &egui::Context, id: Id, seg: f32) -> f32 {
    if !animaciones() {
        return 1.0;
    }
    let ahora = ctx.input(|i| i.time);
    let cuadro = ctx.cumulative_frame_nr();
    let clave = id.with("aparicion");
    let (desde, ultimo) = ctx.data(|d| d.get_temp::<(f64, u64)>(clave)).unwrap_or((ahora, 0));
    let desde = if ultimo + 1 < cuadro { ahora } else { desde };
    ctx.data_mut(|d| d.insert_temp(clave, (desde, cuadro)));
    let t = ((ahora - desde) as f32 / seg.max(0.01)).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    suave(t)
}

/// Avance (0 → 1) desde un momento dado (`desde`, en segundos de `input.time`).
pub fn desde(ctx: &egui::Context, desde: f64, seg: f32) -> f32 {
    if !animaciones() {
        return 1.0;
    }
    let t = ((ctx.input(|i| i.time) - desde) as f32 / seg.max(0.01)).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    suave(t)
}

/// Mezcla de dos colores.
pub fn mezclar(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tono {
    Normal,
    Peligro,
    Aviso,
}

impl Tono {
    fn colores(self) -> (Color32, Color32) {
        match self {
            Tono::Normal => (ACENTO, ACENTO_OSCURO),
            Tono::Peligro => (AIRE, AIRE_OSCURO),
            Tono::Aviso => (AMBAR, AMBAR_OSCURO),
        }
    }
}

/// Ventana emergente (modal): oscurece lo de atrás y no deja tocarlo hasta cerrarla.
pub struct Dialogo {
    id: Id,
    icono: String,
    titulo: String,
    subtitulo: Option<String>,
    ancho: f32,
    tono: Tono,
}

pub struct RespuestaDialogo<R> {
    #[allow(dead_code)]
    pub interno: R,
    /// Se pidió cerrar (✖, Esc o clic fuera).
    pub cerrar: bool,
}

impl Dialogo {
    pub fn new(id: &str, icono: &str, titulo: impl Into<String>) -> Self {
        Dialogo {
            id: Id::unique(id),
            icono: icono.into(),
            titulo: titulo.into(),
            subtitulo: None,
            ancho: 420.0,
            tono: Tono::Normal,
        }
    }

    pub fn subtitulo(mut self, s: impl Into<String>) -> Self {
        let s = s.into();
        self.subtitulo = (!s.is_empty()).then_some(s);
        self
    }

    pub fn ancho(mut self, a: f32) -> Self {
        self.ancho = a;
        self
    }

    pub fn tono(mut self, t: Tono) -> Self {
        self.tono = t;
        self
    }

    pub fn mostrar<R>(self, ctx: &egui::Context, contenido: impl FnOnce(&mut egui::Ui) -> R) -> RespuestaDialogo<R> {
        let a = aparicion(ctx, self.id, 0.22);
        let (color, oscuro) = self.tono.colores();
        let area =
            egui::Modal::default_area(self.id).anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, (1.0 - a) * 24.0));
        let marco = egui::Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0, mezclar(BORDE, color, 0.35)))
            .corner_radius(CornerRadius::same(14))
            .shadow(egui::Shadow { offset: [0, 14], blur: 40, spread: 0, color: Color32::from_black_alpha(170) });
        let mut cerrar = false;
        let r = egui::Modal::new(self.id)
            .area(area)
            .frame(marco)
            .backdrop_color(Color32::from_black_alpha((150.0 * a) as u8))
            .show(ctx, |ui| {
                ui.multiply_opacity(a);
                ui.set_width(self.ancho);
                // Cabecera.
                egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 12, top: 14, bottom: 12 }).show(
                    ui,
                    |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(Vec2::splat(38.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 19.0, oscuro);
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                &self.icono,
                                egui::FontId::proportional(18.0),
                                color,
                            );
                            ui.add_space(4.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.label(
                                    RichText::new(&self.titulo)
                                        .size(18.0)
                                        .strong()
                                        .family(crate::tema::negrita())
                                        .color(TEXTO),
                                );
                                if let Some(s) = &self.subtitulo {
                                    ui.label(RichText::new(s).size(12.5).color(TENUE));
                                }
                            });
                            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                                let b = egui::Button::new(RichText::new("✖").size(13.0).color(TENUE))
                                    .frame(false)
                                    .min_size(Vec2::splat(26.0));
                                if ui.add(b).on_hover_text("Cerrar (Esc)").clicked() {
                                    cerrar = true;
                                }
                            });
                        });
                    },
                );
                let y = ui.cursor().top();
                let r = ui.max_rect();
                ui.painter().hline(r.x_range(), y, Stroke::new(1.0, BORDE));
                egui::Frame::new()
                    .inner_margin(egui::Margin { left: 18, right: 18, top: 14, bottom: 16 })
                    .show(ui, contenido)
                    .inner
            });
        let cerrar = cerrar || r.should_close();
        RespuestaDialogo { interno: r.inner, cerrar }
    }
}

/// Botones de abajo de un diálogo, a la derecha: "Cancelar" y el principal.
/// Devuelve `(aceptar, cancelar)`.
pub fn pie(ui: &mut egui::Ui, principal: &str, tono: Tono, habilitado: bool) -> (bool, bool) {
    let mut si = false;
    let mut no = false;
    ui.add_space(6.0);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if habilitado {
            if ui.add(boton_principal(principal, tono)).clicked() {
                si = true;
            }
        } else {
            ui.add_enabled(false, boton_secundario(principal));
        }
        if ui.add(boton_secundario("Cancelar")).clicked() {
            no = true;
        }
    });
    (si, no)
}

pub fn boton_principal(texto: &str, tono: Tono) -> egui::Button<'static> {
    let fondo = match tono {
        Tono::Normal => ACENTO,
        Tono::Peligro => Color32::from_rgb(0xC2, 0x34, 0x3A),
        Tono::Aviso => Color32::from_rgb(0xC9, 0x84, 0x12),
    };
    egui::Button::new(RichText::new(texto.to_string()).strong().family(crate::tema::negrita()).color(Color32::WHITE))
        .fill(fondo)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(110.0, 34.0))
}

pub fn boton_secundario(texto: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(texto.to_string()).color(TEXTO))
        .fill(TARJETA)
        .stroke(Stroke::new(1.0, BORDE))
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(96.0, 34.0))
}
