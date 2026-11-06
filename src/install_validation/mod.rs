//! Shared installation checks. Only enabled occurrences reach this module;
//! unknown local/HTTP versions are never inferred from names or asset paths.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use crate::integrate::IntegrationError;
use crate::providers::{ModInfo, ModSpecification, modio};
use crate::state::InstallEntry;

fn resolved_modio_file(info: &ModInfo) -> Result<Option<(u32, u32)>, IntegrationError> {
    if info.provider != "modio" {
        return Ok(None);
    }
    let (id, file) = modio::pinned_file(&info.resolution.url.0).ok_or_else(|| {
        IntegrationError::GenericError {
            msg: format!(
                "Missing resolved mod.io file ID for {:?} ({}). Installation stopped.",
                info.name, info.spec.url
            ),
        }
    })?;
    if info.modio_id.is_some_and(|metadata_id| metadata_id != id) {
        return Err(IntegrationError::GenericError {
            msg: format!(
                "Inconsistent mod.io identity for {:?}: metadata ID {:?}, resolved ID {id}. Installation stopped.",
                info.name, info.modio_id
            ),
        });
    }
    Ok(Some((id, file)))
}

/// Validate before deduplication so no conflicting occurrence can disappear.
fn validate_versions<'a>(
    entries: impl IntoIterator<Item = (&'a ModInfo, String)>,
) -> Result<(), IntegrationError> {
    let mut versions = BTreeMap::<u32, (&str, BTreeMap<u32, Vec<String>>)>::new();
    for (info, location) in entries {
        if let Some((id, file)) = resolved_modio_file(info)? {
            versions
                .entry(id)
                .or_insert_with(|| (&info.name, BTreeMap::new()))
                .1
                .entry(file)
                .or_default()
                .push(location);
        }
    }
    let conflicts: Vec<_> = versions
        .into_iter()
        .filter(|(_, (_, files))| files.len() > 1)
        .map(|(id, (name, files))| {
            let places = files
                .into_iter()
                .map(|(file, places)| format!("  File {file}: {}", places.join("; ")))
                .collect::<Vec<_>>()
                .join("\n");
            format!("{name:?} (mod.io ID {id}):\n{places}")
        })
        .collect();
    if conflicts.is_empty() {
        return Ok(());
    }
    Err(IntegrationError::GenericError {
        msg: format!(
            "Installation stopped: conflicting enabled mod versions.\n{}\nDisable one version of each listed mod before installing. No output archive was written.",
            conflicts.join("\n")
        ),
    })
}

/// GUI and CLI call this after resolution, before downloading. Inputs are already
/// in stable priority order; identical resolved artifacts keep their first entry.
pub(crate) fn installation_inputs(
    entries: &[InstallEntry],
    resolved: &HashMap<ModSpecification, ModInfo>,
) -> Result<Vec<ModInfo>, IntegrationError> {
    let resolved_entries = entries
        .iter()
        .map(|entry| {
            resolved
                .get(&entry.spec)
                .map(|info| (info, entry.location.clone()))
                .ok_or_else(|| IntegrationError::GenericError {
                    msg: format!(
                        "Missing resolved metadata for {} at {}",
                        entry.spec.url, entry.location
                    ),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_versions(
        resolved_entries
            .iter()
            .map(|(info, location)| (*info, location.clone())),
    )?;
    let mut files = HashSet::new();
    let mut other = HashSet::new();
    let mut result = Vec::new();
    for (info, _) in resolved_entries {
        if let Some(identity) = resolved_modio_file(info)? {
            if !files.insert(identity) {
                continue;
            }
        } else if !other.insert((info.provider, &info.resolution.url)) {
            continue;
        }
        result.push(info.clone());
    }
    Ok(result)
}

/// Defensive packer boundary, before opening the output or processing assets.
/// Check every version first, even when a preceding local input is unreadable.
pub(crate) fn prepare_packer_inputs(
    mods: Vec<(ModInfo, PathBuf)>,
) -> Result<Vec<(ModInfo, PathBuf)>, IntegrationError> {
    validate_versions(mods.iter().enumerate().map(|(index, (info, path))| {
        (
            info,
            format!("install input #{} ({})", index + 1, path.display()),
        )
    }))?;
    let mut modio_files = HashSet::new();
    let mut content = HashSet::new();
    let mut unique = Vec::with_capacity(mods.len());
    for (info, path) in mods {
        if let Some(identity) = resolved_modio_file(&info)? {
            if !modio_files.insert(identity) {
                continue;
            }
        } else if matches!(info.provider, "file" | "http") {
            // Compare complete downloaded bytes, not URL spelling. Distinct
            // mod.io identities stay distinct even when their bytes are alike.
            let digest = crate::mod_identity::local_digest(&path).map_err(|e| {
                IntegrationError::GenericError {
                    msg: format!("Cannot check mod {}: {e}", path.display()),
                }
            })?;
            if !content.insert(digest) {
                continue;
            }
        }
        unique.push((info, path));
    }
    Ok(unique)
}

#[cfg(test)]
mod tests;
