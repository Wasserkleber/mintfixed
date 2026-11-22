use std::time::Duration;

use super::{
    BlobRef, CacheUpdate, Deserialize, DrgModio, HashMap, HashSet, MODIO_PROVIDER_ID, ModInfo,
    ModProviderCache, ModResolution, ModResponse, ModSpecification, ModioMod, ModioProvider,
    ProviderCache, ProviderError, Serialize, SystemTime, UNIX_EPOCH, format_spec,
    is_unavailable_error, parse_url, process_modio_tags, warn,
};
use crate::providers::ModProvider;

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
    #[serde(default, deserialize_with = "deserialize_unavailable")]
    pub(super) unavailable_names: HashMap<String, Unavailable>,
}

/// A mod.io mod that answered as gone: how many times in a row, and when it
/// was last asked. Update cache asks again only once `retry_due`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unavailable {
    pub(super) failures: u32,
    pub(super) last_checked: Option<SystemTime>,
}

/// The first recheck waits this long; each further failure doubles it, up to
/// `UNAVAILABLE_RETRY_MAX`, so a deleted mod costs about one request a week.
const UNAVAILABLE_RETRY_BASE: Duration = Duration::from_secs(60 * 60);
const UNAVAILABLE_RETRY_MAX: Duration = Duration::from_secs(7 * 24 * 60 * 60);

impl Unavailable {
    pub(super) fn retry_due(&self, now: SystemTime) -> bool {
        let Some(last) = self.last_checked else {
            return true;
        };
        let wait = UNAVAILABLE_RETRY_BASE
            .saturating_mul(1 << self.failures.saturating_sub(1).min(16))
            .min(UNAVAILABLE_RETRY_MAX);
        now.duration_since(last).is_ok_and(|waited| waited >= wait)
    }
}

// Caches written before the retry interval stored `true` per slug: read those
// as one failure with no check time, so they are asked about once more.
fn deserialize_unavailable<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, Unavailable>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry {
        Flag(#[allow(dead_code)] bool),
        Retry(Unavailable),
    }
    let entries = HashMap::<String, Entry>::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .map(|(name, entry)| {
            let entry = match entry {
                Entry::Retry(retry) => retry,
                Entry::Flag(_) => Unavailable {
                    failures: 1,
                    last_checked: None,
                },
            };
            (name, entry)
        })
        .collect())
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
        let mut lock = cache.write().unwrap();
        let entry = lock
            .get_mut::<ModioCache>(MODIO_PROVIDER_ID)
            .unavailable_names
            .entry(parsed.name_id.to_owned())
            .or_insert(Unavailable {
                failures: 0,
                last_checked: None,
            });
        entry.failures = entry.failures.saturating_add(1);
        entry.last_checked = Some(SystemTime::now());
    }
}

/// Whether a mod flagged unavailable may be asked about again: true for a mod
/// that is not flagged.
pub(crate) fn unavailable_retry_due(cache: &ProviderCache, url: &str) -> bool {
    let Ok(parsed) = parse_url(url) else {
        return true;
    };
    cache
        .read()
        .unwrap()
        .get::<ModioCache>(MODIO_PROVIDER_ID)
        .and_then(|c| c.unavailable_names.get(parsed.name_id))
        .is_none_or(|u| u.retry_due(SystemTime::now()))
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
        update: &CacheUpdate,
    ) -> Result<(), ProviderError> {
        use futures::stream::{self, StreamExt};

        let now = SystemTime::now();
        let strict = update.strict;
        let referenced = update
            .referenced
            .iter()
            .filter_map(|spec| parse_url(&spec.url).ok())
            .map(|parsed| parsed.name_id.to_owned())
            .collect::<HashSet<_>>();

        let (last_update, name_map, recheck) = {
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
                // Mods flagged unavailable that the active profile lists and whose
                // retry interval has passed. The rest wait: a mod nothing uses,
                // or one asked about recently, costs no request.
                prov.unavailable_names
                    .iter()
                    .filter(|(name, u)| referenced.contains(*name) && u.retry_due(now))
                    .map(|(name, _)| name.clone())
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
        // Recheck the due ones even if mod.io reports no new event for them.
        // A recheck that fails for any reason only waits for its next turn:
        // it neither stops the update nor holds back last_update_time.
        let mut recheck_only = HashSet::new();
        for (id, name) in &name_map {
            if recheck.contains(name) && mod_ids.insert(*id) {
                recheck_only.insert(format_spec(name, *id, None));
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
        let mut retry_later = false;
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
            // One mod that can't be refreshed never fails the update: its
            // cached metadata and files stay as they are and keep being used.
            for (spec, result) in results {
                match result {
                    Ok((original, info)) => {
                        clear_unavailable(&cache, &spec.url);
                        precise_mod_specs.insert(info.spec.clone());
                        mods_map.insert(original, info);
                    }
                    // mod.io says the mod is gone: flag it so the UI can show it.
                    Err(error) if is_unavailable_error(&error) => {
                        warn!(
                            "mod.io mod unavailable; keeping its cached version: {} ({error})",
                            spec.url
                        );
                        mark_unavailable(&cache, &spec.url);
                    }
                    // Anything else (network, rate limit, auth) may pass, so
                    // the mod isn't flagged, but it must be asked again next
                    // time, see last_update_time below. The strict setting
                    // makes it fail the update instead.
                    Err(error) if recheck_only.contains(&spec) => {
                        warn!(
                            "could not recheck unavailable mod {}; asking again later ({error})",
                            spec.url
                        );
                    }
                    Err(error) if strict => return Err(error),
                    Err(error) => {
                        warn!(
                            "could not update {}; keeping its cached version ({error})",
                            spec.url
                        );
                        retry_later = true;
                    }
                }
            }
            to_resolve = crate::providers::dependencies::pending_dependencies(
                mods_map.values(),
                &precise_mod_specs,
                &attempted,
            );
        }

        // The next update only asks about mods changed since this time, so
        // after a failed refresh leave it alone: the mod that failed is asked
        // about again next time instead of silently staying stale.
        if !retry_later {
            let mut lock = cache.write().unwrap();
            let c = lock.get_mut::<ModioCache>(MODIO_PROVIDER_ID);
            c.last_update_time = Some(now);
        }

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
