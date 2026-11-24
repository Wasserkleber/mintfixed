//! Profile export controls and background ZIP worker.
use super::export_files::{ProfileExportKind, portable_profile_export_path, save_profile_export};
use crate::gui::{ModData, ModOrGroup, profile_package};
use std::collections::BTreeMap;

pub(in crate::gui) fn copy_options(
    ui: &mut egui::Ui,
    mod_data: &ModData,
) -> Option<Result<String, String>> {
    // One export state, shared by JSON/TXT and portable ZIP, like Group Export.
    type ExportState = std::sync::Arc<std::sync::Mutex<Option<Result<std::path::PathBuf, String>>>>;
    let job_id = ui.make_persistent_id("portable-profile-export-worker");
    let mut job: Option<ExportState> = ui.data(|d| d.get_temp(job_id));
    let mut feedback: Option<Result<String, String>> = None;

    if let Some(active) = job.clone() {
        let finished = active.lock().ok().and_then(|mut slot| slot.take());
        if let Some(result) = finished {
            job = None;
            feedback = Some(
                result.map(|path| format!("Exported portable profile ZIP to {}", path.display())),
            );
        } else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Packaging local mod files...");
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    ui.label("Choose a profile to export:");
    let profile_id = ui.make_persistent_id("portable-profile-export-target");
    let mut selected: String = ui
        .data(|d| d.get_temp(profile_id))
        .unwrap_or_else(|| mod_data.active_profile.clone());
    if !mod_data.profiles.contains_key(&selected) {
        selected = if mod_data.profiles.contains_key(&mod_data.active_profile) {
            mod_data.active_profile.clone()
        } else {
            mod_data.profiles.keys().next().cloned().unwrap_or_default()
        };
    }
    egui::ComboBox::from_id_salt("portable-profile-target-picker")
        .selected_text(if selected.is_empty() {
            "Choose profile"
        } else {
            selected.as_str()
        })
        .show_ui(ui, |ui| {
            for name in mod_data.profiles.keys() {
                ui.selectable_value(&mut selected, name.clone(), name);
            }
        });
    ui.data_mut(|d| d.insert_temp(profile_id, selected.clone()));

    let format_id = ui.id().with("profile-export-format");
    let mut kind = ui
        .data(|d| d.get_temp::<ProfileExportKind>(format_id))
        .unwrap_or_default();
    let bundle_id = ui.id().with("profile-export-include-local-paks");
    let mut include_local: bool = ui.data(|d| d.get_temp(bundle_id)).unwrap_or(false);

    ui.horizontal(|ui| {
        ui.radio_value(&mut kind, ProfileExportKind::Json, "JSON");
        // Portable profile packages must include a JSON definition to retain
        // groups, member overrides and priority settings on import.
        ui.add_enabled_ui(!include_local, |ui| {
            ui.radio_value(&mut kind, ProfileExportKind::Txt, "TXT");
        });
    });
    ui.checkbox(
        &mut include_local,
        "Include local PAK/ZIP files\n(portable ZIP)",
    );
    if include_local {
        kind = ProfileExportKind::Json;
        ui.label("Includes local mod files and preserves profile settings and groups.");
    } else {
        match kind {
            ProfileExportKind::Json => {
                ui.label("Preserves mod order, enabled states, settings and groups.");
            }
            ProfileExportKind::Txt => {
                ui.label("Exports enabled mod URLs only (one per line).");
            }
        }
    }
    ui.data_mut(|d| {
        d.insert_temp(format_id, kind);
        d.insert_temp(bundle_id, include_local);
    });

    if ui
        .add_enabled(
            job.is_none() && !selected.is_empty(),
            crate::gui::mint_text_button("Export profile"),
        )
        .clicked()
    {
        if include_local {
            // Same worker, integrity checks and no-overwrite logic as the
            // existing portable-profile ZIP exporter; only its UI changes.
            let path = match portable_profile_export_path(&selected) {
                Ok(path) => Some(path),
                Err(error) => {
                    feedback = Some(Err(error));
                    None
                }
            };
            if let Some(path) = path {
                match profile_package::prepare_export(mod_data, &selected) {
                    Ok(snapshot) => {
                        let state: ExportState = Default::default();
                        let worker_state = std::sync::Arc::clone(&state);
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let result = profile_package::export_prepared(snapshot, &path)
                                .map(|_| path);
                            if let Ok(mut slot) = worker_state.lock() {
                                *slot = Some(result);
                            }
                            ctx.request_repaint();
                        });
                        job = Some(state);
                    }
                    Err(error) => feedback = Some(Err(error)),
                }
            }
        } else {
            let contents = match kind {
                ProfileExportKind::Txt => {
                    let mut urls = String::new();
                    mod_data.for_each_enabled_mod(&selected, |mc| {
                        urls.push_str(&mc.spec.url);
                        urls.push('\n');
                    });
                    Ok(urls)
                }
                ProfileExportKind::Json => {
                    let profile = &mod_data.profiles[&selected];
                    let groups: BTreeMap<&str, &crate::state::ModGroup> = profile
                        .mods
                        .iter()
                        .filter_map(|entry| match entry {
                            ModOrGroup::Group { group_name, .. } => mod_data
                                .groups
                                .get(group_name)
                                .map(|group| (group_name.as_str(), group)),
                            ModOrGroup::Individual(_) => None,
                        })
                        .collect();
                    let payload = serde_json::json!({
                        "format": "mintfixed-profile-mod-list",
                        "version": 1,
                        "profile_name": &selected,
                        "mods": &profile.mods,
                        "groups": groups,
                    });
                    serde_json::to_string_pretty(&payload)
                        .map_err(|error| format!("Could not serialize profile JSON: {error}"))
                }
            };
            feedback = Some(contents.and_then(|contents| {
                // Only copy after the TXT file has been saved successfully.
                let path = save_profile_export(&selected, kind, &contents)?;
                if matches!(kind, ProfileExportKind::Txt) {
                    ui.ctx().copy_text(contents);
                    Ok(format!(
                        "Exported profile \"{selected}\" to {} and copied TXT to clipboard",
                        path.display()
                    ))
                } else {
                    Ok(format!(
                        "Exported profile \"{selected}\" to {}",
                        path.display()
                    ))
                }
            }));
        }
    }

    if let Some(active) = job {
        ui.data_mut(|d| d.insert_temp(job_id, active));
    } else {
        ui.data_mut(|d| d.remove::<ExportState>(job_id));
    }
    feedback
}
