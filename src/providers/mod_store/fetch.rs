// ModStore orchestration methods mechanically extracted; bodies unchanged.
use super::*;

impl ModStore {
    pub async fn fetch_mods(
        &self,
        mods: &[&ModResolution],
        update: bool,
        tx: Option<Sender<FetchProgress>>,
    ) -> Result<Vec<PathBuf>, ProviderError> {
        use futures::stream::{self, StreamExt, TryStreamExt};

        stream::iter(
            mods.iter()
                .map(|res| self.fetch_mod(res, update, tx.clone())),
        )
        .boxed() // without this the future becomes !Send https://github.com/rust-lang/rust/issues/104382
        .buffer_unordered(5)
        .try_collect::<Vec<_>>()
        .await
    }

    pub async fn fetch_mods_ordered(
        &self,
        mods: &[&ModResolution],
        update: bool,
        tx: Option<Sender<FetchProgress>>,
    ) -> Result<Vec<PathBuf>, ProviderError> {
        use futures::stream::{self, StreamExt, TryStreamExt};

        stream::iter(
            mods.iter()
                .map(|res| self.fetch_mod(res, update, tx.clone())),
        )
        .boxed() // without this the future becomes !Send https://github.com/rust-lang/rust/issues/104382
        .buffered(5)
        .try_collect::<Vec<_>>()
        .await
    }

    pub async fn fetch_mod(
        &self,
        res: &ModResolution,
        update: bool,
        tx: Option<Sender<FetchProgress>>,
    ) -> Result<PathBuf, ProviderError> {
        self.get_provider(&res.url.0)?
            .fetch_mod(
                res,
                update,
                self.cache.clone(),
                &self.blob_cache.clone(),
                tx,
            )
            .await
    }
}
