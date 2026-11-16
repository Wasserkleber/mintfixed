//! Crash reporter verification, markers, staging, and rollback.
use super::{VERSION, ZIP_SHA256};
use anyhow::{Context, Result, bail};
use fs_err as fs;
use sha2::{Digest, Sha256};
use std::path::Path;
use tracing::{info, warn};

/// A complete marker also binds the installed exe hash, not just the version.
/// Markers created by the previous installer had only two lines; refresh them.
/// The hook cannot read Mintfixed's config, so the setting reaches it as the
/// `disabled` marker it already honours. Touches only `mintfixed-bpcrash/`.
pub(super) fn sync_disabled_marker(dir: &Path, enabled: bool) -> Result<()> {
    let marker = dir.join("disabled");
    if enabled {
        match fs::remove_file(&marker) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    } else if dir.is_dir() {
        // ponytail: no folder means nothing installed, so nothing for the hook to launch.
        fs::write(&marker, "Disabled in Mintfixed settings.")?;
    }
    Ok(())
}

pub(super) fn installed_release_matches(marker: &Path, exe: &Path) -> Result<bool> {
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
pub(super) fn restore_interrupted_update(dir: &Path) -> Result<()> {
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
pub(super) fn install_verified_executable(
    dir: &Path,
    bytes: &[u8],
    exe_digest: &str,
) -> Result<()> {
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
