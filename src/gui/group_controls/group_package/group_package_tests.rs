use super::*;
use crate::packages::{IncludedFile, bundle_filename, original_file_name, unique_destination_name};
use std::io::Write;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};
#[test]
fn group_import_guard_requires_valid_settings_folder_for_missing_file() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing.pak");
    let mut group = ModGroup::default();
    group.mods.push(crate::state::ModConfig {
        id: crate::state::new_id(),
        description: None,
        spec: ModSpecification::new(missing.to_string_lossy().into_owned()),
        enabled: false,
        required: false,
        member_id: None,
        priority: 5,
    });
    assert!(group_import_requires_mod_folder(&group, false, None));
    assert!(group_import_requires_mod_folder(
        &group,
        false,
        Some(&missing)
    ));
    assert!(!group_import_requires_mod_folder(
        &group,
        false,
        Some(root.path())
    ));
    // Validation cannot mutate imported items/settings.
    assert_eq!(group.mods[0].priority, 5);
    assert!(!group.mods[0].enabled);
}

#[test]
fn group_import_guard_allows_existing_and_remote_paths_without_folder() {
    let root = tempfile::tempdir().unwrap();
    let existing = root.path().join("exists.pak");
    fs::write(&existing, b"test").unwrap();
    let mut group = ModGroup::default();
    for url in [
        existing.to_string_lossy().into_owned(),
        "https://mod.io/g/drg/m/example".to_owned(),
    ] {
        group.mods.push(crate::state::ModConfig {
            id: crate::state::new_id(),
            description: None,
            spec: ModSpecification::new(url),
            enabled: true,
            required: false,
            member_id: None,
            priority: 0,
        });
    }
    assert!(!group_import_requires_mod_folder(&group, false, None));
}

#[test]
fn bundled_paths_do_not_count_but_unbundled_missing_refs_do() {
    let root = tempfile::tempdir().unwrap();
    let mut group = ModGroup::default();
    let bundle_entry = "mintfixed-bundle:mods/0000_Example.pak";
    group.mods.push(crate::state::ModConfig {
        id: crate::state::new_id(),
        description: None,
        spec: ModSpecification::new(bundle_entry.into()),
        enabled: true,
        required: false,
        member_id: None,
        priority: 0,
    });
    assert!(group_import_requires_mod_folder(&group, true, None));
    // Treat that same fake link as missing if it occurs in ordinary JSON/TXT.
    assert!(group_import_requires_mod_folder(&group, false, None));
    group.mods.push(crate::state::ModConfig {
        id: crate::state::new_id(),
        description: None,
        spec: ModSpecification::new(
            root.path()
                .join("not-bundled.zip")
                .to_string_lossy()
                .into_owned(),
        ),
        enabled: true,
        required: false,
        member_id: None,
        priority: 0,
    });
    let missing = missing_external_group_paths(&group, true);
    assert_eq!(missing.len(), 1);
    assert!(missing[0].ends_with("not-bundled.zip"));
    assert!(group_import_requires_mod_folder(&group, true, None));
}

#[test]
fn configured_mod_folder_is_required_for_bundled_files() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("not-a-folder");
    let mut group = ModGroup::default();
    group.mods.push(crate::state::ModConfig {
        id: crate::state::new_id(),
        description: None,
        spec: ModSpecification::new("mintfixed-bundle:mods/0000_Example.pak".into()),
        enabled: true,
        required: false,
        member_id: None,
        priority: 0,
    });
    assert!(group_import_requires_mod_folder(&group, true, None));
    assert!(group_import_requires_mod_folder(
        &group,
        true,
        Some(&missing)
    ));
    assert!(!group_import_requires_mod_folder(
        &group,
        true,
        Some(root.path())
    ));
    assert_eq!(
        bundle_destination(Some(root.path())).as_deref(),
        Some(root.path())
    );
    assert!(bundle_destination(None).is_none());
    assert!(bundle_destination(Some(&missing)).is_none());
}

#[test]
fn remote_only_bundle_needs_no_configured_mod_folder() {
    let mut group = ModGroup::default();
    group.mods.push(crate::state::ModConfig {
        id: crate::state::new_id(),
        description: None,
        spec: ModSpecification::new("https://mod.io/g/drg/m/example".into()),
        enabled: true,
        required: false,
        member_id: None,
        priority: 0,
    });
    assert!(!group_import_requires_mod_folder(&group, true, None));
}

use crate::providers::ModSpecification;

#[test]
fn package_filenames_are_unique_and_safe() {
    assert_eq!(
        bundle_filename(0, r"C:\mods\tool.pak").unwrap(),
        "mods/tool.pak"
    );
    assert_eq!(
        bundle_filename(1, r"D:\more\tool.pak").unwrap(),
        "mods/tool.pak"
    );
    assert!(!safe_archive_path("mods/../escape.pak"));
    assert!(!safe_archive_path("mods/subfolder/file.pak"));
    assert!(!safe_archive_path("C:\\escape.pak"));
    assert!(!local_archive("https://mod.io/g/drg/m/example"));
}

#[test]
fn package_round_trip_preserves_json_settings_and_files() {
    let folder = tempfile::tempdir().unwrap();
    let one = folder.path().join("one.pak");
    let two = folder.path().join("two.zip");
    fs::write(&one, b"PAK-TEST-CONTENT").unwrap();
    fs::write(&two, b"ZIP-TEST-CONTENT").unwrap();
    let mut group = ModGroup::default();
    for (index, url) in [
        one.to_string_lossy().into_owned(),
        two.to_string_lossy().into_owned(),
        "https://mod.io/g/drg/m/test".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        let mut mod_config = crate::state::ModConfig {
            id: crate::state::new_id(),
            description: None,
            spec: ModSpecification::new(url),
            enabled: index != 0,
            required: false,
            member_id: None,
            priority: 7,
        };
        if index == 0 {
            mod_config.description = Some("Important".into());
        }
        group.mods.push(mod_config);
    }
    for json in [true, false] {
        let bundle = folder.path().join(if json {
            "test-json.zip"
        } else {
            "test-txt.zip"
        });
        export_bundle("Example", &group, &bundle, json).unwrap();
        let pending = inspect_bundle(&bundle).unwrap();
        assert_eq!(pending.suggested_name.as_deref(), Some("Example"));
        let extracted = extract_bundle(&bundle, folder.path()).unwrap();
        assert_eq!(
            fs::read(&extracted.mods[0].spec.url).unwrap().as_slice(),
            b"PAK-TEST-CONTENT"
        );
        assert_eq!(
            fs::read(&extracted.mods[1].spec.url).unwrap().as_slice(),
            b"ZIP-TEST-CONTENT"
        );
        assert_eq!(extracted.mods[2].spec.url, "https://mod.io/g/drg/m/test");
        if json {
            assert_eq!(extracted.mods[0].priority, 7);
            assert!(!extracted.mods[0].enabled);
            assert_eq!(extracted.mods[0].description.as_deref(), Some("Important"));
        } else {
            assert_eq!(extracted.mods[0].priority, 0);
            assert!(extracted.mods[0].enabled);
        }
    }
}

#[test]
fn unsafe_zip_entries_are_rejected_before_import() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("danger.zip");
    let mut writer = ZipWriter::new(File::create(&path).unwrap());
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer.start_file("../outside.pak", stored).unwrap();
    writer.write_all(b"BAD").unwrap();
    writer.finish().unwrap();
    assert!(inspect_bundle(&path).is_err());
    assert!(!folder.path().join("outside.pak").exists());
}

#[test]
fn package_rejects_modified_mod_before_extracting() {
    let folder = tempfile::tempdir().unwrap();
    let source = folder.path().join("source.pak");
    fs::write(&source, b"TEST-CONTENT-TO-CORRUPT").unwrap();
    let mut group = ModGroup::default();
    group.mods.push(crate::state::ModConfig {
        id: crate::state::new_id(),
        description: None,
        spec: ModSpecification::new(source.to_string_lossy().into()),
        enabled: true,
        required: false,
        member_id: None,
        priority: 0,
    });
    let bundle = folder.path().join("corrupted.zip");
    export_bundle("Corrupted", &group, &bundle, true).unwrap();
    let mut bytes = fs::read(&bundle).unwrap();
    let marker = b"TEST-CONTENT-TO-CORRUPT";
    let position = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    bytes[position] ^= 1;
    fs::write(&bundle, bytes).unwrap();
    assert!(extract_bundle(&bundle, folder.path()).is_err());
    assert!(!folder.path().join("Corrupted (mintfixed group)").exists());
}

#[test]
fn missing_pak_causes_no_partial_export() {
    let folder = tempfile::tempdir().unwrap();
    let mut group = ModGroup::default();
    group.mods.push(crate::state::ModConfig {
        id: crate::state::new_id(),
        description: None,
        spec: ModSpecification::new(folder.path().join("missing.pak").to_string_lossy().into()),
        enabled: true,
        required: false,
        member_id: None,
        priority: 0,
    });
    let output = folder.path().join("out.zip");
    assert!(export_bundle("Missing", &group, &output, true).is_err());
    assert!(!output.exists());
}

#[test]
fn old_numbered_zip_manifest_restores_unprefixed_name() {
    let older = IncludedFile {
        path: "mods/0001_OBNC_patchedMEV7.pak".into(),
        size: 0,
        sha256: String::new(),
        original_name: None,
    };
    assert_eq!(original_file_name(&older).unwrap(), "OBNC_patchedMEV7.pak");
    let newer = IncludedFile {
        original_name: Some("True Original Name.pak".into()),
        ..older
    };
    assert_eq!(
        original_file_name(&newer).unwrap(),
        "True Original Name.pak"
    );
    let mut used = HashSet::new();
    assert_eq!(unique_destination_name("Same.pak", &mut used), "Same.pak");
    assert_eq!(
        unique_destination_name("same.pak", &mut used),
        "same (2).pak"
    );
}
#[test]
fn reused_local_file_is_never_replaced_and_keeps_original_path() {
    let root = tempfile::tempdir().unwrap();
    let configured = root.path().join("configured");
    fs::create_dir(&configured).unwrap();
    let from = root.path().join("OriginalMod.pak");
    let present = configured.join("OriginalMod.pak");
    fs::write(&from, b"matching-mod-content").unwrap();
    fs::write(&present, b"matching-mod-content").unwrap();
    let original = fs::read(&present).unwrap();
    let group = ModGroup {
        id: crate::state::new_id(),
        mods: vec![crate::state::ModConfig {
            id: crate::state::new_id(),
            description: None,
            spec: ModSpecification::new(from.to_string_lossy().into_owned()),
            enabled: true,
            required: false,
            member_id: None,
            priority: 0,
        }],
        description: None,
        tint_color: None,
    };
    let archive = root.path().join("bundle.zip");
    export_bundle("Test", &group, &archive, true).unwrap();
    let restored = extract_bundle(&archive, &configured).unwrap();
    assert_eq!(Path::new(&restored.mods[0].spec.url), present);
    assert_eq!(fs::read(&present).unwrap(), original);
    assert!(!configured.join("Test (mintfixed group)").exists());
}

#[test]
fn filename_conflict_preserves_existing_and_restores_original_name() {
    let root = tempfile::tempdir().unwrap();
    let configured = root.path().join("configured");
    fs::create_dir(&configured).unwrap();
    let from = root.path().join("Something.pak");
    fs::write(&from, b"new-file").unwrap();
    fs::write(configured.join("Something.pak"), b"different-existing-file").unwrap();
    let group = ModGroup {
        id: crate::state::new_id(),
        mods: vec![crate::state::ModConfig {
            id: crate::state::new_id(),
            description: None,
            spec: ModSpecification::new(from.to_string_lossy().into_owned()),
            enabled: true,
            required: false,
            member_id: None,
            priority: 0,
        }],
        description: None,
        tint_color: None,
    };
    let archive = root.path().join("bundle.zip");
    export_bundle("Test", &group, &archive, true).unwrap();
    let imported = extract_bundle(&archive, &configured).unwrap();
    let path = Path::new(&imported.mods[0].spec.url);
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        "Something (2).pak"
    );
    assert_eq!(fs::read(path).unwrap(), b"new-file");
    assert_eq!(
        fs::read(configured.join("Something.pak")).unwrap(),
        b"different-existing-file"
    );
}

#[test]
fn import_new_mod_goes_into_root_without_subfolder() {
    let root = tempfile::tempdir().unwrap();
    let configured = root.path().join("configured");
    fs::create_dir(&configured).unwrap();
    let from = root.path().join("NewMod.pak");
    fs::write(&from, b"new mod contents").unwrap();
    let mut group = ModGroup::default();
    group.mods.push(crate::state::ModConfig {
        id: crate::state::new_id(),
        description: None,
        spec: ModSpecification::new(from.to_string_lossy().into_owned()),
        enabled: true,
        required: false,
        member_id: None,
        priority: 0,
    });
    let archive = root.path().join("group_bundle.zip");
    export_bundle("Test", &group, &archive, true).unwrap();
    let imported = extract_bundle(&archive, &configured).unwrap();
    assert_eq!(
        Path::new(&imported.mods[0].spec.url),
        configured.join("NewMod.pak")
    );
    assert_eq!(
        fs::read(configured.join("NewMod.pak")).unwrap(),
        b"new mod contents"
    );
    assert!(!configured.join("Test (mintfixed group)").exists());
}

#[test]
fn export_uses_original_zip_member_names_and_manifest_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("ElytrasModManager_P.pak");
    fs::write(&a, b"first payload").unwrap();
    let other = temp.path().join("different");
    fs::create_dir(&other).unwrap();
    let b = other.join("ElytrasModManager_P.pak");
    fs::write(&b, b"different payload").unwrap();
    let mut group = ModGroup::default();
    for source in [&a, &b] {
        group.mods.push(crate::state::ModConfig {
            id: crate::state::new_id(),
            description: None,
            spec: ModSpecification::new(source.to_string_lossy().into_owned()),
            enabled: true,
            required: false,
            member_id: None,
            priority: 0,
        });
    }
    let zip_path = temp.path().join("group-export.zip");
    export_bundle("Example", &group, &zip_path, true).unwrap();
    let mut zip = ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
    assert!(zip.by_name("mods/ElytrasModManager_P.pak").is_ok());
    assert!(zip.by_name("mods/ElytrasModManager_P (2).pak").is_ok());
    let manifest: Manifest =
        serde_json::from_reader(zip.by_name("manifest.json").unwrap()).unwrap();
    assert_eq!(manifest.files.len(), 2);
    assert_eq!(
        manifest.files[0].original_name.as_deref(),
        Some("ElytrasModManager_P.pak")
    );
    assert_eq!(
        manifest.files[1].original_name.as_deref(),
        Some("ElytrasModManager_P.pak")
    );
    assert_ne!(manifest.files[0].sha256, manifest.files[1].sha256);
    drop(zip);
    let destination = temp.path().join("imports");
    fs::create_dir(&destination).unwrap();
    let restored = extract_bundle(&zip_path, &destination).unwrap();
    assert_eq!(
        Path::new(&restored.mods[0].spec.url)
            .file_name()
            .unwrap()
            .to_string_lossy(),
        "ElytrasModManager_P.pak"
    );
    assert_eq!(
        Path::new(&restored.mods[1].spec.url)
            .file_name()
            .unwrap()
            .to_string_lossy(),
        "ElytrasModManager_P (2).pak"
    );
    assert_eq!(
        fs::read(&restored.mods[0].spec.url).unwrap(),
        b"first payload"
    );
    assert_eq!(
        fs::read(&restored.mods[1].spec.url).unwrap(),
        b"different payload"
    );
}

#[test]
fn zip_member_paths_allow_safe_spaces_and_reject_traversal() {
    assert!(safe_archive_path("mods/Mod With Spaces (2).pak"));
    assert!(!safe_archive_path("mods/../escape.pak"));
    assert!(!safe_archive_path("mods/subdirectory/escape.pak"));
    assert!(!safe_archive_path("mods/C:evil.pak"));
}
