//! Estado del navegador entre arranques: rutas del perfil y de la sesión, y
//! lectura/escritura atómica de las pestañas abiertas.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Pestañas de la última sesión, para restaurarlas al arrancar.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionState {
    pub tabs: Vec<TabState>,
    #[serde(default)]
    pub active: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabState {
    pub url: String,
    #[serde(default)]
    pub title: String,
}

impl SessionState {
    /// URLs en orden, con la pestaña activa al final: el motor activa la
    /// última que abre.
    pub fn ordered_urls(&self) -> Vec<String> {
        let active = self.active.min(self.tabs.len().saturating_sub(1));
        let mut urls: Vec<String> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != active)
            .map(|(_, tab)| tab.url.clone())
            .collect();
        if let Some(tab) = self.tabs.get(active) {
            urls.push(tab.url.clone());
        }
        urls
    }
}

/// `$XDG_DATA_HOME/navegador` (o `~/.local/share/navegador`).
pub fn data_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(home()?).join(".local/share"),
    };
    Some(base.join("navegador"))
}

/// `$XDG_STATE_HOME/navegador` (o `~/.local/state/navegador`).
pub fn state_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(home()?).join(".local/state"),
    };
    Some(base.join("navegador"))
}

pub fn session_path() -> Option<PathBuf> {
    Some(state_dir()?.join("sesion.json"))
}

fn home() -> Option<String> {
    std::env::var("HOME").ok().filter(|home| !home.is_empty())
}

pub fn load(path: &Path) -> Option<SessionState> {
    let text = std::fs::read_to_string(path).ok()?;
    let state: SessionState = serde_json::from_str(&text).ok()?;
    let state = SessionState {
        tabs: state
            .tabs
            .into_iter()
            .filter(|tab| is_restorable(&tab.url))
            .collect(),
        active: state.active,
    };
    (!state.tabs.is_empty()).then_some(state)
}

/// Guarda sin dejar un archivo a medias: escribe a un temporal y lo renombra.
pub fn save(path: &Path, state: &SessionState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(state)?)?;
    std::fs::rename(&temporary, path)
}

/// No restauramos páginas en blanco repetidas ni esquemas internos que no
/// aportan nada tras reiniciar.
pub fn is_restorable(url: &str) -> bool {
    let url = url.trim();
    !url.is_empty()
        && !url.starts_with("about:")
        && !url.starts_with("chrome://")
        && !url.starts_with("chrome-error://")
        && !url.starts_with("data:")
        && url != "null"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_pestana_activa_queda_al_final() {
        let state = SessionState {
            tabs: vec![
                TabState {
                    url: "https://a.test/".into(),
                    title: "A".into(),
                },
                TabState {
                    url: "https://b.test/".into(),
                    title: "B".into(),
                },
                TabState {
                    url: "https://c.test/".into(),
                    title: "C".into(),
                },
            ],
            active: 1,
        };
        assert_eq!(
            state.ordered_urls(),
            vec!["https://a.test/", "https://c.test/", "https://b.test/"]
        );
    }

    #[test]
    fn la_activa_fuera_de_rango_no_pierde_pestanas() {
        let state = SessionState {
            tabs: vec![TabState {
                url: "https://a.test/".into(),
                title: String::new(),
            }],
            active: 9,
        };
        assert_eq!(state.ordered_urls(), vec!["https://a.test/"]);
    }

    #[test]
    fn guardar_y_recuperar_conserva_las_pestanas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("estado/sesion.json");
        let state = SessionState {
            tabs: vec![
                TabState {
                    url: "https://a.test/".into(),
                    title: "A".into(),
                },
                TabState {
                    url: "about:blank".into(),
                    title: String::new(),
                },
            ],
            active: 1,
        };
        save(&path, &state).unwrap();
        let loaded = load(&path).expect("la sesión debe leerse");
        assert_eq!(
            loaded.tabs,
            vec![TabState {
                url: "https://a.test/".into(),
                title: "A".into(),
            }],
            "las páginas en blanco no se restauran"
        );
    }

    #[test]
    fn una_sesion_vacia_no_se_restaura() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sesion.json");
        save(&path, &SessionState::default()).unwrap();
        assert!(load(&path).is_none());
    }
}
