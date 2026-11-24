use super::*;
use crate::providers::ModResolution;
use crate::state::{ModConfig, ModProfile_v0_1_0};

fn spec(url: &str) -> ModSpecification {
    ModSpecification {
        url: url.to_owned(),
    }
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

fn info(url: &str) -> ModInfo {
    ModInfo {
        provider: "modio",
        name: url.to_owned(),
        spec: spec(url),
        versions: vec![],
        resolution: ModResolution::resolvable(url.to_owned().into()),
        suggested_require: false,
        suggested_dependencies: vec![],
        modio_tags: None,
        modio_id: None,
    }
}

#[test]
fn pins_identify_aliases_without_network_or_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
    assert_eq!(
        Artifact::cached(&store, &spec("https://mod.io/g/drg/m/old#123/10")),
        Artifact::cached(&store, &spec("https://mod.io/g/drg/m/new#123/10"))
    );
    assert_ne!(
        Artifact::cached(&store, &spec("https://mod.io/g/drg/m/old#123/10")),
        Artifact::cached(&store, &spec("https://mod.io/g/drg/m/old#123/11"))
    );
}

#[test]
fn unknown_versions_are_pending_not_guessed_from_similar_names() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
    let data = Data {
        bitfix_scripts: Vec::new(),
        active_profile: "p".into(),
        groups: BTreeMap::new(),
        profiles: BTreeMap::from([(
            "p".into(),
            ModProfile_v0_1_0 {
                id: crate::state::new_id(),
                mods: vec![
                    ModOrGroup::Individual(member("https://mod.io/g/drg/m/example")),
                    ModOrGroup::Individual(member("https://mod.io/g/drg/m/example-2")),
                ],
            },
        )]),
    };
    let issues = ProfileIssues::collect(&data, "p", &store);
    assert!(issues.versions.is_empty());
    assert!(issues.duplicates.is_empty());
    assert_eq!(issues.pending.len(), 2);
}
#[test]
fn readding_latest_or_its_pin_promotes_disabled_standalone_but_other_versions_remain_distinct()
{
    let dir = tempfile::tempdir().unwrap();
    // Use the current versioned format so numeric mod IDs are decoded via
    // the cache reader's numeric-key compatibility path.
    let cache = serde_json::json!({"version": "0.0.0", "cache": {"modio": {
        "type": "ModioCache", "mod_id_map": {"speedometer": 123},
        "modfile_blobs": {}, "dependencies": {}, "last_update_time": null,
        "mods": {"123": {"name_id": "speedometer", "name": "Speedometer",
            "latest_modfile": 10, "modfiles": [], "tags": []}}
    }}});
    std::fs::write(
        dir.path().join("cache.json"),
        serde_json::to_vec(&cache).unwrap(),
    )
    .unwrap();
    let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
    let latest = "https://mod.io/g/drg/m/speedometer";
    let pin = "https://mod.io/g/drg/m/speedometer#123/10";
    // An old or renamed URL slug must not override the real cached title.
    // The fixture deliberately has no cached dependency metadata.
    let issues_data = Data {
        bitfix_scripts: Vec::new(),
        active_profile: "p".into(),
        profiles: BTreeMap::from([(
            "p".into(),
            ModProfile_v0_1_0 {
                id: crate::state::new_id(),
                mods: vec![
                    ModOrGroup::Individual(member("https://mod.io/g/drg/m/old-slug#123/10")),
                    ModOrGroup::Individual(member("https://mod.io/g/drg/m/old-slug#123/11")),
                ],
            },
        )]),
        groups: BTreeMap::new(),
    };
    let issues = ProfileIssues::collect(&issues_data, "p", &store);
    assert_eq!(issues.versions.len(), 1);
    assert!(
        issues.versions[0].starts_with("Speedometer (mod.io ID 123) has multiple versions:")
    );
    let mut existing = member(latest);
    existing.enabled = false;
    let mut profile = ModProfile_v0_1_0 {
        id: crate::state::new_id(),
        mods: vec![ModOrGroup::Individual(existing)],
    };
    assert!(promote_standalone(&mut profile, &spec(pin), &store));
    assert!(promote_standalone(&mut profile, &spec(latest), &store));
    assert_eq!(profile.mods.len(), 1);
    let ModOrGroup::Individual(existing) = &profile.mods[0] else {
        panic!()
    };
    assert!(existing.enabled);
    assert_eq!(existing.spec.url, latest);
    assert!(!promote_standalone(
        &mut profile,
        &spec("https://mod.io/g/drg/m/speedometer#123/11"),
        &store
    ));

    let mut resolved = HashMap::from([(spec(latest), info(pin)), (spec(pin), info(pin))]);
    let explicit = take_explicit(
        vec![spec(latest), spec(pin), spec(latest)],
        &mut resolved,
        &store,
    );
    assert_eq!(explicit.len(), 1);
    assert_eq!(explicit[0].0, spec(latest));
    assert!(
        resolved.is_empty(),
        "Skipped aliases must not return as dependencies"
    );
}

#[test]
fn renamed_local_copies_match_and_edits_invalidate_identity() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
    let a = dir.path().join("original.pak");
    let b = dir.path().join("renamed.pak");
    std::fs::write(&a, b"pak content one").unwrap();
    std::fs::copy(&a, &b).unwrap();
    let a = spec(a.to_str().unwrap());
    let b = spec(b.to_str().unwrap());
    store.refresh_local_files(&[a.clone(), b.clone()]);
    let revision = store.metadata_revision();
    assert_eq!(Artifact::cached(&store, &a), Artifact::cached(&store, &b));
    let mut profile = ModProfile_v0_1_0 {
        id: crate::state::new_id(),
        mods: vec![ModOrGroup::Individual(member(&a.url))],
    };
    assert!(promote_standalone(&mut profile, &b, &store));
    // Same length, different bytes: no filename/length-based assumption.
    std::fs::write(&b.url, b"pak content two").unwrap();
    store.refresh_local_files(&[a.clone(), b.clone()]);
    assert_ne!(revision, store.metadata_revision());
    assert_ne!(Artifact::cached(&store, &a), Artifact::cached(&store, &b));
    let mut profile = ModProfile_v0_1_0 {
        id: crate::state::new_id(),
        mods: vec![ModOrGroup::Individual(member(&a.url))],
    };
    assert!(!promote_standalone(&mut profile, &b, &store));
    std::fs::remove_file(&b.url).unwrap();
    store.refresh_local_files(std::slice::from_ref(&b));
    assert!(store.cached_local_file(&b).is_none());
}

#[test]
fn readding_moves_to_top_removes_old_copies_and_preserves_other_versions_and_groups() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
    let pin = "https://mod.io/g/drg/m/a#123/10";
    let alias = "https://mod.io/g/drg/m/renamed#123/10";
    let different = "https://mod.io/g/drg/m/a#123/11";
    let mut old = member(pin);
    old.enabled = false;
    old.priority = 42;
    old.required = true;
    let mut profile = ModProfile_v0_1_0 {
        id: crate::state::new_id(),
        mods: vec![
            ModOrGroup::Individual(member("unrelated")),
            ModOrGroup::Individual(old),
            ModOrGroup::Group {
                id: crate::state::new_id(),
                group_name: "Tools".into(),
                enabled: true,
                member_overrides: Default::default(),
            },
            ModOrGroup::Individual(member(pin)),
            ModOrGroup::Individual(member(different)),
        ],
    };
    assert!(promote_standalone(&mut profile, &spec(alias), &store));
    assert_eq!(profile.mods.len(), 4);
    let ModOrGroup::Individual(top) = &profile.mods[0] else {
        panic!()
    };
    assert_eq!(top.spec, spec(alias));
    assert!(top.enabled && top.required);
    assert_eq!(top.priority, 42);
    assert!(
        matches!(&profile.mods[1], ModOrGroup::Individual(mc) if mc.spec.url == "unrelated")
    );
    assert!(
        matches!(&profile.mods[2], ModOrGroup::Group { group_name, .. } if group_name == "Tools")
    );
    assert!(matches!(&profile.mods[3], ModOrGroup::Individual(mc) if mc.spec.url == different));
    assert!(promote_standalone(&mut profile, &spec(alias), &store));
    assert_eq!(profile.mods.len(), 4);
}
