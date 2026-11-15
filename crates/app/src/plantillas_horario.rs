//! Plantillas de horario: listas de horas con nombre («Pauta diaria», «Noticieros»…)
//! que se aplican a cualquier publicidad. Se guardan en `datos/horarios.json` (entra en el respaldo).

use std::path::PathBuf;

use nucleo::pauta::formato;
use serde::{Deserialize, Serialize};
use time::Time;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plantilla {
    pub nombre: String,
    #[serde(with = "formato::horas")]
    pub horas: Vec<Time>,
}

pub struct Plantillas {
    ruta: PathBuf,
    pub lista: Vec<Plantilla>,
}

impl Plantillas {
    /// Lee el archivo (si no existe o está dañado, empieza vacía).
    pub fn abrir(ruta: PathBuf) -> Self {
        let lista = std::fs::read_to_string(&ruta)
            .ok()
            .and_then(|t| serde_json::from_str::<Vec<Plantilla>>(&t).ok())
            .unwrap_or_default();
        Plantillas { ruta, lista }
    }

    /// Guarda (o reemplaza, si ya hay una con ese nombre) una plantilla.
    pub fn guardar(&mut self, nombre: &str, horas: &[Time]) -> Result<(), String> {
        let nombre = nombre.trim();
        if nombre.is_empty() {
            return Err("Ponle un nombre a la plantilla.".into());
        }
        if horas.is_empty() {
            return Err("No hay horas para guardar.".into());
        }
        let mut horas = horas.to_vec();
        horas.sort();
        horas.dedup();
        let p = Plantilla { nombre: nombre.to_string(), horas };
        match self.lista.iter_mut().find(|x| x.nombre.eq_ignore_ascii_case(nombre)) {
            Some(x) => *x = p,
            None => self.lista.push(p),
        }
        self.lista.sort_by_key(|p| p.nombre.to_lowercase());
        self.escribir()
    }

    pub fn borrar(&mut self, nombre: &str) -> Result<(), String> {
        self.lista.retain(|p| p.nombre != nombre);
        self.escribir()
    }

    fn escribir(&self) -> Result<(), String> {
        if let Some(c) = self.ruta.parent() {
            std::fs::create_dir_all(c).map_err(|e| e.to_string())?;
        }
        let texto = serde_json::to_string_pretty(&self.lista).map_err(|e| e.to_string())?;
        std::fs::write(&self.ruta, texto).map_err(|e| format!("No se pudo guardar la plantilla: {e}"))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use time::macros::time;

    #[test]
    fn guardar_reemplazar_y_borrar() {
        let d = std::env::temp_dir().join(format!("relevo-plantillas-{}", std::process::id()));
        let ruta = d.join("horarios.json");
        let mut p = Plantillas::abrir(ruta.clone());
        assert!(p.lista.is_empty());
        assert!(p.guardar(" ", &[time!(8:00)]).is_err());
        p.guardar("Diaria", &[time!(10:30), time!(6:00), time!(6:00)]).unwrap();
        p.guardar("diaria", &[time!(20:00)]).unwrap(); // mismo nombre: reemplaza
        p.guardar("Noche", &[time!(22:00)]).unwrap();
        let p2 = Plantillas::abrir(ruta.clone());
        assert_eq!(p2.lista.len(), 2);
        assert_eq!(p2.lista[0].horas, vec![time!(20:00)]);
        p.borrar("Noche").unwrap();
        assert_eq!(Plantillas::abrir(ruta).lista.len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }
}
