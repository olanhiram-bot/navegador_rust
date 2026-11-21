//! Shell con GPUI: la misma sesión auditable, dibujada por GPU.
//!
//! El motor y los recibos viven en el crate `shell`; aquí solo se pinta y se
//! traduce ratón/teclado a comandos. Incluye pestañas, navegación, zoom por
//! sitio, búsqueda en página, historial, marcadores, descargas, permisos y
//! ajustes persistentes.

mod embed;


use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use engine_cdp::DownloadState;
use gpui::{
    App, Application, Bounds, ClipboardItem, Context, FocusHandle, KeyDownEvent, MouseButton,
    MouseDownEvent, ObjectFit, Point, RenderImage, ScrollDelta, ScrollWheelEvent, SharedString,
    Window, WindowBounds, WindowOptions, div, img, prelude::*, px, rgb, size,
};
use shell::engine::{EngineHandle, Event, Options, TabId, TabInfo, spawn_with_options};
use shell::estado;

const TAB_BAR: f32 = 32.0;
const TOOLBAR: f32 = 42.0;
const STATUS: f32 = 22.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    None,
    Address,
    Find,
    History,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    None,
    Settings,
    History,
    Bookmarks,
    Downloads,
}

struct DownloadItem {
    id: String,
    name: String,
    path: PathBuf,
    state: DownloadState,
    received: u64,
    total: u64,
}

struct Shell {
    handle: EngineHandle,
    focus: FocusHandle,
    tabs: Vec<TabInfo>,
    active: Option<TabId>,
    address: String,
    field: Field,
    status: String,
    frame: Option<Arc<RenderImage>>,
    previous_frame: Option<Arc<RenderImage>>,
    css: (f32, f32),
    page_size: (f32, f32),
    last_viewport: (u32, u32, f64),
    pending: Option<(String, card::CardView)>,
    result: Option<String>,

    paths: estado::Paths,
    zoom: BTreeMap<String, f32>,
    motor: String,
    permisos: BTreeMap<String, estado::Permisos>,
    bookmarks: Vec<estado::Bookmark>,
    history: Vec<estado::Visit>,
    closed: Vec<String>,
    last_saved: Option<estado::SessionState>,
    last_visit: String,

    panel: Panel,
    find_text: String,
    find_total: usize,
    find_current: i64,
    history_query: String,
    downloads: Vec<DownloadItem>,

    handoff: Option<session::Handoff>,
    agent_open: bool,
    context: Option<(f32, f32)>,
    show_context: bool,
    menu_open: bool,

    native: bool,
    browser_pid: Option<u32>,
    embedded: Option<embed::Embed>,
    embed_attempt: Option<std::time::Instant>,
    screencast_stopped: bool,
}

impl Shell {
    fn new(cx: &mut Context<Self>, initial_urls: Vec<String>, paths: estado::Paths) -> Self {
        // En modo nativo se usa un perfil aparte: la instancia normal puede
        // tener el perfil principal bloqueado.
        let profile = estado::data_dir().map(|dir| {
            if native_mode() {
                dir.join("perfil-nativo")
            } else {
                dir.join("perfil")
            }
        });
        let log_path = estado::state_dir()
            .map(|dir| dir.join("recibos.jsonl"))
            .unwrap_or_else(|| PathBuf::from("navegador-recibos.jsonl"));
        let options = Options {
            initial_urls: initial_urls.clone(),
            log_path,
            profile,
            downloads: estado::downloads_dir(),
        };
        let handle = spawn_with_options(options, Arc::new(|| {}));
        let focus = cx.focus_handle();

        let settings = paths
            .settings
            .as_deref()
            .map(estado::load_settings)
            .unwrap_or_default();
        let history = paths
            .history
            .as_deref()
            .map(|path| estado::load_history(path, 500))
            .unwrap_or_default();
        let address = initial_urls
            .last()
            .cloned()
            .unwrap_or_else(|| "about:blank".into());

        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(12))
                    .await;
                let alive = this.update(cx, |shell, cx| {
                    shell.drain(cx);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();

        Shell {
            handle,
            focus,
            tabs: Vec::new(),
            active: None,
            address,
            field: Field::None,
            status: "cargando…".into(),
            frame: None,
            previous_frame: None,
            css: (1.0, 1.0),
            page_size: (1.0, 1.0),
            last_viewport: (0, 0, 0.0),
            pending: None,
            result: None,
            paths,
            zoom: settings.zoom,
            motor: settings.motor,
            permisos: settings.permisos,
            bookmarks: settings.marcadores,
            history,
            closed: Vec::new(),
            last_saved: None,
            last_visit: String::new(),
            panel: Panel::None,
            find_text: String::new(),
            find_total: 0,
            find_current: -1,
            history_query: String::new(),
            downloads: Vec::new(),
            handoff: None,
            agent_open: false,
            context: None,
            show_context: false,
            menu_open: false,
            native: native_mode(),
            browser_pid: None,
            embedded: None,
            embed_attempt: None,
            screencast_stopped: false,
        }
    }

    fn drain(&mut self, cx: &mut Context<Self>) {
        while let Ok(event) = self.handle.events.try_recv() {
            match event {
                Event::TabOpened(info) => {
                    if !self.tabs.iter().any(|tab| tab.id == info.id) {
                        self.tabs.push(info);
                    }
                }
                Event::TabClosed(id) => {
                    if let Some(tab) = self.tabs.iter().find(|tab| tab.id == id)
                        && estado::is_restorable(&tab.url)
                    {
                        self.closed.push(tab.url.clone());
                        if self.closed.len() > 10 {
                            self.closed.remove(0);
                        }
                    }
                    self.tabs.retain(|tab| tab.id != id);
                    if self.active == Some(id) {
                        self.frame = None;
                    }
                }
                Event::Selected(active) => {
                    self.active = active;
                    self.frame = None;
                    if let Some(tab) =
                        active.and_then(|id| self.tabs.iter().find(|tab| tab.id == id))
                    {
                        self.address = tab.url.clone();
                        self.status = format!("{} · {} elementos", tab.title, tab.elements);
                    } else {
                        self.status = "sin pestañas".into();
                    }
                }
                Event::Snapshot { tab, info, .. } => {
                    if let Some(known) = self.tabs.iter_mut().find(|known| known.id == tab) {
                        known.url = info.url.clone();
                        known.title = info.title.clone();
                        known.elements = info.elements;
                    }
                    if Some(tab) == self.active {
                        if !matches!(self.field, Field::Address) {
                            self.address = info.url.clone();
                        }
                        self.status = format!(
                            "{} · generación {} · {} elementos",
                            if info.title.is_empty() {
                                info.url.clone()
                            } else {
                                info.title.clone()
                            },
                            info.generation,
                            info.elements
                        );
                        self.record_visit(&info.url, &info.title);
                        self.apply_stored_permissions(&info.url);
                    }
                }
                Event::Frame { tab, frame } => {
                    if Some(tab) == self.active {
                        self.css = (frame.css_width as f32, frame.css_height as f32);
                        if let Some(image) = image::RgbaImage::from_raw(
                            frame.width as u32,
                            frame.height as u32,
                            frame.rgba,
                        ) {
                            let render = RenderImage::new(vec![image::Frame::new(image)]);
                            self.previous_frame = self.frame.replace(Arc::new(render));
                        }
                    }
                }
                Event::Act { tab, outcome } => {
                    if Some(tab) != self.active {
                        continue;
                    }
                    match &outcome {
                        session::ActOutcome::ApprovalRequired { hash, card, .. } => {
                            self.pending = Some((hash.clone(), card.clone()));
                            self.status = "esperando tu aprobación".into();
                        }
                        other => {
                            self.pending = None;
                            self.result = Some(other.render_text());
                            self.status = "resultado de la acción".into();
                        }
                    }
                }
                Event::Found {
                    tab,
                    total,
                    current,
                } => {
                    if Some(tab) == self.active {
                        self.find_total = total;
                        self.find_current = current;
                        self.status = if total == 0 {
                            format!("sin coincidencias para «{}»", self.find_text)
                        } else {
                            format!("coincidencia {} de {total}", current + 1)
                        };
                    }
                }
                Event::Download(download) => {
                    if let Some(item) = self
                        .downloads
                        .iter_mut()
                        .find(|item| item.id == download.id)
                    {
                        item.state = download.state;
                        if let DownloadState::Progress { received, total } = download.state {
                            item.received = received;
                            item.total = total;
                        }
                    } else {
                        self.downloads.push(DownloadItem {
                            id: download.id.clone(),
                            name: download.name.clone(),
                            path: download.path.clone(),
                            state: download.state,
                            received: 0,
                            total: 0,
                        });
                    }
                    self.status = match download.state {
                        DownloadState::Started => format!("descargando «{}»…", download.name),
                        DownloadState::Done => {
                            format!("descarga completa: {}", download.path.display())
                        }
                        DownloadState::Canceled => format!("descarga cancelada: {}", download.name),
                        DownloadState::Progress { .. } => self.status.clone(),
                    };
                }
                Event::BrowserPid(pid) => self.browser_pid = Some(pid),
                Event::CopyText(_, text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                Event::Notice(notice) => self.status = notice,
                Event::Error(error) => self.status = format!("error: {error}"),
                Event::Handoff { report, .. } => {
                    self.handoff = Some(report);
                    self.agent_open = true;
                }
                
                Event::Receipts { .. } => {}
            }
        }
        self.persist_session();
    }

    fn active_url(&self) -> String {
        self.active
            .and_then(|id| self.tabs.iter().find(|tab| tab.id == id))
            .map(|tab| tab.url.clone())
            .unwrap_or_else(|| self.address.clone())
    }

    fn origin(&self) -> Option<String> {
        estado::zoom_key(&self.active_url())
    }

    fn navigate(&mut self) {
        let url = shell::app::browser_url_with(&self.address, &self.motor);
        self.address = url.clone();
        self.field = Field::None;
        self.show_context = false;
        self.menu_open = false;
        self.handle.navigate(url);
    }

    fn record_visit(&mut self, url: &str, title: &str) {
        if url == self.last_visit || !estado::is_restorable(url) {
            return;
        }
        self.last_visit = url.to_string();
        let visit = estado::Visit {
            url: url.to_string(),
            title: title.to_string(),
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|time| time.as_secs())
                .unwrap_or(0),
        };
        if let Some(path) = &self.paths.history {
            let _ = estado::append_history(path, &visit);
        }
        self.history.insert(0, visit);
        self.history.truncate(500);
    }

    fn save_settings(&mut self) {
        let Some(path) = self.paths.settings.clone() else {
            return;
        };
        let settings = estado::Settings {
            zoom: self.zoom.clone(),
            permisos: self.permisos.clone(),
            marcadores: self.bookmarks.clone(),
            motor: self.motor.clone(),
        };
        if let Err(error) = estado::save_settings(&path, &settings) {
            self.status = format!("no se pudieron guardar los ajustes: {error}");
        }
    }

    fn persist_session(&mut self) {
        let Some(path) = self.paths.session.clone() else {
            return;
        };
        if self.tabs.iter().any(|tab| !estado::is_restorable(&tab.url)) {
            return;
        }
        let tabs: Vec<estado::TabState> = self
            .tabs
            .iter()
            .map(|tab| estado::TabState {
                url: tab.url.clone(),
                title: tab.title.clone(),
            })
            .collect();
        let active = self
            .active
            .and_then(|id| self.tabs.iter().position(|tab| tab.id == id))
            .unwrap_or(0);
        let state = estado::SessionState { tabs, active };
        if state.tabs.is_empty() || self.last_saved.as_ref() == Some(&state) {
            return;
        }
        if let Err(error) = estado::save(&path, &state) {
            self.status = format!("no se pudo guardar la sesión: {error}");
            return;
        }
        self.last_saved = Some(state);
    }

    fn toggle_bookmark(&mut self) {
        let url = self.active_url();
        if let Some(index) = self
            .bookmarks
            .iter()
            .position(|bookmark| bookmark.url == url)
        {
            self.bookmarks.remove(index);
            self.status = "marcador eliminado".into();
        } else {
            let title = self
                .tabs
                .iter()
                .find(|tab| tab.url == url)
                .map(|tab| tab.title.clone())
                .unwrap_or_default();
            self.bookmarks.insert(0, estado::Bookmark { url, title });
            self.status = "marcador guardado (Ctrl+D para quitarlo)".into();
        }
        self.save_settings();
    }

    fn zoom_for(&self, url: &str) -> f32 {
        estado::zoom_key(url)
            .and_then(|origin| self.zoom.get(&origin).copied())
            .unwrap_or(1.0)
    }

    fn adjust_zoom(&mut self, delta: f32) {
        let url = self.active_url();
        let value = self.zoom_for(&url) + delta;
        self.apply_zoom(&url, value);
    }

    fn apply_zoom(&mut self, url: &str, value: f32) {
        let Some(origin) = estado::zoom_key(url) else {
            self.status = "esta página no admite zoom".into();
            return;
        };
        let zoom = (value * 10.0).round().clamp(5.0, 30.0) / 10.0;
        self.zoom.insert(origin, zoom);
        self.last_viewport = (0, 0, 0.0);
        self.status = format!("zoom {}%", (zoom * 100.0).round() as i32);
        self.save_settings();
    }

    fn apply_stored_permissions(&mut self, url: &str) {
        let Some(origin) = estado::zoom_key(url) else {
            return;
        };
        if let Some(permisos) = self.permisos.get(&origin).copied() {
            self.handle
                .permissions(origin, permisos.granted(), permisos.denied());
        }
    }

    fn find_next(&mut self, forward: bool) {
        if self.find_text.is_empty() {
            self.handle.find(String::new(), 0);
            self.find_total = 0;
            self.find_current = -1;
            return;
        }
        let index = if self.find_total == 0 || self.find_current < 0 {
            0
        } else if forward {
            self.find_current + 1
        } else {
            self.find_current - 1
        };
        self.handle.find(self.find_text.clone(), index);
    }

    fn open_panel(&mut self, panel: Panel) {
        self.panel = panel;
        self.menu_open = false;
        self.show_context = false;
        if panel == Panel::History {
            self.field = Field::History;
        }
    }

    fn page_css(&self, position: Point<gpui::Pixels>) -> (f64, f64) {
        let (w, h) = self.page_size;
        let x = (f32::from(position.x) / w) as f64 * self.css.0 as f64;
        let y = ((f32::from(position.y) - TAB_BAR - TOOLBAR) / h) as f64 * self.css.1 as f64;
        (x, y)
    }

    fn page_click(&mut self, event: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.field = Field::None;
        self.menu_open = false;
        self.show_context = false;
        if self.pending.is_some() {
            cx.notify();
            return;
        }
        let (x, y) = self.page_css(event.position);
        if x >= 0.0 && y >= 0.0 {
            self.handle.click(x, y);
        }
        cx.notify();
    }

    fn page_middle(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending.is_some() {
            return;
        }
        let (x, y) = self.page_css(event.position);
        self.handle.open_link_new_tab(x, y, true);
        cx.notify();
    }

    fn page_right(&mut self, event: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        self.context = Some((f32::from(event.position.x), f32::from(event.position.y)));
        self.show_context = true;
        self.menu_open = false;
        cx.notify();
    }

    fn page_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (x, y) = self.page_css(event.position);
        let delta = match event.delta {
            ScrollDelta::Pixels(point) => f32::from(point.y),
            ScrollDelta::Lines(point) => point.y * 40.0,
        };
        self.handle.wheel(x, y, -delta as f64);
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let control = keystroke.modifiers.control || keystroke.modifiers.platform;
        let shift = keystroke.modifiers.shift;
        let key = keystroke.key.as_str();

        if control {
            match (key, shift) {
                ("l", _) => {
                    self.field = Field::Address;
                    self.address.clear();
                    self.menu_open = false;
                    cx.notify();
                    return;
                }
                ("t", false) => {
                    self.handle.new_tab(None);
                    return;
                }
                ("t", true) => {
                    if let Some(url) = self.closed.pop() {
                        self.handle.new_tab(Some(url));
                    }
                    return;
                }
                ("w", _) => {
                    if let Some(id) = self.active {
                        self.handle.close_tab(id);
                    }
                    return;
                }
                ("r", _) => {
                    self.handle.reload();
                    return;
                }
                ("q", _) => {
                    cx.quit();
                    return;
                }
                ("f", _) => {
                    self.field = Field::Find;
                    self.panel = Panel::None;
                    self.find_text.clear();
                    self.find_total = 0;
                    self.find_current = -1;
                    self.handle.find(String::new(), 0);
                    cx.notify();
                    return;
                }
                ("d", false) => {
                    self.toggle_bookmark();
                    cx.notify();
                    return;
                }
                ("b", true) => {
                    self.open_panel(Panel::Bookmarks);
                    cx.notify();
                    return;
                }
                ("h", _) => {
                    self.open_panel(Panel::History);
                    cx.notify();
                    return;
                }
                ("j", _) => {
                    self.open_panel(Panel::Downloads);
                    cx.notify();
                    return;
                }
                ("a", true) => {
                    self.agent_open = !self.agent_open;
                    cx.notify();
                    return;
                }
                ("," | "comma", _) => {
                    self.open_panel(Panel::Settings);
                    cx.notify();
                    return;
                }
                ("=" | "equal" | "plus" | "+", _) => {
                    self.adjust_zoom(0.1);
                    cx.notify();
                    return;
                }
                ("-" | "minus", _) => {
                    self.adjust_zoom(-0.1);
                    cx.notify();
                    return;
                }
                ("0" | "num0", _) => {
                    let url = self.active_url();
                    self.apply_zoom(&url, 1.0);
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }

        match self.field {
            Field::Address => {
                match key {
                    "enter" => self.navigate(),
                    "escape" => self.field = Field::None,
                    "backspace" => {
                        self.address.pop();
                    }
                    _ => {
                        if let Some(text) = &keystroke.key_char
                            && !control
                        {
                            self.address.push_str(text);
                        }
                    }
                }
                cx.notify();
                return;
            }
            Field::Find => {
                match key {
                    "enter" => self.find_next(!shift),
                    "escape" => {
                        self.field = Field::None;
                        self.handle.find(String::new(), 0);
                        self.find_total = 0;
                        self.find_current = -1;
                    }
                    "backspace" => {
                        self.find_text.pop();
                        self.handle.find(self.find_text.clone(), 0);
                    }
                    _ => {
                        if let Some(text) = &keystroke.key_char
                            && !control
                        {
                            self.find_text.push_str(text);
                            self.handle.find(self.find_text.clone(), 0);
                        }
                    }
                }
                cx.notify();
                return;
            }
            Field::History => {
                match key {
                    "escape" => {
                        self.field = Field::None;
                        self.panel = Panel::None;
                    }
                    "backspace" => {
                        self.history_query.pop();
                    }
                    _ => {
                        if let Some(text) = &keystroke.key_char
                            && !control
                        {
                            self.history_query.push_str(text);
                        }
                    }
                }
                cx.notify();
                return;
            }
            Field::None => {}
        }

        if self.panel != Panel::None {
            if key == "escape" {
                self.panel = Panel::None;
                cx.notify();
            }
            return;
        }
        if self.show_context || self.menu_open {
            if key == "escape" {
                self.show_context = false;
                self.menu_open = false;
                cx.notify();
            }
            return;
        }

        if let Some((hash, _)) = self.pending.clone() {
            match key {
                "enter" => {
                    self.handle.approve(hash);
                    self.pending = None;
                }
                "escape" => {
                    self.handle.deny();
                    self.pending = None;
                }
                _ => {}
            }
            cx.notify();
            return;
        }
        if self.result.is_some() {
            if key == "escape" || key == "enter" {
                self.result = None;
                cx.notify();
            }
            return;
        }

        if let Some(text) = &keystroke.key_char
            && !control
            && !keystroke.modifiers.alt
        {
            self.handle.insert_text(text.clone());
            return;
        }
        let special = match key {
            "enter" => Some(("Enter", "Enter", 13)),
            "backspace" => Some(("Backspace", "Backspace", 8)),
            "delete" => Some(("Delete", "Delete", 46)),
            "escape" => Some(("Escape", "Escape", 27)),
            "tab" => Some(("Tab", "Tab", 9)),
            "left" => Some(("ArrowLeft", "ArrowLeft", 37)),
            "up" => Some(("ArrowUp", "ArrowUp", 38)),
            "right" => Some(("ArrowRight", "ArrowRight", 39)),
            "down" => Some(("ArrowDown", "ArrowDown", 40)),
            "home" => Some(("Home", "Home", 36)),
            "end" => Some(("End", "End", 35)),
            "pageup" => Some(("PageUp", "PageUp", 33)),
            "pagedown" => Some(("PageDown", "PageDown", 34)),
            _ => None,
        };
        if let Some((name, code, virtual_key)) = special {
            let mut modifiers = 0;
            if keystroke.modifiers.alt {
                modifiers |= 1;
            }
            if control {
                modifiers |= 2;
            }
            if keystroke.modifiers.platform {
                modifiers |= 4;
            }
            if shift {
                modifiers |= 8;
            }
            self.handle.key(name, code, virtual_key, modifiers);
        }
        cx.notify();
    }

    fn suggestions(&self) -> Vec<(String, String)> {
        let needle = self.address.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let matches = |url: &str, title: &str| {
            url.to_lowercase().contains(&needle) || title.to_lowercase().contains(&needle)
        };
        for visit in &self.history {
            if matches(&visit.url, &visit.title) && seen.insert(visit.url.clone()) {
                out.push((visit.url.clone(), visit.title.clone()));
            }
        }
        for bookmark in &self.bookmarks {
            if matches(&bookmark.url, &bookmark.title) && seen.insert(bookmark.url.clone()) {
                out.push((bookmark.url.clone(), bookmark.title.clone()));
            }
        }
        for tab in &self.tabs {
            if matches(&tab.url, &tab.title) && seen.insert(tab.url.clone()) {
                out.push((tab.url.clone(), tab.title.clone()));
            }
        }
        out.truncate(6);
        out
    }
}

fn nav_button(
    cx: &mut Context<Shell>,
    id: &'static str,
    label: &'static str,
    action: fn(&mut Shell),
) -> impl IntoElement {
    div()
        .id(id)
        .h(px(26.))
        .px_2()
        .flex()
        .justify_center()
        .items_center()
        .rounded_md()
        .text_xs()
        .text_color(rgb(0xcfd3e0))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x2a2d36)))
        .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
            action(shell);
            cx.notify();
        }))
        .child(label)
}

fn menu_item(
    cx: &mut Context<Shell>,
    id: &'static str,
    label: impl Into<SharedString>,
    action: fn(&mut Shell),
) -> impl IntoElement {
    div()
        .id(id)
        .px_3()
        .py_1()
        .text_xs()
        .text_color(rgb(0xe6e8ef))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x333641)))
        .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
            action(shell);
            cx.notify();
        }))
        .child(label.into())
}

fn panel_close(cx: &mut Context<Shell>) -> impl IntoElement {
    div()
        .id("cerrar-panel")
        .px_2()
        .py_0p5()
        .rounded_md()
        .bg(rgb(0x333641))
        .text_color(rgb(0xe6e8ef))
        .cursor_pointer()
        .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
            shell.panel = Panel::None;
            shell.field = Field::None;
            cx.notify();
        }))
        .child("✕")
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focus.is_focused(window) && self.field == Field::None && self.pending.is_none() {
            window.focus(&self.focus);
        }
        if let Some(old) = self.previous_frame.take() {
            let _ = cx.drop_image(old, Some(window));
        }

        let viewport = window.viewport_size();
        let page_w = f32::from(viewport.width).max(320.0);
        let page_h = (f32::from(viewport.height) - TAB_BAR - TOOLBAR - STATUS).max(240.0);
        self.page_size = (page_w, page_h);
        let zoom = self.zoom_for(&self.active_url()).max(0.5);

        // Vista nativa: la ventana de Chromium se incrusta bajo la barra.
        if self.native {
            let overlays = self.panel != Panel::None
                || self.menu_open
                || self.show_context
                || self.field == Field::Find
                || self.pending.is_some()
                || self.result.is_some();
            let bounds = (
                0i16,
                (TAB_BAR + TOOLBAR) as i16,
                page_w as u16,
                page_h as u16,
            );
            if let Some(embed) = &mut self.embedded {
                embed.place(bounds.0, bounds.1, bounds.2, bounds.3);
                embed.set_visible(!overlays);
                if !self.screencast_stopped {
                    self.screencast_stopped = true;
                    self.handle.screencast(false);
                }
            } else {
                let due = self
                    .embed_attempt
                    .is_none_or(|at| at.elapsed() > Duration::from_millis(700));
                if due && self.browser_pid.is_some() {
                    self.embed_attempt = Some(std::time::Instant::now());
                    match embed::Embed::attach(self.browser_pid.unwrap()) {
                        Ok(mut embed) => {
                            embed.place(bounds.0, bounds.1, bounds.2, bounds.3);
                            self.embedded = Some(embed);
                        }
                        Err(error) => {
                            self.status = format!("vista nativa: {error:#}");
                        }
                    }
                }
            }
        }

        if !self.native {
            let css_w = (page_w / zoom).max(320.0);
            let css_h = (page_h / zoom).max(240.0);
            let scale = (window.scale_factor() * zoom).clamp(1.0, 3.0) as f64;
            let key = (css_w as u32, css_h as u32, scale);
            if key != self.last_viewport {
                self.last_viewport = key;
                self.handle.viewport(key.0, key.1, scale);
            }
        }

        let tabs = self.tabs.clone();
        let active = self.active;
        let address = match self.field {
            Field::Address => format!("{}|", self.address),
            _ => self.address.clone(),
        };
        let status = self.status.clone();
        let pending = self.pending.clone();
        let result = self.result.clone();
        let panel = self.panel;
        let find_open = self.field == Field::Find;
        let menu_open = self.menu_open;
        let suggestions = if self.field == Field::Address {
            self.suggestions()
        } else {
            Vec::new()
        };
        let context = self.context;
        let show_context = self.show_context;

        let tab_strip = div()
            .h(px(TAB_BAR))
            .w_full()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .bg(rgb(0x0b0c0f))
            .children(tabs.into_iter().map(|tab| {
                let selected = Some(tab.id) == active;
                let color = if selected {
                    rgb(0xffffff)
                } else {
                    rgb(0x9aa0ad)
                };
                let fill = if selected {
                    rgb(0x2d2f36)
                } else {
                    rgb(0x1a1c22)
                };
                let title = if tab.title.is_empty() {
                    tab.url.clone()
                } else {
                    tab.title.clone()
                };
                div()
                    .id(SharedString::from(format!("tab-{}", tab.id)))
                    .h(px(24.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .bg(fill)
                    .text_xs()
                    .text_color(color)
                    .cursor_pointer()
                    .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
                        shell.handle.select_tab(tab.id);
                        cx.notify();
                    }))
                    .child(if title.chars().count() > 22 {
                        format!("{}…", title.chars().take(22).collect::<String>())
                    } else {
                        title
                    })
            }))
            .child(
                div()
                    .id("nueva-pestana")
                    .w(px(24.))
                    .h(px(24.))
                    .flex()
                    .justify_center()
                    .items_center()
                    .text_color(rgb(0x9aa0ad))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x2a2d36)))
                    .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
                        shell.handle.new_tab(None);
                        cx.notify();
                    }))
                    .child("+"),
            );

        let toolbar = div()
            .h(px(TOOLBAR))
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .bg(rgb(0x111215))
            .child(nav_button(cx, "atras", "Atrás", |shell| {
                shell.handle.back()
            }))
            .child(nav_button(cx, "adelante", "Adelante", |shell| {
                shell.handle.forward()
            }))
            .child(nav_button(cx, "recargar", "Recargar", |shell| {
                shell.handle.reload()
            }))
            .child(
                div()
                    .flex_1()
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(rgb(0x262832))
                    .text_sm()
                    .text_color(rgb(0xd6d9e2))
                    .id("omnibox")
                    .cursor_text()
                    .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
                        shell.field = Field::Address;
                        shell.menu_open = false;
                        cx.notify();
                    }))
                    .child(address),
            )
            .child(
                div()
                    .id("menu")
                    .w(px(30.))
                    .h(px(26.))
                    .flex()
                    .justify_center()
                    .items_center()
                    .rounded_md()
                    .text_color(rgb(0xcfd3e0))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x2a2d36)))
                    .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
                        shell.menu_open = !shell.menu_open;
                        cx.notify();
                    }))
                    .child("☰"),
            );

        let loading = self.frame.is_none();
        let zoom_label = format!("{}%", (zoom * 100.0).round() as i32);
        let mut page = div()
            .relative()
            .flex_1()
            .w_full()
            .bg(rgb(0x0c0d0f))
            .overflow_hidden()
            .id("pagina")
            .on_mouse_down(MouseButton::Left, cx.listener(Self::page_click))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::page_middle))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::page_right))
            .on_scroll_wheel(cx.listener(Self::page_wheel));
        if self.native {
            // El contenido lo pinta la ventana incrustada.
        } else if let Some(frame) = self.frame.clone() {
            page = page.child(img(frame).object_fit(ObjectFit::Fill).w_full().h_full());
        } else if loading {
            page = page.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .justify_center()
                    .items_center()
                    .text_color(rgb(0x9aa0ad))
                    .child("cargando la página…"),
            );
        }

        let status_bar = div()
            .h(px(STATUS))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .bg(rgb(0x0b0c0f))
            .text_xs()
            .text_color(rgb(0x9aa0ad))
            .child(div().child(status))
            .child(div().child(zoom_label));

        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0b0c0f))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(tab_strip)
            .child(toolbar)
            .child(page)
            .child(status_bar);

        // Menú ☰
        if menu_open {
            root = root.child(
                div()
                    .absolute()
                    .right(px(12.))
                    .top(px(TAB_BAR + TOOLBAR))
                    .w(px(210.))
                    .py_1()
                    .rounded_md()
                    .bg(rgb(0x20222a))
                    .border_1()
                    .border_color(rgb(0x3a3d47))
                    .flex()
                    .flex_col()
                    .child(menu_item(cx, "menu-ajustes", "Ajustes", |shell| {
                        shell.open_panel(Panel::Settings)
                    }))
                    .child(menu_item(cx, "menu-historial", "Historial", |shell| {
                        shell.open_panel(Panel::History)
                    }))
                    .child(menu_item(cx, "menu-marcadores", "Marcadores", |shell| {
                        shell.open_panel(Panel::Bookmarks)
                    }))
                    .child(menu_item(cx, "menu-descargas", "Descargas", |shell| {
                        shell.open_panel(Panel::Downloads)
                    }))
                    .child(menu_item(
                        cx,
                        "menu-permisos",
                        "Permisos de este sitio",
                        |shell| shell.open_panel(Panel::Settings),
                    )),
            );
        }

        // Sugerencias del omnibox
        if !suggestions.is_empty() {
            let width = (page_w * 0.55).clamp(320.0, 620.0);
            root = root.child(
                div()
                    .absolute()
                    .left(px(110.))
                    .top(px(TAB_BAR + TOOLBAR + 2.))
                    .w(px(width))
                    .rounded_md()
                    .bg(rgb(0x20222a))
                    .border_1()
                    .border_color(rgb(0x3a3d47))
                    .flex()
                    .flex_col()
                    .p_1()
                    .children(suggestions.into_iter().map(|(url, title)| {
                        let name = if title.is_empty() {
                            url.clone()
                        } else {
                            title.clone()
                        };
                        let target = url.clone();
                        div()
                            .id(SharedString::from(format!("sug-{url}")))
                            .px_2()
                            .py_1()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x333641)))
                            .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
                                shell.address = target.clone();
                                shell.navigate();
                                cx.notify();
                            }))
                            .child(div().text_xs().text_color(rgb(0xe6e8ef)).child(name))
                            .child(div().text_xs().text_color(rgb(0x9aa0ad)).child(url))
                    })),
            );
        }

        // Barra de búsqueda en página
        if find_open {
            let counter = if self.find_text.is_empty() {
                String::new()
            } else if self.find_total == 0 {
                "0".into()
            } else {
                format!("{} de {}", self.find_current + 1, self.find_total)
            };
            let text = self.find_text.clone();
            root = root.child(
                div()
                    .absolute()
                    .right(px(18.))
                    .top(px(TAB_BAR + TOOLBAR + 6.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(0x20222a))
                    .border_1()
                    .border_color(rgb(0x3a3d47))
                    .text_xs()
                    .text_color(rgb(0xe6e8ef))
                    .child(
                        div()
                            .w(px(220.))
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .bg(rgb(0x111215))
                            .child(if text.is_empty() {
                                "Buscar en la página".to_string()
                            } else {
                                text
                            }),
                    )
                    .child(div().text_color(rgb(0x9aa0ad)).child(counter))
                    .child(nav_button(cx, "buscar-prev", "Anterior", |shell| {
                        shell.find_next(false)
                    }))
                    .child(nav_button(cx, "buscar-next", "Siguiente", |shell| {
                        shell.find_next(true)
                    })),
            );
        }

        // Menú contextual
        if show_context && let Some((x, y)) = context {
            let (ctx_x, ctx_y) = self.page_css(gpui::point(px(x), px(y)));
            let item = |id: &'static str, label: &'static str| {
                div()
                    .id(id)
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(0xe6e8ef))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x333641)))
                    .child(label)
            };
            let nueva = item("ctx-nueva", "Abrir enlace en pestaña nueva").on_click(cx.listener(
                move |shell, _: &gpui::ClickEvent, _, cx| {
                    shell.handle.open_link_new_tab(ctx_x, ctx_y, false);
                    shell.show_context = false;
                    cx.notify();
                },
            ));
            let segundo = item("ctx-segundo", "Abrir enlace en segundo plano").on_click(
                cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
                    shell.handle.open_link_new_tab(ctx_x, ctx_y, true);
                    shell.show_context = false;
                    cx.notify();
                }),
            );
            let copiar = item("ctx-copiar", "Copiar dirección del enlace").on_click(cx.listener(
                move |shell, _: &gpui::ClickEvent, _, cx| {
                    shell.handle.copy_link(ctx_x, ctx_y);
                    shell.show_context = false;
                    cx.notify();
                },
            ));
            let atras = item("ctx-atras", "Atrás").on_click(cx.listener(
                |shell, _: &gpui::ClickEvent, _, cx| {
                    shell.handle.back();
                    shell.show_context = false;
                    cx.notify();
                },
            ));
            let adelante = item("ctx-adelante", "Adelante").on_click(cx.listener(
                |shell, _: &gpui::ClickEvent, _, cx| {
                    shell.handle.forward();
                    shell.show_context = false;
                    cx.notify();
                },
            ));
            let recargar = item("ctx-recargar", "Recargar").on_click(cx.listener(
                |shell, _: &gpui::ClickEvent, _, cx| {
                    shell.handle.reload();
                    shell.show_context = false;
                    cx.notify();
                },
            ));
            root = root.child(
                div()
                    .absolute()
                    .left(px(x.min(page_w - 240.0)))
                    .top(px(y.min(page_h)))
                    .w(px(230.))
                    .py_1()
                    .rounded_md()
                    .bg(rgb(0x20222a))
                    .border_1()
                    .border_color(rgb(0x3a3d47))
                    .flex()
                    .flex_col()
                    .child(nueva)
                    .child(segundo)
                    .child(copiar)
                    .child(div().h(px(1.)).my_1().bg(rgb(0x3a3d47)))
                    .child(atras)
                    .child(adelante)
                    .child(recargar),
            );
        }

        // Paneles
        if panel != Panel::None {
            let title = match panel {
                Panel::Settings => "Ajustes",
                Panel::History => "Historial",
                Panel::Bookmarks => "Marcadores",
                Panel::Downloads => "Descargas",
                Panel::None => "",
            };
            let mut content = div().flex().flex_col().gap_2();
            match panel {
                Panel::Settings => {
                    content = content
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().text_color(rgb(0x9aa0ad)).child("Motor de búsqueda"))
                                .child(nav_button(cx, "motor-ddg", "DuckDuckGo", |shell| {
                                    shell.motor = "duckduckgo".into();
                                    shell.save_settings();
                                }))
                                .child(nav_button(cx, "motor-google", "Google", |shell| {
                                    shell.motor = "google".into();
                                    shell.save_settings();
                                })),
                        )
                        .child(div().text_color(rgb(0x9aa0ad)).child(
                            match estado::downloads_dir() {
                                Some(dir) => format!("Descargas: {}", dir.display()),
                                None => "Sin carpeta de descargas".into(),
                            },
                        ));
                    if let Some(origin) = self.origin() {
                        let actual = self.permisos.get(&origin).copied().unwrap_or_default();
                        let mk =
                            |id: &'static str,
                             label: &'static str,
                             checked: bool,
                             campo: fn(&mut estado::Permisos)| {
                                let origin = origin.clone();
                                div()
                                    .id(id)
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .cursor_pointer()
                                    .on_click(cx.listener(
                                        move |shell, _: &gpui::ClickEvent, _, cx| {
                                            let mut p = shell
                                                .permisos
                                                .get(&origin)
                                                .copied()
                                                .unwrap_or_default();
                                            campo(&mut p);
                                            shell.permisos.insert(origin.clone(), p);
                                            shell.handle.permissions(
                                                origin.clone(),
                                                p.granted(),
                                                p.denied(),
                                            );
                                            shell.save_settings();
                                            cx.notify();
                                        },
                                    ))
                                    .child(
                                        div()
                                            .w(px(14.))
                                            .h(px(14.))
                                            .rounded_sm()
                                            .border_1()
                                            .border_color(rgb(0x555a68))
                                            .bg(if checked {
                                                rgb(0x3a5496)
                                            } else {
                                                rgb(0x111215)
                                            }),
                                    )
                                    .child(label)
                            };
                        content = content
                            .child(
                                div()
                                    .mt_2()
                                    .text_color(rgb(0x9aa0ad))
                                    .child(format!("Permisos de {origin}")),
                            )
                            .child(mk(
                                "perm-notif",
                                "Notificaciones",
                                actual.notificaciones,
                                |p| p.notificaciones = !p.notificaciones,
                            ))
                            .child(mk("perm-ubi", "Ubicación", actual.ubicacion, |p| {
                                p.ubicacion = !p.ubicacion
                            }))
                            .child(mk("perm-cam", "Cámara", actual.camara, |p| {
                                p.camara = !p.camara
                            }))
                            .child(mk("perm-mic", "Micrófono", actual.microfono, |p| {
                                p.microfono = !p.microfono
                            }));
                    }
                    content = content.child(
                        div()
                            .mt_2()
                            .text_color(rgb(0x9aa0ad))
                            .child(format!("Navegador {}", env!("CARGO_PKG_VERSION"))),
                    );
                }
                Panel::History => {
                    let query = self.history_query.to_lowercase();
                    let rows: Vec<(String, String)> = self
                        .history
                        .iter()
                        .filter(|visit| {
                            query.is_empty()
                                || visit.url.to_lowercase().contains(&query)
                                || visit.title.to_lowercase().contains(&query)
                        })
                        .take(100)
                        .map(|visit| (visit.url.clone(), visit.title.clone()))
                        .collect();
                    content = content
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .bg(rgb(0x111215))
                                .text_color(if self.history_query.is_empty() {
                                    rgb(0x777d8c)
                                } else {
                                    rgb(0xe6e8ef)
                                })
                                .child(if self.history_query.is_empty() {
                                    "Filtrar historial".to_string()
                                } else {
                                    self.history_query.clone()
                                }),
                        )
                        .child(nav_button(cx, "hist-borrar", "Borrar historial", |shell| {
                            if let Some(path) = &shell.paths.history {
                                let _ = estado::clear_history(path);
                            }
                            shell.history.clear();
                            shell.last_visit.clear();
                            shell.status = "historial borrado".into();
                        }))
                        .children(rows.into_iter().map(|(url, title)| {
                            let name = if title.is_empty() {
                                url.clone()
                            } else {
                                title.clone()
                            };
                            let target = url.clone();
                            div()
                                .id(SharedString::from(format!("hist-{url}")))
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(0x333641)))
                                .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
                                    shell.panel = Panel::None;
                                    shell.field = Field::None;
                                    shell.handle.navigate(target.clone());
                                    cx.notify();
                                }))
                                .child(div().text_color(rgb(0xe6e8ef)).child(name))
                                .child(div().text_color(rgb(0x9aa0ad)).child(url))
                        }));
                }
                Panel::Bookmarks => {
                    if self.bookmarks.is_empty() {
                        content = content.child(
                            div()
                                .text_color(rgb(0x9aa0ad))
                                .child("Sin marcadores. Pulsa Ctrl+D en una página."),
                        );
                    }
                    content =
                        content.children(self.bookmarks.clone().into_iter().map(|bookmark| {
                            let url = bookmark.url.clone();
                            let name = if bookmark.title.is_empty() {
                                url.clone()
                            } else {
                                bookmark.title.clone()
                            };
                            let open_url = url.clone();
                            div()
                                .id(SharedString::from(format!("bm-{url}")))
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(0x333641)))
                                .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
                                    shell.panel = Panel::None;
                                    shell.field = Field::None;
                                    shell.handle.navigate(open_url.clone());
                                    cx.notify();
                                }))
                                .child(div().text_color(rgb(0xe6e8ef)).child(name))
                                .child(div().text_color(rgb(0x9aa0ad)).child(url))
                        }));
                }
                Panel::Downloads => {
                    if self.downloads.is_empty() {
                        content = content.child(
                            div()
                                .text_color(rgb(0x9aa0ad))
                                .child("Todavía no hay descargas en esta sesión."),
                        );
                    }
                    content = content.children(self.downloads.iter().map(|item| {
                        let name = item.name.clone();
                        let path = item.path.clone();
                        let detail = match item.state {
                            DownloadState::Done => "completada · pulsa para abrir".to_string(),
                            DownloadState::Canceled => "cancelada".to_string(),
                            DownloadState::Started => "empezando…".to_string(),
                            DownloadState::Progress { .. } => {
                                if item.total > 0 {
                                    format!("{} %", (item.received * 100 / item.total).min(100))
                                } else {
                                    "descargando…".into()
                                }
                            }
                        };
                        div()
                            .id(SharedString::from(format!("dl-{}", item.id)))
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x333641)))
                            .on_click(cx.listener(move |_, _: &gpui::ClickEvent, _, _| {
                                let _ = webbrowser::open(&path.to_string_lossy());
                            }))
                            .child(div().text_color(rgb(0xe6e8ef)).child(name))
                            .child(div().text_color(rgb(0x9aa0ad)).child(detail))
                    }));
                }
                Panel::None => {}
            }
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .justify_center()
                    .items_start()
                    .pt(px(TAB_BAR + TOOLBAR + 16.))
                    .child(
                        div()
                            .id("panel-scroll")
                            .w(px(620.))
                            .max_h(px(page_h - 40.0))
                            .overflow_y_scroll()
                            .p_4()
                            .rounded_lg()
                            .bg(rgb(0x20222a))
                            .border_1()
                            .border_color(rgb(0x3a3d47))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .text_sm()
                            .text_color(rgb(0xe6e8ef))
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .items_center()
                                    .child(div().text_lg().child(title))
                                    .child(panel_close(cx)),
                            )
                            .child(content),
                    ),
            );
        }

        // Panel del agente
        if self.agent_open {
            let handoff = self.handoff.clone();
            let body = match &handoff {
                Some(report) => vec![
                    format!("URL: {}", report.url),
                    format!("Título: {}", report.title),
                    format!("Generación: {}", report.generation),
                    format!("Elementos: {}", report.elements.len()),
                    format!("Recibos: {}", report.receipts),
                    format!(
                        "Último recibo: {}",
                        report.last_receipt_hash.as_deref().unwrap_or("—")
                    ),
                    format!("Log: {}", report.log_path.display()),
                ],
                None => vec!["Sin handoff todavía. Pulsa «Ver handoff».".to_string()],
            };
            root = root.child(
                div()
                    .absolute()
                    .right(px(16.))
                    .top(px(TAB_BAR + TOOLBAR + 8.))
                    .w(px(420.))
                    .p_3()
                    .rounded_lg()
                    .bg(rgb(0x20222a))
                    .border_1()
                    .border_color(rgb(0x3a3d47))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_xs()
                    .text_color(rgb(0xe6e8ef))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(div().text_lg().child("Agente"))
                            .child(
                                div()
                                    .id("cerrar-agente")
                                    .px_2()
                                    .rounded_md()
                                    .bg(rgb(0x333641))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
                                        shell.agent_open = false;
                                        cx.notify();
                                    }))
                                    .child("✕"),
                            ),
                    )
                    .children(body.into_iter().map(|line| {
                        div()
                            .text_color(rgb(0x9aa0ad))
                            .whitespace_normal()
                            .child(line)
                    }))
                    .child(nav_button(cx, "ver-handoff", "Ver handoff", |shell| {
                        shell.handle.handoff()
                    })),
            );
        }

        // Tarjeta de aprobación
        if let Some((hash, card)) = pending {
            let approve = hash.clone();
            let rows = card.rows.clone();
            let title = card.title.clone();
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .justify_center()
                    .items_center()
                    .bg(gpui::rgba(0x000000aa))
                    .child(
                        div()
                            .w(px(520.))
                            .p_5()
                            .rounded_lg()
                            .bg(rgb(0x20222a))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .text_sm()
                            .text_color(rgb(0xe6e8ef))
                            .child(div().text_lg().child(title))
                            .children(rows.into_iter().map(|row| {
                                div()
                                    .flex()
                                    .justify_between()
                                    .gap_4()
                                    .child(div().text_color(rgb(0x9aa0ad)).child(row.label))
                                    .child(row.value)
                            }))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .mt_2()
                                    .child(
                                        div()
                                            .id("aprobar")
                                            .px_3()
                                            .py_1()
                                            .rounded_md()
                                            .bg(rgb(0x3a5496))
                                            .cursor_pointer()
                                            .on_click(cx.listener(
                                                move |shell, _: &gpui::ClickEvent, _, cx| {
                                                    shell.handle.approve(approve.clone());
                                                    shell.pending = None;
                                                    cx.notify();
                                                },
                                            ))
                                            .child("Aprobar"),
                                    )
                                    .child(
                                        div()
                                            .id("denegar")
                                            .px_3()
                                            .py_1()
                                            .rounded_md()
                                            .bg(rgb(0x3a3d47))
                                            .cursor_pointer()
                                            .on_click(cx.listener(
                                                |shell, _: &gpui::ClickEvent, _, cx| {
                                                    shell.handle.deny();
                                                    shell.pending = None;
                                                    cx.notify();
                                                },
                                            ))
                                            .child("Denegar"),
                                    ),
                            ),
                    ),
            );
        }

        // Resultado
        if let Some(result) = result {
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .justify_center()
                    .items_center()
                    .bg(gpui::rgba(0x000000aa))
                    .child(
                        div()
                            .w(px(560.))
                            .p_5()
                            .rounded_lg()
                            .bg(rgb(0x20222a))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .text_xs()
                            .text_color(rgb(0xe6e8ef))
                            .child(div().text_lg().child("Resultado"))
                            .child(div().whitespace_normal().child(result))
                            .child(
                                div()
                                    .id("cerrar-resultado")
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(0x3a3d47))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
                                        shell.result = None;
                                        cx.notify();
                                    }))
                                    .child("Cerrar"),
                            ),
                    ),
            );
        }

        root
    }
}

fn native_mode() -> bool {
    std::env::var_os("NAVEGADOR_NATIVO").is_some()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--nativo") {
        // Debe fijarse antes de crear el motor, que lee la variable.
        unsafe { std::env::set_var("NAVEGADOR_NATIVO", "1200x800") };
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("navegador-gpui {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|arg| arg == "--nativo") {
        // Chromium corre en el X11 del escritorio con una ventana `--app`;
        // la shell la incrusta (requiere X11/XWayland).
        unsafe {
            std::env::set_var("NAVEGADOR_NATIVO", "1024x768");
        }
    }
    let cli_url = args
        .into_iter()
        .filter(|arg| arg != "--nativo")
        .find(|arg| !arg.starts_with("--"))
        .map(|url| shell::app::browser_url(&url));

    let paths = estado::Paths {
        session: estado::session_path(),
        settings: estado::settings_path(),
        history: estado::history_path(),
    };
    let initial_urls = match cli_url {
        Some(url) => vec![url],
        None => paths
            .session
            .as_deref()
            .and_then(estado::load)
            .map(|state| state.ordered_urls())
            .unwrap_or_else(|| vec!["about:blank".to_string()]),
    };

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.), px(780.)), cx);
        let urls = initial_urls.clone();
        let paths = paths.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Navegador".into()),
                    ..Default::default()
                }),
                app_id: Some("navegador".into()),
                ..Default::default()
            },
            move |_, cx| cx.new(|cx| Shell::new(cx, urls.clone(), paths.clone())),
        )
        .expect("no se pudo abrir la ventana");
        cx.activate(true);
    });
}
