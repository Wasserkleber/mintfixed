use super::{
    ModConfig, ModData_v0_1_0 as ModData, ModGroup, ModOrGroup, ModProfile_v0_1_0 as ModProfile,
};
use crate::providers::ModSpecification;

#[test]
fn enabling_legacy_group_dependency_is_profile_local() {
    let mut data: ModData = serde_json::from_value(serde_json::json!({
        "active_profile": "A",
        "profiles": {
            "A": {"mods": [{"group_name": "tools", "enabled": false}]},
            "B": {"mods": [{"group_name": "tools", "enabled": true}]}
        },
        "groups": {"tools": {"mods": [
            {"spec": {"url": "a"}, "required": false, "enabled": false},
            {"spec": {"url": "b"}, "required": false, "enabled": false}
        ]}}
    }))
    .unwrap();

    assert!(data.enable_dependency("A", &ModSpecification::new("a".into())));
    data.groups.get_mut("tools").unwrap().mods.swap(0, 1);
    let data: ModData = serde_json::from_value(serde_json::to_value(data).unwrap()).unwrap();
    for profile in ["A", "B"] {
        let mut mods = Vec::new();
        data.for_each_enabled_mod(profile, |mc| mods.push(mc.spec.url.clone()));
        if profile == "A" {
            assert_eq!(mods, ["a"]);
        } else {
            assert!(mods.is_empty());
        }
    }
}

#[test]
fn test_for_each_mod() {
    let mod_1 = ModConfig {
        id: crate::state::new_id(),
        description: None,
        member_id: None,
        spec: ModSpecification::new("a".to_string()),
        required: false,
        enabled: false,
        priority: 50,
    };

    let mod_2 = ModConfig {
        id: crate::state::new_id(),
        description: None,
        member_id: None,
        spec: ModSpecification::new("b".to_string()),
        required: true,
        enabled: false,
        priority: 50,
    };

    let mod_3 = ModConfig {
        id: crate::state::new_id(),
        description: None,
        member_id: None,
        spec: ModSpecification::new("c".to_string()),
        required: false,
        enabled: true,
        priority: 50,
    };

    let mod_data = ModData {
        bitfix_scripts: Vec::new(),
        active_profile: "default".to_string(),
        profiles: [(
            "default".to_string(),
            ModProfile {
                id: crate::state::new_id(),
                mods: vec![
                    ModOrGroup::Individual(mod_1),
                    ModOrGroup::Group {
                        id: crate::state::new_id(),
                        member_overrides: Default::default(),
                        group_name: "mg1".to_string(),
                        enabled: false,
                    },
                ],
            },
        )]
        .into(),
        groups: [(
            "mg1".to_string(),
            ModGroup {
                tint_color: None,
                id: crate::state::new_id(),
                description: None,
                mods: vec![mod_2, mod_3],
            },
        )]
        .into(),
    };

    let mut counter = 0;
    mod_data.for_each_mod("default", |_| {
        counter += 1;
    });
    assert_eq!(counter, 3);
}

#[test]
fn test_for_each_enabled_mod() {
    let mod_1 = ModConfig {
        id: crate::state::new_id(),
        description: None,
        member_id: None,
        spec: ModSpecification::new("a".to_string()),
        required: false,
        enabled: false,
        priority: 50,
    };

    let mod_2 = ModConfig {
        id: crate::state::new_id(),
        description: None,
        member_id: None,
        spec: ModSpecification::new("b".to_string()),
        required: true,
        enabled: false,
        priority: 50,
    };

    let mod_3 = ModConfig {
        id: crate::state::new_id(),
        description: None,
        member_id: None,
        spec: ModSpecification::new("c".to_string()),
        required: false,
        enabled: true,
        priority: 50,
    };

    let mod_data = ModData {
        bitfix_scripts: Vec::new(),
        active_profile: "default".to_string(),
        profiles: [(
            "default".to_string(),
            ModProfile {
                id: crate::state::new_id(),
                mods: vec![
                    ModOrGroup::Individual(mod_1),
                    ModOrGroup::Group {
                        id: crate::state::new_id(),
                        member_overrides: Default::default(),
                        group_name: "mg1".to_string(),
                        enabled: true,
                    },
                ],
            },
        )]
        .into(),
        groups: [(
            "mg1".to_string(),
            ModGroup {
                tint_color: None,
                id: crate::state::new_id(),
                description: None,
                mods: vec![mod_2, mod_3],
            },
        )]
        .into(),
    };

    let mut counter = 0;
    mod_data.for_each_enabled_mod("default", |_| {
        counter += 1;
    });
    assert_eq!(counter, 1);
}

#[test]
fn test_any_mod() {
    let mod_1 = ModConfig {
        id: crate::state::new_id(),
        description: None,
        member_id: None,
        spec: ModSpecification::new("a".to_string()),
        required: false,
        enabled: false,
        priority: 50,
    };

    let mod_2 = ModConfig {
        id: crate::state::new_id(),
        description: None,
        member_id: None,
        spec: ModSpecification::new("b".to_string()),
        required: true,
        enabled: false,
        priority: 50,
    };

    let mod_3 = ModConfig {
        id: crate::state::new_id(),
        description: None,
        member_id: None,
        spec: ModSpecification::new("c".to_string()),
        required: false,
        enabled: true,
        priority: 50,
    };

    let mod_data = ModData {
        bitfix_scripts: Vec::new(),
        active_profile: "default".to_string(),
        profiles: [(
            "default".to_string(),
            ModProfile {
                id: crate::state::new_id(),
                mods: vec![
                    ModOrGroup::Individual(mod_1),
                    ModOrGroup::Group {
                        id: crate::state::new_id(),
                        member_overrides: Default::default(),
                        group_name: "mg1".to_string(),
                        enabled: true,
                    },
                ],
            },
        )]
        .into(),
        groups: [(
            "mg1".to_string(),
            ModGroup {
                tint_color: None,
                id: crate::state::new_id(),
                description: None,
                mods: vec![mod_2, mod_3],
            },
        )]
        .into(),
    };

    let any_required = mod_data.any_mod("default", |mc, _| mc.required);
    assert!(any_required);
}
#[test]
fn global_bitfix_is_identical_in_every_profile() {
    let mut data = ModData::default();
    data.profiles.insert("other".into(), ModProfile::default());
    data.bitfix_scripts.push(ModConfig {
        id: crate::state::new_id(),
        description: None,
        member_id: None,
        spec: ModSpecification::new("C:\\mods\\test.lua".into()),
        required: false,
        enabled: true,
        priority: 0,
    });
    assert_eq!(
        data.ordered_install_specs("default"),
        data.ordered_install_specs("other")
    );
    assert_eq!(data.ordered_install_specs("other").len(), 1);
    data.bitfix_scripts[0].enabled = false;
    assert!(data.ordered_install_specs("default").is_empty());
    assert!(data.ordered_install_specs("other").is_empty());
}
