use super::*;
use crate::providers::ModResolution;
use crate::state::{ModData_v0_1_0 as Data, ModOrGroup};
use serde_json::json;

fn info(provider: &'static str, name: &str, url: &str) -> ModInfo {
    ModInfo {
        provider,
        name: name.into(),
        spec: ModSpecification::new(url.into()),
        resolution: ModResolution::resolvable(url.to_owned().into()),
        versions: vec![],
        suggested_require: false,
        suggested_dependencies: vec![],
        modio_tags: None,
        modio_id: None,
    }
}

fn profile() -> Data {
    serde_json::from_value(json!({
        "active_profile": "My profile",
        "profiles": {
            "My profile": {"mods": [
                {"spec":{"url":"old"},"required":false,"enabled":true,"priority":0},
                {"group_name":"Utilities","enabled":true,"member_overrides":{"member":true}},
                {"group_name":"Inactive","enabled":false}
            ]},
            "Other profile": {"mods":[{"group_name":"Inactive","enabled":true}]}
        },
        "groups": {
            "Utilities":{"mods":[{"spec":{"url":"new"},"required":false,"enabled":false,"member_id":"member","priority":10}]},
            "Inactive":{"mods":[{"spec":{"url":"disabled"},"required":false,"enabled":true}]}
        }
    })).unwrap()
}

fn resolved() -> HashMap<ModSpecification, ModInfo> {
    [("old", 10), ("new", 11), ("disabled", 12)]
        .into_iter()
        .map(|(spec, file)| {
            (
                ModSpecification::new(spec.into()),
                info(
                    "modio",
                    "Speedometer",
                    &format!("https://mod.io/g/drg/m/speedometer#123/{file}"),
                ),
            )
        })
        .collect()
}

#[test]
fn conflict_reports_name_profile_locations_and_both_resolved_files() {
    let entries = profile().ordered_install_entries("My profile");
    assert_eq!(
        entries
            .iter()
            .map(|e| e.spec.url.as_str())
            .collect::<Vec<_>>(),
        ["new", "old"]
    );
    let error = installation_inputs(&entries, &resolved())
        .unwrap_err()
        .to_string();
    for expected in [
        "Speedometer",
        "mod.io ID 123",
        "File 10",
        "File 11",
        "profile \"My profile\"",
        "standalone row 1",
        "group \"Utilities\" (row 2), mod #1",
    ] {
        assert!(error.contains(expected), "{error}");
    }
    assert!(!error.contains("File 12"));
    assert!(!error.contains("Other profile"));
}

#[test]
fn disabled_parent_or_member_override_does_not_block_installation() {
    let mut data = profile();
    if let ModOrGroup::Group { enabled, .. } =
        &mut data.profiles.get_mut("My profile").unwrap().mods[1]
    {
        *enabled = false;
    }
    assert_eq!(
        installation_inputs(&data.ordered_install_entries("My profile"), &resolved())
            .unwrap()
            .len(),
        1
    );
    if let ModOrGroup::Group {
        enabled,
        member_overrides,
        ..
    } = &mut data.profiles.get_mut("My profile").unwrap().mods[1]
    {
        *enabled = true;
        member_overrides.forget("member"); // shared member defaults to disabled
    }
    assert_eq!(
        installation_inputs(&data.ordered_install_entries("My profile"), &resolved())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn group_to_group_conflict_includes_both_reference_locations() {
    let mut data = profile();
    data.profiles.get_mut("My profile").unwrap().mods.remove(0);
    if let ModOrGroup::Group { enabled, .. } =
        &mut data.profiles.get_mut("My profile").unwrap().mods[1]
    {
        *enabled = true;
    }
    let error = installation_inputs(&data.ordered_install_entries("My profile"), &resolved())
        .unwrap_err()
        .to_string();
    for expected in [
        "group \"Utilities\" (row 1)",
        "group \"Inactive\" (row 2)",
        "File 11",
        "File 12",
    ] {
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn latest_and_pin_resolving_to_same_file_install_once_in_stable_priority_order() {
    let mut data = profile();
    let mut resolved = resolved();
    let latest = resolved[&ModSpecification::new("new".into())]
        .resolution
        .clone();
    resolved
        .get_mut(&ModSpecification::new("old".into()))
        .unwrap()
        .resolution = latest;
    let entries = data.ordered_install_entries("My profile");
    let selected = installation_inputs(&entries, &resolved).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].spec, resolved[&entries[0].spec].spec);
    data.groups.get_mut("Utilities").unwrap().mods[0].priority = 0;
    let tied = data.ordered_install_entries("My profile");
    assert_eq!(
        tied.iter().map(|e| e.spec.url.as_str()).collect::<Vec<_>>(),
        ["old", "new"]
    );
    let group = data.groups.remove("Utilities").unwrap();
    data.groups.insert("Renamed".into(), group);
    assert!(entries[0].location.contains("Utilities")); // install-time snapshot
}

#[test]
fn every_duplicate_location_is_retained_when_another_version_conflicts() {
    let mut entries = profile().ordered_install_entries("My profile");
    let mut same = entries[1].clone();
    same.location = "second occurrence of old version".into();
    entries.push(same);
    let error = installation_inputs(&entries, &resolved())
        .unwrap_err()
        .to_string();
    assert!(error.contains("standalone row 1"));
    assert!(error.contains("second occurrence of old version"));
    let missing = installation_inputs(&entries, &HashMap::new())
        .unwrap_err()
        .to_string();
    assert!(missing.contains("Missing resolved metadata"));
    assert!(missing.contains("Utilities"));
}

#[test]
fn packer_rechecks_versions_before_reading_game_or_truncating_output() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("mods_P.pak");
    std::fs::write(&output, b"previous working installation").unwrap();
    let a = info(
        "modio",
        "Speedometer",
        "https://mod.io/g/drg/m/speedometer#123/10",
    );
    let b = info(
        "modio",
        "Speedometer",
        "https://mod.io/g/drg/m/speedometer#123/11",
    );
    let missing = dir.path().join("not-downloaded.pak");
    let error = crate::integrate::integrate(
        dir.path().join("FSD-WindowsNoEditor.pak"),
        mint_lib::mod_info::MetaConfig {},
        vec![
            (info("file", "Local", "local"), missing.clone()),
            (a, missing.clone()),
            (b, missing),
        ],
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("conflicting enabled mod versions"),
        "{error}"
    );
    assert!(error.contains("File 10") && error.contains("File 11"));
    assert_eq!(
        std::fs::read(&output).unwrap(),
        b"previous working installation"
    );
}

#[test]
fn missing_or_inconsistent_resolved_identity_is_rejected_at_packer() {
    let path = PathBuf::from("unused.pak");
    let unknown = info("modio", "Unknown", "https://mod.io/g/drg/m/unknown");
    assert!(
        prepare_packer_inputs(vec![(unknown, path.clone())])
            .unwrap_err()
            .to_string()
            .contains("Missing resolved mod.io file ID")
    );
    let mut mismatch = info("modio", "Mismatch", "https://mod.io/g/drg/m/known#123/10");
    mismatch.modio_id = Some(456);
    assert!(
        prepare_packer_inputs(vec![(mismatch, path)])
            .unwrap_err()
            .to_string()
            .contains("Inconsistent mod.io identity")
    );
}

#[test]
fn packer_deduplicates_same_modio_file_but_keeps_distinct_mods() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("same-bytes.pak");
    std::fs::write(&path, b"same bytes do not merge distinct mod.io identities").unwrap();
    let selected = prepare_packer_inputs(vec![
        (
            info("modio", "First", "https://mod.io/g/drg/m/one#123/10"),
            path.clone(),
        ),
        (
            info("modio", "Alias", "https://mod.io/g/drg/m/alias#123/10"),
            path.clone(),
        ),
        (
            info("modio", "Distinct", "https://mod.io/g/drg/m/two#456/20"),
            path,
        ),
    ])
    .unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|(info, _)| info.name.as_str())
            .collect::<Vec<_>>(),
        ["First", "Distinct"]
    );
}

#[test]
fn downloaded_http_copies_and_local_copies_are_processed_once() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.pak");
    let b = dir.path().join("b.pak");
    let c = dir.path().join("c.pak");
    std::fs::write(&a, b"same complete pak bytes").unwrap();
    std::fs::copy(&a, &b).unwrap();
    std::fs::write(&c, b"different pak bytes").unwrap();
    let selected = prepare_packer_inputs(vec![
        (
            info("http", "Highest priority", "https://one.example/a.pak"),
            a,
        ),
        (
            info("http", "HTTP mirror", "https://two.example/b.pak"),
            b.clone(),
        ),
        (info("file", "Local copy", "local.pak"), b),
        (info("file", "Different mod", "other.pak"), c),
    ])
    .unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|(info, _)| info.name.as_str())
            .collect::<Vec<_>>(),
        ["Highest priority", "Different mod"]
    );
}
