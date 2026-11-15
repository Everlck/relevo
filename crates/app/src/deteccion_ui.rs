//! Ventana «Detección de corte»: elegir la señal, marcar el logo con el mouse,
//! aprenderlo, ver el parecido en un gráfico y decidir qué hacer al detectar el corte.

use super::*;
use crate::arranque::Monitor;
use crate::config::DeteccionCfg;
use crate::deteccion::{self, Estado, Vista};
use std::collections::VecDeque;
use std::sync::Mutex;

pub(super) struct DetUi {
    pub abierta: bool,
    textura: Option<(u64, egui::TextureHandle)>,
    tex_logo: Option<(usize, egui::TextureHandle)>,
    /// Dirección o ruta que se está escribiendo (se aplica con Conectar o Enter).
    origen: String,
    pub capturadoras: Arc<Mutex<Option<Vec<String>>>>,
    pub monitores: Arc<Mutex<Option<Vec<Monitor>>>>,
    /// Punto donde empezó a marcarse la zona del logo (0–1).
    zona_desde: Option<egui::Vec2>,
    /// Lo último guardado en config.toml.
    pub guardado: DeteccionCfg,
    cambio: Option<Instant>,
    /// Últimos avisos: (hora, texto).
    pub registro: VecDeque<(String, String)>,
}

impl DetUi {
    pub fn new(cfg: &DeteccionCfg) -> Self {
        DetUi {
            abierta: false,
            textura: None,
            tex_logo: None,
            origen: cfg.origen.clone(),
            capturadoras: Default::default(),
            monitores: Default::default(),
            zona_desde: None,
            guardado: cfg.clone(),
            cambio: None,
            registro: VecDeque::new(),
        }
    }

    pub fn anotar(&mut self, hora: String, texto: impl Into<String>) {
        self.registro.push_front((hora, texto.into()));
        self.registro.truncate(30);
    }
}

fn seccion(ui: &mut egui::Ui, numero: &str, titulo: &str, hecho: bool, f: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(TARJETA)
        .stroke(Stroke::new(1.0, BORDE))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                let (fondo, texto) = if hecho { (TV, "✔") } else { (BORDE, numero) };
                ui.painter().circle_filled(r.center(), 10.0, fondo);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, texto, FontId::proportional(11.5), Color32::WHITE);
                ui.label(RichText::new(titulo).size(14.0).family(tema::negrita()).color(TEXTO));
            });
            ui.add_space(4.0);
            f(ui);
        });
    ui.add_space(8.0);
}

impl Ventana {
    pub(super) fn abrir_deteccion(&mut self) {
        self.det.abierta = true;
        self.det.origen = self.cfg.deteccion.origen.clone();
        self.buscar_dispositivos();
    }

    /// Lista las capturadoras y los monitores (en segundo plano).
    pub(super) fn buscar_dispositivos(&mut self) {
        if let Some(f) = self.vista.ffmpeg().map(Path::to_path_buf) {
            let c = self.det.capturadoras.clone();
            *c.lock().expect("capturadoras") = None;
            std::thread::spawn(move || {
                let v = deteccion::capturadoras(&f);
                *c.lock().expect("capturadoras") = Some(v);
            });
        }
        let m = self.det.monitores.clone();
        *m.lock().expect("monitores") = None;
        std::thread::spawn(move || {
            let v = crate::arranque::monitores();
            *m.lock().expect("monitores") = Some(v);
        });
    }

    pub(super) fn ventana_deteccion(&mut self, ctx: &egui::Context) {
        // Los cambios se guardan solos en config.toml cuando se dejan de mover.
        if self.cfg.deteccion != self.det.guardado {
            let quieto = !ctx.input(|i| i.pointer.any_down());
            match self.det.cambio {
                Some(t) if quieto && t.elapsed() > Duration::from_millis(600) => {
                    self.ajustes.guardar_deteccion(&self.cfg.deteccion);
                    self.det.guardado = self.cfg.deteccion.clone();
                    self.det.cambio = None;
                }
                Some(_) => ctx.request_repaint_after(Duration::from_millis(200)),
                None => self.det.cambio = Some(Instant::now()),
            }
        }
        if !self.det.abierta {
            return;
        }
        let v = self.detector.as_ref().map(|d| d.vista());
        let mut cerrar = false;
        let sub = "Relevo mira la señal del canal y vigila su logo: cuando desaparece, el canal fue a comerciales";
        let r = Dialogo::new("relevo-deteccion", "✂", "Detección de corte").subtitulo(sub).ancho(980.0).mostrar(
            ctx,
            |ui| {
                if self.vista.ffmpeg().is_none() {
                    ui.label(
                        RichText::new("Hace falta ffmpeg (se descarga solo: mira 🎞 ffmpeg en la barra de abajo).")
                            .color(AMBAR),
                    );
                    return;
                }
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(560.0);
                        self.det_imagen(ui, v.as_ref());
                        ui.add_space(8.0);
                        self.det_grafico(ui, v.as_ref());
                    });
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("det-pasos")
                            .max_height(520.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                self.det_senal(ui, v.as_ref());
                                self.det_logo(ui, v.as_ref());
                                self.det_accion(ui);
                                self.det_registro(ui);
                            });
                    });
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    // Interruptor general.
                    let activa = self.cfg.deteccion.activa;
                    let (texto, fondo) = if activa {
                        ("●  Detección ENCENDIDA", Color32::from_rgb(0x1F, 0x7A, 0x4A))
                    } else {
                        ("○  Detección APAGADA", Color32::from_rgb(0x3A, 0x40, 0x4C))
                    };
                    let b = egui::Button::new(
                        RichText::new(texto).size(14.0).family(tema::negrita()).color(Color32::WHITE),
                    )
                    .fill(fondo)
                    .corner_radius(CornerRadius::same(8))
                    .min_size(Vec2::new(230.0, 36.0));
                    if ui.add(b).on_hover_text("Clic para encenderla o apagarla").clicked() {
                        self.cfg.deteccion.activa = !activa;
                    }
                    let nota = if activa {
                        "Vigilando. Se puede apagar también desde ✂, en la barra de abajo."
                    } else {
                        "Apagada: aquí se puede probar, pero no avisa ni inicia la tanda."
                    };
                    ui.label(RichText::new(nota).size(12.0).color(TENUE));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(dialogo::boton_secundario("Cerrar")).clicked() {
                            cerrar = true;
                        }
                    });
                });
            },
        );
        if r.cerrar || cerrar {
            self.det.abierta = false;
        }
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    /// La señal en grande; arrastrando sobre ella se marca la zona del logo.
    fn det_imagen(&mut self, ui: &mut egui::Ui, v: Option<&Vista>) {
        let ancho = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ancho, ancho * 9.0 / 16.0), Sense::click_and_drag());
        let p = ui.painter().with_clip_rect(rect);
        p.rect_filled(rect, 8.0, Color32::BLACK);
        if let Some(cuadro) = v.and_then(|v| v.cuadro.as_ref().map(|c| (v.serie, c))) {
            let (serie, cuadro) = cuadro;
            if self.det.textura.as_ref().map(|t| t.0) != Some(serie) {
                let img = egui::ColorImage::from_rgb([crate::senal::ANCHO, crate::senal::ALTO], cuadro);
                match &mut self.det.textura {
                    Some((s, t)) => {
                        t.set(img, egui::TextureOptions::LINEAR);
                        *s = serie;
                    }
                    None => {
                        let t = ui.ctx().load_texture("deteccion", img, egui::TextureOptions::LINEAR);
                        self.det.textura = Some((serie, t));
                    }
                }
            }
            if let Some((_, t)) = &self.det.textura {
                egui::Image::new(t).corner_radius(8).fit_to_exact_size(rect.size()).paint_at(ui, rect);
            }
        } else {
            let texto = match v.map(|v| &v.estado) {
                Some(Estado::Error(e)) => format!("⚠ {e}"),
                Some(_) => "Conectando con la señal…".into(),
                None => "Elige de dónde viene la señal (paso 1)".into(),
            };
            let g = p.layout(texto, FontId::proportional(14.0), TENUE, rect.width() - 60.0);
            p.galley(rect.center() - g.size() / 2.0, g, TENUE);
        }

        // Marcar la zona arrastrando.
        let a_parte = |pos: egui::Pos2| {
            Vec2::new(
                ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
            )
        };
        if resp.drag_started() {
            self.det.zona_desde = resp.interact_pointer_pos().map(a_parte);
        }
        if let (Some(desde), Some(pos)) = (self.det.zona_desde, resp.interact_pointer_pos()) {
            let hasta = a_parte(pos);
            let (x0, x1) = (desde.x.min(hasta.x), desde.x.max(hasta.x));
            let (y0, y1) = (desde.y.min(hasta.y), desde.y.max(hasta.y));
            if x1 - x0 > 0.03 && y1 - y0 > 0.03 {
                let d = &mut self.cfg.deteccion;
                // (Redondeado, para que el logo aprendido se reconozca al reabrir.)
                let r3 = |x: f32| (x as f64 * 1000.0).round() / 1000.0;
                d.x = r3(x0);
                d.y = r3(y0);
                d.ancho = r3(x1 - x0);
                d.alto = r3(y1 - y0);
            }
        }
        if resp.drag_stopped() {
            self.det.zona_desde = None;
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let d = &self.cfg.deteccion;
        let zona = egui::Rect::from_min_size(
            rect.min + Vec2::new(rect.width() * d.x as f32, rect.height() * d.y as f32),
            Vec2::new(rect.width() * d.ancho as f32, rect.height() * d.alto as f32),
        )
        .intersect(rect);
        // Lo de fuera de la zona, un poco oscurecido.
        for r in [
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), zona.top())),
            egui::Rect::from_min_max(egui::pos2(rect.left(), zona.bottom()), rect.max),
            egui::Rect::from_min_max(egui::pos2(rect.left(), zona.top()), egui::pos2(zona.left(), zona.bottom())),
            egui::Rect::from_min_max(egui::pos2(zona.right(), zona.top()), egui::pos2(rect.right(), zona.bottom())),
        ] {
            p.rect_filled(r, 0.0, Color32::from_black_alpha(90));
        }
        let color = match v.map(|v| &v.estado) {
            Some(Estado::Programa) => TV,
            Some(Estado::Corte) => AIRE,
            _ => AMBAR,
        };
        p.rect_stroke(zona, 2.0, Stroke::new(2.0, color), StrokeKind::Outside);
        let (donde, ancla) = if zona.top() - rect.top() > 18.0 {
            (zona.left_top() + Vec2::new(0.0, -4.0), Align2::LEFT_BOTTOM)
        } else {
            (zona.left_bottom() + Vec2::new(0.0, 4.0), Align2::LEFT_TOP)
        };
        p.text(donde, ancla, "LOGO", FontId::new(10.5, tema::negrita()), color);

        // Estado, arriba a la izquierda.
        let (texto, fondo) = match v.map(|v| &v.estado) {
            Some(Estado::Programa) => ("✔ PROGRAMA", Color32::from_rgb(0x1F, 0x7A, 0x4A)),
            Some(Estado::Corte) => ("✂ CORTE", Color32::from_rgb(0xB0, 0x2A, 0x31)),
            Some(Estado::SinAprender) => ("Falta aprender el logo", Color32::from_rgb(0x8A, 0x5A, 0x10)),
            Some(Estado::Conectando) => ("Conectando…", Color32::from_rgb(0x3A, 0x40, 0x4C)),
            Some(Estado::Error(_)) => ("Sin señal", Color32::from_rgb(0xB0, 0x2A, 0x31)),
            None => ("Sin configurar", Color32::from_rgb(0x3A, 0x40, 0x4C)),
        };
        let g = p.layout_no_wrap(texto.into(), FontId::new(12.0, tema::negrita()), Color32::WHITE);
        let caja = egui::Rect::from_min_size(rect.min + Vec2::splat(10.0), g.size() + Vec2::new(16.0, 8.0));
        p.rect_filled(caja, 6.0, fondo);
        p.galley(caja.min + Vec2::new(8.0, 4.0), g, Color32::WHITE);
        ui.label(
            RichText::new("Arrastra sobre la imagen para marcar dónde está el logo del canal.")
                .size(11.5)
                .color(MUY_TENUE),
        );
    }

    /// Gráfico del parecido de los últimos 90 s, con la línea del mínimo (se mueve
    /// con el mouse).
    fn det_grafico(&mut self, ui: &mut egui::Ui, v: Option<&Vista>) {
        let historia: &[f32] = v.map(|v| v.historia.as_slice()).unwrap_or(&[]);
        let umbral = self.cfg.deteccion.umbral as f32;
        ui.horizontal(|ui| {
            ui.label(RichText::new("PARECIDO CON EL LOGO").size(11.0).family(tema::negrita()).color(TENUE));
            if let Some(v) = v.filter(|v| matches!(v.estado, Estado::Programa | Estado::Corte)) {
                let c = if v.puntaje >= umbral { TV } else { AIRE };
                ui.label(RichText::new(format!("{:.2}", v.puntaje)).monospace().size(13.0).color(c));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("últimos 90 s").size(11.0).color(MUY_TENUE));
            });
        });
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 96.0), Sense::click_and_drag());
        let p = ui.painter().with_clip_rect(rect);
        p.rect_filled(rect, 6.0, Color32::from_rgb(0x0C, 0x0E, 0x13));
        let y_de = |valor: f32| rect.bottom() - 4.0 - (rect.height() - 8.0) * valor.clamp(0.0, 1.0);
        // Mover el mínimo con el mouse.
        if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.clicked()) {
            let valor = (rect.bottom() - 4.0 - pos.y) / (rect.height() - 8.0);
            self.cfg.deteccion.umbral = ((valor.clamp(0.05, 0.9) as f64) * 100.0).round() / 100.0;
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        let yu = y_de(umbral);
        p.rect_filled(egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), yu)), 0.0, TV.gamma_multiply(0.07));
        p.rect_filled(egui::Rect::from_min_max(egui::pos2(rect.left(), yu), rect.max), 0.0, AIRE.gamma_multiply(0.07));
        if historia.len() >= 2 {
            let n = 450.0f32;
            let paso = rect.width() / n;
            let x0 = rect.right() - paso * historia.len() as f32;
            for (i, par) in historia.windows(2).enumerate() {
                let a = egui::pos2(x0 + paso * i as f32, y_de(par[0]));
                let b = egui::pos2(x0 + paso * (i + 1) as f32, y_de(par[1]));
                let c = if par[1] >= umbral { TV } else { AIRE };
                p.line_segment([a, b], Stroke::new(1.6, c));
            }
        } else {
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Aquí se verá el parecido cuando el logo esté aprendido",
                FontId::proportional(12.0),
                MUY_TENUE,
            );
        }
        p.hline(rect.x_range(), yu, Stroke::new(1.5, Color32::WHITE));
        p.text(
            egui::pos2(rect.right() - 6.0, yu - 3.0),
            Align2::RIGHT_BOTTOM,
            format!("mínimo {umbral:.2}"),
            FontId::proportional(10.5),
            Color32::WHITE,
        );
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Sobre la línea hay logo (programa); debajo, corte. La línea se mueve con el mouse.")
                    .size(11.5)
                    .color(MUY_TENUE),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Punto medio entre lo más alto y lo más bajo que se vio.
                let mut orden: Vec<f32> = historia.to_vec();
                orden.sort_by(|a, b| a.total_cmp(b));
                let sugerido = (orden.len() > 50)
                    .then(|| (orden[orden.len() / 10], orden[orden.len() * 9 / 10]))
                    .filter(|(bajo, alto)| alto - bajo > 0.25)
                    .map(|(bajo, alto)| (bajo + alto) / 2.0);
                if ui
                    .add_enabled(sugerido.is_some(), egui::Button::new("Ajustar solo"))
                    .on_hover_text("Pone el mínimo en medio de lo visto con programa y con comerciales")
                    .on_disabled_hover_text("Hace falta haber visto programa y comerciales en estos 90 s")
                    .clicked()
                {
                    if let Some(s) = sugerido {
                        self.cfg.deteccion.umbral = (s as f64 * 100.0).round() / 100.0;
                    }
                }
            });
        });
    }

    fn det_senal(&mut self, ui: &mut egui::Ui, v: Option<&Vista>) {
        let con_senal = v.is_some_and(|v| v.cuadro.is_some());
        let caps = self.det.capturadoras.lock().expect("capturadoras").clone();
        let mons = self.det.monitores.lock().expect("monitores").clone();
        let mut buscar = false;
        let mut abrir_iptv = false;
        let antes = {
            let d = &self.cfg.deteccion;
            (d.fuente.clone(), d.origen.clone(), d.monitor)
        };
        seccion(ui, "1", "Señal del canal", con_senal, |ui| {
            let d = &mut self.cfg.deteccion;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (k, t) in [
                    ("capturadora", "🎥 Capturadora"),
                    ("web", "🌐 Web"),
                    ("iptv", "📺 IPTV"),
                    ("pantalla", "🖥 Pantalla"),
                    ("archivo", "🎞 Archivo"),
                ] {
                    let sel = d.fuente == k || (k == "web" && d.fuente == "url");
                    let b =
                        egui::Button::new(RichText::new(t).size(12.5).color(if sel { Color32::WHITE } else { TEXTO }))
                            .fill(if sel { ACENTO_OSCURO } else { PANEL })
                            .stroke(Stroke::new(1.0, if sel { ACENTO } else { BORDE }))
                            .min_size(Vec2::new(0.0, 30.0));
                    if ui.add(b).clicked() && !sel {
                        d.fuente = k.into();
                        d.origen.clear();
                        self.det.origen.clear();
                    }
                }
            });
            ui.add_space(4.0);
            match d.fuente.as_str() {
                "capturadora" => {
                    ui.horizontal(|ui| {
                        let actual =
                            if d.origen.is_empty() { "Elegir capturadora…".to_string() } else { d.origen.clone() };
                        egui::ComboBox::from_id_salt("det-cap").width(250.0).selected_text(actual).show_ui(ui, |ui| {
                            match &caps {
                                None => {
                                    ui.label("Buscando…");
                                }
                                Some(l) if l.is_empty() => {
                                    ui.label("No se encontró ninguna");
                                }
                                Some(l) => {
                                    for n in l {
                                        if ui.selectable_label(d.origen == *n, n).clicked() {
                                            d.origen = n.clone();
                                        }
                                    }
                                }
                            }
                        });
                        if ui.button("⟳").on_hover_text("Volver a buscar").clicked() {
                            buscar = true;
                        }
                    });
                    let n = caps.as_ref().map(|l| l.len());
                    let t = match n {
                        None => "Buscando capturadoras…".to_string(),
                        Some(0) => "No hay capturadoras de video conectadas (o están en uso por otro programa).".into(),
                        Some(n) => format!("{n} dispositivo(s) de video en esta PC."),
                    };
                    ui.label(RichText::new(t).size(11.5).color(MUY_TENUE));
                }
                "web" | "url" => {
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.det.origen)
                                .desired_width(230.0)
                                .hint_text("https://… (la página con la señal en vivo)"),
                        );
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if ui
                            .add(dialogo::boton_principal("Conectar", Tono::Normal).min_size(Vec2::new(80.0, 28.0)))
                            .clicked()
                            || enter
                        {
                            d.fuente = "web".into();
                            d.origen = self.det.origen.trim().to_string();
                        }
                    });
                    let t = match v {
                        Some(Vista { flujo: Some(f), .. }) if con_senal => {
                            let corto: String = f.chars().take(52).collect();
                            format!("✔ Video encontrado: {corto}…")
                        }
                        Some(Vista { estado: Estado::Error(e), .. }) => format!("⚠ {e}"),
                        _ if d.origen.is_empty() => {
                            "Pega la dirección de la página (o del video .m3u8) y pulsa Conectar. Relevo busca el video dentro.".into()
                        }
                        _ => "Buscando el video en la página…".into(),
                    };
                    ui.add(egui::Label::new(RichText::new(t).size(11.5).color(TENUE)).wrap());
                }
                "iptv" => {
                    let i = &self.cfg.iptv;
                    let t = if i.direccion.trim().is_empty() {
                        "Primero indica el canal en ⚙ Ajustes → Señal por IPTV.".to_string()
                    } else {
                        let corto: String = i.direccion.trim().chars().take(52).collect();
                        let nota = if i.activa { "" } else { " (IPTV desactivada en CasparCG: solo se mira)" };
                        format!("Mira el mismo canal que CasparCG: {corto}{nota}")
                    };
                    ui.add(egui::Label::new(RichText::new(t).size(11.5).color(TENUE)).wrap());
                    if ui.small_button("⚙ Ajustes de IPTV").clicked() {
                        abrir_iptv = true;
                    }
                }
                "pantalla" => {
                    ui.horizontal(|ui| {
                        let actual = match d.monitor {
                            0 => "Todo el escritorio".to_string(),
                            n => mons
                                .iter()
                                .flatten()
                                .find(|m| m.numero == n)
                                .map(|m| m.describir())
                                .unwrap_or_else(|| format!("Monitor {n}")),
                        };
                        egui::ComboBox::from_id_salt("det-mon").width(250.0).selected_text(actual).show_ui(ui, |ui| {
                            ui.selectable_value(&mut d.monitor, 0, "Todo el escritorio");
                            for m in mons.iter().flatten() {
                                ui.selectable_value(&mut d.monitor, m.numero, m.describir());
                            }
                        });
                        if ui.button("⟳").on_hover_text("Volver a buscar").clicked() {
                            buscar = true;
                        }
                    });
                    ui.add(
                        egui::Label::new(
                            RichText::new("Abre la señal (por ejemplo, la página del canal) a pantalla completa en ese monitor y déjala ahí.")
                                .size(11.5)
                                .color(MUY_TENUE),
                        )
                        .wrap(),
                    );
                }
                _ => {
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.det.origen)
                                .desired_width(250.0)
                                .hint_text("un video grabado del canal"),
                        );
                        if r.lost_focus() {
                            d.origen = self.det.origen.trim().to_string();
                        }
                        if ui.button("…").clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_file() {
                                self.det.origen = p.display().to_string();
                                d.origen = self.det.origen.clone();
                            }
                        }
                    });
                    ui.label(
                        RichText::new("Para probar sin la señal en vivo (se repite sin fin).")
                            .size(11.5)
                            .color(MUY_TENUE),
                    );
                }
            }
        });
        if buscar {
            self.buscar_dispositivos();
        }
        if abrir_iptv {
            self.det.abierta = false;
            self.ajustes.abrir_iptv(&self.cfg);
        }
        // Al cambiar de señal se apaga: con otra imagen el logo puede no coincidir y
        // daría un corte falso (con la tanda al aire, si está en «iniciar sola»).
        let d = &mut self.cfg.deteccion;
        if d.activa && antes != (d.fuente.clone(), d.origen.clone(), d.monitor) {
            d.activa = false;
            let hora = self.hora(time::macros::format_description!("[hour]:[minute]:[second]"));
            self.det.anotar(hora, "Se apagó al cambiar la señal: revisa el logo y enciéndela");
        }
    }

    fn det_logo(&mut self, ui: &mut egui::Ui, v: Option<&Vista>) {
        let aprendido = v.is_some_and(|v| v.logo.is_some());
        let mut aprender = false;
        seccion(ui, "2", "Logo del canal", aprendido, |ui| {
            ui.horizontal(|ui| {
                // El logo aprendido (sus bordes).
                let (rect, _) = ui.allocate_exact_size(Vec2::new(96.0, 54.0), Sense::hover());
                ui.painter().rect_filled(rect, 4.0, Color32::BLACK);
                if let Some(logo) = v.and_then(|v| v.logo.as_ref()) {
                    let clave = Arc::as_ptr(logo) as usize;
                    if self.det.tex_logo.as_ref().map(|t| t.0) != Some(clave) {
                        let img = egui::ColorImage::from_gray([logo.0, logo.1], &logo.2);
                        let t = ui.ctx().load_texture("logo-aprendido", img, egui::TextureOptions::LINEAR);
                        self.det.tex_logo = Some((clave, t));
                    }
                    if let Some((_, t)) = &self.det.tex_logo {
                        egui::Image::new(t).fit_to_exact_size(rect.size()).paint_at(ui, rect);
                    }
                } else {
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "sin logo",
                        FontId::proportional(11.0),
                        MUY_TENUE,
                    );
                }
                ui.vertical(|ui| match v.and_then(|v| v.aprendiendo) {
                    Some(a) => {
                        ui.add(egui::ProgressBar::new(a).desired_width(190.0).text("Aprendiendo…"));
                        ui.label(RichText::new("No cambies de canal estos segundos.").size(11.5).color(TENUE));
                    }
                    None => {
                        let puede = v.is_some_and(|v| v.cuadro.is_some());
                        let t = if aprendido { "Aprender de nuevo" } else { "Aprender el logo ahora" };
                        let b = if aprendido {
                            dialogo::boton_secundario(t)
                        } else {
                            dialogo::boton_principal(t, Tono::Normal)
                        };
                        if ui
                            .add_enabled(puede, b.min_size(Vec2::new(190.0, 30.0)))
                            .on_hover_text("Con el programa al aire (no en comerciales). Tarda 8 segundos.")
                            .on_disabled_hover_text("Primero tiene que llegar la señal")
                            .clicked()
                        {
                            aprender = true;
                        }
                        let nota = if aprendido {
                            "Así ve Relevo el logo. Si mueves la zona, hay que aprenderlo otra vez."
                        } else {
                            "Marca el logo en la imagen y púlsalo con el programa al aire."
                        };
                        ui.add(egui::Label::new(RichText::new(nota).size(11.5).color(MUY_TENUE)).wrap());
                    }
                });
            });
        });
        if aprender {
            if let Some(d) = &self.detector {
                d.aprender();
            }
        }
    }

    fn det_accion(&mut self, ui: &mut egui::Ui) {
        seccion(ui, "3", "Al detectar el corte", true, |ui| {
            let d = &mut self.cfg.deteccion;
            ui.radio_value(&mut d.accion, "avisar".into(), "Solo avisar (cartel azul en la pantalla)");
            ui.radio_value(&mut d.accion, "iniciar".into(), "Iniciar la tanda sola");
            ui.add_space(4.0);
            ui.label(RichText::new("Cuando vuelve el logo con publicidad al aire").size(12.5).color(TENUE));
            ui.radio_value(&mut d.al_volver, "avisar".into(), "Solo avisar (cartel verde)");
            ui.radio_value(&mut d.al_volver, "volver".into(), "Volver al canal (corta la tanda)")
                .on_hover_text("Solo en modo Transmisión; en modo Publicidad, avisa");
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Dar el corte tras").size(12.5).color(TENUE));
                ui.add(egui::DragValue::new(&mut d.segundos).range(0.4..=20.0).speed(0.1).suffix(" s"))
                    .on_hover_text("Segundos seguidos sin logo para dar el corte por empezado");
                ui.label(RichText::new("sin logo").size(12.5).color(TENUE));
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("No repetir antes de").size(12.5).color(TENUE));
                ui.add(egui::DragValue::new(&mut d.espera_min).range(0.0..=60.0).speed(0.5).suffix(" min"))
                    .on_hover_text("Tiempo mínimo entre un corte y el siguiente");
            });
        });
    }

    fn det_registro(&mut self, ui: &mut egui::Ui) {
        if self.det.registro.is_empty() {
            return;
        }
        ui.label(RichText::new("ÚLTIMOS AVISOS").size(11.0).family(tema::negrita()).color(TENUE));
        for (hora, texto) in self.det.registro.iter().take(6) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(hora).monospace().size(11.5).color(MUY_TENUE));
                ui.label(RichText::new(texto).size(12.0).color(TEXTO));
            });
        }
    }
}
