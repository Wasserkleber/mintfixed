use super::MAX_DOWNLOAD_RESTARTS;
use super::{
    BlobCache, DrgModio, FetchProgress, InvalidUrlSnafu, MODIO_PROVIDER_ID, ModCtxtIoSnafu,
    ModIoModUrl, ModResolution, ModioCache, ModioProvider, PathBuf, ProviderCache, ProviderError,
    Sender, error, info, modio, parse_url, warn,
};
use snafu::ResultExt;

impl<M: DrgModio + Send + Sync> ModioProvider<M> {
    pub(super) async fn fetch_mod_impl(
        &self,
        res: &ModResolution,
        _update: bool,
        cache: ProviderCache,
        blob_cache: &BlobCache,
        tx: Option<Sender<FetchProgress>>,
    ) -> Result<PathBuf, ProviderError> {
        let url = &res.url;
        let parsed = parse_url(&res.url.0)?;

        if let ModIoModUrl {
            name_id: _,
            mod_id: Some(mod_id),
            modfile_id: Some(modfile_id),
        } = parsed
        {
            Ok(
                if let Some(path) = {
                    cache
                        .read()
                        .unwrap()
                        .get::<ModioCache>(MODIO_PROVIDER_ID)
                        .and_then(|c| c.modfile_blobs.get(&modfile_id))
                        .and_then(|r| blob_cache.get_path(r))
                } {
                    if let Some(tx) = tx {
                        tx.send(FetchProgress::Complete {
                            resolution: res.clone(),
                        })
                        .await
                        .unwrap();
                    }
                    path
                } else {
                    let file = self
                        .modio
                        .fetch_file(res.url.0.clone(), mod_id, modfile_id)
                        .await?;

                    info!("downloading mod {url:?}...");

                    use futures::stream::TryStreamExt;
                    use tokio::io::AsyncWriteExt;

                    let mut file = file;
                    let mut retries = 0_u32;
                    // Never write a partial download to the blob cache. Restart with
                    // fresh mod.io file information if a CDN stream breaks mid-file.
                    let downloaded = 'download: loop {
                        let size = file.filesize;
                        let action: modio::download::DownloadAction = file.into();
                        let mut cursor = std::io::Cursor::new(Vec::new());
                        let mut stream = Box::pin(self.modio.download(action).stream());
                        loop {
                            match stream.try_next().await {
                                Ok(Some(bytes)) => {
                                    cursor
                                        .write_all(&bytes)
                                        .await
                                        .with_context(|_| ModCtxtIoSnafu { mod_id })?;
                                    if let Some(tx) = &tx {
                                        tx.send(FetchProgress::Progress {
                                            resolution: res.clone(),
                                            progress: cursor.get_ref().len() as u64,
                                            size,
                                        })
                                        .await
                                        .unwrap();
                                    }
                                    continue;
                                }
                                Ok(None) if cursor.get_ref().len() as u64 == size => {
                                    break 'download cursor.into_inner();
                                }
                                Ok(None) => {
                                    warn!(
                                        "mod.io download {}: received {} of {size} bytes",
                                        res.url.0,
                                        cursor.get_ref().len()
                                    );
                                }
                                Err(error) => {
                                    // The modio error may wrap a reqwest error from
                                    // the CDN rather than expose its HTTP status directly.
                                    let status = error.status().or_else(|| {
                                        std::error::Error::source(&error)
                                            .and_then(|source| {
                                                source.downcast_ref::<reqwest::Error>()
                                            })
                                            .and_then(|cause| cause.status())
                                    });
                                    let transient = status.is_none_or(|code| {
                                        code == reqwest::StatusCode::REQUEST_TIMEOUT
                                            || code.as_u16() == 425
                                            || code == reqwest::StatusCode::TOO_MANY_REQUESTS
                                            || code.is_server_error()
                                    });
                                    if error.is_auth()
                                        || error.is_builder()
                                        || error.is_download()
                                        || error.is_validation()
                                        || error.is_decode()
                                        || !transient
                                    {
                                        return Err(ProviderError::ModCtxtModioError {
                                            source: error,
                                            mod_id,
                                        });
                                    }
                                    warn!("mod.io download {} interrupted: {error}", res.url.0);
                                }
                            }
                            // A partial stream may return EOF or an error: discard the
                            // partial buffer and obtain a fresh signed download URL.
                            break;
                        }
                        drop(stream);
                        retries = retries.saturating_add(1);
                        if retries > MAX_DOWNLOAD_RESTARTS {
                            return Err(ProviderError::ModCtxtIoError {
                                mod_id,
                                source: std::io::Error::new(
                                    std::io::ErrorKind::UnexpectedEof,
                                    format!(
                                        "mod.io download {} remained interrupted after {MAX_DOWNLOAD_RESTARTS} restarts",
                                        res.url.0
                                    ),
                                ),
                            });
                        }
                        let delay = (1_u64 << retries.min(6)).min(60);
                        warn!(
                            "restarting mod.io download {} in {delay}s (attempt {retries})",
                            res.url.0
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(delay)).await;
                        file = self
                            .modio
                            .fetch_file(res.url.0.clone(), mod_id, modfile_id)
                            .await?;
                    };

                    let blob = blob_cache.write(&downloaded)?;
                    let path = blob_cache.get_path(&blob).unwrap();

                    cache
                        .write()
                        .unwrap()
                        .get_mut::<ModioCache>(MODIO_PROVIDER_ID)
                        .modfile_blobs
                        .insert(modfile_id, blob);

                    if let Some(tx) = tx {
                        tx.send(FetchProgress::Complete {
                            resolution: res.clone(),
                        })
                        .await
                        .unwrap();
                    }

                    path
                },
            )
        } else {
            InvalidUrlSnafu {
                url: url.0.to_string(),
            }
            .fail()?
        }
    }
}
