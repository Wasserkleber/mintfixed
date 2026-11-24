use super::*;
use std::io::Write;
use zip::{ZipWriter, write::SimpleFileOptions};
use crate::providers::ModSpecification;
use crate::state::{MemberOverrides, ModConfig, ModProfile_v0_1_0 as Profile};

fn member(url: String, priority: i32) -> ModConfig {
    ModConfig {
        id: crate::state::new_id(),
        description: Some("Keep settings".into()),
        spec: ModSpecification::new(url),
        enabled: false,
        required: true,
        member_id: Some(crate::state::new_id()),
        priority,
    }
}
fn sample(root: &Path) -> (Data, PathBuf, PathBuf) {
    let a = root.join("mod.pak");
    let b = root.join("mod.zip");
    fs::write(&a, b"one").unwrap();
    fs::write(&b, b"two").unwrap();
    let mut data = Data::default();
    let member_a = member(a.to_string_lossy().into_owned(), 13);
    let mut member_b = member(b.to_string_lossy().into_owned(), 42);
    let mut overrides = MemberOverrides::default();
    overrides.set(&mut member_b, true);
    data.groups.insert(
        "Tools".into(),
        ModGroup {
            id: crate::state::new_id(),
            mods: vec![member_b],
            description: Some("Group notes".into()),
            tint_color: None,
        },
    );
    data.profiles.insert(
        "Portable".into(),
        Profile {
            id: crate::state::new_id(),
            mods: vec![
                ModOrGroup::Individual(member_a),
                ModOrGroup::Group {
                    id: crate::state::new_id(),
                    group_name: "Tools".into(),
                    enabled: false,
                    member_overrides: overrides,
                },
            ],
        },
    );
    data.active_profile = "Portable".into();
    (data, a, b)
}
#[test]
fn full_profile_roundtrip_keeps_groups_disabled_mods_priorities_and_files() {
    let root = tempfile::tempdir().unwrap();
    let (data, _, _) = sample(root.path());
    let bundle = root.path().join("portable.zip");
    export_prepared(prepare_export(&data, "Portable").unwrap(), &bundle).unwrap();
    let (name, json, has_files) = inspect_bundle(&bundle).unwrap();
    assert_eq!(name, "Portable");
    assert!(has_files);
    assert!(require_mod_folder(&json, true, None).is_err());
    assert!(require_mod_folder(&json, true, Some(root.path())).is_ok());
    let mut extracted = extract_bundle(&bundle, Some(root.path())).unwrap();
    let doc: ProfileDocument = serde_json::from_str(&extracted.json).unwrap();
    assert_eq!(doc.mods.len(), 2);
    assert_eq!(doc.groups["Tools"].mods[0].priority, 42);
    assert!(!doc.groups["Tools"].mods[0].enabled);
    if let ModOrGroup::Group {
        member_overrides,
        enabled,
        ..
    } = &doc.mods[1]
    {
        assert!(!*enabled);
        assert!(member_overrides.enabled(&doc.groups["Tools"].mods[0]));
    } else {
        panic!("Group placement was not preserved");
    }
    assert_eq!(
        doc.groups["Tools"].description.as_deref(),
        Some("Group notes")
    );
    assert!(
        !doc.groups["Tools"].mods[0]
            .spec
            .url
            .starts_with(shared::PREFIX)
    );
    for url in urls(&doc) {
        assert!(Path::new(url).exists());
    }
    extracted.commit();
}
#[test]
fn repeated_exports_use_no_overwrite_and_missing_file_fails_before_zip_creation() {
    let root = tempfile::tempdir().unwrap();
    let (data, a, _) = sample(root.path());
    let bundle = root.path().join("portable.zip");
    export_prepared(prepare_export(&data, "Portable").unwrap(), &bundle).unwrap();
    assert!(export_prepared(prepare_export(&data, "Portable").unwrap(), &bundle).is_err());
    fs::remove_file(a).unwrap();
    assert!(prepare_export(&data, "Portable").is_err());
}
#[test]
fn corrupted_payload_is_rejected_without_creating_import_folder() {
    let root = tempfile::tempdir().unwrap();
    let (data, _, _) = sample(root.path());
    let bundle = root.path().join("portable.zip");
    export_prepared(prepare_export(&data, "Portable").unwrap(), &bundle).unwrap();
    let mut bytes = fs::read(&bundle).unwrap();
    let offset = bytes.windows(3).position(|v| v == b"one").unwrap();
    bytes[offset] ^= 1;
    fs::write(&bundle, bytes).unwrap();
    assert!(extract_bundle(&bundle, Some(root.path())).is_err());
    assert!(!root.path().join("Portable (mintfixed profile)").exists());
}
#[test]
fn remote_only_profile_needs_no_mod_folder() {
    let root = tempfile::tempdir().unwrap();
    let (mut data, _, _) = sample(root.path());
    if let ModOrGroup::Individual(item) =
        &mut data.profiles.get_mut("Portable").unwrap().mods[0]
    {
        item.spec.url = "https://mod.io/g/drg/m/foo".into();
    }
    data.groups.get_mut("Tools").unwrap().mods[0].spec.url =
        "https://mod.io/g/drg/m/bar".into();
    let bundle = root.path().join("portable.zip");
    export_prepared(prepare_export(&data, "Portable").unwrap(), &bundle).unwrap();
    let (_, json, has_files) = inspect_bundle(&bundle).unwrap();
    assert!(!has_files);
    assert!(require_mod_folder(&json, true, None).is_ok());
    let extracted = extract_bundle(&bundle, None).unwrap();
    assert!(!extracted.json.is_empty());
}
#[test]
fn untrusted_entries_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("evil.zip");
    let mut writer = ZipWriter::new(File::create(&path).unwrap());
    writer
        .start_file("../escape.pak", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"bad").unwrap();
    writer.finish().unwrap();
    assert!(inspect_bundle(&path).is_err());
}

#[test]
fn profile_import_reuses_existing_local_mods_without_creating_folder() {
    let root = tempfile::tempdir().unwrap();
    let (data, source, _) = sample(root.path());
    let configured = root.path().join("configured");
    fs::create_dir(&configured).unwrap();
    let existing = configured.join("mod.pak");
    fs::write(&existing, fs::read(&source).unwrap()).unwrap();
    // Both source files already available: no extraction subfolder needed.
    let other = configured.join("mod.zip");
    fs::write(&other, b"two").unwrap();
    let archive = root.path().join("bundle.zip");
    export_prepared(prepare_export(&data, "Portable").unwrap(), &archive).unwrap();
    let mut imported = extract_bundle(&archive, Some(&configured)).unwrap();
    assert!(imported.files.is_empty());
    let doc: ProfileDocument = serde_json::from_str(&imported.json).unwrap();
    let urls = urls(&doc);
    assert!(urls.iter().any(|u| Path::new(u) == existing));
    assert!(urls.iter().any(|u| Path::new(u) == other));
    imported.commit();
}

#[test]
fn uncommitted_flat_import_only_removes_created_files() {
    let root = tempfile::tempdir().unwrap();
    let (data, source, _) = sample(root.path());
    let configured = root.path().join("configured");
    fs::create_dir(&configured).unwrap();
    let reused = configured.join("mod.pak");
    fs::copy(source, &reused).unwrap();
    let archive = root.path().join("bundle.zip");
    export_prepared(prepare_export(&data, "Portable").unwrap(), &archive).unwrap();
    {
        let result = extract_bundle(&archive, Some(&configured)).unwrap();
        let doc: ProfileDocument = serde_json::from_str(&result.json).unwrap();
        assert!(urls(&doc).iter().any(|url| Path::new(url) == reused));
        assert!(configured.join("mod.zip").is_file());
    }
    assert!(reused.is_file(), "reused local file must survive rollback");
    assert!(
        !configured.join("mod.zip").exists(),
        "new file must be removed on rollback"
    );
}
#[test]
fn committed_flat_profile_import_leaves_new_files() {
    let root = tempfile::tempdir().unwrap();
    let (data, _, _) = sample(root.path());
    let configured = root.path().join("configured");
    fs::create_dir(&configured).unwrap();
    let archive = root.path().join("bundle.zip");
    export_prepared(prepare_export(&data, "Portable").unwrap(), &archive).unwrap();
    let mut result = extract_bundle(&archive, Some(&configured)).unwrap();
    assert!(configured.join("mod.pak").is_file());
    assert!(configured.join("mod.zip").is_file());
    result.commit();
    drop(result);
    assert!(configured.join("mod.pak").is_file());
    assert!(configured.join("mod.zip").is_file());
}

#[test]
fn profile_export_uses_real_pak_names() {
    let temp = tempfile::tempdir().unwrap();
    let (data, _, _) = sample(temp.path());
    let dest = temp.path().join("clean-profile.zip");
    export_prepared(prepare_export(&data, "Portable").unwrap(), &dest).unwrap();
    let mut zip = ZipArchive::new(File::open(&dest).unwrap()).unwrap();
    assert!(zip.by_name("mods/mod.pak").is_ok());
    assert!(zip.by_name("mods/mod.zip").is_ok());
    let manifest: Manifest =
        serde_json::from_reader(zip.by_name("manifest.json").unwrap()).unwrap();
    assert_eq!(manifest.files.len(), 2);
    assert!(
        manifest
            .files
            .iter()
            .any(|entry| entry.original_name.as_deref() == Some("mod.pak"))
    );
    assert!(
        manifest
            .files
            .iter()
            .any(|entry| entry.original_name.as_deref() == Some("mod.zip"))
    );
    assert!(
        manifest
            .files
            .iter()
            .all(|entry| !entry.path.starts_with("mods/0000_"))
    );
}

#[test]
fn profile_export_handles_duplicate_basename_without_numbering_all_files() {
    let root = tempfile::tempdir().unwrap();
    let (mut data, _original, _) = sample(root.path());
    let folder = root.path().join("other");
    fs::create_dir(&folder).unwrap();
    let second = folder.join("mod.pak");
    fs::write(&second, b"other mod contents").unwrap();
    data.groups.get_mut("Tools").unwrap().mods[0].spec.url =
        second.to_string_lossy().into_owned();
    let dest = root.path().join("duplicate-names.zip");
    export_prepared(prepare_export(&data, "Portable").unwrap(), &dest).unwrap();
    let mut zip = ZipArchive::new(File::open(&dest).unwrap()).unwrap();
    assert!(zip.by_name("mods/mod.pak").is_ok());
    assert!(zip.by_name("mods/mod (2).pak").is_ok());
    let manifest: Manifest =
        serde_json::from_reader(zip.by_name("manifest.json").unwrap()).unwrap();
    assert_eq!(manifest.files.len(), 2);
    assert!(
        manifest
            .files
            .iter()
            .all(|entry| entry.original_name.as_deref() == Some("mod.pak"))
    );
}
