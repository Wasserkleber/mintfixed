/// Returns whether groups changed and an optional success message for the main status bar.
pub(in crate::gui) fn controls(
    ui: &mut Ui,
    data: &mut Data,
    name: &mut String,
    files_pending: bool,
    configured_mod_folder: Option<&std::path::Path>,
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
    let mut requested_export: Option<(PathBuf, String, GroupExportKind, bool)> = None;
    let mut requested_import: Option<PathBuf> = None;
    let mut requested_import_name: Option<String> = None;
    let mut success_message = None;

    // ZIP copying and SHA-256 verification run off the GUI thread.
    if let Some(job) = &export_editor.job {
        let completed = job.lock().ok().and_then(|mut value| value.take());
        if let Some(result) = completed {
            export_editor.job = None;
            match result {
                Ok(path) => {
                    let message = format!("Exported group package to {}", path.display());
                    export_editor.success = Some(message.clone());
                    export_editor.error = None;
                    success_message = Some(message);
                }
                Err(error) => {
                    export_editor.success = None;
                    export_editor.error = Some(error);
                }
            }
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
        }
    }
    if let Some(job) = &import_editor.job {
        let completed = job.lock().ok().and_then(|mut value| value.take());
        if let Some(result) = completed {
            import_editor.job = None;
            match result {
                Ok((name, group)) => {
                    import_editor.pending = Some(PendingGroupImport {
                        suggested_name: Some(name.clone()), group,
                    });
                    import_editor.bundle_path = None;
                    requested_import_name = Some(name);
                }
                Err(error) => import_editor.error = Some(error),
            }
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
        }
    }
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
    crate::gui::management_popup::show(
        ui,
        popup,
        &button,
        |ui| {
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
                let attach_section = egui::CollapsingHeader::new("Add existing group")
                    .id_salt(("attach-existing", &data.active_profile))
                    .show(ui, |ui| {
                        // Keep the expanded list compact; additional groups remain scrollable.
                        group_picker_scroll().show(ui, |ui| {
                            for key in eligible {
                                if group_option_row(
                                    ui,
                                    key.as_str(),
                                    false,
                                    data.groups
                                        .get(key.as_str())
                                        .and_then(|group| group.tint_color.as_deref()),
                                )
                                .clicked()
                                {
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
                crate::gui::management_popup::scroll_expanded_header(
                    &attach_section.header_response,
                    attach_section.body_response.is_some(),
                );
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
                group_picker_button(
                    ui,
                    &editor.selected,
                    data.groups
                        .get(&editor.selected)
                        .and_then(|group| group.tint_color.as_deref()),
                    &mut editor.picker_open,
                    "Choose a group to rename",
                );
                if editor.picker_open {
                    group_picker_scroll().show(ui, |ui| {
                        for key in data.groups.keys() {
                            if group_option_row(
                                ui,
                                key.as_str(),
                                editor.selected == key.as_str(),
                                data.groups
                                    .get(key.as_str())
                                    .and_then(|group| group.tint_color.as_deref()),
                            )
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
                if group_action_button(
                    ui,
                    "Rename in all profiles",
                    data.groups
                        .get(&editor.selected)
                        .and_then(|g| g.tint_color.as_deref()),
                    available && !files_pending,
                )
                .clicked()
                {
                    requested_rename = Some((editor.selected.clone(), trimmed.to_owned()));
                }
                if let Some(error) = &editor.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
            });
            crate::gui::management_popup::scroll_expanded_header(
                &rename_section.header_response,
                rename_section.body_response.is_some(),
            );
            if rename_section.header_response.clicked() {
                editor.success = None;
                editor.picker_open = false;
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
                    group_picker_button(
                        ui,
                        &duplicate_editor.selected,
                        data.groups
                            .get(&duplicate_editor.selected)
                            .and_then(|group| group.tint_color.as_deref()),
                        &mut duplicate_editor.picker_open,
                        "Choose a group to duplicate",
                    );
                    if duplicate_editor.picker_open {
                        group_picker_scroll().show(ui, |ui| {
                            for key in data.groups.keys() {
                                if group_option_row(
                                    ui,
                                    key.as_str(),
                                    duplicate_editor.selected == key.as_str(),
                                    data.groups
                                        .get(key.as_str())
                                        .and_then(|group| group.tint_color.as_deref()),
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
                    if group_action_button(
                        ui,
                        "Duplicate group",
                        data.groups
                            .get(&duplicate_editor.selected)
                            .and_then(|g| g.tint_color.as_deref()),
                        available,
                    )
                    .clicked()
                    {
                        requested_duplicate =
                            Some((duplicate_editor.selected.clone(), trimmed.to_owned()));
                    }
                    if let Some(error) = &duplicate_editor.error {
                        ui.colored_label(ui.visuals().warn_fg_color, error);
                    }
                });
            crate::gui::management_popup::scroll_expanded_header(
                &duplicate_section.header_response,
                duplicate_section.body_response.is_some(),
            );
            if duplicate_section.header_response.clicked() {
                duplicate_editor.success = None;
                duplicate_editor.picker_open = false;
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
                    group_picker_button(
                        ui,
                        &export_editor.selected,
                        data.groups
                            .get(&export_editor.selected)
                            .and_then(|group| group.tint_color.as_deref()),
                        &mut export_editor.picker_open,
                        "Choose a group to export",
                    );
                    if export_editor.picker_open {
                        group_picker_scroll().show(ui, |ui| {
                            for key in data.groups.keys() {
                                if group_option_row(
                                    ui,
                                    key.as_str(),
                                    export_editor.selected == key.as_str(),
                                    data.groups
                                        .get(key.as_str())
                                        .and_then(|group| group.tint_color.as_deref()),
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
        ui.checkbox(&mut export_editor.include_files, "Include local PAK/ZIP files (portable ZIP)");
        if export_editor.job.is_some() {
            ui.label("Packaging local mod files...");
        }
                    ui.label(if export_editor.include_files {
            "Creates one ZIP with the selected definition and local PAK/ZIP files. Remote URLs remain links."
        } else {
            match export_editor.kind {
                GroupExportKind::Json => "Preserves mod order, settings and descriptions.",
                GroupExportKind::Txt => "One mod path or URL per line; no settings.",
            }
        });
                    if group_action_button(
                        ui,
                        "Export group",
                        data.groups
                            .get(&export_editor.selected)
                            .and_then(|g| g.tint_color.as_deref()),
                        data.groups.contains_key(&export_editor.selected) && export_editor.job.is_none(),
        )
        .clicked()
                    {
                        let kind = export_editor.kind;
                        let include_files = export_editor.include_files;
        let extension = if include_files { "zip" } else { kind.extension() };
        match group_export_path(&export_editor.selected, extension) {
                            Ok(path) => {
                                requested_export = Some((path, export_editor.selected.clone(), kind, include_files));
                                export_editor.error = None;
                            }
                            Err(error) => {
                                export_editor.success = None;
                                export_editor.error = Some(error);
                            }
                        }
                    }
                });
            crate::gui::management_popup::scroll_expanded_header(
                &export_section.header_response,
                export_section.body_response.is_some(),
            );
            if export_section.header_response.clicked() {
                export_editor.success = None;
                export_editor.error = None;
                export_editor.picker_open = false;
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
                ui.label("Import a JSON/TXT group or a portable ZIP with local mod files.");
                ui.label("Imported groups are added to the active profile; existing groups are never overwritten.");
                if files_pending {
                    ui.label("Wait for dropped files to finish before importing.");
                }
                if ui.add_enabled(!files_pending, Button::new("Choose file to import...")).clicked() {
                    requested_import = rfd::FileDialog::new()
                        .add_filter("Group exports, mod lists and packages", &["json", "txt", "zip"])
                        .pick_file();
                }
                if import_editor.job.is_some() {
            ui.label("Verifying and extracting local mod files...");
        }
        if import_editor.bundle_path.is_some()
            && let Some(folder) = configured_mod_folder.filter(|folder| folder.is_dir()) {
                ui.label(format!(
                    "New bundled PAK/ZIP files will be saved directly in {}.",
                    folder.display(),
                ));
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
                        if ui.add_enabled(import_editor.job.is_none(), Button::new("Cancel")).clicked() {
            import_editor.pending = None;
            import_editor.bundle_path = None;
                            import_editor.name.clear();
                            import_editor.error = None;
                        }
                        if ui.add_enabled(
                            !files_pending && import_editor.job.is_none() && !candidate.is_empty()
                                && !data.groups.contains_key(candidate.as_str()),
                            Button::new("Import group"),
                        ).clicked() {
                            requested_import_name = Some(candidate);
                        }
                    });
                }
            });
            crate::gui::management_popup::scroll_expanded_header(
                &import_section.header_response,
                import_section.body_response.is_some(),
            );
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
                let response = group_picker_button(
                    ui,
                    &delete_editor.selected,
                    data.groups.get(&delete_editor.selected)
                        .and_then(|group| group.tint_color.as_deref()),
                    &mut delete_editor.picker_open,
                    "Choose a group to delete",
                );
                if response.clicked() {
                    delete_editor.confirming = false;
                }
                if delete_editor.picker_open {
                    group_picker_scroll().show(ui, |ui| {
                        for key in data.groups.keys() {
                            if group_option_row(
                                    ui,
                                    key.as_str(),
                                    delete_editor.selected == key.as_str(),
                                    data.groups.get(key.as_str()).and_then(|group| group.tint_color.as_deref()),
                                ).clicked() {
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
                        if ui.add_enabled(!files_pending, Button::new("Confirm delete").fill(colors::DARK_RED)).clicked() {
                            requested_delete = Some(delete_editor.selected.clone());
                        }
                    });
                } else if group_action_button(
                    ui, "Delete group",
                    data.groups.get(&delete_editor.selected).and_then(|g| g.tint_color.as_deref()),
                    !delete_editor.selected.is_empty() && !files_pending,
                ).clicked() {
                    delete_editor.confirming = true;
                    delete_editor.success = None;
                }
                if let Some(error) = &delete_editor.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
            });
            crate::gui::management_popup::scroll_expanded_header(
                &delete_section.header_response,
                delete_section.body_response.is_some(),
            );
            if delete_section.header_response.clicked() {
                delete_editor.success = None;
                delete_editor.picker_open = false;
            }
            let bottom = delete_section.body_response.as_ref()
                .map_or(delete_section.header_response.rect.bottom(), |r| r.rect.bottom());
            crate::gui::management_popup::record_content_end(ui, popup, bottom);
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
    if let Some((path, name, kind, include_files)) = requested_export {
        if include_files {
            if let Some(group) = data.groups.get(&name).cloned() {
                let result = std::sync::Arc::new(std::sync::Mutex::new(None));
                let worker_result = result.clone();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let outcome = super::group_package::export_bundle(
                        &name, &group, &path, kind == GroupExportKind::Json,
                    ).map(|()| path);
                    if let Ok(mut slot) = worker_result.lock() { *slot = Some(outcome); }
                    ctx.request_repaint();
                });
                export_editor.job = Some(result);
                export_editor.success = None;
                export_editor.error = None;
            } else {
                export_editor.error = Some("The selected group no longer exists".into());
            }
        } else {
            let result = match kind {
                GroupExportKind::Json => serialize_single_group(data, &name),
                GroupExportKind::Txt => export_group_mod_list(data, &name),
            }.and_then(|contents| {
                use std::io::Write as _;
                std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
                    .and_then(|mut file| file.write_all(contents.as_bytes()))
                    .map_err(|e| format!("Failed to save export: {e}"))
                    .map(|()| contents)
            });
            match result {
                Ok(contents) => {
                    let copied = matches!(kind, GroupExportKind::Txt);
                    if copied {
                        ui.ctx().copy_text(contents);
                    }
                    let message = if copied {
                        format!("Exported group \"{name}\" to {} and copied TXT to clipboard", path.display())
                    } else {
                        format!("Exported group \"{name}\" to {}", path.display())
                    };
                    export_editor.success = Some(message.clone());
                    export_editor.error = None;
                    success_message = Some(message);
                }
                Err(error) => {
                    export_editor.success = None;
                    export_editor.error = Some(error);
                }
            }
        }
        ui.ctx().request_repaint();
    }
    if let Some(path) = requested_import {
        match read_single_group_import(&path) {
            Ok(pending) => {
                import_editor.bundle_path = (
                    path.extension().and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
                        && pending.group.mods.iter().any(|item| {
                            item.spec.url.starts_with("mintfixed-bundle:")
                        })
                ).then(|| path.clone());
                import_editor.name = pending.suggested_name.clone().unwrap_or_default();
                import_editor.pending = Some(pending);
                import_editor.error = None;
                import_editor.success = None;
            }
            Err(error) => {
                import_editor.bundle_path = None;
                import_editor.pending = None;
                import_editor.name.clear();
                import_editor.success = None;
                import_editor.error = Some(error);
            }
        }
        ui.ctx().request_repaint();
    }
    if let Some(name) = requested_import_name {
        if import_editor.pending.as_ref().is_some_and(|pending| {
            super::group_package::group_import_requires_mod_folder(
                &pending.group, import_editor.bundle_path.is_some(), configured_mod_folder,
            )
        }) {
            import_editor.error = Some(
                "Cannot import this group: configure a valid local mods folder in Settings first.".into(),
            );
        } else if let Some(archive) = import_editor.bundle_path.clone() {
            if import_editor.job.is_none() {
                if let Some(folder) = super::group_package::bundle_destination(configured_mod_folder) {
                    let result = std::sync::Arc::new(std::sync::Mutex::new(None));
                    let worker_result = result.clone();
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let outcome = super::group_package::extract_bundle(&archive, &folder)
                            .map(|group| (name, group));
                        if let Ok(mut slot) = worker_result.lock() { *slot = Some(outcome); }
                        ctx.request_repaint();
                    });
                    import_editor.job = Some(result);
                    import_editor.error = None;
                } else {
                    import_editor.error = Some(
                        "Cannot import this group: configure a valid local mods folder in Settings first.".into(),
                    );
                }
            }
        } else {
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
                        let missing = crate::gui::missing_local_files::group_missing_paths(&data.groups[&name]);
                        crate::gui::missing_local_files::notify(
                            ui.ctx(),
                            crate::gui::missing_local_files::Origin::Group(name.clone()),
                            missing,
                        );
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

