// Main frame concern extracted from app/frame.rs; original statements and ordering preserved.
use crate::gui::*;

impl App {
    pub(in crate::gui) fn frame_toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Keep launching DRG next to the Crash Logger on the left.
                ui.add_enabled_ui(
                    self.integrate_rid.is_none()
                        && self.update_rid.is_none()
                        && self.lint_rid.is_none()
                        && self.self_update_rid.is_none()
                        && self.state.config.drg_pak_path.is_some(),
                    |ui| {
                        if ui
                            .add(crate::gui::mint_text_button("Start DRG"))
                            .on_hover_text(if self.args.is_some() {
                                "Launch using Mintfixed's custom launch command"
                            } else {
                                "Launch Deep Rock Galactic through Steam"
                            })
                            .clicked()
                        {
                            let result = if let Some(args) = &self.args {
                                match args.split_first() {
                                    Some((program, command_args)) => std::process::Command::new(program)
                                        .args(command_args)
                                        .spawn()
                                        .map(|_| ())
                                        .map_err(|err| err.to_string()),
                                    None => Err("No launch command was provided".to_owned()),
                                }
                            } else {
                                opener::open("steam://rungameid/548430")
                                    .map_err(|err| err.to_string())
                            };
                            if let Err(err) = result {
                                self.last_action = Some(LastAction::failure(format!(
                                    "Failed to launch DRG: {err}"
                                )));
                            }
                        }
                    },
                );
                if crash_history::toolbar_button(ui) {
                    self.crash_history.open(&self.state);
                }
                if bitfix_menu::toolbar_button(ui) {
                    self.bitfix_menu.open = true;
                }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.add_enabled_ui(
                    self.integrate_rid.is_none()
                        && self.update_rid.is_none()
                        && self.lint_rid.is_none()
                        && self.self_update_rid.is_none()
                        && self.state.config.drg_pak_path.is_some(),
                    |ui| {
                        ui.add_enabled_ui(self.state.config.drg_pak_path.is_some(), |ui| {
                            let mut button = ui.add(crate::gui::mint_text_button("Install mods"));
                            if self.state.config.drg_pak_path.is_none() {
                                button = button.on_disabled_hover_text(
                                    "DRG install not found. Configure it in the settings menu.",
                                );
                            }

                            if button.clicked() {
                                if !self.preflight_unavailable_mods(false) {
                                    return;
                                }
                                if !self.preflight_bitfix_proxy() {
                                    return;
                                }
                                let data = &self.state.mod_data;
                                let mods = data.ordered_install_entries(&data.active_profile);

                                self.last_action = None;
                                self.cache_install_rid = None;
                                self.cache_install_notice = None;
                                self.cache_install_details = None;
                                self.cache_install_locations.clear();
                                self.install_phase = message::InstallPhase::Resolving;
                                self.integrate_rid = Some(message::Integrate::send(
                                    &mut self.request_counter,
                                    self.state.store.clone(),
                                    mods,
                                    self.state.config.drg_pak_path.as_ref().unwrap().clone(),
                                    self.state.config.deref().into(),
                                    self.tx.clone(),
                                    ctx.clone(),
                                ));
                                self.problematic_mod_id = None;
                            }
                        });

                        ui.add_enabled_ui(self.state.config.drg_pak_path.is_some(), |ui| {
                            let mut button = ui.add(crate::gui::mint_text_button("Uninstall mods"));
                            if self.state.config.drg_pak_path.is_none() {
                                button = button.on_disabled_hover_text(
                                    "DRG install not found. Configure it in the settings menu.",
                                );
                            }
                            if button.clicked() {
                                self.last_action = None;
                                if let Some(pak_path) = &self.state.config.drg_pak_path {
                                    let mut mods = HashSet::default();
                                    let active_profile = self.state.mod_data.active_profile.clone();
                                    self.state.mod_data.for_each_enabled_mod(
                                        &active_profile,
                                        |mc| {
                                            if let Some(modio_id) = self
                                                .state
                                                .store
                                                .get_mod_info(&mc.spec)
                                                .and_then(|i| i.modio_id)
                                            {
                                                mods.insert(modio_id);
                                            }
                                        },
                                    );

                                    debug!("uninstalling mods: pak_path = {}", pak_path.display());
                                    self.last_action = Some(match uninstall(pak_path, mods) {
                                        Ok(()) => LastAction::success(
                                            "Successfully uninstalled mods".to_string(),
                                        ),
                                        Err(e) => LastAction::failure(format!(
                                            "Failed to uninstall mods: {e}"
                                        )),
                                    })
                                }
                            }
                        });

                        if ui
                            .add(crate::gui::mint_text_button("Update cache"))
                            .on_hover_text(
                                "Checks every cached mod for updates, then installs the active profile",
                            )
                            .clicked()
                        {
                            // By default mods no profile uses stay cached, so adding one back needs no
                            // download; a failure to refresh one is only a warning
                            // (ModProvider::update_cache). Archiving them first is the opt-in setting.
                            let mut archived = Ok(0);
                            if self.state.config.archive_unused_on_update {
                                let referenced = crate::mod_identity::all_specs(&self.state.mod_data);
                                archived = self.state.store.archive_unreferenced(&referenced);
                            }
                            match archived {
                                Ok(archived) => {
                                    message::UpdateCache::send(self, ctx);
                                    if archived > 0 {
                                        self.last_action = Some(LastAction::success(format!(
                                            "Archived {archived} unused cached mods; updating cache"
                                        )));
                                    }
                                    self.problematic_mod_id = None;
                                }
                                Err(error) => {
                                    self.last_action = Some(LastAction::failure(format!(
                                        "Cache cleanup failed: {error}"
                                    )));
                                }
                            }
                        }
                    },
                );
                if self.integrate_rid.is_some() {
                    if ui.add(crate::gui::mint_text_button("Cancel")).clicked() {
                        self.integrate_rid.take().unwrap().handle.abort();
                        self.install_phase = message::InstallPhase::Resolving;
                        if self.cache_install_rid.take().is_some() {
                            let notice = self.cache_install_notice.take()
                                .unwrap_or_else(|| "Cache updated".to_string());
                            let mut action = LastAction::failure(format!(
                                "{notice}; installation cancelled"
                            ));
                            action.details = self.cache_install_details.take()
                                .map(|details| format!("{details}\n\nInstallation cancelled"));
                            action.unavailable_locations = std::mem::take(&mut self.cache_install_locations);
                            self.last_action = Some(action);
                        }
                    }
                    ui.spinner();
                }
                if self.update_rid.is_some() {
                    if ui.add(crate::gui::mint_text_button("Cancel")).clicked() {
                        self.update_rid.take().unwrap().handle.abort();
                        self.cache_recovery_progress = None;
                        self.cache_recovery_last_advance = None;
                        self.last_action = Some(LastAction::failure("Cache update cancelled".to_string()));
                    }
                    ui.spinner();
                }
                if ui
                    .add(crate::gui::mint_text_button("Lint mods"))
                    .on_hover_text("Lint mods in the current profile")
                    .clicked()
                {
                    let mods = self.enabled_lint_specs();
                    self.lint_selected_mods = mods.iter().cloned().collect();
                    self.lint_focus = mods.first().cloned();
                    self.lint_filter.clear();
                    self.lints_toggle_window = Some(WindowLintsToggle);
                }
                let settings_icon = egui::Image::new(egui::include_image!("../../../assets/icons/settings.svg"))
                    .fit_to_exact_size(egui::vec2(14.0, 14.0))
                    .tint(ui.visuals().text_color());
                if ui
                    .add(egui::Button::image(settings_icon))
                    .on_hover_text("Open settings")
                    .clicked()
                {
                    self.settings_window = Some(WindowSettings::new(&self.state));
                }
                if let Some(available_update) = &self.available_update
                    && ui
                        .button(egui::RichText::new("\u{26A0}").color(ui.visuals().warn_fg_color))
                        .on_hover_text(format!(
                            "Update available: {}\n{}",
                            available_update.tag_name, available_update.html_url
                        ))
                        .clicked()
                {
                    ui.ctx()
                        .open_url(egui::OpenUrl::new_tab(&available_update.html_url));
                }
                // Render fixed action buttons first (right-to-left), then confine
                // status text to the actual remainder of the toolbar row.
                let status_width = ui.available_width().max(0.0);
                if status_width >= 24.0 {
                    ui.allocate_ui_with_layout(
                        egui::vec2(status_width, ui.spacing().interact_size.y),
                        egui::Layout::left_to_right(Align::Center),
                        |ui| {
                            if self.update_rid.is_some() {
                                let progress = if let Some((completed, total)) = self.cache_recovery_progress {
                                    // Repaint when the inactivity threshold expires, even if
                                    // mod.io hasn't sent another progress event.
                                    let waiting = completed < total
                                        && self.cache_recovery_last_advance.is_some_and(|last| {
                                            let elapsed = last.elapsed();
                                            let threshold = Duration::from_secs(10);
                                            if elapsed < threshold {
                                                ui.ctx().request_repaint_after(threshold - elapsed);
                                                false
                                            } else {
                                                true
                                            }
                                        });
                                    format!(
                                        "Recovering mod.io metadata: ({completed}/{total}){}",
                                        if waiting { " (waiting for mod.io)" } else { "" }
                                    )
                                } else {
                                    "Updating provider caches...".to_owned()
                                };
                                if status_message(ui, &progress) {
                                    self.status_details = Some(progress);
                                    self.status_locations.clear();
                                    self.live_unavailable_details = false;
                                }
                            } else if self.integrate_rid.is_some() {
                                let progress = match self.install_phase {
                                    message::InstallPhase::Resolving => "mod.io rate limits may slow version / dependency checks...",
                                    message::InstallPhase::Downloading => "mod.io rate limits may slow downloads / cache checks...",
                                    message::InstallPhase::Integrating => "Writing mods_P.pak...",
                                };
                                if status_message(ui, progress) {
                                    self.status_details = Some(progress.to_owned());
                                    self.status_locations.clear();
                                    self.live_unavailable_details = false;
                                }
                            } else if let Some(last_action) = &self.last_action {
                                // Hide the badge as well if no room remains for it.
                                if ui.available_width() >= 64.0 {
                                    let (color, message, outcome) = match &last_action.status {
                                        LastActionStatus::Success(message) => (Color32::LIGHT_GREEN, message, "Success"),
                                        LastActionStatus::Failure(message) => (Color32::LIGHT_RED, message, "Error"),
                                    };
                                    ui.label(
                                        egui::RichText::new("STATUS")
                                            .color(Color32::BLACK)
                                            .background_color(color),
                                    );
                                    ui.ctx().request_repaint_after(Duration::from_secs(1));
                                    let summary = format!("({}): {}", last_action.timeago(), message);
                                    if status_message(ui, &summary) {
                                        self.live_unavailable_details = false;
                                        self.status_locations = last_action.unavailable_locations.clone();
                                        self.status_details = Some(last_action.details.clone().unwrap_or_else(|| format!("{outcome} {summary}")));
                                    }
                                }
                            }
                        },
                    );
                }
            });
            });
        });
    }
}
