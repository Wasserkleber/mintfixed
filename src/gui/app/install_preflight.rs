use crate::gui::*;

impl App {
    /// A cache-only check before starting either manual or post-cache installation.
    /// Never silently skip mods or change the user's selection.
    pub(in crate::gui) fn preflight_unavailable_mods(&mut self, after_cache_update: bool) -> bool {
        let data = &self.state.mod_data;
        let store = &self.state.store;
        // Nonexistent local files should never be discovered only after an
        // installation has started. Disabled entries are not blockers.
        let missing_files = unavailable_mods::install_blockers(
            data,
            &data.active_profile,
            |spec| missing_local_files::is_missing_local_path(&spec.url),
            |spec| spec.url.clone(),
        );
        if !missing_files.is_empty() {
            let summary = format!(
                "Installation blocked: {} enabled local {} missing",
                missing_files.len(),
                if missing_files.len() == 1 {
                    "file is"
                } else {
                    "files are"
                }
            );
            let details = format!(
                "{summary}.\n\nRelocate, disable or replace these local mod entries, then retry. Your saved entries and existing installation have not been changed.\n\n{}",
                missing_files
                    .iter()
                    .map(|location| location.label.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n")
            );
            let mut action = LastAction::failure(summary);
            action.details = Some(details.clone());
            self.last_action = Some(action);
            self.status_details = Some(details);
            self.status_locations.clear();
            self.live_unavailable_details = false;
            self.problematic_mod_id = None;
            return false;
        }
        let locations = unavailable_mods::install_blockers(
            data,
            &data.active_profile,
            |spec| store.is_modio_unavailable(spec) && !store.has_cached_modio_download(spec),
            |spec| {
                store
                    .get_mod_info(spec)
                    .map(|info| info.name)
                    .unwrap_or_else(|| spec.url.clone())
            },
        );
        if locations.is_empty() {
            return true;
        }
        let summary = format!(
            "Installation blocked: {} enabled unavailable {} without a cached download",
            locations.len(),
            if locations.len() == 1 {
                "entry"
            } else {
                "entries"
            }
        );
        let previous = if after_cache_update {
            self.last_action.as_ref().map(|action| {
                action
                    .details
                    .clone()
                    .unwrap_or_else(|| match &action.status {
                        LastActionStatus::Success(message) | LastActionStatus::Failure(message) => {
                            message.clone()
                        }
                    })
            })
        } else {
            None
        };
        let mut details = format!(
            "{summary}.\n\nThe selected versions cannot be installed from the cache. Use Show mod to disable or replace these entries, or run Update Cache to check whether they are available again. Your existing installation has not been changed.\n\n{}",
            unavailable_mods::describe(&locations)
        );
        if let Some(previous) = previous {
            details.push_str(&format!("\n\nCache update result:\n{previous}"));
        }
        let mut action = LastAction::failure(summary);
        action.details = Some(details.clone());
        action.unavailable_locations = locations.clone();
        self.last_action = Some(action);
        self.status_details = Some(details);
        self.status_locations = locations;
        self.live_unavailable_details = false;
        self.problematic_mod_id = None;
        false
    }
}
