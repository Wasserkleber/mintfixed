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

    fn receive(self, app: &mut App) {
        if Some(self.rid) == app.self_update_rid.as_ref().map(|r| r.rid) {
            match self.result {
                Ok(original_exe_path) => {
                    info!("self update complete");
                    app.original_exe_path = Some(original_exe_path);
                    app.last_action = Some(LastAction::success("self update complete".to_string()));
                }
                Err(e) => {
                    error!("self update failed");
                    error!("{:#?}", e);
                    app.self_update_rid = None;
                    app.last_action = Some(LastAction::failure("self update failed".to_string()));
                }
            }
            app.integrate_rid = None;
        }
    }
}

#[derive(Debug)]
pub struct FetchSelfUpdateProgress {
    rid: RequestID,
    progress: SelfUpdateProgress,
}

impl FetchSelfUpdateProgress {
    fn receive(self, app: &mut App) {
        if let Some(MessageHandle { rid, state, .. }) = &mut app.self_update_rid
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
            self_update::Extract::from_source(&tmp_archive_path)
                .archive(self_update::ArchiveKind::Zip)
                .extract_file(tmp_dir.path(), bin_name)
                .map_err(Into::into)
                .with_context(|_| SelfUpdateFailedSnafu)?;

            info!("replacing old executable with new executable");
            let tmp_file = tmp_dir.path().join("replacement_tmp");
            let bin_path = tmp_dir.path().join(bin_name);

            let original_exe_path = std::env::current_exe()?;

            self_update::Move::from_source(&bin_path)
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

#[cfg(test)]
mod addition_destination_tests {
    use super::*;

    fn app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs::from_path(dir.path()).unwrap();
        std::fs::write(
            dirs.config_dir.join("mod_data.json"),
            serde_json::to_vec(&crate::state::VersionAnnotatedModData::default()).unwrap(),
        )
        .unwrap();
        let app = App::new(
            &eframe::CreationContext::_new_kittest(egui::Context::default()),
            dirs,
            None,
        )
        .unwrap();
        (dir, app)
    }

    fn result(app: &mut App) -> ResolveMods {
        let rid = app.request_counter.next();
        app.resolve_mod_rid = Some(MessageHandle {
            rid,
            handle: tokio::spawn(async {}),
            state: (),
        });
        let spec = ModSpecification::new("test-mod".to_owned());
        let info = ModInfo {
            provider: "file",
            name: "Test".into(),
            spec: spec.clone(),
            versions: vec![],
            resolution: ModResolution::resolvable("test-mod".to_owned().into()),
            suggested_require: false,
            suggested_dependencies: vec![],
            modio_id: None,
            modio_tags: None,
        };
        ResolveMods {
            rid,
            profile_id: app.state.mod_data.profiles["default"].id.clone(),
            profile: "default".into(),
            input: "submitted input".into(),
            specs: vec![spec.clone()],
            result: Ok(HashMap::from([(spec, info)])),
            is_dependency: false,
            from_bitfix: false,
        }
    }

    #[tokio::test]
    async fn profile_switch_does_not_redirect_result_or_clear_new_input() {
        let (_dir, mut app) = app();
        let result = result(&mut app);
        app.state
            .mod_data
            .profiles
            .insert("Other".into(), Default::default());
        app.state.mod_data.active_profile = "Other".into();
        app.resolve_mod = "new input".into();
        result.receive(&mut app);
        assert_eq!(app.state.mod_data.profiles["default"].mods.len(), 1);
        assert!(app.state.mod_data.profiles["Other"].mods.is_empty());
        assert_eq!(app.resolve_mod, "new input");
        assert!(app.resolve_mod_rid.is_none());
    }

    #[tokio::test]
    async fn second_request_does_not_replace_pending_result() {
        let (_dir, mut app) = app();
        let result = result(&mut app);
        ResolveMods::send(&mut app, &egui::Context::default(), vec![], false);
        assert_eq!(app.resolve_mod_rid.as_ref().unwrap().rid, result.rid);
        result.receive(&mut app);
        assert_eq!(app.state.mod_data.profiles["default"].mods.len(), 1);
    }

    #[tokio::test]
    async fn missing_original_profile_cancels_without_fallback() {
        let (_dir, mut app) = app();
        let result = result(&mut app);
        app.state
            .mod_data
            .profiles
            .insert("Other".into(), Default::default());
        app.state.mod_data.active_profile = "Other".into();
        app.state.mod_data.profiles.remove("default");
        result.receive(&mut app);
        assert!(app.state.mod_data.profiles["Other"].mods.is_empty());
        assert!(app.resolve_mod_rid.is_none());
    }
    #[tokio::test]
    async fn renamed_profile_receives_pending_addition_by_id() {
        let (_dir, mut app) = app();
        let result = result(&mut app);
        let profile = app.state.mod_data.profiles.remove("default").unwrap();
        app.state
            .mod_data
            .profiles
            .insert("Renamed".into(), profile);
        app.state.mod_data.active_profile = "Renamed".into();
        result.receive(&mut app);
        assert_eq!(app.state.mod_data.profiles["Renamed"].mods.len(), 1);
    }

    #[tokio::test]
    async fn replacement_profile_with_same_name_does_not_receive_old_request() {
        let (_dir, mut app) = app();
        let result = result(&mut app);
        app.state
            .mod_data
            .profiles
            .insert("default".into(), Default::default());
        result.receive(&mut app);
        assert!(app.state.mod_data.profiles["default"].mods.is_empty());
        assert!(app.resolve_mod_rid.is_none());
    }
}
