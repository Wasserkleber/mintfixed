use super::*;

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
        app.check_updates_rid = Some(MessageHandle {
            rid,
            handle,
            state: (),
        });
    }

    pub(super) fn receive(self, app: &mut App) {
        if Some(self.rid) == app.check_updates_rid.as_ref().map(|r| r.rid) {
            app.check_updates_rid = None;
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
                        app.available_update = Some(release);
                        app.show_update_time = Some(SystemTime::now());
                    }
                }
                Err(e) => tracing::warn!("failed to fetch update {e}"),
            }
        }
    }
}

pub(super) async fn integrate_async(
    store: Arc<ModStore>,
    ctx: egui::Context,
    entries: Vec<crate::state::InstallEntry>,
    fsd_pak: PathBuf,
    config: MetaConfig,
    rid: RequestID,
    message_tx: Sender<Message>,
) -> Result<crate::installation::report::InstallReport, IntegrationError> {
    let update = false;
    let mod_specs: Vec<_> = entries.iter().map(|entry| entry.spec.clone()).collect();

    let mods = store.resolve_mods(&mod_specs, update).await?;

    let selection = crate::installation::validation::select_installation_inputs(&entries, &mods)?;
    let to_integrate = selection.mods;
    // Report phases from the actual async operation, not from guessed download progress.
    message_tx
        .send(Message::InstallPhase {
            rid,
            phase: InstallPhase::Downloading,
        })
        .await
        .ok();
    ctx.request_repaint();
    let stage_tx = message_tx.clone();
    let stage_ctx = ctx.clone();
    // Index aliases once so every represented row receives fetch progress.
    let identity = |info: &ModInfo| {
        use crate::mod_identity::Artifact;
        match crate::providers::modio::pinned_file(&info.resolution.url.0) {
            Some((id, file)) if info.provider == "modio" => Artifact::Modio(id, file),
            _ => Artifact::Specification(ModSpecification {
                url: info.resolution.url.0.clone(),
            }),
        }
    };
    let mut aliases = HashMap::<_, Vec<ModSpecification>>::new();
    for spec in &mod_specs {
        let info = &mods[spec];
        aliases
            .entry((info.provider, identity(info)))
            .or_default()
            .push(spec.clone());
    }
    let res_map: HashMap<ModResolution, Vec<ModSpecification>> = to_integrate
        .iter()
        .map(|info| {
            (
                info.resolution.clone(),
                aliases
                    .remove(&(info.provider, identity(info)))
                    .unwrap_or_default(),
            )
        })
        .collect();
    let urls = to_integrate
        .iter()
        .map(|m| &m.resolution)
        .collect::<Vec<_>>();

    let (tx, mut rx) = mpsc::channel::<FetchProgress>(10);

    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            if let Some(specs) = res_map.get(progress.resolution()) {
                let progress: SpecFetchProgress = progress.into();
                for spec in specs {
                    message_tx
                        .send(Message::FetchModProgress(FetchModProgress {
                            rid,
                            spec: spec.clone(),
                            progress: progress.clone(),
                        }))
                        .await
                        .unwrap();
                }
                ctx.request_repaint();
            }
        }
    });

    let paths = store.fetch_mods_ordered(&urls, update, Some(tx)).await?;
    stage_tx
        .send(Message::InstallPhase {
            rid,
            phase: InstallPhase::Integrating,
        })
        .await
        .ok();
    stage_ctx.request_repaint();

    tokio::task::spawn_blocking(|| {
        crate::integrate::integrate_with_report(
            fsd_pak,
            config,
            to_integrate.into_iter().zip(paths).collect(),
            selection.locations,
            selection.skipped,
        )
    })
    .await?
}
