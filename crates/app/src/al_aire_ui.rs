//! Sección «Al aire» (arriba): monitor de programa, vúmetros, qué sale, cuánto falta,
//! lo que sigue y los controles de la tanda.

use super::*;

/// Escala de los vúmetros (dBFS).
const DB_MIN: f32 = -60.0;
/// Cuánto baja el nivel mostrado por segundo (como un vúmetro de picos).
const CAIDA_DB_POR_SEG: f32 = 24.0;
/// Cuánto se mantiene la marca del pico máximo.
const RETENCION_PICO: f64 = 1.5;

/// Estado de un canal del vúmetro (lo que se muestra, con su caída suave).
#[derive(Clone, Copy)]
pub(super) struct CanalVu {
    nivel: f32,
    pico: f32,
    pico_en: f64,
}

impl Default for CanalVu {
    fn default() -> Self {
        CanalVu { nivel: DB_MIN, pico: DB_MIN, pico_en: 0.0 }
    }
}

impl Ventana {
    /// Alto del monitor de Al aire. Fuera de Emisión va más bajo, para dejarle sitio a
    /// esa pantalla.
    pub(super) fn alto_monitor(&self) -> f32 {
        if self.pantalla == Pantalla::Emision {
            self.diseno.m.monitor
        } else {
            self.diseno.m.monitor.min(ALTO_DATOS * 0.8)
        }
    }
}

/// Alto de la parte de tiempos y botones a tamaño normal (escala 1).
pub(super) const ALTO_DATOS: f32 = 204.0;

/// Escala de los tiempos y botones para que midan lo mismo que el monitor de video:
/// al achicar o agrandar la sección, se achica o agranda todo junto.
pub(super) fn escala_para(alto_monitor: f32) -> f32 {
    (alto_monitor / ALTO_DATOS).clamp(0.55, 1.5)
}

/// Tope de la escala para que la fila de botones quepa a lo ancho (a escala 1 mide
/// unos 720 px; con «VOLVER A TV», 200 más).
pub(super) fn escala_por_ancho(ancho: f32, volver: bool) -> f32 {
    let necesario = if volver { 920.0 } else { 720.0 };
    (ancho / necesario).clamp(0.55, 1.5)
}

fn a_db(x: f32) -> f32 {
    if x <= 0.000_001 {
        DB_MIN
    } else {
        (20.0 * x.log10()).max(DB_MIN)
    }
}

/// Altura (0–1) en la escala del vúmetro para un nivel en dB.
fn fraccion(db: f32) -> f32 {
    ((db - DB_MIN) / -DB_MIN).clamp(0.0, 1.0)
}

fn color_db(db: f32) -> Color32 {
    if db >= -6.0 {
        AIRE
    } else if db >= -18.0 {
        AMBAR
    } else {
        TV
    }
}

impl Ventana {
    pub(super) fn al_aire(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        // El borde cambia de color suavemente al salir al aire, pausar o volver a TV,
        // y "respira" mientras hay publicidad al aire.
        let ctx = ui.ctx().clone();
        let t = if dialogo::animaciones() { 0.35 } else { 0.0 };
        let k_aire = ctx.animate_bool_with_time(egui::Id::unique("borde-aire"), e.fase == FaseVista::AlAire, t);
        let k_pausa = ctx.animate_bool_with_time(egui::Id::unique("borde-pausa"), e.fase == FaseVista::Pausado, t);
        let mut borde = dialogo::mezclar(dialogo::mezclar(BORDE, AMBAR, k_pausa), AIRE, k_aire);
        if e.fase == FaseVista::AlAire && dialogo::animaciones() {
            let pulso = (ctx.input(|i| i.time) * 2.4).sin() as f32 * 0.5 + 0.5;
            borde = dialogo::mezclar(borde, Color32::from_rgb(0x8A, 0x22, 0x28), pulso * 0.55);
        }
        let grosor = 1.0 + (k_aire + k_pausa).min(1.0);
        // El borde se pinta encima, sin ocupar sitio: aunque engrose al salir al aire,
        // el bloque mide siempre lo mismo y lo de abajo no se mueve.
        let r = egui::Frame::new()
            .fill(PANEL)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                // Tres partes: PROGRAMA (lo que sale al aire) | tiempos y botones | PREVIEW.
                let alto = self.alto_monitor();
                self.escala_aire = escala_para(alto);
                ui.horizontal_top(|ui| {
                    self.monitor_programa(ui, e);
                    ui.add_space(6.0);
                    self.vumetros(ui, alto);
                    ui.add_space(14.0);
                    if self.cfg.en_transmision() {
                        // Transmisión: PROGRAMA | tiempos y botones | PREVIEW.
                        let ancho_preview = alto * 16.0 / 9.0;
                        let medio = (ui.available_width() - ancho_preview - 14.0).max(200.0);
                        // Si no cabe a lo ancho, también se achica.
                        self.escala_aire =
                            self.escala_aire.min(escala_por_ancho(medio, self.cfg.interfaz.boton_volver_a_tv));
                        ui.allocate_ui_with_layout(Vec2::new(medio, alto), Layout::top_down(Align::Min), |ui| {
                            ui.set_max_width(medio);
                            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                            self.datos_al_aire(ui, e);
                        });
                        ui.add_space(14.0);
                        self.monitor_preview(ui, e, alto);
                    } else {
                        // Publicidad: un solo monitor.
                        let ancho = ui.available_width();
                        self.escala_aire =
                            self.escala_aire.min(escala_por_ancho(ancho, self.cfg.interfaz.boton_volver_a_tv));
                        // Ancho fijo: nada de la línea de tiempo se sale por la derecha.
                        ui.allocate_ui_with_layout(Vec2::new(ancho, alto), Layout::top_down(Align::Min), |ui| {
                            ui.set_max_width(ancho);
                            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                            self.datos_al_aire(ui, e);
                        });
                    }
                });
            });
        ui.painter().rect_stroke(r.response.rect, 10.0, Stroke::new(grosor, borde), StrokeKind::Inside);
    }

    /// Vúmetros de la salida de CasparCG (lo que de verdad sale al aire).
    fn vumetros(&mut self, ui: &mut egui::Ui, alto: f32) {
        let salida = self.ctl.salida();
        let ahora = ui.ctx().input(|i| i.time);
        let dt = ui.ctx().input(|i| i.stable_dt).min(0.1);
        let vivo = salida.as_ref().and_then(|s| s.cuando).is_some_and(|c| c.elapsed() < Duration::from_secs(1));
        let picos: Vec<f32> = salida.as_ref().filter(|_| vivo).map(|s| s.picos.clone()).unwrap_or_default();
        // Se muestran hasta 2 canales (L/R); con más, los dos primeros.
        let n = picos.len().clamp(2, 2);
        self.vu.resize(n, CanalVu::default());
        for (i, c) in self.vu.iter_mut().enumerate() {
            let db = picos.get(i).copied().map(a_db).unwrap_or(DB_MIN);
            c.nivel = if db > c.nivel { db } else { (c.nivel - CAIDA_DB_POR_SEG * dt).max(db) };
            if db >= c.pico || ahora - c.pico_en > RETENCION_PICO {
                c.pico = db.max(if ahora - c.pico_en > RETENCION_PICO { c.nivel } else { c.pico });
                c.pico_en = ahora;
            }
        }
        // Solo se redibuja seguido mientras suena algo (o las barras están bajando).
        if picos.iter().any(|p| *p > 0.001) || self.vu.iter().any(|c| c.nivel > DB_MIN) {
            ui.ctx().request_repaint_after(Duration::from_millis(40));
        }

        let ancho_barra = 9.0;
        let escala = 22.0;
        let ancho = escala + n as f32 * (ancho_barra + 3.0) + 2.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ancho, alto), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 6.0, Color32::from_rgb(0x0C, 0x0E, 0x13));
        let zona = egui::Rect::from_min_max(rect.min + Vec2::new(4.0, 8.0), rect.max - Vec2::new(4.0, 18.0));
        // Escala.
        for db in [0.0f32, -6.0, -12.0, -18.0, -30.0, -45.0] {
            let y = zona.bottom() - zona.height() * fraccion(db);
            p.text(
                egui::pos2(zona.left() + escala - 4.0, y),
                Align2::RIGHT_CENTER,
                format!("{}", db.abs() as i32),
                FontId::proportional(8.5),
                MUY_TENUE,
            );
            p.line_segment(
                [egui::pos2(zona.left() + escala - 2.0, y), egui::pos2(zona.right(), y)],
                Stroke::new(0.5, Color32::from_white_alpha(14)),
            );
        }
        // Barras.
        for (i, c) in self.vu.iter().enumerate() {
            let x = zona.left() + escala + i as f32 * (ancho_barra + 3.0);
            let barra = egui::Rect::from_min_max(egui::pos2(x, zona.top()), egui::pos2(x + ancho_barra, zona.bottom()));
            p.rect_filled(barra, 2.0, Color32::from_rgb(0x16, 0x1A, 0x22));
            // Por tramos de color (verde / ámbar / rojo), en segmentos como un medidor.
            let tope = fraccion(c.nivel);
            let segmentos = ((barra.height() / 3.0) as usize).max(10);
            for k in 0..segmentos {
                let f0 = k as f32 / segmentos as f32;
                if f0 >= tope {
                    break;
                }
                let db = DB_MIN + f0 * -DB_MIN;
                let y1 = barra.bottom() - barra.height() * f0;
                let y0 = barra.bottom() - barra.height() * ((k + 1) as f32 / segmentos as f32);
                let r = egui::Rect::from_min_max(egui::pos2(barra.left(), y0 + 0.6), egui::pos2(barra.right(), y1));
                p.rect_filled(r, 0.0, color_db(db));
            }
            if c.pico > DB_MIN + 1.0 {
                let y = barra.bottom() - barra.height() * fraccion(c.pico);
                p.line_segment(
                    [egui::pos2(barra.left(), y), egui::pos2(barra.right(), y)],
                    Stroke::new(2.0, color_db(c.pico)),
                );
            }
            p.text(
                egui::pos2(barra.center().x, rect.bottom() - 9.0),
                Align2::CENTER_CENTER,
                if i == 0 { "L" } else { "R" },
                FontId::proportional(10.0),
                TENUE,
            );
        }
        let ayuda = if self.ctl.es_remoto() {
            "Los niveles de audio solo se ven en la PC principal".to_string()
        } else if !vivo {
            "Sin niveles de audio: CasparCG los envía por OSC (revisa que el tiempo diga «⏱ Tiempo exacto»)".into()
        } else {
            let t: Vec<String> = self.vu.iter().map(|c| format!("{:.0} dB", c.nivel)).collect();
            format!("Audio de la salida de CasparCG (picos): {}", t.join(" · "))
        };
        resp.on_hover_text(ayuda);
    }

    pub(super) fn datos_al_aire(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let actual = e.actual.and_then(|id| e.item(id));
        let proximo = e.proximo.and_then(|id| e.item(id));
        let pos_actual = e.actual.and_then(|id| e.lista.iter().position(|i| i.id == id));
        // Todo el bloque se escala con el alto del monitor (al arrastrar la separación).
        let k = self.escala_aire;
        ui.spacing_mut().item_spacing.y *= k;

        // Fila 1: estado, posición en la tanda y transición · a la derecha, cuánto falta.
        // (Las filas tienen alto fijo: el bloque mide lo mismo con la señal de TV que
        // al aire, y lo de abajo no se mueve al dar Play.)
        ui.horizontal(|ui| {
            ui.allocate_exact_size(Vec2::new(0.0, 42.0 * k), Sense::hover());
            let (texto, color, fondo) = match e.fase {
                FaseVista::AlAire => ("⏺  AL AIRE", Color32::WHITE, Color32::from_rgb(0xB0, 0x2A, 0x31)),
                FaseVista::Pausado => ("⏸  EN PAUSA", Color32::BLACK, AMBAR),
                FaseVista::Tv => ("📺  SEÑAL DE TV", Color32::WHITE, Color32::from_rgb(0x1F, 0x7A, 0x4A)),
            };
            tema::chip_de(k, ui, texto, color, fondo);
            if let Some(i) = pos_actual {
                let restante: f64 =
                    e.lista[i..].iter().map(|x| x.duracion.as_secs_f64()).sum::<f64>() - e.transcurrido.as_secs_f64();
                tema::chip_de(
                    k,
                    ui,
                    &format!("Spot {} de {} · tanda −{}", i + 1, e.lista.len(), mmss(restante.max(0.0))),
                    TEXTO,
                    TARJETA,
                );
            } else if let Some(i) = e.retomar.and_then(|id| e.lista.iter().position(|x| x.id == id)) {
                let resta: f64 = e.lista[i..].iter().map(|x| x.duracion.as_secs_f64()).sum();
                tema::chip_de(
                    k,
                    ui,
                    &format!("⏹ Detenida en el spot {} de {} · faltan {}", i + 1, e.lista.len(), mmss(resta)),
                    Color32::BLACK,
                    AMBAR,
                );
            } else if !e.lista.is_empty() {
                let total: f64 = e.lista.iter().map(|x| x.duracion.as_secs_f64()).sum();
                tema::chip_de(
                    k,
                    ui,
                    &format!("Tanda lista: {} spots · {}", e.lista.len(), mmss(total)),
                    TEXTO,
                    TARJETA,
                );
            }
            self.selector_transicion(ui, e);

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // El modo, siempre en el mismo sitio (a la derecha).
                if !self.ctl.es_remoto() {
                    self.selector_modo(ui, e);
                    ui.add_space(8.0);
                }
                if let Some(item) = actual {
                    let total = item.duracion.as_secs_f64();
                    let pasado = e.transcurrido.as_secs_f64().min(total);
                    let falta = (total - pasado).max(0.0);
                    // Los últimos 5 s el contador destella.
                    let ultimos = falta <= 5.0 && e.fase == FaseVista::AlAire && proximo.is_none();
                    let mut color = if e.fase == FaseVista::Pausado { AMBAR } else { AIRE };
                    if ultimos && dialogo::animaciones() {
                        let k = ((ui.ctx().input(|i| i.time) * 4.0).sin() * 0.5 + 0.5) as f32;
                        color = dialogo::mezclar(color, Color32::WHITE, k * 0.6);
                    }
                    ui.label(RichText::new(format!("−{}", mmss(falta))).font(FontId::monospace(34.0 * k)).color(color));
                }
            });
        });

        // Fila 2: qué sale. El nombre ocupa lo que queda después de reservar sitio a la
        // derecha para el tiempo y los saltos (si no, un nombre largo los empuja fuera).
        let con_saltos = actual.is_some() && self.ctl.tengo_mando(e);
        let reservado = if actual.is_none() {
            0.0
        } else if con_saltos {
            330.0 * k
        } else {
            130.0 * k
        };
        ui.horizontal(|ui| {
            ui.allocate_exact_size(Vec2::new(0.0, 32.0 * k), Sense::hover());
            let ancho_nombre = (ui.available_width() - reservado).max(60.0);
            ui.allocate_ui_with_layout(Vec2::new(ancho_nombre, 32.0 * k), Layout::left_to_right(Align::Center), |ui| {
                ui.set_max_width(ancho_nombre);
                let nombre_y_carpeta = |ui: &mut egui::Ui, previo: Option<&str>, clip: &str| {
                    let (carpeta, nombre) = partir(clip);
                    let mut texto = egui::text::LayoutJob::default();
                    if let Some(p) = previo {
                        texto.append(p, 0.0, egui::TextFormat::simple(FontId::proportional(15.0 * k), AMBAR));
                    }
                    texto.append(
                        nombre,
                        if previo.is_some() { 8.0 } else { 0.0 },
                        egui::TextFormat::simple(FontId::new(24.0 * k, crate::tema::negrita()), TEXTO),
                    );
                    let carpeta = carpeta.trim_end_matches('/');
                    if !carpeta.is_empty() {
                        texto.append(carpeta, 10.0, egui::TextFormat::simple(FontId::proportional(14.0 * k), TENUE));
                    }
                    texto.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
                    // `.truncate()`: si no, en una fila el texto se estira sin límite.
                    ui.add(egui::Label::new(texto).truncate().selectable(false)).on_hover_text(clip);
                };
                match actual {
                    Some(item) => nombre_y_carpeta(ui, None, &item.clip),
                    None => match e.retomar.and_then(|id| e.item(id)) {
                        Some(item) => nombre_y_carpeta(ui, Some("Quedó en"), &item.clip),
                        None => {
                            let t = if e.lista.is_empty() { "Sin tanda" } else { "Listo para salir" };
                            ui.label(
                                RichText::new(t).size(24.0 * k).strong().family(crate::tema::negrita()).color(TENUE),
                            );
                        }
                    },
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(item) = actual {
                    let total = item.duracion.as_secs_f64();
                    let pasado = e.transcurrido.as_secs_f64().min(total);
                    ui.label(
                        RichText::new(format!("{} / {}", mmss(pasado), mmss(total)))
                            .font(FontId::monospace(14.0 * k))
                            .color(TENUE),
                    );
                    // Saltos rápidos (para revisar un spot o el paso al siguiente).
                    if self.ctl.tengo_mando(e) {
                        ui.add_space(6.0 * k);
                        let b = |t: &str| {
                            egui::Button::new(RichText::new(t).size(11.5 * k).color(TEXTO))
                                .fill(TARJETA)
                                .stroke(Stroke::new(1.0, BORDE))
                                .corner_radius(CornerRadius::same(6))
                                .min_size(Vec2::new(0.0, 22.0 * k))
                        };
                        let fin = (total - 3.0).max(0.0);
                        let mut ir = None;
                        if ui
                            .add(b("⇥ 3 s"))
                            .on_hover_text("Ir a los últimos 3 segundos: para ver cómo entra el siguiente spot")
                            .clicked()
                        {
                            ir = Some(fin);
                        }
                        if ui.add(b("+5 s")).on_hover_text("Adelantar 5 segundos").clicked() {
                            ir = Some(pasado + 5.0);
                        }
                        if ui.add(b("−5 s")).on_hover_text("Retroceder 5 segundos").clicked() {
                            ir = Some((pasado - 5.0).max(0.0));
                        }
                        if let Some(t) = ir {
                            self.ctl.enviar(Orden::Buscar(Duration::from_secs_f64(t.max(0.0))));
                        }
                    }
                }
            });
        });

        // Fila 3: línea de tiempo del spot y, debajo, la tanda entera por tramos.
        let puede_saltar = self.ctl.tengo_mando(e) && e.fase != FaseVista::Tv;
        if let Some(orden) = linea_spot(ui, e, actual, puede_saltar, 18.0 * k) {
            self.ctl.enviar(orden);
        }
        ui.add_space(3.0 * k);
        if let Some(orden) = linea_tanda(ui, e, pos_actual, puede_saltar, 13.0 * k) {
            self.ctl.enviar(orden);
        }
        ui.add_space(5.0 * k);

        // Fila 4: lo que sigue · controles.
        ui.horizontal(|ui| {
            ui.allocate_exact_size(Vec2::new(0.0, 56.0 * k), Sense::hover());
            // Con la señal de TV: el spot que sale al dar Play.
            let en_cola = self.en_cola(e).and_then(|id| e.item(id));
            let tarjeta = if e.fase == FaseVista::Tv { en_cola } else { proximo };
            self.tarjeta_sigue(ui, e, tarjeta);
            ui.add_space(10.0 * k);
            self.transporte(ui, e);
        });
    }

    /// Elegir la transición entre spots (y al entrar y salir de la señal de TV).
    fn selector_transicion(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let t = e.transicion;
        let puede = self.ctl.tengo_mando(e);
        let texto = format!("✨ {}", t.describir());
        let k = self.escala_aire;
        let r = ui
            .add_enabled(
                puede,
                egui::Button::new(RichText::new(texto).size(11.5 * k).color(TEXTO))
                    .fill(TARJETA)
                    .stroke(Stroke::new(1.0, BORDE))
                    .corner_radius(CornerRadius::same(10)),
            )
            .on_hover_text("Transición entre spots y al entrar o salir de la señal de TV");
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(190.0);
            ui.label(RichText::new("Transición").size(12.0).color(TENUE));
            let mut nueva = t;
            for tipo in nucleo::TipoTransicion::TODOS {
                if ui.radio(t.tipo == tipo, tipo.nombre()).clicked() {
                    nueva.tipo = tipo;
                }
            }
            if t.tipo != nucleo::TipoTransicion::Corte {
                ui.separator();
                ui.label(RichText::new("Duración").size(12.0).color(TENUE));
                ui.horizontal(|ui| {
                    for s in [0.25f32, 0.5, 1.0, 1.5] {
                        let sel = (t.segundos - s).abs() < 0.01;
                        if ui.selectable_label(sel, format!("{s} s")).clicked() {
                            nueva.segundos = s;
                        }
                    }
                });
            }
            ui.separator();
            ui.label(
                RichText::new("Con fundido, el spot siguiente empieza\nmientras termina el anterior.")
                    .size(11.0)
                    .color(MUY_TENUE),
            );
            if nueva != t {
                self.ctl.enviar(Orden::Transicion(nueva));
            }
        });
    }

    /// "SIGUE": miniatura, nombre y duración del próximo spot.
    fn tarjeta_sigue(&mut self, ui: &mut egui::Ui, e: &Instantanea, proximo: Option<&nucleo::ItemTanda>) {
        // Lo que queda después de los controles (que miden unos 560 px).
        let k = self.escala_aire;
        let ancho = (ui.available_width() - 570.0 * k).clamp(140.0 * k, 340.0);
        egui::Frame::new()
            .fill(TARJETA)
            .stroke(Stroke::new(1.0, BORDE))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_width(ancho);
                ui.set_height(40.0 * k);
                // Alto fijo: con o sin spot, la tarjeta mide lo mismo (lo de abajo no se mueve).
                ui.set_max_height(40.0 * k);
                ui.spacing_mut().interact_size.y = 16.0 * k;
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(56.0 * k, 32.0 * k), Sense::hover());
                    match proximo {
                        Some(p) => {
                            let clip = p.clip.clone();
                            self.pintar_miniatura(ui, rect, Some(&clip), &clip, false);
                        }
                        None => {
                            ui.painter().rect_filled(rect, 4.0, TV_OSCURO);
                            ui.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                "📺",
                                FontId::proportional(14.0 * k),
                                TV,
                            );
                        }
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let (titulo, color) = match e.fase {
                            FaseVista::Tv if e.retomar.is_some() => ("AL DAR PLAY · SE REINICIA", AMBAR),
                            FaseVista::Tv => ("AL DAR PLAY", TENUE),
                            _ => ("SIGUE", TENUE),
                        };
                        ui.label(
                            RichText::new(titulo).size(10.5 * k).strong().family(crate::tema::negrita()).color(color),
                        );
                        match proximo {
                            Some(p) => {
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(nombre_corto(&p.clip))
                                                .size(13.5 * k)
                                                .strong()
                                                .family(crate::tema::negrita()),
                                        )
                                        .truncate(),
                                    );
                                    ui.label(
                                        RichText::new(mmss(p.duracion.as_secs_f64()))
                                            .monospace()
                                            .size(12.0 * k)
                                            .color(TENUE),
                                    );
                                });
                            }
                            None => {
                                let t = match e.fase {
                                    FaseVista::Tv => "—",
                                    _ if e.bucle == Bucle::Uno => "🔁 Se repite este spot",
                                    _ => "Vuelve la señal de TV",
                                };
                                ui.label(RichText::new(t).size(13.0 * k).color(TV));
                            }
                        }
                    });
                });
            });
    }

    pub(super) fn transporte(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let k = self.escala_aire;
        let grande = Vec2::new(46.0, 42.0) * k;
        let hay_tanda = !e.lista.is_empty();
        let al_aire = e.fase != FaseVista::Tv;
        let bloqueado = !self.ctl.tengo_mando(e);
        ui.add_enabled_ui(!bloqueado, |ui| {
            // Con la tanda detenida a mitad, ⏮ ⏭ eligen desde qué spot sigue.
            let detenida = e.fase == FaseVista::Tv && e.retomar.is_some();
            let ayuda_ant = if detenida { "Seguir desde el spot anterior" } else { "Anterior" };
            if ui.add_enabled(al_aire || detenida, boton_icono("⏮", grande, k)).on_hover_text(ayuda_ant).clicked() {
                self.ctl.enviar(Orden::Anterior);
            }
            let (icono, ayuda, fondo) = match e.fase {
                FaseVista::AlAire => ("⏸", "Pausa (Espacio)", AMBAR_OSCURO),
                _ if detenida => (
                    "⏵",
                    "Volver a empezar el spot donde se detuvo y seguir la tanda (Espacio)",
                    Color32::from_rgb(0xB0, 0x2A, 0x31),
                ),
                _ => ("⏵", "Reproducir la tanda (Espacio)", Color32::from_rgb(0xB0, 0x2A, 0x31)),
            };
            if ui
                .add_enabled(hay_tanda, boton_icono(icono, Vec2::new(64.0, 42.0) * k, k).fill(fondo))
                .on_hover_text(ayuda)
                .clicked()
            {
                self.ctl.enviar(Orden::PlayPausa(self.tanda_sel));
            }
            let ayuda_sig = if detenida { "Seguir desde el spot siguiente" } else { "Siguiente" };
            if ui.add_enabled(al_aire || detenida, boton_icono("⏭", grande, k)).on_hover_text(ayuda_sig).clicked() {
                self.ctl.enviar(Orden::Siguiente);
            }
            // ⏹ detiene (Play retoma ese spot); ⏹ otra vez reinicia la tanda.
            let ayuda_stop = if detenida {
                "Reiniciar la tanda: el próximo Play empieza desde el primer spot"
            } else {
                "Detener: vuelve la señal de TV. Play vuelve a empezar este spot y sigue la tanda.\n⏹ otra vez: reiniciar la tanda"
            };
            if ui.add_enabled(al_aire || detenida, boton_icono("⏹", grande, k)).on_hover_text(ayuda_stop).clicked() {
                self.detener();
            }
            ui.add_space(8.0 * k);
            let (icono_bucle, ayuda_bucle, activo) = match e.bucle {
                Bucle::No => ("🔁", "Bucle: no", false),
                Bucle::Lista => ("🔁", "Bucle: repite la tanda", true),
                Bucle::Uno => ("🔂", "Bucle: repite el spot", true),
            };
            if ui
                .add(alternador(icono_bucle, activo, k).min_size(Vec2::new(40.0, 36.0) * k))
                .on_hover_text(format!("{ayuda_bucle}\n(clic: no → tanda → spot)"))
                .clicked()
            {
                self.ctl.enviar(Orden::Bucle(e.bucle.rotar()));
            }
            if ui
                .add(alternador("🔀", e.aleatorio, k).min_size(Vec2::new(40.0, 36.0) * k))
                .on_hover_text("Orden al azar dentro de la tanda")
                .clicked()
            {
                self.ctl.enviar(Orden::Aleatorio(!e.aleatorio));
            }
            ui.add_space(8.0 * k);
            ui.label(RichText::new("🔊").size(16.0 * k).color(TENUE)).on_hover_text("Volumen de la publicidad en CasparCG");
            if !ui.ctx().is_being_dragged(ui.next_auto_id()) {
                self.volumen = e.volumen;
            }
            let r = ui.add_sized([96.0 * k, 24.0 * k], egui::Slider::new(&mut self.volumen, 0.0..=1.0).show_value(false));
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                self.ctl.enviar(Orden::Volumen(self.volumen));
            }
            ui.label(RichText::new(format!("{:.0}%", self.volumen * 100.0)).size(12.5 * k).color(TENUE));
        });
        if self.cfg.interfaz.boton_volver_a_tv {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let boton = egui::Button::new(
                    RichText::new("VOLVER A TV  (Esc)")
                        .size(15.0 * k)
                        .strong()
                        .family(crate::tema::negrita())
                        .color(Color32::WHITE),
                )
                .fill(Color32::from_rgb(0x1F, 0x7A, 0x4A))
                .stroke(Stroke::new(1.0, TV))
                .corner_radius(CornerRadius::same(8))
                .min_size(Vec2::new(190.0, 42.0) * k);
                if ui.add(boton).clicked() {
                    self.ctl.enviar(Orden::VolverATv);
                }
            });
        }
    }
}

/// Color del avance según la fase (en pausa, ámbar).
fn color_avance(fase: FaseVista) -> Color32 {
    if fase == FaseVista::Pausado {
        AMBAR
    } else {
        AIRE
    }
}

/// Punto del spot (segundos) bajo una x de la línea.
fn segundos_en(rect: egui::Rect, x: f32, total: f64) -> f64 {
    (((x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64) * total
}

/// Línea de tiempo del spot al aire: avance, marcas cada segundo (y más largas cada 5),
/// la zona donde entra el siguiente con la transición, y el cabezal. Con la tanda al
/// aire, clic o arrastrar: ir a ese punto.
fn linea_spot(
    ui: &mut egui::Ui,
    e: &Instantanea,
    actual: Option<&nucleo::ItemTanda>,
    puede_saltar: bool,
    alto: f32,
) -> Option<Orden> {
    let total = actual.map(|i| i.duracion.as_secs_f64()).unwrap_or(0.0);
    let activa = puede_saltar && total > 0.0;
    let sentido = if activa { Sense::click_and_drag() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), alto), sentido);
    let p = ui.painter();
    let radio = alto * 0.3;
    p.rect_filled(rect, radio, Color32::from_rgb(0x0C, 0x0E, 0x13));
    let borde = if activa && resp.hovered() { dialogo::mezclar(BORDE, TEXTO, 0.25) } else { BORDE };
    p.rect_stroke(rect, radio, Stroke::new(1.0, borde), StrokeKind::Inside);
    if total <= 0.0 {
        return None;
    }
    let dentro = rect.shrink(2.0);
    let x_de = |seg: f64| dentro.left() + dentro.width() * (seg / total).clamp(0.0, 1.0) as f32;

    // Zona del final donde entra el siguiente (con fundido u otra transición).
    let hay_siguiente = e.proximo.is_some();
    let trans = e.transicion.segundos as f64;
    if hay_siguiente && e.transicion.tipo != nucleo::TipoTransicion::Corte && trans > 0.0 {
        let zona = egui::Rect::from_x_y_ranges(x_de(total - trans)..=dentro.right(), dentro.y_range());
        p.rect_filled(zona, radio * 0.7, Color32::from_rgba_unmultiplied(0x3B, 0x82, 0xF6, 46));
    }
    // Avance.
    let pasado = e.transcurrido.as_secs_f64().min(total);
    let mut lleno = dentro;
    lleno.set_right(x_de(pasado));
    if lleno.width() > 0.5 {
        p.rect_filled(lleno, radio * 0.7, color_avance(e.fase));
    }
    // Marcas: cada segundo (si el spot no es muy largo) y más largas cada 5.
    let paso_menor = if total <= 90.0 {
        1.0
    } else if total <= 300.0 {
        5.0
    } else {
        30.0
    };
    let mut t = paso_menor;
    while t < total {
        let x = x_de(t);
        let mayor = (t / 5.0).fract().abs() < 1e-6 || paso_menor >= 5.0;
        let largo = if mayor { dentro.height() * 0.45 } else { dentro.height() * 0.22 };
        let color = if x <= lleno.right() {
            Color32::from_white_alpha(if mayor { 110 } else { 60 })
        } else {
            Color32::from_white_alpha(if mayor { 60 } else { 26 })
        };
        p.line_segment(
            [egui::pos2(x, dentro.bottom() - largo), egui::pos2(x, dentro.bottom())],
            Stroke::new(1.0, color),
        );
        t += paso_menor;
    }
    // Cabezal.
    let xc = x_de(pasado);
    p.line_segment(
        [egui::pos2(xc, rect.top() + 1.0), egui::pos2(xc, rect.bottom() - 1.0)],
        Stroke::new(2.0, Color32::WHITE),
    );
    p.circle_filled(egui::pos2(xc, rect.center().y), (alto * 0.26).max(3.0), Color32::WHITE);

    if !activa {
        if hay_siguiente && trans > 0.0 && resp.hovered() {
            resp.on_hover_text("La zona azul del final es donde entra el siguiente spot (transición)");
        }
        return None;
    }
    // Al pasar o arrastrar: dónde caería.
    let mut orden = None;
    if let Some(pos) = resp.hover_pos().or(resp.interact_pointer_pos()) {
        let x = pos.x.clamp(dentro.left(), dentro.right());
        let seg = segundos_en(dentro, x, total);
        let fantasma = if resp.dragged() { Color32::from_white_alpha(200) } else { Color32::from_white_alpha(90) };
        p.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], Stroke::new(1.5, fantasma));
        let falta = (total - seg).max(0.0);
        let texto = format!("{}  ·  faltan {}", mmss(seg), mmss(falta));
        if resp.clicked() || resp.drag_stopped() {
            orden = Some(Orden::Buscar(Duration::from_secs_f64(seg)));
        }
        resp.clone().on_hover_text_at_pointer(texto);
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    orden
}

/// La tanda entera como una línea por tramos (uno por spot, proporcional a su
/// duración): lo emitido apagado, el actual con su avance y lo que falta en tenue.
/// Con la tanda al aire, clic en un tramo: ese spot, desde ese punto.
fn linea_tanda(
    ui: &mut egui::Ui,
    e: &Instantanea,
    actual: Option<usize>,
    puede_saltar: bool,
    alto: f32,
) -> Option<Orden> {
    let sentido = if puede_saltar && !e.lista.is_empty() { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), alto), sentido);
    if e.lista.is_empty() {
        ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(0x15, 0x18, 0x1F));
        return None;
    }
    let total: f64 = e.lista.iter().map(|i| i.duracion.as_secs_f64().max(0.5)).sum();
    let p = ui.painter();
    let separacion = 3.0;
    let ancho_util = rect.width() - separacion * (e.lista.len().saturating_sub(1)) as f32;
    let retomar = e.retomar.and_then(|id| e.lista.iter().position(|x| x.id == id));
    let hover = resp.hover_pos();
    let mut x = rect.left();
    let mut bajo: Option<(usize, egui::Rect)> = None;
    let fuente = FontId::proportional((alto * 0.72).max(8.0));
    for (k, item) in e.lista.iter().enumerate() {
        let w = (ancho_util as f64 * item.duracion.as_secs_f64().max(0.5) / total) as f32;
        let tramo = egui::Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(w.max(1.0), rect.height()));
        x += w + separacion;
        let encima = hover.is_some_and(|h| tramo.x_range().contains(h.x));
        if encima {
            bajo = Some((k, tramo));
        }
        let es_actual = Some(k) == actual;
        let mut fondo = match actual {
            Some(a) if k < a => Color32::from_rgb(0x2C, 0x31, 0x3B),
            Some(a) if k == a => Color32::from_rgb(0x4A, 0x1C, 0x21),
            _ => Color32::from_rgb(0x24, 0x2A, 0x36),
        };
        if Some(k) == retomar {
            fondo = AMBAR_OSCURO;
        }
        if encima {
            fondo = dialogo::mezclar(fondo, Color32::WHITE, 0.12);
        }
        p.rect_filled(tramo, 3.0, fondo);
        if es_actual {
            let total_item = item.duracion.as_secs_f64();
            let frac = if total_item > 0.0 { (e.transcurrido.as_secs_f64() / total_item).clamp(0.0, 1.0) } else { 0.0 };
            let mut hecho = tramo;
            hecho.set_width(tramo.width() * frac as f32);
            p.rect_filled(hecho, 3.0, color_avance(e.fase));
            p.rect_stroke(tramo, 3.0, Stroke::new(1.0, color_avance(e.fase)), StrokeKind::Inside);
        } else if Some(k) == retomar {
            p.rect_stroke(tramo, 3.0, Stroke::new(1.0, AMBAR), StrokeKind::Inside);
        }
        // Número del spot, si cabe.
        let numero = (k + 1).to_string();
        if w >= alto * 1.6 {
            let color = if es_actual { Color32::WHITE } else { Color32::from_white_alpha(140) };
            p.text(tramo.left_center() + Vec2::new(4.0, 0.0), Align2::LEFT_CENTER, numero, fuente.clone(), color);
        }
    }
    let (k, tramo) = bajo?;
    let item = &e.lista[k];
    let h = hover.expect("encima");
    let dur = item.duracion.as_secs_f64();
    let seg = segundos_en(tramo, h.x, dur);
    // Cerca del principio del tramo: desde el principio.
    let seg = if seg < dur * 0.04 { 0.0 } else { seg };
    let mut texto = format!("{} · {} · {}", k + 1, nombre_corto(&item.clip), mmss(dur));
    if puede_saltar {
        texto.push_str(&format!("\nClic: ir a {} de este spot", mmss(seg)));
        ui.painter().line_segment(
            [egui::pos2(h.x, rect.top() - 1.0), egui::pos2(h.x, rect.bottom() + 1.0)],
            Stroke::new(1.5, Color32::from_white_alpha(170)),
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    } else if e.fase == FaseVista::Tv {
        texto.push_str("\nCon la tanda al aire, clic para ir a ese punto");
    }
    let clic = resp.clicked();
    resp.on_hover_text_at_pointer(texto);
    (puede_saltar && clic).then(|| Orden::ReproducirDesde(item.id, Duration::from_secs_f64(seg)))
}
