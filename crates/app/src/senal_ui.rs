//! Monitores de la sección «Al aire», como en un mezclador de video:
//! - **PROGRAMA** (izquierda): lo que sale al aire. Con publicidad, el spot; con la
//!   señal de TV, el canal (por IPTV en modo Transmisión; si no, la «señal del canal»
//!   elegida con ⚙).
//! - **PREVIEW** (derecha): lo que *no* está al aire. Con publicidad, el canal; con la
//!   señal de TV, el spot que sale al dar Play (el primero, o donde se detuvo).
//!
//! Nada de esto suena ni sale al aire.

use super::*;
use crate::senal::{self, EstadoSenal, Fuente, Lector};

#[derive(Default)]
pub(super) struct Senales {
    /// Señal del canal elegida con ⚙ (en Transmisión, si no hay otra, el canal IPTV).
    canal: Option<Arc<Lector>>,
    /// El canal IPTV (solo en modo Transmisión: es lo que sale al aire).
    iptv: Option<Arc<Lector>>,
    /// Spot que se muestra en el PREVIEW (y su archivo en esta PC).
    cue: Option<(String, Option<PathBuf>)>,
    tex_preview: Option<(u64, egui::TextureHandle)>,
    tex_programa: Option<(u64, egui::TextureHandle)>,
    /// Lo que se escribe en el selector (se aplica con «Usar»).
    origen: String,
    /// Fuente elegida en el selector (todavía sin aplicar).
    tipo: Option<String>,
}

/// Dibuja un cuadro del lector en `rect` (con su textura, que se actualiza solo si
/// llegó un cuadro nuevo). Devuelve `false` si todavía no hay imagen.
fn pintar(
    ui: &egui::Ui,
    lector: &Lector,
    tex: &mut Option<(u64, egui::TextureHandle)>,
    nombre: &str,
    rect: egui::Rect,
) -> bool {
    let Some((serie, rgb)) = lector.ultimo() else { return false };
    if tex.as_ref().map(|t| t.0) != Some(serie) {
        let img = egui::ColorImage::from_rgb([senal::ANCHO, senal::ALTO], &rgb);
        match tex {
            Some((s, t)) => {
                t.set(img, egui::TextureOptions::LINEAR);
                *s = serie;
            }
            None => *tex = Some((serie, ui.ctx().load_texture(nombre, img, egui::TextureOptions::LINEAR))),
        }
    }
    if let Some((_, t)) = tex {
        egui::Image::new(&*t).corner_radius(6).fit_to_exact_size(rect.size()).paint_at(ui, rect);
    }
    true
}

/// Etiqueta en una esquina del monitor.
fn etiqueta(p: &egui::Painter, rect: egui::Rect, texto: &str, color: Color32) {
    let g = p.layout_no_wrap(texto.to_string(), FontId::new(10.5, tema::negrita()), color);
    let caja = egui::Rect::from_min_size(rect.left_top() + Vec2::new(6.0, 6.0), g.size() + Vec2::new(12.0, 6.0));
    p.rect_filled(caja, 4.0, Color32::from_black_alpha(170));
    p.galley(caja.min + Vec2::new(6.0, 3.0), g, color);
}

impl Ventana {
    /// Abre o suelta los lectores según la configuración (una vez por cuadro de la
    /// interfaz; no hace nada si nada cambió).
    pub(super) fn seguir_senales(&mut self, ctx: &egui::Context) {
        let Some(ffmpeg) = self.vista.ffmpeg().map(Path::to_path_buf) else {
            self.sen.canal = None;
            self.sen.iptv = None;
            return;
        };
        let iptv = self.cfg.iptv.direccion.clone();
        // En Transmisión (solo en la PC principal, que es la que tiene el canal).
        let transmision = self.cfg.en_transmision() && !self.ctl.es_remoto();
        let p = &self.cfg.preview;
        let quiero = Fuente::nueva(&p.fuente, &p.origen, p.monitor, &iptv)
            .or_else(|| transmision.then(|| Fuente::nueva("iptv", "", 0, &iptv)).flatten());
        if self.sen.canal.as_ref().map(|l| &l.fuente) != quiero.as_ref() {
            self.sen.canal = quiero.map(|f| senal::lector(&f, ffmpeg.clone()));
            self.sen.tex_preview = None;
            self.sen.tex_programa = None;
        }
        let quiero = transmision.then(|| Fuente::nueva("iptv", "", 0, &iptv)).flatten();
        if self.sen.iptv.as_ref().map(|l| &l.fuente) != quiero.as_ref() {
            self.sen.iptv = quiero.map(|f| senal::lector(&f, ffmpeg.clone()));
            self.sen.tex_programa = None;
        }
        if self.sen.canal.is_some() || self.sen.iptv.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Con la señal de TV al aire: el canal en vivo en el monitor de programa (en
    /// Transmisión, el canal IPTV; si no, la señal del canal). Devuelve `false` si no
    /// hay ninguna (se dibuja «Señal de TV»).
    pub(super) fn programa_en_vivo(&mut self, ui: &egui::Ui, rect: egui::Rect) -> bool {
        let Some(l) = self.sen.iptv.clone().or_else(|| self.sen.canal.clone()) else { return false };
        let p = ui.painter();
        p.rect_filled(rect, 6.0, Color32::BLACK);
        if !pintar(ui, &l, &mut self.sen.tex_programa, "programa-canal", rect) {
            let t = match l.estado() {
                EstadoSenal::Error(_) => "📺  Canal: sin imagen",
                _ => "📺  Canal: conectando…",
            };
            p.text(rect.center(), Align2::CENTER_CENTER, t, FontId::proportional(13.0), TV);
        }
        true
    }

    /// Monitor PREVIEW: lo que *no* está al aire. Con publicidad, el canal; con la señal
    /// de TV, el spot que sale al dar Play. ⚙ elige la señal del canal.
    pub(super) fn monitor_preview(&mut self, ui: &mut egui::Ui, e: &Instantanea, alto: f32) {
        let tam = Vec2::new(alto * 16.0 / 9.0, alto);
        let (rect, resp) = ui.allocate_exact_size(tam, Sense::click());
        ui.painter().rect_filled(rect, 6.0, Color32::from_rgb(0x0C, 0x0E, 0x13));
        let mostrar_canal = e.fase != FaseVista::Tv;
        // Con la señal de TV al aire: el spot que saldría (cuadro quieto del principio).
        let cola = if mostrar_canal { None } else { self.en_cola(e).and_then(|id| e.item(id)).cloned() };
        let lector = if mostrar_canal { self.sen.canal.clone() } else { None };
        let pie: Option<String>;
        match &cola {
            Some(item) => {
                if self.sen.cue.as_ref().map(|(c, _)| c) != Some(&item.clip) {
                    let ruta = self.ruta_de_clip(&item.clip, e);
                    self.sen.cue = Some((item.clip.clone(), ruta));
                }
                let archivo = self.sen.cue.as_ref().and_then(|(_, r)| r.clone());
                let segundo = item.duracion.as_secs_f64().min(0.5);
                self.monitor_cue.seguir(ui.ctx(), archivo.as_deref().map(|a| (a, segundo)), true);
                match self.monitor_cue.textura() {
                    Some(t) => {
                        egui::Image::new(t).corner_radius(6).fit_to_exact_size(tam).paint_at(ui, rect);
                    }
                    None => {
                        let clip = item.clip.clone();
                        self.pintar_miniatura(ui, rect, Some(&clip), &clip, false);
                    }
                }
                let antes = if e.retomar.is_some() { "⏵ reinicia: " } else { "⏵ " };
                pie = Some(format!("{antes}{}", nombre_corto(&item.clip)));
            }
            None => {
                self.sen.cue = None;
                self.monitor_cue.seguir(ui.ctx(), None, true);
                let hay_imagen = match &lector {
                    Some(l) => pintar(ui, l, &mut self.sen.tex_preview, "preview", rect),
                    None => false,
                };
                let p = ui.painter();
                if !hay_imagen {
                    let (t, c) = if !mostrar_canal {
                        ("La tanda está vacía".to_string(), MUY_TENUE)
                    } else {
                        match lector.as_ref().map(|l| l.estado()) {
                            None if self.vista.ffmpeg().is_none() => ("Hace falta ffmpeg".to_string(), AMBAR),
                            None => ("Señal del canal sin elegir\n⚙ para elegirla".to_string(), MUY_TENUE),
                            Some(EstadoSenal::Error(e)) => (format!("⚠ {e}"), AIRE),
                            Some(_) => ("Conectando…".to_string(), TENUE),
                        }
                    };
                    let g = p.layout(t, FontId::proportional(11.5), c, rect.width() - 24.0);
                    p.galley(rect.center() - g.size() / 2.0, g, c);
                }
                pie = lector.as_ref().map(|l| l.fuente.describir());
            }
        }
        let p = ui.painter();
        let (titulo, color) = if mostrar_canal {
            ("PREVIEW · CANAL", TV)
        } else if e.retomar.is_some() {
            ("PREVIEW · QUEDÓ AQUÍ", AMBAR)
        } else {
            ("PREVIEW · AL DAR PLAY", ACENTO)
        };
        etiqueta(p, rect, titulo, color);
        // Abajo: qué se ve.
        if let Some(pie) = pie {
            let texto: String = pie.chars().take(40).collect();
            let g = p.layout_no_wrap(texto, FontId::proportional(10.0), Color32::from_white_alpha(200));
            let caja = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 6.0, rect.bottom() - g.size().y - 10.0),
                g.size() + Vec2::new(10.0, 4.0),
            );
            p.rect_filled(caja, 3.0, Color32::from_black_alpha(150));
            p.galley(caja.min + Vec2::new(5.0, 2.0), g, Color32::WHITE);
        }
        p.rect_stroke(rect, 6.0, Stroke::new(1.0, BORDE), StrokeKind::Inside);
        let boton = self.boton_senal_canal(ui, rect);
        if resp.clicked() && self.sen.canal.is_none() && mostrar_canal {
            egui::Popup::open_id(ui.ctx(), boton.with("popup"));
        }
        let _ = resp.on_hover_text(
            "Preview: lo que no está al aire. Con publicidad, el canal; con la señal de TV, el spot que sale al dar Play.\nSin sonido.",
        );
    }

    /// Botón ⚙ (arriba a la derecha de un monitor) para elegir la señal del canal.
    /// Devuelve su id.
    pub(super) fn boton_senal_canal(&mut self, ui: &mut egui::Ui, rect: egui::Rect) -> egui::Id {
        let r_boton = egui::Rect::from_min_size(rect.right_top() + Vec2::new(-30.0, 6.0), Vec2::splat(24.0));
        let boton = ui.put(
            r_boton,
            egui::Button::new(RichText::new("⚙").size(13.0)).fill(Color32::from_black_alpha(170)).corner_radius(4),
        );
        let boton = boton.on_hover_text("Elegir la señal del canal");
        egui::Popup::menu(&boton)
            .id(boton.id.with("popup"))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| self.selector_preview(ui));
        boton.id
    }

    fn selector_preview(&mut self, ui: &mut egui::Ui) {
        ui.set_width(300.0);
        let actual = self.cfg.preview.clone();
        if self.sen.tipo.is_none() {
            // Al abrir: lo que está elegido ahora.
            self.sen.tipo = Some(actual.fuente.clone());
            self.sen.origen = actual.origen.clone();
        }
        let tipo = self.sen.tipo.clone().unwrap_or_else(|| actual.fuente.clone());
        ui.label(RichText::new("Señal del canal").size(13.0).family(tema::negrita()).color(TEXTO));
        ui.label(
            RichText::new(
                "Modo Publicidad: se ve en el monitor con la señal de TV. Modo Transmisión: en PREVIEW mientras hay publicidad al aire.",
            )
            .size(11.0)
            .color(MUY_TENUE),
        );
        ui.add_space(4.0);
        let mut elegido = tipo.clone();
        for (k, t) in [
            ("ninguna", "Ninguna"),
            ("capturadora", "🎥 Capturadora"),
            ("web", "🌐 Página web o video"),
            ("iptv", "📺 Canal IPTV"),
            ("pantalla", "🖥 Pantalla"),
            ("archivo", "🎞 Archivo"),
        ] {
            ui.radio_value(&mut elegido, k.to_string(), t);
        }
        if elegido != tipo {
            self.sen.tipo = Some(elegido.clone());
            self.sen.origen = if elegido == actual.fuente { actual.origen.clone() } else { String::new() };
            if elegido == "capturadora" || elegido == "pantalla" {
                self.buscar_dispositivos();
            }
        }
        ui.separator();
        let mut nueva: Option<crate::config::PreviewCfg> = None;
        let caps = self.det.capturadoras.lock().expect("capturadoras").clone();
        let mons = self.det.monitores.lock().expect("monitores").clone();
        match elegido.as_str() {
            "ninguna" => {
                if actual.fuente != "ninguna" {
                    nueva = Some(crate::config::PreviewCfg::default());
                }
            }
            "iptv" => {
                if self.cfg.iptv.direccion.trim().is_empty() {
                    ui.label(
                        RichText::new("Primero indica el canal en ⚙ Ajustes → Señal por IPTV.").size(11.5).color(AMBAR),
                    );
                } else if actual.fuente != "iptv" {
                    nueva =
                        Some(crate::config::PreviewCfg { fuente: "iptv".into(), origen: String::new(), monitor: 0 });
                }
            }
            "capturadora" => match &caps {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Buscando capturadoras…");
                    });
                }
                Some(l) if l.is_empty() => {
                    ui.label(RichText::new("No hay capturadoras de video conectadas.").size(11.5).color(TENUE));
                }
                Some(l) => {
                    for n in l {
                        let sel = actual.fuente == "capturadora" && actual.origen == *n;
                        if ui.selectable_label(sel, n).clicked() && !sel {
                            nueva = Some(crate::config::PreviewCfg {
                                fuente: "capturadora".into(),
                                origen: n.clone(),
                                monitor: 0,
                            });
                        }
                    }
                }
            },
            "pantalla" => {
                let mut opciones = vec![(0u32, "Todo el escritorio".to_string())];
                opciones.extend(mons.iter().flatten().map(|m| (m.numero, m.describir())));
                for (n, t) in opciones {
                    let sel = actual.fuente == "pantalla" && actual.monitor == n;
                    if ui.selectable_label(sel, t).clicked() && !sel {
                        nueva = Some(crate::config::PreviewCfg {
                            fuente: "pantalla".into(),
                            origen: String::new(),
                            monitor: n,
                        });
                    }
                }
            }
            otro => {
                // Web o archivo: se escribe y se aplica con «Usar».
                let pista = if otro == "web" { "https://… (página o video .m3u8)" } else { "ruta del video" };
                ui.horizontal(|ui| {
                    let r =
                        ui.add(egui::TextEdit::singleline(&mut self.sen.origen).desired_width(200.0).hint_text(pista));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if otro == "archivo" && ui.button("…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_file() {
                            self.sen.origen = p.display().to_string();
                        }
                    }
                    let o = self.sen.origen.trim().to_string();
                    if (ui.add_enabled(!o.is_empty(), egui::Button::new("Usar")).clicked() || enter) && !o.is_empty() {
                        nueva = Some(crate::config::PreviewCfg { fuente: otro.to_string(), origen: o, monitor: 0 });
                    }
                });
            }
        }
        ui.separator();
        // Atajo: lo mismo que mira la detección de corte (comparten el mismo ffmpeg).
        let d = &self.cfg.deteccion;
        let misma = d.fuente == actual.fuente && d.origen == actual.origen && d.monitor == actual.monitor;
        if ui.add_enabled(!misma, egui::Button::new("✂ Lo mismo que la detección de corte")).clicked() {
            nueva = Some(crate::config::PreviewCfg {
                fuente: d.fuente.clone(),
                origen: d.origen.clone(),
                monitor: d.monitor,
            });
        }
        ui.label(RichText::new("Sin sonido. No sale al aire.").size(11.0).color(MUY_TENUE));
        if let Some(n) = nueva {
            self.sen.tipo = None;
            self.cfg.preview = n.clone();
            self.ajustes.guardar_con(|c| c.preview = n.clone());
        }
    }
}
