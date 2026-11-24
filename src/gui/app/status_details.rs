use super::*;
// The local-file installation blocker can contain both a contextual entry
// (path + profile/group) and the same path again as a plain row.  Keep the
// contextual entry; suppress only the redundant plain row in status details.
// Do not change other status/error reports or the saved installation state.
fn missing_local_path_key(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if !raw.to_ascii_lowercase().ends_with(".pak") && !raw.to_ascii_lowercase().ends_with(".zip") {
        return None;
    }
    let raw = raw.strip_prefix(r"\\?\").unwrap_or(raw);
    Some(raw.replace('/', "\\").to_ascii_lowercase())
}

fn compact_missing_local_file_details(details: &str) -> String {
    if !details.starts_with("Installation blocked:")
        || !(details.contains("enabled local file is missing")
            || details.contains("enabled local files are missing"))
    {
        return details.to_owned();
    }

    let mut result = Vec::new();
    let mut preceding_context_path: Option<String> = None;
    for line in details.split('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            result.push(line);
            continue;
        }
        // Only suppress a bare path directly after its contextual entry,
        // allowing for a blank line between the two formats.
        if let Some(previous) = preceding_context_path.take()
            && missing_local_path_key(trimmed).as_deref() == Some(previous.as_str())
        {
            if result
                .last()
                .is_some_and(|last: &&str| last.trim().is_empty())
            {
                result.pop();
            }
            continue;
        }
        preceding_context_path = trimmed
            .split_once(" — profile ")
            .or_else(|| trimmed.split_once(" — group "))
            .and_then(|(path, _)| missing_local_path_key(path));
        result.push(line);
    }
    result.join("\n")
}

impl App {
    pub(super) fn frame_status_details(&mut self, ctx: &egui::Context) {
        let mut show = None;
        let mut refresh_cache = false;
        if self.status_details.is_some() {
            let store = &self.state.store;
            unavailable_mods::check_downloads(&mut self.status_locations, false, |spec| {
                store.has_cached_modio_download(spec)
            });
        }
        if let Some(details) = self.status_details.as_ref() {
            let compacted_details = compact_missing_local_file_details(details);
            let mut open = true;
            let busy = self.integrate_rid.is_some()
                || self.update_rid.is_some()
                || self.lint_rid.is_some()
                || self.resolve_mod_rid.is_some()
                || self.pending_group_files > 0;
            egui::Window::new(if self.live_unavailable_details { "Unavailable mods" } else { "Status details" })
                .id(egui::Id::new("mod_status_details"))
                .open(&mut open)
                .resizable(true)
                .default_width(540.0)
                .show(ctx, |ui| {
                    if ui.button("Copy message").clicked() {
                        ui.ctx().copy_text(unavailable_mods::copy_details(&compacted_details, &self.status_locations));
                    }
                    if !self.status_locations.is_empty() {
                        refresh_cache = ui.button("Refresh cache status")
                            .on_hover_text("Check the listed versions on disk again. Does not download anything.")
                            .clicked();
                    }
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(400.0)
                        .show(ui, |ui| {
                            let report = unavailable_mods::describe(&self.status_locations);
                            let (before, after) = if self.status_locations.is_empty() {
                                (compacted_details.as_str(), "")
                            } else {
                                compacted_details.split_once(&report).unwrap_or((compacted_details.as_str(), ""))
                            };
                            ui.add(egui::Label::new(before.trim()).wrap());
                            if !self.status_locations.is_empty() {
                                ui.separator();
                                ui.strong("Unavailable mod locations");
                            }
                            for location in &self.status_locations {
                                let target = location.target.as_ref()
                                    .and_then(|target| target.resolve(&self.state.mod_data));
                                ui.horizontal(|ui| {
                                    let hint = if busy {
                                        "Wait for the current operation to finish."
                                    } else if location.target.is_none() {
                                        "This group is not used by any profile. Add it to a profile first."
                                    } else if target.is_none() {
                                        "This entry is no longer at its saved location. Update Cache to refresh the list."
                                    } else {
                                        "Open this profile and group, then scroll to the mod."
                                    };
                                    if ui.add_enabled(!busy && target.is_some(), egui::Button::new("Show mod"))
                                        .on_hover_text(hint).clicked() {
                                        show = target;
                                    }
                                    ui.vertical(|ui| {
                                        ui.add(egui::Label::new(&location.label).wrap());
                                        let color = if location.is_cached() {
                                            ui.visuals().text_color()
                                        } else { ui.visuals().warn_fg_color };
                                        ui.colored_label(color, location.cache_status())
                                            .on_hover_text("Checks the version listed here. Cached files still undergo the normal installation checks.");
                                    });
                                });
                            }
                            if !after.trim().is_empty() {
                                ui.separator();
                                ui.add(egui::Label::new(after.trim()).wrap());
                            }
                        });
                });
            if !open {
                self.status_details = None;
                self.status_locations.clear();
                self.live_unavailable_details = false;
            }
        }
        if refresh_cache {
            let store = &self.state.store;
            unavailable_mods::check_downloads(&mut self.status_locations, true, |spec| {
                store.has_cached_modio_download(spec)
            });
            ctx.request_repaint();
        }
        if let Some(target) = show {
            let previous = self.state.mod_data.active_profile.clone();
            self.state.mod_data.active_profile = target.profile.clone();
            if let Err(error) = self.state.mod_data.save() {
                self.state.mod_data.active_profile = previous;
                self.status_details = Some(format!("Could not save profile selection: {error}"));
                self.status_locations.clear();
                self.live_unavailable_details = false;
                return;
            }
            self.edit_history
                .reset(&self.state.mod_data, self.state.mod_data.revision());
            // Use the normal search highlighting, while retaining the exact-ID
            // scroll target so duplicate names cannot redirect navigation.
            let member =
                if let Some(group) = &target.group {
                    self.state.mod_data.groups.get(group).and_then(|group| {
                        group.mods.iter().find(|member| member.id == target.mod_id)
                    })
                } else {
                    self.state
                        .mod_data
                        .profiles
                        .get(&target.profile)
                        .and_then(|profile| {
                            profile.mods.iter().find_map(|entry| match entry {
                                ModOrGroup::Individual(member) if member.id == target.mod_id => {
                                    Some(member)
                                }
                                _ => None,
                            })
                        })
                };
            self.search_string = member
                .map(|member| {
                    self.state
                        .store
                        .get_mod_info(&member.spec)
                        .map(|info| info.name)
                        .unwrap_or_else(|| member.spec.url.clone())
                })
                .unwrap_or_default();
            self.scroll_to_match = false;
            if let Some(group) = &target.group {
                self.groups_to_open
                    .insert((target.profile.clone(), group.clone()));
            }
            self.show_mod_target = Some(target);
            self.status_details = None;
            self.status_locations.clear();
            self.live_unavailable_details = false;
            ctx.request_repaint();
        }
    }
}

impl App {
    pub(super) fn refresh_unavailable_overview(&mut self) {
        let data = &self.state.mod_data;
        let store = &self.state.store;
        let changed =
            self.unavailable_overview
                .refresh((data.revision(), store.metadata_revision()), || {
                    unavailable_mods::entries(
                        data,
                        |spec| store.is_modio_unavailable(spec),
                        |spec| {
                            store
                                .get_mod_info(spec)
                                .map(|info| info.name)
                                .unwrap_or_else(|| spec.url.clone())
                        },
                    )
                });
        if changed && self.live_unavailable_details {
            self.open_unavailable_overview();
        }
    }

    fn open_unavailable_overview(&mut self) {
        self.status_details = Some(self.unavailable_overview.description());
        self.status_locations = self.unavailable_overview.entries.clone();
        self.live_unavailable_details = true;
    }

    pub(super) fn unavailable_overview_button(&mut self, ui: &mut egui::Ui) {
        let count = self.unavailable_overview.entries.len();
        if count == 0 {
            return;
        }
        ui.horizontal(|ui| {
            let hint = "Click to see which mods are unavailable on mod.io.";
            let icon = icon_buttons::mod_error_button(ui).on_hover_text(hint);
            let label = ui
                .add(
                    egui::Label::new(format!(
                        "{count} unavailable {}",
                        if count == 1 { "entry" } else { "entries" }
                    ))
                    .sense(egui::Sense::click()),
                )
                .on_hover_text(hint);
            if icon.clicked() || label.clicked() {
                self.open_unavailable_overview();
                ui.ctx().request_repaint();
            }
        });
    }
}

#[cfg(test)]
mod missing_local_status_compaction_tests {
    use super::compact_missing_local_file_details;

    #[test]
    fn removes_plain_paths_but_preserves_profile_and_group() {
        let original = concat!(
            "Installation blocked: 2 enabled local files are missing.\n\n",
            "Relocate, disable or replace the files.\n\n",
            "\\\\?\\C:\\mods\\alpha.pak — profile \"test\", group \"t\"\n\n",
            "C:\\mods\\alpha.pak\n\n",
            "C:\\mods\\beta.zip — profile \"test\", group \"t\"\n\n",
            "C:\\mods\\beta.zip"
        );
        let expected = concat!(
            "Installation blocked: 2 enabled local files are missing.\n\n",
            "Relocate, disable or replace the files.\n\n",
            "\\\\?\\C:\\mods\\alpha.pak — profile \"test\", group \"t\"\n\n",
            "C:\\mods\\beta.zip — profile \"test\", group \"t\""
        );
        assert_eq!(compact_missing_local_file_details(original), expected);
    }

    #[test]
    fn preserves_different_files_and_multiple_profile_occurrences() {
        let original = concat!(
            "Installation blocked: 3 enabled local files are missing.\n\n",
            "C:\\mods\\alpha.pak — profile \"one\"\n\n",
            "C:\\mods\\beta.pak\n\n",
            "C:\\mods\\alpha.pak — profile \"two\"\n\n",
            "C:\\mods\\alpha.pak"
        );
        let expected = concat!(
            "Installation blocked: 3 enabled local files are missing.\n\n",
            "C:\\mods\\alpha.pak — profile \"one\"\n\n",
            "C:\\mods\\beta.pak\n\n",
            "C:\\mods\\alpha.pak — profile \"two\""
        );
        assert_eq!(compact_missing_local_file_details(original), expected);
    }

    #[test]
    fn removes_duplicate_for_single_missing_local_file() {
        let original = concat!(
            "Installation blocked: 1 enabled local file is missing.\n\n",
            "Relocate, disable or replace these local mod entries, then retry. Your saved entries and existing installation have not been changed.\n\n",
            "\\\\?\\C:\\mods\\CustomMissionSelector_S6.pak — profile \"test3\"\n\n",
            "\\\\?\\C:\\mods\\CustomMissionSelector_S6.pak"
        );
        let expected = concat!(
            "Installation blocked: 1 enabled local file is missing.\n\n",
            "Relocate, disable or replace these local mod entries, then retry. Your saved entries and existing installation have not been changed.\n\n",
            "\\\\?\\C:\\mods\\CustomMissionSelector_S6.pak — profile \"test3\""
        );
        assert_eq!(compact_missing_local_file_details(original), expected);
    }

    #[test]
    fn also_removes_duplicate_without_blank_line() {
        let original = "Installation blocked: 1 enabled local file is missing.\n\nC:\\mods\\a.pak — profile \"x\"\nC:\\mods\\a.pak";
        let expected = "Installation blocked: 1 enabled local file is missing.\n\nC:\\mods\\a.pak — profile \"x\"";
        assert_eq!(compact_missing_local_file_details(original), expected);
    }

    #[test]
    fn leaves_unrelated_status_messages_unchanged() {
        let original = "Cache updated.\n\nC:\\mods\\a.pak — profile \"x\"\n\nC:\\mods\\a.pak";
        assert_eq!(compact_missing_local_file_details(original), original);
    }

    #[test]
    fn is_idempotent() {
        let original = "Installation blocked: 1 enabled local file is missing.\n\nC:\\mods\\a.pak — profile \"x\"\n\nC:\\mods\\a.pak";
        let compacted = compact_missing_local_file_details(original);
        assert_eq!(compact_missing_local_file_details(&compacted), compacted);
    }
}
