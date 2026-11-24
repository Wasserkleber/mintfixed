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

#[test]
fn configured_mod_folder_finds_exact_filename_in_subfolders() {
    let temp = tempfile::tempdir().unwrap();
    let subfolder = temp.path().join("nested").join("another");
    std::fs::create_dir_all(&subfolder).unwrap();
    let target = subfolder.join("RainbowNitra.pak");
    std::fs::write(&target, b"mod").unwrap();
    std::fs::write(temp.path().join("RainbowNitraOlder.pak"), b"other").unwrap();
    let wanted = BTreeSet::from(["rainbownitra.pak".into()]);
    let matches = find_in_mod_folder(temp.path(), &wanted).unwrap();
    assert_eq!(matches["rainbownitra.pak"], vec![target]);
}

#[test]
fn identical_names_in_multiple_folders_are_ambiguous() {
    let temp = tempfile::tempdir().unwrap();
    for folder in ["first", "second"] {
        let dir = temp.path().join(folder);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("same.zip"), b"mod").unwrap();
    }
    let wanted = BTreeSet::from(["same.zip".into()]);
    let matches = find_in_mod_folder(temp.path(), &wanted).unwrap();
    assert_eq!(matches["same.zip"].len(), 2);
}

#[test]
fn automatic_relocation_preserves_group_members_and_does_not_guess() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("mod.pak");
    std::fs::write(&target, b"mod").unwrap();
    let old = r"C:\AnotherPC\Mods\mod.pak";
    let mut data = Data::default();
    data.groups.insert(
        "Imported".into(),
        ModGroup {
            id: crate::state::new_id(),
            tint_color: None,
            description: None,
            mods: vec![member(old)],
        },
    );
    data.groups.insert(
        "Other".into(),
        ModGroup {
            id: crate::state::new_id(),
            tint_color: None,
            description: None,
            mods: vec![member(old)],
        },
    );
    let id = data.groups["Imported"].mods[0].id.clone();
    let mut notice = Notice {
        origin: Origin::Group("Imported".into()),
        paths: vec![old.into()],
        error: None,
        scan_error: None,
        relinked: Vec::new(),
        auto_count: 0,
        ambiguous: BTreeMap::new(),
        scan: Scan::Pending,
    };
    let matches = BTreeMap::from([("mod.pak".into(), vec![target.clone()])]);
    assert!(apply_matches(&mut notice, &mut data, &matches));
    assert!(notice.paths.is_empty());
    assert_eq!(
        data.groups["Imported"].mods[0].spec.url,
        target.to_string_lossy()
    );
    assert_eq!(data.groups["Imported"].mods[0].id, id);
    assert_eq!(data.groups["Imported"].mods[0].priority, 7);
    assert!(!data.groups["Imported"].mods[0].enabled);
    assert_eq!(data.groups["Other"].mods[0].spec.url, old);

    let mut ambiguous = Notice {
        origin: Origin::Group("Other".into()),
        paths: vec![old.into()],
        error: None,
        scan_error: None,
        relinked: Vec::new(),
        auto_count: 0,
        ambiguous: BTreeMap::new(),
        scan: Scan::Pending,
    };
    let duplicates = BTreeMap::from([("mod.pak".into(), vec![target.clone(), target])]);
    assert!(!apply_matches(&mut ambiguous, &mut data, &duplicates));
    assert_eq!(ambiguous.paths, vec![old]);
    assert_eq!(ambiguous.ambiguous[old], 2);
    assert_eq!(data.groups["Other"].mods[0].spec.url, old);
}

#[test]
fn only_mod_archive_filenames_qualify_for_automatic_search() {
    assert_eq!(
        filename_key(r"C:\Old\Mods\Example.PAK"),
        Some("example.pak".into())
    );
    assert_eq!(
        filename_key("/home/old/Mods/file.zip"),
        Some("file.zip".into())
    );
    assert_eq!(filename_key("/home/old/Mods/folder"), None);
    assert_eq!(filename_key("file.txt"), None);
}
