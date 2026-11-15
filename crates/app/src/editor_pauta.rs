//! El formulario de una publicidad (pauta): qué spot sale, de qué cliente, qué días y a
//! qué horas. Lo usa la pantalla Publicidades.

use nucleo::pauta::{formato, Dias, Pauta};
use time::{Date, Time};

/// Formulario con los campos en texto, tal como los escribe el operador.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Borrador {
    pub id: u64,
    pub cliente: String,
    pub nombre: String,
    pub clip: String,
    pub duracion_seg: f64,
    pub inicio: String,
    pub sin_fin: bool,
    pub fin: String,
    pub tipo_dias: u8,
    pub semana: [bool; 7],
    pub cada_n: u32,
    pub fechas: String,
    pub horas: Vec<Time>,
    pub nueva_hora: String,
    /// Minutos antes de cada hora en que ya puede salir.
    pub tolerancia: u32,
    /// Minutos después de cada hora en que todavía puede salir.
    pub despues: u32,
    pub prioridad: u8,
    pub activa: bool,
    pub notas: String,
}

impl Borrador {
    pub fn nuevo(hoy: Date, tolerancia: u32) -> Self {
        Borrador {
            id: 0,
            cliente: String::new(),
            nombre: String::new(),
            clip: String::new(),
            duracion_seg: 0.0,
            inicio: formato::texto_fecha_dma(hoy),
            sin_fin: true,
            fin: String::new(),
            tipo_dias: 0,
            semana: [true, true, true, true, true, false, false],
            cada_n: 2,
            fechas: String::new(),
            horas: Vec::new(),
            nueva_hora: String::new(),
            tolerancia,
            despues: tolerancia,
            prioridad: 0,
            activa: true,
            notas: String::new(),
        }
    }

    pub fn desde(p: &Pauta) -> Self {
        let (tipo_dias, semana, cada_n, fechas) = match &p.dias {
            Dias::Todos => (0, [true; 7], 2, String::new()),
            Dias::Semana { dias } => (1, *dias, 2, String::new()),
            Dias::CadaN { n } => (2, [true; 7], *n, String::new()),
            Dias::Fechas { fechas } => {
                (3, [true; 7], 2, fechas.iter().map(|f| formato::texto_fecha_dma(*f)).collect::<Vec<_>>().join(", "))
            }
        };
        Borrador {
            id: p.id,
            cliente: p.cliente.clone(),
            nombre: p.nombre.clone(),
            clip: p.clip.clone(),
            duracion_seg: p.duracion_seg,
            inicio: formato::texto_fecha_dma(p.inicio),
            sin_fin: p.fin.is_none(),
            fin: p.fin.map(formato::texto_fecha_dma).unwrap_or_default(),
            tipo_dias,
            semana,
            cada_n,
            fechas,
            horas: p.horas.clone(),
            nueva_hora: String::new(),
            tolerancia: p.tolerancia_min,
            despues: p.despues(),
            prioridad: p.prioridad,
            activa: p.activa,
            notas: p.notas.clone(),
        }
    }

    /// Convierte el formulario en una pauta, o explica qué falta.
    pub fn validar(&self) -> Result<Pauta, String> {
        if self.clip.is_empty() {
            return Err("Elige el video del spot.".into());
        }
        let inicio = formato::leer_fecha_dma(&self.inicio).ok_or("La fecha de inicio debe ser DD/MM/AAAA.")?;
        let fin = if self.sin_fin {
            None
        } else {
            let f = formato::leer_fecha_dma(&self.fin).ok_or("La fecha de fin debe ser DD/MM/AAAA.")?;
            if f < inicio {
                return Err("La fecha de fin es anterior a la de inicio.".into());
            }
            Some(f)
        };
        let dias = match self.tipo_dias {
            0 => Dias::Todos,
            1 => {
                if !self.semana.iter().any(|d| *d) {
                    return Err("Marca al menos un día de la semana.".into());
                }
                Dias::Semana { dias: self.semana }
            }
            2 => Dias::CadaN { n: self.cada_n.max(1) },
            _ => {
                let mut fechas = Vec::new();
                for t in self.fechas.split([',', ';', ' ', '\n']).filter(|t| !t.trim().is_empty()) {
                    fechas.push(formato::leer_fecha_dma(t).ok_or(format!("Fecha no válida: {t}"))?);
                }
                if fechas.is_empty() {
                    return Err("Escribe al menos una fecha.".into());
                }
                fechas.sort();
                fechas.dedup();
                Dias::Fechas { fechas }
            }
        };
        if self.horas.is_empty() {
            return Err("Añade al menos una hora.".into());
        }
        let nombre = if self.nombre.trim().is_empty() {
            self.clip.rsplit('/').next().unwrap_or(&self.clip).to_string()
        } else {
            self.nombre.trim().to_string()
        };
        Ok(Pauta {
            id: self.id,
            nombre,
            clip: self.clip.clone(),
            duracion_seg: self.duracion_seg,
            inicio,
            fin,
            dias,
            horas: self.horas.clone(),
            tolerancia_min: self.tolerancia,
            despues_min: (self.despues != self.tolerancia).then_some(self.despues),
            prioridad: self.prioridad,
            activa: self.activa,
            cliente: self.cliente.trim().to_string(),
            notas: self.notas.trim().to_string(),
        })
    }

    /// Añade las horas escritas en `nueva_hora` (se pueden pegar muchas juntas, en
    /// 24 h o con AM/PM: «6:00, 8:30, 10:45 ... 8:00PM, 10pm»).
    pub fn agregar_horas(&mut self) -> Result<usize, String> {
        let nuevas = formato::leer_horas(&self.nueva_hora)
            .map_err(|t| format!("No entendí «{t}». Escribe la hora como 08:30, 8:30 pm o 20:30."))?;
        let mut n = 0;
        for h in nuevas {
            if !self.horas.contains(&h) {
                self.horas.push(h);
                n += 1;
            }
        }
        self.horas.sort();
        self.nueva_hora.clear();
        Ok(n)
    }

    /// Añade una hora (si no estaba).
    pub fn poner_hora(&mut self, h: Time) {
        if !self.horas.contains(&h) {
            self.horas.push(h);
            self.horas.sort();
        }
    }

    /// Añade horas cada `cada_min` minutos entre `desde` y `hasta` (incluidas).
    pub fn generar_horas(&mut self, desde: &str, hasta: &str, cada_min: u32) -> Result<usize, String> {
        let d = formato::leer_hora(desde).ok_or(format!("Hora no válida: {desde} (usa 08:00 o 8 am)"))?;
        let h = formato::leer_hora(hasta).ok_or(format!("Hora no válida: {hasta} (usa 20:00 o 8 pm)"))?;
        if cada_min == 0 {
            return Err("El intervalo debe ser mayor que cero.".into());
        }
        let (a, b) = (d.hour() as u32 * 60 + d.minute() as u32, h.hour() as u32 * 60 + h.minute() as u32);
        if b < a {
            return Err("La hora final es anterior a la inicial.".into());
        }
        let mut n = 0;
        let mut m = a;
        while m <= b {
            let t = Time::from_hms((m / 60) as u8, (m % 60) as u8, 0).map_err(|e| e.to_string())?;
            if !self.horas.contains(&t) {
                self.horas.push(t);
                n += 1;
            }
            m += cada_min;
        }
        self.horas.sort();
        Ok(n)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use time::macros::{date, time};

    #[test]
    fn validar_borrador() {
        let mut b = Borrador::nuevo(date!(2026 - 11 - 11), 15);
        assert!(b.validar().is_err()); // sin video
        b.clip = "CLIENTES/FERRETERIA X/SPOT 30S".into();
        b.duracion_seg = 30.0;
        assert_eq!(b.validar().unwrap_err(), "Añade al menos una hora.");
        b.nueva_hora = "10:30, 8".into();
        b.agregar_horas().unwrap();
        assert_eq!(b.horas, vec![time!(8:00), time!(10:30)]);
        let p = b.validar().unwrap();
        assert_eq!(p.nombre, "SPOT 30S");
        assert_eq!(p.inicio, date!(2026 - 11 - 11));
        assert!(p.fin.is_none());

        b.sin_fin = false;
        b.fin = "01/11/2026".into();
        assert!(b.validar().is_err()); // fin antes del inicio

        b.sin_fin = true;
        b.tipo_dias = 1;
        b.semana = [false; 7];
        assert!(b.validar().is_err());

        // Ida y vuelta: pauta → borrador → pauta.
        b.tipo_dias = 2;
        b.cada_n = 3;
        let p = b.validar().unwrap();
        assert_eq!(Borrador::desde(&p).validar().unwrap(), p);
    }

    #[test]
    fn generar_horas_cada_intervalo() {
        let mut b = Borrador::nuevo(date!(2026 - 11 - 11), 15);
        assert_eq!(b.generar_horas("08:00", "12:00", 120).unwrap(), 3);
        assert_eq!(b.horas, vec![time!(8:00), time!(10:00), time!(12:00)]);
        // No repite las que ya estaban.
        assert_eq!(b.generar_horas("10:00", "11:00", 30).unwrap(), 2);
        assert_eq!(b.horas.len(), 5);
        assert!(b.generar_horas("12:00", "08:00", 60).is_err());
        // Cliente y notas van a la pauta.
        b.clip = "X".into();
        b.cliente = " Ferretería X ".into();
        b.notas = "Contrato hasta diciembre".into();
        let p = b.validar().unwrap();
        assert_eq!(p.cliente, "Ferretería X");
        assert_eq!(Borrador::desde(&p).validar().unwrap(), p);
    }

    #[test]
    fn hora_mal_escrita() {
        let mut b = Borrador::nuevo(date!(2026 - 11 - 11), 15);
        b.nueva_hora = "25:99".into();
        assert!(b.agregar_horas().is_err());
        b.nueva_hora = "6:00, 8:30 ... 8:00PM, 10:pm".into();
        assert_eq!(b.agregar_horas().unwrap(), 4);
        assert_eq!(b.horas, vec![time!(6:00), time!(8:30), time!(20:00), time!(22:00)]);
        // Antes y después distintos van a la pauta y vuelven.
        b.clip = "X".into();
        b.tolerancia = 5;
        b.despues = 40;
        let p = b.validar().unwrap();
        assert_eq!((p.tolerancia_min, p.despues()), (5, 40));
        assert_eq!(Borrador::desde(&p).validar().unwrap(), p);
    }
}
