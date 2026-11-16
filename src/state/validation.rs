use super::*;
use serde_json::Value;
use std::collections::HashSet;

pub(super) fn invalid(reason: impl Into<String>) -> StateError {
    StateError::InvalidModData {
        reason: reason.into(),
    }
}

fn check_id(value: &Value, seen: &mut HashSet<String>) -> Result<(), StateError> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing stable ID"))?;
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("malformed stable ID"));
    }
    if !seen.insert(id.to_ascii_lowercase()) {
        return Err(invalid("duplicate stable ID"));
    }
    Ok(())
}

pub(super) fn validate_ids(value: &Value) -> Result<(), StateError> {
    let mut seen = HashSet::new();
    for collection in ["profiles", "groups"] {
        let entries = value
            .get(collection)
            .and_then(Value::as_object)
            .ok_or_else(|| invalid(format!("missing {collection}")))?;
        for entry in entries.values() {
            check_id(entry, &mut seen)?;
            for row in entry
                .get("mods")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("missing ordered mod list"))?
            {
                check_id(row, &mut seen)?;
            }
        }
    }
    if let Some(scripts) = value.get("bitfix_scripts") {
        let scripts = scripts
            .as_array()
            .ok_or_else(|| invalid("invalid global Bitfix scripts"))?;
        for script in scripts {
            check_id(script, &mut seen)?;
        }
    }
    Ok(())
}

pub(super) fn validate_links(data: &ModData_v0_1_0) -> Result<(), StateError> {
    if !data.profiles.contains_key(&data.active_profile) {
        return Err(invalid("active profile does not exist"));
    }
    for profile in data.profiles.values() {
        for row in &profile.mods {
            if let ModOrGroup::Group { group_name, .. } = row
                && !data.groups.contains_key(group_name)
            {
                return Err(invalid(format!("missing shared group: {group_name}")));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod mod_data_tests {
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
}

#[cfg(test)]
mod effective_selection_tests {
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
        let mut shared = ModGroup::default();
        shared.mods = vec![item("skip.pak", 99, true), item("enable.pak", 4, false)];
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
        let mut shared = ModGroup::default();
        shared.mods = vec![item("shared.pak", 1, true)];
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
}
