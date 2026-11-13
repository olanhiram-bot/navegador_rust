//! Servidor MCP (Model Context Protocol) sobre stdio.
//!
//! Expone una sesión de navegador a un host de agente con seis herramientas:
//! `open`, `snapshot`, `act`, `read`, `screenshot` y `handoff`.
//!
//! No hay atajo: `act` siempre pasa por descriptor → política → tarjeta →
//! dispatch → recibo. Si la política pide aprobación, la respuesta lleva el
//! hash exacto y el host debe repetir `act` con `approve`; cualquier cambio en
//! la página invalida ese hash.

use std::path::PathBuf;

use anyhow::{Context, Result};
use base64::Engine as _;
use engine_cdp::Chrome;
use receipt::{Identity, ReceiptLog};
use serde_json::{Value, json};
use session::{ActOptions, ActOutcome, Session, SessionConfig};

pub const PROTOCOL_VERSION: &str = "2024-11-05";

pub struct McpServer {
    session: Option<Session<Chrome>>,
    config: SessionConfig,
    log_path: PathBuf,
}

impl McpServer {
    pub fn new(log_path: impl Into<PathBuf>, config: SessionConfig) -> Self {
        McpServer {
            session: None,
            config,
            log_path: log_path.into(),
        }
    }

    /// Procesa un mensaje JSON-RPC. Devuelve `None` para notificaciones.
    pub async fn handle(&mut self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned();
        let method = message.get("method")?.as_str()?.to_string();

        if method.starts_with("notifications/") {
            return None;
        }

        let outcome: Result<Value, (i64, String)> = match method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": {
                    "name": "navegador",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_definitions() })),
            "tools/call" => {
                self.call_tool(message.get("params").unwrap_or(&Value::Null))
                    .await
            }
            other => Err((-32601, format!("método desconocido: {other}"))),
        };

        // Una notificación nunca lleva id: no se responde.
        let id = id?;
        Some(match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": code, "message": message }
            }),
        })
    }

    async fn call_tool(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "falta el nombre de la herramienta".to_string()))?;
        let empty = json!({});
        let args = params.get("arguments").unwrap_or(&empty);

        let result = match name {
            "open" => self.tool_open(args).await,
            "snapshot" => self.tool_snapshot().await,
            "act" => self.tool_act(args).await,
            "read" => self.tool_read().await,
            "screenshot" => self.tool_screenshot().await,
            "handoff" => self.tool_handoff().await,
            other => return Err((-32602, format!("herramienta desconocida: {other}"))),
        };

        Ok(match result {
            Ok(content) => json!({ "content": content, "isError": false }),
            Err(error) => json!({
                "content": [{ "type": "text", "text": format!("{error:#}") }],
                "isError": true
            }),
        })
    }

    async fn tool_open(&mut self, args: &Value) -> Result<Vec<Value>> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .context("open necesita {\"url\": \"https://...\"}")?;
        let snapshot = self.ensure_session().await?.open(url).await?;
        Ok(vec![text_block(&snapshot.render_text())])
    }

    async fn tool_snapshot(&mut self) -> Result<Vec<Value>> {
        let snapshot = self.open_session()?.snapshot().await?;
        Ok(vec![text_block(&snapshot.render_text())])
    }

    async fn tool_act(&mut self, args: &Value) -> Result<Vec<Value>> {
        let reference = args
            .get("ref")
            .or_else(|| args.get("reference"))
            .and_then(Value::as_str)
            .context("act necesita {\"ref\": \"@e3\"}")?;
        let options = ActOptions {
            amount: args.get("amount").and_then(Value::as_f64),
            recipient: args
                .get("recipient")
                .and_then(Value::as_str)
                .map(str::to_string),
            approved_hash: args
                .get("approve")
                .or_else(|| args.get("approved_hash"))
                .and_then(Value::as_str)
                .map(str::to_string),
            generation: args
                .get("generation")
                .and_then(Value::as_u64)
                .map(|generation| generation as u32),
        };

        let outcome = self.open_session()?.act(reference, options).await?;
        let payload = outcome_json(&outcome);
        Ok(vec![text_block(&serde_json::to_string_pretty(&payload)?)])
    }

    async fn tool_read(&mut self) -> Result<Vec<Value>> {
        let text = self.open_session()?.read().await?;
        Ok(vec![text_block(&text)])
    }

    async fn tool_screenshot(&mut self) -> Result<Vec<Value>> {
        let png = self.open_session()?.screenshot().await?;
        let data = base64::engine::general_purpose::STANDARD.encode(&png);
        Ok(vec![json!({
            "type": "image",
            "data": data,
            "mimeType": "image/png"
        })])
    }

    async fn tool_handoff(&mut self) -> Result<Vec<Value>> {
        let session = self.open_session()?;
        let public_key = session.public_key();
        let report = session.handoff().await;
        Ok(vec![text_block(&serde_json::to_string_pretty(&json!({
            "url": report.url,
            "title": report.title,
            "generation": report.generation,
            "elements": report.elements.len(),
            "receipts": report.receipts,
            "last_receipt_hash": report.last_receipt_hash,
            "log": report.log_path.display().to_string(),
            "public_key": public_key,
            "summary": report.render_text(),
        }))?)])
    }

    /// La sesión se crea en el primer `open`: lanzar Chromium en el arranque
    /// del servidor MCP solo retrasaría el handshake.
    async fn ensure_session(&mut self) -> Result<&mut Session<Chrome>> {
        if self.session.is_none() {
            let log = ReceiptLog::open(&self.log_path).context("abrir el log de recibos")?;
            let identity = Identity::generate().context("generar la identidad local")?;
            let chrome = Chrome::launch().await.context("lanzar Chromium")?;
            self.session = Some(Session::new(chrome, identity, log, self.config.clone()));
        }
        Ok(self.session.as_mut().expect("la sesión acaba de crearse"))
    }

    fn open_session(&mut self) -> Result<&mut Session<Chrome>> {
        self.session
            .as_mut()
            .context("no hay sesión abierta: llama a open primero")
    }
}

fn text_block(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn outcome_json(outcome: &ActOutcome) -> Value {
    match outcome {
        ActOutcome::Denied {
            descriptor,
            decision,
            card,
        } => json!({
            "outcome": "denied",
            "hash": descriptor.hash_hex,
            "reasons": decision.reasons(),
            "card": card::render_text(card),
        }),
        ActOutcome::ApprovalRequired {
            descriptor,
            hash,
            card,
        } => json!({
            "outcome": "approval_required",
            "hash": hash,
            "descriptor_hash": descriptor.hash_hex,
            "warnings": card.warnings,
            "card": card::render_text(card),
            "next": "repite act con {\"approve\": \"<hash>\"} para ejecutar",
        }),
        ActOutcome::Executed {
            receipt,
            card,
            seq,
            outcome,
        } => json!({
            "outcome": "executed",
            "receipt_seq": seq,
            "hash": receipt.descriptor.hash_hex,
            "public_key": receipt.public_key,
            "wire": receipt.wire,
            "dispatch": outcome,
            "card": card::render_text(card),
        }),
        ActOutcome::NotExecuted {
            descriptor,
            outcome,
        } => json!({
            "outcome": "not_executed",
            "hash": descriptor.hash_hex,
            "dispatch": outcome,
        }),
    }
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "open",
            "description": "Navega a una URL y devuelve el snapshot con refs @eN estables en esta generación.",
            "inputSchema": {
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"]
            }
        },
        {
            "name": "snapshot",
            "description": "Refresca la foto de elementos accionables sin cambiar de generación.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "act",
            "description": "Actúa sobre una ref @eN. Pasa por descriptor, política y tarjeta. Si la política pide aprobación, devuelve el hash a aprobar; repetir con approve lo ejecuta y deja recibo firmado.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "ref": { "type": "string", "description": "Referencia @eN del snapshot" },
                    "generation": { "type": "integer", "description": "Generación del snapshot del que salió la ref" },
                    "amount": { "type": "number", "description": "Importe que la persona autorizó" },
                    "recipient": { "type": "string", "description": "Destinatario que la persona autorizó" },
                    "approve": { "type": "string", "description": "Hash exacto aprobado por la persona" }
                },
                "required": ["ref"]
            }
        },
        {
            "name": "read",
            "description": "Texto visible de la página, sin interpretar.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "screenshot",
            "description": "PNG de la vista actual como contenido de imagen.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "handoff",
            "description": "Pasa el control a la persona: URL, generación, elementos, recibos y ruta del log.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initialize_anuncia_tools_y_ignora_notificaciones() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = McpServer::new(dir.path().join("recibos.jsonl"), SessionConfig::default());

        let hello = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {}
            }))
            .await
            .expect("initialize responde");
        assert_eq!(hello["result"]["protocolVersion"], PROTOCOL_VERSION);

        assert!(
            server
                .handle(json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/initialized"
                }))
                .await
                .is_none()
        );

        let listed = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list"
            }))
            .await
            .unwrap();
        let names: Vec<&str> = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        for expected in ["open", "snapshot", "act", "read", "screenshot", "handoff"] {
            assert!(names.contains(&expected), "falta {expected}: {names:?}");
        }
    }

    #[tokio::test]
    async fn un_metodo_desconocido_devuelve_error_de_protocolo() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = McpServer::new(dir.path().join("recibos.jsonl"), SessionConfig::default());
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "recursos/listar"
            }))
            .await
            .unwrap();
        assert_eq!(response["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn act_sin_sesion_es_error_de_herramienta_no_de_protocolo() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = McpServer::new(dir.path().join("recibos.jsonl"), SessionConfig::default());
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "act",
                    "arguments": { "ref": "@e1" }
                }
            }))
            .await
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("open")
        );
    }
}
