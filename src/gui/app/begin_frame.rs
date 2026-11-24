// Main frame concern extracted from app/frame.rs; original statements and ordering preserved.
use crate::gui::*;

impl App {
    pub(in crate::gui) fn frame_begin(&mut self, ctx: &egui::Context) {
        // VSync limits frames to the monitor's refresh rate, which may exceed 60 Hz.
        // Pace updates during rapid input without requesting any frames when idle.
        const MIN_FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);
        if let Some(previous) = self.last_ui_frame_start
            && let Some(remaining) = MIN_FRAME_INTERVAL.checked_sub(previous.elapsed())
        {
            std::thread::sleep(remaining);
        }
        self.last_ui_frame_start = Some(Instant::now());

        if self.needs_restart
            && let Some(original_exe_path) = &self.original_exe_path
        {
            debug!("needs restart");
            self.needs_restart = false;

            debug!("restarting...");
            let _child = std::process::Command::new(original_exe_path)
                .spawn()
                .unwrap();
            debug!("created child process");

            std::process::exit(0);
        }

        // do some init things that depend on ctx so cannot be done earlier
        if !self.has_run_init {
            self.has_run_init = true;

            let theme = GuiTheme::into_egui_theme(self.state.config.gui_theme);
            ctx.memory_mut(|m| m.options.theme_preference = theme);

            message::CheckUpdates::send(self, ctx);
        }

        // message handling
        while let Ok(msg) = self.rx.try_recv() {
            // Only act on the completion of the currently running cache update.
            let cache_was_running = self.update_rid.is_some();
            let is_cache_result = matches!(&msg, message::Message::UpdateCache(_));
            // Match the exact automatic installation that is completing.
            let finishing_cache_install = matches!(&msg, message::Message::Integrate(_))
                && self.cache_install_rid.is_some()
                && self.cache_install_rid == self.integrate_rid.as_ref().map(|r| r.rid);
            msg.handle(self);
            // The shared store already contains the recovered metadata; invalidate
            // cached profile warnings and redraw names/tags immediately.
            if is_cache_result && cache_was_running && self.update_rid.is_none() {
                self.profile_issues.clear();
                self.cache_recovery_progress = None;
                self.cache_recovery_last_advance = None;
                ctx.request_repaint();
            }
            if finishing_cache_install && self.integrate_rid.is_none() {
                self.cache_install_rid = None;
                let notice = self
                    .cache_install_notice
                    .take()
                    .unwrap_or_else(|| "Cache updated".to_string());
                let cache_details = self
                    .cache_install_details
                    .take()
                    .unwrap_or_else(|| notice.clone());
                if let Some(action) = self.last_action.as_mut() {
                    let install_details =
                        action
                            .details
                            .take()
                            .unwrap_or_else(|| match &action.status {
                                LastActionStatus::Success(message) => message.clone(),
                                LastActionStatus::Failure(reason) => {
                                    format!("Installation failed: {reason}")
                                }
                            });
                    action.details = Some(format!("{cache_details}\n\n{install_details}"));
                    action.unavailable_locations =
                        std::mem::take(&mut self.cache_install_locations);
                    match &mut action.status {
                        LastActionStatus::Success(message) => {
                            *message = format!("{notice}; {message}");
                        }
                        LastActionStatus::Failure(reason) => {
                            *reason = format!("{notice}; installation failed: {reason}");
                        }
                    }
                }
            }
            if is_cache_result
                && cache_was_running
                && self.update_rid.is_none()
                && self.integrate_rid.is_none()
                && self
                    .last_action
                    .as_ref()
                    .is_some_and(|action| matches!(&action.status, LastActionStatus::Success(_)))
            {
                // Use the same profile ordering and integration as the Install Mods button.
                if let Some(fsd_pak) = self.state.config.drg_pak_path.clone() {
                    if !self.preflight_unavailable_mods(true) {
                        continue;
                    }
                    if !self.preflight_bitfix_proxy() {
                        continue;
                    }
                    let data = &self.state.mod_data;
                    let mods = data.ordered_install_entries(&data.active_profile);
                    self.integrate_rid = Some(message::Integrate::send(
                        &mut self.request_counter,
                        self.state.store.clone(),
                        mods,
                        fsd_pak,
                        self.state.config.deref().into(),
                        self.tx.clone(),
                        ctx.clone(),
                    ));
                    // Retain the cache result before the installation clears the status.
                    // It may contain warnings about unavailable mod.io mods.
                    self.cache_install_notice = self.last_action.as_ref().and_then(|action| {
                        if let LastActionStatus::Success(message) = &action.status {
                            Some(message.clone())
                        } else {
                            None
                        }
                    });
                    self.cache_install_details = self
                        .last_action
                        .as_ref()
                        .and_then(|action| action.details.clone());
                    self.cache_install_locations = self
                        .last_action
                        .as_ref()
                        .map(|action| action.unavailable_locations.clone())
                        .unwrap_or_default();
                    self.cache_install_rid = self.integrate_rid.as_ref().map(|r| r.rid);
                    self.install_phase = message::InstallPhase::Resolving;
                    self.problematic_mod_id = None;
                    // The install is not yet successful; the live phase label is shown below.
                    self.last_action = None;
                } else {
                    self.last_action = Some(LastAction::failure(
                        "cache updated, but DRG install not found".to_string(),
                    ));
                }
            }
        }

        // Record finished edits, including asynchronously resolved mods.
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.edit_history
                .sync(&self.state.mod_data, self.state.mod_data.revision());
        }
        // Preserve native text-edit undo while the user is typing.
        if !ctx.wants_keyboard_input()
            && self.integrate_rid.is_none()
            && self.update_rid.is_none()
            && self.lint_rid.is_none()
        {
            let shortcut = ctx.input(|i| {
                if !i.modifiers.command {
                    return None;
                }
                if i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))
                {
                    Some(true)
                } else if !i.modifiers.shift && i.key_pressed(egui::Key::Z) {
                    Some(false)
                } else {
                    None
                }
            });
            if let Some(redo) = shortcut {
                self.apply_mod_history(ctx, redo);
            }
        }
    }
}
