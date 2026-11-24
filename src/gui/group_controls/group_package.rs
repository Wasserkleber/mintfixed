//! Portable group packages: one ZIP contains a JSON/TXT group definition and
//! copies of its local PAK/ZIP mods. Remote mod.io URLs are left untouched.
//! Archive names and sizes are validated; extraction never trusts archive paths.
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use super::import_export::{parse_mod_list_txt, parse_single_group_json};
use super::management::PendingGroupImport;
use crate::packages::{ArchiveKind, Manifest, MAX_DEFINITION, MAX_FILES, MAX_MANIFEST, MAX_TOTAL, PREFIX, bounded_text, extract_mod_files, local_archive, safe_archive_path, unique_bundle_filename, validate_entries};
use crate::state::ModGroup;

const FORMAT: &str = "mintfixed-group-package";
const VERSION: u32 = 1;

struct Preview {
    manifest: Manifest,
    group: ModGroup,
}

fn definition(name: &str, group: &ModGroup, json: bool) -> Result<String, String> {
    if json {
        let groups = BTreeMap::from([(name.to_owned(), group.clone())]);
        serde_json::to_string_pretty(&serde_json::json!({
            "format": "mintfixed-groups", "version": 1, "groups": groups
        }))
        .map_err(|e| format!("Cannot serialize group: {e}"))
    } else {
        if group
            .mods
            .iter()
            .any(|m| m.spec.url.contains('\r') || m.spec.url.contains('\n'))
        {
            return Err("A mod URL contains a newline and cannot be exported as TXT".into());
        }
        Ok(group
            .mods
            .iter()
            .map(|m| m.spec.url.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n")
    }
}

/// Create a new archive without overwriting any previous export. On a partial
/// failure remove the incomplete archive, so it cannot later be imported.
pub(super) fn export_bundle(
    name: &str,
    group: &ModGroup,
    destination: &Path,
    json: bool,
) -> Result<(), String> {
    let mut modified = group.clone();
    let mut sources: Vec<(PathBuf, String, u64)> = Vec::new();
    let mut declared_total = 0_u64;
    let mut archive_names = HashSet::new();
    for item in modified.mods.iter_mut() {
        if !local_archive(&item.spec.url) {
            continue;
        }
        if sources.len() >= MAX_FILES {
            return Err("This group contains too many local files to package".into());
        }
        let source = PathBuf::from(&item.spec.url);
        let metadata = fs::metadata(&source)
            .map_err(|e| format!("Missing local mod {}: {e}", source.display()))?;
        if !metadata.is_file() {
            return Err(format!("Not a regular file: {}", source.display()));
        }
        declared_total = declared_total
            .checked_add(metadata.len())
            .ok_or("Total package size overflow")?;
        if declared_total > MAX_TOTAL {
            return Err("Bundled mods exceed the 8 GiB size limit".into());
        }
        let member = unique_bundle_filename(&item.spec.url, &mut archive_names)?;
        item.spec.url = format!("{PREFIX}{member}");
        sources.push((source, member, metadata.len()));
    }

    let entry_name = if json { "group.json" } else { "group.txt" };
    let text = definition(name, &modified, json)?;
    if text.len() as u64 > MAX_DEFINITION {
        return Err("Group definition exceeds 8 MiB".into());
    }

    crate::packages::write_package(
        destination,
        Manifest {
            format: FORMAT.into(),
            version: VERSION,
            name: name.into(),
            definition: entry_name.into(),
            files: Vec::new(),
        },
        text.as_bytes(),
        sources,
        ArchiveKind::Group,
    )
}


fn preview(path: &Path) -> Result<Preview, String> {
    let file = File::open(path).map_err(|e| format!("Cannot read package: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Invalid group ZIP: {e}"))?;
    let names = validate_entries(&mut archive, ArchiveKind::Group)?;
    let manifest_text = bounded_text(
        &mut archive
            .by_name("manifest.json")
            .map_err(|_| "Missing manifest.json")?,
        MAX_MANIFEST,
        ArchiveKind::Group,
    )?;
    let manifest: Manifest = serde_json::from_str(&manifest_text)
        .map_err(|e| format!("Invalid group package manifest: {e}"))?;
    if manifest.format != FORMAT || manifest.version != VERSION {
        return Err("Unsupported group package format/version".into());
    }
    if manifest.name.trim().is_empty() || manifest.name.len() > 512 {
        return Err("Invalid group name in ZIP".into());
    }
    if manifest.definition != "group.json" && manifest.definition != "group.txt" {
        return Err("The group definition must be group.json or group.txt".into());
    }
    if manifest.files.len() > MAX_FILES || archive.len() != manifest.files.len() + 2 {
        return Err("The ZIP contains unexpected or unlisted files".into());
    }
    let mut known = HashSet::new();
    for file in &manifest.files {
        if !safe_archive_path(&file.path) || !known.insert(file.path.clone()) {
            return Err(format!(
                "Invalid or duplicate packaged filename: {}",
                file.path
            ));
        }
        if file.sha256.len() != 64 || !file.sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("Invalid checksum for {}", file.path));
        }
        if archive
            .by_name(&file.path)
            .map_err(|_| format!("Missing {}", file.path))?
            .size()
            != file.size
        {
            return Err(format!("Incorrect file size in manifest: {}", file.path));
        }
    }
    for name in names {
        if name != "manifest.json"
            && name != manifest.definition
            && !known.iter().any(|v| v.eq_ignore_ascii_case(&name))
        {
            return Err(format!("Unexpected file in ZIP: {name}"));
        }
    }
    let text = bounded_text(
        &mut archive
            .by_name(&manifest.definition)
            .map_err(|_| "Missing group definition")?,
        MAX_DEFINITION,
        ArchiveKind::Group,
    )?;
    let pending = if manifest.definition == "group.json" {
        parse_single_group_json(&text)?
    } else {
        let mut groups = parse_mod_list_txt(&text, &manifest.name)?;
        PendingGroupImport {
            suggested_name: Some(manifest.name.clone()),
            group: groups.remove(&manifest.name).ok_or("Missing group")?,
        }
    };
    if pending.suggested_name.as_deref() != Some(manifest.name.as_str()) {
        return Err("The group name differs from the package manifest".into());
    }
    let mut referenced = HashSet::new();
    for item in &pending.group.mods {
        if let Some(path) = item.spec.url.strip_prefix(PREFIX) {
            if !known.contains(path) {
                return Err(format!("Unknown bundled mod reference: {path}"));
            }
            referenced.insert(path.to_owned());
        }
    }
    if referenced != known {
        return Err("The group does not reference every packaged mod".into());
    }
    Ok(Preview {
        manifest,
        group: pending.group,
    })
}

/// Inspect only: no files are written before the user confirms group import.
pub(super) fn inspect_bundle(path: &Path) -> Result<PendingGroupImport, String> {
    let parsed = preview(path)?;
    Ok(PendingGroupImport {
        suggested_name: Some(parsed.manifest.name),
        group: parsed.group,
    })
}

/// Missing *external* local references in a pending group import.
/// A portable ZIP holds its `mintfixed-bundle:` entries inside the archive,
/// so those references do not require finding an original machine's file.
pub(super) fn missing_external_group_paths(group: &ModGroup, portable_zip: bool) -> Vec<String> {
    crate::gui::missing_local_files::group_missing_paths(group)
        .into_iter()
        .filter(|path| !(portable_zip && path.starts_with(PREFIX)))
        .collect()
}

/// Require a valid Settings folder for missing external paths or bundled local files.
/// Remote-only bundles and existing local references need no extraction folder.
pub(super) fn group_import_requires_mod_folder(
    group: &ModGroup,
    portable_zip: bool,
    configured_folder: Option<&Path>,
) -> bool {
    (portable_zip
        && group
            .mods
            .iter()
            .any(|item| item.spec.url.starts_with(PREFIX))
        || !missing_external_group_paths(group, portable_zip).is_empty())
        && !configured_folder.is_some_and(Path::is_dir)
}

/// Use Settings only; an absent/inaccessible directory is an import error.
pub(super) fn bundle_destination(configured: Option<&Path>) -> Option<PathBuf> {
    configured
        .filter(|folder| folder.is_dir())
        .map(Path::to_path_buf)
}

/// Extract validated bundle files directly into the Settings mod folder.
/// Existing files are never overwritten; matching SHA-256 files are reused.
pub(super) fn extract_bundle(path: &Path, folder: &Path) -> Result<ModGroup, String> {
    let parsed = preview(path)?;
    let file = File::open(path).map_err(|e| format!("Cannot reopen ZIP: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Cannot reopen ZIP: {e}"))?;
    let mut installed = extract_mod_files(
        &mut archive,
        &parsed.manifest.files,
        folder,
        &parsed.manifest.name,
        "group",
    )?;
    let mut imported_group = parsed.group;
    for item in &mut imported_group.mods {
        if let Some(relative) = item.spec.url.strip_prefix(PREFIX) {
            item.spec.url = installed
                .mapping
                .get(relative)
                .ok_or("Missing extracted mod")?
                .clone();
        }
    }
    installed.commit();
    Ok(imported_group)
}

#[cfg(test)]
mod group_package_tests {
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
}
