impl Drag {

    // multi-preview-v3: preview every selected row, not just the grabbed row.
    // Row hitboxes and drop targets stay in their original scroll-list positions.
    pub fn paint_row<R>(
        ui: &mut Ui,
        profile: &str,
        group: Option<&str>,
        index: usize,
        key: &str,
        draw: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        let drag = egui::DragAndDrop::payload::<Self>(ui.ctx());
        let batch = drag.as_ref().and_then(|source| {
            if source.profile != profile || source.is_group {
                return None;
            }
            let ids = Self::dragged_mod_ids(ui.ctx(), source);
            if ids.len() < 2 {
                return None;
            }
            ids.iter()
                .position(|id| id == key)
                .map(|position| (position, ids.len(), source.key == key))
        });

        if let Some((position, count, is_source)) = batch {
            let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) else {
                return draw(ui);
            };
            // A unique layer per mod is essential: transforming a shared layer
            // repeatedly would shift previously drawn rows a second time.
            let layer = egui::LayerId::new(
                egui::Order::Tooltip,
                ui.id().with(("moving-row", profile, key)),
            );
            // mintfixed-multi-drag-scroll-clip-v1: the floating rows must not inherit the ScrollArea clip.
            // They are translated after painting, so the clip needs to contain
            // the ORIGINAL row even when it has scrolled outside the viewport.
            // Restrict this to the dedicated Tooltip layer, never the list UI.
            let painted = ui.scope_builder(
                egui::UiBuilder::new().layer_id(layer),
                |preview_ui| {
                    preview_ui.set_clip_rect(egui::Rect::EVERYTHING);
                    draw(preview_ui)
                },
            );
            let rect = painted.response.rect;
            // Pack preview rows at their real height, with only 1 px between them.
            // The row contents
            // (name, version, color and warnings) are still their actual UI.
            let stride = rect.height() + 1.0;
            let desired = egui::pos2(
                pointer.x - 22.0,
                pointer.y + 24.0 + position as f32 * stride,
            );
            ui.ctx().transform_layer_shapes(
                layer,
                egui::emath::TSTransform::from_translation(desired - rect.min),
            );
            if is_source {
                let counter_layer = egui::LayerId::new(
                    egui::Order::Tooltip,
                    ui.id().with(("moving-count", profile)),
                );
                ui.ctx().layer_painter(counter_layer).text(
                    pointer + egui::vec2(16.0, 4.0),
                    egui::Align2::LEFT_TOP,
                    format!("Moving {count} mods"),
                    egui::FontId::proportional(13.0),
                    ui.visuals().selection.stroke.color,
                );
            }
            return painted.inner;
        }

        // Keep the pre-existing single-mod and group drag behavior unchanged.
        let active = drag.is_some_and(|source| {
            source.profile == profile
                && source.group.as_deref() == group
                && source.index == index
                && source.key == key
        });
        if !active {
            return draw(ui);
        }
        let positions = ui.input(|i| Some((i.pointer.interact_pos()?, i.pointer.press_origin()?)));
        let Some((pointer, press_origin)) = positions else {
            return draw(ui);
        };
        let layer = egui::LayerId::new(egui::Order::Tooltip, ui.id().with("moving-row"));
        let painted = ui.scope_builder(egui::UiBuilder::new().layer_id(layer), draw);
        let current_row = painted.response.rect.min;
        let anchor_id = egui::Id::new(DRAG_PAINT_ANCHOR_ID);
        let initial_row = ui.ctx().data_mut(|data| {
            if let Some(anchor) = data.get_temp::<DragPaintAnchor>(anchor_id)
                && anchor.press_origin == press_origin {
                    return anchor.row_origin;
                }
            data.insert_temp(
                anchor_id,
                DragPaintAnchor {
                    press_origin,
                    row_origin: current_row,
                },
            );
            current_row
        });
        let delta = drag_paint_translation(pointer, press_origin, initial_row, current_row);
        ui.ctx().transform_layer_shapes(
            layer,
            egui::emath::TSTransform::from_translation(delta),
        );
        painted.inner
    }


    pub fn handle(
        ui: &mut Ui,
        profile: &str,
        group: Option<&str>,
        index: usize,
        key: &str,
        is_group: bool,
    ) {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(18.0, ui.spacing().interact_size.y),
            egui::Sense::click_and_drag(),
        );
        let center = rect.center();
        for offset in [-4.0, 0.0, 4.0] {
            ui.painter().hline(
                center.x - 5.0..=center.x + 5.0,
                center.y + offset,
                (1.5, ui.visuals().text_color()),
            );
        }
        let response = response.on_hover_cursor(egui::CursorIcon::Grab);

        // Multi-selection is controlled by the grip only. Existing drag
        // payload, row painting and boundary behavior remain unchanged.
        if !is_group {
            Self::register_visible(ui.ctx(), profile, key);
        }
        let selected = !is_group && Self::is_selected(ui.ctx(), profile, key);
        if selected {
            let marker = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 1.0, rect.top() + 3.0),
                egui::pos2(rect.left() + 3.5, rect.bottom() - 3.0),
            );
            let accent = if ui.visuals().dark_mode {
                egui::Color32::from_rgb(102, 164, 228)
            } else {
                egui::Color32::from_rgb(58, 117, 186)
            };
            ui.painter().rect_filled(marker, egui::CornerRadius::same(2), accent);
        }
        let response = if is_group {
            response
        } else {
            response.on_hover_text("Drag to move; Ctrl+click to select; Shift+click for a range. Use Select in the toolbar for checkboxes.")
        };
        if !is_group && response.clicked() {
            Self::selection_click(ui.ctx(), profile, key);
        }
        if response.drag_started() {
            Self::selection_drag_started(ui.ctx(), profile, key, is_group);
            // Reset the preview origin for every drag, even if it begins at the
            // same cursor coordinates as the previous drag.
            ui.ctx().data_mut(|data| {
                data.remove::<DragPaintAnchor>(egui::Id::new(DRAG_PAINT_ANCHOR_ID));
            });
            response.dnd_set_drag_payload(Self {
                profile: profile.into(),
                group: group.map(str::to_owned),
                index,
                key: key.into(),
                is_group,
            });
        }
    }

    pub fn cancel_if_invalid(ctx: &egui::Context, profile: &str, manual: bool) {
        if egui::DragAndDrop::payload::<Self>(ctx)
            .is_some_and(|drag| drag.profile != profile || !manual)
        {
            egui::DragAndDrop::clear_payload(ctx);
        }
    }

    pub fn apply(
        &self,
        data: &mut Data,
        profile: &str,
        dest: Target,
        store: &ModStore,
    ) -> Result<(), &'static str> {
        if self.profile != profile {
            return Err("Move mods within one profile at a time");
        }
        let list = &data
            .profiles
            .get(profile)
            .ok_or("The source profile no longer exists")?
            .mods;
        let member = match &self.group {
            Some(name) => data.groups.get(name).and_then(|g| g.mods.get(self.index)),
            None => match list.get(self.index) {
                Some(ModOrGroup::Individual(mc)) if !self.is_group => Some(mc),
                Some(ModOrGroup::Group { id, .. }) if self.is_group && *id == self.key => None,
                _ => return Err("The dragged row has changed"),
            },
        };
        if !self.is_group && member.is_none_or(|m| m.id != self.key) {
            return Err("The dragged mod has changed");
        }
        let same_list = self.group == dest.group;
        let slot = if let Some(name) = &dest.group {
            if self.is_group {
                return Err("Groups cannot contain groups");
            }
            let group = data
                .groups
                .get(name)
                .ok_or("The destination group no longer exists")?;
            if !same_list {
                let source = crate::mod_identity::Artifact::cached(store, &member.unwrap().spec);
                if group
                    .mods
                    .iter()
                    .any(|m| crate::mod_identity::Artifact::cached(store, &m.spec) == source)
                {
                    return Err("That mod is already in this group");
                }
            }
            dest.slot.min(group.mods.len())
        } else {
            if !same_list && !self.is_group {
                let source = crate::mod_identity::Artifact::cached(store, &member.unwrap().spec);
                if list.iter().any(|item| {
                    matches!(item, ModOrGroup::Individual(mc)
                    if crate::mod_identity::Artifact::cached(store, &mc.spec) == source)
                }) {
                    return Err("That mod already has a standalone entry");
                }
            }
            dest.slot.min(list.len())
        };
        // Resolve the originating reference before mutating any shared collection.
        let effective = if !same_list && !self.is_group {
            let mc = member.unwrap();
            let enabled = if let Some(source) = &self.group {
                let refs: Vec<_> = list
                    .iter()
                    .filter_map(|item| match item {
                        ModOrGroup::Group {
                            group_name,
                            enabled,
                            member_overrides,
                            ..
                        } if group_name == source => Some(*enabled && member_overrides.enabled(mc)),
                        _ => None,
                    })
                    .collect();
                if refs.len() != 1 {
                    return Err("Move requires one reference to the source group in this profile");
                }
                refs[0]
            } else {
                mc.enabled
            };
            if let Some(destination) = &dest.group {
                let refs: Vec<_> = list
                    .iter()
                    .filter_map(|item| match item {
                        ModOrGroup::Group {
                            group_name,
                            enabled,
                            ..
                        } if group_name == destination => Some(*enabled),
                        _ => None,
                    })
                    .collect();
                if refs.len() != 1 {
                    return Err(
                        "Move requires one reference to the destination group in this profile",
                    );
                }
                if enabled && !refs[0] {
                    return Err(
                        "Enable the destination group before moving an enabled mod into it",
                    );
                }
            }
            Some(enabled)
        } else {
            None
        };
        let slot = slot - usize::from(same_list && self.index < slot);
        if same_list && self.index == slot {
            return Ok(());
        }

        let mut item = if let Some(name) = &self.group {
            ModOrGroup::Individual(data.groups.get_mut(name).unwrap().mods.remove(self.index))
        } else {
            data.profiles
                .get_mut(profile)
                .unwrap()
                .mods
                .remove(self.index)
        };

        if let Some(effective) = effective
            && let ModOrGroup::Individual(mc) = &mut item
        {
            // Only removed source memberships lose overrides. Never copy one
            // profile's choices into another destination reference.
            if let (Some(source), Some(id)) = (&self.group, &mc.member_id) {
                for p in data.profiles.values_mut() {
                    for entry in &mut p.mods {
                        if let ModOrGroup::Group {
                            group_name,
                            member_overrides,
                            ..
                        } = entry
                            && group_name == source
                        {
                            member_overrides.forget(id);
                        }
                    }
                }
            }
            // A new membership must not reuse an ID copied into another group.
            mc.member_id = None;
            if let Some(destination) = &dest.group {
                for entry in &mut data.profiles.get_mut(profile).unwrap().mods {
                    if let ModOrGroup::Group {
                        group_name,
                        member_overrides,
                        ..
                    } = entry
                        && group_name == destination
                    {
                        member_overrides.set(mc, effective);
                    }
                }
            } else {
                mc.enabled = effective;
            }
        }
        match dest.group {
            Some(name) => {
                let ModOrGroup::Individual(mc) = item else {
                    unreachable!()
                };
                data.groups.get_mut(&name).unwrap().mods.insert(slot, mc);
            }
            None => data
                .profiles
                .get_mut(profile)
                .unwrap()
                .mods
                .insert(slot, item),
        }
        Ok(())
    }
}

