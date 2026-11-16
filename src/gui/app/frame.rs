// Main eframe UI frame and event handling; extracted without changing behavior.
use super::*;

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
                if let Some(action) = self.last_action.as_mut() {
                    match &mut action.status {
                        LastActionStatus::Success(message) => {
                            *message = format!("{notice}; {message}");
                            if let Some(details) = action.details.as_mut() {
                                *details = format!("{notice}.\n{details}");
                            }
                        }
                        LastActionStatus::Failure(reason) => {
                            *reason = format!("{notice}; installation failed: {reason}");
                            if let Some(details) = action.details.as_mut() {
                                *details = format!("{notice}.\n{details}");
                            }
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
                self.apply_mod_history(redo);
            }
        }
        // begin draw

        self.show_update_window(ctx);
        #[cfg(all(feature = "hook", target_os = "windows"))]
        self.startup_crash_reporter_update.show(ctx);
        self.show_provider_parameters(ctx);
        self.show_profile_windows(ctx);
        self.show_settings(ctx);
        folder_import::tick(self, ctx);
        folder_import::show_preview(self, ctx);
        self.show_lints_toggle(ctx);
        self.show_lint_report(ctx);
        // Check the configured installation without accessing the disk every frame.
        if self
            .crash_monitor
            .as_ref()
            .is_none_or(|monitor| !monitor.matches_installation(&self.state))
        {
            self.crash_monitor = Some(crash_notice::CrashMonitor::start(&self.state, ctx));
        }
        if let Some(monitor) = &mut self.crash_monitor {
            monitor.poll(&mut self.recent_crash);
        }
        crash_notice::show(ctx, &mut self.recent_crash);

        self.crash_history.show(ctx, &self.state);
        self.show_bitfix_menu(ctx);
        self.show_bitfix_conflict(ctx);
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
                            .button("Start DRG")
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
            ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                ui.add_enabled_ui(
                    self.integrate_rid.is_none()
                        && self.update_rid.is_none()
                        && self.lint_rid.is_none()
                        && self.self_update_rid.is_none()
                        && self.state.config.drg_pak_path.is_some(),
                    |ui| {
                        ui.add_enabled_ui(self.state.config.drg_pak_path.is_some(), |ui| {
                            let mut button = ui.button("Install mods");
                            if self.state.config.drg_pak_path.is_none() {
                                button = button.on_disabled_hover_text(
                                    "DRG install not found. Configure it in the settings menu.",
                                );
                            }

                            if button.clicked() {
                                if !self.preflight_bitfix_proxy() {
                                    return;
                                }
                                let data = &self.state.mod_data;
                                let mods = data.ordered_install_entries(&data.active_profile);

                                self.last_action = None;
                                self.cache_install_rid = None;
                                self.cache_install_notice = None;
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
                            let mut button = ui.button("Uninstall mods");
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
                            .button("Update cache")
                            .on_hover_text(
                                "Archives unused cache entries, checks for mod updates, then installs the active profile",
                            )
                            .clicked()
                        {
                            let referenced = crate::mod_identity::all_specs(&self.state.mod_data);
                            match self.state.store.archive_unreferenced(&referenced) {
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
                    if ui.button("Cancel").clicked() {
                        self.integrate_rid.take().unwrap().handle.abort();
                        self.install_phase = message::InstallPhase::Resolving;
                        if self.cache_install_rid.take().is_some() {
                            let notice = self.cache_install_notice.take()
                                .unwrap_or_else(|| "Cache updated".to_string());
                            self.last_action = Some(LastAction::failure(format!(
                                "{notice}; installation cancelled"
                            )));
                        }
                    }
                    ui.spinner();
                }
                if self.update_rid.is_some() {
                    if ui.button("Cancel").clicked() {
                        self.update_rid.take().unwrap().handle.abort();
                        self.cache_recovery_progress = None;
                        self.cache_recovery_last_advance = None;
                        self.last_action = Some(LastAction::failure("Cache update cancelled".to_string()));
                    }
                    ui.spinner();
                }
                if ui
                    .button("Lint mods")
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
                                }
                            } else if self.integrate_rid.is_some() {
                                let progress = match self.install_phase {
                                    message::InstallPhase::Resolving => "Resolving mod versions and dependencies...",
                                    message::InstallPhase::Downloading => "Downloading / checking cached mods...",
                                    message::InstallPhase::Integrating => "Writing mods_P.pak...",
                                };
                                if status_message(ui, progress) {
                                    self.status_details = Some(progress.to_owned());
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
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.integrate_rid.is_some() || self.update_rid.is_some() || self.lint_rid.is_some()
            {
                ui.disable();
            }
            // profile selection

            let mut group_changed = false;
            let mut group_success = None;
            let buttons = |ui: &mut Ui, mod_data: &mut ModData| {
                (group_changed, group_success) = group_controls::controls(
                    ui,
                    mod_data,
                    &mut self.new_group_name,
                    self.pending_group_files > 0,
                );

                // TODO find better icon, flesh out multiple-view usage, fix GUI locking
                /*
                if ui
                    .button("pop out")
                    .on_hover_text_at_pointer("pop out")
                    .clicked()
                {
                    self.open_profiles.insert(mod_data.active_profile.clone());
                }
                */
            };

            let mut imported_profile = None;
            let profile_import_button = |ui: &mut Ui, mod_data: &mut ModData, modified: &mut bool| {
                imported_profile = profile_import::ui_inline(ui, mod_data, modified);
            };
            let mut copy_feedback: Option<Result<String, String>> = None;
            let profile_copy_button = |ui: &mut Ui, mod_data: &mut ModData| {
                copy_feedback = profile_management::copy_options(ui, mod_data);
                copy_feedback.clone()
            };
            let profile_toolbar_changed = named_combobox::ui(
                ui,
                "profile",
                self.state.mod_data.deref_mut().deref_mut(),
                Some(buttons),
                Some(profile_import_button),
                Some(profile_copy_button),
                self.resolve_mod_rid.is_none() && self.pending_group_files == 0,
            );
            if profile_toolbar_changed || group_changed {
                self.state.mod_data.save().unwrap();
                self.edit_history.reset(&self.state.mod_data, self.state.mod_data.revision());
            }
            if let Some(message) = group_success {
                self.last_action = Some(LastAction::success(message));
            }
            if let Some(message) = imported_profile {
                self.last_action = Some(LastAction::success(message));
            }
            if let Some(result) = copy_feedback {
                self.last_action = Some(match result {
                    Ok(message) => LastAction::success(message),
                    Err(message) => LastAction::failure(message),
                });
            }

            ui.separator();

            let mut open_local_folder_import = false;
            ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                if self.resolve_mod_rid.is_some() {
                    ui.spinner();
                }
                if ui
                    .add_enabled(
                        self.state.config.local_mod_folder.is_some() && self.resolve_mod_rid.is_none(),
                        egui::Button::new("Import .pak/.zip"),
                    )
                    .on_hover_text("Scan the local mods folder configured in Settings; manage exclusions in the import dialog")
                    .clicked()
                {
                    open_local_folder_import = true;
                }
                ui.with_layout(ui.layout().with_main_justify(true), |ui| {
                    // define multiline layouter to be able to show multiple lines in a single line widget
                    let font_id = FontSelection::default().resolve(ui.style());
                    let text_color = ui.visuals().widgets.inactive.text_color();
                    let mut multiline_layouter = move |ui: &Ui, text: &str, wrap_width: f32| {
                        let layout_job = LayoutJob::simple(
                            text.to_string(),
                            font_id.clone(),
                            text_color,
                            wrap_width,
                        );
                        ui.fonts(|f| f.layout_job(layout_job))
                    };

                    let resolve = ui.add_enabled(
                        self.resolve_mod_rid.is_none(),
                        egui::TextEdit::singleline(&mut self.resolve_mod)
                            .layouter(&mut multiline_layouter)
                            .hint_text("Add mod..."),
                    );
                    if is_committed(&resolve) {
                        if looks_like_structured_json(&self.resolve_mod) {
                            self.last_action = Some(LastAction::failure(
                                "JSON cannot be added as mod URLs. Use Profiles > Import Profile (JSON).".into(),
                            ));
                        } else {
                            message::ResolveMods::send(self, ctx, self.parse_mods(), false);
                            self.problematic_mod_id = None;
                        }
                    }
                });
            });

            if open_local_folder_import
                && let Some(folder) = self.state.config.local_mod_folder.clone() {
                    folder_import::open_preview(self, ctx, folder);
                }
            let profile = self.state.mod_data.active_profile.clone();

            ui.horizontal(|ui| {
                let (mut sort_category, mut is_ascending) = self
                    .get_sorting_config()
                    .map(|c| (Some(c.sort_category), c.is_ascending))
                    .unwrap_or_default();

                let selected_text = match sort_category {
                    None => "Manual".to_owned(),
                    Some(category) => format!(
                        "{} {}",
                        category.as_str(),
                        if is_ascending { "⏶" } else { "⏷" },
                    ),
                };

                let mut clicked = false;
                egui::ComboBox::from_id_salt("mod_sort_by")
                    .selected_text(format!("Sort by: {selected_text}"))
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        if ui.radio_value(&mut sort_category, None, "Manual").clicked() {
                            clicked = true;
                        }
                        for category in SortBy::iter() {
                            let mut radio_label = category.as_str().to_owned();
                            if sort_category == Some(category) {
                                radio_label.push_str(if is_ascending { " ⏶" } else { " ⏷" });
                            }
                            let resp = ui.radio_value(&mut sort_category, Some(category), radio_label);
                            if resp.clicked() {
                                clicked = true;
                                if resp.changed() {
                                    is_ascending = true;
                                } else {
                                    // Clicking the active category reverses its direction.
                                    is_ascending = !is_ascending;
                                }
                            }
                        }
                    });

                if clicked {
                    self.update_sorting_config(sort_category, is_ascending);
                }

                // Keep history controls beside sorting while preserving their shortcuts.
                let history_enabled = self.resolve_mod_rid.is_none()
                    && self.integrate_rid.is_none()
                    && self.update_rid.is_none()
                    && self.lint_rid.is_none();
                let undo_image = egui::Image::new(egui::include_image!("../../../assets/icons/undo.svg"))
                    .fit_to_exact_size(egui::vec2(17.0, 17.0))
                    .tint(ui.visuals().text_color());
                if ui.add_enabled(
                    history_enabled && self.edit_history.can_undo(),
                    egui::Button::image(undo_image),
                )
                .on_hover_text("Undo mod-list change (Ctrl+Z)")
                .clicked() {
                    self.apply_mod_history(false);
                }
                let redo_image = egui::Image::new(egui::include_image!("../../../assets/icons/redo.svg"))
                    .fit_to_exact_size(egui::vec2(17.0, 17.0))
                    .tint(ui.visuals().text_color());
                if ui.add_enabled(
                    history_enabled && self.edit_history.can_redo(),
                    egui::Button::image(redo_image),
                )
                .on_hover_text("Redo mod-list change (Ctrl+Y / Ctrl+Shift+Z)")
                .clicked() {
                    self.apply_mod_history(true);
                }

                ui.add_space(16.);
                // TODO: actually implement mod groups.
                let search_string = &mut self.search_string;
                let lower = search_string.to_lowercase();
                let any_matches = self.state.mod_data.any_mod(&profile, |mc, _| {
                    self.state
                        .store
                        .get_mod_info(&mc.spec)
                        .map(|i| i.name.to_lowercase().contains(&lower))
                        .unwrap_or(false)
                });

                let mut text_edit = egui::TextEdit::singleline(search_string).hint_text("Search");
                if !any_matches {
                    text_edit = text_edit.text_color(ui.visuals().error_fg_color);
                }
                let res = ui
                    .scope_builder(
                        egui::UiBuilder::new().layout(egui::Layout::bottom_up(Align::RIGHT)),
                        |ui| ui.add(text_edit),
                    )
                    .inner;
                if res.changed() {
                    self.scroll_to_match = true;
                }
                if res.lost_focus()
                    && ui.input(|i| {
                        i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape)
                    })
                {
                    search_string.clear();
                    self.scroll_to_match = false;
                } else if self.focus_search {
                    res.request_focus();
                    self.focus_search = false;
                }
            });
            ui.add_space(4.);

            Drag::cancel_if_invalid(ctx, &profile, self.get_sorting_config().is_none());
            let file_group = self.ui_profile(ui, &profile);

            // must access memory outside of input lock to prevent deadlock
            let is_anything_focused = ctx.memory(|m| m.focused().is_some());
            ctx.input(|i| {
                if !i.raw.dropped_files.is_empty()
                    && self.integrate_rid.is_none()
                    && self.update_rid.is_none()
                {
                    if let Some((group, prepend)) = &file_group {
                        let paths = i
                            .raw
                            .dropped_files
                            .iter()
                            .filter_map(|f| f.path.clone())
                            .collect();
                        group_controls::drop_files(
                            self,
                            ctx,
                            profile.clone(),
                            group.clone(),
                            *prepend,
                            paths,
                        );
                    } else {
                        let mut mods = String::new();
                        for f in i
                            .raw
                            .dropped_files
                            .iter()
                            .filter_map(|f| f.path.as_ref().map(|p| p.to_string_lossy()))
                        {
                            mods.push_str(&f);
                            mods.push('\n');
                        }

                        self.resolve_mod = mods.trim().to_string();
                        message::ResolveMods::send(self, ctx, self.parse_mods(), false);
                        self.problematic_mod_id = None;
                    }
                }
                for e in &i.events {
                    match e {
                        egui::Event::Paste(s) => {
                            if self.integrate_rid.is_none()
                                && self.update_rid.is_none()
                                && self.lint_rid.is_none()
                                && !is_anything_focused
                            {
                                // A global paste with no focused text field is a mod-list shortcut.
                                // Never feed profile/group JSON keys (such as `id`) to mod providers.
                                if looks_like_structured_json(s) {
                                    self.last_action = Some(LastAction::failure(
                                        "Profile JSON detected. Open Profiles > Import Profile (JSON), click its text field, then paste.".into(),
                                    ));
                                } else {
                                    self.resolve_mod = s.trim().to_string();
                                    message::ResolveMods::send(self, ctx, self.parse_mods(), false);
                                }
                            }
                        }
                        egui::Event::Text(text)
                            if !is_anything_focused && !text.trim().is_empty() => {
                                self.search_string = text.to_string();
                                self.scroll_to_match = true;
                                self.focus_search = true;
                            }
                        _ => {}
                    }
                }
            });
        });
        self.show_group_mod_removal_confirmation(ctx);
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.edit_history
                .sync(&self.state.mod_data, self.state.mod_data.revision());
        }
    }
}
