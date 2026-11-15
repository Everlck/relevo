use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use crate::error::{Error, Resultado};

/// Respuesta de CasparCG a un comando.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Respuesta {
    /// Código de estado: 2xx correcto, 4xx error del comando, 5xx error del servidor.
    pub codigo: u16,
    /// Primera línea completa, por ejemplo `202 PLAY OK`.
    pub cabecera: String,
    /// Líneas de datos (solo en respuestas 200 y 201).
    pub datos: Vec<String>,
}

impl Respuesta {
    pub fn es_correcta(&self) -> bool {
        (200..300).contains(&self.codigo)
    }
}

/// Lee una línea y le quita el `\r\n` final. Devuelve `None` si la conexión se cerró.
async fn leer_linea<R: AsyncBufRead + Unpin>(lector: &mut R) -> Resultado<Option<String>> {
    let mut linea = String::new();
    let leidos = lector.read_line(&mut linea).await?;
    if leidos == 0 {
        return Ok(None);
    }
    while linea.ends_with('\n') || linea.ends_with('\r') {
        linea.pop();
    }
    Ok(Some(linea))
}

/// Lee una respuesta completa siguiendo las reglas de AMCP:
/// - `200 ... OK`: varias líneas de datos hasta una línea vacía.
/// - `201 ... OK`: exactamente una línea de datos.
/// - cualquier otro código: solo la cabecera.
pub async fn leer_respuesta<R: AsyncBufRead + Unpin>(lector: &mut R) -> Resultado<Respuesta> {
    let cabecera = leer_linea(lector).await?.ok_or_else(|| Error::Desconectado("CasparCG cerró la conexión".into()))?;

    let codigo: u16 = cabecera
        .get(..3)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| Error::Protocolo(format!("cabecera inesperada: {cabecera:?}")))?;

    let mut datos = Vec::new();
    match codigo {
        200 => loop {
            match leer_linea(lector).await? {
                Some(l) if l.is_empty() => break,
                Some(l) => datos.push(l),
                None => return Err(Error::Desconectado("conexión cerrada a mitad de respuesta".into())),
            }
        },
        201 => match leer_linea(lector).await? {
            Some(l) => datos.push(l),
            None => return Err(Error::Desconectado("conexión cerrada a mitad de respuesta".into())),
        },
        _ => {}
    }

    Ok(Respuesta { codigo, cabecera, datos })
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use tokio::io::BufReader;

    async fn leer(texto: &str) -> Resultado<Respuesta> {
        let mut lector = BufReader::new(texto.as_bytes());
        leer_respuesta(&mut lector).await
    }

    #[tokio::test]
    async fn respuesta_sin_datos() {
        let r = leer("202 PLAY OK\r\n").await.unwrap();
        assert_eq!(r.codigo, 202);
        assert!(r.datos.is_empty());
        assert!(r.es_correcta());
    }

    #[tokio::test]
    async fn respuesta_201_una_linea() {
        let r = leer("201 VERSION OK\r\n2.3.3 LTS\r\n").await.unwrap();
        assert_eq!(r.datos, vec!["2.3.3 LTS"]);
    }

    #[tokio::test]
    async fn respuesta_200_varias_lineas() {
        let texto =
            "200 CLS OK\r\n\"A\" MOVIE 1 20240101000000 25 1/25\r\n\"B\" MOVIE 2 20240101000000 50 1/25\r\n\r\n";
        let r = leer(texto).await.unwrap();
        assert_eq!(r.datos.len(), 2);
    }

    #[tokio::test]
    async fn respuesta_200_vacia() {
        let r = leer("200 CLS OK\r\n\r\n").await.unwrap();
        assert!(r.datos.is_empty());
    }

    #[tokio::test]
    async fn respuesta_error() {
        let r = leer("404 PLAY FAILED\r\n").await.unwrap();
        assert_eq!(r.codigo, 404);
        assert!(!r.es_correcta());
    }

    #[tokio::test]
    async fn cabecera_invalida() {
        assert!(matches!(leer("hola\r\n").await, Err(Error::Protocolo(_))));
    }

    #[tokio::test]
    async fn conexion_cerrada() {
        assert!(matches!(leer("").await, Err(Error::Desconectado(_))));
    }
}
