//! Lanzador de Chromium y operaciones de alto nivel: navegar, extraer el
//! descriptor y despachar una acción verificada.

use std::path::PathBuf;
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

/// Chromium headless controlado por CDP.
pub struct Chrome {
    child: Child,
    _profile: tempfile::TempDir,
    port: u16,
    cdp: Cdp,
    fetch_armed: bool,
}

impl Chrome {
    pub async fn launch() -> Result<Self> {
        let binary = find_binary()?;
        let profile = tempfile::tempdir().context("no se pudo crear el perfil temporal")?;
        let profile_path = profile.path().to_path_buf();

        let mut command = Command::new(&binary);
        command
            .arg("--headless=new")
            .arg("--no-sandbox")
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage")
            .arg("--disable-extensions")
            .arg("--disable-background-networking")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--mute-audio")
            .arg("--hide-scrollbars")
            .arg("--window-size=1024,768")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile_path.display()))
            .arg("about:blank")
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        // Grupo de procesos propio: al cerrar la sesión matamos también a los
        // zygotes y renderers, que si no quedan huérfanos.
        #[cfg(unix)]
        command.process_group(0);

        let child = command
            .spawn()
            .with_context(|| format!("no se pudo lanzar {}", binary.display()))?;
        let port = wait_for_port(&profile_path, Duration::from_secs(20)).await?;
        let page_ws = page_target_ws(port).await?;
        let cdp = Cdp::connect(&page_ws).await?;

        Ok(Chrome {
            child,
            _profile: profile,
            port,
            cdp,
            fetch_armed: false,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
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
        self.cdp.call("Network.enable", json!({})).await?;
        self.cdp
            .call("Page.navigate", json!({ "url": url }))
            .await
            .with_context(|| format!("no se pudo navegar a {url}"))?;

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
            if value
                .get("result")
                .and_then(|r| r.get("value"))
                .and_then(Value::as_str)
                == Some("complete")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // Deja que los scripts de la página terminen de correr.
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
            "({})({})",
            EXTRACT_JS,
            serde_json::to_string(selector).expect("selector serializable")
        );
        let text = self.evaluate_isolated(&expression).await?;

        let value: Value = serde_json::from_str(&text).context("JSON de extracción inválido")?;
        if let Some(error) = value.get("error").and_then(Value::as_str) {
            bail!("extracción: {error} (selector {selector:?})");
        }
        let raw: RawAction = serde_json::from_value(value).context("RawAction inválido")?;
        Ok(ActionDescriptor::from_raw(raw, provenance, generation))
    }

    /// Foto de los elementos accionables, con refs `@eN` ligadas a `generation`.
    pub async fn snapshot(&mut self, generation: u32) -> Result<Snapshot> {
        let text = self
            .evaluate_isolated(&format!("({SNAPSHOT_JS})()"))
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
            .evaluate_isolated("document.body ? document.body.innerText : ''")
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
                Some("Network.requestWillBeSent") => {
                    // Evidencia de defensa en profundidad. La decisión ya la tomó
                    // `Fetch` antes de que la petición saliera.
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
}

impl Drop for Chrome {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            // El hijo es líder de su propio grupo: así caen también zygotes y
            // renderers, que de otro modo quedan huérfanos.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }
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

async fn wait_for_port(profile: &std::path::Path, timeout: Duration) -> Result<u16> {
    let file = profile.join("DevToolsActivePort");
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
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

async fn page_target_ws(port: u16) -> Result<String> {
    let body = http_get(port, "/json/list").await?;
    let targets: Value = serde_json::from_str(&body).context("/json/list no es JSON")?;
    let list = targets.as_array().context("/json/list no es un array")?;
    list.iter()
        .find(|t| t.get("type").and_then(Value::as_str) == Some("page"))
        .and_then(|t| t.get("webSocketDebuggerUrl"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .context("no hay un target de tipo page")
}

async fn http_get(port: u16, path: &str) -> Result<String> {
    let fut = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .context("no se pudo conectar al endpoint HTTP de CDP")?;
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
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
