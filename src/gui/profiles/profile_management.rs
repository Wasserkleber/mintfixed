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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ProfileExportKind {
    #[default]
    Json,
    Txt,
}

impl ProfileExportKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Txt => "txt",
        }
    }
}

/// Create a new export file beside the executable, without replacing an old one.
fn save_profile_export(
    profile_name: &str,
    kind: ProfileExportKind,
    contents: &str,
) -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("Cannot locate the mintfixed executable: {error}"))?;
    save_profile_export_beside(&exe, profile_name, kind.extension(), contents)
}

fn save_profile_export_beside(
    exe: &std::path::Path,
    name: &str,
    extension: &str,
    contents: &str,
) -> Result<std::path::PathBuf, String> {
    use std::io::Write as _;

    let folder = exe
        .parent()
        .ok_or("Cannot determine the executable's folder")?
        .join("mintfixed exports");
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("Cannot create export folder {}: {error}", folder.display()))?;

    // Make profile names safe for Windows filenames (including reserved device names).
    let safe: String = name
        .chars()
        .take(100)
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    let safe = safe.trim_matches(|ch: char| ch == '.' || ch.is_whitespace());
    let safe = if safe.is_empty() { "profile" } else { safe };
    let device = safe
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(
        device.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    let safe = if reserved {
        format!("_{safe}")
    } else {
        safe.to_owned()
    };

    for number in 1..=10_000 {
        let filename = if number == 1 {
            format!("{safe}.{extension}")
        } else {
            format!("{safe} ({number}).{extension}")
        };
        let path = folder.join(filename);
        // Atomic create_new also handles another export starting at the same time.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(contents.as_bytes()) {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(format!(
                        "Could not write export {}: {error}",
                        path.display()
                    ));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Could not create export {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Err(format!(
        "Too many exports for profile \"{name}\" in {}",
        folder.display()
    ))
}

/// Export the selected profile as JSON, TXT or a portable ZIP bundle.
/// JSON preserves group definitions, order and overrides; TXT is enabled mod URLs.
// Portable profiles use the same export directory as JSON/TXT profiles and groups.
// The ZIP writer uses create_new(), so choosing a free path never overwrites a file.
fn portable_profile_export_path(name: &str) -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("Cannot locate the mintfixed executable: {error}"))?;
    portable_profile_export_path_beside(&exe, name)
}

fn portable_profile_export_path_beside(
    exe: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf, String> {
    let folder = exe
        .parent()
        .ok_or("Cannot determine the executable's folder")?
        .join("mintfixed exports");
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("Cannot create export folder {}: {error}", folder.display()))?;
    let mut safe = super::group_controls::group_package::safe_component(name);
    // Windows device names are invalid even when followed by .zip.
    if matches!(
        safe.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        safe.insert(0, '_');
    }
    for number in 1..=10_000 {
        let filename = if number == 1 {
            format!("{safe}-profile.zip")
        } else {
            format!("{safe}-profile ({number}).zip")
        };
        let path = folder.join(filename);
        match std::fs::metadata(&path) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(path),
            Err(error) => {
                return Err(format!(
                    "Cannot access export path {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Err(format!(
        "Too many portable profile exports in {}",
        folder.display()
    ))
}

pub(super) fn copy_options(
    ui: &mut egui::Ui,
    mod_data: &ModData,
) -> Option<Result<String, String>> {
    // One export state, shared by JSON/TXT and portable ZIP, like Group Export.
    type ExportState = std::sync::Arc<std::sync::Mutex<Option<Result<std::path::PathBuf, String>>>>;
    let job_id = ui.make_persistent_id("portable-profile-export-worker");
    let mut job: Option<ExportState> = ui.data(|d| d.get_temp(job_id));
    let mut feedback: Option<Result<String, String>> = None;

    if let Some(active) = job.clone() {
        let finished = active.lock().ok().and_then(|mut slot| slot.take());
        if let Some(result) = finished {
            job = None;
            feedback = Some(
                result.map(|path| format!("Exported portable profile ZIP to {}", path.display())),
            );
        } else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Packaging local mod files...");
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    ui.label("Choose a profile to export:");
    let profile_id = ui.make_persistent_id("portable-profile-export-target");
    let mut selected: String = ui
        .data(|d| d.get_temp(profile_id))
        .unwrap_or_else(|| mod_data.active_profile.clone());
    if !mod_data.profiles.contains_key(&selected) {
        selected = if mod_data.profiles.contains_key(&mod_data.active_profile) {
            mod_data.active_profile.clone()
        } else {
            mod_data.profiles.keys().next().cloned().unwrap_or_default()
        };
    }
    egui::ComboBox::from_id_salt("portable-profile-target-picker")
        .selected_text(if selected.is_empty() {
            "Choose profile"
        } else {
            selected.as_str()
        })
        .show_ui(ui, |ui| {
            for name in mod_data.profiles.keys() {
                ui.selectable_value(&mut selected, name.clone(), name);
            }
        });
    ui.data_mut(|d| d.insert_temp(profile_id, selected.clone()));

    let format_id = ui.id().with("profile-export-format");
    let mut kind = ui
        .data(|d| d.get_temp::<ProfileExportKind>(format_id))
        .unwrap_or_default();
    let bundle_id = ui.id().with("profile-export-include-local-paks");
    let mut include_local: bool = ui.data(|d| d.get_temp(bundle_id)).unwrap_or(false);

    ui.horizontal(|ui| {
        ui.radio_value(&mut kind, ProfileExportKind::Json, "JSON");
        // Portable profile packages must include a JSON definition to retain
        // groups, member overrides and priority settings on import.
        ui.add_enabled_ui(!include_local, |ui| {
            ui.radio_value(&mut kind, ProfileExportKind::Txt, "TXT");
        });
    });
    ui.checkbox(
        &mut include_local,
        "Include local PAK/ZIP files\n(portable ZIP)",
    );
    if include_local {
        kind = ProfileExportKind::Json;
        ui.label("Includes local mod files and preserves profile settings and groups.");
    } else {
        match kind {
            ProfileExportKind::Json => {
                ui.label("Preserves mod order, enabled states, settings and groups.");
            }
            ProfileExportKind::Txt => {
                ui.label("Exports enabled mod URLs only (one per line).");
            }
        }
    }
    ui.data_mut(|d| {
        d.insert_temp(format_id, kind);
        d.insert_temp(bundle_id, include_local);
    });

    if ui
        .add_enabled(
            job.is_none() && !selected.is_empty(),
            egui::Button::new("Export profile"),
        )
        .clicked()
    {
        if include_local {
            // Same worker, integrity checks and no-overwrite logic as the
            // existing portable-profile ZIP exporter; only its UI changes.
            let path = match portable_profile_export_path(&selected) {
                Ok(path) => Some(path),
                Err(error) => {
                    feedback = Some(Err(error));
                    None
                }
            };
            if let Some(path) = path {
                match super::profile_package::prepare_export(mod_data, &selected) {
                    Ok(snapshot) => {
                        let state: ExportState = Default::default();
                        let worker_state = std::sync::Arc::clone(&state);
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let result = super::profile_package::export_prepared(snapshot, &path)
                                .map(|_| path);
                            if let Ok(mut slot) = worker_state.lock() {
                                *slot = Some(result);
                            }
                            ctx.request_repaint();
                        });
                        job = Some(state);
                    }
                    Err(error) => feedback = Some(Err(error)),
                }
            }
        } else {
            let contents = match kind {
                ProfileExportKind::Txt => {
                    let mut urls = String::new();
                    mod_data.for_each_enabled_mod(&selected, |mc| {
                        urls.push_str(&mc.spec.url);
                        urls.push('\n');
                    });
                    Ok(urls)
                }
                ProfileExportKind::Json => {
                    let profile = &mod_data.profiles[&selected];
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
                        "profile_name": &selected,
                        "mods": &profile.mods,
                        "groups": groups,
                    });
                    serde_json::to_string_pretty(&payload)
                        .map_err(|error| format!("Could not serialize profile JSON: {error}"))
                }
            };
            feedback = Some(contents.and_then(|contents| {
                // Only copy after the TXT file has been saved successfully.
                let path = save_profile_export(&selected, kind, &contents)?;
                if matches!(kind, ProfileExportKind::Txt) {
                    ui.ctx().copy_text(contents);
                    Ok(format!(
                        "Exported profile \"{selected}\" to {} and copied TXT to clipboard",
                        path.display()
                    ))
                } else {
                    Ok(format!(
                        "Exported profile \"{selected}\" to {}",
                        path.display()
                    ))
                }
            }));
        }
    }

    if let Some(active) = job {
        ui.data_mut(|d| d.insert_temp(job_id, active));
    } else {
        ui.data_mut(|d| d.remove::<ExportState>(job_id));
    }
    feedback
}
#[cfg(test)]
mod profile_export_file_tests {
    use super::*;
    #[test]
    fn portable_zip_export_path_uses_group_export_folder_and_does_not_overwrite() {
        let base = std::env::temp_dir().join(format!(
            "mintfixed-portable-zip-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir_all(&base).unwrap();
        let exe = base.join("mintfixed.exe");
        let first = portable_profile_export_path_beside(&exe, "Test: Profile").unwrap();
        assert_eq!(first.parent().unwrap(), base.join("mintfixed exports"));
        assert_eq!(first.file_name().unwrap(), "Test__Profile-profile.zip");
        std::fs::write(&first, b"original").unwrap();
        let second = portable_profile_export_path_beside(&exe, "Test: Profile").unwrap();
        assert_eq!(second.file_name().unwrap(), "Test__Profile-profile (2).zip");
        assert_eq!(std::fs::read(&first).unwrap(), b"original");
        let reserved = portable_profile_export_path_beside(&exe, "CON").unwrap();
        assert_eq!(reserved.file_name().unwrap(), "_CON-profile.zip");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn profile_export_creates_exe_adjacent_folder_and_never_overwrites() {
        let base = std::env::temp_dir().join(format!(
            "mintfixed-profile-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        let exe = base.join("mintfixed.exe");
        let first = save_profile_export_beside(&exe, "Test: Profile", "json", "first").unwrap();
        assert_eq!(first.parent().unwrap(), base.join("mintfixed exports"));
        assert_eq!(first.file_name().unwrap(), "Test_ Profile.json");
        let second = save_profile_export_beside(&exe, "Test: Profile", "json", "second").unwrap();
        assert_eq!(second.file_name().unwrap(), "Test_ Profile (2).json");
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "second");
        let txt = save_profile_export_beside(&exe, "CON", "txt", "url\n").unwrap();
        assert_eq!(txt.file_name().unwrap(), "_CON.txt");
        std::fs::remove_dir_all(base).unwrap();
    }
}
