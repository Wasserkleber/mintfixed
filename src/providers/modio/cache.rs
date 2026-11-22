use super::{
    BlobRef, Deserialize, DrgModio, HashMap, HashSet, MODIO_PROVIDER_ID, ModInfo, ModProviderCache,
    ModResolution, ModResponse, ModSpecification, ModioMod, ModioProvider, ProviderCache,
    ProviderError, Serialize, SystemTime, UNIX_EPOCH, format_spec, is_unavailable_error, parse_url,
    process_modio_tags,
};
use crate::providers::ModProvider;
use tracing::info;

/// Look up the display name directly from cached mod.io metadata.
/// Unlike `get_mod_info`, this does not require dependency metadata.
pub(crate) fn cached_name(id: u32, cache: &ProviderCache) -> Option<String> {
    let cache = cache.read().unwrap();
    cache
        .get::<ModioCache>(MODIO_PROVIDER_ID)?
        .mods
        .get(&id)
        .map(|mod_| mod_.name.clone())
        .filter(|name| !name.trim().is_empty())
}

// typetag/Serde can buffer a provider as generic content before deserializing
// it. JSON object keys then arrive as strings, so a plain HashMap<u32, _>
// rejects otherwise valid mod.io IDs (e.g. "123"). Parse the keys explicitly.
fn deserialize_modio_id_map<'de, D, V>(deserializer: D) -> Result<HashMap<u32, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: Deserialize<'de>,
{
    let entries = HashMap::<String, V>::deserialize(deserializer)?;
    entries
        .into_iter()
        .map(|(key, value)| {
            let id = key.parse::<u32>().map_err(serde::de::Error::custom)?;
            Ok((id, value))
        })
        .collect()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModioCache {
    pub(super) mod_id_map: HashMap<String, u32>,
    #[serde(deserialize_with = "deserialize_modio_id_map")]
    pub(super) modfile_blobs: HashMap<u32, BlobRef>,
    #[serde(deserialize_with = "deserialize_modio_id_map")]
    pub(super) dependencies: HashMap<u32, Vec<u32>>,
    #[serde(deserialize_with = "deserialize_modio_id_map")]
    pub(super) mods: HashMap<u32, ModioMod>,
    pub(super) last_update_time: Option<SystemTime>,
    #[serde(default)]
    pub(super) unavailable_names: HashMap<String, bool>,
}

impl Default for ModioCache {
    fn default() -> Self {
        Self {
            mod_id_map: Default::default(),
            modfile_blobs: Default::default(),
            dependencies: Default::default(),
            mods: Default::default(),
            last_update_time: Some(SystemTime::now()),
            unavailable_names: HashMap::new(),
        }
    }
}

// Only a canonical mod.io slug is persisted. Never remove cached metadata/blobs.
pub(crate) fn mark_unavailable(cache: &ProviderCache, url: &str) {
    if let Ok(parsed) = parse_url(url) {
        cache
            .write()
            .unwrap()
            .get_mut::<ModioCache>(MODIO_PROVIDER_ID)
            .unavailable_names
            .insert(parsed.name_id.to_owned(), true);
    }
}

pub(crate) fn clear_unavailable(cache: &ProviderCache, url: &str) {
    if let Ok(parsed) = parse_url(url) {
        cache
            .write()
            .unwrap()
            .get_mut::<ModioCache>(MODIO_PROVIDER_ID)
            .unavailable_names
            .remove(parsed.name_id);
    }
}

pub(crate) fn is_unavailable(cache: &ProviderCache, url: &str) -> bool {
    let Ok(parsed) = parse_url(url) else {
        return false;
    };
    cache
        .read()
        .unwrap()
        .get::<ModioCache>(MODIO_PROVIDER_ID)
        .is_some_and(|c| c.unavailable_names.contains_key(parsed.name_id))
}

pub(crate) fn cache_counts(cache: &ProviderCache) -> (usize, usize) {
    cache
        .read()
        .unwrap()
        .get::<ModioCache>(MODIO_PROVIDER_ID)
        .map(|c| (c.mods.len(), c.unavailable_names.len()))
        .unwrap_or((0, 0))
}

/// Drops every mod that mod.io reported gone and that nothing still needs:
/// not a profile, group or bitfix entry (`referenced`), and not a cached
/// dependency of one. Its metadata, alias, dependency list and blob refs
/// leave cache.json, so later cache updates stop asking mod.io about it.
/// Gone mods that are still referenced keep everything, so their profiles
/// keep integrating from the cached files. Only names in `unavailable_names`
/// are candidates, and those are set only by a definite "gone" answer, so a
/// network failure can never remove anything here. Returns the dropped slugs.
pub(crate) fn forget_unreferenced_unavailable(
    cache: &ProviderCache,
    referenced: &[ModSpecification],
) -> Vec<String> {
    let mut lock = cache.write().unwrap();
    // get_mut would create an empty mod.io cache; only look at an existing one.
    if lock
        .get::<ModioCache>(MODIO_PROVIDER_ID)
        .is_none_or(|c| c.unavailable_names.is_empty())
    {
        return Vec::new();
    }
    let c = lock.get_mut::<ModioCache>(MODIO_PROVIDER_ID);

    // First collect what the profiles still need: each referenced mod, by
    // slug, explicit ID and cached alias, then its cached dependencies.
    let mut keep_names = HashSet::new();
    let mut keep_ids = HashSet::new();
    for spec in referenced {
        if !spec.url.starts_with("https://mod.io/") {
            continue;
        }
        let Ok(parsed) = parse_url(&spec.url) else {
            // A mod.io reference we can't read might be one of these mods;
            // keep everything rather than guess.
            return Vec::new();
        };
        keep_names.insert(parsed.name_id.to_owned());
        keep_ids.extend(parsed.mod_id);
        keep_ids.extend(c.mod_id_map.get(parsed.name_id).copied());
    }
    keep_ids.extend(
        c.mods
            .iter()
            .filter(|(_, m)| keep_names.contains(&m.name_id))
            .map(|(id, _)| *id),
    );
    let mut pending: Vec<u32> = keep_ids.iter().copied().collect();
    while let Some(id) = pending.pop() {
        for dep in c.dependencies.get(&id).into_iter().flatten() {
            if keep_ids.insert(*dep) {
                pending.push(*dep);
            }
        }
    }
    keep_names.extend(
        keep_ids
            .iter()
            .filter_map(|id| c.mods.get(id).map(|m| m.name_id.clone())),
    );

    // Then drop each gone mod outside that set, with everything keyed by it.
    let gone: Vec<String> = c
        .unavailable_names
        .keys()
        .filter(|name| !keep_names.contains(*name))
        .cloned()
        .collect();
    for name in &gone {
        let mut ids: HashSet<u32> = c
            .mods
            .iter()
            .filter(|(_, m)| &m.name_id == name)
            .map(|(id, _)| *id)
            .collect();
        ids.extend(c.mod_id_map.remove(name));
        for id in ids {
            if let Some(m) = c.mods.remove(&id) {
                for file in m.modfiles.iter().map(|f| f.id).chain(m.latest_modfile) {
                    c.modfile_blobs.remove(&file);
                }
            }
            c.dependencies.remove(&id);
        }
        c.unavailable_names.remove(name);
        info!(
            "dropped {name} from the mod.io cache: mod.io no longer lists it and no profile uses it"
        );
    }
    gone
}

#[typetag::serde]
impl ModProviderCache for ModioCache {
    fn new() -> Self {
        Default::default()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl<M: DrgModio + Send + Sync> ModioProvider<M> {
    pub(super) async fn update_cache_impl(
        &self,
        cache: ProviderCache,
    ) -> Result<(), ProviderError> {
        use futures::stream::{self, StreamExt};

        let now = SystemTime::now();

        let (last_update, name_map, previously_unavailable) = {
            let cache = cache.read().unwrap();
            let Some(prov) = cache.get::<ModioCache>(MODIO_PROVIDER_ID) else {
                return Ok(()); // no existing mods, nothing to update
            };
            (
                prov.last_update_time,
                prov.mods
                    .iter()
                    .map(|(id, mod_)| (*id, mod_.name_id.clone()))
                    .collect::<HashMap<_, _>>(),
                prov.unavailable_names
                    .keys()
                    .cloned()
                    .collect::<HashSet<_>>(),
            )
        };

        // A freshly recreated cache has nothing to update yet. Resolving
        // profile/group references after this step will populate it.
        if name_map.is_empty() {
            return Ok(());
        }

        let last_update = last_update
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .unwrap_or_default();

        let mut mod_ids = self
            .modio
            .fetch_mod_updates_since(
                name_map.keys().cloned().collect::<Vec<u32>>(),
                last_update.as_secs(),
            )
            .await?;
        // Recheck flagged mods even if mod.io reports no new event for them.
        for (id, name) in &name_map {
            if previously_unavailable.contains(name) {
                mod_ids.insert(*id);
            }
        }

        // TODO most of this is ripped from generic provider code. the resolution process is overly
        // complex and should be redone now that there's a much better understanding of what
        // exactly is required
        let mut to_resolve = mod_ids
            .iter()
            .filter_map(|id| name_map.get(id).map(|name| format_spec(name, *id, None)))
            .collect::<HashSet<_>>();

        let mut mods_map = HashMap::new();

        // used to deduplicate dependencies from mods already present in the mod list
        let mut precise_mod_specs = HashSet::new();

        pub async fn resolve_mod<M: DrgModio>(
            prov: &ModioProvider<M>,
            cache: ProviderCache,
            original_spec: ModSpecification,
        ) -> Result<(ModSpecification, ModInfo), ProviderError> {
            let mut spec = original_spec.clone();
            loop {
                match prov.resolve_mod(&spec, true, cache.clone()).await? {
                    ModResponse::Resolve(m) => {
                        return Ok((original_spec, m));
                    }
                    ModResponse::Redirect(redirected_spec) => spec = redirected_spec,
                };
            }
        }

        // Track attempts as well as successes: an unavailable dependency must
        // not be re-enqueued forever by the next dependency-expansion pass.
        let mut attempted = HashSet::new();
        while !to_resolve.is_empty() {
            let batch = std::mem::take(&mut to_resolve);
            attempted.extend(batch.iter().cloned());
            let results = stream::iter(batch.into_iter().map(|spec| {
                let cache = cache.clone();
                async move {
                    let result = resolve_mod(self, cache, spec.clone()).await;
                    (spec, result)
                }
            }))
            .boxed()
            .buffer_unordered(5)
            .collect::<Vec<_>>()
            .await;

            // Collect the whole batch before inspecting individual outcomes.
            // A 404 must not cancel other already-running mod updates.
            for (spec, result) in results {
                match result {
                    Ok((original, info)) => {
                        clear_unavailable(&cache, &spec.url);
                        precise_mod_specs.insert(info.spec.clone());
                        mods_map.insert(original, info);
                    }
                    // The provider can't tell whether a profile still uses
                    // this mod; the caller warns for those that do and drops
                    // the rest (see forget_unreferenced_unavailable).
                    Err(error) if is_unavailable_error(&error) => {
                        info!(
                            "mod.io mod unavailable; preserving cached files and metadata: {} ({error})",
                            spec.url
                        );
                        mark_unavailable(&cache, &spec.url);
                    }
                    Err(error) => return Err(error),
                }
            }
            to_resolve = crate::providers::dependencies::pending_dependencies(
                mods_map.values(),
                &precise_mod_specs,
                &attempted,
            );
        }

        let mut lock = cache.write().unwrap();
        let c = lock.get_mut::<ModioCache>(MODIO_PROVIDER_ID);
        c.last_update_time = Some(now);

        Ok(())
    }
    pub(super) fn get_mod_info_impl(
        &self,
        spec: &ModSpecification,
        cache: ProviderCache,
    ) -> Option<ModInfo> {
        let url = &spec.url;
        let parsed = parse_url(url).ok()?;

        let cache = cache.read().unwrap();
        let prov = cache.get::<ModioCache>(MODIO_PROVIDER_ID)?;

        let mod_id = if let Some(mod_id) = parsed.mod_id {
            mod_id
        } else {
            prov.mod_id_map.get(parsed.name_id).cloned()?
        };
        let mod_ = prov.mods.get(&mod_id)?;
        let modfile_id = if let Some(modfile_id) = parsed.modfile_id {
            modfile_id
        } else {
            mod_.latest_modfile?
        };

        let deps = prov
            .dependencies
            .get(&mod_id)?
            .iter()
            .map(|id| {
                prov.mods
                    .get(id)
                    .map(|m| format_spec(&m.name_id, *id, None))
            })
            .collect::<Option<Vec<_>>>()?;

        Some(ModInfo {
            provider: MODIO_PROVIDER_ID,
            spec: format_spec(&mod_.name_id, mod_id, None),
            name: mod_.name.clone(),
            versions: mod_
                .modfiles
                .iter()
                .map(|f| format_spec(&mod_.name_id, mod_id, Some(f.id)))
                .collect(),
            resolution: ModResolution::resolvable(
                format_spec(&mod_.name_id, mod_id, Some(modfile_id))
                    .url
                    .into(),
            ),
            suggested_require: mod_.tags.contains("RequiredByAll"),
            suggested_dependencies: deps,
            modio_tags: Some(process_modio_tags(&mod_.tags)),
            modio_id: Some(mod_id),
        })
    }
    pub(super) fn get_version_name_impl(
        &self,
        spec: &ModSpecification,
        cache: ProviderCache,
    ) -> Option<String> {
        let parsed = parse_url(&spec.url).ok()?;

        let cache = cache.read().unwrap();
        let prov = cache.get::<ModioCache>(MODIO_PROVIDER_ID);

        let mod_id = if let Some(mod_id) = parsed.mod_id {
            mod_id
        } else {
            prov.and_then(|c| c.mod_id_map.get(parsed.name_id).cloned())?
        };

        let mod_ = prov.and_then(|c| c.mods.get(&mod_id).cloned())?;

        if let Some(file_id) = parsed.modfile_id {
            if let Some(file) = mod_.modfiles.iter().find(|f| f.id == file_id) {
                if let Some(version) = &file.version {
                    Some(format!("{} - {}", file.id, version))
                } else {
                    Some(file_id.to_string())
                }
            } else {
                Some(file_id.to_string())
            }
        } else {
            Some("latest".to_string())
        }
    }
}
