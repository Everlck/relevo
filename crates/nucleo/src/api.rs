//! API HTTP sencilla para botoneras (Stream Deck…) y otros programas de la red.
//!
//! Cada acción es una dirección; sirve con GET (lo que hace la acción *Website* del
//! Stream Deck) o POST. Todas piden la clave: `?clave=…` o la cabecera `X-Clave`.
//!
//! ```text
//! http://IP:7801/api/estado
//! http://IP:7801/api/volver-a-tv?clave=…
//! ```
//!
//! Las órdenes llegan al núcleo como si fueran de la PC principal (mismo mando).

use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::controlador::{Controlador, FaseVista, ModoTanda, Orden, Origen, Rol};

/// Acciones disponibles: (dirección, descripción).
pub const ACCIONES: &[(&str, &str)] = &[
    ("/api/estado", "Qué está al aire, cuánto falta, qué sigue"),
    ("/api/iniciar-corte", "Pone el modo AUTO y lanza la tanda del corte"),
    ("/api/play", "Reproduce la tanda (o la reanuda)"),
    ("/api/pausa", "Pausa"),
    ("/api/play-pausa", "Play o pausa, según el caso"),
    ("/api/siguiente", "Siguiente spot"),
    ("/api/anterior", "Spot anterior"),
    ("/api/volver-a-tv", "Vuelve a la señal de TV (siempre funciona)"),
    ("/api/auto", "Modo AUTO"),
    ("/api/manual", "Modo MANUAL"),
];

#[derive(Serialize)]
struct Estado {
    fase: &'static str,
    modo: &'static str,
    al_aire: Option<String>,
    restante_seg: Option<f64>,
    siguiente: Option<String>,
    spots_en_tanda: usize,
    casparcg: bool,
}

fn estado(ctl: &Controlador) -> Estado {
    let e = ctl.ver();
    let actual = e.actual.and_then(|id| e.item(id));
    Estado {
        fase: match e.fase {
            FaseVista::Tv => "tv",
            FaseVista::AlAire => "al_aire",
            FaseVista::Pausado => "pausado",
        },
        modo: if e.modo == ModoTanda::Auto { "auto" } else { "manual" },
        al_aire: actual.map(|i| i.clip.clone()),
        restante_seg: actual.map(|i| (i.duracion.as_secs_f64() - e.transcurrido.as_secs_f64()).max(0.0)),
        siguiente: e.proximo.and_then(|id| e.item(id)).map(|i| i.clip.clone()),
        spots_en_tanda: e.lista.len(),
        casparcg: e.conectado,
    }
}

/// Abre la API en `puerto`. Sin clave no se abre.
pub async fn servir(puerto: u16, clave: String, ctl: Controlador) -> std::io::Result<()> {
    if clave.trim().is_empty() {
        return Err(std::io::Error::other("falta la clave de la API en config.toml ([api] clave)"));
    }
    let escucha = TcpListener::bind(("0.0.0.0", puerto)).await?;
    tracing::info!("API: escuchando en el puerto {puerto}");
    tokio::spawn(async move {
        loop {
            let Ok((flujo, _)) = escucha.accept().await else { continue };
            let (ctl, clave) = (ctl.clone(), clave.clone());
            tokio::spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(10), atender(flujo, &clave, &ctl)).await;
            });
        }
    });
    Ok(())
}

async fn atender(mut flujo: TcpStream, clave: &str, ctl: &Controlador) -> std::io::Result<()> {
    // Solo la cabecera (estas peticiones no traen cuerpo que interese).
    let mut buf = Vec::with_capacity(1024);
    let mut trozo = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 8192 {
        let n = flujo.read(&mut trozo).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&trozo[..n]);
    }
    let texto = String::from_utf8_lossy(&buf);
    let mut lineas = texto.lines();
    let primera = lineas.next().unwrap_or_default();
    let mut partes = primera.split_whitespace();
    let metodo = partes.next().unwrap_or_default().to_uppercase();
    let destino = partes.next().unwrap_or("/");
    let (ruta, consulta) = destino.split_once('?').unwrap_or((destino, ""));
    let ruta = ruta.trim_end_matches('/');
    let clave_cabecera = lineas
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("x-clave"))
        .map(|(_, v)| v.trim().to_string());
    let clave_consulta =
        consulta.split('&').filter_map(|p| p.split_once('=')).find(|(k, _)| *k == "clave").map(|(_, v)| decodificar(v));

    let (codigo, cuerpo) = if metodo == "OPTIONS" {
        (204, String::new())
    } else if ruta.is_empty() || ruta == "/api" {
        (200, ayuda())
    } else if clave_cabecera.or(clave_consulta).as_deref() != Some(clave) {
        tokio::time::sleep(Duration::from_millis(300)).await; // frena los intentos a ciegas
        (401, r#"{"ok":false,"error":"clave incorrecta"}"#.to_string())
    } else {
        responder(ruta, ctl).await
    };
    let tipo = if cuerpo.starts_with('{') { "application/json" } else { "text/plain; charset=utf-8" };
    let estado_http = match codigo {
        200 => "200 OK",
        204 => "204 No Content",
        401 => "401 Unauthorized",
        404 => "404 Not Found",
        _ => "409 Conflict",
    };
    let respuesta = format!(
        "HTTP/1.1 {estado_http}\r\nContent-Type: {tipo}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: X-Clave\r\nConnection: close\r\n\r\n{cuerpo}",
        cuerpo.len()
    );
    flujo.write_all(respuesta.as_bytes()).await?;
    flujo.shutdown().await
}

async fn responder(ruta: &str, ctl: &Controlador) -> (u16, String) {
    let origen = Origen { id: 0, nombre: "Botonera (API)".into(), rol: Rol::Admin };
    let e = ctl.ver();
    let orden = match ruta {
        "/api/estado" => None,
        "/api/play" => (e.fase != FaseVista::AlAire).then_some(Orden::PlayPausa(None)),
        "/api/pausa" => (e.fase == FaseVista::AlAire).then_some(Orden::PlayPausa(None)),
        "/api/play-pausa" => Some(Orden::PlayPausa(None)),
        "/api/siguiente" => Some(Orden::Siguiente),
        "/api/anterior" => Some(Orden::Anterior),
        "/api/volver-a-tv" | "/api/tv" => Some(Orden::VolverATv),
        "/api/auto" => Some(Orden::Modo(ModoTanda::Auto)),
        "/api/manual" => Some(Orden::Modo(ModoTanda::Manual)),
        "/api/iniciar-corte" => {
            if e.fase == FaseVista::AlAire {
                return (409, r#"{"ok":false,"error":"ya hay una tanda al aire"}"#.into());
            }
            // El núcleo atiende en orden: primero arma la tanda AUTO, luego le da Play.
            ctl.enviar_como(origen.clone(), Orden::Modo(ModoTanda::Auto));
            Some(Orden::PlayPausa(None))
        }
        _ => return (404, r#"{"ok":false,"error":"no existe esa acción (abre /api para ver la lista)"}"#.into()),
    };
    let hubo_orden = orden.is_some();
    if let Some(o) = orden {
        ctl.enviar_como(origen, o);
        // Un momento para que el estado refleje la orden.
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let est = estado(ctl);
    if ruta == "/api/iniciar-corte" && hubo_orden && est.fase == "tv" {
        return (409, r#"{"ok":false,"error":"no hay pases de la pauta en ventana ahora"}"#.into());
    }
    let cuerpo = serde_json::json!({ "ok": true, "estado": est });
    (200, cuerpo.to_string())
}

fn ayuda() -> String {
    let mut t = String::from("API de Relevo. Cada acción necesita la clave: ?clave=...  (o la cabecera X-Clave)\n\n");
    for (r, d) in ACCIONES {
        t.push_str(&format!("{r:<22} {d}\n"));
    }
    t
}

/// `%20` y `+` → espacio, etc. (para claves con caracteres especiales).
fn decodificar(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or(())
                {
                    Ok(x) => {
                        out.push(x);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::controlador;
    use std::sync::Arc;

    async fn pedir(puerto: u16, ruta: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", puerto)).await.unwrap();
        s.write_all(format!("GET {ruta} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
        let mut r = String::new();
        s.read_to_string(&mut r).await.unwrap();
        let codigo = r.split_whitespace().nth(1).unwrap().parse().unwrap();
        let cuerpo = r.split("\r\n\r\n").nth(1).unwrap_or_default().to_string();
        (codigo, cuerpo)
    }

    #[test]
    fn decodifica_claves() {
        assert_eq!(decodificar("a%20b+c"), "a b c");
        assert_eq!(decodificar("x%2"), "x%2");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn botonera() {
        let base = std::env::temp_dir().join(format!("relevo-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let medios = base.join("media");
        std::fs::create_dir_all(&medios).unwrap();
        let pc = crate::remoto::pruebas::caspar_falso(medios).await;
        let ctl = controlador::iniciar(
            &tokio::runtime::Handle::current(),
            amcp::Cliente::new("127.0.0.1", pc),
            crate::remoto::pruebas::opciones(&base),
            Arc::new(|| {}),
        );
        assert!(servir(0, "".into(), ctl.clone()).await.is_err(), "sin clave no se abre");
        let puerto = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        servir(puerto, "s3creta 1".into(), ctl.clone()).await.unwrap();

        let (c, t) = pedir(puerto, "/api").await;
        assert_eq!(c, 200);
        assert!(t.contains("/api/volver-a-tv"));
        assert_eq!(pedir(puerto, "/api/estado").await.0, 401);
        assert_eq!(pedir(puerto, "/api/estado?clave=mala").await.0, 401);
        let (c, t) = pedir(puerto, "/api/estado?clave=s3creta%201").await;
        assert_eq!(c, 200, "{t}");
        assert!(t.contains(r#""fase":"tv""#));
        assert_eq!(pedir(puerto, "/api/nada?clave=s3creta+1").await.0, 404);

        // Sin pases en ventana, iniciar el corte no puede.
        let (c, _) = pedir(puerto, "/api/iniciar-corte?clave=s3creta+1").await;
        assert_eq!(c, 409);
        assert_eq!(ctl.ver().modo, ModoTanda::Auto);

        // Con un spot en la tanda: play, y volver a TV.
        pedir(puerto, "/api/manual?clave=s3creta+1").await;
        ctl.enviar(Orden::Agregar {
            clip: "SPOT".into(),
            duracion: Duration::from_secs(10),
            destino: crate::Destino::Final,
        });
        let (_, t) = pedir(puerto, "/api/play?clave=s3creta+1").await;
        assert!(t.contains(r#""fase":"al_aire""#), "{t}");
        let (_, t) = pedir(puerto, "/api/volver-a-tv?clave=s3creta+1").await;
        assert!(t.contains(r#""fase":"tv""#), "{t}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
