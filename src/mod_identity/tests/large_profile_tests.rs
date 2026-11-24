//! Large-profile regression and opt-in scaling coverage for the cached
//! dependency-warning code. All fixtures are synthetic and offline.
use super::*;
use crate::state::{MemberOverrides, ModConfig, ModGroup, ModProfile_v0_1_0};
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

fn spec(url: impl Into<String>) -> ModSpecification {
    ModSpecification { url: url.into() }
}

fn member(id: impl Into<String>, url: impl Into<String>, enabled: bool) -> ModConfig {
    ModConfig {
        id: id.into(),
        description: None,
        spec: spec(url),
        required: false,
        member_id: None,
        enabled,
        priority: 0,
    }
}

fn standalone_data(count: usize) -> Data {
    Data {
        active_profile: "big".into(),
        bitfix_scripts: Vec::new(),
        groups: BTreeMap::new(),
        profiles: BTreeMap::from([(
            "big".into(),
            ModProfile_v0_1_0 {
                id: "profile-big".into(),
                mods: (0..count)
                    .map(|i| {
                        ModOrGroup::Individual(member(
                            format!("mod-{i}"),
                            format!("synthetic://profile/{i}.pak"),
                            i % 5 != 0,
                        ))
                    })
                    .collect(),
            },
        )]),
    }
}

#[test]
fn large_profile_4096_mods_index_agrees_with_pairwise_matching() {
    // Verifies the optimized lookup against the pre-optimization reference
    // behavior, including numeric IDs, slugs, pins, local files and near misses.
    const COUNT: usize = 4096;
    let installed: Vec<_> = (0..COUNT)
        .map(|i| {
            if i % 7 == 0 {
                spec(format!("synthetic://file-{i}.pak"))
            } else {
                spec(format!(
                    "https://mod.io/g/drg/m/mod-{i}#{}/{}",
                    100_000 + i,
                    1 + i % 8
                ))
            }
        })
        .collect();
    let index = DependencyIndex::from_specs(installed.iter());
    let mut matches = 0;
    let mut misses = 0;
    for q in 0..960 {
        let i = (q * 73) % COUNT;
        let required = if i.is_multiple_of(7) {
            match q % 3 {
                0 => installed[i].clone(),
                1 => spec(format!("synthetic://file-{i}.pak2")),
                _ => spec(format!("synthetic://file-{}.pak", i + COUNT)),
            }
        } else {
            match q % 6 {
                0 => installed[i].clone(),
                1 => spec(format!("https://mod.io/g/drg/m/renamed#{}/9", 100_000 + i)),
                2 => spec(format!("https://mod.io/g/drg/m/mod-{i}")),
                3 => spec(format!("https://mod.io/g/drg/m/mod-{i}#{}", 100_000 + i)),
                4 => spec(format!("https://mod.io/g/drg/m/mod-{i}#{}", 400_000 + i)),
                _ => spec(format!("https://mod.io/g/drg/m/mod-{i}-extra")),
            }
        };
        let reference = installed
            .iter()
            .any(|candidate| candidate.satisfies_dependency(&required));
        assert_eq!(
            index.contains(&required),
            reference,
            "large-profile index disagrees for {}",
            required.url
        );
        if reference {
            matches += 1;
        } else {
            misses += 1;
        }
    }
    assert!(matches > 100, "fixture must exercise successful matches");
    assert!(misses > 100, "fixture must exercise missing dependencies");
}

#[test]
fn large_profile_6000_standalone_and_shared_group_entries() {
    // 6,000 standalone rows, four instances of the same 80-member group:
    // one disabled, one with 20 member overrides, two fully enabled.
    let mut data = standalone_data(6000);
    let mut shared: Vec<_> = (0..80)
        .map(|i| {
            member(
                format!("shared-{i}"),
                format!("synthetic://shared/{i}.pak"),
                true,
            )
        })
        .collect();
    let mut partial = MemberOverrides::default();
    for mc in shared.iter_mut().take(20) {
        partial.set(mc, false);
    }
    data.groups.insert(
        "Shared".into(),
        ModGroup {
            id: "shared-definition".into(),
            description: None,
            tint_color: None,
            mods: shared,
        },
    );
    let profile = data.profiles.get_mut("big").unwrap();
    // Duplicate an enabled entry to exercise the cached duplicate warnings.
    profile.mods.push(ModOrGroup::Individual(member(
        "duplicate",
        "synthetic://profile/1.pak",
        true,
    )));
    for (id, enabled, member_overrides) in [
        ("first", true, MemberOverrides::default()),
        ("partial", true, partial),
        ("off", false, MemberOverrides::default()),
        ("second", true, MemberOverrides::default()),
    ] {
        profile.mods.push(ModOrGroup::Group {
            id: id.into(),
            group_name: "Shared".into(),
            enabled,
            member_overrides,
        });
    }

    let members = enabled_members(&data, "big");
    // 4,800 enabled standalone + duplicate + 80 + 60 + 0 + 80.
    assert_eq!(members.len(), 5021);
    for (scope, expected) in [("first", 80), ("partial", 60), ("off", 0), ("second", 80)] {
        assert_eq!(
            members.iter().filter(|m| m.group_entry_id == scope).count(),
            expected,
            "wrong enabled-member count for {scope}"
        );
    }
    assert_eq!(
        members
            .iter()
            .filter(|m| m.group_entry_id == "partial" && m.mod_id == "shared-0")
            .count(),
        0
    );
    assert_eq!(
        members
            .iter()
            .filter(|m| m.group_entry_id == "first" && m.mod_id == "shared-0")
            .count(),
        1
    );

    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
    let issues = ProfileIssues::collect(&data, "big", &store);
    assert!(issues.duplicate(None, 1));
    assert!(issues.duplicate(None, 6000));
    assert!(issues.missing_dependencies(None, "duplicate").is_empty());
    assert!(issues.pending.is_empty());
    assert!(issues.versions.is_empty());

    // Disabling one group entry must remove only that instance's members.
    for item in &mut data.profiles.get_mut("big").unwrap().mods {
        if let ModOrGroup::Group { id, enabled, .. } = item
            && id == "second"
        {
            *enabled = false;
        }
    }
    let changed = enabled_members(&data, "big");
    assert_eq!(changed.len(), 5021 - 80);
    assert_eq!(
        changed
            .iter()
            .filter(|m| m.group_entry_id == "first")
            .count(),
        80
    );
    assert!(!changed.iter().any(|m| m.group_entry_id == "second"));
}

#[test]
fn large_profile_12000_mods_cached_warning_lookup_is_scoped() {
    // This checks that warning entries with the same member ID do not bleed
    // between independent group instances, even in a large populated cache.
    let mut issues = ProfileIssues::default();
    for i in 0..12_000 {
        let scope = if i % 2 == 0 { "first" } else { "second" };
        issues
            .missing_dependencies
            .entry(scope.into())
            .or_default()
            .insert(
                format!("member-{i}"),
                vec![spec(format!("synthetic://missing/{i}"))],
            );
    }
    for i in (0..12_000).step_by(127) {
        let scope = if i % 2 == 0 { "first" } else { "second" };
        let other = if scope == "first" { "second" } else { "first" };
        let id = format!("member-{i}");
        assert_eq!(issues.missing_dependencies(Some(scope), &id).len(), 1);
        assert!(issues.missing_dependencies(Some(other), &id).is_empty());
        assert!(issues.missing_dependencies(None, &id).is_empty());
    }
}

#[test]
#[ignore = "optional manual scaling sample; run with --ignored --nocapture"]
fn large_profile_scaling_benchmark() {
    // No timing assertions: CI and developers' machines vary substantially.
    // This is a quick, repeatable observation of full collection + lookup work.
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
    for count in [1_000usize, 5_000, 20_000] {
        let data = standalone_data(count);
        let start = Instant::now();
        let issues = ProfileIssues::collect(&data, "big", &store);
        let collect_elapsed = start.elapsed();
        assert!(issues.versions.is_empty());
        let members = enabled_members(&data, "big");
        assert_eq!(members.len(), count * 4 / 5);
        let start = Instant::now();
        let index = DependencyIndex::from_specs(members.iter().map(|m| &m.spec));
        let build_elapsed = start.elapsed();
        let start = Instant::now();
        let mut found = 0;
        for i in 0..10_000 {
            let url = format!("synthetic://profile/{}.pak", (i * 7) % count);
            if index.contains(&spec(url)) {
                found += 1;
            }
        }
        let lookup_elapsed = start.elapsed();
        assert!(found > 0);
        eprintln!(
            "{count:>6} profile mods: collect {collect_elapsed:?}, index {build_elapsed:?}, 10000 lookups {lookup_elapsed:?}"
        );
    }
}
