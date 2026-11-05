//! Profile Management popup (create, rename, import and delete profiles).
//! Kept separate from the reusable profile selector and the JSON importer.

use super::is_committed;
use super::named_combobox::NamedEntries;
use super::{ModData, ModOrGroup};
use std::collections::BTreeMap;

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
    copy_ui: Option<impl FnOnce(&mut egui::Ui, &mut N)>,
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

    let mut copy_ui = copy_ui;
    let mut import_ui = import_ui;
    egui::popup_below_widget(
        ui,
        popup_id,
        &response,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_min_width(385.0);
            // Match the Groups menu: size the popup to its expanded sections.
            // Only the JSON text editor has its own bounded scroll area.
            ui.vertical(|ui| {
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
                    egui::CollapsingHeader::new("Copy Profile")
                        .id_salt("manage-copy-profile")
                        .show(ui, |ui| {
                            if let Some(copy_ui) = copy_ui.take() {
                                copy_ui(ui, entries);
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

/// Copy the same TXT/JSON payloads as the former Copy Mods toolbar button.
/// STATUS feedback is returned to mod.rs without borrowing the parent App.
pub(super) fn copy_options(
    ui: &mut egui::Ui,
    mod_data: &ModData,
) -> Option<Result<String, String>> {
    ui.label("Copy the active profile to the clipboard.");
    if ui.button("Copy mod list (TXT)").clicked() {
        let mut mods = String::new();
        let active_profile = mod_data.active_profile.clone();
        mod_data.for_each_enabled_mod(&active_profile, |mc| {
            mods.push_str(&mc.spec.url);
            mods.push('\n');
        });
        ui.ctx().copy_text(mods);
        ui.memory_mut(|mem| mem.close_popup());
        return Some(Ok("Copied mod list (TXT) to clipboard".into()));
    }
    if ui.button("Copy profile with groups (JSON)").clicked() {
        let profile = &mod_data.profiles[&mod_data.active_profile];
        // Preserve ordering, disabled mods, group references, and member overrides.
        // Include only the group definitions used by this profile.
        let groups: BTreeMap<&str, &crate::state::ModGroup> = profile
            .mods
            .iter()
            .filter_map(|entry| match entry {
                ModOrGroup::Group { group_name, .. } => mod_data
                    .groups
                    .get(group_name)
                    .map(|group| (group_name.as_str(), group)),
                ModOrGroup::Individual(_) => None,
            })
            .collect();
        let payload = serde_json::json!({
            "format": "mintfixed-profile-mod-list",
            "version": 1,
            "profile_name": &mod_data.active_profile,
            "mods": &profile.mods,
            "groups": groups,
        });
        return Some(match serde_json::to_string_pretty(&payload) {
            Ok(json) => {
                ui.ctx().copy_text(json);
                ui.memory_mut(|mem| mem.close_popup());
                Ok("Copied profile with groups (JSON) to clipboard".into())
            }
            Err(error) => Err(format!("Could not copy group JSON: {error}")),
        });
    }
    None
}
