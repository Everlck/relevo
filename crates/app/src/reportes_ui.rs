//! Pantalla **Reportes**: qué salió al aire en un periodo, resumen por cliente con el
//! cumplimiento de la pauta, alertas de contratos, y exportar a Excel o PDF.

use egui_extras::{Column, TableBuilder};
use nucleo::controlador::Ranura;
use nucleo::pauta::formato;
use nucleo::reportes::{self as rep, Reporte};

use super::*;
use crate::exportar;

/// Estado de la pantalla de reportes.
pub(super) struct EstadoReportes {
    periodo: &'static str,
    desde: time::Date,
    hasta: time::Date,
    texto_desde: String,
    texto_hasta: String,
    pedido: Option<Ranura>,
    pub reporte: Option<Reporte>,
    error: Option<String>,
    /// Cliente elegido (None = todos).
    cliente: Option<String>,
    /// Hay que pedir el reporte (al abrir la pantalla o cambiar el periodo).
    pendiente: bool,
    /// Ver el simulador en vez de lo emitido.
    simular: bool,
    dia_simulado: time::Date,
    texto_dia: String,
}

impl EstadoReportes {
    pub fn new(hoy: time::Date) -> Self {
        let (d, h) = rep::periodo("mes", hoy);
        EstadoReportes {
            periodo: "mes",
            desde: d,
            hasta: h,
            texto_desde: formato::texto_fecha_dma(d),
            texto_hasta: formato::texto_fecha_dma(h),
            pedido: None,
            reporte: None,
            error: None,
            cliente: None,
            pendiente: true,
            simular: false,
            dia_simulado: hoy,
            texto_dia: formato::texto_fecha_dma(hoy),
        }
    }

    fn fijar(&mut self, d: time::Date, h: time::Date) {
        self.desde = d.min(h);
        self.hasta = h.max(d);
        self.texto_desde = formato::texto_fecha_dma(self.desde);
        self.texto_hasta = formato::texto_fecha_dma(self.hasta);
        self.pendiente = true;
    }
}

impl Ventana {
    fn ahora_local(&self) -> (time::Date, time::Time) {
        let t = time::OffsetDateTime::now_utc().to_offset(self.zona);
        (t.date(), t.time())
    }

    /// Cuando se abre la pantalla: el reporte se vuelve a pedir.
    pub(super) fn al_abrir_reportes(&mut self) {
        self.reportes.pendiente = true;
    }

    pub(super) fn pantalla_reportes(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("📊 Lo que salió al aire").size(14.0))
                        .selected(!self.reportes.simular),
                )
                .clicked()
            {
                self.reportes.simular = false;
                self.reportes.pendiente = true;
            }
            if ui
                .add(egui::Button::new(RichText::new("🔮 Simular un día").size(14.0)).selected(self.reportes.simular))
                .on_hover_text("Qué saldría en cada corte según la pauta, antes de que pase")
                .clicked()
            {
                self.reportes.simular = true;
            }
        });
        ui.add_space(6.0);
        if self.reportes.simular {
            self.simulador(ui, e);
            return;
        }
        // Pedir / recibir el reporte.
        if self.reportes.pendiente {
            self.reportes.pendiente = false;
            let (hoy, ahora) = self.ahora_local();
            self.reportes.pedido = Some(self.ctl.reporte(self.reportes.desde, self.reportes.hasta, hoy, ahora));
            self.reportes.error = None;
        }
        if let Some(p) = &self.reportes.pedido {
            let listo = p.lock().expect("ranura").take();
            match listo {
                Some(Ok(r)) => {
                    if let Some(c) = &self.reportes.cliente {
                        if !r.clientes.iter().any(|x| &x.cliente == c) {
                            self.reportes.cliente = None;
                        }
                    }
                    self.reportes.reporte = Some(r);
                    self.reportes.pedido = None;
                }
                Some(Err(m)) => {
                    self.reportes.error = Some(m);
                    self.reportes.pedido = None;
                }
                None => ui.ctx().request_repaint_after(Duration::from_millis(100)),
            }
        }

        self.barra_reportes(ui, e);
        ui.add_space(8.0);

        let reporte = self.reportes.reporte.clone();
        let Some(r) = reporte else {
            if let Some(m) = &self.reportes.error {
                mensaje_vacio(ui, "⚠", "No se pudo armar el reporte", m);
            } else {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| ui.add(egui::Spinner::new().size(24.0)));
            }
            return;
        };
        let vista = match &self.reportes.cliente {
            Some(c) => r.de_cliente(c),
            None => r.clone(),
        };

        StripBuilder::new(ui)
            .size(Size::remainder().at_least(520.0))
            .size(Size::relative(0.40).at_least(420.0))
            .horizontal(|mut strip| {
                strip.cell(|ui| {
                    tema::marco_columna().show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        self.tabla_salidas(ui, &vista);
                    });
                });
                strip.cell(|ui| {
                    tema::marco_columna().show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        egui::ScrollArea::vertical().id_salt("rep_lado").auto_shrink([false, false]).show(ui, |ui| {
                            self.resumen_clientes(ui, &r);
                            ui.add_space(10.0);
                            alertas(ui, &e.alertas);
                        });
                    });
                });
            });
    }

    fn barra_reportes(&mut self, ui: &mut egui::Ui, _e: &Instantanea) {
        let (hoy, _) = self.ahora_local();
        ui.horizontal(|ui| {
            tema::titulo(ui, "REPORTES");
            ui.add_space(8.0);
            for (clave, texto) in [
                ("hoy", "Hoy"),
                ("ayer", "Ayer"),
                ("semana", "Esta semana"),
                ("mes", "Este mes"),
                ("mes_pasado", "Mes pasado"),
            ] {
                if ui.add(egui::Button::new(texto).selected(self.reportes.periodo == clave)).clicked() {
                    self.reportes.periodo = clave;
                    let (d, h) = rep::periodo(clave, hoy);
                    self.reportes.fijar(d, h);
                }
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Del").color(TENUE));
            let r1 = ui.add(egui::TextEdit::singleline(&mut self.reportes.texto_desde).desired_width(100.0));
            ui.label(RichText::new("al").color(TENUE));
            let r2 = ui.add(egui::TextEdit::singleline(&mut self.reportes.texto_hasta).desired_width(100.0));
            let enter = (r1.lost_focus() || r2.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Ver").clicked() || enter {
                match (
                    formato::leer_fecha_dma(&self.reportes.texto_desde),
                    formato::leer_fecha_dma(&self.reportes.texto_hasta),
                ) {
                    (Some(d), Some(h)) => {
                        self.reportes.periodo = "";
                        self.reportes.fijar(d, h);
                    }
                    _ => self.avisar_error("Escribe las fechas como día/mes/año, por ejemplo 01/11/2026"),
                }
            }
            if ui.button("↻").on_hover_text("Actualizar").clicked() {
                self.reportes.pendiente = true;
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let hay = self.reportes.reporte.is_some();
                if ui
                    .add_enabled(hay, egui::Button::new("⬇ PDF").fill(ACENTO_OSCURO))
                    .on_hover_text("Constancia de emisión (del cliente elegido, o de todos)")
                    .clicked()
                {
                    self.exportar(false);
                }
                if ui
                    .add_enabled(hay, egui::Button::new("⬇ Excel").fill(TV_OSCURO))
                    .on_hover_text("Resumen por cliente y detalle de cada salida")
                    .clicked()
                {
                    self.exportar(true);
                }
                // Cliente.
                let clientes: Vec<String> = self
                    .reportes
                    .reporte
                    .as_ref()
                    .map(|r| r.clientes.iter().map(|c| c.cliente.clone()).collect())
                    .unwrap_or_default();
                let actual = self.reportes.cliente.clone().unwrap_or_else(|| "Todos los clientes".into());
                egui::ComboBox::from_id_salt("rep_cliente").width(200.0).selected_text(actual).show_ui(ui, |ui| {
                    if ui.selectable_label(self.reportes.cliente.is_none(), "Todos los clientes").clicked() {
                        self.reportes.cliente = None;
                    }
                    for c in clientes {
                        let sel = self.reportes.cliente.as_deref() == Some(c.as_str());
                        if ui.selectable_label(sel, &c).clicked() {
                            self.reportes.cliente = Some(c);
                        }
                    }
                });
                ui.label(RichText::new("Cliente").color(TENUE));
            });
        });
    }

    fn tabla_salidas(&mut self, ui: &mut egui::Ui, r: &Reporte) {
        let total: f64 = r.salidas.iter().map(|s| s.salida.emitido).sum();
        let cortadas = r.salidas.iter().filter(|s| !s.salida.completo).count();
        ui.horizontal(|ui| {
            tema::titulo(ui, "SALIDAS AL AIRE");
            ui.label(RichText::new(exportar::periodo_texto(r)).size(12.0).color(TENUE));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if cortadas > 0 {
                    ui.label(RichText::new(format!("· {cortadas} cortadas")).size(12.0).color(AMBAR));
                }
                ui.label(
                    RichText::new(format!("{} salidas · {} al aire", r.salidas.len(), mmss(total)))
                        .size(12.0)
                        .color(TEXTO),
                );
            });
        });
        ui.add_space(4.0);
        if r.salidas.is_empty() {
            mensaje_vacio(
                ui,
                "📊",
                "No hay salidas al aire en este periodo",
                "Cada spot que sale al aire queda anotado aquí (desde esta versión de Relevo).",
            );
            return;
        }
        let varios_dias = r.desde != r.hasta;
        let alto = ui.available_height();
        let mut tabla = TableBuilder::new(ui)
            .striped(true)
            .max_scroll_height(alto)
            .cell_layout(Layout::left_to_right(Align::Center));
        if varios_dias {
            tabla = tabla.column(Column::exact(82.0));
        }
        tabla
            .column(Column::exact(66.0))
            .column(Column::initial(150.0).at_least(100.0).clip(true))
            .column(Column::remainder().at_least(140.0).clip(true))
            .column(Column::exact(52.0))
            .column(Column::exact(118.0))
            .column(Column::exact(64.0))
            .column(Column::exact(110.0).clip(true))
            .header(22.0, |mut h| {
                let mut cols = vec!["HORA", "CLIENTE", "SPOT", "DUR.", "ESTADO", "MODO", "QUIÉN"];
                if varios_dias {
                    cols.insert(0, "FECHA");
                }
                for t in cols {
                    h.col(|ui| {
                        ui.label(RichText::new(t).size(11.0).color(MUY_TENUE));
                    });
                }
            })
            .body(|body| {
                // Lo más reciente arriba.
                let filas: Vec<&rep::SalidaFecha> = r.salidas.iter().rev().collect();
                body.rows(26.0, filas.len(), |mut row| {
                    let s = filas[row.index()];
                    let x = &s.salida;
                    if varios_dias {
                        row.col(|ui| {
                            ui.label(RichText::new(exportar::dma(&s.fecha)).size(12.5).color(TENUE));
                        });
                    }
                    row.col(|ui| {
                        ui.label(RichText::new(&x.hora).monospace().size(12.5));
                    });
                    row.col(|ui| {
                        ui.label(RichText::new(&x.cliente).size(13.0).strong().family(crate::tema::negrita()));
                    });
                    row.col(|ui| {
                        ui.label(RichText::new(nombre_corto(&x.clip)).size(12.5).color(TENUE)).on_hover_text(&x.clip);
                    });
                    row.col(|ui| {
                        ui.label(RichText::new(mmss(x.duracion)).monospace().size(12.5));
                    });
                    row.col(|ui| {
                        if x.completo {
                            ui.label(RichText::new("✔ Completo").size(12.5).color(TV));
                        } else {
                            ui.label(RichText::new(format!("✂ Cortado a {}", mmss(x.emitido))).size(12.5).color(AMBAR));
                        }
                    });
                    row.col(|ui| {
                        let c = if x.modo == "AUTO" { ACENTO } else { TENUE };
                        let r = ui.label(RichText::new(&x.modo).size(12.0).color(c));
                        if let Some(p) = &x.pase {
                            r.on_hover_text(format!("Pase de la pauta de las {}", p.rsplit('@').next().unwrap_or("")));
                        }
                    });
                    row.col(|ui| {
                        ui.label(RichText::new(&x.quien).size(12.0).color(TENUE));
                    });
                });
            });
    }

    fn resumen_clientes(&mut self, ui: &mut egui::Ui, r: &Reporte) {
        tema::titulo(ui, "POR CLIENTE");
        ui.add_space(4.0);
        if r.clientes.is_empty() {
            ui.label(RichText::new("Sin datos en este periodo").color(MUY_TENUE));
            return;
        }
        for c in &r.clientes {
            let elegido = self.reportes.cliente.as_deref() == Some(c.cliente.as_str());
            let marco = tema::marco_tarjeta().stroke(Stroke::new(1.0, if elegido { ACENTO } else { BORDE }));
            let resp = marco
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&c.cliente).size(14.0).strong().family(crate::tema::negrita()));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| match c.cumplimiento() {
                            Some(p) => {
                                let color = if p >= 99.5 {
                                    TV
                                } else if p >= 80.0 {
                                    AMBAR
                                } else {
                                    AIRE
                                };
                                ui.label(
                                    RichText::new(format!("{p:.0} %"))
                                        .size(16.0)
                                        .strong()
                                        .family(crate::tema::negrita())
                                        .color(color),
                                )
                                .on_hover_text("Pases de la pauta que salieron completos, de los que ya tocaban");
                            }
                            None => {
                                ui.label(RichText::new("sin pauta").size(12.0).color(MUY_TENUE));
                            }
                        });
                    });
                    let mut t = format!("{} salidas · {} al aire", c.salidas, mmss(c.segundos));
                    if c.cortadas > 0 {
                        t.push_str(&format!(" · {} cortadas", c.cortadas));
                    }
                    ui.label(RichText::new(t).size(12.0).color(TENUE));
                    if c.programados > 0 {
                        let mut p = format!("Pauta: {} de {} pases", c.cumplidos, c.programados);
                        if c.perdidos > 0 {
                            p.push_str(&format!(" · {} sin cumplir", c.perdidos));
                        }
                        let color = if c.perdidos > 0 { AMBAR } else { TENUE };
                        ui.label(RichText::new(p).size(12.0).color(color));
                        if c.programados > 0 {
                            let frac = c.cumplidos as f32 / c.programados as f32;
                            ui.add(egui::ProgressBar::new(frac).desired_height(5.0).fill(if frac >= 0.995 {
                                TV
                            } else {
                                AMBAR
                            }));
                        }
                    }
                })
                .response
                .interact(Sense::click());
            if resp.on_hover_text("Clic: ver solo este cliente").clicked() {
                self.reportes.cliente = if elegido { None } else { Some(c.cliente.clone()) };
            }
            ui.add_space(4.0);
        }
    }

    fn exportar(&mut self, excel: bool) {
        let Some(r) = self.reportes.reporte.clone() else { return };
        let cliente = self.reportes.cliente.clone();
        let datos = match &cliente {
            Some(c) => r.de_cliente(c),
            None => r.clone(),
        };
        let quien = cliente.clone().unwrap_or_else(|| "todos".into());
        let limpio: String =
            quien.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        let (ext, filtro) = if excel { ("xlsx", "Excel") } else { ("pdf", "PDF") };
        let nombre = format!("Reporte {} {} a {}.{ext}", limpio.trim(), r.desde, r.hasta);
        let mut dialogo =
            rfd::FileDialog::new().set_title("Guardar reporte").set_file_name(&nombre).add_filter(filtro, &[ext]);
        if let Some(d) = dirs::document_dir() {
            dialogo = dialogo.set_directory(d);
        }
        let Some(ruta) = dialogo.save_file() else { return };
        let generado = self.hora(time::macros::format_description!("[day]/[month]/[year] [hour]:[minute]"));
        let res = if excel {
            exportar::excel(&datos, &ruta)
        } else {
            exportar::pdf(&datos, &ruta, cliente.as_deref(), &generado)
        };
        match res {
            Ok(()) => {
                self.avisar_local(format!("Reporte guardado: {}", ruta.display()));
                exportar::abrir(&ruta);
            }
            Err(m) => self.avisar_error(format!("No se pudo guardar el reporte: {m}")),
        }
    }
}

impl Ventana {
    fn simulador(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let (hoy, _) = self.ahora_local();
        // Día a simular.
        ui.horizontal(|ui| {
            tema::titulo(ui, "SIMULADOR DEL DÍA");
            ui.add_space(8.0);
            for (t, d) in [
                ("Hoy", hoy),
                ("Mañana", hoy + time::Duration::days(1)),
                ("Pasado mañana", hoy + time::Duration::days(2)),
            ] {
                if ui.add(egui::Button::new(t).selected(self.reportes.dia_simulado == d)).clicked() {
                    self.reportes.dia_simulado = d;
                    self.reportes.texto_dia = formato::texto_fecha_dma(d);
                }
            }
            ui.label(RichText::new("Día").color(TENUE));
            let r = ui.add(egui::TextEdit::singleline(&mut self.reportes.texto_dia).desired_width(100.0));
            if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) || ui.button("Ver").clicked() {
                match formato::leer_fecha_dma(&self.reportes.texto_dia) {
                    Some(d) => self.reportes.dia_simulado = d,
                    None => self.avisar_error("Escribe la fecha como día/mes/año, por ejemplo 15/11/2026"),
                }
            }
            let tope = match e.corte.tope {
                Some(s) => {
                    format!("Cortes de {}:{:02}{}", s / 60, s % 60, if e.corte.relleno { " con relleno" } else { "" })
                }
                None => "Cortes sin tope de duración".into(),
            };
            ui.label(RichText::new(tope).size(12.0).color(TENUE))
                .on_hover_text("Se cambia en la tanda, en modo AUTO (Duración del corte)");
        });
        ui.add_space(6.0);

        // Simulación (con la pauta, el orden, la duración del corte y el relleno actuales).
        let relleno: Vec<(String, Duration)> = if e.corte.relleno {
            e.corte
                .clips_relleno
                .iter()
                .filter_map(|c| e.medio(c).map(|m| (c.clone(), Duration::from_secs_f64(m.duracion_seg.max(0.0)))))
                .collect()
        } else {
            Vec::new()
        };
        let tope = e.corte.tope.map(|s| Duration::from_secs(s as u64));
        // Spots de otros clientes para rellenar (en la simulación, en orden alfabético).
        let clientes: Vec<(String, Duration)> = if e.corte.relleno && self.cfg.pauta.relleno_clientes {
            let mut v: Vec<(String, Duration)> = Vec::new();
            for p in e.pautas.iter().filter(|p| p.toca(self.reportes.dia_simulado)) {
                if !v.iter().any(|(c, _)| c.eq_ignore_ascii_case(&p.clip)) && p.duracion_seg > 0.0 {
                    v.push((p.clip.clone(), Duration::from_secs_f64(p.duracion_seg)));
                }
            }
            v.sort();
            v
        } else {
            Vec::new()
        };
        let (cortes, perdidos) = nucleo::pauta::simular_dia(
            &e.pautas,
            self.reportes.dia_simulado,
            self.cfg.pauta.orden,
            tope,
            self.cfg.pauta.recuperar_min,
            &clientes,
            &relleno,
        );
        let total: Duration = cortes.iter().map(|c| c.total).sum();
        let faltan: Vec<String> = e
            .pautas
            .iter()
            .filter(|p| p.activa && p.toca(self.reportes.dia_simulado))
            .filter(|p| {
                !(p.clip.contains(':') || p.clip.starts_with('/')) && e.medio(&p.clip).is_none() && !e.medios.is_empty()
            })
            .map(|p| p.nombre.clone())
            .collect();

        StripBuilder::new(ui)
            .size(Size::remainder().at_least(520.0))
            .size(Size::relative(0.34).at_least(360.0))
            .horizontal(|mut strip| {
                strip.cell(|ui| {
                    tema::marco_columna().show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        ui.horizontal(|ui| {
                            tema::titulo(ui, "CORTES");
                            ui.label(
                                RichText::new(format!("{} cortes · {} de publicidad", cortes.len(), mmss(total.as_secs_f64())))
                                    .size(12.0)
                                    .color(TENUE),
                            );
                        });
                        ui.add_space(4.0);
                        if cortes.is_empty() {
                            mensaje_vacio(ui, "🔮", "No hay pases ese día", "Ninguna pauta activa toca ese día.");
                            return;
                        }
                        egui::ScrollArea::vertical().id_salt("sim_cortes").auto_shrink([false, false]).show(ui, |ui| {
                            for c in &cortes {
                                corte_simulado(ui, c, e.corte.tope);
                                ui.add_space(4.0);
                            }
                        });
                    });
                });
                strip.cell(|ui| {
                    tema::marco_columna().show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        tema::titulo(ui, "RESUMEN");
                        ui.add_space(4.0);
                        let pases: usize = cortes.iter().map(|c| c.tanda.iter().filter(|t| t.pase.is_some()).count()).sum();
                        ui.label(RichText::new(format!("{pases} pases saldrían en {} cortes", cortes.len())).size(13.0));
                        if perdidos.is_empty() {
                            ui.label(RichText::new("✔ Todos los pases entran en algún corte").size(13.0).color(TV));
                        } else {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!("⚠ {} pases no entrarían en su ventana:", perdidos.len()))
                                    .size(13.0)
                                    .strong().family(crate::tema::negrita())
                                    .color(AIRE),
                            );
                            for p in &perdidos {
                                ui.label(
                                    RichText::new(format!("  {} · {}", crate::tema::hora(p.hora), p.nombre))
                                        .size(12.5)
                                        .color(AIRE),
                                );
                            }
                            ui.label(
                                RichText::new("Sube la duración del corte, cambia sus horas o dale más tolerancia.")
                                    .size(11.5)
                                    .color(MUY_TENUE),
                            );
                        }
                        if !faltan.is_empty() {
                            ui.add_space(8.0);
                            ui.label(RichText::new("⚠ Sin video en CasparCG:").size(13.0).strong().family(crate::tema::negrita()).color(AIRE));
                            for n in &faltan {
                                ui.label(RichText::new(format!("  {n}")).size(12.5).color(AIRE));
                            }
                        }
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(
                                "La simulación supone un corte a la hora de cada pase y arma la tanda como el modo AUTO \
                                 (prioridades primero, duración del corte y relleno actuales).",
                            )
                            .size(11.5)
                            .color(MUY_TENUE),
                        );
                    });
                });
            });
    }
}

fn corte_simulado(ui: &mut egui::Ui, c: &nucleo::pauta::CorteSimulado, tope: Option<u32>) {
    tema::marco_tarjeta().show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(crate::tema::hora(c.hora)).font(FontId::monospace(18.0)).strong());
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("{} spots · {}", c.tanda.len(), mmss(c.total.as_secs_f64())))
                    .size(12.5)
                    .color(TENUE),
            );
            if let Some(t) = tope {
                let frac = (c.total.as_secs_f32() / t as f32).clamp(0.0, 1.0);
                ui.add(egui::ProgressBar::new(frac).desired_width(120.0).desired_height(6.0).fill(if frac >= 0.999 {
                    TV
                } else {
                    ACENTO
                }))
                .on_hover_text(format!(
                    "{} de {}:{:02}",
                    mmss(c.total.as_secs_f64()),
                    t / 60,
                    t % 60
                ));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for t in &c.tanda {
                let (texto, fondo, color) = match &t.pase {
                    Some(_) => (format!("{} {}", t.nombre, mmss(t.duracion.as_secs_f64())), ACENTO_OSCURO, TEXTO),
                    None => (
                        format!("↻ {} {}", nombre_corto(&t.clip), mmss(t.duracion.as_secs_f64())),
                        TARJETA_HOVER,
                        TENUE,
                    ),
                };
                egui::Frame::new()
                    .fill(fondo)
                    .corner_radius(CornerRadius::same(5))
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.label(RichText::new(texto).size(12.0).color(color));
                    });
            }
        });
        if !c.no_entran.is_empty() {
            ui.label(
                RichText::new(format!("No entran (pasan al siguiente corte): {}", c.no_entran.join(", ")))
                    .size(11.5)
                    .color(AMBAR),
            );
        }
    });
}

/// Lista de alertas (contratos por vencer, videos que faltan).
pub(super) fn alertas(ui: &mut egui::Ui, alertas: &[rep::Alerta]) {
    tema::titulo(ui, "ALERTAS DE LA PAUTA");
    ui.add_space(4.0);
    if alertas.is_empty() {
        ui.label(
            RichText::new("✔ Sin alertas: ningún contrato por vencer y todos los videos están en CasparCG")
                .size(12.0)
                .color(TV),
        );
        return;
    }
    for a in alertas {
        let c = if a.grave { AIRE } else { AMBAR };
        ui.label(RichText::new(format!("⚠ {}", a.texto)).size(12.5).color(c));
    }
}
