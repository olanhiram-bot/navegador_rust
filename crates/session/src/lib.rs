//! SessionCore: la única puerta entre el agente y el navegador.
//!
//! Toda acción consecuente pasa por la misma cadena:
//!
//! ```text
//! ref @eN → descriptor ground-truth → política → tarjeta → dispatch → recibo
//! ```
//!
//! La sesión no sabe qué motor hay debajo: Chromium, Servo o un motor falso de
//! test implementan el mismo trait `Engine`. Lo que decide (política, tarjeta,
//! recibo) vive aquí y en los crates que ya existían.
//!
//! Dos invariantes se sostienen en el tipo:
//!   - una ref `@eN` solo es válida dentro de su `generation`, y antes de
//!     despachar se comprueba que el elemento sigue siendo el mismo;
//!   - si el despacho observó una petición (`Sent`), la sesión firma un recibo,
//!     lo verifica y lo anexa al log antes de devolver el resultado.

use std::path::{Path, PathBuf};
use std::time::Duration;

use card::CardView;
use descriptor::{ActionDescriptor, Decision, Intent, Provenance};
use engine::{DispatchOutcome, ElementRef, Engine, Snapshot, Transport};
use receipt::{Identity, Receipt, ReceiptLog, WireRequest};

/// Qué orígenes puede leer y a cuáles puede enviar datos la sesión.
///
/// Un origen en `read` y no en `write` es de solo lectura: se puede abrir,
/// leer y capturar, pero cualquier acción que envíe datos allí se deniega.
#[derive(Debug, Clone, PartialEq)]
pub enum OriginPolicy {
    /// Cualquier documento visitado se puede leer; las acciones solo pueden ir
    /// a orígenes ya visitados en la sesión (comportamiento del MVP).
    Visited,
    /// Solo los orígenes declarados: `read` para abrir y leer, `write` para
    /// actuar. Es también la lista de egreso: una acción cuyo destino no esté
    /// en `write` se deniega antes del click.
    Allowlist {
        read: Vec<String>,
        write: Vec<String>,
    },
}

/// Política de la sesión. El importe y el destinatario concretos se pasan en
/// cada `act`; aquí viven los límites que no cambian.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionConfig {
    pub provenance: Provenance,
    pub max_amount: f64,
    pub require_approval: bool,
    pub policy: OriginPolicy,
    /// Orígenes permitidos además de los documentos visitados en la sesión.
    pub extra_allowed_origins: Vec<String>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            provenance: Provenance::User,
            max_amount: 100.0,
            require_approval: true,
            policy: OriginPolicy::Visited,
            extra_allowed_origins: Vec::new(),
        }
    }
}

/// Lo que el agente declara en un `act`. `approved_hash` solo puede venir de
/// una aprobación humana (o del hash que devolvió el paso anterior).
///
/// `generation` es la generación del snapshot del que salió la ref. Si no se
/// indica, se asume la actual; el motor re-extrae y confirma de todos modos que
/// el elemento sigue siendo el mismo (tag, nombre y posición).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActOptions {
    pub amount: Option<f64>,
    pub recipient: Option<String>,
    pub approved_hash: Option<String>,
    pub generation: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("no hay snapshot: llama a snapshot u open antes de actuar")]
    NoSnapshot,
    #[error("referencia desconocida: {0}")]
    UnknownReference(String),
    #[error(
        "referencia obsoleta: {reference} pertenece a la generación {found}, la actual es {current}"
    )]
    StaleReference {
        reference: String,
        found: u32,
        current: u32,
    },
    #[error("el elemento de {0} cambió desde el snapshot; pide uno nuevo")]
    ElementChanged(String),
    #[error("origen no permitido para lectura: {origin}")]
    OriginNotReadable { origin: String },
    #[error("el motor falló: {0}")]
    Engine(#[from] anyhow::Error),
    #[error("el recibo no verifica: {0}")]
    Receipt(#[from] receipt::ReceiptError),
    #[error("no se pudo escribir el log: {0}")]
    Log(#[from] receipt::LogError),
}

/// Resultado de un `act`. Solo `Executed` lleva recibo, y `Executed` es el
/// único estado en el que la petición salió de verdad.
#[derive(Debug)]
pub enum ActOutcome {
    /// La política denegó la acción; no se hizo click.
    Denied {
        descriptor: ActionDescriptor,
        decision: Decision,
        card: CardView,
    },
    /// Hace falta una aprobación atada a este hash exacto.
    ApprovalRequired {
        descriptor: ActionDescriptor,
        hash: String,
        card: CardView,
    },
    /// Se ejecutó y la petición observada coincidía con lo aprobado.
    Executed {
        receipt: Receipt,
        card: CardView,
        seq: u64,
        outcome: DispatchOutcome,
    },
    /// Se intentó, pero el binding abortó: no salió ninguna petición.
    NotExecuted {
        descriptor: ActionDescriptor,
        outcome: DispatchOutcome,
    },
}

impl ActOutcome {
    pub fn receipt(&self) -> Option<&Receipt> {
        match self {
            ActOutcome::Executed { receipt, .. } => Some(receipt),
            _ => None,
        }
    }

    pub fn is_executed(&self) -> bool {
        matches!(self, ActOutcome::Executed { .. })
    }

    /// Texto para CLI y MCP: siempre incluye la tarjeta de lo que se iba a
    /// hacer, también cuando se denegó.
    pub fn render_text(&self) -> String {
        match self {
            ActOutcome::Denied { card, decision, .. } => {
                let mut out = card::render_text(card);
                out.push_str(&format!("  denegada por política: {decision:?}\n"));
                out
            }
            ActOutcome::ApprovalRequired { card, hash, .. } => {
                let mut out = card::render_text(card);
                out.push_str(&format!("  requiere aprobación del hash {hash}\n"));
                out
            }
            ActOutcome::Executed {
                card, receipt, seq, ..
            } => {
                let mut out = card::render_text(card);
                out.push_str(&format!(
                    "  ejecutada · recibo {seq} · hash {}\n",
                    receipt.descriptor.hash_hex
                ));
                out
            }
            ActOutcome::NotExecuted {
                descriptor,
                outcome,
            } => {
                format!(
                    "acción no ejecutada: {outcome:?}\n  descriptor: {}\n",
                    descriptor.hash_hex
                )
            }
        }
    }
}

/// Informe para pasarle el control a una persona sin perder el contexto.
#[derive(Debug, Clone)]
pub struct Handoff {
    pub url: String,
    pub title: String,
    pub generation: u32,
    pub elements: Vec<ElementRef>,
    pub receipts: usize,
    pub last_receipt_hash: Option<String>,
    pub log_path: PathBuf,
}

impl Handoff {
    pub fn render_text(&self) -> String {
        let mut out = format!(
            "handoff · generación {} · {}\n  título: {}\n  elementos: {}\n  recibos: {}\n",
            self.generation,
            self.url,
            if self.title.is_empty() {
                "(sin título)"
            } else {
                &self.title
            },
            self.elements.len(),
            self.receipts
        );
        out.push_str(&format!("  log: {}\n", self.log_path.display()));
        if let Some(hash) = &self.last_receipt_hash {
            out.push_str(&format!("  último recibo: {hash}\n"));
        }
        out.push_str("  el control está en la persona; el agente debe esperar\n");
        out
    }
}

/// Sesión compartida por una persona y un agente.
pub struct Session<E: Engine> {
    engine: E,
    identity: Identity,
    log: ReceiptLog,
    config: SessionConfig,
    generation: u32,
    snapshot: Option<Snapshot>,
    allowed_origins: Vec<String>,
    url: String,
    title: String,
}

impl<E: Engine> Session<E> {
    pub fn new(engine: E, identity: Identity, log: ReceiptLog, config: SessionConfig) -> Self {
        let allowed_origins = config.extra_allowed_origins.clone();
        Session {
            engine,
            identity,
            log,
            config,
            generation: 0,
            snapshot: None,
            allowed_origins,
            url: String::new(),
            title: String::new(),
        }
    }

    pub fn public_key(&self) -> String {
        self.identity.public_hex()
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// URL del snapshot vivo, si lo hay.
    pub fn url(&self) -> Option<&str> {
        self.snapshot.as_ref().map(|snapshot| snapshot.url.as_str())
    }

    pub fn log(&self) -> &ReceiptLog {
        &self.log
    }

    pub fn log_path(&self) -> &Path {
        self.log.path()
    }

    /// Navega, abre una generación nueva y devuelve el snapshot ya listo.
    ///
    /// Con `OriginPolicy::Allowlist` la navegación se deniega si el origen no
    /// está en la lista de lectura.
    pub async fn open(&mut self, url: &str) -> Result<Snapshot, SessionError> {
        if let OriginPolicy::Allowlist { read, .. } = &self.config.policy {
            match origin_of(url) {
                Some(origin) if read.contains(&origin) => {}
                Some(origin) => return Err(SessionError::OriginNotReadable { origin }),
                None => {
                    return Err(SessionError::OriginNotReadable {
                        origin: url.to_string(),
                    });
                }
            }
        }
        self.engine.navigate(url).await?;
        self.generation += 1;
        self.url = url.to_string();
        self.note_origin(url);
        self.snapshot_now().await
    }

    /// Rehace el snapshot en la generación actual: las refs `@eN` se mantienen.
    pub async fn snapshot(&mut self) -> Result<Snapshot, SessionError> {
        if self.generation == 0 {
            return Err(SessionError::NoSnapshot);
        }
        self.snapshot_now().await
    }

    /// Ref `@eN` del snapshot vivo para un selector CSS, si sigue presente.
    pub fn reference_for_selector(&self, selector: &str) -> Option<&ElementRef> {
        self.snapshot
            .as_ref()?
            .elements
            .iter()
            .find(|element| element.selector == selector)
    }

    /// Invalida todas las refs (la página puede haber cambiado por su cuenta).
    pub fn refresh(&mut self) {
        self.generation += 1;
        self.snapshot = None;
    }

    pub async fn act(
        &mut self,
        reference: &str,
        options: ActOptions,
    ) -> Result<ActOutcome, SessionError> {
        let reference = normalize_reference(reference);
        if let Some(generation) = options.generation
            && generation != self.generation
        {
            return Err(SessionError::StaleReference {
                reference,
                found: generation,
                current: self.generation,
            });
        }
        let snapshot = self.snapshot.as_ref().ok_or(SessionError::NoSnapshot)?;
        let element = snapshot
            .find(&reference)
            .ok_or_else(|| SessionError::UnknownReference(reference.clone()))?;

        if element.generation != self.generation {
            return Err(SessionError::StaleReference {
                reference: reference.clone(),
                found: element.generation,
                current: self.generation,
            });
        }

        let selector = element.selector.clone();
        let observed = self
            .engine
            .extract(&selector, self.generation, self.config.provenance)
            .await?;

        // La ref vuelve a encontrar el elemento, pero hay que confirmar que es
        // el mismo: si la página reescribió el DOM, el selector puede apuntar a
        // otro nodo con otro significado.
        if !element.same_element(
            &observed.target.tag,
            &observed.target.name,
            observed.target.x,
            observed.target.y,
        ) {
            self.refresh();
            return Err(SessionError::ElementChanged(reference));
        }

        let allowed_action_origins = match &self.config.policy {
            OriginPolicy::Visited => self.allowed_origins.clone(),
            OriginPolicy::Allowlist { write, .. } => write.clone(),
        };
        let intent = Intent {
            provenance: self.config.provenance,
            verb: None,
            amount: options.amount,
            recipient: options.recipient.clone(),
            allowed_action_origins,
            max_amount: self.config.max_amount,
            require_approval: self.config.require_approval,
        };
        let decision = intent.evaluate(&observed, options.approved_hash.as_deref());
        let view = card::build(&observed, &decision);

        match &decision {
            Decision::Deny { .. } => {
                return Ok(ActOutcome::Denied {
                    descriptor: observed,
                    decision,
                    card: view,
                });
            }
            Decision::RequireApproval { hash_hex, .. } => {
                return Ok(ActOutcome::ApprovalRequired {
                    descriptor: observed,
                    hash: hash_hex.clone(),
                    card: view,
                });
            }
            Decision::Allow => {}
        }

        let outcome = self.engine.dispatch(&observed).await?;

        // Tras cualquier intento de despacho las refs dejan de ser fiables: la
        // página puede haber navegado o reescrito el formulario.
        self.refresh();

        match &outcome {
            DispatchOutcome::Sent { url, method, body } => {
                let signed = self.identity.sign(
                    &observed,
                    WireRequest {
                        url: url.clone(),
                        method: method.clone(),
                        body: body.clone(),
                    },
                    "sent",
                );
                // Si el recibo no verifica, no se despacha evidencia rota: el
                // error sube y la acción queda sin recibo (y sin log).
                receipt::verify(&signed)?;
                let entry = self.log.append(signed)?;
                Ok(ActOutcome::Executed {
                    receipt: entry.receipt.clone(),
                    card: view,
                    seq: entry.seq,
                    outcome,
                })
            }
            _ => Ok(ActOutcome::NotExecuted {
                descriptor: observed,
                outcome,
            }),
        }
    }

    pub async fn read(&mut self) -> Result<String, SessionError> {
        Ok(self.engine.read_text().await?)
    }

    pub async fn screenshot(&mut self) -> Result<Vec<u8>, SessionError> {
        Ok(self.engine.screenshot().await?)
    }

    pub async fn handoff(&mut self) -> Handoff {
        let snapshot = self.snapshot.as_ref();
        Handoff {
            url: snapshot
                .map(|s| s.url.clone())
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| self.url.clone()),
            title: snapshot
                .map(|s| s.title.clone())
                .unwrap_or_else(|| self.title.clone()),
            generation: self.generation,
            elements: snapshot.map(|s| s.elements.clone()).unwrap_or_default(),
            receipts: self.log.entries().len(),
            last_receipt_hash: self
                .log
                .last_receipt()
                .map(|receipt| receipt.descriptor.hash_hex.clone()),
            log_path: self.log.path().to_path_buf(),
        }
    }

    async fn snapshot_now(&mut self) -> Result<Snapshot, SessionError> {
        let snapshot = self.engine.snapshot(self.generation).await?;
        self.url = snapshot.url.clone();
        self.title = snapshot.title.clone();
        self.snapshot = Some(snapshot.clone());
        Ok(snapshot)
    }

    fn note_origin(&mut self, url: &str) {
        let Some(origin) = origin_of(url) else {
            return;
        };
        if !self.allowed_origins.contains(&origin) {
            self.allowed_origins.push(origin);
        }
    }
}

impl<E: Engine + Transport> Session<E> {
    pub async fn start_screencast(&mut self) -> Result<(), SessionError> {
        self.engine.start_screencast().await?;
        Ok(())
    }

    pub async fn stop_screencast(&mut self) -> Result<(), SessionError> {
        self.engine.stop_screencast().await?;
        Ok(())
    }

    pub async fn next_event(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<serde_json::Value>, SessionError> {
        Ok(self.engine.next_event(timeout).await?)
    }

    pub async fn ack_screencast_frame(&mut self, session_id: i64) -> Result<(), SessionError> {
        self.engine.ack_screencast_frame(session_id).await?;
        Ok(())
    }

    pub async fn set_viewport(
        &mut self,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Result<(), SessionError> {
        self.engine.set_viewport(width, height, scale).await?;
        Ok(())
    }

    pub async fn history(&mut self, delta: i64) -> Result<bool, SessionError> {
        Ok(self.engine.history(delta).await?)
    }

    pub async fn link_href(&mut self, selector: &str) -> Result<Option<String>, SessionError> {
        Ok(self.engine.link_href(selector).await?)
    }

    /// Selector del control que un Enter enviaría desde el campo enfocado.
    pub async fn form_submitter(&self) -> Result<Option<String>, SessionError> {
        Ok(self.engine.form_submitter().await?)
    }

    /// Fija permisos del origen en esta pestaña.
    pub async fn set_permissions(
        &self,
        origin: &str,
        granted: &[String],
        denied: &[String],
    ) -> Result<(), SessionError> {
        Ok(self.engine.set_permissions(origin, granted, denied).await?)
    }

    /// Busca texto en la página; `index` elige la coincidencia resaltada.
    pub async fn find(&self, text: &str, index: i64) -> Result<engine::FindResult, SessionError> {
        Ok(self.engine.find(text, index).await?)
    }

    pub async fn youtube_search_query(
        &self,
        selector: &str,
    ) -> Result<Option<String>, SessionError> {
        Ok(self.engine.youtube_search_query(selector).await?)
    }

    pub async fn insert_text(&mut self, text: &str) -> Result<(), SessionError> {
        self.engine.insert_text(text).await?;
        Ok(())
    }

    pub async fn key(
        &mut self,
        key: &str,
        code: &str,
        virtual_key: i64,
        modifiers: u32,
    ) -> Result<(), SessionError> {
        self.engine.key(key, code, virtual_key, modifiers).await?;
        Ok(())
    }

    pub async fn select_text(&self, start: [f64; 2], end: [f64; 2]) -> Result<(), SessionError> {
        Ok(self.engine.select_text(start, end).await?)
    }

    pub async fn selected_text(&self) -> Result<String, SessionError> {
        Ok(self.engine.selected_text().await?)
    }

    /// Click de foco para campos de texto; nunca para botones.
    pub async fn click_at(&mut self, x: f64, y: f64) -> Result<(), SessionError> {
        self.engine.click_at(x, y).await?;
        Ok(())
    }

    pub async fn scroll(&mut self, x: f64, y: f64, delta_y: f64) -> Result<(), SessionError> {
        self.engine.scroll(x, y, delta_y).await?;
        Ok(())
    }

    /// Destino de un enlace visible que acaba de aparecer en el DOM.
    pub async fn link_at(&mut self, x: f64, y: f64) -> Result<Option<String>, SessionError> {
        let Some(selector) = self.engine.hit_test(x, y).await? else {
            return Ok(None);
        };
        Ok(self.engine.link_href(&selector).await?)
    }

    /// Elemento accionable bajo un punto, solo si sigue en el snapshot vivo.
    pub async fn element_at(&mut self, x: f64, y: f64) -> Result<Option<ElementRef>, SessionError> {
        let Some(selector) = self.engine.hit_test(x, y).await? else {
            return Ok(None);
        };
        let snapshot = self.snapshot.as_ref().ok_or(SessionError::NoSnapshot)?;
        Ok(snapshot
            .elements
            .iter()
            .find(|element| element.selector == selector)
            .cloned())
    }
}

fn origin_of(url: &str) -> Option<String> {
    let origin = url::Url::parse(url).ok()?.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

fn normalize_reference(reference: &str) -> String {
    let trimmed = reference.trim();
    if trimmed.starts_with('@') {
        trimmed.to_lowercase()
    } else {
        format!("@{}", trimmed.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use descriptor::{RawAction, Target, Verb};
    use engine::Snapshot;

    struct FakeEngine {
        url: String,
        snafu: bool,
    }

    impl FakeEngine {
        fn new() -> Self {
            FakeEngine {
                url: String::new(),
                snafu: false,
            }
        }
    }

    impl Engine for FakeEngine {
        async fn navigate(&mut self, url: &str) -> anyhow::Result<()> {
            self.url = url.to_string();
            Ok(())
        }

        async fn snapshot(&mut self, generation: u32) -> anyhow::Result<Snapshot> {
            Ok(Snapshot {
                generation,
                url: self.url.clone(),
                title: "Transferencia".into(),
                elements: vec![ElementRef {
                    reference: "@e1".into(),
                    generation,
                    selector: "#go".into(),
                    role: "button".into(),
                    name: "Confirmar transferencia de $1.00 a Ana".into(),
                    tag: "BUTTON".into(),
                    x: 10.0,
                    y: 10.0,
                    disabled: false,
                }],
                text: "Transferencia a Ana por $1.00".into(),
            })
        }

        async fn extract(
            &self,
            _selector: &str,
            generation: u32,
            provenance: Provenance,
        ) -> anyhow::Result<ActionDescriptor> {
            let mut fields = BTreeMap::new();
            fields.insert("amount".into(), "1.00".into());
            fields.insert("to".into(), "ana".into());
            Ok(ActionDescriptor::from_raw(
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
                    visible_text: "Transferencia a Ana por $1.00".into(),
                    target: Target {
                        selector: "#go".into(),
                        tag: "BUTTON".into(),
                        name: "Confirmar transferencia de $1.00 a Ana".into(),
                        x: 10.0,
                        y: 10.0,
                    },
                },
                provenance,
                generation,
            ))
        }

        async fn dispatch(
            &mut self,
            approved: &ActionDescriptor,
        ) -> anyhow::Result<DispatchOutcome> {
            if self.snafu {
                return Ok(DispatchOutcome::Blocked {
                    url: "http://127.0.0.1:6000/collect".into(),
                    method: "POST".into(),
                    body: None,
                    reason: "URL distinta".into(),
                });
            }
            Ok(DispatchOutcome::Sent {
                url: approved.action_url.clone().unwrap(),
                method: "POST".into(),
                body: Some(approved.expected_body()),
            })
        }

        async fn screenshot(&mut self) -> anyhow::Result<Vec<u8>> {
            Ok(vec![0x89, b'P', b'N', b'G'])
        }

        async fn read_text(&mut self) -> anyhow::Result<String> {
            Ok("Transferencia a Ana por $1.00".into())
        }
    }

    fn session() -> (Session<FakeEngine>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let log = ReceiptLog::open(dir.path().join("recibos.jsonl")).unwrap();
        let session = Session::new(
            FakeEngine::new(),
            Identity::from_seed_hex(&"99".repeat(32)).unwrap(),
            log,
            SessionConfig::default(),
        );
        (session, dir)
    }

    #[tokio::test]
    async fn un_act_pasa_por_politica_tarjeta_y_recibo() {
        let (mut session, _dir) = session();
        let snapshot = session
            .open("http://127.0.0.1:5000/legit.html")
            .await
            .unwrap();
        assert_eq!(snapshot.generation, 1);
        assert_eq!(snapshot.elements[0].reference, "@e1");

        let pending = session
            .act(
                "@e1",
                ActOptions {
                    amount: Some(1.0),
                    recipient: Some("ana".into()),
                    approved_hash: None,
                    generation: None,
                },
            )
            .await
            .unwrap();
        let hash = match pending {
            ActOutcome::ApprovalRequired { hash, card, .. } => {
                assert!(card.warnings.is_empty(), "{:?}", card.warnings);
                hash
            }
            other => panic!("se esperaba aprobación: {}", other.render_text()),
        };

        let done = session
            .act(
                "@e1",
                ActOptions {
                    amount: Some(1.0),
                    recipient: Some("ana".into()),
                    approved_hash: Some(hash),
                    generation: None,
                },
            )
            .await
            .unwrap();
        let receipt = done.receipt().expect("la acción ejecutada deja recibo");
        receipt::verify(receipt).unwrap();
        assert_eq!(session.log().entries().len(), 1);
        session.log().verify().unwrap();

        // El recibo prueba que la petición fue la aprobada.
        assert_eq!(receipt.wire.method, "POST");
        assert!(receipt.wire.body.as_deref().unwrap().contains("to=ana"));
    }

    #[tokio::test]
    async fn una_ref_de_otra_generacion_se_rechaza() {
        let (mut session, _dir) = session();
        let first = session
            .open("http://127.0.0.1:5000/legit.html")
            .await
            .unwrap();
        let old = first.elements[0].reference.clone();

        session
            .open("http://127.0.0.1:5000/otra.html")
            .await
            .unwrap();

        match session
            .act(
                &old,
                ActOptions {
                    generation: Some(1),
                    ..ActOptions::default()
                },
            )
            .await
        {
            Err(SessionError::StaleReference { found, current, .. }) => {
                assert_eq!(found, 1);
                assert_eq!(current, 2);
            }
            other => panic!("se esperaba ref obsoleta: {other:?}", other = other.err()),
        }
    }

    #[tokio::test]
    async fn si_el_binding_aborta_no_hay_recibo() {
        let dir = tempfile::tempdir().unwrap();
        let log = ReceiptLog::open(dir.path().join("recibos.jsonl")).unwrap();
        let mut engine = FakeEngine::new();
        engine.snafu = true;
        let mut session = Session::new(
            engine,
            Identity::from_seed_hex(&"aa".repeat(32)).unwrap(),
            log,
            SessionConfig::default(),
        );

        session
            .open("http://127.0.0.1:5000/legit.html")
            .await
            .unwrap();
        let pending = session
            .act(
                "@e1",
                ActOptions {
                    amount: Some(1.0),
                    recipient: Some("ana".into()),
                    approved_hash: None,
                    generation: None,
                },
            )
            .await
            .unwrap();
        let hash = match pending {
            ActOutcome::ApprovalRequired { hash, .. } => hash,
            other => panic!("se esperaba aprobación: {}", other.render_text()),
        };

        let blocked = session
            .act(
                "@e1",
                ActOptions {
                    amount: Some(1.0),
                    recipient: Some("ana".into()),
                    approved_hash: Some(hash),
                    generation: None,
                },
            )
            .await
            .unwrap();
        assert!(!blocked.is_executed());
        assert!(blocked.receipt().is_none());
        assert!(session.log().entries().is_empty());
    }

    #[tokio::test]
    async fn una_ref_desconocida_no_toca_el_motor() {
        let (mut session, _dir) = session();
        session
            .open("http://127.0.0.1:5000/legit.html")
            .await
            .unwrap();
        assert!(matches!(
            session.act("@e99", ActOptions::default()).await,
            Err(SessionError::UnknownReference(_))
        ));
    }
}
