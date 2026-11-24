//! Missing local file diagnostics for imported profiles and groups.
//! Scan on import and before an installation, not every idle GUI frame.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::state::{ModData_v0_1_0 as Data, ModGroup, ModOrGroup};

#[derive(Clone, Debug)]
pub(super) enum Origin {
    Profile(String),
    Group(String),
}

#[derive(Clone, Debug)]
struct Notice {
    origin: Origin,
    paths: Vec<String>,
    error: Option<String>,
    scan_error: Option<String>,
    relinked: Vec<(String, String)>,
    auto_count: usize,
    ambiguous: BTreeMap<String, usize>,
    scan: Scan,
}

#[derive(Clone, Debug)]
enum Scan {
    Pending,
    Running {
        folder: PathBuf,
        result: Arc<Mutex<Option<Result<Matches, String>>>>,
    },
    Finished {
        folder: PathBuf,
    },
}

type Matches = BTreeMap<String, Vec<PathBuf>>;

/// Match by filename, never by partial name, and restrict automatic recovery
/// to local mod archives. Folder references remain manual relocation cases.
fn filename_key(value: &str) -> Option<String> {
    let filename = value.rsplit(['/', '\\']).next()?.trim();
    let extension = Path::new(filename).extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("pak") || extension.eq_ignore_ascii_case("zip") {
        Some(filename.to_lowercase())
    } else {
        None
    }
}

/// Scan the configured folder only when triggered by an import. Work is run
/// on a worker thread, never in the egui frame callback. Avoid symlink/junction
/// traversals and collect only the filenames actually requested by this import.
fn find_in_mod_folder(root: &Path, wanted: &BTreeSet<String>) -> Result<Matches, String> {
    if !root.is_dir() {
        return Err(format!(
            "The Settings local mods folder is not accessible: {}",
            root.display()
        ));
    }
    let mut matches = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if folder == root => {
                return Err(format!("Cannot read {}: {error}", folder.display()));
            }
            Err(error) => {
                tracing::warn!(
                    "Skipping unreadable mod folder {}: {error}",
                    folder.display()
                );
                continue;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                let Some(key) = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(filename_key)
                else {
                    continue;
                };
                if wanted.contains(&key) {
                    matches.entry(key).or_insert_with(Vec::new).push(path);
                }
            }
        }
    }
    // Deterministic order makes ambiguous results easy to inspect/test.
    for paths in matches.values_mut() {
        paths.sort();
    }
    Ok(matches)
}

/// A filename alone cannot prove file identity, but exactly one same-name
/// archive in the user-selected mods folder is a useful conservative match.
/// Never pick arbitrarily when multiple files share that name.
fn apply_matches(notice: &mut Notice, data: &mut Data, matches: &Matches) -> bool {
    let mut changed = false;
    notice.ambiguous.clear();
    for old in notice.paths.clone() {
        let Some(key) = filename_key(&old) else {
            continue;
        };
        let Some(candidates) = matches.get(&key) else {
            continue;
        };
        match candidates.as_slice() {
            [candidate] => {
                if relocate(data, &notice.origin, &old, candidate).is_ok() {
                    notice
                        .relinked
                        .push((old.clone(), candidate.display().to_string()));
                    notice.auto_count += 1;
                    notice.paths.retain(|path| path != &old);
                    changed = true;
                }
            }
            [_, _, ..] => {
                notice.ambiguous.insert(old, candidates.len());
            }
            [] => {}
        }
    }
    changed
}

/// Start or poll one scan for the active import. A changed Settings path
/// invalidates any in-flight result. No scanning happens in idle frames.
fn resolve_from_settings(
    ctx: &egui::Context,
    notice: &mut Notice,
    data: &mut Data,
    configured_folder: Option<&Path>,
) -> bool {
    if notice.paths.is_empty() {
        return false;
    }
    let Some(folder) = configured_folder else {
        notice.scan = Scan::Pending;
        return false;
    };
    if !folder.is_dir() {
        notice.scan = Scan::Pending;
        notice.scan_error = Some(format!(
            "Configured local mods folder is unavailable: {}",
            folder.display()
        ));
        return false;
    }
    let folder = folder.to_path_buf();
    let stale = match &notice.scan {
        Scan::Pending => false,
        Scan::Running { folder: active, .. } | Scan::Finished { folder: active } => {
            active != &folder
        }
    };
    if stale {
        notice.scan = Scan::Pending;
    }
    if matches!(&notice.scan, Scan::Pending) {
        let wanted: BTreeSet<String> = notice
            .paths
            .iter()
            .filter_map(|path| filename_key(path))
            .collect();
        if wanted.is_empty() {
            notice.scan = Scan::Finished { folder };
            return false;
        }
        notice.scan_error = None;
        let result = Arc::new(Mutex::new(None));
        let worker_result = Arc::clone(&result);
        let worker_folder = folder.clone();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let found = find_in_mod_folder(&worker_folder, &wanted);
            if let Ok(mut slot) = worker_result.lock() {
                *slot = Some(found);
            }
            worker_ctx.request_repaint();
        });
        notice.scan = Scan::Running { folder, result };
        return false;
    }
    let Scan::Running { folder, result } = &notice.scan else {
        return false;
    };
    let finished_folder = folder.clone();
    let outcome = result.lock().ok().and_then(|mut slot| slot.take());
    let Some(outcome) = outcome else { return false };
    notice.scan = Scan::Finished {
        folder: finished_folder,
    };
    match outcome {
        Ok(matches) => apply_matches(notice, data, &matches),
        Err(error) => {
            notice.scan_error = Some(error);
            false
        }
    }
}

const NOTICE_ID: &str = "mintfixed-import-missing-local-files";

// These are deliberately recognizable filesystem paths, not arbitrary invalid
// provider strings. In particular, HTTP downloads ending in .pak are NOT local.
fn looks_like_local_path(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.contains("://") {
        return false;
    }
    let bytes = value.as_bytes();
    let windows_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let rooted = value.starts_with("\\\\")
        || value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with(".\\")
        || value.starts_with("..\\");
    let extension = Path::new(value).extension().and_then(|ext| ext.to_str());
    let mod_file = extension
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip"));
    windows_absolute || rooted || mod_file
}

pub(super) fn is_missing_local_path(value: &str) -> bool {
    looks_like_local_path(value) && !Path::new(value).exists()
}

pub(super) fn group_missing_paths(group: &ModGroup) -> Vec<String> {
    group
        .mods
        .iter()
        .map(|member| member.spec.url.as_str())
        .filter(|path| is_missing_local_path(path))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn profile_missing_paths(data: &Data, profile_name: &str) -> Vec<String> {
    let Some(profile) = data.profiles.get(profile_name) else {
        return Vec::new();
    };
    let mut missing = BTreeSet::new();
    for entry in &profile.mods {
        match entry {
            ModOrGroup::Individual(member) => {
                if is_missing_local_path(&member.spec.url) {
                    missing.insert(member.spec.url.clone());
                }
            }
            ModOrGroup::Group { group_name, .. } => {
                if let Some(group) = data.groups.get(group_name) {
                    missing.extend(group_missing_paths(group));
                }
            }
        }
    }
    missing.into_iter().collect()
}

pub(super) fn notify(ctx: &egui::Context, origin: Origin, paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new(NOTICE_ID),
            Notice {
                origin,
                paths,
                error: None,
                scan_error: None,
                relinked: Vec::new(),
                auto_count: 0,
                ambiguous: BTreeMap::new(),
                scan: Scan::Pending,
            },
        );
    });
    ctx.request_repaint();
}

/// Change only entries in the just-imported profile or group. All other
/// profiles, groups, IDs, priorities, and enabled/member overrides are intact.
fn relocate(
    data: &mut Data,
    origin: &Origin,
    old_path: &str,
    new_path: &Path,
) -> Result<usize, String> {
    let is_mod_file = Path::new(old_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip"));
    if is_mod_file && !new_path.is_file() {
        return Err("Choose an existing .pak or .zip file.".into());
    }
    if !is_mod_file && !new_path.exists() {
        return Err("Choose an existing local folder or file.".into());
    }
    let updated = new_path.to_string_lossy().to_string();
    if is_mod_file
        && !Path::new(&updated)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip"))
    {
        return Err("The replacement must be a .pak or .zip file.".into());
    }
    fn update_members(mods: &mut [crate::state::ModConfig], old: &str, new: &str) -> usize {
        let mut changed = 0;
        for member in mods {
            if member.spec.url == old {
                member.spec.url = new.to_owned();
                changed += 1;
            }
        }
        changed
    }
    let mut count = 0;
    match origin {
        Origin::Profile(name) => {
            let profile = data
                .profiles
                .get_mut(name)
                .ok_or("The imported profile was removed or renamed")?;
            let names: BTreeSet<_> = profile
                .mods
                .iter()
                .filter_map(|entry| match entry {
                    ModOrGroup::Group { group_name, .. } => Some(group_name.clone()),
                    _ => None,
                })
                .collect();
            for entry in &mut profile.mods {
                if let ModOrGroup::Individual(member) = entry
                    && member.spec.url == old_path
                {
                    member.spec.url.clone_from(&updated);
                    count += 1;
                }
            }
            for name in names {
                if let Some(group) = data.groups.get_mut(&name) {
                    count += update_members(&mut group.mods, old_path, &updated);
                }
            }
        }
        Origin::Group(name) => {
            let group = data
                .groups
                .get_mut(name)
                .ok_or("The imported group was removed or renamed")?;
            count += update_members(&mut group.mods, old_path, &updated);
        }
    }
    if count == 0 {
        Err("No matching imported entry remains. It may have been edited or removed.".into())
    } else {
        Ok(count)
    }
}

/// Returns true only when a saved URL was changed and the caller must save it.
pub(super) fn show(ctx: &egui::Context, data: &mut Data, configured_folder: Option<&Path>) -> bool {
    let key = egui::Id::new(NOTICE_ID);
    let Some(mut notice) = ctx.data(|memory| memory.get_temp::<Notice>(key)) else {
        return false;
    };
    let mut changed = resolve_from_settings(ctx, &mut notice, data, configured_folder);
    let mut open = true;
    let mut dismiss = false;
    let title = match &notice.origin {
        Origin::Profile(_) => "Imported profile: missing local files",
        Origin::Group(_) => "Imported group: missing local files",
    };
    egui::Window::new(title)
        .id(key.with("window"))
        .collapsible(false)
        .resizable(true)
        .default_width(490.0)
        .open(&mut open)
        .show(ctx, |ui| {
            let (kind, name) = match &notice.origin {
                Origin::Profile(name) => ("profile", name.as_str()),
                Origin::Group(name) => ("group", name.as_str()),
            };
            let total = notice.paths.len() + notice.relinked.len();
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "Imported {kind} \"{name}\": {total} missing local {} detected.",
                    if total == 1 { "file" } else { "files" }
                ),
            );
            ui.label("Original paths were missing. Entries, IDs, priorities, and enabled settings are preserved.");
            if !notice.relinked.is_empty() {
                ui.label(format!("Reconnected {} file(s), including {} automatically from the configured local mods folder.", notice.relinked.len(), notice.auto_count));
                if notice.auto_count > 0 {
                    ui.small("Automatic matches use the exact filename (case-insensitive). Verify the file is the intended mod if different versions share its name.");
                }
            }
            if !notice.paths.is_empty() {
                match &notice.scan {
                    Scan::Running { .. } => { ui.spinner(); ui.label("Searching Settings -> Local mods folder and subfolders..."); }
                    Scan::Pending if configured_folder.is_none() => { ui.small("Set Settings -> Local mods folder to search automatically, or choose Relocate."); }
                    _ => {}
                }
                ui.small("Files without a unique match remain unchanged. Select Relocate to fix them manually; nothing is deleted.");
            }
            if let Some(error) = &notice.scan_error {
                ui.colored_label(ui.visuals().warn_fg_color, error);
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .show(ui, |ui| {
                    // Clone because a successful relocation removes a row in the same frame.
                    for old in notice.paths.clone() {
                        if let Some(count) = notice.ambiguous.get(&old) {
                            ui.colored_label(ui.visuals().warn_fg_color,
                                format!("{count} files share this filename in the configured folder. Select the correct one manually."));
                        }
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(egui::RichText::new(&old).monospace()).wrap());
                            if ui.button("Relocate...").clicked() {
                                let extension = Path::new(&old)
                                    .extension()
                                    .and_then(|ext| ext.to_str());
                                let file_expected = extension.is_some_and(|ext| {
                                    ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip")
                                });
                                let replacement = if file_expected {
                                    let mut dialog = rfd::FileDialog::new()
                                        .add_filter("Local mods", &["pak", "zip"]);
                                    if let Some(folder) = configured_folder
                                        && folder.is_dir() {
                                            dialog = dialog.set_directory(folder);
                                        }
                                    dialog.pick_file()
                                } else {
                                    let mut dialog = rfd::FileDialog::new();
                                    if let Some(folder) = configured_folder
                                        && folder.is_dir() {
                                            dialog = dialog.set_directory(folder);
                                        }
                                    dialog.pick_folder()
                                };
                                if let Some(replacement) = replacement {
                                    match relocate(data, &notice.origin, &old, &replacement) {
                                        Ok(_) => {
                                            notice.paths.retain(|path| path != &old);
                                            notice.ambiguous.remove(&old);
                                            notice.relinked.push((old.clone(), replacement.display().to_string()));
                                            notice.error = None;
                                            changed = true;
                                        }
                                        Err(error) => notice.error = Some(error),
                                    }
                                }
                            }
                        });
                        ui.separator();
                    }
                });
            if !notice.relinked.is_empty() {
                ui.collapsing("Relinked file locations", |ui| {
                    for (old, new) in &notice.relinked {
                        ui.label(format!("{} -> {}", old, new));
                    }
                });
            }
            if let Some(error) = &notice.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            ui.horizontal(|ui| {
                if !notice.paths.is_empty()
                    && matches!(&notice.scan, Scan::Finished { .. })
                    && ui.button("Search local mods folder again").clicked()
                {
                    notice.scan = Scan::Pending;
                    notice.ambiguous.clear();
                    notice.scan_error = None;
                    ui.ctx().request_repaint();
                }
                if ui.button("Copy missing paths").clicked() {
                    ui.ctx().copy_text(notice.paths.join("\n"));
                }
                if ui.button("Dismiss").clicked() {
                    dismiss = true;
                }
            });
        });
    if !open || dismiss {
        ctx.data_mut(|memory| memory.remove::<Notice>(key));
    } else {
        ctx.data_mut(|memory| memory.insert_temp(key, notice));
    }
    changed
}

#[cfg(test)]
#[path = "missing_local_files/tests.rs"]
mod tests;
