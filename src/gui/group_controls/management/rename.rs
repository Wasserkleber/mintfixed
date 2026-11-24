fn show_rename(
    ui: &mut Ui,
    data: &Data,
    editor: &mut RenameEditor,
    files_pending: bool,
    requested_rename: &mut Option<(String, String)>,
) {
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
            *requested_rename = Some((editor.selected.clone(), trimmed.to_owned()));
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
}
