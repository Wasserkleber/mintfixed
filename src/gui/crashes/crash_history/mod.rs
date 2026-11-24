//! Crash report history menu for Mintfixed's bottom toolbar.
//! Independent of report installation and the one-time startup notification.

use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, SystemTime},
};

use eframe::egui;
use mint_lib::DRGInstallation;

use crate::state::State;

const HEADER_BYTES: u64 = 2048;
const MAX_RETAINED_CRASHES: usize = 50;
const MAX_RETAINED_SELF_TESTS: usize = 2;
// An old filename can still belong to a report being appended by the reporter.
const MIN_CLEANUP_AGE: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(in crate::gui) struct CrashHistory {
    open: bool,
    reports: Vec<CrashReport>,
    error: Option<String>,
    directory: Option<PathBuf>,
    cache: ReportCache,
    pending: Option<mpsc::Receiver<Result<ScanResult, String>>>,
    refresh_requested: bool,
}

#[derive(Debug, Clone)]
struct CrashReport {
    name: String,
    timestamp: String,
    path: PathBuf,
}

type ReportCache = HashMap<PathBuf, CachedReport>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    modified: SystemTime,
    len: u64,
}

#[derive(Clone, Copy)]
struct CachedReport {
    stamp: FileStamp,
    kind: ReportKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReportKind {
    Crash,
    SelfTest,
}

#[derive(Default)]
struct ScanResult {
    reports: Vec<CrashReport>,
    cache: ReportCache,
    cleanup_warning: Option<String>,
}

// Same size, padding and tinting as the Settings cogwheel in the footer.
pub(in crate::gui) fn toolbar_button(ui: &mut egui::Ui) -> bool {
    let icon = egui::Image::new(egui::include_image!("../../../assets/icons/crash.svg"))
        .fit_to_exact_size(egui::vec2(14.0, 14.0))
        .tint(ui.visuals().text_color());
    ui.add(egui::Button::image(icon))
        .on_hover_text("Crash history")
        .clicked()
}

impl CrashHistory {
    pub(in crate::gui) fn open(&mut self, state: &State) {
        self.sync_directory(state);
        self.open = true;
        if self.pending.is_none() {
            self.refresh_requested = true;
        }
    }

    fn sync_directory(&mut self, state: &State) {
        match report_dir(state) {
            Ok(directory) => {
                if self.directory.as_ref() != Some(&directory) {
                    self.directory = Some(directory);
                    self.reports.clear();
                    self.cache.clear();
                    self.error = None;
                    // Dropping the receiver prevents results for an old installation
                    // from being displayed. The worker never joins on the GUI thread.
                    self.pending = None;
                    self.refresh_requested = true;
                }
            }
            Err(message) => {
                self.directory = None;
                self.reports.clear();
                self.cache.clear();
                self.pending = None;
                self.refresh_requested = false;
                self.error = Some(message);
            }
        }
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        self.refresh_requested = false;
        if self.pending.is_some() {
            return;
        }
        let Some(directory) = self.directory.clone() else {
            return;
        };
        let cache = std::mem::take(&mut self.cache);
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        self.error = None;
        match std::thread::Builder::new()
            .name("mintfixed-crash-history".to_owned())
            .spawn(move || {
                let result = scan_reports(&directory, &cache);
                if sender.send(result).is_ok() {
                    repaint.request_repaint();
                }
            }) {
            Ok(_) => self.pending = Some(receiver),
            Err(error) => {
                self.error = Some(format!("Could not start crash history scan: {error}"));
            }
        }
    }

    fn poll_refresh(&mut self) {
        let Some(receiver) = &self.pending else {
            return;
        };
        match receiver.try_recv() {
            Ok(Ok(result)) => {
                self.pending = None;
                self.reports = result.reports;
                self.cache = result.cache;
                self.error = result.cleanup_warning;
            }
            Ok(Err(message)) => {
                self.pending = None;
                self.error = Some(message);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                self.error = Some("Crash history scan stopped unexpectedly.".to_owned());
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    pub(in crate::gui) fn show(&mut self, ctx: &egui::Context, state: &State) {
        if self.open {
            self.sync_directory(state);
        }
        self.poll_refresh();
        if !self.open {
            return;
        }
        if self.refresh_requested {
            self.refresh(ctx);
        }
        let loading = self.pending.is_some();
        if loading {
            // Also recover from a disconnected worker that could not request repaint.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        let mut open = self.open;
        egui::Window::new("DRG crash history")
            .id(egui::Id::new("mintfixed-crash-history-window"))
            .open(&mut open)
            .default_width(740.0)
            // Keep horizontal resizing, but let the content determine the height.
            .resizable([true, false])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Crash reports");
                    if ui
                        .add_enabled(
                            !loading && self.directory.is_some(),
                            egui::Button::new("Refresh"),
                        )
                        .on_hover_text("Rescan for new crash reports")
                        .clicked()
                    {
                        self.refresh(ctx);
                    }
                    if self.pending.is_some() {
                        ui.spinner();
                        ui.label(if self.reports.is_empty() {
                            "Loading crash reports…"
                        } else {
                            "Refreshing…"
                        });
                    }
                });
                ui.label(format!(
                    "Keeps the newest {MAX_RETAINED_CRASHES} crashes and {MAX_RETAINED_SELF_TESTS} startup self-tests. Older reports are permanently deleted."
                ));
                ui.label(format!(
                    "{} crash report(s) — newest first",
                    self.reports.len()
                ));
                if self.reports.is_empty() && self.error.is_none() && self.pending.is_none() {
                    ui.label("No crash reports found. Startup self-tests are excluded.");
                } else if !self.reports.is_empty() {
                    ui.separator();
                    let row_height = ui
                        .spacing()
                        .interact_size
                        .y
                        .max(ui.text_style_height(&egui::TextStyle::Body))
                        .max(
                            ui.text_style_height(&egui::TextStyle::Button)
                                + 2.0 * ui.spacing().button_padding.y,
                        );
                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .auto_shrink([false, true])
                        .show_rows(ui, row_height, self.reports.len(), |ui, rows| {
                            for report in &self.reports[rows] {
                                ui.push_id(&report.path, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::Label::new(report.timestamp.as_str()).extend(),
                                        );
                                        ui.add(egui::Label::new(report.name.as_str()).extend())
                                            .on_hover_text(report.path.display().to_string());
                                        // Keep actions aligned to the right of each row.
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .button("Copy path")
                                                    .on_hover_text(
                                                        "Copy the full local report path",
                                                    )
                                                    .clicked()
                                                {
                                                    ui.ctx().copy_text(
                                                        report.path.display().to_string(),
                                                    );
                                                }
                                                if ui.button("Open report").clicked()
                                                    && let Err(error) = open_report(&report.path)
                                                {
                                                    self.error = Some(format!(
                                                        "Could not open {}: {error}",
                                                        report.name
                                                    ));
                                                }
                                            },
                                        );
                                    });
                                });
                            }
                        });
                }
                if let Some(message) = &self.error {
                    // This also shows an error caused by pressing Open report.
                    ui.colored_label(ui.visuals().error_fg_color, message);
                }
            });
        self.open = open;
    }
}

fn report_dir(state: &State) -> Result<PathBuf, String> {
    let pak = state
        .config
        .drg_pak_path
        .as_deref()
        .ok_or_else(|| "DRG installation is not configured.".to_owned())?;
    let installation = DRGInstallation::from_pak_path(pak)
        .map_err(|error| format!("Could not resolve DRG installation: {error}"))?;
    Ok(installation.binaries_directory().join("mintfixed-bpcrash"))
}

fn scan_reports(dir: &Path, previous: &ReportCache) -> Result<ScanResult, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ScanResult::default());
        }
        Err(error) => return Err(format!("Could not read {}: {error}", dir.display())),
    };
    let mut candidates = Vec::new();
    // Finish enumeration before deleting anything. A partial directory listing
    // cannot reliably establish which files are the newest.
    for entry in entries {
        let entry = entry.map_err(|error| format!("Could not list {}: {error}", dir.display()))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("bpcrash_") || !name.ends_with(".txt") {
            continue;
        }
        let Some(timestamp) = report_timestamp(name) else {
            continue;
        };
        // Never follow a symlink when deciding what can be permanently deleted.
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let stamp = file_stamp(&metadata);
        let cached = previous
            .get(&path)
            .filter(|cached| stamp == Some(cached.stamp));
        let Some(kind) = cached
            .map(|cached| cached.kind)
            .or_else(|| classify_report(&path))
        else {
            // Never delete unrelated, incomplete or unreadable files.
            continue;
        };
        candidates.push((
            CrashReport {
                name: name.to_owned(),
                timestamp,
                path,
            },
            stamp,
            kind,
        ));
    }
    // Use the reporter's timestamp, not mtime (post-crash triage can append).
    candidates.sort_unstable_by(|a, b| b.0.name.cmp(&a.0.name));
    let mut result = ScanResult::default();
    let mut deleted = 0;
    let mut deferred = 0;
    let mut failed = 0;
    let mut first_error = None;
    let mut crashes = 0;
    let mut self_tests = 0;
    for (report, stamp, kind) in candidates {
        // Each class has its own newest-first retention budget. Clean game
        // launches must never consume slots reserved for actual crashes.
        let (count, limit) = match kind {
            ReportKind::Crash => (&mut crashes, MAX_RETAINED_CRASHES),
            ReportKind::SelfTest => (&mut self_tests, MAX_RETAINED_SELF_TESTS),
        };
        *count += 1;
        if *count > limit {
            match prune_report(&report.path, stamp) {
                Ok(true) => {
                    deleted += 1;
                    continue;
                }
                Ok(false) => deferred += 1,
                Err(error) => {
                    failed += 1;
                    if first_error.is_none() {
                        first_error = Some(format!("{}: {error}", report.name));
                    }
                }
            }
        }
        // Preserve reports that could not safely be deleted; retry on refresh.
        if let Some(stamp) = stamp {
            result
                .cache
                .insert(report.path.clone(), CachedReport { stamp, kind });
        }
        if kind == ReportKind::Crash {
            result.reports.push(report);
        }
    }
    if deleted > 0 {
        tracing::info!(deleted, "Removed old BP-Crash-Reporter files");
    }
    if deferred > 0 || failed > 0 {
        let detail = first_error.map_or_else(String::new, |error| format!(" ({error})"));
        let warning = format!(
            "Crash report cleanup: {deferred} deferred, {failed} failed{detail}. Refresh to retry."
        );
        tracing::warn!("{warning}");
        result.cleanup_warning = Some(warning);
    }
    Ok(result)
}

fn file_stamp(metadata: &fs::Metadata) -> Option<FileStamp> {
    Some(FileStamp {
        modified: metadata.modified().ok()?,
        len: metadata.len(),
    })
}

fn prune_report(path: &Path, scanned_stamp: Option<FileStamp>) -> std::io::Result<bool> {
    let Some(stamp) = scanned_stamp else {
        return Ok(false);
    };
    if !stamp
        .modified
        .elapsed()
        .is_ok_and(|age| age >= MIN_CLEANUP_AGE)
    {
        return Ok(false);
    }
    // Recheck before removing: a report may have been changed, replaced or
    // removed by another process since the directory was scanned.
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || file_stamp(&metadata) != Some(stamp) {
        return Ok(false);
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
    }
}

fn report_timestamp(name: &str) -> Option<String> {
    let date = name.get(8..16)?;
    let time = name.get(17..23)?;
    if name.as_bytes().get(16) != Some(&b'_')
        || name.as_bytes().get(23) != Some(&b'_')
        || !date
            .bytes()
            .chain(time.bytes())
            .all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    Some(format!(
        "{}-{}-{} {}:{}:{}",
        &date[..4],
        &date[4..6],
        &date[6..8],
        &time[..2],
        &time[2..4],
        &time[4..6]
    ))
}

fn classify_report(path: &Path) -> Option<ReportKind> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(HEADER_BYTES).read_to_end(&mut bytes).ok()?;
    let header = String::from_utf8_lossy(&bytes);
    if header.lines().next().map(str::trim) != Some("=== BPCrashHandler report ===") {
        return None;
    }
    let reason = header
        .lines()
        .take(8)
        .find_map(|line| line.trim().strip_prefix("reason  :"))?
        .trim()
        .to_ascii_lowercase();
    if reason.contains("self test") || reason.contains("self-test") {
        Some(ReportKind::SelfTest)
    } else if reason.contains("exception") || reason.contains("fatal") || reason.contains("crash") {
        Some(ReportKind::Crash)
    } else {
        // Unknown report types are not eligible for permanent deletion.
        None
    }
}

fn open_report(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        // Avoid relying on the user's .txt association (which can be broken).
        std::process::Command::new("notepad.exe")
            .arg(path)
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(target_os = "windows"))]
    {
        opener::open(path).map_err(|error| std::io::Error::other(error.to_string()))
    }
}

#[cfg(test)]
mod tests;
