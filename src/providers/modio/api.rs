use super::*;

#[derive(Default)]
struct LoggingMiddleware {
    requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Middleware for LoggingMiddleware {
    async fn handle(
        &self,
        req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        let mut retries = 0_u32;
        loop {
            info!(
                "request started {} {:?}",
                self.requests
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                req.url().path()
            );
            let res = next.clone().run(req.try_clone().unwrap(), extensions).await;
            let retry_after = match &res {
                Ok(response)
                    if response.status() == reqwest::StatusCode::REQUEST_TIMEOUT
                        || response.status().as_u16() == 425
                        || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || response.status().is_server_error() =>
                {
                    response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok())
                }
                Err(reqwest_middleware::Error::Reqwest(error))
                    if error.is_connect()
                        || error.is_timeout()
                        || error.is_body()
                        || error.is_request() =>
                {
                    warn!(
                        "transient mod.io request failure at {}: {error}",
                        req.url().path()
                    );
                    None
                }
                _ => return res,
            };
            retries = retries.saturating_add(1);
            // Do not hammer mod.io if its Retry-After is zero or malformed.
            let fallback = (1_u64 << retries.min(6)).min(60);
            let delay = retry_after
                .filter(|seconds| *seconds > 0)
                .unwrap_or(fallback);
            warn!(
                "mod.io request {}: retry {retries} in {delay}s",
                req.url().path()
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
        }
    }
}

#[cfg_attr(test, automock)]
#[async_trait::async_trait]
pub trait DrgModio: Sync + Send {
    fn with_parameters(parameters: &HashMap<String, String>) -> Result<Self, DrgModioError>
    where
        Self: Sized;
    async fn check(&self) -> Result<(), DrgModioError>;
    async fn fetch_mod(&self, url: String, id: u32) -> Result<ModioMod, DrgModioError>;
    async fn fetch_files(&self, url: String, mod_id: u32) -> Result<ModioMod, DrgModioError>;
    async fn fetch_file(
        &self,
        url: String,
        mod_id: u32,
        modfile_id: u32,
    ) -> Result<modio::files::File, DrgModioError>;
    async fn fetch_dependencies(&self, url: String, mod_id: u32)
    -> Result<Vec<u32>, DrgModioError>;
    async fn fetch_mods_by_name(
        &self,
        name_id: &str,
    ) -> Result<Vec<ModioModResponse>, DrgModioError>;
    async fn fetch_mods_by_ids(
        &self,
        filter_ids: Vec<u32>,
    ) -> Result<Vec<modio::mods::Mod>, DrgModioError>;
    async fn fetch_mod_updates_since(
        &self,
        mod_ids: Vec<u32>,
        last_update: u64,
    ) -> Result<HashSet<u32>, DrgModioError>;
    fn download<A: 'static>(&self, action: A) -> modio::download::Downloader
    where
        modio::download::DownloadAction: From<A>;
}

#[async_trait::async_trait]
impl DrgModio for modio::Modio {
    fn with_parameters(parameters: &HashMap<String, String>) -> Result<Self, DrgModioError> {
        let client = reqwest_middleware::ClientBuilder::new(reqwest::Client::new())
            .with::<LoggingMiddleware>(Default::default())
            .build();
        let modio = modio::Modio::new(
            modio::Credentials::with_token(
                "".to_owned(), // TODO patch modio to not use API key at all
                parameters.get("oauth").context(MissingOauthTokenSnafu)?,
            ),
            client,
        )
        .context(GenericModioSnafu)?;

        Ok(modio)
    }

    async fn check(&self) -> Result<(), DrgModioError> {
        use modio::filter::Eq;
        use modio::mods::filters::Id;

        self.game(MODIO_DRG_ID)
            .mods()
            .search(Id::eq(0))
            .collect()
            .await
            .context(CheckFailedSnafu)?;
        Ok(())
    }

    async fn fetch_mod(&self, url: String, id: u32) -> Result<ModioMod, DrgModioError> {
        use modio::filter::{In, NotEq};
        use modio::mods::filters::Id;

        let files = self
            .game(MODIO_DRG_ID)
            .mod_(id)
            .files()
            .search(Id::ne(0))
            .collect()
            .await
            .with_context(|_| FetchModFilesFailedSnafu {
                mod_id: id,
                url: url.clone(),
            })?;

        let mods = self
            .game(MODIO_DRG_ID)
            .mods()
            .search(Id::_in(vec![id]))
            .collect()
            .await
            .context(FetchModFailedSnafu { mod_id: id, url })?;

        let r#mod = mods.into_iter().next().ok_or(DrgModioError::GenericError {
            msg: "mod.io returned no mod for the requested ID",
        })?;

        Ok(ModioMod::new(r#mod, files))
    }

    async fn fetch_files(&self, url: String, mod_id: u32) -> Result<ModioMod, DrgModioError> {
        use modio::filter::{In, NotEq};
        use modio::mods::filters::Id;

        let files = self
            .game(MODIO_DRG_ID)
            .mod_(mod_id)
            .files()
            .search(Id::ne(0))
            .collect()
            .await
            .with_context(|_| FetchModFilesFailedSnafu {
                mod_id,
                url: url.clone(),
            })?;

        let mods = self
            .game(MODIO_DRG_ID)
            .mods()
            .search(Id::_in(vec![mod_id]))
            .collect()
            .await
            .context(FetchModFailedSnafu { mod_id, url })?;

        let r#mod = mods.into_iter().next().ok_or(DrgModioError::GenericError {
            msg: "mod.io returned no mod for the requested ID",
        })?;

        Ok(ModioMod::new(r#mod, files))
    }

    async fn fetch_file(
        &self,
        url: String,
        mod_id: u32,
        modfile_id: u32,
    ) -> Result<modio::files::File, DrgModioError> {
        use modio::files::filters::Id;
        use modio::filter::Eq;

        let files = self
            .game(MODIO_DRG_ID)
            .mod_(mod_id)
            .files()
            .search(Id::eq(modfile_id))
            .collect()
            .await
            .with_context(|_| FetchModFileFailedSnafu {
                url: url.clone(),
                mod_id,
                modfile_id,
            })?;

        files.into_iter().next().ok_or(DrgModioError::GenericError {
            msg: "mod.io returned no mod file for the requested ID",
        })
    }

    async fn fetch_dependencies(
        &self,
        url: String,
        mod_id: u32,
    ) -> Result<Vec<u32>, DrgModioError> {
        Ok(self
            .game(MODIO_DRG_ID)
            .mod_(mod_id)
            .dependencies()
            .list()
            .await
            .with_context(|_| FetchDependenciesFailedSnafu { url, mod_id })?
            .into_iter()
            .map(|d| d.mod_id)
            .collect::<Vec<_>>())
    }

    async fn fetch_mods_by_name(
        &self,
        name_id: &str,
    ) -> Result<Vec<ModioModResponse>, DrgModioError> {
        use modio::filter::Eq;
        use modio::mods::filters::NameId;

        let filter = NameId::eq(name_id);
        Ok(self
            .game(MODIO_DRG_ID)
            .mods()
            .search(filter)
            .collect()
            .await
            .context(GenericModioSnafu)?
            .into_iter()
            .map(|m| m.into())
            .collect())
    }

    async fn fetch_mods_by_ids(
        &self,
        filter_ids: Vec<u32>,
    ) -> Result<Vec<modio::mods::Mod>, DrgModioError> {
        use modio::filter::In;
        use modio::mods::filters::Id;

        let filter = Id::_in(filter_ids);

        Ok(self
            .game(MODIO_DRG_ID)
            .mods()
            .search(filter)
            .collect()
            .await
            .context(GenericModioSnafu)?)
    }

    async fn fetch_mod_updates_since(
        &self,
        mod_ids: Vec<u32>,
        last_update: u64,
    ) -> Result<HashSet<u32>, DrgModioError> {
        use modio::filter::Cmp;
        use modio::filter::In;
        use modio::filter::NotIn;

        use modio::mods::EventType as EventTypes;
        use modio::mods::filters::DateAdded;
        use modio::mods::filters::events::EventType;
        use modio::mods::filters::events::ModId;

        let events = self
            .game(MODIO_DRG_ID)
            .mods()
            .events(
                EventType::not_in(vec![
                    EventTypes::ModCommentAdded,
                    EventTypes::ModCommentDeleted,
                ])
                .and(ModId::_in(mod_ids))
                .and(DateAdded::gt(last_update)),
            )
            .collect()
            .await
            .context(GenericModioSnafu)?;
        Ok(events.iter().map(|e| e.mod_id).collect::<HashSet<_>>())
    }

    fn download<A>(&self, action: A) -> modio::download::Downloader
    where
        modio::download::DownloadAction: From<A>,
    {
        self.download(action)
    }
}
