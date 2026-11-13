//! Descriptor de acción ground-truth, política y binding.
//!
//! Este crate no sabe nada de navegadores. Recibe lo que se observó en el DOM
//! (`RawAction`), lo sella con un hash canónico y decide si una intención
//! humana/plan puede autorizarlo.
//!
//! Las tres piezas del MVP viven aquí:
//!   1. `ActionDescriptor`: qué se va a ejecutar de verdad.
//!   2. `Intent`/`Decision`: si eso coincide con lo que la persona pidió.
//!   3. `verify_wire`: si lo que salió por la red es lo que se aprobó.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const SCHEMA: &str = "navegador.action/1";
const AMOUNT_EPSILON: f64 = 0.005;

// ---------------------------------------------------------------------------
// Tipos base
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verb {
    Click,
    Submit,
    Type,
    Navigate,
    Download,
}

impl fmt::Display for Verb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Verb::Click => "click",
            Verb::Submit => "submit",
            Verb::Type => "type",
            Verb::Navigate => "navigate",
            Verb::Download => "download",
        };
        f.write_str(s)
    }
}

/// De dónde salió la intención. Una intención originada en la página jamás
/// puede autorizar una acción con consecuencias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    User,
    Plan,
    Page,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub selector: String,
    pub tag: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
}

/// Desajuste entre lo que la página *dice* y lo que la página *hace*.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Inconsistency {
    /// El texto visible muestra un importe distinto del que se enviará.
    VisibleAmountMismatch {
        visible_amount: f64,
        actual_amount: f64,
    },
    /// El formulario apunta a otro origen que el documento.
    CrossOriginAction {
        document_origin: String,
        action_origin: String,
    },
}

impl Inconsistency {
    pub fn describe(&self) -> String {
        match self {
            Inconsistency::VisibleAmountMismatch {
                visible_amount,
                actual_amount,
            } => {
                format!("la página muestra {visible_amount:.2} pero se enviarán {actual_amount:.2}")
            }
            Inconsistency::CrossOriginAction {
                document_origin,
                action_origin,
            } => format!("la acción sale de {document_origin} hacia {action_origin}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Descriptor
// ---------------------------------------------------------------------------

/// Lo que el motor observó en el DOM, sin interpretar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawAction {
    pub verb: Verb,
    pub document_origin: String,
    pub document_url: String,
    pub action_url: Option<String>,
    pub method: Option<String>,
    pub fields: BTreeMap<String, String>,
    pub amount: Option<f64>,
    pub recipient: Option<String>,
    pub file: Option<String>,
    pub visible_text: String,
    pub target: Target,
}

/// El descriptor sellado. `hash_hex` cubre todos los demás campos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionDescriptor {
    pub schema: String,
    pub generation: u32,
    pub provenance: Provenance,
    pub verb: Verb,
    pub document_origin: String,
    pub document_url: String,
    pub action_url: Option<String>,
    pub action_origin: Option<String>,
    pub method: Option<String>,
    pub fields: BTreeMap<String, String>,
    pub amount: Option<f64>,
    pub recipient: Option<String>,
    pub file: Option<String>,
    pub visible_text: String,
    pub visible_amounts: Vec<f64>,
    pub cross_origin: bool,
    pub inconsistencies: Vec<Inconsistency>,
    pub target: Target,
    pub hash_hex: String,
}

impl ActionDescriptor {
    pub fn from_raw(raw: RawAction, provenance: Provenance, generation: u32) -> Self {
        let action_origin = raw.action_url.as_deref().and_then(origin_of);
        let cross_origin = match (&action_origin, raw.document_origin.as_str()) {
            (Some(a), d) if !d.is_empty() && d != "null" => a != d,
            _ => false,
        };

        let visible_amounts = visible_amounts(&raw.visible_text);
        let mut inconsistencies = Vec::new();

        if let (Some(actual), Some(_)) = (raw.amount, visible_amounts.first()) {
            let agrees = visible_amounts
                .iter()
                .any(|v| (v - actual).abs() <= AMOUNT_EPSILON);
            if !agrees {
                inconsistencies.push(Inconsistency::VisibleAmountMismatch {
                    visible_amount: visible_amounts[0],
                    actual_amount: actual,
                });
            }
        }
        if cross_origin && let (Some(a), d) = (&action_origin, &raw.document_origin) {
            inconsistencies.push(Inconsistency::CrossOriginAction {
                document_origin: d.clone(),
                action_origin: a.clone(),
            });
        }

        let mut d = ActionDescriptor {
            schema: SCHEMA.to_string(),
            generation,
            provenance,
            verb: raw.verb,
            document_origin: raw.document_origin,
            document_url: raw.document_url,
            action_url: raw.action_url,
            action_origin,
            method: raw.method.map(|m| m.to_uppercase()),
            fields: raw.fields,
            amount: raw.amount,
            recipient: raw.recipient,
            file: raw.file,
            visible_text: raw.visible_text,
            visible_amounts,
            cross_origin,
            inconsistencies,
            target: raw.target,
            hash_hex: String::new(),
        };
        d.seal();
        d
    }

    fn seal(&mut self) {
        self.hash_hex = self.compute_hash();
    }

    /// sha256 del JSON canónico sin el propio hash.
    pub fn compute_hash(&self) -> String {
        let mut value = serde_json::to_value(self).expect("el descriptor es serializable");
        if let Value::Object(map) = &mut value {
            map.remove("hash_hex");
        }
        let canonical = canonical_json(&value);
        hex::encode(Sha256::digest(canonical.as_bytes()))
    }

    pub fn hash_is_valid(&self) -> bool {
        !self.hash_hex.is_empty() && self.compute_hash() == self.hash_hex
    }

    pub fn to_canonical_json(&self) -> String {
        canonical_json(&serde_json::to_value(self).expect("serializable"))
    }

    /// Cuerpo urlencoded que el formulario producirá.
    pub fn expected_body(&self) -> String {
        let mut ser = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in &self.fields {
            ser.append_pair(k, v);
        }
        ser.finish()
    }
}

fn origin_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let origin = parsed.origin();
    if origin.is_tuple() {
        Some(origin.ascii_serialization())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// JSON canónico
// ---------------------------------------------------------------------------

/// JSON con claves ordenadas y sin ambigüedad de formato.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).expect("clave serializable"),
                        canonical_json(&map[k])
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        other => serde_json::to_string(other).expect("valor serializable"),
    }
}

// ---------------------------------------------------------------------------
// Intención y política
// ---------------------------------------------------------------------------

/// Lo que la persona (o el plan) pidió, en términos verificables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    pub provenance: Provenance,
    /// Verbo esperado. `None` significa "el que haga el elemento"; la tarjeta
    /// muestra el real y la política lo comprueba igual.
    pub verb: Option<Verb>,
    pub amount: Option<f64>,
    pub recipient: Option<String>,
    /// Orígenes donde esta intención puede ejecutarse.
    pub allowed_action_origins: Vec<String>,
    pub max_amount: f64,
    /// Si es true, toda acción consecuente exige un hash aprobado.
    pub require_approval: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Allow,
    RequireApproval {
        hash_hex: String,
        reasons: Vec<String>,
    },
    Deny {
        reasons: Vec<String>,
    },
}

impl Decision {
    pub fn is_denied(&self) -> bool {
        matches!(self, Decision::Deny { .. })
    }

    pub fn is_allowed(&self) -> bool {
        matches!(self, Decision::Allow)
    }

    pub fn hash(&self) -> Option<&str> {
        match self {
            Decision::RequireApproval { hash_hex, .. } => Some(hash_hex),
            _ => None,
        }
    }

    pub fn reasons(&self) -> &[String] {
        match self {
            Decision::Allow => &[],
            Decision::RequireApproval { reasons, .. } | Decision::Deny { reasons } => reasons,
        }
    }
}

impl Intent {
    /// Decide si el descriptor observado satisface la intención.
    ///
    /// `approved_hash` es el hash que la persona aprobó explícitamente. Si no
    /// coincide con el descriptor actual, la aprobación no sirve.
    pub fn evaluate(&self, d: &ActionDescriptor, approved_hash: Option<&str>) -> Decision {
        let mut deny: Vec<String> = Vec::new();

        if self.provenance == Provenance::Page {
            deny.push("la intención se originó en la página, no en la persona o el plan".into());
        }
        if !d.hash_is_valid() {
            deny.push("el descriptor no coincide con su propio hash".into());
        }
        if let Some(want) = self.verb
            && d.verb != want
        {
            deny.push(format!(
                "la acción observada es {} pero la intención era {}",
                d.verb, want
            ));
        }
        match &d.action_origin {
            None => deny.push("no se pudo resolver el origen real de la acción".into()),
            Some(origin) => {
                if !self
                    .allowed_action_origins
                    .iter()
                    .any(|allowed| allowed == origin)
                {
                    deny.push(format!("origen de acción no permitido: {origin}"));
                }
            }
        }
        if let Some(amount) = d.amount
            && amount > self.max_amount + AMOUNT_EPSILON
        {
            deny.push(format!(
                "importe {amount:.2} excede el máximo permitido {:.2}",
                self.max_amount
            ));
        }
        match (self.amount, d.amount) {
            (Some(want), Some(got)) => {
                if (want - got).abs() > AMOUNT_EPSILON {
                    deny.push(format!(
                        "importe real {got:.2} distinto de la intención {want:.2}"
                    ));
                }
            }
            (Some(want), None) => deny.push(format!(
                "la intención autoriza {want:.2} pero no se pudo leer el importe real"
            )),
            _ => {}
        }
        match (&self.recipient, &d.recipient) {
            (Some(want), Some(got)) => {
                if normalize_recipient(want) != normalize_recipient(got) {
                    deny.push(format!(
                        "destinatario real {got:?} distinto de la intención {want:?}"
                    ));
                }
            }
            (Some(want), None) => deny.push(format!(
                "la intención nombra a {want:?} pero no se pudo leer el destinatario real"
            )),
            _ => {}
        }

        if !deny.is_empty() {
            return Decision::Deny { reasons: deny };
        }

        let mut reasons: Vec<String> = d.inconsistencies.iter().map(|i| i.describe()).collect();
        if d.cross_origin {
            reasons.push("la acción cruza de origen".to_string());
        }
        if self.require_approval || !reasons.is_empty() {
            if approved_hash == Some(d.hash_hex.as_str()) {
                return Decision::Allow;
            }
            return Decision::RequireApproval {
                hash_hex: d.hash_hex.clone(),
                reasons,
            };
        }
        Decision::Allow
    }
}

fn normalize_recipient(s: &str) -> String {
    s.trim().to_lowercase()
}

// ---------------------------------------------------------------------------
// Binding: lo aprobado contra lo que salió por la red
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BindingError {
    #[error("método observado {observed} ≠ aprobado {approved}")]
    Method { approved: String, observed: String },
    #[error("URL observada {observed} ≠ aprobada {approved}")]
    Url { approved: String, observed: String },
    #[error("campo {field}: aprobado {approved:?}, observado {observed:?}")]
    Field {
        field: String,
        approved: Option<String>,
        observed: Option<String>,
    },
}

/// Comprueba que la petición interceptada es exactamente la aprobada.
pub fn verify_wire(
    approved: &ActionDescriptor,
    observed_url: &str,
    observed_method: &str,
    observed_body: Option<&str>,
) -> Result<(), BindingError> {
    let want_method = approved.method.clone().unwrap_or_else(|| "GET".into());
    if !want_method.eq_ignore_ascii_case(observed_method) {
        return Err(BindingError::Method {
            approved: want_method,
            observed: observed_method.to_string(),
        });
    }

    let want_url = approved
        .action_url
        .as_deref()
        .ok_or_else(|| BindingError::Url {
            approved: "<ninguna>".into(),
            observed: observed_url.to_string(),
        })?;
    if strip_fragment(want_url) != strip_fragment(observed_url) {
        return Err(BindingError::Url {
            approved: want_url.to_string(),
            observed: observed_url.to_string(),
        });
    }

    let observed_params: BTreeMap<String, String> = if want_method.eq_ignore_ascii_case("POST") {
        parse_pairs(observed_body.unwrap_or(""))
    } else {
        url::Url::parse(observed_url)
            .ok()
            .map(|u| parse_pairs(u.query().unwrap_or("")))
            .unwrap_or_default()
    };

    let mut keys: Vec<&String> = approved.fields.keys().collect();
    for key in observed_params.keys() {
        if !approved.fields.contains_key(key) {
            keys.push(key);
        }
    }
    for key in keys {
        let want = approved.fields.get(key);
        let got = observed_params.get(key);
        if want != got {
            return Err(BindingError::Field {
                field: key.clone(),
                approved: want.cloned(),
                observed: got.cloned(),
            });
        }
    }
    Ok(())
}

fn strip_fragment(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

fn parse_pairs(s: &str) -> BTreeMap<String, String> {
    url::form_urlencoded::parse(s.as_bytes())
        .into_owned()
        .collect()
}

// ---------------------------------------------------------------------------
// Importes visibles
// ---------------------------------------------------------------------------

/// Extrae importes monetarios del texto visible (`$1.00`, `€1.234,56`).
pub fn visible_amounts(text: &str) -> Vec<f64> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i], '$' | '€' | '£') {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let start = j;
            while j < chars.len()
                && (chars[j].is_ascii_digit() || chars[j] == '.' || chars[j] == ',')
            {
                j += 1;
            }
            let raw: String = chars[start..j].iter().collect();
            if let Some(v) = parse_amount(&raw) {
                out.push(v);
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

fn parse_amount(raw: &str) -> Option<f64> {
    if raw.is_empty() {
        return None;
    }
    let cleaned = if raw.contains(',') && raw.contains('.') {
        if raw.rfind(',') > raw.rfind('.') {
            raw.replace('.', "").replace(',', ".")
        } else {
            raw.replace(',', "")
        }
    } else if raw.contains(',') {
        let decimals = raw.split(',').nth(1).map(|s| s.len()).unwrap_or(0);
        if decimals == 3 {
            raw.replace(',', "")
        } else {
            raw.replace(',', ".")
        }
    } else {
        raw.to_string()
    };
    cleaned.parse::<f64>().ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(amount: &str, to: &str, action: &str, visible: &str) -> RawAction {
        let mut fields = BTreeMap::new();
        fields.insert("amount".to_string(), amount.to_string());
        fields.insert("to".to_string(), to.to_string());
        RawAction {
            verb: Verb::Submit,
            document_origin: "http://127.0.0.1:5000".into(),
            document_url: "http://127.0.0.1:5000/pay".into(),
            action_url: Some(action.into()),
            method: Some("POST".into()),
            fields,
            amount: parse_amount(amount),
            recipient: Some(to.into()),
            file: None,
            visible_text: visible.into(),
            target: Target {
                selector: "#go".into(),
                tag: "BUTTON".into(),
                name: "Confirmar".into(),
                x: 10.0,
                y: 10.0,
            },
        }
    }

    fn intent(amount: f64, recipient: &str) -> Intent {
        Intent {
            provenance: Provenance::User,
            verb: Some(Verb::Submit),
            amount: Some(amount),
            recipient: Some(recipient.into()),
            allowed_action_origins: vec!["http://127.0.0.1:5000".into()],
            max_amount: 100.0,
            require_approval: true,
        }
    }

    #[test]
    fn el_hash_es_estable_y_detecta_mutaciones() {
        let d = ActionDescriptor::from_raw(
            raw(
                "1.00",
                "ana",
                "http://127.0.0.1:5000/pay",
                "Transferencia de $1.00 a Ana",
            ),
            Provenance::User,
            1,
        );
        assert!(d.hash_is_valid());
        let mut copy = d.clone();
        copy.amount = Some(9999.0);
        assert!(!copy.hash_is_valid());
        assert_ne!(copy.compute_hash(), d.hash_hex);
    }

    #[test]
    fn una_intencion_originada_en_la_pagina_siempre_se_deniega() {
        let d = ActionDescriptor::from_raw(
            raw(
                "1.00",
                "ana",
                "http://127.0.0.1:5000/pay",
                "Transferencia de $1.00 a Ana",
            ),
            Provenance::Page,
            1,
        );
        let mut i = intent(1.0, "ana");
        i.provenance = Provenance::Page;
        assert!(i.evaluate(&d, Some(&d.hash_hex)).is_denied());
    }

    #[test]
    fn una_tarea_legitima_pasa_tras_aprobar() {
        let d = ActionDescriptor::from_raw(
            raw(
                "1.00",
                "ana",
                "http://127.0.0.1:5000/pay",
                "Transferencia de $1.00 a Ana",
            ),
            Provenance::User,
            1,
        );
        let i = intent(1.0, "ana");
        let pending = i.evaluate(&d, None);
        assert!(matches!(pending, Decision::RequireApproval { .. }));
        assert!(i.evaluate(&d, Some(&d.hash_hex)).is_allowed());
    }

    #[test]
    fn un_importe_distinto_de_la_intencion_se_deniega() {
        let d = ActionDescriptor::from_raw(
            raw(
                "1000.00",
                "ana",
                "http://127.0.0.1:5000/pay",
                "Transferencia de $1.00 a Ana",
            ),
            Provenance::User,
            1,
        );
        assert!(i_test(1.0, "ana").evaluate(&d, None).is_denied());
        assert_eq!(d.inconsistencies.len(), 1);
    }

    fn i_test(amount: f64, recipient: &str) -> Intent {
        intent(amount, recipient)
    }

    #[test]
    fn un_destinatario_distinto_se_deniega() {
        let d = ActionDescriptor::from_raw(
            raw(
                "1.00",
                "attacker-acct-999",
                "http://127.0.0.1:5000/pay",
                "Transferencia de $1.00 a Ana",
            ),
            Provenance::User,
            1,
        );
        assert!(intent(1.0, "ana").evaluate(&d, None).is_denied());
    }

    #[test]
    fn un_origen_de_accion_no_permitido_se_deniega() {
        let d = ActionDescriptor::from_raw(
            raw(
                "1.00",
                "ana",
                "http://127.0.0.1:6000/collect",
                "Transferencia de $1.00 a Ana",
            ),
            Provenance::User,
            1,
        );
        assert!(d.cross_origin);
        assert!(intent(1.0, "ana").evaluate(&d, None).is_denied());
    }

    #[test]
    fn el_binding_acepta_la_peticion_exacta() {
        let d = ActionDescriptor::from_raw(
            raw(
                "1.00",
                "ana",
                "http://127.0.0.1:5000/pay",
                "Transferencia de $1.00 a Ana",
            ),
            Provenance::User,
            1,
        );
        assert!(
            verify_wire(
                &d,
                "http://127.0.0.1:5000/pay",
                "POST",
                Some("to=ana&amount=1.00")
            )
            .is_ok()
        );
        assert!(
            verify_wire(
                &d,
                "http://127.0.0.1:5000/pay",
                "POST",
                Some("to=mula&amount=1.00")
            )
            .is_err()
        );
        assert!(
            verify_wire(
                &d,
                "http://127.0.0.1:5000/pay",
                "POST",
                Some("to=ana&amount=9999.00")
            )
            .is_err()
        );
        assert!(
            verify_wire(
                &d,
                "http://127.0.0.1:6000/collect",
                "POST",
                Some("to=ana&amount=1.00")
            )
            .is_err()
        );
    }

    #[test]
    fn parsea_importes_visibles() {
        assert_eq!(visible_amounts("Pagar $1.00 ahora"), vec![1.0]);
        assert_eq!(visible_amounts("Total: €1.234,56"), vec![1234.56]);
        assert_eq!(visible_amounts("sin importe"), Vec::<f64>::new());
    }
}
