use super::*;
use crate::state::{ModData_v0_1_0 as Data, ModProfile_v0_1_0 as Profile};

#[derive(Clone)]
pub(super) struct Drag {
    profile: String,
    group: Option<String>,
    index: usize,
    key: String,
    pub(super) is_group: bool,
}

pub(super) struct Target {
    pub group: Option<String>,
    pub slot: usize,
}

impl Drag {
    pub fn paint_row<R>(
        ui: &mut Ui, profile: &str, group: Option<&str>, index: usize,
        draw: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        let active = egui::DragAndDrop::payload::<Self>(ui.ctx()).is_some_and(|drag| {
            drag.profile == profile && drag.group.as_deref() == group && drag.index == index
        });
        if !active { return draw(ui); }
        let delta = ui.input(|i| {
            Some(i.pointer.interact_pos()? - i.pointer.press_origin()?)
        });
        let Some(delta) = delta else { return draw(ui) };
        // Move only the paint. Keep the layout and drop targets in their saved positions.
        let layer = egui::LayerId::new(egui::Order::Tooltip, ui.id().with("moving-row"));
        let result = ui.scope_builder(egui::UiBuilder::new().layer_id(layer), draw).inner;
        ui.ctx().transform_layer_shapes(layer, egui::emath::TSTransform::from_translation(delta));
        result
    }

    pub fn handle(ui: &mut Ui, profile: &str, group: Option<&str>, index: usize, key: &str, is_group: bool) {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(18.0, ui.spacing().interact_size.y), egui::Sense::drag(),
        );
        let center = rect.center();
        for offset in [-4.0, 0.0, 4.0] {
            ui.painter().hline(center.x - 5.0..=center.x + 5.0, center.y + offset,
                (1.5, ui.visuals().text_color()));
        }
        let response = response.on_hover_cursor(egui::CursorIcon::Grab);

        if response.drag_started() {
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

    pub fn apply(&self, data: &mut Data, profile: &str, dest: Target, store: &ModStore) -> Result<(), &'static str> {
        if self.profile != profile {
            return Err("Move mods within one profile at a time");
        }
        let list = &data.profiles[profile].mods;
        let member = match &self.group {
            Some(name) => data.groups.get(name).and_then(|g| g.mods.get(self.index)),
            None => match list.get(self.index) {
                Some(ModOrGroup::Individual(mc)) if !self.is_group => Some(mc),
                Some(ModOrGroup::Group { group_name, .. }) if self.is_group && *group_name == self.key => None,
                _ => return Err("The dragged row has changed"),
            },
        };
        if !self.is_group && member.is_none_or(|m| m.spec.url != self.key) {
            return Err("The dragged mod has changed");
        }
        let same_list = self.group == dest.group;
        let slot = if let Some(name) = &dest.group {
            if self.is_group {
                return Err("Groups cannot contain groups");
            }
            let group = data.groups.get(name).ok_or("The destination group no longer exists")?;
            if !same_list {
                let source = crate::mod_identity::Artifact::cached(store, &member.unwrap().spec);
                if group.mods.iter().any(|m| crate::mod_identity::Artifact::cached(store, &m.spec) == source) {
                    return Err("That mod is already in this group");
                }
            }
            dest.slot.min(group.mods.len())
        } else {
            if !same_list && !self.is_group {
                let source = crate::mod_identity::Artifact::cached(store, &member.unwrap().spec);
                if list.iter().any(|item| matches!(item, ModOrGroup::Individual(mc)
                    if crate::mod_identity::Artifact::cached(store, &mc.spec) == source)) {
                    return Err("That mod already has a standalone entry");
                }
            }
            dest.slot.min(list.len())
        };
        let slot = slot - usize::from(same_list && self.index < slot);
        if same_list && self.index == slot {
            return Ok(());
        }

        let mut item = if let Some(name) = &self.group {
            ModOrGroup::Individual(data.groups.get_mut(name).unwrap().mods.remove(self.index))
        } else {
            data.profiles.get_mut(profile).unwrap().mods.remove(self.index)
        };

        // Transfer profile-specific switches when moving a shared member.
        if !same_list && let Some(source) = &self.group
            && let ModOrGroup::Individual(mc) = &mut item
        {
            let mut active_enabled = mc.enabled;
            for (profile_name, p) in &mut data.profiles {
                let mut value = None;
                for entry in &mut p.mods {
                    if let ModOrGroup::Group { group_name, member_overrides, .. } = entry
                        && group_name == source
                    {
                        value = Some(member_overrides.enabled(mc));
                        if let Some(id) = &mc.member_id {
                            member_overrides.forget(id);
                        }
                    }
                }
                if let Some(enabled) = value {
                    if profile_name == profile {
                        active_enabled = enabled;
                    }
                    for entry in &mut p.mods {
                        if let ModOrGroup::Group { group_name, member_overrides, .. } = entry
                            && dest.group.as_ref() == Some(group_name)
                        {
                            member_overrides.set(mc, enabled);
                        }
                    }
                }
            }
            if dest.group.is_none() {
                mc.enabled = active_enabled;
                mc.member_id = None;
            }
        }
        match dest.group {
            Some(name) => {
                let ModOrGroup::Individual(mc) = item else { unreachable!() };
                data.groups.get_mut(&name).unwrap().mods.insert(slot, mc);
            }
            None => data.profiles.get_mut(profile).unwrap().mods.insert(slot, item),
        }
        Ok(())
    }
}

pub(super) fn row_target(ui: &Ui, response: &egui::Response, drag: &mut Option<(Drag, Target)>, group: Option<&str>, index: usize) {
    // A member or header gets first choice for mod drops. Whole groups always
    // use the enclosing profile row, even over the members of an expanded group.
    if drag.is_some() {
        return;
    }
    let Some(source) = egui::DragAndDrop::payload::<Drag>(ui.ctx()) else { return };
    if group.is_some() && source.is_group {
        return;
    }
    let mut rect = response.rect;
    rect.max.x = rect.max.x.max(ui.max_rect().right());
    if !ui.rect_contains_pointer(rect) {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_interact_pos() else { return };
    let after = pointer.y > rect.center().y;
    let y = if after { rect.bottom() } else { rect.top() };
    ui.painter().hline(rect.x_range(), y, (2.0, ui.visuals().selection.stroke.color));
    if ui.input(|input| input.pointer.any_released()) {
        *drag = Some(((*source).clone(), Target {
            group: group.map(str::to_owned),
            slot: index + usize::from(after),
        }));
    }
}

#[derive(Clone, Default)]
struct RenameEditor {
    selected: String,
    name: String,
    error: Option<String>,
    success: Option<String>,
    picker_open: bool,
}

#[derive(Clone, Default)]
struct DeleteEditor {
    selected: String,
    picker_open: bool,
    confirming: bool,
    error: Option<String>,
    success: Option<String>,
}

#[derive(Clone, Default)]
struct DuplicateEditor {
    selected: String,
    name: String,
    picker_open: bool,
    error: Option<String>,
    success: Option<String>,
}

/// Renaming changes the shared collection's label, never its contents or order.
fn rename_group(data: &mut Data, old: &str, requested: &str) -> Result<bool, &'static str> {
    let name = requested.trim();
    if name.is_empty() { return Err("Enter a group name"); }
    if !data.groups.contains_key(old) { return Err("That group no longer exists"); }
    if name == old { return Ok(false); }
    if data.groups.contains_key(name) { return Err("A group with that name already exists"); }
    let group = data.groups.remove(old).unwrap();
    data.groups.insert(name.to_owned(), group);
    for profile in data.profiles.values_mut() {
        for item in &mut profile.mods {
            if let ModOrGroup::Group { group_name, .. } = item {
                if group_name == old { *group_name = name.to_owned(); }
            }
        }
    }
    Ok(true)
}

/// Delete the shared collection and detach it from every profile.
/// Standalone mods and other groups are untouched.
fn delete_group(data: &mut Data, name: &str) -> Result<(), &'static str> {
    if data.groups.remove(name).is_none() {
        return Err("That group no longer exists");
    }
    for profile in data.profiles.values_mut() {
        profile.mods.retain(|item| !matches!(item, ModOrGroup::Group { group_name, .. } if group_name == name));
    }
    Ok(())
}

/// Create a separate shared group with the same members and their settings.
/// Only the active profile gets a reference to the new group.
fn duplicate_group(data: &mut Data, source: &str, requested: &str) -> Result<String, &'static str> {
    let name = requested.trim();
    if name.is_empty() { return Err("Enter a name for the duplicate group"); }
    if data.groups.contains_key(name) { return Err("A group with that name already exists"); }
    let copy = data.groups.get(source).ok_or("That group no longer exists")?.clone();

    // Keep this profile's switches when the source group is attached here.
    let reference = data.profiles[&data.active_profile].mods.iter().find_map(|item| {
        if let ModOrGroup::Group { group_name, enabled, member_overrides } = item
            && group_name == source
        {
            Some(ModOrGroup::Group {
                group_name: name.to_owned(),
                enabled: *enabled,
                member_overrides: member_overrides.clone(),
            })
        } else {
            None
        }
    }).unwrap_or(ModOrGroup::Group {
        group_name: name.to_owned(),
        enabled: true,
        member_overrides: Default::default(),
    });
    data.groups.insert(name.to_owned(), copy);
    data.profiles.get_mut(&data.active_profile).unwrap().mods.insert(0, reference);
    Ok(name.to_owned())
}

/// A full-width dropdown row with the name left-aligned.
/// The whole row receives hover highlighting and click input.
fn group_option_row(ui: &mut Ui, name: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let highlighted = selected || response.hovered() || response.has_focus();
        if highlighted {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(2), ui.visuals().selection.bg_fill);
        }
        let text_color = if highlighted {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            egui::pos2(rect.left() + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::TextStyle::Button.resolve(ui.style()),
            text_color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Use the same compact, always-visible scrollbar for every group selector.
fn group_picker_scroll() -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .max_height(140.0)
        .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysVisible)
}

/// Returns whether groups changed and an optional success message for the main status bar.
pub(super) fn controls(ui: &mut Ui, data: &mut Data, name: &mut String, files_pending: bool) -> (bool, Option<String>) {
    let profile = data.profiles.get_mut(&data.active_profile).unwrap();
    let mut changed = false;
    let image = egui::Image::new(egui::include_image!("../../assets/groups.svg"))
        .fit_to_exact_size(egui::vec2(16.0, 16.0))
        .tint(ui.visuals().text_color());
    let button = ui.add(Button::image(image)).on_hover_text("Groups");
    let popup = button.id.with("groups-menu");
    let editor_id = popup.with("rename-editor");
    let duplicate_id = popup.with("duplicate-editor");
    let delete_id = popup.with("delete-editor");
    let mut editor = ui.ctx().data_mut(|d| d.get_temp::<RenameEditor>(editor_id)).unwrap_or_default();
    let mut duplicate_editor = ui.ctx().data_mut(|d| d.get_temp::<DuplicateEditor>(duplicate_id)).unwrap_or_default();
    let mut delete_editor = ui.ctx().data_mut(|d| d.get_temp::<DeleteEditor>(delete_id)).unwrap_or_default();
    let mut requested_rename = None;
    let mut requested_duplicate = None;
    let mut requested_delete = None;
    let mut success_message = None;
    if button.clicked() {
        editor.picker_open = false;
        editor.success = None;
        duplicate_editor.picker_open = false;
        duplicate_editor.success = None;
        delete_editor.picker_open = false;
        delete_editor.confirming = false;
        delete_editor.success = None;
        ui.memory_mut(|mem| mem.toggle_popup(popup));
    }
    egui::popup_below_widget(ui, popup, &button, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(180.0);
        ui.label("New group");
        ui.text_edit_singleline(name);
        let trimmed = name.trim();
        if ui.add_enabled(!trimmed.is_empty() && !data.groups.contains_key(trimmed), Button::new("Create group")).clicked() {
            let created = trimmed.to_owned();
            data.groups.insert(created.clone(), Default::default());
            attach(profile, &created);
            name.clear();
            changed = true;
            success_message = Some(format!("Created group \"{created}\"."));
            ui.memory_mut(|mem| mem.close_popup());
        }
        ui.separator();
        let eligible: Vec<_> = data.groups.keys().filter(|key| {
            !profile.mods.iter().any(|item| {
                matches!(item, ModOrGroup::Group { group_name, .. } if group_name == *key)
            })
        }).collect();
        if eligible.is_empty() {
            ui.add_enabled(false, Button::new("Add existing group"));
            ui.label("No other groups available for this profile.");
        } else {
            egui::CollapsingHeader::new("Add existing group")
                .id_salt(("attach-existing", &data.active_profile))
                .show(ui, |ui| {
                    // Keep the expanded list compact; additional groups remain scrollable.
                    group_picker_scroll().show(ui, |ui| {
                        for key in eligible {
                            if group_option_row(ui, key.as_str(), false).clicked() {
                                attach(profile, key);
                                changed = true;
                                success_message = Some(format!("Added existing group \"{key}\" to the active profile."));
                                ui.memory_mut(|mem| mem.close_popup());
                                break;
                            }
                        }
                    });
                });
        }
        ui.separator();
        let rename_section = egui::CollapsingHeader::new("Rename group").show(ui, |ui| {
            if let Some(success) = &editor.success {
                ui.colored_label(egui::Color32::from_rgb(100, 200, 120), success);
            }
            if data.groups.is_empty() {
                ui.label("No groups to rename.");
                return;
            }
            // Don't select the first group automatically. Require an explicit choice.
            if !editor.selected.is_empty() && !data.groups.contains_key(&editor.selected) {
                editor.selected.clear();
                // Keep the new name even if the previous group was removed.
                editor.error = None;
                editor.picker_open = false;
            }
            // A compact button, not an input field or a nested popup.
            // Draw the chevron ourselves so it works with fonts lacking arrow glyphs.
            let response = ui.add(
                Button::new(format!("{}    ", if editor.selected.is_empty() {
                    "Choose group"
                } else {
                    editor.selected.as_str()
                }))
                    .min_size(egui::vec2(110.0, ui.spacing().interact_size.y)),
            ).on_hover_text("Choose a group to rename");
            if response.clicked() {
                editor.picker_open = !editor.picker_open;
            }
            let x = response.rect.right() - 12.0;
            let y = response.rect.center().y;
            let arrow = if editor.picker_open {
                vec![egui::pos2(x - 4.0, y + 2.0), egui::pos2(x + 4.0, y + 2.0), egui::pos2(x, y - 2.0)]
            } else {
                vec![egui::pos2(x - 4.0, y - 2.0), egui::pos2(x + 4.0, y - 2.0), egui::pos2(x, y + 2.0)]
            };
            ui.painter().add(egui::Shape::convex_polygon(
                arrow, ui.visuals().text_color(), egui::Stroke::NONE,
            ));
            if editor.picker_open {
                group_picker_scroll().show(ui, |ui| {
                    for key in data.groups.keys() {
                        if group_option_row(ui, key.as_str(), editor.selected == key.as_str()).clicked() {
                            editor.selected = key.clone();
                            // Keep any new name entered before choosing the source group.
                            editor.error = None;
                            editor.success = None;
                            editor.picker_open = false;
                        }
                    }
                });
            }
            ui.add_space(8.0);
            if ui.add(egui::TextEdit::singleline(&mut editor.name)
                .hint_text("New group name")).changed() {
                editor.error = None;
                editor.success = None;
            }
            ui.label("Renames this shared group in every profile.");
            if files_pending {
                ui.label("Wait for dropped files to finish before renaming.");
            }
            let trimmed = editor.name.trim();
            let available = !editor.selected.is_empty() && !trimmed.is_empty() && trimmed != editor.selected
                && !data.groups.contains_key(trimmed);
            if !trimmed.is_empty() && trimmed != editor.selected && data.groups.contains_key(trimmed) {
                ui.colored_label(ui.visuals().warn_fg_color, "A group with that name already exists.");
            }
            if ui.add_enabled(available && !files_pending, Button::new("Rename in all profiles")).clicked() {
                requested_rename = Some((editor.selected.clone(), trimmed.to_owned()));
            }
            if let Some(error) = &editor.error { ui.colored_label(ui.visuals().warn_fg_color, error); }
        });
        if rename_section.header_response.clicked() {
            editor.success = None;
        }
        ui.separator();
        let duplicate_section = egui::CollapsingHeader::new("Duplicate group")
            .id_salt("duplicate-group-section")
            .show(ui, |ui| {
                if let Some(success) = &duplicate_editor.success {
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 120), success);
                }
                if data.groups.is_empty() {
                    ui.label("No groups to duplicate.");
                    return;
                }
                if !duplicate_editor.selected.is_empty() && !data.groups.contains_key(&duplicate_editor.selected) {
                    duplicate_editor.selected.clear();
                    duplicate_editor.picker_open = false;
                    duplicate_editor.error = None;
                }
                let response = ui.add(
                    Button::new(format!("{}    ", if duplicate_editor.selected.is_empty() {
                        "Choose group"
                    } else {
                        duplicate_editor.selected.as_str()
                    }))
                    .min_size(egui::vec2(110.0, ui.spacing().interact_size.y)),
                ).on_hover_text("Choose a group to duplicate");
                if response.clicked() {
                    duplicate_editor.picker_open = !duplicate_editor.picker_open;
                }
                let x = response.rect.right() - 12.0;
                let y = response.rect.center().y;
                let arrow = if duplicate_editor.picker_open {
                    vec![egui::pos2(x - 4.0, y + 2.0), egui::pos2(x + 4.0, y + 2.0), egui::pos2(x, y - 2.0)]
                } else {
                    vec![egui::pos2(x - 4.0, y - 2.0), egui::pos2(x + 4.0, y - 2.0), egui::pos2(x, y + 2.0)]
                };
                ui.painter().add(egui::Shape::convex_polygon(
                    arrow, ui.visuals().text_color(), egui::Stroke::NONE,
                ));
                if duplicate_editor.picker_open {
                    group_picker_scroll().show(ui, |ui| {
                        for key in data.groups.keys() {
                            if group_option_row(ui, key.as_str(), duplicate_editor.selected == key.as_str()).clicked() {
                                duplicate_editor.selected = key.clone();
                                // Keep any new name entered before choosing the source group.
                                duplicate_editor.error = None;
                                duplicate_editor.success = None;
                                duplicate_editor.picker_open = false;
                            }
                        }
                    });
                }
                ui.add_space(8.0);
                if ui.add(egui::TextEdit::singleline(&mut duplicate_editor.name)
                    .hint_text("New group name")).changed() {
                    duplicate_editor.error = None;
                    duplicate_editor.success = None;
                }
                ui.label("Creates a separate copy in the active profile.");
                if files_pending {
                    ui.label("Wait for dropped files to finish before duplicating.");
                }
                let trimmed = duplicate_editor.name.trim();
                if !trimmed.is_empty() && data.groups.contains_key(trimmed) {
                    ui.colored_label(ui.visuals().warn_fg_color, "A group with that name already exists.");
                }
                let available = !duplicate_editor.selected.is_empty() && !trimmed.is_empty()
                    && !data.groups.contains_key(trimmed) && !files_pending;
                if ui.add_enabled(available, Button::new("Duplicate group")).clicked() {
                    requested_duplicate = Some((duplicate_editor.selected.clone(), trimmed.to_owned()));
                }
                if let Some(error) = &duplicate_editor.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
            });
        if duplicate_section.header_response.clicked() {
            duplicate_editor.success = None;
        }
        ui.separator();
        let delete_section = egui::CollapsingHeader::new("Delete group")
            .id_salt("delete-group-section")
            .show(ui, |ui| {
                if let Some(success) = &delete_editor.success {
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 120), success);
                }
                if data.groups.is_empty() {
                    ui.label("No groups to delete.");
                    return;
                }
                if !delete_editor.selected.is_empty() && !data.groups.contains_key(&delete_editor.selected) {
                    delete_editor = DeleteEditor::default();
                }
                // The selector stays inside the Groups popup, avoiding nested popup issues.
                let response = ui.add(
                    Button::new(format!("{}    ", if delete_editor.selected.is_empty() {
                        "Choose group"
                    } else {
                        delete_editor.selected.as_str()
                    }))
                    .min_size(egui::vec2(110.0, ui.spacing().interact_size.y)),
                ).on_hover_text("Choose a group to delete");
                if response.clicked() {
                    delete_editor.picker_open = !delete_editor.picker_open;
                    delete_editor.confirming = false;
                }
                let x = response.rect.right() - 12.0;
                let y = response.rect.center().y;
                let arrow = if delete_editor.picker_open {
                    vec![egui::pos2(x - 4.0, y + 2.0), egui::pos2(x + 4.0, y + 2.0), egui::pos2(x, y - 2.0)]
                } else {
                    vec![egui::pos2(x - 4.0, y - 2.0), egui::pos2(x + 4.0, y - 2.0), egui::pos2(x, y + 2.0)]
                };
                ui.painter().add(egui::Shape::convex_polygon(
                    arrow, ui.visuals().text_color(), egui::Stroke::NONE,
                ));
                if delete_editor.picker_open {
                    group_picker_scroll().show(ui, |ui| {
                        for key in data.groups.keys() {
                            if group_option_row(ui, key.as_str(), delete_editor.selected == key.as_str()).clicked() {
                                delete_editor.selected = key.clone();
                                delete_editor.picker_open = false;
                                delete_editor.confirming = false;
                                delete_editor.error = None;
                                delete_editor.success = None;
                            }
                        }
                    });
                }
                if files_pending {
                    ui.label("Wait for dropped files to finish before deleting.");
                }
                if delete_editor.confirming {
                    ui.colored_label(ui.visuals().warn_fg_color,
                        format!("Delete \"{}\" from every profile? All mods within this shared group will be removed from the profiles.", delete_editor.selected));
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            delete_editor.confirming = false;
                        }
                        if ui.add_enabled(!files_pending, Button::new("Confirm delete")).clicked() {
                            requested_delete = Some(delete_editor.selected.clone());
                        }
                    });
                } else if ui.add_enabled(!delete_editor.selected.is_empty() && !files_pending,
                    Button::new("Delete group")).clicked() {
                    delete_editor.confirming = true;
                    delete_editor.success = None;
                }
                if let Some(error) = &delete_editor.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
            });
        if delete_section.header_response.clicked() {
            delete_editor.success = None;
        }
        ui.separator();
        egui::CollapsingHeader::new("Export groups")
            .id_salt("export-groups-section")
            .show(ui, |_ui| {});
        ui.separator();
        egui::CollapsingHeader::new("Import groups")
            .id_salt("import-groups-section")
            .show(ui, |_ui| {});
    });
    if let Some((old, new)) = requested_rename {
        match rename_group(data, &old, &new) {
            Ok(renamed) => {
                changed |= renamed;
                // Require a fresh selection after renaming a group.
                editor.selected.clear();
                if duplicate_editor.selected == old {
                    duplicate_editor.selected = new.clone();
                }
                if delete_editor.selected == old {
                    delete_editor.selected = new.clone();
                    delete_editor.confirming = false;
                }
                editor.name.clear();
                editor.error = None;
                editor.picker_open = false;
                if renamed {
                    let message = format!("Renamed group \"{old}\" to \"{new}\" in all profiles.");
                    editor.success = Some(message.clone());
                    success_message = Some(message);
                }
                egui::DragAndDrop::clear_payload(ui.ctx());
                // Keep the menu open so the inline confirmation can be read.
                ui.ctx().request_repaint();
            }
            Err(error) => editor.error = Some(error.to_owned()),
        }
    }
    if let Some((source, new_name)) = requested_duplicate {
        match duplicate_group(data, &source, &new_name) {
            Ok(created) => {
                changed = true;
                let message = format!("Duplicated group \"{source}\" as \"{created}\".");
                success_message = Some(message.clone());
                duplicate_editor = DuplicateEditor {
                    success: Some(message),
                    ..Default::default()
                };
                egui::DragAndDrop::clear_payload(ui.ctx());
                // Keep the menu open so the inline confirmation can be read.
                ui.ctx().request_repaint();
            }
            Err(error) => duplicate_editor.error = Some(error.to_owned()),
        }
    }
    if let Some(target) = requested_delete {
        match delete_group(data, &target) {
            Ok(()) => {
                changed = true;
                let message = format!("Deleted group \"{target}\" from all profiles.");
                success_message = Some(message.clone());
                delete_editor = DeleteEditor {
                    success: Some(message),
                    ..Default::default()
                };
                if editor.selected == target {
                    editor = RenameEditor::default();
                }
                if duplicate_editor.selected == target {
                    duplicate_editor = DuplicateEditor::default();
                }
                egui::DragAndDrop::clear_payload(ui.ctx());
                // Leave the Groups menu open to show the success message.
                ui.ctx().request_repaint();
            }
            Err(error) => {
                delete_editor.confirming = false;
                delete_editor.error = Some(error.to_owned());
            }
        }
    }
    ui.ctx().data_mut(|d| {
        d.insert_temp(editor_id, editor);
        d.insert_temp(duplicate_id, duplicate_editor);
        d.insert_temp(delete_id, delete_editor);
    });
    (changed, success_message)
}

fn attach(profile: &mut Profile, name: &str) {
    profile.mods.insert(0, ModOrGroup::Group {
        group_name: name.into(),
        enabled: true,
        member_overrides: Default::default(),
    });
}

pub(super) fn drop_files(app: &mut App, ctx: &egui::Context, profile: String, group: String, prepend: bool, paths: Vec<PathBuf>) {
    app.pending_group_files += 1;
    let tx = app.tx.clone();
    let ctx = ctx.clone();
    let store = app.state.store.clone();
    let mut identity_specs = crate::mod_identity::all_specs(&app.state.mod_data);
    tokio::spawn(async move {
        let files = tokio::task::spawn_blocking(move || {
            let mut specs = Vec::new();
            for path in paths {
                if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pak")) {
                    return Err(format!("Expected a .pak file: {}", path.display()));
                }
                let file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                repak::PakBuilder::new().reader(&mut std::io::BufReader::new(file))
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let path = path.into_os_string().into_string().map_err(|_| "Unsupported file path".to_string())?;
                specs.push(ModSpecification::new(path));
            }
            if specs.is_empty() {
                return Err("No local .pak files in the drop".into());
            }
            identity_specs.extend(specs.iter().cloned());
            store.refresh_local_files(&identity_specs);
            Ok(specs)
        }).await.unwrap_or_else(|e| Err(e.to_string()));
        if tx.send(message::Message::GroupFiles { profile, group, prepend, files }).await.is_ok() {
            ctx.request_repaint();
        }
    });
}

pub(super) fn add_files(app: &mut App, profile: String, group: String, prepend: bool, files: Result<Vec<ModSpecification>, String>) {
    let result = files.and_then(|files| {
        let data = &mut app.state.mod_data;
        if !data.profiles.get(&profile).is_some_and(|p| p.mods.iter().any(|item| {
            matches!(item, ModOrGroup::Group { group_name, .. } if group_name == &group)
        })) {
            return Err("The destination group was removed from the profile".into());
        }
        let target = data.groups.get_mut(&group).ok_or("The destination group no longer exists")?;
        let mut seen: HashSet<_> = target.mods.iter()
            .map(|mc| crate::mod_identity::Artifact::cached(&app.state.store, &mc.spec)).collect();
        for spec in &files {
            if !seen.insert(crate::mod_identity::Artifact::cached(&app.state.store, spec)) {
                return Err(format!("{} is already in the group or drop", spec.url));
            }
        }
        let slot = if prepend { 0 } else { target.mods.len() };
        target.mods.splice(slot..slot, files.into_iter().map(|spec| ModConfig {
            spec, enabled: true, member_id: None, required: false, priority: 0,
        }));
        data.save().map_err(|e| e.to_string())?;
        let message = format!("Added files to {group}");
        app.groups_to_open.insert((profile, group));
        Ok(message)
    });
    app.last_action = Some(match result {
        Ok(message) => LastAction::success(message),
        Err(message) => LastAction::failure(message),
    });
}

pub(super) struct UndoMove {
    before: Data,
    after: Vec<u8>,
}

impl UndoMove {
    pub fn new(before: Data, after: &Data) -> Self {
        Self { before, after: serde_json::to_vec(after).expect("serializable mod data") }
    }

    pub fn apply(self, data: &mut Data) -> Result<(), &'static str> {
        if serde_json::to_vec(data).expect("serializable mod data") != self.after {
            return Err("The profile or groups have changed since the move");
        }
        *data = self.before;
        Ok(())
    }
}

/// Hover timers are scoped to the rendered group header, including its window.
/// Requiring a recent observation prevents an old timer surviving a profile switch.
#[derive(Clone, Copy, Default)]
struct HoverOpen {
    started: Option<f64>,
    last_seen: f64,
}

impl HoverOpen {
    fn update(&mut self, now: f64, active: bool) -> bool {
        if !active {
            self.started = None;
            self.last_seen = now;
            return false;
        }
        if now < self.last_seen || now - self.last_seen > 0.2 {
            self.started = None;
        }
        self.last_seen = now;
        let start = *self.started.get_or_insert(now);
        now - start >= 0.5
    }
}

pub(super) fn hover_open(ui: &Ui, header: &egui::Response, closed: bool, profile: &str, manual: bool) -> bool {
    let internal = manual && egui::DragAndDrop::payload::<Drag>(ui.ctx())
        .is_some_and(|drag| !drag.is_group && drag.profile == profile);
    let external = ui.input(|i| !i.raw.hovered_files.is_empty());
    let active = closed && (internal || external) && ui.rect_contains_pointer(header.rect);
    let id = header.id.with("drag-hover-open");
    let now = ui.input(|i| i.time);
    let open = ui.ctx().data_mut(|data| {
        let mut timer = data.get_temp::<HoverOpen>(id).unwrap_or_default();
        let open = timer.update(now, active);
        data.insert_temp(id, timer);
        open
    });
    if active {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    open
}

fn edge_scroll_delta(view: egui::Rect, pointer: egui::Pos2, dt: f32) -> f32 {
    if !view.contains(pointer) || view.height() <= 0.0 { return 0.0; }
    let band = 36.0_f32.min(view.height() * 0.25);
    let top = ((view.top() + band - pointer.y) / band).clamp(0.0, 1.0);
    let bottom = ((pointer.y - (view.bottom() - band)) / band).clamp(0.0, 1.0);
    // Positive moves content down (scroll up); negative scrolls down.
    (top - bottom) * 480.0 * dt.clamp(0.0, 0.05)
}

pub(super) fn auto_scroll(ui: &Ui, profile: &str, manual: bool) {
    let internal = manual && egui::DragAndDrop::payload::<Drag>(ui.ctx())
        .is_some_and(|drag| drag.profile == profile);
    let external = ui.input(|i| !i.raw.hovered_files.is_empty());
    if !internal && !external { return; }
    // clip_rect is the visible part of the scroll area, not its full content.
    let view = ui.clip_rect();
    let delta = ui.input(|i| i.pointer.hover_pos()
        .map(|pointer| edge_scroll_delta(view, pointer, i.stable_dt))
        .unwrap_or(0.0));
    if delta != 0.0 {
        ui.scroll_with_delta(egui::vec2(0.0, delta));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
}

#[cfg(test)]
mod drag_polish_tests {
    use super::*;

    #[test]
    fn hover_requires_dwell_and_resets_after_leaving_or_missing_frames() {
        let mut timer = HoverOpen::default();
        assert!(!timer.update(1.0, true));
        for i in 1..5 { assert!(!timer.update(1.0 + i as f64 * 0.1, true)); }
        assert!(timer.update(1.51, true));
        assert!(!timer.update(1.52, false));
        assert!(!timer.update(1.53, true));
        assert!(!timer.update(5.0, true));
        assert!(!timer.update(5.1, true));
    }

    #[test]
    fn edge_scroll_has_direction_speed_limit_and_no_motion_outside_view() {
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(210.0, 420.0));
        assert!(edge_scroll_delta(rect, egui::pos2(50.0, 21.0), 0.016) > 0.0);
        assert!(edge_scroll_delta(rect, egui::pos2(50.0, 419.0), 0.016) < 0.0);
        assert_eq!(edge_scroll_delta(rect, rect.center(), 0.016), 0.0);
        assert_eq!(edge_scroll_delta(rect, egui::pos2(9.0, 21.0), 0.016), 0.0);
        assert!(edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 10.0) <= 24.0);
        assert_eq!(edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 0.032),
            2.0 * edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 0.016));
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;

    fn fixture() -> Data {
        serde_json::from_value(serde_json::json!({
            "active_profile": "A",
            "profiles": {
                "A": {"mods": [
                    {"spec": {"url": "standalone"}, "required": false, "enabled": true},
                    {"group_name": "Tools", "enabled": true, "member_overrides": {"m1": false}}
                ]},
                "B": {"mods": [
                    {"group_name": "Tools", "enabled": false},
                    {"group_name": "Tools", "enabled": true, "member_overrides": {"m1": true}}
                ]}
            },
            "groups": {
                "Tools": {"mods": [{"spec": {"url": "https://mod.io/g/drg/m/test#123/10"},
                    "required": true, "enabled": true, "priority": 7, "member_id": "m1"}]},
                "Other": {"mods": []}
            }
        })).unwrap()
    }

    #[test]
    fn rename_updates_all_references_without_changing_members_order_or_switches() {
        let mut data = fixture();
        let before = serde_json::to_value(&data).unwrap();
        assert!(rename_group(&mut data, "Tools", "  Utilities  ").unwrap());
        assert!(!data.groups.contains_key("Tools"));
        assert!(data.groups.contains_key("Utilities"));
        for profile in data.profiles.values() {
            for item in &profile.mods {
                if let ModOrGroup::Group { group_name, .. } = item {
                    assert_eq!(group_name, "Utilities");
                }
            }
        }
        // Verify a round trip through disk representation, including all overrides.
        let mut restored: Data = serde_json::from_value(serde_json::to_value(&data).unwrap()).unwrap();
        rename_group(&mut restored, "Utilities", "Tools").unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), before);
    }

    #[test]
    fn duplicate_group_keeps_mods_and_switches_but_not_other_profile_references() {
        let mut data = fixture();
        let source = serde_json::to_value(&data.groups["Tools"]).unwrap();
        let other_profile = serde_json::to_value(&data.profiles["B"]).unwrap();
        assert_eq!(duplicate_group(&mut data, "Tools", "  Tools Backup  ").unwrap(), "Tools Backup");
        assert_eq!(serde_json::to_value(&data.groups["Tools Backup"]).unwrap(), source);
        assert_eq!(serde_json::to_value(&data.groups["Tools"]).unwrap(), source);
        assert_eq!(serde_json::to_value(&data.profiles["B"]).unwrap(), other_profile);
        assert!(matches!(&data.profiles["A"].mods[0], ModOrGroup::Group {
            group_name, enabled: true, member_overrides,
        } if group_name == "Tools Backup" && !member_overrides.enabled(&data.groups["Tools Backup"].mods[0])));
        data.groups.get_mut("Tools Backup").unwrap().mods[0].priority = 99;
        assert_eq!(data.groups["Tools"].mods[0].priority, 7);
    }

    #[test]
    fn duplicate_group_rejects_invalid_names_without_changes() {
        let mut data = fixture();
        let before = serde_json::to_value(&data).unwrap();
        for (source, name) in [("Missing", "New"), ("Tools", "  "),
            ("Tools", "Other"), ("Tools", "Tools")]
        {
            assert!(duplicate_group(&mut data, source, name).is_err());
        }
        assert_eq!(serde_json::to_value(&data).unwrap(), before);
    }

    #[test]
    fn delete_shared_group_removes_references_from_all_profiles() {
        let mut data = fixture();
        delete_group(&mut data, "Tools").unwrap();
        assert!(!data.groups.contains_key("Tools"));
        assert!(data.groups.contains_key("Other"));
        assert_eq!(data.profiles["A"].mods.len(), 1);
        assert!(matches!(data.profiles["A"].mods[0], ModOrGroup::Individual(_)));
        assert!(data.profiles["B"].mods.is_empty());
    }

    #[test]
    fn delete_missing_group_does_not_change_data() {
        let mut data = fixture();
        let before = serde_json::to_value(&data).unwrap();
        assert!(delete_group(&mut data, "Missing").is_err());
        assert_eq!(serde_json::to_value(&data).unwrap(), before);
    }

    #[test]
    fn invalid_and_unchanged_names_leave_data_intact() {
        let mut data = fixture();
        let before = serde_json::to_value(&data).unwrap();
        assert!(rename_group(&mut data, "Tools", "  ").is_err());
        assert!(rename_group(&mut data, "Tools", "Other").is_err());
        assert!(rename_group(&mut data, "Missing", "New").is_err());
        assert!(!rename_group(&mut data, "Tools", " Tools ").unwrap());
        assert_eq!(serde_json::to_value(&data).unwrap(), before);
    }
}
