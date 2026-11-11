//! bitfix (<https://github.com/trumank/bitfix>): a proxy DLL that applies Lua byte patches to the
//! game exe at startup. A profile entry whose name ends in `.lua` (a local path or a URL) is a
//! bitfix patch rather than a pak. Integration copies the selected patches into
//! `Binaries/Win64/bitfix/` and installs the DLL as `d3d9.dll` beside the exe; the mint hook
//! already holds `x3daudio1_7.dll`. Patches are tracked in a manifest so that disabling them, or
//! uninstalling, removes exactly what mint placed and never a hand-installed file.
//!
//! The DLL is the official CI build of trumank/bitfix@7cd8ee9 (release asset
//! `bitfix-7cd8ee9.zip`), bundled in `assets/bitfix/` with its MIT license. bitfix only publishes a
//! rolling `latest` pre-release that CI re-cuts on every push, so a pinned download URL would rot.
//! To update: replace `assets/bitfix/bitfix.dll` and bump `DLL_SHA256` below.

use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Path;

use fs_err as fs;
use mint_lib::{DRGInstallation, DRGInstallationType};
use tracing::info;

use crate::integrate::IntegrationError;
use crate::providers::ModInfo;

const DLL: &[u8] = include_bytes!("../assets/bitfix/bitfix.dll");
#[cfg(test)]
const DLL_SHA256: &str = "b07861dac526f3617cd9652bc31e157b2240da65c66ab2fa64135ba7ba0ca5f6";
const PROXY_NAME: &str = "d3d9.dll";
const PATCH_DIR: &str = "bitfix";
const MANIFEST: &str = "mint_installed.txt";

pub fn is_patch(info: &ModInfo) -> bool {
    info.name.to_ascii_lowercase().ends_with(".lua")
}

fn err(msg: String) -> IntegrationError {
    IntegrationError::GenericError { msg }
}

fn remove_if_exists(path: &Path) -> std::io::Result<()> {
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

/// Remove mint's bitfix DLL, unless hand-installed `*.lua` patches still rely on it.
fn remove_dll_if_unused(bin: &Path, dir: &Path) -> Result<(), IntegrationError> {
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
    std::fs::remove_dir(dir).ok(); // ponytail: only succeeds when empty, which is all we want
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn conflicting_proxy_does_not_delete_previous_patches() {
        let tmp = tempfile::tempdir().unwrap();
        let installation = DRGInstallation {
            root: tmp.path().join("FSD"),
            installation_type: DRGInstallationType::Steam,
        };
        let bin = installation.binaries_directory();
        let dir = bin.join(PATCH_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("old.lua"), "return {}").unwrap();
        fs::write(dir.join(MANIFEST), "old.lua").unwrap();
        fs::write(bin.join(PROXY_NAME), "other tool").unwrap();
        let source = tmp.path().join("new.lua");
        fs::write(&source, "return {}").unwrap();
        assert!(sync(&installation, &[("new.lua", &source)]).is_err());
        assert_eq!(
            fs::read_to_string(dir.join("old.lua")).unwrap(),
            "return {}"
        );
        assert_eq!(fs::read_to_string(dir.join(MANIFEST)).unwrap(), "old.lua");
        assert_eq!(fs::read(bin.join(PROXY_NAME)).unwrap(), b"other tool");
    }
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn bundled_dll_matches_pinned_release() {
        assert_eq!(hex::encode(Sha256::digest(DLL)), DLL_SHA256);
    }

    #[test]
    fn sync_installs_and_removes_only_its_own_files() {
        let tmp = tempfile::tempdir().unwrap();
        let installation = DRGInstallation {
            root: tmp.path().join("FSD"),
            installation_type: DRGInstallationType::Steam,
        };
        let bin = installation.binaries_directory();
        let dir = bin.join(PATCH_DIR);
        fs::create_dir_all(&dir).unwrap();
        let src = tmp.path().join("fix.lua");
        fs::write(&src, "return {}").unwrap();
        let disabled = dir.join("hand.lua.disabled");
        fs::write(&disabled, "x").unwrap();

        sync(&installation, &[("fix.lua", &src)]).unwrap();
        assert_eq!(fs::read(bin.join(PROXY_NAME)).unwrap(), DLL);
        assert_eq!(
            fs::read_to_string(dir.join("fix.lua")).unwrap(),
            "return {}"
        );

        // A hand-installed patch that differs is never overwritten.
        let other = tmp.path().join("other").join("hand.lua");
        fs::create_dir_all(other.parent().unwrap()).unwrap();
        fs::write(&other, "new").unwrap();
        fs::write(dir.join("hand.lua"), "old").unwrap();
        assert!(sync(&installation, &[("hand.lua", &other)]).is_err());
        assert_eq!(fs::read_to_string(dir.join("hand.lua")).unwrap(), "old");

        // While a hand-installed patch remains, the DLL stays for it.
        sync(&installation, &[]).unwrap();
        assert!(!dir.join("fix.lua").exists());
        assert!(bin.join(PROXY_NAME).exists());

        fs::remove_file(dir.join("hand.lua")).unwrap();
        sync(&installation, &[]).unwrap();
        assert!(!bin.join(PROXY_NAME).exists());
        assert!(!dir.join(MANIFEST).exists());
        assert!(disabled.exists());
    }

    #[test]
    fn sync_refuses_a_foreign_proxy_dll() {
        let tmp = tempfile::tempdir().unwrap();
        let installation = DRGInstallation {
            root: tmp.path().join("FSD"),
            installation_type: DRGInstallationType::Steam,
        };
        let bin = installation.binaries_directory();
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join(PROXY_NAME), "reshade").unwrap();
        let src = tmp.path().join("fix.lua");
        fs::write(&src, "return {}").unwrap();
        assert!(sync(&installation, &[("fix.lua", &src)]).is_err());
        assert_eq!(fs::read(bin.join(PROXY_NAME)).unwrap(), b"reshade");
        sync(&installation, &[]).unwrap();
        assert_eq!(fs::read(bin.join(PROXY_NAME)).unwrap(), b"reshade");
    }
}
