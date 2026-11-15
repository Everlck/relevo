//! Transiciones entre spots (las de CasparCG: corte, fundido, barrido…).

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TipoTransicion {
    /// Cambio directo, sin efecto.
    #[default]
    Corte,
    /// Fundido cruzado (MIX).
    Fundido,
    /// Barrido (WIPE).
    Barrido,
    /// Empuje (PUSH).
    Empuje,
    /// Deslizar (SLIDE).
    Deslizar,
}

impl TipoTransicion {
    pub const TODOS: [TipoTransicion; 5] = [
        TipoTransicion::Corte,
        TipoTransicion::Fundido,
        TipoTransicion::Barrido,
        TipoTransicion::Empuje,
        TipoTransicion::Deslizar,
    ];

    /// Palabra de AMCP.
    pub fn amcp(self) -> &'static str {
        match self {
            TipoTransicion::Corte => "CUT",
            TipoTransicion::Fundido => "MIX",
            TipoTransicion::Barrido => "WIPE",
            TipoTransicion::Empuje => "PUSH",
            TipoTransicion::Deslizar => "SLIDE",
        }
    }

    pub fn nombre(self) -> &'static str {
        match self {
            TipoTransicion::Corte => "Corte",
            TipoTransicion::Fundido => "Fundido",
            TipoTransicion::Barrido => "Barrido",
            TipoTransicion::Empuje => "Empuje",
            TipoTransicion::Deslizar => "Deslizar",
        }
    }

    /// Lee "corte", "fundido", "mix", "wipe"…
    pub fn desde_texto(t: &str) -> Option<Self> {
        Some(match t.trim().to_lowercase().as_str() {
            "corte" | "cut" | "" => TipoTransicion::Corte,
            "fundido" | "mix" | "fade" => TipoTransicion::Fundido,
            "barrido" | "wipe" => TipoTransicion::Barrido,
            "empuje" | "push" => TipoTransicion::Empuje,
            "deslizar" | "slide" => TipoTransicion::Deslizar,
            _ => return None,
        })
    }
}

/// Transición y su duración. Se usa al encadenar spots, al salir de la señal de TV
/// y al volver a ella.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transicion {
    pub tipo: TipoTransicion,
    /// Segundos (se convierten a cuadros con la frecuencia del canal).
    pub segundos: f32,
}

impl Default for Transicion {
    fn default() -> Self {
        Transicion { tipo: TipoTransicion::Corte, segundos: 0.5 }
    }
}

impl Transicion {
    /// `true` si hay efecto (no es un corte).
    pub fn activa(&self) -> bool {
        self.tipo != TipoTransicion::Corte && self.segundos > 0.0
    }

    /// Cuadros para una frecuencia (cuadros por segundo) del canal.
    pub fn cuadros(&self, fps: f64) -> u32 {
        if !self.activa() {
            return 0;
        }
        (self.segundos as f64 * fps).round().clamp(1.0, 600.0) as u32
    }

    /// Para AMCP: `Some(("MIX", 15))`, o `None` si es un corte.
    pub fn para_amcp(&self, fps: f64) -> Option<(&'static str, u32)> {
        self.activa().then(|| (self.tipo.amcp(), self.cuadros(fps)))
    }

    /// Cuánto se adelanta el cambio al siguiente spot (CasparCG empieza la transición
    /// antes de que termine el actual).
    pub fn adelanto(&self, fps: f64) -> Duration {
        if !self.activa() || fps <= 0.0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.cuadros(fps) as f64 / fps)
    }

    pub fn describir(&self) -> String {
        if self.activa() {
            format!("{} {:.1} s", self.tipo.nombre(), self.segundos)
        } else {
            "Corte".into()
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn cuadros_y_amcp() {
        let t = Transicion { tipo: TipoTransicion::Fundido, segundos: 0.5 };
        assert_eq!(t.cuadros(29.97), 15);
        assert_eq!(t.para_amcp(25.0), Some(("MIX", 13)));
        assert_eq!(t.adelanto(25.0), Duration::from_millis(520));
        let c = Transicion { tipo: TipoTransicion::Corte, segundos: 0.5 };
        assert_eq!(c.para_amcp(25.0), None);
        assert_eq!(c.adelanto(25.0), Duration::ZERO);
        assert_eq!(TipoTransicion::desde_texto("MIX"), Some(TipoTransicion::Fundido));
        assert_eq!(TipoTransicion::desde_texto("x"), None);
    }
}
