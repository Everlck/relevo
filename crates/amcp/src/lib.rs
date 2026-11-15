//! Cliente AMCP para controlar CasparCG Server.
//!
//! AMCP es el protocolo de texto de CasparCG: se envía una línea de comando
//! terminada en `\r\n` por TCP (puerto 5250) y el servidor responde con un
//! código de estado y, a veces, líneas de datos.
//!
//! Este crate se encarga de:
//! - conectar y **reconectar solo** si CasparCG se reinicia o se cae la red,
//! - leer las respuestas sin desincronizarse,
//! - ofrecer comandos con tipos (`play`, `loadbg`, `stop`, `clear`, `cls`...).

mod cliente;
mod error;
mod medio;
mod respuesta;
mod rutas;

pub use cliente::{Cliente, OpcionesReproduccion};
pub use error::{Error, Resultado};
pub use medio::{Medio, TipoMedio};
pub use respuesta::Respuesta;
pub use rutas::Rutas;

/// Pone un parámetro entre comillas y escapa lo necesario para AMCP.
///
/// También cambia `\` por `/`, porque CasparCG usa `/` para las subcarpetas
/// aunque corra en Windows.
pub fn citar(parametro: &str) -> String {
    let normalizado = parametro.replace('\\', "/");
    let mut salida = String::with_capacity(normalizado.len() + 2);
    salida.push('"');
    for c in normalizado.chars() {
        if c == '"' {
            salida.push_str("\\\"");
        } else {
            salida.push(c);
        }
    }
    salida.push('"');
    salida
}

#[cfg(test)]
mod pruebas {
    use super::citar;

    #[test]
    fn citar_simple() {
        assert_eq!(citar("SPOT1"), "\"SPOT1\"");
    }

    #[test]
    fn citar_con_espacios_y_subcarpetas_windows() {
        assert_eq!(citar(r"Clientes\Ferreteria X\spot 30s"), "\"Clientes/Ferreteria X/spot 30s\"");
    }

    #[test]
    fn citar_escapa_comillas() {
        assert_eq!(citar("a\"b"), "\"a\\\"b\"");
    }
}
