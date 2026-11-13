//! Action Card: la vista que ve la persona antes de aprobar.
//!
//! Propiedades que el MVP exige:
//!   - muestra el ground truth (importe y destinatario reales, origen real);
//!   - Denegar es la acción dominante y la predeterminada;
//!   - el contenido de la página nunca se interpreta como marcado;
//!   - la aprobación queda atada al hash del descriptor.
//!
//! El modelo (`CardView`) es independiente del renderizador. Eso permite
//! probarlo sin pantalla y usar egui solo cuando hay display.

use descriptor::{ActionDescriptor, Decision, Inconsistency};

#[cfg(feature = "gui")]
pub mod gui;

#[derive(Debug, Clone, PartialEq)]
pub struct CardRow {
    pub label: String,
    pub value: String,
    pub emphasis: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CardView {
    pub title: String,
    pub rows: Vec<CardRow>,
    pub warnings: Vec<String>,
    pub denied: bool,
    /// Hash que la persona debe aprobar. `None` si la acción está denegada.
    pub approve_hash: Option<String>,
    pub deny_first: bool,
}

impl CardView {
    pub fn warning_lines(&self) -> Vec<String> {
        self.warnings.clone()
    }
}

pub fn build(descriptor: &ActionDescriptor, decision: &Decision) -> CardView {
    let denied = decision.is_denied();
    let mut rows = Vec::new();

    rows.push(row("Acción", descriptor.verb.to_string(), false));
    if let Some(amount) = descriptor.amount {
        rows.push(row("Importe real", format!("{amount:.2}"), true));
    } else {
        rows.push(row("Importe real", "no determinado".into(), true));
    }
    rows.push(row(
        "Destinatario real",
        descriptor
            .recipient
            .as_deref()
            .map(clean)
            .unwrap_or_else(|| "no determinado".into()),
        true,
    ));
    rows.push(row(
        "Origen de la acción",
        descriptor
            .action_origin
            .as_deref()
            .map(clean)
            .unwrap_or_else(|| "desconocido".into()),
        true,
    ));
    rows.push(row("Documento", clean(&descriptor.document_origin), false));

    if !descriptor.fields.is_empty() {
        let fields = descriptor
            .fields
            .iter()
            .map(|(key, value)| format!("{}={}", clean(key), clean(value)))
            .collect::<Vec<_>>()
            .join(" · ");
        rows.push(row("Se enviará", fields, false));
    }

    let mut warnings: Vec<String> = Vec::new();
    for inconsistency in &descriptor.inconsistencies {
        warnings.push(inconsistency.describe());
    }
    if descriptor.cross_origin {
        warnings.push("la acción envía datos a otro origen".into());
    }
    for reason in decision.reasons() {
        if !warnings.iter().any(|w| w == reason) {
            warnings.push(reason.clone());
        }
    }
    if descriptor
        .inconsistencies
        .iter()
        .any(|i| matches!(i, Inconsistency::VisibleAmountMismatch { .. }))
    {
        warnings.push("no confíes en la etiqueta: la tarjeta muestra el valor real".into());
    }
    warnings.dedup();

    CardView {
        title: if denied {
            "Acción bloqueada".into()
        } else {
            "Confirmar acción".into()
        },
        rows,
        warnings,
        denied,
        approve_hash: decision.hash().map(str::to_string),
        deny_first: true,
    }
}

fn row(label: &str, value: String, emphasis: bool) -> CardRow {
    CardRow {
        label: label.into(),
        value,
        emphasis,
    }
}

/// Convierte contenido de la página en una sola línea de texto plano.
pub fn clean(input: &str) -> String {
    let mut single = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_control() {
            single.push(' ');
        } else {
            single.push(ch);
        }
    }
    let collapsed = single.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(160).collect()
}

/// Render de texto para CLI, tests y entornos sin pantalla.
pub fn render_text(view: &CardView) -> String {
    let mut out = String::new();
    out.push_str(&format!("┌─ {} ", view.title));
    out.push_str(&"─".repeat(46usize.saturating_sub(view.title.chars().count())));
    out.push_str("┐\n");

    for row in &view.rows {
        let marker = if row.emphasis { "▸" } else { " " };
        out.push_str(&format!(
            "  {marker} {:<20} {}\n",
            format!("{}:", row.label),
            row.value
        ));
    }

    if !view.warnings.is_empty() {
        out.push_str("  ⚠ avisos\n");
        for warning in &view.warnings {
            out.push_str(&format!("    · {warning}\n"));
        }
    }

    out.push_str("  ────────────────────────────────────────────\n");
    if view.denied {
        out.push_str("  [ Denegar ]  (predeterminada)\n");
    } else {
        out.push_str("  [ Denegar ]  (predeterminada)   [ Aprobar ]\n");
        if let Some(hash) = &view.approve_hash {
            let short: String = hash.chars().take(16).collect();
            out.push_str(&format!("  aprobación ligada al hash {short}…\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use descriptor::{Intent, Provenance, RawAction, Target, Verb};
    use std::collections::BTreeMap;

    fn raw(amount: &str, to: &str, action: &str, visible: &str) -> RawAction {
        let mut fields = BTreeMap::new();
        fields.insert("amount".into(), amount.into());
        fields.insert("to".into(), to.into());
        RawAction {
            verb: Verb::Submit,
            document_origin: "http://127.0.0.1:5000".into(),
            document_url: "http://127.0.0.1:5000/pay".into(),
            action_url: Some(action.into()),
            method: Some("POST".into()),
            fields,
            amount: amount.parse().ok(),
            recipient: Some(to.into()),
            file: None,
            visible_text: visible.into(),
            target: Target {
                selector: "#go".into(),
                tag: "BUTTON".into(),
                name: "Confirmar".into(),
                x: 1.0,
                y: 1.0,
            },
        }
    }

    fn intent() -> Intent {
        Intent {
            provenance: Provenance::User,
            verb: Some(Verb::Submit),
            amount: Some(1.0),
            recipient: Some("ana".into()),
            allowed_action_origins: vec!["http://127.0.0.1:5000".into()],
            max_amount: 100.0,
            require_approval: true,
        }
    }

    #[test]
    fn la_tarjeta_muestra_el_importe_real_y_el_aviso() {
        let d = ActionDescriptor::from_raw(
            raw(
                "1000.00",
                "attacker-acct-999",
                "http://127.0.0.1:5000/pay",
                "Enviar $1.00 a Ana",
            ),
            Provenance::User,
            1,
        );
        let decision = intent().evaluate(&d, None);
        let view = build(&d, &decision);

        assert!(view.denied);
        assert_eq!(view.approve_hash, None);
        assert!(view.deny_first);
        let text = render_text(&view);
        assert!(text.contains("1000.00"), "{text}");
        assert!(text.contains("attacker-acct-999"), "{text}");
        assert!(
            view.warnings
                .iter()
                .any(|w| w.contains("muestra") && w.contains("1000.00")),
            "{:?}",
            view.warnings
        );
    }

    #[test]
    fn la_tarjeta_legitima_ofrece_aprobar_con_hash() {
        let d = ActionDescriptor::from_raw(
            raw(
                "1.00",
                "ana",
                "http://127.0.0.1:5000/pay",
                "Enviar $1.00 a Ana",
            ),
            Provenance::User,
            1,
        );
        let decision = intent().evaluate(&d, None);
        let view = build(&d, &decision);

        assert!(!view.denied);
        assert_eq!(view.approve_hash.as_deref(), Some(d.hash_hex.as_str()));
        assert!(view.warnings.is_empty(), "{:?}", view.warnings);
    }

    #[test]
    fn el_contenido_de_la_pagina_no_puede_inyectar_lineas_ni_marcado() {
        let d = ActionDescriptor::from_raw(
            raw(
                "1.00",
                "ana\n<script>alert(1)</script>",
                "http://127.0.0.1:5000/pay",
                "Enviar $1.00 a Ana",
            ),
            Provenance::User,
            1,
        );
        assert_eq!(
            clean("ana\n<script>alert(1)</script>"),
            "ana <script>alert(1)</script>"
        );

        let decision = intent().evaluate(&d, None);
        let text = render_text(&build(&d, &decision));
        let recipient_lines: Vec<&str> = text
            .lines()
            .filter(|line| line.contains("Destinatario real"))
            .collect();
        assert_eq!(recipient_lines.len(), 1);
        assert!(recipient_lines[0].contains("<script>alert(1)</script>"));
        assert!(!recipient_lines[0].contains('\n'));
    }
}
