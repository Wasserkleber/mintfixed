use std::collections::{BTreeSet, HashSet};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(test)]
use mockall::{automock, predicate::*};

use ::modio;

use reqwest::{Request, Response};
use reqwest_middleware::{Middleware, Next};
use serde::{Deserialize, Serialize};
use task_local_extensions::Extensions;
use tracing::*;

use crate::providers::*;

// The public provider surface remains stable; implementation lives in focused modules.
mod api;
mod cache;
mod download;
mod errors;
mod metadata;
mod rate_limit;
mod resolution;
#[cfg(test)]
#[path = "tests/mod.rs"]
mod test;
mod url;

pub use api::DrgModio;
#[cfg(test)]
use api::MockDrgModio;
pub use cache::ModioCache;
pub(super) use cache::cached_name;
pub(crate) use cache::{
    cache_counts, clear_unavailable, is_unavailable, mark_unavailable, unavailable_retry_due,
};
pub use errors::DrgModioError;
pub(crate) use errors::is_unavailable_error;
use errors::*; // Snafu context selectors and classification helper.
use metadata::process_modio_tags;
pub use metadata::{ModioFile, ModioMod, ModioModResponse};
use rate_limit::{MAX_DOWNLOAD_RESTARTS, RetryPolicy, global_rate_gate};
use url::{ModIoModUrl, format_spec, parse_url};
pub(super) use url::{cached_file, has_cached_download};
pub(crate) use url::{is_valid_modio_url, pinned_file};

const MODIO_DRG_ID: u32 = 2475;
const MODIO_PROVIDER_ID: &str = "modio";

inventory::submit! {
    super::ProviderFactory {
        id: MODIO_PROVIDER_ID,
        new: ModioProvider::<modio::Modio>::new_provider,
        can_provide: |url| parse_url(url).is_ok(),
        parameters: &[
            super::ProviderParameter {
                id: "oauth",
                name: "Personal Access Token",
                description: "mod.io Personal Access Token",
                link: Some("https://mod.io/me/access"),
            },
        ]
    }
}

pub struct ModioProvider<M: DrgModio> {
    modio: M,
}

impl<M: DrgModio + 'static> ModioProvider<M> {
    fn new_provider(
        parameters: &HashMap<String, String>,
    ) -> Result<Arc<dyn ModProvider>, ProviderError> {
        Ok(Arc::new(Self::new(M::with_parameters(parameters)?)))
    }
    fn new(modio: M) -> Self {
        Self { modio }
    }
}

#[async_trait::async_trait]
impl<M: DrgModio + Send + Sync> ModProvider for ModioProvider<M> {
    async fn resolve_mod(
        &self,
        spec: &ModSpecification,
        update: bool,
        cache: ProviderCache,
    ) -> Result<ModResponse, ProviderError> {
        self.resolve_mod_impl(spec, update, cache).await
    }
    async fn fetch_mod(
        &self,
        res: &ModResolution,
        _update: bool,
        cache: ProviderCache,
        blob_cache: &BlobCache,
        tx: Option<Sender<FetchProgress>>,
    ) -> Result<PathBuf, ProviderError> {
        self.fetch_mod_impl(res, _update, cache, blob_cache, tx)
            .await
    }
    async fn update_cache(
        &self,
        cache: ProviderCache,
        update: &CacheUpdate,
    ) -> Result<(), ProviderError> {
        self.update_cache_impl(cache, update).await
    }
    async fn check(&self) -> Result<(), ProviderError> {
        self.modio.check().await.map_err(Into::into)
    }
    fn get_mod_info(&self, spec: &ModSpecification, cache: ProviderCache) -> Option<ModInfo> {
        self.get_mod_info_impl(spec, cache)
    }
    fn is_pinned(&self, spec: &ModSpecification, _cache: ProviderCache) -> bool {
        parse_url(&spec.url)
            .ok()
            .is_some_and(|p| p.modfile_id.is_some())
    }
    fn get_version_name(&self, spec: &ModSpecification, cache: ProviderCache) -> Option<String> {
        self.get_version_name_impl(spec, cache)
    }
}
