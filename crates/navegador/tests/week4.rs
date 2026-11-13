//! Gate de la fase 4: política de orígenes y escenarios realistas.
//!
//! - Tres escenarios legítimos (compra, perfil, reserva) se ejecutan y dejan
//!   recibo: **0 falsos bloqueos**.
//! - Un origen de solo lectura se puede abrir y leer, pero no recibir datos.
//! - Un origen fuera de la lista de lectura no se abre.
//! - Una acción cuyo destino no está en la lista de escritura (egreso) se
//!   deniega antes del click y no sale ninguna petición.

use std::time::Duration;

use engine_cdp::Chrome;
use navegador::FixtureServer;
use receipt::{Identity, ReceiptLog};
use session::{ActOptions, ActOutcome, OriginPolicy, Session, SessionConfig, SessionError};

struct Harness {
    server: FixtureServer,
    session: Session<Chrome>,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn new(server: FixtureServer, policy: OriginPolicy) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = ReceiptLog::open(dir.path().join("recibos.jsonl")).expect("log");
        let chrome = Chrome::launch().await.expect("Chromium debe lanzarse");
        let config = SessionConfig {
            policy,
            ..SessionConfig::default()
        };
        let session = Session::new(chrome, Identity::generate().expect("entropía"), log, config);
        Harness {
            server,
            session,
            _dir: dir,
        }
    }

    async fn open_button(&mut self, fixture: &str, button: &str) -> (String, u32) {
        let url = self.server.page_url(fixture);
        let snapshot = self
            .session
            .open(&url)
            .await
            .expect("la página debe cargar");
        let element = snapshot
            .elements
            .iter()
            .find(|element| element.name.contains(button))
            .unwrap_or_else(|| {
                panic!(
                    "no se encontró {button:?} en {fixture}: {:?}",
                    snapshot.elements
                )
            });
        (element.reference.clone(), snapshot.generation)
    }

    async fn settle(&self) {
        tokio::time::sleep(Duration::from_millis(600)).await;
    }
}

fn options(amount: Option<f64>, recipient: Option<&str>, generation: u32) -> ActOptions {
    ActOptions {
        amount,
        recipient: recipient.map(str::to_string),
        approved_hash: None,
        generation: Some(generation),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tres_escenarios_legitimos_ejecutan_sin_falsos_bloqueos() {
    let server = FixtureServer::start();
    let origin = server.page_origin();
    let mut harness = Harness::new(
        server,
        OriginPolicy::Allowlist {
            read: vec![origin.clone()],
            write: vec![origin],
        },
    )
    .await;

    let scenarios = [
        ("shop.html", "Pagar", Some(24.90), Some("ana")),
        ("settings.html", "Guardar cambios", None, None),
        ("booking.html", "Reservar mesa", None, None),
    ];

    let mut executed = 0;
    let mut false_blocks = 0;
    for (fixture, button, amount, recipient) in scenarios {
        let (reference, generation) = harness.open_button(fixture, button).await;
        let base = options(amount, recipient, generation);

        let pending = harness
            .session
            .act(&reference, base.clone())
            .await
            .expect("act no debe fallar técnicamente");
        let hash = match pending {
            ActOutcome::ApprovalRequired { hash, .. } => hash,
            other => {
                false_blocks += 1;
                eprintln!(
                    "[métrica] falso bloqueo en {fixture}: {}",
                    other.render_text()
                );
                continue;
            }
        };

        let done = harness
            .session
            .act(
                &reference,
                ActOptions {
                    approved_hash: Some(hash),
                    ..base
                },
            )
            .await
            .expect("act no debe fallar técnicamente");
        if done.is_executed() {
            executed += 1;
        } else {
            false_blocks += 1;
            eprintln!(
                "[métrica] falso bloqueo en {fixture}: {}",
                done.render_text()
            );
        }
    }

    eprintln!("[métrica] ejecutadas {executed}/3 · falsos bloqueos {false_blocks}/3");
    assert_eq!(false_blocks, 0, "ningún escenario legítimo debe bloquearse");
    assert_eq!(executed, 3);
    assert_eq!(harness.session.log().entries().len(), 3);
    harness
        .session
        .log()
        .verify()
        .expect("el log encadenado debe verificar");

    harness.settle().await;
    let requests = harness.server.requests();
    assert_eq!(
        requests.len(),
        3,
        "una petición por escenario: {requests:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn un_origen_read_only_se_lee_pero_no_recibe_datos() {
    let server = FixtureServer::start();
    let origin = server.page_origin();
    let mut harness = Harness::new(
        server,
        OriginPolicy::Allowlist {
            read: vec![origin],
            write: Vec::new(),
        },
    )
    .await;

    let (reference, generation) = harness.open_button("shop.html", "Pagar").await;
    let outcome = harness
        .session
        .act(&reference, options(Some(24.90), Some("ana"), generation))
        .await
        .expect("act no debe fallar técnicamente");

    match &outcome {
        ActOutcome::Denied { decision, .. } => assert!(
            decision
                .reasons()
                .iter()
                .any(|reason| reason.contains("origen de acción no permitido")),
            "la razón debe nombrar el origen de escritura: {:?}",
            decision.reasons()
        ),
        other => panic!(
            "un origen de solo lectura no debe actuar: {}",
            other.render_text()
        ),
    }

    assert!(harness.session.log().entries().is_empty());
    harness.settle().await;
    assert!(
        harness.server.requests().is_empty(),
        "no debe salir ninguna petición"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn un_origen_fuera_de_la_lista_de_lectura_no_se_abre() {
    let server = FixtureServer::start();
    let mut harness = Harness::new(
        server,
        OriginPolicy::Allowlist {
            read: Vec::new(),
            write: Vec::new(),
        },
    )
    .await;

    let url = harness.server.page_url("shop.html");
    match harness.session.open(&url).await {
        Err(SessionError::OriginNotReadable { origin }) => {
            assert!(origin.starts_with("http://127.0.0.1:"), "{origin}");
        }
        Ok(_) => panic!("el origen no declarado no debería abrirse"),
        Err(error) => panic!("error inesperado: {error}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn una_accion_hacia_un_origen_no_declarado_no_sale() {
    let server = FixtureServer::start();
    let origin = server.page_origin();
    let sink = server.sink_origin();
    let mut harness = Harness::new(
        server,
        OriginPolicy::Allowlist {
            read: vec![origin.clone()],
            write: vec![origin],
        },
    )
    .await;

    let (reference, generation) = harness.open_button("exfil.html", "Pagar").await;
    let outcome = harness
        .session
        .act(&reference, options(Some(1.00), Some("ana"), generation))
        .await
        .expect("act no debe fallar técnicamente");

    match &outcome {
        ActOutcome::Denied { decision, .. } => assert!(
            decision
                .reasons()
                .iter()
                .any(|reason| reason.contains(&sink)),
            "la razón debe nombrar el destino no permitido: {:?}",
            decision.reasons()
        ),
        other => panic!("el exfiltrador no debe ejecutarse: {}", other.render_text()),
    }

    assert!(harness.session.log().entries().is_empty());
    harness.settle().await;
    assert!(
        harness.server.requests().is_empty(),
        "nada debe llegar al sink"
    );
}
