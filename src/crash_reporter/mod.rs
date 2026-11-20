//! Install a verified, pinned BP-Crash-Reporter release alongside the DRG game hook.
//! All installation events go to Mintfixed's existing (append-mode) mint.log.
//! The reporter is an external executable, not a .pak mod.
//! Bump VERSION, ZIP_URL and ZIP_SHA256 together when shipping a new release.
use std::{
    io::{Cursor, Read},
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use fs_err as fs;
use mint_lib::DRGInstallationType;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

mod install_files;
mod symbols;
#[cfg(target_os = "windows")]
mod windows_process;

use install_files::{
    install_verified_executable, installed_release_matches, restore_interrupted_update,
    sync_disabled_marker,
};
#[cfg(target_os = "windows")]
pub(crate) use windows_process::stop;
#[cfg(target_os = "windows")]
use windows_process::target_game_is_running;

const VERSION: &str = "1.3.0";
const ZIP_URL: &str = "https://github.com/Elytras/BP-Crash-Reporter/releases/download/1.3.0/bpcrash-1.3.0-windows.zip";
// Pinned to the checksum published with upstream release 1.3.0.
const ZIP_SHA256: &str = "7b8ecd0375633feb2ca9c80f168b9257e88289a44e08c36c24a5045c5304d549";
const MAX_ZIP_BYTES: u64 = 8 * 1024 * 1024;
const MAX_EXE_BYTES: u64 = 16 * 1024 * 1024;

// Called from synchronous integration, which may itself execute inside Tokio.
// reqwest::blocking must run outside that runtime to avoid a nested-runtime panic.
// A startup update check and Install Mods must never replace the same executable
// at the same time. Both paths use the same lock for the entire operation.
static CRASH_REPORTER_INSTALL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn version() -> &'static str {
    VERSION
}

pub(crate) fn install(
    bin_dir: &Path,
    installation_type: &DRGInstallationType,
    enabled: bool,
) -> Result<()> {
    let _guard = CRASH_REPORTER_INSTALL_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("BP-Crash-Reporter installation lock was poisoned"))?;
    install_unlocked(bin_dir, installation_type, enabled)
}

/// Start a verified upgrade only when the configured version is not installed.
/// Inform the GUI before downloading; return false without any notification if
/// the installed exe and three-line marker already match the pinned release.
/// Preserve the existing installer (including its rollback and safety checks).
pub(crate) fn install_with_notice<F>(
    bin_dir: &Path,
    installation_type: &DRGInstallationType,
    enabled: bool,
    on_update: F,
) -> Result<bool>
where
    F: FnOnce(),
{
    let _guard = CRASH_REPORTER_INSTALL_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("BP-Crash-Reporter installation lock was poisoned"))?;
    if !enabled {
        install_unlocked(bin_dir, installation_type, false)?;
        return Ok(false);
    }
    let dir = bin_dir.join("mintfixed-bpcrash");
    let marker = dir.join(format!("bpcrash-{VERSION}.installed"));
    let exe = dir.join("bpcrash.exe");
    let requires_update = !installed_release_matches(&marker, &exe)?;
    if requires_update {
        // Do not disrupt a running DRG session or show an update window for
        // an update that cannot start yet. Retry on the next Mintfixed launch.
        #[cfg(target_os = "windows")]
        {
            let process = match installation_type {
                DRGInstallationType::Steam => "FSD-Win64-Shipping.exe",
                DRGInstallationType::Xbox => "FSD-WinGDK-Shipping.exe",
            };
            if target_game_is_running(process)? {
                bail!(
                    "DRG is running; postponed BP-Crash-Reporter update until the game is closed"
                );
            }
        }
        on_update();
    }
    install_unlocked(bin_dir, installation_type, true)?;
    Ok(requires_update)
}

fn install_unlocked(
    bin_dir: &Path,
    installation_type: &DRGInstallationType,
    enabled: bool,
) -> Result<()> {
    sync_disabled_marker(&bin_dir.join("mintfixed-bpcrash"), enabled)?;
    if !enabled {
        #[cfg(target_os = "windows")]
        stop(bin_dir)?;
        info!("BP-Crash-Reporter: disabled in settings; not installing or autolaunching");
        return Ok(());
    }
    std::thread::scope(|scope| {
        scope
            .spawn(|| install_inner(bin_dir, installation_type))
            .join()
    })
    .map_err(|_| anyhow::anyhow!("BP-Crash-Reporter installer thread panicked"))?
}

fn install_inner(bin_dir: &Path, installation_type: &DRGInstallationType) -> Result<()> {
    let dir = bin_dir.join("mintfixed-bpcrash");
    info!(
        "BP-Crash-Reporter: checking installation version {VERSION} at {}",
        dir.display()
    );
    if dir.join("disabled").exists() {
        info!("BP-Crash-Reporter: skipped (disabled marker exists)");
        return Ok(());
    }

    let exe = dir.join("bpcrash.exe");
    let marker = dir.join(format!("bpcrash-{VERSION}.installed"));
    let process = match installation_type {
        DRGInstallationType::Steam => "FSD-Win64-Shipping.exe",
        DRGInstallationType::Xbox => "FSD-WinGDK-Shipping.exe",
    };

    // The first two marker lines are version + release ZIP hash. The third
    // is the extracted executable hash. Old two-line markers are migrated by
    // verifying a fresh download once, rather than trusting their presence.
    restore_interrupted_update(&dir)?;
    if installed_release_matches(&marker, &exe)? {
        info!("BP-Crash-Reporter: current version {VERSION} verified; skipping download");
    } else {
        info!(
            "BP-Crash-Reporter: installation missing, outdated or unverified; updating to {VERSION}"
        );
        info!("BP-Crash-Reporter: download started (version {VERSION}): {ZIP_URL}");
        let response = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()?
            .get(ZIP_URL)
            .header(reqwest::header::USER_AGENT, "mintfixed-bpcrash-installer")
            .send()
            .context("BP-Crash-Reporter request failed")?
            .error_for_status()
            .context("BP-Crash-Reporter download failed")?;
        let mut archive_bytes = Vec::new();
        response
            .take(MAX_ZIP_BYTES + 1)
            .read_to_end(&mut archive_bytes)?;
        if archive_bytes.len() as u64 > MAX_ZIP_BYTES {
            bail!("BP-Crash-Reporter archive exceeds size limit");
        }
        info!(
            "BP-Crash-Reporter: downloaded {} bytes",
            archive_bytes.len()
        );
        let digest = hex::encode(Sha256::digest(&archive_bytes));
        if digest != ZIP_SHA256 {
            bail!("BP-Crash-Reporter checksum mismatch: expected {ZIP_SHA256}, got {digest}");
        }
        info!("BP-Crash-Reporter: SHA-256 verified ({digest})");

        let mut archive = zip::ZipArchive::new(Cursor::new(archive_bytes))?;
        let index = (0..archive.len())
            .find(|&i| {
                archive
                    .by_index(i)
                    .map(|entry| {
                        entry.is_file()
                            && entry
                                .enclosed_name()
                                .and_then(|p| p.file_name().map(|name| name == "bpcrash.exe"))
                                .unwrap_or(false)
                    })
                    .unwrap_or(false)
            })
            .context("official BP-Crash-Reporter archive has no bpcrash.exe")?;
        let file = archive.by_index(index)?;
        if file.size() > MAX_EXE_BYTES {
            bail!("BP-Crash-Reporter executable exceeds size limit");
        }
        let mut bytes = Vec::new();
        file.take(MAX_EXE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_EXE_BYTES || !bytes.starts_with(b"MZ") {
            bail!("invalid BP-Crash-Reporter executable");
        }
        let exe_digest = hex::encode(Sha256::digest(&bytes));
        info!(
            "BP-Crash-Reporter: extracted executable ({} bytes, sha256={exe_digest})",
            bytes.len()
        );

        // Never stop the watcher while the game is running: that would leave
        // this session without crash reporting. A later Install Mods retries.
        #[cfg(target_os = "windows")]
        if target_game_is_running(process)? {
            bail!("DRG is running; postponed BP-Crash-Reporter update until the game is closed");
        }
        #[cfg(target_os = "windows")]
        if exe.exists() {
            // A normal Windows watcher may outlive DRG. Only terminate the PID
            // Mintfixed recorded, after verifying the exact executable path.
            stop(bin_dir).context("failed to stop Mintfixed-managed BP-Crash-Reporter watcher")?;
        }

        install_verified_executable(&dir, &bytes, &exe_digest)?;
        // Old marker files are deliberately removed only after success. The
        // marker is not the source of truth: next launch verifies the EXE hash.
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("bpcrash-")
                && name.ends_with(".installed")
                && name != format!("bpcrash-{VERSION}.installed")
                && let Err(err) = fs::remove_file(entry.path())
            {
                warn!("BP-Crash-Reporter: could not clean obsolete marker {name}: {err}");
            }
        }
        info!(
            "BP-Crash-Reporter: installed/updated version {VERSION} at {}",
            exe.display()
        );
    }

    let cfg = dir.join("bpcrash.cfg");
    if !cfg.exists() {
        fs::create_dir_all(&dir)?;
        fs::write(&cfg, format!("process = {process}\n"))?;
        info!(
            "BP-Crash-Reporter: created config {} (process={process})",
            cfg.display()
        );
    } else {
        info!(
            "BP-Crash-Reporter: existing config retained at {}",
            cfg.display()
        );
    }
    let license = dir.join("BP-Crash-Reporter-LICENSE.txt");
    if !license.exists() {
        fs::write(&license, include_str!("../../assets/bpcrash-LICENSE.txt"))?;
        info!(
            "BP-Crash-Reporter: installed license notice at {}",
            license.display()
        );
    }
    symbols::spawn_fetch(dir.clone(), bin_dir.join(process));
    info!("BP-Crash-Reporter: installation ready at {}", dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upgrade_marker_requires_executable_hash() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("bpcrash.exe");
        let marker = temp.path().join(format!("bpcrash-{VERSION}.installed"));
        fs::write(&exe, b"MZtest").unwrap();
        fs::write(&marker, format!("{VERSION}\n{ZIP_SHA256}\n")).unwrap();
        assert!(!installed_release_matches(&marker, &exe).unwrap());
        let hash = hex::encode(Sha256::digest(b"MZtest"));
        fs::write(&marker, format!("{VERSION}\n{ZIP_SHA256}\n{hash}\n")).unwrap();
        assert!(installed_release_matches(&marker, &exe).unwrap());
        fs::write(&exe, b"MZchanged").unwrap();
        assert!(!installed_release_matches(&marker, &exe).unwrap());
    }

    #[test]
    fn upgrading_preserves_config_and_reports() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let exe = dir.join("bpcrash.exe");
        let cfg = dir.join("bpcrash.cfg");
        let report = dir.join("bpcrash_example.txt");
        fs::write(&exe, b"MZprevious").unwrap();
        fs::write(&cfg, b"process = custom.exe\n").unwrap();
        fs::write(&report, b"important crash report").unwrap();
        let bytes = b"MZnext";
        let hash = hex::encode(Sha256::digest(bytes));
        install_verified_executable(dir, bytes, &hash).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), bytes.to_vec());
        assert!(
            installed_release_matches(&dir.join(format!("bpcrash-{VERSION}.installed")), &exe)
                .unwrap()
        );
        assert_eq!(fs::read(&cfg).unwrap(), b"process = custom.exe\n");
        assert_eq!(fs::read(&report).unwrap(), b"important crash report");
        assert!(!dir.join("bpcrash.exe.previous").exists());
    }

    #[test]
    fn interrupted_update_restores_previous_exe() {
        let temp = tempfile::tempdir().unwrap();
        let previous = temp.path().join("bpcrash.exe.previous");
        fs::write(&previous, b"MZold").unwrap();
        restore_interrupted_update(temp.path()).unwrap();
        assert_eq!(fs::read(temp.path().join("bpcrash.exe")).unwrap(), b"MZold");
        assert!(!previous.exists());
    }

    #[test]
    fn disabled_marker_follows_setting() {
        let dir = tempfile::tempdir().unwrap();
        let reporter = dir.path().join("mintfixed-bpcrash");
        sync_disabled_marker(&reporter, false).unwrap();
        assert!(!reporter.exists());
        fs::create_dir(&reporter).unwrap();
        sync_disabled_marker(&reporter, false).unwrap();
        assert!(reporter.join("disabled").is_file());
        sync_disabled_marker(&reporter, true).unwrap();
        assert!(!reporter.join("disabled").exists());
        sync_disabled_marker(&reporter, true).unwrap();
    }

    #[test]
    fn release_hash_format() {
        assert!(!VERSION.is_empty());
        assert!(ZIP_URL.contains(VERSION));
        assert_eq!(ZIP_SHA256.len(), 64);
        assert!(ZIP_SHA256.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
