use super::*;
use crate::providers::ModResolution;
use crate::state::ModGroup;

fn mod_config(name: &str) -> ModConfig {
    ModConfig {
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
        sorted_mod_indices(entries, Some(&sorting(SortBy::Name, false)), |_| None),
        [0, 2, 1, 3, 4, 6, 5, 7]
    );
    assert_eq!(
        sorted_mod_indices(entries, Some(&sorting(SortBy::Name, true)), |_| None),
        [0, 1, 2, 3, 4, 5, 6, 7]
    );
    assert_eq!(
        sorted_mod_indices(entries, None, |_| panic!(
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
            let order = sorted_mod_indices([Some(&zulu), Some(&alpha)], Some(&config), |spec| {
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
                sorted_mod_indices([Some(&zulu), None, Some(&alpha)], Some(&config), |_| None);
            assert_eq!(order, [0, 1, 2]);
            assert!(sorted_mod_indices([], Some(&config), |_| None).is_empty());
            assert_eq!(
                sorted_mod_indices([None, None], Some(&config), |_| None),
                [0, 1]
            );
            assert_eq!(
                sorted_mod_indices([Some(&alpha), Some(&alpha)], Some(&config), |_| None),
                [0, 1]
            );
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
        context.enable_accesskit();
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

fn button_rects(output: &egui::FullOutput, label: &str) -> Vec<egui::Rect> {
    let mut rects: Vec<_> = output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter(|(_, node)| {
            node.role() == egui::accesskit::Role::Button && node.label() == Some(label)
        })
        .filter_map(|(_, node)| node.bounds())
        .map(|rect| {
            egui::Rect::from_min_max(
                egui::pos2(rect.x0 as f32, rect.y0 as f32),
                egui::pos2(rect.x1 as f32, rect.y1 as f32),
            )
        })
        .collect();
    rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
    rects
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
            group_name: "Group".into(),
            enabled: true,
        },
        ModOrGroup::Group {
            group_name: "Empty".into(),
            enabled: false,
        },
    ];
    test.app.state.mod_data.groups.insert(
        "Group".into(),
        ModGroup {
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
fn sorted_group_uses_local_indices_for_toggle_and_duplicate_removal() {
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
            group_name: "Group".into(),
            enabled: true,
        },
        ModOrGroup::Individual(alpha.clone()),
    ];
    test.app.state.mod_data.groups.insert(
        "Group".into(),
        ModGroup {
            mods: vec![zulu, alpha],
        },
    );
    test.app.update_sorting_config(Some(SortBy::Name), false);
    test.open_group("Group");
    let output = test.frame(vec![]);
    let alpha_rect = text_rect(&output, "alpha.pak");
    let zulu_rect = text_rect(&output, "zulu.pak");
    assert!(alpha_rect.top() < zulu_rect.top());
    assert_eq!(
        button_rects(&output, "Remove duplicate").len(),
        2,
        "Only the two alpha entries are duplicates; a group member must not match itself"
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
    assert!(!test.app.state.mod_data.groups["Group"].mods[0].enabled);
    assert!(test.app.state.mod_data.groups["Group"].mods[1].enabled);
    let output = test.frame(vec![]);
    test.click(button_rects(&output, "Remove duplicate")[0].center());
    let group = &test.app.state.mod_data.groups["Group"];
    assert_eq!(group.mods.len(), 1);
    assert!(group.mods[0].spec.url.ends_with("zulu.pak"));
    assert_eq!(test.app.state.mod_data.profiles["default"].mods.len(), 2);
    let reloaded = State::init(Dirs::from_path(test.directory.path()).unwrap()).unwrap();
    assert_eq!(reloaded.mod_data.groups["Group"].mods.len(), 1);
    assert!(!reloaded.mod_data.groups["Group"].mods[0].enabled);
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
    test.click(button_rects(&output, "Delete mod")[0].center());
    let mods = &test.app.state.mod_data.profiles["default"].mods;
    assert_eq!(mods.len(), 1);
    assert!(matches!(&mods[0], ModOrGroup::Individual(mc) if mc.spec.url.ends_with("zulu.pak")));
}

#[test]
fn svg_icons_render_and_refresh_cached_textures_at_display_scale() {
    let context = egui::Context::default();
    let icons = [
        Icon::Add,
        Icon::Delete,
        Icon::Settings,
        Icon::Copy,
        Icon::Duplicate,
        Icon::Drag,
        Icon::Folder,
        Icon::Web,
        Icon::Warning,
        Icon::Error,
        Icon::Light,
        Icon::Dark,
        Icon::System,
    ];
    for scale in [1.0, 1.5, 2.0] {
        context.set_pixels_per_point(scale);
        for _ in 0..2 {
            let _ = context.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for icon in icons {
                        icons::show(ui, icon);
                    }
                });
            });
        }
        for icon in icons {
            let (pixels, texture) = context
                .data(|data| {
                    data.get_temp::<(u32, egui::TextureHandle)>(egui::Id::new((
                        "material-icon",
                        icon as u8,
                    )))
                })
                .unwrap();
            assert_eq!(pixels, (icons::SIZE * scale) as u32);
            assert_eq!(texture.size(), [pixels as usize; 2]);
        }
        let output = context.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for icon in icons {
                    icons::show(ui, icon);
                }
            });
        });
        assert!(
            output.textures_delta.set.is_empty(),
            "Unchanged icons must reuse cached textures"
        );
    }
}

#[test]
fn rows_stay_compact_without_a_header() {
    let mut test = TestApp::new();
    let alpha = test.local_mod("alpha.pak");
    let zulu = test.local_mod("zulu.pak");
    test.app
        .state
        .mod_data
        .profiles
        .get_mut("default")
        .unwrap()
        .mods = vec![ModOrGroup::Individual(alpha), ModOrGroup::Individual(zulu)];
    for category in [None, Some(SortBy::Name)] {
        test.app.update_sorting_config(category, false);
        let _ = test.frame(vec![]);
        let output = test.frame(vec![]);
        let alpha = text_rect(&output, "alpha.pak");
        let zulu = text_rect(&output, "zulu.pak");
        assert!(alpha.top() < 15.0);
        assert!(zulu.top() - alpha.top() <= 22.0);
        assert!(button_rects(&output, "Delete mod")[0].height() <= 20.0);
    }
}

#[test]
fn footer_centers_text_and_icons_without_increasing_button_height() {
    for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
        let mut test = TestApp::new();
        test.app.has_run_init = true;
        test.context.set_theme(theme);
        let mut frame = eframe::Frame::_new_kittest();
        let mut output = None;
        for _ in 0..3 {
            output = Some(test.context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| eframe::App::update(&mut test.app, ctx, &mut frame),
            ));
        }
        let output = output.unwrap();
        let settings = button_rects(&output, "Open settings")[0];
        assert!(settings.top() > 470.0 && settings.bottom() <= 500.0);
        for label in [
            "Lint mods",
            "Update cache",
            "Uninstall mods",
            "Install mods",
        ] {
            let button = button_rects(&output, label)[0];
            let text = text_rect(&output, label);
            assert!(button.height() <= 20.0);
            assert!(
                (button.center().y - settings.center().y).abs() <= 0.5,
                "{label}"
            );
            assert!(
                (text.center().y - button.center().y).abs() <= 0.5,
                "{label}: text {text:?}, button {button:?}"
            );
        }
    }
}

#[test]
fn settings_width_is_stable_when_notices_are_expanded_and_collapsed() {
    for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
        let mut test = TestApp::new();
        test.app.settings_window = Some(WindowSettings::new(&test.app.state));
        let context = test.context.clone();
        context.set_theme(theme);
        context.all_styles_mut(|style| style.animation_time = 0.0);
        let mut frame = |events| {
            context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| test.app.show_settings(ctx),
            )
        };
        let bounds =
            || context.memory(|memory| memory.area_rect(egui::Id::new("Settings")).unwrap());
        for _ in 0..3 {
            frame(vec![]);
        }
        let initial = bounds();
        for expanded in [true, false, true, false] {
            let output = frame(vec![]);
            let pos = text_rect(&output, "Third-party notices").center();
            for pressed in [true, false] {
                frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            for _ in 0..3 {
                frame(vec![]);
            }
            let current = bounds();
            assert!(
                (current.width() - initial.width()).abs() <= 1.0,
                "{theme:?}, expanded {expanded}: initial {initial:?}, current {current:?}"
            );
            assert_eq!(current.height() > initial.height() + 100.0, expanded);
        }
    }
}
