use std::io;

/// Errores al hablar con CasparCG.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no hay conexión con CasparCG ({0})")]
    Desconectado(String),

    #[error("CasparCG no respondió a tiempo")]
    SinRespuesta,

    #[error("error de red con CasparCG: {0}")]
    Red(#[from] io::Error),

    #[error("respuesta no válida de CasparCG: {0}")]
    Protocolo(String),

    #[error("CasparCG rechazó el comando ({codigo}): {mensaje}")]
    Servidor { codigo: u16, mensaje: String },
}

impl Error {
    /// `true` si el error obliga a abrir una conexión nueva.
    pub(crate) fn rompe_conexion(&self) -> bool {
        matches!(self, Error::Red(_) | Error::SinRespuesta | Error::Protocolo(_) | Error::Desconectado(_))
    }
}

pub type Resultado<T> = Result<T, Error>;
