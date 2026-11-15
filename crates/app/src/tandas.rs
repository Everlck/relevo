//! Tandas guardadas con nombre (archivos JSON en la carpeta `tandas`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Archivo {
    version: u32,
    spots: Vec<Spot>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Spot {
    clip: String,
    duracion_seg: f64,
}

pub struct Tandas {
    carpeta: PathBuf,
}

impl Tandas {
    pub fn new(carpeta: PathBuf) -> Self {
        Tandas { carpeta }
    }

    pub fn carpeta(&self) -> &Path {
        &self.carpeta
    }

    /// Nombres de las tandas guardadas, en orden alfabético.
    pub fn nombres(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.carpeta)
            .map(|l| {
                l.flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        (p.extension().is_some_and(|x| x == "json"))
                            .then(|| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                            .flatten()
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by_key(|s| s.to_lowercase());
        v
    }

    pub fn guardar(&self, nombre: &str, spots: &[(String, Duration)]) -> anyhow::Result<()> {
        let nombre = limpiar_nombre(nombre);
        anyhow::ensure!(!nombre.is_empty(), "el nombre está vacío");
        std::fs::create_dir_all(&self.carpeta)?;
        let a = Archivo {
            version: 1,
            spots: spots.iter().map(|(c, d)| Spot { clip: c.clone(), duracion_seg: d.as_secs_f64() }).collect(),
        };
        std::fs::write(self.carpeta.join(format!("{nombre}.json")), serde_json::to_string_pretty(&a)?)?;
        Ok(())
    }

    pub fn abrir(&self, nombre: &str) -> anyhow::Result<Vec<(String, Duration)>> {
        let texto = std::fs::read_to_string(self.carpeta.join(format!("{nombre}.json")))?;
        let a: Archivo = serde_json::from_str(&texto)?;
        Ok(a.spots.into_iter().map(|s| (s.clip, Duration::from_secs_f64(s.duracion_seg.max(0.0)))).collect())
    }

    pub fn borrar(&self, nombre: &str) -> anyhow::Result<()> {
        std::fs::remove_file(self.carpeta.join(format!("{nombre}.json")))?;
        Ok(())
    }
}

/// Quita los caracteres que Windows no permite en nombres de archivo.
pub fn limpiar_nombre(nombre: &str) -> String {
    nombre
        .chars()
        .map(|c| if r#"\/:*?"<>|"#.contains(c) { '-' } else { c })
        .collect::<String>()
        .trim()
        .trim_end_matches('.')
        .to_string()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn guardar_abrir_y_borrar() {
        let d = std::env::temp_dir().join(format!("relevo-tandas-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let t = Tandas::new(d.clone());
        let spots =
            vec![("CLIENTES/A".to_string(), Duration::from_secs(30)), ("ID".to_string(), Duration::from_secs(10))];
        t.guardar("Tanda: mañana", &spots).unwrap();
        assert_eq!(t.nombres(), vec!["Tanda- mañana"]);
        assert_eq!(t.abrir("Tanda- mañana").unwrap(), spots);
        t.borrar("Tanda- mañana").unwrap();
        assert!(t.nombres().is_empty());
    }

    #[test]
    fn nombre_vacio() {
        let d = std::env::temp_dir().join(format!("relevo-tandas2-{}", std::process::id()));
        assert!(Tandas::new(d).guardar("  ", &[]).is_err());
    }
}
