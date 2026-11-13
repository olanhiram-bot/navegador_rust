//! Gate de la fase 3.
//!
//! Un host MCP completa un flujo de tres pasos (open → snapshot → act) hablando
//! JSON-RPC por stdio con el binario real. La acción consecuente sale con
//! recibo firmado y el log encadenado verifica en otro proceso.
//!
//! Condición del gate: 100% de acciones consecuentes con recibo.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

struct Host {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Host {
    fn start(log: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_navegador-mcp"))
            .arg("--log")
            .arg(log)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .expect("el servidor MCP debe arrancar");
        let stdin = BufWriter::new(child.stdin.take().expect("stdin"));
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Host {
            child,
            stdin,
            stdout,
            next_id: 0,
        }
    }

    async fn send(&mut self, message: &Value) {
        let line = serde_json::to_string(message).unwrap();
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.write_all(b"\n").await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn read(&mut self) -> Value {
        let mut line = String::new();
        let read = tokio::time::timeout(Duration::from_secs(60), self.stdout.read_line(&mut line))
            .await
            .expect("el servidor MCP no respondió a tiempo")
            .expect("leer stdout del servidor MCP");
        assert!(read > 0, "el servidor MCP cerró stdout");
        serde_json::from_str(&line).expect("respuesta JSON-RPC válida")
    }

    async fn notify(&mut self, method: &str) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method }))
            .await;
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await;
        let response = self.read().await;
        assert_eq!(response["id"], json!(id), "respuesta fuera de orden");
        assert!(
            response.get("error").is_none(),
            "error de protocolo: {response}"
        );
        response["result"].clone()
    }

    /// Llama una tool y devuelve su primer bloque de contenido.
    async fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        assert_eq!(result["isError"], false, "la tool falló: {result}");
        result["content"][0].clone()
    }

    async fn text(&mut self, name: &str, arguments: Value) -> String {
        let block = self.tool(name, arguments).await;
        assert_eq!(block["type"], "text");
        block["text"].as_str().unwrap().to_string()
    }

    fn kill(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn un_host_mcp_completa_tres_pasos_y_toda_accion_deja_recibo() {
    let server = navegador::FixtureServer::start();
    let dir = tempfile::tempdir().expect("tempdir");
    let log_path = dir.path().join("recibos.jsonl");
    let mut host = Host::start(&log_path);

    // Handshake MCP.
    let init = host
        .request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "gate-test", "version": "0" }
            }),
        )
        .await;
    assert_eq!(init["serverInfo"]["name"], "navegador");
    host.notify("notifications/initialized").await;

    let tools = host.request("tools/list", json!({})).await;
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 6, "tools: {names:?}");

    // Paso 1: open.
    let opened = host
        .text("open", json!({ "url": server.page_url("legit.html") }))
        .await;
    assert!(opened.contains("generación 1"), "{opened}");
    assert!(opened.contains("Confirmar transferencia"), "{opened}");

    // Paso 2: snapshot (refs estables dentro de la generación).
    let snapshot = host.text("snapshot", json!({})).await;
    assert!(snapshot.contains("generación 1"), "{snapshot}");
    let reference = snapshot
        .split_whitespace()
        .find(|token| token.starts_with("@e"))
        .expect("el snapshot debe traer una ref @eN")
        .to_string();

    // Lectura antes de actuar: la página sigue siendo la tarea legítima.
    let text = host.text("read", json!({})).await;
    assert!(text.contains("Transferencia"), "{text}");

    // Paso 3a: act sin aprobación → la política pide el hash exacto.
    let pending: Value = serde_json::from_str(
        &host
            .text(
                "act",
                json!({
                    "ref": reference,
                    "generation": 1,
                    "amount": 1.00,
                    "recipient": "ana"
                }),
            )
            .await,
    )
    .unwrap();
    assert_eq!(pending["outcome"], "approval_required", "{pending}");
    let hash = pending["hash"]
        .as_str()
        .expect("hash a aprobar")
        .to_string();
    assert!(pending["card"].as_str().unwrap().contains("1.00"));
    assert!(server.requests().is_empty(), "todavía no debía salir nada");

    // Paso 3b: act con la aprobación de ese hash → ejecuta y deja recibo.
    let executed: Value = serde_json::from_str(
        &host
            .text(
                "act",
                json!({
                    "ref": reference,
                    "generation": 1,
                    "amount": 1.00,
                    "recipient": "ana",
                    "approve": hash
                }),
            )
            .await,
    )
    .unwrap();
    assert_eq!(executed["outcome"], "executed", "{executed}");
    assert_eq!(executed["receipt_seq"], 0);
    assert_eq!(executed["wire"]["method"], "POST");
    assert!(
        executed["public_key"].as_str().unwrap().len() == 64,
        "el recibo debe llevar la clave pública"
    );

    // La petición aprobada llegó al servidor una sola vez.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "peticiones: {requests:?}");
    assert!(requests[0].body.contains("to=ana"), "{requests:?}");
    assert!(requests[0].body.contains("amount=1.00"), "{requests:?}");

    // Tras ejecutar, la generación avanzó y las refs anteriores ya no sirven.
    let after = host.text("snapshot", json!({})).await;
    assert!(after.contains("generación 2"), "{after}");

    let shot = host.tool("screenshot", json!({})).await;
    assert_eq!(shot["type"], "image");
    assert_eq!(shot["mimeType"], "image/png");
    assert!(!shot["data"].as_str().unwrap().is_empty());

    // Handoff: el contexto completo para que una persona tome el control.
    let handoff: Value = serde_json::from_str(&host.text("handoff", json!({})).await).unwrap();
    assert_eq!(handoff["receipts"], 1, "{handoff}");
    assert_eq!(handoff["generation"], 2);
    assert_eq!(handoff["log"], log_path.display().to_string());

    // El log encadenado verifica y contiene un recibo válido.
    assert_eq!(
        receipt::verify_file(&log_path).expect("el log debe verificar"),
        1
    );
    let json = std::fs::read_to_string(&log_path).unwrap();
    let first: Value = serde_json::from_str(json.lines().next().unwrap()).unwrap();
    assert_eq!(first["seq"], 0);
    assert_eq!(first["prev"], "genesis");

    host.kill();
}
