//! Import full profiles from "Copy profile with groups (JSON)".
//! Keep profile restoration independent from the Groups manager.

use super::profile_package;
use crate::state::{
    MemberOverrides, ModData_v0_1_0 as Data, ModGroup, ModOrGroup, ModProfile_v0_1_0 as Profile,
};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const PROFILE_FORMAT: &str = "mintfixed-profile-mod-list";
const PROFILE_VERSION: u32 = 1;
const MAX_PROFILE_JSON_BYTES: usize = 8 * 1024 * 1024;

#[derive(serde::Deserialize, serde::Serialize)]
struct ProfileExport {
    format: String,
    version: u32,
    profile_name: String,
    mods: Vec<ModOrGroup>,
    groups: BTreeMap<String, ModGroup>,
}

#[derive(Clone)]
struct ImportJob {
    state: Arc<Mutex<Option<Result<profile_package::ExtractedBundle, String>>>>,
    requested_name: String,
}

#[derive(Clone, Default)]
struct ImportEditor {
    json: String,
    name: String,
    error: Option<String>,
    package_path: Option<PathBuf>,
    // The source file is tracked for both JSON and ZIP; no raw JSON editor.
    source_path: Option<PathBuf>,
    job: Option<ImportJob>,
}

fn parse_profile(text: &str) -> Result<ProfileExport, String> {
    if text.len() > MAX_PROFILE_JSON_BYTES {
        return Err("Profile JSON must not exceed 8 MiB".into());
    }
    let export: ProfileExport =
        serde_json::from_str(text).map_err(|e| format!("Invalid profile JSON: {e}"))?;
    if export.format != PROFILE_FORMAT || export.version != PROFILE_VERSION {
        return Err("Unsupported profile JSON format/version".into());
    }
    if export.profile_name.trim().is_empty() {
        return Err("The exported profile has no name".into());
    }
    for (name, group) in &export.groups {
        if name.trim().is_empty() {
            return Err("An exported group has no name".into());
        }
        if group
            .mods
            .iter()
            .any(|member| member.spec.url.trim().is_empty())
        {
            return Err(format!("Group \"{name}\" contains an empty mod URL"));
        }
    }
    for entry in &export.mods {
        match entry {
            ModOrGroup::Individual(mc) if mc.spec.url.trim().is_empty() => {
                return Err("Profile contains an empty mod URL".into());
            }
            ModOrGroup::Group { group_name, .. } if !export.groups.contains_key(group_name) => {
                return Err(format!("Profile refers to missing group \"{group_name}\""));
            }
            _ => {}
        }
    }
    Ok(export)
}

fn unused_name(base: &str, reserved: &HashSet<String>) -> String {
    if !reserved.contains(base) {
        return base.to_owned();
    }
    let candidate = format!("{base} (imported)");
    if !reserved.contains(&candidate) {
        return candidate;
    }
    for suffix in 2.. {
        let candidate = format!("{base} (imported {suffix})");
        if !reserved.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// Prepare all data before modifying the saved state. Existing profiles and
/// shared groups are never overwritten. New IDs avoid cross-profile collisions.
fn import_profile(data: &mut Data, text: &str, requested_name: &str) -> Result<String, String> {
    let export = parse_profile(text)?;
    let requested_name = requested_name.trim();
    let base_name = if requested_name.is_empty() {
        export.profile_name.trim()
    } else {
        requested_name
    };
    // Match Group Management: require the user to resolve a profile-name conflict.
    // Existing profiles must never be overwritten or silently renamed.
    if data.profiles.contains_key(base_name) {
        return Err("A profile with that name already exists.".into());
    }
    let profile_name = base_name.to_owned();

    let mut used_groups: HashSet<String> = data.groups.keys().cloned().collect();
    let mut group_names = BTreeMap::new();
    let mut new_groups = BTreeMap::new();
    for (old_name, original) in &export.groups {
        let new_name = unused_name(old_name, &used_groups);
        used_groups.insert(new_name.clone());
        group_names.insert(old_name.clone(), new_name.clone());

        let mut copy = original.clone();
        copy.id = crate::state::new_id();
        copy.mods = original
            .mods
            .iter()
            .map(|mc| {
                let mut copy = mc.duplicate();
                copy.id = crate::state::new_id();
                copy.member_id = mc.member_id.as_ref().map(|_| crate::state::new_id());
                copy
            })
            .collect();
        new_groups.insert(new_name, copy);
    }

    let mut entries = Vec::with_capacity(export.mods.len());
    for entry in &export.mods {
        match entry {
            ModOrGroup::Individual(original) => {
                let mut copied = original.duplicate();
                copied.id = crate::state::new_id();
                copied.member_id = None;
                entries.push(ModOrGroup::Individual(copied));
            }
            ModOrGroup::Group {
                group_name,
                enabled,
                member_overrides,
                ..
            } => {
                let dest = &group_names[group_name]; // validated by parse_profile
                let old_group = &export.groups[group_name];
                let new_group = new_groups.get_mut(dest).expect("prepared group");
                let mut overrides = MemberOverrides::default();
                for (old, new) in old_group.mods.iter().zip(new_group.mods.iter_mut()) {
                    overrides.set(new, member_overrides.enabled(old));
                }
                entries.push(ModOrGroup::Group {
                    id: crate::state::new_id(),
                    group_name: dest.clone(),
                    enabled: *enabled,
                    member_overrides: overrides,
                });
            }
        }
    }

    let profile = Profile {
        id: crate::state::new_id(),
        mods: entries,
    };
    data.groups.extend(new_groups);
    data.profiles.insert(profile_name.clone(), profile);
    data.active_profile = profile_name.clone();
    Ok(profile_name)
}

/// Render the importer directly inside Profile Management (no nested popup).
/// Returns a status message when a new profile is imported successfully.
pub(in crate::gui) fn ui_inline(
    ui: &mut egui::Ui,
    data: &mut Data,
    modified: &mut bool,
    configured_folder: Option<&Path>,
) -> Option<String> {
    let state_id = ui.make_persistent_id("profile-import-json-fields");
    let mut editor: ImportEditor = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let mut success = None;

    // Import state is written only after the worker verified and extracted
    // all bundled files. An abandoned job cleans up uncommitted extractions.
    if let Some(job) = editor.job.clone() {
        let finished = job.state.lock().ok().and_then(|mut slot| slot.take());
        if let Some(finished) = finished {
            editor.job = None;
            match finished {
                Ok(mut result) => match import_profile(data, &result.json, &job.requested_name) {
                    Ok(name) => {
                        result.commit();
                        *modified = true;
                        let missing =
                            crate::gui::missing_local_files::profile_missing_paths(data, &name);
                        crate::gui::missing_local_files::notify(
                            ui.ctx(),
                            crate::gui::missing_local_files::Origin::Profile(name.clone()),
                            missing,
                        );
                        success = Some(format!(
                            "Imported profile \"{name}\" with its groups and bundled files."
                        ));
                        editor = ImportEditor::default();
                        ui.memory_mut(|mem| mem.close_popup());
                    }
                    Err(error) => editor.error = Some(error),
                },
                Err(error) => editor.error = Some(error),
            }
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
    // profile-import-file-first-like-groups
    ui.label("Import a profile JSON or a portable ZIP with local mod files.");
    ui.label("Imported profiles include their groups; existing profiles are not overwritten.");
    if ui
        .add_enabled(
            editor.job.is_none(),
            egui::Button::new("Choose file to import..."),
        )
        .clicked()
        && let Some(path) = rfd::FileDialog::new()
            .add_filter("Profile JSON or portable ZIP", &["json", "zip"])
            .pick_file()
    {
        let is_zip = path
            .extension()
            .and_then(|v| v.to_str())
            .is_some_and(|v| v.eq_ignore_ascii_case("zip"));
        let result = if is_zip {
            profile_package::inspect_bundle(&path)
                .map(|(name, text, _)| (name, text, Some(path.clone())))
        } else {
            std::fs::metadata(&path)
                .map_err(|e| format!("Cannot read JSON file: {e}"))
                .and_then(|meta| {
                    if meta.len() > MAX_PROFILE_JSON_BYTES as u64 {
                        Err("Profile JSON must not exceed 8 MiB".into())
                    } else {
                        std::fs::read_to_string(&path)
                            .map_err(|e| format!("Cannot read JSON file: {e}"))
                    }
                })
                .and_then(|json| {
                    let parsed = parse_profile(&json)?;
                    Ok((parsed.profile_name, json, None))
                })
        };
        match result {
            Ok((name, json, package_path)) => {
                editor.json = json;
                editor.name = name;
                editor.package_path = package_path;
                editor.source_path = Some(path.clone());
                editor.error = None;
            }
            Err(error) => {
                editor.json.clear();
                editor.name.clear();
                editor.package_path = None;
                editor.source_path = None;
                editor.error = Some(error);
            }
        }
    }
    if editor.source_path.is_some()
        && let Ok(parsed) = parse_profile(&editor.json)
    {
        ui.label(format!("Profile in file: {}", parsed.profile_name));
    }
    if editor.job.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Verifying and extracting bundled mods...");
        });
    }
    if let Some(error) = &editor.error {
        ui.colored_label(ui.visuals().error_fg_color, format!("❌ {error}"));
    }
    if editor.source_path.is_some() {
        ui.add_space(6.0);
        ui.label("Imported profile name:");
        ui.add(egui::TextEdit::singleline(&mut editor.name).hint_text("New profile name"));
        // Use the effective name, including the exported name when the field is blank.
        // This is the same inline conflict warning used by Group Management.
        let typed_name = editor.name.trim();
        let exported_name = if typed_name.is_empty() {
            parse_profile(&editor.json)
                .ok()
                .map(|profile| profile.profile_name)
        } else {
            None
        };
        let import_name = if typed_name.is_empty() {
            exported_name.as_deref().unwrap_or("").trim()
        } else {
            typed_name
        };
        let name_conflict = !import_name.is_empty() && data.profiles.contains_key(import_name);
        if name_conflict {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "A profile with that name already exists.",
            );
        }

        ui.horizontal(|ui| {
            if ui
                .add_enabled(editor.job.is_none(), egui::Button::new("Cancel"))
                .clicked()
            {
                editor = ImportEditor::default();
            }
            if ui
                .add_enabled(
                    editor.source_path.is_some()
                        && editor.job.is_none()
                        && !editor.name.trim().is_empty()
                        && !name_conflict,
                    egui::Button::new("Import Profile"),
                )
                .clicked()
            {
                match profile_package::require_mod_folder(
                    &editor.json,
                    editor.package_path.is_some(),
                    configured_folder,
                ) {
                    Err(error) => editor.error = Some(error),
                    Ok(()) => {
                        if let Some(path) = editor.package_path.clone() {
                            let state = Arc::new(Mutex::new(None));
                            let worker_state = Arc::clone(&state);
                            let folder = configured_folder.map(Path::to_path_buf);
                            let ctx = ui.ctx().clone();
                            std::thread::spawn(move || {
                                let outcome =
                                    profile_package::extract_bundle(&path, folder.as_deref());
                                if let Ok(mut slot) = worker_state.lock() {
                                    *slot = Some(outcome);
                                }
                                ctx.request_repaint();
                            });
                            editor.job = Some(ImportJob {
                                state,
                                requested_name: editor.name.clone(),
                            });
                            editor.error = None;
                        } else {
                            match import_profile(data, &editor.json, &editor.name) {
                                Ok(name) => {
                                    *modified = true;
                                    let missing =
                                        crate::gui::missing_local_files::profile_missing_paths(
                                            data, &name,
                                        );
                                    crate::gui::missing_local_files::notify(
                                        ui.ctx(),
                                        crate::gui::missing_local_files::Origin::Profile(
                                            name.clone(),
                                        ),
                                        missing,
                                    );
                                    success = Some(format!(
                                        "Imported profile \"{name}\" with its groups."
                                    ));
                                    editor = ImportEditor::default();
                                    ui.memory_mut(|mem| mem.close_popup());
                                }
                                Err(error) => editor.error = Some(error),
                            }
                        }
                    }
                }
            }
        });
    }
    ui.data_mut(|d| d.insert_temp(state_id, editor));
    success
}

#[cfg(test)]
mod tests;
