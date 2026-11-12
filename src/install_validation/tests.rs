use super::*;
use crate::mod_identity::ProfileIssues;
use crate::providers::{ModResolution, ModStore};
use crate::state::{MemberOverrides, ModConfig, ModGroup, ModProfile_v0_1_0};
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
        mint_lib::mod_info::MetaConfig {
            bundled_crash_reporter: false,
        },
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

fn spec(url: &str) -> ModSpecification {
    ModSpecification::new(url.into())
}

fn resolved_info(url: &str) -> ModInfo {
    info("modio", url, url)
}

fn member(url: &str) -> ModConfig {
    ModConfig {
        id: crate::state::new_id(),
        description: None,
        spec: spec(url),
        required: false,
        member_id: None,
        enabled: true,
        priority: 0,
    }
}

#[test]
fn disabled_groups_warn_but_do_not_block_and_shared_members_are_not_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
    let a = "https://mod.io/g/drg/m/a#123/10";
    let b = "https://mod.io/g/drg/m/a#123/11";
    let data = Data {
        bitfix_scripts: Default::default(),
        active_profile: "p".into(),
        profiles: BTreeMap::from([(
            "p".into(),
            ModProfile_v0_1_0 {
                id: crate::state::new_id(),
                mods: vec![
                    ModOrGroup::Individual(member(a)),
                    ModOrGroup::Group {
                        id: crate::state::new_id(),
                        group_name: "same".into(),
                        enabled: true,
                        member_overrides: MemberOverrides::default(),
                    },
                    ModOrGroup::Group {
                        id: crate::state::new_id(),
                        group_name: "other".into(),
                        enabled: false,
                        member_overrides: MemberOverrides::default(),
                    },
                ],
            },
        )]),
        groups: BTreeMap::from([
            (
                "same".into(),
                ModGroup {
                    tint_color: None,
                    id: crate::state::new_id(),
                    description: None,
                    mods: vec![member(a)],
                },
            ),
            (
                "other".into(),
                ModGroup {
                    tint_color: None,
                    id: crate::state::new_id(),
                    description: None,
                    mods: vec![member(b), member(b)],
                },
            ),
        ]),
    };
    let issues = ProfileIssues::collect(&data, "p", &store);
    assert_eq!(issues.versions.len(), 1);
    assert!(issues.versions[0].starts_with("a (mod.io ID 123)"));
    assert!(issues.versions[0].contains("disabled"));
    assert!(!issues.duplicate(None, 0));
    assert!(!issues.duplicate(Some("same"), 0));
    assert!(issues.duplicate(Some("other"), 0));
    assert!(issues.duplicate(Some("other"), 1));
    let inputs = data.ordered_install_specs("p");
    let resolved = HashMap::from([(spec(a), resolved_info(a))]);
    assert_eq!(
        installation_inputs(&InstallEntry::requested(&inputs), &resolved)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn installer_coalesces_aliases_keeps_order_and_rejects_version_conflicts() {
    let a = spec("alias-a");
    let b = spec("alias-b");
    let c = spec("another-mod");
    let mut resolved = HashMap::from([
        (a.clone(), resolved_info("https://mod.io/g/drg/m/a#123/10")),
        (
            b.clone(),
            resolved_info("https://mod.io/g/drg/m/renamed#123/10"),
        ),
        (c.clone(), resolved_info("https://mod.io/g/drg/m/c#456/20")),
    ]);
    let inputs = vec![c.clone(), a.clone(), b.clone()];
    let selected = installation_inputs(&InstallEntry::requested(&inputs), &resolved).unwrap();
    assert_eq!(selected.len(), 2);
    assert_eq!(selected[0].name, resolved[&c].name);
    assert_eq!(selected[1].name, resolved[&a].name);
    resolved.insert(b, resolved_info("https://mod.io/g/drg/m/a#123/11"));
    assert!(installation_inputs(&InstallEntry::requested(&inputs), &resolved).is_err());
    assert!(installation_inputs(&InstallEntry::requested(&[spec("missing")]), &resolved).is_err());
}

#[test]
fn local_packer_dedup_keeps_first_priority_and_checks_files_again() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.pak");
    let b = dir.path().join("b.pak");
    let c = dir.path().join("c.pak");
    std::fs::write(&a, b"same special asset bytes").unwrap();
    std::fs::copy(&a, &b).unwrap();
    std::fs::write(&c, b"different content").unwrap();
    let local = |name: &str| {
        let mut value = resolved_info(name);
        value.provider = "file";
        value
    };
    let inputs = vec![
        (local("first"), b.clone()),
        (local("copy"), a),
        (local("last"), c),
    ];
    let unique = prepare_packer_inputs(inputs).unwrap();
    assert_eq!(unique.len(), 2);
    assert_eq!(unique[0].0.name, "first");
    assert_eq!(unique[1].0.name, "last");
    std::fs::remove_file(&b).unwrap();
    assert!(prepare_packer_inputs(unique).is_err());
}

#[test]
fn dependency_warnings_match_requested_and_resolved_urls_and_deduplicate() {
    let mut parent = info("modio", "Parent", "parent");
    parent.suggested_dependencies = ["requested", "resolved", "missing", "missing"]
        .into_iter()
        .map(spec)
        .collect();
    let mut dependency = info("modio", "Dependency", "resolved");
    dependency.spec = spec("requested");
    assert_eq!(
        missing_dependencies([&parent, &dependency]),
        vec!["missing"]
    );
    assert!(missing_dependencies([&dependency]).is_empty());
    assert!(missing_dependencies(std::iter::empty()).is_empty());
}
