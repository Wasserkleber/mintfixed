use super::*;

// Session-only drag selection. It is never written to mod_data.json.
// The handles of visible mods register themselves every frame, which makes
// Shift-click and Ctrl+A respect collapsed groups and current list rendering.
#[derive(Clone, Default)]
struct ModMultiSelection {
    mode: bool,
    selected: std::collections::BTreeSet<String>,
    anchor: Option<String>,
    visible: Vec<String>,
    previous_visible: Vec<String>,
    dragging: Vec<String>,
}

type ModSelectionMap = std::collections::HashMap<String, ModMultiSelection>;
const MOD_SELECTION_ID: &str = "mintfixed-mod-drag-selection";

// egui's temp data is Clone-based. Keep only a cheap Arc clone in the
// IdTypeMap instead of cloning every selected ID for every rendered row.
#[derive(Clone, Default)]
struct ModSelectionStore(std::sync::Arc<std::sync::Mutex<ModSelectionMap>>);

impl Drag {
    fn selection_store(ctx: &egui::Context) -> ModSelectionStore {
        ctx.data_mut(|memory| {
            let id = egui::Id::new(MOD_SELECTION_ID);
            if let Some(store) = memory.get_temp::<ModSelectionStore>(id) {
                return store;
            }
            let store = ModSelectionStore::default();
            memory.insert_temp(id, store.clone());
            store
        })
    }

    fn update_selection(ctx: &egui::Context, f: impl FnOnce(&mut ModSelectionMap)) {
        let store = Self::selection_store(ctx);
        let mut map = store.0.lock().expect("selection mutex poisoned");
        f(&mut map);
    }

    fn read_selection<R>(ctx: &egui::Context, f: impl FnOnce(&ModSelectionMap) -> R) -> R {
        let store = Self::selection_store(ctx);
        let map = store.0.lock().expect("selection mutex poisoned");
        f(&map)
    }

    /// A history restore can move stable IDs between shared groups and profiles.
    /// Discard transient selection, range anchors and cached visible row order.
    pub(in crate::gui) fn clear_history_selection(ctx: &egui::Context) {
        Self::update_selection(ctx, |map| map.clear());
        if egui::DragAndDrop::payload::<Self>(ctx).is_some() {
            egui::DragAndDrop::clear_payload(ctx);
        }
        ctx.data_mut(|data| {
            data.remove::<DragPaintAnchor>(egui::Id::new(DRAG_PAINT_ANCHOR_ID));
        });
    }

    pub(in crate::gui) fn selection_mode(ctx: &egui::Context, profile: &str) -> bool {
        Self::read_selection(ctx, |map| map.get(profile).is_some_and(|s| s.mode))
    }

    pub(in crate::gui) fn set_selection_mode(ctx: &egui::Context, profile: &str, mode: bool) {
        Self::update_selection(ctx, |map| {
            let state = map.entry(profile.to_owned()).or_default();
            state.mode = mode;
            if !mode {
                state.selected.clear();
                state.anchor = None;
                state.dragging.clear();
            }
        });
        ctx.request_repaint();
    }

    pub(in crate::gui) fn selected_ids(ctx: &egui::Context, profile: &str) -> std::collections::BTreeSet<String> {
        Self::read_selection(ctx, |map| map.get(profile).map(|s| s.selected.clone()).unwrap_or_default())
    }

    pub(in crate::gui) fn clear_selected(ctx: &egui::Context, profile: &str) {
        Self::update_selection(ctx, |map| {
            if let Some(state) = map.get_mut(profile) {
                state.selected.clear();
                state.anchor = None;
                state.dragging.clear();
            }
        });
        ctx.request_repaint();
    }

    pub(in crate::gui) fn select_visible(ctx: &egui::Context, profile: &str) {
        Self::update_selection(ctx, |map| {
            let state = map.entry(profile.to_owned()).or_default();
            state.mode = true;
            state.selected = state.previous_visible.iter().cloned().collect();
            state.anchor = state.previous_visible.first().cloned();
        });
        ctx.request_repaint();
    }

    // Keep selections in collapsed groups, but never retain a removed mod.
    pub(in crate::gui) fn retain_valid_selection(ctx: &egui::Context, profile: &str, data: &Data) {
        Self::update_selection(ctx, |map| {
            if let Some(state) = map.get_mut(profile) {
                state.selected.retain(|id| Self::find_mod(data, profile, id).is_some());
                if state.anchor.as_ref().is_some_and(|id| Self::find_mod(data, profile, id).is_none()) {
                    state.anchor = None;
                }
            }
        });
    }

    pub(in crate::gui) fn selection_checkbox(ui: &mut Ui, profile: &str, key: &str) {
        if !Self::selection_mode(ui.ctx(), profile) {
            return;
        }
        let mut selected = Self::is_selected(ui.ctx(), profile, key);
        if ui.add(egui::Checkbox::without_text(&mut selected))
            .on_hover_text("Select mod (Shift+click for a range)").changed()
        {
            Self::selection_click(ui.ctx(), profile, key);
        }
    }

    pub(in crate::gui) fn group_selection_checkbox(ui: &mut Ui, profile: &str, ids: &[String]) {
        if !Self::selection_mode(ui.ctx(), profile) || ids.is_empty() {
            return;
        }
        let count = Self::read_selection(ui.ctx(), |map| {
            map.get(profile).map_or(0, |s| ids.iter().filter(|id| s.selected.contains(*id)).count())
        });
        let mut all = count == ids.len();
        if ui.add(egui::Checkbox::without_text(&mut all))
            .on_hover_text("Select or deselect all group members, including collapsed mods").changed()
        {
            Self::update_selection(ui.ctx(), |map| {
                let state = map.entry(profile.to_owned()).or_default();
                if all {
                    state.selected.extend(ids.iter().cloned());
                } else {
                    for id in ids { state.selected.remove(id); }
                }
                state.anchor = None;
            });
            ui.ctx().request_repaint();
        }
        if count > 0 {
            ui.weak(format!("{count} selected"));
        }
    }

    pub(in crate::gui) fn select_name(ui: &mut Ui, profile: &str, key: &str, job: egui::text::LayoutJob, url: &str, manual: bool) -> egui::Response {
        let modifiers = ui.input(|i| i.modifiers);
        if manual && (Self::selection_mode(ui.ctx(), profile) || modifiers.ctrl || modifiers.shift) {
            let response = ui.add(egui::Label::new(job).sense(egui::Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Click to select; Shift+click for a range");
            if response.clicked() {
                Self::selection_click(ui.ctx(), profile, key);
            }
            response
        } else {
            ui.hyperlink_to(job, url)
        }
    }

    pub(in crate::gui) fn begin_selection_frame(ui: &mut Ui, profile: &str, manual: bool) {
        let keyboard_free = !ui.ctx().wants_keyboard_input();
        let ctrl_a = manual && keyboard_free
            && ui.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::A));
        let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
        Self::update_selection(ui.ctx(), |map| {
            let state = map.entry(profile.to_owned()).or_default();
            state.previous_visible = std::mem::take(&mut state.visible);
            if !manual || escape {
                state.mode = false;
                state.selected.clear();
                state.anchor = None;
                state.dragging.clear();
            } else if ctrl_a {
                state.mode = true;
                state.selected = state.previous_visible.iter().cloned().collect();
                state.anchor = state.previous_visible.first().cloned();
            }
        });
    }

    pub(super) fn register_visible(ctx: &egui::Context, profile: &str, key: &str) {
        Self::update_selection(ctx, |map| {
            let state = map.entry(profile.to_owned()).or_default();
            if !state.visible.iter().any(|id| id == key) {
                state.visible.push(key.to_owned());
            }
        });
    }

    pub(in crate::gui) fn is_selected(ctx: &egui::Context, profile: &str, key: &str) -> bool {
        Self::read_selection(ctx, |map| {
            map.get(profile).is_some_and(|state| state.selected.contains(key))
        })
    }

    pub(super) fn selection_click(ctx: &egui::Context, profile: &str, key: &str) {
        let modifiers = ctx.input(|i| i.modifiers);
        Self::update_selection(ctx, |map| {
            let state = map.entry(profile.to_owned()).or_default();
            state.mode |= modifiers.ctrl || modifiers.shift;
            if modifiers.shift {
                let range = state.anchor.as_deref().and_then(|anchor| {
                    let visible = if state.previous_visible.contains(&key.to_owned()) {
                        &state.previous_visible
                    } else {
                        &state.visible
                    };
                    let a = visible.iter().position(|id| id == anchor)?;
                    let b = visible.iter().position(|id| id == key)?;
                    Some(visible[a.min(b)..=a.max(b)].to_vec())
                });
                if let Some(range) = range {
                    if !modifiers.ctrl {
                        state.selected.clear();
                    }
                    state.selected.extend(range);
                } else {
                    state.selected.insert(key.to_owned());
                    state.anchor = Some(key.to_owned());
                }
            } else if modifiers.ctrl || state.mode {
                if !state.selected.insert(key.to_owned()) {
                    state.selected.remove(key);
                }
                state.anchor = Some(key.to_owned());
            } else {
                state.selected.clear();
                state.selected.insert(key.to_owned());
                state.anchor = Some(key.to_owned());
            }
        });
        ctx.request_repaint();
    }

    pub(super) fn selection_drag_started(ctx: &egui::Context, profile: &str, key: &str, is_group: bool) {
        Self::update_selection(ctx, |map| {
            let state = map.entry(profile.to_owned()).or_default();
            if is_group {
                state.dragging.clear();
                return;
            }
            if !state.selected.contains(key) {
                state.selected.clear();
                state.selected.insert(key.to_owned());
                state.anchor = Some(key.to_owned());
            }
            let order = if state.previous_visible.contains(&key.to_owned()) {
                &state.previous_visible
            } else {
                &state.visible
            };
            state.dragging = order
                .iter()
                .filter(|id| state.selected.contains(*id))
                .cloned()
                .collect();
            // Collapsed group members are selected too. Append them after the
            // visible preview; apply_batch reconstructs actual profile order.
            let hidden: Vec<_> = state.selected.iter()
                .filter(|id| !state.dragging.contains(*id)).cloned().collect();
            state.dragging.extend(hidden);
            if !state.dragging.iter().any(|id| id == key) {
                state.dragging.push(key.to_owned());
            }
        });
    }

    pub(in crate::gui) fn dragged_mod_ids(ctx: &egui::Context, source: &Drag) -> Vec<String> {
        if source.is_group {
            return Vec::new();
        }
        Self::read_selection(ctx, |map| {
            map.get(&source.profile)
                .map(|state| state.dragging.clone())
                .filter(|ids| ids.iter().any(|id| id == &source.key))
                .unwrap_or_else(|| vec![source.key.clone()])
        })
    }

    /// Resolve an entry by stable ID, not the potentially shifted list index.
    fn find_mod(data: &Data, profile: &str, id: &str) -> Option<(Option<String>, usize)> {
        let current = data.profiles.get(profile)?;
        for (index, item) in current.mods.iter().enumerate() {
            match item {
                ModOrGroup::Individual(mc) if mc.id == id => return Some((None, index)),
                ModOrGroup::Group { group_name, .. } => {
                    if let Some(index) = data.groups.get(group_name)?
                        .mods.iter().position(|mc| mc.id == id)
                    {
                        return Some((Some(group_name.clone()), index));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Move the visible selection in original list order as one transaction.
    /// The existing single-mod apply() still performs enabled-state and
    /// member-override checks for each entry. No partial edit ever escapes.
    pub(in crate::gui) fn apply_batch(
        &self,
        data: &mut Data,
        profile: &str,
        target: Target,
        store: &ModStore,
        ids: &[String],
    ) -> Result<(usize, usize), &'static str> {
        if self.profile != profile || self.is_group {
            return Err("Move mods within one profile at a time");
        }
        let chosen: std::collections::BTreeSet<&str> = ids.iter().map(String::as_str).collect();
        if chosen.len() != ids.len() || !chosen.contains(self.key.as_str()) {
            return Err("The selected mods have changed");
        }
        // Reconstruct original order, independent of the order of Ctrl-clicks.
        let current = data.profiles.get(profile).ok_or("The source profile no longer exists")?;
        let mut ordered = Vec::new();
        for item in &current.mods {
            match item {
                ModOrGroup::Individual(mc) if chosen.contains(mc.id.as_str()) => {
                    ordered.push(mc.id.clone());
                }
                ModOrGroup::Group { group_name, .. } => {
                    if let Some(group) = data.groups.get(group_name) {
                        ordered.extend(group.mods.iter()
                            .filter(|mc| chosen.contains(mc.id.as_str()))
                            .map(|mc| mc.id.clone()));
                    }
                }
                _ => {}
            }
        }
        if ordered.len() != chosen.len() {
            return Err("A selected mod was removed before the drop");
        }
        // Anchor the insertion to the first unselected entry at/after the
        // original target slot. Source removals cannot displace this ID.
        let anchor = match &target.group {
            Some(group_name) => data.groups.get(group_name)
                .ok_or("The destination group no longer exists")?
                .mods.iter().skip(target.slot)
                .find(|mc| !chosen.contains(mc.id.as_str()))
                .map(|mc| mc.id.clone()),
            None => current.mods.iter().skip(target.slot)
                .find(|item| match item {
                    ModOrGroup::Individual(mc) => !chosen.contains(mc.id.as_str()),
                    ModOrGroup::Group { .. } => true,
                })
                .map(|item| match item {
                    ModOrGroup::Individual(mc) => mc.id.clone(),
                    ModOrGroup::Group { id, .. } => id.clone(),
                }),
        };
        let mut staged = data.clone();
        let mut moved = 0;
        let mut skipped = 0;
        for key in ordered {
            let (source_group, index) = Self::find_mod(&staged, profile, &key)
                .ok_or("A selected mod was removed before the drop")?;
            // Duplicates stay at their source; valid selected mods still move.
            if source_group != target.group {
                let spec = match &source_group {
                    Some(name) => &staged.groups[name].mods[index].spec,
                    None => match &staged.profiles[profile].mods[index] {
                        ModOrGroup::Individual(mc) => &mc.spec,
                        _ => return Err("The dragged row has changed"),
                    },
                };
                let artifact = crate::mod_identity::Artifact::cached(store, spec);
                let duplicate = match &target.group {
                    Some(name) => staged.groups[name].mods.iter()
                        .any(|mc| crate::mod_identity::Artifact::cached(store, &mc.spec) == artifact),
                    None => staged.profiles[profile].mods.iter().any(|item| {
                        matches!(item, ModOrGroup::Individual(mc)
                            if crate::mod_identity::Artifact::cached(store, &mc.spec) == artifact)
                    }),
                };
                if duplicate {
                    skipped += 1;
                    continue;
                }
            }
            let slot = match &target.group {
                Some(name) => {
                    let list = &staged.groups[name].mods;
                    anchor.as_ref()
                        .and_then(|id| list.iter().position(|mc| mc.id == *id))
                        .unwrap_or(list.len())
                }
                None => {
                    let list = &staged.profiles[profile].mods;
                    anchor.as_ref()
                        .and_then(|id| list.iter().position(|item| match item {
                            ModOrGroup::Individual(mc) => mc.id == *id,
                            ModOrGroup::Group { id: group_id, .. } => group_id == id,
                        }))
                        .unwrap_or(list.len())
                }
            };
            let drag = Self {
                profile: profile.to_owned(),
                group: source_group,
                index,
                key,
                is_group: false,
            };
            drag.apply(&mut staged, profile, Target { group: target.group.clone(), slot }, store)?;
            moved += 1;
        }
        *data = staged;
        Ok((moved, skipped))
    }
}

#[cfg(test)]
mod history_selection_tests {
    use super::*;

    #[test]
    fn selection_mode_and_clear_are_session_only() {
        let ctx = egui::Context::default();
        Drag::set_selection_mode(&ctx, "A", true);
        Drag::selection_click(&ctx, "A", "one");
        Drag::selection_click(&ctx, "A", "two");
        assert_eq!(Drag::selected_ids(&ctx, "A").len(), 2);
        Drag::selection_click(&ctx, "A", "one");
        assert_eq!(Drag::selected_ids(&ctx, "A").len(), 1);
        Drag::clear_selected(&ctx, "A");
        assert!(Drag::selection_mode(&ctx, "A"));
        assert!(Drag::selected_ids(&ctx, "A").is_empty());
        Drag::set_selection_mode(&ctx, "A", false);
        assert!(!Drag::selection_mode(&ctx, "A"));
    }

    #[test]
    fn dragging_includes_selected_collapsed_members() {
        let ctx = egui::Context::default();
        Drag::set_selection_mode(&ctx, "A", true);
        Drag::selection_click(&ctx, "A", "visible");
        Drag::selection_click(&ctx, "A", "hidden");
        Drag::register_visible(&ctx, "A", "visible");
        Drag::selection_drag_started(&ctx, "A", "visible", false);
        assert_eq!(Drag::read_selection(&ctx, |m| m["A"].dragging.clone()), vec![String::from("visible"), String::from("hidden")]);
    }

    #[test]
    fn history_restore_clears_drag_selection_across_profiles() {
        let ctx = egui::Context::default();
        Drag::update_selection(&ctx, |map| {
            for profile in ["A", "B"] {
                let state = map.entry(profile.to_owned()).or_default();
                state.selected.insert("mod-id".into());
                state.anchor = Some("mod-id".into());
                state.visible.push("mod-id".into());
                state.previous_visible.push("mod-id".into());
                state.dragging.push("mod-id".into());
            }
        });
        assert!(Drag::is_selected(&ctx, "A", "mod-id"));
        Drag::clear_history_selection(&ctx);
        assert!(!Drag::is_selected(&ctx, "A", "mod-id"));
        assert!(!Drag::is_selected(&ctx, "B", "mod-id"));
        assert!(Drag::read_selection(&ctx, |map| map.is_empty()));
    }
}
