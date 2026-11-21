//! UI de la shell: pestañas, barra de navegador, página, tarjeta y agente.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use card::CardView;
use eframe::egui;
use egui::{Align, Color32, Layout, Rect, Sense, Vec2};
use engine_cdp::ElementRef;
use session::{ActOutcome, Handoff};

use crate::engine::{EngineHandle, Event, TabId, TabInfo, google_verification};

const CHROME: Color32 = Color32::from_rgb(17, 18, 21);
const TAB_ACTIVE: Color32 = Color32::from_rgb(45, 47, 54);
const TAB_IDLE: Color32 = Color32::from_rgb(29, 30, 35);
const OMNI: Color32 = Color32::from_rgb(38, 40, 47);
const PAGE_BG: Color32 = Color32::from_rgb(12, 13, 15);
const ACCENT: Color32 = Color32::from_rgb(124, 158, 255);
const MUTED: Color32 = Color32::from_rgb(154, 158, 170);
const DANGER: Color32 = Color32::from_rgb(255, 128, 128);

struct TabFrame {
    texture: egui::TextureHandle,
    css: Vec2,
}

#[derive(Clone)]
struct PendingCard {
    card: CardView,
    hash: String,
}

struct DownloadItem {
    id: String,
    name: String,
    path: PathBuf,
    state: engine_cdp::DownloadState,
    received: u64,
    total: u64,
}

#[derive(Default)]
struct Receipts {
    count: usize,
    last: Option<String>,
    log: Option<PathBuf>,
}

pub struct ShellApp {
    handle: EngineHandle,
    tabs: Vec<TabInfo>,
    active: Option<TabId>,
    address: String,
    status: String,
    frames: HashMap<TabId, TabFrame>,
    pending: Option<PendingCard>,
    result: Option<String>,
    agent_open: bool,
    agent_control: bool,
    handoff: Option<Handoff>,
    receipts: Receipts,
    elements: Vec<ElementRef>,
    generation: u32,
    selection_origin: Option<egui::Pos2>,
    last_viewport: (Vec2, f32),
    /// Pide el foco para la página tras navegar: el teclado va a ella sin
    /// tener que hacer clic primero.
    focus_page: bool,
    /// Estado de foco de la ventana, para no gastar CPU en segundo plano.
    window_focused: Option<bool>,
    downloads: Vec<DownloadItem>,
    downloads_open: bool,
    bookmarks: Vec<crate::estado::Bookmark>,
    bookmarks_open: bool,
    history: Vec<crate::estado::Visit>,
    history_open: bool,
    history_query: String,
    history_path: Option<PathBuf>,
    settings_open: bool,
    motor: String,
    /// Dónde se dibujan las sugerencias del omnibox.
    suggest_rect: Option<Rect>,
    /// Pestañas cerradas, para Ctrl+Shift+T.
    closed_tabs: Vec<String>,
    /// Posición (CSS) del último clic derecho en la página.
    context_at: Option<(f64, f64)>,
    /// Última página anotada, para no duplicar visitas seguidas.
    last_visit: String,
    /// Barra de búsqueda dentro de la página (Ctrl+F).
    find_open: bool,
    find_text: String,
    find_total: usize,
    find_current: i64,
    find_focus: bool,
    /// Último texto enviado al motor, para no repetir la búsqueda cada frame.
    find_query: String,
    /// Zoom por origen, persistido en `ajustes.json`.
    zoom: std::collections::BTreeMap<String, f32>,
    /// Permisos por origen, también en `ajustes.json`.
    permisos: std::collections::BTreeMap<String, crate::estado::Permisos>,
    permisos_open: bool,
    settings_path: Option<PathBuf>,
    /// Dónde se guarda la sesión; `None` con perfil efímero.
    session_path: Option<PathBuf>,
    saved_session: Option<crate::estado::SessionState>,
}

impl ShellApp {
    pub fn new(
        options: crate::engine::Options,
        paths: crate::estado::Paths,
        cc: &eframe::CreationContext<'_>,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = CHROME;
        visuals.window_fill = Color32::from_rgb(32, 34, 40);
        visuals.extreme_bg_color = Color32::from_rgb(15, 16, 19);
        visuals.selection.bg_fill = Color32::from_rgb(58, 84, 150);
        ctx.set_visuals(visuals);

        let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
        style.spacing.item_spacing = egui::vec2(6.0, 6.0);
        style.spacing.button_padding = egui::vec2(9.0, 4.0);
        ctx.set_style_of(egui::Theme::Dark, style);

        let session_path = paths.session.clone();
        let settings_path = paths.settings.clone();
        let settings = settings_path
            .as_deref()
            .map(crate::estado::load_settings)
            .unwrap_or_default();
        let zoom = settings.zoom;
        let permisos = settings.permisos;
        let bookmarks = settings.marcadores;
        let motor = settings.motor;
        let history = paths
            .history
            .as_deref()
            .map(|path| crate::estado::load_history(path, 500))
            .unwrap_or_default();
        let first = options
            .initial_urls
            .last()
            .cloned()
            .unwrap_or_else(|| "about:blank".to_string());
        let repaint_ctx = ctx.clone();
        ShellApp {
            handle: crate::engine::spawn_with_options(
                options,
                Arc::new(move || repaint_ctx.request_repaint()),
            ),
            tabs: Vec::new(),
            active: None,
            address: first,
            status: "cargando…".to_string(),
            frames: HashMap::new(),
            pending: None,
            result: None,
            agent_open: false,
            agent_control: false,
            handoff: None,
            receipts: Receipts::default(),
            elements: Vec::new(),
            generation: 0,
            selection_origin: None,
            last_viewport: (Vec2::ZERO, 0.0),
            focus_page: false,
            window_focused: None,
            downloads: Vec::new(),
            downloads_open: false,
            bookmarks,
            bookmarks_open: false,
            history,
            history_open: false,
            history_query: String::new(),
            history_path: paths.history.clone(),
            settings_open: false,
            motor,
            suggest_rect: None,
            closed_tabs: Vec::new(),
            context_at: None,
            last_visit: String::new(),
            find_open: false,
            find_text: String::new(),
            find_total: 0,
            find_current: -1,
            find_focus: false,
            find_query: String::new(),
            zoom,
            permisos,
            permisos_open: false,
            settings_path,
            session_path,
            saved_session: None,
        }
    }

    /// Guarda las pestañas abiertas si cambiaron desde el último guardado.
    fn persist_session(&mut self) {
        let Some(path) = &self.session_path else {
            return;
        };
        // Una pestaña recién abierta aún no tiene URL útil; guardar ahora la
        // quitaría de la sesión y no volvería tras reiniciar.
        if self
            .tabs
            .iter()
            .any(|tab| !crate::estado::is_restorable(&tab.url))
        {
            return;
        }
        let tabs: Vec<crate::estado::TabState> = self
            .tabs
            .iter()
            .filter(|tab| crate::estado::is_restorable(&tab.url))
            .map(|tab| crate::estado::TabState {
                url: tab.url.clone(),
                title: tab.title.clone(),
            })
            .collect();
        let active = self
            .active
            .and_then(|id| self.tabs.iter().position(|tab| tab.id == id))
            .unwrap_or(0);
        let state = crate::estado::SessionState { tabs, active };
        if self.saved_session.as_ref() == Some(&state) {
            return;
        }
        if state.tabs.is_empty() {
            // Cerrar la última pestaña no borra la sesión: al reabrir, la
            // ventana vuelve a la página inicial en lugar de quedar vacía.
            return;
        }
        if let Err(error) = crate::estado::save(path, &state) {
            self.status = format!("no se pudo guardar la sesión: {error}");
            return;
        }
        self.saved_session = Some(state);
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        // Si llegan varios frames entre repintados, solo sirve el último: pintar
        // todos es trabajo perdido y se nota como tirones al hacer scroll.
        let mut latest_frame = None;
        while let Ok(event) = self.handle.events.try_recv() {
            match event {
                Event::Frame { tab, frame } => {
                    if Some(tab) == self.active {
                        latest_frame = Some((tab, frame));
                    }
                }
                Event::TabOpened(info) => {
                    if !self.tabs.iter().any(|tab| tab.id == info.id) {
                        self.tabs.push(info);
                    }
                }
                Event::TabClosed(id) => {
                    if let Some(tab) = self.tabs.iter().find(|tab| tab.id == id)
                        && crate::estado::is_restorable(&tab.url)
                    {
                        self.closed_tabs.push(tab.url.clone());
                        if self.closed_tabs.len() > 10 {
                            self.closed_tabs.remove(0);
                        }
                    }
                    self.tabs.retain(|tab| tab.id != id);
                    self.frames.remove(&id);
                }
                Event::Selected(active) => {
                    latest_frame = None;
                    self.selection_origin = None;
                    self.last_viewport = (Vec2::ZERO, 0.0);
                    self.active = active;
                    self.focus_page = active.is_some();
                    self.pending = None;
                    self.result = None;
                    self.elements.clear();
                    self.reset_find();
                    match active.and_then(|id| self.tabs.iter().find(|tab| tab.id == id)) {
                        Some(tab) => {
                            self.address = tab.url.clone();
                            self.generation = tab.generation;
                            self.status = format!(
                                "{} · generación {} · {} elementos",
                                display_title(tab),
                                tab.generation,
                                tab.elements
                            );
                        }
                        None => {
                            self.address.clear();
                            self.status = "sin pestañas · pulsa + para abrir una".to_string();
                            self.receipts = Receipts::default();
                        }
                    }
                }
                Event::Snapshot {
                    tab,
                    info,
                    elements,
                } => {
                    if Some(tab) == self.active && self.find_open {
                        self.reset_find();
                    }
                    if Some(tab) == self.active {
                        self.apply_stored_permissions(&info.url);
                        if info.url != self.last_visit && crate::estado::is_restorable(&info.url) {
                            self.last_visit = info.url.clone();
                            let visit = crate::estado::Visit {
                                url: info.url.clone(),
                                title: info.title.clone(),
                                at: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|time| time.as_secs())
                                    .unwrap_or(0),
                            };
                            if let Some(path) = &self.history_path {
                                let _ = crate::estado::append_history(path, &visit);
                            }
                            self.history.insert(0, visit);
                            self.history.truncate(500);
                        }
                    }
                    if let Some(known) = self.tabs.iter_mut().find(|known| known.id == tab) {
                        known.url = info.url.clone();
                        known.title = info.title.clone();
                        known.generation = info.generation;
                        known.elements = info.elements;
                    }
                    if Some(tab) != self.active {
                        continue;
                    }
                    if self.generation != info.generation {
                        self.pending = None;
                    }
                    self.generation = info.generation;
                    self.elements = elements;
                    if !ctx.memory(|m| m.has_focus(egui::Id::unique("address"))) {
                        self.address = info.url.clone();
                    }
                    self.status = format!(
                        "{} · generación {} · {} elementos",
                        if info.title.is_empty() {
                            info.url.as_str()
                        } else {
                            info.title.as_str()
                        },
                        info.generation,
                        info.elements
                    );
                }
                Event::Act { tab, outcome } => {
                    if Some(tab) != self.active {
                        continue;
                    }
                    if let ActOutcome::ApprovalRequired { hash, card, .. } = &outcome {
                        self.pending = Some(PendingCard {
                            card: card.clone(),
                            hash: hash.clone(),
                        });
                        self.status = "esperando tu aprobación".to_string();
                    } else {
                        self.pending = None;
                        self.status = match &outcome {
                            ActOutcome::Executed { seq, .. } => {
                                format!("acción ejecutada · recibo {seq}")
                            }
                            ActOutcome::Denied { .. } => "acción bloqueada por la política".into(),
                            ActOutcome::NotExecuted { .. } => "no se envió ninguna petición".into(),
                            _ => "resultado de la acción".into(),
                        };
                        self.result = Some(outcome.render_text());
                    }
                }
                Event::Handoff { tab, report } => {
                    if Some(tab) != self.active {
                        continue;
                    }
                    self.handoff = Some(report);
                    self.agent_open = true;
                }
                Event::Receipts {
                    tab,
                    count,
                    last,
                    log,
                } => {
                    if Some(tab) == self.active {
                        self.receipts = Receipts {
                            count,
                            last,
                            log: Some(log),
                        };
                    }
                }
                Event::CopyText(tab, text) => {
                    if Some(tab) == self.active {
                        ctx.copy_text(text);
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
                        if let engine_cdp::DownloadState::Progress { received, total } =
                            download.state
                        {
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
                        engine_cdp::DownloadState::Started => {
                            format!("descargando «{}»…", download.name)
                        }
                        engine_cdp::DownloadState::Done => {
                            format!("descarga completa: {}", download.path.display())
                        }
                        engine_cdp::DownloadState::Canceled => {
                            format!("descarga cancelada: {}", download.name)
                        }
                        engine_cdp::DownloadState::Progress { .. } => self.status.clone(),
                    };
                }
                Event::Notice(notice) => self.status = notice,
                Event::Error(error) => self.status = format!("error: {error}"),
            }
        }

        if let Some((tab, frame)) = latest_frame {
            self.apply_frame(ctx, tab, frame);
        }
        self.persist_session();
    }

    fn apply_frame(&mut self, ctx: &egui::Context, tab: TabId, frame: crate::engine::Frame) {
        let image =
            egui::ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
        let css = Vec2::new(frame.css_width as f32, frame.css_height as f32);
        match self.frames.get_mut(&tab) {
            Some(existing) => {
                if existing.texture.size() == image.size {
                    existing
                        .texture
                        .set_partial([0, 0], image, texture_options());
                } else {
                    existing.texture.set(image, texture_options());
                }
                existing.css = css;
            }
            None => {
                let texture = ctx.load_texture(format!("página-{tab}"), image, texture_options());
                self.frames.insert(tab, TabFrame { texture, css });
            }
        }
    }

    fn navigate(&mut self) {
        let url = browser_url_with(&self.address, &self.motor);
        self.address = url.clone();
        self.status = format!("navegando a {url}…");
        self.handle.navigate(url);
        self.focus_page = true;
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let tabs = self.tabs.clone();
        let mut select = None;
        let mut close = None;
        let mut open = false;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for tab in &tabs {
                let selected = Some(tab.id) == self.active;
                let fill = if selected { TAB_ACTIVE } else { TAB_IDLE };
                let text_color = if selected { Color32::WHITE } else { MUTED };
                let tab_rect = egui::Frame::new()
                    .fill(fill)
                    .corner_radius(egui::CornerRadius {
                        nw: 8,
                        ne: 8,
                        sw: 0,
                        se: 0,
                    })
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (dot, _) =
                                ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
                            ui.painter().circle_filled(
                                dot.center(),
                                3.2,
                                if selected {
                                    ACCENT
                                } else {
                                    Color32::from_gray(105)
                                },
                            );
                            let title = truncate(&display_title(tab), 26);
                            let label = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(title).size(12.5).color(text_color),
                                )
                                .sense(Sense::click()),
                            );
                            if label.clicked() {
                                select = Some(tab.id);
                            }
                            if selected {
                                let x = ui.add(
                                    egui::Label::new(
                                        egui::RichText::new("×").size(13.0).color(MUTED),
                                    )
                                    .sense(Sense::click()),
                                );
                                if x.clicked() {
                                    close = Some(tab.id);
                                }
                            }
                        });
                    })
                    .response
                    .rect;
                if selected {
                    ui.painter().line_segment(
                        [
                            egui::pos2(tab_rect.left() + 7.0, tab_rect.bottom()),
                            egui::pos2(tab_rect.right() - 7.0, tab_rect.bottom()),
                        ],
                        egui::Stroke::new(2.0, ACCENT),
                    );
                }
            }
            if ui
                .add(
                    egui::Button::new(egui::RichText::new("+").size(16.0).color(MUTED))
                        .frame(false),
                )
                .on_hover_text("Nueva pestaña")
                .clicked()
            {
                open = true;
            }
        });

        if let Some(id) = select {
            self.handle.select_tab(id);
        }
        if let Some(id) = close {
            self.handle.close_tab(id);
        }
        if open {
            self.status = "abriendo pestaña…".to_string();
            self.handle.new_tab(None);
        }
    }

    fn open_external(&mut self, url: &str) {
        self.status = match webbrowser::open(url) {
            Ok(()) => "Página enviada al navegador del sistema".into(),
            Err(error) => format!("error al abrir el navegador del sistema: {error}"),
        };
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let mut back = false;
        let mut forward = false;
        let mut reload = false;
        let mut submit = false;

        ui.horizontal(|ui| {
            back = nav_button(ui, NavIcon::Back, "Atrás");
            forward = nav_button(ui, NavIcon::Forward, "Adelante");
            reload = nav_button(ui, NavIcon::Reload, "Recargar");
            ui.add_space(4.0);

            let omni_width = (ui.available_width() - 175.0).max(80.0);
            egui::Frame::new()
                .fill(OMNI)
                .corner_radius(15.0)
                .inner_margin(egui::Margin::symmetric(12, 5))
                .show(ui, |ui| {
                    ui.set_width((omni_width - 26.0).max(120.0));
                    ui.horizontal(|ui| {
                        let current_url = self
                            .tabs
                            .iter()
                            .find(|tab| Some(tab.id) == self.active)
                            .map_or("", |tab| tab.url.as_str());
                        security_glyph(ui, current_url);
                        let focus =
                            ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::L));
                        let id = egui::Id::unique("address");
                        if focus {
                            ui.memory_mut(|m| m.request_focus(id));
                        }
                        let mut output = egui::TextEdit::singleline(&mut self.address)
                            .frame(egui::Frame::NONE)
                            .desired_width(f32::INFINITY)
                            .id(id)
                            .hint_text("Busca o escribe una dirección")
                            .show(ui);
                        if focus {
                            output.state.cursor.set_char_range(Some(
                                egui::text::CCursorRange::select_all(&output.galley),
                            ));
                            output.state.store(ui.ctx(), id);
                        }
                        let field = output.response;
                        if field.has_focus() {
                            self.suggest_rect = Some(field.rect);
                        }
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            submit = true;
                        }
                    });
                });

            let external_url = self
                .tabs
                .iter()
                .find(|tab| Some(tab.id) == self.active)
                .map(|tab| tab.url.clone())
                .filter(|url| url.starts_with("https://") || url.starts_with("http://"));
            ui.menu_button("☰", |ui| {
                let open = external_url.is_some();
                if ui
                    .add_enabled(open, egui::Button::new("Abrir fuera"))
                    .on_hover_text(
                        "Abrir esta página en el navegador del sistema (sesión independiente)",
                    )
                    .clicked()
                    && let Some(url) = external_url.clone()
                {
                    self.open_external(&url);
                    ui.close();
                }
                ui.separator();
                if ui.button("Marcadores").clicked() {
                    self.bookmarks_open = true;
                    ui.close();
                }
                if ui.button("Historial").clicked() {
                    self.history_open = true;
                    ui.close();
                }
                if ui
                    .button(format!("Descargas ({})", self.downloads.len()))
                    .clicked()
                {
                    self.downloads_open = true;
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.active.is_some(),
                        egui::Button::new("Permisos de este sitio"),
                    )
                    .clicked()
                {
                    self.permisos_open = true;
                    ui.close();
                }
            });
            if ui
                .selectable_label(self.agent_open, egui::RichText::new("Agente").size(12.0))
                .on_hover_text("Panel del agente")
                .clicked()
            {
                self.agent_open = !self.agent_open;
            }
        });

        if back {
            self.handle.back();
        }
        if forward {
            self.handle.forward();
        }
        if reload {
            self.handle.reload();
        }
        if submit {
            self.navigate();
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let color = if self.status.starts_with("error") {
                DANGER
            } else {
                MUTED
            };
            ui.add(
                egui::Label::new(egui::RichText::new(&self.status).size(11.5).color(color))
                    .truncate(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} refs · generación {}",
                        self.elements.len(),
                        self.generation
                    ))
                    .size(11.0)
                    .color(MUTED),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(if self.agent_control {
                        "control: agente"
                    } else {
                        "control: persona"
                    })
                    .size(11.0)
                    .color(if self.agent_control { ACCENT } else { MUTED }),
                );
            });
        });
    }

    fn agent_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("Agente").size(17.0));
            ui.label(
                egui::RichText::new(if self.agent_control {
                    "conectado (MCP)"
                } else {
                    "en espera"
                })
                .size(11.0)
                .color(if self.agent_control { ACCENT } else { MUTED }),
            );
        });
        ui.separator();

        if ui
            .add_sized(
                [ui.available_width(), 28.0],
                egui::Button::new(egui::RichText::new(if self.agent_control {
                    "Recuperar control"
                } else {
                    "Ceder control"
                })),
            )
            .clicked()
        {
            if self.agent_control {
                self.agent_control = false;
                self.status = "control recuperado por la persona".to_string();
            } else {
                self.agent_control = true;
                self.pending = None;
                self.status = "control cedido al agente".to_string();
                self.handle.handoff();
            }
        }
        if self.agent_control {
            ui.label(
                egui::RichText::new(
                    "La shell no acepta clicks mientras el agente conduce. Sus acciones pasan \
                     por MCP, su tarjeta y su recibo.",
                )
                .size(11.5)
                .color(MUTED),
            );
        }

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Recibos").strong().size(12.0));
        ui.label(
            egui::RichText::new(format!(
                "{} · último {}",
                self.receipts.count,
                self.receipts.last.as_deref().map(short_hash).unwrap_or("—")
            ))
            .size(11.5),
        );
        if let Some(log) = &self.receipts.log {
            ui.label(
                egui::RichText::new(log.display().to_string())
                    .monospace()
                    .size(9.5)
                    .color(MUTED),
            );
        }

        ui.add_space(6.0);
        ui.label(egui::RichText::new("Refs vivas").strong().size(12.0));
        ui.label(
            egui::RichText::new(format!(
                "{} elementos · generación {}",
                self.elements.len(),
                self.generation
            ))
            .size(11.0)
            .color(MUTED),
        );
        if self.elements.is_empty() {
            ui.label(
                egui::RichText::new("sin snapshot vivo")
                    .color(MUTED)
                    .size(11.0),
            );
        } else {
            egui::ScrollArea::vertical()
                .max_height(170.0)
                .id_salt("refs")
                .show(ui, |ui| {
                    for element in self.elements.iter().take(24) {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} {:<9} {}",
                                element.reference, element.role, element.name
                            ))
                            .monospace()
                            .size(10.5),
                        );
                    }
                });
        }

        ui.separator();
        ui.label(egui::RichText::new("Handoff").strong().size(12.0));
        match &self.handoff {
            Some(report) => {
                egui::ScrollArea::vertical()
                    .id_salt("handoff")
                    .max_height(200.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(report.render_text())
                                .monospace()
                                .size(10.5),
                        );
                    });
            }
            None => {
                ui.label(
                    egui::RichText::new(
                        "Cede el control para generar el informe que recibe el agente.",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
            }
        }
    }

    fn pending_window(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending.clone() else {
            return;
        };
        let mut approve = false;
        let mut deny = false;

        egui::Window::new(pending.card.title.clone())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                for row in &pending.card.rows {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&row.label).strong().size(12.0));
                        if row.emphasis {
                            ui.label(egui::RichText::new(&row.value).color(DANGER).strong());
                        } else {
                            ui.label(&row.value);
                        }
                    });
                }
                for warning in &pending.card.warnings {
                    ui.label(egui::RichText::new(format!("⚠ {warning}")).color(DANGER));
                }
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("hash {}", short_hash(&pending.hash)))
                        .monospace()
                        .size(10.0)
                        .color(MUTED),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Denegar").clicked() {
                        deny = true;
                    }
                    if pending.card.approve_hash.is_some() {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Aprobar").color(Color32::WHITE),
                                )
                                .fill(ACCENT),
                            )
                            .clicked()
                        {
                            approve = true;
                        }
                    }
                });
            });

        if deny {
            self.handle.deny();
            self.pending = None;
        }
        if approve {
            self.handle.approve(pending.hash.clone());
            self.pending = None;
        }
    }

    fn result_window(&mut self, ctx: &egui::Context) {
        let Some(text) = self.result.clone() else {
            return;
        };
        egui::Window::new("Resultado")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(520.0);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(&text).monospace().size(11.0));
                    });
                ui.add_space(6.0);
                if ui.button("Cerrar").clicked() {
                    self.result = None;
                }
            });
    }

    /// Buscador flotante. No altera el contenido: resalta por CSS y lleva la
    /// coincidencia actual a la vista.
    fn downloads_window(&mut self, ctx: &egui::Context) {
        if !self.downloads_open {
            return;
        }
        let mut open = self.downloads_open;
        egui::Window::new("Descargas")
            .id(egui::Id::unique("ventana-descargas"))
            .open(&mut open)
            .anchor(egui::Align2::RIGHT_BOTTOM, Vec2::new(-24.0, -48.0))
            .default_width(420.0)
            .show(ctx, |ui| {
                if self.downloads.is_empty() {
                    ui.label("Todavía no hay descargas en esta sesión.");
                    return;
                }
                for item in &self.downloads {
                    ui.horizontal(|ui| {
                        let detail = match item.state {
                            engine_cdp::DownloadState::Done => "completada".to_string(),
                            engine_cdp::DownloadState::Canceled => "cancelada".to_string(),
                            engine_cdp::DownloadState::Started => "empezando…".to_string(),
                            engine_cdp::DownloadState::Progress { received, total } => {
                                if total > 0 {
                                    format!("{} %", (received * 100 / total).min(100))
                                } else {
                                    "descargando…".to_string()
                                }
                            }
                        };
                        ui.label(egui::RichText::new(&item.name).size(12.0));
                        ui.label(egui::RichText::new(detail).size(11.0).color(MUTED));
                        if item.state == engine_cdp::DownloadState::Done
                            && ui.button("Abrir").clicked()
                        {
                            let _ = webbrowser::open(&item.path.to_string_lossy());
                        }
                        if item.state == engine_cdp::DownloadState::Done
                            && ui.button("Carpeta").clicked()
                            && let Some(dir) = item.path.parent()
                        {
                            let _ = webbrowser::open(&dir.to_string_lossy());
                        }
                    });
                }
            });
        self.downloads_open = open;
    }

    fn permisos_window(&mut self, ctx: &egui::Context) {
        if !self.permisos_open {
            return;
        }
        let Some(origin) = self
            .active_url()
            .and_then(|url| crate::estado::zoom_key(&url))
        else {
            self.permisos_open = false;
            return;
        };
        let mut permisos = self.permisos.get(&origin).copied().unwrap_or_default();
        let before = permisos;
        let mut open = self.permisos_open;
        egui::Window::new("Permisos del sitio")
            .id(egui::Id::unique("ventana-permisos"))
            .open(&mut open)
            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-18.0, 58.0))
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(&origin).size(12.0).color(MUTED));
                ui.add_space(4.0);
                ui.checkbox(&mut permisos.notificaciones, "Notificaciones");
                ui.checkbox(&mut permisos.ubicacion, "Ubicación");
                ui.checkbox(&mut permisos.camara, "Cámara");
                ui.checkbox(&mut permisos.microfono, "Micrófono");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Lo desmarcado se deniega: el sitio no puede pedirlo sin avisarte.",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
            });
        self.permisos_open = open;
        if permisos != before {
            self.permisos.insert(origin.clone(), permisos);
            self.handle
                .permissions(origin.clone(), permisos.granted(), permisos.denied());
            self.save_settings();
        }
    }

    fn apply_stored_permissions(&self, url: &str) {
        let Some(origin) = crate::estado::zoom_key(url) else {
            return;
        };
        if let Some(permisos) = self.permisos.get(&origin) {
            self.handle
                .permissions(origin.clone(), permisos.granted(), permisos.denied());
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = self.settings_open;
        let mut motor = self.motor.clone();
        egui::Window::new("Ajustes")
            .id(egui::Id::unique("ventana-ajustes"))
            .open(&mut open)
            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-18.0, 58.0))
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Motor de búsqueda");
                    egui::ComboBox::from_id_salt("motor")
                        .selected_text(match motor.as_str() {
                            "google" => "Google",
                            _ => "DuckDuckGo",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut motor, "duckduckgo".into(), "DuckDuckGo");
                            ui.selectable_value(&mut motor, "google".into(), "Google");
                        });
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Descargas").strong());
                match crate::estado::downloads_dir() {
                    Some(dir) => {
                        ui.label(egui::RichText::new(dir.display().to_string()).size(11.0).color(MUTED));
                        if ui.button("Abrir carpeta de descargas").clicked() {
                            let _ = webbrowser::open(&dir.to_string_lossy());
                        }
                    }
                    None => {
                        ui.label("Sin carpeta de descargas configurada.");
                    }
                }
                ui.add_space(6.0);
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("Navegador {}", env!("CARGO_PKG_VERSION")))
                        .size(11.0)
                        .color(MUTED),
                );
                ui.label(
                    egui::RichText::new(
                        "Perfil y sesión se guardan en ~/.local/share/navegador y ~/.local/state/navegador.",
                    )
                    .size(11.0)
                    .color(MUTED),
                );
            });
        self.settings_open = open;
        if motor != self.motor {
            self.motor = motor;
            self.save_settings();
            self.status = format!("motor de búsqueda: {}", self.motor);
        }
    }

    fn suggestions(&self) -> Vec<(String, String)> {
        let needle = self.address.trim().to_lowercase();
        if needle.is_empty() || needle.starts_with("about:") {
            return Vec::new();
        }
        let current = self.active_url().unwrap_or_default().to_lowercase();
        if needle == current {
            return Vec::new();
        }
        let mut out: Vec<(String, String)> = Vec::new();
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

    fn suggestion_bar(&mut self, ctx: &egui::Context) {
        let focused = ctx.memory(|memory| memory.has_focus(egui::Id::unique("address")));
        let Some(rect) = self.suggest_rect else {
            return;
        };
        if !focused {
            return;
        }
        let suggestions = self.suggestions();
        if suggestions.is_empty() {
            return;
        }
        let mut chosen = None;
        egui::Area::new(egui::Id::unique("sugerencias"))
            .fixed_pos(egui::pos2(rect.left(), rect.bottom() + 4.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(rect.width().max(320.0));
                    for (url, title) in &suggestions {
                        let name = if title.is_empty() { url } else { title };
                        let label = egui::RichText::new(truncate(name, 60)).size(12.0);
                        let response = ui.add(egui::Button::new(label).frame(false));
                        if response.clicked() {
                            chosen = Some(url.clone());
                        }
                        ui.label(
                            egui::RichText::new(truncate(url, 70))
                                .size(10.5)
                                .color(MUTED),
                        );
                    }
                });
            });
        if let Some(url) = chosen {
            self.address = url.clone();
            self.handle.navigate(url);
            self.focus_page = true;
            self.suggest_rect = None;
        }
    }

    fn find_bar(&mut self, ctx: &egui::Context) {
        if !self.find_open {
            return;
        }
        let mut close = false;
        egui::Window::new("buscar")
            .id(egui::Id::unique("barra-buscar"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-18.0, 58.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.find_text)
                            .id(egui::Id::unique("buscar-texto"))
                            .hint_text("Buscar en la página")
                            .desired_width(200.0),
                    );
                    if self.find_focus {
                        response.request_focus();
                        self.find_focus = false;
                    }
                    if self.find_text != self.find_query {
                        self.find_query = self.find_text.clone();
                        self.find_total = 0;
                        self.find_current = -1;
                        let text = self.find_text.clone();
                        self.handle.find(text, 0);
                    }
                    let counter = if self.find_text.is_empty() {
                        String::new()
                    } else if self.find_total == 0 {
                        "0".to_string()
                    } else {
                        format!("{} de {}", self.find_current + 1, self.find_total)
                    };
                    ui.label(egui::RichText::new(counter).color(MUTED).size(12.0));
                    if ui.button("Anterior").clicked() {
                        self.step_find(false);
                    }
                    if ui.button("Siguiente").clicked() {
                        self.step_find(true);
                    }
                    if ui.button("×").on_hover_text("Cerrar (Esc)").clicked() {
                        close = true;
                    }
                });
            });

        let enter =
            ctx.input(|input| input.key_pressed(egui::Key::Enter) && !input.modifiers.shift);
        let enter_back =
            ctx.input(|input| input.key_pressed(egui::Key::Enter) && input.modifiers.shift);
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            close = true;
        }
        if enter {
            self.step_find(true);
        } else if enter_back {
            self.step_find(false);
        }
        if close {
            self.close_find();
        }
    }

    fn step_find(&mut self, forward: bool) {
        if self.find_text.is_empty() {
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

    fn zoom_for(&self, url: &str) -> f32 {
        crate::estado::zoom_key(url)
            .and_then(|origin| self.zoom.get(&origin).copied())
            .unwrap_or(1.0)
    }

    fn adjust_zoom(&mut self, delta: f32) {
        let Some(url) = self.active_url() else {
            return;
        };
        let current = self.zoom_for(&url);
        self.apply_zoom(&url, current + delta);
    }

    fn set_zoom(&mut self, value: f32) {
        let Some(url) = self.active_url() else {
            return;
        };
        self.apply_zoom(&url, value);
    }

    fn apply_zoom(&mut self, url: &str, value: f32) {
        let Some(origin) = crate::estado::zoom_key(url) else {
            self.status = "esta página no admite zoom".to_string();
            return;
        };
        let zoom = (value * 10.0).round().clamp(5.0, 30.0) / 10.0;
        self.zoom.insert(origin, zoom);
        // Fuerza un viewport nuevo con el zoom aplicado.
        self.last_viewport = (Vec2::ZERO, 0.0);
        self.status = format!("zoom {}%", (zoom * 100.0).round() as i32);
        self.save_settings();
    }

    fn toggle_bookmark(&mut self, url: String) {
        if let Some(index) = self
            .bookmarks
            .iter()
            .position(|bookmark| bookmark.url == url)
        {
            self.bookmarks.remove(index);
            self.status = "marcador eliminado".to_string();
        } else {
            let title = self
                .tabs
                .iter()
                .find(|tab| tab.url == url)
                .map(|tab| tab.title.clone())
                .unwrap_or_default();
            self.bookmarks
                .insert(0, crate::estado::Bookmark { url, title });
            self.status = "marcador guardado (Ctrl+D para quitarlo)".to_string();
        }
        self.save_settings();
    }

    fn bookmarks_window(&mut self, ctx: &egui::Context) {
        if !self.bookmarks_open {
            return;
        }
        let mut open = self.bookmarks_open;
        let mut navigate = None;
        let mut remove = None;
        egui::Window::new("Marcadores")
            .id(egui::Id::unique("ventana-marcadores"))
            .open(&mut open)
            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-18.0, 58.0))
            .default_width(420.0)
            .show(ctx, |ui| {
                if self.bookmarks.is_empty() {
                    ui.label("Sin marcadores. Pulsa Ctrl+D en una página para guardarla.");
                    return;
                }
                for (index, bookmark) in self.bookmarks.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let name = if bookmark.title.is_empty() {
                            bookmark.url.as_str()
                        } else {
                            bookmark.title.as_str()
                        };
                        if ui.link(truncate(name, 48)).clicked() {
                            navigate = Some(bookmark.url.clone());
                        }
                        ui.label(
                            egui::RichText::new(truncate(&bookmark.url, 40))
                                .size(10.5)
                                .color(MUTED),
                        );
                        if ui.small_button("Quitar").clicked() {
                            remove = Some(index);
                        }
                    });
                }
            });
        self.bookmarks_open = open;
        if let Some(index) = remove {
            self.bookmarks.remove(index);
            self.save_settings();
        }
        if let Some(url) = navigate {
            self.handle.navigate(url);
            self.bookmarks_open = false;
        }
    }

    fn history_window(&mut self, ctx: &egui::Context) {
        if !self.history_open {
            return;
        }
        let query = self.history_query.to_lowercase();
        let rows: Vec<(String, String, String)> = self
            .history
            .iter()
            .filter(|visit| {
                query.is_empty()
                    || visit.url.to_lowercase().contains(&query)
                    || visit.title.to_lowercase().contains(&query)
            })
            .take(200)
            .map(|visit| (visit.url.clone(), visit.title.clone(), visit.at.to_string()))
            .collect();
        let mut open = self.history_open;
        let mut navigate = None;
        let mut clear = false;
        egui::Window::new("Historial")
            .id(egui::Id::unique("ventana-historial"))
            .open(&mut open)
            .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-18.0, 58.0))
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.history_query)
                            .hint_text("Filtrar historial")
                            .desired_width(260.0),
                    );
                    if ui.button("Borrar historial").clicked() {
                        clear = true;
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        if rows.is_empty() {
                            ui.label("Nada por aquí.");
                        }
                        for (url, title, at) in &rows {
                            let name = if title.is_empty() { url } else { title };
                            ui.horizontal(|ui| {
                                if ui.link(truncate(name, 52)).on_hover_text(url).clicked() {
                                    navigate = Some(url.clone());
                                }
                                ui.label(
                                    egui::RichText::new(truncate(url, 44))
                                        .size(10.5)
                                        .color(MUTED),
                                );
                                let _ = at;
                            });
                        }
                    });
            });
        self.history_open = open;
        if clear {
            if let Some(path) = &self.history_path {
                let _ = crate::estado::clear_history(path);
            }
            self.history.clear();
            self.last_visit.clear();
            self.status = "historial borrado".to_string();
        }
        if let Some(url) = navigate {
            self.handle.navigate(url);
            self.history_open = false;
        }
    }

    fn save_settings(&mut self) {
        let Some(path) = self.settings_path.clone() else {
            return;
        };
        let settings = crate::estado::Settings {
            zoom: self.zoom.clone(),
            permisos: self.permisos.clone(),
            marcadores: self.bookmarks.clone(),
            motor: self.motor.clone(),
        };
        if let Err(error) = crate::estado::save_settings(&path, &settings) {
            self.status = format!("no se pudieron guardar los ajustes: {error}");
        }
    }

    fn active_url(&self) -> Option<String> {
        let id = self.active?;
        self.tabs
            .iter()
            .find(|tab| tab.id == id)
            .map(|tab| tab.url.clone())
    }

    fn reset_find(&mut self) {
        self.find_open = false;
        self.find_total = 0;
        self.find_current = -1;
        self.find_query.clear();
    }

    fn close_find(&mut self) {
        self.reset_find();
        self.handle.find(String::new(), 0);
        self.status = "listo".to_string();
    }

    fn page(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.active else {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("sin pestañas: pulsa + para abrir una").color(MUTED));
            });
            return;
        };

        let current_url = self
            .tabs
            .iter()
            .find(|info| info.id == tab)
            .map(|info| info.url.clone())
            .unwrap_or_default();
        if current_url == "about:blank" {
            ui.add_space(ui.available_height() * 0.22);
            ui.vertical_centered(|ui| {
                ui.heading("Nueva pestaña");
                ui.add_space(10.0);
                if ui.button("Buscar o escribir una dirección").clicked() {
                    self.address.clear();
                    ui.memory_mut(|m| m.request_focus(egui::Id::unique("address")));
                }
            });
            return;
        }
        if let Some(query) = google_verification(&current_url) {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.strong("Google solicita una verificación");
                ui.label(
                    "Completa la verificación en esta página o elige dónde continuar la búsqueda.",
                );
                ui.horizontal_wrapped(|ui| {
                    let google = match &query {
                        Some(text) => {
                            format!("https://www.google.com/search?q={}", encode_query(text))
                        }
                        None => "https://www.google.com/".into(),
                    };
                    if ui.button("Abrir Google en el sistema").clicked() {
                        self.open_external(&google);
                    }
                    if let Some(text) = query
                        && ui.button("Buscar con DuckDuckGo").clicked()
                    {
                        self.handle
                            .navigate(format!("https://duckduckgo.com/?q={}", encode_query(&text)));
                    }
                });
            });
        }

        let available = ui.available_size();
        let target = egui::vec2(available.x.max(320.0), available.y.max(240.0));
        // El zoom reduce los píxeles CSS visibles y sube la densidad: la página
        // se ve más grande sin perder nitidez.
        let zoom = self.zoom_for(&current_url).max(0.5);
        let css_target = egui::vec2((target.x / zoom).max(320.0), (target.y / zoom).max(240.0));
        // Nitidez sin reventar memoria: si el viewport a escala 2 supera el
        // presupuesto de píxeles, bajamos el deviceScaleFactor. Cada frame
        // RGBA cuesta width*height*4 y se nota en la fluidez.
        let mut scale = ui.ctx().pixels_per_point().clamp(1.0, 3.0) * zoom;
        const BUDGET: f32 = 8_300_000.0;
        if css_target.x * css_target.y * scale * scale > BUDGET {
            scale = (BUDGET / (css_target.x * css_target.y))
                .sqrt()
                .clamp(1.0, scale);
        }
        if (css_target - self.last_viewport.0).length() > 2.0
            || (scale - self.last_viewport.1).abs() > 0.01
        {
            self.last_viewport = (css_target, scale);
            self.handle.viewport(
                css_target.x.round() as u32,
                css_target.y.round() as u32,
                scale as f64,
            );
        }

        let Some(frame) = self.frames.get(&tab) else {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("cargando la página…").color(MUTED));
            });
            return;
        };

        let css = frame.css;
        let aspect = (css.x / css.y).max(0.1);
        let mut size = target;
        if size.x / size.y > aspect {
            size.x = size.y * aspect;
        } else {
            size.y = size.x / aspect;
        }

        let texture = frame.texture.id();
        ui.vertical_centered(|ui| {
            let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
            // Tras navegar, el teclado va a la página. Si nada tiene el foco,
            // también: así el teclado nunca se queda sin destinatario.
            if self.focus_page || ui.ctx().memory(|memory| memory.focused().is_none()) {
                response.request_focus();
                self.focus_page = false;
            }
            ui.painter()
                .rect_filled(rect.expand(1.0), 0.0, Color32::from_gray(46));
            ui.painter().image(
                texture,
                rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );

            if response.hovered() {
                let scroll = ui.input(|input| input.smooth_scroll_delta.y);
                if scroll != 0.0
                    && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
                {
                    let x = ((pointer.x - rect.min.x) / rect.width()) as f64 * css.x as f64;
                    let y = ((pointer.y - rect.min.y) / rect.height()) as f64 * css.y as f64;
                    self.handle.wheel(x, y, -(scroll as f64));
                }
            }

            let primary = egui::PointerButton::Primary;
            if response.drag_started_by(primary) {
                self.selection_origin = ui.input(|i| i.pointer.press_origin());
            }
            let released = response.drag_stopped_by(primary);
            if (released || response.dragged_by(primary))
                && !self.agent_control
                && self.pending.is_none()
                && let Some(pointer) = response.interact_pointer_pos()
                && let Some(origin) = self.selection_origin
                && (released || response.drag_started() || response.drag_delta() != Vec2::ZERO)
            {
                response.request_focus();
                let point = |p: egui::Pos2| {
                    let pos = rect.clamp(p) - rect.min;
                    [
                        (pos.x / rect.width() * css.x) as f64,
                        (pos.y / rect.height() * css.y) as f64,
                    ]
                };
                self.handle.select_text(tab, point(origin), point(pointer));
            }
            if released {
                self.selection_origin = None;
            }
            if response.clicked()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                response.request_focus();
                if self.pending.is_some() {
                    self.status = "resuelve la tarjeta antes de seguir actuando".into();
                } else if self.agent_control {
                    self.status =
                        "el agente tiene el control: recupera el control para actuar".into();
                } else {
                    let x = ((pointer.x - rect.min.x) / rect.width()) as f64 * css.x as f64;
                    let y = ((pointer.y - rect.min.y) / rect.height()) as f64 * css.y as f64;
                    self.status = "consultando el elemento bajo el puntero…".into();
                    self.handle.click(x, y);
                }
            }

            let to_css = |pointer: egui::Pos2| {
                (
                    ((pointer.x - rect.min.x) / rect.width()) as f64 * css.x as f64,
                    ((pointer.y - rect.min.y) / rect.height()) as f64 * css.y as f64,
                )
            };
            if response.secondary_clicked()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                self.context_at = Some(to_css(pointer));
            }
            if response.middle_clicked()
                && !self.agent_control
                && self.pending.is_none()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let (x, y) = to_css(pointer);
                self.handle.open_link_new_tab(x, y, true);
            }
            let context_at = self.context_at;
            response.context_menu(|ui| {
                let Some((x, y)) = context_at else {
                    return;
                };
                if ui.button("Abrir enlace en pestaña nueva").clicked() {
                    self.handle.open_link_new_tab(x, y, false);
                    ui.close();
                }
                if ui.button("Abrir enlace en segundo plano").clicked() {
                    self.handle.open_link_new_tab(x, y, true);
                    ui.close();
                }
                if ui.button("Copiar dirección del enlace").clicked() {
                    self.handle.copy_link(x, y);
                    ui.close();
                }
                ui.separator();
                if ui.button("Atrás").clicked() {
                    self.handle.back();
                    ui.close();
                }
                if ui.button("Adelante").clicked() {
                    self.handle.forward();
                    ui.close();
                }
                if ui.button("Recargar").clicked() {
                    self.handle.reload();
                    ui.close();
                }
                ui.separator();
                if ui.button("Copiar selección").clicked() {
                    if let Some(tab) = self.active {
                        self.handle.copy_text(tab);
                    }
                    ui.close();
                }
            });

            if response.has_focus() {
                // La página es dueña del teclado: Tab, flechas y Escape no
                // deben mover el foco dentro de la ventana.
                ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(
                        response.id,
                        egui::EventFilter {
                            tab: true,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: true,
                        },
                    );
                });
                // Tab y Shift+Tab van a la página (campos del formulario) en vez
                // de mover el foco dentro de la ventana.
                let next = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
                let previous =
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab));
                if next || previous {
                    self.handle
                        .key("Tab", "Tab", 9, if previous { 8 } else { 0 });
                }
                self.forward_keyboard(ui);
            }
        });
    }

    /// Reenvía lo tecleado a la página. El texto va como `insertText` (respeta
    /// la distribución del teclado) y las teclas especiales como keyDown/Up.
    fn forward_keyboard(&mut self, ui: &egui::Ui) {
        if self.pending.is_some() || self.agent_control {
            return;
        }
        let events = ui.input(|input| input.events.clone());
        let physical_space = events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Space,
                    pressed: true,
                    ..
                }
            )
        });
        for event in events {
            match event {
                egui::Event::Text(text) if physical_space && text == " " => continue,
                egui::Event::Copy => {
                    if let Some(tab) = self.active {
                        self.handle.copy_text(tab);
                    }
                }
                egui::Event::Text(text) | egui::Event::Paste(text) => {
                    if !text.is_empty() {
                        self.handle.insert_text(text);
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if let Some((name, code, virtual_key)) = special_key(key) {
                        self.handle
                            .key(name, code, virtual_key, cdp_modifiers(modifiers));
                    }
                }
                _ => {}
            }
        }
    }
}

impl eframe::App for ShellApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events(ui.ctx());
        let focused = ui.ctx().input(|input| input.focused);
        match (self.window_focused, focused) {
            // Solo se pausa tras haber estado enfocada: al arrancar sin foco
            // (Xvfb, un escritorio que tarda) la página debe seguir pintando.
            (Some(true), false) => {
                self.window_focused = Some(false);
                self.handle.screencast(false);
            }
            (Some(false), true) => {
                self.window_focused = Some(true);
                self.handle.screencast(true);
            }
            (None, _) => self.window_focused = Some(focused),
            _ => {}
        }
        let shortcut = |key| ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key));
        if shortcut(egui::Key::T) {
            self.handle.new_tab(None);
        }
        if shortcut(egui::Key::W)
            && let Some(id) = self.active
        {
            self.handle.close_tab(id);
        }
        if shortcut(egui::Key::R) {
            self.handle.reload();
        }
        if shortcut(egui::Key::D)
            && let Some(url) = self.active_url()
        {
            self.toggle_bookmark(url);
        }
        if shortcut(egui::Key::H) {
            self.history_open = true;
        }
        if ui.input_mut(|input| {
            input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::T,
            )
        }) && let Some(url) = self.closed_tabs.pop()
        {
            self.handle.new_tab(Some(url));
        }
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
            self.settings_open = true;
        }
        if ui.input_mut(|input| {
            input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::B,
            )
        }) {
            self.bookmarks_open = true;
        }
        if shortcut(egui::Key::Q) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if shortcut(egui::Key::F) {
            self.find_open = true;
            self.find_focus = true;
        }
        if shortcut(egui::Key::Plus) || shortcut(egui::Key::Equals) {
            self.adjust_zoom(0.1);
        }
        if shortcut(egui::Key::Minus) {
            self.adjust_zoom(-0.1);
        }
        if shortcut(egui::Key::Num0) {
            self.set_zoom(1.0);
        }
        self.find_bar(ui.ctx());
        self.downloads_window(ui.ctx());
        self.permisos_window(ui.ctx());
        self.bookmarks_window(ui.ctx());
        self.history_window(ui.ctx());
        self.settings_window(ui.ctx());
        self.suggestion_bar(ui.ctx());

        egui::Panel::top("chrome")
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(8, 5)),
            )
            .show(ui, |ui| {
                self.tab_strip(ui);
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                self.toolbar(ui);
            });

        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| self.status_bar(ui));

        if self.agent_open {
            egui::Panel::right("agente")
                .resizable(true)
                .default_size(360.0)
                .show(ui, |ui| self.agent_panel(ui));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(PAGE_BG))
            .show(ui, |ui| self.page(ui));

        self.pending_window(ui.ctx());
        self.result_window(ui.ctx());
    }
}

/// Candado para HTTPS, globo para el resto: pista visual de navegador sin
/// depender de glifos que la fuente no tenga.
fn security_glyph(ui: &mut egui::Ui, url: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
    let center = rect.center();
    let stroke = egui::Stroke::new(1.1, MUTED);
    if url.starts_with("https://") {
        ui.painter().rect_stroke(
            Rect::from_center_size(center + egui::vec2(0.0, 1.6), egui::vec2(7.4, 5.4)),
            1.2,
            stroke,
            egui::StrokeKind::Inside,
        );
        let mut shackle = Vec::new();
        for step in 0..=10 {
            let angle = std::f32::consts::PI + (step as f32 / 10.0) * std::f32::consts::PI;
            shackle.push(egui::pos2(
                center.x + 2.7 * angle.cos(),
                center.y - 0.6 + 2.7 * angle.sin(),
            ));
        }
        ui.painter().add(egui::Shape::line(shackle, stroke));
    } else {
        ui.painter().circle_stroke(center, 4.2, stroke);
        ui.painter().line_segment(
            [
                egui::pos2(center.x - 4.2, center.y),
                egui::pos2(center.x + 4.2, center.y),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                egui::pos2(center.x, center.y - 4.2),
                egui::pos2(center.x, center.y + 4.2),
            ],
            stroke,
        );
    }
}

fn texture_options() -> egui::TextureOptions {
    egui::TextureOptions {
        magnification: egui::TextureFilter::Linear,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: None,
    }
}

/// Enter, borrado, flechas… El texto imprimible llega como `Event::Text` y va
/// por `insertText`; aquí no se duplica.
fn special_key(key: egui::Key) -> Option<(&'static str, &'static str, i64)> {
    use egui::Key::*;
    Some(match key {
        Tab => ("Tab", "Tab", 9),
        Space => (" ", "Space", 32),
        Enter => ("Enter", "Enter", 13),
        Backspace => ("Backspace", "Backspace", 8),
        Delete => ("Delete", "Delete", 46),
        Escape => ("Escape", "Escape", 27),
        ArrowLeft => ("ArrowLeft", "ArrowLeft", 37),
        ArrowUp => ("ArrowUp", "ArrowUp", 38),
        ArrowRight => ("ArrowRight", "ArrowRight", 39),
        ArrowDown => ("ArrowDown", "ArrowDown", 40),
        Home => ("Home", "Home", 36),
        End => ("End", "End", 35),
        PageUp => ("PageUp", "PageUp", 33),
        PageDown => ("PageDown", "PageDown", 34),
        _ => return None,
    })
}

/// Bits de modificadores de CDP: Alt=1, Ctrl=2, Meta=4, Shift=8.
fn cdp_modifiers(modifiers: egui::Modifiers) -> u32 {
    let mut bits = 0;
    if modifiers.alt {
        bits |= 1;
    }
    if modifiers.ctrl || modifiers.command {
        bits |= 2;
    }
    if modifiers.shift {
        bits |= 8;
    }
    bits
}

#[derive(Clone, Copy)]
enum NavIcon {
    Back,
    Forward,
    Reload,
}

/// Iconos dibujados a mano: la fuente por defecto de egui no trae flechas ni
/// el símbolo de recargar, y salían como cuadros vacíos.
fn nav_button(ui: &mut egui::Ui, icon: NavIcon, tip: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), Sense::click());
    let color = if response.hovered() {
        Color32::WHITE
    } else {
        Color32::from_gray(205)
    };
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 7.0, Color32::from_rgb(44, 46, 53));
    }

    let center = rect.center();
    let stroke = egui::Stroke::new(1.6, color);
    match icon {
        NavIcon::Back | NavIcon::Forward => {
            let dir = if matches!(icon, NavIcon::Back) {
                -1.0
            } else {
                1.0
            };
            let start = egui::pos2(center.x - dir * 5.5, center.y);
            let end = egui::pos2(center.x + dir * 5.5, center.y);
            ui.painter().line_segment([start, end], stroke);
            ui.painter()
                .line_segment([end, end + egui::vec2(-dir * 4.2, -4.2)], stroke);
            ui.painter()
                .line_segment([end, end + egui::vec2(-dir * 4.2, 4.2)], stroke);
        }
        NavIcon::Reload => {
            let radius = 5.5;
            let mut points = Vec::new();
            for step in 0..=20 {
                let angle = -std::f32::consts::FRAC_PI_2
                    + (step as f32 / 20.0) * std::f32::consts::TAU * 0.8;
                points.push(egui::pos2(
                    center.x + radius * angle.cos(),
                    center.y + radius * angle.sin(),
                ));
            }
            let end = *points.last().expect("el arco tiene puntos");
            ui.painter().add(egui::Shape::line(points, stroke));
            ui.painter()
                .line_segment([end, end + egui::vec2(-2.6, -3.2)], stroke);
            ui.painter()
                .line_segment([end, end + egui::vec2(2.6, -3.2)], stroke);
        }
    }

    let clicked = response.clicked();
    response.on_hover_text(tip);
    clicked
}

fn display_title(tab: &TabInfo) -> String {
    if !tab.title.is_empty() {
        tab.title.clone()
    } else if tab.url.is_empty() || tab.url == "about:blank" {
        "nueva pestaña".to_string()
    } else {
        tab.url.clone()
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max).collect::<String>())
    } else {
        text.to_string()
    }
}

fn short_hash(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

/// Lo que escribe la persona en la barra: URL si lo parece, búsqueda si no.
pub fn browser_url(input: &str) -> String {
    browser_url_with(input, "duckduckgo")
}

/// URL de búsqueda para el motor elegido en ajustes.
pub fn search_url(engine: &str, query: &str) -> String {
    match engine {
        "google" => format!("https://www.google.com/search?q={}", encode_query(query)),
        _ => format!("https://duckduckgo.com/?q={}", encode_query(query)),
    }
}

pub fn browser_url_with(input: &str, engine: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "about:blank".into();
    }
    if trimmed.starts_with("about:") || trimmed.starts_with("data:") {
        return trimmed.to_string();
    }
    let looks_like_url = trimmed.contains("://")
        || (!trimmed.contains(' ')
            && (trimmed.contains('.')
                || trimmed.starts_with("localhost")
                || trimmed.starts_with("127.0.0.1")
                || trimmed.starts_with("[::1]")));
    if looks_like_url {
        normalize_url(trimmed)
    } else {
        search_url(engine, trimmed)
    }
}

fn encode_query(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Añade esquema si falta. `localhost` e IPs van por HTTP; el resto, HTTPS.
pub fn normalize_url(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.contains("://") {
        return trimmed.to_string();
    }
    if trimmed.starts_with("localhost")
        || trimmed.starts_with("127.0.0.1")
        || trimmed.starts_with("[::1]")
    {
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::{browser_url, normalize_url};

    #[test]
    fn recupera_solo_la_consulta_de_google() {
        let url = "https://www.google.com/sorry/index?continue=https%3A%2F%2Fwww.google.com%2Fsearch%3Fq%3Dcaf%25C3%25A9%2Bfrio&q=token";
        assert_eq!(
            super::google_verification(url),
            Some(Some("café frio".into()))
        );
        assert_eq!(
            super::google_verification(
                "https://www.google.com/sorry/index?continue=https://example.org/search?q=test"
            ),
            Some(None)
        );
        assert_eq!(
            super::google_verification("https://www.google.com.example.org/sorry/index"),
            None
        );
    }

    #[test]
    fn completa_el_esquema_sin_romper_urls_explicitas() {
        assert_eq!(normalize_url("ejemplo.com"), "https://ejemplo.com");
        assert_eq!(normalize_url("127.0.0.1:8977/a"), "http://127.0.0.1:8977/a");
        assert_eq!(normalize_url("localhost:3000"), "http://localhost:3000");
        assert_eq!(normalize_url("http://x.test/a"), "http://x.test/a");
        assert_eq!(normalize_url("  https://x.test  "), "https://x.test");
    }

    #[test]
    fn el_motor_de_busqueda_se_puede_elegir() {
        assert_eq!(
            super::browser_url_with("gatos", "google"),
            "https://www.google.com/search?q=gatos"
        );
        assert_eq!(
            super::browser_url_with("gatos", "duckduckgo"),
            "https://duckduckgo.com/?q=gatos"
        );
    }

    #[test]
    fn las_palabras_sueltas_van_a_buscar() {
        assert_eq!(
            browser_url("navegador ai rust"),
            "https://duckduckgo.com/?q=navegador+ai+rust"
        );
        assert_eq!(browser_url("about:blank"), "about:blank");
        assert_eq!(browser_url("   "), "about:blank");
        assert_eq!(browser_url("ejemplo.com"), "https://ejemplo.com");
        assert_eq!(browser_url("127.0.0.1:8977"), "http://127.0.0.1:8977");
    }
}
