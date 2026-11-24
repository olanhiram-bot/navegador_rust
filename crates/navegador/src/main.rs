//! CLI del MVP.
//!
//! ```text
//! navegador audit  <url> <selector> [importe] [destinatario]
//! navegador run    <url> <selector> [importe] [destinatario] [--yes|--gui] [--key HEX] [--out RUTA]
//! navegador verify <recibo.json>
//! navegador verify-log <recibos.jsonl>
//! navegador keygen
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use descriptor::{Intent, Provenance};
use engine_cdp::{Chrome, DispatchOutcome};
use receipt::{Identity, WireRequest};

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
    let (command, positional, flags) = parse_args()?;

    match command.as_str() {
        "audit" => audit(&positional).await,
        "run" => run(&positional, &flags).await,
        "verify" => verify(&positional),
        "verify-log" => verify_log(&positional),
        "keygen" => keygen(),
        other => {
            usage();
            bail!("comando desconocido: {other}")
        }
    }
}

async fn audit(positional: &[String]) -> Result<ExitCode> {
    let (url, selector, intent) = task_args(positional)?;
    let mut chrome = Chrome::launch().await?;
    chrome.navigate(&url).await?;
    let descriptor = chrome.extract(&selector, 1, Provenance::User).await?;
    let decision = intent.evaluate(&descriptor, None);
    let view = card::build(&descriptor, &decision);

    println!("{}", card::render_text(&view));
    println!("{}", serde_json::to_string_pretty(&descriptor)?);
    println!("{}", serde_json::to_string_pretty(&decision)?);

    if !descriptor.hash_is_valid() {
        bail!("el descriptor no coincide con su hash");
    }
    if decision.is_denied() {
        return Ok(ExitCode::from(2));
    }
    Ok(ExitCode::SUCCESS)
}

async fn run(positional: &[String], flags: &HashMap<String, String>) -> Result<ExitCode> {
    let (url, selector, intent) = task_args(positional)?;
    let mut chrome = Chrome::launch().await?;
    chrome.navigate(&url).await?;
    let descriptor = chrome.extract(&selector, 1, Provenance::User).await?;

    let decision = intent.evaluate(&descriptor, None);
    let view = card::build(&descriptor, &decision);
    println!("{}", card::render_text(&view));

    if decision.is_denied() {
        eprintln!("acción denegada por la política");
        return Ok(ExitCode::from(2));
    }

    let approved_hash = match decision.hash() {
        Some(_) if flags.contains_key("gui") => match gui_approval(&view)? {
            Some(approved) => Some(approved),
            None => {
                println!("denegado por la persona");
                return Ok(ExitCode::from(3));
            }
        },
        Some(hash) if flags.contains_key("yes") => Some(hash.to_string()),
        Some(_) => bail!("se requiere aprobación: usa --yes o --gui"),
        None => None,
    };

    let final_decision = intent.evaluate(&descriptor, approved_hash.as_deref());
    if !final_decision.is_allowed() {
        eprintln!("la aprobación no cubre este descriptor: {final_decision:?}");
        return Ok(ExitCode::from(2));
    }

    let outcome = chrome.dispatch(&descriptor).await?;
    match outcome {
        DispatchOutcome::Sent { url, method, body } => {
            let identity = match flags.get("key") {
                Some(seed) => Identity::from_seed_hex(seed)?,
                None => Identity::generate()?,
            };
            let signed = identity.sign(
                &descriptor,
                WireRequest {
                    url,
                    method,
                    body: body.clone(),
                },
                "sent",
            );
            receipt::verify(&signed).context("el recibo recién firmado no verifica")?;

            let json = serde_json::to_string_pretty(&signed)?;
            match flags.get("out") {
                Some(path) => {
                    std::fs::write(path, &json)?;
                    println!("recibo escrito en {path}");
                }
                None => println!("{json}"),
            }
            println!("recibo verificado · clave {}", identity.public_hex());
            Ok(ExitCode::SUCCESS)
        }
        other => {
            eprintln!("acción no ejecutada: {other:?}");
            Ok(ExitCode::from(4))
        }
    }
}

fn verify(positional: &[String]) -> Result<ExitCode> {
    let path = positional
        .first()
        .context("falta la ruta del recibo: navegador verify <recibo.json>")?;
    let text = std::fs::read_to_string(path).with_context(|| format!("no se pudo leer {path}"))?;

    match receipt::verify_json(&text) {
        Ok(signed) => {
            let d = &signed.descriptor;
            println!("RECIBO VÁLIDO");
            println!("  emitido:      {}", signed.issued_at_unix);
            println!("  clave:        {}", signed.public_key);
            println!("  acción:       {}", d.verb);
            println!(
                "  importe:      {}",
                d.amount
                    .map(|a| format!("{a:.2}"))
                    .unwrap_or_else(|| "—".into())
            );
            println!("  destinatario: {}", d.recipient.as_deref().unwrap_or("—"));
            println!(
                "  origen:       {}",
                d.action_origin.as_deref().unwrap_or("—")
            );
            println!("  petición:     {} {}", signed.wire.method, signed.wire.url);
            println!("  hash:         {}", d.hash_hex);
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            eprintln!("RECIBO RECHAZADO: {error}");
            Ok(ExitCode::from(2))
        }
    }
}

fn verify_log(positional: &[String]) -> Result<ExitCode> {
    let [path] = positional else {
        bail!("uso: navegador verify-log <recibos.jsonl>");
    };
    match receipt::verify_file(Path::new(path)) {
        Ok(count) => {
            println!("Cadena válida · {count} recibos");
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            eprintln!("Cadena no válida: {error}");
            Ok(ExitCode::from(2))
        }
    }
}

fn keygen() -> Result<ExitCode> {
    let identity = Identity::generate()?;
    println!("semilla (secreto, guárdala): {}", identity.seed_hex());
    println!("clave pública:               {}", identity.public_hex());
    Ok(ExitCode::SUCCESS)
}

#[cfg(feature = "gui")]
fn gui_approval(view: &card::CardView) -> Result<Option<String>> {
    match card::gui::show(view.clone()).map_err(anyhow::Error::msg)? {
        card::gui::Approval::Approved(hash) => Ok(Some(hash)),
        card::gui::Approval::Denied => Ok(None),
    }
}

#[cfg(not(feature = "gui"))]
fn gui_approval(_view: &card::CardView) -> Result<Option<String>> {
    bail!("este binario se compiló sin GUI; usa --yes o compila con --features gui")
}

fn task_args(positional: &[String]) -> Result<(String, String, Intent)> {
    if positional.len() < 2 {
        usage();
        bail!("faltan <url> y <selector>");
    }
    let url = positional[0].clone();
    let selector = positional[1].clone();
    let amount: Option<f64> = positional.get(2).and_then(|v| v.parse().ok());
    let recipient: Option<String> = positional.get(3).cloned();

    let document_origin = url::Url::parse(&url)
        .ok()
        .and_then(|u| {
            let origin = u.origin();
            origin.is_tuple().then(|| origin.ascii_serialization())
        })
        .unwrap_or_default();

    let intent = Intent {
        provenance: Provenance::User,
        verb: Some(descriptor::Verb::Submit),
        amount,
        recipient,
        allowed_action_origins: vec![document_origin],
        max_amount: amount.unwrap_or(1.0).max(1.0) * 10.0,
        require_approval: true,
    };
    Ok((url, selector, intent))
}

fn parse_args() -> Result<(String, Vec<String>, HashMap<String, String>)> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        bail!("falta el comando");
    };

    let mut positional = Vec::new();
    let mut flags = HashMap::new();
    while let Some(arg) = args.next() {
        if let Some(name) = arg.strip_prefix("--") {
            let name = name.to_string();
            if name == "yes" || name == "gui" {
                flags.insert(name, "true".into());
            } else {
                let value = args
                    .next()
                    .with_context(|| format!("--{name} necesita un valor"))?;
                flags.insert(name, value);
            }
        } else {
            positional.push(arg);
        }
    }
    Ok((command, positional, flags))
}

fn usage() {
    eprintln!(
        "uso:\n  navegador audit  <url> <selector> [importe] [destinatario]\n  navegador run    <url> <selector> [importe] [destinatario] [--yes|--gui] [--key HEX] [--out RUTA]\n  navegador verify <recibo.json>\n  navegador verify-log <recibos.jsonl>\n  navegador keygen"
    );
}
