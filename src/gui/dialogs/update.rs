// app-update dialog; methods moved intact from gui/mod.rs.
use super::*;

impl App {
    pub(super) fn show_update_window(&mut self, ctx: &egui::Context) {
        if let (Some(update), Some(update_time)) =
            (self.available_update.as_ref(), self.show_update_time)
        {
            let now = SystemTime::now();
            let wait_time = Duration::from_secs(10);
            egui::Area::new("available-update-overlay".into())
                .movable(false)
                .fixed_pos(Pos2::ZERO)
                .order(egui::Order::Background)
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(Color32::from_rgba_unmultiplied(0, 0, 0, 127))
                        .show(ui, |ui| {
                            ui.allocate_space(ui.available_size());
                        })
                });
            if let Some(MessageHandle { state, .. }) = &self.self_update_rid {
                egui::Window::new("Update progress")
                    .collapsible(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.with_layout(egui::Layout::top_down_justified(Align::Center), |ui| {
                            match state {
                                SelfUpdateProgress::Pending => {
                                    ui.add(egui::ProgressBar::new(0.0).show_percentage());
                                }
                                SelfUpdateProgress::Progress { progress, size } => {
                                    ui.add(
                                        egui::ProgressBar::new(*progress as f32 / *size as f32)
                                            .show_percentage(),
                                    );
                                }
                                SelfUpdateProgress::Complete => {
                                    ui.add(egui::ProgressBar::new(1.0).show_percentage());
                                    ui.label(
                                        egui::RichText::new("Update successful.")
                                            .color(Color32::LIGHT_GREEN),
                                    );

                                    if ui.button("Restart").clicked() {
                                        self.needs_restart = true;
                                    }
                                }
                            };
                        });
                    });
            } else {
                egui::Window::new(format!("Update available: {}", update.tag_name))
                    .collapsible(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .resizable(false)
                    .show(ctx, |ui| {
                        CommonMarkViewer::new().max_image_width(Some(512)).show(
                            ui,
                            &mut self.cache,
                            &update.body,
                        );
                        ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                            if ui
                                .add(egui::Button::new("Install update"))
                                .on_hover_text("Download and install the update.")
                                .clicked()
                            {
                                self.self_update_rid = Some(message::SelfUpdate::send(
                                    &mut self.request_counter,
                                    self.tx.clone(),
                                    ctx.clone(),
                                ));
                            }

                            let elapsed = now.duration_since(update_time).unwrap_or_default();
                            if elapsed > wait_time {
                                if ui.button("Close").clicked() {
                                    self.show_update_time = None;
                                }
                            } else {
                                ui.spinner();
                            }
                        });
                    });
            }
        }
    }
}
