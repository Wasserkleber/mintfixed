use super::import_export::*;
use super::*;

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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum GroupExportKind {
    #[default]
    Json,
    Txt,
}

impl GroupExportKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Txt => "txt",
        }
    }
}

#[derive(Clone, Default)]
struct ExportEditor {
    selected: String,
    kind: GroupExportKind,
    picker_open: bool,
    success: Option<String>,
    error: Option<String>,
}

#[derive(Clone)]
pub(super) struct PendingGroupImport {
    pub(super) suggested_name: Option<String>,
    pub(super) group: crate::state::ModGroup,
}

#[derive(Clone, Default)]
struct ImportEditor {
    pending: Option<PendingGroupImport>,
    name: String,
    success: Option<String>,
    error: Option<String>,
}

/// Remove a member by its stable entry ID, cleaning up overrides in every
/// profile that references this shared group. Never delete by a stale row index.
pub(in crate::gui) fn remove_group_member(
    data: &mut Data,
    group_name: &str,
    mod_id: &str,
) -> Result<(), &'static str> {
    let group = data
        .groups
        .get_mut(group_name)
        .ok_or("Group no longer exists")?;
    let index = group
        .mods
        .iter()
        .position(|mc| mc.id == mod_id)
        .ok_or("Mod is no longer in the group")?;
    let removed = group.mods.remove(index);
    if let Some(member_id) = removed.member_id {
        for entry in data
            .profiles
            .values_mut()
            .flat_map(|profile| &mut profile.mods)
        {
            if let ModOrGroup::Group {
                group_name: entry_group,
                member_overrides,
                ..
            } = entry
                && entry_group == group_name
            {
                member_overrides.forget(&member_id);
            }
        }
    }
    Ok(())
}

/// Renaming changes the shared collection's label, never its contents or order.
pub(super) fn rename_group(
    data: &mut Data,
    old: &str,
    requested: &str,
) -> Result<bool, &'static str> {
    let name = requested.trim();
    if name.is_empty() {
        return Err("Enter a group name");
    }
    if !data.groups.contains_key(old) {
        return Err("That group no longer exists");
    }
    if name == old {
        return Ok(false);
    }
    if data.groups.contains_key(name) {
        return Err("A group with that name already exists");
    }
    let group = data.groups.remove(old).unwrap();
    data.groups.insert(name.to_owned(), group);
    for profile in data.profiles.values_mut() {
        for item in &mut profile.mods {
            if let ModOrGroup::Group { group_name, .. } = item
                && group_name == old
            {
                *group_name = name.to_owned();
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
        profile.mods.retain(
            |item| !matches!(item, ModOrGroup::Group { group_name, .. } if group_name == name),
        );
    }
    Ok(())
}

/// Create a separate shared group with the same members and their settings.
/// Only the active profile gets a reference to the new group.
fn duplicate_group(data: &mut Data, source: &str, requested: &str) -> Result<String, &'static str> {
    let name = requested.trim();
    if name.is_empty() {
        return Err("Enter a name for the duplicate group");
    }
    if data.groups.contains_key(name) {
        return Err("A group with that name already exists");
    }
    let original = data
        .groups
        .get(source)
        .ok_or("That group no longer exists")?;
    let mut copy = original.clone();
    copy.id = crate::state::new_id();
    copy.mods = original.mods.iter().map(ModConfig::duplicate).collect();

    // Keep this profile's switches when the source group is attached here.
    let reference = data.profiles[&data.active_profile]
        .mods
        .iter()
        .find_map(|item| {
            if let ModOrGroup::Group {
                group_name,
                enabled,
                member_overrides,
                ..
            } = item
                && group_name == source
            {
                Some(ModOrGroup::Group {
                    id: crate::state::new_id(),
                    group_name: name.to_owned(),
                    enabled: *enabled,
                    member_overrides: {
                        let mut overrides = crate::state::MemberOverrides::default();
                        for (old, new) in original.mods.iter().zip(&mut copy.mods) {
                            overrides.set(new, member_overrides.enabled(old));
                        }
                        overrides
                    },
                })
            } else {
                None
            }
        })
        .unwrap_or(ModOrGroup::Group {
            id: crate::state::new_id(),
            group_name: name.to_owned(),
            enabled: true,
            member_overrides: Default::default(),
        });
    data.groups.insert(name.to_owned(), copy);
    data.profiles
        .get_mut(&data.active_profile)
        .unwrap()
        .mods
        .insert(0, reference);
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
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(2),
                ui.visuals().selection.bg_fill,
            );
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
pub(in crate::gui) fn controls(
    ui: &mut Ui,
    data: &mut Data,
    name: &mut String,
    files_pending: bool,
) -> (bool, Option<String>) {
    let profile = data.profiles.get_mut(&data.active_profile).unwrap();
    let mut changed = false;
    let button = ui.button("Group Management").on_hover_text("Manage groups");
    let popup = button.id.with("groups-menu");
    let editor_id = popup.with("rename-editor");
    let duplicate_id = popup.with("duplicate-editor");
    let delete_id = popup.with("delete-editor");
    let export_id = popup.with("export-editor");
    let import_id = popup.with("import-editor");
    let mut export_editor = ui
        .ctx()
        .data_mut(|d| d.get_temp::<ExportEditor>(export_id))
        .unwrap_or_default();
    let mut import_editor = ui
        .ctx()
        .data_mut(|d| d.get_temp::<ImportEditor>(import_id))
        .unwrap_or_default();
    let mut editor = ui
        .ctx()
        .data_mut(|d| d.get_temp::<RenameEditor>(editor_id))
        .unwrap_or_default();
    let mut duplicate_editor = ui
        .ctx()
        .data_mut(|d| d.get_temp::<DuplicateEditor>(duplicate_id))
        .unwrap_or_default();
    let mut delete_editor = ui
        .ctx()
        .data_mut(|d| d.get_temp::<DeleteEditor>(delete_id))
        .unwrap_or_default();
    let mut requested_rename = None;
    let mut requested_duplicate = None;
    let mut requested_delete = None;
    let mut requested_export: Option<(PathBuf, String, GroupExportKind)> = None;
    let mut requested_import: Option<PathBuf> = None;
    let mut requested_import_name: Option<String> = None;
    let mut success_message = None;
    if button.clicked() {
        editor.picker_open = false;
        editor.success = None;
        duplicate_editor.picker_open = false;
        duplicate_editor.success = None;
        delete_editor.picker_open = false;
        delete_editor.confirming = false;
        delete_editor.success = None;
        export_editor.picker_open = false;
        export_editor.success = None;
        import_editor.success = None;
        ui.memory_mut(|mem| mem.toggle_popup(popup));
    }
    egui::popup_below_widget(
        ui,
        popup,
        &button,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_min_width(180.0);
            ui.label("New group");
            ui.text_edit_singleline(name);
            let trimmed = name.trim();
            if ui
                .add_enabled(
                    !trimmed.is_empty() && !data.groups.contains_key(trimmed),
                    Button::new("Create group"),
                )
                .clicked()
            {
                let created = trimmed.to_owned();
                data.groups.insert(created.clone(), Default::default());
                attach(profile, &created);
                name.clear();
                changed = true;
                success_message = Some(format!("Created group \"{created}\"."));
                ui.memory_mut(|mem| mem.close_popup());
            }
            ui.separator();
            let eligible: Vec<_> = data
                .groups
                .keys()
                .filter(|key| {
                    !profile.mods.iter().any(|item| {
                matches!(item, ModOrGroup::Group { group_name, .. } if group_name == *key)
            })
                })
                .collect();
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
                                    success_message = Some(format!(
                                        "Added existing group \"{key}\" to the active profile."
                                    ));
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
                let response = ui
                    .add(
                        Button::new(format!(
                            "{}    ",
                            if editor.selected.is_empty() {
                                "Choose group"
                            } else {
                                editor.selected.as_str()
                            }
                        ))
                        .min_size(egui::vec2(110.0, ui.spacing().interact_size.y)),
                    )
                    .on_hover_text("Choose a group to rename");
                if response.clicked() {
                    editor.picker_open = !editor.picker_open;
                }
                let x = response.rect.right() - 12.0;
                let y = response.rect.center().y;
                let arrow = if editor.picker_open {
                    vec![
                        egui::pos2(x - 4.0, y + 2.0),
                        egui::pos2(x + 4.0, y + 2.0),
                        egui::pos2(x, y - 2.0),
                    ]
                } else {
                    vec![
                        egui::pos2(x - 4.0, y - 2.0),
                        egui::pos2(x + 4.0, y - 2.0),
                        egui::pos2(x, y + 2.0),
                    ]
                };
                ui.painter().add(egui::Shape::convex_polygon(
                    arrow,
                    ui.visuals().text_color(),
                    egui::Stroke::NONE,
                ));
                if editor.picker_open {
                    group_picker_scroll().show(ui, |ui| {
                        for key in data.groups.keys() {
                            if group_option_row(ui, key.as_str(), editor.selected == key.as_str())
                                .clicked()
                            {
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
                if ui
                    .add(egui::TextEdit::singleline(&mut editor.name).hint_text("New group name"))
                    .changed()
                {
                    editor.error = None;
                    editor.success = None;
                }
                ui.label("Renames this shared group in every profile.");
                if files_pending {
                    ui.label("Wait for dropped files to finish before renaming.");
                }
                let trimmed = editor.name.trim();
                let available = !editor.selected.is_empty()
                    && !trimmed.is_empty()
                    && trimmed != editor.selected
                    && !data.groups.contains_key(trimmed);
                if !trimmed.is_empty()
                    && trimmed != editor.selected
                    && data.groups.contains_key(trimmed)
                {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "A group with that name already exists.",
                    );
                }
                if ui
                    .add_enabled(
                        available && !files_pending,
                        Button::new("Rename in all profiles"),
                    )
                    .clicked()
                {
                    requested_rename = Some((editor.selected.clone(), trimmed.to_owned()));
                }
                if let Some(error) = &editor.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
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
                    if !duplicate_editor.selected.is_empty()
                        && !data.groups.contains_key(&duplicate_editor.selected)
                    {
                        duplicate_editor.selected.clear();
                        duplicate_editor.picker_open = false;
                        duplicate_editor.error = None;
                    }
                    let response = ui
                        .add(
                            Button::new(format!(
                                "{}    ",
                                if duplicate_editor.selected.is_empty() {
                                    "Choose group"
                                } else {
                                    duplicate_editor.selected.as_str()
                                }
                            ))
                            .min_size(egui::vec2(110.0, ui.spacing().interact_size.y)),
                        )
                        .on_hover_text("Choose a group to duplicate");
                    if response.clicked() {
                        duplicate_editor.picker_open = !duplicate_editor.picker_open;
                    }
                    let x = response.rect.right() - 12.0;
                    let y = response.rect.center().y;
                    let arrow = if duplicate_editor.picker_open {
                        vec![
                            egui::pos2(x - 4.0, y + 2.0),
                            egui::pos2(x + 4.0, y + 2.0),
                            egui::pos2(x, y - 2.0),
                        ]
                    } else {
                        vec![
                            egui::pos2(x - 4.0, y - 2.0),
                            egui::pos2(x + 4.0, y - 2.0),
                            egui::pos2(x, y + 2.0),
                        ]
                    };
                    ui.painter().add(egui::Shape::convex_polygon(
                        arrow,
                        ui.visuals().text_color(),
                        egui::Stroke::NONE,
                    ));
                    if duplicate_editor.picker_open {
                        group_picker_scroll().show(ui, |ui| {
                            for key in data.groups.keys() {
                                if group_option_row(
                                    ui,
                                    key.as_str(),
                                    duplicate_editor.selected == key.as_str(),
                                )
                                .clicked()
                                {
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
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut duplicate_editor.name)
                                .hint_text("New group name"),
                        )
                        .changed()
                    {
                        duplicate_editor.error = None;
                        duplicate_editor.success = None;
                    }
                    ui.label("Creates a separate copy in the active profile.");
                    if files_pending {
                        ui.label("Wait for dropped files to finish before duplicating.");
                    }
                    let trimmed = duplicate_editor.name.trim();
                    if !trimmed.is_empty() && data.groups.contains_key(trimmed) {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "A group with that name already exists.",
                        );
                    }
                    let available = !duplicate_editor.selected.is_empty()
                        && !trimmed.is_empty()
                        && !data.groups.contains_key(trimmed)
                        && !files_pending;
                    if ui
                        .add_enabled(available, Button::new("Duplicate group"))
                        .clicked()
                    {
                        requested_duplicate =
                            Some((duplicate_editor.selected.clone(), trimmed.to_owned()));
                    }
                    if let Some(error) = &duplicate_editor.error {
                        ui.colored_label(ui.visuals().warn_fg_color, error);
                    }
                });
            if duplicate_section.header_response.clicked() {
                duplicate_editor.success = None;
            }
            ui.separator();
            let export_section = egui::CollapsingHeader::new("Export groups")
                .id_salt("export-groups-section")
                .show(ui, |ui| {
                    if data.groups.is_empty() {
                        export_editor.selected.clear();
                        export_editor.picker_open = false;
                        export_editor.success = None;
                        export_editor.error = None;
                        ui.label("No groups to export.");
                        return;
                    }
                    if let Some(message) = &export_editor.success {
                        ui.colored_label(egui::Color32::from_rgb(100, 200, 120), message);
                    }
                    if let Some(error) = &export_editor.error {
                        ui.colored_label(ui.visuals().warn_fg_color, error);
                    }
                    ui.label("Choose a group to export:");
                    if !data.groups.contains_key(&export_editor.selected) {
                        export_editor.selected.clear();
                    }
                    let response = ui.add(
                        Button::new(format!(
                            "{}    ",
                            if export_editor.selected.is_empty() {
                                "Choose group"
                            } else {
                                export_editor.selected.as_str()
                            }
                        ))
                        .min_size(egui::vec2(110.0, ui.spacing().interact_size.y)),
                    );
                    if response.clicked() {
                        export_editor.picker_open = !export_editor.picker_open;
                    }
                    let x = response.rect.right() - 12.0;
                    let y = response.rect.center().y;
                    let arrow = if export_editor.picker_open {
                        vec![
                            egui::pos2(x - 4.0, y + 2.0),
                            egui::pos2(x + 4.0, y + 2.0),
                            egui::pos2(x, y - 2.0),
                        ]
                    } else {
                        vec![
                            egui::pos2(x - 4.0, y - 2.0),
                            egui::pos2(x + 4.0, y - 2.0),
                            egui::pos2(x, y + 2.0),
                        ]
                    };
                    ui.painter().add(egui::Shape::convex_polygon(
                        arrow,
                        ui.visuals().text_color(),
                        egui::Stroke::NONE,
                    ));
                    if export_editor.picker_open {
                        group_picker_scroll().show(ui, |ui| {
                            for key in data.groups.keys() {
                                if group_option_row(
                                    ui,
                                    key.as_str(),
                                    export_editor.selected == key.as_str(),
                                )
                                .clicked()
                                {
                                    export_editor.selected = key.clone();
                                    export_editor.picker_open = false;
                                    export_editor.error = None;
                                }
                            }
                        });
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut export_editor.kind, GroupExportKind::Json, "JSON");
                        ui.radio_value(&mut export_editor.kind, GroupExportKind::Txt, "TXT");
                    });
                    ui.label(match export_editor.kind {
                        GroupExportKind::Json => "Preserves mod order, settings and descriptions.",
                        GroupExportKind::Txt => "One mod path or URL per line; no settings.",
                    });
                    if ui
                        .add_enabled(
                            data.groups.contains_key(&export_editor.selected),
                            Button::new("Export group"),
                        )
                        .clicked()
                    {
                        let kind = export_editor.kind;
                        let extension = kind.extension();
                        let filter = match kind {
                            GroupExportKind::Json => "JSON group export",
                            GroupExportKind::Txt => "MINT mod list",
                        };
                        requested_export = rfd::FileDialog::new()
                            .add_filter(filter, &[extension])
                            .set_file_name(format!("mintfixed-group.{extension}"))
                            .save_file()
                            .map(|path| {
                                (
                                    path.with_extension(extension),
                                    export_editor.selected.clone(),
                                    kind,
                                )
                            });
                    }
                });
            if export_section.header_response.clicked() {
                export_editor.success = None;
                export_editor.error = None;
            }
            ui.separator();
            let import_section = egui::CollapsingHeader::new("Import groups")
            .id_salt("import-groups-section")
            .show(ui, |ui| {
                if let Some(message) = &import_editor.success {
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 120), message);
                }
                if let Some(error) = &import_editor.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
                ui.label("Import one JSON group or a MINT-style TXT mod list.");
                ui.label("Imported groups are added to the active profile; existing groups are never overwritten.");
                if files_pending {
                    ui.label("Wait for dropped files to finish before importing.");
                }
                if ui.add_enabled(!files_pending, Button::new("Choose file to import...")).clicked() {
                    requested_import = rfd::FileDialog::new()
                        .add_filter("Group exports and mod lists", &["json", "txt"])
                        .pick_file();
                }
                if let Some(pending) = &import_editor.pending {
                    if let Some(suggested) = &pending.suggested_name {
                        ui.label(format!("Group in file: {suggested}"));
                    } else {
                        ui.label("This file has no group name. Please enter one.");
                    }
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(&mut import_editor.name)
                        .hint_text("New group name"));
                    let candidate = import_editor.name.trim().to_owned();
                    if !candidate.is_empty() && data.groups.contains_key(candidate.as_str()) {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "A group with that name already exists.",
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            import_editor.pending = None;
                            import_editor.name.clear();
                            import_editor.error = None;
                        }
                        if ui.add_enabled(
                            !files_pending && !candidate.is_empty()
                                && !data.groups.contains_key(candidate.as_str()),
                            Button::new("Import group"),
                        ).clicked() {
                            requested_import_name = Some(candidate);
                        }
                    });
                }
            });
            if import_section.header_response.clicked() {
                import_editor.success = None;
                import_editor.error = None;
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
        },
    );
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
    if let Some((path, name, kind)) = requested_export {
        let result = match kind {
            GroupExportKind::Json => serialize_single_group(data, &name),
            GroupExportKind::Txt => export_group_mod_list(data, &name),
        }
        .and_then(|contents| {
            std::fs::write(&path, contents).map_err(|e| format!("Failed to save export: {e}"))
        });
        match result {
            Ok(()) => {
                let message = format!("Exported group \"{name}\" to {}", path.display());
                export_editor.success = Some(message.clone());
                export_editor.error = None;
                success_message = Some(message);
            }
            Err(error) => {
                export_editor.success = None;
                export_editor.error = Some(error);
            }
        }
        ui.ctx().request_repaint();
    }
    if let Some(path) = requested_import {
        match read_single_group_import(&path) {
            Ok(pending) => {
                import_editor.name = pending.suggested_name.clone().unwrap_or_default();
                import_editor.pending = Some(pending);
                import_editor.error = None;
                import_editor.success = None;
            }
            Err(error) => {
                import_editor.pending = None;
                import_editor.name.clear();
                import_editor.success = None;
                import_editor.error = Some(error);
            }
        }
        ui.ctx().request_repaint();
    }
    if let Some(name) = requested_import_name {
        if let Some(pending) = &import_editor.pending {
            // Explicit name validation. Never silently rename or overwrite a group.
            if data.groups.contains_key(&name) {
                import_editor.error = Some("A group with that name already exists".into());
            } else {
                let source =
                    std::collections::BTreeMap::from([(name.clone(), pending.group.clone())]);
                match import_groups(data, source) {
                    Ok(_) => {
                        changed = true;
                        let message = format!("Imported group \"{name}\" into the active profile.");
                        import_editor.pending = None;
                        import_editor.name.clear();
                        import_editor.error = None;
                        import_editor.success = Some(message.clone());
                        success_message = Some(message);
                        egui::DragAndDrop::clear_payload(ui.ctx());
                    }
                    Err(error) => import_editor.error = Some(error),
                }
            }
        }
        ui.ctx().request_repaint();
    }
    ui.ctx().data_mut(|d| {
        d.insert_temp(export_id, export_editor);
        d.insert_temp(import_id, import_editor);
        d.insert_temp(editor_id, editor);
        d.insert_temp(duplicate_id, duplicate_editor);
        d.insert_temp(delete_id, delete_editor);
    });
    (changed, success_message)
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
        }))
        .unwrap()
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
        let mut restored: Data =
            serde_json::from_value(serde_json::to_value(&data).unwrap()).unwrap();
        rename_group(&mut restored, "Utilities", "Tools").unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), before);
    }

    #[test]
    fn duplicate_group_keeps_mods_and_switches_but_not_other_profile_references() {
        let mut data = fixture();
        let source = serde_json::to_value(&data.groups["Tools"]).unwrap();
        let other_profile = serde_json::to_value(&data.profiles["B"]).unwrap();
        assert_eq!(
            duplicate_group(&mut data, "Tools", "  Tools Backup  ").unwrap(),
            "Tools Backup"
        );
        let copy = &data.groups["Tools Backup"];
        let original = &data.groups["Tools"];
        assert_ne!(copy.id, original.id);
        assert_eq!(copy.mods.len(), original.mods.len());
        for (a, b) in copy.mods.iter().zip(&original.mods) {
            assert_ne!(a.id, b.id);
            if b.member_id.is_some() {
                assert_ne!(a.member_id, b.member_id);
            }
            assert_eq!(
                (&a.spec, a.enabled, a.required, a.priority),
                (&b.spec, b.enabled, b.required, b.priority)
            );
        }
        assert_eq!(serde_json::to_value(&data.groups["Tools"]).unwrap(), source);
        assert_eq!(
            serde_json::to_value(&data.profiles["B"]).unwrap(),
            other_profile
        );
        assert!(matches!(&data.profiles["A"].mods[0], ModOrGroup::Group {
            group_name, enabled: true, member_overrides, .. } if group_name == "Tools Backup" && !member_overrides.enabled(&data.groups["Tools Backup"].mods[0])));
        data.groups.get_mut("Tools Backup").unwrap().mods[0].priority = 99;
        assert_eq!(data.groups["Tools"].mods[0].priority, 7);
    }

    #[test]
    fn duplicate_group_rejects_invalid_names_without_changes() {
        let mut data = fixture();
        let before = serde_json::to_value(&data).unwrap();
        for (source, name) in [
            ("Missing", "New"),
            ("Tools", "  "),
            ("Tools", "Other"),
            ("Tools", "Tools"),
        ] {
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
        assert!(matches!(
            data.profiles["A"].mods[0],
            ModOrGroup::Individual(_)
        ));
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

    #[test]
    fn removing_shared_group_member_clears_overrides_in_every_profile() {
        let mut data = fixture();
        let member_id = data.groups["Tools"].mods[0].id.clone();
        let other_before = serde_json::to_value(&data.groups["Other"]).unwrap();
        remove_group_member(&mut data, "Tools", &member_id).unwrap();
        assert!(data.groups["Tools"].mods.is_empty());
        assert_eq!(
            serde_json::to_value(&data.groups["Other"]).unwrap(),
            other_before
        );
        for entry in data.profiles.values().flat_map(|profile| &profile.mods) {
            if let ModOrGroup::Group {
                group_name,
                member_overrides,
                ..
            } = entry
                && group_name == "Tools"
            {
                assert_eq!(
                    serde_json::to_value(member_overrides).unwrap(),
                    serde_json::to_value(crate::state::MemberOverrides::default()).unwrap()
                );
            }
        }
    }

    #[test]
    fn removing_missing_group_member_keeps_data_unchanged() {
        let mut data = fixture();
        let before = serde_json::to_value(&data).unwrap();
        assert!(remove_group_member(&mut data, "Tools", "no-such-id").is_err());
        assert!(remove_group_member(&mut data, "Missing", "no-such-id").is_err());
        assert_eq!(serde_json::to_value(&data).unwrap(), before);
    }
}
