// app-update dialog; methods moved intact from gui/mod.rs.
use crate::gui::*;

impl App {
    pub(in crate::gui) fn show_update_window(&mut self, ctx: &egui::Context) {
        if let (Some(update), Some(update_time)) = (
            self.update_state.available_update.as_ref(),
            self.update_state.show_update_time,
        ) {
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
            if let Some(MessageHandle { state, .. }) = &self.update_state.self_update_rid {
                egui::Window::new("Update progress")
                    .collapsible(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.with_layout(egui::Layout::top_down_justified(Align::Center), |ui| {
                            // Match the mod installation progress palette.
                            let (accent, track) = if ui.visuals().dark_mode {
                                (
                                    egui::Color32::from_rgb(49, 126, 89),
                                    egui::Color32::from_rgb(65, 68, 72),
                                )
                            } else {
                                (
                                    egui::Color32::from_rgb(45, 119, 82),
                                    egui::Color32::from_rgb(220, 223, 225),
                                )
                            };
                            ui.visuals_mut().extreme_bg_color = track;
                            let update_bar = |fraction| {
                                egui::ProgressBar::new(fraction)
                                    .fill(accent)
                                    .corner_radius(4_u8)
                            };
                            match state {
                                SelfUpdateProgress::Pending => {
                                    ui.add(update_bar(0.0).show_percentage());
                                }
                                SelfUpdateProgress::Progress { progress, size } => {
                                    ui.add(
                                        update_bar(if *size == 0 {
                                            0.0
                                        } else {
                                            *progress as f32 / *size as f32
                                        })
                                        .show_percentage(),
                                    );
                                }
                                SelfUpdateProgress::Complete => {
                                    ui.add(update_bar(1.0).show_percentage());
                                    ui.label(
                                        egui::RichText::new("Update successful.")
                                            .color(Color32::LIGHT_GREEN),
                                    );

                                    if ui.add(crate::gui::mint_text_button("Restart")).clicked() {
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
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width().max(0.0), ui.spacing().interact_size.y),
                            egui::Layout::right_to_left(Align::Center),
                            |ui| {
                                if ui
                                    .add(crate::gui::mint_text_button("Install update"))
                                    .on_hover_text("Download and install the update.")
                                    .clicked()
                                {
                                    self.update_state.self_update_rid =
                                        Some(message::SelfUpdate::send(
                                            &mut self.request_counter,
                                            self.tx.clone(),
                                            ctx.clone(),
                                        ));
                                }

                                let elapsed = now.duration_since(update_time).unwrap_or_default();
                                if elapsed > wait_time {
                                    if ui.add(crate::gui::mint_text_button("Close")).clicked() {
                                        self.update_state.show_update_time = None;
                                    }
                                } else {
                                    ui.spinner();
                                }
                            },
                        );
                    });
            }
        }
    }
}
