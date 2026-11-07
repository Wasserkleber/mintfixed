//! On-demand local-mod import preview with persistent explicit exclusions.
//! All file-system walking and SHA-256 comparisons run off the egui thread.
use super::*;
use std::collections::{BTreeSet, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

const EXCLUSIONS_FILE: &str = "excluded_mod_imports.json";

#[derive(Default)]
pub(super) struct FolderImport {
    scan: Option<ScanJob>,
    preview: Option<Preview>,
    exclusions_manager: Option<Exclusions>,
}

struct ScanJob {
    rx: Receiver<Result<Vec<PathBuf>, String>>,
    folder: PathBuf,
}

struct Preview {
    mods: Vec<Candidate>,
}

struct Candidate {
    path: PathBuf,
    selected: bool,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Exclusions {
    #[serde(default)]
    excluded_paths: BTreeSet<String>,
}

impl Exclusions {
    fn path(cache_dir: &Path) -> PathBuf {
        cache_dir.join(EXCLUSIONS_FILE)
    }

    fn load(cache_dir: &Path) -> Result<Self, String> {
        match std::fs::read(Self::path(cache_dir)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| format!("Invalid {EXCLUSIONS_FILE}: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Cannot read {EXCLUSIONS_FILE}: {e}")),
        }
    }

    fn save(&self, cache_dir: &Path) -> Result<(), String> {
        // Save by replacement, not in-place truncation; avoid losing exclusions on a crash.
        let mut tmp = tempfile::NamedTempFile::new_in(cache_dir)
            .map_err(|e| format!("Cannot create exclusion file: {e}"))?;
        serde_json::to_writer_pretty(&mut tmp, self)
            .map_err(|e| format!("Cannot serialize exclusions: {e}"))?;
        tmp.flush().map_err(|e| format!("Cannot flush exclusions: {e}"))?;
        tmp.as_file().sync_all().map_err(|e| format!("Cannot sync exclusions: {e}"))?;
        tmp.persist(Self::path(cache_dir))
            .map_err(|e| format!("Cannot save exclusions: {e}"))?;
        Ok(())
    }
}

fn path_key(path: &Path) -> String {
    let value = path.to_string_lossy().to_string();
    if cfg!(windows) { value.to_lowercase() } else { value }
}

fn supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pak") || e.eq_ignore_ascii_case("zip"))
}

fn visit(folder: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(folder)
        .map_err(|e| format!("Cannot scan {}: {e}", folder.display()))?
    {
        let entry = entry.map_err(|e| format!("Cannot read directory entry: {e}"))?;
        let kind = entry.file_type()
            .map_err(|e| format!("Cannot inspect {}: {e}", entry.path().display()))?;
        // Do not follow symbolic links / junctions into unrelated trees.
        if kind.is_symlink() { continue; }
        if kind.is_dir() {
            visit(&entry.path(), paths)?;
        } else if kind.is_file() && supported(&entry.path()) {
            paths.push(entry.path());
        }
    }
    Ok(())
}

fn candidates(
    folder: &Path,
    cache_dir: &Path,
    existing: &[ModSpecification],
) -> Result<Vec<PathBuf>, String> {
    let folder = std::fs::canonicalize(folder)
        .map_err(|e| format!("Invalid local mods folder: {e}"))?;
    if !folder.is_dir() { return Err("The local mods path is not a directory".into()); }
    // Fail closed if exclusions cannot be loaded; never silently import ignored files.
    let exclusions = Exclusions::load(cache_dir)?;
    let mut known_paths = HashSet::new();
    let mut existing_local = Vec::new();
    for spec in existing {
        let path = Path::new(&spec.url);
        if !path.is_file() { continue; }
        if let Ok(path) = std::fs::canonicalize(path) {
            if known_paths.insert(path_key(&path)) { existing_local.push(path); }
        }
    }
    let mut found = Vec::new();
    visit(&folder, &mut found)?;
    found.sort();
    let mut unseen = Vec::new();
    for path in found {
        let Ok(path) = std::fs::canonicalize(path) else { continue };
        let key = path_key(&path);
        if !exclusions.excluded_paths.contains(&key) && known_paths.insert(key) {
            unseen.push(path);
        }
    }
    // The common no-new-files case does not reread gigabytes of already imported paks.
    if unseen.is_empty() { return Ok(Vec::new()); }
    let mut known_hashes = HashSet::new();
    for path in existing_local {
        if let Ok(hash) = crate::mod_identity::local_digest(&path) {
            known_hashes.insert(hash);
        }
    }
    let mut result = Vec::new();
    for path in unseen {
        match crate::mod_identity::local_digest(&path) {
            Ok(hash) if known_hashes.insert(hash) => result.push(path),
            Ok(_) => (),
            Err(e) => tracing::warn!("Skipping local mod {}: {e}", path.display()),
        }
    }
    Ok(result)
}

fn start(app: &mut App, ctx: &egui::Context, folder: PathBuf) -> Result<(), String> {
    if app.folder_import.scan.is_some() {
        return Err("A local-mod folder scan is already running".into());
    }
    let folder = std::fs::canonicalize(&folder)
        .map_err(|error| format!("Cannot access local mods folder: {error}"))?;
    if !folder.is_dir() {
        return Err(format!("Not a folder: {}", folder.display()));
    }
    let cache_dir = app.state.dirs.cache_dir.clone();
    if !Exclusions::path(&cache_dir).exists() {
        Exclusions::default().save(&cache_dir)?;
    }
    let existing = crate::mod_identity::all_specs(&app.state.mod_data);
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    let worker_folder = folder.clone();
    std::thread::spawn(move || {
        let result = candidates(&worker_folder, &cache_dir, &existing);
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    app.folder_import.scan = Some(ScanJob { rx, folder });
    Ok(())
}

pub(super) fn open_preview(app: &mut App, ctx: &egui::Context, folder: PathBuf) {
    if !folder.is_dir() {
        app.last_action = Some(LastAction::failure(format!("Not a folder: {}", folder.display())));
        return;
    }
    if app.folder_import.scan.is_some() {
        app.last_action = Some(LastAction::success("A local-mod scan is already running".into()));
        return;
    }
    // This is the ONLY place that starts a folder scan: opening Import all.
    app.folder_import.preview = None;
    match start(app, ctx, folder) {
        Ok(()) => app.last_action = Some(LastAction::success("Scanning local mods folder...".into())),
        Err(e) => app.last_action = Some(LastAction::failure(e)),
    }
}

pub(super) fn open_exclusions(app: &mut App) {
    match Exclusions::load(&app.state.dirs.cache_dir) {
        Ok(e) => app.folder_import.exclusions_manager = Some(e),
        Err(e) => app.last_action = Some(LastAction::failure(e)),
    }
}

/// Poll a scan initiated explicitly by opening the Import local mods menu.
/// Nothing is scanned at startup, periodically, or after a normal deletion.
pub(super) fn tick(app: &mut App, _ctx: &egui::Context) {
    if let Some(job) = app.folder_import.scan.take() {
        match job.rx.try_recv() {
            Ok(Ok(paths)) => {
                // Discard stale results if the configured folder changed mid-scan.
                if app.state.config.local_mod_folder.as_ref() == Some(&job.folder) {
                    app.folder_import.preview = Some(Preview {
                        mods: paths.into_iter()
                            .map(|path| Candidate { path, selected: true })
                            .collect(),
                    });
                }
            }
            Ok(Err(error)) => app.last_action = Some(LastAction::failure(error)),
            Err(TryRecvError::Empty) => app.folder_import.scan = Some(job),
            Err(TryRecvError::Disconnected) => app.last_action = Some(LastAction::failure(
                "The local-mod folder scan stopped unexpectedly".into(),
            )),
        }
    }
}

pub(super) fn show_preview(app: &mut App, ctx: &egui::Context) {
    let Some(mut preview) = app.folder_import.preview.take() else { return };
    let mut open = true;
    #[derive(Clone, Copy)]
    enum Action { None, Import, Exclude(usize), ExcludeUnchecked, Cancel }
    let mut action = Action::None;
    egui::Window::new("Import local mods")
        .open(&mut open)
        .default_width(680.0)
        .min_width(610.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.label(format!("{} new mod(s) found. Select mods to import. Only explicitly excluded mods are hidden.", preview.mods.len()));
            if preview.mods.is_empty() {
                ui.label("No new mods to import.");
            }
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                for (index, candidate) in preview.mods.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        // Keep selection at the left and a consistent Exclude column on the right.
                        // Only the filename is shortened; the full path stays visible on hover.
                        let full_path = candidate.path.display().to_string();
                        ui.checkbox(&mut candidate.selected, "").on_hover_text(&full_path);
                        let filename = candidate.path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let label_width = (ui.available_width() - 80.0).max(80.0);
                        let label = ui.add_sized(
                            [label_width, 20.0],
                            egui::Label::new(filename)
                                .truncate()
                                .sense(egui::Sense::click()),
                        ).on_hover_text(&full_path);
                        if label.clicked() { candidate.selected = !candidate.selected; }
                        if ui.small_button("Exclude").clicked() {
                            action = Action::Exclude(index);
                        }
                    });
                }
            });
            ui.separator();
            // One footer row; the primary action and Cancel stay right-aligned.
            ui.horizontal(|ui| {
                if ui.button("Select all").clicked() {
                    for item in &mut preview.mods { item.selected = true; }
                }
                if ui.button("Select none").clicked() {
                    for item in &mut preview.mods { item.selected = false; }
                }
                if ui.button("Exclude unchecked").clicked() {
                    action = Action::ExcludeUnchecked;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() { action = Action::Cancel; }
                    if ui.add_enabled(
                        app.resolve_mod_rid.is_none(),
                        egui::Button::new("Import selected"),
                    ).clicked() { action = Action::Import; }
                });
            });
        });
    let mut close = !open;
    match action {
        Action::None => (),
        Action::Cancel => close = true,
        Action::Exclude(index) => {
            let key = path_key(&preview.mods[index].path);
            let cache_dir = &app.state.dirs.cache_dir;
            let res = Exclusions::load(cache_dir).and_then(|mut data| {
                data.excluded_paths.insert(key);
                data.save(cache_dir)
            });
            match res {
                Ok(()) => { preview.mods.remove(index); }
                Err(e) => app.last_action = Some(LastAction::failure(e)),
            }
        }
        Action::ExcludeUnchecked => {
            // Checking a box affects only this import. "Exclude unchecked" is
            // the explicit action that persists ignored paths.
            let cache_dir = &app.state.dirs.cache_dir;
            let result = Exclusions::load(cache_dir).and_then(|mut data| {
                for item in &preview.mods {
                    if !item.selected {
                        data.excluded_paths.insert(path_key(&item.path));
                    }
                }
                data.save(cache_dir)
            });
            match result {
                Ok(()) => {
                    preview.mods.retain(|item| item.selected);
                    app.last_action = Some(LastAction::success("Local mod exclusions saved".into()));
                }
                Err(error) => app.last_action = Some(LastAction::failure(error)),
            }
        }
        Action::Import => {
            // Unselected mods are not excluded; they reappear on the next scan.
            let selected = preview.mods.iter().filter(|item| item.selected)
                .map(|item| ModSpecification::new(item.path.to_string_lossy().to_string()))
                .collect::<Vec<_>>();
            if !selected.is_empty() {
                message::ResolveMods::send(app, ctx, selected, false);
            }
            close = true;
        }
    }
    if !close { app.folder_import.preview = Some(preview); }
}

pub(super) fn show_exclusions(app: &mut App, ctx: &egui::Context) {
    let Some(mut exclusions) = app.folder_import.exclusions_manager.take() else { return };
    let mut open = true;
    let mut remove = None;
    let mut add = None;
    egui::Window::new("Excluded local mods")
        .open(&mut open)
        .default_width(520.0)
        .show(ctx, |ui| {
            if ui.button("Exclude file...").clicked() {
                add = rfd::FileDialog::new()
                    .add_filter("Local mods", &["pak", "zip"])
                    .pick_file();
            }
            if exclusions.excluded_paths.is_empty() {
                ui.label("No excluded local mods.");
            }
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                for path in &exclusions.excluded_paths {
                    ui.horizontal(|ui| {
                        ui.label(path);
                        if ui.small_button("Allow again").clicked() { remove = Some(path.clone()); }
                    });
                }
            });
        });
    if let Some(path) = add {
        match std::fs::canonicalize(path) {
            Ok(path) => {
                let key = path_key(&path);
                exclusions.excluded_paths.insert(key.clone());
                if let Err(e) = exclusions.save(&app.state.dirs.cache_dir) {
                    exclusions.excluded_paths.remove(&key);
                    app.last_action = Some(LastAction::failure(e));
                }
            }
            Err(e) => app.last_action = Some(LastAction::failure(format!("Cannot exclude file: {e}"))),
        }
    }
    if let Some(key) = remove {
        exclusions.excluded_paths.remove(&key);
        if let Err(e) = exclusions.save(&app.state.dirs.cache_dir) {
            exclusions.excluded_paths.insert(key);
            app.last_action = Some(LastAction::failure(e));
        }
    }
    if open { app.folder_import.exclusions_manager = Some(exclusions); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_skips_existing_content_and_excluded_paths() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("mods");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        let existing = folder.join("already.pak");
        let duplicate = folder.join("duplicate.zip");
        let excluded = folder.join("excluded.pak");
        let fresh = folder.join("fresh.pak");
        std::fs::write(&existing, b"same contents").unwrap();
        std::fs::write(&duplicate, b"same contents").unwrap();
        std::fs::write(&excluded, b"excluded contents").unwrap();
        std::fs::write(&fresh, b"new contents").unwrap();
        let mut exclusions = Exclusions::default();
        exclusions.excluded_paths.insert(path_key(&excluded.canonicalize().unwrap()));
        exclusions.save(&cache).unwrap();
        let specs = [ModSpecification::new(existing.display().to_string())];
        let result = candidates(&folder, &cache, &specs).unwrap();
        assert_eq!(result, vec![fresh.canonicalize().unwrap()]);
        // If the file is no longer imported, the explicit exclusion still survives.
        let result = candidates(&folder, &cache, &[]).unwrap();
        assert!(!result.contains(&excluded.canonicalize().unwrap()));
    }

    #[test]
    fn removed_mod_returns_to_import_menu_unless_explicitly_excluded() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("watched");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        let local = folder.join("removed.pak");
        std::fs::write(&local, b"mod contents").unwrap();
        let spec = ModSpecification::new(local.display().to_string());
        // While imported, it should not appear as a candidate.
        assert!(candidates(&folder, &cache, &[spec]).unwrap().is_empty());
        // After removing it from Mintfixed, it is offered for manual import.
        assert_eq!(candidates(&folder, &cache, &[]).unwrap(),
                   vec![local.canonicalize().unwrap()]);
        // Only a deliberate exclusion suppresses it on subsequent menu opens.
        let mut exclusions = Exclusions::load(&cache).unwrap();
        exclusions.excluded_paths.insert(path_key(&local.canonicalize().unwrap()));
        exclusions.save(&cache).unwrap();
        assert!(candidates(&folder, &cache, &[]).unwrap().is_empty());
    }

    #[test]
    fn invalid_exclusions_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(EXCLUSIONS_FILE), b"not json").unwrap();
        assert!(Exclusions::load(temp.path()).is_err());
    }
}

/// Save the watched folder independently of the main Settings Save button.
/// Invalid intermediate text is not persisted, and a failed disk write is rolled back.
pub(super) fn save_folder_setting(state: &mut State, text: &str) -> Result<bool, String> {
    let folder = if text.trim().is_empty() {
        None
    } else {
        let path = std::fs::canonicalize(text.trim())
            .map_err(|e| format!("Invalid local mods folder: {e}"))?;
        if !path.is_dir() { return Err("Local mods path is not a folder".into()); }
        Some(path)
    };
    if state.config.local_mod_folder == folder { return Ok(false); }
    let previous = std::mem::replace(&mut state.config.local_mod_folder, folder);
    if let Err(error) = state.config.save() {
        state.config.local_mod_folder = previous;
        return Err(format!("Failed to save local mods folder: {error}"));
    }
    Ok(true)
}

pub(super) fn reset_schedule(app: &mut App) {
    app.folder_import.scan = None;
    app.folder_import.preview = None;
}
