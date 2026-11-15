use serde::{Deserialize, Serialize};

/// Tipo de archivo según CasparCG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TipoMedio {
    Movie,
    Still,
    Audio,
    Otro,
}

/// Un archivo de la carpeta de medios de CasparCG (una línea de `CLS`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Medio {
    /// Nombre tal como lo usa CasparCG: ruta relativa, sin extensión, con `/`.
    pub nombre: String,
    pub tipo: TipoMedio,
    pub tamano_bytes: u64,
    /// Fecha de modificación en formato `AAAAMMDDhhmmss`.
    pub modificado: String,
    pub cuadros: u64,
    /// Cuadros por segundo (0 si no aplica).
    pub fps: f64,
    /// Duración en segundos (0 si no aplica).
    pub duracion_seg: f64,
}

impl Medio {
    /// Interpreta una línea de `CLS`, por ejemplo:
    /// `"CLIENTES/SPOT1" MOVIE 6445960 20170413141130 268 1/25`
    pub fn desde_linea_cls(linea: &str) -> Option<Medio> {
        let linea = linea.trim();
        let resto = linea.strip_prefix('"')?;
        let fin = resto.find('"')?;
        let nombre = resto[..fin].to_string();
        let mut campos = resto[fin + 1..].split_whitespace();

        let tipo = match campos.next()? {
            "MOVIE" => TipoMedio::Movie,
            "STILL" => TipoMedio::Still,
            "AUDIO" => TipoMedio::Audio,
            _ => TipoMedio::Otro,
        };
        let tamano_bytes = campos.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        let modificado = campos.next().unwrap_or_default().to_string();
        let cuadros: u64 = campos.next().and_then(|v| v.parse().ok()).unwrap_or(0);

        // La base de tiempo viene como fracción "num/den": cada cuadro dura num/den segundos.
        let (num, den) = campos
            .next()
            .and_then(|f| f.split_once('/'))
            .and_then(|(n, d)| Some((n.parse::<f64>().ok()?, d.parse::<f64>().ok()?)))
            .unwrap_or((0.0, 0.0));

        let (fps, duracion_seg) =
            if num > 0.0 && den > 0.0 { (den / num, cuadros as f64 * num / den) } else { (0.0, 0.0) };

        Some(Medio { nombre, tipo, tamano_bytes, modificado, cuadros, fps, duracion_seg })
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn linea_de_video() {
        let m = Medio::desde_linea_cls("\"CLIENTES/SPOT 1\" MOVIE 6445960 20170413141130 750 1/25").unwrap();
        assert_eq!(m.nombre, "CLIENTES/SPOT 1");
        assert_eq!(m.tipo, TipoMedio::Movie);
        assert_eq!(m.tamano_bytes, 6_445_960);
        assert_eq!(m.cuadros, 750);
        assert!((m.fps - 25.0).abs() < 1e-9);
        assert!((m.duracion_seg - 30.0).abs() < 1e-9);
    }

    #[test]
    fn linea_ntsc() {
        let m = Medio::desde_linea_cls("\"SPOT\" MOVIE 1 20240101000000 900 1001/30000").unwrap();
        assert!((m.fps - 29.97).abs() < 0.01);
        assert!((m.duracion_seg - 30.03).abs() < 0.01);
    }

    #[test]
    fn linea_de_imagen() {
        let m = Medio::desde_linea_cls("\"LOGO\" STILL 1234 20240101000000 0 0/1").unwrap();
        assert_eq!(m.tipo, TipoMedio::Still);
        assert_eq!(m.duracion_seg, 0.0);
    }

    #[test]
    fn linea_invalida() {
        assert!(Medio::desde_linea_cls("sin comillas").is_none());
    }
}
