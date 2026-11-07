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
        if !path.is_file() { continue; }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue; };
        if !name.starts_with("bpcrash_") || !name.ends_with(".txt") { continue; }
        // Names contain the reporter's local timestamp, so lexical ordering
        // ignores the later modification time caused by post-exit triage.
        if latest.as_ref().is_some_and(|(previous, _, _)| name <= previous.as_str()) {
            continue;
        }
        let Ok(file) = std::fs::File::open(&path) else { continue; };
        let mut bytes = Vec::new();
        if file.take(MAX_REPORT_READ).read_to_end(&mut bytes).is_err() { continue; }
        let body = String::from_utf8_lossy(&bytes);
        if !is_crash_report(&body) { continue; }
        let detail = body.lines()
            .find(|line| line.trim_start().starts_with("assert  :"))
            .and_then(|line| line.split_once(':').map(|(_, s)| s.trim().to_owned()))
            .filter(|line| !line.is_empty())
            .unwrap_or_default();
        latest = Some((name.to_owned(), path, detail));
    }
    let (name, path, detail) = latest?;
    // Full path disambiguates multiple DRG installations using the same appdata.
    let id = path.to_string_lossy();
    if std::fs::read_to_string(seen_file).ok().is_some_and(|seen| seen.trim() == id.as_ref() as &str) {
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
    if !text.starts_with("=== BPCrashHandler report ===") { return false; }
    let Some(reason) = text.lines().take(8)
        .find_map(|line| line.trim().strip_prefix("reason  :")) else { return false; };
    // Ignore the two load-time self-tests generated on every successful game start.
    let reason = reason.trim().to_ascii_lowercase();
    !reason.contains("self test") && !reason.contains("self-test")
        && (reason.contains("exception") || reason.contains("fatal") || reason.contains("crash"))
}

fn format_report_timestamp(name: &str) -> Option<String> {
    let date = name.get(8..16)?;
    let time = name.get(17..23)?;
    if !date.bytes().chain(time.bytes()).all(|b| b.is_ascii_digit()) { return None; }
    if name.as_bytes().get(16) != Some(&b'_') { return None; }
    Some(format!("{}-{}-{} {}:{}:{}", &date[..4], &date[4..6], &date[6..8],
                 &time[..2], &time[2..4], &time[4..6]))
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
    let Some(crash) = recent.as_mut() else { return; };
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
            if ui.link(crash.name.as_str()).on_hover_text(crash.path.display().to_string()).clicked() {
                if let Err(e) = open_report(&crash.path) {
                    crash.open_error = Some(format!("Could not open report: {e}"));
                }
            }
            ui.horizontal(|ui| {
                if ui.button("Open report").clicked() {
                    if let Err(e) = open_report(&crash.path) {
                        crash.open_error = Some(format!("Could not open report: {e}"));
                    }
                }
                if ui.button("Open folder").clicked() {
                    if let Some(dir) = crash.path.parent() {
                        if let Err(e) = opener::open(dir) {
                            crash.open_error = Some(format!("Could not open folder: {e}"));
                        }
                    }
                }
                if ui.button("Copy path").clicked() {
                    ui.ctx().copy_text(crash.path.display().to_string());
                }
                if ui.button("Dismiss").clicked() { dismissed = true; }
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
        std::fs::write(dir.join(name), format!(
            "=== BPCrashHandler report ===\nreason  : {reason}\n\nassert  : {detail}\n"
        )).unwrap();
    }

    #[test]
    fn self_tests_are_ignored_and_latest_crash_is_shown() {
        let temp = tempfile::tempdir().unwrap();
        let reports = temp.path().join("reporter");
        std::fs::create_dir(&reports).unwrap();
        write_report(&reports, "bpcrash_20261107_090400_1_01_t1.txt", "load-time self test (armed)", "");
        write_report(&reports, "bpcrash_20261107_090509_1_02_t1.txt", "first-chance exception #1", "older error");
        write_report(&reports, "bpcrash_20261107_115333_2_03_t2.txt", "first-chance exception #1", "missing parent");
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
        write_report(&reports, "bpcrash_20261107_100000_2_03_t2.txt", "first-chance exception #1", "new error");
        assert!(find_latest_unseen_report(&reports, &seen).is_some());
    }
}
