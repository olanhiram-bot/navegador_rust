//! `navegador-mcp`: servidor MCP por stdio.
//!
//! ```text
//! navegador-mcp [--log RUTA] [--max-amount N] [--allow-origin ORIGEN]...
//! ```
//!
//! Habla JSON-RPC delimitado por líneas. Cuando el host cierra stdin, o envía
//! SIGTERM/SIGINT, el servidor sale del bucle y `Drop` mata el grupo de
//! procesos de Chromium. Sin capturar esas señales el proceso moriría sin
//! ejecutar destructores y dejaría Chromium huérfano.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use mcp::McpServer;
use serde_json::{Value, json};
use session::SessionConfig;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> ExitCode {
    match real_main().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::from(1)
        }
    }
}

async fn real_main() -> Result<ExitCode> {
    let (log_path, config) = parse_args()?;
    let mut server = McpServer::new(&log_path, config);

    // `tokio::io::stdin` lee en un hilo bloqueante que el runtime espera al
    // apagarse: si el host sigue con el pipe abierto, el cierre se cuelga.
    // Un hilo propio con canal no ata el apagado del runtime.
    let mut lines = stdin_lines();
    let mut out = BufWriter::new(tokio::io::stdout());

    // Por defecto SIGTERM/SIGINT terminan el proceso sin destructores: la
    // sesión no se dropea y Chromium sobrevive. Capturarlas convierte el
    // cierre en una salida limpia por el mismo camino que el EOF de stdin.
    let mut terminate = signal(SignalKind::terminate()).context("instalar SIGTERM")?;
    let mut interrupt = signal(SignalKind::interrupt()).context("instalar SIGINT")?;

    loop {
        let line = tokio::select! {
            _ = terminate.recv() => {
                eprintln!("navegador-mcp: SIGTERM; cerrando y matando Chromium");
                break;
            }
            _ = interrupt.recv() => {
                eprintln!("navegador-mcp: SIGINT; cerrando y matando Chromium");
                break;
            }
            line = lines.recv() => match line {
                Some(Ok(line)) => line,
                Some(Err(error)) => return Err(error).context("leer stdin"),
                None => break,
            },
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => tokio::select! {
                _ = terminate.recv() => {
                    eprintln!("navegador-mcp: SIGTERM; cerrando y matando Chromium");
                    break;
                }
                _ = interrupt.recv() => {
                    eprintln!("navegador-mcp: SIGINT; cerrando y matando Chromium");
                    break;
                }
                response = server.handle(message) => response,
            },
            Err(error) => Some(json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32700, "message": format!("JSON inválido: {error}") }
            })),
        };

        if let Some(response) = response {
            let encoded = serde_json::to_string(&response)?;
            out.write_all(encoded.as_bytes()).await?;
            out.write_all(b"\n").await?;
            out.flush().await?;
        }
    }

    // Explícito: al dropear la sesión, `Chrome::Drop` mata su grupo entero.
    drop(server);
    Ok(ExitCode::SUCCESS)
}

/// Lee stdin en un hilo propio. `None` en el canal significa EOF.
fn stdin_lines() -> mpsc::UnboundedReceiver<std::io::Result<String>> {
    use std::io::BufRead as _;

    let (sender, receiver) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

fn parse_args() -> Result<(PathBuf, SessionConfig)> {
    let mut log_path = PathBuf::from("navegador-recibos.jsonl");
    let mut config = SessionConfig::default();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--log" => {
                log_path = PathBuf::from(args.next().context("--log necesita una ruta")?);
            }
            "--max-amount" => {
                config.max_amount = args
                    .next()
                    .context("--max-amount necesita un número")?
                    .parse()
                    .context("--max-amount no es un número")?;
            }
            "--allow-origin" => {
                config
                    .extra_allowed_origins
                    .push(args.next().context("--allow-origin necesita un origen")?);
            }
            "--help" | "-h" => {
                println!(
                    "uso: navegador-mcp [--log RUTA] [--max-amount N] [--allow-origin ORIGEN]..."
                );
                std::process::exit(0);
            }
            other => bail!("argumento desconocido: {other}"),
        }
    }

    if config.max_amount <= 0.0 {
        bail!("--max-amount debe ser positivo");
    }
    Ok((log_path, config))
}
