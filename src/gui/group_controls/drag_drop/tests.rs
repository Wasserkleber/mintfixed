#[cfg(test)]
mod drag_polish_tests {
    use super::*;

    #[test]
    fn edge_scroll_reaches_2400_pixels_per_second_near_edges() {
        let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(200.0, 400.0));
        let dt = 1.0 / 60.0;
        let upward = edge_scroll_delta(rect, egui::pos2(100.0, 1.0), dt);
        let downward = edge_scroll_delta(rect, egui::pos2(100.0, 399.0), dt);
        let expected = 2_400.0 * dt * (79.0_f32 / 80.0).powi(2);
        assert!((upward - expected).abs() < 0.001);
        assert!((downward + expected).abs() < 0.001);
    }

    #[test]
    fn drag_preview_stays_with_pointer_when_scrolling_down() {
        let press = egui::pos2(100.0, 150.0);
        let pointer = egui::pos2(105.0, 220.0);
        let initial_row = egui::pos2(30.0, 130.0);
        let scrolled_row = egui::pos2(30.0, -70.0);
        let translation = drag_paint_translation(pointer, press, initial_row, scrolled_row);
        assert_eq!(scrolled_row + translation, egui::pos2(35.0, 200.0));
    }

    #[test]
    fn drag_preview_stays_with_pointer_when_scrolling_up() {
        let press = egui::pos2(100.0, 150.0);
        let pointer = egui::pos2(105.0, 80.0);
        let initial_row = egui::pos2(30.0, 130.0);
        let scrolled_row = egui::pos2(30.0, 330.0);
        let translation = drag_paint_translation(pointer, press, initial_row, scrolled_row);
        assert_eq!(scrolled_row + translation, egui::pos2(35.0, 60.0));
    }

    #[test]
    fn drag_preview_does_not_drift_when_pointer_is_stationary() {
        let press = egui::pos2(100.0, 150.0);
        let initial_row = egui::pos2(30.0, 130.0);
        for y in [-170.0, 80.0, 130.0, 270.0] {
            let current_row = egui::pos2(30.0, y);
            let delta = drag_paint_translation(press, press, initial_row, current_row);
            assert_eq!(current_row + delta, initial_row);
        }
    }

    #[test]
    fn hover_requires_dwell_and_resets_after_leaving_or_missing_frames() {
        let mut timer = HoverOpen::default();
        assert!(!timer.update(1.0, true));
        for i in 1..5 {
            assert!(!timer.update(1.0 + i as f64 * 0.1, true));
        }
        assert!(timer.update(1.51, true));
        assert!(!timer.update(1.52, false));
        assert!(!timer.update(1.53, true));
        assert!(!timer.update(5.0, true));
        assert!(!timer.update(5.1, true));
    }

    #[test]
    fn edge_scroll_continues_past_top_and_bottom_but_not_left_or_right() {
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(210.0, 420.0));
        let dt = 1.0 / 60.0;
        assert_eq!(edge_scroll_delta(rect, egui::pos2(100.0, -20.0), dt), 2_400.0 * dt);
        assert_eq!(edge_scroll_delta(rect, egui::pos2(100.0, 460.0), dt), -2_400.0 * dt);
        assert_eq!(edge_scroll_delta(rect, egui::pos2(9.0, -20.0), dt), 0.0);
        assert_eq!(edge_scroll_delta(rect, egui::pos2(211.0, 460.0), dt), 0.0);
    }

    #[test]
    fn edge_scroll_has_direction_speed_limit_and_no_motion_outside_view() {
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(210.0, 420.0));
        assert!(edge_scroll_delta(rect, egui::pos2(50.0, 21.0), 0.016) > 0.0);
        assert!(edge_scroll_delta(rect, egui::pos2(50.0, 419.0), 0.016) < 0.0);
        assert_eq!(edge_scroll_delta(rect, rect.center(), 0.016), 0.0);
        assert_eq!(edge_scroll_delta(rect, egui::pos2(9.0, 21.0), 0.016), 0.0);
        assert!(edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 10.0) <= 120.0);
        assert_eq!(
            edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 0.032),
            2.0 * edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 0.016)
        );
    }
}

#[cfg(test)]
mod move_switch_tests {
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
}

#[cfg(test)]
mod multi_drag_tests {
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
}
