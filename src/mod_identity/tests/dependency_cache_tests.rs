use super::*;
use crate::state::{MemberOverrides, ModConfig, ModGroup, ModProfile_v0_1_0};

fn spec(url: &str) -> ModSpecification {
    ModSpecification {
        url: url.to_owned(),
    }
}

fn member(id: &str, url: &str, enabled: bool) -> ModConfig {
    ModConfig {
        id: id.to_owned(),
        description: None,
        spec: spec(url),
        required: false,
        member_id: None,
        enabled,
        priority: 0,
    }
}

#[test]
fn indexed_matching_preserves_pairwise_dependency_semantics() {
    let installed = [
        spec("https://mod.io/g/drg/m/renamed#123/10"),
        spec("https://mod.io/g/drg/m/other#124/2"),
        spec("https://mod.io/g/drg/m/latest"),
        spec("https://example.test/foo.pak"),
        spec("https://mod.io/g/drg/m/bad#123/not-a-file"),
    ];
    let index = DependencyIndex::from_specs(installed.iter());
    for required in [
        "https://mod.io/g/drg/m/old#123/11",
        "https://mod.io/g/drg/m/renamed#999/11",
        "https://mod.io/g/drg/m/renamed",
        "https://mod.io/g/drg/m/other#124",
        "https://mod.io/g/drg/m/other#125",
        "https://mod.io/g/drg/m/latest#345/6",
        "https://mod.io/g/drg/m/latest-plus",
        "https://example.test/foo.pak",
        "https://example.test/foo.pak2",
        "https://mod.io/g/drg/m/bad#123/not-a-file",
        "https://mod.io/g/drg/m/bad#123",
    ] {
        let required = spec(required);
        assert_eq!(
            index.contains(&required),
            installed
                .iter()
                .any(|candidate| candidate.satisfies_dependency(&required)),
            "indexed match differs for {}",
            required.url
        );
    }
}

#[test]
fn indexed_modhub_replacements_match_pairwise_and_are_forward_only() {
    let required = spec("https://mod.io/g/drg/m/mod-hub");
    let required_pin = spec("https://mod.io/g/drg/m/mod-hub#567/891");
    for installed in [
        spec("https://mod.io/g/drg/m/unified-ui"),
        spec("https://mod.io/g/drg/m/unified-ui#123/10"),
        spec("https://mod.io/g/drg/m/test"),
        spec("https://mod.io/g/drg/m/mod-hub"),
        spec("https://mod.io/g/drg/m/unrelated"),
    ] {
        let index = DependencyIndex::from_specs(std::iter::once(&installed));
        for dep in [&required, &required_pin] {
            assert_eq!(index.contains(dep), installed.satisfies_dependency(dep));
        }
    }
    let source_index = DependencyIndex::from_specs(std::iter::once(&required));
    let replacement = spec("https://mod.io/g/drg/m/unified-ui");
    assert_eq!(
        source_index.contains(&replacement),
        required.satisfies_dependency(&replacement),
    );
}

#[test]
fn effective_enablement_and_shared_group_instances_are_separate() {
    let mut shared = member("m", "https://mod.io/g/drg/m/required", true);
    let mut overrides = MemberOverrides::default();
    overrides.set(&mut shared, false);
    let data = Data {
        active_profile: "p".into(),
        bitfix_scripts: Vec::new(),
        groups: BTreeMap::from([(
            "Shared".into(),
            ModGroup {
                id: "definition".into(),
                description: None,
                tint_color: None,
                mods: vec![shared],
            },
        )]),
        profiles: BTreeMap::from([(
            "p".into(),
            ModProfile_v0_1_0 {
                id: "profile".into(),
                mods: vec![
                    ModOrGroup::Individual(member("disabled", "https://example.test/a", false)),
                    ModOrGroup::Group {
                        id: "parent-disabled".into(),
                        group_name: "Shared".into(),
                        enabled: false,
                        member_overrides: MemberOverrides::default(),
                    },
                    ModOrGroup::Group {
                        id: "member-disabled".into(),
                        group_name: "Shared".into(),
                        enabled: true,
                        member_overrides: overrides,
                    },
                    ModOrGroup::Group {
                        id: "active-instance".into(),
                        group_name: "Shared".into(),
                        enabled: true,
                        member_overrides: MemberOverrides::default(),
                    },
                ],
            },
        )]),
    };
    let members = enabled_members(&data, "p");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].group_entry_id, "active-instance");
    assert_eq!(members[0].mod_id, "m");
    let mut issues = ProfileIssues::default();
    issues
        .missing_dependencies
        .entry("active-instance".into())
        .or_default()
        .insert("m".into(), vec![spec("missing")]);
    assert_eq!(
        issues
            .missing_dependencies(Some("active-instance"), "m")
            .len(),
        1
    );
    assert!(
        issues
            .missing_dependencies(Some("member-disabled"), "m")
            .is_empty()
    );
    assert!(
        issues
            .missing_dependencies(Some("parent-disabled"), "m")
            .is_empty()
    );
    assert!(issues.missing_dependencies(None, "m").is_empty());
}
