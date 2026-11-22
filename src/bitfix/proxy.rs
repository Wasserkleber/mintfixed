//! Proxy DLL ownership and cleanup. Do not remove foreign d3d9.dll files.
use super::{DLL, PROXY_NAME};
use crate::integrate::IntegrationError;
use fs_err as fs;
use mint_lib::DRGInstallation;
use std::{io::ErrorKind, path::Path};
use tracing::info;

pub(super) fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Return an existing foreign d3d9 proxy without changing any files.
/// A bytewise match is an already-installed bundled bitfix DLL.
pub fn conflicting_proxy(
    installation: &DRGInstallation,
) -> std::io::Result<Option<std::path::PathBuf>> {
    let path = installation.binaries_directory().join(PROXY_NAME);
    match fs::read(&path) {
        Ok(existing) if existing == DLL => Ok(None),
        Ok(_) => Ok(Some(path)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Remove mint's bitfix DLL, unless hand-installed `*.lua` patches still rely on it.
pub(super) fn remove_dll_if_unused(bin: &Path, dir: &Path) -> Result<(), IntegrationError> {
    let dll = bin.join(PROXY_NAME);
    if !fs::read(&dll).is_ok_and(|bytes| bytes == DLL) {
        return Ok(());
    }
    let has_patches = std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lua"))
        })
    });
    if has_patches {
        info!(
            "bitfix: keeping {} for hand-installed patches",
            dll.display()
        );
        return Ok(());
    }
    info!("bitfix: removing {}", dll.display());
    remove_if_exists(&dll)?;
    std::fs::remove_dir(dir).ok(); // Only succeeds when empty, which is all we want
    Ok(())
}
