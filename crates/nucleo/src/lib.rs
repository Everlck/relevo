//! Núcleo de la app de publicidad.
//!
//! - [`reproductor`]: la lógica de la tanda (qué suena, qué sigue, bucle, aleatorio,
//!   cuándo volver a TV). Es lógica pura y está cubierta por pruebas.
//! - [`controlador`]: ejecuta esa lógica contra CasparCG en segundo plano y publica
//!   una [`Instantanea`] que la interfaz dibuja.
//!
//! La interfaz nunca habla con CasparCG directamente: envía [`Orden`]es al controlador.
//! Así, más adelante, una PC remota podrá enviar las mismas órdenes por la red.

pub mod api;
pub mod archivos;
pub mod biblioteca;
pub mod controlador;
pub mod iptv;
pub mod osc;
pub mod pauta;
pub mod remoto;
pub mod reportes;
pub mod reproductor;
pub mod respaldo;
pub mod transicion;
pub mod usuarios;

pub use controlador::{
    iniciar, Controlador, Destino, Evento, Instantanea, Miniatura, ModoExternos, ModoTanda, Orden, Origen, PaseVista,
    PedidoSubida, Rol, VistaCorte, VistaRespaldo,
};
pub use iptv::{EstadoIptv, Iptv, VistaIptv};
pub use reproductor::{Bucle, Fase, ItemTanda};
pub use transicion::{TipoTransicion, Transicion};
