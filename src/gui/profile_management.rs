//! Profile Management popup (create, rename, import and delete profiles).
//! Kept separate from the reusable profile selector and the JSON importer.

use super::is_committed;
use super::named_combobox::NamedEntries;

// Persistent input state for the single profile management menu.
#[derive(Debug, Clone, Default)]
struct ProfileManagementEditor {
    new_name: String,
    rename_name: String,
    rename_target: String,
    delete_target: Option<String>,
}

/// Four profile actions in one popup. Inline editors avoid nesting egui popups,
/// which would otherwise close each other or lose their text-field focus.
pub(super) fn ui<E, N>(
    ui: &mut egui::Ui,
    name: &str,
    entries: &mut N,
    modified: &mut bool,
    import_ui: Option<impl FnOnce(&mut egui::Ui, &mut N, &mut bool)>,
) where
    N: NamedEntries<E>,
{
    let response = ui
        .button("Profile Management")
        .on_hover_text("Create, rename, import or delete a profile");
    let popup_id = ui.make_persistent_id(format!("manage-{name}"));
    let state_id = popup_id.with("fields");
    let mut editor: ProfileManagementEditor =
        ui.data(|data| data.get_temp(state_id)).unwrap_or_default();

    if response.clicked() {
        if ui.memory(|mem| mem.is_popup_open(popup_id)) {
            ui.memory_mut(|mem| mem.close_popup());
        } else {
            editor = ProfileManagementEditor {
                rename_name: entries.selected_name().to_owned(),
                rename_target: entries.selected_name().to_owned(),
                ..Default::default()
            };
            ui.memory_mut(|mem| mem.open_popup(popup_id));
        }
    }

    let mut import_ui = import_ui;
    // Constrain the popup to the room below the toolbar. Keep it tall on
    // regular windows, but avoid pushing actions below the visible screen.
    let available_height = (ui.ctx().screen_rect().bottom() - response.rect.bottom() - 12.0)
        .max(120.0)
        .min(520.0);
    egui::popup_below_widget(
        ui,
        popup_id,
        &response,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_min_width(385.0);
            // The menu reaches farther down than the old 390-point panel.
            // Extra space remains available when the sections are collapsed;
            // lengthy import content still scrolls instead of covering buttons.
            egui::ScrollArea::vertical()
                .id_salt("profile-management-scroll")
                .max_height(available_height)
                .show(ui, |ui| {
                    ui.set_min_height(available_height - 8.0);
                    egui::CollapsingHeader::new("New Profile")
                        .id_salt("manage-new-profile")
                        .show(ui, |ui| {
                            let field = ui.add(
                                egui::TextEdit::singleline(&mut editor.new_name)
                                    .hint_text("New profile name"),
                            );
                            let candidate = editor.new_name.trim().to_owned();
                            let valid = !candidate.is_empty() && !entries.contains(&candidate);
                            if !candidate.is_empty() && entries.contains(&candidate) {
                                ui.colored_label(
                                    ui.visuals().warn_fg_color,
                                    "Profile name already exists.",
                                );
                            }
                            let clicked = ui
                                .add_enabled(valid, egui::Button::new("Create Profile"))
                                .clicked();
                            if valid && (clicked || is_committed(&field)) {
                                entries.add_new(&candidate);
                                *modified = true;
                                ui.memory_mut(|mem| mem.close_popup());
                            }
                        });
                    ui.separator();
                    egui::CollapsingHeader::new("Rename Profile")
                        .id_salt("manage-rename-profile")
                        .show(ui, |ui| {
                            let current = editor.rename_target.as_str();
                            ui.label(format!("Renaming: {current}"));
                            let field = ui.add(
                                egui::TextEdit::singleline(&mut editor.rename_name)
                                    .hint_text("New profile name"),
                            );
                            let candidate = editor.rename_name.trim().to_owned();
                            let valid = !candidate.is_empty()
                                && candidate != current
                                && !entries.contains(&candidate)
                                && entries.selected_name() == current;
                            if !candidate.is_empty()
                                && candidate != current
                                && entries.contains(&candidate)
                            {
                                ui.colored_label(
                                    ui.visuals().warn_fg_color,
                                    "Profile name already exists.",
                                );
                            }
                            let clicked = ui
                                .add_enabled(valid, egui::Button::new("Rename Profile"))
                                .clicked();
                            if valid && (clicked || is_committed(&field)) {
                                entries.rename_selected(candidate);
                                *modified = true;
                                ui.memory_mut(|mem| mem.close_popup());
                            }
                        });
                    ui.separator();
                    egui::CollapsingHeader::new("Import Profile (JSON)")
                        .id_salt("manage-import-profile")
                        .show(ui, |ui| {
                            // This renders the actual importer directly inside this menu;
                            // no nested popup or competing keyboard focus.
                            if let Some(import_ui) = import_ui.take() {
                                import_ui(ui, entries, modified);
                            }
                        });
                    ui.separator();
                    egui::CollapsingHeader::new("Delete Profile")
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
                                    if ui.button("Cancel").clicked() {
                                        editor.delete_target = None;
                                    }
                                    let can_delete =
                                        entries.selected_name() == target && entries.len() > 1;
                                    if ui
                                        .add_enabled(
                                            can_delete,
                                            egui::Button::new("Confirm Delete"),
                                        )
                                        .clicked()
                                    {
                                        entries.remove_selected();
                                        *modified = true;
                                        editor.delete_target = None;
                                        ui.memory_mut(|mem| mem.close_popup());
                                    }
                                });
                            } else if ui.button("Delete active profile...").clicked() {
                                // Capture the name so switching profiles cannot delete another one.
                                editor.delete_target = Some(entries.selected_name().to_owned());
                            }
                        });
                });
        },
    );
    ui.data_mut(|data| data.insert_temp(state_id, editor));
}
