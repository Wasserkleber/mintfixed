use super::*;
use crate::state::{ModData_v0_1_0 as Data, ModProfile_v0_1_0 as Profile};

#[derive(Clone)]
pub(super) struct Drag {
    profile: String,
    group: Option<String>,
    index: usize,
    key: String,
    pub(super) is_group: bool,
}

pub(super) struct Target {
    pub group: Option<String>,
    pub slot: usize,
}

impl Drag {
    pub fn paint_row<R>(
        ui: &mut Ui, profile: &str, group: Option<&str>, index: usize,
        draw: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        let active = egui::DragAndDrop::payload::<Self>(ui.ctx()).is_some_and(|drag| {
            drag.profile == profile && drag.group.as_deref() == group && drag.index == index
        });
        if !active { return draw(ui); }
        let delta = ui.input(|i| {
            Some(i.pointer.interact_pos()? - i.pointer.press_origin()?)
        });
        let Some(delta) = delta else { return draw(ui) };
        // Move only the paint. Keep the layout and drop targets in their saved positions.
        let layer = egui::LayerId::new(egui::Order::Tooltip, ui.id().with("moving-row"));
        let result = ui.scope_builder(egui::UiBuilder::new().layer_id(layer), draw).inner;
        ui.ctx().transform_layer_shapes(layer, egui::emath::TSTransform::from_translation(delta));
        result
    }

    pub fn handle(ui: &mut Ui, profile: &str, group: Option<&str>, index: usize, key: &str, is_group: bool) {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(18.0, ui.spacing().interact_size.y), egui::Sense::drag(),
        );
        let center = rect.center();
        for offset in [-4.0, 0.0, 4.0] {
            ui.painter().hline(center.x - 5.0..=center.x + 5.0, center.y + offset,
                (1.5, ui.visuals().text_color()));
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

    pub fn apply(&self, data: &mut Data, profile: &str, dest: Target) -> Result<(), &'static str> {
        if self.profile != profile {
            return Err("Move mods within one profile at a time");
        }
        let list = &data.profiles[profile].mods;
        let member = match &self.group {
            Some(name) => data.groups.get(name).and_then(|g| g.mods.get(self.index)),
            None => match list.get(self.index) {
                Some(ModOrGroup::Individual(mc)) if !self.is_group => Some(mc),
                Some(ModOrGroup::Group { group_name, .. }) if self.is_group && *group_name == self.key => None,
                _ => return Err("The dragged row has changed"),
            },
        };
        if !self.is_group && member.is_none_or(|m| m.spec.url != self.key) {
            return Err("The dragged mod has changed");
        }
        let same_list = self.group == dest.group;
        let slot = if let Some(name) = &dest.group {
            if self.is_group {
                return Err("Groups cannot contain groups");
            }
            let group = data.groups.get(name).ok_or("The destination group no longer exists")?;
            if !same_list && group.mods.iter().any(|m| m.spec.url == self.key) {
                return Err("That mod is already in this group");
            }
            dest.slot.min(group.mods.len())
        } else {
            dest.slot.min(list.len())
        };
        let slot = slot - usize::from(same_list && self.index < slot);
        if same_list && self.index == slot {
            return Ok(());
        }

        let mut item = if let Some(name) = &self.group {
            ModOrGroup::Individual(data.groups.get_mut(name).unwrap().mods.remove(self.index))
        } else {
            data.profiles.get_mut(profile).unwrap().mods.remove(self.index)
        };

        // Transfer profile-specific switches when moving a shared member.
        if !same_list && let Some(source) = &self.group
            && let ModOrGroup::Individual(mc) = &mut item
        {
            let mut active_enabled = mc.enabled;
            for (profile_name, p) in &mut data.profiles {
                let mut value = None;
                for entry in &mut p.mods {
                    if let ModOrGroup::Group { group_name, member_overrides, .. } = entry
                        && group_name == source
                    {
                        value = Some(member_overrides.enabled(mc));
                        if let Some(id) = &mc.member_id {
                            member_overrides.forget(id);
                        }
                    }
                }
                if let Some(enabled) = value {
                    if profile_name == profile {
                        active_enabled = enabled;
                    }
                    for entry in &mut p.mods {
                        if let ModOrGroup::Group { group_name, member_overrides, .. } = entry
                            && dest.group.as_ref() == Some(group_name)
                        {
                            member_overrides.set(mc, enabled);
                        }
                    }
                }
            }
            if dest.group.is_none() {
                mc.enabled = active_enabled;
                mc.member_id = None;
            }
        }
        match dest.group {
            Some(name) => {
                let ModOrGroup::Individual(mc) = item else { unreachable!() };
                data.groups.get_mut(&name).unwrap().mods.insert(slot, mc);
            }
            None => data.profiles.get_mut(profile).unwrap().mods.insert(slot, item),
        }
        Ok(())
    }
}

pub(super) fn row_target(ui: &Ui, response: &egui::Response, drag: &mut Option<(Drag, Target)>, group: Option<&str>, index: usize) {
    // A member or header gets first choice for mod drops. Whole groups always
    // use the enclosing profile row, even over the members of an expanded group.
    if drag.is_some() {
        return;
    }
    let Some(source) = egui::DragAndDrop::payload::<Drag>(ui.ctx()) else { return };
    if group.is_some() && source.is_group {
        return;
    }
    let mut rect = response.rect;
    rect.max.x = rect.max.x.max(ui.max_rect().right());
    if !ui.rect_contains_pointer(rect) {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_interact_pos() else { return };
    let after = pointer.y > rect.center().y;
    let y = if after { rect.bottom() } else { rect.top() };
    ui.painter().hline(rect.x_range(), y, (2.0, ui.visuals().selection.stroke.color));
    if ui.input(|input| input.pointer.any_released()) {
        *drag = Some(((*source).clone(), Target {
            group: group.map(str::to_owned),
            slot: index + usize::from(after),
        }));
    }
}

pub(super) fn controls(ui: &mut Ui, data: &mut Data, name: &mut String) -> bool {
    let profile = data.profiles.get_mut(&data.active_profile).unwrap();
    let mut changed = false;
    let image = egui::Image::new(egui::include_image!("../../assets/groups.svg"))
        .fit_to_exact_size(egui::vec2(16.0, 16.0))
        .tint(ui.visuals().text_color());
    let button = ui.add(Button::image(image)).on_hover_text("Groups");
    let popup = button.id.with("groups-menu");
    if button.clicked() {
        ui.memory_mut(|mem| mem.toggle_popup(popup));
    }
    egui::popup_below_widget(ui, popup, &button, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(180.0);
        ui.label("New group");
        ui.text_edit_singleline(name);
        let trimmed = name.trim();
        if ui.add_enabled(!trimmed.is_empty() && !data.groups.contains_key(trimmed), Button::new("Create group")).clicked() {
            data.groups.insert(trimmed.into(), Default::default());
            attach(profile, trimmed);
            name.clear();
            changed = true;
            ui.memory_mut(|mem| mem.close_popup());
        }
        ui.separator();
        let eligible: Vec<_> = data.groups.keys().filter(|key| {
            !profile.mods.iter().any(|item| {
                matches!(item, ModOrGroup::Group { group_name, .. } if group_name == *key)
            })
        }).collect();
        if eligible.is_empty() {
            ui.add_enabled(false, Button::new("Add existing group"));
            ui.label("No other groups available for this profile.");
        } else {
            egui::CollapsingHeader::new("Add existing group")
                .id_salt(("attach-existing", &data.active_profile))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                        for key in eligible {
                            if ui.button(key.as_str()).clicked() {
                                attach(profile, key);
                                changed = true;
                                ui.memory_mut(|mem| mem.close_popup());
                                break;
                            }
                        }
                    });
                });
        }
    });
    changed
}

fn attach(profile: &mut Profile, name: &str) {
    profile.mods.insert(0, ModOrGroup::Group {
        group_name: name.into(),
        enabled: true,
        member_overrides: Default::default(),
    });
}

pub(super) fn drop_files(app: &mut App, ctx: &egui::Context, profile: String, group: String, prepend: bool, paths: Vec<PathBuf>) {
    let tx = app.tx.clone();
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let files = tokio::task::spawn_blocking(move || {
            let mut specs = Vec::new();
            for path in paths {
                if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pak")) {
                    return Err(format!("Expected a .pak file: {}", path.display()));
                }
                let file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                repak::PakBuilder::new().reader(&mut std::io::BufReader::new(file))
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let path = path.into_os_string().into_string().map_err(|_| "Unsupported file path".to_string())?;
                specs.push(ModSpecification::new(path));
            }
            if specs.is_empty() {
                return Err("No local .pak files in the drop".into());
            }
            Ok(specs)
        }).await.unwrap_or_else(|e| Err(e.to_string()));
        if tx.send(message::Message::GroupFiles { profile, group, prepend, files }).await.is_ok() {
            ctx.request_repaint();
        }
    });
}

pub(super) fn add_files(app: &mut App, profile: String, group: String, prepend: bool, files: Result<Vec<ModSpecification>, String>) {
    let result = files.and_then(|files| {
        let data = &mut app.state.mod_data;
        if !data.profiles.get(&profile).is_some_and(|p| p.mods.iter().any(|item| {
            matches!(item, ModOrGroup::Group { group_name, .. } if group_name == &group)
        })) {
            return Err("The destination group was removed from the profile".into());
        }
        let target = data.groups.get_mut(&group).ok_or("The destination group no longer exists")?;
        let mut seen: HashSet<_> = target.mods.iter().map(|mc| &mc.spec).collect();
        for spec in &files {
            if !seen.insert(spec) {
                return Err(format!("{} is already in the group or drop", spec.url));
            }
        }
        let slot = if prepend { 0 } else { target.mods.len() };
        target.mods.splice(slot..slot, files.into_iter().map(|spec| ModConfig {
            spec, enabled: true, member_id: None, required: false, priority: 0,
        }));
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

pub(super) struct UndoMove {
    before: Data,
    after: Vec<u8>,
}

impl UndoMove {
    pub fn new(before: Data, after: &Data) -> Self {
        Self { before, after: serde_json::to_vec(after).expect("serializable mod data") }
    }

    pub fn apply(self, data: &mut Data) -> Result<(), &'static str> {
        if serde_json::to_vec(data).expect("serializable mod data") != self.after {
            return Err("The profile or groups have changed since the move");
        }
        *data = self.before;
        Ok(())
    }
}
