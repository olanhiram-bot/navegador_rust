//! CLI mínima del MVP.
//!
//! `navegador audit <url> <selector> [importe] [destinatario]`
//!
//! Abre la página, construye el descriptor ground-truth y lo evalúa contra la
//! intención. No despacha nada: solo audita.

use anyhow::{bail, Result};
use descriptor::{Intent, Provenance};
use engine_cdp::Chrome;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 || args[0] != "audit" {
        eprintln!("uso: navegador audit <url> <selector> [importe] [destinatario]");
        std::process::exit(64);
    }

    let url = &args[1];
    let selector = &args[2];
    let amount: Option<f64> = args.get(3).and_then(|v| v.parse().ok());
    let recipient: Option<String> = args.get(4).cloned();

    let chrome = Chrome::launch().await?;
    chrome.navigate(url).await?;
    let descriptor = chrome.extract(selector, 1, Provenance::User).await?;

    let action_origin = descriptor
        .action_origin
        .clone()
        .unwrap_or_else(|| descriptor.document_origin.clone());
    let intent = Intent {
        provenance: Provenance::User,
        verb: descriptor.verb,
        amount,
        recipient,
        allowed_action_origins: vec![descriptor.document_origin.clone()],
        max_amount: amount.unwrap_or(0.0).max(1.0) * 10.0,
        require_approval: true,
    };

    let decision = intent.evaluate(&descriptor, None);

    println!("{}", serde_json::to_string_pretty(&descriptor)?);
    println!();
    println!("origen de la acción: {action_origin}");
    println!("{}", serde_json::to_string_pretty(&decision)?);

    if decision.is_denied() {
        std::process::exit(2);
    }
    if !descriptor.hash_is_valid() {
        bail!("el descriptor no coincide con su hash");
    }
    Ok(())
}
