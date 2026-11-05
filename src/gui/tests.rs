use super::*;
use crate::providers::ModResolution;
use crate::state::ModGroup;

fn mod_config(name: &str) -> ModConfig {
    ModConfig {
        description: None,
        member_id: None,
        spec: ModSpecification::new(name.to_owned()),
        enabled: true,
        required: false,
        priority: 0,
    }
}

fn sorting(sort_category: SortBy, is_ascending: bool) -> SortingConfig {
    SortingConfig {
        sort_category,
        is_ascending,
    }
}

#[test]
fn sorting_preserves_group_boundaries_and_manual_order() {
    let mods = [
        mod_config("z"),
        mod_config("a"),
        mod_config("y"),
        mod_config("b"),
    ];
    let entries = [
        None,
        Some(&mods[0]),
        Some(&mods[1]),
        None,
        None,
        Some(&mods[2]),
        Some(&mods[3]),
        None,
    ];
    let before = serde_json::to_value(&mods).unwrap();
    assert_eq!(
        sorted_mod_indices(None, entries, Some(&sorting(SortBy::Name, false)), |_| None),
        [0, 2, 1, 3, 4, 6, 5, 7]
    );
    assert_eq!(
        sorted_mod_indices(None, entries, Some(&sorting(SortBy::Name, true)), |_| None),
        [0, 1, 2, 3, 4, 5, 6, 7]
    );
    assert_eq!(
        sorted_mod_indices(None, entries, None, |_| panic!(
            "Manual order must not query metadata"
        )),
        (0..entries.len()).collect::<Vec<_>>()
    );
    assert_eq!(serde_json::to_value(&mods).unwrap(), before);
}

#[test]
fn all_sort_categories_support_both_directions_and_missing_metadata() {
    let mut alpha = mod_config("alpha");
    alpha.priority = 10;
    let mut zulu = mod_config("zulu");
    zulu.enabled = false;
    zulu.priority = 20;
    let metadata = |spec: &ModSpecification| {
        let first = spec.url == "alpha";
        Some(ModInfo {
            provider: if first { "file" } else { "modio" },
            name: spec.url.clone(),
            spec: spec.clone(),
            versions: vec![],
            resolution: ModResolution::resolvable(spec.url.clone().into()),
            suggested_require: false,
            suggested_dependencies: vec![],
            modio_id: None,
            modio_tags: Some(ModioTags {
                qol: false,
                gameplay: false,
                audio: false,
                visual: false,
                framework: false,
                versions: Default::default(),
                required_status: if first {
                    RequiredStatus::Optional
                } else {
                    RequiredStatus::RequiredByAll
                },
                approval_status: if first {
                    ApprovalStatus::Verified
                } else {
                    ApprovalStatus::Sandbox
                },
            }),
        })
    };
    for category in SortBy::iter() {
        for direction in [false, true] {
            let config = sorting(category, direction);
            let calls = std::cell::Cell::new(0);
            let order = sorted_mod_indices(None, [Some(&zulu), Some(&alpha)], Some(&config), |spec| {
                calls.set(calls.get() + 1);
                metadata(spec)
            });
            assert_eq!(
                order,
                if direction { [0, 1] } else { [1, 0] },
                "{category:?}"
            );
            assert_eq!(calls.get(), 2);
            let order =
                sorted_mod_indices(None, [Some(&zulu), None, Some(&alpha)], Some(&config), |_| None);
            assert_eq!(order, [0, 1, 2]);
            assert!(sorted_mod_indices(None, [], Some(&config), |_| None).is_empty());
            assert_eq!(
                sorted_mod_indices(None, [None, None], Some(&config), |_| None),
                [0, 1]
            );
            assert_eq!(
                sorted_mod_indices(None, [Some(&alpha), Some(&alpha)], Some(&config), |_| None),
                [0, 1]
            );
        }
    }
}

#[test]
fn adjacent_mods_sort_with_one_missing_metadata() {
    let mut known = mod_config("alpha");
    known.priority = 10;
    let mut missing = mod_config("zulu");
    missing.enabled = false;
    missing.priority = 20;
    let metadata = ModInfo {
        provider: "modio",
        name: "Alpha".to_owned(),
        spec: known.spec.clone(),
        versions: vec![],
        resolution: ModResolution::resolvable(known.spec.url.clone().into()),
        suggested_require: false,
        suggested_dependencies: vec![],
        modio_id: None,
        modio_tags: Some(ModioTags {
            qol: false,
            gameplay: false,
            audio: false,
            visual: false,
            framework: false,
            versions: Default::default(),
            required_status: RequiredStatus::Optional,
            approval_status: ApprovalStatus::Verified,
        }),
    };
    for (category, normal, reversed) in [
        (SortBy::Enabled, ["alpha", "zulu"], ["zulu", "alpha"]),
        (SortBy::Name, ["zulu", "alpha"], ["alpha", "zulu"]),
        (SortBy::Priority, ["alpha", "zulu"], ["zulu", "alpha"]),
        (SortBy::Provider, ["zulu", "alpha"], ["alpha", "zulu"]),
        (SortBy::RequiredStatus, ["zulu", "alpha"], ["alpha", "zulu"]),
        (
            SortBy::ApprovalCategory,
            ["zulu", "alpha"],
            ["alpha", "zulu"],
        ),
    ] {
        for (direction, expected) in [(false, normal), (true, reversed)] {
            for mods in [[&known, &missing], [&missing, &known]] {
                let order = sorted_mod_indices(None,
                    mods.map(Some),
                    Some(&sorting(category, direction)),
                    |spec| (spec.url == known.spec.url).then(|| metadata.clone()),
                );
                let sorted_urls: Vec<_> = order
                    .iter()
                    .map(|&index| mods[index].spec.url.as_str())
                    .collect();
                assert_eq!(
                    sorted_urls, expected,
                    "{category:?}, direction={direction}, first={}",
                    mods[0].spec.url
                );
            }
        }
    }
}

#[test]
fn adjacent_mods_sort_with_both_missing_metadata() {
    let mut alpha = mod_config("alpha");
    alpha.priority = 10;
    let mut zulu = mod_config("zulu");
    zulu.enabled = false;
    zulu.priority = 20;
    for (category, normal, reversed) in [
        (SortBy::Enabled, ["alpha", "zulu"], ["zulu", "alpha"]),
        (SortBy::Name, ["alpha", "zulu"], ["zulu", "alpha"]),
        (SortBy::Priority, ["alpha", "zulu"], ["zulu", "alpha"]),
        (SortBy::Provider, ["alpha", "zulu"], ["alpha", "zulu"]),
        (SortBy::RequiredStatus, ["alpha", "zulu"], ["alpha", "zulu"]),
        (
            SortBy::ApprovalCategory,
            ["alpha", "zulu"],
            ["alpha", "zulu"],
        ),
    ] {
        for (direction, expected) in [(false, normal), (true, reversed)] {
            for mods in [[&alpha, &zulu], [&zulu, &alpha]] {
                let order =
                    sorted_mod_indices(None, mods.map(Some), Some(&sorting(category, direction)), |_| {
                        None
                    });
                let sorted_urls: Vec<_> = order
                    .iter()
                    .map(|&index| mods[index].spec.url.as_str())
                    .collect();
                assert_eq!(
                    sorted_urls, expected,
                    "{category:?}, direction={direction}, first={}",
                    mods[0].spec.url
                );
            }
        }
    }
}

struct TestApp {
    app: App,
    context: egui::Context,
    directory: tempfile::TempDir,
}

impl TestApp {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let context = egui::Context::default();
        let app = App::new(
            &eframe::CreationContext::_new_kittest(context.clone()),
            Dirs::from_path(directory.path()).unwrap(),
            None,
        )
        .unwrap();
        Self {
            app,
            context,
            directory,
        }
    }

    fn local_mod(&self, name: &str) -> ModConfig {
        let path = self.directory.path().join(name);
        std::fs::write(&path, b"UI test fixture").unwrap();
        mod_config(path.to_str().unwrap())
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| self.app.ui_profile(ui, "default"));
            },
        )
    }

    fn click(&mut self, pos: egui::Pos2) {
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
    }

    fn open_group(&mut self, name: &str) {
        let output = self.frame(vec![]);
        self.click(text_rect(&output, name).center());
        self.frame(vec![]);
    }
}

fn text_rect(output: &egui::FullOutput, text: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(shape) if shape.galley.text() == text => {
                Some(egui::Rect::from_min_size(shape.pos, shape.galley.size()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Text not rendered: {text}"))
}

#[test]
fn grouped_profile_renders_every_sort_mode_after_reload() {
    let mut test = TestApp::new();
    let alpha = test.local_mod("alpha.pak");
    let zulu = test.local_mod("zulu.pak");
    test.app
        .state
        .mod_data
        .profiles
        .get_mut("default")
        .unwrap()
        .mods = vec![
        ModOrGroup::Individual(zulu.clone()),
        ModOrGroup::Individual(alpha.clone()),
        ModOrGroup::Group {
            member_overrides: Default::default(),
            group_name: "Group".into(),
            enabled: true,
        },
        ModOrGroup::Group {
            member_overrides: Default::default(),
            group_name: "Empty".into(),
            enabled: false,
        },
    ];
    test.app.state.mod_data.groups.insert(
        "Group".into(),
        ModGroup {
            description: None,
            mods: vec![zulu, alpha],
        },
    );
    test.app
        .state
        .mod_data
        .groups
        .insert("Empty".into(), ModGroup::default());
    test.app.state.mod_data.save().unwrap();
    let original = serde_json::to_value(&*test.app.state.mod_data).unwrap();
    test.open_group("Group");
    test.open_group("Empty");
    for category in SortBy::iter() {
        for direction in [false, true] {
            test.app.update_sorting_config(Some(category), direction);
            test.app.state = State::init(Dirs::from_path(test.directory.path()).unwrap()).unwrap();
            let output = test.frame(vec![]);
            assert!(text_rect(&output, "Group").is_positive());
            assert!(text_rect(&output, "Empty").is_positive());
            assert_eq!(
                serde_json::to_value(&*test.app.state.mod_data).unwrap(),
                original
            );
        }
    }
    test.app.update_sorting_config(None, false);
    let output = test.frame(vec![]);
    assert!(text_rect(&output, "zulu.pak").top() < text_rect(&output, "alpha.pak").top());
    assert_eq!(
        serde_json::to_value(&*test.app.state.mod_data).unwrap(),
        original
    );
}

#[test]
fn sorted_group_toggle_preserves_identical_standalone_entry() {
    let mut test = TestApp::new();
    let alpha = test.local_mod("alpha.pak");
    let zulu = test.local_mod("zulu.pak");
    test.app
        .state
        .mod_data
        .profiles
        .get_mut("default")
        .unwrap()
        .mods = vec![
        ModOrGroup::Group {
            member_overrides: Default::default(),
            group_name: "Group".into(),
            enabled: true,
        },
        ModOrGroup::Individual(alpha.clone()),
    ];
    test.app.state.mod_data.groups.insert(
        "Group".into(),
        ModGroup {
            description: None,
            mods: vec![zulu, alpha],
        },
    );
    let second_profile = test.app.state.mod_data.profiles["default"].clone();
    test.app.state.mod_data.profiles.insert("second".into(), second_profile);
    test.app.update_sorting_config(Some(SortBy::Name), false);
    test.open_group("Group");
    let output = test.frame(vec![]);
    let alpha_rect = text_rect(&output, "alpha.pak");
    let zulu_rect = text_rect(&output, "zulu.pak");
    assert!(alpha_rect.top() < zulu_rect.top());
    assert_eq!(
        output
            .shapes
            .iter()
            .filter(|shape| matches!(
                &shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "⚠"
            ))
            .count(),
        0,
        "An identical standalone entry and group member should not warn"
    );
    let toggle_pos = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Circle(circle) if (circle.center.y - zulu_rect.center().y).abs() < 2.0 => {
                Some(circle.center)
            }
            _ => None,
        })
        .expect("Group member toggle is rendered");
    test.click(toggle_pos);
    assert!(test.app.state.mod_data.groups["Group"].mods[0].enabled);
    assert!(test.app.state.mod_data.groups["Group"].mods[1].enabled);
    let reloaded = State::init(Dirs::from_path(test.directory.path()).unwrap()).unwrap();
    assert_eq!(reloaded.mod_data.groups["Group"].mods.len(), 2);
    for (profile, expected) in [("default", false), ("second", true)] {
        let mut includes_zulu = false;
        reloaded.mod_data.for_each_enabled_mod(profile, |mc| {
            includes_zulu |= mc.spec.url.ends_with("zulu.pak");
        });
        assert_eq!(includes_zulu, expected, "profile {profile}");
    }
}

#[test]
fn sorted_individual_delete_targets_the_displayed_mod() {
    let mut test = TestApp::new();
    let alpha = test.local_mod("alpha.pak");
    let zulu = test.local_mod("zulu.pak");
    test.app
        .state
        .mod_data
        .profiles
        .get_mut("default")
        .unwrap()
        .mods = vec![ModOrGroup::Individual(zulu), ModOrGroup::Individual(alpha)];
    test.app.update_sorting_config(Some(SortBy::Name), false);
    let output = test.frame(vec![]);
    assert!(text_rect(&output, "alpha.pak").top() < text_rect(&output, "zulu.pak").top());
    test.click(text_rect(&output, " 🗑 ").center());
    let mods = &test.app.state.mod_data.profiles["default"].mods;
    assert_eq!(mods.len(), 1);
    assert!(matches!(&mods[0], ModOrGroup::Individual(mc) if mc.spec.url.ends_with("zulu.pak")));
}

#[test]
fn dropping_files_on_closed_group_prepends_batch_and_opens_it() {
    let mut test = TestApp::new();
    let existing = test.local_mod("existing.pak");
    let a = test.local_mod("a.pak").spec;
    let b = test.local_mod("b.pak").spec;
    test.app.state.mod_data.groups.insert("Tools".into(), ModGroup { description: None, mods: vec![existing] });
    test.app.state.mod_data.profiles.get_mut("default").unwrap().mods = vec![ModOrGroup::Group {
        group_name: "Tools".into(), enabled: true, member_overrides: Default::default(),
    }];
    test.frame(vec![]);
    group_controls::add_files(&mut test.app, "default".into(), "Tools".into(), true,
        Ok(vec![a.clone(), b.clone()]));
    let mods = &test.app.state.mod_data.groups["Tools"].mods;
    assert_eq!((&mods[0].spec, &mods[1].spec), (&a, &b));
    assert!(test.app.groups_to_open.contains(&("default".into(), "Tools".into())));
    test.frame(vec![]);
    assert!(test.app.groups_to_open.is_empty());
}
