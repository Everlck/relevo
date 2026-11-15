//! Pantalla **Publicidades**: registrar los spots de los clientes y sus horarios.
//!
//! A la izquierda, la lista (por cliente, con buscador y filtros); en el medio, la
//! ficha de la publicidad elegida (spot, vigencia, días, horarios y opciones); a la
//! derecha, cómo queda: los pases de hoy y de los próximos 7 días.

use super::*;
use crate::editor_pauta::Borrador;
use nucleo::pauta::{formato, Pauta};

const DIAS_CORTOS: [&str; 7] = ["L", "M", "X", "J", "V", "S", "D"];
const DIAS_SEMANA: [&str; 7] = ["Lun", "Mar", "Mié", "Jue", "Vie", "Sáb", "Dom"];

/// Qué publicidades se ven en la lista.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Filtro {
    Todas,
    Hoy,
    Activas,
    Pausadas,
    Terminadas,
}

pub(super) struct EstadoPublicidades {
    borrador: Borrador,
    /// Cómo estaba al abrirla (para saber si hay cambios sin guardar).
    original: Borrador,
    filtro: String,
    filtro_estado: Filtro,
    filtro_video: String,
    error: Option<String>,
    confirmar_borrar: bool,
    /// Se pidió abrir otra (o una nueva) con cambios sin guardar.
    cambiar_a: Option<Option<u64>>,
    /// Recién guardada una nueva: se espera a que el núcleo le dé número.
    esperando: Option<Pauta>,
    gen_desde: String,
    gen_hasta: String,
    gen_cada: u32,
    tolerancia_defecto: u32,
    /// Plantillas de horario guardadas.
    plantillas: crate::plantillas_horario::Plantillas,
    /// Nombre para guardar las horas como plantilla (`Some` = se está escribiendo).
    nombre_plantilla: Option<String>,
    /// Mensaje corto de lo último que se hizo con las horas.
    aviso_horas: Option<String>,
    /// Poner el cursor en el nombre de la plantilla (recién abierto).
    enfocar_plantilla: bool,
}

impl EstadoPublicidades {
    pub(super) fn new(hoy: time::Date, tolerancia: u32, plantillas: crate::plantillas_horario::Plantillas) -> Self {
        let b = Borrador::nuevo(hoy, tolerancia);
        EstadoPublicidades {
            borrador: b.clone(),
            original: b,
            filtro: String::new(),
            filtro_estado: Filtro::Todas,
            filtro_video: String::new(),
            error: None,
            confirmar_borrar: false,
            cambiar_a: None,
            esperando: None,
            gen_desde: "08:00".into(),
            gen_hasta: "20:00".into(),
            gen_cada: 120,
            tolerancia_defecto: tolerancia,
            plantillas,
            nombre_plantilla: None,
            aviso_horas: None,
            enfocar_plantilla: false,
        }
    }

    fn hay_cambios(&self) -> bool {
        let mut a = self.borrador.clone();
        a.nueva_hora.clear();
        a != self.original
    }

    fn cargar(&mut self, b: Borrador) {
        self.aviso_horas = None;
        self.nombre_plantilla = None;
        self.original = b.clone();
        self.borrador = b;
        self.error = None;
        self.confirmar_borrar = false;
        self.filtro_video.clear();
    }

    /// Ficha en blanco para una publicidad nueva.
    pub(super) fn nueva(&mut self, hoy: time::Date) {
        self.cargar(Borrador::nuevo(hoy, self.tolerancia_defecto));
    }

    /// Ficha nueva con este video ya elegido.
    pub(super) fn nueva_con_video(&mut self, hoy: time::Date, clip: &str, duracion: Duration) {
        let mut b = Borrador::nuevo(hoy, self.tolerancia_defecto);
        b.clip = clip.to_string();
        b.duracion_seg = duracion.as_secs_f64();
        self.original = Borrador::nuevo(hoy, self.tolerancia_defecto);
        self.borrador = b;
        self.error = None;
    }
}

/// Estado de una publicidad en una fecha: texto y color.
fn estado_de(p: &Pauta, hoy: time::Date) -> (&'static str, Color32) {
    if !p.activa {
        ("Pausada", TENUE)
    } else if p.fin.is_some_and(|f| f < hoy) {
        ("Terminada", MUY_TENUE)
    } else if p.inicio > hoy {
        ("Empieza pronto", ACENTO)
    } else if p.toca(hoy) {
        ("Hoy", TV)
    } else {
        ("Activa", ACENTO)
    }
}

fn pasa_filtro(p: &Pauta, f: Filtro, hoy: time::Date) -> bool {
    match f {
        Filtro::Todas => true,
        Filtro::Hoy => p.toca(hoy),
        Filtro::Activas => p.activa && !p.fin.is_some_and(|x| x < hoy),
        Filtro::Pausadas => !p.activa,
        Filtro::Terminadas => p.fin.is_some_and(|x| x < hoy),
    }
}

/// «08:00 13:00 20:00» (o «08:00 13:00 +3»).
fn resumen_horas(horas: &[time::Time], max: usize) -> String {
    let mut v: Vec<String> = horas.iter().take(max).map(|h| crate::tema::hora(*h)).collect();
    if horas.len() > max {
        v.push(format!("+{}", horas.len() - max));
    }
    v.join(" ")
}

impl Ventana {
    pub(super) fn pantalla_publicidades(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let hoy = self.hoy();
        self.adoptar_guardada(e);

        // Cabecera: título, resumen y «Nueva».
        ui.horizontal(|ui| {
            ui.label(RichText::new("Publicidades").size(22.0).strong().family(tema::negrita()));
            ui.add_space(10.0);
            let activas = e.pautas.iter().filter(|p| p.activa && !p.fin.is_some_and(|f| f < hoy)).count();
            let pases_hoy: usize = e.pautas.iter().filter(|p| p.toca(hoy)).map(|p| p.horas.len()).sum();
            let clientes: std::collections::BTreeSet<String> =
                e.pautas.iter().map(|p| p.cliente_o_nombre().to_lowercase()).collect();
            for (t, c) in [
                (format!("{} publicidades", e.pautas.len()), TENUE),
                (format!("{} clientes", clientes.len()), TENUE),
                (format!("{activas} activas"), ACENTO),
                (format!("{pases_hoy} pases hoy"), TV),
            ] {
                tema::chip(ui, &t, c, TARJETA);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let nueva = egui::Button::new(
                    RichText::new("+ Nueva publicidad").strong().family(tema::negrita()).color(Color32::WHITE),
                )
                .fill(ACENTO_OSCURO)
                .stroke(Stroke::new(1.0, ACENTO))
                .min_size(Vec2::new(0.0, 32.0));
                if ui.add(nueva).clicked() {
                    self.pedir_abrir(None, hoy, e);
                }
            });
        });
        ui.add_space(8.0);

        let alto = ui.available_height();
        let ancho = ui.available_width();
        let ancho_lista = 330.0_f32.min(ancho * 0.3);
        let ancho_vista = if ancho > 1180.0 { 300.0 } else { 0.0 };
        // Lo que ocupan las separaciones entre columnas.
        let separaciones = 24.0 + ui.spacing().item_spacing.x * 4.0;
        let ancho_ficha = (ancho - ancho_lista - ancho_vista - separaciones).max(420.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(ancho_lista, alto), Layout::top_down(Align::Min), |ui| {
                ui.set_width(ancho_lista);
                ui.set_height(alto);
                tema::marco_columna().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height(ui.available_height());
                    self.lista_publicidades(ui, e, hoy);
                });
            });
            ui.add_space(12.0);
            ui.allocate_ui_with_layout(Vec2::new(ancho_ficha, alto), Layout::top_down(Align::Min), |ui| {
                ui.set_width(ancho_ficha);
                ui.set_height(alto);
                tema::marco_columna().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_height(ui.available_height());
                    self.ficha_publicidad(ui, e, hoy, ancho_vista == 0.0);
                });
            });
            if ancho_vista > 0.0 {
                ui.add_space(12.0);
                ui.allocate_ui_with_layout(Vec2::new(ancho_vista, alto), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(ancho_vista);
                    ui.set_height(alto);
                    tema::marco_columna().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_height(ui.available_height());
                        egui::ScrollArea::vertical().id_salt("vista_pub").auto_shrink([false, false]).show(ui, |ui| {
                            self.vista_publicidad(ui, e, hoy);
                        });
                    });
                });
            }
        });
        self.confirmar_cambio(ui, e, hoy);
    }

    /// Una nueva recién guardada: cuando el núcleo le da número, queda abierta.
    fn adoptar_guardada(&mut self, e: &Instantanea) {
        let Some(esp) = &self.publicidades.esperando else { return };
        let encontrada = e
            .pautas
            .iter()
            .filter(|p| {
                let mut q = (*p).clone();
                q.id = 0;
                q == *esp
            })
            .max_by_key(|p| p.id);
        if let Some(p) = encontrada {
            let b = Borrador::desde(p);
            self.publicidades.cargar(b);
            self.publicidades.esperando = None;
        }
    }

    /// Abrir otra publicidad (`Some(id)`) o una nueva (`None`); si hay cambios sin
    /// guardar, primero se pregunta.
    fn pedir_abrir(&mut self, id: Option<u64>, hoy: time::Date, e: &Instantanea) {
        if self.publicidades.hay_cambios() {
            self.publicidades.cambiar_a = Some(id);
        } else {
            self.abrir_publicidad(id, hoy, e);
        }
    }

    fn abrir_publicidad(&mut self, id: Option<u64>, hoy: time::Date, e: &Instantanea) {
        match id.and_then(|id| e.pautas.iter().find(|p| p.id == id)) {
            Some(p) => self.publicidades.cargar(Borrador::desde(p)),
            None => self.publicidades.nueva(hoy),
        }
    }

    fn confirmar_cambio(&mut self, ui: &egui::Ui, e: &Instantanea, hoy: time::Date) {
        let Some(destino) = self.publicidades.cambiar_a else { return };
        let mut descartar = false;
        let mut seguir = false;
        let r =
            Dialogo::new("relevo-cambios-pub", "✎", "Hay cambios sin guardar").ancho(420.0).mostrar(ui.ctx(), |ui| {
                ui.label(
                    RichText::new("Si sigues, se pierden los cambios de esta publicidad.").size(13.5).color(TEXTO),
                );
                let (si, no) = dialogo::pie(ui, "Descartar cambios", Tono::Peligro, true);
                descartar = si;
                seguir = no;
            });
        if r.cerrar || seguir {
            self.publicidades.cambiar_a = None;
        }
        if descartar {
            self.publicidades.cambiar_a = None;
            self.abrir_publicidad(destino, hoy, e);
        }
    }

    // ------------------------------------------------------------------
    // Lista
    // ------------------------------------------------------------------

    fn lista_publicidades(&mut self, ui: &mut egui::Ui, e: &Instantanea, hoy: time::Date) {
        ui.add(
            egui::TextEdit::singleline(&mut self.publicidades.filtro)
                .hint_text("🔍  Buscar cliente, spot o video…")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (f, t) in [
                (Filtro::Todas, "Todas"),
                (Filtro::Hoy, "Hoy"),
                (Filtro::Activas, "Activas"),
                (Filtro::Pausadas, "Pausadas"),
                (Filtro::Terminadas, "Terminadas"),
            ] {
                let n = e.pautas.iter().filter(|p| pasa_filtro(p, f, hoy)).count();
                let activo = self.publicidades.filtro_estado == f;
                let b = egui::Button::new(RichText::new(format!("{t} {n}")).size(12.0).color(if activo {
                    TEXTO
                } else {
                    TENUE
                }))
                .fill(if activo { ACENTO_OSCURO } else { Color32::TRANSPARENT })
                .stroke(if activo { Stroke::new(1.0, ACENTO) } else { Stroke::new(1.0, BORDE) })
                .corner_radius(CornerRadius::same(99));
                if ui.add(b).clicked() {
                    self.publicidades.filtro_estado = f;
                }
            }
        });
        ui.add_space(6.0);

        let filtro = self.publicidades.filtro.trim().to_lowercase();
        let mut pautas: Vec<&Pauta> = e
            .pautas
            .iter()
            .filter(|p| pasa_filtro(p, self.publicidades.filtro_estado, hoy))
            .filter(|p| {
                filtro.is_empty()
                    || p.nombre.to_lowercase().contains(&filtro)
                    || p.cliente.to_lowercase().contains(&filtro)
                    || p.clip.to_lowercase().contains(&filtro)
            })
            .collect();
        pautas.sort_by_key(|p| (p.cliente_o_nombre().to_lowercase(), !p.activa, p.nombre.to_lowercase()));

        egui::ScrollArea::vertical().id_salt("lista_pub").auto_shrink([false, false]).show(ui, |ui| {
            if e.pautas.is_empty() {
                mensaje_vacio(
                    ui,
                    "📺",
                    "Todavía no hay publicidades",
                    "Crea la primera con «+ Nueva publicidad»: su video, sus días y sus horarios.",
                );
                return;
            }
            if pautas.is_empty() {
                mensaje_vacio(ui, "🔍", "Nada con ese filtro", "Prueba con otra palabra o elige «Todas».");
                return;
            }
            let mut abrir: Option<u64> = None;
            let mut alternar: Option<u64> = None;
            let mut cliente_actual = String::new();
            for p in &pautas {
                // Encabezado de cada cliente (con cuántas tiene).
                let cliente = p.cliente_o_nombre().to_string();
                if !cliente.eq_ignore_ascii_case(&cliente_actual) {
                    cliente_actual = cliente.clone();
                    let n = pautas.iter().filter(|q| q.cliente_o_nombre().eq_ignore_ascii_case(&cliente)).count();
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(cliente.to_uppercase()).size(11.0).strong().color(MUY_TENUE));
                        ui.label(RichText::new(format!("· {n}")).size(11.0).color(MUY_TENUE));
                    });
                }
                let sel = self.publicidades.borrador.id == p.id && p.id != 0;
                let (estado, color) = estado_de(p, hoy);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
                let encima = resp.hovered();
                let fondo = if sel {
                    ACENTO_OSCURO
                } else if encima {
                    TARJETA_HOVER
                } else {
                    TARJETA
                };
                let pintor = ui.painter();
                pintor.rect_filled(rect, 7.0, fondo);
                if sel {
                    pintor.rect_stroke(rect, 7.0, Stroke::new(1.0, ACENTO), StrokeKind::Inside);
                }
                // Punto de estado.
                pintor.circle_filled(rect.left_center() + Vec2::new(12.0, -8.0), 4.0, color);
                let x = rect.left() + 24.0;
                let ancho_texto = rect.width() - 24.0 - 82.0;
                let mut nombre = p.nombre.clone();
                if p.prioridad > 0 {
                    nombre = format!("{} {nombre}", "★".repeat(p.prioridad as usize));
                }
                pintor.text(
                    egui::pos2(x, rect.top() + 14.0),
                    Align2::LEFT_CENTER,
                    recortar_ancho(ui, &nombre, &FontId::new(14.0, tema::negrita()), ancho_texto),
                    FontId::new(14.0, tema::negrita()),
                    if p.activa { TEXTO } else { TENUE },
                );
                let sub = format!("{} · {}", p.describir_dias(), resumen_horas(&p.horas, 4));
                pintor.text(
                    egui::pos2(x, rect.bottom() - 12.0),
                    Align2::LEFT_CENTER,
                    recortar_ancho(ui, &sub, &FontId::proportional(12.0), ancho_texto),
                    FontId::proportional(12.0),
                    TENUE,
                );
                pintor.text(
                    egui::pos2(rect.right() - 10.0, rect.top() + 14.0),
                    Align2::RIGHT_CENTER,
                    estado,
                    FontId::proportional(11.5),
                    color,
                );
                // Al pasar: pausar / activar sin abrirla.
                if encima {
                    let caja = egui::Rect::from_center_size(
                        egui::pos2(rect.right() - 22.0, rect.bottom() - 13.0),
                        Vec2::new(30.0, 20.0),
                    );
                    let r = ui.interact(caja, egui::Id::unique(("pausar_pub", p.id)), Sense::click());
                    let texto = if p.activa { "⏸" } else { "⏵" };
                    ui.painter().rect_filled(caja, 4.0, if r.hovered() { BORDE } else { FONDO });
                    ui.painter().text(caja.center(), Align2::CENTER_CENTER, texto, FontId::proportional(12.0), TEXTO);
                    if r.on_hover_text(if p.activa { "Pausar" } else { "Activar" }).clicked() {
                        alternar = Some(p.id);
                    }
                }
                let resp = resp.on_hover_text(format!(
                    "{}\n{} · {}\n{}",
                    p.clip,
                    p.describir_dias(),
                    resumen_horas(&p.horas, 12),
                    if p.notas.is_empty() { "" } else { &p.notas }
                ));
                if resp.clicked() && alternar.is_none() {
                    abrir = Some(p.id);
                }
                ui.add_space(3.0);
            }
            if let Some(id) = alternar {
                if let Some(p) = e.pautas.iter().find(|p| p.id == id) {
                    let mut q = p.clone();
                    q.activa = !q.activa;
                    self.ctl.enviar(Orden::GuardarPauta(q));
                    if self.publicidades.borrador.id == id && !self.publicidades.hay_cambios() {
                        self.publicidades.borrador.activa = !p.activa;
                        self.publicidades.original.activa = !p.activa;
                    }
                }
            }
            if let Some(id) = abrir {
                if id != self.publicidades.borrador.id {
                    self.pedir_abrir(Some(id), hoy, e);
                }
            }
        });
    }

    // ------------------------------------------------------------------
    // Ficha
    // ------------------------------------------------------------------

    fn ficha_publicidad(&mut self, ui: &mut egui::Ui, e: &Instantanea, hoy: time::Date, con_vista: bool) {
        let nueva = self.publicidades.borrador.id == 0;
        // Encabezado de la ficha.
        ui.horizontal(|ui| {
            let titulo = if nueva {
                "Nueva publicidad".to_string()
            } else if self.publicidades.borrador.nombre.trim().is_empty() {
                "Publicidad".to_string()
            } else {
                self.publicidades.borrador.nombre.clone()
            };
            ui.add(egui::Label::new(RichText::new(titulo).size(19.0).strong().family(tema::negrita())).truncate());
            if let Some(p) = e.pautas.iter().find(|p| p.id == self.publicidades.borrador.id && !nueva) {
                let (t, c) = estado_de(p, hoy);
                tema::chip(ui, t, c, TARJETA);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !nueva {
                    let texto =
                        if self.publicidades.confirmar_borrar { "¿Borrar? Pulsa otra vez" } else { "🗑 Borrar" };
                    if ui.add(egui::Button::new(RichText::new(texto).color(AIRE))).clicked() {
                        if self.publicidades.confirmar_borrar {
                            self.ctl.enviar(Orden::BorrarPauta(self.publicidades.borrador.id));
                            self.publicidades.nueva(hoy);
                        } else {
                            self.publicidades.confirmar_borrar = true;
                        }
                    }
                    if ui
                        .button("🗐 Duplicar")
                        .on_hover_text("Crear otra igual (por ejemplo, para otro horario)")
                        .clicked()
                    {
                        let mut b = self.publicidades.borrador.clone();
                        b.id = 0;
                        b.nombre = format!("{} (copia)", b.nombre);
                        self.publicidades.original = Borrador::nuevo(hoy, self.publicidades.tolerancia_defecto);
                        self.publicidades.borrador = b;
                    }
                }
            });
        });
        ui.add_space(6.0);

        // Pie fijo con Guardar / Descartar: se reserva su alto.
        let alto_pie = 52.0;
        let alto = (ui.available_height() - alto_pie).max(100.0);
        egui::ScrollArea::vertical().id_salt("ficha_pub").max_height(alto).auto_shrink([false, false]).show(ui, |ui| {
            self.seccion_spot(ui, e);
            self.seccion_vigencia(ui, hoy);
            self.seccion_dias(ui);
            self.seccion_horarios(ui, e);
            self.seccion_opciones(ui);
            if con_vista {
                seccion(ui, "CÓMO QUEDA", |ui| self.vista_publicidad(ui, e, hoy));
            }
        });
        ui.separator();
        self.pie_ficha(ui, hoy);
    }

    fn seccion_spot(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let video_sel = self.video_seleccionado(e);
        let clientes: Vec<String> = {
            let mut v: Vec<String> =
                e.pautas.iter().map(|p| p.cliente.trim().to_string()).filter(|c| !c.is_empty()).collect();
            v.sort_by_key(|c| c.to_lowercase());
            v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            v
        };
        seccion(ui, "SPOT", |ui| {
            ui.horizontal_top(|ui| {
                // Miniatura del video.
                let (rect, _) = ui.allocate_exact_size(Vec2::new(176.0, 99.0), Sense::hover());
                let clip = self.publicidades.borrador.clip.clone();
                if clip.is_empty() {
                    ui.painter().rect_filled(rect, 6.0, FONDO);
                    ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, BORDE), StrokeKind::Inside);
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "Sin video",
                        FontId::proportional(13.0),
                        MUY_TENUE,
                    );
                } else {
                    self.pintar_miniatura(ui, rect, Some(&clip), &clip, false);
                    let d = self.publicidades.borrador.duracion_seg;
                    let caja =
                        egui::Rect::from_min_size(rect.right_bottom() - Vec2::new(48.0, 22.0), Vec2::new(44.0, 18.0));
                    ui.painter().rect_filled(caja, 4.0, Color32::from_black_alpha(190));
                    ui.painter().text(caja.center(), Align2::CENTER_CENTER, mmss(d), FontId::monospace(12.0), TEXTO);
                }
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    let b = &mut self.publicidades.borrador;
                    egui::Grid::new("pub_spot").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label(RichText::new("Cliente").color(TENUE));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut b.cliente)
                                    .hint_text("Ej.: Ferretería X")
                                    .desired_width(220.0),
                            );
                            if !clientes.is_empty() {
                                ui.menu_button("⏷", |ui| {
                                    ui.label(RichText::new("Clientes que ya tienes").size(11.0).color(MUY_TENUE));
                                    for c in &clientes {
                                        if ui.button(c).clicked() {
                                            b.cliente = c.clone();
                                            ui.close();
                                        }
                                    }
                                })
                                .response
                                .on_hover_text("Elegir un cliente que ya existe");
                            }
                        });
                        ui.end_row();
                        ui.label(RichText::new("Nombre del spot").color(TENUE));
                        ui.add(
                            egui::TextEdit::singleline(&mut b.nombre)
                                .hint_text("Ej.: Ofertas de diciembre 30 s")
                                .desired_width(260.0),
                        );
                        ui.end_row();
                        ui.label(RichText::new("Video").color(TENUE));
                        if b.clip.is_empty() {
                            ui.label(RichText::new("Elige uno de la lista de abajo").color(AMBAR));
                        } else {
                            ui.add(
                                egui::Label::new(RichText::new(&b.clip).strong().family(tema::negrita())).truncate(),
                            );
                        }
                        ui.end_row();
                    });
                    if let Some((clip, dur)) = &video_sel {
                        if *clip != b.clip
                            && ui
                                .button(format!(
                                    "Usar el elegido en Archivos: {}",
                                    clip.rsplit('/').next().unwrap_or(clip)
                                ))
                                .clicked()
                        {
                            b.clip = clip.clone();
                            b.duracion_seg = dur.as_secs_f64();
                        }
                    }
                });
            });
            ui.add_space(6.0);
            // Videos de CasparCG para elegir.
            ui.add(
                egui::TextEdit::singleline(&mut self.publicidades.filtro_video)
                    .hint_text("🔍  Buscar el video en CasparCG…")
                    .desired_width(f32::INFINITY),
            );
            let filtro = self.publicidades.filtro_video.to_lowercase();
            egui::Frame::new().fill(FONDO).corner_radius(6).inner_margin(4).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("videos_pub").max_height(120.0).auto_shrink([false, true]).show(
                    ui,
                    |ui| {
                        ui.set_width(ui.available_width());
                        let mut n = 0;
                        for m in e.medios.iter().filter(|m| m.tipo == amcp::TipoMedio::Movie) {
                            if !filtro.is_empty() && !m.nombre.to_lowercase().contains(&filtro) {
                                continue;
                            }
                            n += 1;
                            let b = &mut self.publicidades.borrador;
                            let sel = m.nombre == b.clip;
                            let r = ui.add(egui::Button::selectable(
                                sel,
                                RichText::new(format!("{}   {}", m.nombre, mmss(m.duracion_seg))).size(13.0),
                            ));
                            if r.clicked() {
                                b.clip = m.nombre.clone();
                                b.duracion_seg = m.duracion_seg;
                                if b.nombre.trim().is_empty() {
                                    b.nombre = m.nombre.rsplit('/').next().unwrap_or(&m.nombre).to_string();
                                }
                            }
                        }
                        if n == 0 {
                            ui.label(RichText::new("No hay videos en CasparCG con ese nombre").color(TENUE));
                        }
                    },
                );
            });
        });
    }

    fn seccion_vigencia(&mut self, ui: &mut egui::Ui, hoy: time::Date) {
        seccion(ui, "VIGENCIA", |ui| {
            let b = &mut self.publicidades.borrador;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Desde").color(TENUE));
                ui.add(egui::TextEdit::singleline(&mut b.inicio).hint_text("DD/MM/AAAA").desired_width(100.0));
                if ui.small_button("Hoy").clicked() {
                    b.inicio = formato::texto_fecha_dma(hoy);
                }
                ui.add_space(12.0);
                ui.label(RichText::new("hasta").color(TENUE));
                ui.add_enabled(
                    !b.sin_fin,
                    egui::TextEdit::singleline(&mut b.fin).hint_text("DD/MM/AAAA").desired_width(100.0),
                );
                ui.checkbox(&mut b.sin_fin, "Sin fecha de fin");
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Atajos:").size(12.0).color(MUY_TENUE));
                let desde = formato::leer_fecha_dma(&b.inicio).unwrap_or(hoy);
                for (t, dias) in [("1 semana", 6), ("15 días", 14), ("1 mes", 29), ("3 meses", 89)] {
                    if ui.small_button(t).on_hover_text("Desde la fecha de inicio").clicked() {
                        b.sin_fin = false;
                        b.fin = formato::texto_fecha_dma(desde + time::Duration::days(dias));
                    }
                }
                if ui.small_button("Fin de mes").clicked() {
                    let mut f = desde;
                    while let Some(s) = f.next_day() {
                        if s.month() != desde.month() {
                            break;
                        }
                        f = s;
                    }
                    b.sin_fin = false;
                    b.fin = formato::texto_fecha_dma(f);
                }
            });
        });
    }

    fn seccion_dias(&mut self, ui: &mut egui::Ui) {
        seccion(ui, "DÍAS", |ui| {
            let b = &mut self.publicidades.borrador;
            ui.horizontal_wrapped(|ui| {
                for (v, t) in [(0, "Todos los días"), (1, "Algunos días"), (2, "Intercalado"), (3, "Fechas sueltas")]
                {
                    let activo = b.tipo_dias == v;
                    let boton =
                        egui::Button::new(RichText::new(t).size(13.0).color(if activo { TEXTO } else { TENUE }))
                            .fill(if activo { ACENTO_OSCURO } else { Color32::TRANSPARENT })
                            .stroke(Stroke::new(1.0, if activo { ACENTO } else { BORDE }))
                            .min_size(Vec2::new(0.0, 28.0));
                    if ui.add(boton).clicked() {
                        b.tipo_dias = v;
                    }
                }
            });
            ui.add_space(4.0);
            match b.tipo_dias {
                1 => {
                    ui.horizontal_wrapped(|ui| {
                        for (i, d) in DIAS_CORTOS.iter().enumerate() {
                            let on = b.semana[i];
                            let boton =
                                egui::Button::new(RichText::new(*d).strong().family(tema::negrita()).color(if on {
                                    Color32::WHITE
                                } else {
                                    TENUE
                                }))
                                .fill(if on { ACENTO_OSCURO } else { TARJETA })
                                .stroke(Stroke::new(1.0, if on { ACENTO } else { BORDE }))
                                .min_size(Vec2::new(36.0, 32.0));
                            if ui.add(boton).on_hover_text(DIAS_SEMANA[i]).clicked() {
                                b.semana[i] = !on;
                            }
                        }
                        ui.add_space(8.0);
                        if ui.small_button("Lun a Vie").clicked() {
                            b.semana = [true, true, true, true, true, false, false];
                        }
                        if ui.small_button("Fin de semana").clicked() {
                            b.semana = [false, false, false, false, false, true, true];
                        }
                    });
                }
                2 => {
                    ui.horizontal(|ui| {
                        ui.label("Un día sí y");
                        let mut descanso = b.cada_n.saturating_sub(1).max(1);
                        ui.add(egui::DragValue::new(&mut descanso).range(1..=30));
                        ui.label("día(s) no, empezando en la fecha de inicio.");
                        b.cada_n = descanso + 1;
                    });
                }
                3 => {
                    ui.add(
                        egui::TextEdit::multiline(&mut b.fechas)
                            .hint_text("15/11/2026, 22/11/2026, …")
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                }
                _ => {
                    ui.label(RichText::new("Sale todos los días de la vigencia.").size(12.5).color(TENUE));
                }
            }
        });
    }

    fn seccion_horarios(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let recuperar = self.cfg.pauta.recuperar_min;
        seccion(ui, "HORARIOS", |ui| {
            let est = &mut self.publicidades;
            let (antes, despues) = (est.borrador.tolerancia, est.borrador.despues);
            // Las horas puestas, como fichas que se pueden quitar.
            ui.horizontal_wrapped(|ui| {
                let mut quitar = None;
                for (i, h) in est.borrador.horas.iter().enumerate() {
                    let r = egui::Frame::new()
                        .fill(ACENTO_OSCURO)
                        .stroke(Stroke::new(1.0, ACENTO))
                        .corner_radius(99)
                        .inner_margin(egui::Margin { left: 10, right: 4, top: 2, bottom: 2 })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                ui.add(
                                    egui::Label::new(RichText::new(tema::hora(*h)).monospace().strong().color(TEXTO))
                                        .selectable(false),
                                );
                                if ui
                                    .add(egui::Button::new(RichText::new("×").size(13.0)).frame(false))
                                    .on_hover_text("Quitar")
                                    .clicked()
                                {
                                    quitar = Some(i);
                                }
                            });
                        });
                    r.response.on_hover_text(format!("Puede salir entre {}", texto_ventana(*h, antes, despues)));
                }
                if let Some(i) = quitar {
                    est.borrador.horas.remove(i);
                }
                if est.borrador.horas.is_empty() {
                    ui.label(RichText::new("Todavía sin horarios").color(AMBAR));
                } else {
                    ui.label(RichText::new(format!("{} al día", est.borrador.horas.len())).size(12.0).color(TENUE));
                    if ui.small_button("Quitar todas").clicked() {
                        est.borrador.horas.clear();
                    }
                }
            });
            ui.add_space(6.0);
            // Escribir o pegar horas.
            ui.horizontal(|ui| {
                let ancho = (ui.available_width() - 90.0).clamp(200.0, 460.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut est.borrador.nueva_hora)
                        .hint_text("Escribe o pega: 6:00, 8:30, 10:45 ... 8:00 PM, 10 pm")
                        .desired_width(ancho),
                );
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("+ Añadir").clicked() || enter {
                    match est.borrador.agregar_horas() {
                        Ok(n) => {
                            est.error = None;
                            est.aviso_horas = Some(match n {
                                0 => "Esas horas ya estaban.".to_string(),
                                1 => "Se añadió 1 hora.".to_string(),
                                n => format!("Se añadieron {n} horas."),
                            });
                        }
                        Err(err) => est.error = Some(err),
                    }
                    if enter {
                        r.request_focus();
                    }
                }
            });
            ui.label(
                RichText::new("Sirve en 24 h (20:30) o con AM/PM (8:30 pm). Puedes pegar la lista entera del cliente.")
                    .size(11.5)
                    .color(MUY_TENUE),
            );
            if let Some(a) = &est.aviso_horas {
                ui.label(RichText::new(a).size(12.0).color(TV));
            }
            ui.add_space(4.0);
            // Plantillas y copiar de otra publicidad.
            ui.horizontal_wrapped(|ui| {
                let mut aplicar: Option<(Vec<time::Time>, bool, String)> = None;
                let mut borrar: Option<String> = None;
                egui::ComboBox::from_id_salt("pub_plantillas").width(150.0).selected_text("📋 Plantillas").show_ui(
                    ui,
                    |ui| {
                        if est.plantillas.lista.is_empty() {
                            ui.label(
                                RichText::new(
                                    "Aún no hay plantillas.\nPon unas horas y pulsa «Guardar como plantilla».",
                                )
                                .size(12.0)
                                .color(TENUE),
                            );
                        }
                        for p in &est.plantillas.lista {
                            ui.horizontal(|ui| {
                                let t = format!("{}  ·  {}", p.nombre, resumen_horas(&p.horas, 5));
                                if ui
                                    .selectable_label(false, t)
                                    .on_hover_text(format!(
                                        "{}\nClic: usar estas horas (reemplaza las de ahora)",
                                        resumen_horas(&p.horas, 60)
                                    ))
                                    .clicked()
                                {
                                    aplicar = Some((p.horas.clone(), true, p.nombre.clone()));
                                }
                                if ui.small_button("+").on_hover_text("Añadirlas a las que ya hay").clicked() {
                                    aplicar = Some((p.horas.clone(), false, p.nombre.clone()));
                                }
                                if ui.small_button("🗑").on_hover_text("Borrar la plantilla").clicked() {
                                    borrar = Some(p.nombre.clone());
                                }
                            });
                        }
                    },
                );
                // Copiar las horas de otra publicidad.
                let otras: Vec<&Pauta> =
                    e.pautas.iter().filter(|p| p.id != est.borrador.id && !p.horas.is_empty()).collect();
                if !otras.is_empty() {
                    egui::ComboBox::from_id_salt("pub_copiar_horas")
                        .width(170.0)
                        .selected_text("🗐 Copiar de otra…")
                        .show_ui(ui, |ui| {
                            for p in otras {
                                let t = format!("{} · {}", p.nombre, resumen_horas(&p.horas, 4));
                                if ui.selectable_label(false, t).on_hover_text(resumen_horas(&p.horas, 60)).clicked() {
                                    aplicar = Some((p.horas.clone(), true, p.nombre.clone()));
                                }
                            }
                        });
                }
                if !est.borrador.horas.is_empty() {
                    if est.nombre_plantilla.is_none() {
                        if ui
                            .button("💾 Guardar como plantilla")
                            .on_hover_text("Para usar estas mismas horas en otras publicidades")
                            .clicked()
                        {
                            est.nombre_plantilla = Some(String::new());
                            est.enfocar_plantilla = true;
                        }
                    } else {
                        let mut cerrar = false;
                        if let Some(nombre) = est.nombre_plantilla.as_mut() {
                            let r = ui.add(
                                egui::TextEdit::singleline(nombre)
                                    .hint_text("Nombre (p. ej. Pauta diaria)")
                                    .desired_width(170.0),
                            );
                            if std::mem::take(&mut est.enfocar_plantilla) {
                                r.request_focus();
                            }
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if ui.button("Guardar").clicked() || enter {
                                match est.plantillas.guardar(nombre, &est.borrador.horas) {
                                    Ok(()) => {
                                        est.aviso_horas = Some(format!("Plantilla «{}» guardada.", nombre.trim()));
                                        est.error = None;
                                        cerrar = true;
                                    }
                                    Err(err) => est.error = Some(err),
                                }
                            }
                            if ui.button("Cancelar").clicked() {
                                cerrar = true;
                            }
                        }
                        if cerrar {
                            est.nombre_plantilla = None;
                        }
                    }
                }
                if let Some((horas, reemplazar, de)) = aplicar {
                    if reemplazar {
                        est.borrador.horas.clear();
                    }
                    for h in horas {
                        est.borrador.poner_hora(h);
                    }
                    est.aviso_horas = Some(format!(
                        "{} las horas de «{de}».",
                        if reemplazar { "Se pusieron" } else { "Se añadieron" }
                    ));
                }
                if let Some(n) = borrar {
                    match est.plantillas.borrar(&n) {
                        Ok(()) => est.aviso_horas = Some(format!("Plantilla «{n}» borrada.")),
                        Err(err) => est.error = Some(err),
                    }
                }
            });
            // Generar: cada X entre A y B.
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("O repetir cada").size(12.5).color(TENUE));
                egui::ComboBox::from_id_salt("pub_cada")
                    .width(90.0)
                    .selected_text(texto_intervalo(est.gen_cada))
                    .show_ui(ui, |ui| {
                        for m in [15, 30, 45, 60, 90, 120, 180, 240] {
                            ui.selectable_value(&mut est.gen_cada, m, texto_intervalo(m));
                        }
                    });
                ui.label(RichText::new("desde").size(12.5).color(TENUE));
                ui.add(egui::TextEdit::singleline(&mut est.gen_desde).desired_width(70.0));
                ui.label(RichText::new("hasta").size(12.5).color(TENUE));
                ui.add(egui::TextEdit::singleline(&mut est.gen_hasta).desired_width(70.0));
                if ui.button("Generar").on_hover_text("Añade esas horas a las que ya hay").clicked() {
                    match est.borrador.generar_horas(&est.gen_desde, &est.gen_hasta, est.gen_cada) {
                        Ok(n) => {
                            est.error = None;
                            est.aviso_horas = Some(format!("Se añadieron {n} horas."));
                        }
                        Err(err) => est.error = Some(err),
                    }
                }
            });

            // Ventana: cuánto antes y cuánto después de su hora puede salir.
            ui.add_space(10.0);
            ui.label(RichText::new("¿Qué tan exacta es la hora?").strong().color(TEXTO));
            ui.label(
                RichText::new(
                    "El canal no corta siempre a la misma hora: cada hora es aproximada y el spot entra en el primer corte que caiga dentro de su margen.",
                )
                .size(11.5)
                .color(MUY_TENUE),
            );
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Puede salir desde").color(TENUE));
                ui.add(egui::DragValue::new(&mut est.borrador.tolerancia).range(0..=180).speed(1).suffix(" min"));
                ui.label(RichText::new("antes hasta").color(TENUE));
                ui.add(egui::DragValue::new(&mut est.borrador.despues).range(0..=240).speed(1).suffix(" min"));
                ui.label(RichText::new("después de cada hora.").color(TENUE));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Rápido:").size(12.0).color(MUY_TENUE));
                for (t, a, d, ayuda) in [
                    ("Exacta", 5, 5, "± 5 min"),
                    ("Normal", 15, 15, "± 15 min"),
                    ("Suele atrasarse", 10, 45, "10 min antes, 45 después"),
                    ("Flexible", 30, 60, "30 min antes, 1 h después"),
                ] {
                    let elegido = est.borrador.tolerancia == a && est.borrador.despues == d;
                    if ui.selectable_label(elegido, t).on_hover_text(ayuda).clicked() {
                        est.borrador.tolerancia = a;
                        est.borrador.despues = d;
                    }
                }
            });
            if let Some(h) = est.borrador.horas.first() {
                let mut t = format!(
                    "Ejemplo: la de {} sale en un corte entre {}.",
                    tema::hora(*h),
                    texto_ventana(*h, antes, despues)
                );
                if recuperar > 0 {
                    t.push_str(&format!(
                        " Si no hubo corte en ese margen, queda pendiente y entra solo en los cortes de las {} siguientes.",
                        texto_intervalo(recuperar)
                    ));
                }
                ui.label(RichText::new(t).size(12.0).color(TENUE));
            }
        });
    }

    fn seccion_opciones(&mut self, ui: &mut egui::Ui) {
        seccion(ui, "OPCIONES", |ui| {
            let b = &mut self.publicidades.borrador;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Prioridad").color(TENUE));
                for (v, t, ayuda) in [
                    (0u8, "Normal", "Sale en su orden"),
                    (1, "★ Alta", "Sale antes que las normales"),
                    (2, "★★ Muy alta", "Sale primero en la tanda"),
                ] {
                    let activo = b.prioridad == v;
                    let color = if v > 0 { AMBAR } else { TEXTO };
                    let boton =
                        egui::Button::new(RichText::new(t).size(13.0).color(if activo { color } else { TENUE }))
                            .fill(if activo { ACENTO_OSCURO } else { Color32::TRANSPARENT })
                            .stroke(Stroke::new(1.0, if activo { ACENTO } else { BORDE }))
                            .min_size(Vec2::new(0.0, 28.0));
                    if ui.add(boton).on_hover_text(ayuda).clicked() {
                        b.prioridad = v;
                    }
                }
                ui.add_space(16.0);
                ui.checkbox(&mut b.activa, "Activa").on_hover_text("Desmárcala para pausarla sin borrarla");
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Notas").color(TENUE));
            ui.add(
                egui::TextEdit::multiline(&mut b.notas)
                    .hint_text("Contrato, contacto, observaciones…")
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
        });
    }

    fn pie_ficha(&mut self, ui: &mut egui::Ui, hoy: time::Date) {
        let cambios = self.publicidades.hay_cambios();
        ui.horizontal(|ui| {
            ui.set_min_height(40.0);
            if let Some(err) = &self.publicidades.error {
                ui.label(RichText::new(format!("⚠ {err}")).color(AIRE));
            } else if cambios {
                ui.label(RichText::new("● Cambios sin guardar").color(AMBAR));
            } else if self.publicidades.borrador.id != 0 {
                ui.label(RichText::new("✔ Guardada").color(TV));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let texto = if self.publicidades.borrador.id == 0 { "Crear publicidad" } else { "Guardar cambios" };
                let guardar =
                    egui::Button::new(RichText::new(texto).strong().family(tema::negrita()).color(Color32::WHITE))
                        .fill(Color32::from_rgb(0x1F, 0x7A, 0x4A))
                        .min_size(Vec2::new(150.0, 34.0));
                let listo = cambios || self.publicidades.borrador.id == 0;
                if ui.add_enabled(listo, guardar).on_hover_text("Ctrl+S").clicked()
                    || (listo && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)))
                {
                    self.guardar_publicidad(hoy);
                }
                if cambios && ui.button("Descartar").clicked() {
                    let original = self.publicidades.original.clone();
                    self.publicidades.cargar(original);
                }
            });
        });
    }

    fn guardar_publicidad(&mut self, hoy: time::Date) {
        let est = &mut self.publicidades;
        // Una hora escrita sin pulsar «Añadir» se toma igual.
        if !est.borrador.nueva_hora.trim().is_empty() {
            if let Err(err) = est.borrador.agregar_horas() {
                est.error = Some(err);
                return;
            }
        }
        match est.borrador.validar() {
            Ok(p) => {
                let nueva = p.id == 0;
                self.ctl.enviar(Orden::GuardarPauta(p.clone()));
                est.error = None;
                if nueva {
                    est.esperando = Some(p);
                } else {
                    est.original = est.borrador.clone();
                }
                let _ = hoy;
            }
            Err(err) => est.error = Some(err),
        }
    }

    // ------------------------------------------------------------------
    // Cómo queda
    // ------------------------------------------------------------------

    fn vista_publicidad(&mut self, ui: &mut egui::Ui, e: &Instantanea, hoy: time::Date) {
        let b = &self.publicidades.borrador;
        tema::titulo(ui, "HOY");
        ui.add_space(4.0);
        // Los pases de hoy de esta publicidad (si ya está guardada).
        let prefijo = format!("{}@", b.id);
        let hoy_pases: Vec<&nucleo::PaseVista> =
            if b.id == 0 { Vec::new() } else { e.pases_hoy.iter().filter(|p| p.clave.starts_with(&prefijo)).collect() };
        if hoy_pases.is_empty() {
            let t = if b.id == 0 { "Guárdala para verla en la pauta de hoy." } else { "No sale hoy." };
            ui.label(RichText::new(t).size(12.5).color(TENUE));
        }
        for p in hoy_pases {
            let (t, c) = match p.estado {
                EstadoPase::Emitido => ("✔ salió", TV),
                EstadoPase::Vigente => ("AHORA", AMBAR),
                EstadoPase::Pendiente => ("pendiente", AMBAR),
                EstadoPase::Perdido => ("perdido", AIRE),
                EstadoPase::Programado => ("programado", TENUE),
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(crate::tema::hora_txt(&p.hora)).monospace().strong());
                ui.label(RichText::new(t).size(12.0).color(c));
                if let Some(h) = formato::leer_hora(&p.hora) {
                    ui.label(
                        RichText::new(format!("entre {}", texto_ventana(h, b.tolerancia, b.despues)))
                            .size(11.0)
                            .color(MUY_TENUE),
                    );
                }
            });
        }

        ui.add_space(12.0);
        tema::titulo(ui, "PRÓXIMOS 7 DÍAS");
        ui.add_space(4.0);
        let pauta = match b.validar() {
            Ok(p) => p,
            Err(_) => {
                ui.label(RichText::new("Completa el video y los horarios para verla.").size(12.5).color(TENUE));
                return;
            }
        };
        let mut total = 0;
        let ancho = ui.available_width();
        for k in 0..7 {
            let fecha = hoy + time::Duration::days(k);
            let toca = pauta.toca(fecha);
            if toca {
                total += pauta.horas.len();
            }
            let dia = DIAS_SEMANA[fecha.weekday().number_days_from_monday() as usize];
            let etiqueta = if k == 0 { "Hoy".to_string() } else { format!("{dia} {:02}", fecha.day()) };
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ancho, 22.0), Sense::hover());
            let p = ui.painter();
            p.text(
                rect.left_center(),
                Align2::LEFT_CENTER,
                &etiqueta,
                FontId::proportional(12.0),
                if k == 0 { TEXTO } else { TENUE },
            );
            // Barra del día (0 a 24 h) con un punto por pase.
            let barra = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 52.0, rect.center().y - 4.0),
                egui::pos2(rect.right() - 4.0, rect.center().y + 4.0),
            );
            p.rect_filled(barra, 4.0, if toca { Color32::from_rgb(0x1C, 0x22, 0x2C) } else { FONDO });
            for h in [6, 12, 18] {
                let x = barra.left() + barra.width() * h as f32 / 24.0;
                p.line_segment([egui::pos2(x, barra.top()), egui::pos2(x, barra.bottom())], Stroke::new(1.0, BORDE));
            }
            if toca {
                for h in &pauta.horas {
                    let min = h.hour() as f32 * 60.0 + h.minute() as f32;
                    let x = barra.left() + barra.width() * min / 1440.0;
                    p.circle_filled(egui::pos2(x, barra.center().y), 4.0, ACENTO);
                }
            }
            if toca {
                resp.on_hover_text(format!("{etiqueta}: {}", resumen_horas(&pauta.horas, 24)));
            } else {
                resp.on_hover_text(format!("{etiqueta}: no sale"));
            }
        }
        ui.horizontal(|ui| {
            ui.add_space(52.0);
            for (t, h) in [("0", 0.0), ("6", 6.0), ("12", 12.0), ("18", 18.0)] {
                let _ = h;
                ui.label(RichText::new(t).size(10.0).color(MUY_TENUE));
                ui.add_space((ancho - 52.0) / 4.0 - 14.0);
            }
        });
        ui.add_space(6.0);
        let segundos = total as f64 * pauta.duracion_seg;
        ui.label(
            RichText::new(format!("{total} pases en 7 días · {} al aire", duracion_larga(segundos)))
                .size(12.5)
                .color(TEXTO),
        );
    }
}

/// Un bloque de la ficha con su título.
fn seccion(ui: &mut egui::Ui, titulo: &str, contenido: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(TARJETA)
        .stroke(Stroke::new(1.0, BORDE))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(titulo).size(11.0).strong().family(tema::negrita()).color(MUY_TENUE));
            ui.add_space(6.0);
            contenido(ui);
        });
    ui.add_space(8.0);
}

/// «10:25 y 11:00»: el margen en que puede salir el pase de las `h`.
fn texto_ventana(h: time::Time, antes: u32, despues: u32) -> String {
    let m = h.hour() as i32 * 60 + h.minute() as i32;
    let hora = |m: i32| {
        let m = m.clamp(0, 23 * 60 + 59);
        time::Time::from_hms((m / 60) as u8, (m % 60) as u8, 0).map(tema::hora).unwrap_or_default()
    };
    format!("{} y {}", hora(m - antes as i32), hora(m + despues as i32))
}

fn texto_intervalo(min: u32) -> String {
    match min {
        m if m < 60 => format!("{m} min"),
        60 => "1 hora".into(),
        m if m % 60 == 0 => format!("{} horas", m / 60),
        m => format!("{} h {} min", m / 60, m % 60),
    }
}
