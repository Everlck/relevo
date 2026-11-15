//! Señal de TV por IPTV: CasparCG reproduce el canal (una dirección de red) en una
//! capa debajo de la publicidad. Mientras hay publicidad al aire, el sonido del canal
//! se baja a cero; al volver a la señal de TV, se sube otra vez.
//!
//! Relevo vigila la capa por OSC: si el canal deja de llegar, lo vuelve a pedir.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Qué reproducir y en qué capa.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Iptv {
    /// Dirección del canal: http(s)://….m3u8, udp://@239.x.x.x:1234, rtp://, rtmp://, srt://…
    pub direccion: String,
    /// Capa de CasparCG (debajo de la de publicidad).
    pub capa: u16,
}

/// Cómo está la señal por IPTV.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum EstadoIptv {
    /// IPTV desactivada.
    #[default]
    Apagada,
    /// Se pidió el canal y se espera a que llegue.
    Conectando,
    EnVivo,
    /// Se envió, pero CasparCG no manda OSC: no se puede comprobar si llega.
    SinVigilancia,
    /// Dejó de llegar; se reintenta.
    Caida(String),
}

/// Lo que la interfaz muestra de la IPTV.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VistaIptv {
    pub estado: EstadoIptv,
    pub direccion: String,
    pub capa: u16,
    /// Veces que se volvió a pedir el canal desde que se activó.
    pub reconexiones: u32,
    /// `true` mientras el sonido del canal está bajado (publicidad al aire).
    pub silenciado: bool,
}

/// Tiempo para que el canal empiece a llegar (un HLS puede tardar en arrancar).
pub const ESPERA_INICIO: Duration = Duration::from_secs(20);
/// Sin señales de vida durante esto, se da por caída.
pub const SIN_VIDA: Duration = Duration::from_secs(6);
/// Si en este tiempo no llega ningún OSC de CasparCG, no se puede vigilar.
pub const SIN_OSC: Duration = Duration::from_secs(5);

/// Decide qué hacer con la vigilancia (lógica pura, para las pruebas).
#[derive(Debug, Clone, Default)]
pub struct Vigilancia {
    /// Cuándo se pidió el canal por última vez.
    pub pedido: Option<Instant>,
    pub reintentos: u32,
    pub reconexiones: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Paso {
    Nada,
    /// Volver a pedir el canal.
    Pedir,
}

impl Vigilancia {
    /// Espera entre reintentos (desde el último pedido): 5, 10, 20, 40, 60, 60… segundos.
    /// El primer reintento es inmediato: en cuanto pasan 6 s sin señal.
    pub fn espera(&self) -> Duration {
        let s = 5u64.saturating_mul(1u64 << self.reintentos.min(4));
        Duration::from_secs(s.min(60))
    }

    pub fn pedido(&mut self, ahora: Instant) {
        self.pedido = Some(ahora);
    }

    /// `osc`: último mensaje OSC de CasparCG (cualquiera). `vida`: última señal de vida
    /// de la capa del canal.
    pub fn revisar(&mut self, ahora: Instant, osc: Option<Instant>, vida: Option<Instant>) -> (EstadoIptv, Paso) {
        let Some(pedido) = self.pedido else { return (EstadoIptv::Conectando, Paso::Pedir) };
        let hay_osc = osc.is_some_and(|t| ahora.duration_since(t) < SIN_OSC);
        if !hay_osc {
            return (EstadoIptv::SinVigilancia, Paso::Nada);
        }
        // Señal de vida posterior al pedido y reciente.
        let vivo = vida.is_some_and(|t| t >= pedido && ahora.duration_since(t) < SIN_VIDA);
        if vivo {
            self.reintentos = 0;
            return (EstadoIptv::EnVivo, Paso::Nada);
        }
        let desde = ahora.duration_since(pedido);
        let llego_alguna_vez = vida.is_some_and(|t| t >= pedido);
        let limite = if llego_alguna_vez || self.reintentos > 0 { self.espera() } else { ESPERA_INICIO };
        if desde < limite {
            let estado = if llego_alguna_vez || self.reintentos > 0 {
                EstadoIptv::Caida("Se cortó el canal; reintentando…".into())
            } else {
                EstadoIptv::Conectando
            };
            return (estado, Paso::Nada);
        }
        self.reintentos += 1;
        self.reconexiones += 1;
        let motivo = if llego_alguna_vez {
            "Se cortó el canal; reintentando…".to_string()
        } else {
            "El canal no llega; reintentando…".to_string()
        };
        (EstadoIptv::Caida(motivo), Paso::Pedir)
    }
}

/// Canales de una lista M3U: (nombre, dirección).
pub fn leer_m3u(texto: &str) -> Vec<(String, String)> {
    let mut v = Vec::new();
    let mut nombre: Option<String> = None;
    for linea in texto.lines().map(str::trim) {
        if linea.is_empty() {
            continue;
        }
        if let Some(info) = linea.strip_prefix("#EXTINF") {
            // #EXTINF:-1 tvg-id="x" group-title="y",Nombre del canal
            nombre = info.rsplit_once(',').map(|(_, n)| n.trim().to_string()).filter(|n| !n.is_empty());
            continue;
        }
        if linea.starts_with('#') {
            continue;
        }
        if linea.contains("://") {
            let n = nombre.take().unwrap_or_else(|| linea.to_string());
            v.push((n, linea.to_string()));
        }
    }
    v
}

/// `true` si parece una dirección de red que CasparCG puede abrir.
pub fn es_direccion(d: &str) -> bool {
    let d = d.trim().to_lowercase();
    ["http://", "https://", "udp://", "rtp://", "rtmp://", "rtmps://", "rtsp://", "srt://"]
        .iter()
        .any(|p| d.starts_with(p))
        && d.len() > 10
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn lee_una_lista_m3u() {
        let t = "#EXTM3U\n#EXTINF:-1 tvg-id=\"am\" group-title=\"Nacional\",Canal de ejemplo\nhttp://x/am.m3u8\n\
                 #EXTVLCOPT:http-user-agent=VLC\n#EXTINF:-1,Canal 2\nudp://@239.1.1.2:1234\n\nrtp://@239.1.1.3:5000\n";
        let v = leer_m3u(t);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], ("Canal de ejemplo".into(), "http://x/am.m3u8".into()));
        assert_eq!(v[1].0, "Canal 2");
        assert_eq!(v[2].0, "rtp://@239.1.1.3:5000");
    }

    #[test]
    fn reconoce_direcciones() {
        assert!(es_direccion("udp://@239.1.1.1:1234"));
        assert!(es_direccion("https://sitio/canal.m3u8"));
        assert!(!es_direccion("C:\\videos\\a.mp4"));
        assert!(!es_direccion("http://"));
    }

    #[test]
    fn vigila_y_reintenta() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let mut v = Vigilancia::default();
        // Sin pedir todavía: hay que pedirlo.
        assert_eq!(v.revisar(t0, Some(t0), None).1, Paso::Pedir);
        v.pedido(t0);
        // Sin OSC no se puede vigilar.
        assert_eq!(v.revisar(s(30), None, None), (EstadoIptv::SinVigilancia, Paso::Nada));
        // Con OSC, al principio se espera a que arranque.
        assert_eq!(v.revisar(s(5), Some(s(5)), None), (EstadoIptv::Conectando, Paso::Nada));
        // Llega.
        assert_eq!(v.revisar(s(8), Some(s(8)), Some(s(8))), (EstadoIptv::EnVivo, Paso::Nada));
        // Se corta: mientras no pasen 6 s sin señal, todavía se espera.
        assert_eq!(v.revisar(s(12), Some(s(12)), Some(s(8))), (EstadoIptv::EnVivo, Paso::Nada));
        // A los 6 s sin señal se da por caído y se vuelve a pedir enseguida.
        let (e, p) = v.revisar(s(15), Some(s(15)), Some(s(8)));
        assert!(matches!(e, EstadoIptv::Caida(_)) && p == Paso::Pedir, "{e:?}");
        assert_eq!(v.reconexiones, 1);
        v.pedido(s(15));
        // Cada reintento espera más: 10 s, después 20 s…
        assert_eq!(v.espera(), Duration::from_secs(10));
        let (e, p) = v.revisar(s(20), Some(s(20)), Some(s(8)));
        assert!(matches!(e, EstadoIptv::Caida(_)) && p == Paso::Nada, "{e:?}");
        assert_eq!(v.revisar(s(25), Some(s(25)), Some(s(8))).1, Paso::Pedir);
        v.pedido(s(25));
        assert_eq!(v.espera(), Duration::from_secs(20));
        // Vuelve: se reinicia la espera.
        assert_eq!(v.revisar(s(30), Some(s(30)), Some(s(30))).0, EstadoIptv::EnVivo);
        assert_eq!(v.espera(), Duration::from_secs(5));
    }
}
