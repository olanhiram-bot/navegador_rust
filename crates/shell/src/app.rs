//! UI de la shell: pestañas, página, tarjeta y panel del agente.

use std::collections::HashMap;
use std::path::PathBuf;

use card::CardView;
use eframe::egui;
use egui::{Align2, Color32, Rect, Sense, Vec2};
use engine_cdp::ElementRef;
use session::{ActOutcome, Handoff};

use crate::engine::{EngineHandle, Event, TabId, TabInfo};

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
}

impl ShellApp {
    pub fn new(initial_url: String, log_path: PathBuf, _cc: &eframe::CreationContext<'_>) -> Self {
        ShellApp {
            handle: crate::engine::spawn(initial_url.clone(), log_path),
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
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.handle.events.try_recv() {
            match event {
                Event::Frame { tab, frame } => {
                    if Some(tab) != self.active {
                        continue;
                    }
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [frame.width, frame.height],
                        &frame.rgba,
                    );
                    let css = Vec2::new(frame.css_width as f32, frame.css_height as f32);
                    match self.frames.get_mut(&tab) {
                        Some(existing) => {
                            existing.texture.set(image, egui::TextureOptions::LINEAR);
                            existing.css = css;
                        }
                        None => {
                            let texture = ctx.load_texture(
                                format!("página-{tab}"),
                                image,
                                egui::TextureOptions::LINEAR,
                            );
                            self.frames.insert(tab, TabFrame { texture, css });
                        }
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
                    self.generation = info.generation;
                    self.elements = elements;
                    self.address = info.url.clone();
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
                Event::Notice(notice) => self.status = notice,
                Event::Error(error) => self.status = format!("error: {error}"),
            }
        }
    }

    fn navigate(&mut self) {
        let url = normalize_url(&self.address);
        self.address = url.clone();
        self.status = format!("navegando a {url}…");
        self.handle.navigate(url);
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let tabs = self.tabs.clone();
        for tab in &tabs {
            let selected = Some(tab.id) == self.active;
            if ui.selectable_label(selected, display_title(tab)).clicked() {
                self.handle.select_tab(tab.id);
            }
            if selected && ui.small_button("×").clicked() {
                self.handle.close_tab(tab.id);
            }
        }
        if ui.button("+").clicked() {
            self.status = "abriendo pestaña…".to_string();
            self.handle.new_tab(None);
        }
    }

    fn agent_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("Agente");
            ui.label(
                egui::RichText::new(if self.agent_control {
                    "control: agente"
                } else {
                    "control: persona"
                })
                .size(12.0)
                .weak(),
            );
        });
        ui.separator();

        if ui
            .button(if self.agent_control {
                "Recuperar control"
            } else {
                "Ceder control"
            })
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
                    "La shell no acepta clicks mientras el agente conduce. Las acciones del \
                     agente pasan por MCP, su tarjeta y su recibo.",
                )
                .size(12.0),
            );
        }
        ui.add_space(6.0);

        ui.label(egui::RichText::new("Recibos de esta pestaña").strong());
        ui.label(format!(
            "{} · último {}",
            self.receipts.count,
            self.receipts.last.as_deref().map(short_hash).unwrap_or("—")
        ));
        if let Some(log) = &self.receipts.log {
            ui.label(
                egui::RichText::new(log.display().to_string())
                    .monospace()
                    .size(10.0)
                    .weak(),
            );
        }
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!(
                "generación {} · {} refs vivas",
                self.generation,
                self.elements.len()
            ))
            .size(12.0),
        );
        if self.elements.is_empty() {
            ui.label(egui::RichText::new("sin snapshot vivo").weak().size(12.0));
        } else {
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .id_salt("refs")
                .show(ui, |ui| {
                    for element in self.elements.iter().take(24) {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} {:<9} {}",
                                element.reference, element.role, element.name
                            ))
                            .monospace()
                            .size(11.0),
                        );
                    }
                });
        }

        ui.separator();
        ui.label(egui::RichText::new("Handoff").strong());
        match &self.handoff {
            Some(report) => {
                egui::ScrollArea::vertical()
                    .id_salt("handoff")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(report.render_text())
                                .monospace()
                                .size(11.0),
                        );
                    });
            }
            None => {
                ui.label(
                    egui::RichText::new(
                        "Cede el control para generar el informe que recibe el agente.",
                    )
                    .size(12.0)
                    .weak(),
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
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                for row in &pending.card.rows {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&row.label).strong().size(12.0));
                        if row.emphasis {
                            ui.label(
                                egui::RichText::new(&row.value)
                                    .color(Color32::LIGHT_RED)
                                    .strong(),
                            );
                        } else {
                            ui.label(&row.value);
                        }
                    });
                }
                for warning in &pending.card.warnings {
                    ui.label(egui::RichText::new(format!("⚠ {warning}")).color(Color32::LIGHT_RED));
                }
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("hash {}", short_hash(&pending.hash)))
                        .monospace()
                        .size(10.0)
                        .weak(),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Denegar").clicked() {
                        deny = true;
                    }
                    if pending.card.approve_hash.is_some() && ui.button("Aprobar").clicked() {
                        approve = true;
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
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
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
                ui.label("sin pestañas: pulsa + para abrir una");
            });
            return;
        };
        let Some(frame) = self.frames.get(&tab) else {
            ui.centered_and_justified(|ui| ui.label("esperando el primer frame…"));
            return;
        };

        let css = frame.css;
        let aspect = (css.x / css.y).max(0.1);
        let mut size = ui.available_size();
        if size.x / size.y > aspect {
            size.x = size.y * aspect;
        } else {
            size.y = size.x / aspect;
        }

        let texture = frame.texture.id();
        ui.vertical_centered(|ui| {
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
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

            if response.clicked()
                && let Some(pointer) = response.interact_pointer_pos()
            {
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
        });
    }
}

impl eframe::App for ShellApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events(ui.ctx());

        egui::Panel::top("barra").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                self.tab_strip(ui);
                ui.separator();
                ui.add_space(4.0);
                let width = (ui.available_width() - 190.0).max(160.0);
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.address)
                        .desired_width(width)
                        .hint_text("https://…"),
                );
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter || ui.button("Ir").clicked() {
                    self.navigate();
                }
                if ui.selectable_label(self.agent_open, "Agente").clicked() {
                    self.agent_open = !self.agent_open;
                }
            });
            ui.add_space(2.0);
            ui.label(egui::RichText::new(&self.status).size(12.0).weak());
            ui.add_space(2.0);
        });

        if self.agent_open {
            egui::Panel::right("agente")
                .resizable(true)
                .default_size(370.0)
                .show(ui, |ui| self.agent_panel(ui));
        }

        egui::CentralPanel::default().show(ui, |ui| self.page(ui));

        self.pending_window(ui.ctx());
        self.result_window(ui.ctx());
    }
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

fn short_hash(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
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
    use super::normalize_url;

    #[test]
    fn completa_el_esquema_sin_romper_urls_explicitas() {
        assert_eq!(normalize_url("ejemplo.com"), "https://ejemplo.com");
        assert_eq!(normalize_url("127.0.0.1:8977/a"), "http://127.0.0.1:8977/a");
        assert_eq!(normalize_url("localhost:3000"), "http://localhost:3000");
        assert_eq!(normalize_url("http://x.test/a"), "http://x.test/a");
        assert_eq!(normalize_url("  https://x.test  "), "https://x.test");
    }
}
