use super::*;
use crate::providers::ModSpecification;
use crate::state::ModConfig;

fn mod_config(url: &str, priority: i32) -> ModConfig {
    ModConfig {
        id: crate::state::new_id(),
        description: Some("Original mod description".into()),
        spec: ModSpecification::new(url.into()),
        enabled: true,
        required: true,
        member_id: Some(crate::state::new_id()),
        priority,
    }
}

fn sample() -> String {
    let mut member = mod_config("https://mod.io/g/drg/m/example", 12);
    let mut overrides = MemberOverrides::default();
    overrides.set(&mut member, false);
    let group = ModGroup {
        tint_color: None,
        id: crate::state::new_id(),
        mods: vec![member],
        description: Some("Group description".into()),
    };
    let mut standalone = mod_config("C:\\mods\\custom.pak", 7);
    standalone.priority = 7;
    let export = ProfileExport {
        format: PROFILE_FORMAT.into(),
        version: PROFILE_VERSION,
        profile_name: "Example".into(),
        mods: vec![
            ModOrGroup::Group {
                id: crate::state::new_id(),
                group_name: "Tools".into(),
                enabled: true,
                member_overrides: overrides,
            },
            ModOrGroup::Individual(standalone),
        ],
        groups: BTreeMap::from([("Tools".into(), group)]),
    };
    serde_json::to_string(&export).unwrap()
}

#[test]
fn preserves_order_settings_and_overrides_with_new_ids() {
    let input = sample();
    let old = parse_profile(&input).unwrap();
    let mut data = Data::default();
    let created = import_profile(&mut data, &input, "").unwrap();
    let profile = &data.profiles[&created];
    assert_eq!(data.active_profile, created);
    assert_eq!(profile.mods.len(), 2);
    let group = &data.groups["Tools"];
    assert_eq!(group.description.as_deref(), Some("Group description"));
    assert_ne!(group.id, old.groups["Tools"].id);
    assert_ne!(group.mods[0].id, old.groups["Tools"].mods[0].id);
    assert_eq!(group.mods[0].priority, 12);
    assert!(group.mods[0].required);
    assert_eq!(
        group.mods[0].description.as_deref(),
        Some("Original mod description")
    );
    match &profile.mods[0] {
        ModOrGroup::Group {
            group_name,
            member_overrides,
            ..
        } => {
            assert_eq!(group_name, "Tools");
            assert!(!member_overrides.enabled(&group.mods[0]));
        }
        _ => panic!("first item should be a group"),
    }
    match &profile.mods[1] {
        ModOrGroup::Individual(mc) => {
            assert_eq!(mc.priority, 7);
            assert_ne!(
                mc.id,
                match &old.mods[1] {
                    ModOrGroup::Individual(before) => before.id.clone(),
                    _ => unreachable!(),
                }
            );
        }
        _ => panic!("second item should be a standalone mod"),
    }
}

#[test]
fn repeat_import_requires_new_profile_name_and_preserves_existing_state() {
    let mut data = Data::default();
    let input = sample();
    let created = import_profile(&mut data, &input, "Example").unwrap();
    assert_eq!(created, "Example");
    let old_profile_count = data.profiles.len();
    let old_group_count = data.groups.len();
    let old_active_profile = data.active_profile.clone();
    let old_profile_id = data.profiles["Example"].id.clone();

    let error = import_profile(&mut data, &input, "Example").unwrap_err();
    assert_eq!(error, "A profile with that name already exists.");
    // An empty name also resolves to the exported name, and must be checked.
    assert_eq!(import_profile(&mut data, &input, "").unwrap_err(), error);
    assert_eq!(data.profiles.len(), old_profile_count);
    assert_eq!(data.groups.len(), old_group_count);
    assert_eq!(data.active_profile, old_active_profile);
    assert_eq!(data.profiles["Example"].id, old_profile_id);

    // Explicitly choosing another profile name succeeds; conflicting shared
    // group names still get unique names, preserving the original groups.
    let imported = import_profile(&mut data, &input, "Example (second)").unwrap();
    assert_eq!(imported, "Example (second)");
    assert!(data.groups.contains_key("Tools"));
    assert!(data.groups.contains_key("Tools (imported)"));
    assert!(matches!(&data.profiles["Example (second)"].mods[0],
        ModOrGroup::Group { group_name, .. } if group_name == "Tools (imported)"));
}
#[test]
fn rejects_malformed_json_without_mutating_state() {
    let mut data = Data::default();
    let before_profiles = data.profiles.len();
    let before_groups = data.groups.len();
    assert!(
        import_profile(
            &mut data,
            r#"{"format":"wrong","version":1,"profile_name":"X","mods":[],"groups":{}}"#,
            ""
        )
        .is_err()
    );
    assert!(import_profile(&mut data, r#"{"format":"mintfixed-profile-mod-list","version":1,"profile_name":"X","mods":[{"group_name":"missing","enabled":true}],"groups":{}}"#, "").is_err());
    assert_eq!(data.profiles.len(), before_profiles);
    assert_eq!(data.groups.len(), before_groups);
}
