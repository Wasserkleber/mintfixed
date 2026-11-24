use std::time::SystemTime;
use mint_lib::{error::GenericError, update::GitHubRelease};
use crate::gui::{App, request_counter::RequestID};
use super::{Message, MessageHandle};

#[derive(Debug)]
pub struct CheckUpdates {
    rid: RequestID,
    result: Result<GitHubRelease, GenericError>,
}

impl CheckUpdates {
    pub fn send(app: &mut App, ctx: &egui::Context) {
        let rid = app.request_counter.next();
        let tx = app.tx.clone();
        let ctx = ctx.clone();

        let handle = tokio::spawn(async move {
            tx.send(Message::CheckUpdates(Self {
                rid,
                result: mint_lib::update::get_latest_release().await,
            }))
            .await
            .unwrap();
            ctx.request_repaint();
        });
        app.update_state.check_updates_rid = Some(MessageHandle {
            rid,
            handle,
            state: (),
        });
    }

    pub(super) fn receive(self, app: &mut App) {
        if Some(self.rid) == app.update_state.check_updates_rid.as_ref().map(|r| r.rid) {
            app.update_state.check_updates_rid = None;
            match self.result {
                Ok(release) => {
                    if let (Ok(version), Some(Ok(release_version))) = (
                        semver::Version::parse(env!("CARGO_PKG_VERSION")),
                        release
                            .tag_name
                            .strip_prefix('v')
                            .map(semver::Version::parse),
                    ) && release_version > version
                    {
                        app.update_state.available_update = Some(release);
                        app.update_state.show_update_time = Some(SystemTime::now());
                    }
                }
                Err(e) => tracing::warn!("failed to fetch update {e}"),
            }
        }
    }
}
