//! Portable group packages: one ZIP contains a JSON/TXT group definition and
//! copies of its local PAK/ZIP mods. Remote mod.io URLs are left untouched.
//! Archive names and sizes are validated; extraction never trusts archive paths.
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use super::group_document::{PendingGroupImport, parse_mod_list_txt, parse_single_group_json};
use crate::packages::{
    ArchiveKind, MAX_DEFINITION, MAX_FILES, MAX_MANIFEST, MAX_TOTAL, Manifest, PREFIX,
    bounded_text, extract_mod_files, local_archive, safe_archive_path, unique_bundle_filename,
    validate_entries,
};
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
pub(crate) fn export_bundle(
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
pub(crate) fn inspect_bundle(path: &Path) -> Result<PendingGroupImport, String> {
    let parsed = preview(path)?;
    Ok(PendingGroupImport {
        suggested_name: Some(parsed.manifest.name),
        group: parsed.group,
    })
}

/// Missing *external* local references in a pending group import.
/// A portable ZIP holds its `mintfixed-bundle:` entries inside the archive,
/// so those references do not require finding an original machine's file.
pub(crate) fn missing_external_group_paths(group: &ModGroup, portable_zip: bool) -> Vec<String> {
    crate::state::local_files::group_missing_paths(group)
        .into_iter()
        .filter(|path| !(portable_zip && path.starts_with(PREFIX)))
        .collect()
}

/// Require a valid Settings folder for missing external paths or bundled local files.
/// Remote-only bundles and existing local references need no extraction folder.
pub(crate) fn group_import_requires_mod_folder(
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
pub(crate) fn bundle_destination(configured: Option<&Path>) -> Option<PathBuf> {
    configured
        .filter(|folder| folder.is_dir())
        .map(Path::to_path_buf)
}

/// Extract validated bundle files directly into the Settings mod folder.
/// Existing files are never overwritten; matching SHA-256 files are reused.
pub(crate) fn extract_bundle(path: &Path, folder: &Path) -> Result<ModGroup, String> {
    let parsed = preview(path)?;
    let file = File::open(path).map_err(|e| format!("Cannot reopen ZIP: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Cannot reopen ZIP: {e}"))?;
    let mut installed = extract_mod_files(&mut archive, &parsed.manifest.files, folder)?;
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
#[path = "tests/group_package_tests.rs"]
mod group_package_tests;
