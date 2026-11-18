//! Missing local file diagnostics for imported profiles and groups.
//! Scan on import and before an installation, not every idle GUI frame.
use std::collections::BTreeSet;
use std::path::Path;

use crate::state::{ModData_v0_1_0 as Data, ModGroup, ModOrGroup};

#[derive(Clone, Debug)]
pub(super) enum Origin {
    Profile(String),
    Group(String),
}

#[derive(Clone, Debug)]
struct Notice {
    origin: Origin,
    paths: Vec<String>,
    error: Option<String>,
}

const NOTICE_ID: &str = "mintfixed-import-missing-local-files";

// These are deliberately recognizable filesystem paths, not arbitrary invalid
// provider strings. In particular, HTTP downloads ending in .pak are NOT local.
fn looks_like_local_path(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.contains("://") {
        return false;
    }
    let bytes = value.as_bytes();
    let windows_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let rooted = value.starts_with("\\\\")
        || value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with(".\\")
        || value.starts_with("..\\");
    let extension = Path::new(value).extension().and_then(|ext| ext.to_str());
    let mod_file = extension
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip"));
    windows_absolute || rooted || mod_file
}

pub(super) fn is_missing_local_path(value: &str) -> bool {
    looks_like_local_path(value) && !Path::new(value).exists()
}

pub(super) fn group_missing_paths(group: &ModGroup) -> Vec<String> {
    group
        .mods
        .iter()
        .map(|member| member.spec.url.as_str())
        .filter(|path| is_missing_local_path(path))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn profile_missing_paths(data: &Data, profile_name: &str) -> Vec<String> {
    let Some(profile) = data.profiles.get(profile_name) else {
        return Vec::new();
    };
    let mut missing = BTreeSet::new();
    for entry in &profile.mods {
        match entry {
            ModOrGroup::Individual(member) => {
                if is_missing_local_path(&member.spec.url) {
                    missing.insert(member.spec.url.clone());
                }
            }
            ModOrGroup::Group { group_name, .. } => {
                if let Some(group) = data.groups.get(group_name) {
                    missing.extend(group_missing_paths(group));
                }
            }
        }
    }
    missing.into_iter().collect()
}

pub(super) fn notify(ctx: &egui::Context, origin: Origin, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new(NOTICE_ID),
            Notice {
                origin,
                paths,
                error: None,
            },
        );
    });
    ctx.request_repaint();
}

/// Change only entries in the just-imported profile or group. All other
/// profiles, groups, IDs, priorities, and enabled/member overrides are intact.
fn relocate(
    data: &mut Data,
    origin: &Origin,
    old_path: &str,
    new_path: &Path,
) -> Result<usize, String> {
    let is_mod_file = Path::new(old_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip"));
    if is_mod_file && !new_path.is_file() {
        return Err("Choose an existing .pak or .zip file.".into());
    }
    if !is_mod_file && !new_path.exists() {
        return Err("Choose an existing local folder or file.".into());
    }
    let updated = new_path.to_string_lossy().to_string();
    if is_mod_file
        && !Path::new(&updated)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip"))
    {
        return Err("The replacement must be a .pak or .zip file.".into());
    }
    fn update_members(mods: &mut [crate::state::ModConfig], old: &str, new: &str) -> usize {
        let mut changed = 0;
        for member in mods {
            if member.spec.url == old {
                member.spec.url = new.to_owned();
                changed += 1;
            }
        }
        changed
    }
    let mut count = 0;
    match origin {
        Origin::Profile(name) => {
            let profile = data
                .profiles
                .get_mut(name)
                .ok_or("The imported profile was removed or renamed")?;
            let names: BTreeSet<_> = profile
                .mods
                .iter()
                .filter_map(|entry| match entry {
                    ModOrGroup::Group { group_name, .. } => Some(group_name.clone()),
                    _ => None,
                })
                .collect();
            for entry in &mut profile.mods {
                if let ModOrGroup::Individual(member) = entry
                    && member.spec.url == old_path
                {
                    member.spec.url.clone_from(&updated);
                    count += 1;
                }
            }
            for name in names {
                if let Some(group) = data.groups.get_mut(&name) {
                    count += update_members(&mut group.mods, old_path, &updated);
                }
            }
        }
        Origin::Group(name) => {
            let group = data
                .groups
                .get_mut(name)
                .ok_or("The imported group was removed or renamed")?;
            count += update_members(&mut group.mods, old_path, &updated);
        }
    }
    if count == 0 {
        Err("No matching imported entry remains. It may have been edited or removed.".into())
    } else {
        Ok(count)
    }
}

/// Returns true only when a saved URL was changed and the caller must save it.
pub(super) fn show(ctx: &egui::Context, data: &mut Data) -> bool {
    let key = egui::Id::new(NOTICE_ID);
    let Some(mut notice) = ctx.data(|memory| memory.get_temp::<Notice>(key)) else {
        return false;
    };
    let mut open = true;
    let mut dismiss = false;
    let mut changed = false;
    let title = match &notice.origin {
        Origin::Profile(_) => "Imported profile: missing local files",
        Origin::Group(_) => "Imported group: missing local files",
    };
    egui::Window::new(title)
        .id(key.with("window"))
        .collapsible(false)
        .resizable(true)
        .default_width(490.0)
        .open(&mut open)
        .show(ctx, |ui| {
            let (kind, name) = match &notice.origin {
                Origin::Profile(name) => ("profile", name.as_str()),
                Origin::Group(name) => ("group", name.as_str()),
            };
            let number = notice.paths.len();
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "Imported {kind} \"{name}\" with {number} missing local {}.",
                    if number == 1 { "file" } else { "files" }
                ),
            );
            ui.label("The files could not be found at their saved locations. Their entries and settings have been preserved.");
            ui.small("Choose Relocate to point an entry to an existing local file. Nothing is deleted automatically.");
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .show(ui, |ui| {
                    // Clone because a successful relocation removes a row in the same frame.
                    for old in notice.paths.clone() {
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(egui::RichText::new(&old).monospace()).wrap());
                            if ui.button("Relocate...").clicked() {
                                let extension = Path::new(&old)
                                    .extension()
                                    .and_then(|ext| ext.to_str());
                                let file_expected = extension.is_some_and(|ext| {
                                    ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip")
                                });
                                let replacement = if file_expected {
                                    rfd::FileDialog::new()
                                        .add_filter("Local mods", &["pak", "zip"])
                                        .pick_file()
                                } else {
                                    rfd::FileDialog::new().pick_folder()
                                };
                                if let Some(replacement) = replacement {
                                    match relocate(data, &notice.origin, &old, &replacement) {
                                        Ok(_) => {
                                            notice.paths.retain(|path| path != &old);
                                            notice.error = None;
                                            changed = true;
                                        }
                                        Err(error) => notice.error = Some(error),
                                    }
                                }
                            }
                        });
                        ui.separator();
                    }
                });
            if let Some(error) = &notice.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|ui| {
                if ui.button("Copy missing paths").clicked() {
                    ui.ctx().copy_text(notice.paths.join("\n"));
                }
                if ui.button("Dismiss").clicked() {
                    dismiss = true;
                }
            });
        });
    if !open || dismiss || notice.paths.is_empty() {
        ctx.data_mut(|memory| memory.remove::<Notice>(key));
    } else {
        ctx.data_mut(|memory| memory.insert_temp(key, notice));
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ModSpecification;
    use crate::state::ModConfig;

    fn member(path: &str) -> ModConfig {
        ModConfig {
            id: crate::state::new_id(),
            spec: ModSpecification::new(path.into()),
            description: Some("Preserve this".into()),
            enabled: false,
            required: true,
            member_id: None,
            priority: 7,
        }
    }

    #[test]
    fn discriminates_urls_from_local_paths() {
        assert!(!looks_like_local_path(
            "https://example.com/download/mod.pak"
        ));
        assert!(!looks_like_local_path("https://mod.io/g/drg/m/example"));
        assert!(!looks_like_local_path("mod-id"));
        assert!(looks_like_local_path(r"C:\Mods\missing.pak"));
        assert!(looks_like_local_path(r"\\server\share\Mods"));
        assert!(looks_like_local_path("./mods/missing.zip"));
        assert!(looks_like_local_path("/home/me/mods/missing.pak"));
    }

    #[test]
    fn detects_missing_group_and_standalone_paths_but_keeps_settings() {
        let missing = format!("nonexistent-{}.pak", crate::state::new_id());
        let group_missing = format!("nonexistent-{}.zip", crate::state::new_id());
        let mut data = Data::default();
        data.profiles
            .get_mut("default")
            .unwrap()
            .mods
            .push(ModOrGroup::Individual(member(&missing)));
        data.groups.insert(
            "Imported tools".into(),
            ModGroup {
                id: crate::state::new_id(),
                tint_color: None,
                description: None,
                mods: vec![
                    member(&group_missing),
                    member(&group_missing),
                    member("https://mod.io/g/drg/m/valid"),
                ],
            },
        );
        data.profiles
            .get_mut("default")
            .unwrap()
            .mods
            .push(ModOrGroup::Group {
                id: crate::state::new_id(),
                group_name: "Imported tools".into(),
                enabled: false,
                member_overrides: Default::default(),
            });
        let missing_paths = profile_missing_paths(&data, "default");
        assert_eq!(missing_paths.len(), 2);
        assert!(missing_paths.contains(&missing));
        assert!(missing_paths.contains(&group_missing));
        assert!(!data.groups["Imported tools"].mods[0].enabled);
        assert_eq!(data.groups["Imported tools"].mods[0].priority, 7);
    }

    #[test]
    fn relocating_imported_group_preserves_ids_settings_and_other_groups() {
        let missing = format!("nonexistent-{}.pak", crate::state::new_id());
        let replacement =
            std::env::temp_dir().join(format!("mintfixed-{}.pak", crate::state::new_id()));
        std::fs::write(&replacement, b"test file").unwrap();
        let mut data = Data::default();
        data.groups.insert(
            "Imported".into(),
            ModGroup {
                id: crate::state::new_id(),
                tint_color: None,
                description: None,
                mods: vec![member(&missing)],
            },
        );
        data.groups.insert(
            "Other".into(),
            ModGroup {
                id: crate::state::new_id(),
                tint_color: None,
                description: None,
                mods: vec![member(&missing)],
            },
        );
        let original_id = data.groups["Imported"].mods[0].id.clone();
        let updated = relocate(
            &mut data,
            &Origin::Group("Imported".into()),
            &missing,
            &replacement,
        )
        .unwrap();
        assert_eq!(updated, 1);
        let relocated = &data.groups["Imported"].mods[0];
        assert_eq!(relocated.spec.url, replacement.to_string_lossy());
        assert_eq!(relocated.id, original_id);
        assert_eq!(relocated.priority, 7);
        assert!(!relocated.enabled);
        assert!(relocated.required);
        assert_eq!(data.groups["Other"].mods[0].spec.url, missing);
        std::fs::remove_file(&replacement).unwrap();
    }

    #[test]
    fn relocating_profile_updates_only_its_members_and_referenced_groups() {
        let old = format!("nonexistent-{}.pak", crate::state::new_id());
        let replacement =
            std::env::temp_dir().join(format!("mintfixed-{}.pak", crate::state::new_id()));
        std::fs::write(&replacement, b"test file").unwrap();
        let mut data = Data::default();
        data.profiles
            .get_mut("default")
            .unwrap()
            .mods
            .push(ModOrGroup::Individual(member(&old)));
        data.groups.insert(
            "Used by import".into(),
            ModGroup {
                id: crate::state::new_id(),
                tint_color: None,
                description: None,
                mods: vec![member(&old)],
            },
        );
        data.groups.insert(
            "Unrelated".into(),
            ModGroup {
                id: crate::state::new_id(),
                tint_color: None,
                description: None,
                mods: vec![member(&old)],
            },
        );
        data.profiles
            .get_mut("default")
            .unwrap()
            .mods
            .push(ModOrGroup::Group {
                id: crate::state::new_id(),
                group_name: "Used by import".into(),
                enabled: true,
                member_overrides: Default::default(),
            });
        let changed = relocate(
            &mut data,
            &Origin::Profile("default".into()),
            &old,
            &replacement,
        )
        .unwrap();
        assert_eq!(changed, 2);
        assert_eq!(
            data.groups["Used by import"].mods[0].spec.url,
            replacement.to_string_lossy()
        );
        assert_eq!(data.groups["Unrelated"].mods[0].spec.url, old);
        let standalone = match &data.profiles["default"].mods[0] {
            ModOrGroup::Individual(member) => member,
            _ => panic!("expected an individual mod"),
        };
        assert_eq!(standalone.spec.url, replacement.to_string_lossy());
        assert_eq!(standalone.priority, 7);
        assert!(!standalone.enabled);
        std::fs::remove_file(&replacement).unwrap();
    }

    #[test]
    fn existing_local_files_are_not_reported() {
        let path = std::env::temp_dir().join(format!("mintfixed-{}.pak", crate::state::new_id()));
        std::fs::write(&path, b"test file").unwrap();
        assert!(!is_missing_local_path(&path.to_string_lossy()));
        std::fs::remove_file(&path).unwrap();
        assert!(is_missing_local_path(&path.to_string_lossy()));
    }
}
