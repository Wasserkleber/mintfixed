//! Portable group packages: one ZIP contains a JSON/TXT group definition and
//! copies of its local PAK/ZIP mods. Remote mod.io URLs are left untouched.
//! Archive names and sizes are validated; extraction never trusts archive paths.
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use super::import_export::{parse_mod_list_txt, parse_single_group_json};
use super::management::PendingGroupImport;
use crate::state::ModGroup;

const FORMAT: &str = "mintfixed-group-package";
const VERSION: u32 = 1;
const PREFIX: &str = "mintfixed-bundle:";
const MAX_FILES: usize = 1000;
const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
const MAX_DEFINITION: u64 = 8 * 1024 * 1024;
const MAX_MANIFEST: u64 = 512 * 1024;

#[derive(Clone, Serialize, Deserialize)]
struct IncludedFile {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    name: String,
    definition: String,
    files: Vec<IncludedFile>,
}

struct Preview {
    manifest: Manifest,
    group: ModGroup,
}

fn local_archive(url: &str) -> bool {
    if url.starts_with("http://") || url.starts_with("https://") {
        return false;
    }
    let extension = Path::new(url).extension().and_then(|v| v.to_str());
    matches!(extension, Some(ext) if ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip"))
}

fn safe_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .take(90)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = sanitized.replace("..", "_");
    let trimmed = sanitized.trim_matches('.');
    if trimmed.is_empty() {
        "mod".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn bundle_filename(index: usize, url: &str) -> Result<String, String> {
    let original = url.rsplit(['/', '\\']).next().unwrap_or_default();
    let extension = Path::new(original)
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| format!("Invalid local mod filename: {original}"))?;
    if !["pak", "zip"]
        .iter()
        .any(|ext| extension.eq_ignore_ascii_case(ext))
    {
        return Err(format!(
            "Only local .pak/.zip files can be included: {original}"
        ));
    }
    let stem = original
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(original);
    Ok(format!(
        "mods/{index:04}_{}.{}",
        safe_component(stem),
        extension.to_ascii_lowercase()
    ))
}

fn safe_archive_path(path: &str) -> bool {
    if !path.starts_with("mods/") {
        return false;
    }
    let file = &path[5..];
    !file.is_empty()
        && file.len() <= 110
        && !file.contains("..")
        && file
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && matches!(Path::new(file).extension().and_then(|e| e.to_str()), Some(e) if e.eq_ignore_ascii_case("pak") || e.eq_ignore_ascii_case("zip"))
}

fn copy_and_hash<R: Read, W: Write>(
    input: &mut R,
    output: &mut W,
    limit: u64,
) -> Result<(u64, String), String> {
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|e| format!("Cannot read mod file: {e}"))?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or("File size overflow")?;
        if total > limit {
            return Err("Bundled mod exceeds the archive size limit".into());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| format!("Cannot write mod file: {e}"))?;
        hasher.update(&buffer[..count]);
    }
    Ok((total, format!("{:x}", hasher.finalize())))
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
    for (index, item) in modified.mods.iter_mut().enumerate() {
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
        let member = bundle_filename(index, &item.spec.url)?;
        item.spec.url = format!("{PREFIX}{member}");
        sources.push((source, member, metadata.len()));
    }

    let entry_name = if json { "group.json" } else { "group.txt" };
    let text = definition(name, &modified, json)?;
    if text.len() as u64 > MAX_DEFINITION {
        return Err("Group definition exceeds 8 MiB".into());
    }

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|e| format!("Cannot create package {}: {e}", destination.display()))?;
    let outcome = (|| -> Result<(), String> {
        let mut archive = ZipWriter::new(file);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        archive
            .start_file(entry_name, stored)
            .map_err(|e| e.to_string())?;
        archive
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
        let mut files = Vec::new();
        for (source, member, expected_size) in sources {
            archive
                .start_file(member.as_str(), stored)
                .map_err(|e| e.to_string())?;
            let mut input = File::open(&source)
                .map_err(|e| format!("Cannot open {}: {e}", source.display()))?;
            let (size, hash) = copy_and_hash(&mut input, &mut archive, MAX_TOTAL)?;
            if size != expected_size {
                return Err(format!(
                    "Local file changed during export: {}",
                    source.display()
                ));
            }
            files.push(IncludedFile {
                path: member,
                size,
                sha256: hash,
            });
        }
        let manifest = Manifest {
            format: FORMAT.into(),
            version: VERSION,
            name: name.into(),
            definition: entry_name.into(),
            files,
        };
        let manifest_text = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        if manifest_text.len() as u64 > MAX_MANIFEST {
            return Err("Too much package metadata".into());
        }
        archive
            .start_file("manifest.json", stored)
            .map_err(|e| e.to_string())?;
        archive
            .write_all(&manifest_text)
            .map_err(|e| e.to_string())?;
        archive
            .finish()
            .map_err(|e| format!("Cannot finalize ZIP: {e}"))?;
        Ok(())
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(destination);
    }
    outcome
}

fn bounded_text<R: Read>(source: &mut R, max: u64) -> Result<String, String> {
    let mut bytes = Vec::new();
    source
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err("Group metadata exceeds the allowed size".into());
    }
    String::from_utf8(bytes).map_err(|e| format!("Group metadata is not UTF-8: {e}"))
}

fn preview(path: &Path) -> Result<Preview, String> {
    let file = File::open(path).map_err(|e| format!("Cannot read package: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Invalid group ZIP: {e}"))?;
    if archive.len() > MAX_FILES + 2 {
        return Err("Too many ZIP entries".into());
    }
    let mut names = HashSet::new();
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().to_owned();
        if entry.is_dir()
            || name.contains('\\')
            || name.contains('\0')
            || name.starts_with('/')
            || name.contains("..")
            || name.contains(':')
        {
            return Err(format!("Unsafe path in group ZIP: {name}"));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("Symbolic links are not allowed in group packages".into());
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err("Duplicate ZIP entry names".into());
        }
        total = total.checked_add(entry.size()).ok_or("ZIP size overflow")?;
        if total > MAX_TOTAL + MAX_DEFINITION + MAX_MANIFEST {
            return Err("ZIP exceeds the 8 GiB limit".into());
        }
        if name.starts_with("mods/") && entry.compression() != CompressionMethod::Stored {
            return Err("Packaged mod files must use ZIP stored mode".into());
        }
    }
    let manifest_text = bounded_text(
        &mut archive
            .by_name("manifest.json")
            .map_err(|_| "Missing manifest.json")?,
        MAX_MANIFEST,
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

/// Extract into a new dedicated subfolder inside the chosen directory.
/// Never overwrite existing PAK files or follow ZIP paths outside the folder.
pub(super) fn extract_bundle(path: &Path, folder: &Path) -> Result<ModGroup, String> {
    let parsed = preview(path)?;
    if !folder.is_dir() {
        return Err(format!(
            "Extraction folder does not exist: {}",
            folder.display()
        ));
    }
    let base_name = safe_component(&parsed.manifest.name);
    let mut target = None;
    for index in 1..=10_000 {
        let name = if index == 1 {
            format!("{base_name} (mintfixed group)")
        } else {
            format!("{base_name} (mintfixed group {index})")
        };
        let candidate = folder.join(name);
        if !candidate.exists() {
            target = Some(candidate);
            break;
        }
    }
    let target = target.ok_or("Too many imported group folders")?;
    let staging = tempfile::Builder::new()
        .prefix(".mintfixed-group-")
        .tempdir_in(folder)
        .map_err(|e| format!("Cannot prepare extraction folder: {e}"))?;
    let file = File::open(path).map_err(|e| format!("Cannot reopen ZIP: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Cannot reopen ZIP: {e}"))?;
    let mut extracted = BTreeMap::new();
    for file in &parsed.manifest.files {
        let filename = file
            .path
            .strip_prefix("mods/")
            .ok_or("Invalid packaged mod path")?;
        let temporary = staging.path().join(filename);
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| format!("Cannot create local mod {}: {e}", temporary.display()))?;
        let mut source = archive
            .by_name(&file.path)
            .map_err(|e| format!("Cannot read {}: {e}", file.path))?;
        let (size, sha256) = copy_and_hash(&mut source, &mut output, file.size)?;
        if size != file.size || !sha256.eq_ignore_ascii_case(&file.sha256) {
            return Err(format!(
                "Bundled mod failed integrity verification: {}",
                file.path
            ));
        }
        output
            .flush()
            .map_err(|e| format!("Cannot flush imported mod: {e}"))?;
        extracted.insert(
            file.path.clone(),
            target.join(filename).to_string_lossy().into_owned(),
        );
    }
    let mut imported_group = parsed.group;
    for item in &mut imported_group.mods {
        if let Some(relative) = item.spec.url.strip_prefix(PREFIX) {
            item.spec.url = extracted
                .get(relative)
                .ok_or("Missing extracted mod")?
                .clone();
        }
    }
    fs::rename(staging.path(), &target).map_err(|e| format!("Cannot finish extraction: {e}"))?;
    Ok(imported_group)
}

#[cfg(test)]
mod group_package_tests {
    use super::*;
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
            "mods/0000_tool.pak"
        );
        assert_eq!(
            bundle_filename(1, r"D:\more\tool.pak").unwrap(),
            "mods/0001_tool.pak"
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
}
