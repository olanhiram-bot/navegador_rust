//! Puente entre la shell y Chromium.
//!
//! Cada pestaña tiene su propio `Session<Chrome>`: política, generación, refs,
//! tarjeta y recibo. El hilo del motor es el único dueño de los navegadores; la
//! UI manda comandos y recibe frames ya decodificados, snapshots, actos y el
//! informe de handoff.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine as _;
use engine_cdp::{Chrome, ElementRef};
use receipt::{Identity, ReceiptLog};
use serde_json::Value;
use session::{ActOptions, ActOutcome, Handoff, Session, SessionConfig};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub type TabId = u64;

/// Un frame de la página ya decodificado.
#[derive(Debug)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    /// Tamaño CSS de la página, para mapear el puntero.
    pub css_width: f64,
    pub css_height: f64,
}

#[derive(Debug, Clone)]
pub struct TabInfo {
    pub id: TabId,
    pub url: String,
    pub title: String,
    pub generation: u32,
    pub elements: usize,
}

#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub url: String,
    pub title: String,
    pub generation: u32,
    pub elements: usize,
}

#[derive(Debug)]
pub enum Event {
    Frame {
        tab: TabId,
        frame: Frame,
    },
    TabOpened(TabInfo),
    TabClosed(TabId),
    Selected(Option<TabId>),
    Snapshot {
        tab: TabId,
        info: SnapshotInfo,
        elements: Vec<ElementRef>,
    },
    Act {
        tab: TabId,
        outcome: ActOutcome,
    },
    Handoff {
        tab: TabId,
        report: Handoff,
    },
    Receipts {
        tab: TabId,
        count: usize,
        last: Option<String>,
        log: PathBuf,
    },
    Notice(String),
    Error(String),
}

pub enum Command {
    NewTab { url: Option<String> },
    CloseTab(TabId),
    SelectTab(TabId),
    Navigate(String),
    Wheel { x: f64, y: f64, delta_y: f64 },
    Click { x: f64, y: f64 },
    Approve { hash: String },
    Deny,
    Handoff,
    Shutdown,
}

/// Mando de la shell hacia el motor.
pub struct EngineHandle {
    pub commands: UnboundedSender<Command>,
    pub events: Receiver<Event>,
    done: Receiver<()>,
}

impl EngineHandle {
    pub fn new_tab(&self, url: Option<String>) {
        let _ = self.commands.send(Command::NewTab { url });
    }

    pub fn close_tab(&self, tab: TabId) {
        let _ = self.commands.send(Command::CloseTab(tab));
    }

    pub fn select_tab(&self, tab: TabId) {
        let _ = self.commands.send(Command::SelectTab(tab));
    }

    pub fn navigate(&self, url: impl Into<String>) {
        let _ = self.commands.send(Command::Navigate(url.into()));
    }

    pub fn wheel(&self, x: f64, y: f64, delta_y: f64) {
        let _ = self.commands.send(Command::Wheel { x, y, delta_y });
    }

    pub fn click(&self, x: f64, y: f64) {
        let _ = self.commands.send(Command::Click { x, y });
    }

    pub fn approve(&self, hash: impl Into<String>) {
        let _ = self.commands.send(Command::Approve { hash: hash.into() });
    }

    pub fn deny(&self) {
        let _ = self.commands.send(Command::Deny);
    }

    pub fn handoff(&self) {
        let _ = self.commands.send(Command::Handoff);
    }

    /// Espera a que el motor cierre Chromium. Acotado para no colgar la salida.
    pub fn shutdown(&self) {
        let _ = self.commands.send(Command::Shutdown);
        let _ = self.done.recv_timeout(Duration::from_secs(5));
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        let _ = self.done.recv_timeout(Duration::from_secs(3));
    }
}

pub fn spawn(initial_url: String, log_path: PathBuf) -> EngineHandle {
    let (commands, command_rx) = unbounded_channel();
    let (events_tx, events) = channel();
    let (done_tx, done) = channel();

    std::thread::spawn(move || {
        if let Err(error) = run(initial_url, log_path, command_rx, events_tx.clone()) {
            let _ = events_tx.send(Event::Error(format!("{error:#}")));
        }
        let _ = done_tx.send(());
    });

    EngineHandle {
        commands,
        events,
        done,
    }
}

struct Pending {
    reference: String,
    options: ActOptions,
}

struct Tab {
    session: Session<Chrome>,
    log_path: PathBuf,
    pending: Option<Pending>,
}

struct State {
    tabs: BTreeMap<TabId, Tab>,
    active: Option<TabId>,
    next_id: TabId,
    log_base: PathBuf,
}

fn run(
    initial_url: String,
    log_path: PathBuf,
    mut commands: UnboundedReceiver<Command>,
    events: Sender<Event>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("crear el runtime del motor")?;

    runtime.block_on(async move {
        let mut state = State {
            tabs: BTreeMap::new(),
            active: None,
            next_id: 0,
            log_base: log_path,
        };

        open_tab(&mut state, &initial_url, &events).await?;

        loop {
            loop {
                match commands.try_recv() {
                    Ok(command) => match handle_command(&mut state, command, &events).await {
                        Ok(true) => return Ok(()),
                        Ok(false) => {}
                        // Un comando fallido no tumba el motor ni las demás pestañas.
                        Err(error) => {
                            let _ = events.send(Event::Error(format!("{error:#}")));
                        }
                    },
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }

            let ids: Vec<TabId> = std::iter::once(state.active)
                .flatten()
                .chain(
                    state
                        .tabs
                        .keys()
                        .copied()
                        .filter(|id| Some(*id) != state.active),
                )
                .collect();

            for id in ids {
                let event = {
                    let Some(tab) = state.tabs.get_mut(&id) else {
                        continue;
                    };
                    tab.session.next_event(Duration::from_millis(10)).await?
                };
                if let Some(event) = event
                    && event.get("method").and_then(Value::as_str) == Some("Page.screencastFrame")
                {
                    handle_frame(&mut state, id, &event, &events).await;
                }
            }
        }
    })
}

/// `true` significa cerrar la shell.
async fn handle_command(
    state: &mut State,
    command: Command,
    events: &Sender<Event>,
) -> Result<bool> {
    match command {
        Command::Shutdown => return Ok(true),
        Command::NewTab { url } => {
            open_tab(state, url.as_deref().unwrap_or("about:blank"), events).await?;
        }
        Command::CloseTab(id) => close_tab(state, id, events).await?,
        Command::SelectTab(id) => select_tab(state, id, events).await?,
        Command::Navigate(url) => navigate(state, &url, events).await?,
        Command::Wheel { x, y, delta_y } => {
            if let Some(tab) = active_tab(state) {
                tab.session.scroll(x, y, delta_y).await?;
            }
        }
        Command::Click { x, y } => click(state, x, y, events).await?,
        Command::Approve { hash } => approve(state, &hash, events).await?,
        Command::Deny => deny(state, events),
        Command::Handoff => handoff(state, events).await,
    }
    Ok(false)
}

fn active_tab(state: &mut State) -> Option<&mut Tab> {
    state.active.and_then(|id| state.tabs.get_mut(&id))
}

async fn open_tab(state: &mut State, url: &str, events: &Sender<Event>) -> Result<TabId> {
    let id = state.next_id;
    state.next_id += 1;
    let log_path = tab_log_path(&state.log_base, id);
    let log =
        ReceiptLog::open(&log_path).with_context(|| format!("abrir el log de la pestaña {id}"))?;
    let chrome = Chrome::launch().await.context("lanzar Chromium")?;
    let mut session = Session::new(
        chrome,
        Identity::generate().context("generar la identidad local")?,
        log,
        SessionConfig::default(),
    );

    let snapshot = session.open(url).await?;
    session.start_screencast().await?;

    let info = TabInfo {
        id,
        url: snapshot.url.clone(),
        title: snapshot.title.clone(),
        generation: snapshot.generation,
        elements: snapshot.elements.len(),
    };
    let elements = snapshot.elements.clone();
    state.active = Some(id);
    state.tabs.insert(
        id,
        Tab {
            session,
            log_path,
            pending: None,
        },
    );

    let _ = events.send(Event::TabOpened(info));
    let _ = events.send(Event::Selected(Some(id)));
    let _ = events.send(Event::Snapshot {
        tab: id,
        info: snapshot_info(&snapshot),
        elements,
    });
    send_receipts(state, id, events);
    Ok(id)
}

async fn close_tab(state: &mut State, id: TabId, events: &Sender<Event>) -> Result<()> {
    if state.tabs.remove(&id).is_none() {
        return Ok(());
    }
    let _ = events.send(Event::TabClosed(id));
    if state.active != Some(id) {
        return Ok(());
    }

    state.active = None;
    let next = state.tabs.keys().next().copied();
    match next {
        Some(next) => select_tab(state, next, events).await?,
        None => {
            let _ = events.send(Event::Selected(None));
        }
    }
    Ok(())
}

async fn select_tab(state: &mut State, id: TabId, events: &Sender<Event>) -> Result<()> {
    if !state.tabs.contains_key(&id) {
        return Ok(());
    }
    if let Some(old) = state.active
        && old != id
        && let Some(tab) = state.tabs.get_mut(&old)
    {
        tab.session.stop_screencast().await.ok();
    }

    let (info, elements) = {
        let tab = state
            .tabs
            .get_mut(&id)
            .expect("la pestaña existe: se comprobó arriba");
        // Una aprobación solo vale en la pestaña donde se mostró la tarjeta.
        tab.pending = None;
        tab.session.start_screencast().await.ok();
        let snapshot = tab.session.snapshot().await?;
        (snapshot_info(&snapshot), snapshot.elements)
    };

    state.active = Some(id);
    let _ = events.send(Event::Selected(Some(id)));
    let _ = events.send(Event::Snapshot {
        tab: id,
        info,
        elements,
    });
    send_receipts(state, id, events);
    Ok(())
}

async fn navigate(state: &mut State, url: &str, events: &Sender<Event>) -> Result<()> {
    let Some(id) = state.active else {
        return Ok(());
    };
    let info = {
        let Some(tab) = state.tabs.get_mut(&id) else {
            return Ok(());
        };
        let snapshot = tab.session.open(url).await?;
        tab.pending = None;
        tab.session.start_screencast().await.ok();
        (snapshot_info(&snapshot), snapshot.elements)
    };
    let _ = events.send(Event::Snapshot {
        tab: id,
        info: info.0,
        elements: info.1,
    });
    send_receipts(state, id, events);
    Ok(())
}

async fn click(state: &mut State, x: f64, y: f64, events: &Sender<Event>) -> Result<()> {
    let Some(id) = state.active else {
        return Ok(());
    };
    let element = {
        let Some(tab) = state.tabs.get_mut(&id) else {
            return Ok(());
        };
        tab.session.element_at(x, y).await?
    };

    let Some(element) = element else {
        let _ = events.send(Event::Notice(
            "no hay un elemento accionable en ese punto".into(),
        ));
        return Ok(());
    };

    if element.role != "button" {
        let _ = events.send(Event::Notice(format!(
            "{} «{}» no es un botón: la shell solo actúa sobre botones por ahora",
            element.role, element.name
        )));
        return Ok(());
    }

    let options = ActOptions {
        generation: Some(state.tabs[&id].session.generation()),
        ..ActOptions::default()
    };
    let outcome = {
        let tab = state.tabs.get_mut(&id).expect("pestaña activa");
        tab.session.act(&element.reference, options.clone()).await?
    };

    settle_act(state, id, &element.reference, options, outcome, events).await
}

async fn approve(state: &mut State, hash: &str, events: &Sender<Event>) -> Result<()> {
    let Some(id) = state.active else {
        return Ok(());
    };
    let Some(pending) = state.tabs.get_mut(&id).and_then(|tab| tab.pending.take()) else {
        let _ = events.send(Event::Notice("no hay ninguna aprobación pendiente".into()));
        return Ok(());
    };

    if pending.options.approved_hash.as_deref() != Some(hash) {
        let _ = events.send(Event::Notice(
            "la aprobación no coincide con el hash pendiente".into(),
        ));
        if let Some(tab) = state.tabs.get_mut(&id) {
            tab.pending = Some(pending);
        }
        return Ok(());
    }

    let outcome = {
        let tab = state.tabs.get_mut(&id).expect("pestaña activa");
        tab.session
            .act(&pending.reference, pending.options.clone())
            .await?
    };
    settle_act(
        state,
        id,
        &pending.reference,
        pending.options,
        outcome,
        events,
    )
    .await
}

fn deny(state: &mut State, events: &Sender<Event>) {
    if let Some(tab) = active_tab(state) {
        tab.pending = None;
    }
    let _ = events.send(Event::Notice("acción denegada por la persona".into()));
}

async fn handoff(state: &mut State, events: &Sender<Event>) {
    let Some(id) = state.active else {
        return;
    };
    let report = {
        let Some(tab) = state.tabs.get_mut(&id) else {
            return;
        };
        tab.session.handoff().await
    };
    let _ = events.send(Event::Handoff { tab: id, report });
}

async fn settle_act(
    state: &mut State,
    id: TabId,
    reference: &str,
    options: ActOptions,
    outcome: ActOutcome,
    events: &Sender<Event>,
) -> Result<()> {
    match &outcome {
        ActOutcome::ApprovalRequired { hash, .. } => {
            if let Some(tab) = state.tabs.get_mut(&id) {
                tab.pending = Some(Pending {
                    reference: reference.to_string(),
                    options: ActOptions {
                        approved_hash: Some(hash.clone()),
                        ..options
                    },
                });
            }
        }
        ActOutcome::Executed { .. } | ActOutcome::NotExecuted { .. } => {
            if let Some(tab) = state.tabs.get_mut(&id) {
                tab.pending = None;
            }
            refresh_snapshot(state, id, events).await?;
            send_receipts(state, id, events);
        }
        _ => {}
    }

    let _ = events.send(Event::Act { tab: id, outcome });
    Ok(())
}

async fn refresh_snapshot(state: &mut State, id: TabId, events: &Sender<Event>) -> Result<()> {
    let Some(tab) = state.tabs.get_mut(&id) else {
        return Ok(());
    };
    // `act` invalidó el snapshot y avanzó la generación.
    let snapshot = tab.session.snapshot().await?;
    let _ = events.send(Event::Snapshot {
        tab: id,
        info: snapshot_info(&snapshot),
        elements: snapshot.elements,
    });
    Ok(())
}

fn send_receipts(state: &State, id: TabId, events: &Sender<Event>) {
    let Some(tab) = state.tabs.get(&id) else {
        return;
    };
    let _ = events.send(Event::Receipts {
        tab: id,
        count: tab.session.log().entries().len(),
        last: tab
            .session
            .log()
            .last_receipt()
            .map(|receipt| receipt.descriptor.hash_hex.clone()),
        log: tab.log_path.clone(),
    });
}

async fn handle_frame(state: &mut State, tab: TabId, event: &Value, events: &Sender<Event>) {
    let params = &event["params"];
    let Some(session_id) = params.get("sessionId").and_then(Value::as_i64) else {
        return;
    };
    let Some(data) = params.get("data").and_then(Value::as_str) else {
        return;
    };
    let metadata = params.get("metadata").cloned().unwrap_or(Value::Null);
    let css_width = metadata
        .get("deviceWidth")
        .and_then(Value::as_f64)
        .unwrap_or(1024.0);
    let css_height = metadata
        .get("deviceHeight")
        .and_then(Value::as_f64)
        .unwrap_or(768.0);

    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data)
        && let Ok(image) = image::load_from_memory(&bytes)
    {
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        let _ = events.send(Event::Frame {
            tab,
            frame: Frame {
                width: width as usize,
                height: height as usize,
                rgba: rgba.into_raw(),
                css_width,
                css_height,
            },
        });
    }

    if let Some(tab) = state.tabs.get_mut(&tab) {
        let _ = tab.session.ack_screencast_frame(session_id).await;
    }
}

fn snapshot_info(snapshot: &engine_cdp::Snapshot) -> SnapshotInfo {
    SnapshotInfo {
        url: snapshot.url.clone(),
        title: snapshot.title.clone(),
        generation: snapshot.generation,
        elements: snapshot.elements.len(),
    }
}

fn tab_log_path(base: &Path, id: TabId) -> PathBuf {
    let stem = base
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("navegador-recibos");
    let extension = base
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("jsonl");
    base.with_file_name(format!("{stem}-tab{id}.{extension}"))
}
