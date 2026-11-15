//! Arranque de la ventana según el modo:
//! - **principal**: arranca el núcleo (CasparCG, pauta…) y el servicio de red.
//! - **remoto**: muestra la pantalla de conexión y, al entrar, la misma interfaz.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use egui::{Color32, RichText, Vec2};
use nucleo::remoto::{self, Conexion};

use crate::arranque;
use crate::config::Config;
use crate::tandas::Tandas;
use crate::tema::*;
use crate::ventana::{Extras, Ventana};

pub struct Aplicacion {
    pantalla: Pantalla,
}

enum Pantalla {
    Conectar(Box<Conectar>),
    Principal(Box<Ventana>),
    /// Estado transitorio mientras se cambia de pantalla.
    Vacia,
}

/// Pantalla de conexión de una PC remota.
struct Conectar {
    cfg: Config,
    ruta_config: PathBuf,
    base: PathBuf,
    zona: time::UtcOffset,
    rt: tokio::runtime::Runtime,
    ctx: egui::Context,
    servidor: String,
    usuario: String,
    clave: String,
    intento: Option<(nucleo::Controlador, Arc<Mutex<Conexion>>)>,
    error: Option<String>,
}

impl Aplicacion {
    pub fn new(
        cfg: Config,
        ruta_config: PathBuf,
        base: PathBuf,
        zona: time::UtcOffset,
        rt: tokio::runtime::Runtime,
        ctx: egui::Context,
    ) -> Self {
        if cfg.remoto.es_remoto() {
            let c = Conectar {
                servidor: cfg.remoto.servidor.clone(),
                usuario: cfg.remoto.usuario.clone(),
                clave: String::new(),
                cfg,
                ruta_config,
                base,
                zona,
                rt,
                ctx,
                intento: None,
                error: None,
            };
            return Aplicacion { pantalla: Pantalla::Conectar(Box::new(c)) };
        }
        Aplicacion { pantalla: Pantalla::Principal(Box::new(principal(cfg, ruta_config, base, zona, rt, ctx))) }
    }
}

/// Arranca todo en la PC principal.
fn principal(
    cfg: Config,
    ruta_config: PathBuf,
    base: PathBuf,
    zona: time::UtcOffset,
    rt: tokio::runtime::Runtime,
    ctx: egui::Context,
) -> Ventana {
    let carpeta_datos = base.join("datos");

    // Abrir CasparCG / scanner y configurar el OSC, en segundo plano.
    let estado_arranque = arranque::Compartido::default();
    let ctx_a = ctx.clone();
    arranque::arrancar(
        arranque::Ajustes {
            host: cfg.casparcg.host.clone(),
            puerto: cfg.casparcg.puerto,
            osc_puerto: cfg.casparcg.osc_puerto,
            carpeta_programa: cfg.casparcg.carpeta_programa.clone(),
            iniciar: cfg.casparcg.iniciar_automaticamente,
            canal: cfg.casparcg.canal,
            pantalla: cfg.casparcg.pantalla.clone(),
            como: cfg.casparcg.como_abrir(&carpeta_datos),
        },
        estado_arranque.clone(),
        move || ctx_a.request_repaint(),
    );
    // Que la salida de video de CasparCG no quede minimizada (en negro).
    if cfg.casparcg.salida_siempre_visible && arranque::es_local(&cfg.casparcg.host) {
        arranque::vigilar_salida();
    }

    let extra: Vec<PathBuf> =
        [cfg.casparcg.carpeta_programa.trim()].iter().filter(|c| !c.is_empty()).map(PathBuf::from).collect();
    let ffmpeg = crate::ffmpeg::buscar(&cfg.archivos.ffmpeg, &extra);
    let caspar = amcp::Cliente::new(&cfg.casparcg.host, cfg.casparcg.puerto);
    let carpeta = cfg.casparcg.carpeta_medios.trim();
    let ctx_c = ctx.clone();
    let ctl = nucleo::iniciar(
        rt.handle(),
        caspar,
        nucleo::controlador::Opciones {
            canal: cfg.casparcg.canal,
            capa: cfg.casparcg.capa_publicidad,
            zona,
            osc_puerto: cfg.casparcg.osc_puerto,
            carpeta_medios: (!carpeta.is_empty()).then(|| PathBuf::from(carpeta)),
            carpeta_datos: carpeta_datos.clone(),
            orden: cfg.pauta.orden,
            externos: cfg.archivos.externos,
            ffmpeg: ffmpeg.clone(),
            estandar: cfg.biblioteca.estandar(),
            tope_corte: cfg.pauta.tope(),
            relleno: cfg.pauta.relleno.clone(),
            relleno_clientes: cfg.pauta.relleno_clientes,
            recuperar_min: cfg.pauta.recuperar_min,
            carpeta_respaldos: if cfg.respaldo.carpeta.trim().is_empty() {
                base.join("respaldos")
            } else {
                PathBuf::from(cfg.respaldo.carpeta.trim())
            },
            respaldos_a_guardar: cfg.respaldo.guardar.max(1),
            respaldar_tambien: vec![ruta_config.clone()],
            transicion: cfg.transiciones.transicion(),
            iptv: cfg.iptv_para_nucleo(),
        },
        Arc::new(move || {
            crate::ventana::CAMBIO.store(true, std::sync::atomic::Ordering::Relaxed);
            ctx_c.request_repaint();
        }),
    );

    // Usuarios y servicio de red para las PCs remotas.
    let mut problema_red = None;
    let usuarios = match nucleo::usuarios::Usuarios::cargar(&carpeta_datos) {
        Ok(u) => Some(Arc::new(u)),
        Err(e) => {
            problema_red = Some(format!("No se pudieron leer los usuarios: {e}"));
            None
        }
    };
    let mut puerto_red = None;
    if cfg.remoto.servir {
        if let Some(u) = &usuarios {
            let r = rt.block_on(nucleo::remoto::servir(cfg.remoto.puerto, ctl.clone(), u.clone()));
            match r {
                Ok(()) => puerto_red = Some(cfg.remoto.puerto),
                Err(e) => {
                    problema_red =
                        Some(format!("No se pudo abrir el puerto {} para el modo remoto: {e}", cfg.remoto.puerto))
                }
            }
        }
    }

    // API para botoneras.
    let mut puerto_api = None;
    if cfg.api.activa {
        match rt.block_on(nucleo::api::servir(cfg.api.puerto, cfg.api.clave.clone(), ctl.clone())) {
            Ok(()) => puerto_api = Some(cfg.api.puerto),
            Err(e) => {
                let m = format!("La API no se abrió (puerto {}): {e}", cfg.api.puerto);
                problema_red = Some(match problema_red {
                    Some(p) => format!("{p}\n{m}"),
                    None => m,
                });
            }
        }
    }

    let tandas = Tandas::new(base.join("tandas"));
    let extras = Extras {
        puerto_api,
        arranque: Some(estado_arranque),
        usuarios,
        conexion: None,
        puerto_red,
        problema_red,
        ffmpeg,
        carpeta_local: carpeta_datos.clone(),
        ruta_config: ruta_config.clone(),
    };
    Ventana::new(ctl, cfg, zona, rt, tandas, extras, ctx)
}

impl eframe::App for Aplicacion {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let siguiente = match &mut self.pantalla {
            Pantalla::Principal(v) => {
                v.ui(ui, frame);
                None
            }
            Pantalla::Conectar(c) => c.ui(ui),
            Pantalla::Vacia => None,
        };
        if siguiente.is_some() {
            // Pasa a la interfaz principal con la conexión ya hecha.
            if let Pantalla::Conectar(c) = std::mem::replace(&mut self.pantalla, Pantalla::Vacia) {
                let c = *c;
                let (ctl, con) = c.intento.expect("intento conectado");
                guardar_conexion(&c.ruta_config, &c.servidor, &c.usuario);
                let ffmpeg = crate::ffmpeg::buscar(&c.cfg.archivos.ffmpeg, &[]);
                let extras = Extras {
                    arranque: None,
                    usuarios: None,
                    conexion: Some(con),
                    puerto_red: None,
                    puerto_api: None,
                    problema_red: None,
                    ffmpeg,
                    carpeta_local: c.base.join("datos"),
                    ruta_config: c.ruta_config.clone(),
                };
                let tandas = Tandas::new(c.base.join("tandas"));
                let v = Ventana::new(ctl, c.cfg, c.zona, c.rt, tandas, extras, c.ctx);
                self.pantalla = Pantalla::Principal(Box::new(v));
            }
        }
    }
}

impl Conectar {
    /// Devuelve `Some(())` cuando ya se conectó.
    fn ui(&mut self, ui: &mut egui::Ui) -> Option<()> {
        // ¿Cómo va el intento?
        if let Some((_, con)) = &self.intento {
            let estado = con.lock().expect("conexion").clone();
            match estado {
                Conexion::Conectada { .. } => return Some(()),
                Conexion::Rechazada(m) => {
                    self.error = Some(m);
                    self.intento = None;
                }
                Conexion::Caida(m) => {
                    // Al conectar por primera vez, un fallo se muestra (no se reintenta en silencio).
                    self.error = Some(m);
                    self.intento = None;
                }
                Conexion::Conectando => {}
            }
        }
        let conectando = self.intento.is_some();

        egui::CentralPanel::default().frame(egui::Frame::new().fill(FONDO)).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.18);
                ui.label(RichText::new("Relevo").size(40.0).strong().family(crate::tema::negrita()));
                ui.label(RichText::new("CONECTAR A LA PC PRINCIPAL").size(13.0).color(TENUE).extra_letter_spacing(1.5));
                ui.add_space(20.0);
                marco_tarjeta().inner_margin(egui::Margin::same(20)).show(ui, |ui| {
                    ui.set_width(380.0);
                    egui::Grid::new("conectar").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                        ui.label("PC principal (IP)");
                        ui.add_enabled(
                            !conectando,
                            egui::TextEdit::singleline(&mut self.servidor)
                                .hint_text("192.168.1.50")
                                .desired_width(220.0),
                        );
                        ui.end_row();
                        ui.label("Usuario");
                        ui.add_enabled(!conectando, egui::TextEdit::singleline(&mut self.usuario).desired_width(220.0));
                        ui.end_row();
                        ui.label("Contraseña");
                        let r = ui.add_enabled(
                            !conectando,
                            egui::TextEdit::singleline(&mut self.clave).password(true).desired_width(220.0),
                        );
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            self.intentar();
                        }
                        ui.end_row();
                    });
                    ui.add_space(10.0);
                    if let Some(e) = &self.error {
                        ui.label(RichText::new(format!("⚠ {e}")).color(AIRE));
                        ui.add_space(6.0);
                    }
                    ui.horizontal(|ui| {
                        if conectando {
                            ui.add(egui::Spinner::new());
                            ui.label("Conectando…");
                            if ui.button("Cancelar").clicked() {
                                self.intento = None;
                            }
                        } else {
                            let b = egui::Button::new(
                                RichText::new("Conectar").strong().family(crate::tema::negrita()).color(Color32::WHITE),
                            )
                            .fill(ACENTO_OSCURO)
                            .min_size(Vec2::new(140.0, 34.0));
                            if ui.add(b).clicked() {
                                self.intentar();
                            }
                        }
                    });
                });
                ui.add_space(12.0);
                ui.label(
                    RichText::new(format!(
                        "Puerto {} · Para usar esta PC como principal, pon modo = \"principal\" en config.toml",
                        self.cfg.remoto.puerto
                    ))
                    .size(12.0)
                    .color(MUY_TENUE),
                );
            });
        });
        if conectando {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        None
    }

    fn intentar(&mut self) {
        if self.servidor.trim().is_empty() || self.usuario.trim().is_empty() {
            self.error = Some("Escribe la IP de la PC principal y tu usuario.".into());
            return;
        }
        self.error = None;
        let ctx = self.ctx.clone();
        let r = remoto::conectar(
            self.rt.handle(),
            remoto::Ajustes {
                servidor: self.servidor.trim().to_string(),
                puerto: self.cfg.remoto.puerto,
                usuario: self.usuario.trim().to_string(),
                clave: self.clave.clone(),
            },
            Arc::new(move || {
                crate::ventana::CAMBIO.store(true, std::sync::atomic::Ordering::Relaxed);
                ctx.request_repaint();
            }),
        );
        self.intento = Some(r);
    }
}

/// Recuerda la IP y el usuario en config.toml (sin la contraseña).
fn guardar_conexion(ruta: &std::path::Path, servidor: &str, usuario: &str) {
    let Ok(texto) = std::fs::read_to_string(ruta) else { return };
    let mut salida = String::new();
    let mut en_remoto = false;
    for linea in texto.lines() {
        let l = linea.trim_start();
        if l.starts_with('[') {
            en_remoto = l.starts_with("[remoto]");
        }
        if en_remoto && l.starts_with("servidor") && l.contains('=') {
            salida.push_str(&format!("servidor = '{}'\n", servidor.trim()));
        } else if en_remoto && l.starts_with("usuario") && l.contains('=') {
            salida.push_str(&format!("usuario = '{}'\n", usuario.trim()));
        } else {
            salida.push_str(linea);
            salida.push('\n');
        }
    }
    let _ = std::fs::write(ruta, salida);
}
