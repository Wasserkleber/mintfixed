use super::*;

/// Capture object identities, never fall back to a newly-created namesake.
#[derive(Debug)]
pub(in crate::gui) struct GroupDestination {
    profile_id: String,
    group_id: String,
    reference_id: String,
}
impl GroupDestination {
    pub(super) fn capture(data: &Data, profile: &str, group: &str) -> Option<Self> {
        let profile = data.profiles.get(profile)?;
        let reference = profile.mods.iter().find(
            |row| matches!(row, ModOrGroup::Group { group_name, .. } if group_name == group),
        )?;
        Some(Self {
            profile_id: profile.id.clone(),
            group_id: data.groups.get(group)?.id.clone(),
            reference_id: reference.id().to_owned(),
        })
    }
    pub(in crate::gui) fn resolve(&self, data: &Data) -> Option<(String, String)> {
        let (profile_name, profile) = data
            .profiles
            .iter()
            .find(|(_, p)| p.id == self.profile_id)?;
        let (group_name, _) = data.groups.iter().find(|(_, g)| g.id == self.group_id)?;
        profile
            .mods
            .iter()
            .any(|row| {
                matches!(row,
            ModOrGroup::Group { id, group_name: name, .. }
                if id == &self.reference_id && name == group_name)
            })
            .then(|| (profile_name.clone(), group_name.clone()))
    }
}

pub(in crate::gui) fn drop_files(
    app: &mut App,
    ctx: &egui::Context,
    profile: String,
    group: String,
    prepend: bool,
    paths: Vec<PathBuf>,
) {
    if paths.iter().any(|path| {
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("lua"))
    }) {
        app.last_action = Some(LastAction::failure(
            crate::gui::bitfix_menu::REDIRECT_HINT.into(),
        ));
        return;
    }
    let Some(target) = GroupDestination::capture(&app.state.mod_data, &profile, &group) else {
        app.last_action = Some(LastAction::failure(
            "The group destination no longer exists.".into(),
        ));
        return;
    };
    app.pending_group_files += 1;
    let tx = app.tx.clone();
    let ctx = ctx.clone();
    let store = app.state.store.clone();
    let mut identity_specs = crate::mod_identity::all_specs(&app.state.mod_data);
    tokio::spawn(async move {
        let files = tokio::task::spawn_blocking(move || {
            let mut specs = Vec::new();
            for path in paths {
                let ext = path.extension().and_then(|ext| ext.to_str());
                if ext.is_some_and(|ext| ext.eq_ignore_ascii_case("lua")) {
                    // A bitfix patch: installed beside the game exe, not packed.
                } else if ext.is_some_and(|ext| ext.eq_ignore_ascii_case("pak")) {
                    let file = std::fs::File::open(&path)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    repak::PakBuilder::new()
                        .reader(&mut std::io::BufReader::new(file))
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                } else {
                    return Err(format!(
                        "Expected a .pak or bitfix .lua file: {}",
                        path.display()
                    ));
                }
                let path = path
                    .into_os_string()
                    .into_string()
                    .map_err(|_| "Unsupported file path".to_string())?;
                specs.push(ModSpecification::new(path));
            }
            if specs.is_empty() {
                return Err("No local .pak or .lua files in the drop".into());
            }
            identity_specs.extend(specs.iter().cloned());
            store.refresh_local_files(&identity_specs);
            Ok(specs)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        if tx
            .send(message::Message::GroupFiles {
                target,
                prepend,
                files,
            })
            .await
            .is_ok()
        {
            ctx.request_repaint();
        }
    });
}

pub(in crate::gui) fn add_files(
    app: &mut App,
    profile: String,
    group: String,
    prepend: bool,
    files: Result<Vec<ModSpecification>, String>,
) {
    let result = files.and_then(|files| {
        let data = &mut app.state.mod_data;
        if !data.profiles.get(&profile).is_some_and(|p| {
            p.mods.iter().any(
                |item| matches!(item, ModOrGroup::Group { group_name, .. } if group_name == &group),
            )
        }) {
            return Err("The destination group was removed from the profile".into());
        }
        let target = data
            .groups
            .get_mut(&group)
            .ok_or("The destination group no longer exists")?;
        let mut seen: HashSet<_> = target
            .mods
            .iter()
            .map(|mc| crate::mod_identity::Artifact::cached(&app.state.store, &mc.spec))
            .collect();
        for spec in &files {
            if !seen.insert(crate::mod_identity::Artifact::cached(
                &app.state.store,
                spec,
            )) {
                return Err(format!("{} is already in the group or drop", spec.url));
            }
        }
        let slot = if prepend { 0 } else { target.mods.len() };
        target.mods.splice(
            slot..slot,
            files.into_iter().map(|spec| ModConfig {
                id: crate::state::new_id(),
                description: None,
                spec,
                enabled: true,
                member_id: None,
                required: false,
                priority: 0,
            }),
        );
        data.save().map_err(|e| e.to_string())?;
        let message = format!("Added files to {group}");
        app.groups_to_open.insert((profile, group));
        Ok(message)
    });
    app.last_action = Some(match result {
        Ok(message) => LastAction::success(message),
        Err(message) => LastAction::failure(message),
    });
}
