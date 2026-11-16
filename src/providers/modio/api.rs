#[cfg(test)]
use super::automock;
use super::{
    Arc, CheckFailedSnafu, DrgModioError, Extensions, FetchDependenciesFailedSnafu,
    FetchModFailedSnafu, FetchModFileFailedSnafu, FetchModFilesFailedSnafu, GenericModioSnafu,
    HashMap, HashSet, MODIO_DRG_ID, Middleware, MissingOauthTokenSnafu, ModioMod, ModioModResponse,
    Next, Request, Response, error, info, modio, warn,
};
use super::{RetryPolicy, global_rate_gate};
use snafu::{OptionExt, ResultExt};

#[derive(Default)]
struct LoggingMiddleware {
    requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    policy: RetryPolicy,
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
            global_rate_gate().wait().await;
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
            let next_retry = retries.saturating_add(1);
            // 429 responses gate *all* mod.io client instances, including when
            // this request has exhausted its own retry budget.
            let rate_limited = res
                .as_ref()
                .is_ok_and(|response| response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS);
            let delay = self.policy.delay(next_retry, retry_after);
            if rate_limited {
                global_rate_gate().defer(delay);
            }
            if retries >= self.policy.max_retries {
                warn!(
                    "mod.io request {}: exhausted {} retries",
                    req.url().path(),
                    retries
                );
                return res;
            }
            retries = next_retry;
            warn!(
                "mod.io request {}: retry {retries} in {}s",
                req.url().path(),
                delay.as_secs_f64()
            );
            if !rate_limited {
                tokio::time::sleep(delay).await;
            }
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

#[cfg(test)]
mod http_tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // A local HTTP stub: no live mod.io network and no external mock crates.
    async fn server(
        statuses: Vec<u16>,
    ) -> (
        String,
        tokio::task::JoinHandle<()>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}/", listener.local_addr().unwrap());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let recorded = calls.clone();
        let handle = tokio::spawn(async move {
            for status in statuses {
                let (mut socket, _) =
                    tokio::time::timeout(Duration::from_secs(5), listener.accept())
                        .await
                        .expect("request timeout")
                        .unwrap();
                let mut request = [0_u8; 2048];
                let _ = socket.read(&mut request).await.unwrap();
                recorded.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let reason = match status {
                    200 => "OK",
                    404 => "Not Found",
                    410 => "Gone",
                    429 => "Too Many Requests",
                    500 => "Internal Server Error",
                    _ => "Status",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: 2\r\nConnection: close\r\nRetry-After: 0\r\n\r\nOK"
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (address, handle, calls)
    }

    fn test_client(max_retries: u32) -> reqwest_middleware::ClientWithMiddleware {
        let middleware = LoggingMiddleware {
            policy: RetryPolicy {
                max_retries,
                base_delay: Duration::ZERO,
            },
            ..Default::default()
        };
        reqwest_middleware::ClientBuilder::new(reqwest::Client::new())
            .with(middleware)
            .build()
    }

    #[tokio::test]
    async fn response_429_retries_then_succeeds() {
        let (url, server, calls) = server(vec![429, 200]).await;
        let response = test_client(1).get(&url).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        server.await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn response_429_has_bounded_retries() {
        let (url, server, calls) = server(vec![429, 429, 429]).await;
        let response = test_client(2).get(&url).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
        server.await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn missing_mod_404_is_never_retried() {
        let (url, server, calls) = server(vec![404]).await;
        let response = test_client(5).get(&url).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        server.await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn response_500_retries_and_recovers() {
        let (url, server, calls) = server(vec![500, 200]).await;
        let response = test_client(1).get(&url).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        server.await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn truncated_http_body_is_reported_as_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 32\r\nConnection: close\r\n\r\nshort",
                )
                .await
                .unwrap();
        });
        let response = reqwest::Client::new().get(url).send().await.unwrap();
        assert!(
            response.bytes().await.is_err(),
            "short body must not be accepted"
        );
        server.await.unwrap();
    }
}
