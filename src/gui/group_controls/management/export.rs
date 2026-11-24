use super::*;

pub(super) fn show_export(
    ui: &mut Ui,
    data: &Data,
    export_editor: &mut ExportEditor,
    requested_export: &mut Option<(PathBuf, String, GroupExportKind, bool)>,
) {
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
                        *requested_export = Some((path, export_editor.selected.clone(), kind, include_files));
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
}
