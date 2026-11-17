use std::collections::BTreeSet;

use crate::providers::ModSpecification;
use crate::state::{ModData_v0_1_0, ModOrGroup};

#[derive(Clone, Debug)]
pub(super) struct Location {
    pub label: String,
    pub target: Option<Target>,
}

#[derive(Clone, Debug)]
pub(super) struct Target {
    profile_id: String,
    group_entry_id: Option<String>,
    mod_id: String,
}

#[derive(Clone, Debug)]
pub(super) struct ResolvedTarget {
    pub profile: String,
    pub group: Option<String>,
    pub group_entry_id: Option<String>,
    pub mod_id: String,
}

impl Target {
    // Resolve against current data: names and positions can change while details are open.
    pub fn resolve(&self, data: &ModData_v0_1_0) -> Option<ResolvedTarget> {
        let (name, profile) = data
            .profiles
            .iter()
            .find(|(_, p)| p.id == self.profile_id)?;
        for entry in &profile.mods {
            let group = match (entry, &self.group_entry_id) {
                (ModOrGroup::Individual(member), None) if member.id == self.mod_id => None,
                (ModOrGroup::Group { id, group_name, .. }, Some(wanted)) if id == wanted => {
                    let group = data.groups.get(group_name)?;
                    if !group.mods.iter().any(|m| m.id == self.mod_id) {
                        return None;
                    }
                    Some(group_name.clone())
                }
                _ => continue,
            };
            return Some(ResolvedTarget {
                profile: name.clone(),
                group,
                group_entry_id: self.group_entry_id.clone(),
                mod_id: self.mod_id.clone(),
            });
        }
        None
    }
}

/// Keep every saved occurrence, including disabled members and unused groups.
pub(super) fn entries(
    data: &ModData_v0_1_0,
    mut unavailable: impl FnMut(&ModSpecification) -> bool,
    mut name: impl FnMut(&ModSpecification) -> String,
) -> Vec<Location> {
    let mut locations = Vec::new();
    let mut used_groups = BTreeSet::new();
    for (profile_name, profile) in &data.profiles {
        for entry in &profile.mods {
            match entry {
                ModOrGroup::Individual(member) if unavailable(&member.spec) => {
                    locations.push(Location {
                        label: format!(
                            "{} — profile {:?}\n  {}",
                            name(&member.spec),
                            profile_name,
                            member.spec.url
                        ),
                        target: Some(Target {
                            profile_id: profile.id.clone(),
                            group_entry_id: None,
                            mod_id: member.id.clone(),
                        }),
                    });
                }
                ModOrGroup::Group { id, group_name, .. } => {
                    used_groups.insert(group_name.as_str());
                    if let Some(group) = data.groups.get(group_name) {
                        for member in &group.mods {
                            if unavailable(&member.spec) {
                                locations.push(Location {
                                    label: format!(
                                        "{} — profile {:?}, group {:?}\n  {}",
                                        name(&member.spec),
                                        profile_name,
                                        group_name,
                                        member.spec.url
                                    ),
                                    target: Some(Target {
                                        profile_id: profile.id.clone(),
                                        group_entry_id: Some(id.clone()),
                                        mod_id: member.id.clone(),
                                    }),
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for (group_name, group) in &data.groups {
        if !used_groups.contains(group_name.as_str()) {
            for member in &group.mods {
                if unavailable(&member.spec) {
                    locations.push(Location {
                        label: format!(
                            "{} — group {:?} (not used by any profile)\n  {}",
                            name(&member.spec),
                            group_name,
                            member.spec.url
                        ),
                        target: None,
                    });
                }
            }
        }
    }
    locations.sort_by(|a, b| a.label.cmp(&b.label));
    locations
}

pub(super) fn describe(locations: &[Location]) -> String {
    if locations.is_empty() {
        "Unavailable entries remain in the cache, but none are referenced by saved profiles or groups.".into()
    } else {
        format!(
            "Unavailable mods in saved profiles and groups:\n\n{}",
            locations
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        )
    }
}

#[cfg(test)]
pub(super) fn locations(
    data: &ModData_v0_1_0,
    unavailable: impl FnMut(&ModSpecification) -> bool,
    name: impl FnMut(&ModSpecification) -> String,
) -> String {
    describe(&entries(data, unavailable, name))
}

/// Cache the overview by saved-data and provider metadata revisions. No list
/// scans or provider lookups are needed on unchanged idle frames.
#[derive(Default)]
pub(super) struct Overview {
    revision: Option<(u64, (u64, u64))>,
    pub entries: Vec<Location>,
}

impl Overview {
    pub fn refresh(
        &mut self,
        revision: (u64, (u64, u64)),
        build: impl FnOnce() -> Vec<Location>,
    ) -> bool {
        if self.revision == Some(revision) {
            return false;
        }
        self.entries = build();
        self.revision = Some(revision);
        true
    }

    pub fn description(&self) -> String {
        if self.entries.is_empty() {
            "No unavailable mods are referenced by saved profiles or groups.".into()
        } else {
            describe(&self.entries)
        }
    }
}

/// Only enabled occurrences in the selected profile can block this install.
/// Check each group occurrence's overrides without mutating shared members.
pub(super) fn install_blockers(
    data: &ModData_v0_1_0,
    profile_name: &str,
    mut blocked: impl FnMut(&ModSpecification) -> bool,
    mut name: impl FnMut(&ModSpecification) -> String,
) -> Vec<Location> {
    let Some(profile) = data.profiles.get(profile_name) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut add = |member: &crate::state::ModConfig, group: Option<(&str, &str)>| {
        if !blocked(&member.spec) {
            return;
        }
        let group_label = group
            .map(|(_, name)| format!(", group {name:?}"))
            .unwrap_or_default();
        result.push(Location {
            label: format!(
                "{} — profile {:?}{}\n  {}",
                name(&member.spec),
                profile_name,
                group_label,
                member.spec.url
            ),
            target: Some(Target {
                profile_id: profile.id.clone(),
                group_entry_id: group.map(|(id, _)| id.to_owned()),
                mod_id: member.id.clone(),
            }),
        });
    };
    for entry in &profile.mods {
        match entry {
            ModOrGroup::Individual(member) if member.enabled => add(member, None),
            ModOrGroup::Group {
                id,
                group_name,
                enabled: true,
                member_overrides,
            } => {
                if let Some(group) = data.groups.get(group_name) {
                    for member in &group.mods {
                        if member_overrides.enabled(member) {
                            add(member, Some((id, group_name)));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    result
}
