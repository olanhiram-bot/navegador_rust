//! Interfaz de motor y modelo de snapshot.
//!
//! El motor solo observa y ejecuta. Todo lo que decide (política, tarjeta,
//! recibo) vive fuera. El trait permite que `SessionCore` y el servidor MCP no
//! sepan si debajo hay Chromium, Servo o un motor falso de test.

use std::future::Future;
use std::time::Duration;

use descriptor::{ActionDescriptor, Provenance};
use serde::{Deserialize, Serialize};

/// Resultado de intentar ejecutar una acción aprobada.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DispatchOutcome {
    /// La petición coincidía con lo aprobado y se dejó salir.
    Sent {
        url: String,
        method: String,
        body: Option<String>,
    },
    /// El DOM cambió entre la aprobación y el despacho: no se hizo click.
    RejectedBeforeDispatch {
        approved_hash: String,
        observed_hash: String,
    },
    /// Se hizo click, pero la petición interceptada no era la aprobada.
    Blocked {
        url: String,
        method: String,
        body: Option<String>,
        reason: String,
    },
    /// No se observó ninguna petición atribuible a la acción.
    NoRequest { reason: String },
}

/// Un elemento accionable del snapshot.
///
/// `reference` es `@eN` y solo es válida dentro de su `generation`. El
/// `selector` es la ruta CSS con la que el motor vuelve a encontrar el
/// elemento; en `act` se comprueba que sigue siendo el mismo (tag, nombre y
/// posición) antes de construir el descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementRef {
    pub reference: String,
    pub generation: u32,
    pub selector: String,
    pub role: String,
    pub name: String,
    pub tag: String,
    pub x: f64,
    pub y: f64,
    pub disabled: bool,
}

impl ElementRef {
    /// Dos elementos son el mismo si coinciden en identidad y posición.
    pub fn same_element(&self, tag: &str, name: &str, x: f64, y: f64) -> bool {
        const POSITION_TOLERANCE: f64 = 2.0;
        self.tag.eq_ignore_ascii_case(tag)
            && normalize(&self.name) == normalize(name)
            && (self.x - x).abs() <= POSITION_TOLERANCE
            && (self.y - y).abs() <= POSITION_TOLERANCE
    }
}

fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Foto de la página en una generación concreta.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub generation: u32,
    pub url: String,
    pub title: String,
    pub elements: Vec<ElementRef>,
    pub text: String,
}

impl Snapshot {
    pub fn find(&self, reference: &str) -> Option<&ElementRef> {
        self.elements
            .iter()
            .find(|element| element.reference == reference)
    }

    /// Render de texto: lo que ve el agente en MCP y lo que se imprime en CLI.
    pub fn render_text(&self) -> String {
        let mut out = format!(
            "generación {} · {} · {}\n",
            self.generation,
            self.url,
            if self.title.is_empty() {
                "(sin título)"
            } else {
                &self.title
            }
        );
        for element in &self.elements {
            out.push_str(&format!(
                "{}  {:<9} {}{}\n",
                element.reference,
                element.role,
                if element.name.is_empty() {
                    "(sin nombre)"
                } else {
                    &element.name
                },
                if element.disabled {
                    "  [deshabilitado]"
                } else {
                    ""
                }
            ));
        }
        out
    }
}

/// Transporte que necesita una shell: píxeles, eventos e hit-test.
///
/// No incluye ejecutar acciones: la shell sigue pasando por `Session::act`,
/// que es lo único que firma recibos. Este trait solo mueve lo que se ve y lo
/// que se toca.
pub trait Transport {
    fn start_screencast(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn stop_screencast(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn next_event(
        &mut self,
        timeout: Duration,
    ) -> impl Future<Output = anyhow::Result<Option<serde_json::Value>>> + Send;

    fn ack_screencast_frame(
        &mut self,
        session_id: i64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Ajusta el viewport de la página al área visible de la shell.
    fn set_viewport(
        &mut self,
        width: u32,
        height: u32,
        scale: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Atrás (`-1`), adelante (`1`). `false` si no hay historial en esa dirección.
    fn history(&mut self, delta: i64) -> impl Future<Output = anyhow::Result<bool>> + Send;

    /// URL absoluta del enlace bajo un selector, si es un enlace.
    fn link_href(
        &mut self,
        selector: &str,
    ) -> impl Future<Output = anyhow::Result<Option<String>>> + Send;

    fn youtube_search_query(
        &self,
        selector: &str,
    ) -> impl Future<Output = anyhow::Result<Option<String>>> + Send;

    /// Texto compuesto por el teclado; respeta la distribución del sistema.
    fn insert_text(&mut self, text: &str) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Tecla especial (Enter, Backspace, flechas…).
    fn key(
        &mut self,
        key: &str,
        code: &str,
        virtual_key: i64,
        modifiers: u32,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn select_text(
        &self,
        start: [f64; 2],
        end: [f64; 2],
    ) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn selected_text(&self) -> impl Future<Output = anyhow::Result<String>> + Send;

    /// Click real para enfocar un campo; los botones no pasan por aquí.
    fn click_at(&mut self, x: f64, y: f64) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn scroll(
        &mut self,
        x: f64,
        y: f64,
        delta_y: f64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Selector CSS del elemento accionable bajo un punto de la página.
    fn hit_test(
        &self,
        x: f64,
        y: f64,
    ) -> impl Future<Output = anyhow::Result<Option<String>>> + Send;
}

/// El motor mínimo que la sesión necesita.
/// Resultado de buscar texto en la página.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindResult {
    pub total: usize,
    /// Índice de la coincidencia resaltada; `-1` si no hay ninguna.
    pub current: i64,
}

pub trait Engine {
    fn navigate(&mut self, url: &str) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn snapshot(
        &mut self,
        generation: u32,
    ) -> impl Future<Output = anyhow::Result<Snapshot>> + Send;

    fn extract(
        &self,
        selector: &str,
        generation: u32,
        provenance: Provenance,
    ) -> impl Future<Output = anyhow::Result<ActionDescriptor>> + Send;

    fn dispatch(
        &mut self,
        approved: &ActionDescriptor,
    ) -> impl Future<Output = anyhow::Result<DispatchOutcome>> + Send;

    /// PNG de la vista actual.
    fn screenshot(&mut self) -> impl Future<Output = anyhow::Result<Vec<u8>>> + Send;

    /// Texto visible de la página, sin interpretar.
    fn read_text(&mut self) -> impl Future<Output = anyhow::Result<String>> + Send;

    /// Selector del control que un Enter enviaría desde el campo con el foco.
    fn form_submitter(&self) -> impl Future<Output = anyhow::Result<Option<String>>> + Send {
        async { Ok(None) }
    }

    /// Valor de `target` del enlace que corresponde a un selector, si lo hay.
    fn link_target(
        &self,
        _selector: &str,
    ) -> impl Future<Output = anyhow::Result<Option<String>>> + Send {
        async { Ok(None) }
    }

    /// Fija permisos del origen: `granted` y `denied` son nombres de CDP
    /// (`notifications`, `geolocation`, `audioCapture`, `videoCapture`).
    fn set_permissions(
        &self,
        _origin: &str,
        _granted: &[String],
        _denied: &[String],
    ) -> impl Future<Output = anyhow::Result<()>> + Send {
        async { Ok(()) }
    }

    /// Busca texto en la página y resalta las coincidencias.
    fn find(
        &self,
        _text: &str,
        _index: i64,
    ) -> impl Future<Output = anyhow::Result<FindResult>> + Send {
        async {
            Ok(FindResult {
                total: 0,
                current: -1,
            })
        }
    }
}
