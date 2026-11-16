//! Plain-text lint report formatting and durable per-run file output.
use super::workflow::LintRun;
use crate::mod_lints::{LintReport, SplitAssetPair};
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(in crate::gui) fn format_txt(report: &LintReport, run: &LintRun) -> String {
    let mut out = String::new();
    let mut findings = 0usize;
    let _ = writeln!(out, "MINTFIXED LINT REPORT");
    let _ = writeln!(out, "=====================");
    let _ = writeln!(out, "Profile: {}", run.profile);
    let _ = writeln!(out, "Scope: {}", run.scope.label());
    if let Some(anchor) = &run.focus {
        let _ = writeln!(out, "Focus mod: {}", anchor.url);
    }
    let _ = writeln!(out, "Scanned mods: {}", run.mods.len());
    let _ = writeln!(
        out,
        "Checks: {}",
        if run.checks.is_empty() {
            "(none)".into()
        } else {
            run.checks.join(", ")
        }
    );
    let _ = writeln!(out, "\nScanned mod URLs:");
    for spec in &run.mods {
        let _ = writeln!(out, "  - {}", spec.url);
    }
    let _ = writeln!(
        out,
        "\nNOTE: Shared asset paths suggest potential overrides; they do not prove runtime incompatibility."
    );

    macro_rules! section {
        ($name:expr, $entries:expr, $item:pat => $body:block) => {
            if let Some(entries) = &$entries {
                let _ = writeln!(out, "\n{} ({}):", $name, entries.len());
                findings += entries.len();
                if entries.is_empty() { let _ = writeln!(out, "  No findings"); }
                for $item in entries $body
            }
        };
    }
    section!("Conflicting asset paths", report.conflicting_mods, (path, mods) => {
        let _ = writeln!(out, "  Asset: {}", path);
        for spec in mods { let _ = writeln!(out, "    - {}", spec.url); }
    });
    section!("AssetRegistry.bin files", report.asset_register_bin_mods, (spec, paths) => {
        let _ = writeln!(out, "  {}", spec.url);
        for path in paths { let _ = writeln!(out, "    - {}", path); }
    });
    section!("Shader files", report.shader_file_mods, (spec, paths) => {
        let _ = writeln!(out, "  {}", spec.url);
        for path in paths { let _ = writeln!(out, "    - {}", path); }
    });
    section!("Outdated pak version", report.outdated_pak_version_mods, (spec, version) => {
        let _ = writeln!(out, "  {}: {:?}", spec.url, version);
    });
    section!("Empty archives", report.empty_archive_mods, spec => {
        let _ = writeln!(out, "  {}", spec.url);
    });
    section!("Archives without .pak files", report.archive_with_only_non_pak_files_mods, spec => {
        let _ = writeln!(out, "  {}", spec.url);
    });
    section!("Archives containing multiple .pak files", report.archive_with_multiple_paks_mods, spec => {
        let _ = writeln!(out, "  {}", spec.url);
    });
    section!("Non-asset files", report.non_asset_file_mods, (spec, paths) => {
        let _ = writeln!(out, "  {}", spec.url);
        for path in paths { let _ = writeln!(out, "    - {}", path); }
    });
    section!("Split .uasset/.uexp pairs", report.split_asset_pairs_mods, (spec, files) => {
        let _ = writeln!(out, "  {}", spec.url);
        for (path, kind) in files {
            let missing = match kind {
                SplitAssetPair::MissingUasset => ".uasset",
                SplitAssetPair::MissingUexp => ".uexp",
            };
            let _ = writeln!(out, "    - {} (missing {})", path, missing);
        }
    });
    section!("Unmodified game assets", report.unmodified_game_assets_mods, (spec, paths) => {
        let _ = writeln!(out, "  {}", spec.url);
        for path in paths { let _ = writeln!(out, "    - {}", path); }
    });
    let _ = writeln!(
        out,
        "\nSUMMARY: {} entries flagged across all enabled checks",
        findings
    );
    out
}

/// create_new avoids destroying a previous report, even when scans finish within one second.
pub(in crate::gui) fn save_txt(
    data_dir: &Path,
    report: &LintReport,
    run: &LintRun,
) -> io::Result<PathBuf> {
    let dir = data_dir.join("lint-reports");
    fs::create_dir_all(&dir)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let slug: String = run
        .profile
        .chars()
        .take(48)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let content = format_txt(report, run);
    for index in 0..1000 {
        let path = dir.join(format!("lint_{}_{stamp}_{index}.txt", slug));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(content.as_bytes()) {
                    let _ = fs::remove_file(&path);
                    return Err(error);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Cannot allocate unique lint report filename",
    ))
}
