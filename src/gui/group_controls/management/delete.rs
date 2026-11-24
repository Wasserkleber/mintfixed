use super::*;

pub(super) fn show_delete(
    ui: &mut Ui,
    data: &Data,
    delete_editor: &mut DeleteEditor,
    files_pending: bool,
    requested_delete: &mut Option<String>,
    popup: egui::Id,
) {
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
            *delete_editor = DeleteEditor::default();
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
                if ui.add(crate::gui::mint_text_button("Cancel")).clicked() {
                    delete_editor.confirming = false;
                }
                if ui.add_enabled(!files_pending, crate::gui::mint_text_button("Confirm delete").fill(colors::DARK_RED)).clicked() {
                    *requested_delete = Some(delete_editor.selected.clone());
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
}
