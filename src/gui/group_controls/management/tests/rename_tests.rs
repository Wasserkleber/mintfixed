use super::*;

fn fixture() -> Data {
    serde_json::from_value(serde_json::json!({
        "active_profile": "A",
        "profiles": {
            "A": {"mods": [
                {"spec": {"url": "standalone"}, "required": false, "enabled": true},
                {"group_name": "Tools", "enabled": true, "member_overrides": {"m1": false}}
            ]},
            "B": {"mods": [
                {"group_name": "Tools", "enabled": false},
                {"group_name": "Tools", "enabled": true, "member_overrides": {"m1": true}}
            ]}
        },
        "groups": {
            "Tools": {"mods": [{"spec": {"url": "https://mod.io/g/drg/m/test#123/10"},
                "required": true, "enabled": true, "priority": 7, "member_id": "m1"}]},
            "Other": {"mods": []}
        }
    }))
    .unwrap()
}

#[test]
fn rename_updates_all_references_without_changing_members_order_or_switches() {
    let mut data = fixture();
    let before = serde_json::to_value(&data).unwrap();
    assert!(rename_group(&mut data, "Tools", "  Utilities  ").unwrap());
    assert!(!data.groups.contains_key("Tools"));
    assert!(data.groups.contains_key("Utilities"));
    for profile in data.profiles.values() {
        for item in &profile.mods {
            if let ModOrGroup::Group { group_name, .. } = item {
                assert_eq!(group_name, "Utilities");
            }
        }
    }
    // Verify a round trip through disk representation, including all overrides.
    let mut restored: Data = serde_json::from_value(serde_json::to_value(&data).unwrap()).unwrap();
    rename_group(&mut restored, "Utilities", "Tools").unwrap();
    assert_eq!(serde_json::to_value(&restored).unwrap(), before);
}

#[test]
fn duplicate_group_keeps_mods_and_switches_but_not_other_profile_references() {
    let mut data = fixture();
    let source = serde_json::to_value(&data.groups["Tools"]).unwrap();
    let other_profile = serde_json::to_value(&data.profiles["B"]).unwrap();
    assert_eq!(
        duplicate_group(&mut data, "Tools", "  Tools Backup  ").unwrap(),
        "Tools Backup"
    );
    let copy = &data.groups["Tools Backup"];
    let original = &data.groups["Tools"];
    assert_ne!(copy.id, original.id);
    assert_eq!(copy.mods.len(), original.mods.len());
    for (a, b) in copy.mods.iter().zip(&original.mods) {
        assert_ne!(a.id, b.id);
        if b.member_id.is_some() {
            assert_ne!(a.member_id, b.member_id);
        }
        assert_eq!(
            (&a.spec, a.enabled, a.required, a.priority),
            (&b.spec, b.enabled, b.required, b.priority)
        );
    }
    assert_eq!(serde_json::to_value(&data.groups["Tools"]).unwrap(), source);
    assert_eq!(
        serde_json::to_value(&data.profiles["B"]).unwrap(),
        other_profile
    );
    assert!(matches!(&data.profiles["A"].mods[0], ModOrGroup::Group {
        group_name, enabled: true, member_overrides, .. } if group_name == "Tools Backup" && !member_overrides.enabled(&data.groups["Tools Backup"].mods[0])));
    data.groups.get_mut("Tools Backup").unwrap().mods[0].priority = 99;
    assert_eq!(data.groups["Tools"].mods[0].priority, 7);
}

#[test]
fn duplicate_group_rejects_invalid_names_without_changes() {
    let mut data = fixture();
    let before = serde_json::to_value(&data).unwrap();
    for (source, name) in [
        ("Missing", "New"),
        ("Tools", "  "),
        ("Tools", "Other"),
        ("Tools", "Tools"),
    ] {
        assert!(duplicate_group(&mut data, source, name).is_err());
    }
    assert_eq!(serde_json::to_value(&data).unwrap(), before);
}

#[test]
fn delete_shared_group_removes_references_from_all_profiles() {
    let mut data = fixture();
    delete_group(&mut data, "Tools").unwrap();
    assert!(!data.groups.contains_key("Tools"));
    assert!(data.groups.contains_key("Other"));
    assert_eq!(data.profiles["A"].mods.len(), 1);
    assert!(matches!(
        data.profiles["A"].mods[0],
        ModOrGroup::Individual(_)
    ));
    assert!(data.profiles["B"].mods.is_empty());
}

#[test]
fn delete_missing_group_does_not_change_data() {
    let mut data = fixture();
    let before = serde_json::to_value(&data).unwrap();
    assert!(delete_group(&mut data, "Missing").is_err());
    assert_eq!(serde_json::to_value(&data).unwrap(), before);
}

#[test]
fn invalid_and_unchanged_names_leave_data_intact() {
    let mut data = fixture();
    let before = serde_json::to_value(&data).unwrap();
    assert!(rename_group(&mut data, "Tools", "  ").is_err());
    assert!(rename_group(&mut data, "Tools", "Other").is_err());
    assert!(rename_group(&mut data, "Missing", "New").is_err());
    assert!(!rename_group(&mut data, "Tools", " Tools ").unwrap());
    assert_eq!(serde_json::to_value(&data).unwrap(), before);
}

#[test]
fn removing_shared_group_member_clears_overrides_in_every_profile() {
    let mut data = fixture();
    let member_id = data.groups["Tools"].mods[0].id.clone();
    let other_before = serde_json::to_value(&data.groups["Other"]).unwrap();
    remove_group_member(&mut data, "Tools", &member_id).unwrap();
    assert!(data.groups["Tools"].mods.is_empty());
    assert_eq!(
        serde_json::to_value(&data.groups["Other"]).unwrap(),
        other_before
    );
    for entry in data.profiles.values().flat_map(|profile| &profile.mods) {
        if let ModOrGroup::Group {
            group_name,
            member_overrides,
            ..
        } = entry
            && group_name == "Tools"
        {
            assert_eq!(
                serde_json::to_value(member_overrides).unwrap(),
                serde_json::to_value(crate::state::MemberOverrides::default()).unwrap()
            );
        }
    }
}

#[test]
fn removing_missing_group_member_keeps_data_unchanged() {
    let mut data = fixture();
    let before = serde_json::to_value(&data).unwrap();
    assert!(remove_group_member(&mut data, "Tools", "no-such-id").is_err());
    assert!(remove_group_member(&mut data, "Missing", "no-such-id").is_err());
    assert_eq!(serde_json::to_value(&data).unwrap(), before);
}
