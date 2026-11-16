// ModStore orchestration methods mechanically extracted; bodies unchanged.
use super::*;
use crate::state::config::ConfigWrapper;
use snafu::prelude::*;
use std::path::Path;

impl ModStore {
    pub fn new<P: AsRef<Path>>(
        cache_path: P,
        parameters: &HashMap<String, HashMap<String, String>>,
    ) -> Result<Self, ProviderError> {
        let mut providers = HashMap::new();
        for prov in Self::get_provider_factories() {
            let params = parameters.get(prov.id).cloned().unwrap_or_default();
            if prov.parameters.iter().all(|p| params.contains_key(p.id)) {
                let Ok(provider) = (prov.new)(&params) else {
                    return Err(ProviderError::InitProviderFailed {
                        id: prov.id,
                        parameters: params.to_owned(),
                    });
                };
                providers.insert(prov.id, provider);
            }
        }

        let cache_metadata_path = cache_path.as_ref().join("cache.json");

        let cache = read_cache_metadata_or_default(&cache_metadata_path)?;
        let cache = ConfigWrapper::new(&cache_metadata_path, cache);
        cache.save().unwrap();

        Ok(Self {
            cache_path: cache_path.as_ref().to_path_buf(),
            local_refresh: Default::default(),
            local_files: Default::default(),
            providers: RwLock::new(providers),
            cache: Arc::new(RwLock::new(cache)),
            blob_cache: BlobCache::new(cache_path.as_ref().join("blobs")),
        })
    }

    pub fn get_provider_factories() -> impl Iterator<Item = &'static ProviderFactory> {
        inventory::iter::<ProviderFactory>()
    }

    pub fn add_provider(
        &self,
        provider_factory: &ProviderFactory,
        parameters: &HashMap<String, String>,
    ) -> Result<(), ProviderError> {
        let provider = (provider_factory.new)(parameters)?;
        self.providers
            .write()
            .unwrap()
            .insert(provider_factory.id, provider);
        Ok(())
    }

    pub async fn add_provider_checked(
        &self,
        provider_factory: &ProviderFactory,
        parameters: &HashMap<String, String>,
    ) -> Result<(), ProviderError> {
        let provider = (provider_factory.new)(parameters)?;
        provider.check().await?;
        self.providers
            .write()
            .unwrap()
            .insert(provider_factory.id, provider);
        Ok(())
    }

    pub fn get_provider(&self, url: &str) -> Result<Arc<dyn ModProvider>, ProviderError> {
        let factory = Self::get_provider_factories()
            .find(|f| (f.can_provide)(url))
            .context(ProviderNotFoundSnafu {
                url: url.to_string(),
            })?;
        let lock = self.providers.read().unwrap();
        Ok(match lock.get(factory.id) {
            Some(e) => e.clone(),
            None => NoProviderSnafu {
                url: url.to_string(),
                factory,
            }
            .fail()?,
        })
    }
}
