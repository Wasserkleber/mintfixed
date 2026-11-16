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

use crate::providers::ModInfo;

mod proxy;
mod sync;

pub use proxy::conflicting_proxy;
pub use sync::sync;

const DLL: &[u8] = include_bytes!("../../assets/bitfix/bitfix.dll");
#[cfg(test)]
const DLL_SHA256: &str = "b07861dac526f3617cd9652bc31e157b2240da65c66ab2fa64135ba7ba0ca5f6";
const PROXY_NAME: &str = "d3d9.dll";
const PATCH_DIR: &str = "bitfix";
const MANIFEST: &str = "mint_installed.txt";

pub fn is_patch(info: &ModInfo) -> bool {
    info.name.to_ascii_lowercase().ends_with(".lua")
}

#[cfg(test)]
mod tests {
    use fs_err as fs;
    use mint_lib::{DRGInstallation, DRGInstallationType};
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
