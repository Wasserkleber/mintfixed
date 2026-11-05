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

    pub fn cancel_if_invalid(ctx: &egui::Context, profile: &str, manual: bool) {
        if egui::DragAndDrop::payload::<Self>(ctx)
            .is_some_and(|drag| drag.profile != profile || !manual)
        {
            egui::DragAndDrop::clear_payload(ctx);
        }
    }

    pub fn apply(&self, data: &mut Data, profile: &str, dest: Target, store: &ModStore) -> Result<(), &'static str> {
        if self.profile != profile {
            return Err("Move mods within one profile at a time");
        }
        let list = &data.profiles.get(profile).ok_or("The source profile no longer exists")?.mods;
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
            let group = data.groups.get(name).ok_or("The destination group no longer exists")?;
            if !same_list {
                let source = crate::mod_identity::Artifact::cached(store, &member.unwrap().spec);
                if group.mods.iter().any(|m| crate::mod_identity::Artifact::cached(store, &m.spec) == source) {
                    return Err("That mod is already in this group");
                }
            }
            dest.slot.min(group.mods.len())
        } else {
            if !same_list && !self.is_group {
                let source = crate::mod_identity::Artifact::cached(store, &member.unwrap().spec);
                if list.iter().any(|item| matches!(item, ModOrGroup::Individual(mc)
                    if crate::mod_identity::Artifact::cached(store, &mc.spec) == source)) {
                    return Err("That mod already has a standalone entry");
                }
            }
            dest.slot.min(list.len())
        };
        // Resolve the originating reference before mutating any shared collection.
        let effective = if !same_list && !self.is_group {
            let mc = member.unwrap();
            let enabled = if let Some(source) = &self.group {
                let refs: Vec<_> = list.iter().filter_map(|item| match item {
                    ModOrGroup::Group { group_name, enabled, member_overrides, .. } if group_name == source =>
                        Some(*enabled && member_overrides.enabled(mc)),
                    _ => None,
                }).collect();
                if refs.len() != 1 { return Err("Move requires one reference to the source group in this profile"); }
                refs[0]
            } else { mc.enabled };
            if let Some(destination) = &dest.group {
                let refs: Vec<_> = list.iter().filter_map(|item| match item {
                    ModOrGroup::Group { group_name, enabled, .. } if group_name == destination => Some(*enabled),
                    _ => None,
                }).collect();
                if refs.len() != 1 { return Err("Move requires one reference to the destination group in this profile"); }
                if enabled && !refs[0] {
                    return Err("Enable the destination group before moving an enabled mod into it");
                }
            }
            Some(enabled)
        } else { None };
        let slot = slot - usize::from(same_list && self.index < slot);
        if same_list && self.index == slot {
            return Ok(());
        }

        let mut item = if let Some(name) = &self.group {
            ModOrGroup::Individual(data.groups.get_mut(name).unwrap().mods.remove(self.index))
        } else {
            data.profiles.get_mut(profile).unwrap().mods.remove(self.index)
        };

        if let Some(effective) = effective && let ModOrGroup::Individual(mc) = &mut item {
            // Only removed source memberships lose overrides. Never copy one
            // profile's choices into another destination reference.
            if let (Some(source), Some(id)) = (&self.group, &mc.member_id) {
                for p in data.profiles.values_mut() {
                    for entry in &mut p.mods {
                        if let ModOrGroup::Group { group_name, member_overrides, .. } = entry {
                            if group_name == source { member_overrides.forget(id); }
                        }
                    }
                }
            }
            // A new membership must not reuse an ID copied into another group.
            mc.member_id = None;
            if let Some(destination) = &dest.group {
                for entry in &mut data.profiles.get_mut(profile).unwrap().mods {
                    if let ModOrGroup::Group { group_name, member_overrides, .. } = entry {
                        if group_name == destination { member_overrides.set(mc, effective); }
                    }
                }
            } else {
                mc.enabled = effective;
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

#[derive(Clone, Default)]
struct RenameEditor {
    selected: String,
    name: String,
    error: Option<String>,
    success: Option<String>,
    picker_open: bool,
}

#[derive(Clone, Default)]
struct DeleteEditor {
    selected: String,
    picker_open: bool,
    confirming: bool,
    error: Option<String>,
    success: Option<String>,
}

#[derive(Clone, Default)]
struct DuplicateEditor {
    selected: String,
    name: String,
    picker_open: bool,
    error: Option<String>,
    success: Option<String>,
}

#[derive(Clone, Default)]
struct ExportEditor {
    selected: String,
    picker_open: bool,
    success: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Default)]
struct ImportEditor {
    success: Option<String>,
    error: Option<String>,
}

const GROUP_EXPORT_FORMAT: &str = "mintfixed-groups";
const GROUP_EXPORT_VERSION: u32 = 1;

/// JSON retains the member configs (including mod URLs, enabled switches and priorities).
/// Plain .txt exports deliberately contain only the URLs, like MINT's mod export.
#[derive(serde::Serialize, serde::Deserialize)]
struct GroupExportFile {
    format: String,
    version: u32,
    groups: std::collections::BTreeMap<String, crate::state::ModGroup>,
}

fn serialize_groups(data: &Data) -> Result<String, String> {
    serde_json::to_string_pretty(&GroupExportFile {
        format: GROUP_EXPORT_FORMAT.to_owned(),
        version: GROUP_EXPORT_VERSION,
        groups: data.groups.clone(),
    }).map_err(|e| format!("Failed to serialize groups: {e}"))
}

fn parse_groups_json(text: &str) -> Result<std::collections::BTreeMap<String, crate::state::ModGroup>, String> {
    let export: GroupExportFile = serde_json::from_str(text)
        .map_err(|e| format!("Invalid group export JSON: {e}"))?;
    if export.format != GROUP_EXPORT_FORMAT || export.version != GROUP_EXPORT_VERSION {
        return Err("Unsupported group export format/version".into());
    }
    if export.groups.is_empty() {
        return Err("This export contains no groups".into());
    }
    for (name, group) in &export.groups {
        if name.trim().is_empty() {
            return Err("An imported group has an empty name".into());
        }
        if group.mods.iter().any(|m| m.spec.url.trim().is_empty()) {
            return Err(format!("Group \"{name}\" contains an empty mod URL"));
        }
    }
    Ok(export.groups)
}

fn parse_mod_list_txt(text: &str, group_name: &str) -> Result<std::collections::BTreeMap<String, crate::state::ModGroup>, String> {
    let group_name = group_name.trim();
    if group_name.is_empty() {
        return Err("Cannot determine a group name from the filename".into());
    }
    let mut seen = HashSet::new();
    let mods = text.lines().map(str::trim).filter(|line| !line.is_empty())
        .filter(|line| seen.insert((*line).to_owned()))
        .map(|url| ModConfig {
            id: crate::state::new_id(),
            description: None,
            spec: ModSpecification::new(url.to_owned()),
            enabled: true,
            required: false,
            member_id: None,
            priority: 0,
        }).collect::<Vec<_>>();
    if mods.is_empty() {
        return Err("The mod list is empty".into());
    }
    Ok(std::collections::BTreeMap::from([(group_name.to_owned(), crate::state::ModGroup { id: crate::state::new_id(), mods, description: None })]))
}

fn read_group_export(path: &std::path::Path) -> Result<std::collections::BTreeMap<String, crate::state::ModGroup>, String> {
    const MAX_BYTES: u64 = 8 * 1024 * 1024;
    let size = std::fs::metadata(path).map_err(|e| format!("Cannot read file: {e}"))?.len();
    if size > MAX_BYTES {
        return Err("Group import files must not exceed 8 MiB".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read file: {e}"))?;
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or_default();
    if ext.eq_ignore_ascii_case("txt") {
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        parse_mod_list_txt(&text, name)
    } else if ext.eq_ignore_ascii_case("json") {
        parse_groups_json(&text)
    } else {
        Err("Select a .json group export or a .txt mod list".into())
    }
}

/// Import never overwrites existing groups. All imported names are unique and
/// all imported groups are attached to the active profile.
fn import_groups(data: &mut Data, incoming: std::collections::BTreeMap<String, crate::state::ModGroup>) -> Result<Vec<String>, String> {
    if incoming.is_empty() {
        return Err("Nothing to import".into());
    }
    // Prepare and validate everything before mutating the saved state.
    let mut reserved: HashSet<String> = data.groups.keys().cloned().collect();
    let mut prepared = Vec::new();
    for (name, mut group) in incoming {
        let base = name.trim();
        if base.is_empty() || group.mods.iter().any(|m| m.spec.url.trim().is_empty()) {
            return Err("Import contains an empty group name or mod URL".into());
        }
        let mut candidate = base.to_owned();
        if reserved.contains(&candidate) {
            candidate = format!("{base} (imported)");
            let mut suffix = 2;
            while reserved.contains(&candidate) {
                candidate = format!("{base} (imported {suffix})");
                suffix += 1;
            }
        }
        reserved.insert(candidate.clone());
        // Imports are new objects, even when importing our own export repeatedly.
        group.id = crate::state::new_id();
        group.mods = group.mods.iter().map(ModConfig::duplicate).collect();
        prepared.push((candidate, group));
    }
    let names = prepared.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>();
    for (name, group) in prepared {
        data.groups.insert(name.clone(), group);
        attach(data.profiles.get_mut(&data.active_profile).unwrap(), &name);
    }
    Ok(names)
}

fn export_group_mod_list(data: &Data, name: &str) -> Result<String, String> {
    let group = data.groups.get(name).ok_or("The selected group no longer exists")?;
    if group.mods.iter().any(|m| m.spec.url.contains('\n') || m.spec.url.contains('\r')) {
        return Err("A mod URL contains a newline and cannot be exported to TXT".into());
    }
    Ok(group.mods.iter().map(|m| m.spec.url.as_str()).collect::<Vec<_>>().join("\n") + "\n")
}

#[cfg(test)]
mod group_import_export_tests {
    use super::*;

    #[test]
    fn txt_import_keeps_order_and_deduplicates_lines() {
        let incoming = parse_mod_list_txt(
            "C:\\mods\\a.pak\nhttps://mod.io/g/drg/m/example#123\nC:\\mods\\a.pak\n",
            "My group",
        ).unwrap();
        let mods = &incoming["My group"].mods;
        assert_eq!(mods.len(), 2);
        assert_eq!(mods[0].spec.url, "C:\\mods\\a.pak");
        assert_eq!(mods[1].spec.url, "https://mod.io/g/drg/m/example#123");
        assert_eq!(mods[0].priority, 0);
        assert!(mods[0].enabled);
    }

    #[test]
    fn json_roundtrip_preserves_group_settings() {
        let group = crate::state::ModGroup { id: crate::state::new_id(), description: None, mods: vec![ModConfig {
            id: crate::state::new_id(),
            description: None,
            spec: ModSpecification::new("https://mod.io/g/drg/m/example#123".into()),
            enabled: false, required: true, member_id: Some("member1".into()), priority: 14,
        }] };
        let groups = std::collections::BTreeMap::from([("Example".into(), group)]);
        let bundle = GroupExportFile {
            format: GROUP_EXPORT_FORMAT.into(), version: GROUP_EXPORT_VERSION, groups,
        };
        let decoded = parse_groups_json(&serde_json::to_string(&bundle).unwrap()).unwrap();
        let member = &decoded["Example"].mods[0];
        assert_eq!(member.priority, 14);
        assert!(!member.enabled);
        assert!(member.required);
        assert_eq!(member.member_id.as_deref(), Some("member1"));
    }

    #[test]
    fn import_collision_creates_unique_name_and_preserves_existing_group() {
        let mut data: Data = serde_json::from_value(serde_json::json!({
            "active_profile": "A",
            "profiles": { "A": { "mods": [] } },
            "groups": { "Example": { "mods": [] } }
        })).unwrap();
        let incoming = parse_mod_list_txt("https://mod.io/g/drg/m/example#123\n", "Example").unwrap();
        let names = import_groups(&mut data, incoming).unwrap();
        assert_eq!(names, vec!["Example (imported)"]);
        assert!(data.groups["Example"].mods.is_empty());
        assert_eq!(data.groups["Example (imported)"].mods.len(), 1);
        assert!(data.profiles["A"].mods.iter().any(|item|
            matches!(item, ModOrGroup::Group { group_name, .. } if group_name == "Example (imported)")));
    }

    #[test]
    fn repeated_json_import_allocates_fresh_ids_and_preserves_descriptions() {
        let mut data = Data::default();
        let mut incoming = parse_mod_list_txt("https://mod.io/g/drg/m/example", "Tools").unwrap();
        incoming.get_mut("Tools").unwrap().description = Some("Group help".into());
        incoming.get_mut("Tools").unwrap().mods[0].description = Some("Mod help".into());
        let bundle = GroupExportFile { format: GROUP_EXPORT_FORMAT.into(), version: GROUP_EXPORT_VERSION, groups: incoming };
        let text = serde_json::to_string(&bundle).unwrap();
        for _ in 0..2 { import_groups(&mut data, parse_groups_json(&text).unwrap()).unwrap(); }
        let a = &data.groups["Tools"];
        let b = &data.groups["Tools (imported)"];
        assert_ne!(a.id, b.id);
        assert_ne!(a.mods[0].id, b.mods[0].id);
        assert_ne!(a.id, bundle.groups["Tools"].id);
        assert_ne!(a.mods[0].id, bundle.groups["Tools"].mods[0].id);
        assert_eq!(a.description.as_deref(), Some("Group help"));
        assert_eq!(b.mods[0].description.as_deref(), Some("Mod help"));
        assert_ne!(data.profiles["default"].mods[0].id(), data.profiles["default"].mods[1].id());
    }

    #[test]
    fn unsupported_bundle_version_is_rejected() {
        let invalid = r#"{"format":"mintfixed-groups","version":99,"groups":{"x":{"mods":[]}}}"#;
        assert!(parse_groups_json(invalid).is_err());
    }
}

/// Renaming changes the shared collection's label, never its contents or order.
fn rename_group(data: &mut Data, old: &str, requested: &str) -> Result<bool, &'static str> {
    let name = requested.trim();
    if name.is_empty() { return Err("Enter a group name"); }
    if !data.groups.contains_key(old) { return Err("That group no longer exists"); }
    if name == old { return Ok(false); }
    if data.groups.contains_key(name) { return Err("A group with that name already exists"); }
    let group = data.groups.remove(old).unwrap();
    data.groups.insert(name.to_owned(), group);
    for profile in data.profiles.values_mut() {
        for item in &mut profile.mods {
            if let ModOrGroup::Group { group_name, .. } = item {
                if group_name == old { *group_name = name.to_owned(); }
            }
        }
    }
    Ok(true)
}

/// Delete the shared collection and detach it from every profile.
/// Standalone mods and other groups are untouched.
fn delete_group(data: &mut Data, name: &str) -> Result<(), &'static str> {
    if data.groups.remove(name).is_none() {
        return Err("That group no longer exists");
    }
    for profile in data.profiles.values_mut() {
        profile.mods.retain(|item| !matches!(item, ModOrGroup::Group { group_name, .. } if group_name == name));
    }
    Ok(())
}

/// Create a separate shared group with the same members and their settings.
/// Only the active profile gets a reference to the new group.
fn duplicate_group(data: &mut Data, source: &str, requested: &str) -> Result<String, &'static str> {
    let name = requested.trim();
    if name.is_empty() { return Err("Enter a name for the duplicate group"); }
    if data.groups.contains_key(name) { return Err("A group with that name already exists"); }
    let original = data.groups.get(source).ok_or("That group no longer exists")?;
    let mut copy = original.clone();
    copy.id = crate::state::new_id();
    copy.mods = original.mods.iter().map(ModConfig::duplicate).collect();

    // Keep this profile's switches when the source group is attached here.
    let reference = data.profiles[&data.active_profile].mods.iter().find_map(|item| {
        if let ModOrGroup::Group { group_name, enabled, member_overrides, .. } = item
            && group_name == source
        {
            Some(ModOrGroup::Group {
                id: crate::state::new_id(),
                group_name: name.to_owned(),
                enabled: *enabled,
                member_overrides: {
                    let mut overrides = crate::state::MemberOverrides::default();
                    for (old, new) in original.mods.iter().zip(&mut copy.mods) {
                        overrides.set(new, member_overrides.enabled(old));
                    }
                    overrides
                },
            })
        } else {
            None
        }
    }).unwrap_or(ModOrGroup::Group {
        id: crate::state::new_id(),
        group_name: name.to_owned(),
        enabled: true,
        member_overrides: Default::default(),
    });
    data.groups.insert(name.to_owned(), copy);
    data.profiles.get_mut(&data.active_profile).unwrap().mods.insert(0, reference);
    Ok(name.to_owned())
}

/// A full-width dropdown row with the name left-aligned.
/// The whole row receives hover highlighting and click input.
fn group_option_row(ui: &mut Ui, name: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let highlighted = selected || response.hovered() || response.has_focus();
        if highlighted {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(2), ui.visuals().selection.bg_fill);
        }
        let text_color = if highlighted {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            egui::pos2(rect.left() + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::TextStyle::Button.resolve(ui.style()),
            text_color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Use the same compact, always-visible scrollbar for every group selector.
fn group_picker_scroll() -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .max_height(140.0)
        .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysVisible)
}

/// Returns whether groups changed and an optional success message for the main status bar.
pub(super) fn controls(ui: &mut Ui, data: &mut Data, name: &mut String, files_pending: bool) -> (bool, Option<String>) {
    let profile = data.profiles.get_mut(&data.active_profile).unwrap();
    let mut changed = false;
    let image = egui::Image::new(egui::include_image!("../../assets/groups.svg"))
        .fit_to_exact_size(egui::vec2(16.0, 16.0))
        .tint(ui.visuals().text_color());
    let button = ui.add(Button::image(image)).on_hover_text("Groups");
    let popup = button.id.with("groups-menu");
    let editor_id = popup.with("rename-editor");
    let duplicate_id = popup.with("duplicate-editor");
    let delete_id = popup.with("delete-editor");
    let export_id = popup.with("export-editor");
    let import_id = popup.with("import-editor");
    let mut export_editor = ui.ctx().data_mut(|d| d.get_temp::<ExportEditor>(export_id)).unwrap_or_default();
    let mut import_editor = ui.ctx().data_mut(|d| d.get_temp::<ImportEditor>(import_id)).unwrap_or_default();
    let mut editor = ui.ctx().data_mut(|d| d.get_temp::<RenameEditor>(editor_id)).unwrap_or_default();
    let mut duplicate_editor = ui.ctx().data_mut(|d| d.get_temp::<DuplicateEditor>(duplicate_id)).unwrap_or_default();
    let mut delete_editor = ui.ctx().data_mut(|d| d.get_temp::<DeleteEditor>(delete_id)).unwrap_or_default();
    let mut requested_rename = None;
    let mut requested_duplicate = None;
    let mut requested_delete = None;
    let mut requested_export: Option<(PathBuf, Option<String>)> = None;
    let mut requested_import: Option<PathBuf> = None;
    let mut success_message = None;
    if button.clicked() {
        editor.picker_open = false;
        editor.success = None;
        duplicate_editor.picker_open = false;
        duplicate_editor.success = None;
        delete_editor.picker_open = false;
        delete_editor.confirming = false;
        delete_editor.success = None;
        export_editor.picker_open = false;
        export_editor.success = None;
        import_editor.success = None;
        ui.memory_mut(|mem| mem.toggle_popup(popup));
    }
    egui::popup_below_widget(ui, popup, &button, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(180.0);
        ui.label("New group");
        ui.text_edit_singleline(name);
        let trimmed = name.trim();
        if ui.add_enabled(!trimmed.is_empty() && !data.groups.contains_key(trimmed), Button::new("Create group")).clicked() {
            let created = trimmed.to_owned();
            data.groups.insert(created.clone(), Default::default());
            attach(profile, &created);
            name.clear();
            changed = true;
            success_message = Some(format!("Created group \"{created}\"."));
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
                    // Keep the expanded list compact; additional groups remain scrollable.
                    group_picker_scroll().show(ui, |ui| {
                        for key in eligible {
                            if group_option_row(ui, key.as_str(), false).clicked() {
                                attach(profile, key);
                                changed = true;
                                success_message = Some(format!("Added existing group \"{key}\" to the active profile."));
                                ui.memory_mut(|mem| mem.close_popup());
                                break;
                            }
                        }
                    });
                });
        }
        ui.separator();
        let rename_section = egui::CollapsingHeader::new("Rename group").show(ui, |ui| {
            if let Some(success) = &editor.success {
                ui.colored_label(egui::Color32::from_rgb(100, 200, 120), success);
            }
            if data.groups.is_empty() {
                ui.label("No groups to rename.");
                return;
            }
            // Don't select the first group automatically. Require an explicit choice.
            if !editor.selected.is_empty() && !data.groups.contains_key(&editor.selected) {
                editor.selected.clear();
                // Keep the new name even if the previous group was removed.
                editor.error = None;
                editor.picker_open = false;
            }
            // A compact button, not an input field or a nested popup.
            // Draw the chevron ourselves so it works with fonts lacking arrow glyphs.
            let response = ui.add(
                Button::new(format!("{}    ", if editor.selected.is_empty() {
                    "Choose group"
                } else {
                    editor.selected.as_str()
                }))
                    .min_size(egui::vec2(110.0, ui.spacing().interact_size.y)),
            ).on_hover_text("Choose a group to rename");
            if response.clicked() {
                editor.picker_open = !editor.picker_open;
            }
            let x = response.rect.right() - 12.0;
            let y = response.rect.center().y;
            let arrow = if editor.picker_open {
                vec![egui::pos2(x - 4.0, y + 2.0), egui::pos2(x + 4.0, y + 2.0), egui::pos2(x, y - 2.0)]
            } else {
                vec![egui::pos2(x - 4.0, y - 2.0), egui::pos2(x + 4.0, y - 2.0), egui::pos2(x, y + 2.0)]
            };
            ui.painter().add(egui::Shape::convex_polygon(
                arrow, ui.visuals().text_color(), egui::Stroke::NONE,
            ));
            if editor.picker_open {
                group_picker_scroll().show(ui, |ui| {
                    for key in data.groups.keys() {
                        if group_option_row(ui, key.as_str(), editor.selected == key.as_str()).clicked() {
                            editor.selected = key.clone();
                            // Keep any new name entered before choosing the source group.
                            editor.error = None;
                            editor.success = None;
                            editor.picker_open = false;
                        }
                    }
                });
            }
            ui.add_space(8.0);
            if ui.add(egui::TextEdit::singleline(&mut editor.name)
                .hint_text("New group name")).changed() {
                editor.error = None;
                editor.success = None;
            }
            ui.label("Renames this shared group in every profile.");
            if files_pending {
                ui.label("Wait for dropped files to finish before renaming.");
            }
            let trimmed = editor.name.trim();
            let available = !editor.selected.is_empty() && !trimmed.is_empty() && trimmed != editor.selected
                && !data.groups.contains_key(trimmed);
            if !trimmed.is_empty() && trimmed != editor.selected && data.groups.contains_key(trimmed) {
                ui.colored_label(ui.visuals().warn_fg_color, "A group with that name already exists.");
            }
            if ui.add_enabled(available && !files_pending, Button::new("Rename in all profiles")).clicked() {
                requested_rename = Some((editor.selected.clone(), trimmed.to_owned()));
            }
            if let Some(error) = &editor.error { ui.colored_label(ui.visuals().warn_fg_color, error); }
        });
        if rename_section.header_response.clicked() {
            editor.success = None;
        }
        ui.separator();
        let duplicate_section = egui::CollapsingHeader::new("Duplicate group")
            .id_salt("duplicate-group-section")
            .show(ui, |ui| {
                if let Some(success) = &duplicate_editor.success {
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 120), success);
                }
                if data.groups.is_empty() {
                    ui.label("No groups to duplicate.");
                    return;
                }
                if !duplicate_editor.selected.is_empty() && !data.groups.contains_key(&duplicate_editor.selected) {
                    duplicate_editor.selected.clear();
                    duplicate_editor.picker_open = false;
                    duplicate_editor.error = None;
                }
                let response = ui.add(
                    Button::new(format!("{}    ", if duplicate_editor.selected.is_empty() {
                        "Choose group"
                    } else {
                        duplicate_editor.selected.as_str()
                    }))
                    .min_size(egui::vec2(110.0, ui.spacing().interact_size.y)),
                ).on_hover_text("Choose a group to duplicate");
                if response.clicked() {
                    duplicate_editor.picker_open = !duplicate_editor.picker_open;
                }
                let x = response.rect.right() - 12.0;
                let y = response.rect.center().y;
                let arrow = if duplicate_editor.picker_open {
                    vec![egui::pos2(x - 4.0, y + 2.0), egui::pos2(x + 4.0, y + 2.0), egui::pos2(x, y - 2.0)]
                } else {
                    vec![egui::pos2(x - 4.0, y - 2.0), egui::pos2(x + 4.0, y - 2.0), egui::pos2(x, y + 2.0)]
                };
                ui.painter().add(egui::Shape::convex_polygon(
                    arrow, ui.visuals().text_color(), egui::Stroke::NONE,
                ));
                if duplicate_editor.picker_open {
                    group_picker_scroll().show(ui, |ui| {
                        for key in data.groups.keys() {
                            if group_option_row(ui, key.as_str(), duplicate_editor.selected == key.as_str()).clicked() {
                                duplicate_editor.selected = key.clone();
                                // Keep any new name entered before choosing the source group.
                                duplicate_editor.error = None;
                                duplicate_editor.success = None;
                                duplicate_editor.picker_open = false;
                            }
                        }
                    });
                }
                ui.add_space(8.0);
                if ui.add(egui::TextEdit::singleline(&mut duplicate_editor.name)
                    .hint_text("New group name")).changed() {
                    duplicate_editor.error = None;
                    duplicate_editor.success = None;
                }
                ui.label("Creates a separate copy in the active profile.");
                if files_pending {
                    ui.label("Wait for dropped files to finish before duplicating.");
                }
                let trimmed = duplicate_editor.name.trim();
                if !trimmed.is_empty() && data.groups.contains_key(trimmed) {
                    ui.colored_label(ui.visuals().warn_fg_color, "A group with that name already exists.");
                }
                let available = !duplicate_editor.selected.is_empty() && !trimmed.is_empty()
                    && !data.groups.contains_key(trimmed) && !files_pending;
                if ui.add_enabled(available, Button::new("Duplicate group")).clicked() {
                    requested_duplicate = Some((duplicate_editor.selected.clone(), trimmed.to_owned()));
                }
                if let Some(error) = &duplicate_editor.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
            });
        if duplicate_section.header_response.clicked() {
            duplicate_editor.success = None;
        }
        ui.separator();
        let delete_section = egui::CollapsingHeader::new("Delete group")
            .id_salt("delete-group-section")
            .show(ui, |ui| {
                if let Some(success) = &delete_editor.success {
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 120), success);
                }
                if data.groups.is_empty() {
                    ui.label("No groups to delete.");
                    return;
                }
                if !delete_editor.selected.is_empty() && !data.groups.contains_key(&delete_editor.selected) {
                    delete_editor = DeleteEditor::default();
                }
                // The selector stays inside the Groups popup, avoiding nested popup issues.
                let response = ui.add(
                    Button::new(format!("{}    ", if delete_editor.selected.is_empty() {
                        "Choose group"
                    } else {
                        delete_editor.selected.as_str()
                    }))
                    .min_size(egui::vec2(110.0, ui.spacing().interact_size.y)),
                ).on_hover_text("Choose a group to delete");
                if response.clicked() {
                    delete_editor.picker_open = !delete_editor.picker_open;
                    delete_editor.confirming = false;
                }
                let x = response.rect.right() - 12.0;
                let y = response.rect.center().y;
                let arrow = if delete_editor.picker_open {
                    vec![egui::pos2(x - 4.0, y + 2.0), egui::pos2(x + 4.0, y + 2.0), egui::pos2(x, y - 2.0)]
                } else {
                    vec![egui::pos2(x - 4.0, y - 2.0), egui::pos2(x + 4.0, y - 2.0), egui::pos2(x, y + 2.0)]
                };
                ui.painter().add(egui::Shape::convex_polygon(
                    arrow, ui.visuals().text_color(), egui::Stroke::NONE,
                ));
                if delete_editor.picker_open {
                    group_picker_scroll().show(ui, |ui| {
                        for key in data.groups.keys() {
                            if group_option_row(ui, key.as_str(), delete_editor.selected == key.as_str()).clicked() {
                                delete_editor.selected = key.clone();
                                delete_editor.picker_open = false;
                                delete_editor.confirming = false;
                                delete_editor.error = None;
                                delete_editor.success = None;
                            }
                        }
                    });
                }
                if files_pending {
                    ui.label("Wait for dropped files to finish before deleting.");
                }
                if delete_editor.confirming {
                    ui.colored_label(ui.visuals().warn_fg_color,
                        format!("Delete \"{}\" from every profile? All mods within this shared group will be removed from the profiles.", delete_editor.selected));
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            delete_editor.confirming = false;
                        }
                        if ui.add_enabled(!files_pending, Button::new("Confirm delete")).clicked() {
                            requested_delete = Some(delete_editor.selected.clone());
                        }
                    });
                } else if ui.add_enabled(!delete_editor.selected.is_empty() && !files_pending,
                    Button::new("Delete group")).clicked() {
                    delete_editor.confirming = true;
                    delete_editor.success = None;
                }
                if let Some(error) = &delete_editor.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
            });
        if delete_section.header_response.clicked() {
            delete_editor.success = None;
        }
        ui.separator();
        let export_section = egui::CollapsingHeader::new("Export groups")
            .id_salt("export-groups-section")
            .show(ui, |ui| {
                if let Some(message) = &export_editor.success {
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 120), message);
                }
                if let Some(error) = &export_editor.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
                ui.label("Save all groups with their settings:");
                if ui.add_enabled(!data.groups.is_empty(), Button::new("Export all groups (.json)")).clicked() {
                    requested_export = rfd::FileDialog::new()
                        .add_filter("JSON group export", &["json"])
                        .set_file_name("mintfixed-groups.json")
                        .save_file().map(|path| (path, None));
                }
                ui.add_space(5.0);
                ui.label("Or save one group's mod URLs as a MINT-style list:");
                if !data.groups.contains_key(&export_editor.selected) {
                    export_editor.selected.clear();
                }
                let response = ui.add(Button::new(format!("{}    ",
                    if export_editor.selected.is_empty() { "Choose group" } else { export_editor.selected.as_str() }
                )).min_size(egui::vec2(110.0, ui.spacing().interact_size.y)));
                if response.clicked() { export_editor.picker_open = !export_editor.picker_open; }
                let x = response.rect.right() - 12.0;
                let y = response.rect.center().y;
                let arrow = if export_editor.picker_open {
                    vec![egui::pos2(x - 4.0, y + 2.0), egui::pos2(x + 4.0, y + 2.0), egui::pos2(x, y - 2.0)]
                } else {
                    vec![egui::pos2(x - 4.0, y - 2.0), egui::pos2(x + 4.0, y - 2.0), egui::pos2(x, y + 2.0)]
                };
                ui.painter().add(egui::Shape::convex_polygon(
                    arrow, ui.visuals().text_color(), egui::Stroke::NONE,
                ));
                if export_editor.picker_open {
                    group_picker_scroll().show(ui, |ui| {
                        for key in data.groups.keys() {
                            if group_option_row(ui, key.as_str(), export_editor.selected == key.as_str()).clicked() {
                                export_editor.selected = key.clone();
                                export_editor.picker_open = false;
                                export_editor.error = None;
                            }
                        }
                    });
                }
                if ui.add_enabled(!export_editor.selected.is_empty(), Button::new("Export selected group (.txt)")).clicked() {
                    requested_export = rfd::FileDialog::new()
                        .add_filter("MINT mod list", &["txt"])
                        .set_file_name("mintfixed-group.txt")
                        .save_file().map(|path| (path, Some(export_editor.selected.clone())));
                }
            });
        if export_section.header_response.clicked() {
            export_editor.success = None;
            export_editor.error = None;
        }
        ui.separator();
        let import_section = egui::CollapsingHeader::new("Import groups")
            .id_salt("import-groups-section")
            .show(ui, |ui| {
                if let Some(message) = &import_editor.success {
                    ui.colored_label(egui::Color32::from_rgb(100, 200, 120), message);
                }
                if let Some(error) = &import_editor.error {
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
                ui.label("Import a group bundle (.json) or a MINT mod list (.txt).");
                ui.label("Imported groups are added to the active profile; existing groups are never overwritten.");
                if files_pending {
                    ui.label("Wait for dropped files to finish before importing.");
                }
                if ui.add_enabled(!files_pending, Button::new("Choose file to import...")).clicked() {
                    requested_import = rfd::FileDialog::new()
                        .add_filter("Group exports and mod lists", &["json", "txt"])
                        .pick_file();
                }
            });
        if import_section.header_response.clicked() {
            import_editor.success = None;
            import_editor.error = None;
        }
    });
    if let Some((old, new)) = requested_rename {
        match rename_group(data, &old, &new) {
            Ok(renamed) => {
                changed |= renamed;
                // Require a fresh selection after renaming a group.
                editor.selected.clear();
                if duplicate_editor.selected == old {
                    duplicate_editor.selected = new.clone();
                }
                if delete_editor.selected == old {
                    delete_editor.selected = new.clone();
                    delete_editor.confirming = false;
                }
                editor.name.clear();
                editor.error = None;
                editor.picker_open = false;
                if renamed {
                    let message = format!("Renamed group \"{old}\" to \"{new}\" in all profiles.");
                    editor.success = Some(message.clone());
                    success_message = Some(message);
                }
                egui::DragAndDrop::clear_payload(ui.ctx());
                // Keep the menu open so the inline confirmation can be read.
                ui.ctx().request_repaint();
            }
            Err(error) => editor.error = Some(error.to_owned()),
        }
    }
    if let Some((source, new_name)) = requested_duplicate {
        match duplicate_group(data, &source, &new_name) {
            Ok(created) => {
                changed = true;
                let message = format!("Duplicated group \"{source}\" as \"{created}\".");
                success_message = Some(message.clone());
                duplicate_editor = DuplicateEditor {
                    success: Some(message),
                    ..Default::default()
                };
                egui::DragAndDrop::clear_payload(ui.ctx());
                // Keep the menu open so the inline confirmation can be read.
                ui.ctx().request_repaint();
            }
            Err(error) => duplicate_editor.error = Some(error.to_owned()),
        }
    }
    if let Some(target) = requested_delete {
        match delete_group(data, &target) {
            Ok(()) => {
                changed = true;
                let message = format!("Deleted group \"{target}\" from all profiles.");
                success_message = Some(message.clone());
                delete_editor = DeleteEditor {
                    success: Some(message),
                    ..Default::default()
                };
                if editor.selected == target {
                    editor = RenameEditor::default();
                }
                if duplicate_editor.selected == target {
                    duplicate_editor = DuplicateEditor::default();
                }
                egui::DragAndDrop::clear_payload(ui.ctx());
                // Leave the Groups menu open to show the success message.
                ui.ctx().request_repaint();
            }
            Err(error) => {
                delete_editor.confirming = false;
                delete_editor.error = Some(error.to_owned());
            }
        }
    }
    if let Some((path, group)) = requested_export {
        let result = if let Some(group) = group.as_deref() {
            export_group_mod_list(data, group)
        } else {
            serialize_groups(data)
        }.and_then(|contents| std::fs::write(&path, contents)
            .map_err(|e| format!("Failed to save export: {e}")));
        match result {
            Ok(()) => {
                let message = format!("Exported groups to {}", path.display());
                export_editor.success = Some(message.clone());
                export_editor.error = None;
                success_message = Some(message);
            }
            Err(error) => { export_editor.success = None; export_editor.error = Some(error); }
        }
        ui.ctx().request_repaint();
    }
    if let Some(path) = requested_import {
        let result = read_group_export(&path).and_then(|groups| import_groups(data, groups));
        match result {
            Ok(names) => {
                changed = true;
                let message = format!("Imported {} group(s) into the active profile: {}", names.len(), names.join(", "));
                import_editor.success = Some(message.clone());
                import_editor.error = None;
                success_message = Some(message);
                egui::DragAndDrop::clear_payload(ui.ctx());
            }
            Err(error) => { import_editor.success = None; import_editor.error = Some(error); }
        }
        ui.ctx().request_repaint();
    }
    ui.ctx().data_mut(|d| {
        d.insert_temp(export_id, export_editor);
        d.insert_temp(import_id, import_editor);
        d.insert_temp(editor_id, editor);
        d.insert_temp(duplicate_id, duplicate_editor);
        d.insert_temp(delete_id, delete_editor);
    });
    (changed, success_message)
}

fn attach(profile: &mut Profile, name: &str) {
    profile.mods.insert(0, ModOrGroup::Group {
        id: crate::state::new_id(),
        group_name: name.into(),
        enabled: true,
        member_overrides: Default::default(),
    });
}

/// Capture object identities, never fall back to a newly-created namesake.
#[derive(Debug)]
pub(super) struct GroupDestination {
    profile_id: String,
    group_id: String,
    reference_id: String,
}
impl GroupDestination {
    fn capture(data: &Data, profile: &str, group: &str) -> Option<Self> {
        let profile = data.profiles.get(profile)?;
        let reference = profile.mods.iter().find(|row|
            matches!(row, ModOrGroup::Group { group_name, .. } if group_name == group))?;
        Some(Self { profile_id: profile.id.clone(), group_id: data.groups.get(group)?.id.clone(),
            reference_id: reference.id().to_owned() })
    }
    pub(super) fn resolve(&self, data: &Data) -> Option<(String, String)> {
        let (profile_name, profile) = data.profiles.iter().find(|(_, p)| p.id == self.profile_id)?;
        let (group_name, _) = data.groups.iter().find(|(_, g)| g.id == self.group_id)?;
        profile.mods.iter().any(|row| matches!(row,
            ModOrGroup::Group { id, group_name: name, .. }
                if id == &self.reference_id && name == group_name))
            .then(|| (profile_name.clone(), group_name.clone()))
    }
}

pub(super) fn drop_files(app: &mut App, ctx: &egui::Context, profile: String, group: String, prepend: bool, paths: Vec<PathBuf>) {
    let Some(target) = GroupDestination::capture(&app.state.mod_data, &profile, &group) else {
        app.last_action = Some(LastAction::failure("The group destination no longer exists.".into()));
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
            identity_specs.extend(specs.iter().cloned());
            store.refresh_local_files(&identity_specs);
            Ok(specs)
        }).await.unwrap_or_else(|e| Err(e.to_string()));
        if tx.send(message::Message::GroupFiles { target, prepend, files }).await.is_ok() {
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
        let mut seen: HashSet<_> = target.mods.iter()
            .map(|mc| crate::mod_identity::Artifact::cached(&app.state.store, &mc.spec)).collect();
        for spec in &files {
            if !seen.insert(crate::mod_identity::Artifact::cached(&app.state.store, spec)) {
                return Err(format!("{} is already in the group or drop", spec.url));
            }
        }
        let slot = if prepend { 0 } else { target.mods.len() };
        target.mods.splice(slot..slot, files.into_iter().map(|spec| ModConfig {
            id: crate::state::new_id(),
            description: None,
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

pub(super) fn hover_open(ui: &Ui, header: &egui::Response, closed: bool, profile: &str, manual: bool) -> bool {
    let internal = manual && egui::DragAndDrop::payload::<Drag>(ui.ctx())
        .is_some_and(|drag| !drag.is_group && drag.profile == profile);
    let external = ui.input(|i| !i.raw.hovered_files.is_empty());
    let active = closed && (internal || external) && ui.rect_contains_pointer(header.rect);
    let id = header.id.with("drag-hover-open");
    let now = ui.input(|i| i.time);
    let open = ui.ctx().data_mut(|data| {
        let mut timer = data.get_temp::<HoverOpen>(id).unwrap_or_default();
        let open = timer.update(now, active);
        data.insert_temp(id, timer);
        open
    });
    if active {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    open
}

fn edge_scroll_delta(view: egui::Rect, pointer: egui::Pos2, dt: f32) -> f32 {
    if !view.contains(pointer) || view.height() <= 0.0 { return 0.0; }
    let band = 36.0_f32.min(view.height() * 0.25);
    let top = ((view.top() + band - pointer.y) / band).clamp(0.0, 1.0);
    let bottom = ((pointer.y - (view.bottom() - band)) / band).clamp(0.0, 1.0);
    // Positive moves content down (scroll up); negative scrolls down.
    (top - bottom) * 480.0 * dt.clamp(0.0, 0.05)
}

pub(super) fn auto_scroll(ui: &Ui, profile: &str, manual: bool) {
    let internal = manual && egui::DragAndDrop::payload::<Drag>(ui.ctx())
        .is_some_and(|drag| drag.profile == profile);
    let external = ui.input(|i| !i.raw.hovered_files.is_empty());
    if !internal && !external { return; }
    // clip_rect is the visible part of the scroll area, not its full content.
    let view = ui.clip_rect();
    let delta = ui.input(|i| i.pointer.hover_pos()
        .map(|pointer| edge_scroll_delta(view, pointer, i.stable_dt))
        .unwrap_or(0.0));
    if delta != 0.0 {
        ui.scroll_with_delta(egui::vec2(0.0, delta));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
}

#[cfg(test)]
mod drag_polish_tests {
    use super::*;

    #[test]
    fn hover_requires_dwell_and_resets_after_leaving_or_missing_frames() {
        let mut timer = HoverOpen::default();
        assert!(!timer.update(1.0, true));
        for i in 1..5 { assert!(!timer.update(1.0 + i as f64 * 0.1, true)); }
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
        assert_eq!(edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 0.032),
            2.0 * edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 0.016));
    }
}

#[cfg(test)]
mod rename_tests {
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
        })).unwrap()
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
        assert_eq!(duplicate_group(&mut data, "Tools", "  Tools Backup  ").unwrap(), "Tools Backup");
        let copy = &data.groups["Tools Backup"];
        let original = &data.groups["Tools"];
        assert_ne!(copy.id, original.id);
        assert_eq!(copy.mods.len(), original.mods.len());
        for (a, b) in copy.mods.iter().zip(&original.mods) {
            assert_ne!(a.id, b.id);
            if b.member_id.is_some() { assert_ne!(a.member_id, b.member_id); }
            assert_eq!((&a.spec, a.enabled, a.required, a.priority), (&b.spec, b.enabled, b.required, b.priority));
        }
        assert_eq!(serde_json::to_value(&data.groups["Tools"]).unwrap(), source);
        assert_eq!(serde_json::to_value(&data.profiles["B"]).unwrap(), other_profile);
        assert!(matches!(&data.profiles["A"].mods[0], ModOrGroup::Group {
            group_name, enabled: true, member_overrides, .. } if group_name == "Tools Backup" && !member_overrides.enabled(&data.groups["Tools Backup"].mods[0])));
        data.groups.get_mut("Tools Backup").unwrap().mods[0].priority = 99;
        assert_eq!(data.groups["Tools"].mods[0].priority, 7);
    }

    #[test]
    fn duplicate_group_rejects_invalid_names_without_changes() {
        let mut data = fixture();
        let before = serde_json::to_value(&data).unwrap();
        for (source, name) in [("Missing", "New"), ("Tools", "  "),
            ("Tools", "Other"), ("Tools", "Tools")]
        {
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
        assert!(matches!(data.profiles["A"].mods[0], ModOrGroup::Individual(_)));
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
        Drag { profile: "A".into(), group: Some("Source".into()), index: 0,
            key: data.groups["Source"].mods[0].id.clone(), is_group: false }
    }

    fn enabled(data: &Data, profile: &str, url: &str) -> bool {
        let mut found = false;
        data.for_each_enabled_mod(profile, |mc| { if mc.spec.url == url { found = true; } });
        found
    }

    #[test]
    fn moving_out_of_disabled_parent_stays_off_and_undo_restores_every_profile() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModStore::new(dir.path(), &Default::default()).unwrap();
        let mut data = data(false, true);
        let before = data.clone();
        drag(&data).apply(&mut data, "A", Target { group: None, slot: 0 }, &store).unwrap();
        assert!(!enabled(&data, "A", "moving"));
        let ModOrGroup::Individual(mc) = &data.profiles["A"].mods[0] else { panic!() };
        assert_eq!(mc.priority, 7);
        assert_eq!(mc.id, before.groups["Source"].mods[0].id);
        assert!(!mc.enabled);
        let undo = UndoMove::new(before.clone(), &data);
        undo.apply(&mut data).unwrap();
        assert_eq!(serde_json::to_value(data).unwrap(), serde_json::to_value(before).unwrap());
    }

    #[test]
    fn destination_profiles_inherit_default_instead_of_source_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModStore::new(dir.path(), &Default::default()).unwrap();
        let mut data = data(false, true);
        drag(&data).apply(&mut data, "A", Target { group: Some("Destination".into()), slot: 1 }, &store).unwrap();
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
        assert!(drag(&data).apply(&mut data, "A", Target { group: Some("Destination".into()), slot: 0 }, &store).is_err());
        assert_eq!(serde_json::to_value(data).unwrap(), before);
    }
    #[test]
    fn group_destination_survives_rename_but_rejects_replacement_reference() {
        let mut data = data(false, true);
        let target = GroupDestination::capture(&data, "A", "Source").unwrap();
        rename_group(&mut data, "Source", "Renamed").unwrap();
        let profile = data.profiles.remove("A").unwrap();
        data.profiles.insert("Renamed Profile".into(), profile);
        assert_eq!(target.resolve(&data), Some(("Renamed Profile".into(), "Renamed".into())));
        if let ModOrGroup::Group { id, .. } = &mut data.profiles.get_mut("Renamed Profile").unwrap().mods[0] {
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
        assert!(drag.apply(&mut data, "A", Target { group: None, slot: 0 }, &store).is_err());
        assert_eq!(serde_json::to_value(&data).unwrap(), before);
    }

}
