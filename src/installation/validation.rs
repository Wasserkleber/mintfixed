//! Shared installation checks. Only enabled occurrences reach this module;
//! unknown local/HTTP versions are never inferred from names or asset paths.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use super::report::SkippedMod;
use crate::integrate::IntegrationError;
use crate::providers::{ModInfo, ModSpecification, modio};
use crate::state::InstallEntry;

pub(crate) struct InstallSelection {
    pub mods: Vec<ModInfo>,
    pub locations: Vec<String>,
    pub skipped: Vec<SkippedMod>,
}

/// Report missing suggested dependencies without logging or blocking installation.
/// Match both the requested and resolved URLs, as the callers did before.
pub(crate) fn missing_dependencies<'a>(
    mods: impl IntoIterator<Item = &'a ModInfo>,
) -> Vec<&'a str> {
    let mods: Vec<_> = mods.into_iter().collect();
    let present: HashSet<_> = mods
        .iter()
        .flat_map(|info| [info.spec.url.as_str(), info.resolution.url.0.as_str()])
        .collect();
    let missing: HashSet<_> = mods
        .iter()
        .flat_map(|info| info.suggested_dependencies.iter())
        .map(|spec| spec.url.as_str())
        .filter(|url| !present.contains(url))
        .collect();
    let mut missing: Vec<_> = missing.into_iter().collect();
    missing.sort_unstable();
    missing
}

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
#[cfg(test)]
pub(crate) fn installation_inputs(
    entries: &[InstallEntry],
    resolved: &HashMap<ModSpecification, ModInfo>,
) -> Result<Vec<ModInfo>, IntegrationError> {
    Ok(select_installation_inputs(entries, resolved)?.mods)
}

pub(crate) fn select_installation_inputs(
    entries: &[InstallEntry],
    resolved: &HashMap<ModSpecification, ModInfo>,
) -> Result<InstallSelection, IntegrationError> {
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
    let mut result = InstallSelection {
        mods: Vec::new(),
        locations: Vec::new(),
        skipped: Vec::new(),
    };
    for (info, location) in resolved_entries {
        let duplicate = if let Some(identity) = resolved_modio_file(info)? {
            !files.insert(identity)
        } else {
            !other.insert((info.provider, &info.resolution.url))
        };
        if duplicate {
            result.skipped.push(SkippedMod {
                name: info.name.clone(),
                location,
                reason: "same resolved mod file",
            });
        } else {
            result.mods.push(info.clone());
            result.locations.push(location);
        }
    }
    Ok(result)
}

/// Defensive packer boundary, before opening the output or processing assets.
/// Check every version first, even when a preceding local input is unreadable.
#[cfg(test)]
pub(crate) fn prepare_packer_inputs(
    mods: Vec<(ModInfo, PathBuf)>,
) -> Result<Vec<(ModInfo, PathBuf)>, IntegrationError> {
    prepare_packer_inputs_with_report(mods, &[], &mut Vec::new())
}

pub(crate) fn prepare_packer_inputs_with_report(
    mods: Vec<(ModInfo, PathBuf)>,
    locations: &[String],
    skipped: &mut Vec<SkippedMod>,
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
    for (index, (info, path)) in mods.into_iter().enumerate() {
        let location = locations
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("install input #{} ({})", index + 1, path.display()));
        if let Some(identity) = resolved_modio_file(&info)? {
            if !modio_files.insert(identity) {
                skipped.push(SkippedMod {
                    name: info.name.clone(),
                    location,
                    reason: "same resolved mod file",
                });
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
                skipped.push(SkippedMod {
                    name: info.name.clone(),
                    location,
                    reason: "identical local/HTTP file contents",
                });
                continue;
            }
        }
        unique.push((info, path));
    }
    Ok(unique)
}

#[cfg(test)]
#[path = "tests/validation.rs"]
mod tests;
