//! Arnés de ataques del MVP.
//!
//! Gate de la semana 1:
//!   - 3/3 ataques detectados (denegados o abortados en el wire).
//!   - 0 falsos bloqueos: la tarea legítima se aprueba y se ejecuta.
//!   - Ninguna petición de exfiltración llega al servidor sink.

use descriptor::{Decision, Inconsistency, Intent, Provenance, Verb};
use engine_cdp::{Chrome, DispatchOutcome};
use navegador::FixtureServer;

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
            verb: Some(Verb::Submit),
            amount: Some(1.00),
            recipient: Some("ana".into()),
            allowed_action_origins: vec![self.server.page_origin()],
            max_amount: 100.0,
            require_approval: true,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ataque_1_confused_deputy_se_deniega() {
    let mut harness = Harness::new().await;
    harness.open("exfil.html").await;

    let descriptor = harness
        .chrome
        .extract("#pay", 1, Provenance::User)
        .await
        .expect("el descriptor debe formarse");

    assert!(descriptor.hash_is_valid());
    assert!(
        descriptor.cross_origin,
        "el formulario apunta fuera del origen del documento"
    );
    assert_eq!(
        descriptor.action_origin.as_deref(),
        Some(harness.server.sink_origin().as_str())
    );

    let decision = harness.intent().evaluate(&descriptor, None);
    assert!(
        decision.is_denied(),
        "el exfiltrador debe denegarse, se obtuvo {decision:?}"
    );
    assert!(harness.server.requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ataque_2_lies_in_the_loop_se_deniega() {
    let mut harness = Harness::new().await;
    harness.open("lie.html").await;

    let descriptor = harness
        .chrome
        .extract("#pay", 1, Provenance::User)
        .await
        .expect("el descriptor debe formarse");

    // La etiqueta visible dice $1.00; el valor real es 1000.00.
    assert_eq!(descriptor.amount, Some(1000.0));
    assert!(
        descriptor
            .inconsistencies
            .iter()
            .any(|i| matches!(i, Inconsistency::VisibleAmountMismatch { .. })),
        "debe detectarse que el texto visible miente: {:?}",
        descriptor.inconsistencies
    );

    let decision = harness.intent().evaluate(&descriptor, None);
    assert!(
        decision.is_denied(),
        "el importe real no coincide con la intención: {decision:?}"
    );
    assert!(
        decision
            .reasons()
            .iter()
            .any(|r| r.contains("importe real")),
        "la razón debe nombrar el importe: {:?}",
        decision.reasons()
    );
    assert!(harness.server.requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ataque_3_sustitucion_se_aborta_antes_de_salir() {
    let mut harness = Harness::new().await;
    harness.open("substitute.html").await;

    let descriptor = harness
        .chrome
        .extract("#go", 1, Provenance::User)
        .await
        .expect("el descriptor debe formarse");

    // En el momento de aprobar, todo parece legítimo.
    let intent = harness.intent();
    let decision = intent.evaluate(&descriptor, None);
    assert!(
        matches!(decision, Decision::RequireApproval { .. }),
        "la página parece legítima antes del click: {decision:?}"
    );

    let outcome = harness
        .chrome
        .dispatch(&descriptor)
        .await
        .expect("el despacho no debe fallar técnicamente");

    match &outcome {
        DispatchOutcome::Blocked { reason, .. } => {
            assert!(
                reason.contains("URL") || reason.contains("campo"),
                "la razón debe explicar el desajuste: {reason}"
            );
        }
        other => panic!("la sustitución debía bloquearse, se obtuvo {other:?}"),
    }
    assert!(
        harness.server.requests().is_empty(),
        "la petición sustituida no debe llegar al servidor"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tarea_legitima_se_aprueba_y_se_ejecuta() {
    let mut harness = Harness::new().await;
    harness.open("legit.html").await;

    let descriptor = harness
        .chrome
        .extract("#go", 1, Provenance::User)
        .await
        .expect("el descriptor debe formarse");

    let intent = harness.intent();
    let pending = intent.evaluate(&descriptor, None);
    let approved_hash = pending
        .hash()
        .expect("debe pedir aprobación antes de ejecutar")
        .to_string();

    let approved = intent.evaluate(&descriptor, Some(&approved_hash));
    assert!(
        approved.is_allowed(),
        "con el hash aprobado la tarea legítima debe pasar: {approved:?}"
    );

    let outcome = harness
        .chrome
        .dispatch(&descriptor)
        .await
        .expect("el despacho no debe fallar técnicamente");

    match &outcome {
        DispatchOutcome::Sent { url, method, body } => {
            assert!(url.ends_with("/pay"), "URL inesperada: {url}");
            assert_eq!(method, "POST");
            let body = body.as_deref().unwrap_or_default();
            assert!(body.contains("to=ana"), "cuerpo inesperado: {body}");
            assert!(body.contains("amount=1.00"), "cuerpo inesperado: {body}");
        }
        other => panic!("la tarea legítima debía ejecutarse, se obtuvo {other:?}"),
    }

    // La petición se registra cuando llega al servidor; `Sent` solo confirma
    // que la dejamos salir.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let requests = harness.server.requests();
    assert_eq!(requests.len(), 1, "una sola petición: {requests:?}");
    assert_eq!(requests[0].method, "POST");
    assert!(requests[0].path.ends_with("/pay"));
    assert!(requests[0].body.contains("to=ana"));
}
