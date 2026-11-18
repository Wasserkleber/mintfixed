// Session-only drag selection. It is never written to mod_data.json.
// The handles of visible mods register themselves every frame, which makes
// Shift-click and Ctrl+A respect collapsed groups and current list rendering.
#[derive(Clone, Default)]
struct ModMultiSelection {
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

    pub(in crate::gui) fn begin_selection_frame(ui: &mut Ui, profile: &str, manual: bool) {
        let ctrl_a = manual
            && !ui.ctx().wants_keyboard_input()
            && ui.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::A));
        let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
        Self::update_selection(ui.ctx(), |map| {
            let state = map.entry(profile.to_owned()).or_default();
            // Previous frame represents actually painted handles, not hidden
            // group members or rows from other profiles.
            state.previous_visible = std::mem::take(&mut state.visible);
            if !manual {
                state.selected.clear();
                state.anchor = None;
                state.dragging.clear();
                return;
            }
            if !state.previous_visible.is_empty() {
                state.selected.retain(|id| state.previous_visible.contains(id));
            }
            if escape {
                state.selected.clear();
                state.anchor = None;
            } else if ctrl_a {
                state.selected = state.previous_visible.iter().cloned().collect();
                state.anchor = state.previous_visible.first().cloned();
            }
        });
    }

    fn register_visible(ctx: &egui::Context, profile: &str, key: &str) {
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

    fn selection_click(ctx: &egui::Context, profile: &str, key: &str) {
        let modifiers = ctx.input(|i| i.modifiers);
        Self::update_selection(ctx, |map| {
            let state = map.entry(profile.to_owned()).or_default();
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
            } else if modifiers.ctrl {
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
    }

    fn selection_drag_started(ctx: &egui::Context, profile: &str, key: &str, is_group: bool) {
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
