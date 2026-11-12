//! Profile Management popup (create, rename, duplicate, copy, import and delete profiles).
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
    rename_picker_open: bool,
    rename_success: Option<String>,
    duplicate_target: String,
    duplicate_name: String,
    duplicate_picker_open: bool,
    duplicate_success: Option<String>,
    export_feedback: Option<Result<String, String>>,
    delete_target: Option<String>,
}

// Inline, scrollable picker: a nested egui popup would close Profile Management.
fn profile_option_row(ui: &mut egui::Ui, name: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let highlighted = selected || response.hovered() || response.has_focus();
        if highlighted {
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(2),
                ui.visuals().selection.bg_fill,
            );
        }
        let text_color = if highlighted {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            egui::pos2(rect.left() + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::TextStyle::Button.resolve(ui.style()),
            text_color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn choose_profile<E, N>(
    ui: &mut egui::Ui,
    entries: &mut N,
    target: &mut String,
    picker_open: &mut bool,
) -> bool
where
    N: NamedEntries<E>,
{
    if !target.is_empty() && !entries.contains(target) {
        target.clear();
    }
    let response = ui.add(
        egui::Button::new(if target.is_empty() {
            "Choose profile"
        } else {
            target.as_str()
        })
        .min_size(egui::vec2(135.0, ui.spacing().interact_size.y)),
    );
    if response.clicked() {
        *picker_open = !*picker_open;
    }
    // Draw the arrow in the button, as in Group Management.
    let x = response.rect.right() - 12.0;
    let y = response.rect.center().y;
    let arrow = if *picker_open {
        [
            egui::pos2(x - 4.0, y + 2.0),
            egui::pos2(x + 4.0, y + 2.0),
            egui::pos2(x, y - 2.0),
        ]
    } else {
        [
            egui::pos2(x - 4.0, y - 2.0),
            egui::pos2(x + 4.0, y - 2.0),
            egui::pos2(x, y + 2.0),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        arrow.to_vec(),
        ui.visuals().text_color(),
        egui::Stroke::NONE,
    ));
    if !*picker_open {
        return false;
    }
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .max_height(140.0)
        .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .show(ui, |ui| {
            for (name, _) in entries.entries() {
                if profile_option_row(ui, name, target.as_str() == name.as_str()).clicked() {
                    chosen = Some(name.clone());
                }
            }
        });
    if let Some(name) = chosen {
        *target = name;
        *picker_open = false;
        return true;
    }
    false
}
/// Profile actions in one popup. Inline editors avoid nesting egui popups,
/// which would otherwise close each other or lose their text-field focus.
pub(super) fn ui<E, N>(
    ui: &mut egui::Ui,
    name: &str,
    entries: &mut N,
    modified: &mut bool,
    import_ui: Option<impl FnOnce(&mut egui::Ui, &mut N, &mut bool)>,
    copy_ui: Option<impl FnOnce(&mut egui::Ui, &mut N) -> Option<Result<String, String>>>,
) where
    N: NamedEntries<E>,
{
    let response = ui
        .button("Profile Management")
        .on_hover_text("Create, rename, duplicate, copy, import or delete a profile");
    let popup_id = ui.make_persistent_id(format!("manage-{name}"));
    let state_id = popup_id.with("fields");
    let mut editor: ProfileManagementEditor =
        ui.data(|data| data.get_temp(state_id)).unwrap_or_default();

    if response.clicked() {
        if ui.memory(|mem| mem.is_popup_open(popup_id)) {
            ui.memory_mut(|mem| mem.close_popup());
        } else {
            // Choose any source profile rather than silently locking actions to active.
            editor = ProfileManagementEditor::default();
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
            // Keep collapsed sections compact and wrap long messages instead of growing wide.
            ui.set_width(280.0);
            ui.spacing_mut().item_spacing.y = 3.0;
            // Match the Groups menu: size the popup to its expanded sections.
            // Only the JSON text editor has its own bounded scroll area.
            ui.vertical(|ui| {
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
                    .add_enabled(valid, egui::Button::new("Create profile"))
                    .clicked();
                if valid && (clicked || is_committed(&field)) {
                    entries.add_new(&candidate);
                    *modified = true;
                    editor.new_name.clear();
                    ui.memory_mut(|mem| mem.close_popup());
                }
                ui.separator();
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
                            egui::TextEdit::singleline(&mut editor.rename_name)
                                .hint_text("New profile name"),
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
                if rename_section.header_response.clicked() {
                    editor.rename_picker_open = false;
                }
                ui.separator();
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
                                editor.duplicate_name =
                                    format!("{} - Copy", editor.duplicate_target);
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
                        let valid = !target.is_empty()
                            && entries.contains(&target)
                            && !requested.is_empty()
                            && !exists;
                        let clicked = ui
                            .add_enabled(valid, egui::Button::new("Duplicate Profile"))
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
                if duplicate_section.header_response.clicked() {
                    editor.duplicate_picker_open = false;
                }
                ui.separator();
                egui::CollapsingHeader::new("Export Profile")
                    .id_salt("manage-copy-profile")
                    .show(ui, |ui| {
                        if let Some(copy_ui) = copy_ui.take()
                            && let Some(result) = copy_ui(ui, entries)
                        {
                            editor.export_feedback = Some(result);
                        }
                        if let Some(result) = &editor.export_feedback {
                            match result {
                                Ok(message) => {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(100, 200, 120),
                                        message,
                                    );
                                }
                                Err(message) => {
                                    ui.colored_label(ui.visuals().warn_fg_color, message);
                                }
                            }
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
                                    .add_enabled(can_delete, egui::Button::new("Confirm Delete"))
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
    ui.label("Export the active profile by copying it to the clipboard.");
    if ui.button("Copy mod list (TXT)").clicked() {
        let mut mods = String::new();
        let active_profile = mod_data.active_profile.clone();
        mod_data.for_each_enabled_mod(&active_profile, |mc| {
            mods.push_str(&mc.spec.url);
            mods.push('\n');
        });
        ui.ctx().copy_text(mods);
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
                Ok("Copied profile with groups (JSON) to clipboard".into())
            }
            Err(error) => Err(format!("Could not copy group JSON: {error}")),
        });
    }
    None
}
