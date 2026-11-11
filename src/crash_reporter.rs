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
#[cfg(target_os = "windows")]
use windows::{
    Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, QueryFullProcessImageNameW,
                TerminateProcess, WaitForSingleObject,
            },
        },
    },
    core::PWSTR,
};

const VERSION: &str = "1.2.1";
const ZIP_URL: &str = "https://github.com/Elytras/BP-Crash-Reporter/releases/download/1.2.1/bpcrash-1.2.1-windows.zip";
// Pinned to the checksum published with upstream release 1.2.1.
const ZIP_SHA256: &str = "df8ba682ce28d91e2693b6b3b512f9457fb00a3e5fdb49e6ce18e819c046e76b";
const MAX_ZIP_BYTES: u64 = 8 * 1024 * 1024;
const MAX_EXE_BYTES: u64 = 16 * 1024 * 1024;

// Called from synchronous integration, which may itself execute inside Tokio.
// reqwest::blocking must run outside that runtime to avoid a nested-runtime panic.
pub(crate) fn install(bin_dir: &Path, installation_type: &DRGInstallationType) -> Result<()> {
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
        fs::write(&license, include_str!("../assets/bpcrash-LICENSE.txt"))?;
        info!(
            "BP-Crash-Reporter: installed license notice at {}",
            license.display()
        );
    }
    info!("BP-Crash-Reporter: installation ready at {}", dir.display());
    Ok(())
}

/// A complete marker also binds the installed exe hash, not just the version.
/// Markers created by the previous installer had only two lines; refresh them.
fn installed_release_matches(marker: &Path, exe: &Path) -> Result<bool> {
    let marker_text = match fs::read_to_string(marker) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).context("cannot read crash reporter installation marker"),
    };
    let mut lines = marker_text.lines();
    if lines.next() != Some(VERSION) || lines.next() != Some(ZIP_SHA256) {
        return Ok(false);
    }
    let Some(expected_exe_hash) = lines.next() else {
        return Ok(false);
    };
    if expected_exe_hash.len() != 64 || lines.next().is_some() {
        return Ok(false);
    }
    let bytes = match fs::read(exe) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).context("cannot check installed crash reporter executable"),
    };
    Ok(hex::encode(Sha256::digest(&bytes)) == expected_exe_hash)
}

/// If the process was interrupted after renaming the old exe but before the
/// new exe was installed, restore the old one on the next Install Mods.
fn restore_interrupted_update(dir: &Path) -> Result<()> {
    let exe = dir.join("bpcrash.exe");
    let previous = dir.join("bpcrash.exe.previous");
    if !exe.exists() && previous.exists() {
        fs::rename(&previous, &exe).context("cannot recover previous BP-Crash-Reporter")?;
        info!("BP-Crash-Reporter: restored previous executable from an interrupted update");
    }
    Ok(())
}

/// Stage and verify *before* touching the installed exe. If replacement fails,
/// try to restore the previous version; leave crash reports and cfg untouched.
fn install_verified_executable(dir: &Path, bytes: &[u8], exe_digest: &str) -> Result<()> {
    fs::create_dir_all(dir)?;
    let exe = dir.join("bpcrash.exe");
    let staging = dir.join("bpcrash.exe.new");
    let previous = dir.join("bpcrash.exe.previous");
    let marker = dir.join(format!("bpcrash-{VERSION}.installed"));
    let staged_marker = dir.join(format!("bpcrash-{VERSION}.installed.new"));

    fs::write(&staging, bytes).context("cannot stage BP-Crash-Reporter executable")?;
    if hex::encode(Sha256::digest(fs::read(&staging)?)) != exe_digest {
        bail!("staged BP-Crash-Reporter executable failed SHA-256 verification");
    }
    fs::write(
        &staged_marker,
        format!("{VERSION}\n{ZIP_SHA256}\n{exe_digest}\n"),
    )?;

    // A leftover previous file is safe to remove only while the current exe
    // exists. Otherwise restore_interrupted_update() must recover it first.
    if exe.exists() && previous.exists() {
        fs::remove_file(&previous).context("cannot clear previous crash reporter backup")?;
    }
    let had_previous = exe.exists();
    if had_previous {
        fs::rename(&exe, &previous).context(
            "cannot replace crash reporter; close DRG and any manually launched reporter",
        )?;
    }
    if let Err(err) = fs::rename(&staging, &exe) {
        if had_previous && let Err(restore_err) = fs::rename(&previous, &exe) {
            return Err(anyhow::anyhow!(
                "install failed: {err}; rollback failed: {restore_err}"
            ));
        }
        return Err(err).context("cannot install updated crash reporter");
    }
    // The newly installed version is now on disk. If committing its marker
    // fails, roll back the executable and leave the old marker intact.
    let commit = (|| -> Result<()> {
        if marker.exists() {
            fs::remove_file(&marker)?;
        }
        fs::rename(&staged_marker, &marker)?;
        Ok(())
    })();
    if let Err(err) = commit {
        if had_previous {
            let _ = fs::remove_file(&exe);
            if let Err(restore_err) = fs::rename(&previous, &exe) {
                return Err(anyhow::anyhow!(
                    "marker commit failed: {err}; rollback failed: {restore_err}"
                ));
            }
        } else {
            let _ = fs::remove_file(&exe);
        }
        return Err(err).context("could not finalize crash reporter update");
    }
    if had_previous && let Err(err) = fs::remove_file(&previous) {
        warn!(
            "BP-Crash-Reporter: old executable backup left at {}: {err}",
            previous.display()
        );
    }
    Ok(())
}

/// A Windows watcher should never be terminated while DRG is running.
#[cfg(target_os = "windows")]
fn target_game_is_running(exe_name: &str) -> Result<bool> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .context("cannot enumerate running processes")?;
    let result = (|| -> Result<bool> {
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        unsafe { Process32FirstW(snapshot, &mut entry) }
            .context("failed to enumerate Windows processes")?;
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|&ch| ch == 0)
                .unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..end]).eq_ignore_ascii_case(exe_name) {
                return Ok(true);
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
        Ok(false)
    })();
    let _ = unsafe { CloseHandle(snapshot) };
    result
}

/// Stop only Mintfixed's recorded watcher if the PID is still the exact installed
/// executable. Never terminate a manually installed reporter elsewhere.
#[cfg(target_os = "windows")]
pub(crate) fn stop(bin_dir: &Path) -> Result<()> {
    let dir = bin_dir.join("mintfixed-bpcrash");
    let pid_file = dir.join("bpcrash.pid");
    let pid = match fs::read_to_string(&pid_file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
    {
        Some(pid) => pid,
        None => {
            info!("BP-Crash-Reporter: no Mintfixed-managed process to stop");
            return Ok(());
        }
    };
    let handle = match unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    } {
        Ok(handle) => handle,
        Err(_) => {
            info!("BP-Crash-Reporter: recorded PID {pid} is no longer available");
            let _ = fs::remove_file(pid_file);
            return Ok(());
        }
    };
    let mut buffer = vec![0u16; 32768];
    let mut len = buffer.len() as u32;
    let path_matches = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    }
    .is_ok()
        && String::from_utf16_lossy(&buffer[..len as usize])
            .eq_ignore_ascii_case(&dir.join("bpcrash.exe").to_string_lossy());
    if path_matches {
        let terminated = unsafe { TerminateProcess(handle, 0) };
        if let Err(err) = terminated {
            let _ = unsafe { CloseHandle(handle) };
            bail!("failed to stop Mintfixed-managed watcher (pid {pid}): {err}");
        }
        if unsafe { WaitForSingleObject(handle, 5000) } != WAIT_OBJECT_0 {
            let _ = unsafe { CloseHandle(handle) };
            bail!("timed out waiting for Mintfixed-managed watcher to stop (pid {pid})");
        }
        info!("BP-Crash-Reporter: stopped Mintfixed-managed watcher (pid {pid})");
    } else {
        info!("BP-Crash-Reporter: PID {pid} does not match installed reporter; left it running");
    }
    let _ = unsafe { CloseHandle(handle) };
    let _ = fs::remove_file(pid_file);
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
    fn release_hash_format() {
        assert!(!VERSION.is_empty());
        assert!(ZIP_URL.contains(VERSION));
        assert_eq!(ZIP_SHA256.len(), 64);
        assert!(ZIP_SHA256.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
