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
use shell::engine::Options;

fn main() -> eframe::Result<()> {
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        println!("navegador-shell {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let mut url: Option<String> = None;
    let mut log_path = PathBuf::from("navegador-recibos.jsonl");
    let mut profile = shell::estado::data_dir().map(|dir| dir.join("perfil"));
    let mut restore = true;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--log" => {
                log_path = PathBuf::from(args.next().unwrap_or_else(|| {
                    eprintln!("--log necesita una ruta");
                    std::process::exit(2);
                }));
            }
            "--perfil" => {
                profile = Some(PathBuf::from(args.next().unwrap_or_else(|| {
                    eprintln!("--perfil necesita un directorio");
                    std::process::exit(2);
                })));
            }
            "--efimero" => {
                profile = None;
                restore = false;
            }
            "--sin-restaurar" => restore = false,
            other => url = Some(shell::app::browser_url(other)),
        }
    }

    let session_path = if restore && profile.is_some() {
        shell::estado::session_path()
    } else {
        None
    };
    let settings_path = if profile.is_some() {
        shell::estado::settings_path()
    } else {
        None
    };
    let history_path = if profile.is_some() {
        shell::estado::history_path()
    } else {
        None
    };
    let initial_urls = match url {
        Some(url) => vec![url],
        None => session_path
            .as_deref()
            .and_then(shell::estado::load)
            .map(|state| state.ordered_urls())
            .unwrap_or_else(|| vec!["about:blank".to_string()]),
    };

    let paths = shell::estado::Paths {
        session: session_path,
        settings: settings_path,
        history: history_path,
    };
    let options = Options {
        initial_urls,
        log_path,
        profile,
        downloads: shell::estado::downloads_dir(),
    };
    let options_eframe = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 780.0])
            .with_title("Navegador"),
        ..Default::default()
    };

    eframe::run_native(
        "Navegador",
        options_eframe,
        Box::new(move |cc| Ok(Box::new(ShellApp::new(options, paths, cc)))),
    )
}
