use super::*;
use crate::state::ModGroup;

fn mod_config(name: &str) -> ModConfig {
    ModConfig {
        id: crate::state::new_id(),
        description: None,
        member_id: None,
        spec: ModSpecification::new(name.to_owned()),
        enabled: true,
        required: false,
        priority: 0,
    }
}

#[test]
fn unavailable_details_include_all_saved_locations() {
    use crate::state::{ModData_v0_1_0, ModProfile_v0_1_0};
    let mut data = ModData_v0_1_0::default();
    data.profiles.clear();
    data.groups.clear();
    let mut deleted = mod_config("deleted-url");
    deleted.enabled = false;
    data.groups.insert(
        "Shared".into(),
        ModGroup {
            mods: vec![deleted.clone(), mod_config("available-url")],
            ..Default::default()
        },
    );
    data.groups.insert(
        "Unused".into(),
        ModGroup {
            mods: vec![deleted.clone()],
            ..Default::default()
        },
    );
    for profile in ["A", "B"] {
        data.profiles.insert(
            profile.into(),
            ModProfile_v0_1_0 {
                mods: vec![ModOrGroup::Group {
                    id: crate::state::new_id(),
                    group_name: "Shared".into(),
                    enabled: false,
                    member_overrides: Default::default(),
                }],
                ..Default::default()
            },
        );
    }
    data.profiles
        .get_mut("A")
        .unwrap()
        .mods
        .push(ModOrGroup::Individual(deleted));
    let details = unavailable_mods::locations(
        &data,
        |spec| spec.url == "deleted-url",
        |_| "Deleted mod".into(),
    );
    assert!(details.contains("Deleted mod — profile \"A\"\n"));
    assert!(details.contains("profile \"A\", group \"Shared\""));
    assert!(details.contains("profile \"B\", group \"Shared\""));
    assert!(details.contains("group \"Unused\" (not used by any profile)"));
    assert_eq!(details.matches("  deleted-url").count(), 4);
    assert!(!details.contains("available-url"));
}

#[test]
fn unavailable_details_support_url_names_and_unreferenced_cache_entries() {
    use crate::state::{ModData_v0_1_0, ModProfile_v0_1_0};
    let mut data = ModData_v0_1_0::default();
    data.profiles.clear();
    data.groups.clear();
    data.profiles.insert(
        "A".into(),
        ModProfile_v0_1_0 {
            mods: vec![ModOrGroup::Individual(mod_config("deleted-url"))],
            ..Default::default()
        },
    );
    let details = unavailable_mods::locations(&data, |_| true, |spec| spec.url.clone());
    assert!(details.contains("deleted-url — profile \"A\""));
    let details = unavailable_mods::locations(&data, |_| false, |spec| spec.url.clone());
    assert!(details.contains("none are referenced"));
}

#[test]
fn unavailable_details_navigation_survives_renames_and_rejects_removed_entries() {
    use crate::state::{ModData_v0_1_0, ModProfile_v0_1_0};
    let mut data = ModData_v0_1_0::default();
    data.profiles.clear();
    data.groups.clear();
    let mut member = mod_config("deleted-url");
    member.enabled = false;
    data.groups.insert(
        "Shared".into(),
        ModGroup {
            mods: vec![member.clone()],
            ..Default::default()
        },
    );
    for name in ["A", "B"] {
        data.profiles.insert(
            name.into(),
            ModProfile_v0_1_0 {
                mods: vec![ModOrGroup::Group {
                    id: crate::state::new_id(),
                    group_name: "Shared".into(),
                    enabled: false,
                    member_overrides: Default::default(),
                }],
                ..Default::default()
            },
        );
    }
    let entries = unavailable_mods::entries(&data, |_| true, |_| "Deleted".into());
    let target = entries[1].target.as_ref().unwrap();
    assert_eq!(target.resolve(&data).unwrap().profile, "B");
    let profile = data.profiles.remove("B").unwrap();
    data.profiles.insert("Renamed".into(), profile);
    let group = data.groups.remove("Shared").unwrap();
    data.groups.insert("Renamed group".into(), group);
    for profile in data.profiles.values_mut() {
        if let ModOrGroup::Group { group_name, .. } = &mut profile.mods[0] {
            *group_name = "Renamed group".into();
        }
    }
    let resolved = target.resolve(&data).unwrap();
    assert_eq!(resolved.profile, "Renamed");
    assert_eq!(resolved.group.as_deref(), Some("Renamed group"));
    assert_eq!(resolved.mod_id, member.id);
    assert!(!data.groups["Renamed group"].mods[0].enabled);
    data.groups.get_mut("Renamed group").unwrap().mods.clear();
    assert!(target.resolve(&data).is_none());
    // A new mod with the same URL must not become the old target.
    data.groups
        .get_mut("Renamed group")
        .unwrap()
        .mods
        .push(mod_config("deleted-url"));
    assert!(target.resolve(&data).is_none());
}

#[test]
fn unavailable_details_navigation_distinguishes_duplicate_urls_and_unused_groups() {
    use crate::state::{ModData_v0_1_0, ModProfile_v0_1_0};
    let mut data = ModData_v0_1_0::default();
    data.profiles.clear();
    data.groups.clear();
    let first = mod_config("same-url");
    let second = mod_config("same-url");
    data.profiles.insert(
        "A".into(),
        ModProfile_v0_1_0 {
            mods: vec![
                ModOrGroup::Individual(first.clone()),
                ModOrGroup::Individual(second.clone()),
            ],
            ..Default::default()
        },
    );
    data.groups.insert(
        "Unused".into(),
        ModGroup {
            mods: vec![first.clone()],
            ..Default::default()
        },
    );
    let entries = unavailable_mods::entries(&data, |_| true, |_| "Same".into());
    let targets: Vec<_> = entries.iter().filter_map(|e| e.target.as_ref()).collect();
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].resolve(&data).unwrap().mod_id, first.id);
    assert_eq!(targets[1].resolve(&data).unwrap().mod_id, second.id);
    assert_eq!(entries.iter().filter(|e| e.target.is_none()).count(), 1);
    data.profiles.clear();
    assert!(targets[0].resolve(&data).is_none());
}

#[test]
fn unavailable_overview_refreshes_after_recovery_and_saved_edits() {
    use crate::state::{ModData_v0_1_0, ModProfile_v0_1_0};
    let mut data = ModData_v0_1_0::default();
    data.profiles.clear();
    data.groups.clear();
    data.profiles.insert(
        "A".into(),
        ModProfile_v0_1_0 {
            mods: vec![ModOrGroup::Individual(mod_config("deleted-url"))],
            ..Default::default()
        },
    );
    let mut overview = unavailable_mods::Overview::default();
    assert!(overview.refresh((0, (0, 0)), || unavailable_mods::entries(
        &data,
        |_| true,
        |_| "Deleted".into()
    )));
    assert_eq!(overview.entries.len(), 1);
    assert!(!overview.refresh((0, (0, 0)), || panic!("idle frame rebuilt overview")));
    // Provider recovery removes the warning even without another saved edit.
    assert!(overview.refresh((0, (1, 0)), || unavailable_mods::entries(
        &data,
        |_| false,
        |_| "Recovered".into()
    )));
    assert!(overview.entries.is_empty());
    assert!(overview.description().starts_with("No unavailable mods"));
    overview.refresh((0, (2, 0)), || {
        unavailable_mods::entries(&data, |_| true, |_| "Deleted".into())
    });
    assert_eq!(overview.entries.len(), 1);
    data.profiles.get_mut("A").unwrap().mods.clear();
    // Removing the final reference also removes the persistent badge.
    overview.refresh((1, (2, 0)), || {
        unavailable_mods::entries(&data, |_| true, |_| "Deleted".into())
    });
    assert!(overview.entries.is_empty());
}

#[test]
fn unavailable_preflight_respects_profile_selection_and_group_overrides() {
    use crate::state::{MemberOverrides, ModData_v0_1_0, ModProfile_v0_1_0};
    let mut data = ModData_v0_1_0::default();
    data.profiles.clear();
    data.groups.clear();
    let mut disabled = mod_config("disabled");
    disabled.enabled = false;
    let mut enabled = mod_config("enabled");
    let mut overrides = MemberOverrides::default();
    overrides.set(&mut disabled, true);
    overrides.set(&mut enabled, false);
    data.groups.insert(
        "Shared".into(),
        ModGroup {
            mods: vec![disabled.clone(), enabled.clone()],
            ..Default::default()
        },
    );
    data.groups.insert(
        "Unused".into(),
        ModGroup {
            mods: vec![mod_config("unused")],
            ..Default::default()
        },
    );
    data.profiles.insert(
        "Active".into(),
        ModProfile_v0_1_0 {
            mods: vec![
                ModOrGroup::Individual(disabled.clone()),
                ModOrGroup::Individual(mod_config("cached")),
                ModOrGroup::Individual(mod_config("healthy")),
                ModOrGroup::Individual(mod_config("missing")),
                ModOrGroup::Group {
                    id: crate::state::new_id(),
                    group_name: "Shared".into(),
                    enabled: true,
                    member_overrides: overrides,
                },
            ],
            ..Default::default()
        },
    );
    data.profiles.insert(
        "Other".into(),
        ModProfile_v0_1_0 {
            mods: vec![ModOrGroup::Individual(mod_config("other"))],
            ..Default::default()
        },
    );
    let blockers = unavailable_mods::install_blockers(
        &data,
        "Active",
        |spec| spec.url != "cached" && spec.url != "healthy",
        |spec| spec.url.clone(),
    );
    assert_eq!(blockers.len(), 2);
    assert!(blockers[0].label.starts_with("missing"));
    let target = blockers[1].target.as_ref().unwrap().resolve(&data).unwrap();
    assert_eq!(target.group.as_deref(), Some("Shared"));
    assert_eq!(target.mod_id, disabled.id);
    assert!(!data.groups["Shared"].mods[0].enabled);
    if let ModOrGroup::Group { enabled, .. } = &mut data.profiles.get_mut("Active").unwrap().mods[4]
    {
        *enabled = false;
    }
    let blockers = unavailable_mods::install_blockers(
        &data,
        "Active",
        |spec| spec.url != "cached" && spec.url != "healthy",
        |spec| spec.url.clone(),
    );
    assert_eq!(blockers.len(), 1);
}

#[test]
fn unavailable_download_status_checks_listed_versions_and_refreshes_on_request() {
    use crate::state::{ModData_v0_1_0, ModProfile_v0_1_0};
    let mut data = ModData_v0_1_0::default();
    data.profiles.clear();
    data.groups.clear();
    let first = mod_config("https://mod.io/g/drg/m/example#7/41");
    let second = mod_config("https://mod.io/g/drg/m/example#7/42");
    data.profiles.insert(
        "A".into(),
        ModProfile_v0_1_0 {
            mods: vec![
                ModOrGroup::Individual(first.clone()),
                ModOrGroup::Individual(second.clone()),
            ],
            ..Default::default()
        },
    );
    data.groups.insert(
        "Unused".into(),
        ModGroup {
            mods: vec![first.clone()],
            ..Default::default()
        },
    );
    let mut locations = unavailable_mods::entries(&data, |_| true, |_| "Example".into());
    let report = unavailable_mods::describe(&locations);
    let mut checks = 0;
    unavailable_mods::check_downloads(&mut locations, false, |spec| {
        checks += 1;
        spec == &first.spec
    });
    assert_eq!(checks, 2); // shared specifications require one disk check
    assert_eq!(
        locations.iter().filter(|entry| entry.is_cached()).count(),
        2
    );
    assert!(locations.iter().any(|entry| entry.spec == second.spec
        && entry.cache_status() == "Not cached — disable or replace"));
    unavailable_mods::check_downloads(&mut locations, false, |_| panic!("idle disk check"));
    let copied = unavailable_mods::copy_details(&report, &locations);
    assert!(copied.contains("Cached — can install"));
    assert!(copied.contains("Not cached — disable or replace"));
    // Cache annotation must not break status-details report splitting.
    assert_eq!(unavailable_mods::describe(&locations), report);
    unavailable_mods::check_downloads(&mut locations, true, |_| false);
    assert!(locations.iter().all(|entry| !entry.is_cached()));
    unavailable_mods::check_downloads(&mut locations, true, |_| true);
    assert!(locations.iter().all(|entry| entry.is_cached()));
}
