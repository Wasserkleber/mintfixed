// ModStore orchestration methods mechanically extracted; bodies unchanged.
use super::*;
use tracing::info;

impl ModStore {
    pub async fn update_cache(&self, update: &CacheUpdate) -> Result<(), ProviderError> {
        let providers = self.providers.read().unwrap().clone();
        for (name, provider) in providers.iter() {
            info!("updating cache for {name} provider");
            provider.update_cache(self.cache.clone(), update).await?;
        }
        Ok(())
    }

    /// Write status flags and updated metadata immediately (not just on app exit).
    pub fn save_cache_metadata(&self) -> Result<(), ProviderError> {
        let mut cache = self.cache.write().unwrap();
        cache.revision = cache.revision.wrapping_add(1);
        cache
            .save()
            .map_err(|error| ProviderError::CacheCleanupFailed {
                reason: format!("Could not save cache metadata: {error}"),
            })
    }
}
