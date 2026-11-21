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
}

impl ShellApp {
    pub fn new(initial_url: String, log_path: PathBuf, cc: &eframe::CreationContext<'_>) -> Self {
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

        let repaint_ctx = ctx.clone();
        ShellApp {
            handle: crate::engine::spawn_with_notify(
                initial_url.clone(),
                log_path,
                Arc::new(move || repaint_ctx.request_repaint()),
            ),
            tabs: Vec::new(),
            active: None,
            address: initial_url,
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
        }
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
                    self.tabs.retain(|tab| tab.id != id);
                    self.frames.remove(&id);
                }
                Event::Selected(active) => {
                    latest_frame = None;
                    self.selection_origin = None;
                    self.last_viewport = (Vec2::ZERO, 0.0);
                    self.active = active;
                    self.pending = None;
                    self.result = None;
                    self.elements.clear();
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
                Event::Notice(notice) => self.status = notice,
                Event::Error(error) => self.status = format!("error: {error}"),
            }
        }

        if let Some((tab, frame)) = latest_frame {
            self.apply_frame(ctx, tab, frame);
        }
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
        let url = browser_url(&self.address);
        self.address = url.clone();
        self.status = format!("navegando a {url}…");
        self.handle.navigate(url);
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
                        if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            submit = true;
                        }
                    });
                });

            let external_url = self
                .tabs
                .iter()
                .find(|tab| Some(tab.id) == self.active)
                .map(|tab| tab.url.as_str())
                .filter(|url| url.starts_with("https://") || url.starts_with("http://"));
            if ui
                .add_enabled(external_url.is_some(), egui::Button::new("Abrir fuera"))
                .on_hover_text(
                    "Abrir esta página en el navegador del sistema (sesión independiente)",
                )
                .clicked()
                && let Some(url) = external_url.map(str::to_owned)
            {
                self.open_external(&url);
            }
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
            .map(|info| info.url.as_str())
            .unwrap_or("");
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
        if let Some(query) = google_verification(current_url) {
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
        // Nitidez sin reventar memoria: si el viewport a escala 2 supera el
        // presupuesto de píxeles, bajamos el deviceScaleFactor. Cada frame
        // RGBA cuesta width*height*4 y se nota en la fluidez.
        let mut scale = ui.ctx().pixels_per_point().clamp(1.0, 3.0);
        const BUDGET: f32 = 8_300_000.0;
        if target.x * target.y * scale * scale > BUDGET {
            scale = (BUDGET / (target.x * target.y)).sqrt().clamp(1.0, scale);
        }
        if (target - self.last_viewport.0).length() > 2.0
            || (scale - self.last_viewport.1).abs() > 0.01
        {
            self.last_viewport = (target, scale);
            self.handle.viewport(
                target.x.round() as u32,
                target.y.round() as u32,
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

            if response.has_focus() {
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
        format!("https://duckduckgo.com/?q={}", encode_query(trimmed))
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
