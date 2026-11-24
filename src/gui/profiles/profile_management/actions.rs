//! Create, rename, duplicate and delete sections of the management popup.
use super::{ProfileManagementEditor, picker::choose_profile};
use crate::gui::{is_committed, named_combobox::NamedEntries};

pub(super) fn show_create<E, N>(
    ui: &mut egui::Ui,
    entries: &mut N,
    editor: &mut ProfileManagementEditor,
    modified: &mut bool,
) where
    N: NamedEntries<E>,
{
    ui.label("New profile");
    let field = ui.text_edit_singleline(&mut editor.new_name);
    let candidate = editor.new_name.trim().to_owned();
    let valid = !candidate.is_empty() && !entries.contains(&candidate);
    if !candidate.is_empty() && entries.contains(&candidate) {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            "A profile with that name already exists.",
        );
    }
    let clicked = ui
        .add_enabled(valid, crate::gui::mint_text_button("Create profile"))
        .clicked();
    if valid && (clicked || is_committed(&field)) {
        entries.add_new(&candidate);
        *modified = true;
        editor.new_name.clear();
        ui.memory_mut(|mem| mem.close_popup());
    }
}

pub(super) fn show_rename<E, N>(
    ui: &mut egui::Ui,
    entries: &mut N,
    editor: &mut ProfileManagementEditor,
    modified: &mut bool,
) where
    N: NamedEntries<E>,
{
    let rename_section = egui::CollapsingHeader::new("Rename Profile")
        .id_salt("manage-rename-profile")
        .show(ui, |ui| {
            if choose_profile(
                ui,
                entries,
                &mut editor.rename_target,
                &mut editor.rename_picker_open,
            ) {
                editor.rename_success = None;
            }
            let current = editor.rename_target.clone();
            let field = ui.add(
                egui::TextEdit::singleline(&mut editor.rename_name).hint_text("New profile name"),
            );
            if field.changed() {
                editor.rename_success = None;
            }
            let candidate = editor.rename_name.trim().to_owned();
            let valid = !current.is_empty()
                && entries.contains(&current)
                && !candidate.is_empty()
                && candidate != current
                && !entries.contains(&candidate);
            if !candidate.is_empty() && candidate != current && entries.contains(&candidate) {
                ui.colored_label(ui.visuals().warn_fg_color, "Profile name already exists.");
            }
            let clicked = ui
                .add_enabled(valid, crate::gui::mint_text_button("Rename Profile"))
                .clicked();
            if valid && (clicked || is_committed(&field)) {
                entries.rename_named(&current, candidate.clone());
                *modified = true;
                editor.rename_name.clear();
                editor.rename_target.clear();
                editor.rename_picker_open = false;
                editor.rename_success =
                    Some(format!("Renamed profile \"{current}\" to \"{candidate}\"."));
            }
            if let Some(message) = &editor.rename_success {
                ui.colored_label(egui::Color32::from_rgb(100, 200, 120), message);
            }
        });
    crate::gui::management_popup::scroll_expanded_header(
        &rename_section.header_response,
        rename_section.body_response.is_some(),
    );
    if rename_section.header_response.clicked() {
        editor.rename_picker_open = false;
    }
}

pub(super) fn show_duplicate<E, N>(
    ui: &mut egui::Ui,
    entries: &mut N,
    editor: &mut ProfileManagementEditor,
    modified: &mut bool,
) where
    N: NamedEntries<E>,
{
    let duplicate_section = egui::CollapsingHeader::new("Duplicate Profile")
        .id_salt("manage-duplicate-profile")
        .show(ui, |ui| {
            let previous_target = editor.duplicate_target.clone();
            if choose_profile(
                ui,
                entries,
                &mut editor.duplicate_target,
                &mut editor.duplicate_picker_open,
            ) {
                editor.duplicate_success = None;
                if editor.duplicate_name.is_empty()
                    || editor.duplicate_name == format!("{previous_target} - Copy")
                {
                    editor.duplicate_name = format!("{} - Copy", editor.duplicate_target);
                }
            }
            let target = editor.duplicate_target.clone();
            let field = ui.add(
                egui::TextEdit::singleline(&mut editor.duplicate_name)
                    .hint_text("Name for duplicated profile"),
            );
            if field.changed() {
                editor.duplicate_success = None;
            }
            let requested = editor.duplicate_name.trim().to_owned();
            let exists = entries.contains(&requested);
            if !requested.is_empty() && exists {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "A profile with that name already exists.",
                );
            }
            let valid =
                !target.is_empty() && entries.contains(&target) && !requested.is_empty() && !exists;
            let clicked = ui
                .add_enabled(valid, crate::gui::mint_text_button("Duplicate Profile"))
                .clicked();
            if valid && (clicked || is_committed(&field)) {
                entries.duplicate_named(&target, requested.clone());
                *modified = true;
                editor.duplicate_name.clear();
                editor.duplicate_target.clear();
                editor.duplicate_picker_open = false;
                editor.duplicate_success = Some(format!(
                    "Duplicated profile \"{target}\" as \"{requested}\"."
                ));
            }
            if let Some(message) = &editor.duplicate_success {
                ui.colored_label(egui::Color32::from_rgb(100, 200, 120), message);
            }
        });
    crate::gui::management_popup::scroll_expanded_header(
        &duplicate_section.header_response,
        duplicate_section.body_response.is_some(),
    );
    if duplicate_section.header_response.clicked() {
        editor.duplicate_picker_open = false;
    }
}

pub(super) fn show_delete<E, N>(
    ui: &mut egui::Ui,
    entries: &mut N,
    editor: &mut ProfileManagementEditor,
    modified: &mut bool,
    popup_id: egui::Id,
) where
    N: NamedEntries<E>,
{
    let delete_section = egui::CollapsingHeader::new("Delete Profile")
        .id_salt("manage-delete-profile")
        .show(ui, |ui| {
            if entries.len() <= 1 {
                ui.label("At least one profile must remain.");
            } else if let Some(target) = editor.delete_target.clone() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("Delete profile \"{target}\"? This cannot be undone."),
                );
                ui.horizontal(|ui| {
                    if ui.add(crate::gui::mint_text_button("Cancel")).clicked() {
                        editor.delete_target = None;
                    }
                    let can_delete = entries.selected_name() == target && entries.len() > 1;
                    if ui
                        .add_enabled(can_delete, crate::gui::mint_text_button("Confirm Delete"))
                        .clicked()
                    {
                        entries.remove_selected();
                        *modified = true;
                        editor.delete_target = None;
                        ui.memory_mut(|mem| mem.close_popup());
                    }
                });
            } else if ui
                .add(crate::gui::mint_text_button("Delete active profile..."))
                .clicked()
            {
                // Capture the name so switching profiles cannot delete another one.
                editor.delete_target = Some(entries.selected_name().to_owned());
            }
        });
    crate::gui::management_popup::scroll_expanded_header(
        &delete_section.header_response,
        delete_section.body_response.is_some(),
    );
    let bottom = delete_section
        .body_response
        .as_ref()
        .map_or(delete_section.header_response.rect.bottom(), |r| {
            r.rect.bottom()
        });
    crate::gui::management_popup::record_content_end(ui, popup_id, bottom);
}
