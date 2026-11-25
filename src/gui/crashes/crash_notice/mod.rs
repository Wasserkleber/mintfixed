//! Detect BP-Crash-Reporter crash reports on Mintfixed startup.
//! Keep this independent of the reporter installer and the in-game hook.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::state::State;
use mint_lib::DRGInstallation;

const MAX_REPORT_READ: u64 = 128 * 1024;
const MAX_ATTRIBUTION_READ: u64 = 32 * 1024 * 1024;
const MAX_STARTUP_SCAN: usize = 50;
const SEEN_FILE: &str = "bpcrash-last-acknowledged.txt";
const ATTRIBUTION_MARKER: &str = "--- mintfixed mod attribution (bpcrash.exe) ---";
const BLUEPRINT_MARKER: &str = "--- blueprint function compatibility (mintfixed) ---";

#[derive(Clone, Debug, PartialEq, Eq)]
enum ModAttribution {
    Pending,
    Unavailable,
    NoMatch,
    Candidates(Vec<String>),
}

#[derive(Clone)]
pub(in crate::gui) struct RecentCrash {
    path: PathBuf,
    seen_file: PathBuf,
    name: String,
    timestamp: String,
    detail: Option<String>,
    attribution: ModAttribution,
    open_error: Option<String>,
}

/// Only look inside the selected DRG installation; never scan arbitrary folders.
/// A report is acknowledged when the user closes the popup or clicks Dismiss.
pub(in crate::gui) fn find_recent_unseen(state: &State) -> Option<RecentCrash> {
    let pak = state.config.drg_pak_path.as_deref()?;
    let installation = DRGInstallation::from_pak_path(pak).ok()?;
    let reports_dir = installation.binaries_directory().join("mintfixed-bpcrash");
    let seen_file = state.dirs.data_dir.join(SEEN_FILE);
    find_latest_unseen_report(&reports_dir, &seen_file)
}

fn find_latest_unseen_report(reports_dir: &Path, seen_file: &Path) -> Option<RecentCrash> {
    // Names contain the reporter's local timestamp, so lexical ordering
    // ignores the later modification time caused by post-exit triage. Walk
    // newest first and stop at the first real crash: every game start writes
    // two self-test reports, so opening the whole folder grows without bound.
    let mut names = report_paths(reports_dir);
    names.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    // Only the newest names count as "recent"; a crash behind more
    // than this many later reports (about 25 clean game starts) is not shown.
    let latest = names
        .into_iter()
        .take(MAX_STARTUP_SCAN)
        .find_map(|(name, path)| {
            let file = std::fs::File::open(&path).ok()?;
            let mut bytes = Vec::new();
            file.take(MAX_REPORT_READ).read_to_end(&mut bytes).ok()?;
            let body = String::from_utf8_lossy(&bytes);
            if !is_crash_report(&body) {
                return None;
            }
            let detail = body
                .lines()
                .find(|line| line.trim_start().starts_with("assert  :"))
                .and_then(|line| line.split_once(':').map(|(_, s)| s.trim().to_owned()))
                .filter(|line| !line.is_empty())
                .unwrap_or_default();
            Some((name, path, detail))
        });
    let (name, path, detail) = latest?;
    // Full path disambiguates multiple DRG installations using the same appdata.
    let id = path.to_string_lossy();
    if std::fs::read_to_string(seen_file)
        .ok()
        .is_some_and(|seen| seen.trim() == id.as_ref() as &str)
    {
        return None;
    }
    Some(RecentCrash {
        timestamp: format_report_timestamp(&name).unwrap_or_else(|| name.clone()),
        name,
        attribution: read_mod_attribution(&path),
        path,
        seen_file: seen_file.to_path_buf(),
        detail: (!detail.is_empty()).then_some(detail),
        open_error: None,
    })
}

fn read_mod_attribution(path: &Path) -> ModAttribution {
    let Ok(file) = std::fs::File::open(path) else {
        return ModAttribution::Pending;
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_ATTRIBUTION_READ + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return ModAttribution::Pending;
    }
    if bytes.len() as u64 > MAX_ATTRIBUTION_READ {
        return ModAttribution::Unavailable;
    }
    parse_mod_attribution(&String::from_utf8_lossy(&bytes))
}

// BP-Crash-Reporter 1.3.0 appends this section after the game exits.
// Only trust its structured attribution fields, never arbitrary stack text.
fn parse_mod_attribution(body: &str) -> ModAttribution {
    let mut lines = body.lines();
    if !lines.any(|line| line.trim() == ATTRIBUTION_MARKER) {
        return ModAttribution::Pending;
    }
    let mut ranked = std::collections::BTreeMap::new();
    let mut blueprint = std::collections::BTreeMap::new();
    let mut snapshot_matches = false;
    let mut in_blueprint = false;
    for line in lines {
        let line = line.trim();
        if line == BLUEPRINT_MARKER {
            in_blueprint = true;
            continue;
        }
        if line.starts_with("---") {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if value.is_empty() {
            continue;
        }
        if !in_blueprint {
            match key {
                "status" => {
                    return if value == "no_recognized_assets_or_missing_functions" {
                        ModAttribution::NoMatch
                    } else {
                        ModAttribution::Unavailable
                    };
                }
                "result.snapshot_crash_match" => {
                    snapshot_matches = value == "installed_pak_size_and_mtime_match";
                }
                _ => {
                    if let Some(index) = key
                        .strip_prefix("rank[")
                        .and_then(|key| key.strip_suffix("].mod"))
                        .and_then(|index| index.parse::<usize>().ok())
                        .filter(|index| (1..=3).contains(index))
                    {
                        ranked.insert(index, value.to_owned());
                    }
                }
            }
        } else if let Some(index) = key
            .strip_prefix("mod_evidence[")
            .and_then(|key| key.strip_suffix("].name"))
            .and_then(|index| index.parse::<usize>().ok())
            .filter(|index| *index > 0)
        {
            blueprint.insert(index, value.to_owned());
        }
    }
    if !snapshot_matches {
        return ModAttribution::Pending;
    }
    let mut names = Vec::new();
    for name in ranked.into_values().chain(blueprint.into_values()) {
        if !names.contains(&name) {
            names.push(name);
        }
        if names.len() == 3 {
            break;
        }
    }
    if names.is_empty() {
        ModAttribution::NoMatch
    } else {
        ModAttribution::Candidates(names)
    }
}

fn is_crash_report(text: &str) -> bool {
    if !text.starts_with("=== BPCrashHandler report ===") {
        return false;
    }
    let Some(reason) = text
        .lines()
        .take(8)
        .find_map(|line| line.trim().strip_prefix("reason  :"))
    else {
        return false;
    };
    // Ignore the two load-time self-tests generated on every successful game start.
    let reason = reason.trim().to_ascii_lowercase();
    !reason.contains("self test")
        && !reason.contains("self-test")
        && (reason.contains("exception") || reason.contains("fatal") || reason.contains("crash"))
}

fn format_report_timestamp(name: &str) -> Option<String> {
    let date = name.get(8..16)?;
    let time = name.get(17..23)?;
    if !date.bytes().chain(time.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    if name.as_bytes().get(16) != Some(&b'_') {
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

// Open text reports with a known text editor on Windows. Using the system
// default file association can fail if .txt has a broken handler.
fn open_report(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
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

pub(in crate::gui) fn show(ctx: &egui::Context, recent: &mut Option<RecentCrash>) {
    let Some(crash) = recent.as_mut() else {
        return;
    };
    let mut open = true;
    let mut dismissed = false;
    egui::Window::new("Recent DRG crash")
        .id(egui::Id::new("mintfixed-recent-crash"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.label("Mintfixed found a new DRG crash report since you last checked.");
            ui.label(format!("Crash: {}", crash.timestamp));
            if let Some(detail) = &crash.detail {
                ui.separator();
                ui.label(egui::RichText::new("Error").strong());
                ui.add(egui::Label::new(detail.as_str()).wrap());
            }
            ui.separator();
            ui.label(egui::RichText::new("Possible culprit mods").strong());
            match &crash.attribution {
                ModAttribution::Candidates(names) => {
                    for name in names {
                        ui.add(egui::Label::new(format!("• {name}")).wrap());
                    }
                    ui.weak("Based on crash evidence; not a confirmed cause.");
                }
                ModAttribution::Pending => {
                    ui.weak("Attribution not available yet (analysis runs after game exit).");
                }
                ModAttribution::Unavailable => {
                    ui.weak("Mod attribution unavailable for this report.");
                }
                ModAttribution::NoMatch => {
                    ui.weak("No matching mod identified.");
                }
            }
            ui.separator();
            ui.label("Crash report:");
            if ui
                .link(crash.name.as_str())
                .on_hover_text(crash.path.display().to_string())
                .clicked()
                && let Err(e) = open_report(&crash.path)
            {
                crash.open_error = Some(format!("Could not open report: {e}"));
            }
            ui.horizontal(|ui| {
                if ui.button("Open report").clicked()
                    && let Err(e) = open_report(&crash.path)
                {
                    crash.open_error = Some(format!("Could not open report: {e}"));
                }
                if ui.button("Open folder").clicked()
                    && let Some(dir) = crash.path.parent()
                    && let Err(e) = opener::open(dir)
                {
                    crash.open_error = Some(format!("Could not open folder: {e}"));
                }
                if ui.button("Copy path").clicked() {
                    ui.ctx().copy_text(crash.path.display().to_string());
                }
                if ui.button("Dismiss").clicked() {
                    dismissed = true;
                }
            });
            if let Some(err) = &crash.open_error {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
        });
    if dismissed || !open {
        if let Err(e) = std::fs::write(&crash.seen_file, crash.path.to_string_lossy().as_bytes()) {
            tracing::warn!("Could not save acknowledged DRG crash report: {e}");
        }
        *recent = None;
    }
}

#[cfg(test)]
mod tests;

// Check for new crash reports without causing periodic GUI repaints or blocking
// the UI thread on filesystem access. The worker is parked between checks.
const CRASH_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

// Refreshes only update an existing notice; they must never reopen one.
enum CrashEvent {
    New(RecentCrash),
    Attribution(PathBuf, ModAttribution),
}

type ReportStamp = (u64, std::time::SystemTime);

fn report_stamp(path: &Path) -> Option<ReportStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

fn refresh_attributions(
    watched: &mut Vec<(RecentCrash, Option<ReportStamp>)>,
    mut send: impl FnMut(CrashEvent),
) {
    watched.retain_mut(|(crash, stamp)| {
        if std::fs::read_to_string(&crash.seen_file)
            .ok()
            .is_some_and(|seen| seen.trim() == crash.path.to_string_lossy().as_ref() as &str)
        {
            return false;
        }
        let Some(current) = report_stamp(&crash.path) else {
            return false;
        };
        if *stamp != Some(current) {
            *stamp = Some(current);
            let attribution = read_mod_attribution(&crash.path);
            if crash.attribution != attribution {
                crash.attribution = attribution.clone();
                send(CrashEvent::Attribution(crash.path.clone(), attribution));
            }
        }
        true
    });
}

pub(in crate::gui) struct CrashMonitor {
    pak_path: Option<PathBuf>,
    received: std::sync::mpsc::Receiver<CrashEvent>,
    stop: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
    pending: Option<RecentCrash>,
}

impl CrashMonitor {
    pub(in crate::gui) fn start(state: &State, ctx: &egui::Context) -> Self {
        let pak_path = state.config.drg_pak_path.clone();
        let reports_dir = pak_path
            .as_deref()
            .and_then(|pak| DRGInstallation::from_pak_path(pak).ok())
            .map(|installation| installation.binaries_directory().join("mintfixed-bpcrash"));
        let seen_file = state.dirs.data_dir.join(SEEN_FILE);
        let (send_report, received) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();

        let worker = reports_dir.and_then(|reports_dir| {
            let repaint = ctx.clone();
            match std::thread::Builder::new()
                .name("mintfixed-crash-monitor".to_owned())
                .spawn(move || {
                    // Seed existing filenames. Startup detection is handled by
                    // find_recent_unseen; the worker only reports later files.
                    let mut known = std::collections::HashSet::new();
                    for (name, path) in report_paths(&reports_dir) {
                        // Reports written during startup can still have an
                        // incomplete header. Recheck these rather than losing
                        // a crash that races with the monitor's creation.
                        let just_written = std::fs::metadata(&path)
                            .and_then(|meta| meta.modified())
                            .ok()
                            .and_then(|time| time.elapsed().ok())
                            .is_some_and(|age| age < std::time::Duration::from_secs(10));
                        if !just_written {
                            known.insert(name);
                        }
                    }
                    let mut watched: Vec<(RecentCrash, Option<ReportStamp>)> = Vec::new();
                    if let Some(crash) = find_latest_unseen_report(&reports_dir, &seen_file) {
                        // The startup notice may have been read before attribution was appended.
                        // Send the worker's first observation even if the file is already complete.
                        let _ = send_report.send(CrashEvent::Attribution(
                            crash.path.clone(),
                            crash.attribution.clone(),
                        ));
                        repaint.request_repaint();
                        watched.push((crash, None));
                    }
                    while let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                        stop_rx.recv_timeout(CRASH_POLL_INTERVAL)
                    {
                        if let Some(crash) = find_new_report(&reports_dir, &seen_file, &mut known) {
                            watched.retain(|(previous, _)| previous.path != crash.path);
                            // Bound retained reports even if several crashes arrive before dismissal.
                            if watched.len() >= 8 {
                                watched.remove(0);
                            }
                            watched.push((crash.clone(), None));
                            if send_report.send(CrashEvent::New(crash)).is_err() {
                                break;
                            }
                            repaint.request_repaint();
                        }
                        refresh_attributions(&mut watched, |event| {
                            if send_report.send(event).is_ok() {
                                repaint.request_repaint();
                            }
                        });
                    }
                }) {
                Ok(worker) => Some(worker),
                Err(error) => {
                    tracing::warn!("Could not start DRG crash monitor: {error}");
                    None
                }
            }
        });
        Self {
            pak_path,
            received,
            stop: worker.as_ref().map(|_| stop_tx),
            worker,
            pending: None,
        }
    }

    pub(in crate::gui) fn matches_installation(&self, state: &State) -> bool {
        self.pak_path.as_ref() == state.config.drg_pak_path.as_ref()
    }

    pub(in crate::gui) fn poll(&mut self, shown: &mut Option<RecentCrash>) {
        while let Ok(event) = self.received.try_recv() {
            let report = match event {
                CrashEvent::New(report) => report,
                CrashEvent::Attribution(path, attribution) => {
                    if let Some(current) = shown.as_mut().filter(|report| report.path == path) {
                        current.attribution = attribution.clone();
                    }
                    if let Some(pending) =
                        self.pending.as_mut().filter(|report| report.path == path)
                    {
                        pending.attribution = attribution;
                    }
                    continue;
                }
            };
            if let Some(current) = shown.as_mut().filter(|current| current.path == report.path) {
                current.attribution = report.attribution;
                continue;
            }
            // If multiple reports arrive together, prefer the newest. Keep
            // it pending while another popup is on screen.
            if self
                .pending
                .as_ref()
                .is_none_or(|previous| report.name > previous.name)
            {
                self.pending = Some(report);
            }
        }
        if shown.is_none()
            && let Some(report) = self.pending.take()
        {
            // The user may have dismissed this report since it was sent
            // by the worker; check only when there is a pending report.
            let id = report.path.to_string_lossy();
            let acknowledged = std::fs::read_to_string(&report.seen_file)
                .ok()
                .is_some_and(|seen| seen.trim() == &*id);
            if !acknowledged {
                *shown = Some(report);
            }
        }
    }
}

impl Drop for CrashMonitor {
    fn drop(&mut self) {
        // Wake the parked worker immediately so quitting Mintfixed does not
        // wait for the 5-second timeout.
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

// Listing filenames is cheap; no report contents are touched unless the name
// is new. Ignore unrelated files (status, pid, loader logs, etc.).
fn report_paths(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !name.starts_with("bpcrash_")
                || !name.ends_with(".txt")
                || format_report_timestamp(&name).is_none()
            {
                return None;
            }
            if !entry.file_type().ok()?.is_file() {
                return None;
            }
            Some((name, entry.path()))
        })
        .collect()
}

fn find_new_report(
    reports_dir: &Path,
    seen_file: &Path,
    known: &mut std::collections::HashSet<String>,
) -> Option<RecentCrash> {
    let mut new_files: Vec<_> = report_paths(reports_dir)
        .into_iter()
        .filter(|(name, _)| !known.contains(name))
        .collect();
    // Fixed-width timestamps make names sortable independently of last-write
    // timestamps (the reporter can append native triage later).
    new_files.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let mut newest: Option<RecentCrash> = None;
    for (name, path) in new_files {
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let mut bytes = Vec::new();
        if file.take(MAX_REPORT_READ).read_to_end(&mut bytes).is_err() {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        // A newly created report may still be written. Retry an incomplete
        // header next tick instead of permanently dropping the crash.
        if !text.starts_with("=== BPCrashHandler report ===")
            || !text
                .lines()
                .take(8)
                .any(|line| line.trim().starts_with("reason  :"))
        {
            continue;
        }
        known.insert(name.clone());
        if !is_crash_report(&text) {
            continue;
        }
        let id = path.to_string_lossy();
        if std::fs::read_to_string(seen_file)
            .ok()
            .is_some_and(|acknowledged| acknowledged.trim() == &*id)
        {
            continue;
        }
        let detail = text
            .lines()
            .find(|line| line.trim_start().starts_with("assert  :"))
            .and_then(|line| {
                line.split_once(':')
                    .map(|(_, value)| value.trim().to_owned())
            })
            .filter(|value| !value.is_empty());
        let report = RecentCrash {
            timestamp: format_report_timestamp(&name).unwrap_or_else(|| name.clone()),
            name,
            attribution: read_mod_attribution(&path),
            path,
            seen_file: seen_file.to_path_buf(),
            detail,
            open_error: None,
        };
        if newest
            .as_ref()
            .is_none_or(|previous| report.name > previous.name)
        {
            newest = Some(report);
        }
    }
    newest
}

#[cfg(test)]
#[path = "tests/live_crash_monitor_tests.rs"]
mod live_crash_monitor_tests;
