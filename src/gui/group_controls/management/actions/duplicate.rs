use super::*;

pub(super) fn show_duplicate(
    ui: &mut Ui,
    data: &Data,
    duplicate_editor: &mut DuplicateEditor,
    files_pending: bool,
    requested_duplicate: &mut Option<(String, String)>,
) {
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
                available,
            )
            .clicked()
            {
                *requested_duplicate =
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
}
