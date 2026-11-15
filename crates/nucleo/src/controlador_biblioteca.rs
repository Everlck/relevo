//! Parte del núcleo que maneja la biblioteca: revisa los videos de la carpeta de
//! medios, hace los trabajos (nivelar, convertir, reemplazar) de a uno y en segundo
//! plano, y lleva la papelera.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::{Interno, Nucleo, TipoEvento};
use crate::biblioteca::{self as bib, EstadoTrabajo, Estandar, Ficha, Papelera, TipoTrabajo, Trabajo, VistaBiblioteca};

/// Lo que el núcleo guarda de la biblioteca.
pub(super) struct Biblioteca {
    pub ffmpeg: Option<PathBuf>,
    pub estandar: Estandar,
    /// Clave del clip → revisión.
    pub fichas: HashMap<String, Ficha>,
    pub fichas_pub: Arc<HashMap<String, Ficha>>,
    fichas_cambiaron: bool,
    /// Clave del clip → archivo.
    pub indice: HashMap<String, PathBuf>,
    cola: VecDeque<String>,
    revisando: Option<String>,
    total_revision: usize,
    hechas_revision: usize,
    pub trabajos: Vec<Trabajo>,
    siguiente_trabajo: u64,
    cancelar: HashMap<u64, Arc<AtomicBool>>,
    /// Archivo listo de cada trabajo terminado.
    resultados: HashMap<u64, PathBuf>,
    pub papelera: Papelera,
    pub disco: Option<u64>,
    disco_avisado: bool,
    pub cambios: u64,
    indexando: bool,
}

impl Biblioteca {
    pub fn new(ffmpeg: Option<PathBuf>, estandar: Estandar, carpeta_datos: &Path) -> Self {
        let fichas = bib::cargar_fichas(carpeta_datos);
        Biblioteca {
            ffmpeg,
            estandar,
            fichas_pub: Arc::new(fichas.clone()),
            fichas,
            fichas_cambiaron: false,
            indice: HashMap::new(),
            cola: VecDeque::new(),
            revisando: None,
            total_revision: 0,
            hechas_revision: 0,
            trabajos: Vec::new(),
            siguiente_trabajo: 1,
            cancelar: HashMap::new(),
            resultados: HashMap::new(),
            papelera: Papelera::abrir(carpeta_datos),
            disco: None,
            disco_avisado: false,
            cambios: 0,
            indexando: false,
        }
    }

    pub fn vista(&self) -> VistaBiblioteca {
        VistaBiblioteca {
            trabajos: self.trabajos.clone(),
            revision: (self.revisando.is_some() || !self.cola.is_empty())
                .then_some((self.hechas_revision, self.total_revision)),
            papelera: self.papelera.entradas.clone(),
            disco_libre: self.disco,
            estandar: self.estandar.clone(),
            ffmpeg: self.ffmpeg.is_some(),
            cambios: self.cambios,
        }
    }
}

impl Nucleo {
    /// Vuelve a leer qué videos hay en la carpeta de medios (en segundo plano).
    pub(super) fn indexar(&mut self) {
        let Some(medios) = self.carpeta_medios.clone() else { return };
        if self.bib.indexando {
            return;
        }
        self.bib.indexando = true;
        let fichas = self.bib.fichas.clone();
        let tx = self.tx_interno.clone();
        tokio::task::spawn_blocking(move || {
            let indice = bib::indice(&medios);
            let mut por_revisar: Vec<String> = indice
                .iter()
                .filter(|(c, p)| fichas.get(*c).is_none_or(|f| !bib::vigente(f, p)))
                .map(|(c, _)| c.clone())
                .collect();
            por_revisar.sort();
            let libre = bib::espacio_libre(&medios);
            let _ = tx.send(Interno::Indice { indice, por_revisar, libre });
        });
    }

    pub(super) fn indice_listo(
        &mut self,
        indice: HashMap<String, PathBuf>,
        por_revisar: Vec<String>,
        libre: Option<u64>,
    ) {
        self.bib.indexando = false;
        // Las fichas de archivos que ya no están se olvidan.
        let antes = self.bib.fichas.len();
        self.bib.fichas.retain(|c, _| indice.contains_key(c));
        if self.bib.fichas.len() != antes {
            self.bib.fichas_cambiaron = true;
        }
        self.bib.indice = indice;
        self.disco_libre(libre);
        if self.bib.ffmpeg.is_none() {
            return;
        }
        let mut nuevas = 0;
        for c in por_revisar {
            if !self.bib.cola.contains(&c) && self.bib.revisando.as_ref() != Some(&c) {
                self.bib.cola.push_back(c);
                nuevas += 1;
            }
        }
        if nuevas > 0 {
            if self.bib.revisando.is_none() {
                self.bib.total_revision = self.bib.cola.len();
                self.bib.hechas_revision = 0;
            } else {
                self.bib.total_revision += nuevas;
            }
        }
        self.revisar_siguiente();
        self.publicar_fichas();
    }

    fn disco_libre(&mut self, libre: Option<u64>) {
        self.bib.disco = libre;
        match libre {
            Some(l) if l < bib::DISCO_BAJO && !self.bib.disco_avisado => {
                self.bib.disco_avisado = true;
                let gb = l as f64 / 1_073_741_824.0;
                let prev = self.origen.clone();
                self.origen = super::Origen::local();
                self.avisar(format!("Queda poco espacio en el disco de los videos: {gb:.1} GB libres"));
                self.origen = prev;
            }
            Some(l) if l >= bib::DISCO_BAJO * 2 => self.bib.disco_avisado = false,
            _ => {}
        }
    }

    /// Mide el espacio libre (cada minuto).
    pub(super) fn revisar_disco(&mut self) {
        if let Some(m) = self.carpeta_medios.clone() {
            let l = bib::espacio_libre(&m);
            self.disco_libre(l);
        }
    }

    fn revisar_siguiente(&mut self) {
        if self.bib.revisando.is_some() {
            return;
        }
        let Some(ffmpeg) = self.bib.ffmpeg.clone() else { return };
        let Some(medios) = self.carpeta_medios.clone() else { return };
        while let Some(c) = self.bib.cola.pop_front() {
            let Some(archivo) = self.bib.indice.get(&c).cloned() else {
                self.bib.hechas_revision += 1;
                continue;
            };
            self.bib.revisando = Some(c.clone());
            let tx = self.tx_interno.clone();
            tokio::task::spawn_blocking(move || {
                let mut f = bib::analizar(&ffmpeg, &archivo);
                f.ruta = bib::relativa(&archivo, &medios);
                let _ = tx.send(Interno::Revisado { clave: c, ficha: f });
            });
            return;
        }
        // Terminó la revisión.
        self.bib.total_revision = 0;
        self.bib.hechas_revision = 0;
        self.guardar_fichas();
    }

    pub(super) fn revisado(&mut self, clave: String, ficha: Ficha) {
        self.bib.revisando = None;
        self.bib.hechas_revision += 1;
        self.bib.fichas.insert(clave, ficha);
        self.bib.fichas_cambiaron = true;
        if self.bib.hechas_revision.is_multiple_of(20) {
            self.guardar_fichas();
        }
        self.revisar_siguiente();
        self.publicar_fichas();
    }

    fn guardar_fichas(&mut self) {
        bib::guardar_fichas(&self.op.carpeta_datos, &self.bib.fichas);
    }

    /// Las fichas se publican aparte (son muchas): solo cuando cambian.
    fn publicar_fichas(&mut self) {
        if self.bib.fichas_cambiaron {
            self.bib.fichas_cambiaron = false;
            self.bib.fichas_pub = Arc::new(self.bib.fichas.clone());
        }
    }

    /// Vuelve a revisar un clip (o todos).
    pub(super) fn pedir_revision(&mut self, clip: Option<String>) {
        if self.bib.ffmpeg.is_none() {
            self.avisar("Para revisar los videos hace falta ffmpeg en la PC principal (winget install ffmpeg).".into());
            return;
        }
        match clip {
            Some(c) => {
                let c = bib::clave(&c);
                self.bib.fichas.remove(&c);
                self.bib.fichas_cambiaron = true;
                if !self.bib.cola.contains(&c) {
                    self.bib.cola.push_front(c);
                    self.bib.total_revision += 1;
                }
                self.revisar_siguiente();
            }
            None => {
                self.bib.fichas.clear();
                self.bib.fichas_cambiaron = true;
                self.indexar();
            }
        }
        self.publicar_fichas();
    }

    // ---------- Trabajos ----------

    /// El clip está en la tanda (al aire o por salir): su archivo no se puede tocar.
    fn en_uso(&self, clip: &str) -> bool {
        self.rep.lista().iter().any(|i| i.clip.eq_ignore_ascii_case(clip))
    }

    fn archivo_de(&self, clip: &str) -> Option<PathBuf> {
        if let Some(r) = self.bib.indice.get(&bib::clave(clip)) {
            return Some(r.clone());
        }
        // Un video recién agregado puede no estar todavía en el índice (se arma en
        // segundo plano): se busca directamente en la carpeta.
        let medios = self.carpeta_medios.as_ref()?;
        let clip = clip.replace('\\', "/");
        let (carpeta, nombre) = clip.rsplit_once('/').unwrap_or(("", clip.as_str()));
        std::fs::read_dir(medios.join(carpeta)).ok()?.flatten().map(|e| e.path()).find(|p| {
            crate::archivos::es_video(p)
                && p.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(nombre))
        })
    }

    pub(super) fn pedir_trabajo(&mut self, clip: String, tipo: TipoTrabajo) {
        if self.bib.ffmpeg.is_none() && !matches!(tipo, TipoTrabajo::Reemplazar { .. }) {
            self.avisar("Para esto hace falta ffmpeg en la PC principal (winget install ffmpeg).".into());
            return;
        }
        if self.archivo_de(&clip).is_none() {
            self.avisar(format!("No encontré el archivo de «{clip}» en la carpeta de CasparCG."));
            return;
        }
        if let TipoTrabajo::Reemplazar { desde } = &tipo {
            if !desde.is_file() || !crate::archivos::es_video(desde) {
                self.avisar(format!("«{}» no es un video.", desde.display()));
                return;
            }
            // Una PC remota solo puede usar lo que subió (está dentro de la carpeta de medios).
            let temporal = self.op.carpeta_datos.join("temporal");
            if self.origen.id != 0
                && !desde.starts_with(&temporal)
                && self.carpeta_medios.as_ref().is_none_or(|m| !desde.starts_with(m))
            {
                self.avisar("Ese archivo no está en la PC principal.".into());
                return;
            }
        }
        if self
            .bib
            .trabajos
            .iter()
            .any(|t| t.clip.eq_ignore_ascii_case(&clip) && !matches!(t.estado, EstadoTrabajo::Error(_)))
        {
            self.avisar(format!("«{clip}» ya tiene un trabajo pendiente."));
            return;
        }
        let id = self.bib.siguiente_trabajo;
        self.bib.siguiente_trabajo += 1;
        self.anotar(TipoEvento::Info, format!("{}: {clip} (pedido por {})", tipo.nombre(), self.origen.nombre));
        self.bib.trabajos.push(Trabajo {
            id,
            clip,
            tipo,
            estado: EstadoTrabajo::EnCola,
            progreso: 0.0,
            quien: self.origen.nombre.clone(),
        });
        self.siguiente_de_la_cola();
    }

    fn siguiente_de_la_cola(&mut self) {
        if self.bib.trabajos.iter().any(|t| t.estado == EstadoTrabajo::Haciendo) {
            return;
        }
        let Some(t) = self.bib.trabajos.iter().find(|t| t.estado == EstadoTrabajo::EnCola) else { return };
        let (id, clip, tipo) = (t.id, t.clip.clone(), t.tipo.clone());
        let original = self.archivo_de(&clip);
        let Some(t) = self.bib.trabajos.iter_mut().find(|t| t.id == id) else { return };
        let Some(original) = original else {
            t.estado = EstadoTrabajo::Error("el archivo ya no está".into());
            return;
        };
        t.estado = EstadoTrabajo::Haciendo;
        t.progreso = 0.0;
        let ficha = self.bib.fichas.get(&bib::clave(&clip)).cloned();
        let est = self.bib.estandar.clone();
        let ffmpeg = self.bib.ffmpeg.clone();
        let temporal = self.op.carpeta_datos.join("temporal");
        let cancelar = Arc::new(AtomicBool::new(false));
        self.bib.cancelar.insert(id, cancelar.clone());
        let tx = self.tx_interno.clone();
        tokio::task::spawn_blocking(move || {
            let r = (|| -> Result<PathBuf, String> {
                std::fs::create_dir_all(&temporal).map_err(|e| e.to_string())?;
                let ext = match &tipo {
                    TipoTrabajo::Reemplazar { desde } => {
                        desde.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or("mp4".into())
                    }
                    _ => "mp4".into(),
                };
                let salida = temporal.join(format!("trabajo-{id}.{ext}"));
                // La revisión hace falta para saber el volumen; si no está, se hace ahora.
                let ficha = match (ficha, &ffmpeg) {
                    (Some(f), _) => f,
                    (None, Some(ff)) => bib::analizar(ff, &original),
                    (None, None) => Ficha::default(),
                };
                if matches!(tipo, TipoTrabajo::Nivelar) && bib::ganancia(&ficha, &est).is_none() {
                    return Err("no tiene audio que nivelar".into());
                }
                let mut ultimo = 0.0f32;
                let ff = ffmpeg.clone().unwrap_or_default();
                bib::hacer(&ff, &tipo, &original, &ficha, &est, &salida, &cancelar, |p| {
                    if p - ultimo >= 0.01 || p >= 1.0 {
                        ultimo = p;
                        let _ = tx.send(Interno::TrabajoAvance { id, progreso: p });
                    }
                })?;
                Ok(salida)
            })();
            let _ = tx.send(Interno::TrabajoHecho { id, resultado: r });
        });
    }

    pub(super) fn trabajo_avance(&mut self, id: u64, progreso: f32) {
        if let Some(t) = self.bib.trabajos.iter_mut().find(|t| t.id == id) {
            t.progreso = progreso;
        }
    }

    pub(super) async fn trabajo_hecho(&mut self, id: u64, resultado: Result<PathBuf, String>) {
        self.bib.cancelar.remove(&id);
        let Some(t) = self.bib.trabajos.iter_mut().find(|t| t.id == id) else {
            // Se canceló: se borra lo que haya quedado.
            if let Ok(p) = resultado {
                let _ = std::fs::remove_file(p);
            }
            return;
        };
        match resultado {
            Ok(salida) => {
                t.estado = EstadoTrabajo::Esperando;
                t.progreso = 1.0;
                self.bib.resultados.insert(id, salida);
                self.aplicar_trabajos().await;
            }
            Err(e) => {
                let clip = t.clip.clone();
                t.estado = EstadoTrabajo::Error(e.clone());
                self.anotar(TipoEvento::Aviso, format!("No se pudo procesar «{clip}»: {e}"));
            }
        }
        self.siguiente_de_la_cola();
    }

    /// Cambia los archivos de los trabajos terminados cuyo spot no está en la tanda.
    pub(super) async fn aplicar_trabajos(&mut self) {
        let listos: Vec<(u64, String, TipoTrabajo)> = self
            .bib
            .trabajos
            .iter()
            .filter(|t| t.estado == EstadoTrabajo::Esperando && !self.en_uso(&t.clip))
            .map(|t| (t.id, t.clip.clone(), t.tipo.clone()))
            .collect();
        if listos.is_empty() {
            return;
        }
        let Some(medios) = self.carpeta_medios.clone() else { return };
        for (id, clip, tipo) in listos {
            self.bib.trabajos.retain(|t| t.id != id);
            let Some(nuevo) = self.bib.resultados.remove(&id) else { continue };
            // Lo subido desde otra PC ya se copió: se borra el temporal.
            if let TipoTrabajo::Reemplazar { desde } = &tipo {
                if desde.starts_with(self.op.carpeta_datos.join("temporal")) {
                    let _ = std::fs::remove_file(desde);
                }
            }
            let Some(original) = self.archivo_de(&clip) else {
                let _ = std::fs::remove_file(&nuevo);
                self.avisar(format!("«{clip}» ya no está en la carpeta de CasparCG; no se cambió nada."));
                continue;
            };
            let motivo = format!("versión anterior ({})", tipo.nombre().to_lowercase());
            let ext = nuevo.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or("mp4".into());
            let base = original.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let destino = original.with_file_name(format!("{base}.{ext}"));
            let r = (|| -> Result<(), String> {
                self.bib.papelera.tirar(&original, &medios, &motivo, bib::ahora()).map_err(|e| e.to_string())?;
                bib::mover(&nuevo, &destino).map_err(|e| e.to_string())
            })();
            match r {
                Ok(()) => {
                    self.anotar(
                        TipoEvento::Info,
                        format!(
                            "Listo: {} de «{clip}» (la versión anterior quedó en la papelera)",
                            tipo.nombre().to_lowercase()
                        ),
                    );
                    self.bib.cambios += 1;
                    self.bib.fichas.remove(&bib::clave(&clip));
                    self.bib.fichas_cambiaron = true;
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&nuevo);
                    self.avisar(format!("No se pudo cambiar el archivo de «{clip}»: {e}"));
                }
            }
        }
        self.recargar_tras_cambio().await;
    }

    /// Tras cambiar archivos: CasparCG vuelve a leer su carpeta y se revisa lo nuevo.
    async fn recargar_tras_cambio(&mut self) {
        // El escáner de CasparCG tarda un momento en notar los cambios.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        self.recargar_medios().await;
        self.indexar();
        self.publicar_fichas();
    }

    pub(super) fn cancelar_trabajo(&mut self, id: u64) {
        if let Some(c) = self.bib.cancelar.get(&id) {
            c.store(true, Ordering::SeqCst);
        }
        if let Some(p) = self.bib.resultados.remove(&id) {
            let _ = std::fs::remove_file(p);
        }
        if let Some(t) = self.bib.trabajos.iter().find(|t| t.id == id) {
            // Un archivo subido para reemplazar que ya no se usará.
            if let TipoTrabajo::Reemplazar { desde } = &t.tipo {
                if desde.starts_with(self.op.carpeta_datos.join("temporal")) {
                    let _ = std::fs::remove_file(desde);
                }
            }
            let t = format!("Cancelado: {} de «{}»", t.tipo.nombre().to_lowercase(), t.clip);
            self.anotar(TipoEvento::Info, t);
        }
        self.bib.trabajos.retain(|t| t.id != id);
        self.siguiente_de_la_cola();
    }

    // ---------- Borrar y papelera ----------

    pub(super) async fn borrar_clip(&mut self, clip: String) {
        if self.en_uso(&clip) {
            self.avisar(format!("«{clip}» está en la tanda. Quítalo de la tanda antes de borrarlo."));
            return;
        }
        let (Some(archivo), Some(medios)) = (self.archivo_de(&clip), self.carpeta_medios.clone()) else {
            self.avisar(format!("No encontré el archivo de «{clip}»."));
            return;
        };
        if !archivo.exists() {
            // Pasa si se borró hace un momento y la lista aún no se había actualizado.
            self.avisar(format!("«{clip}» ya no está en la carpeta de medios (quizás ya se borró)."));
            self.recargar_tras_cambio().await;
            return;
        }
        let motivo = format!("borrado por {}", self.origen.nombre);
        match self.bib.papelera.tirar(&archivo, &medios, &motivo, bib::ahora()) {
            Ok(_) => {
                let pautas = self.pautas.iter().filter(|p| p.clip.eq_ignore_ascii_case(&clip)).count();
                let extra = if pautas > 0 {
                    format!(" Ojo: {pautas} pauta(s) lo usan y no saldrá hasta restaurarlo.")
                } else {
                    String::new()
                };
                self.anotar(TipoEvento::Info, format!("A la papelera: {clip}.{extra}"));
                if pautas > 0 {
                    self.avisar(format!("«{clip}» se fue a la papelera.{extra}"));
                }
                self.bib.cambios += 1;
                self.recargar_tras_cambio().await;
            }
            Err(e) => self.avisar(format!("No se pudo mover «{clip}» a la papelera: {e}")),
        }
    }

    pub(super) async fn restaurar(&mut self, id: u64) {
        let Some(medios) = self.carpeta_medios.clone() else { return };
        match self.bib.papelera.restaurar(id, &medios) {
            Ok(r) => {
                self.anotar(TipoEvento::Info, format!("Restaurado: {}", bib::relativa(&r, &medios)));
                self.bib.cambios += 1;
                self.recargar_tras_cambio().await;
            }
            Err(e) => self.avisar(format!("No se pudo restaurar: {e}")),
        }
    }

    pub(super) fn eliminar_de_papelera(&mut self, id: Option<u64>) {
        let ids: Vec<u64> = match id {
            Some(i) => vec![i],
            None => self.bib.papelera.entradas.iter().map(|e| e.id).collect(),
        };
        let n = ids.len();
        for i in ids {
            if let Err(e) = self.bib.papelera.eliminar(i) {
                self.avisar(format!("No se pudo borrar de la papelera: {e}"));
                return;
            }
        }
        if n > 0 {
            self.anotar(TipoEvento::Info, format!("Borrado(s) para siempre: {n} video(s) de la papelera"));
        }
        self.revisar_disco();
    }

    pub(super) fn purgar_papelera(&mut self) {
        let n = self.bib.papelera.purgar(bib::ahora());
        if n > 0 {
            self.anotar(
                TipoEvento::Info,
                format!("Papelera: se borraron {n} video(s) de hace más de {} días", bib::DIAS_PAPELERA),
            );
        }
    }
}
