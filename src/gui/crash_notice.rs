//! Detect BP-Crash-Reporter crash reports on Mintfixed startup.
//! Keep this independent of the reporter installer and the in-game hook.

use std::io::Read;
use std::path::{Path, PathBuf};

use super::State;
use mint_lib::DRGInstallation;

const MAX_REPORT_READ: u64 = 128 * 1024;
const SEEN_FILE: &str = "bpcrash-last-acknowledged.txt";

pub(super) struct RecentCrash {
    path: PathBuf,
    seen_file: PathBuf,
    name: String,
    timestamp: String,
    detail: Option<String>,
    open_error: Option<String>,
}

/// Only look inside the selected DRG installation; never scan arbitrary folders.
/// A report is acknowledged when the user closes the popup or clicks Dismiss.
pub(super) fn find_recent_unseen(state: &State) -> Option<RecentCrash> {
    let pak = state.config.drg_pak_path.as_deref()?;
    let installation = DRGInstallation::from_pak_path(pak).ok()?;
    let reports_dir = installation.binaries_directory().join("mintfixed-bpcrash");
    let seen_file = state.dirs.data_dir.join(SEEN_FILE);
    find_latest_unseen_report(&reports_dir, &seen_file)
}

fn find_latest_unseen_report(reports_dir: &Path, seen_file: &Path) -> Option<RecentCrash> {
    let mut latest: Option<(String, PathBuf, String)> = None;
    for entry in std::fs::read_dir(reports_dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !name.starts_with("bpcrash_") || !name.ends_with(".txt") {
            continue;
        }
        // Names contain the reporter's local timestamp, so lexical ordering
        // ignores the later modification time caused by post-exit triage.
        if latest
            .as_ref()
            .is_some_and(|(previous, _, _)| name <= previous.as_str())
        {
            continue;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let mut bytes = Vec::new();
        if file.take(MAX_REPORT_READ).read_to_end(&mut bytes).is_err() {
            continue;
        }
        let body = String::from_utf8_lossy(&bytes);
        if !is_crash_report(&body) {
            continue;
        }
        let detail = body
            .lines()
            .find(|line| line.trim_start().starts_with("assert  :"))
            .and_then(|line| line.split_once(':').map(|(_, s)| s.trim().to_owned()))
            .filter(|line| !line.is_empty())
            .unwrap_or_default();
        latest = Some((name.to_owned(), path, detail));
    }
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
        path,
        seen_file: seen_file.to_path_buf(),
        detail: (!detail.is_empty()).then_some(detail),
        open_error: None,
    })
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

pub(super) fn show(ctx: &egui::Context, recent: &mut Option<RecentCrash>) {
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
mod tests {
    use super::*;

    fn write_report(dir: &Path, name: &str, reason: &str, detail: &str) {
        std::fs::write(
            dir.join(name),
            format!("=== BPCrashHandler report ===\nreason  : {reason}\n\nassert  : {detail}\n"),
        )
        .unwrap();
    }

    #[test]
    fn self_tests_are_ignored_and_latest_crash_is_shown() {
        let temp = tempfile::tempdir().unwrap();
        let reports = temp.path().join("reporter");
        std::fs::create_dir(&reports).unwrap();
        write_report(
            &reports,
            "bpcrash_20261107_090400_1_01_t1.txt",
            "load-time self test (armed)",
            "",
        );
        write_report(
            &reports,
            "bpcrash_20261107_090509_1_02_t1.txt",
            "first-chance exception #1",
            "older error",
        );
        write_report(
            &reports,
            "bpcrash_20261107_115333_2_03_t2.txt",
            "first-chance exception #1",
            "missing parent",
        );
        let seen = temp.path().join("ack.txt");
        let latest = find_latest_unseen_report(&reports, &seen).unwrap();
        assert!(latest.name.contains("115333"));
        assert_eq!(latest.detail.as_deref(), Some("missing parent"));
        assert_eq!(latest.timestamp, "2026-11-07 11:53:33");
    }

    #[test]
    fn acknowledged_crash_does_not_reappear_but_new_one_does() {
        let temp = tempfile::tempdir().unwrap();
        let reports = temp.path().join("reporter");
        std::fs::create_dir(&reports).unwrap();
        let first = "bpcrash_20261107_090509_1_03_t1.txt";
        write_report(&reports, first, "first-chance exception #1", "error");
        let seen = temp.path().join("ack.txt");
        let crash = find_latest_unseen_report(&reports, &seen).unwrap();
        std::fs::write(&seen, crash.path.to_string_lossy().as_bytes()).unwrap();
        assert!(find_latest_unseen_report(&reports, &seen).is_none());
        write_report(
            &reports,
            "bpcrash_20261107_100000_2_03_t2.txt",
            "first-chance exception #1",
            "new error",
        );
        assert!(find_latest_unseen_report(&reports, &seen).is_some());
    }
}

// Check for new crash reports without causing periodic GUI repaints or blocking
// the UI thread on filesystem access. The worker is parked between checks.
const CRASH_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

pub(super) struct CrashMonitor {
    pak_path: Option<PathBuf>,
    received: std::sync::mpsc::Receiver<RecentCrash>,
    stop: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
    pending: Option<RecentCrash>,
}

impl CrashMonitor {
    pub(super) fn start(state: &State, ctx: &egui::Context) -> Self {
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
                    loop {
                        match stop_rx.recv_timeout(CRASH_POLL_INTERVAL) {
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) | Ok(()) => break,
                        }
                        if let Some(crash) = find_new_report(&reports_dir, &seen_file, &mut known) {
                            if send_report.send(crash).is_err() {
                                break;
                            }
                            // A new report is the ONLY reason to wake the UI.
                            repaint.request_repaint();
                        }
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

    pub(super) fn matches_installation(&self, state: &State) -> bool {
        self.pak_path.as_ref() == state.config.drg_pak_path.as_ref()
    }

    pub(super) fn poll(&mut self, shown: &mut Option<RecentCrash>) {
        while let Ok(report) = self.received.try_recv() {
            if shown
                .as_ref()
                .is_some_and(|current| current.path == report.path)
            {
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
        if shown.is_none() {
            if let Some(report) = self.pending.take() {
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
mod live_crash_monitor_tests {
    use super::*;
    use std::collections::HashSet;

    fn report(dir: &Path, name: &str, reason: &str) {
        std::fs::write(
            dir.join(name),
            format!("=== BPCrashHandler report ===\nreason  : {reason}\n\nassert  : test error\n"),
        )
        .unwrap();
    }

    #[test]
    fn watches_only_new_crashes_and_ignores_self_tests() {
        let dir = tempfile::tempdir().unwrap();
        let seen = dir.path().join("ack.txt");
        report(
            dir.path(),
            "bpcrash_20261111_110000_1_01_t1.txt",
            "first-chance exception #1",
        );
        let mut known: HashSet<String> = report_paths(dir.path())
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert!(find_new_report(dir.path(), &seen, &mut known).is_none());

        report(
            dir.path(),
            "bpcrash_20261111_110500_1_02_t1.txt",
            "load-time self test (armed)",
        );
        assert!(find_new_report(dir.path(), &seen, &mut known).is_none());
        report(
            dir.path(),
            "bpcrash_20261111_110600_1_03_t1.txt",
            "first-chance exception #1",
        );
        let found = find_new_report(dir.path(), &seen, &mut known).unwrap();
        assert!(found.name.contains("110600"));
        assert_eq!(found.detail.as_deref(), Some("test error"));
        assert!(find_new_report(dir.path(), &seen, &mut known).is_none());
    }

    #[test]
    fn incomplete_reports_are_retried() {
        let dir = tempfile::tempdir().unwrap();
        let seen = dir.path().join("ack.txt");
        let mut known = HashSet::new();
        let name = "bpcrash_20261111_120000_1_01_t1.txt";
        std::fs::write(dir.path().join(name), "=== BPCrashHandler report ===\n").unwrap();
        assert!(find_new_report(dir.path(), &seen, &mut known).is_none());
        assert!(!known.contains(name));
        report(dir.path(), name, "UE fatal crash");
        assert!(find_new_report(dir.path(), &seen, &mut known).is_some());
    }
}
