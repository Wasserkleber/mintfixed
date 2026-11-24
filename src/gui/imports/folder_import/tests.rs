use super::*;

#[test]
fn scan_skips_existing_content_and_excluded_paths() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("mods");
    let cache = temp.path().join("cache");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    let existing = folder.join("already.pak");
    let duplicate = folder.join("duplicate.zip");
    let excluded = folder.join("excluded.pak");
    let fresh = folder.join("fresh.pak");
    std::fs::write(&existing, b"same contents").unwrap();
    std::fs::write(&duplicate, b"same contents").unwrap();
    std::fs::write(&excluded, b"excluded contents").unwrap();
    std::fs::write(&fresh, b"new contents").unwrap();
    let mut exclusions = Exclusions::default();
    exclusions
        .excluded_paths
        .insert(path_key(&excluded.canonicalize().unwrap()));
    exclusions.save(&cache).unwrap();
    let specs = [ModSpecification::new(existing.display().to_string())];
    let result = candidates(&folder, &cache, &specs).unwrap();
    assert_eq!(result, vec![fresh.canonicalize().unwrap()]);
    // If the file is no longer imported, the explicit exclusion still survives.
    let result = candidates(&folder, &cache, &[]).unwrap();
    assert!(!result.contains(&excluded.canonicalize().unwrap()));
}

#[test]
fn removed_mod_returns_to_import_menu_unless_explicitly_excluded() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("watched");
    let cache = temp.path().join("cache");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    let local = folder.join("removed.pak");
    std::fs::write(&local, b"mod contents").unwrap();
    let spec = ModSpecification::new(local.display().to_string());
    // While imported, it should not appear as a candidate.
    assert!(candidates(&folder, &cache, &[spec]).unwrap().is_empty());
    // After removing it from Mintfixed, it is offered for manual import.
    assert_eq!(
        candidates(&folder, &cache, &[]).unwrap(),
        vec![local.canonicalize().unwrap()]
    );
    // Only a deliberate exclusion suppresses it on subsequent menu opens.
    let mut exclusions = Exclusions::load(&cache).unwrap();
    exclusions
        .excluded_paths
        .insert(path_key(&local.canonicalize().unwrap()));
    exclusions.save(&cache).unwrap();
    assert!(candidates(&folder, &cache, &[]).unwrap().is_empty());
}

#[test]
fn import_only_hides_active_profile_mods_and_its_referenced_groups() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("mods");
    let cache = temp.path().join("cache");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::create_dir_all(&cache).unwrap();

    let active = folder.join("active.pak");
    let other = folder.join("other-profile.pak");
    let shared = folder.join("referenced-group.pak");
    let unused = folder.join("unreferenced-group.pak");
    std::fs::write(&active, b"active content").unwrap();
    std::fs::write(&other, b"other content").unwrap();
    std::fs::write(&shared, b"shared content").unwrap();
    std::fs::write(&unused, b"unused content").unwrap();

    let config = |path: &Path| crate::state::ModConfig {
        id: crate::state::new_id(),
        spec: ModSpecification::new(path.display().to_string()),
        description: None,
        required: false,
        member_id: None,
        enabled: true,
        priority: 0,
    };
    let mut data = crate::state::ModData_v0_1_0::default();
    let profile = data.profiles.get_mut("default").unwrap();
    profile
        .mods
        .push(crate::state::ModOrGroup::Individual(config(&active)));
    // Referenced shared-group members count, even when the group is disabled.
    profile.mods.push(crate::state::ModOrGroup::Group {
        id: crate::state::new_id(),
        group_name: "default".into(),
        enabled: false,
        member_overrides: Default::default(),
    });
    data.groups
        .get_mut("default")
        .unwrap()
        .mods
        .push(config(&shared));
    data.groups.insert(
        "unused".into(),
        crate::state::ModGroup {
            mods: vec![config(&unused)],
            ..Default::default()
        },
    );
    data.profiles.insert("other".into(), Default::default());
    let other_profile = data.profiles.get_mut("other").unwrap();
    other_profile
        .mods
        .push(crate::state::ModOrGroup::Individual(config(&other)));
    // The same mod is also referenced in another profile.
    other_profile
        .mods
        .push(crate::state::ModOrGroup::Individual(config(&active)));

    let found = candidates(&folder, &cache, &active_profile_specs(&data)).unwrap();
    assert_eq!(
        found,
        vec![
            other.canonicalize().unwrap(),
            unused.canonicalize().unwrap()
        ]
    );

    // Deleting the standalone row from the active profile makes it
    // importable again, even though another profile still references it.
    data.profiles
        .get_mut("default")
        .unwrap()
        .mods
        .retain(|item| !matches!(item, crate::state::ModOrGroup::Individual(_)));
    let found = candidates(&folder, &cache, &active_profile_specs(&data)).unwrap();
    assert_eq!(
        found,
        vec![
            active.canonicalize().unwrap(),
            other.canonicalize().unwrap(),
            unused.canonicalize().unwrap(),
        ]
    );
}

#[test]
fn invalid_exclusions_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(EXCLUSIONS_FILE), b"not json").unwrap();
    assert!(Exclusions::load(temp.path()).is_err());
}
