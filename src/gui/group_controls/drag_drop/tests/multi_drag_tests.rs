use super::*;

fn sample() -> Data {
    serde_json::from_value(serde_json::json!({
        "active_profile": "A",
        "profiles": {"A": {"mods": [
            {"spec": {"url": "a"}, "required": false, "enabled": true},
            {"spec": {"url": "b"}, "required": false, "enabled": true},
            {"spec": {"url": "c"}, "required": false, "enabled": true},
            {"group_name": "Dest", "enabled": true},
            {"spec": {"url": "d"}, "required": false, "enabled": true}
        ]}},
        "groups": {"Dest": {"mods": [
            {"spec": {"url": "already"}, "required": false, "enabled": true}
        ]}}
    })).unwrap()
}

fn selected_ids(data: &Data) -> Vec<String> {
    [0, 2].iter().map(|&index| match &data.profiles["A"].mods[index] {
        ModOrGroup::Individual(mc) => mc.id.clone(),
        _ => unreachable!(),
    }).collect()
}

#[test]
fn noncontiguous_mods_move_into_group_in_original_order() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &Default::default()).unwrap();
    let mut data = sample();
    let ids = selected_ids(&data);
    let source = Drag { profile: "A".into(), group: None, index: 0,
        key: ids[0].clone(), is_group: false };
    assert_eq!(source.apply_batch(&mut data, "A",
        Target { group: Some("Dest".into()), slot: 0 }, &store,
        &ids.into_iter().rev().collect::<Vec<_>>()).unwrap(), (2, 0));
    let urls = data.groups["Dest"].mods.iter().map(|mc| mc.spec.url.as_str()).collect::<Vec<_>>();
    assert_eq!(urls, vec!["a", "c", "already"]);
}

#[test]
fn same_list_reorder_keeps_relative_selection_order() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &Default::default()).unwrap();
    let mut data = sample();
    let ids = selected_ids(&data);
    let source = Drag { profile: "A".into(), group: None, index: 0,
        key: ids[0].clone(), is_group: false };
    source.apply_batch(&mut data, "A", Target { group: None, slot: 5 }, &store, &ids).unwrap();
    let urls = data.profiles["A"].mods.iter().filter_map(|item| match item {
        ModOrGroup::Individual(mc) => Some(mc.spec.url.as_str()),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(urls, vec!["b", "d", "a", "c"]);
}

#[test]
fn existing_destination_duplicate_is_skipped_but_other_mod_moves() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &Default::default()).unwrap();
    let mut data = sample();
    data.groups.get_mut("Dest").unwrap().mods[0].spec.url = "a".into();
    let ids = selected_ids(&data);
    let source = Drag { profile: "A".into(), group: None, index: 0,
        key: ids[0].clone(), is_group: false };
    assert_eq!(source.apply_batch(&mut data, "A", Target {
        group: Some("Dest".into()), slot: 1 }, &store, &ids).unwrap(), (1, 1));
    assert!(data.profiles["A"].mods.iter().any(|item| {
        matches!(item, ModOrGroup::Individual(mc) if mc.spec.url == "a")
    }));
    assert_eq!(data.groups["Dest"].mods.iter().map(|mc| mc.spec.url.as_str()).collect::<Vec<_>>(),
               vec!["a", "c"]);
}

#[test]
fn whole_batch_restores_with_one_undo() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &Default::default()).unwrap();
    let mut data = sample();
    let before = data.clone();
    let ids = selected_ids(&data);
    let source = Drag { profile: "A".into(), group: None, index: 0,
        key: ids[0].clone(), is_group: false };
    source.apply_batch(&mut data, "A", Target {
        group: Some("Dest".into()), slot: 0 }, &store, &ids).unwrap();
    let mut history = crate::gui::edit_history::ModEditHistory::new(&before, 0);
    history.sync(&data, 1);
    let mut wrapped = crate::state::config::ConfigWrapper::memory(
        crate::state::VersionAnnotatedModData::V0_3_0(data));
    history.restore(&mut wrapped, false).unwrap();
    assert_eq!(serde_json::to_value(&**wrapped).unwrap(),
               serde_json::to_value(&before).unwrap());
}

#[test]
fn rejected_batch_leaves_all_lists_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModStore::new(dir.path(), &Default::default()).unwrap();
    let mut data = sample();
    if let ModOrGroup::Group { enabled, .. } = &mut data.profiles.get_mut("A").unwrap().mods[3] {
        *enabled = false;
    }
    let before = serde_json::to_value(&data).unwrap();
    let ids = selected_ids(&data);
    let source = Drag { profile: "A".into(), group: None, index: 0,
        key: ids[0].clone(), is_group: false };
    assert!(source.apply_batch(&mut data, "A", Target {
        group: Some("Dest".into()), slot: 0 }, &store, &ids).is_err());
    assert_eq!(serde_json::to_value(&data).unwrap(), before);
}
