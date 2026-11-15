//! Pantalla **CasparCG**: si CasparCG y su media scanner están abiertos (y cuántas
//! copias), botones para abrirlos, reiniciarlos o cerrarlos, y sus consolas dentro
//! de Relevo (lo que escriben, leído de sus archivos de registro).

use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::arranque::Procesos;

/// Cada cuánto se mira qué programas están abiertos (solo con la pestaña a la vista).
const CADA_SONDEO: Duration = Duration::from_secs(3);
/// Cada cuánto se lee lo nuevo de la consola.
const CADA_LECTURA: Duration = Duration::from_millis(700);
/// Al abrir un registro grande, solo se lee su final.
const INICIO_MAXIMO: u64 = 256 * 1024;
/// Líneas que se guardan como mucho.
const MAX_LINEAS: usize = 5000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Consola {
    Caspar,
    Scanner,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum Nivel {
    Detalle,
    Info,
    Aviso,
    Error,
}

pub(super) struct Linea {
    texto: String,
    nivel: Nivel,
    consulta: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Accion {
    AbrirCaspar,
    ReiniciarCaspar,
    CerrarCaspar,
    AbrirScanner,
    ReiniciarScanner,
}

impl Accion {
    fn pide_confirmar(self) -> bool {
        matches!(self, Accion::ReiniciarCaspar | Accion::CerrarCaspar)
    }
    /// Corta la salida de CasparCG: solo en señal de TV.
    fn corta_salida(self) -> bool {
        matches!(self, Accion::ReiniciarCaspar | Accion::CerrarCaspar)
    }
}

pub(super) struct EstadoCaspar {
    procesos: Arc<Mutex<Option<Procesos>>>,
    sondeando: Arc<AtomicBool>,
    ultimo_sondeo: Option<Instant>,
    /// Lo que devolvió la última acción (abrir, cerrar…), para avisar.
    resultado: Arc<Mutex<Option<Result<String, String>>>>,
    trabajando: Arc<AtomicBool>,
    confirmar: Option<Accion>,
    consola: Consola,
    archivo: Option<PathBuf>,
    leido: u64,
    ultima_lectura: Option<Instant>,
    lineas: Vec<Linea>,
    resto: String,
    buscar: String,
    nivel: Nivel,
    ocultar_consultas: bool,
    seguir: bool,
}

impl EstadoCaspar {
    pub(super) fn new() -> Self {
        EstadoCaspar {
            procesos: Arc::new(Mutex::new(None)),
            sondeando: Arc::new(AtomicBool::new(false)),
            ultimo_sondeo: None,
            resultado: Arc::new(Mutex::new(None)),
            trabajando: Arc::new(AtomicBool::new(false)),
            confirmar: None,
            consola: Consola::Caspar,
            archivo: None,
            leido: 0,
            ultima_lectura: None,
            lineas: Vec::new(),
            resto: String::new(),
            buscar: String::new(),
            nivel: Nivel::Detalle,
            ocultar_consultas: true,
            seguir: true,
        }
    }

    /// Mira en segundo plano qué está abierto (tasklist tarda un poco).
    fn sondear(&mut self, carpeta: Option<PathBuf>, ya: bool) {
        let toca = ya || self.ultimo_sondeo.is_none_or(|t| t.elapsed() >= CADA_SONDEO);
        if !toca || self.sondeando.swap(true, Ordering::SeqCst) {
            return;
        }
        self.ultimo_sondeo = Some(Instant::now());
        let destino = self.procesos.clone();
        let bandera = self.sondeando.clone();
        std::thread::spawn(move || {
            let p = arranque::procesos_abiertos(carpeta.as_deref());
            *destino.lock().expect("procesos") = Some(p);
            bandera.store(false, Ordering::SeqCst);
        });
    }

    fn cambiar_consola(&mut self, c: Consola) {
        if self.consola != c {
            self.consola = c;
            self.reiniciar_lectura(None);
        }
    }

    fn reiniciar_lectura(&mut self, archivo: Option<PathBuf>) {
        self.archivo = archivo;
        self.leido = 0;
        self.lineas.clear();
        self.resto.clear();
    }

    /// Lee lo nuevo del archivo de la consola elegida (y cambia al más reciente).
    fn leer(&mut self, archivo: Option<PathBuf>) {
        if self.ultima_lectura.is_some_and(|t| t.elapsed() < CADA_LECTURA) {
            return;
        }
        self.ultima_lectura = Some(Instant::now());
        if archivo != self.archivo {
            self.reiniciar_lectura(archivo);
        }
        let Some(ruta) = self.archivo.clone() else {
            return;
        };
        let Ok(mut f) = std::fs::File::open(&ruta) else {
            return;
        };
        let largo = f.metadata().map(|m| m.len()).unwrap_or(0);
        if largo < self.leido {
            // El archivo se volvió a empezar (CasparCG o el scanner se reabrieron).
            self.reiniciar_lectura(Some(ruta));
        }
        if largo == self.leido {
            return;
        }
        let mut desde = self.leido;
        let mut cortado = false;
        if desde == 0 && largo > INICIO_MAXIMO {
            desde = largo - INICIO_MAXIMO;
            cortado = true;
        }
        if f.seek(SeekFrom::Start(desde)).is_err() {
            return;
        }
        let mut bytes = Vec::new();
        if f.take(largo - desde).read_to_end(&mut bytes).is_err() {
            return;
        }
        self.leido = desde + bytes.len() as u64;
        let mut texto = std::mem::take(&mut self.resto);
        texto.push_str(&String::from_utf8_lossy(&bytes));
        let mut partes: Vec<&str> = texto.split('\n').collect();
        // La última, si no terminó en salto de línea, queda para la próxima lectura.
        let ultima = partes.pop().unwrap_or("");
        if cortado && !partes.is_empty() {
            partes.remove(0); // la primera quedó a medias
        }
        for l in partes {
            let l = l.trim_end_matches('\r');
            if !l.trim().is_empty() {
                self.lineas.push(clasificar(l));
            }
        }
        self.resto = ultima.to_string();
        if self.lineas.len() > MAX_LINEAS {
            let sobra = self.lineas.len() - MAX_LINEAS;
            self.lineas.drain(..sobra);
        }
    }
}

/// Nivel de una línea de registro de CasparCG (`[fecha] [nivel] mensaje`) o, si no
/// tiene ese formato (el scanner), por las palabras que usa.
pub(super) fn clasificar(texto: &str) -> Linea {
    let bajo = texto.to_lowercase();
    let nivel_caspar = texto
        .strip_prefix('[')
        .and_then(|r| r.split_once("] ["))
        .and_then(|(_, r)| r.split_once(']'))
        .map(|(n, _)| n.trim().to_lowercase());
    let nivel = match nivel_caspar.as_deref() {
        Some("error" | "fatal" | "critical") => Nivel::Error,
        Some("warning" | "warn") => Nivel::Aviso,
        Some("info") => Nivel::Info,
        Some(_) => Nivel::Detalle,
        None if bajo.contains("error") || bajo.contains("exception") => Nivel::Error,
        None if bajo.contains("warn") => Nivel::Aviso,
        None => Nivel::Info,
    };
    Linea { texto: texto.to_string(), nivel, consulta: es_consulta(&bajo) }
}

/// Las preguntas que Relevo le hace a CasparCG cada poco (y sus respuestas): llenan
/// la consola y no dicen nada.
fn es_consulta(bajo: &str) -> bool {
    let Some(i) = bajo.find("received message from").or_else(|| bajo.find("sent message to")) else {
        return false;
    };
    let resto = &bajo[i..];
    let msg = resto.split_once(": ").map(|(_, m)| m).unwrap_or(resto);
    let msg = msg.trim_start_matches(|c: char| c.is_ascii_digit() || c == ' ');
    ["version", "info", "cls", "tls", "cinf", "thumbnail", "data list", "ping", "201 ", "200 "]
        .iter()
        .any(|c| msg.starts_with(c))
}

impl Ventana {
    pub(super) fn pantalla_caspar(&mut self, ui: &mut egui::Ui, e: &Instantanea) {
        let carpeta = self.arranque.as_ref().and_then(|a| a.lock().expect("arranque").carpeta.clone());
        self.caspar.sondear(carpeta.clone(), false);
        // Lo que terminó de hacer la última acción.
        let hecho = self.caspar.resultado.lock().expect("resultado").take();
        if let Some(r) = hecho {
            match r {
                Ok(m) => self.avisar_local(m),
                Err(m) => self.avisar_error(m),
            }
            self.caspar.sondear(carpeta.clone(), true);
        }
        let procesos = self.caspar.procesos.lock().expect("procesos").clone();
        let como = self.cfg.casparcg.como_abrir(&self.carpeta_datos_local);
        let en_tv = e.fase == FaseVista::Tv;

        let Some(carpeta) = carpeta else {
            mensaje_vacio(
                ui,
                "🔍",
                "No se encontró CasparCG en esta PC",
                "Indica su carpeta en Ajustes › CasparCG › Carpeta del programa",
            );
            return;
        };

        let mut pedir: Option<Accion> = None;
        ui.columns(2, |cols| {
            pedir = self.tarjeta_proceso(&mut cols[0], e, procesos.as_ref(), true, en_tv);
            pedir = pedir.or(self.tarjeta_proceso(&mut cols[1], e, procesos.as_ref(), false, en_tv));
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Carpeta: {}", carpeta.display())).size(12.0).color(MUY_TENUE));
            let modo =
                if como.ocultas { "Consolas: ocultas (se ven aquí)" } else { "Consolas: en sus propias ventanas" };
            ui.label(RichText::new(format!("·  {modo}")).size(12.0).color(MUY_TENUE))
                .on_hover_text("Se cambia en Ajustes › CasparCG; vale desde la próxima vez que se abran");
        });
        ui.add_space(8.0);

        if let Some(a) = pedir {
            if a.pide_confirmar() {
                self.caspar.confirmar = Some(a);
            } else {
                self.hacer(ui.ctx(), a, &carpeta, &como);
            }
        }
        self.confirmar_accion(ui, &carpeta, &como, en_tv);
        self.consolas(ui, &carpeta, &como);
    }

    /// Tarjeta de CasparCG (`es_caspar`) o del media scanner.
    fn tarjeta_proceso(
        &mut self,
        ui: &mut egui::Ui,
        e: &Instantanea,
        procesos: Option<&Procesos>,
        es_caspar: bool,
        en_tv: bool,
    ) -> Option<Accion> {
        let mut pedir = None;
        let trabajando = self.caspar.trabajando.load(Ordering::SeqCst);
        tema::marco_columna().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let pids: Option<&Vec<u32>> = procesos.map(|p| if es_caspar { &p.caspar } else { &p.scanner });
            let sin_scanner = !es_caspar && procesos.is_some_and(|p| p.nombre_scanner.is_none());
            let (color, fondo, estado) = match pids.map(|v| v.len()) {
                None => (TENUE, TARJETA, "Buscando…".to_string()),
                _ if sin_scanner => (AMBAR, AMBAR_OSCURO, "No está en la carpeta".to_string()),
                // Responde por AMCP pero su proceso no está en esta PC (u otro nombre).
                Some(0) if es_caspar && e.conectado => (TV, TV_OSCURO, "Responde".to_string()),
                Some(0) => (AIRE, AIRE_OSCURO, "Cerrado".to_string()),
                Some(1) => (TV, TV_OSCURO, "Abierto".to_string()),
                Some(n) => (AMBAR, AMBAR_OSCURO, format!("{n} copias abiertas")),
            };
            ui.horizontal(|ui| {
                let t = if es_caspar { "CasparCG Server" } else { "Media scanner" };
                ui.label(RichText::new(t).size(16.0).family(tema::negrita()).color(TEXTO));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    tema::chip(ui, &estado, color, fondo);
                });
            });
            ui.add_space(2.0);
            let mut detalle = Vec::new();
            if let Some(v) = pids.filter(|v| !v.is_empty()) {
                let l: Vec<String> = v.iter().map(|p| p.to_string()).collect();
                detalle.push(format!("PID {}", l.join(", ")));
            }
            if es_caspar && e.conectado && pids.is_some_and(|v| v.is_empty()) {
                detalle.push("No se ve su proceso en esta PC".into());
            }
            if es_caspar {
                if e.conectado {
                    detalle.push(format!(
                        "AMCP conectado{}",
                        e.version.as_ref().map(|v| format!(" · v{v}")).unwrap_or_default()
                    ));
                } else {
                    detalle.push("AMCP sin conexión".into());
                }
            } else if let Some(n) = procesos.and_then(|p| p.nombre_scanner.as_ref()) {
                detalle.push(n.clone());
            }
            ui.label(RichText::new(detalle.join("  ·  ")).size(12.5).color(TENUE));
            if pids.is_some_and(|v| v.len() > 1) {
                ui.add_space(4.0);
                let t = if es_caspar {
                    "Hay más de una copia: cada una abre su propia salida de video. Reinícialo para dejar solo una."
                } else {
                    "Hay más de una copia del scanner. Reinícialo para dejar solo una."
                };
                ui.label(RichText::new(t).size(12.0).color(AMBAR));
            }
            ui.add_space(8.0);
            // Si ya responde por AMCP, no se ofrece abrir otra copia.
            let hay_proceso = pids.is_some_and(|v| !v.is_empty());
            let abierto = hay_proceso || (es_caspar && e.conectado);
            let conocido = pids.is_some() && !sin_scanner;
            ui.horizontal(|ui| {
                let boton = |t: &str| egui::Button::new(RichText::new(t).size(13.0)).min_size(Vec2::new(0.0, 28.0));
                let libre = conocido && !trabajando;
                if es_caspar {
                    if ui.add_enabled(libre && !abierto, boton("▶ Abrir")).clicked() {
                        pedir = Some(Accion::AbrirCaspar);
                    }
                    let corta = libre && hay_proceso && en_tv;
                    let espera = "Espera a que termine la tanda (solo en señal de TV)";
                    let r = ui.add_enabled(corta, boton("⟳ Reiniciar"));
                    if r.clicked() {
                        pedir = Some(Accion::ReiniciarCaspar);
                    }
                    if !en_tv {
                        r.on_disabled_hover_text(espera);
                    }
                    let r = ui.add_enabled(corta, boton("⏹ Cerrar"));
                    if r.clicked() {
                        pedir = Some(Accion::CerrarCaspar);
                    }
                    if !en_tv {
                        r.on_disabled_hover_text(espera);
                    }
                    // Si la ventana de video quedó minimizada (en negro), volver a mostrarla.
                    if ui
                        .add_enabled(hay_proceso, boton("🖵 Mostrar salida"))
                        .on_hover_text("Vuelve a mostrar la ventana de video de CasparCG si quedó minimizada")
                        .clicked()
                    {
                        let n = pids.map(|v| arranque::restaurar_salida(v)).unwrap_or(0);
                        let m = if n > 0 {
                            "Se volvió a mostrar la salida de video de CasparCG".to_string()
                        } else {
                            "La salida de video no estaba minimizada".to_string()
                        };
                        *self.caspar.resultado.lock().expect("resultado") = Some(Ok(m));
                    }
                } else {
                    if ui.add_enabled(libre && !abierto, boton("▶ Abrir")).clicked() {
                        pedir = Some(Accion::AbrirScanner);
                    }
                    if ui
                        .add_enabled(libre && abierto, boton("⟳ Reiniciar"))
                        .on_hover_text("Vuelve a leer la carpeta de medios")
                        .clicked()
                    {
                        pedir = Some(Accion::ReiniciarScanner);
                    }
                }
                if trabajando {
                    ui.spinner();
                }
            });
        });
        pedir
    }

    fn confirmar_accion(&mut self, ui: &mut egui::Ui, carpeta: &Path, como: &arranque::ComoAbrir, en_tv: bool) {
        let Some(a) = self.caspar.confirmar else {
            return;
        };
        let (icono, titulo, texto, boton) = match a {
            Accion::CerrarCaspar => {
                ("⏹", "¿Cerrar CasparCG?", "Se corta la salida de video hasta que lo vuelvas a abrir.", "Cerrar")
            }
            _ => (
                "⟳",
                "¿Reiniciar CasparCG?",
                "Se cierran todas sus copias y se abre una sola. La salida se corta unos segundos.",
                "Reiniciar",
            ),
        };
        let mut si = false;
        let mut no = false;
        let modal = Dialogo::new("relevo-confirmar-caspar", icono, titulo).tono(Tono::Peligro).ancho(400.0).mostrar(
            ui.ctx(),
            |ui| {
                ui.label(RichText::new(texto).size(13.5).color(TEXTO));
                let (s, n) = dialogo::pie(ui, boton, Tono::Peligro, en_tv || !a.corta_salida());
                si = s;
                no = n;
            },
        );
        if modal.cerrar || no {
            self.caspar.confirmar = None;
        }
        if si {
            self.caspar.confirmar = None;
            self.hacer(ui.ctx(), a, carpeta, como);
        }
    }

    /// Hace la acción en segundo plano (reiniciar espera unos segundos).
    fn hacer(&mut self, ctx: &egui::Context, a: Accion, carpeta: &Path, como: &arranque::ComoAbrir) {
        if self.caspar.trabajando.swap(true, Ordering::SeqCst) {
            return;
        }
        let carpeta = carpeta.to_path_buf();
        let como = como.clone();
        let resultado = self.caspar.resultado.clone();
        let trabajando = self.caspar.trabajando.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = match a {
                Accion::AbrirCaspar => arranque::abrir_caspar(&carpeta, &como).map(|_| "Abriendo CasparCG…"),
                Accion::ReiniciarCaspar => {
                    arranque::reiniciar_caspar(&carpeta, &como).map(|_| "CasparCG se está reiniciando…")
                }
                Accion::CerrarCaspar => arranque::cerrar_caspar().map(|_| "CasparCG cerrado"),
                Accion::AbrirScanner => {
                    arranque::abrir_scanner(&carpeta, &como, false).map(|_| "Abriendo el media scanner…")
                }
                Accion::ReiniciarScanner => {
                    arranque::abrir_scanner(&carpeta, &como, true).map(|_| "El media scanner se está reiniciando…")
                }
            };
            // Un momento para que el programa aparezca (o desaparezca) en la lista.
            std::thread::sleep(Duration::from_millis(1200));
            *resultado.lock().expect("resultado") = Some(r.map(String::from).map_err(|e| format!("No se pudo: {e}")));
            trabajando.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    /// El archivo que se muestra en la consola elegida.
    fn archivo_de_consola(&self, carpeta: &Path, como: &arranque::ComoAbrir) -> Option<PathBuf> {
        match self.caspar.consola {
            Consola::Caspar => {
                // El registro más nuevo de CasparCG; o lo que escribió sin consola, si es más nuevo.
                let logs = arranque::carpeta_logs(carpeta);
                let mut candidatos: Vec<PathBuf> = std::fs::read_dir(&logs)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|d| d.path())
                    .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("log")))
                    .collect();
                candidatos.push(como.carpeta_consolas.join("casparcg.log"));
                mas_reciente(candidatos)
            }
            Consola::Scanner => Some(como.carpeta_consolas.join("scanner.log")).filter(|p| p.exists()),
        }
    }

    fn consolas(&mut self, ui: &mut egui::Ui, carpeta: &Path, como: &arranque::ComoAbrir) {
        let archivo = self.archivo_de_consola(carpeta, como);
        self.caspar.leer(archivo);
        // Mientras la pestaña está a la vista, se sigue leyendo.
        ui.ctx().request_repaint_after(CADA_LECTURA);

        tema::marco_columna().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                tema::titulo(ui, "CONSOLA");
                ui.add_space(8.0);
                for (c, t) in [(Consola::Caspar, "CasparCG"), (Consola::Scanner, "Media scanner")] {
                    if ui
                        .add(egui::Button::new(RichText::new(t).size(13.0)).selected(self.caspar.consola == c))
                        .clicked()
                    {
                        self.caspar.cambiar_consola(c);
                    }
                }
                ui.add_space(12.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.caspar.buscar).hint_text("🔍  Buscar…").desired_width(180.0),
                );
                egui::ComboBox::from_id_salt("nivel_consola")
                    .selected_text(match self.caspar.nivel {
                        Nivel::Detalle => "Todo",
                        Nivel::Info => "Info y más",
                        Nivel::Aviso => "Avisos y errores",
                        Nivel::Error => "Solo errores",
                    })
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.caspar.nivel, Nivel::Detalle, "Todo");
                        ui.selectable_value(&mut self.caspar.nivel, Nivel::Info, "Info y más");
                        ui.selectable_value(&mut self.caspar.nivel, Nivel::Aviso, "Avisos y errores");
                        ui.selectable_value(&mut self.caspar.nivel, Nivel::Error, "Solo errores");
                    });
                if self.caspar.consola == Consola::Caspar {
                    ui.checkbox(&mut self.caspar.ocultar_consultas, "Ocultar consultas de Relevo")
                        .on_hover_text("VERSION, INFO, CLS… que Relevo pregunta cada poco");
                }
                ui.checkbox(&mut self.caspar.seguir, "Seguir al final");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("🗁 Abrir carpeta").clicked() {
                        let dir = self
                            .caspar
                            .archivo
                            .as_ref()
                            .and_then(|a| a.parent().map(Path::to_path_buf))
                            .unwrap_or_else(|| arranque::carpeta_logs(carpeta));
                        crate::exportar::abrir(&dir);
                    }
                    if ui.small_button("Limpiar").on_hover_text("Solo borra lo que se ve aquí").clicked() {
                        self.caspar.lineas.clear();
                    }
                });
            });
            if let Some(a) = &self.caspar.archivo {
                ui.label(RichText::new(a.display().to_string()).size(11.5).color(MUY_TENUE));
            }
            ui.add_space(4.0);

            let filtro = self.caspar.buscar.trim().to_lowercase();
            let ocultar = self.caspar.ocultar_consultas && self.caspar.consola == Consola::Caspar;
            let nivel = self.caspar.nivel;
            let visibles: Vec<&Linea> = self
                .caspar
                .lineas
                .iter()
                .filter(|l| l.nivel >= nivel && !(ocultar && l.consulta))
                .filter(|l| filtro.is_empty() || l.texto.to_lowercase().contains(&filtro))
                .collect();

            let fondo = egui::Frame::new().fill(Color32::from_rgb(0x08, 0x0A, 0x0E)).corner_radius(6).inner_margin(8);
            fondo.show(ui, |ui| {
                ui.set_width(ui.available_width());
                let alto = ui.available_height().max(120.0);
                if visibles.is_empty() {
                    ui.allocate_ui(Vec2::new(ui.available_width(), alto), |ui| {
                        let (t, sub) = vacio_consola(self.caspar.consola, self.caspar.archivo.is_some(), como.ocultas);
                        mensaje_vacio(ui, "⌨", t, sub);
                    });
                    return;
                }
                let fila = 16.0;
                egui::ScrollArea::both()
                    .id_salt(("consola", self.caspar.consola == Consola::Caspar))
                    .auto_shrink([false, false])
                    .max_height(alto)
                    .stick_to_bottom(self.caspar.seguir)
                    .show_rows(ui, fila, visibles.len(), |ui, filas| {
                        for l in &visibles[filas] {
                            let color = match l.nivel {
                                Nivel::Error => AIRE,
                                Nivel::Aviso => AMBAR,
                                Nivel::Info => TEXTO,
                                Nivel::Detalle => TENUE,
                            };
                            ui.add(
                                egui::Label::new(RichText::new(&l.texto).monospace().size(12.0).color(color))
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        }
                    });
            });
        });
    }
}

fn vacio_consola(c: Consola, hay_archivo: bool, ocultas: bool) -> (&'static str, &'static str) {
    match (c, hay_archivo) {
        (_, true) => ("Nada que mostrar", "Cambia el filtro o espera a que escriba algo"),
        (Consola::Caspar, false) => ("CasparCG todavía no escribió su registro", "Aparece cuando CasparCG se abre"),
        (Consola::Scanner, false) if ocultas => {
            ("El scanner todavía no escribió nada", "Se ve aquí cuando Relevo lo abre sin consola")
        }
        (Consola::Scanner, false) => (
            "El scanner está en su propia ventana",
            "Para verlo aquí, elige «Consolas ocultas» en Ajustes › CasparCG y reinícialo",
        ),
    }
}

fn mas_reciente(rutas: Vec<PathBuf>) -> Option<PathBuf> {
    rutas
        .into_iter()
        .filter_map(|p| std::fs::metadata(&p).and_then(|m| m.modified()).ok().map(|t| (t, p)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn clasifica_las_lineas_de_casparcg() {
        let l = clasificar("[2026-11-14 10:00:01.123] [error]   Failed to open file");
        assert_eq!(l.nivel, Nivel::Error);
        assert_eq!(clasificar("[2026-11-14 10:00:01.123] [warning] x").nivel, Nivel::Aviso);
        assert_eq!(clasificar("[2026-11-14 10:00:01.123] [info]    x").nivel, Nivel::Info);
        assert_eq!(clasificar("[2026-11-14 10:00:01.123] [debug]   x").nivel, Nivel::Detalle);
        // El scanner no tiene ese formato.
        assert_eq!(clasificar("Error: ENOENT").nivel, Nivel::Error);
        assert_eq!(clasificar("scanning media").nivel, Nivel::Info);
    }

    #[test]
    fn reconoce_las_consultas_de_relevo() {
        let a = "[2026-11-14 10:00:01.123] [info]    Received message from 127.0.0.1: VERSION\\r\\n";
        assert!(clasificar(a).consulta);
        let b = "[2026-11-14 10:00:01.123] [info]    Sent message to 127.0.0.1:5250: 201 VERSION OK";
        assert!(clasificar(b).consulta);
        let c = "[2026-11-14 10:00:01.123] [info]    Received message from 127.0.0.1: PLAY 1-10 \"spot\"";
        assert!(!clasificar(c).consulta);
        assert!(!clasificar("[2026-11-14 10:00:01.123] [info]    Initialized video modes.").consulta);
    }

    #[test]
    fn lee_solo_lo_nuevo_del_archivo() {
        let dir = std::env::temp_dir().join(format!("relevo-consola-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ruta = dir.join("casparcg.log");
        std::fs::write(&ruta, "uno\ndos\ntr").unwrap();
        let mut e = EstadoCaspar::new();
        e.leer(Some(ruta.clone()));
        assert_eq!(e.lineas.len(), 2);
        // Se completa la línea a medias.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&ruta)
            .and_then(|mut f| {
                use std::io::Write;
                f.write_all(b"es\ncuatro\n")
            })
            .unwrap();
        e.ultima_lectura = None;
        e.leer(Some(ruta.clone()));
        let t: Vec<&str> = e.lineas.iter().map(|l| l.texto.as_str()).collect();
        assert_eq!(t, ["uno", "dos", "tres", "cuatro"]);
        // Si el archivo vuelve a empezar, se lee desde cero.
        std::fs::write(&ruta, "nuevo\n").unwrap();
        e.ultima_lectura = None;
        e.leer(Some(ruta.clone()));
        assert_eq!(e.lineas.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
