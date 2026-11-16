// GUI profile rows and group rendering. Extracted without changing behavior.
use super::*;

impl App {
    pub(super) fn ui_profile(&mut self, ui: &mut Ui, profile: &str) -> Option<(String, bool)> {
        if egui::DragAndDrop::payload::<Drag>(ui.ctx()).is_some() {
            ui.style_mut().interaction.selectable_labels = false;
        }
        let profile_name = profile;
        let mut sorting_config = self.get_sorting_config();
        if sorting_config.is_some() && ui.button("Switch to Manual to move mods").clicked() {
            self.update_sorting_config(None, false);
            sorting_config = None;
        }

        let revision = (
            self.state.mod_data.revision(),
            self.state.store.metadata_revision(),
        );
        if self.profile_issues.get(profile).map(|entry| entry.0) != Some(revision) {
            let issues = crate::mod_identity::ProfileIssues::collect(
                &self.state.mod_data,
                profile,
                &self.state.store,
            );
            self.profile_issues
                .insert(profile.to_owned(), (revision, issues));
        }
        let issues = &self.profile_issues[profile].1;
        for warning in &issues.versions {
            ui.colored_label(ui.visuals().warn_fg_color, warning);
        }
        if !issues.pending.is_empty() {
            ui.label(format!(
                "{} mod.io entries awaiting version metadata",
                issues.pending.len()
            ))
            .on_hover_text(issues.pending.join("\n"));
        }

        let ModData {
            profiles, groups, ..
        } = self.state.mod_data.deref_mut().deref_mut();

        struct Ctx {
            needs_save: bool,
            scroll_to_match: bool,
            btn_remove: Option<(Option<String>, usize)>,
            add_deps: Option<Vec<ModSpecification>>,
            drag: Option<(Drag, Target)>,
            file_group: Option<(String, bool)>,
        }
        let mut ctx = Ctx {
            needs_save: false,
            scroll_to_match: self.scroll_to_match,
            btn_remove: None,
            add_deps: None,
            drag: None,
            file_group: None,
        };

        let mut ui_profile = |ui: &mut Ui, profile: &mut ModProfile| {
            let enabled_specs = profile
                .mods
                .iter()
                .enumerate()
                .flat_map(|(i, m)| -> Box<dyn Iterator<Item = _>> {
                    match m {
                        ModOrGroup::Individual(mc) => Box::new(
                            mc.enabled
                                .then_some(((None, i), mc.spec.clone()))
                                .into_iter(),
                        ),
                        ModOrGroup::Group {
                            group_name,
                            enabled,
                            member_overrides,
                            ..
                        } => Box::new(
                            enabled
                                .then(|| groups.get(group_name))
                                .flatten()
                                .into_iter()
                                .flat_map(move |g| {
                                    g.mods.iter().enumerate().filter_map(move |(index, mc)| {
                                        member_overrides.enabled(mc).then_some((
                                            (Some(group_name.clone()), index),
                                            mc.spec.clone(),
                                        ))
                                    })
                                }),
                        ),
                    }
                })
                .collect::<Vec<_>>();

            let group_tint_snapshot: std::collections::BTreeMap<String, Option<String>> = groups
                .iter()
                .map(|(name, group)| (name.clone(), group.tint_color.clone()))
                .collect();

            let mut ui_mod =
                |ctx: &mut Ctx,
                 ui: &mut Ui,
                 group: Option<&str>,
                 row_index: usize,
                 mc: &mut ModConfig,
                 overrides: Option<&mut MemberOverrides>| {
                    let mut enabled = overrides.as_ref().map_or(mc.enabled, |o| o.enabled(mc));
                    let group_control_tint = group
                        .and_then(|name| group_tint_snapshot.get(name))
                        .and_then(|color| color.as_deref())
                        .map(|color| {
                            mod_list_style::swatch_color(Some(color), ui.visuals().dark_mode)
                        });
                    if !enabled {
                        let vis = ui.visuals_mut();
                        vis.override_text_color = Some(vis.text_color());
                        vis.hyperlink_color = vis.text_color();
                    }

                    if ui
                        .add(toggle_switch(&mut enabled))
                        .on_hover_text_at_pointer("Enabled?")
                        .changed()
                    {
                        if let Some(overrides) = overrides {
                            overrides.set(mc, enabled);
                        } else {
                            mc.enabled = enabled;
                        }
                        ctx.needs_save = true;
                    }

                    /*
                    if ui
                        .add(egui::Checkbox::without_text(&mut mc.required))
                        .changed()
                    {
                        needs_save = true;
                    }
                    */

                    if issues.duplicate(group, row_index)
                        && icon_buttons::warning_button(ui, group_control_tint)
                            .on_hover_text("Remove this duplicate entry")
                            .clicked()
                    {
                        ctx.btn_remove = Some((group.map(str::to_owned), row_index));
                    }

                    let info = self.state.store.get_mod_info(&mc.spec);
                    if self.state.store.is_modio_unavailable(&mc.spec) {
                        icon_buttons::warning_button(ui, group_control_tint)
                            .on_hover_text("Unavailable on mod.io. Cached metadata and downloaded mod files were preserved; other mod updates can continue.");
                    }
                    if let Some(ref info) = info
                        && let Some(modio_id) = info.modio_id
                        && self.problematic_mod_id.is_some_and(|id| id == modio_id)
                    {
                        let icon = egui::Button::new(RichText::new("❌").color(Color32::WHITE))
                            .fill(Color32::RED);
                        ui.add_enabled(false, icon);
                    }

                    if enabled && let Some(req) = &self.integrate_rid {
                        match req.state.get(&mc.spec) {
                            Some(SpecFetchProgress::Progress { progress, size }) => {
                                ui.add(
                                    egui::ProgressBar::new(*progress as f32 / *size as f32)
                                        .show_percentage()
                                        .desired_width(100.0),
                                );
                            }
                            Some(SpecFetchProgress::Complete) => {
                                ui.add(egui::ProgressBar::new(1.0).desired_width(100.0));
                            }
                            None => {
                                ui.spinner();
                            }
                        }
                    }

                    if let Some(info) = &info {
                        let previous_spec = mc.spec.clone();
                        ui.scope(|ui| {
                            mod_list_style::tint_group_member_controls(ui, group_control_tint);
                            egui::ComboBox::from_id_salt(row_index)
                                .selected_text(
                                    self.state
                                        .store
                                        .get_version_name(&mc.spec)
                                        .unwrap_or_default(),
                                )
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut mc.spec.url,
                                        info.spec.url.to_string(),
                                        self.state
                                            .store
                                            .get_version_name(&info.spec)
                                            .unwrap_or_default(),
                                    );
                                    for version in info.versions.iter().rev() {
                                        ui.selectable_value(
                                            &mut mc.spec.url,
                                            version.url.to_string(),
                                            self.state
                                                .store
                                                .get_version_name(version)
                                                .unwrap_or_default(),
                                        );
                                    }
                                });
                        });

                        ctx.needs_save |= mc.spec != previous_spec;

                        ui.scope(|ui| {
                            mod_list_style::tint_group_member_controls(ui, group_control_tint);
                        ui.style_mut().spacing.interact_size.x = 30.;
                        let dark = ui.visuals().dark_mode;
                        match priority_preview_value(ui, mc.priority).cmp(&0) {
                            std::cmp::Ordering::Less => {
                                ui.visuals_mut().override_text_color = Some(if dark {
                                    Color32::LIGHT_RED
                                } else {
                                    Color32::DARK_RED
                                });
                            }
                            std::cmp::Ordering::Greater => {
                                ui.visuals_mut().override_text_color = Some(if dark {
                                    Color32::LIGHT_GREEN
                                } else {
                                    Color32::DARK_GREEN
                                });
                            }
                            _ => {}
                        }
                        let previous_priority = mc.priority;
                        priority_drag_value(ui, &mut mc.priority)
                            .on_hover_text_at_pointer("Load Priority\nIn case of asset conflict, mods with higher priority take precedent.\nCan have duplicate values.",);
                        if mc.priority != previous_priority {
                            ctx.needs_save = true;
                        }
                    });

                        if icon_buttons::copy_url_button(ui, group_control_tint)
                            .on_hover_text_at_pointer("copy URL")
                            .clicked()
                        {
                            ui.ctx().copy_text(mc.spec.url.to_string());
                        }

                        if enabled {
                            let missing_deps = info
                                .suggested_dependencies
                                .iter()
                                .filter(|d| {
                                    !enabled_specs.iter().any(|(_, s)| s.satisfies_dependency(d))
                                })
                                .collect::<Vec<_>>();

                            if !missing_deps.is_empty() {
                                let mut msg = "Add missing dependencies:".to_string();
                                for dep in &missing_deps {
                                    msg.push('\n');
                                    msg.push_str(&dep.url);
                                }
                                if icon_buttons::warning_button(ui, group_control_tint)
                                    .on_hover_text(msg)
                                    .clicked()
                                {
                                    ctx.add_deps =
                                        Some(missing_deps.into_iter().cloned().collect());
                                }
                            }
                        }

                        match info.provider {
                            "modio" => {
                                let texture: &egui::TextureHandle =
                                    self.modio_texture_handle.get_or_insert_with(|| {
                                        let image =
                                            image::load_from_memory(MODIO_LOGO_PNG).unwrap();
                                        let size = [image.width() as _, image.height() as _];
                                        let image_buffer = image.to_rgba8();
                                        let pixels = image_buffer.as_flat_samples();
                                        let image = egui::ColorImage::from_rgba_unmultiplied(
                                            size,
                                            pixels.as_slice(),
                                        );

                                        ui.ctx().load_texture(
                                            "modio-logo",
                                            image,
                                            Default::default(),
                                        )
                                    });
                                let mut img = egui::Image::new(texture)
                                    .fit_to_exact_size([16.0, 16.0].into());
                                if !enabled {
                                    img = img.tint(Color32::LIGHT_RED);
                                }
                                ui.add(img);
                            }
                            "http" => {
                                ui.label("🌐");
                            }
                            "file" => {
                                icon_buttons::folder_badge(ui, group_control_tint);
                            }
                            _ => unimplemented!("unimplemented provider kind"),
                        }

                        let search = searchable_text(&info.name, &self.search_string, {
                            TextFormat {
                                color: ui.visuals().hyperlink_color,
                                ..Default::default()
                            }
                        });

                        let res = ui.hyperlink_to(search.job, &mc.spec.url);
                        if search.is_match && ctx.scroll_to_match {
                            res.scroll_to_me(None);
                            ctx.scroll_to_match = false;
                        }

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if let Some(description) = mc
                                .description
                                .as_deref()
                                .filter(|text| !text.trim().is_empty())
                            {
                                description_help_button(ui, description);
                            }
                            mod_tags::render_mod_tags(
                                ui,
                                info,
                                &self.search_string,
                                &mut ctx.scroll_to_match,
                            );
                        });
                    } else {
                        if icon_buttons::copy_url_button(ui, group_control_tint)
                            .on_hover_text_at_pointer("Copy URL")
                            .clicked()
                        {
                            ui.ctx().copy_text(mc.spec.url.to_string());
                        }

                        let search = searchable_text(&mc.spec.url, &self.search_string, {
                            TextFormat {
                                color: ui.visuals().hyperlink_color,
                                ..Default::default()
                            }
                        });

                        let res = ui.hyperlink_to(search.job, &mc.spec.url);
                        if search.is_match && ctx.scroll_to_match {
                            res.scroll_to_me(None);
                            ctx.scroll_to_match = false;
                        }
                        if let Some(description) = mc
                            .description
                            .as_deref()
                            .filter(|text| !text.trim().is_empty())
                        {
                            description_help_button(ui, description);
                        }
                    }
                };

            let mut ui_item = |ctx: &mut Ctx,
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
                        ui.push_id(("mod", row_index), |ui| {
                            ui_mod(ctx, ui, None, row_index, mc, None);
                        });
                    }
                    ModOrGroup::Group {
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
                            let fully_closed = collapse.openness(ui.ctx()) <= 0.0;
                            // Tinted names are slightly desaturated for legibility on
                            // the existing colored header background in both themes.
                            let group_header_color = mod_list_style::group_header_text_color(
                                ui,
                                selected_group_tint.as_deref(),
                            );
                            // Highlight the group while its matching members are hidden.
                            let group_title = if fully_closed && matching_mod_in_group {
                                RichText::new(group_name.as_str())
                                    .strong()
                                    .background_color(Color32::YELLOW)
                                    .color(Color32::BLACK)
                            } else {
                                let mut title = RichText::new(group_name.as_str()).strong();
                                if let Some(color) = group_header_color {
                                    title = title.color(color);
                                }
                                title
                            };
                            // mintfixed: soft neutral text for uncolored group headers
                            let group_title = if selected_group_tint.is_none()
                                && !(fully_closed && matching_mod_in_group)
                            {
                                group_title.color(if ui.visuals().dark_mode {
                                    Color32::from_gray(164)
                                } else {
                                    Color32::from_gray(92)
                                })
                            } else {
                                group_title
                            };
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
                                title_clicked = ui
                                    .add(egui::Label::new(group_title).sense(egui::Sense::click()))
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked();
                                ui.label(
                                    RichText::new(format!(
                                        "{} mod{}",
                                        group_member_count,
                                        if group_member_count == 1 { "" } else { "s" }
                                    ))
                                    .small()
                                    .weak(),
                                );
                                // Reuse the orange SVG warning used by individual mods.
                                // Place it on the header so collapsed groups expose the issue.
                                if let Some(tooltip) = unavailable_group_tooltip.as_deref() {
                                    icon_buttons::warning_button(ui, None)
                                        .on_hover_text_at_pointer(tooltip);
                                }
                                let selected_color = groups
                                    .get(group_name)
                                    .and_then(|group| group.tint_color.clone());
                                let mut requested_color: Option<Option<String>> = None;
                                // group color control — right aligned in the remaining header width
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.scope(|ui| {
                                        ui.spacing_mut().button_padding = egui::vec2(2.0, 0.0);
                                        // Invisible at rest; highlight only when hovered or selected.
                                        ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                                        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                                        ui.menu_image_button(
                                            egui::Image::new(egui::include_image!("../../../assets/icons/group_palette.svg"))
                                                .fit_to_exact_size(egui::vec2(15.0, 15.0))
                                                .tint(mod_list_style::swatch_color(
                                                    selected_color.as_deref(),
                                                    ui.visuals().dark_mode,
                                                )),
                                            |ui| {
                                                if mod_list_style::color_menu_option(ui, None, "None", selected_color.is_none()) {
                                                    requested_color = Some(None);
                                                    ui.close_menu();
                                                }
                                                for &(key, label) in mod_list_style::COLOR_OPTIONS {
                                                    if mod_list_style::color_menu_option(
                                                        ui,
                                                        Some(key),
                                                        label,
                                                        selected_color.as_deref() == Some(key),
                                                    ) {
                                                        requested_color = Some(Some(key.to_owned()));
                                                        ui.close_menu();
                                                    }
                                                }
                                            },
                                        )
                                        .response
                                        .on_hover_text("Group color");
                                    });
                                    if let Some(description) = group_description.as_deref() {
                                        description_help_button(ui, description);
                                    }
                                });
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
                        // Give the group header a full-width drop target, but leave
                        // narrow bands for inserting a mod *outside* the group.
                        let mut header_row = header_response.rect;
                        header_row.min.x = ui.max_rect().left();
                        header_row.max.x = ui.max_rect().right();
                        let edge = 5.0_f32.min(header_row.height() * 0.2);
                        // An expanded group's members immediately follow the header,
                        // so the bottom edge should drop into the group, not after it.
                        let bottom_edge = if fully_closed { edge } else { 0.0 };
                        let mut header_drop_area = header_row;
                        header_drop_area.min.y += edge;
                        header_drop_area.max.y -= bottom_edge;
                        let above_group = egui::Rect::from_min_max(
                            header_row.min,
                            egui::pos2(header_row.max.x, header_drop_area.min.y),
                        );
                        let below_group = egui::Rect::from_min_max(
                            egui::pos2(header_row.min.x, header_drop_area.max.y),
                            header_row.max,
                        );
                        if group_controls::hover_open(
                            ui,
                            &header_response,
                            header_drop_area,
                            fully_closed,
                            profile_name,
                            sorting_config.is_none(),
                        ) {
                            self.groups_to_open
                                .insert((profile_name.to_owned(), group_name.clone()));
                            ui.ctx().request_repaint();
                        }
                        let mut area = header_drop_area;
                        if let Some(body) = &body_response {
                            area = area.union(body.rect);
                        }
                        if ui.rect_contains_pointer(area) {
                            ctx.file_group = Some((group_name.clone(), fully_closed));
                        }
                        let over_group_header = ui.rect_contains_pointer(header_drop_area);
                        let over_above_group = ui.rect_contains_pointer(above_group);
                        let over_below_group =
                            fully_closed && ui.rect_contains_pointer(below_group);
                        let dragging_mod = egui::DragAndDrop::payload::<Drag>(ui.ctx())
                            .is_some_and(|d| !d.is_group);
                        // Highlight the destination without drawing lines over the
                        // group name. A slim rounded bar indicates a profile-level
                        // insertion; a soft tint indicates a drop into the group.
                        let accent = ui.visuals().selection.stroke.color;
                        let group_fill =
                            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 42);
                        if sorting_config.is_none() && dragging_mod {
                            if over_above_group || over_below_group {
                                let y = if over_above_group {
                                    above_group.center().y
                                } else {
                                    below_group.center().y
                                };
                                let marker = egui::Rect::from_center_size(
                                    egui::pos2(header_row.center().x, y),
                                    egui::vec2((header_row.width() - 16.0).max(0.0), 3.0),
                                );
                                ui.painter().rect_filled(
                                    marker,
                                    egui::CornerRadius::same(2),
                                    accent,
                                );
                            } else if over_group_header {
                                ui.painter().rect_filled(
                                    header_drop_area.shrink2(egui::vec2(3.0, 0.0)),
                                    egui::CornerRadius::same(4),
                                    group_fill,
                                );
                            }
                        } else if ctx
                            .file_group
                            .as_ref()
                            .is_some_and(|(name, _)| name == group_name)
                            && ui.input(|i| !i.raw.hovered_files.is_empty())
                        {
                            // Use the same drop-into-group feedback for local .pak files.
                            ui.painter().rect_filled(
                                header_drop_area.shrink2(egui::vec2(3.0, 0.0)),
                                egui::CornerRadius::same(4),
                                group_fill,
                            );
                        }
                        if sorting_config.is_none()
                            && dragging_mod
                            && (over_above_group || over_group_header || over_below_group)
                            && ui.input(|i| i.pointer.any_released())
                            && ctx.drag.is_none()
                            && let Some(source) = egui::DragAndDrop::payload::<Drag>(ui.ctx())
                        {
                            let target = if over_above_group {
                                Target {
                                    group: None,
                                    slot: row_index,
                                }
                            } else if over_below_group {
                                Target {
                                    group: None,
                                    slot: row_index + 1,
                                }
                            } else {
                                // Dropping on the header always prepends to the group,
                                // whether it was already open or expanded on hover.
                                Target {
                                    group: Some(group_name.clone()),
                                    slot: 0,
                                }
                            };
                            ctx.drag = Some(((*source).clone(), target));
                        }
                    }
                }
            };

            if sorting_config.is_some() {
                let order = sorted_mod_indices(
                    None,
                    profile.mods.iter().map(|item| match item {
                        ModOrGroup::Individual(mc) => Some(mc),
                        ModOrGroup::Group { .. } => None,
                    }),
                    sorting_config.as_ref(),
                    |spec| self.state.store.get_mod_info(spec),
                );
                let mut previous_group_row_tint: Option<&str> = None;
                let mut matching_color_run_offset: usize = 0;
                for (visual_index, store_index) in order.into_iter().enumerate() {
                    // Compare only consecutive visible rows, not unrelated groups.
                    let current_group_row_tint = match &profile.mods[store_index] {
                        ModOrGroup::Group { group_name, .. } => group_tint_snapshot
                            .get(group_name)
                            .and_then(|tint| tint.as_deref()),
                        _ => None,
                    };
                    let same_color_as_previous = mod_list_style::same_selected_color(
                        previous_group_row_tint,
                        current_group_row_tint,
                    );
                    matching_color_run_offset = if same_color_as_previous {
                        matching_color_run_offset + 1
                    } else {
                        0
                    };
                    let stripe_matching_color =
                        current_group_row_tint.is_some() && matching_color_run_offset % 2 == 1;
                    previous_group_row_tint = current_group_row_tint;

                    if matches!(&profile.mods[store_index], ModOrGroup::Individual(mc) if bitfix_menu::is_lua_spec(&mc.spec))
                    {
                        continue;
                    }
                    let frame = match &profile.mods[store_index] {
                        ModOrGroup::Group { group_name, .. } => mod_list_style::row_frame(
                            ui,
                            visual_index,
                            true,
                            group_tint_snapshot
                                .get(group_name)
                                .and_then(|tint| tint.as_deref()),
                            false,
                            stripe_matching_color,
                        ),
                        _ => mod_list_style::row_frame(ui, visual_index, false, None, false, false),
                    };
                    frame.show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui_item(&mut ctx, ui, &mut profile.mods[store_index], store_index);
                        });
                    });
                }
            } else {
                let mut previous_group_row_tint: Option<&str> = None;
                let mut matching_color_run_offset: usize = 0;
                for (index, item) in profile.mods.iter_mut().enumerate() {
                    // Compare only consecutive visible rows, not unrelated groups.
                    let current_group_row_tint = match &*item {
                        ModOrGroup::Group { group_name, .. } => group_tint_snapshot
                            .get(group_name)
                            .and_then(|tint| tint.as_deref()),
                        _ => None,
                    };
                    let same_color_as_previous = mod_list_style::same_selected_color(
                        previous_group_row_tint,
                        current_group_row_tint,
                    );
                    matching_color_run_offset = if same_color_as_previous {
                        matching_color_run_offset + 1
                    } else {
                        0
                    };
                    let stripe_matching_color =
                        current_group_row_tint.is_some() && matching_color_run_offset % 2 == 1;
                    previous_group_row_tint = current_group_row_tint;

                    if matches!(&*item, ModOrGroup::Individual(mc) if bitfix_menu::is_lua_spec(&mc.spec))
                    {
                        continue;
                    }
                    let frame = match item {
                        ModOrGroup::Group { group_name, .. } => mod_list_style::row_frame(
                            ui,
                            index,
                            true,
                            group_tint_snapshot
                                .get(group_name)
                                .and_then(|tint| tint.as_deref()),
                            false,
                            stripe_matching_color,
                        ),
                        _ => mod_list_style::row_frame(ui, index, false, None, false, false),
                    };
                    let row = Drag::paint_row(ui, profile_name, None, index, |ui| {
                        frame.show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (key, is_group) = match item {
                                    ModOrGroup::Individual(mc) => (mc.id.as_str(), false),
                                    ModOrGroup::Group { id, .. } => (id.as_str(), true),
                                };
                                Drag::handle(ui, profile_name, None, index, key, is_group);
                                ui_item(&mut ctx, ui, item, index);
                            });
                        })
                    });
                    // The group header already handles its three drop zones.
                    // Do not paint a second, competing profile-row marker on top
                    // of its highlight. Still handle gaps between rows and group drags.
                    let mut group_row_area = row.response.rect;
                    group_row_area.min.x = ui.max_rect().left();
                    group_row_area.max.x = ui.max_rect().right();
                    let group_handles_mod_drop = matches!(item, ModOrGroup::Group { .. })
                        && egui::DragAndDrop::payload::<Drag>(ui.ctx())
                            .is_some_and(|drag| !drag.is_group)
                        && ui.rect_contains_pointer(group_row_area);
                    if !group_handles_mod_drop {
                        group_controls::row_target(ui, &row.response, &mut ctx.drag, None, index);
                    }
                }
                // Treat all visible empty space below the last row as a top-level
                // insertion target. A single label only provided one row of hit area,
                // making it difficult to drag a mod out of the last expanded group.
                if sorting_config.is_none()
                    && ctx.drag.is_none()
                    && let Some(source) = egui::DragAndDrop::payload::<Drag>(ui.ctx())
                {
                    let viewport = ui.clip_rect();
                    let top = ui.cursor().top().max(viewport.top());
                    if top < viewport.bottom() {
                        let end_area = egui::Rect::from_min_max(
                            egui::pos2(viewport.left(), top),
                            viewport.right_bottom(),
                        );
                        if ui.rect_contains_pointer(end_area) {
                            // Keep the insertion indicator at the end of the list,
                            // regardless of how far into the blank area we hover.
                            let marker = egui::Rect::from_center_size(
                                egui::pos2(end_area.center().x, top),
                                egui::vec2((end_area.width() - 16.0).max(0.0), 3.0),
                            );
                            ui.painter().rect_filled(
                                marker,
                                egui::CornerRadius::same(2),
                                ui.visuals().selection.stroke.color,
                            );
                            if ui.input(|input| input.pointer.any_released()) {
                                ctx.drag = Some((
                                    (*source).clone(),
                                    Target {
                                        group: None,
                                        slot: profile.mods.len(),
                                    },
                                ));
                            }
                        }
                    }
                }
            }
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            group_controls::auto_scroll(ui, profile_name, sorting_config.is_none());
            if let Some(profile) = profiles.get_mut(profile) {
                ui_profile(ui, profile);
            } else {
                ui.label("no such profile");
            }
        });

        if let Some((target, index)) = ctx.btn_remove {
            if let Some(name) = target {
                // The duplicate-entry warning uses this path too: always confirm.
                if let Some(mc) = groups.get(&name).and_then(|group| group.mods.get(index)) {
                    self.pending_group_mod_removal = Some(PendingGroupModRemoval {
                        group_name: name,
                        mod_id: mc.id.clone(),
                        mod_name: self
                            .state
                            .store
                            .get_mod_info(&mc.spec)
                            .map(|info| info.name)
                            .unwrap_or_else(|| mc.spec.url.clone()),
                    });
                }
            } else {
                profiles.get_mut(profile).unwrap().mods.remove(index);
                ctx.needs_save = true;
            }
        }

        if let Some((source, dest)) = ctx.drag {
            let before = self.state.mod_data.deref().deref().clone();
            let destination_group = dest.group.clone();
            match source.apply(&mut self.state.mod_data, profile, dest, &self.state.store) {
                Ok(()) => {
                    if let Some(name) = destination_group {
                        self.groups_to_open.insert((profile.to_owned(), name));
                        ui.ctx().request_repaint();
                    }
                    match self.state.mod_data.save() {
                        Ok(()) => {
                            ctx.needs_save = false;
                        }
                        Err(error) => {
                            *self.state.mod_data.deref_mut().deref_mut() = before;
                            self.last_action =
                                Some(LastAction::failure(format!("Could not save move: {error}")));
                        }
                    }
                }
                Err(error) => self.last_action = Some(LastAction::failure(error.into())),
            }
        }

        if let Some(add_deps) = ctx.add_deps {
            message::ResolveMods::send(self, ui.ctx(), add_deps, true);
            self.problematic_mod_id = None;
        }

        self.scroll_to_match = ctx.scroll_to_match;

        if ctx.needs_save {
            self.state.mod_data.save().unwrap();
        }
        ctx.file_group
    }
}
