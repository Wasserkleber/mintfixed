use super::*;

fn data(source_enabled: bool, destination_enabled: bool) -> Data {
    serde_json::from_value(serde_json::json!({
        "active_profile": "A",
        "profiles": {
            "A": {"mods": [
                {"group_name": "Source", "enabled": source_enabled},
                {"group_name": "Destination", "enabled": destination_enabled}
            ]},
            "B": {"mods": [
                {"group_name": "Source", "enabled": true, "member_overrides": {"m": false}},
                {"group_name": "Destination", "enabled": true, "member_overrides": {"other": false}}
            ]}
        },
        "groups": {
            "Source": {"mods": [{"spec": {"url": "moving"}, "required": false,
                "enabled": true, "member_id": "m", "priority": 7}]},
            "Destination": {"mods": [{"spec": {"url": "staying"}, "required": false,
                "enabled": true, "member_id": "other"}]}
        }
    })).unwrap()
}

fn drag(data: &Data) -> Drag {
    Drag {
        profile: "A".into(),
        group: Some("Source".into()),
        index: 0,
        key: data.groups["Source"].mods[0].id.clone(),
        is_group: false,
    }
}

fn enabled(data: &Data, profile: &str, url: &str) -> bool {
    let mut found = false;
    data.for_each_enabled_mod(profile, |mc| {
        if mc.spec.url == url {
            found = true;
        }
    });
    found
}

#[test]
fn moving_out_of_disabled_parent_stays_off_and_undo_restores_every_profile() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &Default::default()).unwrap();
    let mut data = data(false, true);
    let before = data.clone();
    drag(&data)
        .apply(
            &mut data,
            "A",
            Target {
                group: None,
                slot: 0,
            },
            &store,
        )
        .unwrap();
    assert!(!enabled(&data, "A", "moving"));
    let ModOrGroup::Individual(mc) = &data.profiles["A"].mods[0] else {
        panic!()
    };
    assert_eq!(mc.priority, 7);
    assert_eq!(mc.id, before.groups["Source"].mods[0].id);
    assert!(!mc.enabled);
    let mut history = crate::gui::edit_history::ModEditHistory::new(&before, 0);
    history.sync(&data, 1);
    let mut wrapped = crate::state::config::ConfigWrapper::memory(
        crate::state::VersionAnnotatedModData::V0_3_0(data),
    );
    history.restore(&mut wrapped, false).unwrap();
    assert_eq!(
        serde_json::to_value(&**wrapped).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}

#[test]
fn destination_profiles_inherit_default_instead_of_source_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &Default::default()).unwrap();
    let mut data = data(false, true);
    drag(&data)
        .apply(
            &mut data,
            "A",
            Target {
                group: Some("Destination".into()),
                slot: 1,
            },
            &store,
        )
        .unwrap();
    assert!(!enabled(&data, "A", "moving"));
    assert!(enabled(&data, "B", "moving"));
    assert!(!enabled(&data, "B", "staying"));
    let reloaded: Data = serde_json::from_value(serde_json::to_value(&data).unwrap()).unwrap();
    assert!(!enabled(&reloaded, "A", "moving"));
    assert!(enabled(&reloaded, "B", "moving"));
}

#[test]
fn enabled_move_to_disabled_destination_is_rejected_without_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &Default::default()).unwrap();
    let mut data = data(true, false);
    let before = serde_json::to_value(&data).unwrap();
    assert!(
        drag(&data)
            .apply(
                &mut data,
                "A",
                Target {
                    group: Some("Destination".into()),
                    slot: 0
                },
                &store
            )
            .is_err()
    );
    assert_eq!(serde_json::to_value(data).unwrap(), before);
}
#[test]
fn group_destination_survives_rename_but_rejects_replacement_reference() {
    let mut data = data(false, true);
    let target = GroupDestination::capture(&data, "A", "Source").unwrap();
    rename_group(&mut data, "Source", "Renamed").unwrap();
    let profile = data.profiles.remove("A").unwrap();
    data.profiles.insert("Renamed Profile".into(), profile);
    assert_eq!(
        target.resolve(&data),
        Some(("Renamed Profile".into(), "Renamed".into()))
    );
    if let ModOrGroup::Group { id, .. } =
        &mut data.profiles.get_mut("Renamed Profile").unwrap().mods[0]
    {
        *id = crate::state::new_id();
    }
    assert!(target.resolve(&data).is_none());
}

#[test]
fn replacing_dragged_mod_with_same_url_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &Default::default()).unwrap();
    let mut data = data(false, true);
    let drag = drag(&data);
    let replacement = data.groups["Source"].mods[0].duplicate();
    data.groups.get_mut("Source").unwrap().mods[0] = replacement;
    let before = serde_json::to_value(&data).unwrap();
    assert!(
        drag.apply(
            &mut data,
            "A",
            Target {
                group: None,
                slot: 0
            },
            &store
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(&data).unwrap(), before);
}
