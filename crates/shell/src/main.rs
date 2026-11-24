//! `navegador-shell`: shell mínima con la página visible, barra de dirección y
//! handoff al agente.
//!
//! ```text
//! navegador-shell [URL] [--log RUTA] [--perfil DIR] [--efimero] [--sin-restaurar]
//! navegador-shell --version
//! ```
//!
//! Por defecto usa un perfil persistente y restaura las pestañas de la última
//! sesión. `--efimero` arranca sin perfil guardado ni restauración.

use std::path::PathBuf;

use eframe::egui;
use shell::app::ShellApp;
use shell::startup::Startup;

fn main() -> eframe::Result<()> {
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        println!("navegador-shell {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let startup = Startup::parse(
        std::env::args().skip(1),
        PathBuf::from("navegador-recibos.jsonl"),
    )
    .unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    let options_eframe = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 780.0])
            .with_title("Navegador"),
        ..Default::default()
    };

    eframe::run_native(
        "Navegador",
        options_eframe,
        Box::new(move |cc| {
            Ok(Box::new(ShellApp::new(
                startup.options.clone(),
                startup.paths.clone(),
                cc,
            )))
        }),
    )
}
