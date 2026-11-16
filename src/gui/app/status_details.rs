// Main frame concern extracted from app/frame.rs; original statements and ordering preserved.
use super::*;

impl App {
    pub(super) fn frame_status_details(&mut self, ctx: &egui::Context) {
        if let Some(details) = self.status_details.as_ref() {
            let mut open = true;
            egui::Window::new("Status details")
                .open(&mut open)
                .resizable(true)
                .default_width(540.0)
                .show(ctx, |ui| {
                    if ui.button("Copy message").clicked() {
                        ui.ctx().copy_text(details.clone());
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(400.0)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(details.as_str()).wrap());
                        });
                });
            if !open {
                self.status_details = None;
            }
        }
    }
}
