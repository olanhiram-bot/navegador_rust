//! Ciclo de vida del servidor MCP.
//!
//! Si el host termina el proceso con SIGTERM o SIGINT, el servidor debe salir
//! por su ruta limpia y matar el grupo de procesos de Chromium. Sin capturar
//! esas señales el proceso muere sin destructores y Chromium queda huérfano.

use std::fs;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

struct Host {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    server_pid: u32,
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
        let server_pid = child.id().expect("pid del servidor");
        let stdin = BufWriter::new(child.stdin.take().expect("stdin"));
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Host {
            child,
            stdin,
            stdout,
            next_id: 0,
            server_pid,
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        let line = serde_json::to_string(&message).unwrap();
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.write_all(b"\n").await.unwrap();
        self.stdin.flush().await.unwrap();

        let mut response = String::new();
        let read = tokio::time::timeout(
            Duration::from_secs(60),
            self.stdout.read_line(&mut response),
        )
        .await
        .expect("el servidor MCP no respondió")
        .expect("leer stdout");
        assert!(read > 0, "el servidor MCP cerró stdout");
        let response: Value = serde_json::from_str(&response).expect("JSON-RPC válido");
        assert_eq!(response["id"], json!(id));
        response["result"].clone()
    }
}

/// PID del hijo Chromium del servidor: su PGID es su propio pid.
fn chromium_child_of(parent: u32) -> Option<u32> {
    for entry in fs::read_dir("/proc").ok()? {
        let Ok(entry) = entry else { continue };
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some((_, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        // Tras `comm`: estado, ppid, pgrp, ...
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.get(1).and_then(|value| value.parse::<u32>().ok()) != Some(parent) {
            continue;
        }
        let cmdline = fs::read_to_string(entry.path().join("cmdline")).unwrap_or_default();
        if cmdline.contains("chromium") {
            return Some(pid);
        }
    }
    None
}

/// ¿Queda algún proceso en ese grupo (incluidos zygotes y renderers)?
fn group_alive(pgid: u32) -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some((_, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.get(2).and_then(|value| value.parse::<u32>().ok()) == Some(pgid) {
            return true;
        }
    }
    false
}

async fn signal_kills_chromium(sig: i32) {
    let server = navegador::FixtureServer::start();
    let dir = tempfile::tempdir().expect("tempdir");
    let mut host = Host::start(&dir.path().join("recibos.jsonl"));

    host.request("initialize", json!({})).await;
    let opened = host
        .request(
            "tools/call",
            json!({
                "name": "open",
                "arguments": { "url": server.page_url("legit.html") }
            }),
        )
        .await;
    assert_eq!(opened["isError"], false, "{opened}");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut chrome = None;
    while chrome.is_none() && Instant::now() < deadline {
        chrome = chromium_child_of(host.server_pid);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let chrome = chrome.expect("Chromium debe estar corriendo como hijo del servidor");
    assert!(group_alive(chrome), "el grupo de Chromium debe existir");

    unsafe {
        libc::kill(host.server_pid as i32, sig);
    }

    let status = tokio::time::timeout(Duration::from_secs(15), host.child.wait())
        .await
        .expect("el servidor debe salir tras la señal")
        .expect("wait del servidor");
    assert!(status.success(), "código de salida: {status:?}");

    let deadline = Instant::now() + Duration::from_secs(5);
    while group_alive(chrome) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        !group_alive(chrome),
        "el grupo {chrome} de Chromium sigue vivo tras la señal {sig}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigterm_mata_chromium() {
    signal_kills_chromium(libc::SIGTERM).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigint_mata_chromium() {
    signal_kills_chromium(libc::SIGINT).await;
}
