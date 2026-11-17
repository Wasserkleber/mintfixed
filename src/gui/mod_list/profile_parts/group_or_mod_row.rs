|ctx: &mut Ctx,
                               ui: &mut Ui,
                               mc: &mut ModOrGroup,
                               row_index: usize| {
                ui.scope(|ui| {
                    // Color group rows just like the group header, but keep standalone mods neutral.
                    let delete_accent = match &*mc {
                        ModOrGroup::Group { group_name, .. } => group_tint_snapshot
                            .get(group_name.as_str())
                            .and_then(|color| color.as_deref())
                            .map(|color| {
                                mod_list_style::swatch_color(Some(color), ui.visuals().dark_mode)
                            }),
                        _ => None,
                    };

                    ui.visuals_mut().widgets.hovered.weak_bg_fill = colors::DARK_RED;
                    ui.visuals_mut().widgets.active.weak_bg_fill = colors::DARKER_RED;
                    if icon_buttons::delete_button(ui, delete_accent)
                        .on_hover_text_at_pointer("Delete mod")
                        .clicked()
                    {
                        ctx.btn_remove = Some((None, row_index));
                    };
                });

                match mc {
                    ModOrGroup::Individual(mc) => {
                        let row = ui.push_id(("mod", row_index), |ui| {
                            ui_mod(ctx, ui, None, row_index, mc, None);
                        });
                        if ctx.show_target.as_ref().is_some_and(|t| t.group_entry_id.is_none() && t.mod_id == mc.id) {
                            row.response.scroll_to_me(Some(egui::Align::Center));
                            ctx.show_target = None;
                        }
                    }
                    ModOrGroup::Group {
                        id: group_entry_id,
                        group_name,
                        enabled,
                        member_overrides,
                        ..
                    } => {
                        if ui
                            .add(toggle_switch(enabled))
                            .on_hover_text_at_pointer("Enabled?")
                            .changed()
                        {
                            ctx.needs_save = true;
                        }
                        let open = !self.groups_to_open.is_empty()
                            && self
                                .groups_to_open
                                .remove(&(profile_name.to_owned(), group_name.clone()));
                        let selected_group_tint = groups
                            .get(group_name)
                            .and_then(|group| group.tint_color.clone());
                        let group_description = groups
                            .get(group_name)
                            .and_then(|group| group.description.clone())
                            .filter(|text| !text.trim().is_empty());
                        let group_member_count =
                            groups.get(group_name).map_or(0, |group| group.mods.len());
                        // A shared group may be collapsed or disabled: check every member.
                        let unavailable_member_names = groups
                            .get(group_name)
                            .map(|group| {
                                unavailable_group_mod_names(
                                    &group.mods,
                                    |spec| self.state.store.is_modio_unavailable(spec),
                                    |spec| {
                                        self.state
                                            .store
                                            .get_mod_info(spec)
                                            .map(|info| info.name)
                                            .unwrap_or_else(|| spec.url.clone())
                                    },
                                )
                            })
                            .unwrap_or_default();
                        let unavailable_group_tooltip = (!unavailable_member_names.is_empty())
                            .then(|| {
                                format!(
                                    "{} unavailable mod{} in this group:\n{}",
                                    unavailable_member_names.len(),
                                    if unavailable_member_names.len() == 1 {
                                        ""
                                    } else {
                                        "s"
                                    },
                                    unavailable_member_names.join("\n"),
                                )
                            });
                        // Match children even when the collapsing group does not render their rows.
                        let search_needle = self.search_string.to_lowercase();
                        let matching_mod_in_group = !search_needle.trim().is_empty()
                            && groups.get(group_name).is_some_and(|group| {
                                group.mods.iter().any(|mc| {
                                    let display_name = self
                                        .state
                                        .store
                                        .get_mod_info(&mc.spec)
                                        .map(|info| info.name)
                                        .unwrap_or_else(|| mc.spec.url.clone());
                                    display_name.to_lowercase().contains(&search_needle)
                                })
                            });
                        // egui::CollapsingState::body requires a vertical layout for indentation.
                        let (header_response, body_response, fully_closed) = ui.vertical(|ui| {
                            // Keep the group title clickable and render its help icon
                            // independently, on the same header row.
                            let mut collapse = egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                ui.make_persistent_id((profile_name, group_name.as_str())),
                                false,
                            );
                            if open {
                                collapse.set_open(true);
                            }
                            let fully_open = collapse.openness(ui.ctx()) >= 1.0;
                            let fully_closed = collapse.openness(ui.ctx()) <= 0.0;
                            // Tinted names are slightly desaturated for legibility on
                            // the existing colored header background in both themes.
                            let group_header_color = mod_list_style::group_header_text_color(
                                ui,
                                selected_group_tint.as_deref(),
                            );
                            let group_title = group_header::title(
                                ui,
                                group_name.as_str(),
                                group_header_color,
                                selected_group_tint.is_none(),
                                fully_closed && matching_mod_in_group,
                            );
                            let mut title_clicked = false;
                            // egui draws its built-in collapse arrow before the header
                            // callback, so tint its foreground only while painting this
                            // group header. Save/restore widget styling for the mod rows.
                            let previous_arrow_widgets = group_header_color.map(|color| {
                                let widgets = &mut ui.visuals_mut().widgets;
                                let previous = widgets.clone();
                                for state in [
                                    &mut widgets.inactive,
                                    &mut widgets.hovered,
                                    &mut widgets.active,
                                    &mut widgets.open,
                                ] {
                                    state.fg_stroke.color = color;
                                }
                                previous
                            });
                            let mut header = collapse.show_header(ui, |ui| {
                                let selected_color = groups
                                    .get(group_name)
                                    .and_then(|group| group.tint_color.clone());
                                let (clicked, requested_color) = group_header::render_contents(
                                    ui,
                                    group_title,
                                    group_member_count,
                                    unavailable_group_tooltip.as_deref(),
                                    selected_color.as_deref(),
                                    group_description.as_deref(),
                                );
                                title_clicked = clicked;
                                if let Some(color) = requested_color {
                                    if let Some(group) = groups.get_mut(group_name) {
                                        group.tint_color = color;
                                        ctx.needs_save = true;
                                        ui.ctx().request_repaint();
                                    }
                                }
                            });
                            if title_clicked {
                                header.toggle();
                            }
                            let (_toggle_response, header_contents, body_response) = header.body(|ui| {
                                // The tinted arrow must not recolor the group's mods.
                                if let Some(previous) = &previous_arrow_widgets {
                                    ui.visuals_mut().widgets = previous.clone();
                                }
                                let group = groups.get_mut(group_name).unwrap();
                                let order = sorted_mod_indices(
                                    Some(member_overrides),
                                    group.mods.iter().map(Some),
                                    sorting_config.as_ref(),
                                    |spec| self.state.store.get_mod_info(spec),
                                );
                                for (visual_index, index) in order.into_iter().enumerate() {
                                    if bitfix_menu::is_lua_spec(&group.mods[index].spec) {
                                        continue;
                                    }
                                    let member = Drag::paint_row(ui, profile_name, Some(group_name), index, |ui| {
                                        mod_list_style::row_frame(ui, visual_index, false, selected_group_tint.as_deref(), true, false)
                                            .show(ui, |ui| {
                                                ui.set_min_width(ui.available_width());
ui.push_id(("group", group_name.as_str(), index), |ui| {
                                        ui.horizontal(|ui| {
                                            if sorting_config.is_none() {
                                                Drag::handle(ui, profile_name, Some(group_name), index, &group.mods[index].id, false);
                                            }
                                            ui.scope(|ui| {
                                                // Match the shared group's selected color.
                                                let delete_accent = selected_group_tint.as_deref()
                                                    .map(|color| mod_list_style::swatch_color(Some(color), ui.visuals().dark_mode));

                                                ui.visuals_mut().widgets.hovered.weak_bg_fill = colors::DARK_RED;
                                                ui.visuals_mut().widgets.active.weak_bg_fill = colors::DARKER_RED;
                                                if icon_buttons::delete_button(ui, delete_accent)
                                                    .on_hover_text_at_pointer("Remove mod from shared group (confirmation required)")
                                                    .clicked()
                                                {
                                                    ctx.btn_remove = Some((Some(group_name.clone()), index));
                                                }
                                            });
                                            ui_mod(
                                                ctx,
                                                ui,
                                                Some(group_name),
                                                index,
                                                &mut group.mods[index],
                                                Some(member_overrides),
                                            );
                                        });
                                    })
                                            })
                                            .inner
                                    });
                                    // Wait for the expansion animation so scrolling uses the full row position.
                                    if fully_open && ctx.show_target.as_ref().is_some_and(|t|
                                        t.group_entry_id.as_ref() == Some(&*group_entry_id) && t.mod_id == group.mods[index].id) {
                                        member.response.scroll_to_me(Some(egui::Align::Center));
                                        ctx.show_target = None;
                                    }
                                    if sorting_config.is_none() {
                                        group_controls::row_target(ui, &member.response, &mut ctx.drag, Some(group_name), index);
                                    }
                                }
                            });
                            // Closed groups have no body, so restore the header UI too.
                            if let Some(previous) = previous_arrow_widgets {
                                ui.visuals_mut().widgets = previous;
                            }
                            let header_response = header_contents.response;
                            if fully_closed && matching_mod_in_group && ctx.scroll_to_match {
                                header_response.scroll_to_me(None);
                                ctx.scroll_to_match = false;
                            }
                            let body_response = body_response.map(|body| body.response);
                                (header_response, body_response, fully_closed)
                            }).inner;
                        group_drop_target::render_group_drop_target(
                            ui,
                            &header_response,
                            body_response.as_ref(),
                            fully_closed,
                            profile_name,
                            group_name.as_str(),
                            row_index,
                            sorting_config.is_none(),
                            &mut self.groups_to_open,
                            &mut ctx.file_group,
                            &mut ctx.drag,
                        );
                    }
                }
            }
