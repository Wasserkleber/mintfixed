use super::*;
fn item(url: &str, priority: i32, enabled: bool) -> ModConfig {
    ModConfig {
        id: new_id(),
        spec: ModSpecification::new(url.into()),
        description: None,
        member_id: None,
        required: false,
        enabled,
        priority,
    }
}

#[test]
fn group_overrides_priority_and_disabled_groups_match_installation() {
    let mut data = ModData_v0_1_0::default();
    let mut shared = ModGroup {
        mods: vec![item("skip.pak", 99, true), item("enable.pak", 4, false)],
        ..Default::default()
    };
    let mut overrides = MemberOverrides::default();
    overrides.set(&mut shared.mods[0], false);
    overrides.set(&mut shared.mods[1], true);
    data.groups.insert("shared".into(), shared);
    data.groups.insert(
        "disabled".into(),
        ModGroup {
            mods: vec![item("in-disabled-group.pak", 500, true)],
            ..Default::default()
        },
    );
    let profile = data.profiles.get_mut("default").unwrap();
    profile.mods = vec![
        ModOrGroup::Individual(item("first.pak", 4, true)),
        ModOrGroup::Group {
            id: new_id(),
            group_name: "shared".into(),
            enabled: true,
            member_overrides: overrides,
        },
        ModOrGroup::Individual(item("last.pak", 4, true)),
        ModOrGroup::Group {
            id: new_id(),
            group_name: "disabled".into(),
            enabled: false,
            member_overrides: Default::default(),
        },
        ModOrGroup::Individual(item("old.lua", 900, true)),
        ModOrGroup::Individual(item("off.pak", 999, false)),
    ];
    data.bitfix_scripts = vec![item("global.lua", 20, true)];
    let mut enabled = Vec::new();
    data.for_each_enabled_mod("default", |mc| enabled.push(mc.spec.url.clone()));
    assert_eq!(
        enabled,
        vec![
            "first.pak".to_string(),
            "enable.pak".to_string(),
            "last.pak".to_string(),
            "old.lua".to_string()
        ]
    );
    let entries = data.ordered_install_entries("default");
    let urls: Vec<_> = entries
        .iter()
        .map(|entry| entry.spec.url.as_str())
        .collect();
    assert_eq!(
        urls,
        vec!["global.lua", "first.pak", "enable.pak", "last.pak"]
    );
    assert_eq!(
        entries[2].location,
        r#"profile "default", group "shared" (row 2), mod #2"#
    );
}

#[test]
fn overrides_remain_profile_local_for_shared_group() {
    let mut data = ModData_v0_1_0::default();
    let mut shared = ModGroup {
        mods: vec![item("shared.pak", 1, true)],
        ..Default::default()
    };
    let mut disabled_in_a = MemberOverrides::default();
    disabled_in_a.set(&mut shared.mods[0], false);
    data.groups.insert("shared".into(), shared);
    let profile_a = data.profiles.get_mut("default").unwrap();
    profile_a.mods = vec![ModOrGroup::Group {
        id: new_id(),
        group_name: "shared".into(),
        enabled: true,
        member_overrides: disabled_in_a,
    }];
    let mut profile_b = ModProfile_v0_1_0::default();
    profile_b.mods.push(ModOrGroup::Group {
        id: new_id(),
        group_name: "shared".into(),
        enabled: true,
        member_overrides: Default::default(),
    });
    data.profiles.insert("other".into(), profile_b);
    assert!(data.ordered_install_entries("default").is_empty());
    assert_eq!(data.ordered_install_specs("other")[0].url, "shared.pak");
}
