//! Lógica de la tanda: lista de spots, qué está al aire y qué sigue.
//!
//! Cada operación devuelve las [`Accion`]es que hay que enviar a CasparCG.
//! No hace red ni lee el reloj: el tiempo llega como parámetro (`ahora`),
//! así todo se puede probar sin CasparCG.
//!
//! Cómo suena una tanda en CasparCG:
//! 1. `PLAY` del spot actual.
//! 2. `LOADBG ... AUTO` del siguiente: CasparCG lo encadena solo, sin negro.
//! 3. Tras el último spot se precarga un color transparente con `AUTO`:
//!    al terminar, la capa queda vacía y vuelve a verse la señal de TV.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Color transparente de CasparCG: deja ver la capa de abajo (la señal de TV).
pub const TRANSPARENTE: &str = "#00000000";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemTanda {
    /// Identificador único dentro de la tanda (no cambia al reordenar).
    pub id: u64,
    /// Nombre del clip tal como lo usa CasparCG.
    pub clip: String,
    pub duracion: Duration,
    /// Si el spot viene de la pauta: clave del pase que cumple.
    pub pase: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Bucle {
    /// Reproduce la tanda una vez y vuelve a TV.
    #[default]
    No,
    /// Repite el spot actual sin fin.
    Uno,
    /// Al terminar el último, vuelve a empezar la tanda.
    Lista,
}

impl Bucle {
    /// Siguiente modo al pulsar el botón de bucle.
    pub fn rotar(self) -> Bucle {
        match self {
            Bucle::No => Bucle::Lista,
            Bucle::Lista => Bucle::Uno,
            Bucle::Uno => Bucle::No,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fase {
    Detenido,
    Reproduciendo { inicio: Instant },
    Pausado { transcurrido: Duration },
}

/// Lo que hay que pedirle a CasparCG.
#[derive(Debug, Clone, PartialEq)]
pub enum Accion {
    Play {
        clip: String,
        bucle: bool,
        /// Empezar en este punto del clip (cero: desde el principio).
        desde: Duration,
    },
    /// Saltar a este punto del clip que está al aire.
    Buscar(Duration),
    /// `LOADBG ... AUTO`: empieza solo cuando termina el actual.
    Precargar {
        clip: String,
    },
    Pausa,
    Reanudar,
    /// Cambia el bucle del clip que ya está al aire.
    BucleActual(bool),
    /// Vacía la capa: vuelve la señal de TV.
    Limpiar,
}

impl Accion {
    /// Descripción para mensajes al operador.
    pub fn describir(&self) -> String {
        match self {
            Accion::Play { clip, .. } => format!("poner al aire «{clip}»"),
            Accion::Precargar { clip } if clip == TRANSPARENTE => "preparar la vuelta a TV".into(),
            Accion::Precargar { clip } => format!("precargar «{clip}»"),
            Accion::Buscar(_) => "ir a ese punto del spot".into(),
            Accion::Pausa => "pausar".into(),
            Accion::Reanudar => "reanudar".into(),
            Accion::BucleActual(true) => "activar el bucle".into(),
            Accion::BucleActual(false) => "quitar el bucle".into(),
            Accion::Limpiar => "volver a TV".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Precarga {
    Nada,
    Clip(u64),
    FinATv,
}

pub struct Reproductor {
    lista: Vec<ItemTanda>,
    actual: Option<u64>,
    proximo: Option<u64>,
    fase: Fase,
    bucle: Bucle,
    aleatorio: bool,
    precarga: Precarga,
    siguiente_id: u64,
    azar: fastrand::Rng,
    /// Última vez que CasparCG informó por OSC qué archivo está al aire. Mientras
    /// llegue, el paso al siguiente spot lo decide CasparCG (no nuestro reloj).
    osc_con_archivo: Option<Instant>,
    /// Tras pasar al siguiente spot por nuestro reloj (sin confirmación de CasparCG),
    /// no se precarga el que sigue hasta este momento: si nuestro reloj iba adelantado,
    /// un LOADBG reemplazaría al spot que CasparCG todavía tiene en espera.
    espera_precarga: Option<Instant>,
    /// Con transición (fundido…), CasparCG empieza el siguiente spot este tiempo antes
    /// de que termine el actual.
    adelanto: Duration,
    /// Spot donde se detuvo la tanda a mitad (con ⏹): al dar Play se vuelve a
    /// empezar ese spot y la tanda sigue desde ahí.
    retomar: Option<u64>,
    /// Justo después de saltar a otro punto del spot, el tiempo que informa CasparCG
    /// todavía es el de antes: hasta este momento no se usa para corregir el reloj.
    tras_salto: Option<Instant>,
}

/// Cuánto se ignora el tiempo de CasparCG tras saltar a otro punto.
const ESPERA_SALTO: Duration = Duration::from_millis(700);
/// Un salto no llega más allá de esto antes del final (para que el spot no termine
/// antes de que CasparCG haya saltado).
const FINAL_MINIMO: Duration = Duration::from_millis(300);

/// Margen tras el fin calculado de un spot.
const MARGEN_PRECARGA: Duration = Duration::from_millis(1200);
/// Cuánto se espera la confirmación de CasparCG (OSC) antes de pasar por reloj propio.
const ESPERA_OSC: Duration = Duration::from_millis(1500);
/// Al terminar el último spot, cuánto se espera para limpiar la capa.
const MARGEN_FIN: Duration = Duration::from_millis(400);
/// Sin saber el archivo, solo se aceptan correcciones de tiempo pequeñas.
const CORRECCION_MAXIMA: Duration = Duration::from_millis(1000);

/// Un salto queda entre el principio y un poco antes del final.
fn limitar_salto(pos: Duration, duracion: Duration) -> Duration {
    if duracion.is_zero() {
        return pos;
    }
    pos.min(duracion.saturating_sub(FINAL_MINIMO))
}

/// ¿La ruta que informa CasparCG corresponde a este clip? Se comparan los nombres
/// de archivo sin carpeta ni extensión y sin distinguir mayúsculas.
pub fn mismo_clip(ruta: &str, clip: &str) -> bool {
    fn ultimo(s: &str) -> String {
        s.replace('\\', "/").rsplit('/').next().unwrap_or("").trim().to_lowercase()
    }
    fn sin_ext(s: &str) -> &str {
        match s.rfind('.') {
            Some(i) if i > 0 && s.len() - i <= 5 => &s[..i],
            _ => s,
        }
    }
    let a = ultimo(ruta);
    let b = ultimo(clip);
    let a = sin_ext(&a);
    !a.is_empty() && (a == b || a == sin_ext(&b))
}

impl Default for Reproductor {
    fn default() -> Self {
        Self::new()
    }
}

impl Reproductor {
    pub fn new() -> Self {
        Reproductor {
            lista: Vec::new(),
            actual: None,
            proximo: None,
            fase: Fase::Detenido,
            bucle: Bucle::No,
            aleatorio: false,
            precarga: Precarga::Nada,
            siguiente_id: 1,
            azar: fastrand::Rng::new(),
            osc_con_archivo: None,
            espera_precarga: None,
            adelanto: Duration::ZERO,
            retomar: None,
            tras_salto: None,
        }
    }

    #[cfg(test)]
    fn con_semilla(semilla: u64) -> Self {
        let mut r = Self::new();
        r.azar = fastrand::Rng::with_seed(semilla);
        r
    }

    // ---------- Consultas ----------

    pub fn lista(&self) -> &[ItemTanda] {
        &self.lista
    }
    pub fn actual(&self) -> Option<u64> {
        self.actual
    }
    pub fn proximo(&self) -> Option<u64> {
        self.proximo
    }
    pub fn fase(&self) -> Fase {
        self.fase
    }
    pub fn bucle(&self) -> Bucle {
        self.bucle
    }
    /// Spot donde quedó detenida la tanda (si se detuvo a mitad).
    pub fn retomar(&self) -> Option<u64> {
        self.retomar.filter(|id| self.item(*id).is_some())
    }

    /// Elige desde qué spot sigue la tanda detenida (`None`: desde el principio).
    pub fn poner_retomar(&mut self, id: Option<u64>) {
        if self.fase == Fase::Detenido {
            self.retomar = id.filter(|i| self.item(*i).is_some());
        }
    }

    pub fn aleatorio(&self) -> bool {
        self.aleatorio
    }

    pub fn item(&self, id: u64) -> Option<&ItemTanda> {
        self.lista.iter().find(|i| i.id == id)
    }

    fn indice(&self, id: u64) -> Option<usize> {
        self.lista.iter().position(|i| i.id == id)
    }

    /// Tiempo transcurrido del spot actual (dentro de la vuelta actual si está en bucle).
    pub fn transcurrido(&self, ahora: Instant) -> Duration {
        let bruto = match self.fase {
            Fase::Detenido => return Duration::ZERO,
            Fase::Reproduciendo { inicio } => ahora.saturating_duration_since(inicio),
            Fase::Pausado { transcurrido } => transcurrido,
        };
        match self.actual.and_then(|id| self.item(id)) {
            Some(item) if self.bucle == Bucle::Uno && !item.duracion.is_zero() => {
                Duration::from_nanos((bruto.as_nanos() % item.duracion.as_nanos()) as u64)
            }
            Some(item) if !item.duracion.is_zero() => bruto.min(item.duracion),
            _ => bruto,
        }
    }

    /// Duración total de la tanda.
    pub fn duracion_total(&self) -> Duration {
        self.lista.iter().map(|i| i.duracion).sum()
    }

    // ---------- Edición de la tanda ----------

    /// Agrega un spot al final.
    pub fn agregar(&mut self, clip: &str, duracion: Duration) -> (u64, Vec<Accion>) {
        self.agregar_pase(clip, duracion, None)
    }

    /// Agrega al final un spot que cumple un pase de la pauta.
    pub fn agregar_pase(&mut self, clip: &str, duracion: Duration, pase: Option<String>) -> (u64, Vec<Accion>) {
        let id = self.nuevo_id();
        self.lista.push(ItemTanda { id, clip: clip.to_string(), duracion, pase });
        (id, self.recalcular_precarga())
    }

    /// Inserta un spot justo después del que está al aire (o al inicio si no hay nada).
    pub fn insertar_siguiente(&mut self, clip: &str, duracion: Duration) -> (u64, Vec<Accion>) {
        let id = self.nuevo_id();
        let pos = self.actual.and_then(|a| self.indice(a)).map(|i| i + 1).unwrap_or(0);
        self.lista.insert(pos, ItemTanda { id, clip: clip.to_string(), duracion, pase: None });
        // Aunque esté en aleatorio, "poner como siguiente" se respeta.
        if self.fase != Fase::Detenido && self.bucle != Bucle::Uno {
            self.proximo = Some(id);
            return (id, self.aplicar_precarga_de(Some(id)));
        }
        (id, Vec::new())
    }

    /// Inserta un spot en la posición `pos` (0 = arriba; si es mayor, al final).
    pub fn insertar_en(&mut self, pos: usize, clip: &str, duracion: Duration) -> (u64, Vec<Accion>) {
        let id = self.nuevo_id();
        let pos = pos.min(self.lista.len());
        self.lista.insert(pos, ItemTanda { id, clip: clip.to_string(), duracion, pase: None });
        (id, self.recalcular_precarga())
    }

    /// Quita un spot. El que está al aire no se puede quitar (devuelve `false`).
    pub fn quitar(&mut self, id: u64) -> (bool, Vec<Accion>) {
        if self.actual == Some(id) && self.fase != Fase::Detenido {
            return (false, Vec::new());
        }
        let Some(i) = self.indice(id) else {
            return (false, Vec::new());
        };
        self.lista.remove(i);
        if self.actual == Some(id) {
            self.actual = None;
        }
        if self.retomar == Some(id) {
            // Sigue desde el que quedó en su lugar.
            self.retomar = self.lista.get(i).map(|x| x.id);
        }
        (true, self.recalcular_precarga())
    }

    /// Mueve un spot `delta` posiciones (negativo = hacia arriba).
    pub fn mover(&mut self, id: u64, delta: i32) -> Vec<Accion> {
        let Some(i) = self.indice(id) else {
            return Vec::new();
        };
        let destino = (i as i64 + delta as i64).clamp(0, self.lista.len() as i64 - 1) as usize;
        if destino == i {
            return Vec::new();
        }
        let item = self.lista.remove(i);
        self.lista.insert(destino, item);
        self.recalcular_precarga()
    }

    /// Quita varios spots (no el que está al aire). Devuelve cuántos se quitaron.
    pub fn quitar_varios(&mut self, ids: &[u64]) -> (usize, Vec<Accion>) {
        let mut n = 0;
        let mut acciones = Vec::new();
        for id in ids {
            let (ok, acc) = self.quitar(*id);
            if ok {
                n += 1;
                acciones = acc;
            }
        }
        (n, acciones)
    }

    /// Mueve varios spots juntos (en su orden) para que queden antes del que estaba en
    /// la posición `destino` de la lista actual (`destino` = largo: al final).
    pub fn mover_a(&mut self, ids: &[u64], destino: usize) -> Vec<Accion> {
        let destino = destino.min(self.lista.len());
        let antes = self.lista.iter().take(destino).filter(|i| ids.contains(&i.id)).count();
        let (movidos, resto): (Vec<ItemTanda>, Vec<ItemTanda>) =
            std::mem::take(&mut self.lista).into_iter().partition(|i| ids.contains(&i.id));
        self.lista = resto;
        if movidos.is_empty() {
            return Vec::new();
        }
        let k = (destino - antes).min(self.lista.len());
        self.lista.splice(k..k, movidos);
        self.proximo = None;
        self.recalcular_precarga()
    }

    /// Duplica varios spots: las copias van juntas detrás del último de ellos. Las
    /// copias no cumplen ningún pase de la pauta.
    pub fn duplicar(&mut self, ids: &[u64]) -> Vec<Accion> {
        let Some(ultimo) = self.lista.iter().rposition(|i| ids.contains(&i.id)) else {
            return Vec::new();
        };
        let originales: Vec<ItemTanda> = self.lista.iter().filter(|i| ids.contains(&i.id)).cloned().collect();
        let copias: Vec<ItemTanda> =
            originales.into_iter().map(|i| ItemTanda { id: self.nuevo_id(), pase: None, ..i }).collect();
        self.lista.splice(ultimo + 1..ultimo + 1, copias);
        self.proximo = None;
        self.recalcular_precarga()
    }

    /// Vacía la tanda (si hay algo al aire, se conserva ese spot).
    pub fn limpiar_lista(&mut self) -> Vec<Accion> {
        self.retomar = None;
        let al_aire = if self.fase != Fase::Detenido { self.actual } else { None };
        self.lista.retain(|i| Some(i.id) == al_aire);
        self.recalcular_precarga()
    }

    /// Reemplaza la tanda por otra (por ejemplo, una tanda guardada).
    /// Si hay algo al aire, ese spot se conserva al inicio.
    /// Cada elemento: clip, duración y (si viene de la pauta) la clave del pase.
    pub fn reemplazar(&mut self, nuevos: &[(String, Duration, Option<String>)]) -> Vec<Accion> {
        self.retomar = None;
        let al_aire = if self.fase != Fase::Detenido { self.actual } else { None };
        self.lista.retain(|i| Some(i.id) == al_aire);
        for (clip, duracion, pase) in nuevos {
            let id = self.nuevo_id();
            self.lista.push(ItemTanda { id, clip: clip.clone(), duracion: *duracion, pase: pase.clone() });
        }
        self.proximo = None;
        self.recalcular_precarga()
    }

    /// Ajusta el reloj con el tiempo real que informa CasparCG (OSC). Si CasparCG
    /// dice qué archivo está al aire y ya es el siguiente spot, se pasa a él (y recién
    /// entonces se precarga el que sigue).
    pub fn sincronizar(
        &mut self,
        ahora: Instant,
        transcurrido: Duration,
        total: Duration,
        archivo: Option<&str>,
    ) -> TickResultado {
        let mut res = TickResultado::default();
        let Fase::Reproduciendo { inicio } = self.fase else {
            return res;
        };
        let Some(actual) = self.actual.and_then(|id| self.item(id)).cloned() else {
            return res;
        };
        let recien_salto = self.tras_salto.is_some_and(|t| ahora < t);
        if !recien_salto {
            self.tras_salto = None;
        }
        let local = ahora.saturating_duration_since(inicio);
        let local = if self.bucle == Bucle::Uno && !total.is_zero() {
            Duration::from_nanos((local.as_nanos() % total.as_nanos()) as u64)
        } else {
            local
        };

        if let Some(archivo) = archivo {
            self.osc_con_archivo = Some(ahora);
            let es_actual = mismo_clip(archivo, &actual.clip);
            let prox = self.proximo.and_then(|p| self.item(p)).cloned();
            let es_prox = prox.as_ref().is_some_and(|p| mismo_clip(archivo, &p.clip));
            // Con el mismo clip dos veces seguidas, el cambio se nota porque el tiempo
            // vuelve a empezar cerca del final del anterior.
            let reinicio = self.bucle != Bucle::Uno
                && !recien_salto
                && transcurrido + CORRECCION_MAXIMA < local
                && local + MARGEN_PRECARGA >= actual.duracion;
            if es_prox && (!es_actual || reinicio) {
                // CasparCG confirmó el cambio: se puede precargar ya.
                self.espera_precarga = None;
                self.avanzar(ahora.checked_sub(transcurrido).unwrap_or(ahora), &mut res);
                return res;
            }
            if !es_actual || recien_salto {
                return res; // el anterior terminando, algo ajeno a la tanda, o un salto en curso
            }
            // La duración que cuenta es la de CasparCG (puede diferir un poco de la que
            // dice el archivo, por ejemplo si el audio dura más que el video).
            if let Some(item) = self.actual.and_then(|id| self.lista.iter_mut().find(|i| i.id == id)) {
                if !total.is_zero() && distancia(item.duracion, total) > Duration::from_millis(20) {
                    item.duracion = total;
                }
            }
            if distancia(local, transcurrido) > Duration::from_millis(150) {
                self.fase = Fase::Reproduciendo { inicio: ahora.checked_sub(transcurrido).unwrap_or(ahora) };
            }
            return res;
        }

        // Sin el nombre del archivo: solo si la duración coincide y la corrección es
        // pequeña (un salto grande suele ser el clip anterior terminando o el siguiente).
        if let Some(item) = self.actual.and_then(|id| self.lista.iter_mut().find(|i| i.id == id)) {
            if item.duracion.is_zero() && !total.is_zero() {
                item.duracion = total;
            }
        }
        let duracion = self.actual.and_then(|id| self.item(id)).map(|i| i.duracion).unwrap_or_default();
        if recien_salto || distancia(duracion, total) > Duration::from_millis(600) || transcurrido > total {
            return res;
        }
        let d = distancia(local, transcurrido);
        if d >= CORRECCION_MAXIMA {
            return res;
        }
        if d > Duration::from_millis(150) {
            self.fase = Fase::Reproduciendo { inicio: ahora.checked_sub(transcurrido).unwrap_or(ahora) };
        }
        // El tiempo concuerda con este clip: se adopta la duración de CasparCG.
        if let Some(item) = self.actual.and_then(|id| self.lista.iter_mut().find(|i| i.id == id)) {
            if !total.is_zero() && distancia(item.duracion, total) > Duration::from_millis(20) {
                item.duracion = total;
            }
        }
        res
    }

    /// Pasa al spot próximo (que CasparCG encadenó con AUTO), empezado en `inicio`.
    fn avanzar(&mut self, inicio: Instant, res: &mut TickResultado) -> bool {
        let Some(prox) = self.proximo.filter(|p| self.item(*p).is_some()) else {
            return false;
        };
        self.actual = Some(prox);
        self.fase = Fase::Reproduciendo { inicio };
        self.precarga = Precarga::Nada; // se consumió con AUTO
        res.empezo.push(prox);
        res.acciones.extend(self.recalcular_precarga());
        true
    }

    // ---------- Transporte ----------

    /// Pone al aire el spot `id`.
    pub fn reproducir(&mut self, id: u64, ahora: Instant) -> Vec<Accion> {
        let Some(item) = self.item(id).cloned() else {
            return Vec::new();
        };
        self.actual = Some(id);
        self.retomar = None;
        self.fase = Fase::Reproduciendo { inicio: ahora };
        self.precarga = Precarga::Nada; // PLAY descarta lo precargado
        self.espera_precarga = None;
        let mut acciones =
            vec![Accion::Play { clip: item.clip, bucle: self.bucle == Bucle::Uno, desde: Duration::ZERO }];
        acciones.extend(self.recalcular_precarga());
        acciones
    }

    /// Botón Play/Pausa: arranca la tanda, la pausa o la reanuda.
    pub fn play_pausa(&mut self, seleccion: Option<u64>, ahora: Instant) -> Vec<Accion> {
        match self.fase {
            Fase::Detenido => {
                // Si se detuvo a mitad, se vuelve a empezar el spot donde quedó.
                let id = self
                    .retomar()
                    .or(seleccion.filter(|s| self.item(*s).is_some()))
                    .or(self.lista.first().map(|i| i.id));
                id.map(|id| self.reproducir(id, ahora)).unwrap_or_default()
            }
            Fase::Reproduciendo { .. } => self.pausar(ahora),
            Fase::Pausado { .. } => self.reanudar(ahora),
        }
    }

    pub fn pausar(&mut self, ahora: Instant) -> Vec<Accion> {
        if let Fase::Reproduciendo { inicio } = self.fase {
            self.fase = Fase::Pausado { transcurrido: ahora.saturating_duration_since(inicio) };
            return vec![Accion::Pausa];
        }
        Vec::new()
    }

    pub fn reanudar(&mut self, ahora: Instant) -> Vec<Accion> {
        if let Fase::Pausado { transcurrido } = self.fase {
            self.fase = Fase::Reproduciendo { inicio: ahora - transcurrido };
            return vec![Accion::Reanudar];
        }
        Vec::new()
    }

    /// Salta a otro punto del spot al aire (o en pausa). Devuelve el punto real
    /// (ajustado para no pasarse del final).
    pub fn buscar(&mut self, posicion: Duration, ahora: Instant) -> Vec<Accion> {
        let Some(item) = self.actual.and_then(|id| self.item(id)).cloned() else {
            return Vec::new();
        };
        let pos = limitar_salto(posicion, item.duracion);
        match self.fase {
            Fase::Reproduciendo { .. } => {
                self.fase = Fase::Reproduciendo { inicio: ahora.checked_sub(pos).unwrap_or(ahora) }
            }
            Fase::Pausado { .. } => self.fase = Fase::Pausado { transcurrido: pos },
            Fase::Detenido => return Vec::new(),
        }
        self.tras_salto = Some(ahora + ESPERA_SALTO);
        vec![Accion::Buscar(pos)]
    }

    /// Pone al aire el spot `id` desde ese punto (clic en la línea de la tanda).
    pub fn reproducir_desde(&mut self, id: u64, posicion: Duration, ahora: Instant) -> Vec<Accion> {
        if self.actual == Some(id) && self.fase != Fase::Detenido {
            return self.buscar(posicion, ahora);
        }
        let Some(dur) = self.item(id).map(|i| i.duracion) else {
            return Vec::new();
        };
        let pos = limitar_salto(posicion, dur);
        let mut acciones = self.reproducir(id, ahora);
        if pos.is_zero() {
            return acciones;
        }
        self.fase = Fase::Reproduciendo { inicio: ahora.checked_sub(pos).unwrap_or(ahora) };
        self.tras_salto = Some(ahora + ESPERA_SALTO);
        if let Some(Accion::Play { desde, .. }) = acciones.first_mut() {
            *desde = pos;
        }
        acciones
    }

    /// Salta al siguiente spot. Si no hay siguiente, vuelve a TV. Con la tanda
    /// detenida a mitad, mueve el punto donde seguirá.
    pub fn siguiente(&mut self, ahora: Instant) -> Vec<Accion> {
        if self.fase == Fase::Detenido {
            if let Some(i) = self.retomar().and_then(|r| self.indice(r)) {
                self.retomar = self.lista.get(i + 1).map(|x| x.id);
            }
            return Vec::new();
        }
        match self.proximo.or_else(|| self.calcular_siguiente()) {
            Some(id) => self.reproducir(id, ahora),
            None => self.volver_a_tv(),
        }
    }

    /// Vuelve al spot anterior (o reinicia el primero).
    pub fn anterior(&mut self, ahora: Instant) -> Vec<Accion> {
        if self.fase == Fase::Detenido {
            if let Some(i) = self.retomar().and_then(|r| self.indice(r)) {
                self.retomar = Some(self.lista[i.saturating_sub(1)].id);
            }
            return Vec::new();
        }
        let Some(i) = self.actual.and_then(|a| self.indice(a)) else {
            return Vec::new();
        };
        let id = self.lista[i.saturating_sub(1)].id;
        self.reproducir(id, ahora)
    }

    /// Corta todo y vuelve a la señal de TV. Si había un spot al aire, la tanda queda
    /// detenida ahí: el próximo Play vuelve a empezar ese spot.
    pub fn volver_a_tv(&mut self) -> Vec<Accion> {
        if self.fase != Fase::Detenido {
            self.retomar = self.actual.filter(|id| self.item(*id).is_some());
        }
        self.espera_precarga = None;
        self.fase = Fase::Detenido;
        self.actual = None;
        self.proximo = None;
        self.precarga = Precarga::Nada;
        vec![Accion::Limpiar]
    }

    /// Marca la tanda como detenida sin enviar nada (por ejemplo, si CasparCG rechazó el
    /// PLAY, o al terminar la tanda).
    pub fn forzar_detenido(&mut self) {
        self.retomar = None;
        self.espera_precarga = None;
        self.fase = Fase::Detenido;
        self.actual = None;
        self.proximo = None;
        self.precarga = Precarga::Nada;
    }

    pub fn cambiar_bucle(&mut self, bucle: Bucle, ahora: Instant) -> Vec<Accion> {
        if bucle == self.bucle {
            return Vec::new();
        }
        let pos = self.transcurrido(ahora);
        let antes_uno = self.bucle == Bucle::Uno;
        self.bucle = bucle;
        let mut acciones = Vec::new();
        if self.fase != Fase::Detenido && antes_uno != (bucle == Bucle::Uno) {
            acciones.push(Accion::BucleActual(bucle == Bucle::Uno));
            // Al salir del bucle, el reloj sigue desde la posición dentro de la vuelta actual.
            if let Fase::Reproduciendo { .. } = self.fase {
                self.fase = Fase::Reproduciendo { inicio: ahora - pos };
            }
        }
        acciones.extend(self.recalcular_precarga());
        acciones
    }

    pub fn cambiar_aleatorio(&mut self, activo: bool) -> Vec<Accion> {
        if activo == self.aleatorio {
            return Vec::new();
        }
        self.aleatorio = activo;
        self.proximo = None;
        self.recalcular_precarga()
    }

    /// Vuelve a mandar la precarga actual (por ejemplo, al cambiar la transición).
    pub fn reprecargar(&mut self) -> Vec<Accion> {
        if self.espera_precarga.is_some() {
            return Vec::new();
        }
        self.precarga = Precarga::Nada;
        let prox = self.proximo;
        self.aplicar_precarga_de(prox)
    }

    /// Duración de la transición entre spots (ver `adelanto`).
    pub fn poner_adelanto(&mut self, d: Duration) {
        self.adelanto = d;
    }

    /// Avanza el reloj. Cuando termina un spot, pasa al siguiente (CasparCG ya lo
    /// encadenó por la precarga) y precarga el que viene.
    ///
    /// Si CasparCG informa por OSC qué archivo está al aire, el paso lo hace
    /// [`sincronizar`](Self::sincronizar) en cuanto CasparCG cambia de clip; aquí solo
    /// se pasa si esa confirmación no llega. Al pasar por reloj propio, el siguiente
    /// LOADBG espera un margen para no pisar al spot que CasparCG tiene en espera.
    pub fn tick(&mut self, ahora: Instant) -> TickResultado {
        let mut res = TickResultado::default();
        if self.espera_precarga.is_some_and(|t| ahora >= t) {
            self.espera_precarga = None;
            res.acciones.extend(self.recalcular_precarga());
        }
        loop {
            let Fase::Reproduciendo { inicio } = self.fase else {
                return res;
            };
            if self.bucle == Bucle::Uno {
                return res;
            }
            let Some(item) = self.actual.and_then(|id| self.item(id)).cloned() else {
                return res;
            };
            let hay_proximo = self.proximo.is_some_and(|p| self.item(p).is_some());
            // Con transición, el siguiente empieza un poco antes del final.
            let efectiva = if hay_proximo { item.duracion.saturating_sub(self.adelanto) } else { item.duracion };
            if item.duracion.is_zero() || ahora.saturating_duration_since(inicio) < efectiva {
                return res;
            }
            let fin = inicio + efectiva;
            if hay_proximo {
                let osc_fiable =
                    self.osc_con_archivo.is_some_and(|t| ahora.saturating_duration_since(t) < Duration::from_secs(1));
                if osc_fiable && ahora < fin + ESPERA_OSC {
                    return res; // CasparCG avisará cuando entre el siguiente
                }
                self.espera_precarga = Some(fin + MARGEN_PRECARGA);
                self.avanzar(fin, &mut res);
            } else {
                // Terminó la tanda: CasparCG ya pasó al transparente; se limpia la capa
                // con un pequeño margen para no cortar el final del último spot.
                if ahora < fin + MARGEN_FIN {
                    return res;
                }
                self.forzar_detenido();
                res.termino = true;
                res.acciones.push(Accion::Limpiar);
                return res;
            }
        }
    }

    // ---------- Interno ----------

    fn nuevo_id(&mut self) -> u64 {
        let id = self.siguiente_id;
        self.siguiente_id += 1;
        id
    }

    fn calcular_siguiente(&mut self) -> Option<u64> {
        let i = self.actual.and_then(|a| self.indice(a))?;
        let n = self.lista.len();
        if self.aleatorio && n > 1 {
            let mut j = self.azar.usize(..n - 1);
            if j >= i {
                j += 1;
            }
            return Some(self.lista[j].id);
        }
        if i + 1 < n {
            Some(self.lista[i + 1].id)
        } else if self.bucle == Bucle::Lista && n > 0 {
            Some(self.lista[0].id)
        } else {
            None
        }
    }

    /// Decide qué debe estar precargado detrás del spot actual y genera la acción
    /// solo si cambió.
    fn recalcular_precarga(&mut self) -> Vec<Accion> {
        if self.fase == Fase::Detenido || self.bucle == Bucle::Uno {
            self.proximo = None;
            return Vec::new();
        }
        // En aleatorio se conserva el próximo ya sorteado mientras siga en la lista.
        let conservar =
            self.aleatorio && self.proximo.is_some_and(|p| self.item(p).is_some() && Some(p) != self.actual);
        let prox = if conservar { self.proximo } else { self.calcular_siguiente() };
        self.proximo = prox;
        if self.espera_precarga.is_some() {
            return Vec::new(); // se envía cuando pase la espera (ver `tick`)
        }
        self.aplicar_precarga_de(prox)
    }

    fn aplicar_precarga_de(&mut self, prox: Option<u64>) -> Vec<Accion> {
        let deseada = match prox {
            Some(id) => Precarga::Clip(id),
            None => Precarga::FinATv,
        };
        if deseada == self.precarga {
            return Vec::new();
        }
        self.precarga = deseada.clone();
        let clip = match deseada {
            Precarga::Clip(id) => self.item(id).map(|i| i.clip.clone()).unwrap_or_default(),
            _ => TRANSPARENTE.to_string(),
        };
        vec![Accion::Precargar { clip }]
    }
}

fn distancia(a: Duration, b: Duration) -> Duration {
    a.abs_diff(b)
}

#[derive(Debug, Default)]
pub struct TickResultado {
    pub acciones: Vec<Accion>,
    /// Spots que empezaron en este tick.
    pub empezo: Vec<u64>,
    /// La tanda terminó y se volvió a TV.
    pub termino: bool,
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    fn play(clip: &str, bucle: bool) -> Accion {
        Accion::Play { clip: clip.into(), bucle, desde: Duration::ZERO }
    }
    fn pre(clip: &str) -> Accion {
        Accion::Precargar { clip: clip.into() }
    }

    /// Tanda A(30s), B(20s), C(10s).
    fn tanda() -> (Reproductor, [u64; 3]) {
        let mut r = Reproductor::con_semilla(7);
        let a = r.agregar("A", 30 * S).0;
        let b = r.agregar("B", 20 * S).0;
        let c = r.agregar("C", 10 * S).0;
        (r, [a, b, c])
    }

    #[test]
    fn saltar_dentro_del_spot() {
        let (mut r, [_, b, _]) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        // Adelantar a los 25 s: el reloj sigue desde ahí y el paso a B llega antes.
        assert_eq!(r.buscar(25 * S, t0 + 2 * S), vec![Accion::Buscar(25 * S)]);
        assert_eq!(r.transcurrido(t0 + 3 * S), 26 * S);
        assert_eq!(r.tick(t0 + 7 * S).empezo, vec![b]);
        // No se pasa del final.
        let (mut r, _) = tanda();
        r.play_pausa(None, t0);
        assert_eq!(r.buscar(99 * S, t0), vec![Accion::Buscar(30 * S - FINAL_MINIMO)]);
        // En pausa se mueve el punto sin reanudar.
        r.pausar(t0 + S);
        assert_eq!(r.buscar(4 * S, t0 + 2 * S), vec![Accion::Buscar(4 * S)]);
        assert_eq!(r.transcurrido(t0 + 50 * S), 4 * S);
        // Detenido no hace nada.
        let (mut r, _) = tanda();
        assert!(r.buscar(4 * S, t0).is_empty());
    }

    #[test]
    fn tras_un_salto_no_se_usa_el_tiempo_viejo_de_casparcg() {
        let (mut r, _) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        r.buscar(20 * S, t0 + 5 * S);
        // CasparCG todavía informa el tiempo de antes del salto: se ignora.
        r.sincronizar(t0 + 5 * S + Duration::from_millis(100), 5 * S, 30 * S, Some("A.mp4"));
        assert_eq!(r.transcurrido(t0 + 6 * S), 21 * S);
        // Pasado el momento, vuelve a corregir.
        r.sincronizar(t0 + 7 * S, 21 * S, 30 * S, Some("A.mp4"));
        assert_eq!(r.transcurrido(t0 + 7 * S), 21 * S);
    }

    #[test]
    fn poner_otro_spot_desde_un_punto() {
        let (mut r, [a, b, _]) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        let acc = r.reproducir_desde(b, 15 * S, t0 + S);
        assert_eq!(acc[0], Accion::Play { clip: "B".into(), bucle: false, desde: 15 * S });
        assert_eq!(r.actual(), Some(b));
        assert_eq!(r.transcurrido(t0 + 2 * S), 16 * S);
        // El mismo spot: solo salta.
        r.reproducir_desde(a, Duration::ZERO, t0 + 3 * S);
        assert_eq!(r.reproducir_desde(a, 10 * S, t0 + 4 * S), vec![Accion::Buscar(10 * S)]);
    }

    #[test]
    fn mover_quitar_y_duplicar_varios() {
        let mut r = Reproductor::new();
        let ids: Vec<u64> = ["A", "B", "C", "D", "E"].iter().map(|c| r.agregar(c, S).0).collect();
        let clips = |r: &Reproductor| r.lista().iter().map(|i| i.clip.clone()).collect::<Vec<_>>().join("");
        // B y D al principio, juntos y en su orden.
        r.mover_a(&[ids[3], ids[1]], 0);
        assert_eq!(clips(&r), "BDACE");
        // Al final.
        r.mover_a(&[ids[1], ids[3]], 5);
        assert_eq!(clips(&r), "ACEBD");
        // Antes de la E (posición 2 en ACEBD): A, C y B, D antes de E.
        r.mover_a(&[ids[1], ids[3]], 2);
        assert_eq!(clips(&r), "ACBDE");
        r.duplicar(&[ids[0], ids[2]]);
        assert_eq!(clips(&r), "ACACBDE");
        assert!(r.lista()[2].pase.is_none());
        let (n, _) = r.quitar_varios(&[ids[0], ids[4]]);
        assert_eq!(n, 2);
        assert_eq!(clips(&r), "CACBD");
    }

    #[test]
    fn editar_detenido_no_envia_nada() {
        let mut r = Reproductor::new();
        assert!(r.agregar("A", S).1.is_empty());
        assert_eq!(r.duracion_total(), S);
    }

    #[test]
    fn play_precarga_el_siguiente() {
        let (mut r, [a, b, _]) = tanda();
        let t0 = Instant::now();
        assert_eq!(r.play_pausa(None, t0), vec![play("A", false), pre("B")]);
        assert_eq!(r.actual(), Some(a));
        assert_eq!(r.proximo(), Some(b));
    }

    #[test]
    fn la_tanda_avanza_sola_y_vuelve_a_tv() {
        let (mut r, [_, b, c]) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);

        let res = r.tick(t0 + 29 * S);
        assert!(res.acciones.is_empty());

        let res = r.tick(t0 + 30 * S);
        assert_eq!(res.empezo, vec![b]);
        // Sin confirmación de CasparCG, el siguiente LOADBG espera un margen (si el
        // reloj local iba adelantado, pisaría a B, que CasparCG aún tiene en espera).
        assert!(res.acciones.is_empty());
        assert_eq!(r.tick(t0 + 31 * S).acciones, vec![]);
        assert_eq!(r.tick(t0 + 32 * S).acciones, vec![pre("C")]);

        let res = r.tick(t0 + 50 * S);
        assert_eq!(res.empezo, vec![c]);
        // Tras el último se precarga el transparente para volver a TV sin corte.
        assert_eq!(r.tick(t0 + 52 * S).acciones, vec![pre(TRANSPARENTE)]);

        // La capa se limpia con un pequeño margen tras el fin.
        assert!(!r.tick(t0 + 60 * S).termino);
        let res = r.tick(t0 + 61 * S);
        assert!(res.termino);
        assert_eq!(res.acciones, vec![Accion::Limpiar]);
        assert_eq!(r.fase(), Fase::Detenido);
    }

    #[test]
    fn varios_spots_cortos_en_un_solo_tick() {
        let (mut r, [_, _, c]) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        let res = r.tick(t0 + 55 * S);
        assert_eq!(res.empezo.len(), 2);
        assert_eq!(r.actual(), Some(c));
        assert_eq!(r.transcurrido(t0 + 55 * S), 5 * S);
    }

    #[test]
    fn pausa_detiene_el_reloj() {
        let (mut r, _) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        assert_eq!(r.play_pausa(None, t0 + 10 * S), vec![Accion::Pausa]);
        assert!(r.tick(t0 + 100 * S).acciones.is_empty());
        assert_eq!(r.transcurrido(t0 + 100 * S), 10 * S);
        assert_eq!(r.play_pausa(None, t0 + 100 * S), vec![Accion::Reanudar]);
        assert_eq!(r.transcurrido(t0 + 105 * S), 15 * S);
    }

    #[test]
    fn bucle_de_lista_vuelve_al_primero() {
        let (mut r, [a, _, c]) = tanda();
        r.cambiar_bucle(Bucle::Lista, Instant::now());
        let t0 = Instant::now();
        r.reproducir(c, t0);
        assert_eq!(r.proximo(), Some(a));
        let res = r.tick(t0 + 10 * S);
        assert_eq!(res.empezo, vec![a]);
        assert!(!res.termino);
    }

    #[test]
    fn bucle_de_uno_no_avanza() {
        let (mut r, [a, _, _]) = tanda();
        r.cambiar_bucle(Bucle::Uno, Instant::now());
        let t0 = Instant::now();
        assert_eq!(r.reproducir(a, t0), vec![play("A", true)]);
        assert!(r.tick(t0 + 300 * S).acciones.is_empty());
        assert_eq!(r.actual(), Some(a));
        // 95 s dentro de un spot de 30 s = 5 s de la vuelta actual.
        assert_eq!(r.transcurrido(t0 + 95 * S), 5 * S);
    }

    #[test]
    fn activar_bucle_al_aire_avisa_a_casparcg() {
        let (mut r, _) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        assert_eq!(r.cambiar_bucle(Bucle::Uno, t0 + 5 * S), vec![Accion::BucleActual(true)]);
        let acc = r.cambiar_bucle(Bucle::No, t0 + 65 * S);
        assert_eq!(acc[0], Accion::BucleActual(false));
        // Siguió desde el segundo 5 de la vuelta: faltan 25 s.
        assert_eq!(r.transcurrido(t0 + 65 * S), 5 * S);
        assert!(r.tick(t0 + 89 * S).empezo.is_empty());
        assert_eq!(r.tick(t0 + 90 * S).empezo.len(), 1);
    }

    #[test]
    fn siguiente_y_anterior() {
        let (mut r, [a, b, c]) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        r.siguiente(t0);
        assert_eq!(r.actual(), Some(b));
        r.siguiente(t0);
        assert_eq!(r.actual(), Some(c));
        assert_eq!(r.siguiente(t0), vec![Accion::Limpiar]);
        assert_eq!(r.fase(), Fase::Detenido);
        r.reproducir(b, t0);
        r.anterior(t0);
        assert_eq!(r.actual(), Some(a));
        r.anterior(t0);
        assert_eq!(r.actual(), Some(a));
    }

    #[test]
    fn reordenar_cambia_la_precarga() {
        let (mut r, [_, _, c]) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        // Subir C una posición: queda A, C, B → el siguiente pasa a ser C.
        assert_eq!(r.mover(c, -1), vec![pre("C")]);
        assert_eq!(r.proximo(), Some(c));
    }

    #[test]
    fn quitar_el_siguiente_y_no_el_actual() {
        let (mut r, [a, b, c]) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        assert!(!r.quitar(a).0);
        let (ok, acc) = r.quitar(b);
        assert!(ok);
        assert_eq!(acc, vec![pre("C")]);
        assert_eq!(r.proximo(), Some(c));
    }

    #[test]
    fn agregar_al_final_tras_el_ultimo_cambia_transparente_por_clip() {
        let (mut r, [_, _, c]) = tanda();
        let t0 = Instant::now();
        r.reproducir(c, t0);
        assert_eq!(r.proximo(), None);
        let (d, acc) = r.agregar("D", 5 * S);
        assert_eq!(acc, vec![pre("D")]);
        assert_eq!(r.proximo(), Some(d));
    }

    #[test]
    fn insertar_en_una_posicion() {
        let (mut r, [a, b, c]) = tanda();
        let t0 = Instant::now();
        r.reproducir(a, t0);
        // Entre A (al aire) y B: pasa a ser el siguiente.
        let (x, acc) = r.insertar_en(1, "X", 5 * S);
        assert_eq!(acc, vec![pre("X")]);
        assert_eq!(r.lista().iter().map(|i| i.id).collect::<Vec<_>>(), vec![a, x, b, c]);
        // Más allá del final: al final.
        let (y, _) = r.insertar_en(99, "Y", 5 * S);
        assert_eq!(r.lista().last().map(|i| i.id), Some(y));
        // Arrastrar C arriba de X (de la posición 3 a la 1).
        r.mover(c, -2);
        assert_eq!(r.lista().iter().map(|i| i.id).collect::<Vec<_>>(), vec![a, c, x, b, y]);
        assert_eq!(r.proximo(), Some(c));
    }

    #[test]
    fn insertar_siguiente_respeta_el_orden() {
        let (mut r, [a, b, _]) = tanda();
        let t0 = Instant::now();
        r.reproducir(a, t0);
        let (x, acc) = r.insertar_siguiente("X", 5 * S);
        assert_eq!(acc, vec![pre("X")]);
        assert_eq!(r.lista()[1].id, x);
        assert_eq!(r.lista()[2].id, b);
    }

    #[test]
    fn aleatorio_no_repite_el_actual() {
        let (mut r, [a, _, _]) = tanda();
        r.cambiar_aleatorio(true);
        let t0 = Instant::now();
        r.reproducir(a, t0);
        for _ in 0..50 {
            let actual = r.actual();
            r.siguiente(t0);
            assert_ne!(r.actual(), actual);
        }
    }

    #[test]
    fn volver_a_tv_limpia_todo() {
        let (mut r, _) = tanda();
        r.play_pausa(None, Instant::now());
        assert_eq!(r.volver_a_tv(), vec![Accion::Limpiar]);
        assert_eq!(r.actual(), None);
        assert_eq!(r.fase(), Fase::Detenido);
    }

    #[test]
    fn limpiar_lista_conserva_lo_que_esta_al_aire() {
        let (mut r, [a, _, _]) = tanda();
        r.play_pausa(None, Instant::now());
        r.limpiar_lista();
        assert_eq!(r.lista().len(), 1);
        assert_eq!(r.lista()[0].id, a);
        assert_eq!(r.proximo(), None);
    }

    fn osc(r: &mut Reproductor, t: Instant, trans: Duration, total: Duration, archivo: Option<&str>) -> TickResultado {
        r.sincronizar(t, trans, total, archivo)
    }

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn osc_corrige_el_reloj() {
        let (mut r, [_, b, _]) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        // CasparCG dice que van 10,5 s aunque el reloj local diga 10 s.
        osc(&mut r, t0 + 10 * S, 10 * S + 500 * MS, 30 * S, None);
        assert_eq!(r.transcurrido(t0 + 10 * S), 10 * S + 500 * MS);
        // Por eso el cambio al siguiente llega medio segundo antes.
        assert_eq!(r.tick(t0 + 29 * S + 500 * MS).empezo, vec![b]);
        // Un salto grande sin saber el archivo no se acepta.
        osc(&mut r, t0 + 31 * S, 15 * S, 20 * S, None);
        assert!(r.transcurrido(t0 + 31 * S) < 2 * S);
    }

    #[test]
    fn osc_de_otro_clip_se_ignora() {
        let (mut r, _) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        osc(&mut r, t0 + 10 * S, 3 * S, 20 * S, None);
        assert_eq!(r.transcurrido(t0 + 10 * S), 10 * S);
        // Con el archivo: si no es el actual ni el siguiente, se ignora.
        osc(&mut r, t0 + 10 * S, 3 * S, 30 * S, Some("D:/media/OTRO.mp4"));
        assert_eq!(r.transcurrido(t0 + 10 * S), 10 * S);
    }

    #[test]
    fn osc_completa_duracion_desconocida() {
        let mut r = Reproductor::new();
        let (a, _) = r.agregar("A", Duration::ZERO);
        let t0 = Instant::now();
        r.reproducir(a, t0);
        osc(&mut r, t0 + S, S, 15 * S, None);
        assert_eq!(r.item(a).unwrap().duracion, 15 * S);
    }

    /// El error de "a veces no sale el segundo": el reloj local iba adelantado y Relevo
    /// daba por terminado el primero antes que CasparCG, y su LOADBG del que seguía
    /// reemplazaba al segundo, que CasparCG tenía en espera.
    #[test]
    fn con_el_archivo_el_cambio_lo_decide_casparcg() {
        let mut r = Reproductor::new();
        let a = r.agregar("PROMOS/UNO", 10 * S).0;
        let b = r.agregar("PROMOS/DOS", 10 * S).0;
        let t0 = Instant::now();
        let acc = r.reproducir(a, t0);
        assert_eq!(acc, vec![play("PROMOS/UNO", false), pre("PROMOS/DOS")]);
        // CasparCG empezó 0,3 s después (cargando el archivo).
        osc(&mut r, t0 + S, 700 * MS, 10 * S, Some("D:/caspar/media/PROMOS/uno.mp4"));
        assert_eq!(r.transcurrido(t0 + S), 700 * MS);
        // A los 10 s locales, CasparCG aún está en el primero: no se pasa ni se precarga nada.
        osc(&mut r, t0 + 10 * S, 9 * S + 700 * MS, 10 * S, Some("D:/caspar/media/PROMOS/uno.mp4"));
        let res = r.tick(t0 + 10 * S + 100 * MS);
        assert!(res.empezo.is_empty() && res.acciones.is_empty(), "{res:?}");
        assert_eq!(r.actual(), Some(a));
        // CasparCG pasa al segundo: recién ahí se cambia y se prepara la vuelta a TV.
        let res = osc(&mut r, t0 + 10 * S + 340 * MS, 40 * MS, 10 * S, Some("D:/caspar/media/PROMOS/dos.mp4"));
        assert_eq!(res.empezo, vec![b]);
        assert_eq!(res.acciones, vec![pre(TRANSPARENTE)]);
        assert_eq!(r.transcurrido(t0 + 10 * S + 340 * MS), 40 * MS);
        // Un tiempo atrasado del primero (mismo largo) ya no mueve el reloj del segundo.
        osc(&mut r, t0 + 10 * S + 360 * MS, 9 * S + 990 * MS, 10 * S, Some("D:/caspar/media/PROMOS/uno.mp4"));
        assert_eq!(r.transcurrido(t0 + 10 * S + 360 * MS), 60 * MS);
        // Fin de la tanda: se limpia la capa.
        assert!(r.tick(t0 + 20 * S + 800 * MS).termino);
    }

    #[test]
    fn sin_el_archivo_un_tiempo_del_anterior_no_rompe_el_siguiente() {
        let mut r = Reproductor::new();
        let a = r.agregar("UNO", 10 * S).0;
        let b = r.agregar("DOS", 10 * S).0;
        let t0 = Instant::now();
        r.reproducir(a, t0);
        assert_eq!(r.tick(t0 + 10 * S).empezo, vec![b]);
        // Llega tarde el tiempo del primero (9,9 de 10 s): no debe tomarse como del segundo.
        osc(&mut r, t0 + 10 * S + 50 * MS, 9 * S + 900 * MS, 10 * S, None);
        assert_eq!(r.transcurrido(t0 + 10 * S + 50 * MS), 50 * MS);
        assert!(!r.tick(t0 + 10 * S + 100 * MS).termino);
    }

    #[test]
    fn si_casparcg_no_confirma_se_pasa_por_reloj() {
        let mut r = Reproductor::new();
        let a = r.agregar("UNO", 10 * S).0;
        let b = r.agregar("DOS", 10 * S).0;
        let t0 = Instant::now();
        r.reproducir(a, t0);
        osc(&mut r, t0 + 10 * S, 9 * S + 990 * MS, 10 * S, Some("uno.mp4"));
        // Se espera la confirmación de CasparCG…
        assert!(r.tick(t0 + 10 * S + 900 * MS).empezo.is_empty());
        // …pero si OSC deja de llegar, se pasa igual.
        assert_eq!(r.tick(t0 + 11 * S + 600 * MS).empezo, vec![b]);
    }

    #[test]
    fn mismo_clip_dos_veces_seguidas() {
        let mut r = Reproductor::new();
        let a = r.agregar("UNO", 10 * S).0;
        let b = r.agregar("UNO", 10 * S).0;
        let t0 = Instant::now();
        r.reproducir(a, t0);
        osc(&mut r, t0 + 9 * S + 900 * MS, 9 * S + 900 * MS, 10 * S, Some("uno.mov"));
        assert_eq!(r.actual(), Some(a));
        // El tiempo vuelve a empezar: entró el segundo.
        let res = osc(&mut r, t0 + 10 * S + 50 * MS, 30 * MS, 10 * S, Some("uno.mov"));
        assert_eq!(res.empezo, vec![b]);
    }

    /// El clip dura en CasparCG 0,3 s más que lo que dice su archivo: sin la ruta por
    /// OSC, Relevo adopta la duración real y no da el spot por terminado antes de tiempo.
    #[test]
    fn adopta_la_duracion_de_casparcg() {
        let mut r = Reproductor::new();
        let a = r.agregar("UNO", 10 * S).0;
        let b = r.agregar("DOS", 10 * S).0;
        let t0 = Instant::now();
        r.reproducir(a, t0);
        osc(&mut r, t0 + 5 * S, 5 * S, 10 * S + 300 * MS, None);
        assert_eq!(r.item(a).unwrap().duracion, 10 * S + 300 * MS);
        assert!(r.tick(t0 + 10 * S + 100 * MS).empezo.is_empty());
        assert_eq!(r.tick(t0 + 10 * S + 300 * MS).empezo, vec![b]);
    }

    #[test]
    fn con_fundido_el_siguiente_empieza_antes() {
        let mut r = Reproductor::new();
        let a = r.agregar("UNO", 10 * S).0;
        let b = r.agregar("DOS", 10 * S).0;
        r.poner_adelanto(500 * MS);
        let t0 = Instant::now();
        r.reproducir(a, t0);
        assert!(r.tick(t0 + 9 * S + 400 * MS).empezo.is_empty());
        assert_eq!(r.tick(t0 + 9 * S + 500 * MS).empezo, vec![b]);
        // El último dura completo (el fundido a TV termina con él).
        assert!(!r.tick(t0 + 19 * S + 600 * MS).termino);
        assert!(r.tick(t0 + 20 * S).termino);
    }

    #[test]
    fn nombres_de_clip() {
        assert!(mismo_clip("D:/caspar/media/PROMOS/volvemos en breve.mp4", "PROMOS/VOLVEMOS EN BREVE"));
        assert!(mismo_clip("D:\\Videos\\spot.mov", "D:/Videos/spot.mov"));
        assert!(mismo_clip("media/spot.v2.mp4", "SPOT.V2"));
        assert!(!mismo_clip("media/spot.mp4", "OTRO"));
        assert!(!mismo_clip("", "X"));
    }

    #[test]
    fn reemplazar_conserva_lo_que_esta_al_aire() {
        let (mut r, [a, _, _]) = tanda();
        r.play_pausa(None, Instant::now());
        let acc = r.reemplazar(&[("X".into(), 5 * S, None), ("Y".into(), 5 * S, Some("p".into()))]);
        assert_eq!(r.lista().len(), 3);
        assert_eq!(r.lista()[0].id, a);
        assert_eq!(acc, vec![pre("X")]);
    }

    #[test]
    fn play_con_seleccion_empieza_desde_ahi() {
        let (mut r, [_, b, _]) = tanda();
        assert_eq!(r.play_pausa(Some(b), Instant::now())[0], play("B", false));
    }

    #[test]
    fn detener_a_mitad_y_retomar_ese_spot() {
        let (mut r, [a, b, c]) = tanda();
        let t0 = Instant::now();
        r.play_pausa(None, t0);
        r.reproducir(b, t0 + 5 * S);
        // ⏹ con B al aire: vuelve a TV, pero recuerda B.
        assert_eq!(r.volver_a_tv(), vec![Accion::Limpiar]);
        assert_eq!(r.retomar(), Some(b));
        // Play (aunque haya otro seleccionado) vuelve a empezar B, y sigue con C.
        assert_eq!(r.play_pausa(Some(a), t0 + 10 * S), vec![play("B", false), pre("C")]);
        assert_eq!(r.retomar(), None);
        // Con la tanda detenida, ⏭ y ⏮ mueven el punto donde seguirá.
        r.volver_a_tv();
        r.siguiente(t0 + 11 * S);
        assert_eq!(r.retomar(), Some(c));
        r.anterior(t0 + 11 * S);
        r.anterior(t0 + 11 * S);
        assert_eq!(r.retomar(), Some(a));
        // Quitar el spot donde quedó: sigue desde el que ocupa su lugar.
        r.quitar(a);
        assert_eq!(r.retomar(), Some(b));
        // Al terminar la tanda sola, no queda nada pendiente.
        r.play_pausa(None, t0 + 20 * S);
        r.reproducir(c, t0 + 20 * S);
        let res = r.tick(t0 + 31 * S);
        assert!(res.termino);
        assert_eq!(r.retomar(), None);
        // Volver a empezar desde el principio.
        r.play_pausa(None, t0 + 40 * S);
        r.volver_a_tv();
        r.poner_retomar(None);
        assert_eq!(r.play_pausa(None, t0 + 50 * S)[0], play("B", false));
    }
}
