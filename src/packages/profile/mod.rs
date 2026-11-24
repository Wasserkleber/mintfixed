//! Portable profile bundles. Keep the existing profile JSON format intact, and
//! use the shared package safety helpers for filenames, limits and hashing.
//! Extraction is staged and verified before any profile is inserted into state.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::packages::{self as shared, ArchiveKind, Manifest};
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

pub(crate) struct PreparedExport {
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
pub(crate) struct ExtractedBundle {
    pub(crate) json: String,
    // Owned new files only; never delete reused mods when an import is cancelled.
    files: Vec<PathBuf>,
    committed: bool,
}
impl ExtractedBundle {
    pub(crate) fn commit(&mut self) {
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
pub(crate) fn prepare_export(data: &Data, profile_name: &str) -> Result<PreparedExport, String> {
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
pub(crate) fn export_prepared(prepared: PreparedExport, path: &Path) -> Result<(), String> {
    shared::write_package(
        path,
        Manifest {
            format: FORMAT.into(),
            version: VERSION,
            name: prepared.name,
            definition: DEFINITION.into(),
            files: Vec::new(),
        },
        &prepared.text,
        prepared.sources,
        ArchiveKind::Profile,
    )
}

fn preview(path: &Path) -> Result<Preview, String> {
    let file = File::open(path).map_err(|e| format!("Cannot read profile ZIP: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Invalid profile ZIP: {e}"))?;
    let names = shared::validate_entries(&mut archive, ArchiveKind::Profile)?;
    let manifest_text = shared::bounded_text(
        &mut archive
            .by_name("manifest.json")
            .map_err(|_| "Missing manifest.json")?,
        shared::MAX_MANIFEST,
        ArchiveKind::Profile,
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
    let definition = shared::bounded_text(
        &mut archive
            .by_name(DEFINITION)
            .map_err(|_| "Missing profile.json")?,
        shared::MAX_DEFINITION,
        ArchiveKind::Profile,
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
pub(crate) fn inspect_bundle(path: &Path) -> Result<(String, String, bool), String> {
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
pub(crate) fn require_mod_folder(
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
                && crate::state::local_files::is_missing_local_path(url))
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
pub(crate) fn extract_bundle(
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
    let installed = shared::extract_mod_files(&mut archive, &parsed.manifest.files, folder)?;
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
#[path = "tests/profile_package_tests.rs"]
mod profile_package_tests;
