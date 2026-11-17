use super::*;

impl App {
    pub(super) fn frame_status_details(&mut self, ctx: &egui::Context) {
        let mut show = None;
        if let Some(details) = self.status_details.as_ref() {
            let mut open = true;
            let busy = self.integrate_rid.is_some()
                || self.update_rid.is_some()
                || self.lint_rid.is_some()
                || self.resolve_mod_rid.is_some()
                || self.pending_group_files > 0;
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
                            let report = unavailable_mods::describe(&self.status_locations);
                            let (before, after) = if self.status_locations.is_empty() {
                                (details.as_str(), "")
                            } else {
                                details.split_once(&report).unwrap_or((details.as_str(), ""))
                            };
                            ui.add(egui::Label::new(before.trim()).wrap());
                            if !self.status_locations.is_empty() {
                                ui.separator();
                                ui.strong("Unavailable mods in saved profiles and groups (including disabled entries)");
                            }
                            for location in &self.status_locations {
                                let target = location.target.as_ref()
                                    .and_then(|target| target.resolve(&self.state.mod_data));
                                ui.horizontal(|ui| {
                                    let hint = if busy {
                                        "Wait for the current operation to finish."
                                    } else if location.target.is_none() {
                                        "This group is not used by any profile. Add it to a profile first."
                                    } else if target.is_none() {
                                        "This entry is no longer at its saved location. Update Cache to refresh the list."
                                    } else {
                                        "Open this profile and group, then scroll to the mod."
                                    };
                                    if ui.add_enabled(!busy && target.is_some(), egui::Button::new("Show mod"))
                                        .on_hover_text(hint).clicked() {
                                        show = target;
                                    }
                                    ui.add(egui::Label::new(&location.label).wrap());
                                });
                            }
                            if !after.trim().is_empty() {
                                ui.separator();
                                ui.add(egui::Label::new(after.trim()).wrap());
                            }
                        });
                });
            if !open {
                self.status_details = None;
                self.status_locations.clear();
            }
        }
        if let Some(target) = show {
            let previous = self.state.mod_data.active_profile.clone();
            self.state.mod_data.active_profile = target.profile.clone();
            if let Err(error) = self.state.mod_data.save() {
                self.state.mod_data.active_profile = previous;
                self.status_details = Some(format!("Could not save profile selection: {error}"));
                self.status_locations.clear();
                return;
            }
            self.edit_history
                .reset(&self.state.mod_data, self.state.mod_data.revision());
            self.search_string.clear();
            self.scroll_to_match = false;
            if let Some(group) = &target.group {
                self.groups_to_open
                    .insert((target.profile.clone(), group.clone()));
            }
            self.show_mod_target = Some(target);
            self.status_details = None;
            self.status_locations.clear();
            ctx.request_repaint();
        }
    }
}
