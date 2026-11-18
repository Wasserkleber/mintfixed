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

        Drag::begin_selection_frame(ui, profile_name, sorting_config.is_none());
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
            show_target: Option<unavailable_mods::ResolvedTarget>,
            scroll_to_match: bool,
            btn_remove: Option<(Option<String>, usize)>,
            add_deps: Option<Vec<ModSpecification>>,
            drag: Option<(Drag, Target)>,
            file_group: Option<(String, bool)>,
        }
        let mut ctx = Ctx {
            needs_save: false,
            show_target: self
                .show_mod_target
                .clone()
                .filter(|target| target.profile == profile_name),
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

            let mut ui_mod = include!("profile_parts/mod_row.rs");
            let mut ui_item = include!("profile_parts/group_or_mod_row.rs");
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
                    let frame = if matches!(&*item, ModOrGroup::Individual(mc)
                        if Drag::is_selected(ui.ctx(), profile_name, &mc.id))
                    {
                        mod_list_style::selected_row_frame(ui, frame)
                    } else {
                        frame
                    };
                    let drag_preview_key = match &*item {
                        ModOrGroup::Individual(mc) => mc.id.clone(),
                        ModOrGroup::Group { id, .. } => id.clone(),
                    };
                    let row =
                        Drag::paint_row(ui, profile_name, None, index, &drag_preview_key, |ui| {
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
            let selected_ids = Drag::dragged_mod_ids(ui.ctx(), &source);
            let batch_size = selected_ids.len();
            let result = if !source.is_group && batch_size > 1 {
                source.apply_batch(
                    &mut self.state.mod_data,
                    profile,
                    dest,
                    &self.state.store,
                    &selected_ids,
                )
            } else {
                source
                    .apply(&mut self.state.mod_data, profile, dest, &self.state.store)
                    .map(|_| (1, 0))
            };
            match result {
                Ok((moved, skipped)) => {
                    if skipped > 0 {
                        self.last_action = Some(LastAction::success(format!(
                            "Moved {moved} mods; skipped {skipped} duplicate(s)"
                        )));
                    }
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
        if self
            .show_mod_target
            .as_ref()
            .is_some_and(|target| target.profile == profile_name)
        {
            self.show_mod_target = ctx.show_target;
        }

        if ctx.needs_save {
            self.state.mod_data.save().unwrap();
        }
        ctx.file_group
    }
}
