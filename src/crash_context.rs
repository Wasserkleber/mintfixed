//! Read-only hand-off from a successful mintfixed installation to BP-Crash-Reporter.
//! No archive extraction, no extra installation, no live game hook, no network.
//!
//! The `mods` argument is the FINAL deduplicated list actually passed to the
//! integration pipeline, not the full saved profile or the entire cache.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::providers::ModInfo;
use serde::Serialize;

const SCHEMA_VERSION: u32 = 2;
#[path = "crash_context_index.rs"]
mod index;
pub(crate) use index::PreparedModIndex;

pub(crate) fn indexed_from_prepass<'a>(
    member: Option<&str>,
    files: impl IntoIterator<Item = &'a str>,
) -> PreparedModIndex {
    index::from_installed_files(member, files)
}
const CONTEXT_FILENAME: &str = "crash_context.json";

#[derive(Serialize)]
struct FileFingerprint {
    path: String,
    exists: bool,
    byte_size: Option<u64>,
    modified_unix_ms: Option<u128>,
}

fn metadata_for(path: &Path) -> FileFingerprint {
    let meta = fs::metadata(path).ok().filter(|value| value.is_file());
    FileFingerprint {
        path: path.to_string_lossy().into_owned(),
        exists: meta.is_some(),
        byte_size: meta.as_ref().map(std::fs::Metadata::len),
        modified_unix_ms: meta
            .as_ref()
            .and_then(|value| value.modified().ok())
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis()),
    }
}

#[derive(Serialize)]
struct InstalledMod {
    name: String,
    provider: String,
    // 0-based order in the exact deduplicated input to the PAK packer.
    // This does NOT claim that a higher number always wins asset collisions.
    integration_order: usize,
    archive: FileFingerprint,
    archive_type: &'static str,

    archive_index: index::ArchiveIndex,
    // Only a query/fragment-stripped mod.io reference: never cache auth tokens.
    modio_reference: Option<String>,
    modio_mod_id: Option<String>,
    modio_file_id: Option<String>,
}

#[derive(Serialize)]
struct CrashContext {
    schema_version: u32,
    producer: &'static str,
    generated_unix_ms: u128,
    pid: u32,
    captured_after_successful_install: bool,
    vanilla_pak: FileFingerprint,
    installed_pak: FileFingerprint,
    // A live SDK dump is a separate feature and must be checked against the game build.
    sdk_snapshot: Option<String>,
    // No full mod_data.json, config.json, auth tokens, or inactive profiles.
    installed_mods: Vec<InstalledMod>,
}

fn now_ms() -> Result<u128, String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("system clock error: {e}"))?
        .as_millis())
}

fn clean_modio_reference(value: &str) -> Option<String> {
    let prefix = value.split(['?', '#']).next()?.trim();
    // Do not accidentally copy a signed HTTP download URL, access token,
    // filesystem path, or opaque user data into a potentially shared report.
    let path = prefix
        .strip_prefix("https://mod.io/g/drg/m/")
        .or_else(|| prefix.strip_prefix("http://mod.io/g/drg/m/"))?;
    if path.is_empty()
        || !path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(format!("https://mod.io/g/drg/m/{path}"))
}

// Mintfixed's resolved mod.io URL uses a `#mod_id/file_id` suffix.
// Never include arbitrary URL fragments in a crash report or context JSON.
fn clean_modio_ids(value: &str) -> (Option<String>, Option<String>) {
    let Some((_, fragment)) = value.split_once('#') else {
        return (None, None);
    };
    let Some((mod_id, file_id)) = fragment.split_once('/') else {
        return (None, None);
    };
    if !mod_id.is_empty()
        && !file_id.is_empty()
        && mod_id.bytes().all(|b| b.is_ascii_digit())
        && file_id.bytes().all(|b| b.is_ascii_digit())
    {
        (Some(mod_id.to_owned()), Some(file_id.to_owned()))
    } else {
        (None, None)
    }
}

fn reporter_directory(vanilla_pak: &Path) -> Result<PathBuf, String> {
    let fsd = vanilla_pak
        .ancestors()
        .find(|ancestor| {
            ancestor
                .file_name()
                .is_some_and(|value| value.to_string_lossy().eq_ignore_ascii_case("FSD"))
        })
        .ok_or_else(|| {
            format!(
                "cannot locate the FSD directory from {}",
                vanilla_pak.display()
            )
        })?;
    let binaries = fsd.join("Binaries");
    for platform in ["Win64", "WinGDK"] {
        let dir = binaries.join(platform).join("mintfixed-bpcrash");
        if dir.join("bpcrash.exe").is_file() {
            return Ok(dir);
        }
    }
    for platform in ["Win64", "WinGDK"] {
        let dir = binaries.join(platform).join("mintfixed-bpcrash");
        if dir.is_dir() {
            return Ok(dir);
        }
    }
    Err("BP-Crash-Reporter directory not found; cannot publish crash context".into())
}

fn write_atomically(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("output has no parent directory")?;
    fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    let unique = format!(
        "{}.{}.{}.tmp",
        CONTEXT_FILENAME,
        std::process::id(),
        now_ms()?
    );
    let staged = parent.join(unique);
    let result = (|| {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .map_err(|e| format!("create {}: {e}", staged.display()))?;
        out.write_all(contents).map_err(|e| e.to_string())?;
        out.sync_all().map_err(|e| e.to_string())?;
        drop(out);
        // Same-volume replacement. The reporter sees either the old complete
        // snapshot or the new complete snapshot, never a partially written one.
        fs::rename(&staged, path).map_err(|e| format!("replace {}: {e}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

/// Do not retain attribution from an installation that has been removed.
/// Best-effort: uninstall success never depends on the optional reporter.
pub(crate) fn invalidate_after_uninstall(merged_pak: &Path) -> Result<(), String> {
    let dir = reporter_directory(merged_pak)?;
    match fs::remove_file(dir.join(CONTEXT_FILENAME)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("remove stale crash context: {err}")),
    }
}

// `mods_P.pak` has already passed the installer's unchanged-key check on
// this path. Reuse a matching manifest instead of reopening all source PAKs.
// We still verify the current merged PAK and each source fingerprint/order;
// an absent, older, or incompatible snapshot triggers a full fallback index.
fn can_reuse_installed_context(
    latest: &Path,
    merged_pak: &Path,
    mods: &[(ModInfo, PathBuf)],
) -> bool {
    let Ok(meta) = fs::metadata(latest) else {
        return false;
    };
    if !meta.is_file() || meta.len() > 64 * 1024 * 1024 {
        return false;
    }
    let Ok(bytes) = fs::read(latest) else {
        return false;
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    if json["schema_version"] != SCHEMA_VERSION
        || json["producer"] != "mintfixed"
        || json["captured_after_successful_install"] != true
    {
        return false;
    }
    if serde_json::to_value(metadata_for(merged_pak)).ok().as_ref() != json.get("installed_pak") {
        return false;
    }
    let Some(entries) = json["installed_mods"].as_array() else {
        return false;
    };
    if entries.len() != mods.len() {
        return false;
    }
    for (i, (entry, (info, archive))) in entries.iter().zip(mods).enumerate() {
        if entry["name"].as_str() != Some(info.name.as_str())
            || entry["provider"].as_str() != Some(info.provider)
            || entry["integration_order"].as_u64() != Some(i as u64)
            || serde_json::to_value(metadata_for(archive)).ok().as_ref() != entry.get("archive")
        {
            return false;
        }
        // Never reuse legacy snapshots with multiple potentially uninstalled ZIP PAKs.
        let Some(paks) = entry["archive_index"]["paks"].as_array() else {
            return false;
        };
        if paks.len() > 1 {
            return false;
        }
        if !matches!(
            entry["archive_index"]["status"].as_str(),
            Some("complete" | "partial" | "unreadable")
        ) {
            return false;
        }
    }
    true
}

/// Best-effort, after install completion: errors are logged and must NEVER
/// turn a successful mod installation into a failed one.
pub(crate) fn write_installed_context(
    vanilla_pak: &Path,
    merged_pak: &Path,
    mods: &[(ModInfo, PathBuf)],
    prepared: Option<&[PreparedModIndex]>,
) -> Result<PathBuf, String> {
    let dir = reporter_directory(vanilla_pak)?;
    let latest = dir.join(CONTEXT_FILENAME);
    if prepared.is_none() && can_reuse_installed_context(&latest, merged_pak, mods) {
        return Ok(latest);
    }
    let stamp = now_ms()?;
    if prepared.is_some_and(|records| records.len() != mods.len()) {
        return Err("pre-pass index count differs from installed mod count".into());
    }
    let installed_mods = mods
        .iter()
        .enumerate()
        .map(|(index, (info, archive))| {
            let selected = prepared
                .and_then(|records| records.get(index))
                .cloned()
                .unwrap_or_else(|| index::index_archive(archive));
            let resolved_url = info.resolution.url.0.clone();
            let (modio_mod_id, modio_file_id) = if info.provider == "modio" {
                clean_modio_ids(&resolved_url)
            } else {
                (None, None)
            };
            InstalledMod {
                name: info.name.clone(),
                provider: info.provider.to_owned(),
                integration_order: index,
                archive: metadata_for(archive),
                archive_type: selected.kind,
                archive_index: selected.index,
                modio_reference: if info.provider == "modio" {
                    clean_modio_reference(&resolved_url)
                } else {
                    None
                },
                modio_mod_id,
                modio_file_id,
            }
        })
        .collect();
    let snapshot_path = dir.join("sdk_snapshot.json");
    let context = CrashContext {
        schema_version: SCHEMA_VERSION,
        producer: "mintfixed",
        generated_unix_ms: stamp,
        pid: std::process::id(),
        captured_after_successful_install: true,
        vanilla_pak: metadata_for(vanilla_pak),
        installed_pak: metadata_for(merged_pak),
        sdk_snapshot: snapshot_path
            .is_file()
            .then(|| snapshot_path.to_string_lossy().into_owned()),
        installed_mods,
    };
    let bytes =
        serde_json::to_vec_pretty(&context).map_err(|e| format!("serialize crash context: {e}"))?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("crash context exceeds the 64 MiB reporter limit".into());
    }
    let latest = dir.join(CONTEXT_FILENAME);
    write_atomically(&latest, &bytes)?;
    tracing::info!(
        "Crash context generated successfully: {} mods, saved to {}",
        context.installed_mods.len(),
        latest.display()
    );
    Ok(latest)
}

#[cfg(test)]
mod crash_context_tests {
    use super::*;

    #[test]
    fn unchanged_snapshot_is_reused_only_while_merged_pak_matches() {
        let temp = tempfile::tempdir().unwrap();
        let merged = temp.path().join("mods_P.pak");
        let latest = temp.path().join(CONTEXT_FILENAME);
        fs::write(&merged, b"current").unwrap();
        let json = serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "producer": "mintfixed",
            "captured_after_successful_install": true,
            "installed_pak": metadata_for(&merged),
            "installed_mods": []
        });
        fs::write(&latest, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(can_reuse_installed_context(&latest, &merged, &[]));
        fs::write(&merged, b"changed length").unwrap();
        assert!(!can_reuse_installed_context(&latest, &merged, &[]));
    }

    #[test]
    fn only_safe_modio_identifiers_are_exported() {
        assert_eq!(
            clean_modio_reference("https://mod.io/g/drg/m/test-mod?token=secret"),
            Some("https://mod.io/g/drg/m/test-mod".into())
        );
        assert_eq!(
            clean_modio_reference("https://example.com/file.zip?token=secret"),
            None
        );
        assert_eq!(clean_modio_reference("C:\\Mods\\Local.pak"), None);
        assert_eq!(clean_modio_reference("https://mod.io/g/drg/m/../bad"), None);
        assert_eq!(
            clean_modio_ids("https://mod.io/g/drg/m/test#123/45"),
            (Some("123".into()), Some("45".into()))
        );
        assert_eq!(
            clean_modio_ids("https://mod.io/g/drg/m/test#private-token"),
            (None, None)
        );
    }

    #[test]
    fn report_dir_requires_real_game_fsd_and_existing_reporter() {
        assert!(reporter_directory(Path::new("C:/other/base.pak")).is_err());
    }

    #[test]
    fn uninstall_invalidates_context_without_touching_other_files() {
        let temp = tempfile::tempdir().unwrap();
        let fsd = temp.path().join("FSD");
        let dir = fsd.join("Binaries").join("Win64").join("mintfixed-bpcrash");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("bpcrash.exe"), b"test").unwrap();
        fs::write(dir.join(CONTEXT_FILENAME), b"old snapshot").unwrap();
        fs::write(dir.join("bpcrash_test.txt"), b"keep").unwrap();
        let merged = fsd.join("Content").join("Paks").join("mods_P.pak");
        invalidate_after_uninstall(&merged).unwrap();
        assert!(!dir.join(CONTEXT_FILENAME).exists());
        assert!(dir.join("bpcrash_test.txt").exists());
        invalidate_after_uninstall(&merged).unwrap();
    }

    #[test]
    fn missing_archive_is_not_claimed_readable() {
        let missing = Path::new("__mintfixed_nonexistent_archive_12345__.zip");
        assert_eq!(index::index_archive(missing).kind, "missing");
        assert!(!metadata_for(missing).exists);
    }
}
