//! Portable profile bundles. Keep the existing profile JSON format intact, and
//! reuse the group-package safety helpers for filenames, limits and hashing.
//! Extraction is staged and verified before any profile is inserted into state.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use super::group_controls::group_package as shared;
use crate::state::{ModData_v0_1_0 as Data, ModGroup, ModOrGroup};

const FORMAT: &str = "mintfixed-profile-package";
const VERSION: u32 = 1;
const DEFINITION: &str = "profile.json";

#[derive(Clone, Serialize, Deserialize)]
struct ProfileDocument {
    format: String,
    version: u32,
    profile_name: String,
    mods: Vec<ModOrGroup>,
    groups: BTreeMap<String, ModGroup>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    name: String,
    definition: String,
    files: Vec<shared::IncludedFile>,
}

pub(super) struct PreparedExport {
    text: Vec<u8>,
    name: String,
    sources: Vec<(PathBuf, String, u64)>,
}

struct Preview {
    manifest: Manifest,
    document: ProfileDocument,
}

/// Successfully extracted files are owned by the pending import until the
/// profile actually enters state. Dropping an uncommitted result cleans them up.
pub(super) struct ExtractedBundle {
    pub(super) json: String,
    // Owned new files only; never delete reused mods when an import is cancelled.
    files: Vec<PathBuf>,
    committed: bool,
}
impl ExtractedBundle {
    pub(super) fn commit(&mut self) {
        self.committed = true;
    }
}
impl Drop for ExtractedBundle {
    fn drop(&mut self) {
        if !self.committed {
            for path in self.files.iter().rev() {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn snapshot(data: &Data, profile_name: &str) -> Result<ProfileDocument, String> {
    let profile = data
        .profiles
        .get(profile_name)
        .ok_or("The active profile no longer exists")?;
    let mut groups = BTreeMap::new();
    for entry in &profile.mods {
        if let ModOrGroup::Group { group_name, .. } = entry {
            let group = data
                .groups
                .get(group_name)
                .ok_or_else(|| format!("Profile refers to a missing group: {group_name}"))?;
            groups.insert(group_name.clone(), group.clone());
        }
    }
    Ok(ProfileDocument {
        format: "mintfixed-profile-mod-list".into(),
        version: 1,
        profile_name: profile_name.to_owned(),
        mods: profile.mods.clone(),
        groups,
    })
}

fn valid_document(doc: &ProfileDocument) -> Result<(), String> {
    if doc.format != "mintfixed-profile-mod-list" || doc.version != 1 {
        return Err("Unsupported profile definition format/version".into());
    }
    if doc.profile_name.trim().is_empty() || doc.profile_name.len() > 512 {
        return Err("Invalid profile name in portable ZIP".into());
    }
    if doc.mods.len() > 100_000 || doc.groups.len() > 10_000 {
        return Err("Profile definition has too many entries".into());
    }
    for entry in &doc.mods {
        match entry {
            ModOrGroup::Individual(mod_config) if mod_config.spec.url.trim().is_empty() => {
                return Err("Profile contains an empty mod URL".into());
            }
            ModOrGroup::Group { group_name, .. } if !doc.groups.contains_key(group_name) => {
                return Err(format!("Profile refers to missing group: {group_name}"));
            }
            _ => {}
        }
    }
    for (name, group) in &doc.groups {
        if name.trim().is_empty() || group.mods.iter().any(|m| m.spec.url.trim().is_empty()) {
            return Err(format!("Invalid group definition: {name}"));
        }
    }
    Ok(())
}

fn visit_urls_mut(
    document: &mut ProfileDocument,
    mut visit: impl FnMut(&mut String) -> Result<(), String>,
) -> Result<(), String> {
    for entry in &mut document.mods {
        if let ModOrGroup::Individual(member) = entry {
            visit(&mut member.spec.url)?;
        }
    }
    for group in document.groups.values_mut() {
        for member in &mut group.mods {
            visit(&mut member.spec.url)?;
        }
    }
    Ok(())
}

fn urls(document: &ProfileDocument) -> Vec<&str> {
    let mut result = Vec::new();
    for entry in &document.mods {
        if let ModOrGroup::Individual(member) = entry {
            result.push(member.spec.url.as_str());
        }
    }
    for group in document.groups.values() {
        for member in &group.mods {
            result.push(member.spec.url.as_str());
        }
    }
    result
}

/// Snapshot the active profile before starting the worker thread. This includes
/// disabled mods and members, not just those enabled for installation.
pub(super) fn prepare_export(data: &Data, profile_name: &str) -> Result<PreparedExport, String> {
    let mut doc = snapshot(data, profile_name)?;
    valid_document(&doc)?;
    let mut sources: Vec<(PathBuf, String, u64)> = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut used_members = HashSet::new();
    let mut declared_total = 0_u64;
    visit_urls_mut(&mut doc, |url| {
        if !shared::local_archive(url) {
            return Ok(());
        }
        if let Some(member) = seen.get(url) {
            *url = format!("{}{}", shared::PREFIX, member);
            return Ok(());
        }
        if sources.len() >= shared::MAX_FILES {
            return Err("Too many local files in this profile (limit 1000)".into());
        }
        let source = PathBuf::from(url.as_str());
        let meta = fs::metadata(&source)
            .map_err(|e| format!("Cannot package local mod {}: {e}", source.display()))?;
        if !meta.is_file() {
            return Err(format!("Not a regular mod file: {}", source.display()));
        }
        declared_total = declared_total
            .checked_add(meta.len())
            .ok_or("Profile ZIP size overflow")?;
        if declared_total > shared::MAX_TOTAL {
            return Err("Profile bundled mods exceed the 8 GiB limit".into());
        }
        let member = shared::unique_bundle_filename(url, &mut used_members)?;
        let original = url.clone();
        *url = format!("{}{}", shared::PREFIX, member);
        seen.insert(original, member.clone());
        sources.push((source, member, meta.len()));
        Ok(())
    })?;
    let text = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
    if text.len() as u64 > shared::MAX_DEFINITION {
        return Err("Profile JSON exceeds the 8 MiB limit".into());
    }
    Ok(PreparedExport {
        text,
        name: doc.profile_name,
        sources,
    })
}

/// Called off the GUI thread. Never overwrite a previous ZIP; partial exports
/// are removed. Each local file is hashed as it is copied into the archive.
pub(super) fn export_prepared(prepared: PreparedExport, path: &Path) -> Result<(), String> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Cannot create portable profile ZIP {}: {e}", path.display()))?;
    let outcome = (|| -> Result<(), String> {
        let mut archive = ZipWriter::new(file);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        archive
            .start_file(DEFINITION, stored)
            .map_err(|e| e.to_string())?;
        archive
            .write_all(&prepared.text)
            .map_err(|e| e.to_string())?;
        let mut files = Vec::new();
        for (source, member, expected) in prepared.sources {
            archive
                .start_file(member.as_str(), stored)
                .map_err(|e| e.to_string())?;
            let mut input = File::open(&source)
                .map_err(|e| format!("Cannot open {}: {e}", source.display()))?;
            let (size, checksum) =
                shared::copy_and_hash(&mut input, &mut archive, shared::MAX_TOTAL)?;
            if size != expected {
                return Err(format!("File changed during export: {}", source.display()));
            }
            files.push(shared::IncludedFile {
                path: member,
                size,
                sha256: checksum,
                original_name: Some(
                    source
                        .file_name()
                        .ok_or("Invalid original filename")?
                        .to_string_lossy()
                        .into_owned(),
                ),
            });
        }
        let manifest = Manifest {
            format: FORMAT.into(),
            version: VERSION,
            name: prepared.name,
            definition: DEFINITION.into(),
            files,
        };
        let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        if json.len() as u64 > shared::MAX_MANIFEST {
            return Err("Profile package manifest exceeds 512 KiB".into());
        }
        archive
            .start_file("manifest.json", stored)
            .map_err(|e| e.to_string())?;
        archive.write_all(&json).map_err(|e| e.to_string())?;
        archive
            .finish()
            .map_err(|e| format!("Cannot finish profile ZIP: {e}"))?;
        Ok(())
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(path);
    }
    outcome
}

fn bounded_text(reader: &mut impl Read, max: u64) -> Result<String, String> {
    let mut bytes = Vec::new();
    reader
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err("Oversized profile ZIP metadata".into());
    }
    String::from_utf8(bytes).map_err(|e| format!("Invalid UTF-8 in profile ZIP: {e}"))
}

fn preview(path: &Path) -> Result<Preview, String> {
    let file = File::open(path).map_err(|e| format!("Cannot read profile ZIP: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Invalid profile ZIP: {e}"))?;
    if archive.len() > shared::MAX_FILES + 2 {
        return Err("Too many files in profile ZIP".into());
    }
    let mut names = HashSet::new();
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name();
        if entry.is_dir()
            || name.contains('\\')
            || name.contains('\0')
            || name.starts_with('/')
            || name.contains("..")
            || name.contains(':')
        {
            return Err(format!("Unsafe entry in profile ZIP: {name}"));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("Symlinks are not allowed in portable profile ZIPs".into());
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err("Duplicate filenames in profile ZIP".into());
        }
        total = total
            .checked_add(entry.size())
            .ok_or("Profile ZIP size overflow")?;
        if total > shared::MAX_TOTAL + shared::MAX_DEFINITION + shared::MAX_MANIFEST {
            return Err("Profile ZIP exceeds 8 GiB limit".into());
        }
        if name.starts_with("mods/") && entry.compression() != CompressionMethod::Stored {
            return Err("Bundled mod files must use ZIP stored mode".into());
        }
    }
    let manifest_text = bounded_text(
        &mut archive
            .by_name("manifest.json")
            .map_err(|_| "Missing manifest.json")?,
        shared::MAX_MANIFEST,
    )?;
    let manifest: Manifest = serde_json::from_str(&manifest_text)
        .map_err(|e| format!("Invalid profile manifest: {e}"))?;
    if manifest.format != FORMAT || manifest.version != VERSION || manifest.definition != DEFINITION
    {
        return Err("Unsupported portable profile ZIP format/version".into());
    }
    if manifest.files.len() > shared::MAX_FILES || archive.len() != manifest.files.len() + 2 {
        return Err("Unexpected/unlisted files in profile ZIP".into());
    }
    let mut listed = HashSet::new();
    for item in &manifest.files {
        if !shared::safe_archive_path(&item.path) || !listed.insert(item.path.to_ascii_lowercase())
        {
            return Err(format!(
                "Invalid or duplicate bundled filename: {}",
                item.path
            ));
        }
        if item.sha256.len() != 64 || !item.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("Invalid checksum: {}", item.path));
        }
        let entry = archive
            .by_name(&item.path)
            .map_err(|_| format!("Missing bundled file: {}", item.path))?;
        if entry.size() != item.size {
            return Err(format!("Incorrect bundled file size: {}", item.path));
        }
    }
    for name in &names {
        if name != "manifest.json" && name != DEFINITION && !listed.contains(name) {
            return Err(format!("Unlisted ZIP entry: {name}"));
        }
    }
    let definition = bounded_text(
        &mut archive
            .by_name(DEFINITION)
            .map_err(|_| "Missing profile.json")?,
        shared::MAX_DEFINITION,
    )?;
    let doc: ProfileDocument = serde_json::from_str(&definition)
        .map_err(|e| format!("Invalid packaged profile definition: {e}"))?;
    valid_document(&doc)?;
    if doc.profile_name != manifest.name {
        return Err("Profile name does not match manifest".into());
    }
    let mut referenced = HashSet::new();
    for url in urls(&doc) {
        if let Some(member) = url.strip_prefix(shared::PREFIX) {
            if !listed.contains(&member.to_ascii_lowercase()) || !shared::safe_archive_path(member)
            {
                return Err(format!("Invalid bundled mod reference: {member}"));
            }
            referenced.insert(member.to_ascii_lowercase());
        }
    }
    if referenced != listed {
        return Err("Profile doesn't reference all packaged mod files".into());
    }
    Ok(Preview {
        manifest,
        document: doc,
    })
}

/// Read a ZIP preview without extracting any mod files.
pub(super) fn inspect_bundle(path: &Path) -> Result<(String, String, bool), String> {
    let parsed = preview(path)?;
    let text = serde_json::to_string_pretty(&parsed.document).map_err(|e| e.to_string())?;
    Ok((
        parsed.document.profile_name,
        text,
        !parsed.manifest.files.is_empty(),
    ))
}

/// Both packages and legacy JSON must require Settings when they refer to
/// missing external local paths. A self-contained bundle still needs a folder
/// to receive its embedded PAKs. Remote-only profiles need neither.
pub(super) fn require_mod_folder(
    json: &str,
    portable: bool,
    configured: Option<&Path>,
) -> Result<(), String> {
    let doc: ProfileDocument =
        serde_json::from_str(json).map_err(|e| format!("Invalid profile JSON: {e}"))?;
    valid_document(&doc)?;
    let need_folder = urls(&doc).into_iter().any(|url| {
        (portable && url.starts_with(shared::PREFIX))
            || (!url.starts_with(shared::PREFIX)
                && crate::gui::missing_local_files::is_missing_local_path(url))
            || (!portable && url.starts_with(shared::PREFIX))
    });
    if need_folder && !configured.is_some_and(Path::is_dir) {
        return Err(
            "Cannot import this profile: configure a valid local mods folder in Settings first."
                .into(),
        );
    }
    Ok(())
}

/// Runs on a worker thread. This checks every file hash in a temporary folder
/// before moving the folder into place. The result owns cleanup until commit.
/// Verify the ZIP and reuse matching local mods. Only genuinely new files
/// are staged, with the new folder owned by the pending import for rollback.
/// Validates the portable ZIP and imports new PAK/ZIP files directly into
/// Settings' mod folder. The pending profile owns cleanup until committed.
pub(super) fn extract_bundle(
    path: &Path,
    configured: Option<&Path>,
) -> Result<ExtractedBundle, String> {
    let parsed = preview(path)?;
    if parsed.manifest.files.is_empty() {
        return Ok(ExtractedBundle {
            json: serde_json::to_string(&parsed.document).map_err(|e| e.to_string())?,
            files: Vec::new(),
            committed: false,
        });
    }
    let folder = configured.filter(|path| path.is_dir()).ok_or(
        "Cannot import this profile: configure a valid local mods folder in Settings first.",
    )?;
    let file = File::open(path).map_err(|e| format!("Cannot reopen profile ZIP: {e}"))?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| format!("Cannot reopen profile ZIP: {e}"))?;
    let installed = shared::extract_mod_files(
        &mut archive,
        &parsed.manifest.files,
        folder,
        &parsed.manifest.name,
        "profile",
    )?;
    let mut document = parsed.document;
    visit_urls_mut(&mut document, |url| {
        if let Some(member) = url.strip_prefix(shared::PREFIX) {
            *url = installed
                .mapping
                .get(member)
                .ok_or("Missing extracted mod")?
                .clone();
        }
        Ok(())
    })?;
    let json = serde_json::to_string(&document).map_err(|e| e.to_string())?;
    let files = installed.into_created();
    Ok(ExtractedBundle {
        json,
        files,
        committed: false,
    })
}

#[cfg(test)]
mod profile_package_tests {
    use super::*;
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
}
