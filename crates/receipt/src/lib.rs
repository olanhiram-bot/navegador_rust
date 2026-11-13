//! Recibo firmado de una acción ejecutada.
//!
//! El recibo prueba dos cosas sin infraestructura:
//!   1. que el descriptor aprobado no fue alterado (hash + firma);
//!   2. que la petición que salió es exactamente la aprobada (binding).
//!
//! La verificación es offline: solo necesita el JSON y la clave pública que
//! lleva dentro.

use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use descriptor::{ActionDescriptor, BindingError};
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use signature::Signer;

pub const RECEIPT_SCHEMA: &str = "navegador.receipt/1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireRequest {
    pub url: String,
    pub method: String,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub schema: String,
    pub issued_at_unix: u64,
    pub descriptor: ActionDescriptor,
    pub wire: WireRequest,
    pub outcome: String,
    pub public_key: String,
    pub signature: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ReceiptError {
    #[error("esquema desconocido: {0}")]
    Schema(String),
    #[error("clave pública inválida")]
    BadPublicKey,
    #[error("firma inválida")]
    InvalidSignature,
    #[error("el descriptor no coincide con su hash")]
    HashMismatch,
    #[error("la petición no coincide con lo aprobado: {0}")]
    Binding(#[from] BindingError),
    #[error("no se pudo leer entropía del sistema")]
    NoEntropy,
    #[error("hex inválido: {0}")]
    Hex(String),
    #[error("JSON inválido: {0}")]
    Json(String),
}

/// Identidad local. La semilla es el secreto; la clave pública va en el recibo.
pub struct Identity {
    signing: SigningKey,
}

impl Identity {
    pub fn generate() -> Result<Self, ReceiptError> {
        let mut seed = [0u8; 32];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut seed))
            .map_err(|_| ReceiptError::NoEntropy)?;
        Ok(Identity {
            signing: SigningKey::from_bytes(&seed),
        })
    }

    pub fn from_seed_hex(seed: &str) -> Result<Self, ReceiptError> {
        let bytes = hex::decode(seed.trim()).map_err(|e| ReceiptError::Hex(e.to_string()))?;
        let seed: [u8; 32] = bytes
            .try_into()
            .map_err(|_| ReceiptError::Hex("la semilla debe tener 32 bytes".into()))?;
        Ok(Identity {
            signing: SigningKey::from_bytes(&seed),
        })
    }

    pub fn seed_hex(&self) -> String {
        hex::encode(self.signing.to_bytes())
    }

    pub fn public_hex(&self) -> String {
        hex::encode(self.signing.verifying_key().to_bytes())
    }

    pub fn sign(&self, descriptor: &ActionDescriptor, wire: WireRequest, outcome: &str) -> Receipt {
        let issued_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let public_key = self.public_hex();

        let payload = payload_value(
            RECEIPT_SCHEMA,
            issued_at_unix,
            descriptor,
            &wire,
            outcome,
            &public_key,
        );
        let bytes = descriptor::canonical_json(&payload);
        let signature = self.signing.sign(bytes.as_bytes());

        Receipt {
            schema: RECEIPT_SCHEMA.to_string(),
            issued_at_unix,
            descriptor: descriptor.clone(),
            wire,
            outcome: outcome.to_string(),
            public_key,
            signature: hex::encode(signature.to_bytes()),
        }
    }
}

fn payload_value(
    schema: &str,
    issued_at_unix: u64,
    descriptor: &ActionDescriptor,
    wire: &WireRequest,
    outcome: &str,
    public_key: &str,
) -> Value {
    serde_json::json!({
        "schema": schema,
        "issued_at_unix": issued_at_unix,
        "descriptor": descriptor,
        "wire": wire,
        "outcome": outcome,
        "public_key": public_key,
    })
}

/// Verifica firma, integridad del descriptor y binding con la petición.
pub fn verify(receipt: &Receipt) -> Result<(), ReceiptError> {
    if receipt.schema != RECEIPT_SCHEMA {
        return Err(ReceiptError::Schema(receipt.schema.clone()));
    }

    let key_bytes = hex::decode(&receipt.public_key).map_err(|_| ReceiptError::BadPublicKey)?;
    let key_bytes: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| ReceiptError::BadPublicKey)?;
    let verifying = VerifyingKey::from_bytes(&key_bytes).map_err(|_| ReceiptError::BadPublicKey)?;

    let signature_bytes =
        hex::decode(&receipt.signature).map_err(|_| ReceiptError::InvalidSignature)?;
    let signature_bytes: [u8; 64] = signature_bytes
        .try_into()
        .map_err(|_| ReceiptError::InvalidSignature)?;
    let signature = Signature::from_bytes(&signature_bytes);

    let payload = payload_value(
        &receipt.schema,
        receipt.issued_at_unix,
        &receipt.descriptor,
        &receipt.wire,
        &receipt.outcome,
        &receipt.public_key,
    );
    let bytes = descriptor::canonical_json(&payload);
    verifying
        .verify_strict(bytes.as_bytes(), &signature)
        .map_err(|_| ReceiptError::InvalidSignature)?;

    if !receipt.descriptor.hash_is_valid() {
        return Err(ReceiptError::HashMismatch);
    }

    descriptor::verify_wire(
        &receipt.descriptor,
        &receipt.wire.url,
        &receipt.wire.method,
        receipt.wire.body.as_deref(),
    )?;

    Ok(())
}

pub fn verify_json(json: &str) -> Result<Receipt, ReceiptError> {
    let receipt: Receipt =
        serde_json::from_str(json).map_err(|e| ReceiptError::Json(e.to_string()))?;
    verify(&receipt)?;
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use descriptor::{Provenance, RawAction, Verb};
    use std::collections::BTreeMap;

    fn descriptor() -> ActionDescriptor {
        let mut fields = BTreeMap::new();
        fields.insert("amount".to_string(), "1.00".to_string());
        fields.insert("to".to_string(), "ana".to_string());
        ActionDescriptor::from_raw(
            RawAction {
                verb: Verb::Submit,
                document_origin: "http://127.0.0.1:5000".into(),
                document_url: "http://127.0.0.1:5000/legit.html".into(),
                action_url: Some("http://127.0.0.1:5000/pay".into()),
                method: Some("POST".into()),
                fields,
                amount: Some(1.0),
                recipient: Some("ana".into()),
                file: None,
                visible_text: "Transferencia de $1.00 a Ana".into(),
                target: descriptor::Target {
                    selector: "#go".into(),
                    tag: "BUTTON".into(),
                    name: "Confirmar".into(),
                    x: 10.0,
                    y: 10.0,
                },
            },
            Provenance::User,
            1,
        )
    }

    fn wire() -> WireRequest {
        WireRequest {
            url: "http://127.0.0.1:5000/pay".into(),
            method: "POST".into(),
            body: Some("to=ana&amount=1.00".into()),
        }
    }

    #[test]
    fn un_recibo_valido_se_verifica() {
        let identity = Identity::from_seed_hex(&"11".repeat(32)).unwrap();
        let receipt = identity.sign(&descriptor(), wire(), "sent");
        verify(&receipt).unwrap();

        let json = serde_json::to_string(&receipt).unwrap();
        verify_json(&json).unwrap();
    }

    #[test]
    fn cambiar_el_importe_despues_de_firmar_invalida_la_firma() {
        let identity = Identity::from_seed_hex(&"22".repeat(32)).unwrap();
        let mut receipt = identity.sign(&descriptor(), wire(), "sent");
        receipt.descriptor.amount = Some(9999.0);
        assert!(matches!(
            verify(&receipt),
            Err(ReceiptError::InvalidSignature)
        ));
    }

    #[test]
    fn firmar_un_descriptor_manipulado_falla_por_hash() {
        let identity = Identity::from_seed_hex(&"22".repeat(32)).unwrap();
        let mut broken = descriptor();
        broken.amount = Some(9999.0);
        assert!(!broken.hash_is_valid());
        let receipt = identity.sign(&broken, wire(), "sent");
        assert!(matches!(verify(&receipt), Err(ReceiptError::HashMismatch)));
    }

    #[test]
    fn firmar_una_peticion_distinta_falla_por_binding() {
        let identity = Identity::from_seed_hex(&"33".repeat(32)).unwrap();
        let receipt = identity.sign(
            &descriptor(),
            WireRequest {
                url: "http://127.0.0.1:5000/pay".into(),
                method: "POST".into(),
                body: Some("to=mula&amount=1.00".into()),
            },
            "sent",
        );
        assert!(matches!(verify(&receipt), Err(ReceiptError::Binding(_))));
    }

    #[test]
    fn cambiar_la_peticion_despues_de_firmar_invalida_la_firma() {
        let identity = Identity::from_seed_hex(&"33".repeat(32)).unwrap();
        let mut receipt = identity.sign(&descriptor(), wire(), "sent");
        receipt.wire.body = Some("to=mula&amount=1.00".into());
        assert!(matches!(
            verify(&receipt),
            Err(ReceiptError::InvalidSignature)
        ));
    }

    #[test]
    fn otra_clave_no_puede_firmar_en_nombre_de_la_primera() {
        let original = Identity::from_seed_hex(&"44".repeat(32)).unwrap();
        let impostor = Identity::from_seed_hex(&"55".repeat(32)).unwrap();
        let mut receipt = original.sign(&descriptor(), wire(), "sent");
        receipt.public_key = impostor.public_hex();
        assert!(matches!(
            verify(&receipt),
            Err(ReceiptError::InvalidSignature)
        ));
    }
}
