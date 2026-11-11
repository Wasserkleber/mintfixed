#[cfg(test)]
use super::management::rename_group;
use super::*;

#[derive(Clone)]
pub(in crate::gui) struct Drag {
    profile: String,
    group: Option<String>,
    index: usize,
    key: String,
    pub(in crate::gui) is_group: bool,
}

pub(in crate::gui) struct Target {
    pub group: Option<String>,
    pub slot: usize,
}

impl Drag {
    pub fn paint_row<R>(
        ui: &mut Ui,
        profile: &str,
        group: Option<&str>,
        index: usize,
        draw: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        let active = egui::DragAndDrop::payload::<Self>(ui.ctx()).is_some_and(|drag| {
            drag.profile == profile && drag.group.as_deref() == group && drag.index == index
        });
        if !active {
            return draw(ui);
        }
        let delta = ui.input(|i| Some(i.pointer.interact_pos()? - i.pointer.press_origin()?));
        let Some(delta) = delta else { return draw(ui) };
        // Move only the paint. Keep the layout and drop targets in their saved positions.
        let layer = egui::LayerId::new(egui::Order::Tooltip, ui.id().with("moving-row"));
        let result = ui
            .scope_builder(egui::UiBuilder::new().layer_id(layer), draw)
            .inner;
        ui.ctx()
            .transform_layer_shapes(layer, egui::emath::TSTransform::from_translation(delta));
        result
    }

    pub fn handle(
        ui: &mut Ui,
        profile: &str,
        group: Option<&str>,
        index: usize,
        key: &str,
        is_group: bool,
    ) {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(18.0, ui.spacing().interact_size.y),
            egui::Sense::drag(),
        );
        let center = rect.center();
        for offset in [-4.0, 0.0, 4.0] {
            ui.painter().hline(
                center.x - 5.0..=center.x + 5.0,
                center.y + offset,
                (1.5, ui.visuals().text_color()),
            );
        }
        let response = response.on_hover_cursor(egui::CursorIcon::Grab);

        if response.drag_started() {
            response.dnd_set_drag_payload(Self {
                profile: profile.into(),
                group: group.map(str::to_owned),
                index,
                key: key.into(),
                is_group,
            });
        }
    }

    pub fn cancel_if_invalid(ctx: &egui::Context, profile: &str, manual: bool) {
        if egui::DragAndDrop::payload::<Self>(ctx)
            .is_some_and(|drag| drag.profile != profile || !manual)
        {
            egui::DragAndDrop::clear_payload(ctx);
        }
    }

    pub fn apply(
        &self,
        data: &mut Data,
        profile: &str,
        dest: Target,
        store: &ModStore,
    ) -> Result<(), &'static str> {
        if self.profile != profile {
            return Err("Move mods within one profile at a time");
        }
        let list = &data
            .profiles
            .get(profile)
            .ok_or("The source profile no longer exists")?
            .mods;
        let member = match &self.group {
            Some(name) => data.groups.get(name).and_then(|g| g.mods.get(self.index)),
            None => match list.get(self.index) {
                Some(ModOrGroup::Individual(mc)) if !self.is_group => Some(mc),
                Some(ModOrGroup::Group { id, .. }) if self.is_group && *id == self.key => None,
                _ => return Err("The dragged row has changed"),
            },
        };
        if !self.is_group && member.is_none_or(|m| m.id != self.key) {
            return Err("The dragged mod has changed");
        }
        let same_list = self.group == dest.group;
        let slot = if let Some(name) = &dest.group {
            if self.is_group {
                return Err("Groups cannot contain groups");
            }
            let group = data
                .groups
                .get(name)
                .ok_or("The destination group no longer exists")?;
            if !same_list {
                let source = crate::mod_identity::Artifact::cached(store, &member.unwrap().spec);
                if group
                    .mods
                    .iter()
                    .any(|m| crate::mod_identity::Artifact::cached(store, &m.spec) == source)
                {
                    return Err("That mod is already in this group");
                }
            }
            dest.slot.min(group.mods.len())
        } else {
            if !same_list && !self.is_group {
                let source = crate::mod_identity::Artifact::cached(store, &member.unwrap().spec);
                if list.iter().any(|item| {
                    matches!(item, ModOrGroup::Individual(mc)
                    if crate::mod_identity::Artifact::cached(store, &mc.spec) == source)
                }) {
                    return Err("That mod already has a standalone entry");
                }
            }
            dest.slot.min(list.len())
        };
        // Resolve the originating reference before mutating any shared collection.
        let effective = if !same_list && !self.is_group {
            let mc = member.unwrap();
            let enabled = if let Some(source) = &self.group {
                let refs: Vec<_> = list
                    .iter()
                    .filter_map(|item| match item {
                        ModOrGroup::Group {
                            group_name,
                            enabled,
                            member_overrides,
                            ..
                        } if group_name == source => Some(*enabled && member_overrides.enabled(mc)),
                        _ => None,
                    })
                    .collect();
                if refs.len() != 1 {
                    return Err("Move requires one reference to the source group in this profile");
                }
                refs[0]
            } else {
                mc.enabled
            };
            if let Some(destination) = &dest.group {
                let refs: Vec<_> = list
                    .iter()
                    .filter_map(|item| match item {
                        ModOrGroup::Group {
                            group_name,
                            enabled,
                            ..
                        } if group_name == destination => Some(*enabled),
                        _ => None,
                    })
                    .collect();
                if refs.len() != 1 {
                    return Err(
                        "Move requires one reference to the destination group in this profile",
                    );
                }
                if enabled && !refs[0] {
                    return Err(
                        "Enable the destination group before moving an enabled mod into it",
                    );
                }
            }
            Some(enabled)
        } else {
            None
        };
        let slot = slot - usize::from(same_list && self.index < slot);
        if same_list && self.index == slot {
            return Ok(());
        }

        let mut item = if let Some(name) = &self.group {
            ModOrGroup::Individual(data.groups.get_mut(name).unwrap().mods.remove(self.index))
        } else {
            data.profiles
                .get_mut(profile)
                .unwrap()
                .mods
                .remove(self.index)
        };

        if let Some(effective) = effective
            && let ModOrGroup::Individual(mc) = &mut item
        {
            // Only removed source memberships lose overrides. Never copy one
            // profile's choices into another destination reference.
            if let (Some(source), Some(id)) = (&self.group, &mc.member_id) {
                for p in data.profiles.values_mut() {
                    for entry in &mut p.mods {
                        if let ModOrGroup::Group {
                            group_name,
                            member_overrides,
                            ..
                        } = entry
                            && group_name == source
                        {
                            member_overrides.forget(id);
                        }
                    }
                }
            }
            // A new membership must not reuse an ID copied into another group.
            mc.member_id = None;
            if let Some(destination) = &dest.group {
                for entry in &mut data.profiles.get_mut(profile).unwrap().mods {
                    if let ModOrGroup::Group {
                        group_name,
                        member_overrides,
                        ..
                    } = entry
                        && group_name == destination
                    {
                        member_overrides.set(mc, effective);
                    }
                }
            } else {
                mc.enabled = effective;
            }
        }
        match dest.group {
            Some(name) => {
                let ModOrGroup::Individual(mc) = item else {
                    unreachable!()
                };
                data.groups.get_mut(&name).unwrap().mods.insert(slot, mc);
            }
            None => data
                .profiles
                .get_mut(profile)
                .unwrap()
                .mods
                .insert(slot, item),
        }
        Ok(())
    }
}

pub(in crate::gui) fn row_target(
    ui: &Ui,
    response: &egui::Response,
    drag: &mut Option<(Drag, Target)>,
    group: Option<&str>,
    index: usize,
) {
    // A member or header gets first choice for mod drops. Whole groups always
    // use the enclosing profile row, even over the members of an expanded group.
    if drag.is_some() {
        return;
    }
    let Some(source) = egui::DragAndDrop::payload::<Drag>(ui.ctx()) else {
        return;
    };
    if group.is_some() && source.is_group {
        return;
    }
    let mut rect = response.rect;
    rect.max.x = rect.max.x.max(ui.max_rect().right());
    // Use the space between profile rows as an insertion target, so dragging
    // a mod between two adjacent groups does not drop it into either group.
    // Keep the drawn marker on the actual row edge, not in the gap.
    let mut hit_rect = rect;
    if group.is_none() {
        let half_gap = ui.spacing().item_spacing.y * 0.5;
        hit_rect.min.y -= half_gap;
        hit_rect.max.y += half_gap;
    }
    if !ui.rect_contains_pointer(hit_rect) {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    let after = pointer.y > rect.center().y;
    let y = if after { rect.bottom() } else { rect.top() };
    // A short rounded insertion marker is clearer than a hard full-row line.
    let marker = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, y),
        egui::vec2((rect.width() - 16.0).max(0.0), 3.0),
    );
    ui.painter().rect_filled(
        marker,
        egui::CornerRadius::same(2),
        ui.visuals().selection.stroke.color,
    );
    if ui.input(|input| input.pointer.any_released()) {
        *drag = Some((
            (*source).clone(),
            Target {
                group: group.map(str::to_owned),
                slot: index + usize::from(after),
            },
        ));
    }
}

/// Capture object identities, never fall back to a newly-created namesake.
#[derive(Debug)]
pub(in crate::gui) struct GroupDestination {
    profile_id: String,
    group_id: String,
    reference_id: String,
}
impl GroupDestination {
    fn capture(data: &Data, profile: &str, group: &str) -> Option<Self> {
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
                if !path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("pak"))
                {
                    return Err(format!("Expected a .pak file: {}", path.display()));
                }
                let file =
                    std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                repak::PakBuilder::new()
                    .reader(&mut std::io::BufReader::new(file))
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let path = path
                    .into_os_string()
                    .into_string()
                    .map_err(|_| "Unsupported file path".to_string())?;
                specs.push(ModSpecification::new(path));
            }
            if specs.is_empty() {
                return Err("No local .pak files in the drop".into());
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

pub(in crate::gui) struct UndoMove {
    before: Data,
    after: Vec<u8>,
}

impl UndoMove {
    pub fn new(before: Data, after: &Data) -> Self {
        Self {
            before,
            after: serde_json::to_vec(after).expect("serializable mod data"),
        }
    }

    pub fn apply(self, data: &mut Data) -> Result<(), &'static str> {
        if serde_json::to_vec(data).expect("serializable mod data") != self.after {
            return Err("The profile or groups have changed since the move");
        }
        *data = self.before;
        Ok(())
    }
}

/// Hover timers are scoped to the rendered group header, including its window.
/// Requiring a recent observation prevents an old timer surviving a profile switch.
#[derive(Clone, Copy, Default)]
struct HoverOpen {
    started: Option<f64>,
    last_seen: f64,
}

impl HoverOpen {
    fn update(&mut self, now: f64, active: bool) -> bool {
        if !active {
            self.started = None;
            self.last_seen = now;
            return false;
        }
        if now < self.last_seen || now - self.last_seen > 0.2 {
            self.started = None;
        }
        self.last_seen = now;
        let start = *self.started.get_or_insert(now);
        now - start >= 0.5
    }
}

pub(in crate::gui) fn hover_open(
    ui: &Ui,
    header: &egui::Response,
    header_drop_area: egui::Rect,
    closed: bool,
    profile: &str,
    manual: bool,
) -> bool {
    let internal = manual
        && egui::DragAndDrop::payload::<Drag>(ui.ctx())
            .is_some_and(|drag| !drag.is_group && drag.profile == profile);
    let external = ui.input(|i| !i.raw.hovered_files.is_empty());
    let active = closed && (internal || external) && ui.rect_contains_pointer(header_drop_area);
    let id = header.id.with("drag-hover-open");
    let now = ui.input(|i| i.time);
    let open = ui.ctx().data_mut(|data| {
        let mut timer = data.get_temp::<HoverOpen>(id).unwrap_or_default();
        let open = timer.update(now, active);
        data.insert_temp(id, timer);
        open
    });
    if active {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
    open
}

fn edge_scroll_delta(view: egui::Rect, pointer: egui::Pos2, dt: f32) -> f32 {
    if !view.contains(pointer) || view.height() <= 0.0 {
        return 0.0;
    }
    let band = 36.0_f32.min(view.height() * 0.25);
    let top = ((view.top() + band - pointer.y) / band).clamp(0.0, 1.0);
    let bottom = ((pointer.y - (view.bottom() - band)) / band).clamp(0.0, 1.0);
    // Positive moves content down (scroll up); negative scrolls down.
    (top - bottom) * 480.0 * dt.clamp(0.0, 0.05)
}

pub(in crate::gui) fn auto_scroll(ui: &Ui, profile: &str, manual: bool) {
    let internal = manual
        && egui::DragAndDrop::payload::<Drag>(ui.ctx()).is_some_and(|drag| drag.profile == profile);
    let external = ui.input(|i| !i.raw.hovered_files.is_empty());
    if !internal && !external {
        return;
    }
    // clip_rect is the visible part of the scroll area, not its full content.
    let view = ui.clip_rect();
    let delta = ui.input(|i| {
        i.pointer
            .hover_pos()
            .map(|pointer| edge_scroll_delta(view, pointer, i.stable_dt))
            .unwrap_or(0.0)
    });
    if delta != 0.0 {
        ui.scroll_with_delta(egui::vec2(0.0, delta));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
}

#[cfg(test)]
mod drag_polish_tests {
    use super::*;

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
    fn edge_scroll_has_direction_speed_limit_and_no_motion_outside_view() {
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(210.0, 420.0));
        assert!(edge_scroll_delta(rect, egui::pos2(50.0, 21.0), 0.016) > 0.0);
        assert!(edge_scroll_delta(rect, egui::pos2(50.0, 419.0), 0.016) < 0.0);
        assert_eq!(edge_scroll_delta(rect, rect.center(), 0.016), 0.0);
        assert_eq!(edge_scroll_delta(rect, egui::pos2(9.0, 21.0), 0.016), 0.0);
        assert!(edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 10.0) <= 24.0);
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
        let undo = UndoMove::new(before.clone(), &data);
        undo.apply(&mut data).unwrap();
        assert_eq!(
            serde_json::to_value(data).unwrap(),
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
