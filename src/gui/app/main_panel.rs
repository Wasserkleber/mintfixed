// Main frame concern extracted from app/frame.rs; original statements and ordering preserved.
use super::*;

impl App {
    pub(super) fn frame_main_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.integrate_rid.is_some() || self.update_rid.is_some() || self.lint_rid.is_some()
            {
                ui.disable();
            }
            // Reserve a background shape before the controls so it paints behind
            // them without introducing another UI scope or changing widget IDs.
            let toolbar_bounds = ui.available_rect_before_wrap();
            let toolbar_background = ui.painter().add(egui::Shape::Noop);
            let panel_fill = ui.visuals().panel_fill;
            let text_color = ui.visuals().text_color();
            let blend = |base: u8, text: u8| ((9 * u16::from(base) + u16::from(text)) / 10) as u8;
            let toolbar_fill = egui::Color32::from_rgb(
                blend(panel_fill.r(), text_color.r()),
                blend(panel_fill.g(), text_color.g()),
                blend(panel_fill.b(), text_color.b()),
            );
            ui.add_space(4.0);

            // profile selection

            let mut group_changed = false;
            let mut group_success = None;
            let configured_group_mod_folder = self.state.config.local_mod_folder.clone();
            let buttons = |ui: &mut Ui, mod_data: &mut ModData| {
                (group_changed, group_success) = group_controls::controls(
                    ui,
                    mod_data,
                    &mut self.new_group_name,
                    self.pending_group_files > 0,
                    configured_group_mod_folder.as_deref(),
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
            let profile_mod_folder = self.state.config.local_mod_folder.clone();
            let profile_import_button = |ui: &mut Ui, mod_data: &mut ModData, modified: &mut bool| {
                imported_profile = profile_import::ui_inline(ui, mod_data, modified, profile_mod_folder.as_deref());
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
                Drag::clear_history_selection(ctx);
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

            let mut open_local_folder_import = false;
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                egui::Layout::right_to_left(Align::Center),
                |ui| {
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
                    self.apply_mod_history(ui.ctx(), false);
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
                    self.apply_mod_history(ui.ctx(), true);
                }

                let selecting = self.get_sorting_config().is_none()
                    && Drag::selection_mode(ui.ctx(), &profile);
                // Match the 17 px history icons plus native button padding.
                let select_height = (17.0 + 2.0 * ui.spacing().button_padding.y)
                    .max(ui.spacing().interact_size.y);
                let select_button = mint_text_button(if selecting { "Exit" } else { "Select" })
                    .min_size(egui::vec2(44.0, select_height));
                if ui
                    .add_enabled(history_enabled, select_button)
                    .on_hover_text("Select multiple mods. Ctrl+click toggles; Shift+click selects a range. Esc exits. Switches to Manual sorting when needed.")
                    .clicked()
                {
                    if !selecting && self.get_sorting_config().is_some() {
                        self.update_sorting_config(None, false);
                    }
                    Drag::set_selection_mode(ui.ctx(), &profile, !selecting);
                }

                ui.add_space(16.);
                // TODO: actually implement mod groups.
                let search_string = &mut self.search_string;
                let show_clear = !search_string.is_empty();
                let lower = search_string.to_lowercase();
                let any_matches = self.state.mod_data.any_mod(&profile, |mc, _| {
                    self.state
                        .store
                        .get_mod_info(&mc.spec)
                        .map(|i| i.name.to_lowercase().contains(&lower))
                        .unwrap_or(false)
                });

                let (res, clear_clicked) =
                    crate::gui::search_field::show(ui, search_string, any_matches, show_clear);
                if clear_clicked {
                    search_string.clear();
                    self.scroll_to_match = false;
                    res.request_focus();
                } else if res.changed() {
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
            let toolbar_rect = egui::Rect::from_min_max(
                egui::pos2(toolbar_bounds.left() - 4.0, toolbar_bounds.top()),
                egui::pos2(toolbar_bounds.right() + 4.0, ui.cursor().top()),
            );
            ui.painter().set(
                toolbar_background,
                egui::Shape::rect_filled(toolbar_rect, egui::CornerRadius::same(4), toolbar_fill),
            );
            // A restrained divider and breathing room before the scrollable mod list.
            ui.painter().line_segment(
                [
                    egui::pos2(toolbar_bounds.left(), toolbar_rect.bottom()),
                    egui::pos2(toolbar_bounds.right(), toolbar_rect.bottom()),
                ],
                egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
            );
            ui.add_space(6.0);

            Drag::cancel_if_invalid(ctx, &profile, self.get_sorting_config().is_none());
            self.unavailable_overview_button(ui);
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
    }
}
