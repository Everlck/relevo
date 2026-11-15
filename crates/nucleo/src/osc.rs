//! Escucha el estado que CasparCG envía por OSC (UDP) para saber el tiempo
//! exacto del clip que está al aire.
//!
//! En `casparcg.config` hay que tener un cliente OSC apuntando a esta PC, por ejemplo:
//!
//! ```xml
//! <osc>
//!   <predefined-clients>
//!     <predefined-client><address>127.0.0.1</address><port>6250</port></predefined-client>
//!   </predefined-clients>
//! </osc>
//! ```

use std::time::Duration;

use rosc::{OscMessage, OscPacket, OscType};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

/// Lo que CasparCG informa de la salida del canal: niveles de audio (picos de cada
/// canal de audio, de 0 a 1) y cuadros por segundo. Lo lee la interfaz (vúmetros).
#[derive(Debug, Clone, Default)]
pub struct EstadoSalida {
    pub picos: Vec<f32>,
    pub cuando: Option<std::time::Instant>,
    pub fps: Option<f64>,
    /// Último paquete OSC de CasparCG (de lo que sea).
    pub ultimo: Option<std::time::Instant>,
    /// Capa donde suena el canal por IPTV (0 = ninguna) y su última señal de vida.
    pub capa_iptv: u16,
    pub iptv_vida: Option<std::time::Instant>,
    iptv_tiempo: Option<f64>,
}

pub type SalidaCompartida = std::sync::Arc<std::sync::Mutex<EstadoSalida>>;

/// `/channel/1/mixer/audio/volume` (picos enteros de 32 bits por canal de audio,
/// CasparCG 2.3+) → niveles de 0 a 1.
pub fn picos_de(m: &OscMessage, canal: u16) -> Option<Vec<f32>> {
    if m.addr != format!("/channel/{canal}/mixer/audio/volume") {
        return None;
    }
    let v: Vec<f32> = m
        .args
        .iter()
        .filter_map(|a| match a {
            OscType::Int(i) => Some((*i as f64).abs() / i32::MAX as f64),
            OscType::Long(i) => Some((*i as f64).abs() / i32::MAX as f64),
            OscType::Float(f) => Some(f.abs() as f64),
            OscType::Double(f) => Some(f.abs()),
            _ => None,
        })
        .map(|x| x.clamp(0.0, 1.0) as f32)
        .collect();
    (!v.is_empty()).then_some(v)
}

/// `/channel/1/framerate` (numerador y denominador) → cuadros por segundo.
pub fn fps_de(m: &OscMessage, canal: u16) -> Option<f64> {
    if m.addr != format!("/channel/{canal}/framerate") {
        return None;
    }
    let n: Vec<f64> = m
        .args
        .iter()
        .filter_map(|a| match a {
            OscType::Int(i) => Some(*i as f64),
            OscType::Long(i) => Some(*i as f64),
            OscType::Float(f) => Some(*f as f64),
            OscType::Double(f) => Some(*f),
            _ => None,
        })
        .collect();
    match n.as_slice() {
        [num, den, ..] if *den > 0.0 => Some(num / den),
        [x] => Some(*x),
        _ => None,
    }
    .filter(|f| *f > 1.0 && *f < 200.0)
}

/// Tiempo de un clip según CasparCG.
#[derive(Debug, Clone, PartialEq)]
pub struct TiempoOsc {
    pub transcurrido: Duration,
    pub total: Duration,
    /// Archivo al aire (`foreground/file/path`), si CasparCG lo informa. Sirve para
    /// saber con certeza a qué clip corresponde el tiempo.
    pub archivo: Option<String>,
}

/// Abre el puerto UDP y envía por `tx` los tiempos de `canal-capa`.
pub async fn escuchar(
    puerto: u16,
    canal: u16,
    capa: u16,
    tx: mpsc::UnboundedSender<TiempoOsc>,
    salida: SalidaCompartida,
) {
    let socket = match UdpSocket::bind(("0.0.0.0", puerto)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("no se pudo abrir el puerto OSC {puerto}: {e}. Se usará el reloj interno.");
            return;
        }
    };
    tracing::info!("escuchando OSC de CasparCG en el puerto {puerto}");
    let mut buf = vec![0u8; 65_536];
    // CasparCG manda en cada cuadro la ruta del archivo antes que el tiempo (a veces en
    // paquetes distintos): se recuerda la última para acompañar al tiempo.
    let mut archivo: Option<String> = None;
    loop {
        let Ok((n, _)) = socket.recv_from(&mut buf).await else {
            continue;
        };
        let Ok((_, paquete)) = rosc::decoder::decode_udp(&buf[..n]) else {
            continue;
        };
        let mut tiempos = Vec::new();
        recorrer(&paquete, canal, capa, &mut archivo, &mut tiempos);
        salida.lock().expect("salida").ultimo = Some(std::time::Instant::now());
        leer_salida(&paquete, canal, &salida);
        for t in tiempos {
            if tx.send(t).is_err() {
                return;
            }
        }
    }
}

fn recorrer(p: &OscPacket, canal: u16, capa: u16, archivo: &mut Option<String>, salida: &mut Vec<TiempoOsc>) {
    match p {
        OscPacket::Message(m) => {
            if let Some(a) = archivo_de(m, canal, capa) {
                *archivo = a;
            } else if let Some(mut t) = tiempo_de(m, canal, capa) {
                t.archivo = archivo.clone();
                salida.push(t);
            }
        }
        OscPacket::Bundle(b) => {
            for c in &b.content {
                recorrer(c, canal, capa, archivo, salida);
            }
        }
    }
}

fn leer_salida(p: &OscPacket, canal: u16, salida: &SalidaCompartida) {
    match p {
        OscPacket::Message(m) => {
            if let Some(v) = picos_de(m, canal) {
                let mut s = salida.lock().expect("salida");
                s.picos = v;
                s.cuando = Some(std::time::Instant::now());
            } else if let Some(f) = fps_de(m, canal) {
                salida.lock().expect("salida").fps = Some(f);
            } else {
                vida_iptv(m, canal, &mut salida.lock().expect("salida"));
            }
        }
        OscPacket::Bundle(b) => {
            for c in &b.content {
                leer_salida(c, canal, salida);
            }
        }
    }
}

/// Señales de vida de la capa del canal por IPTV: que avance su tiempo o, si
/// CasparCG no informa tiempo para ese flujo, que la capa tenga un productor ffmpeg.
pub fn vida_iptv(m: &OscMessage, canal: u16, s: &mut EstadoSalida) {
    if s.capa_iptv == 0 {
        return;
    }
    let prefijo = format!("/channel/{canal}/stage/layer/{}/", s.capa_iptv);
    let Some(resto) = m.addr.strip_prefix(&prefijo) else { return };
    let resto = resto.strip_prefix("foreground/").unwrap_or(resto);
    let ahora = std::time::Instant::now();
    match resto {
        "file/time" => {
            let t = m.args.first().and_then(|a| match a {
                OscType::Float(f) => Some(*f as f64),
                OscType::Double(d) => Some(*d),
                OscType::Int(i) => Some(*i as f64),
                OscType::Long(l) => Some(*l as f64),
                _ => None,
            });
            if let Some(t) = t {
                if s.iptv_tiempo != Some(t) {
                    s.iptv_vida = Some(ahora);
                }
                s.iptv_tiempo = Some(t);
            }
        }
        // Sin tiempo: basta con que haya un productor que no esté vacío.
        "producer" | "file/path" if s.iptv_tiempo.is_none() => {
            let vacio = matches!(m.args.first(), Some(OscType::String(p)) if p.eq_ignore_ascii_case("empty"));
            if !vacio && !m.args.is_empty() {
                s.iptv_vida = Some(ahora);
            }
        }
        _ => {}
    }
}

/// Olvida el tiempo visto (al volver a pedir el canal).
pub fn reiniciar_iptv(s: &mut EstadoSalida, capa: u16) {
    s.capa_iptv = capa;
    s.iptv_vida = None;
    s.iptv_tiempo = None;
}

/// `/channel/1/stage/layer/20/foreground/file/path` → `Some(Some(ruta))`.
/// `.../foreground/producer` que no es un archivo (color, vacío) → `Some(None)`.
pub fn archivo_de(m: &OscMessage, canal: u16, capa: u16) -> Option<Option<String>> {
    let prefijo = format!("/channel/{canal}/stage/layer/{capa}/");
    let resto = m.addr.strip_prefix(&prefijo)?;
    let texto = || match m.args.first() {
        Some(OscType::String(s)) => Some(s.clone()),
        _ => None,
    };
    match resto {
        "foreground/file/path" | "file/path" => texto().map(Some),
        "foreground/producer" => {
            let p = texto()?;
            (!p.eq_ignore_ascii_case("ffmpeg")).then_some(None)
        }
        _ => None,
    }
}

/// Reconoce `/channel/1/stage/layer/20/foreground/file/time` (CasparCG 2.2/2.3)
/// y `/channel/1/stage/layer/20/file/time` (2.0/2.1).
pub fn tiempo_de(m: &OscMessage, canal: u16, capa: u16) -> Option<TiempoOsc> {
    let prefijo = format!("/channel/{canal}/stage/layer/{capa}/");
    let resto = m.addr.strip_prefix(&prefijo)?;
    if resto != "foreground/file/time" && resto != "file/time" {
        return None;
    }
    let nums: Vec<f64> = m
        .args
        .iter()
        .filter_map(|a| match a {
            OscType::Float(f) => Some(*f as f64),
            OscType::Double(d) => Some(*d),
            OscType::Int(i) => Some(*i as f64),
            OscType::Long(l) => Some(*l as f64),
            _ => None,
        })
        .collect();
    let (&t, &total) = (nums.first()?, nums.get(1)?);
    if !(t.is_finite() && total.is_finite()) || t < 0.0 || total <= 0.0 {
        return None;
    }
    Some(TiempoOsc { transcurrido: Duration::from_secs_f64(t), total: Duration::from_secs_f64(total), archivo: None })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn msg(addr: &str, args: Vec<OscType>) -> OscMessage {
        OscMessage { addr: addr.into(), args }
    }

    #[test]
    fn formato_2_3() {
        let m = msg("/channel/1/stage/layer/20/foreground/file/time", vec![OscType::Float(12.5), OscType::Float(30.0)]);
        let t = tiempo_de(&m, 1, 20).unwrap();
        assert_eq!(t.transcurrido, Duration::from_millis(12_500));
        assert_eq!(t.total, Duration::from_secs(30));
    }

    #[test]
    fn formato_2_0_y_otra_capa() {
        let m = msg("/channel/1/stage/layer/20/file/time", vec![OscType::Double(1.0), OscType::Double(2.0)]);
        assert!(tiempo_de(&m, 1, 20).is_some());
        assert!(tiempo_de(&m, 1, 10).is_none());
        assert!(tiempo_de(&m, 2, 20).is_none());
    }

    #[test]
    fn mensajes_que_no_son_tiempo() {
        let m = msg("/channel/1/stage/layer/20/foreground/file/path", vec![OscType::String("x".into())]);
        assert!(tiempo_de(&m, 1, 20).is_none());
        let m = msg("/channel/1/stage/layer/20/foreground/file/time", vec![OscType::Float(1.0)]);
        assert!(tiempo_de(&m, 1, 20).is_none());
    }

    #[tokio::test]
    async fn recibe_por_udp() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        // Puerto libre.
        let puerto = std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        tokio::spawn(escuchar(puerto, 1, 20, tx, Default::default()));
        tokio::time::sleep(Duration::from_millis(100)).await;
        let paquete = OscPacket::Message(msg(
            "/channel/1/stage/layer/20/foreground/file/time",
            vec![OscType::Float(3.0), OscType::Float(20.0)],
        ));
        let bytes = rosc::encoder::encode(&paquete).unwrap();
        let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        s.send_to(&bytes, ("127.0.0.1", puerto)).unwrap();
        let t = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
        assert_eq!(t.total, Duration::from_secs(20));
    }

    #[test]
    fn el_tiempo_lleva_el_archivo_que_vino_antes() {
        let base = "/channel/1/stage/layer/20/foreground/";
        let paquete = OscPacket::Bundle(rosc::OscBundle {
            timetag: rosc::OscTime { seconds: 0, fractional: 0 },
            content: vec![
                OscPacket::Message(msg(&format!("{base}file/path"), vec![OscType::String("D:/media/UNO.mp4".into())])),
                OscPacket::Message(msg(&format!("{base}file/time"), vec![OscType::Float(1.0), OscType::Float(10.0)])),
            ],
        });
        let mut archivo = None;
        let mut salida = Vec::new();
        recorrer(&paquete, 1, 20, &mut archivo, &mut salida);
        assert_eq!(salida[0].archivo.as_deref(), Some("D:/media/UNO.mp4"));
        // En el paquete siguiente solo llega el tiempo: se usa la última ruta.
        let solo =
            OscPacket::Message(msg(&format!("{base}file/time"), vec![OscType::Float(1.1), OscType::Float(10.0)]));
        recorrer(&solo, 1, 20, &mut archivo, &mut salida);
        assert_eq!(salida[1].archivo.as_deref(), Some("D:/media/UNO.mp4"));
        // Un productor que no es un archivo (el transparente) borra la ruta.
        let color = OscPacket::Message(msg(&format!("{base}producer"), vec![OscType::String("color".into())]));
        recorrer(&color, 1, 20, &mut archivo, &mut salida);
        assert_eq!(archivo, None);
    }

    #[test]
    fn vida_de_la_capa_iptv() {
        let mut s = EstadoSalida::default();
        let t = |x: f32| {
            msg("/channel/1/stage/layer/10/foreground/file/time", vec![OscType::Float(x), OscType::Float(0.0)])
        };
        vida_iptv(&t(1.0), 1, &mut s);
        assert!(s.iptv_vida.is_none(), "sin capa de IPTV no se vigila");
        reiniciar_iptv(&mut s, 10);
        vida_iptv(&t(1.0), 1, &mut s);
        let primera = s.iptv_vida.expect("avanza el tiempo");
        std::thread::sleep(Duration::from_millis(5));
        // El tiempo quieto (canal congelado) no es señal de vida.
        vida_iptv(&t(1.0), 1, &mut s);
        assert_eq!(s.iptv_vida, Some(primera));
        // Con tiempo conocido, el productor ya no cuenta.
        let p = msg("/channel/1/stage/layer/10/foreground/producer", vec![OscType::String("ffmpeg".into())]);
        vida_iptv(&p, 1, &mut s);
        assert_eq!(s.iptv_vida, Some(primera));
        // Si CasparCG no informa tiempo, el productor sí cuenta (menos "empty").
        reiniciar_iptv(&mut s, 10);
        let vacio = msg("/channel/1/stage/layer/10/foreground/producer", vec![OscType::String("empty".into())]);
        vida_iptv(&vacio, 1, &mut s);
        assert!(s.iptv_vida.is_none());
        vida_iptv(&p, 1, &mut s);
        assert!(s.iptv_vida.is_some());
        // Otra capa no cuenta.
        reiniciar_iptv(&mut s, 10);
        vida_iptv(&msg("/channel/1/stage/layer/20/foreground/file/time", vec![OscType::Float(3.0)]), 1, &mut s);
        assert!(s.iptv_vida.is_none());
    }

    #[test]
    fn niveles_de_audio_y_fps() {
        let m = msg("/channel/1/mixer/audio/volume", vec![OscType::Int(i32::MAX / 2), OscType::Int(0)]);
        let v = picos_de(&m, 1).unwrap();
        assert!((v[0] - 0.5).abs() < 0.01 && v[1] == 0.0);
        assert!(picos_de(&m, 2).is_none());
        let f = msg("/channel/1/framerate", vec![OscType::Int(30000), OscType::Int(1001)]);
        assert!((fps_de(&f, 1).unwrap() - 29.97).abs() < 0.01);
    }
}
