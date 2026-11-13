//! Motor Chromium vía CDP para el MVP.
//!
//! El motor solo observa y ejecuta; la política y el binding viven en
//! `descriptor`.

mod cdp;
mod chrome;

pub use chrome::{Chrome, DispatchOutcome};
