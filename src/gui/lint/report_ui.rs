//! Lint results window, report export actions and progress.
use super::super::*;

impl App {
    pub(in crate::gui) fn show_lint_report(&mut self, ctx: &egui::Context) {
        if self.lint_state.lint_report_window.is_some() {
            let mut open = true;

            egui::Window::new("Lint results")
                .open(&mut open)
                .resizable(true)
                .show(ctx, |ui| {
                    if let Some(path) = self.lint_state.lint_report_path.clone() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("TXT saved: {}", path.display()));
                            if ui.button("Open TXT").clicked()
                                && let Err(error) = opener::open(&path) {
                                    self.status_state.last_action = Some(LastAction::failure(format!("Could not open lint report: {error}")));
                                }
                            if ui.button("Copy path").clicked() {
                                ui.ctx().copy_text(path.to_string_lossy().to_string());
                            }
                        });
                    }
                    if let Some(error) = &self.lint_state.lint_report_error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                    if let Some(report) = &self.lint_state.lint_report {
                        let scroll_height =
                            (ui.available_height() - 30.0).clamp(0.0, f32::INFINITY);
                        egui::ScrollArea::vertical()
                            .max_height(scroll_height)
                            .show(ui, |ui| {
                                super::report_sections::show(ui, report);
                            });
                    } else if self.lint_state.lint_rid.is_some() {
                        ui.spinner();
                        ui.label("Lint report generating...");
                    } else {
                        ui.label("Lint failed. See the bottom status message for details.");
                    }
                });
            if !open {
                self.lint_state.lint_report_window = None;
            }
        }
    }
}
