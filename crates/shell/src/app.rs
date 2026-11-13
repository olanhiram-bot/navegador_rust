//! UI de la shell: barra de dirección, página y panel de handoff.

use std::path::PathBuf;

use eframe::egui;

use crate::engine::{EngineHandle, Event};

pub struct ShellApp {
    handle: EngineHandle,
    address: String,
    status: String,
    texture: Option<egui::TextureHandle>,
    css_size: (f64, f64),
    handoff: Option<session::Handoff>,
    handoff_open: bool,
    generation: u32,
}

impl ShellApp {
    pub fn new(initial_url: String, log_path: PathBuf, _cc: &eframe::CreationContext<'_>) -> Self {
        ShellApp {
            handle: crate::engine::spawn(initial_url.clone(), log_path),
            address: initial_url,
            status: "cargando…".to_string(),
            texture: None,
            css_size: (1024.0, 768.0),
            handoff: None,
            handoff_open: false,
            generation: 0,
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.handle.events.try_recv() {
            match event {
                Event::Frame(frame) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [frame.width, frame.height],
                        &frame.rgba,
                    );
                    match &mut self.texture {
                        Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                        None => {
                            self.texture = Some(ctx.load_texture(
                                "página",
                                image,
                                egui::TextureOptions::LINEAR,
                            ))
                        }
                    }
                    self.css_size = (frame.css_width, frame.css_height);
                }
                Event::Snapshot(info) => {
                    self.generation = info.generation;
                    self.address = info.url.clone();
                    self.status = format!(
                        "{} · generación {} · {} elementos accionables",
                        if info.title.is_empty() {
                            info.url.as_str()
                        } else {
                            info.title.as_str()
                        },
                        info.generation,
                        info.elements
                    );
                }
                Event::Handoff(report) => {
                    self.handoff = Some(report);
                    self.handoff_open = true;
                    self.status = "control cedido al agente".to_string();
                }
                Event::Error(error) => {
                    self.status = format!("error: {error}");
                }
            }
        }
    }

    fn navigate(&mut self) {
        let url = normalize_url(&self.address);
        self.address = url.clone();
        self.status = format!("navegando a {url}…");
        self.handle.navigate(url);
    }
}

impl eframe::App for ShellApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events(ui.ctx());

        egui::Panel::top("barra").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Navegador").strong());
                let width = (ui.available_width() - 230.0).max(160.0);
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.address)
                        .desired_width(width)
                        .hint_text("https://…"),
                );
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter || ui.button("Ir").clicked() {
                    self.navigate();
                }
                if ui.button("Ceder al agente").clicked() {
                    self.handle.handoff();
                    self.status = "preparando el handoff…".to_string();
                }
            });
            ui.add_space(2.0);
            ui.label(egui::RichText::new(&self.status).size(12.0).weak());
            ui.add_space(2.0);
        });

        let handoff_text = self.handoff.as_ref().map(|report| report.render_text());
        if self.handoff_open
            && let Some(text) = handoff_text
        {
            egui::Panel::right("handoff")
                .resizable(true)
                .default_size(380.0)
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    ui.heading("Control cedido");
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.label(egui::RichText::new(&text).monospace().size(12.0));
                    });
                    ui.separator();
                    ui.label(
                        "El agente conduce esta sesión por MCP. La shell no hace click en \
                         controles de formulario: las acciones pasan por la tarjeta y el recibo.",
                    );
                    ui.add_space(6.0);
                    if ui.button("Recuperar control").clicked() {
                        self.handoff_open = false;
                    }
                });
        }

        egui::CentralPanel::default().show(ui, |ui| {
            let Some(texture) = &self.texture else {
                ui.centered_and_justified(|ui| ui.label("esperando el primer frame…"));
                return;
            };

            let (css_width, css_height) = self.css_size;
            let aspect = (css_width / css_height).max(0.1) as f32;
            let mut size = ui.available_size();
            if size.x / size.y > aspect {
                size.x = size.y * aspect;
            } else {
                size.y = size.x / aspect;
            }

            let response = ui.image((texture.id(), size));
            if response.hovered() {
                let scroll = ui.input(|input| input.smooth_scroll_delta.y);
                if scroll != 0.0
                    && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
                {
                    let rect = response.rect;
                    let x = ((pointer.x - rect.min.x) / rect.width()) as f64 * css_width;
                    let y = ((pointer.y - rect.min.y) / rect.height()) as f64 * css_height;
                    self.handle.wheel(x, y, -(scroll as f64));
                }
            }
        });
    }
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
