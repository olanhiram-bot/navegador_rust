//! Gate de la semana 2: binding 10/10 y recibo verificable.
//!
//!   - la sustitución posterior a la aprobación se aborta siempre;
//!   - el recibo de una acción legítima verifica y detecta manipulación;
//!   - la verificación funciona en otro proceso, sin estado compartido.

use std::collections::BTreeMap;

use descriptor::{ActionDescriptor, Decision, Intent, Provenance, RawAction, Target, Verb};
use engine_cdp::{Chrome, DispatchOutcome};
use navegador::FixtureServer;
use receipt::{Identity, WireRequest};

struct Harness {
    server: FixtureServer,
    chrome: Chrome,
}

impl Harness {
    async fn new() -> Self {
        let server = FixtureServer::start();
        let chrome = Chrome::launch().await.expect("Chromium debe lanzarse");
        Harness { server, chrome }
    }

    async fn open(&mut self, fixture: &str) {
        let url = self.server.page_url(fixture);
        self.chrome
            .navigate(&url)
            .await
            .expect("la página debe cargar");
    }

    fn intent(&self) -> Intent {
        Intent {
            provenance: Provenance::User,
            verb: Verb::Submit,
            amount: Some(1.00),
            recipient: Some("ana".into()),
            allowed_action_origins: vec![self.server.page_origin()],
            max_amount: 100.0,
            require_approval: true,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sustitucion_bloqueada_10_de_10() {
    let mut harness = Harness::new().await;
    let mut blocked = 0;

    for iteration in 0..10 {
        harness.open("substitute.html").await;
        let descriptor = harness
            .chrome
            .extract("#go", 1, Provenance::User)
            .await
            .expect("el descriptor debe formarse");

        // Antes del click todo parece legítimo: por eso hay que aprobar.
        let pending = harness.intent().evaluate(&descriptor, None);
        assert!(
            matches!(pending, Decision::RequireApproval { .. }),
            "iteración {iteration}: {pending:?}"
        );

        let outcome = harness
            .chrome
            .dispatch(&descriptor)
            .await
            .expect("el despacho no debe fallar técnicamente");
        match outcome {
            DispatchOutcome::Blocked { .. } => blocked += 1,
            other => panic!("iteración {iteration}: se esperaba Blocked, se obtuvo {other:?}"),
        }
    }

    assert_eq!(blocked, 10, "la sustitución debe abortarse siempre");
    assert!(
        harness.server.requests().is_empty(),
        "ninguna petición sustituida debe llegar al servidor"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recibo_firmado_verifica_y_detecta_manipulacion() {
    let mut harness = Harness::new().await;
    harness.open("legit.html").await;

    let descriptor = harness
        .chrome
        .extract("#go", 1, Provenance::User)
        .await
        .expect("el descriptor debe formarse");

    let intent = harness.intent();
    let approved_hash = intent
        .evaluate(&descriptor, None)
        .hash()
        .expect("debe pedir aprobación")
        .to_string();
    assert!(
        intent
            .evaluate(&descriptor, Some(&approved_hash))
            .is_allowed()
    );

    let outcome = harness
        .chrome
        .dispatch(&descriptor)
        .await
        .expect("despacho");
    let (url, method, body) = match outcome {
        DispatchOutcome::Sent { url, method, body } => (url, method, body),
        other => panic!("la tarea legítima debía ejecutarse: {other:?}"),
    };

    let identity = Identity::generate().expect("entropía");
    let signed = identity.sign(
        &descriptor,
        WireRequest {
            url: url.clone(),
            method: method.clone(),
            body: body.clone(),
        },
        "sent",
    );
    receipt::verify(&signed).expect("el recibo debe verificar");
    receipt::verify_json(&serde_json::to_string(&signed).unwrap()).expect("JSON round-trip");

    let mut tampered_amount = signed.clone();
    tampered_amount.descriptor.amount = Some(9999.0);
    assert!(
        receipt::verify(&tampered_amount).is_err(),
        "cambiar el importe debe invalidar el recibo"
    );

    let mut tampered_wire = signed.clone();
    tampered_wire.wire.body = Some("to=mula&amount=1.00".into());
    assert!(
        receipt::verify(&tampered_wire).is_err(),
        "cambiar la petición debe invalidar el recibo"
    );
}

fn synthetic_descriptor() -> ActionDescriptor {
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
            target: Target {
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

#[test]
fn el_recibo_se_verifica_en_otro_proceso() {
    let identity = Identity::from_seed_hex(&"77".repeat(32)).expect("semilla");
    let signed = identity.sign(
        &synthetic_descriptor(),
        WireRequest {
            url: "http://127.0.0.1:5000/pay".into(),
            method: "POST".into(),
            body: Some("to=ana&amount=1.00".into()),
        },
        "sent",
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("recibo.json");
    std::fs::write(&path, serde_json::to_string_pretty(&signed).unwrap()).unwrap();

    let binary = env!("CARGO_BIN_EXE_navegador");
    let valid = std::process::Command::new(binary)
        .arg("verify")
        .arg(&path)
        .output()
        .expect("ejecutar navegador verify");
    assert!(
        valid.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&valid.stderr)
    );
    assert!(String::from_utf8_lossy(&valid.stdout).contains("RECIBO VÁLIDO"));

    let mut tampered = signed.clone();
    tampered.wire.body = Some("to=mula&amount=1.00".into());
    std::fs::write(&path, serde_json::to_string_pretty(&tampered).unwrap()).unwrap();

    let rejected = std::process::Command::new(binary)
        .arg("verify")
        .arg(&path)
        .output()
        .expect("ejecutar navegador verify");
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("RECHAZADO"));
}
