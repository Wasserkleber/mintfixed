//! Profile JSON validation and import into persistent state.
use crate::state::{
    MemberOverrides, ModData_v0_1_0 as Data, ModGroup, ModOrGroup, ModProfile_v0_1_0 as Profile,
};
use std::collections::{BTreeMap, HashSet};

const PROFILE_FORMAT: &str = "mintfixed-profile-mod-list";
const PROFILE_VERSION: u32 = 1;
pub(crate) const MAX_PROFILE_JSON_BYTES: usize = 8 * 1024 * 1024;

#[derive(serde::Deserialize, serde::Serialize)]
pub(crate) struct ProfileExport {
    format: String,
    version: u32,
    pub(crate) profile_name: String,
    mods: Vec<ModOrGroup>,
    groups: BTreeMap<String, ModGroup>,
}

pub(crate) fn parse_profile(text: &str) -> Result<ProfileExport, String> {
    if text.len() > MAX_PROFILE_JSON_BYTES {
        return Err("Profile JSON must not exceed 8 MiB".into());
    }
    let export: ProfileExport =
        serde_json::from_str(text).map_err(|e| format!("Invalid profile JSON: {e}"))?;
    if export.format != PROFILE_FORMAT || export.version != PROFILE_VERSION {
        return Err("Unsupported profile JSON format/version".into());
    }
    if export.profile_name.trim().is_empty() {
        return Err("The exported profile has no name".into());
    }
    for (name, group) in &export.groups {
        if name.trim().is_empty() {
            return Err("An exported group has no name".into());
        }
        if group
            .mods
            .iter()
            .any(|member| member.spec.url.trim().is_empty())
        {
            return Err(format!("Group \"{name}\" contains an empty mod URL"));
        }
    }
    for entry in &export.mods {
        match entry {
            ModOrGroup::Individual(mc) if mc.spec.url.trim().is_empty() => {
                return Err("Profile contains an empty mod URL".into());
            }
            ModOrGroup::Group { group_name, .. } if !export.groups.contains_key(group_name) => {
                return Err(format!("Profile refers to missing group \"{group_name}\""));
            }
            _ => {}
        }
    }
    Ok(export)
}

fn unused_name(base: &str, reserved: &HashSet<String>) -> String {
    if !reserved.contains(base) {
        return base.to_owned();
    }
    let candidate = format!("{base} (imported)");
    if !reserved.contains(&candidate) {
        return candidate;
    }
    for suffix in 2.. {
        let candidate = format!("{base} (imported {suffix})");
        if !reserved.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// Prepare all data before modifying the saved state. Existing profiles and
/// shared groups are never overwritten. New IDs avoid cross-profile collisions.
pub(crate) fn import_profile(
    data: &mut Data,
    text: &str,
    requested_name: &str,
) -> Result<String, String> {
    let export = parse_profile(text)?;
    let requested_name = requested_name.trim();
    let base_name = if requested_name.is_empty() {
        export.profile_name.trim()
    } else {
        requested_name
    };
    // Match Group Management: require the user to resolve a profile-name conflict.
    // Existing profiles must never be overwritten or silently renamed.
    if data.profiles.contains_key(base_name) {
        return Err("A profile with that name already exists.".into());
    }
    let profile_name = base_name.to_owned();

    let mut used_groups: HashSet<String> = data.groups.keys().cloned().collect();
    let mut group_names = BTreeMap::new();
    let mut new_groups = BTreeMap::new();
    for (old_name, original) in &export.groups {
        let new_name = unused_name(old_name, &used_groups);
        used_groups.insert(new_name.clone());
        group_names.insert(old_name.clone(), new_name.clone());

        let mut copy = original.clone();
        copy.id = crate::state::new_id();
        copy.mods = original
            .mods
            .iter()
            .map(|mc| {
                let mut copy = mc.duplicate();
                copy.id = crate::state::new_id();
                copy.member_id = mc.member_id.as_ref().map(|_| crate::state::new_id());
                copy
            })
            .collect();
        new_groups.insert(new_name, copy);
    }

    let mut entries = Vec::with_capacity(export.mods.len());
    for entry in &export.mods {
        match entry {
            ModOrGroup::Individual(original) => {
                let mut copied = original.duplicate();
                copied.id = crate::state::new_id();
                copied.member_id = None;
                entries.push(ModOrGroup::Individual(copied));
            }
            ModOrGroup::Group {
                group_name,
                enabled,
                member_overrides,
                ..
            } => {
                let dest = &group_names[group_name]; // validated by parse_profile
                let old_group = &export.groups[group_name];
                let new_group = new_groups.get_mut(dest).expect("prepared group");
                let mut overrides = MemberOverrides::default();
                for (old, new) in old_group.mods.iter().zip(new_group.mods.iter_mut()) {
                    overrides.set(new, member_overrides.enabled(old));
                }
                entries.push(ModOrGroup::Group {
                    id: crate::state::new_id(),
                    group_name: dest.clone(),
                    enabled: *enabled,
                    member_overrides: overrides,
                });
            }
        }
    }

    let profile = Profile {
        id: crate::state::new_id(),
        mods: entries,
    };
    data.groups.extend(new_groups);
    data.profiles.insert(profile_name.clone(), profile);
    data.active_profile = profile_name.clone();
    Ok(profile_name)
}

#[cfg(test)]
mod tests;
