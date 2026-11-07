//! Crash report history menu for Mintfixed's bottom toolbar.
//! Independent of report installation and the one-time startup notification.

use std::{fs, io::Read, path::{Path, PathBuf}};

use eframe::egui;
use mint_lib::DRGInstallation;

use super::State;

const HEADER_BYTES: u64 = 2048;

#[derive(Default)]
pub(super) struct CrashHistory {
    open: bool,
    reports: Vec<CrashReport>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct CrashReport {
    name: String,
    timestamp: String,
    path: PathBuf,
}

// Same size, padding and tinting as the Settings cogwheel in the footer.
pub(super) fn toolbar_button(ui: &mut egui::Ui) -> bool {
    let icon = egui::Image::new(egui::include_image!("../../assets/icons/crash.svg"))
        .fit_to_exact_size(egui::vec2(14.0, 14.0))
        .tint(ui.visuals().text_color());
    ui.add(egui::Button::image(icon))
        .on_hover_text("Crash history")
        .clicked()
}

impl CrashHistory {
    pub(super) fn open(&mut self, state: &State) {
        self.refresh(state);
        self.open = true;
    }

    fn refresh(&mut self, state: &State) {
        match report_dir(state).and_then(|dir| scan_reports(&dir)) {
            Ok(reports) => {
                self.reports = reports;
                self.error = None;
            }
            Err(message) => {
                self.reports.clear();
                self.error = Some(message);
            }
        }
    }

    pub(super) fn show(&mut self, ctx: &egui::Context, state: &State) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("DRG crash history")
            .id(egui::Id::new("mintfixed-crash-history-window"))
            .open(&mut open)
            .default_width(740.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Crash reports");
                    if ui.button("Refresh").on_hover_text("Rescan for new crash reports").clicked() {
                        self.refresh(state);
                    }
                });
                ui.label(format!("{} crash report(s) — newest first", self.reports.len()));
                if self.reports.is_empty() && self.error.is_none() {
                    ui.label("No crash reports found. Startup self-tests are excluded.");
                } else if !self.reports.is_empty() {
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for report in &self.reports {
                                ui.push_id(&report.path, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(report.timestamp.as_str());
                                        ui.label(report.name.as_str())
                                            .on_hover_text(report.path.display().to_string());
                                        // Keep actions aligned to the right of each row.
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui.button("Copy path")
                                                    .on_hover_text("Copy the full local report path")
                                                    .clicked()
                                                {
                                                    ui.ctx().copy_text(report.path.display().to_string());
                                                }
                                                if ui.button("Open report").clicked() {
                                                    if let Err(error) = open_report(&report.path) {
                                                        self.error = Some(format!(
                                                            "Could not open {}: {error}", report.name
                                                        ));
                                                    }
                                                }
                                            },
                                        );
                                    });
                                    ui.separator();
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
    let pak = state.config.drg_pak_path.as_deref()
        .ok_or_else(|| "DRG installation is not configured.".to_owned())?;
    let installation = DRGInstallation::from_pak_path(pak)
        .map_err(|error| format!("Could not resolve DRG installation: {error}"))?;
    Ok(installation.binaries_directory().join("mintfixed-bpcrash"))
}

fn scan_reports(dir: &Path) -> Result<Vec<CrashReport>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("Could not read {}: {error}", dir.display()))?;
    let mut reports = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue; };
        if !name.starts_with("bpcrash_") || !name.ends_with(".txt") { continue; }
        let Some(timestamp) = report_timestamp(name) else { continue; };
        if !is_actual_crash(&path) { continue; }
        reports.push(CrashReport {
            name: name.to_owned(), timestamp, path,
        });
    }
    // The reporter's fixed-width local timestamp prefix sorts lexically.
    // Unlike modification time, this is unaffected by appended post-crash triage.
    reports.sort_unstable_by(|a, b| b.name.cmp(&a.name));
    Ok(reports)
}

fn report_timestamp(name: &str) -> Option<String> {
    let date = name.get(8..16)?;
    let time = name.get(17..23)?;
    if name.as_bytes().get(16) != Some(&b'_')
        || name.as_bytes().get(23) != Some(&b'_')
        || !date.bytes().chain(time.bytes()).all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{}-{}-{} {}:{}:{}",
        &date[..4], &date[4..6], &date[6..8],
        &time[..2], &time[2..4], &time[4..6]))
}

fn is_actual_crash(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else { return false; };
    let mut bytes = Vec::new();
    if file.take(HEADER_BYTES).read_to_end(&mut bytes).is_err() { return false; }
    let header = String::from_utf8_lossy(&bytes);
    if header.lines().next().map(str::trim) != Some("=== BPCrashHandler report ===") {
        return false;
    }
    let Some(reason) = header.lines().take(8)
        .find_map(|line| line.trim().strip_prefix("reason  :")) else { return false; };
    let reason = reason.trim().to_ascii_lowercase();
    !reason.contains("self test") && !reason.contains("self-test")
        && (reason.contains("exception") || reason.contains("fatal") || reason.contains("crash"))
}

fn open_report(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        // Avoid relying on the user's .txt association (which can be broken).
        std::process::Command::new("notepad.exe").arg(path).spawn().map(|_| ())
    }
    #[cfg(not(target_os = "windows"))]
    {
        opener::open(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_report(dir: &Path, name: &str, reason: &str) {
        fs::write(dir.join(name), format!(
            "=== BPCrashHandler report ===\nreason  : {reason}\n\n--- native context ---\n"
        )).unwrap();
    }

    #[test]
    fn crash_history_excludes_self_tests_and_sorts_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        write_report(tmp.path(), "bpcrash_20261107_090400_1_01_t1.txt", "load-time self test (armed)");
        write_report(tmp.path(), "bpcrash_20261107_090509_1_03_t1.txt", "first-chance exception #1");
        write_report(tmp.path(), "bpcrash_20261107_115333_2_03_t2.txt", "UE fatal crash");
        fs::write(tmp.path().join("bpcrash_status.txt"), "loaded").unwrap();
        let reports = scan_reports(tmp.path()).unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].timestamp, "2026-11-07 11:53:33");
        assert!(reports[1].name.contains("090509"));
    }

    #[test]
    fn malformed_timestamps_are_excluded() {
        assert!(report_timestamp("bpcrash_bad.txt").is_none());
        assert_eq!(report_timestamp("bpcrash_20261107_115333_2_03_t2.txt").as_deref(),
            Some("2026-11-07 11:53:33"));
    }
}
