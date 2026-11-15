//! **Vista compacta**: lo justo para operar un corte desde otra sala o con pantalla
//! táctil. Imagen de lo que está al aire, cuenta regresiva y cuatro botones grandes.

use super::*;

impl Ventana {
    pub(super) fn pantalla_compacta(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let actual = e.actual.and_then(|id| e.item(id)).cloned();
        let proximo = e.proximo.and_then(|id| e.item(id)).cloned();
        let ancho = ui.available_width();
        let bloqueado = !self.ctl.tengo_mando(e);

        ui.vertical_centered(|ui| {
            ui.set_max_width(ancho.min(1100.0));

            // Arriba: imagen y tiempo.
            let borde = match e.fase {
                FaseVista::AlAire => AIRE,
                FaseVista::Pausado => AMBAR,
                FaseVista::Tv => BORDE,
            };
            egui::Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(2.0, borde))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal_top(|ui| {
                        let alto = (ui.available_height().min(560.0) * 0.45).clamp(170.0, 300.0);
                        self.monitor_programa_de(ui, e, alto);
                        ui.add_space(18.0);
                        ui.vertical(|ui| {
                            tema::titulo(ui, "AL AIRE");
                            match &actual {
                                Some(item) => {
                                    ui.label(
                                        RichText::new(nombre_corto(&item.clip))
                                            .size(34.0)
                                            .strong()
                                            .family(crate::tema::negrita()),
                                    );
                                    let total = item.duracion.as_secs_f64();
                                    let falta = (total - e.transcurrido.as_secs_f64()).max(0.0);
                                    let color = if e.fase == FaseVista::Pausado { AMBAR } else { AIRE };
                                    ui.label(
                                        RichText::new(format!("-{}", mmss(falta)))
                                            .font(FontId::monospace(64.0))
                                            .color(color),
                                    );
                                    let frac =
                                        if total > 0.0 { (e.transcurrido.as_secs_f64() / total) as f32 } else { 0.0 };
                                    barra_progreso(ui, frac.clamp(0.0, 1.0), e.fase, 16.0);
                                }
                                None => {
                                    ui.label(
                                        RichText::new("Señal de TV")
                                            .size(34.0)
                                            .strong()
                                            .family(crate::tema::negrita())
                                            .color(TV),
                                    );
                                    let n = e.lista.len();
                                    let t = if e.modo == ModoTanda::Auto {
                                        format!("Modo AUTO · {n} spots listos para el corte")
                                    } else {
                                        format!("{n} spots en la tanda")
                                    };
                                    ui.label(RichText::new(t).size(18.0).color(TENUE));
                                }
                            }
                            ui.add_space(6.0);
                            match &proximo {
                                Some(p) => ui.label(
                                    RichText::new(format!(
                                        "Sigue: {}  ({})",
                                        nombre_corto(&p.clip),
                                        mmss(p.duracion.as_secs_f64())
                                    ))
                                    .size(18.0),
                                ),
                                None if actual.is_some() => {
                                    ui.label(RichText::new("Al terminar vuelve la señal de TV").size(16.0).color(TV))
                                }
                                None => ui.label(""),
                            };
                        });
                    });
                });

            ui.add_space(14.0);

            // Botones grandes.
            let alto_boton = 96.0;
            let n = 4.0;
            let sep = 12.0;
            let w = ((ui.available_width() - sep * (n - 1.0)) / n).max(140.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = sep;
                let tam = Vec2::new(w, alto_boton);
                ui.add_enabled_ui(!bloqueado, |ui| {
                    // Iniciar / pausar / reanudar.
                    let (icono, texto, fondo) = match e.fase {
                        FaseVista::AlAire => ("⏸", "PAUSA", AMBAR_OSCURO),
                        FaseVista::Pausado => ("⏵", "SEGUIR", AIRE_OSCURO),
                        FaseVista::Tv => ("⏵", "INICIAR TANDA", AIRE_OSCURO),
                    };
                    let b = egui::Button::new(
                        RichText::new(format!("{icono}  {texto}"))
                            .size(22.0)
                            .strong()
                            .family(crate::tema::negrita())
                            .color(Color32::WHITE),
                    )
                    .fill(fondo)
                    .min_size(tam)
                    .corner_radius(CornerRadius::same(12));
                    if ui.add_enabled(!e.lista.is_empty() || e.fase != FaseVista::Tv, b).clicked() {
                        self.ctl.enviar(Orden::PlayPausa(None));
                    }
                    let b = egui::Button::new(
                        RichText::new("⏭  SIGUIENTE").size(22.0).strong().family(crate::tema::negrita()),
                    )
                    .min_size(tam)
                    .corner_radius(CornerRadius::same(12));
                    if ui.add_enabled(e.fase != FaseVista::Tv, b).clicked() {
                        self.ctl.enviar(Orden::Siguiente);
                    }
                    let auto = e.modo == ModoTanda::Auto;
                    let b = egui::Button::new(
                        RichText::new(if auto { "⟳  MODO AUTO" } else { "✋  MODO MANUAL" })
                            .size(20.0)
                            .strong()
                            .family(crate::tema::negrita()),
                    )
                    .fill(if auto { ACENTO_OSCURO } else { TARJETA })
                    .stroke(Stroke::new(1.0, if auto { ACENTO } else { BORDE }))
                    .min_size(tam)
                    .corner_radius(CornerRadius::same(12));
                    if ui.add(b).on_hover_text("Cambiar entre AUTO (tanda por pauta) y MANUAL").clicked() {
                        self.ctl.enviar(Orden::Modo(if auto { ModoTanda::Manual } else { ModoTanda::Auto }));
                    }
                });
                // Volver a TV (si está activado en Ajustes → Interfaz).
                if self.cfg.interfaz.boton_volver_a_tv {
                    let b = egui::Button::new(
                        RichText::new("📺  VOLVER A TV")
                            .size(22.0)
                            .strong()
                            .family(crate::tema::negrita())
                            .color(Color32::WHITE),
                    )
                    .fill(TV_OSCURO)
                    .stroke(Stroke::new(2.0, TV))
                    .min_size(tam)
                    .corner_radius(CornerRadius::same(12));
                    if ui.add(b).on_hover_text("Siempre funciona (Esc)").clicked() {
                        self.ctl.enviar(Orden::VolverATv);
                    }
                }
            });
            if bloqueado {
                if let Some((_, quien)) = &e.mando {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("🎮 El mando lo tiene {quien}: solo puedes volver a TV."))
                            .size(15.0)
                            .color(AMBAR),
                    );
                }
            }

            ui.add_space(14.0);
            // La tanda que viene.
            tema::marco_tarjeta().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    tema::titulo(ui, "TANDA");
                    if e.lista.is_empty() {
                        ui.label(RichText::new("Vacía").size(16.0).color(MUY_TENUE));
                    }
                    for (i, item) in e.lista.iter().enumerate().take(8) {
                        let al_aire = Some(item.id) == e.actual;
                        let color = if al_aire { AIRE } else { TEXTO };
                        ui.label(
                            RichText::new(format!(
                                "{}{}.  {}   {}",
                                if al_aire { "⏺ " } else { "" },
                                i + 1,
                                nombre_corto(&item.clip),
                                mmss(item.duracion.as_secs_f64())
                            ))
                            .size(17.0)
                            .color(color),
                        );
                    }
                    if e.lista.len() > 8 {
                        ui.label(RichText::new(format!("… y {} más", e.lista.len() - 8)).size(14.0).color(TENUE));
                    }
                });
            });
        });
    }
}
