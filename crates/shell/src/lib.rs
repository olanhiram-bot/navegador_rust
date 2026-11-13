//! Shell mínima: la página visible, una barra de dirección y el handoff.
//!
//! La UI corre en el hilo de eframe; Chromium y su runtime de Tokio viven en un
//! hilo aparte y se hablan por canales. La shell es de solo lectura: muestra la
//! página y permite navegar, pero no hace click en controles de formulario.
//! Las acciones consecuentes siguen pasando por descriptor → tarjeta → recibo
//! en la sesión/MCP, nunca por píxeles.

pub mod app;
pub mod engine;
