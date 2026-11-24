use std::collections::{BTreeMap, HashMap, HashSet};

use crate::providers::{ModInfo, ModSpecification, ModStore};
use crate::state::{ModData_v0_1_0 as Data, ModOrGroup};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Artifact {
    Modio(u32, u32),
    Local([u8; 32]),
    Specification(ModSpecification),
}

impl Artifact {
    pub(crate) fn cached(store: &ModStore, spec: &ModSpecification) -> Self {
        match store.cached_modio_file(spec) {
            Some((id, file)) => Self::Modio(id, file),
            None => store
                .cached_local_file(spec)
                .map(Self::Local)
                .unwrap_or_else(|| Self::Specification(spec.clone())),
        }
    }
}

/// Stream local bytes on a worker, never from the UI rendering path.
/// Re-read on each add/install; a filename or timestamp is not content identity.
pub(crate) fn local_digest(path: &std::path::Path) -> std::io::Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let path = std::fs::canonicalize(path)?;
    let mut file = std::fs::File::open(&path)?;
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(std::io::Error::other("Expected a regular file"));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let after = std::fs::metadata(&path)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        return Err(std::io::Error::other(
            "File changed while checking its contents; try again",
        ));
    }
    Ok(digest.finalize().into())
}

pub(crate) fn all_specs(data: &Data) -> Vec<ModSpecification> {
    let mut specs = HashSet::new();
    for profile in data.profiles.values() {
        for item in &profile.mods {
            if let ModOrGroup::Individual(mc) = item {
                specs.insert(mc.spec.clone());
            }
        }
    }
    for group in data.groups.values() {
        specs.extend(group.mods.iter().map(|mc| mc.spec.clone()));
    }
    specs.extend(data.bitfix_scripts.iter().map(|mc| mc.spec.clone()));
    specs.into_iter().collect()
}

/// Remove every explicit key from the resolver map, even aliases we skip.
/// Otherwise a skipped alias could be mistaken for an automatic dependency.
pub(crate) fn take_explicit(
    specs: Vec<ModSpecification>,
    resolved: &mut HashMap<ModSpecification, ModInfo>,
    store: &ModStore,
) -> Vec<(ModSpecification, ModInfo)> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for spec in specs {
        if let Some(info) = resolved.remove(&spec)
            && seen.insert(Artifact::cached(store, &spec))
        {
            entries.push((spec, info));
        }
    }
    entries
}

/// Re-adding an artifact replaces its old standalone rows at the top.
/// Keep the first row's priority/settings, but honour the newly requested URL/pin.
/// Group membership and genuinely different file versions remain independent.
pub(crate) fn promote_standalone(
    profile: &mut crate::state::ModProfile_v0_1_0,
    spec: &ModSpecification,
    store: &ModStore,
) -> bool {
    let identity = Artifact::cached(store, spec);
    let mut first = None;
    profile.mods.retain(|item| {
        if let ModOrGroup::Individual(mc) = item
            && (mc.spec == *spec || Artifact::cached(store, &mc.spec) == identity)
        {
            if first.is_none() {
                first = Some(mc.clone());
            }
            return false;
        }
        true
    });
    let Some(mut mc) = first else { return false };
    mc.spec = spec.clone();
    mc.enabled = true;
    profile.mods.insert(0, ModOrGroup::Individual(mc));
    true
}

/// Indexed equivalent of `ModSpecification::satisfies_dependency`.
/// Exact URLs also cover non-mod.io/local files and malformed mod.io URLs.
///
/// When both mod.io URLs have numeric IDs, their IDs must match. If either
/// side lacks an ID, their slugs must match. Keep these three sets distinct:
/// collapsing them into one normalized URL would change dependency semantics.
#[derive(Default)]
struct DependencyIndex {
    exact_urls: HashSet<String>,
    modio_slugs: HashSet<String>,
    unpinned_modio_slugs: HashSet<String>,
    modio_ids: HashSet<u32>,
}

impl DependencyIndex {
    fn from_specs<'a>(specs: impl IntoIterator<Item = &'a ModSpecification>) -> Self {
        let mut result = Self::default();
        for spec in specs {
            result.exact_urls.insert(spec.url.clone());
            if let Some((slug, id)) = spec.dependency_identity() {
                result.modio_slugs.insert(slug.to_owned());
                match id {
                    Some(id) => {
                        result.modio_ids.insert(id);
                    }
                    None => {
                        result.unpinned_modio_slugs.insert(slug.to_owned());
                    }
                }
            }
        }
        result
    }

    /// Identity-only lookup, used both for direct requirements and for
    /// configured replacement/alternative candidates.
    fn contains_identity(&self, required: &ModSpecification) -> bool {
        if self.exact_urls.contains(&required.url) {
            return true;
        }
        match required.dependency_identity() {
            Some((slug, Some(id))) => {
                self.modio_ids.contains(&id) || self.unpinned_modio_slugs.contains(slug)
            }
            Some((slug, None)) => self.modio_slugs.contains(slug),
            None => false,
        }
    }

    /// Preserve `ModSpecification::satisfies_dependency`: its matching includes
    /// ModHub replacement and alternative rules in the forward direction.
    fn contains(&self, required: &ModSpecification) -> bool {
        if self.contains_identity(required) {
            return true;
        }
        required.replacement_rule().is_some_and(|rule| {
            self.contains_identity(&ModSpecification::new(rule.replacement.clone()))
                || rule.alternatives.iter().any(|alternative| {
                    self.contains_identity(&ModSpecification::new(alternative.clone()))
                })
        })
    }
}

/// A group can be referenced multiple times in one profile with different
/// enable/disable overrides. Its *entry* ID, not its name, is the warning scope.
struct EnabledMember {
    group_entry_id: String, // Empty string = standalone (group IDs are nonempty).
    mod_id: String,
    spec: ModSpecification,
}

fn enabled_members(data: &Data, profile: &str) -> Vec<EnabledMember> {
    let mut result = Vec::new();
    let Some(profile) = data.profiles.get(profile) else {
        return result;
    };
    for item in &profile.mods {
        match item {
            ModOrGroup::Individual(mc) if mc.enabled => result.push(EnabledMember {
                group_entry_id: String::new(),
                mod_id: mc.id.clone(),
                spec: mc.spec.clone(),
            }),
            ModOrGroup::Group {
                id,
                group_name,
                enabled,
                member_overrides,
                ..
            } if *enabled => {
                if let Some(group) = data.groups.get(group_name) {
                    for mc in &group.mods {
                        if member_overrides.enabled(mc) {
                            result.push(EnabledMember {
                                group_entry_id: id.clone(),
                                mod_id: mc.id.clone(),
                                spec: mc.spec.clone(),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    result
}

#[derive(Default)]
pub(crate) struct ProfileIssues {
    pub(crate) versions: Vec<String>,
    pub(crate) pending: Vec<String>,
    duplicates: HashSet<(Option<String>, usize)>,
    missing_dependencies: HashMap<String, HashMap<String, Vec<ModSpecification>>>,
}

impl ProfileIssues {
    pub(crate) fn duplicate(&self, group: Option<&str>, index: usize) -> bool {
        self.duplicates.contains(&(group.map(str::to_owned), index))
    }

    /// Cached lookup with no allocations in the GUI repaint path.
    pub(crate) fn missing_dependencies(
        &self,
        group_entry_id: Option<&str>,
        mod_id: &str,
    ) -> &[ModSpecification] {
        self.missing_dependencies
            .get(group_entry_id.unwrap_or(""))
            .and_then(|members| members.get(mod_id))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn collect(data: &Data, profile: &str, store: &ModStore) -> Self {
        let mut result = Self::default();
        let mut versions = BTreeMap::<u32, BTreeMap<u32, Vec<String>>>::new();
        let mut names = HashMap::<u32, String>::new();
        let mut seen = HashMap::new();
        let mut visit =
            |spec: &ModSpecification, group: Option<&str>, index: usize, enabled: bool| {
                let artifact = Artifact::cached(store, spec);
                let location = format!(
                    "{} #{}{}",
                    group.unwrap_or("Standalone"),
                    index + 1,
                    if enabled { "" } else { " (disabled)" }
                );
                if let Artifact::Modio(id, file) = &artifact {
                    names.entry(*id).or_insert_with(|| {
                        store
                            .cached_modio_name(*id)
                            .or_else(|| {
                                spec.url
                                    .strip_prefix("https://mod.io/g/drg/m/")
                                    .and_then(|path| path.split(['#', '/']).next())
                                    .filter(|slug| !slug.is_empty())
                                    .map(|slug| slug.replace('-', " "))
                            })
                            .unwrap_or_else(|| "Unknown mod".to_owned())
                    });
                    versions
                        .entry(*id)
                        .or_default()
                        .entry(*file)
                        .or_default()
                        .push(location);
                } else if spec.url.starts_with("https://mod.io/g/drg/m/") {
                    result.pending.push(format!("{location}: {}", spec.url));
                }
                // Preserve the separate standalone check for enabled entries. Within a
                // group, a disabled duplicate is still a duplicate membership.
                if group.is_some() || enabled {
                    let scope = group.map(str::to_owned);
                    if let Some(first) = seen.insert((scope.clone(), artifact), index)
                        && first != index
                    {
                        result.duplicates.insert((scope.clone(), first));
                        result.duplicates.insert((scope, index));
                    }
                }
            };
        for (index, item) in data.profiles[profile].mods.iter().enumerate() {
            match item {
                ModOrGroup::Individual(mc) => visit(&mc.spec, None, index, mc.enabled),
                ModOrGroup::Group {
                    group_name,
                    enabled,
                    member_overrides,
                    ..
                } => {
                    if let Some(group) = data.groups.get(group_name) {
                        for (i, mc) in group.mods.iter().enumerate() {
                            visit(
                                &mc.spec,
                                Some(group_name),
                                i,
                                *enabled && member_overrides.enabled(mc),
                            );
                        }
                    }
                }
            }
        }
        for (id, files) in versions {
            if files.len() > 1 {
                let entries = files
                    .into_iter()
                    .map(|(file, places)| format!("file {file} in {}", places.join(", ")))
                    .collect::<Vec<_>>();
                result.versions.push(format!(
                    "{} (mod.io ID {id}) has multiple versions: {}",
                    names.get(&id).map(String::as_str).unwrap_or("Unknown mod"),
                    entries.join("; ")
                ));
            }
        }
        // One O(n + d) pass *only when the mod-data or provider-metadata
        // revision changes*. The GUI just reads cached warnings every frame.
        let members = enabled_members(data, profile);
        let enabled = DependencyIndex::from_specs(members.iter().map(|member| &member.spec));
        for member in members {
            let Some(info) = store.get_mod_info(&member.spec) else {
                continue;
            };
            let missing: Vec<ModSpecification> = info
                .suggested_dependencies
                .into_iter()
                .filter(|dep| !enabled.contains(dep))
                .collect();
            if !missing.is_empty() {
                result
                    .missing_dependencies
                    .entry(member.group_entry_id)
                    .or_default()
                    .insert(member.mod_id, missing);
            }
        }
        result
    }
}

#[cfg(test)]
mod dependency_cache_tests;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod large_profile_tests;
