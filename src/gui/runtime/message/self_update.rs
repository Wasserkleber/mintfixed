use super::{Message, MessageHandle};
use crate::gui::{
    App, LastAction, SelfUpdateProgress,
    request_counter::{RequestCounter, RequestID},
};
use crate::integrate::{IntegrationError, SelfUpdateFailedSnafu};
#[cfg(target_os = "linux")]
use fs_err as fs;
use snafu::prelude::*;
use std::path::PathBuf;
use tokio::sync::mpsc::{self, Sender};
use tracing::{debug, error, info};

#[derive(Debug)]
pub struct SelfUpdate {
    rid: RequestID,
    result: Result<PathBuf, IntegrationError>,
}

impl SelfUpdate {
    pub fn send(
        rc: &mut RequestCounter,
        tx: Sender<Message>,
        ctx: egui::Context,
    ) -> MessageHandle<SelfUpdateProgress> {
        let rid = rc.next();
        MessageHandle {
            rid,
            handle: tokio::task::spawn(async move {
                let result = self_update_async(ctx.clone(), rid, tx.clone()).await;
                tx.send(Message::SelfUpdate(SelfUpdate { rid, result }))
                    .await
                    .unwrap();
                ctx.request_repaint();
            }),
            state: SelfUpdateProgress::Pending,
        }
    }

    pub(super) fn receive(self, app: &mut App) {
        if Some(self.rid) == app.update_state.self_update_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok(original_exe_path) => {
                    info!("self update complete");
                    app.update_state.original_exe_path = Some(original_exe_path);
                    app.status_state.last_action =
                        Some(LastAction::success("self update complete".to_string()));
                }
                Err(e) => {
                    error!("self update failed");
                    error!("{:#?}", e);
                    app.update_state.self_update_rid = None;
                    app.status_state.last_action =
                        Some(LastAction::failure("self update failed".to_string()));
                }
            }
            app.install_state.integrate_rid = None;
        }
    }
}

#[derive(Debug)]
pub struct FetchSelfUpdateProgress {
    rid: RequestID,
    progress: SelfUpdateProgress,
}

impl FetchSelfUpdateProgress {
    pub(super) fn receive(self, app: &mut App) {
        if let Some(MessageHandle { rid, state, .. }) = &mut app.update_state.self_update_rid
            && *rid == self.rid
        {
            *state = self.progress;
        }
    }
}

async fn self_update_async(
    ctx: egui::Context,
    rid: RequestID,
    message_tx: Sender<Message>,
) -> Result<PathBuf, IntegrationError> {
    use futures::stream::TryStreamExt;
    use tokio::io::AsyncWriteExt;

    let (tx, mut rx) = mpsc::channel::<SelfUpdateProgress>(1);

    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            message_tx
                .send(Message::FetchSelfUpdateProgress(FetchSelfUpdateProgress {
                    rid,
                    progress,
                }))
                .await
                .unwrap();
            ctx.request_repaint();
        }
    });

    let client = reqwest::Client::new();

    let asset_name = if cfg!(target_os = "windows") {
        "mint-x86_64-pc-windows-msvc.zip"
    } else if cfg!(target_os = "linux") {
        "mint-x86_64-unknown-linux-gnu.zip"
    } else {
        unimplemented!("unsupported platform");
    };

    info!("downloading update");

    let response = client
        .get(format!(
            "https://github.com/trumank/mint/releases/latest/download/{asset_name}"
        ))
        .send()
        .await
        .map_err(Into::into)
        .with_context(|_| SelfUpdateFailedSnafu)?
        .error_for_status()
        .map_err(Into::into)
        .with_context(|_| SelfUpdateFailedSnafu)?;
    let size = response.content_length();
    debug!(?response);
    debug!(?size);

    let tmp_dir = tempfile::Builder::new()
        .prefix("self_update")
        .tempdir_in(std::env::current_dir()?)?;
    let tmp_archive_path = tmp_dir.path().join(asset_name);
    let mut tmp_archive = tokio::fs::File::create(&tmp_archive_path)
        .await
        .map_err(Into::into)
        .with_context(|_| SelfUpdateFailedSnafu)?;
    let mut stream = response.bytes_stream();

    let mut total_bytes_written = 0;
    while let Some(bytes) = stream
        .try_next()
        .await
        .map_err(Into::into)
        .with_context(|_| SelfUpdateFailedSnafu)?
    {
        let bytes_written = tmp_archive.write(&bytes).await?;
        total_bytes_written += bytes_written;
        if let Some(size) = size {
            tx.send(SelfUpdateProgress::Progress {
                progress: total_bytes_written as u64,
                size,
            })
            .await
            .unwrap();
        }
    }

    debug!(?tmp_dir);
    debug!(?tmp_archive_path);
    debug!(?tmp_archive);

    let original_exe_path =
        tokio::task::spawn_blocking(move || -> Result<PathBuf, IntegrationError> {
            let bin_name = if cfg!(target_os = "windows") {
                "mint.exe"
            } else if cfg!(target_os = "linux") {
                "mint"
            } else {
                unimplemented!("unsupported platform");
            };

            info!("extracting downloaded update archive");
            ::self_update::Extract::from_source(&tmp_archive_path)
                .archive(::self_update::ArchiveKind::Zip)
                .extract_file(tmp_dir.path(), bin_name)
                .map_err(Into::into)
                .with_context(|_| SelfUpdateFailedSnafu)?;

            info!("replacing old executable with new executable");
            let tmp_file = tmp_dir.path().join("replacement_tmp");
            let bin_path = tmp_dir.path().join(bin_name);

            let original_exe_path = std::env::current_exe()?;

            ::self_update::Move::from_source(&bin_path)
                .replace_using_temp(&tmp_file)
                .to_dest(&original_exe_path)
                .map_err(Into::into)
                .with_context(|_| SelfUpdateFailedSnafu)?;

            #[cfg(target_os = "linux")]
            {
                info!("setting executable permission on new executable");
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&original_exe_path, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }

            Ok(original_exe_path)
        })
        .await??;

    tx.send(SelfUpdateProgress::Complete).await.unwrap();

    info!("update successful");

    Ok(original_exe_path)
}
