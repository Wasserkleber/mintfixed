use super::*;

fn test_mod(url: &str, priority: i32, enabled: bool) -> ModConfig {
    ModConfig {
        id: new_id(),
        spec: ModSpecification::new(url.to_owned()),
        description: None,
        member_id: None,
        required: false,
        enabled,
        priority,
    }
}

#[test]
fn missing_profile_is_safe_and_installation_fails_closed() {
    let mut data = ModData_v0_1_0::default();
    data.bitfix_scripts.push(test_mod("global.lua", 15, true));
    assert!(matches!(
        data.try_ordered_install_entries("missing"),
        Err(SelectionError::MissingProfile { .. })
    ));
    assert!(data.ordered_install_entries("missing").is_empty());
    let mut seen = 0;
    data.for_each_enabled_mod("missing", |_| seen += 1);
    data.for_each_mod("missing", |_| seen += 1);
    data.for_each_mod_mut("missing", |_| seen += 1);
    assert_eq!(seen, 0);
    assert!(!data.any_mod("missing", |_, _| true));
    assert!(!data.any_mod_mut("missing", |_, _| true));
    assert!(!data.enable_dependency("missing", &ModSpecification::new("x".into())));
}

#[test]
fn dangling_group_never_builds_partial_installation() {
    let mut data = ModData_v0_1_0::default();
    data.bitfix_scripts.push(test_mod("global.lua", 15, true));
    data.profiles.get_mut("default").unwrap().mods = vec![
        ModOrGroup::Individual(test_mod("good.pak", 10, true)),
        ModOrGroup::Group {
            id: new_id(),
            group_name: "deleted".into(),
            enabled: true,
            member_overrides: MemberOverrides::default(),
        },
    ];
    assert!(matches!(data.try_ordered_install_entries("default"),
        Err(SelectionError::MissingGroup { group_name, .. }) if group_name == "deleted"
    ));
    assert!(data.ordered_install_entries("default").is_empty());
    let mut seen = Vec::new();
    data.for_each_enabled_mod("default", |mc| seen.push(mc.spec.url.clone()));
    assert_eq!(seen, vec!["good.pak".to_owned()]);
    assert!(data.any_mod("default", |mc, _| mc.spec.url == "good.pak"));
    assert!(!data.any_mod("default", |mc, _| mc.spec.url == "missing.pak"));
    assert!(!data.enable_dependency("default", &ModSpecification::new("absent.pak".into())));
    let mut mutations = 0;
    data.for_each_mod_mut("default", |_| mutations += 1);
    assert_eq!(mutations, 1);
}

#[test]
fn cli_and_gui_selection_consistent_with_overrides_bitfix_and_stable_priority() {
    let mut data = ModData_v0_1_0::default();
    let mut shared = ModGroup {
        mods: vec![
            test_mod("overridden-off.pak", 99, true),
            test_mod("overridden-on.pak", 8, false),
            test_mod("default-on.pak", 8, true),
        ],
        ..Default::default()
    };
    let mut overrides = MemberOverrides::default();
    overrides.set(&mut shared.mods[0], false);
    overrides.set(&mut shared.mods[1], true);
    data.groups.insert("shared".into(), shared);
    data.groups.insert(
        "disabled".into(),
        ModGroup {
            mods: vec![test_mod("disabled-group.pak", 500, true)],
            ..Default::default()
        },
    );
    data.profiles.get_mut("default").unwrap().mods = vec![
        ModOrGroup::Individual(test_mod("solo-first.pak", 8, true)),
        ModOrGroup::Group {
            id: new_id(),
            group_name: "shared".into(),
            enabled: true,
            member_overrides: overrides,
        },
        ModOrGroup::Individual(test_mod("solo-last.pak", 8, true)),
        ModOrGroup::Group {
            id: new_id(),
            group_name: "disabled".into(),
            enabled: false,
            member_overrides: MemberOverrides::default(),
        },
        ModOrGroup::Individual(test_mod("legacy.lua", 1000, true)),
        ModOrGroup::Individual(test_mod("disabled.pak", 1000, false)),
    ];
    data.bitfix_scripts = vec![
        test_mod("global.lua", 30, true),
        test_mod("off-global.lua", 999, false),
    ];

    // Both GUI and CLI call ordered_install_entries; the CLI now also checks the fallible API.
    let cli = data.try_ordered_install_entries("default").unwrap();
    let gui = data.ordered_install_entries("default");
    let cli_manifest: Vec<_> = cli
        .iter()
        .map(|e| (e.spec.url.as_str(), e.location.as_str()))
        .collect();
    let gui_manifest: Vec<_> = gui
        .iter()
        .map(|e| (e.spec.url.as_str(), e.location.as_str()))
        .collect();
    assert_eq!(cli_manifest, gui_manifest);
    let urls: Vec<_> = cli.iter().map(|e| e.spec.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "global.lua",
            "solo-first.pak",
            "overridden-on.pak",
            "default-on.pak",
            "solo-last.pak"
        ]
    );
    assert_eq!(
        cli[2].location,
        r#"profile "default", group "shared" (row 2), mod #2"#
    );
    let specs = data.ordered_install_specs("default");
    let spec_urls: Vec<_> = specs.iter().map(|s| s.url.as_str()).collect();
    assert_eq!(spec_urls, urls);
    let mut enabled = Vec::new();
    data.for_each_enabled_mod("default", |mc| enabled.push(mc.spec.url.clone()));
    assert_eq!(
        enabled.iter().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "solo-first.pak",
            "overridden-on.pak",
            "default-on.pak",
            "solo-last.pak",
            "legacy.lua"
        ]
    );
}

#[test]
fn last_profile_cannot_be_removed() {
    let mut data = ModData_v0_1_0::default();
    data.remove_active_profile();
    assert_eq!(data.active_profile, "default");
    assert!(data.try_get_active_profile().is_some());
}
