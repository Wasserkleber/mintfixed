use super::{App, HashMap, MessageHandle, ModInfo, ModSpecification, ResolveMods};
use crate::{Dirs, providers::ModResolution};

fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let dirs = Dirs::from_path(dir.path()).unwrap();
    std::fs::write(
        dirs.config_dir.join("mod_data.json"),
        serde_json::to_vec(&crate::state::VersionAnnotatedModData::default()).unwrap(),
    )
    .unwrap();
    let app = App::new(
        &eframe::CreationContext::_new_kittest(egui::Context::default()),
        dirs,
        None,
    )
    .unwrap();
    (dir, app)
}

fn result(app: &mut App) -> ResolveMods {
    let rid = app.request_counter.next();
    app.resolve_state.resolve_mod_rid = Some(MessageHandle {
        rid,
        handle: tokio::spawn(async {}),
        state: (),
    });
    let spec = ModSpecification::new("test-mod".to_owned());
    let info = ModInfo {
        provider: "file",
        name: "Test".into(),
        spec: spec.clone(),
        versions: vec![],
        resolution: ModResolution::resolvable("test-mod".to_owned().into()),
        suggested_require: false,
        suggested_dependencies: vec![],
        modio_id: None,
        modio_tags: None,
    };
    ResolveMods {
        rid,
        profile_id: app.state.mod_data.profiles["default"].id.clone(),
        input: "submitted input".into(),
        specs: vec![spec.clone()],
        result: Ok(HashMap::from([(spec, info)])),
        is_dependency: false,
        from_bitfix: false,
    }
}

#[tokio::test]
async fn profile_switch_does_not_redirect_result_or_clear_new_input() {
    let (_dir, mut app) = app();
    let result = result(&mut app);
    app.state
        .mod_data
        .profiles
        .insert("Other".into(), Default::default());
    app.state.mod_data.active_profile = "Other".into();
    app.resolve_state.resolve_mod = "new input".into();
    result.receive(&mut app);
    assert_eq!(app.state.mod_data.profiles["default"].mods.len(), 1);
    assert!(app.state.mod_data.profiles["Other"].mods.is_empty());
    assert_eq!(app.resolve_state.resolve_mod, "new input");
    assert!(app.resolve_state.resolve_mod_rid.is_none());
}

#[tokio::test]
async fn second_request_does_not_replace_pending_result() {
    let (_dir, mut app) = app();
    let result = result(&mut app);
    ResolveMods::send(&mut app, &egui::Context::default(), vec![], false);
    assert_eq!(
        app.resolve_state.resolve_mod_rid.as_ref().unwrap().rid,
        result.rid
    );
    result.receive(&mut app);
    assert_eq!(app.state.mod_data.profiles["default"].mods.len(), 1);
}

#[tokio::test]
async fn missing_original_profile_cancels_without_fallback() {
    let (_dir, mut app) = app();
    let result = result(&mut app);
    app.state
        .mod_data
        .profiles
        .insert("Other".into(), Default::default());
    app.state.mod_data.active_profile = "Other".into();
    app.state.mod_data.profiles.remove("default");
    result.receive(&mut app);
    assert!(app.state.mod_data.profiles["Other"].mods.is_empty());
    assert!(app.resolve_state.resolve_mod_rid.is_none());
}
#[tokio::test]
async fn renamed_profile_receives_pending_addition_by_id() {
    let (_dir, mut app) = app();
    let result = result(&mut app);
    let profile = app.state.mod_data.profiles.remove("default").unwrap();
    app.state
        .mod_data
        .profiles
        .insert("Renamed".into(), profile);
    app.state.mod_data.active_profile = "Renamed".into();
    result.receive(&mut app);
    assert_eq!(app.state.mod_data.profiles["Renamed"].mods.len(), 1);
}

#[tokio::test]
async fn replacement_profile_with_same_name_does_not_receive_old_request() {
    let (_dir, mut app) = app();
    let result = result(&mut app);
    app.state
        .mod_data
        .profiles
        .insert("default".into(), Default::default());
    result.receive(&mut app);
    assert!(app.state.mod_data.profiles["default"].mods.is_empty());
    assert!(app.resolve_state.resolve_mod_rid.is_none());
}
