//! Lanzador de Chromium y operaciones de alto nivel: navegar, extraer el
//! descriptor y despachar una acción verificada.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use descriptor::{ActionDescriptor, Provenance, RawAction};
use engine::{DispatchOutcome, ElementRef, Engine, Snapshot};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

use crate::cdp::Cdp;

const EXTRACT_JS: &str = include_str!("extract.js");
const SNAPSHOT_JS: &str = include_str!("snapshot.js");
const ELEMENT_NAME_JS: &str = include_str!("element-name.js");
const HIT_JS: &str = include_str!("hit.js");
const SUBMITTER_JS: &str = include_str!("submitter.js");

/// Perfil de Chromium: efímero (directorio temporal que se borra al cerrar) o
/// persistente (conserva cookies e inicios de sesión entre arranques).
pub enum Profile {
    Ephemeral(tempfile::TempDir),
    Persistent(PathBuf),
}

impl Profile {
    pub fn ephemeral() -> Result<Self> {
        Ok(Self::Ephemeral(
            tempfile::tempdir().context("no se pudo crear el perfil temporal")?,
        ))
    }

    pub fn persistent(path: impl Into<PathBuf>) -> Self {
        Self::Persistent(path.into())
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Ephemeral(dir) => dir.path(),
            Self::Persistent(path) => path,
        }
    }

    pub fn is_ephemeral(&self) -> bool {
        matches!(self, Self::Ephemeral(_))
    }
}

impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Profile")
            .field("path", &self.path())
            .field("ephemeral", &self.is_ephemeral())
            .finish()
    }
}

/// Un `target` de tipo página dentro del navegador compartido.
#[derive(Debug, Clone)]
pub struct Page {
    id: String,
    ws: String,
}

impl Page {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn ws(&self) -> &str {
        &self.ws
    }
}

/// Proceso Chromium que comparten todas las pestañas de una sesión.
///
/// Un solo proceso (y un solo perfil) significa cookies compartidas entre
/// pestañas, menos RAM y la posibilidad de reutilizar un perfil persistente.
pub struct Browser {
    child: Child,
    profile: Profile,
    /// Candado exclusivo del perfil persistente: dos instancias con el mismo
    /// `--user-data-dir` no pueden coexistir.
    _lock: Option<std::fs::File>,
    /// Display virtual para el modo headful. Sin él, Chromium corre headless y
    /// los captchas lo identifican como bot.
    _display: Option<VirtualDisplay>,
    port: u16,
}

impl Browser {
    pub async fn launch(profile: Profile, audio: bool) -> Result<Self> {
        let binary = find_binary()?;
        let profile_path = profile.path().to_path_buf();
        if let Profile::Persistent(path) = &profile {
            std::fs::create_dir_all(path)
                .with_context(|| format!("no se pudo crear {}", path.display()))?;
        }
        let lock = if profile.is_ephemeral() {
            None
        } else {
            Some(lock_profile(&profile_path)?)
        };
        // Un DevToolsActivePort de una ejecución anterior apuntaría a un puerto
        // muerto o, peor, al de otro programa.
        let _ = std::fs::remove_file(profile_path.join("DevToolsActivePort"));

        // Con Xvfb disponible, Chromium corre headful dentro de un display
        // virtual: los sitios ven un navegador normal. Sin Xvfb se mantiene el
        // modo headless (útil para CI y para quien no quiera instalar X11).
        let display = if use_virtual_display() {
            match VirtualDisplay::start().await {
                Ok(display) => Some(display),
                Err(error) => {
                    eprintln!("Xvfb no arrancó ({error:#}); sigo en headless");
                    None
                }
            }
        } else {
            None
        };

        let mut command = Command::new(&binary);
        command
            .arg("--disable-dev-shm-usage")
            .arg("--disable-extensions")
            .arg("--disable-background-networking")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--hide-scrollbars")
            .arg("--window-size=1600,1000")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile_path.display()))
            .arg("about:blank")
            .kill_on_drop(true);

        if !audio {
            command.arg("--mute-audio");
        }

        match &display {
            Some(display) => {
                command
                    .env("DISPLAY", &display.display)
                    .env_remove("WAYLAND_DISPLAY")
                    .arg("--ozone-platform=x11")
                    .arg("--window-position=0,0");
            }
            None => {
                command.arg("--headless=new").arg("--disable-gpu");
            }
        };
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        // Grupo de procesos propio: al cerrar la sesión matamos también a los
        // zygotes y renderers, que si no quedan huérfanos.
        #[cfg(unix)]
        command.process_group(0);

        // Si la shell muere de golpe (SIGKILL, SIGTERM sin destructores), el
        // navegador no debe quedarse vivo con el perfil bloqueado.
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("no se pudo lanzar {}", binary.display()))?;
        let port =
            wait_for_port(&mut child, &binary, &profile_path, Duration::from_secs(20)).await?;

        Ok(Browser {
            child,
            profile,
            _lock: lock,
            _display: display,
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn profile_dir(&self) -> &Path {
        self.profile.path()
    }

    /// Abre una pestaña nueva dentro de este proceso.
    pub async fn new_page(&self) -> Result<Page> {
        let body = http_request(self.port, "PUT", "/json/new?url=about:blank").await?;
        let target: Value = serde_json::from_str(&body).context("/json/new no es JSON")?;
        let id = target
            .get("id")
            .and_then(Value::as_str)
            .context("el target nuevo no trae id")?
            .to_string();
        let ws = target
            .get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .context("el target nuevo no trae WebSocket")?
            .to_string();
        // Con un perfil persistente, Chromium puede restaurar pestañas de la
        // ejecución anterior y dejar la recién creada en segundo plano; una
        // pestaña de fondo no dibuja, así que el screencast se quedaría mudo.
        let _ = self.activate_page(&id).await;
        Ok(Page { id, ws })
    }

    /// Trae al frente la pestaña indicada. Chromium no dibuja una pestaña de
    /// fondo, así que su screencast se quedaría congelado.
    pub async fn activate_page(&self, id: &str) -> Result<()> {
        http_request(self.port, "GET", &format!("/json/activate/{id}")).await?;
        Ok(())
    }

    pub async fn close_page(&self, id: &str) -> Result<()> {
        http_request(self.port, "GET", &format!("/json/close/{id}")).await?;
        Ok(())
    }

    /// Cierre ordenado: Chromium vuelca cookies e historial al perfil, igual
    /// que al cerrar una ventana.
    pub async fn close(&self) -> Result<()> {
        let version = http_request(self.port, "GET", "/json/version").await?;
        let version: Value = serde_json::from_str(&version).context("/json/version no es JSON")?;
        let ws = version
            .get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .context("/json/version sin WebSocket")?;
        let cdp = Cdp::connect(ws).await?;
        cdp.call("Browser.close", json!({})).await.ok();
        Ok(())
    }

    fn wait_exit(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            // SIGTERM primero: Chromium alcanza a volcar cookies e historial al
            // perfil. Si no termina, el grupo entero cae con SIGKILL.
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            self.wait_exit(Duration::from_secs(10));
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }
}

/// Chromium headless controlado por CDP.
pub struct Chrome {
    browser: Arc<Browser>,
    page: Page,
    cdp: Cdp,
    fetch_armed: bool,
    interactive_navigation: bool,
}

impl Chrome {
    pub async fn launch() -> Result<Self> {
        Self::launch_with_audio(false).await
    }

    pub async fn launch_with_audio(audio: bool) -> Result<Self> {
        let browser = Arc::new(Browser::launch(Profile::ephemeral()?, audio).await?);
        Self::attach(browser).await
    }

    /// Conecta una pestaña nueva al navegador compartido.
    pub async fn attach(browser: Arc<Browser>) -> Result<Self> {
        let page = browser.new_page().await?;
        let cdp = Cdp::connect(page.ws()).await?;
        Ok(Chrome {
            browser,
            page,
            cdp,
            fetch_armed: false,
            interactive_navigation: false,
        })
    }

    pub async fn launch_for_shell(profile: Profile) -> Result<Self> {
        let browser = Arc::new(Browser::launch(profile, true).await?);
        Self::attach_for_shell(browser).await
    }

    /// Pestaña interactiva (navegación del usuario) dentro de un navegador ya
    /// lanzado.
    pub async fn attach_for_shell(browser: Arc<Browser>) -> Result<Self> {
        let mut chrome = Self::attach(browser).await?;
        chrome.interactive_navigation = true;
        Ok(chrome)
    }

    pub fn port(&self) -> u16 {
        self.browser.port()
    }

    /// Directorio del perfil en uso.
    pub fn profile_dir(&self) -> &std::path::Path {
        self.browser.profile_dir()
    }

    /// Proceso Chromium que aloja esta pestaña.
    pub fn browser(&self) -> &Arc<Browser> {
        &self.browser
    }

    /// Identificador del target de esta pestaña.
    pub fn page_id(&self) -> &str {
        self.page.id()
    }

    pub fn pid(&self) -> Option<u32> {
        self.browser.pid()
    }

    pub async fn navigate(&mut self, url: &str) -> Result<()> {
        // Interceptar durante una navegación principal cuelga el renderer:
        // el documento queda pausado sin nadie que lo continúe. La
        // interceptación vive solo alrededor del despacho.
        if self.fetch_armed {
            self.cdp.call("Fetch.disable", json!({})).await?;
            self.fetch_armed = false;
        }
        self.cdp.call("Page.enable", json!({})).await?;
        self.cdp.call("Runtime.enable", json!({})).await?;
        let previous_loader = if self.interactive_navigation {
            self.cdp.call("Page.getFrameTree", json!({})).await?["frameTree"]["frame"]["loaderId"]
                .clone()
        } else {
            Value::Null
        };
        let navigation = self
            .cdp
            .call("Page.navigate", json!({ "url": url }))
            .await
            .with_context(|| format!("no se pudo navegar a {url}"))?;

        if let Some(reason) = navigation.get("errorText").and_then(Value::as_str) {
            bail!("Chromium no pudo cargar la página ({reason})");
        }
        let same_document = navigation["loaderId"].is_null();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if Instant::now() > deadline {
                bail!("la página {url} no terminó de cargar");
            }
            let value = self
                .cdp
                .call(
                    "Runtime.evaluate",
                    json!({
                        "expression": "document.readyState",
                        "returnByValue": true,
                    }),
                )
                .await?;
            let ready = value["result"]["value"].as_str();
            if self.interactive_navigation && matches!(ready, Some("interactive" | "complete")) {
                let frame_tree = self.cdp.call("Page.getFrameTree", json!({})).await?;
                if same_document || frame_tree["frameTree"]["frame"]["loaderId"] != previous_loader
                {
                    return Ok(());
                }
            } else if !self.interactive_navigation && ready == Some("complete") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // Los consumidores en modo agente toman el snapshot tras la carga.
        tokio::time::sleep(Duration::from_millis(120)).await;
        Ok(())
    }

    /// Lee el DOM en un mundo aislado y sella el descriptor.
    pub async fn extract(
        &self,
        selector: &str,
        generation: u32,
        provenance: Provenance,
    ) -> Result<ActionDescriptor> {
        let expression = format!(
            "({})({}, {ELEMENT_NAME_JS})",
            EXTRACT_JS,
            serde_json::to_string(selector).expect("selector serializable")
        );
        let text = self.evaluate_isolated(&expression).await?;

        let value: Value = serde_json::from_str(&text).context("JSON de extracción inválido")?;
        if let Some(error) = value.get("error").and_then(Value::as_str) {
            bail!("extracción: {error} (selector {selector:?})");
        }
        let raw: RawAction = serde_path_to_error::deserialize(value).map_err(|error| {
            anyhow::anyhow!(
                "no se pudo interpretar el campo «{}» de la acción",
                error.path()
            )
        })?;
        Ok(ActionDescriptor::from_raw(raw, provenance, generation))
    }

    /// Selector CSS del elemento accionable bajo el punto, en el mundo aislado.
    /// La shell lo cruza con el snapshot para obtener su ref `@eN`.
    pub async fn hit_test(&self, x: f64, y: f64) -> Result<Option<String>> {
        let expression = format!("({HIT_JS})({x}, {y})");
        let text = self.evaluate_isolated(&expression).await?;
        let value: Value = serde_json::from_str(&text).context("JSON de hit-test inválido")?;
        Ok(value
            .get("selector")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    /// Selector del botón que enviaría el formulario del campo con el foco.
    /// `None` si el foco no está en un formulario o no hay control de envío.
    pub async fn form_submitter(&self) -> Result<Option<String>> {
        let expression = format!("({SUBMITTER_JS})()");
        let text = self.evaluate_isolated(&expression).await?;
        let value: Value = serde_json::from_str(&text).context("JSON de formulario inválido")?;
        Ok(value
            .get("selector")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    /// URL absoluta del enlace que corresponde a un selector, si lo hay.
    pub async fn link_href(&self, selector: &str) -> Result<Option<String>> {
        let selector = serde_json::to_string(selector).context("serializar el selector")?;
        let expression = format!(
            "(function(s){{const el=document.querySelector(s); if(!el) return null; \
             const a=el.closest('a[href]'); return JSON.stringify(a ? a.href : null);}})({selector})"
        );
        let text = self.evaluate_isolated(&expression).await?;
        let value: Value = serde_json::from_str(&text).context("JSON de enlace inválido")?;
        Ok(value.as_str().map(str::to_string))
    }

    /// Texto compuesto por el teclado (respeta la distribución activa).
    pub async fn insert_text(&self, text: &str) -> Result<()> {
        self.cdp
            .call("Input.insertText", json!({ "text": text }))
            .await?;
        Ok(())
    }

    /// Tecla especial (Enter, Backspace, flechas…). El texto imprimible va por
    /// `insert_text`; aquí solo se despacha down + up.
    pub async fn key(&self, key: &str, code: &str, virtual_key: i64, modifiers: u32) -> Result<()> {
        for event_type in ["keyDown", "keyUp"] {
            self.cdp
                .call(
                    "Input.dispatchKeyEvent",
                    json!({
                        "type": event_type,
                        "key": key,
                        "code": code,
                        "windowsVirtualKeyCode": virtual_key,
                        "nativeVirtualKeyCode": virtual_key,
                        "text": if event_type == "keyDown" && code == "Space" && modifiers == 0 { " " } else { "" },
                        "modifiers": modifiers,
                    }),
                )
                .await?;
        }
        Ok(())
    }

    /// Click real, solo para enfocar campos de texto y desplegables. Los
    /// botones nunca pasan por aquí: van por descriptor → tarjeta → recibo.
    pub async fn click_at(&self, x: f64, y: f64) -> Result<()> {
        for event_type in ["mousePressed", "mouseReleased"] {
            self.cdp
                .call(
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": event_type,
                        "x": x,
                        "y": y,
                        "button": "left",
                        "clickCount": 1,
                    }),
                )
                .await?;
        }
        Ok(())
    }

    /// Foto de los elementos accionables, con refs `@eN` ligadas a `generation`.
    pub async fn snapshot(&mut self, generation: u32) -> Result<Snapshot> {
        let text = self
            .evaluate_isolated(&format!("({SNAPSHOT_JS})({ELEMENT_NAME_JS})"))
            .await?;
        let value: Value = serde_json::from_str(&text).context("JSON de snapshot inválido")?;

        let url = value
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let page_text = value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let raw_elements = value
            .get("elements")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut elements = Vec::with_capacity(raw_elements.len());
        for (index, raw) in raw_elements.iter().enumerate() {
            let selector = raw
                .get("selector")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if selector.is_empty() {
                continue;
            }
            elements.push(ElementRef {
                reference: format!("@e{}", index + 1),
                generation,
                selector: selector.to_string(),
                role: raw
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("clickable")
                    .to_string(),
                name: raw
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                tag: raw
                    .get("tag")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                x: raw.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                y: raw.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                disabled: raw
                    .get("disabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }

        Ok(Snapshot {
            generation,
            url,
            title,
            elements,
            text: page_text,
        })
    }

    pub async fn read_text(&mut self) -> Result<String> {
        let text = self
            .evaluate_isolated("JSON.stringify(document.body ? document.body.innerText : '')")
            .await?;
        let value: Value =
            serde_json::from_str(&text).context("el texto de la página no es JSON")?;
        Ok(value.as_str().unwrap_or_default().to_string())
    }

    pub async fn screenshot(&mut self) -> Result<Vec<u8>> {
        let response = self
            .cdp
            .call("Page.captureScreenshot", json!({ "format": "png" }))
            .await?;
        let data = response
            .get("data")
            .and_then(Value::as_str)
            .context("la captura no devolvió datos")?;
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .context("la captura no es base64 válido")
    }

    /// Empieza a transmitir frames de la página. La shell los pinta; el motor
    /// solo transporta píxeles, no decide nada.
    pub async fn start_screencast(&self) -> Result<()> {
        self.cdp
            .call(
                "Page.startScreencast",
                json!({
                    "format": "jpeg",
                    "quality": 92,
                    "everyNthFrame": 1,
                }),
            )
            .await?;
        Ok(())
    }

    /// Ajusta la página al área visible de la shell. `width`/`height` son CSS
    /// píxeles y `scale` el devicePixelRatio: a 2 el texto llega nítido aunque
    /// la textura se dibuje más pequeña.
    pub async fn set_viewport(&self, width: u32, height: u32, scale: f64) -> Result<()> {
        let scale = scale.clamp(1.0, 3.0);
        self.cdp
            .call(
                "Emulation.setDeviceMetricsOverride",
                json!({
                    "width": width.max(320),
                    "height": height.max(240),
                    "deviceScaleFactor": scale,
                    "scale": scale,
                    "dontSetVisibleSize": true,
                    "mobile": false,
                }),
            )
            .await?;
        self.cdp
            .call(
                "Emulation.setVisibleSize",
                json!({
                    "width": (width.max(320) as f64 * scale).round() as u32,
                    "height": (height.max(240) as f64 * scale).round() as u32,
                }),
            )
            .await?;
        Ok(())
    }

    /// Navega en el historial de la página. `false` si no hay entrada en esa
    /// dirección.
    pub async fn history(&self, delta: i64) -> Result<bool> {
        let history = self
            .cdp
            .call("Page.getNavigationHistory", json!({}))
            .await?;
        let current = history
            .get("currentIndex")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let Some(index) = current
            .checked_add(delta)
            .and_then(|index| usize::try_from(index).ok())
        else {
            return Ok(false);
        };
        let entry_id = history
            .get("entries")
            .and_then(Value::as_array)
            .and_then(|entries| entries.get(index))
            .and_then(|entry| entry.get("id"))
            .and_then(Value::as_i64);

        let Some(entry_id) = entry_id else {
            return Ok(false);
        };
        self.cdp
            .call(
                "Page.navigateToHistoryEntry",
                json!({ "entryId": entry_id }),
            )
            .await?;
        Ok(true)
    }

    pub async fn stop_screencast(&self) -> Result<()> {
        self.cdp.call("Page.stopScreencast", json!({})).await?;
        Ok(())
    }

    /// Siguiente evento CDP, con timeout. Los eventos se bufferean aparte de
    /// las respuestas, así que esto no interfiere con `call`.
    pub async fn next_event(&mut self, timeout: Duration) -> Result<Option<Value>> {
        self.cdp.next_event(timeout).await
    }

    /// Sin ack, Chromium deja de enviar frames.
    pub async fn ack_screencast_frame(&self, session_id: i64) -> Result<()> {
        self.cdp
            .call(
                "Page.screencastFrameAck",
                json!({ "sessionId": session_id }),
            )
            .await?;
        Ok(())
    }

    /// Rueda del ratón en coordenadas CSS de la página. No hace click: la
    /// shell es de solo lectura y las acciones pasan por la tarjeta.
    pub async fn scroll(&self, x: f64, y: f64, delta_y: f64) -> Result<()> {
        self.cdp
            .call(
                "Input.dispatchMouseEvent",
                json!({
                    "type": "mouseWheel",
                    "x": x,
                    "y": y,
                    "deltaX": 0.0,
                    "deltaY": delta_y,
                }),
            )
            .await?;
        Ok(())
    }

    /// Evalúa en el mundo aislado y devuelve el valor como texto JSON.
    async fn evaluate_isolated(&self, expression: &str) -> Result<String> {
        let frame = self
            .cdp
            .call("Page.getFrameTree", json!({}))
            .await?
            .get("frameTree")
            .and_then(|t| t.get("frame"))
            .and_then(|f| f.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("no se pudo obtener el frame principal")?;

        let world = self
            .cdp
            .call(
                "Page.createIsolatedWorld",
                json!({ "frameId": frame, "worldName": "navegador" }),
            )
            .await?;
        let context_id = world
            .get("executionContextId")
            .and_then(Value::as_i64)
            .context("no se pudo crear el mundo aislado")?;

        let response = self
            .cdp
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "contextId": context_id,
                    "returnByValue": true,
                    "awaitPromise": false,
                }),
            )
            .await?;

        if let Some(details) = response.get("exceptionDetails") {
            bail!("la evaluación falló en JS: {details}");
        }
        let text = response
            .get("result")
            .and_then(|r| r.get("value"))
            .context("la evaluación no devolvió un valor")?;
        Ok(match text {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
    }

    /// Re-verifica el descriptor, hace click real e intercepta la petición
    /// antes de que salga. Si no coincide con lo aprobado, la aborta.
    pub async fn dispatch(&mut self, approved: &ActionDescriptor) -> Result<DispatchOutcome> {
        let observed = self
            .extract(
                &approved.target.selector,
                approved.generation,
                approved.provenance,
            )
            .await?;
        if observed.hash_hex != approved.hash_hex {
            return Ok(DispatchOutcome::RejectedBeforeDispatch {
                approved_hash: approved.hash_hex.clone(),
                observed_hash: observed.hash_hex,
            });
        }

        if !self.fetch_armed {
            self.cdp
                .call(
                    "Fetch.enable",
                    json!({ "patterns": [{ "urlPattern": "*", "requestStage": "Request" }] }),
                )
                .await?;
            self.fetch_armed = true;
        }

        let (x, y) = (approved.target.x, approved.target.y);
        for event_type in ["mousePressed", "mouseReleased"] {
            self.cdp
                .call(
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": event_type,
                        "x": x,
                        "y": y,
                        "button": "left",
                        "clickCount": 1,
                    }),
                )
                .await?;
        }

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(DispatchOutcome::NoRequest {
                    reason: "no se observó ninguna petición tras el click".into(),
                });
            }
            let Some(event) = self.cdp.next_event(left).await? else {
                return Ok(DispatchOutcome::NoRequest {
                    reason: "la sesión CDP terminó antes de observar la petición".into(),
                });
            };

            match event.get("method").and_then(Value::as_str) {
                Some("Fetch.requestPaused") => {
                    let params = &event["params"];
                    let request_id = params
                        .get("requestId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let request = &params["request"];
                    let url = request
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let method = request
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let body = request
                        .get("postData")
                        .and_then(Value::as_str)
                        .map(str::to_string);

                    let resource_type = params
                        .get("resourceType")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    // Candidata a "la acción": una navegación de documento o un
                    // POST. Si la página cambió el destino, igual cae aquí y el
                    // binding la rechaza.
                    let is_candidate = resource_type.eq_ignore_ascii_case("Document")
                        || method.eq_ignore_ascii_case("POST");

                    if !is_candidate {
                        self.continue_request(&request_id).await?;
                        continue;
                    }

                    match descriptor::verify_wire(approved, &url, &method, body.as_deref()) {
                        Ok(()) => {
                            self.continue_request(&request_id).await?;
                            return Ok(DispatchOutcome::Sent { url, method, body });
                        }
                        Err(error) => {
                            self.fail_request(&request_id).await?;
                            return Ok(DispatchOutcome::Blocked {
                                url,
                                method,
                                body,
                                reason: error.to_string(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    async fn continue_request(&self, request_id: &str) -> Result<()> {
        self.cdp
            .call("Fetch.continueRequest", json!({ "requestId": request_id }))
            .await?;
        Ok(())
    }

    async fn fail_request(&self, request_id: &str) -> Result<()> {
        self.cdp
            .call(
                "Fetch.failRequest",
                json!({ "requestId": request_id, "errorReason": "Aborted" }),
            )
            .await?;
        Ok(())
    }
}

impl Engine for Chrome {
    fn navigate(&mut self, url: &str) -> impl std::future::Future<Output = Result<()>> + Send {
        Chrome::navigate(self, url)
    }

    fn snapshot(
        &mut self,
        generation: u32,
    ) -> impl std::future::Future<Output = Result<Snapshot>> + Send {
        Chrome::snapshot(self, generation)
    }

    fn extract(
        &self,
        selector: &str,
        generation: u32,
        provenance: Provenance,
    ) -> impl std::future::Future<Output = Result<ActionDescriptor>> + Send {
        Chrome::extract(self, selector, generation, provenance)
    }

    fn dispatch(
        &mut self,
        approved: &ActionDescriptor,
    ) -> impl std::future::Future<Output = Result<DispatchOutcome>> + Send {
        Chrome::dispatch(self, approved)
    }

    fn screenshot(&mut self) -> impl std::future::Future<Output = Result<Vec<u8>>> + Send {
        Chrome::screenshot(self)
    }

    fn read_text(&mut self) -> impl std::future::Future<Output = Result<String>> + Send {
        Chrome::read_text(self)
    }

    fn form_submitter(&self) -> impl std::future::Future<Output = Result<Option<String>>> + Send {
        Chrome::form_submitter(self)
    }
}

impl engine::Transport for Chrome {
    fn start_screencast(&mut self) -> impl std::future::Future<Output = Result<()>> + Send {
        Chrome::start_screencast(self)
    }

    fn stop_screencast(&mut self) -> impl std::future::Future<Output = Result<()>> + Send {
        Chrome::stop_screencast(self)
    }

    fn next_event(
        &mut self,
        timeout: Duration,
    ) -> impl std::future::Future<Output = Result<Option<Value>>> + Send {
        Chrome::next_event(self, timeout)
    }

    fn ack_screencast_frame(
        &mut self,
        session_id: i64,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        Chrome::ack_screencast_frame(self, session_id)
    }

    fn set_viewport(
        &mut self,
        width: u32,
        height: u32,
        scale: f64,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        Chrome::set_viewport(self, width, height, scale)
    }

    fn history(&mut self, delta: i64) -> impl std::future::Future<Output = Result<bool>> + Send {
        Chrome::history(self, delta)
    }

    fn link_href(
        &mut self,
        selector: &str,
    ) -> impl std::future::Future<Output = Result<Option<String>>> + Send {
        Chrome::link_href(self, selector)
    }

    async fn youtube_search_query(&self, selector: &str) -> Result<Option<String>> {
        let selector = serde_json::to_string(selector)?;
        let expression = format!(
            r#"(() => {{
            if (location.origin !== 'https://www.youtube.com') return null;
            const button = document.querySelector({selector});
            const box = button?.closest('yt-searchbox, ytd-searchbox');
            if (!box || !button.matches('button.ytSearchboxComponentSearchButton, #search-icon-legacy')) return null;
            const input = box.querySelector('input[name="search_query"]');
            return JSON.stringify(input ? input.value.trim() : null);
        }})()"#
        );
        Ok(serde_json::from_str(
            &self.evaluate_isolated(&expression).await?,
        )?)
    }

    fn insert_text(&mut self, text: &str) -> impl std::future::Future<Output = Result<()>> + Send {
        Chrome::insert_text(self, text)
    }

    fn key(
        &mut self,
        key: &str,
        code: &str,
        virtual_key: i64,
        modifiers: u32,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        Chrome::key(self, key, code, virtual_key, modifiers)
    }

    async fn select_text(&self, start: [f64; 2], end: [f64; 2]) -> Result<()> {
        let expression = format!(
            r#"(() => {{
            const start = document.caretRangeFromPoint({}, {});
            const end = document.caretRangeFromPoint({}, {});
            if (start && end) window.getSelection().setBaseAndExtent(
                start.startContainer, start.startOffset, end.startContainer, end.startOffset
            );
            return true;
        }})()"#,
            start[0], start[1], end[0], end[1]
        );
        self.evaluate_isolated(&expression).await?;
        Ok(())
    }

    async fn selected_text(&self) -> Result<String> {
        self.evaluate_isolated("window.getSelection().toString()")
            .await
    }

    fn click_at(&mut self, x: f64, y: f64) -> impl std::future::Future<Output = Result<()>> + Send {
        Chrome::click_at(self, x, y)
    }

    fn scroll(
        &mut self,
        x: f64,
        y: f64,
        delta_y: f64,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        Chrome::scroll(self, x, y, delta_y)
    }

    fn hit_test(
        &self,
        x: f64,
        y: f64,
    ) -> impl std::future::Future<Output = Result<Option<String>>> + Send {
        Chrome::hit_test(self, x, y)
    }
}

// La pestaña no mata al navegador: lo cierra `Browser::drop` cuando ya no queda
// ninguna pestaña que lo use.

/// Display X virtual para correr Chromium headful sin abrir una ventana en el
/// escritorio. Se usa solo si `Xvfb` está instalado y no se pidió headless.
struct VirtualDisplay {
    child: Child,
    display: String,
}

impl VirtualDisplay {
    async fn start() -> Result<Self> {
        // Varias pruebas piden display a la vez: sin este turno, dos podrían
        // elegir el mismo :99 y una acabaría en un X roto.
        let _turn = display_turn().lock().await;
        let binary = find_in_path("Xvfb").context("Xvfb no está en PATH")?;
        for n in 99..=110 {
            if Path::new(&format!("/tmp/.X{n}-lock")).exists() {
                continue;
            }
            let display = format!(":{n}");
            let mut command = Command::new(&binary);
            command
                .arg(&display)
                .args(["-screen", "0", "1920x1200x24", "-nolisten", "tcp", "-ac"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            #[cfg(unix)]
            unsafe {
                command.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
            let mut child = command
                .spawn()
                .with_context(|| format!("lanzar Xvfb en {display}"))?;

            let socket = format!("/tmp/.X11-unix/X{n}");
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if Path::new(&socket).exists() {
                    return Ok(Self { child, display });
                }
                if let Ok(Some(_)) = child.try_wait() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        bail!("Xvfb no arrancó en ningún display :99-:110")
    }
}

fn display_turn() -> &'static tokio::sync::Mutex<()> {
    static TURN: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    TURN.get_or_init(|| tokio::sync::Mutex::new(()))
}

impl Drop for VirtualDisplay {
    fn drop(&mut self) {
        // SIGTERM deja que Xvfb borre su candado de /tmp; con SIGKILL quedaban
        // displays fantasma que agotaban el rango :99-:110.
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                match self.child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        }
        let _ = self.child.start_kill();
    }
}

fn use_virtual_display() -> bool {
    std::env::var_os("NAVEGADOR_HEADLESS").is_none() && find_in_path("Xvfb").is_some()
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

fn find_binary() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("NAVEGADOR_CHROME") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
        bail!("NAVEGADOR_CHROME apunta a {} que no existe", path.display());
    }
    for candidate in [
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/google-chrome",
        "/snap/bin/chromium",
    ] {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Ok(path);
        }
    }
    bail!("no se encontró Chromium; define NAVEGADOR_CHROME")
}

async fn wait_for_port(
    child: &mut Child,
    binary: &Path,
    profile: &Path,
    timeout: Duration,
) -> Result<u16> {
    let file = profile.join("DevToolsActivePort");
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("no se pudo consultar el estado de {}", binary.display()))?
        {
            bail!(
                "{} terminó antes de publicar DevToolsActivePort ({status})",
                binary.display()
            );
        }
        if let Ok(contents) = std::fs::read_to_string(&file)
            && let Some(port) = contents
                .lines()
                .next()
                .and_then(|l| l.trim().parse::<u16>().ok())
        {
            return Ok(port);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("Chromium no escribió DevToolsActivePort en {timeout:?}")
}

/// Candado exclusivo del perfil persistente. El núcleo lo suelta solo si el
/// proceso muere, así que no quedan candados huérfanos.
fn lock_profile(profile: &Path) -> Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

        let path = profile.join(".navegador.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("no se pudo abrir {}", path.display()))?;
        let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if taken != 0 {
            bail!(
                "el perfil {} ya lo usa otra ventana del navegador",
                profile.display()
            );
        }
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        let _ = profile;
        std::fs::File::open(std::env::temp_dir()).context("candado de perfil no disponible")
    }
}

async fn http_request(port: u16, method: &str, path: &str) -> Result<String> {
    let fut = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .context("no se pudo conectar al endpoint HTTP de CDP")?;
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await?;

        let mut buffer = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let read = stream.read(&mut chunk).await?;
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);

            if let Some(head_end) = find(&buffer, b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
                let length = content_length(&head).unwrap_or(0);
                if buffer.len() >= head_end + 4 + length {
                    break;
                }
            }
            if buffer.len() > 8 * 1024 * 1024 {
                bail!("respuesta HTTP de CDP demasiado grande");
            }
        }

        let text = String::from_utf8_lossy(&buffer);
        let (_, body) = text
            .split_once("\r\n\r\n")
            .context("respuesta HTTP de CDP sin cuerpo")?;
        Ok::<String, anyhow::Error>(body.to_string())
    };

    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .context("timeout leyendo el endpoint HTTP de CDP")?
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn el_arranque_reporta_si_chromium_termina_antes_de_publicar_el_puerto() {
        let binary = Path::new("/bin/sh");
        let profile = tempfile::tempdir().unwrap();
        let mut child = Command::new(binary)
            .args(["-c", "exit 7"])
            .kill_on_drop(true)
            .spawn()
            .unwrap();

        let error = tokio::time::timeout(
            Duration::from_secs(5),
            wait_for_port(&mut child, binary, profile.path(), Duration::from_secs(20)),
        )
        .await
        .expect("la salida del proceso debe reportarse antes del timeout de arranque")
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "/bin/sh terminó antes de publicar DevToolsActivePort (exit status: 7)"
        );
    }
}
