//! Registro de emisión y reportes.
//!
//! Cada spot que sale al aire queda anotado en `datos/registro/AAAA-MM-DD.jsonl`
//! (una línea por salida): a qué hora, cuánto se emitió de verdad, si salió completo
//! o se cortó, si venía de la pauta y quién lo lanzó. Con eso se arman los reportes
//! por cliente (constancia de emisión) y el cumplimiento de la pauta.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::{Date, Duration as Dur, Time};

use crate::pauta::{self, formato, Almacen, Pauta};

/// Días antes del fin de un contrato (pauta) en que se empieza a avisar.
pub const DIAS_AVISO_VENCIMIENTO: i64 = 7;

/// Una salida al aire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Salida {
    /// Hora en que empezó, `HH:MM:SS`.
    pub hora: String,
    pub clip: String,
    /// Cliente (nombre de la pauta o, si no viene de una, el de su carpeta).
    pub cliente: String,
    /// Clave del pase de la pauta que cumplió, si venía de la pauta.
    #[serde(default)]
    pub pase: Option<String>,
    /// Duración del spot (s).
    pub duracion: f64,
    /// Segundos que estuvo al aire.
    pub emitido: f64,
    pub completo: bool,
    /// "AUTO" o "MANUAL".
    pub modo: String,
    /// Quién tenía el mando.
    pub quien: String,
}

/// Una salida con su fecha (para los reportes de varios días).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SalidaFecha {
    /// `AAAA-MM-DD`.
    pub fecha: String,
    #[serde(flatten)]
    pub salida: Salida,
}

/// Resumen de un cliente en el periodo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResumenCliente {
    pub cliente: String,
    pub salidas: u32,
    pub completas: u32,
    pub cortadas: u32,
    /// Tiempo total al aire (s).
    pub segundos: f64,
    /// Pases de la pauta cuya hora ya pasó en el periodo.
    pub programados: u32,
    /// De esos, los que salieron completos.
    pub cumplidos: u32,
    /// Los que no salieron o se cortaron.
    pub perdidos: u32,
}

impl ResumenCliente {
    /// Porcentaje de la pauta cumplida (None si no tenía pauta en el periodo).
    pub fn cumplimiento(&self) -> Option<f64> {
        (self.programados > 0).then(|| self.cumplidos as f64 * 100.0 / self.programados as f64)
    }
}

/// Algo que el operador debería saber de la pauta.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alerta {
    pub texto: String,
    pub grave: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reporte {
    pub desde: String,
    pub hasta: String,
    pub salidas: Vec<SalidaFecha>,
    pub clientes: Vec<ResumenCliente>,
    pub alertas: Vec<Alerta>,
}

impl Reporte {
    /// Solo lo de un cliente.
    pub fn de_cliente(&self, cliente: &str) -> Reporte {
        Reporte {
            desde: self.desde.clone(),
            hasta: self.hasta.clone(),
            salidas: self.salidas.iter().filter(|s| s.salida.cliente == cliente).cloned().collect(),
            clientes: self.clientes.iter().filter(|c| c.cliente == cliente).cloned().collect(),
            alertas: Vec::new(),
        }
    }
}

// ======================================================================
// Registro diario
// ======================================================================

fn ruta_dia(carpeta_datos: &Path, fecha: Date) -> PathBuf {
    carpeta_datos.join("registro").join(format!("{}.jsonl", formato::texto_fecha(fecha)))
}

/// Anota una salida (se agrega al final del archivo del día).
pub fn anotar(carpeta_datos: &Path, fecha: Date, s: &Salida) -> std::io::Result<()> {
    let ruta = ruta_dia(carpeta_datos, fecha);
    if let Some(c) = ruta.parent() {
        std::fs::create_dir_all(c)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(ruta)?;
    let linea = serde_json::to_string(s).map_err(std::io::Error::other)?;
    writeln!(f, "{linea}")
}

/// Salidas de un día (las líneas dañadas se saltan).
pub fn leer_dia(carpeta_datos: &Path, fecha: Date) -> Vec<Salida> {
    std::fs::read_to_string(ruta_dia(carpeta_datos, fecha))
        .map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default()
}

/// Cliente de un spot: el nombre de su pauta; si no tiene, la carpeta que lo contiene
/// (`CLIENTES/FERRETERIA X/SPOT 30S` → `FERRETERIA X`); si no, el nombre del spot.
pub fn cliente_de(clip: &str, pase: Option<&str>, pautas: &[Pauta]) -> String {
    if let Some(id) = pase.and_then(|p| p.split('@').next()).and_then(|i| i.parse::<u64>().ok()) {
        if let Some(p) = pautas.iter().find(|p| p.id == id) {
            return p.cliente_o_nombre().to_string();
        }
    }
    if let Some(p) = pautas.iter().find(|p| p.clip.eq_ignore_ascii_case(clip)) {
        return p.cliente_o_nombre().to_string();
    }
    let limpio = clip.replace('\\', "/");
    let partes: Vec<&str> = limpio.split('/').filter(|p| !p.is_empty()).collect();
    let ultimo = partes.last().copied().unwrap_or(clip);
    let ultimo = ultimo.rsplit_once('.').map(|(a, _)| a).unwrap_or(ultimo);
    if partes.len() >= 3 && !limpio.contains(':') && !limpio.starts_with('/') {
        partes[partes.len() - 2].to_string()
    } else {
        ultimo.to_string()
    }
}

// ======================================================================
// Reportes
// ======================================================================

/// Arma el reporte de `desde` a `hasta` (inclusive). `hoy` y `ahora` sirven para no
/// contar como perdidos los pases que todavía no llegaron.
pub fn calcular(carpeta_datos: &Path, pautas: &[Pauta], desde: Date, hasta: Date, hoy: Date, ahora: Time) -> Reporte {
    let almacen = Almacen::new(carpeta_datos.to_path_buf());
    let mut salidas = Vec::new();
    let mut clientes: BTreeMap<String, ResumenCliente> = BTreeMap::new();
    // La pauta se cuenta desde el primer día con registro (antes Relevo no se usaba).
    let desde_registro = primer_dia(carpeta_datos);
    let mut dia = desde;
    let mut vueltas = 0;
    while dia <= hasta && vueltas < 400 {
        vueltas += 1;
        let fecha = formato::texto_fecha(dia);
        let del_dia = leer_dia(carpeta_datos, dia);
        // Pase → ¿salió completo? (un pase cortado no cuenta como cumplido)
        let completos: std::collections::HashMap<String, bool> = del_dia
            .iter()
            .filter_map(|s| s.pase.clone().map(|p| (p, s.completo)))
            .fold(std::collections::HashMap::new(), |mut m, (p, c)| {
                *m.entry(p).or_insert(false) |= c;
                m
            });
        for s in del_dia {
            let c = clientes
                .entry(s.cliente.clone())
                .or_insert_with(|| ResumenCliente { cliente: s.cliente.clone(), ..Default::default() });
            c.salidas += 1;
            if s.completo {
                c.completas += 1;
            } else {
                c.cortadas += 1;
            }
            c.segundos += s.emitido;
            salidas.push(SalidaFecha { fecha: fecha.clone(), salida: s });
        }
        // Cumplimiento de la pauta: solo días que ya llegaron.
        if dia <= hoy && desde_registro.is_some_and(|p| dia >= p) {
            let emitidos: HashSet<String> = almacen.cargar_emitidos(dia).into_iter().map(|e| e.clave).collect();
            for p in pauta::pases_del_dia(pautas, dia) {
                // Ya tocaba: un día pasado, o hoy si ya salió o se le pasó la hora.
                let vencido = dia < hoy
                    || emitidos.contains(&p.clave)
                    || p.estado(ahora, &emitidos) == pauta::EstadoPase::Perdido;
                let salio = emitidos.contains(&p.clave) && completos.get(&p.clave).copied().unwrap_or(true);
                if !(vencido || salio) {
                    continue;
                }
                let c = clientes
                    .entry(p.nombre.clone())
                    .or_insert_with(|| ResumenCliente { cliente: p.nombre.clone(), ..Default::default() });
                c.programados += 1;
                if salio {
                    c.cumplidos += 1;
                } else {
                    c.perdidos += 1;
                }
            }
        }
        let Some(sig) = dia.next_day() else { break };
        dia = sig;
    }
    Reporte {
        desde: formato::texto_fecha(desde),
        hasta: formato::texto_fecha(hasta),
        salidas,
        clientes: clientes.into_values().collect(),
        alertas: alertas(pautas, None, hoy),
    }
}

/// Primer día con algo anotado (registro de salidas o pases emitidos).
fn primer_dia(carpeta_datos: &Path) -> Option<Date> {
    ["registro", "emitidos"]
        .iter()
        .filter_map(|c| std::fs::read_dir(carpeta_datos.join(c)).ok())
        .flat_map(|l| l.flatten())
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            formato::leer_fecha(n.split('.').next().unwrap_or(""))
        })
        .min()
}

/// Avisos de la pauta: contratos por vencer o vencidos, y spots que no están en CasparCG
/// (`medios`: nombres de los clips, si se conocen).
pub fn alertas(pautas: &[Pauta], medios: Option<&[String]>, hoy: Date) -> Vec<Alerta> {
    let mut v = Vec::new();
    for p in pautas.iter().filter(|p| p.activa) {
        if let Some(m) = medios {
            let directo = p.clip.contains(':') || p.clip.starts_with('/');
            if !directo && !m.iter().any(|c| c.eq_ignore_ascii_case(&p.clip)) {
                v.push(Alerta {
                    texto: format!("Falta el video de «{}» en CasparCG ({})", p.nombre, p.clip),
                    grave: true,
                });
            }
        }
        if let Some(fin) = p.fin {
            let dias = (fin - hoy).whole_days();
            if (0..=DIAS_AVISO_VENCIMIENTO).contains(&dias) {
                let cuando = match dias {
                    0 => "hoy".to_string(),
                    1 => "mañana".to_string(),
                    n => format!("en {n} días ({})", formato::texto_fecha_dma(fin)),
                };
                v.push(Alerta { texto: format!("El contrato de «{}» termina {cuando}", p.nombre), grave: dias <= 1 });
            } else if (-3..0).contains(&dias) {
                v.push(Alerta {
                    texto: format!("El contrato de «{}» terminó el {}", p.nombre, formato::texto_fecha_dma(fin)),
                    grave: false,
                });
            }
        }
    }
    v
}

/// Periodos rápidos para los reportes.
pub fn periodo(nombre: &str, hoy: Date) -> (Date, Date) {
    let lunes = hoy - Dur::days(hoy.weekday().number_days_from_monday() as i64);
    let primero = hoy.replace_day(1).unwrap_or(hoy);
    match nombre {
        "ayer" => (hoy - Dur::days(1), hoy - Dur::days(1)),
        "semana" => (lunes, hoy),
        "mes" => (primero, hoy),
        "mes_pasado" => {
            let fin = primero - Dur::days(1);
            (fin.replace_day(1).unwrap_or(fin), fin)
        }
        _ => (hoy, hoy),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use time::macros::{date, time};

    fn pauta(id: u64, nombre: &str, clip: &str, horas: Vec<Time>) -> Pauta {
        Pauta {
            id,
            nombre: nombre.into(),
            clip: clip.into(),
            duracion_seg: 30.0,
            inicio: date!(2026 - 11 - 01),
            fin: Some(date!(2026 - 11 - 15)),
            dias: pauta::Dias::Todos,
            horas,
            tolerancia_min: 15,
            despues_min: None,
            prioridad: 0,
            activa: true,
            cliente: String::new(),
            notas: String::new(),
        }
    }

    fn salida(hora: &str, clip: &str, cliente: &str, emitido: f64, pase: Option<&str>) -> Salida {
        Salida {
            hora: hora.into(),
            clip: clip.into(),
            cliente: cliente.into(),
            pase: pase.map(String::from),
            duracion: 30.0,
            emitido,
            completo: emitido >= 29.0,
            modo: "AUTO".into(),
            quien: "PC principal".into(),
        }
    }

    #[test]
    fn cliente_del_spot() {
        let p = vec![pauta(7, "Ferretería X", "CLIENTES/FERRETERIA X/SPOT 30S", vec![])];
        assert_eq!(cliente_de("otro", Some("7@2026-11-11@10:15"), &p), "Ferretería X");
        assert_eq!(cliente_de("CLIENTES/FERRETERIA X/SPOT 30S", None, &p), "Ferretería X");
        assert_eq!(cliente_de("CLIENTES/FARMACIA SALUD/SPOT 20S", None, &p), "FARMACIA SALUD");
        assert_eq!(cliente_de("IDS/ID CANAL", None, &p), "ID CANAL");
        assert_eq!(cliente_de("C:/Videos/Hotel Plaza.mp4", None, &p), "Hotel Plaza");
    }

    #[test]
    fn reporte_por_cliente_y_cumplimiento() {
        let base = std::env::temp_dir().join(format!("relevo-reporte-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let pautas = vec![pauta(1, "Ferretería X", "CLIENTES/FERRETERIA X/SPOT 30S", vec![time!(10:00), time!(18:00)])];
        let d1 = date!(2026 - 11 - 10);
        let d2 = date!(2026 - 11 - 11);

        // Día 1: salió el pase de las 10:00 (completo); el de las 18:00 no.
        anotar(
            &base,
            d1,
            &salida("10:05:00", "CLIENTES/FERRETERIA X/SPOT 30S", "Ferretería X", 30.0, Some("1@2026-11-10@10:00")),
        )
        .unwrap();
        Almacen::new(base.clone())
            .guardar_emitidos(
                d1,
                &[pauta::Emision {
                    clave: "1@2026-11-10@10:00".into(),
                    clip: "CLIENTES/FERRETERIA X/SPOT 30S".into(),
                    hora: "10:05:00".into(),
                }],
            )
            .unwrap();
        // Día 2: el pase de las 10:00 salió pero se cortó (no cuenta como cumplido); además, un
        // spot manual cortado a los 12 s. Son las 12:00 (el pase de las 18:00 aún no llega).
        anotar(
            &base,
            d2,
            &salida("10:01:00", "CLIENTES/FERRETERIA X/SPOT 30S", "Ferretería X", 8.0, Some("1@2026-11-11@10:00")),
        )
        .unwrap();
        Almacen::new(base.clone())
            .guardar_emitidos(
                d2,
                &[pauta::Emision {
                    clave: "1@2026-11-11@10:00".into(),
                    clip: "CLIENTES/FERRETERIA X/SPOT 30S".into(),
                    hora: "10:01:00".into(),
                }],
            )
            .unwrap();
        anotar(&base, d2, &salida("11:00:00", "CLIENTES/FARMACIA SALUD/SPOT 20S", "FARMACIA SALUD", 12.0, None))
            .unwrap();

        // Del 01/11 al 11/11: los días antes del primer registro (10/11) no cuentan como perdidos.
        let r = calcular(&base, &pautas, date!(2026 - 11 - 01), d2, d2, time!(12:00));
        assert_eq!(r.salidas.len(), 3);
        let f = r.clientes.iter().find(|c| c.cliente == "Ferretería X").unwrap();
        // Programados: 10:00 y 18:00 del día 1, 10:00 del día 2 (18:00 del día 2 todavía no).
        assert_eq!((f.programados, f.cumplidos, f.perdidos), (3, 1, 2));
        assert_eq!((f.salidas, f.cortadas), (2, 1));
        assert!((f.cumplimiento().unwrap() - 33.33).abs() < 0.1);
        let fa = r.clientes.iter().find(|c| c.cliente == "FARMACIA SALUD").unwrap();
        assert_eq!((fa.salidas, fa.cortadas, fa.programados), (1, 1, 0));
        assert!(fa.cumplimiento().is_none());

        let solo = r.de_cliente("FARMACIA SALUD");
        assert_eq!(solo.salidas.len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn alertas_de_contratos_y_videos() {
        let mut p = pauta(1, "Ferretería X", "CLIENTES/FERRETERIA X/SPOT 30S", vec![time!(10:00)]);
        p.fin = Some(date!(2026 - 11 - 12));
        let medios = vec!["OTRO".to_string()];
        let a = alertas(&[p.clone()], Some(&medios), date!(2026 - 11 - 11));
        assert!(a.iter().any(|x| x.texto.contains("termina mañana") && x.grave));
        assert!(a.iter().any(|x| x.texto.contains("Falta el video")));
        // Lejos del fin y con el video: nada.
        p.fin = Some(date!(2026 - 12 - 30));
        let medios = vec!["clientes/ferreteria x/spot 30s".to_string()];
        assert!(alertas(&[p], Some(&medios), date!(2026 - 11 - 11)).is_empty());
    }

    #[test]
    fn periodos() {
        let hoy = date!(2026 - 11 - 11); // miércoles
        assert_eq!(periodo("semana", hoy), (date!(2026 - 11 - 09), hoy));
        assert_eq!(periodo("mes", hoy), (date!(2026 - 11 - 01), hoy));
        assert_eq!(periodo("mes_pasado", hoy), (date!(2026 - 10 - 01), date!(2026 - 10 - 31)));
        assert_eq!(periodo("ayer", hoy), (date!(2026 - 11 - 10), date!(2026 - 11 - 10)));
    }
}
