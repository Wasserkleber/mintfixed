//! GUI controls for opt-in mod.io download timing capture.
use crate::providers::modio_diagnostics as diagnostics;
use std::time::Duration;

pub(super) fn show(ctx: &egui::Context, open: &mut bool) {
    if !*open {
        return;
    }
    egui::Window::new("mod.io download diagnostics")
        .open(open)
        .default_size([760.0, 500.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if diagnostics::is_running() {
                    if ui.button("Stop capture").clicked() {
                        diagnostics::stop();
                    }
                } else if ui.button("Start new capture").clicked() {
                    diagnostics::start();
                }
                if ui.button("Clear").clicked() {
                    diagnostics::clear();
                }
                if ui.button("Copy report").clicked() {
                    ui.ctx().copy_text(diagnostics::report());
                }
            });
            ui.label("Start capture, then add or update mods and install them. Cached files will not be downloaded again. No tokens, request headers, signed URLs or query strings are captured.");
            ui.separator();
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                ui.monospace(diagnostics::report());
            });
        });
    ctx.request_repaint_after(Duration::from_secs(1));
}
