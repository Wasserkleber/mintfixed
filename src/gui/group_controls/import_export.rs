use super::management::PendingGroupImport;
use super::*;

const GROUP_EXPORT_FORMAT: &str = "mintfixed-groups";
const GROUP_EXPORT_VERSION: u32 = 1;

/// JSON retains the member configs (including mod URLs, enabled switches and priorities).
/// Plain .txt exports deliberately contain only the URLs, like MINT's mod export.
#[derive(serde::Serialize, serde::Deserialize)]
struct GroupExportFile {
    format: String,
    version: u32,
    groups: std::collections::BTreeMap<String, crate::state::ModGroup>,
}

pub(super) fn serialize_single_group(data: &Data, name: &str) -> Result<String, String> {
    let group = data
        .groups
        .get(name)
        .ok_or("The selected group no longer exists")?;
    serde_json::to_string_pretty(&GroupExportFile {
        format: GROUP_EXPORT_FORMAT.to_owned(),
        version: GROUP_EXPORT_VERSION,
        groups: std::collections::BTreeMap::from([(name.to_owned(), group.clone())]),
    })
    .map_err(|e| format!("Failed to serialize group: {e}"))
}

pub(super) fn parse_groups_json(
    text: &str,
) -> Result<std::collections::BTreeMap<String, crate::state::ModGroup>, String> {
    let export: GroupExportFile =
        serde_json::from_str(text).map_err(|e| format!("Invalid group export JSON: {e}"))?;
    if export.format != GROUP_EXPORT_FORMAT || export.version != GROUP_EXPORT_VERSION {
        return Err("Unsupported group export format/version".into());
    }
    if export.groups.is_empty() {
        return Err("This export contains no groups".into());
    }
    for (name, group) in &export.groups {
        if name.trim().is_empty() {
            return Err("An imported group has an empty name".into());
        }
        if group.mods.iter().any(|m| m.spec.url.trim().is_empty()) {
            return Err(format!("Group \"{name}\" contains an empty mod URL"));
        }
    }
    Ok(export.groups)
}

pub(super) fn parse_mod_list_txt(
    text: &str,
    group_name: &str,
) -> Result<std::collections::BTreeMap<String, crate::state::ModGroup>, String> {
    let group_name = group_name.trim();
    if group_name.is_empty() {
        return Err("Cannot determine a group name from the filename".into());
    }
    let mut seen = HashSet::new();
    let mods = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| seen.insert((*line).to_owned()))
        .map(|url| ModConfig {
            id: crate::state::new_id(),
            description: None,
            spec: ModSpecification::new(url.to_owned()),
            enabled: true,
            required: false,
            member_id: None,
            priority: 0,
        })
        .collect::<Vec<_>>();
    if mods.is_empty() {
        return Err("The mod list is empty".into());
    }
    Ok(std::collections::BTreeMap::from([(
        group_name.to_owned(),
        crate::state::ModGroup {
            id: crate::state::new_id(),
            mods,
            description: None,
        },
    )]))
}

/// Read exactly one group and defer insertion until the user confirms its name.
pub(super) fn parse_single_group_json(text: &str) -> Result<PendingGroupImport, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("Invalid JSON: {e}"))?;

    let (name, group) = if value.get("groups").is_some() {
        // Existing mintfixed bundle format. Do not silently import one group
        // from a file containing several groups.
        let groups = parse_groups_json(text)?;
        if groups.len() != 1 {
            return Err("Select a JSON file containing exactly one group".into());
        }
        let (name, group) = groups.into_iter().next().unwrap();
        (Some(name), group)
    } else {
        if let Some(format) = value.get("format") {
            if format.as_str() != Some("mintfixed-group")
                || value.get("version").and_then(|v| v.as_u64()) != Some(1)
            {
                return Err("Unsupported group export format/version".into());
            }
        }
        // Also accept one bare ModGroup or { "name": "...", "group": {...} }.
        let content = value.get("group").unwrap_or(&value);
        if content.get("mods").is_none() {
            return Err("This JSON file does not contain a group".into());
        }
        let group = serde_json::from_value(content.clone())
            .map_err(|e| format!("Invalid group data: {e}"))?;
        let name = value
            .get("name")
            .or_else(|| content.get("name"))
            .and_then(|name| name.as_str())
            .map(str::to_owned);
        (name, group)
    };

    if group
        .mods
        .iter()
        .any(|m: &ModConfig| m.spec.url.trim().is_empty())
    {
        return Err("The group contains an empty mod URL".into());
    }
    Ok(PendingGroupImport {
        suggested_name: name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty()),
        group,
    })
}

pub(super) fn read_single_group_import(
    path: &std::path::Path,
) -> Result<PendingGroupImport, String> {
    const MAX_BYTES: u64 = 8 * 1024 * 1024;
    let size = std::fs::metadata(path)
        .map_err(|e| format!("Cannot read file: {e}"))?
        .len();
    if size > MAX_BYTES {
        return Err("Group import files must not exceed 8 MiB".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read file: {e}"))?;
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("json") {
        parse_single_group_json(&text)
    } else if extension.eq_ignore_ascii_case("txt") {
        // TXT is a list of URLs/paths; the filename is not group metadata.
        let mut parsed = parse_mod_list_txt(&text, "Temporary import")?;
        Ok(PendingGroupImport {
            suggested_name: None,
            group: parsed.remove("Temporary import").unwrap(),
        })
    } else {
        Err("Select a .json group export or a .txt mod list".into())
    }
}

/// Import never overwrites existing groups. All imported names are unique and
/// all imported groups are attached to the active profile.
pub(super) fn import_groups(
    data: &mut Data,
    incoming: std::collections::BTreeMap<String, crate::state::ModGroup>,
) -> Result<Vec<String>, String> {
    if incoming.is_empty() {
        return Err("Nothing to import".into());
    }
    // Prepare and validate everything before mutating the saved state.
    let mut reserved: HashSet<String> = data.groups.keys().cloned().collect();
    let mut prepared = Vec::new();
    for (name, mut group) in incoming {
        let base = name.trim();
        if base.is_empty() || group.mods.iter().any(|m| m.spec.url.trim().is_empty()) {
            return Err("Import contains an empty group name or mod URL".into());
        }
        let mut candidate = base.to_owned();
        if reserved.contains(&candidate) {
            candidate = format!("{base} (imported)");
            let mut suffix = 2;
            while reserved.contains(&candidate) {
                candidate = format!("{base} (imported {suffix})");
                suffix += 1;
            }
        }
        reserved.insert(candidate.clone());
        // Imports are new objects, even when importing our own export repeatedly.
        group.id = crate::state::new_id();
        group.mods = group.mods.iter().map(ModConfig::duplicate).collect();
        prepared.push((candidate, group));
    }
    let names = prepared
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    for (name, group) in prepared {
        data.groups.insert(name.clone(), group);
        attach(data.profiles.get_mut(&data.active_profile).unwrap(), &name);
    }
    Ok(names)
}

pub(super) fn export_group_mod_list(data: &Data, name: &str) -> Result<String, String> {
    let group = data
        .groups
        .get(name)
        .ok_or("The selected group no longer exists")?;
    if group
        .mods
        .iter()
        .any(|m| m.spec.url.contains('\n') || m.spec.url.contains('\r'))
    {
        return Err("A mod URL contains a newline and cannot be exported to TXT".into());
    }
    Ok(group
        .mods
        .iter()
        .map(|m| m.spec.url.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n")
}

#[cfg(test)]
mod group_import_export_tests {
    use super::*;

    #[test]
    fn txt_import_keeps_order_and_deduplicates_lines() {
        let incoming = parse_mod_list_txt(
            "C:\\mods\\a.pak\nhttps://mod.io/g/drg/m/example#123\nC:\\mods\\a.pak\n",
            "My group",
        )
        .unwrap();
        let mods = &incoming["My group"].mods;
        assert_eq!(mods.len(), 2);
        assert_eq!(mods[0].spec.url, "C:\\mods\\a.pak");
        assert_eq!(mods[1].spec.url, "https://mod.io/g/drg/m/example#123");
        assert_eq!(mods[0].priority, 0);
        assert!(mods[0].enabled);
    }

    #[test]
    fn json_roundtrip_preserves_group_settings() {
        let group = crate::state::ModGroup {
            id: crate::state::new_id(),
            description: None,
            mods: vec![ModConfig {
                id: crate::state::new_id(),
                description: None,
                spec: ModSpecification::new("https://mod.io/g/drg/m/example#123".into()),
                enabled: false,
                required: true,
                member_id: Some("member1".into()),
                priority: 14,
            }],
        };
        let groups = std::collections::BTreeMap::from([("Example".into(), group)]);
        let bundle = GroupExportFile {
            format: GROUP_EXPORT_FORMAT.into(),
            version: GROUP_EXPORT_VERSION,
            groups,
        };
        let decoded = parse_groups_json(&serde_json::to_string(&bundle).unwrap()).unwrap();
        let member = &decoded["Example"].mods[0];
        assert_eq!(member.priority, 14);
        assert!(!member.enabled);
        assert!(member.required);
        assert_eq!(member.member_id.as_deref(), Some("member1"));
    }

    #[test]
    fn import_collision_creates_unique_name_and_preserves_existing_group() {
        let mut data: Data = serde_json::from_value(serde_json::json!({
            "active_profile": "A",
            "profiles": { "A": { "mods": [] } },
            "groups": { "Example": { "mods": [] } }
        }))
        .unwrap();
        let incoming =
            parse_mod_list_txt("https://mod.io/g/drg/m/example#123\n", "Example").unwrap();
        let names = import_groups(&mut data, incoming).unwrap();
        assert_eq!(names, vec!["Example (imported)"]);
        assert!(data.groups["Example"].mods.is_empty());
        assert_eq!(data.groups["Example (imported)"].mods.len(), 1);
        assert!(data.profiles["A"].mods.iter().any(|item|
            matches!(item, ModOrGroup::Group { group_name, .. } if group_name == "Example (imported)")));
    }

    #[test]
    fn repeated_json_import_allocates_fresh_ids_and_preserves_descriptions() {
        let mut data = Data::default();
        let mut incoming = parse_mod_list_txt("https://mod.io/g/drg/m/example", "Tools").unwrap();
        incoming.get_mut("Tools").unwrap().description = Some("Group help".into());
        incoming.get_mut("Tools").unwrap().mods[0].description = Some("Mod help".into());
        let bundle = GroupExportFile {
            format: GROUP_EXPORT_FORMAT.into(),
            version: GROUP_EXPORT_VERSION,
            groups: incoming,
        };
        let text = serde_json::to_string(&bundle).unwrap();
        for _ in 0..2 {
            import_groups(&mut data, parse_groups_json(&text).unwrap()).unwrap();
        }
        let a = &data.groups["Tools"];
        let b = &data.groups["Tools (imported)"];
        assert_ne!(a.id, b.id);
        assert_ne!(a.mods[0].id, b.mods[0].id);
        assert_ne!(a.id, bundle.groups["Tools"].id);
        assert_ne!(a.mods[0].id, bundle.groups["Tools"].mods[0].id);
        assert_eq!(a.description.as_deref(), Some("Group help"));
        assert_eq!(b.mods[0].description.as_deref(), Some("Mod help"));
        assert_ne!(
            data.profiles["default"].mods[0].id(),
            data.profiles["default"].mods[1].id()
        );
    }

    #[test]
    fn unsupported_bundle_version_is_rejected() {
        let invalid = r#"{"format":"mintfixed-groups","version":99,"groups":{"x":{"mods":[]}}}"#;
        assert!(parse_groups_json(invalid).is_err());
    }

    #[test]
    fn exporting_one_json_group_preserves_its_name_and_mod_settings() {
        let mut data = Data::default();
        let mut group = crate::state::ModGroup::default();
        group.description = Some("A group description".into());
        group.mods.push(ModConfig {
            id: crate::state::new_id(),
            description: Some("A mod description".into()),
            spec: ModSpecification::new("https://mod.io/g/drg/m/example#123".into()),
            enabled: false,
            required: false,
            member_id: None,
            priority: 12,
        });
        data.groups.insert("Tools".to_owned(), group);
        let json = serialize_single_group(&data, "Tools").unwrap();
        let imported = parse_single_group_json(&json).unwrap();
        assert_eq!(imported.suggested_name.as_deref(), Some("Tools"));
        assert_eq!(
            imported.group.description.as_deref(),
            Some("A group description")
        );
        assert_eq!(
            imported.group.mods[0].description.as_deref(),
            Some("A mod description")
        );
        assert_eq!(imported.group.mods[0].priority, 12);
        assert!(!imported.group.mods[0].enabled);
    }

    #[test]
    fn nameless_json_requires_a_name_and_multigroup_json_is_rejected() {
        let imported = parse_single_group_json(r#"{"group":{"mods":[]}}"#).unwrap();
        assert!(imported.suggested_name.is_none());
        let invalid = r#"{"format":"mintfixed-groups","version":1,"groups":{"A":{"mods":[]},"B":{"mods":[]}}}"#;
        assert!(parse_single_group_json(invalid).is_err());
    }
}
