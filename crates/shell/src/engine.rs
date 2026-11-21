//! Puente entre la shell y Chromium.
//!
//! Cada pestaña tiene su propio `Session<Chrome>`: política, generación, refs,
//! tarjeta y recibo. El hilo del motor es el único dueño de los navegadores; la
//! UI manda comandos y recibe frames ya decodificados, snapshots, actos y el
//! informe de handoff.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use base64::Engine as _;
use engine_cdp::{Browser, Chrome, DownloadEvent, ElementRef, Profile};
use receipt::{Identity, ReceiptLog};
use serde_json::Value;
use session::{ActOptions, ActOutcome, Handoff, Session, SessionConfig};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;

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
    CopyText(TabId, String),
    Download(DownloadEvent),
    Found {
        tab: TabId,
        total: usize,
        current: i64,
    },
    Notice(String),
    Error(String),
}

pub enum Command {
    NewTab {
        url: Option<String>,
    },
    CloseTab(TabId),
    SelectTab(TabId),
    Navigate(String),
    Back,
    Forward,
    Reload,
    Viewport {
        width: u32,
        height: u32,
        scale: f64,
    },
    Wheel {
        x: f64,
        y: f64,
        delta_y: f64,
    },
    SelectText(TabId, [f64; 2], [f64; 2]),
    CopyText(TabId),
    Click {
        x: f64,
        y: f64,
    },
    InsertText(String),
    Key {
        key: String,
        code: String,
        virtual_key: i64,
        modifiers: u32,
    },
    Approve {
        hash: String,
    },
    Deny,
    Handoff,
    Find {
        text: String,
        index: i64,
    },
    Permissions {
        origin: String,
        granted: Vec<String>,
        denied: Vec<String>,
    },
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

    pub fn back(&self) {
        let _ = self.commands.send(Command::Back);
    }

    pub fn forward(&self) {
        let _ = self.commands.send(Command::Forward);
    }

    pub fn reload(&self) {
        let _ = self.commands.send(Command::Reload);
    }

    pub fn viewport(&self, width: u32, height: u32, scale: f64) {
        let _ = self.commands.send(Command::Viewport {
            width,
            height,
            scale,
        });
    }

    pub fn wheel(&self, x: f64, y: f64, delta_y: f64) {
        let _ = self.commands.send(Command::Wheel { x, y, delta_y });
    }

    pub fn select_text(&self, tab: TabId, start: [f64; 2], end: [f64; 2]) {
        let _ = self.commands.send(Command::SelectText(tab, start, end));
    }

    pub fn copy_text(&self, tab: TabId) {
        let _ = self.commands.send(Command::CopyText(tab));
    }

    pub fn click(&self, x: f64, y: f64) {
        let _ = self.commands.send(Command::Click { x, y });
    }

    pub fn insert_text(&self, text: impl Into<String>) {
        let _ = self.commands.send(Command::InsertText(text.into()));
    }

    pub fn key(&self, key: &str, code: &str, virtual_key: i64, modifiers: u32) {
        let _ = self.commands.send(Command::Key {
            key: key.to_string(),
            code: code.to_string(),
            virtual_key,
            modifiers,
        });
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

    /// Fija los permisos de un origen en todas sus pestañas.
    pub fn permissions(
        &self,
        origin: impl Into<String>,
        granted: Vec<String>,
        denied: Vec<String>,
    ) {
        let _ = self.commands.send(Command::Permissions {
            origin: origin.into(),
            granted,
            denied,
        });
    }

    /// Busca en la página activa. `index` se toma módulo el total, así que
    /// avanzar y retroceder es sumar o restar uno.
    pub fn find(&self, text: impl Into<String>, index: i64) {
        let _ = self.commands.send(Command::Find {
            text: text.into(),
            index,
        });
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

#[derive(Clone)]
struct Emitter {
    tx: Sender<Event>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl Emitter {
    fn emit(&self, event: Event) {
        let _ = self.tx.send(event);
        (self.notify)();
    }
}

pub fn spawn(initial_url: String, log_path: PathBuf) -> EngineHandle {
    spawn_with_notify(initial_url, log_path, Arc::new(|| {}))
}

/// Arranque del motor con perfil persistente o efímero y varias pestañas.
pub struct Options {
    pub initial_urls: Vec<String>,
    pub log_path: PathBuf,
    /// `None` deja el perfil efímero (se borra al cerrar).
    pub profile: Option<PathBuf>,
    /// Carpeta donde Chromium deja las descargas.
    pub downloads: Option<PathBuf>,
}

/// `notify` despierta a la UI cuando hay un evento nuevo: sin él habría que
/// repintar a ciegas y se gastaba CPU con la página quieta.
pub fn spawn_with_notify(
    initial_url: String,
    log_path: PathBuf,
    notify: Arc<dyn Fn() + Send + Sync>,
) -> EngineHandle {
    spawn_with_options(
        Options {
            initial_urls: vec![initial_url],
            log_path,
            profile: None,
            downloads: None,
        },
        notify,
    )
}

pub fn spawn_with_options(options: Options, notify: Arc<dyn Fn() + Send + Sync>) -> EngineHandle {
    let (commands, command_rx) = unbounded_channel();
    let (events_tx, events) = channel();
    let (done_tx, done) = channel();
    let emitter = Emitter {
        tx: events_tx,
        notify,
    };

    std::thread::spawn(move || {
        if let Err(error) = run(options, command_rx, emitter.clone()) {
            emitter.emit(Event::Error(format!("{error:#}")));
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
    capture_scale: f64,
    session: Session<Chrome>,
    log_path: PathBuf,
    pending: Option<Pending>,
    /// Target CDP de la pestaña: sirve para cerrarla y traerla al frente.
    page_id: String,
}

struct State {
    tabs: BTreeMap<TabId, Tab>,
    active: Option<TabId>,
    next_id: TabId,
    log_base: PathBuf,
    browser: Option<Arc<Browser>>,
    profile: Option<PathBuf>,
    downloads: Option<PathBuf>,
}

fn run(options: Options, mut commands: UnboundedReceiver<Command>, events: Emitter) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("crear el runtime del motor")?;

    runtime.block_on(async move {
        let (frames, frame_rx) = watch::channel(None);
        tokio::spawn(decode_frames(frame_rx, events.clone()));
        let mut state = State {
            tabs: BTreeMap::new(),
            active: None,
            next_id: 0,
            log_base: options.log_path,
            browser: None,
            profile: options.profile,
            downloads: options.downloads,
        };

        for url in &options.initial_urls {
            if let Err(error) = open_tab(&mut state, url, &events).await {
                events.emit(Event::Error(format!("{error:#}")));
            }
        }

        loop {
            let input = if let Some(tab) = active_tab(&mut state) {
                tokio::select! {
                    command = commands.recv() => EngineInput::Command(command),
                    event = tab.session.next_event(Duration::from_millis(10)) => EngineInput::Browser(event?),
                }
            } else {
                EngineInput::Command(commands.recv().await)
            };

            match input {
                EngineInput::Command(None) => return Ok(()),
                EngineInput::Command(Some(command)) => {
                    match handle_command(&mut state, command, &events).await {
                        Ok(true) => return Ok(()),
                        Ok(false) => {}
                        Err(error) => events.emit(Event::Error(format!("{error:#}"))),
                    }
                }
                EngineInput::Browser(Some(event)) => {
                    let id = state.active.expect("evento de la pestaña activa");
                    handle_browser_event(&mut state, id, event, &events, &frames).await;
                }
                EngineInput::Browser(None) => {}
            }

            let background_tabs: Vec<_> = state.tabs.keys().copied().filter(|id| Some(*id) != state.active).collect();
            for id in background_tabs {
                let tab = state.tabs.get_mut(&id).expect("pestaña existente");
                if let Some(event) = tab.session.next_event(Duration::ZERO).await? {
                    handle_browser_event(&mut state, id, event, &events, &frames).await;
                }
            }
        }
    })
}

enum EngineInput {
    Command(Option<Command>),
    Browser(Option<Value>),
}

async fn handle_browser_event(
    state: &mut State,
    id: TabId,
    mut event: Value,
    events: &Emitter,
    frames: &watch::Sender<Option<Arc<EncodedFrame>>>,
) {
    match event["method"].as_str() {
        Some("Page.screencastFrame") => {
            let tab = state.tabs.get_mut(&id).expect("pestaña existente");
            if Some(id) == state.active
                && let Some(frame) = EncodedFrame::from_event(id, tab.capture_scale, &mut event)
            {
                frames.send_replace(Some(Arc::new(frame)));
            }
            if let Some(session_id) = event["params"]["sessionId"].as_i64() {
                tab.session.ack_screencast_frame(session_id).await.ok();
            }
        }
        Some("Page.domContentEventFired" | "Page.navigatedWithinDocument") => {
            refresh_after_page_navigation(state, id, events).await;
        }
        _ => {}
    }
}

/// `true` significa cerrar la shell.
async fn handle_command(state: &mut State, command: Command, events: &Emitter) -> Result<bool> {
    match command {
        Command::Shutdown => {
            state.tabs.clear();
            state.browser = None;
            return Ok(true);
        }
        Command::NewTab { url } => {
            open_tab(state, url.as_deref().unwrap_or("about:blank"), events).await?;
        }
        Command::CloseTab(id) => close_tab(state, id, events).await?,
        Command::SelectTab(id) => select_tab(state, id, events).await?,
        Command::Navigate(url) => navigate(state, &url, events).await?,
        Command::Back => history_move(state, -1, events).await?,
        Command::Forward => history_move(state, 1, events).await?,
        Command::Reload => reload(state, events).await?,
        Command::Viewport {
            width,
            height,
            scale,
        } => {
            if let Some(tab) = active_tab(state) {
                tab.session.set_viewport(width, height, scale).await?;
                tab.capture_scale = scale.clamp(1.0, 3.0);
                if tab.pending.is_none()
                    && let Some(id) = state.active
                {
                    refresh_snapshot(state, id, events).await?;
                }
            }
        }
        Command::Wheel { x, y, delta_y } => {
            if let Some(tab) = active_tab(state) {
                tab.session.scroll(x, y, delta_y).await?;
            }
        }
        Command::SelectText(id, start, end) => {
            if state.active == Some(id) {
                state.tabs[&id].session.select_text(start, end).await?;
            }
        }
        Command::CopyText(id) => {
            if state.active == Some(id) {
                let text = state.tabs[&id].session.selected_text().await?;
                if !text.is_empty() {
                    events.emit(Event::CopyText(id, text));
                }
            }
        }
        Command::Click { x, y } => click(state, x, y, events).await?,
        Command::InsertText(text) => {
            if let Some(tab) = active_tab(state) {
                tab.session.insert_text(&text).await?;
            }
        }
        Command::Key {
            key,
            code,
            virtual_key,
            modifiers,
        } => {
            if key == "Enter" && modifiers == 0 && submit_with_enter(state, events).await? {
                return Ok(false);
            }
            if let Some(tab) = active_tab(state) {
                tab.session.key(&key, &code, virtual_key, modifiers).await?;
            }
        }
        Command::Find { text, index } => {
            if let Some(tab) = active_tab(state) {
                let found = tab.session.find(&text, index).await?;
                let id = state.active.expect("hay pestaña activa");
                let _ = events.emit(Event::Found {
                    tab: id,
                    total: found.total,
                    current: found.current,
                });
            }
        }
        Command::Permissions {
            origin,
            granted,
            denied,
        } => {
            let tabs: Vec<TabId> = state
                .tabs
                .iter()
                .filter(|(_, tab)| {
                    tab.session
                        .url()
                        .and_then(origin_of)
                        .is_some_and(|current| current == origin)
                })
                .map(|(id, _)| *id)
                .collect();
            for id in tabs {
                state.tabs[&id]
                    .session
                    .set_permissions(&origin, &granted, &denied)
                    .await?;
            }
        }
        Command::Approve { hash } => approve(state, &hash, events).await?,
        Command::Deny => deny(state, events),
        Command::Handoff => handoff(state, events).await,
    }
    Ok(false)
}

fn active_tab(state: &mut State) -> Option<&mut Tab> {
    state.active.and_then(|id| state.tabs.get_mut(&id))
}

async fn open_tab(state: &mut State, url: &str, events: &Emitter) -> Result<TabId> {
    let id = state.next_id;
    state.next_id += 1;
    let log_path = tab_log_path(&state.log_base, id);
    let log =
        ReceiptLog::open(&log_path).with_context(|| format!("abrir el log de la pestaña {id}"))?;
    let browser = shared_browser(state, events).await?;
    let chrome = Chrome::attach_for_shell(browser)
        .await
        .context("abrir una pestaña en Chromium")?;
    let page_id = chrome.page_id().to_string();
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
    if let Some(previous) = active_tab(state) {
        previous.session.stop_screencast().await.ok();
    }
    state.active = Some(id);
    state.tabs.insert(
        id,
        Tab {
            capture_scale: 1.0,
            session,
            log_path,
            pending: None,
            page_id,
        },
    );

    let _ = events.emit(Event::TabOpened(info));
    let _ = events.emit(Event::Selected(Some(id)));
    let _ = events.emit(Event::Snapshot {
        tab: id,
        info: snapshot_info(&snapshot),
        elements,
    });
    send_receipts(state, id, events);
    Ok(id)
}

/// Un solo Chromium por sesión: así las pestañas comparten cookies y perfil.
///
/// Si otra ventana u otro Chromium tiene tomado el perfil, se arranca con un
/// perfil temporal y se avisa: es mejor navegar sin las sesiones guardadas que
/// quedarse con la ventana vacía.
async fn shared_browser(state: &mut State, events: &Emitter) -> Result<Arc<Browser>> {
    if let Some(browser) = &state.browser {
        return Ok(browser.clone());
    }
    let browser = match state.profile.clone() {
        Some(path) => match Browser::launch(Profile::persistent(path.clone()), true).await {
            Ok(browser) => browser,
            Err(error) if format!("{error:#}").contains("está en uso") => {
                let _ = events.emit(Event::Notice(format!(
                    "el perfil {} está ocupado por otro Chromium: abro con un perfil temporal y sin tus inicios de sesión",
                    path.display()
                )));
                state.profile = None;
                Browser::launch(Profile::ephemeral()?, true)
                    .await
                    .context("lanzar Chromium con un perfil temporal")?
            }
            Err(error) => return Err(error).context("lanzar Chromium"),
        },
        None => Browser::launch(Profile::ephemeral()?, true)
            .await
            .context("lanzar Chromium")?,
    };
    let browser = Arc::new(browser);
    if let Some(dir) = state.downloads.clone() {
        let emitter = events.clone();
        browser
            .watch_downloads(
                dir,
                Arc::new(move |download| {
                    let _ = emitter.emit(Event::Download(download));
                }),
            )
            .await
            .ok();
    }
    state.browser = Some(browser.clone());
    Ok(browser)
}

async fn close_tab(state: &mut State, id: TabId, events: &Emitter) -> Result<()> {
    let Some(tab) = state.tabs.remove(&id) else {
        return Ok(());
    };
    if let Some(browser) = &state.browser {
        browser.close_page(&tab.page_id).await.ok();
    }
    let _ = events.emit(Event::TabClosed(id));
    if state.active != Some(id) {
        return Ok(());
    }

    state.active = None;
    let next = state.tabs.keys().next().copied();
    match next {
        Some(next) => select_tab(state, next, events).await?,
        None => {
            let _ = events.emit(Event::Selected(None));
        }
    }
    Ok(())
}

async fn select_tab(state: &mut State, id: TabId, events: &Emitter) -> Result<()> {
    if !state.tabs.contains_key(&id) {
        return Ok(());
    }
    if let Some(old) = state.active
        && old != id
        && let Some(tab) = state.tabs.get_mut(&old)
    {
        tab.session.stop_screencast().await.ok();
    }
    if let Some(browser) = &state.browser
        && let Some(tab) = state.tabs.get(&id)
    {
        // Chromium no dibuja la pestaña de fondo: sin activarla, el screencast
        // se queda con el último frame.
        browser.activate_page(&tab.page_id).await.ok();
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
    let _ = events.emit(Event::Selected(Some(id)));
    let _ = events.emit(Event::Snapshot {
        tab: id,
        info,
        elements,
    });
    send_receipts(state, id, events);
    Ok(())
}

async fn navigate(state: &mut State, url: &str, events: &Emitter) -> Result<()> {
    let Some(id) = state.active else {
        open_tab(state, url, events).await?;
        return Ok(());
    };
    let info = {
        let Some(tab) = state.tabs.get_mut(&id) else {
            return Ok(());
        };
        let snapshot = tab.session.open(url).await?;
        tab.pending = None;
        (snapshot_info(&snapshot), snapshot.elements)
    };
    let _ = events.emit(Event::Snapshot {
        tab: id,
        info: info.0,
        elements: info.1,
    });
    send_receipts(state, id, events);
    Ok(())
}

async fn reload(state: &mut State, events: &Emitter) -> Result<()> {
    let Some(id) = state.active else {
        return Ok(());
    };
    let url = state
        .tabs
        .get(&id)
        .and_then(|tab| tab.session.url().map(str::to_string));
    match url {
        Some(url) => navigate(state, &url, events).await,
        None => {
            let _ = events.emit(Event::Notice("no hay una URL que recargar".into()));
            Ok(())
        }
    }
}

async fn history_move(state: &mut State, delta: i64, events: &Emitter) -> Result<()> {
    let Some(id) = state.active else {
        return Ok(());
    };
    let (moved, previous) = {
        let tab = state.tabs.get_mut(&id).expect("pestaña activa");
        let previous = tab.session.url().map(str::to_string);
        (tab.session.history(delta).await?, previous)
    };
    if !moved {
        let _ = events.emit(Event::Notice(
            "no hay más historial en esa dirección".into(),
        ));
        return Ok(());
    }
    let Some(previous) = previous else {
        return Ok(());
    };

    let snapshot = {
        let tab = state.tabs.get_mut(&id).expect("pestaña activa");
        wait_for_url_change(&mut tab.session, &previous).await?
    };
    if let Some(tab) = state.tabs.get_mut(&id) {
        tab.pending = None;
    }
    let _ = events.emit(Event::Snapshot {
        tab: id,
        info: snapshot_info(&snapshot),
        elements: snapshot.elements,
    });
    send_receipts(state, id, events);
    Ok(())
}

/// Tras `navigateToHistoryEntry` el DOM tarda en cambiar; no damos por buena
/// una foto con la URL anterior.
async fn wait_for_url_change(
    session: &mut Session<Chrome>,
    previous: &str,
) -> Result<engine_cdp::Snapshot> {
    let mut last: Option<Result<engine_cdp::Snapshot>> = None;
    for _ in 0..25 {
        tokio::time::sleep(Duration::from_millis(80)).await;
        match session.snapshot().await {
            Ok(snapshot) if snapshot.url != previous => return Ok(snapshot),
            Ok(snapshot) => last = Some(Ok(snapshot)),
            Err(error) => last = Some(Err(error.into())),
        }
    }
    match last {
        Some(Ok(snapshot)) => Ok(snapshot),
        Some(Err(error)) => Err(error),
        None => anyhow::bail!("la navegación no terminó"),
    }
}

fn origin_of(url: &str) -> Option<String> {
    let origin = url::Url::parse(url).ok()?.origin().ascii_serialization();
    (origin != "null").then_some(origin)
}

// Some(None) indica una verificación sin búsqueda recuperable.
pub(crate) fn google_verification(address: &str) -> Option<Option<String>> {
    let url = url::Url::parse(address).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("www.google.com")
        || !url.path().starts_with("/sorry/")
    {
        return None;
    }
    let query = url
        .query_pairs()
        .find(|(key, _)| key == "continue")
        .and_then(|(_, value)| url::Url::parse(&value).ok())
        .filter(|next| next.origin() == url.origin() && next.path() == "/search")
        .and_then(|next| {
            next.query_pairs()
                .find(|(key, _)| key == "q")
                .map(|(_, value)| value.into_owned())
        });
    Some(query)
}

async fn click(state: &mut State, x: f64, y: f64, events: &Emitter) -> Result<()> {
    let Some(id) = state.active else {
        return Ok(());
    };
    // El gesto lo envía el puntero humano. Chromium enruta el click al marco de la verificación.
    let tab = state.tabs.get_mut(&id).expect("pestaña activa");
    if tab.pending.is_none() && tab.session.url().and_then(google_verification).is_some() {
        // Confirma el documento vivo antes de enviar el clic, también tras una redirección.
        let current = tab.session.snapshot().await?;
        if google_verification(&current.url).is_some() {
            tab.session.click_at(x, y).await?;
        } else {
            events.emit(Event::Snapshot {
                tab: id,
                info: snapshot_info(&current),
                elements: current.elements,
            });
        }
        return Ok(());
    }
    let element = {
        let Some(tab) = state.tabs.get_mut(&id) else {
            return Ok(());
        };
        tab.session.element_at(x, y).await?
    };

    let Some(element) = element else {
        let tab = state.tabs.get_mut(&id).expect("pestaña activa");
        if let Some(href) = tab.session.link_at(x, y).await?
            && (href.starts_with("https://") || href.starts_with("http://"))
        {
            return navigate(state, &href, events).await;
        }
        tab.session.select_text([x, y], [x, y]).await?;
        events.emit(Event::Notice("listo".into()));
        return Ok(());
    };

    match element.role.as_str() {
        "button" => {
            let tab = &state.tabs[&id];
            if tab
                .session
                .url()
                .is_some_and(|url| url.starts_with("https://www.youtube.com/"))
                && let Some(query) = tab.session.youtube_search_query(&element.selector).await?
            {
                if query.is_empty() {
                    events.emit(Event::Notice("Escribe lo que quieres buscar".into()));
                    return Ok(());
                }
                let destination = url::Url::parse_with_params(
                    "https://www.youtube.com/results",
                    [("search_query", query)],
                )?;
                return navigate(state, destination.as_str(), events).await;
            }
        }
        "link" => {
            let href = {
                let tab = state.tabs.get_mut(&id).expect("pestaña activa");
                tab.session.link_href(&element.selector).await?
            };
            return match href {
                Some(href) if href.starts_with("http://") || href.starts_with("https://") => {
                    navigate(state, &href, events).await
                }
                _ => {
                    let _ = events.emit(Event::Notice(format!(
                        "el enlace «{}» no tiene un destino navegable",
                        element.name
                    )));
                    Ok(())
                }
            };
        }
        // Un click en un campo no ejecuta nada: solo le da el foco para que el
        // teclado reenviado escriba ahí.
        "textbox" | "searchbox" | "combobox" | "select" => {
            let tab = state.tabs.get_mut(&id).expect("pestaña activa");
            tab.session.click_at(x, y).await?;
            let _ = events.emit(Event::Notice(format!(
                "campo «{}» enfocado · escribe",
                element.name
            )));
            return Ok(());
        }
        _ => {
            let _ = events.emit(Event::Notice(format!(
                "«{}» es un {}: se puede leer, pero no actuar sin tarjeta",
                element.name, element.role
            )));
            return Ok(());
        }
    }

    let options = ActOptions {
        generation: Some(state.tabs[&id].session.generation()),
        ..ActOptions::default()
    };
    let outcome = {
        let tab = state.tabs.get_mut(&id).expect("pestaña activa");
        match tab.session.act(&element.reference, options.clone()).await {
            Err(error @ session::SessionError::ElementChanged(_)) => {
                refresh_snapshot(state, id, events).await?;
                return Err(error.into());
            }
            result => result?,
        }
    };

    settle_act(state, id, &element.reference, options, outcome, events).await?;
    Ok(())
}

/// Enter en un campo de un formulario equivale a pulsar su botón de envío: pasa
/// por tarjeta y recibo igual que un clic, en vez de salir por la red directo.
async fn submit_with_enter(state: &mut State, events: &Emitter) -> Result<bool> {
    let Some(id) = state.active else {
        return Ok(false);
    };
    let element = {
        let tab = &state.tabs[&id];
        let Some(selector) = tab.session.form_submitter().await? else {
            return Ok(false);
        };
        let Some(element) = tab.session.reference_for_selector(&selector) else {
            return Ok(false);
        };
        if element.role != "button" || element.disabled {
            return Ok(false);
        }
        element.clone()
    };
    let options = ActOptions {
        generation: Some(state.tabs[&id].session.generation()),
        ..ActOptions::default()
    };
    let outcome = {
        let tab = state.tabs.get_mut(&id).expect("pestaña activa");
        match tab.session.act(&element.reference, options.clone()).await {
            Err(error @ session::SessionError::ElementChanged(_)) => {
                refresh_snapshot(state, id, events).await?;
                return Err(error.into());
            }
            result => result?,
        }
    };
    settle_act(state, id, &element.reference, options, outcome, events).await?;
    Ok(true)
}

async fn approve(state: &mut State, hash: &str, events: &Emitter) -> Result<()> {
    let Some(id) = state.active else {
        return Ok(());
    };
    let Some(pending) = state.tabs.get_mut(&id).and_then(|tab| tab.pending.take()) else {
        let _ = events.emit(Event::Notice("no hay ninguna aprobación pendiente".into()));
        return Ok(());
    };

    if pending.options.approved_hash.as_deref() != Some(hash) {
        let _ = events.emit(Event::Notice(
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

fn deny(state: &mut State, events: &Emitter) {
    if let Some(tab) = active_tab(state) {
        tab.pending = None;
    }
    let _ = events.emit(Event::Notice("acción denegada por la persona".into()));
}

async fn handoff(state: &mut State, events: &Emitter) {
    let Some(id) = state.active else {
        return;
    };
    let report = {
        let Some(tab) = state.tabs.get_mut(&id) else {
            return;
        };
        tab.session.handoff().await
    };
    let _ = events.emit(Event::Handoff { tab: id, report });
}

async fn settle_act(
    state: &mut State,
    id: TabId,
    reference: &str,
    options: ActOptions,
    outcome: ActOutcome,
    events: &Emitter,
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

    let _ = events.emit(Event::Act { tab: id, outcome });
    Ok(())
}

async fn refresh_snapshot(state: &mut State, id: TabId, events: &Emitter) -> Result<()> {
    let Some(tab) = state.tabs.get_mut(&id) else {
        return Ok(());
    };
    // `act` invalidó el snapshot y avanzó la generación.
    let snapshot = tab.session.snapshot().await?;
    let _ = events.emit(Event::Snapshot {
        tab: id,
        info: snapshot_info(&snapshot),
        elements: snapshot.elements,
    });
    Ok(())
}

fn send_receipts(state: &State, id: TabId, events: &Emitter) {
    let Some(tab) = state.tabs.get(&id) else {
        return;
    };
    let _ = events.emit(Event::Receipts {
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

struct EncodedFrame {
    tab: TabId,
    jpeg: String,
    css_width: f64,
    css_height: f64,
}

impl EncodedFrame {
    fn from_event(tab: TabId, capture_scale: f64, event: &mut Value) -> Option<Self> {
        let params = &mut event["params"];
        let Value::String(jpeg) = params["data"].take() else {
            return None;
        };
        let metadata = &params["metadata"];
        Some(Self {
            tab,
            jpeg,
            css_width: metadata["deviceWidth"].as_f64().unwrap_or(1024.0) / capture_scale,
            css_height: metadata["deviceHeight"].as_f64().unwrap_or(768.0) / capture_scale,
        })
    }

    fn decode(&self) -> Result<Frame> {
        let bytes = base64::engine::general_purpose::STANDARD.decode(&self.jpeg)?;
        let rgba =
            image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)?.into_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(Frame {
            width: width as usize,
            height: height as usize,
            rgba: rgba.into_raw(),
            css_width: self.css_width,
            css_height: self.css_height,
        })
    }
}

async fn decode_frames(mut frames: watch::Receiver<Option<Arc<EncodedFrame>>>, events: Emitter) {
    let log_timing = std::env::var_os("NAVEGADOR_FRAME_LOG").is_some();
    while frames.changed().await.is_ok() {
        let Some(encoded) = frames.borrow_and_update().clone() else {
            continue;
        };
        let tab = encoded.tab;
        let started = Instant::now();
        let encoded_bytes = encoded.jpeg.len();
        if let Ok(Ok(frame)) = tokio::task::spawn_blocking(move || encoded.decode()).await {
            if log_timing {
                eprintln!(
                    "frame {}x{} jpeg={} KB decode={:?}",
                    frame.width,
                    frame.height,
                    encoded_bytes / 1024,
                    started.elapsed()
                );
            }
            events.emit(Event::Frame { tab, frame });
        }
    }
}

async fn refresh_after_page_navigation(state: &mut State, id: TabId, events: &Emitter) {
    let tab = state.tabs.get_mut(&id).expect("pestaña existente");
    let previous = tab.session.url().map(str::to_owned);
    // Una notificación de Chromium puede llegar después de que otro comando
    // ya haya actualizado la sesión. Contrasta con el documento vivo.
    if let Ok(snapshot) = tab.session.snapshot().await
        && previous.as_deref() != Some(&snapshot.url)
    {
        tab.pending = None;
        tab.session.refresh();
        if let Err(error) = refresh_snapshot(state, id, events).await {
            events.emit(Event::Error(format!("{error:#}")));
        }
        send_receipts(state, id, events);
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
