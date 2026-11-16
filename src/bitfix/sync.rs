//! Managed Lua patch installation. Retains manifest-based ownership checks.
use super::proxy::{conflicting_proxy, remove_dll_if_unused, remove_if_exists};
use super::{DLL, MANIFEST, PATCH_DIR, PROXY_NAME};
use crate::integrate::IntegrationError;
use fs_err as fs;
use mint_lib::{DRGInstallation, DRGInstallationType};
use std::{collections::HashSet, io::ErrorKind, path::Path};
use tracing::info;

fn err(msg: String) -> IntegrationError {
    IntegrationError::GenericError { msg }
}

/// Make the game's bitfix setup match `patches` (mod name, local file): an empty list removes
/// what mint installed. Called at the end of integration and by uninstall.
pub fn sync(
    installation: &DRGInstallation,
    patches: &[(&str, &Path)],
) -> Result<(), IntegrationError> {
    let bin = installation.binaries_directory();
    let dir = bin.join(PATCH_DIR);
    let manifest = dir.join(MANIFEST);
    // Check for an incompatible proxy before deleting managed scripts.
    if !patches.is_empty() {
        if let Some(path) = conflicting_proxy(installation)? {
            return Err(err(format!(
                "{} exists and is not mint's bitfix build; open the Bitfix menu to resolve the conflict",
                path.display()
            )));
        }
    }

    // Now remove old managed patches after DLL validation.
    let old: HashSet<String> = match fs::read_to_string(&manifest) {
        Ok(text) => text.lines().map(str::to_string).collect(),
        Err(e) if e.kind() == ErrorKind::NotFound => HashSet::new(),
        Err(e) => return Err(e.into()),
    };
    for name in &old {
        remove_if_exists(&dir.join(name))?;
    }

    if patches.is_empty() {
        remove_if_exists(&manifest)?;
        return remove_dll_if_unused(&bin, &dir);
    }
    if !matches!(installation.installation_type, DRGInstallationType::Steam) {
        return Err(err(
            "bitfix patches are only supported on the Steam version of DRG".into(),
        ));
    }

    // Then the proxy slot: never take over a d3d9.dll that is someone else's (ReShade, a
    // hand-installed bitfix build), and check it before writing anything.
    let dll = bin.join(PROXY_NAME);
    let dll_missing = match fs::read(&dll) {
        Ok(existing) if existing == DLL => false,
        Ok(_) => {
            return Err(err(format!(
                "{} exists and is not mint's bitfix build; remove it to let mint install bitfix",
                dll.display()
            )));
        }
        Err(e) if e.kind() == ErrorKind::NotFound => true,
        Err(e) => return Err(e.into()),
    };

    fs::create_dir_all(&dir)?;
    let mut installed = Vec::new();
    for &(mod_name, path) in patches {
        let name = Path::new(mod_name)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .ok_or_else(|| err(format!("bad bitfix patch name: {mod_name}")))?;
        if installed.contains(&name) {
            return Err(err(format!("two bitfix patches are both named {name}")));
        }
        let data = fs::read(path)?;
        let target = dir.join(&name);
        if !old.contains(&name) && target.exists() {
            // A hand-installed file of the same name: bitfix loads it either way, so keep it when
            // it is the same patch and refuse to clobber it when it is not.
            if fs::read(&target)? == data {
                info!("bitfix: {name} is already installed by hand; leaving it");
                continue;
            }
            return Err(err(format!(
                "{} already exists and was not installed by mint; remove or rename it",
                target.display()
            )));
        }
        fs::write(&target, &data)?;
        installed.push(name);
    }
    fs::write(&manifest, installed.join("\n"))?;

    if dll_missing {
        info!("bitfix: installing {}", dll.display());
        fs::write(&dll, DLL)?;
    }
    info!("bitfix: {} patch(es) installed", installed.len());
    Ok(())
}
