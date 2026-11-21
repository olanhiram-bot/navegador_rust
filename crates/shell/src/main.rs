//! `navegador-shell`: shell mínima con la página visible, barra de dirección y
//! handoff al agente.
//!
//! ```text
//! navegador-shell [URL] [--log RUTA]
//! ```

use std::path::PathBuf;

use eframe::egui;
use shell::app::ShellApp;

fn main() -> eframe::Result<()> {
    let mut url = "about:blank".to_string();
    let mut log_path = PathBuf::from("navegador-recibos.jsonl");

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--log" => {
                log_path = PathBuf::from(args.next().unwrap_or_else(|| {
                    eprintln!("--log necesita una ruta");
                    std::process::exit(2);
                }));
            }
            other => url = shell::app::browser_url(other),
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 780.0])
            .with_title("Navegador"),
        ..Default::default()
    };

    eframe::run_native(
        "Navegador",
        options,
        Box::new(move |cc| Ok(Box::new(ShellApp::new(url, log_path, cc)))),
    )
}
