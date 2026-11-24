//! Opciones de arranque compartidas por las dos ventanas de escritorio.

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::{app::browser_url, engine::Options, estado};

pub struct Startup {
    pub options: Options,
    pub paths: estado::Paths,
    // Mantiene vivos los recibos de la sesión privada hasta que cierre la ventana.
    _temporary_state: Option<tempfile::TempDir>,
}

impl Startup {
    pub fn parse(mut args: impl Iterator<Item = String>, default_log: PathBuf) -> Result<Self> {
        let mut url = None;
        let mut log = None;
        let mut profile = estado::data_dir().map(|dir| dir.join("perfil"));
        let mut ephemeral = false;
        let mut restore = true;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--log" | "--perfil" => {
                    let Some(path) = args.next().filter(|value| !value.starts_with("--")) else {
                        bail!("{arg} necesita una ruta");
                    };
                    if arg == "--log" {
                        log = Some(PathBuf::from(path));
                    } else {
                        profile = Some(PathBuf::from(path));
                    }
                }
                "--efimero" => ephemeral = true,
                "--sin-restaurar" => restore = false,
                other if other.starts_with("--") => bail!("opción desconocida: {other}"),
                other => url = Some(browser_url(other)),
            }
        }

        if ephemeral {
            profile = None;
        }
        let paths = if profile.is_some() {
            estado::Paths {
                session: restore.then(estado::session_path).flatten(),
                settings: estado::settings_path(),
                history: estado::history_path(),
            }
        } else {
            estado::Paths::default()
        };
        let initial_urls = match url {
            Some(url) => vec![url],
            None => paths
                .session
                .as_deref()
                .and_then(estado::load)
                .map(|state| state.ordered_urls())
                .filter(|urls| !urls.is_empty())
                .unwrap_or_else(|| vec!["about:blank".into()]),
        };
        let temporary_state = ephemeral.then(tempfile::tempdir).transpose()?;
        let log_path = log.unwrap_or_else(|| {
            temporary_state
                .as_ref()
                .map(|dir| dir.path().join("recibos.jsonl"))
                .unwrap_or(default_log)
        });
        Ok(Self {
            options: Options {
                initial_urls,
                log_path,
                profile,
                downloads: estado::downloads_dir(),
            },
            paths,
            _temporary_state: temporary_state,
        })
    }
}
