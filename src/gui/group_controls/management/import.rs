fn show_import(
    ui: &mut Ui,
    data: &Data,
    import_editor: &mut ImportEditor,
    files_pending: bool,
    configured_mod_folder: Option<&std::path::Path>,
    requested_import: &mut Option<PathBuf>,
    requested_import_name: &mut Option<String>,
) {
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
        if ui.add_enabled(!files_pending, crate::gui::mint_text_button("Choose file to import...")).clicked() {
            *requested_import = rfd::FileDialog::new()
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
                if ui.add_enabled(import_editor.job.is_none(), crate::gui::mint_text_button("Cancel")).clicked() {
    import_editor.pending = None;
    import_editor.bundle_path = None;
                    import_editor.name.clear();
                    import_editor.error = None;
                }
                if ui.add_enabled(
                    !files_pending && import_editor.job.is_none() && !candidate.is_empty()
                        && !data.groups.contains_key(candidate.as_str()),
                    crate::gui::mint_text_button("Import group"),
                ).clicked() {
                    *requested_import_name = Some(candidate);
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
}
