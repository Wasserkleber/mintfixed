//! Global bitfix Lua management UI, independent of ordinary mod profiles.
use super::*;

pub(super) const REDIRECT_HINT: &str = "Bitfix Lua scripts cannot be added here. Open the Bitfix menu using the button in the bottom-left corner.";

#[derive(Default)]
pub(super) struct BitfixMenu {
    pub(super) open: bool,
    pub(super) input: String,
    /// Tracks the target through the final native drop frame.
    drop_hovered: bool,
    // Keep a snapshot so the confirmation cannot delete a DLL silently changed in the meantime.
    pub(super) pending_conflict: Option<(PathBuf, Vec<u8>)>,
}

pub(super) fn is_lua_spec(spec: &ModSpecification) -> bool {
    spec.url
        .split(['?', '#'])
        .next()
        .unwrap_or(&spec.url)
        .to_ascii_lowercase()
        .ends_with(".lua")
}

pub(super) fn toolbar_button(ui: &mut Ui) -> bool {
    let icon = egui::Image::new(egui::include_image!("../../assets/icons/bitfix.svg"))
        .fit_to_exact_size(egui::vec2(16.0, 16.0))
        .tint(ui.visuals().text_color());
    ui.add(egui::Button::image(icon))
        .on_hover_text("Manage Bitfix Lua scripts")
        .clicked()
}

fn conflict_for_active(state: &State) -> Result<Option<PathBuf>, String> {
    let profile = &state.mod_data.active_profile;
    if !state
        .mod_data
        .ordered_install_entries(profile)
        .iter()
        .any(|entry| is_lua_spec(&entry.spec))
    {
        return Ok(None);
    }
    let pak = state
        .config
        .drg_pak_path
        .as_ref()
        .ok_or("DRG installation is not configured")?;
    let installation = mint_lib::DRGInstallation::from_pak_path(pak)
        .map_err(|e| format!("Failed to locate DRG for Bitfix: {e}"))?;
    crate::bitfix::conflicting_proxy(&installation)
        .map_err(|e| format!("Unable to inspect existing d3d9.dll: {e}"))
}

/// Validate an entire OS drop before importing any scripts.
/// Mixed file types and non-local drops must not result in partial additions.
fn dropped_lua_paths(files: &[egui::DroppedFile]) -> Result<Vec<PathBuf>, &'static str> {
    files
        .iter()
        .map(|file| {
            let path = file.path.as_ref().ok_or(
                "Only local .lua files can be dropped here. Paste a URL into the field instead.",
            )?;
            if !path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lua"))
            {
                return Err(
                    "Only .lua files are accepted in the Bitfix window. No files were added.",
                );
            }
            Ok(path.clone())
        })
        .collect()
}

impl App {
    pub(super) fn preflight_bitfix_proxy(&mut self) -> bool {
        match conflict_for_active(&self.state) {
            Ok(None) => true,
            Ok(Some(path)) => match std::fs::read(&path) {
                Ok(bytes) => {
                    self.bitfix_menu.pending_conflict = Some((path, bytes));
                    self.last_action = Some(LastAction::failure(
                        "Conflicting d3d9.dll detected. Confirm or cancel in the warning window."
                            .into(),
                    ));
                    false
                }
                Err(error) => {
                    self.last_action = Some(LastAction::failure(format!(
                        "Could not inspect d3d9.dll: {error}"
                    )));
                    false
                }
            },
            Err(message) => {
                self.last_action = Some(LastAction::failure(message));
                false
            }
        }
    }

    pub(super) fn start_install_for_active(&mut self, ctx: &egui::Context) {
        if self.integrate_rid.is_some() || self.update_rid.is_some() {
            return;
        }
        if !self.preflight_bitfix_proxy() {
            return;
        }
        let data = &self.state.mod_data;
        let mods = data.ordered_install_entries(&data.active_profile);
        let Some(fsd_pak) = self.state.config.drg_pak_path.clone() else {
            self.last_action = Some(LastAction::failure("DRG install not configured".into()));
            return;
        };
        self.last_action = None;
        self.cache_install_rid = None;
        self.install_phase = message::InstallPhase::Resolving;
        self.integrate_rid = Some(message::Integrate::send(
            &mut self.request_counter,
            self.state.store.clone(),
            mods,
            fsd_pak,
            self.state.config.deref().into(),
            self.tx.clone(),
            ctx.clone(),
        ));
        self.problematic_mod_id = None;
    }

    pub(super) fn show_bitfix_menu(&mut self, ctx: &egui::Context) {
        if !self.bitfix_menu.open {
            return;
        }
        let mut open = self.bitfix_menu.open;
        let mut add = false;
        let mut install = false;
        let mut changed = false;
        let before = self.state.mod_data.deref().deref().clone();
        let window_response = egui::Window::new("Manage Bitfix scripts")
            .open(&mut open)
            .default_width(560.0)
            .resizable(true)
            .show(ctx, |ui| {
                if self.integrate_rid.is_some() || self.update_rid.is_some()
                    || self.resolve_mod_rid.is_some() { ui.disable(); }
                ui.label("Bitfix Lua scripts are shared across all profiles.");
                ui.label("Paste local .lua paths or .lua URLs (one per line):");
                if ui.input(|i| !i.raw.hovered_files.is_empty())
                    && ui.ctx().pointer_hover_pos()
                        .is_some_and(|pos| ui.max_rect().contains(pos)) {
                    ui.colored_label(ui.visuals().selection.stroke.color,
                        "Release to import .lua scripts into the global Bitfix list");
                } else {
                    ui.small("You can also drag and drop .lua files anywhere in this window.");
                }
                ui.add(egui::TextEdit::multiline(&mut self.bitfix_menu.input).desired_rows(1));
                ui.horizontal(|ui| {
                    if ui.add_enabled(
                        self.resolve_mod_rid.is_none() && !self.bitfix_menu.input.trim().is_empty(),
                        Button::new("Add scripts")
                    ).clicked() { add = true; }
                    if ui.button("Browse .lua files").clicked() {
                        if let Some(files) = rfd::FileDialog::new()
                            .add_filter("Bitfix Lua scripts", &["lua"]).pick_files() {
                            self.bitfix_menu.input = files.iter()
                                .map(|f| f.to_string_lossy().into_owned())
                                .collect::<Vec<_>>().join("\n");
                            add = true;
                        }
                    }
                    if ui.add_enabled(
                        self.resolve_mod_rid.is_none() && self.integrate_rid.is_none()
                            && self.update_rid.is_none()
                            && self.state.config.drg_pak_path.is_some(),
                        Button::new("Install mods and Bitfix")
                    ).on_hover_text("Install the active mod profile with globally enabled Bitfix scripts")
                    .clicked() { install = true; }
                });
                ui.separator();
                ui.label(RichText::new("Global Bitfix scripts").strong());
                let mut removed = Vec::new();
                for (index, mc) in self.state.mod_data.bitfix_scripts.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut mc.enabled, "").changed() { changed = true; }
                        ui.label(&mc.spec.url);
                        if ui.button("Remove").clicked() { removed.push(index); }
                    });
                }
                for index in removed.into_iter().rev() {
                    self.state.mod_data.bitfix_scripts.remove(index);
                    changed = true;
                }
                if self.state.mod_data.bitfix_scripts.is_empty() {
                    ui.label("No Bitfix scripts added yet.");
                }
                ui.separator();
                ui.small("Changes apply to every profile. Click Install mods and Bitfix to update DRG. Close DRG first.");
            });
        self.bitfix_menu.open = open;
        if changed {
            if let Err(error) = self.state.mod_data.save() {
                *self.state.mod_data.deref_mut().deref_mut() = before;
                self.last_action = Some(LastAction::failure(format!(
                    "Unable to save Bitfix script changes: {error}"
                )));
            }
        }
        if add {
            let specs: Vec<_> = self
                .bitfix_menu
                .input
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| ModSpecification::new(line.to_owned()))
                .collect();
            message::ResolveMods::send_bitfix(self, ctx, specs);
        }
        if install {
            self.start_install_for_active(ctx);
        }
        // The main mod list processes native drops later in the same frame.
        // Consume a drop over this window first, so it cannot also be imported
        // as a normal profile mod or dropped into a group.
        if let Some(window) = window_response {
            let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
            let pointer_in_window = ctx
                .pointer_hover_pos()
                .map(|pos| window.response.rect.contains(pos));
            if hovering_files {
                if let Some(over) = pointer_in_window {
                    self.bitfix_menu.drop_hovered = over;
                }
            }
            // Windows native file drops may not report pointer_hover_pos().
            // While this menu is open, claim drops before the normal importer.
            let dropped_here = ctx.input(|i| !i.raw.dropped_files.is_empty());
            if dropped_here {
                let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
                self.bitfix_menu.drop_hovered = false;
                if self.resolve_mod_rid.is_some()
                    || self.integrate_rid.is_some()
                    || self.update_rid.is_some()
                {
                    self.last_action = Some(LastAction::failure(
                        "Wait for the current operation before dropping Bitfix scripts.".into(),
                    ));
                } else {
                    match dropped_lua_paths(&dropped) {
                        Ok(paths) if !paths.is_empty() => {
                            // Match Browse .lua files: use the existing global
                            // Bitfix resolver, including its duplicate handling.
                            self.bitfix_menu.input = paths
                                .iter()
                                .map(|path| path.to_string_lossy().into_owned())
                                .collect::<Vec<_>>()
                                .join("\n");
                            let specs = paths
                                .into_iter()
                                .map(|path| {
                                    ModSpecification::new(path.to_string_lossy().into_owned())
                                })
                                .collect();
                            message::ResolveMods::send_bitfix(self, ctx, specs);
                        }
                        Ok(_) => {}
                        Err(error) => {
                            self.last_action = Some(LastAction::failure(error.into()));
                        }
                    }
                }
            } else if !hovering_files {
                self.bitfix_menu.drop_hovered = false;
            }
        } else {
            self.bitfix_menu.drop_hovered = false;
        }
    }

    pub(super) fn show_bitfix_conflict(&mut self, ctx: &egui::Context) {
        let Some((path, original_bytes)) = self.bitfix_menu.pending_conflict.as_ref() else {
            return;
        };
        let mut cancel = false;
        let mut delete = false;
        egui::Window::new("Conflicting Bitfix DLL")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "A different d3d9.dll is already installed. It could belong to another mod or graphics tool.",
                );
                ui.label(path.display().to_string());
                ui.label("Deleting it may disable that tool. This cannot be undone by Mintfixed.");
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() { cancel = true; }
                    if ui.button("Delete old DLL and install").clicked() { delete = true; }
                });
            });
        if cancel {
            self.bitfix_menu.pending_conflict = None;
        } else if delete {
            let result = match std::fs::read(path) {
                Ok(now) if now == *original_bytes => std::fs::remove_file(path)
                    .map_err(|e| format!("Failed to delete conflicting d3d9.dll: {e}")),
                Ok(_) => Err("d3d9.dll changed since the warning opened. Nothing was deleted. Retry installation.".into()),
                Err(e) => Err(format!("Cannot recheck d3d9.dll: {e}")),
            };
            self.bitfix_menu.pending_conflict = None;
            match result {
                Ok(()) => self.start_install_for_active(ctx),
                Err(e) => self.last_action = Some(LastAction::failure(e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_lua_files_validate_as_one_batch() {
        let lua = egui::DroppedFile {
            path: Some(PathBuf::from("C:/mods/fix.LUA")),
            ..Default::default()
        };
        let pak = egui::DroppedFile {
            path: Some(PathBuf::from("C:/mods/other.pak")),
            ..Default::default()
        };
        let unsupported = egui::DroppedFile::default();
        assert_eq!(dropped_lua_paths(&[lua.clone()]).unwrap().len(), 1);
        assert!(dropped_lua_paths(&[lua, pak]).is_err());
        assert!(dropped_lua_paths(&[unsupported]).is_err());
    }

    #[test]
    fn recognizes_bitfix_script_paths_and_urls() {
        for value in [
            "C:\\mods\\fix.lua",
            "/tmp/fix.LUA",
            "https://site.example/fix.lua?download=1",
        ] {
            assert!(is_lua_spec(&ModSpecification::new(value.into())));
        }
        for value in ["C:\\mods\\mod.pak", "https://site.example/fix.lua.zip"] {
            assert!(!is_lua_spec(&ModSpecification::new(value.into())));
        }
    }
}
