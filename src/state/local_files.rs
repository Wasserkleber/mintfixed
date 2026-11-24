//! Local mod path classification and missing-file diagnostics without UI dependencies.
use crate::state::{ModData_v0_1_0 as Data, ModGroup, ModOrGroup};
use std::{collections::BTreeSet, path::Path};

pub(crate) fn looks_like_local_path(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.contains("://") {
        return false;
    }
    let bytes = value.as_bytes();
    let windows_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let rooted = value.starts_with("\\\\")
        || value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with(".\\")
        || value.starts_with("..\\");
    let extension = Path::new(value).extension().and_then(|ext| ext.to_str());
    let mod_file = extension
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pak") || ext.eq_ignore_ascii_case("zip"));
    windows_absolute || rooted || mod_file
}

pub(crate) fn is_missing_local_path(value: &str) -> bool {
    looks_like_local_path(value) && !Path::new(value).exists()
}

pub(crate) fn group_missing_paths(group: &ModGroup) -> Vec<String> {
    group
        .mods
        .iter()
        .map(|member| member.spec.url.as_str())
        .filter(|path| is_missing_local_path(path))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn profile_missing_paths(data: &Data, profile_name: &str) -> Vec<String> {
    let Some(profile) = data.profiles.get(profile_name) else {
        return Vec::new();
    };
    let mut missing = BTreeSet::new();
    for entry in &profile.mods {
        match entry {
            ModOrGroup::Individual(member) => {
                if is_missing_local_path(&member.spec.url) {
                    missing.insert(member.spec.url.clone());
                }
            }
            ModOrGroup::Group { group_name, .. } => {
                if let Some(group) = data.groups.get(group_name) {
                    missing.extend(group_missing_paths(group));
                }
            }
        }
    }
    missing.into_iter().collect()
}
