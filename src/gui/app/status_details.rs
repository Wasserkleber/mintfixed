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
            egui::Window::new(if self.live_unavailable_details { "Unavailable mods" } else { "Status details" })
                .id(egui::Id::new("mod_status_details"))
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
                                ui.strong("Unavailable mod locations");
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
                self.live_unavailable_details = false;
            }
        }
        if let Some(target) = show {
            let previous = self.state.mod_data.active_profile.clone();
            self.state.mod_data.active_profile = target.profile.clone();
            if let Err(error) = self.state.mod_data.save() {
                self.state.mod_data.active_profile = previous;
                self.status_details = Some(format!("Could not save profile selection: {error}"));
                self.status_locations.clear();
                self.live_unavailable_details = false;
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
            self.live_unavailable_details = false;
            ctx.request_repaint();
        }
    }
}

impl App {
    pub(super) fn refresh_unavailable_overview(&mut self) {
        let data = &self.state.mod_data;
        let store = &self.state.store;
        let changed =
            self.unavailable_overview
                .refresh((data.revision(), store.metadata_revision()), || {
                    unavailable_mods::entries(
                        data,
                        |spec| store.is_modio_unavailable(spec),
                        |spec| {
                            store
                                .get_mod_info(spec)
                                .map(|info| info.name)
                                .unwrap_or_else(|| spec.url.clone())
                        },
                    )
                });
        if changed && self.live_unavailable_details {
            self.open_unavailable_overview();
        }
    }

    fn open_unavailable_overview(&mut self) {
        self.status_details = Some(self.unavailable_overview.description());
        self.status_locations = self.unavailable_overview.entries.clone();
        self.live_unavailable_details = true;
    }

    pub(super) fn unavailable_overview_button(&mut self, ui: &mut egui::Ui) {
        let count = self.unavailable_overview.entries.len();
        if count == 0 {
            return;
        }
        ui.horizontal(|ui| {
            let hint = "Show unavailable entries across all saved profiles and groups, including disabled entries. Shared mods are counted once per saved location.";
            let icon = icon_buttons::warning_button(ui, None).on_hover_text(hint);
            let label = ui.add(egui::Label::new(format!("{count} unavailable {}", if count == 1 { "entry" } else { "entries" }))
                .sense(egui::Sense::click())).on_hover_text(hint);
            if icon.clicked() || label.clicked() {
                self.open_unavailable_overview();
                ui.ctx().request_repaint();
            }
        });
    }
}
