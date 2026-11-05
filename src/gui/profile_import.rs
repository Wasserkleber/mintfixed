//! Import full profiles from "Copy profile with groups (JSON)".
//! Keep profile restoration independent from the Groups manager.

use crate::state::{
    MemberOverrides, ModData_v0_1_0 as Data, ModGroup, ModOrGroup, ModProfile_v0_1_0 as Profile,
};
use std::collections::{BTreeMap, HashSet};

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

#[derive(Clone, Default)]
struct ImportEditor {
    json: String,
    name: String,
    error: Option<String>,
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
    let used_profiles: HashSet<_> = data.profiles.keys().cloned().collect();
    let profile_name = unused_name(base_name, &used_profiles);

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

    let mut profile = Profile::default();
    profile.id = crate::state::new_id();
    profile.mods = entries;
    data.groups.extend(new_groups);
    data.profiles.insert(profile_name.clone(), profile);
    data.active_profile = profile_name.clone();
    Ok(profile_name)
}

/// Render the importer directly inside Profile Management (no nested popup).
/// Returns a status message when a new profile is imported successfully.
pub(super) fn ui_inline(ui: &mut egui::Ui, data: &mut Data, modified: &mut bool) -> Option<String> {
    let state_id = ui.make_persistent_id("profile-import-json-fields");
    let mut editor: ImportEditor = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    let mut success = None;

    ui.label("Paste a JSON profile export, or select a .json file.");
    if ui.button("Load JSON file...").clicked() {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("mintfixed profile JSON", &["json"])
            .pick_file()
        {
            let contents = std::fs::metadata(&path)
                .map_err(|e| format!("Cannot read JSON file: {e}"))
                .and_then(|meta| {
                    if meta.len() > MAX_PROFILE_JSON_BYTES as u64 {
                        Err("Profile JSON must not exceed 8 MiB".into())
                    } else {
                        std::fs::read_to_string(&path)
                            .map_err(|e| format!("Cannot read JSON file: {e}"))
                    }
                });
            match contents.and_then(|json| {
                let parsed = parse_profile(&json)?;
                Ok((json, parsed.profile_name))
            }) {
                Ok((json, name)) => {
                    editor.json = json;
                    editor.name = name;
                    editor.error = None;
                }
                Err(error) => editor.error = Some(error),
            }
        }
    }
    // Large JSON must scroll inside its own bounded editor, never grow the menu.
    let edited = egui::ScrollArea::vertical()
        .id_salt("profile-import-json-scroll")
        .max_height(165.0)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut editor.json)
                    .desired_width(380.0)
                    .desired_rows(6)
                    .hint_text("Paste profile JSON here (Ctrl+V)"),
            )
        })
        .inner;
    if edited.changed() {
        editor.error = None;
        if editor.name.trim().is_empty() {
            if let Ok(parsed) = parse_profile(&editor.json) {
                editor.name = parsed.profile_name;
            }
        }
    }
    ui.label("Imported profile name:");
    ui.add(
        egui::TextEdit::singleline(&mut editor.name).hint_text("Uses exported name if left blank"),
    );
    ui.label("Existing profiles and groups will not be overwritten.");
    if let Some(error) = &editor.error {
        ui.colored_label(ui.visuals().warn_fg_color, error);
    }
    ui.horizontal(|ui| {
        if ui.button("Cancel").clicked() {
            editor = ImportEditor::default();
            ui.memory_mut(|mem| mem.close_popup());
        }
        if ui
            .add_enabled(
                !editor.json.trim().is_empty(),
                egui::Button::new("Import Profile"),
            )
            .clicked()
        {
            match import_profile(data, &editor.json, &editor.name) {
                Ok(name) => {
                    *modified = true;
                    success = Some(format!("Imported profile \"{name}\" with its groups."));
                    editor = ImportEditor::default();
                    ui.memory_mut(|mem| mem.close_popup());
                }
                Err(error) => editor.error = Some(error),
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(state_id, editor));
    success
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ModSpecification;
    use crate::state::ModConfig;

    fn mod_config(url: &str) -> ModConfig {
        ModConfig {
            id: crate::state::new_id(),
            description: Some("Original mod description".into()),
            spec: ModSpecification::new(url.into()),
            enabled: true,
            required: true,
            member_id: Some(crate::state::new_id()),
            priority,
        }
    }

    fn sample() -> String {
        let mut member = mod_config("https://mod.io/g/drg/m/example", 12);
        let mut overrides = MemberOverrides::default();
        overrides.set(&mut member, false);
        let group = ModGroup {
            id: crate::state::new_id(),
            mods: vec![member],
            description: Some("Group description".into()),
        };
        let mut standalone = mod_config("C:\\mods\\custom.pak");
        standalone.priority = 7;
        let export = ProfileExport {
            format: PROFILE_FORMAT.into(),
            version: PROFILE_VERSION,
            profile_name: "Example".into(),
            mods: vec![
                ModOrGroup::Group {
                    id: crate::state::new_id(),
                    group_name: "Tools".into(),
                    enabled: true,
                    member_overrides: overrides,
                },
                ModOrGroup::Individual(standalone),
            ],
            groups: BTreeMap::from([("Tools".into(), group)]),
        };
        serde_json::to_string(&export).unwrap()
    }

    #[test]
    fn preserves_order_settings_and_overrides_with_new_ids() {
        let input = sample();
        let old = parse_profile(&input).unwrap();
        let mut data = Data::default();
        let created = import_profile(&mut data, &input, "").unwrap();
        let profile = &data.profiles[&created];
        assert_eq!(data.active_profile, created);
        assert_eq!(profile.mods.len(), 2);
        let group = &data.groups["Tools"];
        assert_eq!(group.description.as_deref(), Some("Group description"));
        assert_ne!(group.id, old.groups["Tools"].id);
        assert_ne!(group.mods[0].id, old.groups["Tools"].mods[0].id);
        assert_eq!(group.mods[0].priority, 12);
        assert!(group.mods[0].required);
        assert_eq!(
            group.mods[0].description.as_deref(),
            Some("Original mod description")
        );
        match &profile.mods[0] {
            ModOrGroup::Group {
                group_name,
                member_overrides,
                ..
            } => {
                assert_eq!(group_name, "Tools");
                assert!(!member_overrides.enabled(&group.mods[0]));
            }
            _ => panic!("first item should be a group"),
        }
        match &profile.mods[1] {
            ModOrGroup::Individual(mc) => {
                assert_eq!(mc.priority, 7);
                assert_ne!(
                    mc.id,
                    match &old.mods[1] {
                        ModOrGroup::Individual(before) => before.id.clone(),
                        _ => unreachable!(),
                    }
                );
            }
            _ => panic!("second item should be a standalone mod"),
        }
    }

    #[test]
    fn repeat_import_renames_profile_and_group_without_overwriting() {
        let mut data = Data::default();
        let input = sample();
        import_profile(&mut data, &input, "Example").unwrap();
        import_profile(&mut data, &input, "Example").unwrap();
        assert!(data.profiles.contains_key("Example"));
        assert!(data.profiles.contains_key("Example (imported)"));
        assert!(data.groups.contains_key("Tools"));
        assert!(data.groups.contains_key("Tools (imported)"));
        assert!(matches!(&data.profiles["Example (imported)"].mods[0],
            ModOrGroup::Group { group_name, .. } if group_name == "Tools (imported)"));
    }

    #[test]
    fn rejects_malformed_json_without_mutating_state() {
        let mut data = Data::default();
        let before_profiles = data.profiles.len();
        let before_groups = data.groups.len();
        assert!(
            import_profile(
                &mut data,
                r#"{"format":"wrong","version":1,"profile_name":"X","mods":[],"groups":{}}"#,
                ""
            )
            .is_err()
        );
        assert!(import_profile(&mut data, r#"{"format":"mintfixed-profile-mod-list","version":1,"profile_name":"X","mods":[{"group_name":"missing","enabled":true}],"groups":{}}"#, "").is_err());
        assert_eq!(data.profiles.len(), before_profiles);
        assert_eq!(data.groups.len(), before_groups);
    }
}
