//! `navegador-mcp`: servidor MCP por stdio.
//!
//! ```text
//! navegador-mcp [--log RUTA] [--max-amount N] [--allow-origin ORIGEN]...
//! ```
//!
//! Habla JSON-RPC delimitado por líneas. Cuando el host cierra stdin, el
//! servidor termina y `Drop` mata Chromium entero.

use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use mcp::McpServer;
use serde_json::{Value, json};
use session::SessionConfig;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

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

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = BufWriter::new(tokio::io::stdout());

    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error).context("leer stdin"),
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => server.handle(message).await,
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

    Ok(ExitCode::SUCCESS)
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
