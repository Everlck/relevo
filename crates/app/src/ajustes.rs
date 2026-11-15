//! Ventana ⚙ Ajustes: cambia `config.toml` desde la app (sin perder sus comentarios).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, RichText, Sense, Stroke, Vec2};

use crate::arranque::{self, Monitor, SalidaPantalla};
use crate::config::Config;
use crate::dialogo::{self, Dialogo, Tono};
use crate::tema::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Seccion {
    CasparCg,
    Salida,
    Iptv,
    Archivos,
    Pauta,
    VistaPrevia,
    Biblioteca,
    Red,
    Respaldo,
    Interfaz,
    Atajos,
}

impl Seccion {
    fn todas(remoto: bool) -> Vec<Seccion> {
        use Seccion::*;
        if remoto {
            vec![Red, Archivos, VistaPrevia, Interfaz, Atajos]
        } else {
            vec![CasparCg, Salida, Iptv, Archivos, Pauta, VistaPrevia, Biblioteca, Red, Respaldo, Interfaz, Atajos]
        }
    }

    fn nombre(self) -> (&'static str, &'static str) {
        match self {
            Seccion::CasparCg => ("📡", "CasparCG"),
            Seccion::Salida => ("🖥", "Salida de video"),
            Seccion::Iptv => ("📺", "Señal por IPTV"),
            Seccion::Archivos => ("🎞", "Archivos y ffmpeg"),
            Seccion::Pauta => ("📅", "Pauta y corte"),
            Seccion::VistaPrevia => ("▶", "Vista previa"),
            Seccion::Biblioteca => ("📚", "Biblioteca"),
            Seccion::Red => ("🔗", "Red y API"),
            Seccion::Respaldo => ("💾", "Respaldos"),
            Seccion::Interfaz => ("✨", "Interfaz"),
            Seccion::Atajos => ("⌨", "Atajos de teclado"),
        }
    }
}

/// Lo que pasó al cerrar la ventana.
pub enum Resultado {
    Nada,
    /// Se guardó. `reiniciar_app`: hay cambios que se aplican al reiniciar Relevo.
    Guardado {
        cfg: Box<Config>,
        reiniciar_app: bool,
        salida_cambiada: bool,
    },
    /// Pidió reiniciar Relevo ahora (después de guardar).
    ReiniciarApp,
}

/// Canales leídos de una lista .m3u: (nombre, dirección), o el error.
type ListaCanales = Result<Vec<(String, String)>, String>;

pub struct VentanaAjustes {
    pub abierta: bool,
    seccion: Seccion,
    original: Config,
    b: Config,
    relleno: String,
    ruta: PathBuf,
    remoto: bool,
    monitores: Arc<Mutex<Option<Vec<Monitor>>>>,
    error: Option<String>,
    /// Después de guardar: hay que reiniciar Relevo para que todo se aplique.
    falta_reiniciar: bool,
    /// Atajo que está esperando una tecla (su clave).
    capturando: Option<&'static str>,
    /// Canales de la lista .m3u (se leen en segundo plano).
    canales: Arc<Mutex<Option<ListaCanales>>>,
    cargando_lista: bool,
    filtro_canal: String,
}

impl VentanaAjustes {
    pub fn new(cfg: &Config, ruta: PathBuf) -> Self {
        VentanaAjustes {
            abierta: false,
            seccion: if cfg.remoto.es_remoto() { Seccion::Red } else { Seccion::CasparCg },
            original: cfg.clone(),
            b: cfg.clone(),
            relleno: cfg.pauta.relleno.join(", "),
            ruta,
            remoto: cfg.remoto.es_remoto(),
            monitores: Arc::new(Mutex::new(None)),
            error: None,
            falta_reiniciar: false,
            capturando: None,
            canales: Arc::new(Mutex::new(None)),
            cargando_lista: false,
            filtro_canal: String::new(),
        }
    }

    /// Abre la ventana con los valores actuales.
    pub fn abrir(&mut self, cfg: &Config) {
        self.original = cfg.clone();
        self.b = cfg.clone();
        self.relleno = cfg.pauta.relleno.join(", ");
        self.error = None;
        self.abierta = true;
        self.buscar_monitores();
    }

    /// Abre la ventana directamente en la sección de IPTV.
    pub fn abrir_iptv(&mut self, cfg: &Config) {
        self.abrir(cfg);
        self.seccion = Seccion::Iptv;
    }

    /// Deja guardada en config.toml la configuración de la detección de corte
    /// (se edita en su propia ventana).
    pub fn guardar_deteccion(&mut self, d: &crate::config::DeteccionCfg) {
        self.guardar_con(|c| c.deteccion = d.clone());
    }

    /// Guarda en config.toml un cambio hecho fuera de esta ventana (el modo, el
    /// preview…), sin tocar lo demás.
    pub fn guardar_con(&mut self, f: impl Fn(&mut Config)) {
        let mut nuevo = self.original.clone();
        f(&mut nuevo);
        match guardar_en(&self.ruta, &self.original, &nuevo) {
            Ok(_) => {
                self.original = nuevo;
                f(&mut self.b);
            }
            Err(e) => tracing::warn!("no se pudo guardar: {e}"),
        }
    }

    fn buscar_monitores(&self) {
        let m = self.monitores.clone();
        *m.lock().expect("monitores") = None;
        std::thread::spawn(move || {
            let v = arranque::monitores();
            *m.lock().expect("monitores") = Some(v);
        });
    }

    /// `al_aire`: no se ofrece reiniciar mientras hay publicidad al aire.
    pub fn mostrar(&mut self, ctx: &egui::Context, al_aire: bool) -> Resultado {
        if !self.abierta {
            return Resultado::Nada;
        }
        let mut resultado = Resultado::Nada;
        let mut guardar = false;
        let mut cancelar = false;
        let mut reiniciar = false;
        // Mientras se espera la tecla de un atajo, Esc solo cancela eso.
        let capturaba = self.capturando.is_some();
        let sub = format!("Se guarda en {}", self.ruta.display());
        let r = Dialogo::new("relevo-ajustes", "⚙", "Ajustes").subtitulo(sub).ancho(820.0).mostrar(ctx, |ui| {
            ui.horizontal_top(|ui| {
                // Menú de secciones.
                ui.vertical(|ui| {
                    ui.set_width(190.0);
                    for s in Seccion::todas(self.remoto) {
                        let (icono, nombre) = s.nombre();
                        let sel = self.seccion == s;
                        let b =
                            egui::Button::new(RichText::new(format!("{icono}   {nombre}")).size(13.5).color(if sel {
                                TEXTO
                            } else {
                                TENUE
                            }))
                            .fill(if sel { ACENTO_OSCURO } else { Color32::TRANSPARENT })
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(8))
                            .min_size(Vec2::new(190.0, 34.0));
                        if ui.add(b).clicked() {
                            self.seccion = s;
                        }
                    }
                });
                ui.add_space(8.0);
                let (linea, _) = ui.allocate_exact_size(Vec2::new(1.0, 470.0), Sense::hover());
                ui.painter().rect_filled(linea, 0.0, BORDE);
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical().id_salt("ajustes").max_height(470.0).auto_shrink([false, false]).show(
                        ui,
                        |ui| {
                            ui.set_width(ui.available_width() - 8.0);
                            let a = dialogo::aparicion(
                                ui.ctx(),
                                egui::Id::unique("seccion-ajustes").with(self.seccion as u8),
                                0.2,
                            );
                            ui.multiply_opacity(a);
                            self.contenido(ui, al_aire);
                        },
                    );
                });
            });
            ui.add_space(10.0);
            if let Some(e) = &self.error {
                ui.label(RichText::new(format!("⚠ {e}")).color(AIRE));
            }
            if self.falta_reiniciar {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("✔ Guardado. Algunos cambios se aplican al reiniciar Relevo.").color(TV));
                    let b = dialogo::boton_principal("⟳ Reiniciar Relevo ahora", Tono::Normal);
                    if ui.add_enabled(!al_aire, b).on_disabled_hover_text("Espera a que termine la tanda").clicked() {
                        reiniciar = true;
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui
                    .add(dialogo::boton_secundario("📝 Abrir config.toml"))
                    .on_hover_text("Para lo que no está aquí")
                    .clicked()
                {
                    crate::exportar::abrir(&self.ruta);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let hay_cambios = self.hay_cambios();
                    if hay_cambios {
                        if ui.add(dialogo::boton_principal("Guardar", Tono::Normal)).clicked() {
                            guardar = true;
                        }
                    } else {
                        ui.add_enabled(false, dialogo::boton_secundario("Guardar"))
                            .on_disabled_hover_text("No hay cambios");
                    }
                    let t = if hay_cambios { "Cancelar" } else { "Cerrar" };
                    if ui.add(dialogo::boton_secundario(t)).clicked() {
                        cancelar = true;
                    }
                });
            });
        });
        if (r.cerrar && !capturaba) || cancelar {
            self.capturando = None;
            self.abierta = false;
            self.falta_reiniciar = false;
        }
        if guardar && self.b.api.activa && self.b.api.clave.trim().is_empty() {
            guardar = false;
            self.error = Some("Para activar la API hace falta una clave (Red y API).".into());
            self.seccion = Seccion::Red;
        }
        if guardar && self.b.iptv.activa {
            let i = &self.b.iptv;
            let error = if !nucleo::iptv::es_direccion(&i.direccion) {
                Some("Falta la dirección del canal IPTV (http://, udp://, rtp://…).".to_string())
            } else if i.capa >= self.b.casparcg.capa_publicidad {
                Some(format!(
                    "La capa del canal IPTV tiene que ser menor que la de publicidad ({}), para quedar debajo.",
                    self.b.casparcg.capa_publicidad
                ))
            } else {
                None
            };
            if let Some(e) = error {
                guardar = false;
                self.error = Some(e);
                self.seccion = Seccion::Iptv;
            }
        }
        if guardar && self.b.remoto.es_remoto() && self.b.remoto.servidor.trim().is_empty() {
            guardar = false;
            self.error = Some("Indica la dirección de la PC principal (Red y API).".into());
            self.seccion = Seccion::Red;
        }
        if guardar {
            self.b.pauta.relleno =
                self.relleno.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            match guardar_en(&self.ruta, &self.original, &self.b) {
                Ok(cambios) => {
                    let reiniciar_app = cambios.iter().any(|c| !se_aplica_ya(c));
                    let salida_cambiada = cambios.iter().any(|c| c == "casparcg.pantalla");
                    self.original = self.b.clone();
                    self.error = None;
                    if reiniciar_app {
                        self.falta_reiniciar = true;
                    } else {
                        self.abierta = false;
                    }
                    resultado = Resultado::Guardado { cfg: Box::new(self.b.clone()), reiniciar_app, salida_cambiada };
                }
                Err(e) => self.error = Some(e),
            }
        }
        if reiniciar {
            resultado = Resultado::ReiniciarApp;
        }
        resultado
    }

    fn hay_cambios(&self) -> bool {
        let mut b = self.b.clone();
        b.pauta.relleno = self.relleno.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        !cambios(&self.original, &b).is_empty()
    }

    fn contenido(&mut self, ui: &mut egui::Ui, al_aire: bool) {
        let _ = al_aire;
        match self.seccion {
            Seccion::CasparCg => {
                titulo(ui, "CasparCG", "Dónde está CasparCG y en qué canal y capa sale la publicidad.");
                rejilla(ui, "g-caspar", |ui| {
                    campo(ui, "Dirección (IP)", "127.0.0.1 si CasparCG está en esta PC", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.b.casparcg.host).desired_width(200.0));
                    });
                    campo(ui, "Puerto AMCP", "Normalmente 5250", |ui| {
                        ui.add(egui::DragValue::new(&mut self.b.casparcg.puerto).range(1..=65535));
                    });
                    campo(ui, "Canal", "Canal de salida al aire", |ui| {
                        ui.add(egui::DragValue::new(&mut self.b.casparcg.canal).range(1..=16));
                    });
                    campo(ui, "Capa de publicidad", "Capa donde se reproducen los spots", |ui| {
                        ui.add(egui::DragValue::new(&mut self.b.casparcg.capa_publicidad).range(1..=999));
                    });
                    campo(ui, "Puerto OSC", "Para el tiempo exacto (6250)", |ui| {
                        ui.add(egui::DragValue::new(&mut self.b.casparcg.osc_puerto).range(1..=65535));
                    });
                    campo(ui, "Carpeta media", "La carpeta media de CasparCG (vacía = la del casparcg.config)", |ui| {
                        carpeta(ui, &mut self.b.casparcg.carpeta_medios);
                    });
                    campo(ui, "Carpeta de CasparCG", "Donde está casparcg.exe (vacía = buscarla sola)", |ui| {
                        carpeta(ui, &mut self.b.casparcg.carpeta_programa);
                    });
                    campo(ui, "Abrir al iniciar", "Abrir CasparCG y su scanner al abrir Relevo", |ui| {
                        ui.checkbox(&mut self.b.casparcg.iniciar_automaticamente, "Sí");
                    });
                    campo(
                        ui,
                        "Detener al cerrar Relevo",
                        "Si cierras Relevo con una publicidad al aire, se detiene y vuelve la señal de TV. Desmárcalo para que CasparCG siga reproduciendo",
                        |ui| {
                            ui.checkbox(&mut self.b.casparcg.detener_al_cerrar, "Sí");
                        },
                    );
                    campo(
                        ui,
                        "Cerrar CasparCG al salir",
                        "Si Relevo abrió CasparCG (y su scanner), al cerrar Relevo se cierran también. Si CasparCG ya estaba abierto antes, no se toca",
                        |ui| {
                            let v = &mut self.b.casparcg.cerrar_al_salir;
                            for (k, t) in [("si", "Sí"), ("preguntar", "Preguntar"), ("no", "No, dejarlo abierto")] {
                                if ui.radio(v.trim().eq_ignore_ascii_case(k), t).clicked() {
                                    *v = k.to_string();
                                }
                            }
                        },
                    );
                    campo(
                        ui,
                        "Salida siempre visible",
                        "Si la ventana de video de CasparCG se minimiza (por ejemplo con Windows + D, «Mostrar escritorio»), se vuelve a mostrar sola: minimizada, CasparCG la deja en negro. Vale al reiniciar Relevo",
                        |ui| {
                            ui.checkbox(&mut self.b.casparcg.salida_siempre_visible, "Sí");
                        },
                    );
                    campo(
                        ui,
                        "Consolas",
                        "Al abrir CasparCG y su scanner desde Relevo: cada uno con su ventana negra, o sin ventanas (lo que escriben se ve en la pestaña CasparCG). Vale desde la próxima vez que se abran.",
                        |ui| {
                            let ocultas = self.b.casparcg.consolas.trim().eq_ignore_ascii_case("ocultas");
                            let actual = if ocultas { "Ocultas (se ven en la pestaña CasparCG)" } else { "En sus propias ventanas" };
                            egui::ComboBox::from_id_salt("aj-consolas").width(300.0).selected_text(actual).show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.b.casparcg.consolas, "visibles".to_string(), "En sus propias ventanas");
                                ui.selectable_value(&mut self.b.casparcg.consolas, "ocultas".to_string(), "Ocultas (se ven en la pestaña CasparCG)");
                            });
                        },
                    );
                });
            }
            Seccion::Salida => self.seccion_salida(ui),
            Seccion::Iptv => self.seccion_iptv(ui),
            Seccion::Archivos => {
                titulo(ui, "Archivos y ffmpeg", "Cómo se usan los videos de otras carpetas y dónde está ffmpeg.");
                rejilla(ui, "g-arch", |ui| {
                    if !self.remoto {
                        campo(
                            ui,
                            "Videos fuera de media",
                            "Directo = desde donde están; Copiar = a media/IMPORTADOS",
                            |ui| {
                                use nucleo::ModoExternos as M;
                                ui.radio_value(&mut self.b.archivos.externos, M::Directo, "Directo");
                                ui.radio_value(&mut self.b.archivos.externos, M::Copiar, "Copiar");
                            },
                        );
                    }
                    campo(ui, "ffmpeg.exe", "Vacío = buscarlo o descargarlo solo", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.b.archivos.ffmpeg)
                                .desired_width(260.0)
                                .hint_text("automático"),
                        );
                        if ui.button("…").on_hover_text("Elegir").clicked() {
                            if let Some(p) = rfd::FileDialog::new().add_filter("ffmpeg", &["exe"]).pick_file() {
                                self.b.archivos.ffmpeg = p.display().to_string();
                            }
                        }
                    });
                    campo(ui, "Buscar actualizaciones", "Una vez al día, si hay versión nueva de ffmpeg", |ui| {
                        ui.checkbox(&mut self.b.archivos.actualizar_ffmpeg, "Sí");
                    });
                });
            }
            Seccion::Pauta => {
                titulo(ui, "Pauta y corte", "Cómo se arma la tanda automática.");
                rejilla(ui, "g-pauta", |ui| {
                    campo(
                        ui,
                        "Orden de los pases",
                        "Hora = el que vence antes; Cercanía = el más cercano a ahora",
                        |ui| {
                            use nucleo::pauta::Orden as O;
                            ui.radio_value(&mut self.b.pauta.orden, O::Hora, "Hora");
                            ui.radio_value(&mut self.b.pauta.orden, O::Cercania, "Cercanía");
                        },
                    );
                    campo(ui, "Tolerancia (min)", "Minutos que un pase puede salir tarde", |ui| {
                        ui.add(egui::DragValue::new(&mut self.b.pauta.tolerancia_por_defecto).range(0..=600));
                    });
                    campo(ui, "Duración del corte", "m:ss · 0:00 = sin tope", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.b.pauta.duracion_corte).desired_width(70.0));
                    });
                    campo(ui, "Carpetas de relleno", "Separadas por comas", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.relleno).desired_width(260.0));
                    });
                    campo(
                        ui,
                        "Rellenar con otros spots",
                        "El tiempo que sobra del corte se llena primero con spots de otros clientes (el que menos salió hoy) y después con las carpetas de relleno",
                        |ui| {
                            ui.checkbox(&mut self.b.pauta.relleno_clientes, "Sí");
                        },
                    );
                    campo(
                        ui,
                        "Recuperar lo que no salió",
                        "Un pase que no salió en su horario, o que se cortó, entra solo en el próximo corte durante este tiempo después de su horario. 0 = no se recupera",
                        |ui| {
                            let mut horas = self.b.pauta.recuperar_min as f32 / 60.0;
                            let r = ui.add(
                                egui::DragValue::new(&mut horas)
                                    .range(0.0..=24.0)
                                    .speed(0.25)
                                    .fixed_decimals(1)
                                    .suffix(" h"),
                            );
                            if r.changed() {
                                self.b.pauta.recuperar_min = (horas * 60.0).round() as u32;
                            }
                            if self.b.pauta.recuperar_min == 0 {
                                ui.label(RichText::new("no se recupera").size(12.0).color(MUY_TENUE));
                            }
                        },
                    );
                    campo(
                        ui,
                        "Transición inicial",
                        "Entre spots y al entrar/salir de TV. Se cambia también en «Al aire»",
                        |ui| {
                            let actual =
                                nucleo::TipoTransicion::desde_texto(&self.b.transiciones.tipo).unwrap_or_default();
                            egui::ComboBox::from_id_salt("aj-trans")
                                .width(110.0)
                                .selected_text(actual.nombre())
                                .show_ui(ui, |ui| {
                                    for t in nucleo::TipoTransicion::TODOS {
                                        if ui.selectable_label(actual == t, t.nombre()).clicked() {
                                            self.b.transiciones.tipo = t.nombre().to_lowercase();
                                        }
                                    }
                                });
                            ui.add(
                                egui::DragValue::new(&mut self.b.transiciones.segundos)
                                    .range(0.0..=5.0)
                                    .speed(0.05)
                                    .suffix(" s"),
                            );
                        },
                    );
                });
            }
            Seccion::VistaPrevia => {
                titulo(ui, "Vista previa", "El reproductor de la columna Archivos.");
                rejilla(ui, "g-vista", |ui| {
                    campo(ui, "Sonido", "Escuchar la vista previa", |ui| {
                        ui.checkbox(&mut self.b.vista_previa.sonido, "Sí");
                    });
                    campo(ui, "Silenciar al aire", "Sin sonido mientras hay publicidad al aire", |ui| {
                        ui.checkbox(&mut self.b.vista_previa.silenciar_al_aire, "Sí");
                    });
                    campo(ui, "Salida de audio", "Nombre del dispositivo (vacío = el predeterminado)", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.b.vista_previa.salida_audio).desired_width(260.0));
                    });
                });
            }
            Seccion::Biblioteca => {
                titulo(ui, "Biblioteca", "El formato con el que se revisan y convierten los spots.");
                rejilla(ui, "g-bib", |ui| {
                    campo(ui, "Resolución", "Ancho×alto, p. ej. 1920x1080", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.b.biblioteca.resolucion).desired_width(110.0));
                    });
                    campo(ui, "Cuadros por segundo", "29.97, 25…", |ui| {
                        ui.add(egui::DragValue::new(&mut self.b.biblioteca.fps).range(1.0..=120.0).speed(0.01));
                    });
                    campo(ui, "Volumen (LUFS)", "Nivel de sonido al nivelar (−23 es lo usual)", |ui| {
                        ui.add(egui::DragValue::new(&mut self.b.biblioteca.lufs).range(-40.0..=-5.0).speed(0.1));
                    });
                    campo(ui, "Tolerancia (LU)", "Cuánto puede alejarse antes de avisar", |ui| {
                        ui.add(egui::DragValue::new(&mut self.b.biblioteca.tolerancia).range(0.0..=10.0).speed(0.1));
                    });
                });
            }
            Seccion::Red => {
                titulo(ui, "Red y API", "Conexión entre PCs y control desde otros programas.");
                rejilla(ui, "g-red", |ui| {
                    campo(
                        ui,
                        "Esta PC es",
                        "La principal controla CasparCG; una remota se conecta a la principal",
                        |ui| {
                            let mut remoto = self.b.remoto.es_remoto();
                            ui.radio_value(&mut remoto, false, "Principal");
                            ui.radio_value(&mut remoto, true, "Remota");
                            self.b.remoto.modo = if remoto { "remoto".into() } else { "principal".into() };
                        },
                    );
                    if self.b.remoto.es_remoto() {
                        campo(ui, "PC principal", "IP:puerto, p. ej. 192.168.1.10:7800", |ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.b.remoto.servidor).desired_width(200.0));
                        });
                    } else {
                        campo(ui, "Aceptar PCs remotas", "Que otras PCs puedan conectarse a esta", |ui| {
                            ui.checkbox(&mut self.b.remoto.servir, "Sí");
                        });
                        campo(ui, "Puerto", "Normalmente 7800", |ui| {
                            ui.add(egui::DragValue::new(&mut self.b.remoto.puerto).range(1..=65535));
                        });
                        campo(ui, "API (botoneras)", "Control por HTTP desde otros programas", |ui| {
                            ui.checkbox(&mut self.b.api.activa, "Activa");
                        });
                        if self.b.api.activa {
                            campo(ui, "Puerto de la API", "Normalmente 7801", |ui| {
                                ui.add(egui::DragValue::new(&mut self.b.api.puerto).range(1..=65535));
                            });
                            campo(ui, "Clave de la API", "Obligatoria: la misma que pongas en la botonera", |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.b.api.clave)
                                        .desired_width(200.0)
                                        .password(true),
                                );
                            });
                        }
                    }
                });
            }
            Seccion::Respaldo => {
                titulo(ui, "Respaldos", "Copia diaria de pautas, registro, usuarios y configuración.");
                rejilla(ui, "g-resp", |ui| {
                    campo(ui, "Carpeta", "Vacía = datos/respaldos. Mejor en otro disco o un USB", |ui| {
                        carpeta(ui, &mut self.b.respaldo.carpeta);
                    });
                    campo(ui, "Guardar", "Cuántos respaldos se conservan", |ui| {
                        ui.add(egui::DragValue::new(&mut self.b.respaldo.guardar).range(1..=365).suffix(" días"));
                    });
                });
            }
            Seccion::Interfaz => {
                titulo(ui, "Interfaz", "Cómo se ve la app.");
                rejilla(ui, "g-int", |ui| {
                    campo(
                        ui,
                        "Tamaño de la interfaz",
                        "Automático: si la ventana es chica (pantalla de menos de 1080 o escala de Windows al 125 %), todo se achica para que quepa",
                        |ui| {
                            let texto =
                                |v: u32| if v == 0 { "Automático".to_string() } else { format!("{v} %") };
                            egui::ComboBox::from_id_salt("aj-escala")
                                .width(140.0)
                                .selected_text(texto(self.b.interfaz.escala))
                                .show_ui(ui, |ui| {
                                    for v in [0, 70, 75, 80, 85, 90, 100, 110, 120, 130] {
                                        ui.selectable_value(&mut self.b.interfaz.escala, v, texto(v));
                                    }
                                });
                        },
                    );
                    campo(
                        ui,
                        "Formato de hora",
                        "Cómo se muestran las horas (al escribirlas se acepta cualquiera)",
                        |ui| {
                            ui.radio_value(&mut self.b.interfaz.hora_12, false, "24 h (20:30)");
                            ui.radio_value(&mut self.b.interfaz.hora_12, true, "12 h (8:30 pm)");
                        },
                    );
                    campo(ui, "Animaciones", "Transiciones suaves. Apágalas si la PC es lenta", |ui| {
                        ui.checkbox(&mut self.b.interfaz.animaciones, "Sí");
                    });
                    campo(ui, "Botón «Volver a TV»", "Mostrarlo (y que la tecla Esc corte la tanda)", |ui| {
                        ui.checkbox(&mut self.b.interfaz.boton_volver_a_tv, "Mostrar");
                    });
                    campo(
                        ui,
                        "Aviso de fin de tanda",
                        "Marco que destella (sin sonido) cuando falta poco para volver a TV",
                        |ui| {
                            ui.add(
                                egui::DragValue::new(&mut self.b.interfaz.aviso_fin_seg)
                                    .range(0..=60)
                                    .suffix(" s antes"),
                            );
                            if self.b.interfaz.aviso_fin_seg == 0 {
                                ui.label(RichText::new("apagado").color(TENUE));
                            }
                        },
                    );
                });
            }
            Seccion::Atajos => self.seccion_atajos(ui),
        }
    }

    fn seccion_atajos(&mut self, ui: &mut egui::Ui) {
        titulo(ui, "Atajos de teclado", "Pulsa «Cambiar» y luego la tecla (sola o con Ctrl, Mayús o Alt). No funcionan mientras escribes en un cuadro de texto.");
        // Tecla pulsada mientras se espera una.
        let tecla = self.capturando.and_then(|_| ui.input(crate::atajos::capturar));
        let mut capturando = self.capturando;
        let lista = self.b.atajos.lista();
        // Para avisar si dos acciones tienen la misma tecla.
        let usados: Vec<String> = lista.iter().map(|(_, _, v)| v.to_string()).collect();
        rejilla(ui, "g-atajos", |ui| {
            for (clave, nombre, valor) in lista {
                let repetido = !valor.is_empty() && usados.iter().filter(|u| *u == valor).count() > 1;
                ui.label(RichText::new(nombre).color(TEXTO));
                ui.horizontal(|ui| {
                    if capturando == Some(clave) {
                        ui.label(RichText::new("Pulsa una tecla…  (Esc: cancelar)").color(ACENTO));
                        if let Some((mods, k)) = tecla {
                            if k != egui::Key::Escape {
                                *valor = crate::atajos::escribir(mods, k);
                            }
                            capturando = None;
                        }
                    } else {
                        let color = if repetido { AMBAR } else { TEXTO };
                        ui.add_sized(
                            [130.0, 26.0],
                            egui::Label::new(RichText::new(crate::atajos::bonito(valor)).monospace().color(color)),
                        );
                        if ui.button("Cambiar").clicked() {
                            capturando = Some(clave);
                        }
                        if !valor.is_empty() && ui.small_button("✖").on_hover_text("Sin atajo").clicked() {
                            valor.clear();
                        }
                        if repetido {
                            ui.label(RichText::new("⚠ repetido").size(11.5).color(AMBAR));
                        }
                    }
                });
                ui.end_row();
            }
        });
        self.capturando = capturando;
        ui.add_space(6.0);
        if ui.small_button("Volver a los atajos de fábrica").clicked() {
            self.b.atajos = Default::default();
        }
    }

    fn seccion_salida(&mut self, ui: &mut egui::Ui) {
        titulo(
            ui,
            "Salida de video",
            "La ventana de video de CasparCG (\"screen\"). Relevo la deja configurada en casparcg.config al abrir; CasparCG la toma al reiniciarse.",
        );
        let mons = self.monitores.lock().expect("monitores").clone();
        let modo = self.b.casparcg.pantalla.trim().to_lowercase();
        let salida = mons.as_deref().map(|m| arranque::decidir_salida(&modo, m));

        // Dibujo de los monitores.
        match &mons {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Buscando monitores…").color(TENUE));
                });
            }
            Some(m) if m.is_empty() => {
                ui.label(RichText::new("No se pudo leer la lista de monitores.").color(TENUE));
            }
            Some(m) => dibujar_monitores(ui, m, salida.as_ref()),
        }
        ui.add_space(10.0);
        let opciones: Vec<(String, String)> = {
            let mut v = vec![
                ("auto".to_string(), "Automático: pantalla completa en el segundo monitor".to_string()),
                ("ventana".to_string(), "En una ventana".to_string()),
                ("no".to_string(), "No tocar casparcg.config".to_string()),
            ];
            for m in mons.iter().flatten() {
                v.push((m.numero.to_string(), format!("Pantalla completa en el {}", m.describir())));
            }
            v
        };
        rejilla(ui, "g-salida", |ui| {
            campo(ui, "Salida", "Dónde se ve el video de CasparCG", |ui| {
                let actual = opciones
                    .iter()
                    .find(|(k, _)| *k == modo)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| format!("Monitor {modo} (no conectado ahora)"));
                egui::ComboBox::from_id_salt("salida-caspar").width(380.0).selected_text(actual).show_ui(ui, |ui| {
                    for (k, t) in &opciones {
                        ui.selectable_value(&mut self.b.casparcg.pantalla, k.clone(), t);
                    }
                });
            });
        });
        ui.add_space(6.0);
        if let Some(s) = &salida {
            let texto = match s {
                SalidaPantalla::NoTocar if modo == "auto" => {
                    "Hay un solo monitor: se deja casparcg.config como está.".to_string()
                }
                SalidaPantalla::NoTocar if modo != "no" => {
                    "Ese monitor no está conectado ahora: no se cambia nada.".into()
                }
                SalidaPantalla::NoTocar => "Relevo no cambia la salida de CasparCG.".into(),
                s => format!("CasparCG saldrá {}.", arranque::describir_salida(s)),
            };
            ui.label(RichText::new(texto).color(TEXTO));
        }
        ui.horizontal(|ui| {
            if ui.small_button("⟳ Volver a buscar monitores").clicked() {
                self.buscar_monitores();
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Antes del primer cambio se guarda una copia: casparcg.config.antes-de-relevo. \
                 Si CasparCG está abierto, abajo aparece «Reiniciar CasparCG» (solo en señal de TV).",
            )
            .size(11.5)
            .color(MUY_TENUE),
        );
    }
}

impl VentanaAjustes {
    fn seccion_iptv(&mut self, ui: &mut egui::Ui) {
        titulo(
            ui,
            "Señal de TV por IPTV (modo Transmisión)",
            "En modo Transmisión, CasparCG reproduce el canal desde la red, en una capa debajo de la publicidad. \
             Al entrar la tanda, la publicidad lo tapa y su sonido se baja; al terminar, vuelve solo. \
             El modo se cambia arriba, en la pantalla principal.",
        );
        // Interruptor grande.
        let activa = self.b.iptv.activa;
        let (texto, fondo) = if activa {
            ("●  MODO TRANSMISIÓN PERMITIDO", Color32::from_rgb(0x1F, 0x7A, 0x4A))
        } else {
            ("○  MODO TRANSMISIÓN NO PERMITIDO (solo publicidad)", Color32::from_rgb(0x3A, 0x40, 0x4C))
        };
        let b = egui::Button::new(RichText::new(texto).size(13.5).family(crate::tema::negrita()).color(Color32::WHITE))
            .fill(fondo)
            .corner_radius(CornerRadius::same(8))
            .min_size(Vec2::new(0.0, 34.0));
        if ui.add(b).on_hover_text("Clic para activarla o desactivarla").clicked() {
            self.b.iptv.activa = !activa;
        }
        ui.add_space(10.0);

        // Lista de canales (opcional).
        let canales = self.canales.lock().expect("canales").clone();
        if canales.is_some() {
            self.cargando_lista = false;
        }
        rejilla(ui, "g-iptv", |ui| {
            campo(ui, "Lista de canales (.m3u)", "Opcional: para elegir el canal por su nombre", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.b.iptv.lista)
                        .desired_width(230.0)
                        .hint_text("http://…/lista.m3u o un archivo"),
                );
                if ui.button("📂").on_hover_text("Elegir archivo .m3u").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Lista", &["m3u", "m3u8"]).pick_file() {
                        self.b.iptv.lista = p.display().to_string();
                    }
                }
                let puede = !self.b.iptv.lista.trim().is_empty() && !self.cargando_lista;
                if ui.add_enabled(puede, egui::Button::new("Cargar")).clicked() {
                    self.cargar_lista();
                }
            });
            if let Some(Ok(lista)) = &canales {
                campo(ui, "Canal", &format!("{} canales en la lista", lista.len()), |ui| {
                    let actual = lista
                        .iter()
                        .find(|(_, u)| *u == self.b.iptv.direccion)
                        .map(|(n, _)| n.clone())
                        .unwrap_or_else(|| "Elegir canal…".into());
                    egui::ComboBox::from_id_salt("iptv-canal").width(300.0).selected_text(actual).show_ui(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.filtro_canal)
                                .hint_text("🔍 Buscar")
                                .desired_width(280.0),
                        );
                        let f = self.filtro_canal.to_lowercase();
                        egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                            for (n, u) in lista.iter().filter(|(n, _)| f.is_empty() || n.to_lowercase().contains(&f)) {
                                if ui.selectable_label(self.b.iptv.direccion == *u, n).clicked() {
                                    self.b.iptv.direccion = u.clone();
                                }
                            }
                        });
                    });
                });
            }
            campo(ui, "Dirección del canal", "La que funcionó en VLC", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.b.iptv.direccion)
                        .desired_width(300.0)
                        .hint_text("udp://@239.1.1.1:1234  ·  http://…/canal.m3u8"),
                );
            });
            campo(
                ui,
                "Capa en CasparCG",
                &format!("Debajo de la publicidad (capa {})", self.b.casparcg.capa_publicidad),
                |ui| {
                    let max = self.b.casparcg.capa_publicidad.saturating_sub(1).max(1);
                    ui.add(egui::DragValue::new(&mut self.b.iptv.capa).range(1..=max));
                },
            );
        });
        if self.cargando_lista {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Leyendo la lista…").color(TENUE));
            });
        }
        if let Some(Err(e)) = &canales {
            ui.label(RichText::new(format!("⚠ {e}")).color(AIRE));
        }
        let d = self.b.iptv.direccion.trim();
        if !d.is_empty() && !nucleo::iptv::es_direccion(d) {
            ui.label(
                RichText::new("⚠ No parece una dirección de red (debe empezar con http://, udp://, rtp://…).")
                    .color(AMBAR),
            );
        }
        ui.add_space(10.0);
        let nota = [
            "• Se aplica al guardar, sin reiniciar. Sin permitirlo, Relevo queda en modo Publicidad: CasparCG no reproduce el canal.",
            "• El canal llega con retraso (unos 20 s con HLS). Es parejo para todo, pero no mezcles: o el canal sale por IPTV o por la mezcladora.",
            "• Para la detección de corte, elige la fuente «📺 IPTV»: así mira exactamente el mismo canal.",
            "• Si el canal deja de llegar, Relevo lo vuelve a pedir solo y avisa en pantalla (sin sonido).",
        ];
        for n in nota {
            ui.label(RichText::new(n).size(11.5).color(MUY_TENUE));
        }
    }

    fn cargar_lista(&mut self) {
        let lista = self.b.iptv.lista.trim().to_string();
        let destino = self.canales.clone();
        *destino.lock().expect("canales") = None;
        self.cargando_lista = true;
        std::thread::spawn(move || {
            let texto: Result<String, String> = if lista.starts_with("http://") || lista.starts_with("https://") {
                ureq::get(&lista).call().map_err(|e| format!("No se pudo bajar la lista: {e}")).and_then(|mut r| {
                    r.body_mut()
                        .with_config()
                        .limit(20 * 1024 * 1024)
                        .read_to_string()
                        .map_err(|e| format!("No se pudo leer la lista: {e}"))
                })
            } else {
                std::fs::read(&lista)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .map_err(|e| format!("No se pudo abrir la lista: {e}"))
            };
            let r = texto.and_then(|t| {
                let v = nucleo::iptv::leer_m3u(&t);
                if v.is_empty() {
                    Err("La lista no tiene canales".into())
                } else {
                    Ok(v)
                }
            });
            *destino.lock().expect("canales") = Some(r);
        });
    }
}

fn dibujar_monitores(ui: &mut egui::Ui, mons: &[Monitor], salida: Option<&SalidaPantalla>) {
    let (izq, arr) = (mons.iter().map(|m| m.x).min().unwrap_or(0), mons.iter().map(|m| m.y).min().unwrap_or(0));
    let der = mons.iter().map(|m| m.x + m.ancho).max().unwrap_or(1);
    let aba = mons.iter().map(|m| m.y + m.alto).max().unwrap_or(1);
    let (ancho_total, alto_total) = ((der - izq).max(1) as f32, (aba - arr).max(1) as f32);
    let caja = Vec2::new(ui.available_width().min(520.0), 130.0);
    let (rect, _) = ui.allocate_exact_size(caja, Sense::hover());
    let escala = ((caja.x - 10.0) / ancho_total).min((caja.y - 10.0) / alto_total);
    let origen = rect.center() - Vec2::new(ancho_total, alto_total) * escala / 2.0;
    let p = ui.painter();
    for m in mons {
        let r = egui::Rect::from_min_size(
            origen + Vec2::new((m.x - izq) as f32, (m.y - arr) as f32) * escala,
            Vec2::new(m.ancho as f32, m.alto as f32) * escala,
        )
        .shrink(3.0);
        let es_salida = matches!(salida, Some(SalidaPantalla::Completa(s)) if s.numero == m.numero);
        let (fondo, borde) = if es_salida { (AIRE_OSCURO, AIRE) } else { (TARJETA, BORDE) };
        p.rect(
            r,
            CornerRadius::same(6),
            fondo,
            Stroke::new(if es_salida { 2.0 } else { 1.0 }, borde),
            egui::StrokeKind::Inside,
        );
        p.text(
            r.center() - Vec2::new(0.0, 10.0),
            Align2::CENTER_CENTER,
            m.numero.to_string(),
            FontId::proportional(26.0),
            TEXTO,
        );
        let pie = if es_salida {
            "CasparCG"
        } else if m.principal {
            "Relevo (principal)"
        } else {
            ""
        };
        p.text(r.center() + Vec2::new(0.0, 16.0), Align2::CENTER_CENTER, pie, FontId::proportional(11.5), TENUE);
        p.text(
            r.left_bottom() + Vec2::new(6.0, -4.0),
            Align2::LEFT_BOTTOM,
            format!("{}×{}", m.ancho, m.alto),
            FontId::proportional(10.0),
            MUY_TENUE,
        );
    }
}

fn titulo(ui: &mut egui::Ui, t: &str, sub: &str) {
    ui.label(RichText::new(t).size(17.0).strong().family(crate::tema::negrita()).color(TEXTO));
    ui.label(RichText::new(sub).size(12.0).color(TENUE));
    ui.add_space(10.0);
}

fn rejilla(ui: &mut egui::Ui, id: &str, f: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id).num_columns(2).spacing([18.0, 12.0]).min_col_width(170.0).show(ui, f);
}

fn campo(ui: &mut egui::Ui, etiqueta: &str, ayuda: &str, f: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new(etiqueta).color(TEXTO));
        ui.label(RichText::new(ayuda).size(11.0).color(MUY_TENUE));
    });
    ui.horizontal(f);
    ui.end_row();
}

fn carpeta(ui: &mut egui::Ui, valor: &mut String) {
    ui.add(egui::TextEdit::singleline(valor).desired_width(260.0).hint_text("automática"));
    if ui.button("📂").on_hover_text("Elegir carpeta").clicked() {
        if let Some(p) = rfd::FileDialog::new().pick_folder() {
            *valor = p.display().to_string();
        }
    }
}

/// Cambios que se aplican sin reiniciar Relevo.
fn se_aplica_ya(clave: &str) -> bool {
    matches!(
        clave,
        "interfaz.animaciones"
            | "interfaz.boton_volver_a_tv"
            | "casparcg.pantalla"
            | "casparcg.consolas"
            | "casparcg.detener_al_cerrar"
            | "casparcg.cerrar_al_salir"
            | "archivos.actualizar_ffmpeg"
            | "transiciones.tipo"
            | "transiciones.segundos"
            | "interfaz.aviso_fin_seg"
            | "interfaz.escala"
            | "interfaz.hora_12"
    ) || clave.starts_with("atajos.")
        || clave.starts_with("deteccion.")
        || clave.starts_with("iptv.")
        || clave.starts_with("preview.")
        || clave.starts_with("emision.")
        || clave.starts_with("actualizacion.")
}

/// Valor de config.toml.
enum Valor {
    Texto(String),
    Entero(i64),
    Real(f64),
    Si(bool),
    Lista(Vec<String>),
}

impl Valor {
    fn a_toml(&self) -> toml_edit::Value {
        match self {
            Valor::Texto(s) => s.as_str().into(),
            Valor::Entero(n) => (*n).into(),
            Valor::Real(x) => (*x).into(),
            Valor::Si(b) => (*b).into(),
            Valor::Lista(v) => toml_edit::Value::Array(v.iter().map(|s| s.as_str()).collect()),
        }
    }
}

/// Cada ajuste de la ventana: (tabla, clave, valor).
fn valores(c: &Config) -> Vec<(&'static str, &'static str, Valor)> {
    use Valor::*;
    let ext = match c.archivos.externos {
        nucleo::ModoExternos::Directo => "directo",
        nucleo::ModoExternos::Copiar => "copiar",
    };
    let orden = match c.pauta.orden {
        nucleo::pauta::Orden::Hora => "hora",
        nucleo::pauta::Orden::Cercania => "cercania",
    };
    vec![
        ("casparcg", "host", Texto(c.casparcg.host.clone())),
        ("casparcg", "puerto", Entero(c.casparcg.puerto as i64)),
        ("casparcg", "canal", Entero(c.casparcg.canal as i64)),
        ("casparcg", "capa_publicidad", Entero(c.casparcg.capa_publicidad as i64)),
        ("casparcg", "carpeta_medios", Texto(c.casparcg.carpeta_medios.clone())),
        ("casparcg", "osc_puerto", Entero(c.casparcg.osc_puerto as i64)),
        ("casparcg", "carpeta_programa", Texto(c.casparcg.carpeta_programa.clone())),
        ("casparcg", "iniciar_automaticamente", Si(c.casparcg.iniciar_automaticamente)),
        ("casparcg", "salida_siempre_visible", Si(c.casparcg.salida_siempre_visible)),
        ("casparcg", "detener_al_cerrar", Si(c.casparcg.detener_al_cerrar)),
        ("casparcg", "cerrar_al_salir", Texto(c.casparcg.cerrar_al_salir.clone())),
        ("casparcg", "consolas", Texto(c.casparcg.consolas.clone())),
        ("casparcg", "pantalla", Texto(c.casparcg.pantalla.clone())),
        ("archivos", "externos", Texto(ext.into())),
        ("archivos", "ffmpeg", Texto(c.archivos.ffmpeg.clone())),
        ("archivos", "actualizar_ffmpeg", Si(c.archivos.actualizar_ffmpeg)),
        ("pauta", "orden", Texto(orden.into())),
        ("pauta", "tolerancia_por_defecto", Entero(c.pauta.tolerancia_por_defecto as i64)),
        ("pauta", "duracion_corte", Texto(c.pauta.duracion_corte.clone())),
        ("pauta", "relleno", Lista(c.pauta.relleno.clone())),
        ("pauta", "relleno_clientes", Si(c.pauta.relleno_clientes)),
        ("pauta", "recuperar_min", Entero(c.pauta.recuperar_min as i64)),
        ("vista_previa", "sonido", Si(c.vista_previa.sonido)),
        ("vista_previa", "salida_audio", Texto(c.vista_previa.salida_audio.clone())),
        ("vista_previa", "silenciar_al_aire", Si(c.vista_previa.silenciar_al_aire)),
        ("biblioteca", "resolucion", Texto(c.biblioteca.resolucion.clone())),
        ("biblioteca", "fps", Real(c.biblioteca.fps)),
        ("biblioteca", "lufs", Real(c.biblioteca.lufs as f64)),
        ("biblioteca", "tolerancia", Real(c.biblioteca.tolerancia as f64)),
        ("remoto", "modo", Texto(c.remoto.modo.clone())),
        ("remoto", "servir", Si(c.remoto.servir)),
        ("remoto", "puerto", Entero(c.remoto.puerto as i64)),
        ("remoto", "servidor", Texto(c.remoto.servidor.clone())),
        ("api", "activa", Si(c.api.activa)),
        ("api", "puerto", Entero(c.api.puerto as i64)),
        ("api", "clave", Texto(c.api.clave.clone())),
        ("respaldo", "carpeta", Texto(c.respaldo.carpeta.clone())),
        ("respaldo", "guardar", Entero(c.respaldo.guardar as i64)),
        ("interfaz", "animaciones", Si(c.interfaz.animaciones)),
        ("interfaz", "boton_volver_a_tv", Si(c.interfaz.boton_volver_a_tv)),
        ("transiciones", "tipo", Texto(c.transiciones.tipo.clone())),
        ("transiciones", "segundos", Real(c.transiciones.segundos as f64)),
        ("interfaz", "aviso_fin_seg", Entero(c.interfaz.aviso_fin_seg as i64)),
        ("interfaz", "escala", Entero(c.interfaz.escala as i64)),
        ("interfaz", "hora_12", Si(c.interfaz.hora_12)),
        ("atajos", "play_pausa", Texto(c.atajos.play_pausa.clone())),
        ("atajos", "siguiente", Texto(c.atajos.siguiente.clone())),
        ("atajos", "anterior", Texto(c.atajos.anterior.clone())),
        ("atajos", "detener", Texto(c.atajos.detener.clone())),
        ("atajos", "anadir", Texto(c.atajos.anadir.clone())),
        ("atajos", "quitar", Texto(c.atajos.quitar.clone())),
        ("atajos", "subir", Texto(c.atajos.subir.clone())),
        ("atajos", "bajar", Texto(c.atajos.bajar.clone())),
        ("atajos", "emision", Texto(c.atajos.emision.clone())),
        ("atajos", "biblioteca", Texto(c.atajos.biblioteca.clone())),
        ("atajos", "reportes", Texto(c.atajos.reportes.clone())),
        ("atajos", "compacta", Texto(c.atajos.compacta.clone())),
        ("deteccion", "activa", Si(c.deteccion.activa)),
        ("deteccion", "fuente", Texto(c.deteccion.fuente.clone())),
        ("deteccion", "origen", Texto(c.deteccion.origen.clone())),
        ("deteccion", "monitor", Entero(c.deteccion.monitor as i64)),
        ("deteccion", "x", Real(c.deteccion.x)),
        ("deteccion", "y", Real(c.deteccion.y)),
        ("deteccion", "ancho", Real(c.deteccion.ancho)),
        ("deteccion", "alto", Real(c.deteccion.alto)),
        ("deteccion", "umbral", Real(c.deteccion.umbral)),
        ("deteccion", "segundos", Real(c.deteccion.segundos)),
        ("deteccion", "accion", Texto(c.deteccion.accion.clone())),
        ("deteccion", "espera_min", Real(c.deteccion.espera_min)),
        ("deteccion", "al_volver", Texto(c.deteccion.al_volver.clone())),
        ("iptv", "activa", Si(c.iptv.activa)),
        ("iptv", "direccion", Texto(c.iptv.direccion.clone())),
        ("iptv", "lista", Texto(c.iptv.lista.clone())),
        ("iptv", "capa", Entero(c.iptv.capa as i64)),
        ("preview", "fuente", Texto(c.preview.fuente.clone())),
        ("preview", "origen", Texto(c.preview.origen.clone())),
        ("preview", "monitor", Entero(c.preview.monitor as i64)),
        ("emision", "modo", Texto(c.emision.modo.clone())),
        ("actualizacion", "activa", Si(c.actualizacion.activa)),
        ("actualizacion", "repo", Texto(c.actualizacion.repo.clone())),
        ("actualizacion", "token", Texto(c.actualizacion.token.clone())),
    ]
}

fn igual(a: &Valor, b: &Valor) -> bool {
    match (a, b) {
        (Valor::Texto(x), Valor::Texto(y)) => x == y,
        (Valor::Entero(x), Valor::Entero(y)) => x == y,
        (Valor::Real(x), Valor::Real(y)) => (x - y).abs() < 1e-9,
        (Valor::Si(x), Valor::Si(y)) => x == y,
        (Valor::Lista(x), Valor::Lista(y)) => x == y,
        _ => false,
    }
}

/// Claves ("tabla.clave") que cambiaron.
fn cambios(viejo: &Config, nuevo: &Config) -> Vec<String> {
    valores(viejo)
        .into_iter()
        .zip(valores(nuevo))
        .filter(|((_, _, a), (_, _, b))| !igual(a, b))
        .map(|((t, k, _), _)| format!("{t}.{k}"))
        .collect()
}

/// Escribe en `texto` (un config.toml) solo lo que cambió, conservando comentarios.
pub fn aplicar_cambios(texto: &str, viejo: &Config, nuevo: &Config) -> Result<(String, Vec<String>), String> {
    let mut doc: toml_edit::DocumentMut = texto.parse().map_err(|e| format!("config.toml no es válido: {e}"))?;
    let mut hechos = Vec::new();
    for ((tabla, clave, a), (_, _, b)) in valores(viejo).into_iter().zip(valores(nuevo)) {
        if igual(&a, &b) {
            continue;
        }
        if !doc.contains_key(tabla) {
            doc[tabla] = toml_edit::table();
        }
        let t = doc[tabla].as_table_mut().ok_or_else(|| format!("[{tabla}] no es una tabla"))?;
        let mut v = b.a_toml();
        if let Some(viejo) = t.get(clave).and_then(|i| i.as_value()) {
            // Se conservan los comentarios de la misma línea.
            *v.decor_mut() = viejo.decor().clone();
        }
        t[clave] = toml_edit::Item::Value(v);
        hechos.push(format!("{tabla}.{clave}"));
    }
    Ok((doc.to_string(), hechos))
}

fn guardar_en(ruta: &Path, viejo: &Config, nuevo: &Config) -> Result<Vec<String>, String> {
    let texto = std::fs::read_to_string(ruta).unwrap_or_default();
    let (nuevo_texto, hechos) = aplicar_cambios(&texto, viejo, nuevo)?;
    // Se comprueba que se pueda leer antes de escribirlo.
    toml::from_str::<Config>(&nuevo_texto).map_err(|e| format!("el resultado no es válido: {e}"))?;
    let tmp = ruta.with_extension("toml.nuevo");
    std::fs::write(&tmp, &nuevo_texto).map_err(|e| format!("no se pudo guardar: {e}"))?;
    std::fs::rename(&tmp, ruta).map_err(|e| format!("no se pudo guardar: {e}"))?;
    tracing::info!("ajustes guardados: {}", hechos.join(", "));
    Ok(hechos)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn guarda_solo_lo_cambiado_sin_perder_comentarios() {
        let texto = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../config.ejemplo.toml")).unwrap();
        let viejo: Config = toml::from_str(&texto).unwrap();
        let mut nuevo = viejo.clone();
        nuevo.casparcg.pantalla = "2".into();
        nuevo.casparcg.canal = 2;
        nuevo.interfaz.animaciones = false;
        nuevo.pauta.relleno = vec!["IDS".into()];
        let (salida, hechos) = aplicar_cambios(&texto, &viejo, &nuevo).unwrap();
        assert_eq!(hechos.len(), 4, "{hechos:?}");
        let leido: Config = toml::from_str(&salida).unwrap();
        assert_eq!(leido.casparcg.pantalla, "2");
        assert_eq!(leido.casparcg.canal, 2);
        assert!(!leido.interfaz.animaciones);
        assert_eq!(leido.pauta.relleno, vec!["IDS".to_string()]);
        // Los comentarios siguen ahí y lo demás no se tocó.
        assert_eq!(texto.matches('#').count(), salida.matches('#').count());
        assert_eq!(texto.lines().count(), salida.lines().count());
        assert!(cambios(&viejo, &viejo).is_empty());
    }

    #[test]
    fn agrega_tablas_que_no_existen() {
        let viejo: Config = toml::from_str("").unwrap();
        let mut nuevo = viejo.clone();
        nuevo.interfaz.animaciones = false;
        let (salida, _) = aplicar_cambios("[casparcg]\nhost = \"x\"\n", &viejo, &nuevo).unwrap();
        assert!(salida.contains("[interfaz]") && salida.contains("animaciones = false"), "{salida}");
    }
}
