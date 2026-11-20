//! Read-only hand-off from a successful mintfixed installation to BP-Crash-Reporter.
//! No archive extraction, no extra installation, no live game hook, no network.
//!
//! The `mods` argument is the FINAL deduplicated list actually passed to the
//! integration pipeline, not the full saved profile or the entire cache.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::providers::ModInfo;
use serde::Serialize;
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: u32 = 1;
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
    archive_sha256: Option<String>,
    // If the original is a ZIP, the reporter must inspect .pak entries inside it.
    nested_paks_must_be_scanned: bool,
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

fn archive_kind(path: &Path) -> &'static str {
    let Ok(mut reader) = File::open(path) else {
        return "missing";
    };
    let mut signature = [0u8; 4];
    match reader.read_exact(&mut signature) {
        Ok(())
            if matches!(
                signature,
                [0x50, 0x4B, 0x03, 0x04] | [0x50, 0x4B, 0x05, 0x06] | [0x50, 0x4B, 0x07, 0x08]
            ) =>
        {
            "zip"
        }
        Ok(()) => "pak_or_unknown",
        Err(_) => "empty_or_unreadable",
    }
}

fn sha256_file(path: &Path) -> Option<String> {
    let mut source = File::open(path).ok()?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Some(format!("{:x}", hash.finalize()))
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

/// Best-effort, after install completion: errors are logged and must NEVER
/// turn a successful mod installation into a failed one.
pub(crate) fn write_installed_context(
    vanilla_pak: &Path,
    merged_pak: &Path,
    mods: &[(ModInfo, PathBuf)],
) -> Result<PathBuf, String> {
    let dir = reporter_directory(vanilla_pak)?;
    let stamp = now_ms()?;
    let installed_mods = mods
        .iter()
        .enumerate()
        .map(|(index, (info, archive))| {
            let archive_type = archive_kind(archive);
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
                archive_type,
                archive_sha256: sha256_file(archive),
                nested_paks_must_be_scanned: archive_type == "zip",
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
    let latest = dir.join(CONTEXT_FILENAME);
    write_atomically(&latest, &bytes)?;
    // Retain an immutable installation-era snapshot as well, so later profile
    // changes do not destroy the evidence needed for an older crash report.
    let history = dir.join("crash_context_history");
    let archive = history.join(format!("crash_context_{stamp}_{}.json", std::process::id()));
    if let Err(error) = write_atomically(&archive, &bytes) {
        tracing::warn!("Could not preserve crash context history: {error}");
    }
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
    fn missing_archive_is_not_claimed_readable() {
        let missing = Path::new("__mintfixed_nonexistent_archive_12345__.zip");
        assert_eq!(archive_kind(missing), "missing");
        assert!(!metadata_for(missing).exists);
    }
}
