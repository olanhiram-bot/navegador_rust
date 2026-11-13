//! Shell mínima: pestañas, página visible, omnibox, tarjeta y panel del agente.
//!
//! La UI corre en el hilo de eframe; Chromium y su runtime de Tokio viven en un
//! hilo aparte y se hablan por canales. Cada pestaña es una `Session<Chrome>`.
//!
//! Un click en la página no se reenvía: se hace hit-test en el mundo aislado,
//! se cruza con el snapshot vivo para obtener la ref `@eN` y se llama a
//! `Session::act`. La acción ejecutada pasa por descriptor → política → tarjeta
//! → recibo, nunca por píxeles.

pub mod app;
pub mod engine;
