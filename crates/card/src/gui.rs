//! Renderizador nativo de la tarjeta (egui).
//!
//! Se ejecuta en el proceso del navegador, no en el de la página. La página no
//! tiene acceso a este contexto: no puede reestilizar la tarjeta, taparla ni
//! inyectar marcado. Todo valor se pinta como texto plano.

use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::CardView;

#[derive(Debug, Clone, PartialEq)]
pub enum Approval {
    Approved(String),
    Denied,
}

/// Muestra la tarjeta y espera una decisión explícita.
pub fn show(view: CardView) -> Result<Approval, String> {
    let result: Arc<Mutex<Option<Approval>>> = Arc::new(Mutex::new(None));
    let shared = result.clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([580.0, 480.0])
            .with_resizable(false)
            .with_title("Navegador — confirmar acción"),
        ..Default::default()
    };

    eframe::run_native(
        "Navegador — confirmar acción",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(CardApp {
                view,
                result: shared,
            }))
        }),
    )
    .map_err(|error| error.to_string())?;

    let decision = result.lock().unwrap().clone();
    decision.ok_or_else(|| "la ventana se cerró sin decisión".to_string())
}

struct CardApp {
    view: CardView,
    result: Arc<Mutex<Option<Approval>>>,
}

impl eframe::App for CardApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(8.0);
            ui.heading(&self.view.title);
            ui.separator();

            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    for row in &self.view.rows {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(format!("{}:", row.label)).strong());
                            let value = if row.emphasis {
                                egui::RichText::new(&row.value).strong().size(17.0)
                            } else {
                                egui::RichText::new(&row.value)
                            };
                            ui.label(value);
                        });
                    }
                });

            if !self.view.warnings.is_empty() {
                ui.add_space(6.0);
                for warning in &self.view.warnings {
                    ui.colored_label(egui::Color32::from_rgb(220, 60, 60), format!("⚠ {warning}"));
                }
            }

            ui.add_space(10.0);
            ui.separator();

            // Denegar es la acción dominante y la predeterminada.
            let deny = ui.add_sized(
                [ui.available_width(), 46.0],
                egui::Button::new(egui::RichText::new("Denegar").size(18.0)),
            );
            let escape = ui.ctx().input(|i| i.key_pressed(egui::Key::Escape));
            if deny.clicked() || escape {
                *self.result.lock().unwrap() = Some(Approval::Denied);
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }

            if let Some(hash) = self.view.approve_hash.clone() {
                ui.add_space(6.0);
                let approve = ui.add_sized(
                    [ui.available_width(), 36.0],
                    egui::Button::new("Aprobar esta acción"),
                );
                if approve.clicked() {
                    *self.result.lock().unwrap() = Some(Approval::Approved(hash));
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        });
    }
}
