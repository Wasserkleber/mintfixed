// Public-API regression tests for mint_lib's installation-path extraction.
use std::path::PathBuf;

use mint_lib::{DRGInstallation, DRGInstallationType, built_info};

#[test]
fn steam_installation_paths_and_legacy_root_exports() {
    let fsd = PathBuf::from("games").join("FSD");
    let pak = fsd.join("Content/Paks/FSD-WindowsNoEditor.pak");
    let install = DRGInstallation::from_pak_path(&pak).unwrap();
    assert!(matches!(
        install.installation_type,
        DRGInstallationType::Steam
    ));
    assert_eq!(install.root, fsd);
    assert_eq!(install.paks_path(), fsd.join("Content/Paks"));
    assert_eq!(install.main_pak(), pak);
    assert_eq!(install.binaries_directory(), fsd.join("Binaries/Win64"));
    assert_eq!(install.installation_type.hook_dll_name(), "x3daudio1_7.dll");
    assert_eq!(install.installation_type.binaries_directory_name(), "Win64");
}

#[test]
fn xbox_installation_paths_and_case_insensitive_pak_name() {
    let fsd = PathBuf::from("games").join("FSD");
    let pak = fsd.join("Content/Paks/fsd-wingdk.pak");
    let install = DRGInstallation::from_pak_path(&pak).unwrap();
    assert!(matches!(
        install.installation_type,
        DRGInstallationType::Xbox
    ));
    assert_eq!(install.root, fsd);
    assert_eq!(install.paks_path(), fsd.join("Content/Paks"));
    assert_eq!(install.main_pak(), fsd.join("Content/Paks/FSD-WinGDK.pak"));
    assert_eq!(install.binaries_directory(), fsd.join("Binaries/WinGDK"));
    assert_eq!(install.installation_type.hook_dll_name(), "d3d9.dll");
    assert_eq!(
        install.installation_type.binaries_directory_name(),
        "WinGDK"
    );
}

#[test]
fn rejects_unknown_pak_or_missing_parent_without_panicking() {
    assert!(DRGInstallationType::from_pak_path("FSD-Unknown.pak").is_err());
    assert!(DRGInstallation::from_pak_path("FSD-WinGDK.pak").is_err());
}

#[test]
fn generated_version_accessors_retain_their_public_contract() {
    let version = built_info::version();
    assert!(!version.trim().is_empty());
    assert_eq!(
        built_info::cli_version(),
        version.strip_prefix("mintfixed ").unwrap_or(version)
    );
}
