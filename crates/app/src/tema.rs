//! Colores y estilo de la app (tema oscuro de sala de control).

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Sense, Stroke, TextStyle};

pub const FONDO: Color32 = Color32::from_rgb(0x0B, 0x0D, 0x11);
pub const PANEL: Color32 = Color32::from_rgb(0x13, 0x16, 0x1D);
pub const TARJETA: Color32 = Color32::from_rgb(0x1A, 0x1E, 0x27);
pub const TARJETA_HOVER: Color32 = Color32::from_rgb(0x22, 0x27, 0x33);
pub const BORDE: Color32 = Color32::from_rgb(0x2A, 0x30, 0x3C);
pub const TEXTO: Color32 = Color32::from_rgb(0xE8, 0xEB, 0xF1);
pub const TENUE: Color32 = Color32::from_rgb(0x8A, 0x93, 0xA4);
pub const MUY_TENUE: Color32 = Color32::from_rgb(0x5A, 0x62, 0x72);

pub const AIRE: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
pub const AIRE_OSCURO: Color32 = Color32::from_rgb(0x4A, 0x16, 0x1A);
pub const TV: Color32 = Color32::from_rgb(0x2F, 0xB3, 0x6D);
pub const TV_OSCURO: Color32 = Color32::from_rgb(0x12, 0x3A, 0x26);
pub const AMBAR: Color32 = Color32::from_rgb(0xF5, 0xA5, 0x24);
pub const AMBAR_OSCURO: Color32 = Color32::from_rgb(0x44, 0x31, 0x0E);
pub const ACENTO: Color32 = Color32::from_rgb(0x3B, 0x82, 0xF6);
pub const ACENTO_OSCURO: Color32 = Color32::from_rgb(0x15, 0x2A, 0x4D);

/// Familia de letra en negrita (Inter SemiBold), para títulos y lo que hay que destacar.
pub fn negrita() -> FontFamily {
    thread_local! {
        static F: FontFamily = FontFamily::Name("negrita".into());
    }
    F.with(|f| f.clone())
}

/// Letra de la app: Inter (normal y seminegrita), con las de egui de reserva para los
/// íconos y símbolos.
fn poner_fuentes(ctx: &egui::Context) {
    use std::sync::Arc;
    let mut f = egui::FontDefinitions::default();
    f.font_data
        .insert("inter".into(), Arc::new(egui::FontData::from_static(include_bytes!("../recursos/Inter-Regular.otf"))));
    f.font_data.insert(
        "inter-sb".into(),
        Arc::new(egui::FontData::from_static(include_bytes!("../recursos/Inter-SemiBold.otf"))),
    );
    let reserva = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    f.families.entry(FontFamily::Proportional).or_default().insert(0, "inter".into());
    let mut negrita = vec!["inter-sb".to_string()];
    negrita.extend(reserva);
    f.families.insert(FontFamily::Name("negrita".into()), negrita);
    ctx.set_fonts(f);
}

pub fn aplicar(ctx: &egui::Context) {
    poner_fuentes(ctx);
    // El tamaño lo maneja Relevo (Ajustes › Interfaz), no Ctrl + / Ctrl −.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    let mut v = egui::Visuals::dark();
    v.panel_fill = FONDO;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(0x08, 0x0A, 0x0D);
    v.faint_bg_color = TARJETA;
    v.window_stroke = Stroke::new(1.0, BORDE);
    v.selection.bg_fill = ACENTO_OSCURO;
    v.selection.stroke = Stroke::new(1.0, ACENTO);
    v.override_text_color = Some(TEXTO);
    v.slider_trailing_fill = true;
    // Ventanas, menús y globos de ayuda: esquinas redondeadas y sombra suave.
    v.window_corner_radius = CornerRadius::same(12);
    v.window_shadow = egui::Shadow { offset: [0, 12], blur: 36, spread: 0, color: Color32::from_black_alpha(160) };
    v.popup_shadow = egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(150) };
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_highlight_topmost = false;

    let radio = CornerRadius::same(7);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = radio;
        // Sin "crecer" al pasar el mouse: los bordes quedan nítidos y nada se mueve.
        w.expansion = 0.0;
    }
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.open.weak_bg_fill = TARJETA_HOVER;
    v.widgets.open.bg_stroke = Stroke::new(1.0, ACENTO);
    v.hyperlink_color = ACENTO;
    v.warn_fg_color = AMBAR;
    v.error_fg_color = AIRE;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDE);
    v.widgets.inactive.weak_bg_fill = TARJETA;
    v.widgets.inactive.bg_fill = TARJETA;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDE);
    v.widgets.hovered.weak_bg_fill = TARJETA_HOVER;
    v.widgets.hovered.bg_fill = TARJETA_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x3A, 0x42, 0x52));
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x2A, 0x30, 0x3E);
    v.widgets.active.bg_fill = ACENTO;
    ctx.set_visuals(v);

    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, FontId::new(21.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(15.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(14.0, FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.window_margin = Margin::same(16);
        s.spacing.menu_margin = Margin::same(8);
        s.spacing.menu_spacing = 4.0;
        // Barras de desplazamiento finas, que no ocupan sitio hasta que se usan.
        s.spacing.scroll = egui::style::ScrollStyle::thin();
        s.spacing.icon_width = 16.0;
        s.spacing.combo_height = 320.0;
        s.interaction.tooltip_delay = 0.35;
        s.animation_time = 0.16;
    });
}

/// Marco de las columnas principales.
pub fn marco_columna() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, BORDE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
}

/// Marco de tarjeta interna.
pub fn marco_tarjeta() -> egui::Frame {
    egui::Frame::new()
        .fill(TARJETA)
        .stroke(Stroke::new(1.0, BORDE))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(10))
}

/// Título de sección en mayúsculas pequeñas.
pub fn titulo(ui: &mut egui::Ui, texto: &str) {
    ui.label(
        egui::RichText::new(texto)
            .size(12.0)
            .strong()
            .family(crate::tema::negrita())
            .color(TENUE)
            .extra_letter_spacing(1.2),
    );
}

/// Color estable a partir de un texto (para las miniaturas provisionales).
pub fn color_de(texto: &str) -> Color32 {
    let mut h: u32 = 2166136261;
    for b in texto.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    let paleta = [
        Color32::from_rgb(0x1E, 0x3A, 0x5F),
        Color32::from_rgb(0x3B, 0x1F, 0x4F),
        Color32::from_rgb(0x1F, 0x4A, 0x3D),
        Color32::from_rgb(0x4F, 0x2E, 0x1B),
        Color32::from_rgb(0x2B, 0x3A, 0x1E),
        Color32::from_rgb(0x4A, 0x1F, 0x2E),
        Color32::from_rgb(0x1D, 0x40, 0x4A),
    ];
    paleta[(h as usize) % paleta.len()]
}

/// `m:ss` a partir de segundos.
pub fn mmss(seg: f64) -> String {
    let s = seg.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

// ----------------------------------------------------------------------
// Piezas del panel de detalle y la vista previa
// ----------------------------------------------------------------------

/// Etiqueta pequeña con fondo (para datos sueltos: "29.97 fps", "11.6 MB").
pub fn chip(ui: &mut egui::Ui, texto: &str, color: Color32, fondo: Color32) -> egui::Response {
    chip_de(1.0, ui, texto, color, fondo)
}

/// `chip` a otra escala (`k` = 1: tamaño normal).
pub fn chip_de(k: f32, ui: &mut egui::Ui, texto: &str, color: Color32, fondo: Color32) -> egui::Response {
    // Se dibuja a mano para que mida siempre lo mismo (no se estira con la fila).
    let galley = ui.painter().layout_no_wrap(texto.to_string(), FontId::proportional(11.5 * k), color);
    let tam = galley.size() + egui::vec2(16.0, 6.0) * k;
    let (rect, resp) = ui.allocate_exact_size(tam, Sense::hover());
    ui.painter().rect_filled(rect, tam.y / 2.0, fondo);
    ui.painter().galley(rect.min + egui::vec2(8.0, 3.0) * k, galley, color);
    resp
}

/// Los tres botones para usar un video en la tanda, del mismo ancho y a lo ancho del
/// panel: "+ Añadir", "⏭ Siguiente" y "▶ Al aire" (destacado).
pub fn acciones_tanda(ui: &mut egui::Ui) -> Option<nucleo::Destino> {
    let mut r = None;
    let alto = 32.0;
    let sep = ui.spacing().item_spacing.x;
    let ancho = ((ui.available_width() - 2.0 * sep) / 3.0).max(60.0);
    ui.horizontal(|ui| {
        let b = |t: &str| {
            egui::Button::new(egui::RichText::new(t).size(13.0))
                .fill(PANEL)
                .stroke(Stroke::new(1.0, BORDE))
                .corner_radius(CornerRadius::same(7))
                .min_size(egui::Vec2::new(ancho, alto))
        };
        if ui.add(b("+  Añadir")).on_hover_text("Al final de la tanda (Enter)").clicked() {
            r = Some(nucleo::Destino::Final);
        }
        if ui.add(b("⏭  Siguiente")).on_hover_text("Justo después del que está al aire").clicked() {
            r = Some(nucleo::Destino::Siguiente);
        }
        let aire = egui::Button::new(
            egui::RichText::new("⏵  Al aire").size(13.0).strong().family(crate::tema::negrita()).color(Color32::WHITE),
        )
        .fill(Color32::from_rgb(0xB0, 0x2A, 0x31))
        .stroke(Stroke::new(1.0, AIRE))
        .corner_radius(CornerRadius::same(7))
        .min_size(egui::Vec2::new(ancho, alto));
        if ui.add(aire).on_hover_text("Ponerlo al aire ahora").clicked() {
            r = Some(nucleo::Destino::AlAire);
        }
    });
    r
}

/// Barra de tiempo: lo reproducido en color, un tirador al pasar el mouse y el tiempo
/// bajo el puntero. Devuelve `Some(segundo)` mientras se arrastra o al hacer clic.
/// `soltado` indica si se terminó de arrastrar (para recién entonces saltar).
pub fn barra_tiempo(ui: &mut egui::Ui, posicion: f64, total: f64) -> (Option<f64>, bool) {
    let alto = 18.0;
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::new(ui.available_width(), alto), Sense::click_and_drag());
    let pista = egui::Rect::from_center_size(rect.center(), egui::Vec2::new(rect.width() - 12.0, 4.0));
    let activo = resp.hovered() || resp.dragged();
    let pista = if activo { pista.expand2(egui::Vec2::new(0.0, 1.5)) } else { pista };
    let frac = if total > 0.0 { (posicion / total).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let p = ui.painter();
    p.rect_filled(pista, 3.0, BORDE);
    let mut hecho = pista;
    hecho.set_width(pista.width() * frac);
    p.rect_filled(hecho, 3.0, ACENTO);
    let x = pista.left() + pista.width() * frac;
    p.circle_filled(egui::pos2(x, pista.center().y), if activo { 7.0 } else { 5.0 }, Color32::WHITE);
    let a_segundo = |px: f32| (((px - pista.left()) / pista.width()).clamp(0.0, 1.0) as f64) * total;
    if let Some(h) = resp.hover_pos() {
        if total > 0.0 && !resp.dragged() {
            p.line_segment(
                [egui::pos2(h.x, pista.top() - 3.0), egui::pos2(h.x, pista.bottom() + 3.0)],
                Stroke::new(1.0, TENUE),
            );
            let s = a_segundo(h.x);
            resp.clone().on_hover_text_at_pointer(format!("{}:{:02}", (s / 60.0) as u64, (s % 60.0) as u64));
        }
    }
    if activo {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let pos = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked() || resp.drag_stopped());
    let soltado = resp.drag_stopped() || resp.clicked();
    (pos.map(|p| a_segundo(p.x)), soltado)
}

/// Botón redondo (reproducir/pausar).
pub fn boton_redondo(ui: &mut egui::Ui, icono: &str, diametro: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(diametro), Sense::click());
    let fondo = if resp.hovered() { Color32::from_rgb(0x5A, 0x9A, 0xFF) } else { ACENTO };
    ui.painter().circle_filled(rect.center(), diametro / 2.0, fondo);
    ui.painter().text(
        rect.center() + egui::Vec2::new(if icono == "⏵" { 1.5 } else { 0.0 }, 0.0),
        egui::Align2::CENTER_CENTER,
        icono,
        egui::FontId::proportional(diametro * 0.48),
        Color32::WHITE,
    );
    resp
}

/// Ícono de la ventana (el mismo dibujo de la cabecera): cuadrado rojo redondeado con
/// el triángulo blanco.
pub fn icono_ventana() -> egui::IconData {
    const N: usize = 64;
    let mut rgba = vec![0u8; N * N * 4];
    let radio = 14.0f32;
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Dentro del cuadrado redondeado.
            let dx = (fx - N as f32 / 2.0).abs() - (N as f32 / 2.0 - radio);
            let dy = (fy - N as f32 / 2.0).abs() - (N as f32 / 2.0 - radio);
            let dist = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
            let alfa = ((radio - dist) + 0.5).clamp(0.0, 1.0);
            // Triángulo: vértices (22,16), (48,32), (22,48).
            let dentro = fx >= 22.0 && (fy - 32.0).abs() <= (48.0 - fx) * 16.0 / 26.0;
            let (r, g, b) = if dentro { (255, 255, 255) } else { (0xD9, 0x3A, 0x40) };
            let i = (y * N + x) * 4;
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = (alfa * 255.0) as u8;
        }
    }
    egui::IconData { rgba, width: N as u32, height: N as u32 }
}

static HORA_12: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Elige cómo se muestran las horas en pantalla: 24 h (`20:30`) o 12 h (`8:30 pm`).
pub fn poner_hora_12(si: bool) {
    HORA_12.store(si, std::sync::atomic::Ordering::Relaxed);
}

/// Una hora para mostrar, en el formato elegido en Ajustes.
pub fn hora(t: time::Time) -> String {
    if HORA_12.load(std::sync::atomic::Ordering::Relaxed) {
        let (h, pm) = match t.hour() {
            0 => (12, false),
            h @ 1..=11 => (h, false),
            12 => (12, true),
            h => (h - 12, true),
        };
        format!("{h}:{:02} {}", t.minute(), if pm { "pm" } else { "am" })
    } else {
        format!("{:02}:{:02}", t.hour(), t.minute())
    }
}

/// Como [`hora`], para un texto `HH:MM` (si no se entiende, se deja igual).
pub fn hora_txt(t: &str) -> String {
    nucleo::pauta::formato::leer_hora(t).map(hora).unwrap_or_else(|| t.to_string())
}
