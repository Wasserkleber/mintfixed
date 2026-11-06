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
        if let Some(info) = resolved.remove(&spec) {
            if seen.insert(Artifact::cached(store, &spec)) {
                entries.push((spec, info));
            }
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
        if let ModOrGroup::Individual(mc) = item {
            if mc.spec == *spec || Artifact::cached(store, &mc.spec) == identity {
                if first.is_none() {
                    first = Some(mc.clone());
                }
                return false;
            }
        }
        true
    });
    let Some(mut mc) = first else { return false };
    mc.spec = spec.clone();
    mc.enabled = true;
    profile.mods.insert(0, ModOrGroup::Individual(mc));
    true
}

#[derive(Default)]
pub(crate) struct ProfileIssues {
    pub(crate) versions: Vec<String>,
    pub(crate) pending: Vec<String>,
    duplicates: HashSet<(Option<String>, usize)>,
}

impl ProfileIssues {
    pub(crate) fn duplicate(&self, group: Option<&str>, index: usize) -> bool {
        self.duplicates.contains(&(group.map(str::to_owned), index))
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
                    if let Some(first) = seen.insert((scope.clone(), artifact), index) {
                        if first != index {
                            result.duplicates.insert((scope.clone(), first));
                            result.duplicates.insert((scope, index));
                        }
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
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install_validation::{installation_inputs, prepare_packer_inputs};
    use crate::providers::ModResolution;
    use crate::state::InstallEntry;
    use crate::state::{MemberOverrides, ModConfig, ModGroup, ModProfile_v0_1_0};

    fn spec(url: &str) -> ModSpecification {
        ModSpecification {
            url: url.to_owned(),
        }
    }

    fn member(url: &str) -> ModConfig {
        ModConfig {
            id: crate::state::new_id(),
            description: None,
            spec: spec(url),
            required: false,
            member_id: None,
            enabled: true,
            priority: 0,
        }
    }

    fn info(url: &str) -> ModInfo {
        ModInfo {
            provider: "modio",
            name: url.to_owned(),
            spec: spec(url),
            versions: vec![],
            resolution: ModResolution::resolvable(url.to_owned().into()),
            suggested_require: false,
            suggested_dependencies: vec![],
            modio_tags: None,
            modio_id: None,
        }
    }

    #[test]
    fn pins_identify_aliases_without_network_or_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
        assert_eq!(
            Artifact::cached(&store, &spec("https://mod.io/g/drg/m/old#123/10")),
            Artifact::cached(&store, &spec("https://mod.io/g/drg/m/new#123/10"))
        );
        assert_ne!(
            Artifact::cached(&store, &spec("https://mod.io/g/drg/m/old#123/10")),
            Artifact::cached(&store, &spec("https://mod.io/g/drg/m/old#123/11"))
        );
    }

    #[test]
    fn disabled_groups_warn_but_do_not_block_and_shared_members_are_not_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
        let a = "https://mod.io/g/drg/m/a#123/10";
        let b = "https://mod.io/g/drg/m/a#123/11";
        let data = Data {
            active_profile: "p".into(),
            profiles: BTreeMap::from([(
                "p".into(),
                ModProfile_v0_1_0 {
                    id: crate::state::new_id(),
                    mods: vec![
                        ModOrGroup::Individual(member(a)),
                        ModOrGroup::Group {
                            id: crate::state::new_id(),
                            group_name: "same".into(),
                            enabled: true,
                            member_overrides: MemberOverrides::default(),
                        },
                        ModOrGroup::Group {
                            id: crate::state::new_id(),
                            group_name: "other".into(),
                            enabled: false,
                            member_overrides: MemberOverrides::default(),
                        },
                    ],
                },
            )]),
            groups: BTreeMap::from([
                (
                    "same".into(),
                    ModGroup {
                        id: crate::state::new_id(),
                        description: None,
                        mods: vec![member(a)],
                    },
                ),
                (
                    "other".into(),
                    ModGroup {
                        id: crate::state::new_id(),
                        description: None,
                        mods: vec![member(b), member(b)],
                    },
                ),
            ]),
        };
        let issues = ProfileIssues::collect(&data, "p", &store);
        assert_eq!(issues.versions.len(), 1);
        assert!(issues.versions[0].starts_with("a (mod.io ID 123)"));
        assert!(issues.versions[0].contains("disabled"));
        assert!(!issues.duplicate(None, 0));
        assert!(!issues.duplicate(Some("same"), 0));
        assert!(issues.duplicate(Some("other"), 0));
        assert!(issues.duplicate(Some("other"), 1));
        let inputs = data.ordered_install_specs("p");
        let resolved = HashMap::from([(spec(a), info(a))]);
        assert_eq!(
            installation_inputs(&InstallEntry::requested(&inputs), &resolved)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn installer_coalesces_aliases_keeps_order_and_rejects_version_conflicts() {
        let a = spec("alias-a");
        let b = spec("alias-b");
        let c = spec("another-mod");
        let mut resolved = HashMap::from([
            (a.clone(), info("https://mod.io/g/drg/m/a#123/10")),
            (b.clone(), info("https://mod.io/g/drg/m/renamed#123/10")),
            (c.clone(), info("https://mod.io/g/drg/m/c#456/20")),
        ]);
        let inputs = vec![c.clone(), a.clone(), b.clone()];
        let selected = installation_inputs(&InstallEntry::requested(&inputs), &resolved).unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].name, resolved[&c].name);
        assert_eq!(selected[1].name, resolved[&a].name);
        resolved.insert(b, info("https://mod.io/g/drg/m/a#123/11"));
        assert!(installation_inputs(&InstallEntry::requested(&inputs), &resolved).is_err());
        assert!(
            installation_inputs(&InstallEntry::requested(&[spec("missing")]), &resolved).is_err()
        );
    }

    #[test]
    fn unknown_versions_are_pending_not_guessed_from_similar_names() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
        let data = Data {
            active_profile: "p".into(),
            groups: BTreeMap::new(),
            profiles: BTreeMap::from([(
                "p".into(),
                ModProfile_v0_1_0 {
                    id: crate::state::new_id(),
                    mods: vec![
                        ModOrGroup::Individual(member("https://mod.io/g/drg/m/example")),
                        ModOrGroup::Individual(member("https://mod.io/g/drg/m/example-2")),
                    ],
                },
            )]),
        };
        let issues = ProfileIssues::collect(&data, "p", &store);
        assert!(issues.versions.is_empty());
        assert!(issues.duplicates.is_empty());
        assert_eq!(issues.pending.len(), 2);
    }
    #[test]
    fn readding_latest_or_its_pin_promotes_disabled_standalone_but_other_versions_remain_distinct()
    {
        let dir = tempfile::tempdir().unwrap();
        // Use the current versioned format so numeric mod IDs are decoded via
        // the cache reader's numeric-key compatibility path.
        let cache = serde_json::json!({"version": "0.0.0", "cache": {"modio": {
            "type": "ModioCache", "mod_id_map": {"speedometer": 123},
            "modfile_blobs": {}, "dependencies": {}, "last_update_time": null,
            "mods": {"123": {"name_id": "speedometer", "name": "Speedometer",
                "latest_modfile": 10, "modfiles": [], "tags": []}}
        }}});
        std::fs::write(
            dir.path().join("cache.json"),
            serde_json::to_vec(&cache).unwrap(),
        )
        .unwrap();
        let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
        let latest = "https://mod.io/g/drg/m/speedometer";
        let pin = "https://mod.io/g/drg/m/speedometer#123/10";
        // An old or renamed URL slug must not override the real cached title.
        // The fixture deliberately has no cached dependency metadata.
        let issues_data = Data {
            active_profile: "p".into(),
            profiles: BTreeMap::from([(
                "p".into(),
                ModProfile_v0_1_0 {
                    id: crate::state::new_id(),
                    mods: vec![
                        ModOrGroup::Individual(member("https://mod.io/g/drg/m/old-slug#123/10")),
                        ModOrGroup::Individual(member("https://mod.io/g/drg/m/old-slug#123/11")),
                    ],
                },
            )]),
            groups: BTreeMap::new(),
        };
        let issues = ProfileIssues::collect(&issues_data, "p", &store);
        assert_eq!(issues.versions.len(), 1);
        assert!(
            issues.versions[0].starts_with("Speedometer (mod.io ID 123) has multiple versions:")
        );
        let mut existing = member(latest);
        existing.enabled = false;
        let mut profile = ModProfile_v0_1_0 {
            id: crate::state::new_id(),
            mods: vec![ModOrGroup::Individual(existing)],
        };
        assert!(promote_standalone(&mut profile, &spec(pin), &store));
        assert!(promote_standalone(&mut profile, &spec(latest), &store));
        assert_eq!(profile.mods.len(), 1);
        let ModOrGroup::Individual(existing) = &profile.mods[0] else {
            panic!()
        };
        assert!(existing.enabled);
        assert_eq!(existing.spec.url, latest);
        assert!(!promote_standalone(
            &mut profile,
            &spec("https://mod.io/g/drg/m/speedometer#123/11"),
            &store
        ));

        let mut resolved = HashMap::from([(spec(latest), info(pin)), (spec(pin), info(pin))]);
        let explicit = take_explicit(
            vec![spec(latest), spec(pin), spec(latest)],
            &mut resolved,
            &store,
        );
        assert_eq!(explicit.len(), 1);
        assert_eq!(explicit[0].0, spec(latest));
        assert!(
            resolved.is_empty(),
            "Skipped aliases must not return as dependencies"
        );
    }

    #[test]
    fn renamed_local_copies_match_and_edits_invalidate_identity() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
        let a = dir.path().join("original.pak");
        let b = dir.path().join("renamed.pak");
        std::fs::write(&a, b"pak content one").unwrap();
        std::fs::copy(&a, &b).unwrap();
        let a = spec(a.to_str().unwrap());
        let b = spec(b.to_str().unwrap());
        store.refresh_local_files(&[a.clone(), b.clone()]);
        let revision = store.metadata_revision();
        assert_eq!(Artifact::cached(&store, &a), Artifact::cached(&store, &b));
        let mut profile = ModProfile_v0_1_0 {
            id: crate::state::new_id(),
            mods: vec![ModOrGroup::Individual(member(&a.url))],
        };
        assert!(promote_standalone(&mut profile, &b, &store));
        // Same length, different bytes: no filename/length-based assumption.
        std::fs::write(&b.url, b"pak content two").unwrap();
        store.refresh_local_files(&[a.clone(), b.clone()]);
        assert_ne!(revision, store.metadata_revision());
        assert_ne!(Artifact::cached(&store, &a), Artifact::cached(&store, &b));
        let mut profile = ModProfile_v0_1_0 {
            id: crate::state::new_id(),
            mods: vec![ModOrGroup::Individual(member(&a.url))],
        };
        assert!(!promote_standalone(&mut profile, &b, &store));
        std::fs::remove_file(&b.url).unwrap();
        store.refresh_local_files(&[b.clone()]);
        assert!(store.cached_local_file(&b).is_none());
    }

    #[test]
    fn local_packer_dedup_keeps_first_priority_and_checks_files_again() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.pak");
        let b = dir.path().join("b.pak");
        let c = dir.path().join("c.pak");
        std::fs::write(&a, b"same special asset bytes").unwrap();
        std::fs::copy(&a, &b).unwrap();
        std::fs::write(&c, b"different content").unwrap();
        let local = |name: &str| {
            let mut value = info(name);
            value.provider = "file";
            value
        };
        let inputs = vec![
            (local("first"), b.clone()),
            (local("copy"), a),
            (local("last"), c),
        ];
        let unique = prepare_packer_inputs(inputs).unwrap();
        assert_eq!(unique.len(), 2);
        assert_eq!(unique[0].0.name, "first");
        assert_eq!(unique[1].0.name, "last");
        std::fs::remove_file(&b).unwrap();
        assert!(prepare_packer_inputs(unique).is_err());
    }

    #[test]
    fn readding_moves_to_top_removes_old_copies_and_preserves_other_versions_and_groups() {
        let dir = tempfile::tempdir().unwrap();
        let store = ModStore::new(dir.path(), &HashMap::new()).unwrap();
        let pin = "https://mod.io/g/drg/m/a#123/10";
        let alias = "https://mod.io/g/drg/m/renamed#123/10";
        let different = "https://mod.io/g/drg/m/a#123/11";
        let mut old = member(pin);
        old.enabled = false;
        old.priority = 42;
        old.required = true;
        let mut profile = ModProfile_v0_1_0 {
            id: crate::state::new_id(),
            mods: vec![
                ModOrGroup::Individual(member("unrelated")),
                ModOrGroup::Individual(old),
                ModOrGroup::Group {
                    id: crate::state::new_id(),
                    group_name: "Tools".into(),
                    enabled: true,
                    member_overrides: Default::default(),
                },
                ModOrGroup::Individual(member(pin)),
                ModOrGroup::Individual(member(different)),
            ],
        };
        assert!(promote_standalone(&mut profile, &spec(alias), &store));
        assert_eq!(profile.mods.len(), 4);
        let ModOrGroup::Individual(top) = &profile.mods[0] else {
            panic!()
        };
        assert_eq!(top.spec, spec(alias));
        assert!(top.enabled && top.required);
        assert_eq!(top.priority, 42);
        assert!(
            matches!(&profile.mods[1], ModOrGroup::Individual(mc) if mc.spec.url == "unrelated")
        );
        assert!(
            matches!(&profile.mods[2], ModOrGroup::Group { group_name, .. } if group_name == "Tools")
        );
        assert!(matches!(&profile.mods[3], ModOrGroup::Individual(mc) if mc.spec.url == different));
        assert!(promote_standalone(&mut profile, &spec(alias), &store));
        assert_eq!(profile.mods.len(), 4);
    }
}
