//! Pantalla **Biblioteca**: todos los videos de CasparCG con su revisión (formato,
//! audio y volumen), y lo que se puede hacer con ellos: nivelar el volumen, convertir
//! al formato del canal, reemplazar el video de un spot, papelera.

use egui_extras::{Column, TableBuilder};
use nucleo::biblioteca::{self as bib, EstadoTrabajo, Ficha, Problema, TipoTrabajo, Trabajo};

use super::*;

/// Qué videos se muestran.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Filtro {
    #[default]
    Todos,
    ConProblemas,
    EnPauta,
    SinUso,
}

/// Algo que se confirma antes de hacerlo.
pub(super) enum Confirmar {
    Borrar { clip: String, pautas: Vec<String> },
    Reemplazar { clip: String, ruta: PathBuf },
    EliminarDePapelera { id: u64, nombre: String },
    VaciarPapelera,
}

/// Un video de la lista, con todo lo que hace falta para dibujarlo.
struct Fila<'a> {
    medio: &'a amcp::Medio,
    ficha: Option<&'a Ficha>,
    problemas: Vec<Problema>,
    pautas: Vec<String>,
    en_tanda: bool,
    trabajo: Option<&'a Trabajo>,
}

impl Ventana {
    /// Cuántos videos tienen algo grave (para el aviso en la pestaña).
    pub(super) fn problemas_graves(&self, e: &Instantanea) -> usize {
        e.medios
            .iter()
            .filter(|m| m.tipo == amcp::TipoMedio::Movie)
            .filter_map(|m| e.ficha(&m.nombre))
            .filter(|f| bib::problemas(f, &e.biblioteca.estandar).iter().any(|p| p.es_grave()))
            .count()
    }

    fn puede_editar(&self) -> bool {
        match &self.conexion {
            None => true,
            Some(c) => {
                matches!(&*c.lock().expect("conexion"), Conexion::Conectada { rol, .. } if *rol >= nucleo::Rol::Editor)
            }
        }
    }

    fn filas<'a>(&self, e: &'a Instantanea) -> Vec<Fila<'a>> {
        let busca = self.bib_busqueda.trim().to_lowercase();
        e.medios
            .iter()
            .filter(|m| m.tipo == amcp::TipoMedio::Movie)
            .filter(|m| busca.is_empty() || m.nombre.to_lowercase().contains(&busca))
            .map(|m| {
                let ficha = e.ficha(&m.nombre);
                let problemas = ficha.map(|f| bib::problemas(f, &e.biblioteca.estandar)).unwrap_or_default();
                let pautas = e
                    .pautas
                    .iter()
                    .filter(|p| p.clip.eq_ignore_ascii_case(&m.nombre))
                    .map(|p| p.nombre.clone())
                    .collect();
                let en_tanda = e.lista.iter().any(|i| i.clip.eq_ignore_ascii_case(&m.nombre));
                let trabajo = e.biblioteca.trabajos.iter().find(|t| t.clip.eq_ignore_ascii_case(&m.nombre));
                Fila { medio: m, ficha, problemas, pautas, en_tanda, trabajo }
            })
            .filter(|f| match self.bib_filtro {
                Filtro::Todos => true,
                Filtro::ConProblemas => !f.problemas.is_empty(),
                Filtro::EnPauta => !f.pautas.is_empty(),
                Filtro::SinUso => f.pautas.is_empty() && !f.en_tanda,
            })
            .collect()
    }

    pub(super) fn pantalla_biblioteca(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        StripBuilder::new(ui)
            .size(Size::remainder().at_least(560.0))
            .size(Size::relative(0.36).at_least(400.0))
            .horizontal(|mut strip| {
                strip.cell(|ui| {
                    tema::marco_columna().show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        self.lista_biblioteca(ui, e);
                    });
                });
                strip.cell(|ui| {
                    tema::marco_columna().show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        egui::ScrollArea::vertical().id_salt("bib_detalle").auto_shrink([false, false]).show(
                            ui,
                            |ui| {
                                self.detalle_biblioteca(ui, e);
                                self.trabajos(ui, e);
                                self.papelera(ui, e);
                            },
                        );
                    });
                });
            });
        self.confirmaciones(ui, e);
    }

    // ------------------------------------------------------------------
    // Lista
    // ------------------------------------------------------------------

    fn lista_biblioteca(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let b = &e.biblioteca;
        let total = e.medios.iter().filter(|m| m.tipo == amcp::TipoMedio::Movie).count();
        let con_problemas = e
            .medios
            .iter()
            .filter(|m| m.tipo == amcp::TipoMedio::Movie)
            .filter_map(|m| e.ficha(&m.nombre))
            .filter(|f| !bib::problemas(f, &b.estandar).is_empty())
            .count();

        ui.horizontal(|ui| {
            tema::titulo(ui, "BIBLIOTECA");
            ui.label(RichText::new(format!("{total} videos")).size(12.0).color(TENUE));
            if con_problemas > 0 {
                ui.label(RichText::new(format!("· {con_problemas} para revisar")).size(12.0).color(AMBAR));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if b.ffmpeg
                    && ui
                        .add_enabled(self.puede_editar(), egui::Button::new("↻ Revisar todo"))
                        .on_hover_text("Vuelve a revisar formato y volumen de todos los videos")
                        .clicked()
                {
                    self.ctl.enviar(Orden::Revisar(None));
                }
                if let Some(libre) = b.disco_libre {
                    let bajo = libre < bib::DISCO_BAJO;
                    let t = format!("💾 {} libres", tamano_legible(libre));
                    ui.label(RichText::new(t).size(12.0).color(if bajo { AIRE } else { TENUE }))
                        .on_hover_text("Espacio libre en el disco de los videos de CasparCG");
                }
                let est = &b.estandar;
                let canal = format!("Canal: {}×{} · {} · {} LUFS", est.ancho, est.alto, fps_corto(est.fps), est.lufs);
                ui.label(RichText::new(canal).size(12.0).color(MUY_TENUE))
                    .on_hover_text("Se cambia en config.toml, sección [biblioteca]");
            });
        });

        if !b.ffmpeg {
            tema::marco_tarjeta().stroke(Stroke::new(1.0, AMBAR)).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(
                    RichText::new(
                        "Para revisar el formato y el volumen de los videos hace falta ffmpeg en la PC principal.\n\
                         Relevo lo descarga solo: mira «🎞 ffmpeg» en la barra de abajo.",
                    )
                    .color(AMBAR),
                );
            });
        }
        if let Some((hechas, total)) = b.revision {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new(format!("Revisando videos… {hechas} de {total}")).size(12.0).color(TENUE));
                let frac = if total > 0 { hechas as f32 / total as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(frac).desired_height(6.0).desired_width(160.0));
            });
        }

        // Buscador y filtros.
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.bib_busqueda).hint_text("🔍  Buscar spot…").desired_width(220.0),
            );
            for (f, t) in [
                (Filtro::Todos, "Todos"),
                (Filtro::ConProblemas, "⚠ Para revisar"),
                (Filtro::EnPauta, "🕓 En pauta"),
                (Filtro::SinUso, "Sin usar"),
            ] {
                if ui.add(egui::Button::new(t).selected(self.bib_filtro == f)).clicked() {
                    self.bib_filtro = f;
                }
            }
        });
        ui.add_space(4.0);

        let filas = self.filas(e);
        if filas.is_empty() {
            let (t, d) = if total == 0 {
                ("No hay videos en CasparCG", "Copia videos a la carpeta media de CasparCG o súbelos desde la tanda.")
            } else {
                ("Ningún video coincide", "Prueba con otra búsqueda u otro filtro.")
            };
            mensaje_vacio(ui, "🎬", t, d);
            return;
        }

        let mut elegido: Option<String> = None;
        let mut doble: Option<String> = None;
        let sel = self.bib_sel.clone();
        let alto = ui.available_height();
        TableBuilder::new(ui)
            .striped(true)
            .sense(Sense::click())
            .max_scroll_height(alto)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(70.0))
            .column(Column::remainder().at_least(160.0).clip(true))
            .column(Column::exact(52.0))
            .column(Column::exact(118.0))
            .column(Column::exact(96.0))
            .column(Column::exact(190.0).clip(true))
            .header(22.0, |mut h| {
                for t in ["", "SPOT", "DUR.", "FORMATO", "VOLUMEN", "ESTADO"] {
                    h.col(|ui| {
                        ui.label(RichText::new(t).size(11.0).color(MUY_TENUE));
                    });
                }
            })
            .body(|body| {
                body.rows(46.0, filas.len(), |mut row| {
                    let f = &filas[row.index()];
                    let nombre = f.medio.nombre.clone();
                    row.set_selected(sel.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(&nombre)));
                    row.col(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(64.0, 36.0), Sense::hover());
                        self.pintar_miniatura(ui, rect, Some(&nombre), &nombre, false);
                    });
                    row.col(|ui| {
                        let (carpeta, corto) = partir(&nombre);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(corto)
                                        .size(14.0)
                                        .strong()
                                        .family(crate::tema::negrita())
                                        .color(TEXTO),
                                );
                                if f.en_tanda {
                                    ui.label(RichText::new("EN TANDA").size(10.0).color(AIRE));
                                }
                            });
                            let mut sub = carpeta.trim_end_matches('/').to_string();
                            if !f.pautas.is_empty() {
                                if !sub.is_empty() {
                                    sub.push_str(" · ");
                                }
                                sub.push_str(&format!("🕓 {} pauta(s)", f.pautas.len()));
                            }
                            ui.label(RichText::new(sub).size(11.0).color(TENUE));
                        });
                    });
                    row.col(|ui| {
                        ui.label(RichText::new(mmss(f.medio.duracion_seg)).monospace().size(12.5));
                    });
                    row.col(|ui| match f.ficha.filter(|x| x.error.is_none()) {
                        Some(x) => {
                            let mal =
                                f.problemas.iter().any(|p| matches!(p, Problema::Resolucion(..) | Problema::Fps(_)));
                            let t = format!("{}×{} · {}", x.ancho, x.alto, fps_corto(x.fps));
                            ui.label(RichText::new(t).size(12.0).color(if mal { AMBAR } else { TENUE }));
                        }
                        None => {
                            ui.label(RichText::new("—").color(MUY_TENUE));
                        }
                    });
                    row.col(|ui| {
                        let (t, c) = volumen(f.ficha, &f.problemas);
                        ui.label(RichText::new(t).size(12.0).color(c));
                    });
                    row.col(|ui| {
                        let (t, c) = estado(f);
                        let r = ui.label(RichText::new(t).size(12.0).color(c));
                        if f.problemas.len() > 1 {
                            let lista: Vec<String> =
                                f.problemas.iter().map(|p| format!("• {}", p.describir())).collect();
                            r.on_hover_text(lista.join("\n"));
                        }
                    });
                    let r = row.response();
                    if r.clicked() {
                        elegido = Some(nombre.clone());
                    }
                    if r.double_clicked() {
                        doble = Some(nombre.clone());
                    }
                });
            });
        if let Some(c) = elegido {
            if self.bib_sel.as_deref() != Some(c.as_str()) {
                self.bib_sel = Some(c);
                // La vista previa abierta era de otro video.
                if self.vista.activa() {
                    self.vista.cerrar();
                }
            }
        }
        if let Some(c) = doble {
            self.previa_de_clip(&c, e);
        }
    }

    fn previa_de_clip(&mut self, clip: &str, e: &Instantanea) {
        if let Some(ruta) = self.ruta_de_clip(clip, e) {
            let dur = e.medio(clip).map(|m| Duration::from_secs_f64(m.duracion_seg.max(0.0)));
            self.abrir_vista_previa(ruta, dur);
        }
    }

    // ------------------------------------------------------------------
    // Detalle del video elegido
    // ------------------------------------------------------------------

    fn detalle_biblioteca(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let Some(clip) = self.bib_sel.clone().filter(|c| e.medio(c).is_some()) else {
            tema::marco_tarjeta().show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                mensaje_vacio(ui, "🎬", "Elige un video", "Verás su revisión y lo que se puede hacer con él.");
                ui.add_space(30.0);
            });
            return;
        };
        let medio = e.medio(&clip).cloned().expect("comprobado");
        let ficha = e.ficha(&clip).cloned();
        let problemas = ficha.as_ref().map(|f| bib::problemas(f, &e.biblioteca.estandar)).unwrap_or_default();
        let pautas: Vec<String> =
            e.pautas.iter().filter(|p| p.clip.eq_ignore_ascii_case(&clip)).map(|p| p.nombre.clone()).collect();
        let en_tanda = e.lista.iter().any(|i| i.clip.eq_ignore_ascii_case(&clip));
        let trabajo = e.biblioteca.trabajos.iter().find(|t| t.clip.eq_ignore_ascii_case(&clip)).cloned();
        let ruta_local = self.ruta_de_clip(&clip, e);
        let editar = self.puede_editar();

        tema::marco_tarjeta().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let (carpeta, corto) = partir(&clip);
            ui.label(RichText::new(corto).size(18.0).strong().family(crate::tema::negrita()));
            let ubic = ficha.as_ref().map(|f| f.ruta.clone()).unwrap_or_else(|| carpeta.to_string());
            ui.label(RichText::new(format!("🗀 media/{ubic}")).size(11.5).color(TENUE));
            ui.add_space(4.0);

            // Imagen: la vista previa si está abierta; si no, la miniatura (clic = vista previa).
            if self.vista.activa() {
                let silenciar = (!self.ctl.es_remoto() && self.cfg.vista_previa.silenciar_al_aire && e.fase != FaseVista::Tv)
                    .then_some("Sin sonido mientras hay publicidad al aire");
                self.vista.ajustar(ui.available_width(), 420.0);
                if let AccionVista::Agregar(destino) = self.vista.mostrar_en(ui, silenciar) {
                    self.ctl.enviar(Orden::Agregar {
                        clip: clip.clone(),
                        duracion: Duration::from_secs_f64(medio.duracion_seg.max(0.0)),
                        destino,
                    });
                }
            } else {
                let w = ui.available_width();
                let h = (w * 9.0 / 16.0).min(220.0);
                let (fila, r) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
                let rect = egui::Rect::from_center_size(fila.center(), Vec2::new(h * 16.0 / 9.0, h));
                self.pintar_miniatura(ui, rect, Some(&clip), &clip, false);
                if ruta_local.is_some() {
                    let p = ui.painter();
                    let c = rect.center();
                    p.circle_filled(c, 26.0, Color32::from_black_alpha(if r.hovered() { 200 } else { 140 }));
                    p.text(c + Vec2::new(2.0, 0.0), Align2::CENTER_CENTER, "⏵", FontId::proportional(26.0), Color32::WHITE);
                    if r.on_hover_text("Ver la vista previa (con sonido)").clicked() {
                        self.previa_de_clip(&clip, e);
                    }
                } else if self.ctl.es_remoto() {
                    r.on_hover_text("La vista previa de los videos de la PC principal solo se ve en ella");
                }
            }
            ui.add_space(6.0);

            // Datos.
            egui::Grid::new("bib_datos").num_columns(4).spacing([12.0, 2.0]).min_row_height(18.0).show(ui, |ui| {
                let f = ficha.as_ref();
                dato(ui, "Duración", &mmss(medio.duracion_seg));
                dato(
                    ui,
                    "Formato",
                    &f.filter(|x| x.ancho > 0).map(|x| format!("{}×{}", x.ancho, x.alto)).unwrap_or("—".into()),
                );
                ui.end_row();
                dato(ui, "Cuadros/s", &f.filter(|x| x.fps > 0.0).map(|x| fps_corto(x.fps)).unwrap_or("—".into()));
                dato(ui, "Códec", &f.filter(|x| !x.codec.is_empty()).map(|x| x.codec.clone()).unwrap_or("—".into()));
                ui.end_row();
                let audio = match f {
                    Some(x) if !x.audio => "sin audio".to_string(),
                    Some(x) => match x.canales {
                        1 => "mono".into(),
                        2 => "estéreo".into(),
                        n => format!("{n} canales"),
                    },
                    None => "—".into(),
                };
                dato(ui, "Audio", &audio);
                let vol = match f.and_then(|x| x.lufs) {
                    Some(l) => {
                        let pico = f.and_then(|x| x.pico).map(|p| format!(" · pico {p:.1}")).unwrap_or_default();
                        format!("{l:.1} LUFS{pico}")
                    }
                    None => "—".into(),
                };
                dato(ui, "Volumen", &vol);
                ui.end_row();
                dato(ui, "Tamaño", &f.map(|x| tamano_legible(x.tamano)).unwrap_or("—".into()));
                ui.end_row();
            });
            ui.add_space(6.0);

            // Revisión.
            match (&ficha, &trabajo) {
                (_, Some(t)) => {
                    let txt = match &t.estado {
                        EstadoTrabajo::EnCola => format!("⏳ {} (en cola)", t.tipo.nombre()),
                        EstadoTrabajo::Haciendo => format!("⏳ {} — {:.0} %", t.tipo.nombre(), t.progreso * 100.0),
                        EstadoTrabajo::Esperando => {
                            format!("⏳ {}: listo, se cambia cuando el spot salga de la tanda", t.tipo.nombre())
                        }
                        EstadoTrabajo::Error(m) => format!("✖ {}: {m}", t.tipo.nombre()),
                    };
                    let color = if matches!(t.estado, EstadoTrabajo::Error(_)) { AIRE } else { ACENTO };
                    ui.label(RichText::new(txt).size(13.0).color(color));
                }
                (None, None) => {
                    let t = if e.biblioteca.ffmpeg { "… todavía sin revisar" } else { "Sin revisar (falta ffmpeg)" };
                    ui.label(RichText::new(t).color(MUY_TENUE));
                }
                (Some(_), None) if problemas.is_empty() => {
                    ui.label(RichText::new("✔ Listo para salir al aire: cumple el formato y el volumen del canal").color(TV));
                }
                (Some(_), None) => {
                    for p in &problemas {
                        let c = if p.es_grave() { AIRE } else { AMBAR };
                        ui.label(RichText::new(format!("⚠ {}", p.describir())).size(13.0).color(c));
                    }
                }
            }
            // Dónde se usa.
            let uso = match (pautas.len(), en_tanda) {
                (0, false) => "No está en ninguna pauta".to_string(),
                (0, true) => "Está en la tanda".to_string(),
                (n, t) => format!(
                    "En {n} pauta(s): {}{}",
                    pautas.join(", "),
                    if t { " · está en la tanda" } else { "" }
                ),
            };
            ui.label(RichText::new(uso).size(12.0).color(TENUE));
            ui.add_space(8.0);

            // Acciones.
            let libre = trabajo.as_ref().is_none_or(|t| matches!(t.estado, EstadoTrabajo::Error(_)));
            let hay_volumen = ficha.as_ref().is_some_and(|f| f.lufs.is_some_and(|l| l > -60.0));
            let pide_volumen = problemas.iter().any(|p| p.es_de_volumen());
            let pide_formato = problemas.iter().any(|p| matches!(p, Problema::Resolucion(..) | Problema::Fps(_)));
            let est = &e.biblioteca.estandar;
            ui.horizontal_wrapped(|ui| {
                let b = boton_accion("🔊 Nivelar volumen", pide_volumen);
                if ui
                    .add_enabled(editar && libre && e.biblioteca.ffmpeg && hay_volumen, b)
                    .on_hover_text(format!("Lleva el volumen a {} LUFS. La imagen no cambia.", est.lufs))
                    .on_disabled_hover_text(motivo_deshabilitado(editar, libre, e.biblioteca.ffmpeg, hay_volumen))
                    .clicked()
                {
                    self.ctl.enviar(Orden::Trabajo { clip: clip.clone(), tipo: TipoTrabajo::Nivelar });
                }
                let b = boton_accion("🎞 Convertir al formato del canal", pide_formato);
                if ui
                    .add_enabled(editar && libre && e.biblioteca.ffmpeg, b)
                    .on_hover_text(format!(
                        "{}×{} a {} cuadros/s, H.264 + AAC, y volumen a {} LUFS",
                        est.ancho,
                        est.alto,
                        fps_corto(est.fps),
                        est.lufs
                    ))
                    .on_disabled_hover_text(motivo_deshabilitado(editar, libre, e.biblioteca.ffmpeg, true))
                    .clicked()
                {
                    self.ctl.enviar(Orden::Trabajo { clip: clip.clone(), tipo: TipoTrabajo::Convertir });
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(editar && libre, egui::Button::new("🗀 Reemplazar con otro video…"))
                    .on_hover_text(
                        "El spot conserva su nombre, sus pautas y su lugar en la tanda.\nTambién puedes arrastrar el nuevo video a esta ventana.",
                    )
                    .clicked()
                {
                    if let Some(ruta) = elegir_video() {
                        self.confirmar = Some(Confirmar::Reemplazar { clip: clip.clone(), ruta });
                    }
                }
                if e.biblioteca.ffmpeg
                    && ui.add_enabled(editar, egui::Button::new("↻ Revisar")).on_hover_text("Volver a revisar este video").clicked()
                {
                    self.ctl.enviar(Orden::Revisar(Some(clip.clone())));
                }
                let b = egui::Button::new(RichText::new("🗑 A la papelera").color(AIRE));
                if ui
                    .add_enabled(editar && !en_tanda && libre, b)
                    .on_disabled_hover_text(if en_tanda { "Está en la tanda: quítalo primero" } else { "No disponible" })
                    .clicked()
                {
                    self.confirmar = Some(Confirmar::Borrar { clip: clip.clone(), pautas: pautas.clone() });
                }
            });
            ui.label(
                RichText::new(format!(
                    "Al nivelar, convertir o reemplazar, la versión anterior queda {} días en la papelera.",
                    bib::DIAS_PAPELERA
                ))
                .size(11.0)
                .color(MUY_TENUE),
            );
        });
    }

    // ------------------------------------------------------------------
    // Trabajos y papelera
    // ------------------------------------------------------------------

    fn trabajos(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        if e.biblioteca.trabajos.is_empty() {
            return;
        }
        ui.add_space(8.0);
        tema::marco_tarjeta().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            tema::titulo(ui, "TRABAJOS");
            for t in &e.biblioteca.trabajos {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width() - 34.0);
                        ui.label(RichText::new(format!("{} · {}", nombre_corto(&t.clip), t.tipo.nombre())).size(13.0));
                        match &t.estado {
                            EstadoTrabajo::EnCola => {
                                ui.label(
                                    RichText::new(format!("En cola (pedido por {})", t.quien)).size(11.5).color(TENUE),
                                );
                            }
                            EstadoTrabajo::Haciendo => {
                                ui.add(
                                    egui::ProgressBar::new(t.progreso)
                                        .desired_height(8.0)
                                        .fill(ACENTO)
                                        .text(RichText::new(format!("{:.0} %", t.progreso * 100.0)).size(10.0)),
                                );
                            }
                            EstadoTrabajo::Esperando => {
                                ui.label(
                                    RichText::new("Listo: se cambia el archivo cuando el spot salga de la tanda")
                                        .size(11.5)
                                        .color(AMBAR),
                                );
                            }
                            EstadoTrabajo::Error(m) => {
                                ui.label(RichText::new(format!("Falló: {m}")).size(11.5).color(AIRE));
                            }
                        }
                    });
                    let ayuda = if matches!(t.estado, EstadoTrabajo::Error(_)) { "Quitar" } else { "Cancelar" };
                    if ui.add_enabled(self.puede_editar(), boton_chico("✖")).on_hover_text(ayuda).clicked() {
                        self.ctl.enviar(Orden::CancelarTrabajo(t.id));
                    }
                });
            }
        });
    }

    fn papelera(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        ui.add_space(8.0);
        let entradas = &e.biblioteca.papelera;
        let editar = self.puede_editar();
        tema::marco_tarjeta().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let titulo = RichText::new(format!("🗑  PAPELERA ({})", entradas.len())).size(12.0).color(TENUE);
            egui::CollapsingHeader::new(titulo).id_salt("bib_papelera").default_open(false).show(ui, |ui| {
                if entradas.is_empty() {
                    ui.label(RichText::new("Vacía").color(MUY_TENUE));
                    return;
                }
                ui.label(
                    RichText::new(format!("Los videos se borran solos a los {} días.", bib::DIAS_PAPELERA))
                        .size(11.0)
                        .color(MUY_TENUE),
                );
                let ahora = bib::ahora();
                for x in entradas.iter().rev() {
                    ui.separator();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(editar, boton_chico("✖")).on_hover_text("Borrar para siempre").clicked() {
                            self.confirmar =
                                Some(Confirmar::EliminarDePapelera { id: x.id, nombre: x.ruta_original.clone() });
                        }
                        if ui.add_enabled(editar, egui::Button::new("↩ Restaurar")).clicked() {
                            self.ctl.enviar(Orden::Restaurar(x.id));
                        }
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(RichText::new(&x.ruta_original).size(13.0)).truncate());
                            let dias = ahora.saturating_sub(x.fecha) / 86_400;
                            let quedan = bib::DIAS_PAPELERA.saturating_sub(dias);
                            let cuando = if dias == 0 { "hoy".to_string() } else { format!("hace {dias} días") };
                            ui.label(
                                RichText::new(format!(
                                    "{} · {cuando} · se borra en {quedan} días · {}",
                                    x.motivo,
                                    tamano_legible(x.tamano)
                                ))
                                .size(11.0)
                                .color(TENUE),
                            );
                        });
                    });
                }
                ui.separator();
                if ui.add_enabled(editar, egui::Button::new(RichText::new("Vaciar papelera").color(AIRE))).clicked() {
                    self.confirmar = Some(Confirmar::VaciarPapelera);
                }
            });
        });
    }

    // ------------------------------------------------------------------
    // Confirmaciones y archivos soltados
    // ------------------------------------------------------------------

    /// Un video arrastrado a la Biblioteca reemplaza al spot elegido (tras confirmar).
    pub(super) fn soltado_en_biblioteca(&mut self, ruta: PathBuf, e: &Instantanea) {
        match self.bib_sel.clone().filter(|c| e.medio(c).is_some()) {
            Some(clip) => self.confirmar = Some(Confirmar::Reemplazar { clip, ruta }),
            None => self.avisar_local("Elige primero en la lista el spot que quieres reemplazar"),
        }
    }

    fn confirmaciones(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let Some(c) = self.confirmar.take() else { return };
        let mut si = false;
        let mut no = false;
        let (icono, titulo, texto, boton, tono) = match &c {
            Confirmar::Borrar { clip, pautas } => {
                let mut t = format!(
                    "«{clip}» se moverá a la papelera (se puede restaurar durante {} días).",
                    bib::DIAS_PAPELERA
                );
                if !pautas.is_empty() {
                    t.push_str(&format!(
                        "\n\n⚠ Lo usan {} pauta(s): {}.\nNo saldrá al aire mientras esté en la papelera.",
                        pautas.len(),
                        pautas.join(", ")
                    ));
                }
                ("🗑", "¿Mandar a la papelera?", t, "Mandar a la papelera", Tono::Aviso)
            }
            Confirmar::Reemplazar { clip, ruta } => {
                let nombre = ruta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let pautas = e.pautas.iter().filter(|p| p.clip.eq_ignore_ascii_case(clip)).count();
                let mut t = format!("El spot «{clip}» pasará a usar el video «{nombre}».");
                if pautas > 0 {
                    t.push_str(&format!("\nSus {pautas} pauta(s) siguen igual."));
                }
                t.push_str("\nLa versión anterior queda en la papelera.");
                if self.ctl.es_remoto() {
                    t.push_str("\n\nEl video se subirá a la PC principal.");
                }
                ("⇄", "¿Reemplazar el video?", t, "Reemplazar", Tono::Normal)
            }
            Confirmar::EliminarDePapelera { nombre, .. } => (
                "⚠",
                "¿Borrar para siempre?",
                format!("«{nombre}» se borrará del disco. No se puede deshacer."),
                "Borrar",
                Tono::Peligro,
            ),
            Confirmar::VaciarPapelera => (
                "⚠",
                "¿Vaciar la papelera?",
                format!("Se borrarán del disco {} video(s). No se puede deshacer.", e.biblioteca.papelera.len()),
                "Vaciar",
                Tono::Peligro,
            ),
        };
        let modal = Dialogo::new("relevo-confirmar-biblioteca", icono, titulo).tono(tono).ancho(440.0).mostrar(
            ui.ctx(),
            |ui| {
                ui.label(RichText::new(texto).color(TEXTO));
                ui.add_space(8.0);
                let (a, b) = dialogo::pie(ui, boton, tono, true);
                si = a;
                no = b;
            },
        );
        if modal.cerrar {
            no = true;
        }
        if si {
            match c {
                Confirmar::Borrar { clip, .. } => self.ctl.enviar(Orden::Borrar(clip)),
                Confirmar::Reemplazar { clip, ruta } => self.ctl.reemplazar_con(clip, ruta),
                Confirmar::EliminarDePapelera { id, .. } => self.ctl.enviar(Orden::EliminarDePapelera(Some(id))),
                Confirmar::VaciarPapelera => self.ctl.enviar(Orden::EliminarDePapelera(None)),
            }
        } else if !no {
            self.confirmar = Some(c);
        }
    }
}

fn boton_accion(texto: &str, resaltar: bool) -> egui::Button<'static> {
    let b = egui::Button::new(RichText::new(texto.to_string()).color(if resaltar { Color32::WHITE } else { TEXTO }));
    if resaltar {
        b.fill(ACENTO_OSCURO).stroke(Stroke::new(1.0, ACENTO))
    } else {
        b
    }
}

fn motivo_deshabilitado(editar: bool, libre: bool, ffmpeg: bool, hay_volumen: bool) -> &'static str {
    if !editar {
        "Hace falta un usuario Editor"
    } else if !libre {
        "Ya tiene un trabajo pendiente"
    } else if !ffmpeg {
        "Falta ffmpeg en la PC principal"
    } else if !hay_volumen {
        "No tiene audio que nivelar"
    } else {
        "No disponible"
    }
}

/// Diálogo de Windows para elegir un video.
fn elegir_video() -> Option<PathBuf> {
    let ext: Vec<&str> = archivos::EXTENSIONES_VIDEO.to_vec();
    rfd::FileDialog::new().set_title("Elegir el nuevo video del spot").add_filter("Videos", &ext).pick_file()
}

fn fps_corto(fps: f64) -> String {
    if (fps - fps.round()).abs() < 0.005 {
        format!("{}", fps.round() as i64)
    } else {
        format!("{fps:.2}")
    }
}

fn volumen(f: Option<&Ficha>, problemas: &[Problema]) -> (String, Color32) {
    let Some(f) = f.filter(|f| f.error.is_none()) else { return ("—".into(), MUY_TENUE) };
    if !f.audio {
        return ("sin audio".into(), AIRE);
    }
    match f.lufs {
        Some(l) if l <= -60.0 => ("silencio".into(), AIRE),
        Some(l) => {
            let c = match problemas.iter().find(|p| p.es_de_volumen()) {
                Some(p) if p.es_grave() => AIRE,
                Some(_) => AMBAR,
                None => TV,
            };
            (format!("{l:.1} LUFS"), c)
        }
        None => ("—".into(), MUY_TENUE),
    }
}

fn estado(f: &Fila) -> (String, Color32) {
    if let Some(t) = f.trabajo {
        return match &t.estado {
            EstadoTrabajo::EnCola => ("⏳ en cola".into(), ACENTO),
            EstadoTrabajo::Haciendo => (format!("⏳ {:.0} %", t.progreso * 100.0), ACENTO),
            EstadoTrabajo::Esperando => ("⏳ espera fin de tanda".into(), AMBAR),
            EstadoTrabajo::Error(_) => ("✖ falló el trabajo".into(), AIRE),
        };
    }
    match (f.ficha, f.problemas.first()) {
        (None, _) => ("… sin revisar".into(), MUY_TENUE),
        (Some(_), None) => ("✔ Listo".into(), TV),
        (Some(_), Some(p)) => {
            let grave = f.problemas.iter().any(|p| p.es_grave());
            let mas = if f.problemas.len() > 1 { format!(" (+{})", f.problemas.len() - 1) } else { String::new() };
            (format!("⚠ {}{mas}", resumen(p)), if grave { AIRE } else { AMBAR })
        }
    }
}

/// Texto corto de un problema, para la columna de estado.
fn resumen(p: &Problema) -> String {
    match p {
        Problema::Ilegible(_) => "no se puede leer".into(),
        Problema::SinAudio => "sin audio".into(),
        Problema::Silencio => "audio en silencio".into(),
        Problema::Fuerte(d) => format!("muy fuerte (+{d:.0} dB)"),
        Problema::Bajo(d) => format!("muy bajo (−{d:.0} dB)"),
        Problema::Resolucion(..) => "otra resolución".into(),
        Problema::Fps(_) => "otros cuadros/s".into(),
    }
}
