use super::*;

/// Returns whether groups changed and an optional success message for the main status bar.
pub(in crate::gui) fn controls(
    ui: &mut Ui,
    data: &mut Data,
    name: &mut String,
    files_pending: bool,
    configured_mod_folder: Option<&std::path::Path>,
) -> (bool, Option<String>) {
    let mut changed = false;
    let button = ui
        .add(crate::gui::mint_text_button("Group Management"))
        .on_hover_text("Manage groups");
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

    poll_package_jobs(
        ui,
        &mut export_editor,
        &mut import_editor,
        &mut requested_import_name,
        &mut success_message,
    );
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
    crate::gui::management_popup::show(ui, popup, &button, |ui| {
        show_creation_and_attachment(ui, data, name, &mut changed, &mut success_message);
        ui.separator();
        show_rename(ui, data, &mut editor, files_pending, &mut requested_rename);
        ui.separator();
        show_duplicate(
            ui,
            data,
            &mut duplicate_editor,
            files_pending,
            &mut requested_duplicate,
        );
        ui.separator();
        show_export(ui, data, &mut export_editor, &mut requested_export);
        ui.separator();
        show_import(
            ui,
            data,
            &mut import_editor,
            files_pending,
            configured_mod_folder,
            &mut requested_import,
            &mut requested_import_name,
        );
        ui.separator();
        show_delete(
            ui,
            data,
            &mut delete_editor,
            files_pending,
            &mut requested_delete,
            popup,
        );
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
    if let Some((path, name, kind, include_files)) = requested_export {
        if include_files {
            if let Some(group) = data.groups.get(&name).cloned() {
                let result = std::sync::Arc::new(std::sync::Mutex::new(None));
                let worker_result = result.clone();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let outcome = crate::gui::group_controls::group_package::export_bundle(
                        &name,
                        &group,
                        &path,
                        kind == GroupExportKind::Json,
                    )
                    .map(|()| path);
                    if let Ok(mut slot) = worker_result.lock() {
                        *slot = Some(outcome);
                    }
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
            }
            .and_then(|contents| {
                use std::io::Write as _;
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
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
                        format!(
                            "Exported group \"{name}\" to {} and copied TXT to clipboard",
                            path.display()
                        )
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
                import_editor.bundle_path = (path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
                    && pending
                        .group
                        .mods
                        .iter()
                        .any(|item| item.spec.url.starts_with("mintfixed-bundle:")))
                .then(|| path.clone());
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
            crate::gui::group_controls::group_package::group_import_requires_mod_folder(
                &pending.group,
                import_editor.bundle_path.is_some(),
                configured_mod_folder,
            )
        }) {
            import_editor.error = Some(
                "Cannot import this group: configure a valid local mods folder in Settings first."
                    .into(),
            );
        } else if let Some(archive) = import_editor.bundle_path.clone() {
            if import_editor.job.is_none() {
                if let Some(folder) = crate::gui::group_controls::group_package::bundle_destination(
                    configured_mod_folder,
                ) {
                    let result = std::sync::Arc::new(std::sync::Mutex::new(None));
                    let worker_result = result.clone();
                    let ctx = ui.ctx().clone();
                    std::thread::spawn(move || {
                        let outcome = crate::gui::group_controls::group_package::extract_bundle(
                            &archive, &folder,
                        )
                        .map(|group| (name, group));
                        if let Ok(mut slot) = worker_result.lock() {
                            *slot = Some(outcome);
                        }
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
                            let missing =
                                crate::state::local_files::group_missing_paths(&data.groups[&name]);
                            crate::gui::missing_local_files::notify(
                                ui.ctx(),
                                crate::gui::missing_local_files::Origin::Group(name.clone()),
                                missing,
                            );
                            let message =
                                format!("Imported group \"{name}\" into the active profile.");
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
