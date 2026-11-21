//! Motor Chromium vía CDP para el MVP.
//!
//! El motor solo observa y ejecuta; la política y el binding viven en
//! `descriptor`, y la interfaz que consume la sesión vive en `engine`.

mod cdp;
mod chrome;

pub use chrome::{Browser, Chrome, Page, Profile};
pub use engine::{DispatchOutcome, ElementRef, Engine, FindResult, Snapshot};
