// settings and provider configuration windows; methods moved intact from gui/mod.rs.
use super::*;

impl App {
    pub(super) fn show_provider_parameters(&mut self, ctx: &egui::Context) {
        let Some(window) = &mut self.window_provider_parameters else {
            return;
        };

        while let Ok((rid, res)) = window.rx.try_recv() {
            if window.check_rid.as_ref().is_some_and(|r| rid == r.0) {
                match res {
                    Ok(()) => {
                        let window = self.window_provider_parameters.take().unwrap();
                        self.state
                            .config
                            .provider_parameters
                            .insert(window.factory.id.to_string(), window.parameters);
                        self.state.config.save().unwrap();
                        return;
                    }
                    Err(e) => {
                        window.check_error = Some(e.to_string());
                    }
                }
                window.check_rid = None;
            }
        }

        let mut open = true;
        let mut check = false;
        egui::Window::new(format!("Configure {} provider", window.factory.id))
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_enabled_ui(window.check_rid.is_none(), |ui| {
                    egui::Grid::new("grid").num_columns(2).show(ui, |ui| {
                        for p in window.factory.parameters {
                            if let Some(link) = p.link {
                                ui.hyperlink_to(p.name, link).on_hover_text(p.description);
                            } else {
                                ui.label(p.name).on_hover_text(p.description);
                            }
                            let res = ui.add(
                                egui::TextEdit::singleline(
                                    window.parameters.entry(p.id.to_string()).or_default(),
                                )
                                .password(true)
                                .desired_width(200.0),
                            );
                            if is_committed(&res) {
                                check = true;
                            }
                            ui.end_row();
                        }
                    });

                    ui.with_layout(Layout::right_to_left(Align::TOP), |ui| {
                        if ui.button("Save").clicked() {
                            check = true;
                        }
                        if window.check_rid.is_some() {
                            ui.spinner();
                        }
                        if let Some(error) = &window.check_error {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                    });
                });
            });
        if !open {
            self.window_provider_parameters = None;
        } else if check {
            window.check_error = None;
            let tx = window.tx.clone();
            let ctx = ctx.clone();
            let rid = self.request_counter.next();
            let store = self.state.store.clone();
            let params = window.parameters.clone();
            let factory = window.factory;
            let handle = tokio::task::spawn(async move {
                let res = store.add_provider_checked(factory, &params).await;
                tx.send((rid, res)).await.unwrap();
                ctx.request_repaint();
            });
            window.check_rid = Some((rid, handle));
        }
    }

    pub(super) fn show_settings(&mut self, ctx: &egui::Context) {
        let mut auto_save_local_folder: Option<(String, bool)> = None;
        let mut rescan_folder = false;
        if let Some(window) = &mut self.settings_window {
            let mut open = true;
            let mut try_save = false;
            egui::Window::new("Settings")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::Grid::new("grid").num_columns(2).striped(true).show(ui, |ui| {
                        let mut job = LayoutJob::default();
                        job.append(
                            "DRG pak",
                            0.0,
                            TextFormat {
                                color: ui.visuals().text_color(),
                                underline: Stroke::new(1.0_f32, ui.visuals().text_color()),
                                ..Default::default()
                            },
                        );
                        ui.label(job).on_hover_cursor(egui::CursorIcon::Help).on_hover_text("Path to FSD-WindowsNoEditor.pak (FSD-WinGDK.pak for Microsoft Store version)\nLocated inside the \"Deep Rock Galactic\" installation directory under FSD/Content/Paks.");
                        ui.horizontal(|ui| {
                            let res = ui.add(
                                egui::TextEdit::singleline(
                                    &mut window.drg_pak_path
                                )
                                .desired_width(200.0),
                            );
                            if res.changed() {
                                window.drg_pak_path_err = None;
                            }
                            if is_committed(&res) {
                                try_save = true;
                            }
                            if ui.button("browse").clicked()
                                && let Some(fsd_pak) = rfd::FileDialog::new()
                                    .add_filter("DRG Pak", &["pak"])
                                    .pick_file()
                                {
                                    window.drg_pak_path = fsd_pak.to_string_lossy().to_string();
                                    window.drg_pak_path_err = None;
                                }
                        });
                        ui.end_row();

                        ui.label("Local mods folder:")
                            .on_hover_text("Choose a local mods folder. Use Import .pak/.zip beside Add mod to scan it; only explicit exclusions stay hidden.");
                        ui.horizontal(|ui| {
                            let res = ui.add(egui::TextEdit::singleline(&mut window.local_mod_folder)
                                .desired_width(200.0)
                                .hint_text("Choose a mods folder..."));
                            if res.changed() {
                                window.local_mod_folder_err = None;
                                auto_save_local_folder = Some((window.local_mod_folder.clone(), false));
                            }
                            if is_committed(&res) {
                                auto_save_local_folder = Some((window.local_mod_folder.clone(), true));
                            }
                            if ui.button("browse").clicked()
                                && let Some(folder) = rfd::FileDialog::new().pick_folder()
                            {
                                window.local_mod_folder = folder.display().to_string();
                                window.local_mod_folder_err = None;
                                auto_save_local_folder = Some((window.local_mod_folder.clone(), true));
                            }
                            if let Some(error) = &window.local_mod_folder_err {
                                ui.colored_label(ui.visuals().error_fg_color, error);
                            }
                        });
                        ui.end_row();
                        let config_dir = &self.state.dirs.config_dir;
                        ui.label("Config directory:");
                        if ui.link(config_dir.display().to_string()).clicked() {
                            opener::open(config_dir).ok();
                        }
                        ui.end_row();

                        let cache_dir = &self.state.dirs.cache_dir;
                        ui.label("Cache directory:");
                        if ui.link(cache_dir.display().to_string()).clicked() {
                            opener::open(cache_dir).ok();
                        }
                        ui.end_row();

                        let data_dir = &self.state.dirs.data_dir;
                        ui.label("Data directory:");
                        if ui.link(data_dir.display().to_string()).clicked() {
                            opener::open(data_dir).ok();
                        }
                        ui.end_row();

                        ui.label("GUI theme:");
                        ui.horizontal(|ui| {
                            ui.horizontal(|ui| {
                                let config = &mut self.state.config;

                                let old_theme = GuiTheme::into_egui_theme(config.gui_theme);
                                let mut theme = old_theme;
                                theme.radio_buttons(ui);
                                if theme != old_theme {
                                    ui.memory_mut(|m| m.options.theme_preference = theme);
                                    config.gui_theme = GuiTheme::from_egui_theme(theme);
                                    config.save().unwrap();
                                }
                            });
                        });
                        ui.end_row();

                        ui.label("Bundled crash reporter:");
                        let config = &mut self.state.config;
                        if ui
                            .add(toggle_switch(&mut config.bundled_crash_reporter))
                            .on_hover_text("Install and autolaunch the pinned BP-Crash-Reporter. Turn off to run your own; applied on the next Install.")
                            .changed()
                        {
                            config.save().unwrap();
                        }
                        ui.end_row();

                        ui.label("Stop Update cache on mod errors:");
                        let config = &mut self.state.config;
                        if ui
                            .add(toggle_switch(&mut config.strict_cache_update))
                            .on_hover_text("When a mod can't be refreshed (network, rate limit, auth), stop Update cache with an error. Off: warn and keep using the cached version. Mods removed from mod.io never stop it.")
                            .changed()
                        {
                            config.save().unwrap();
                        }
                        ui.end_row();

                        ui.label("Archive unused mods on Update cache:");
                        let config = &mut self.state.config;
                        if ui
                            .add(toggle_switch(&mut config.archive_unused_on_update))
                            .on_hover_text("Before updating, move the cached entries of mods no profile or group uses into deprecated_mods.json. Their downloads stay on disk, but adding such a mod back downloads it again. Off: every cached mod is kept and refreshed.")
                            .changed()
                        {
                            config.save().unwrap();
                        }
                        ui.end_row();

                        ui.label("Mod providers:");
                        ui.end_row();

                        for provider_factory in ModStore::get_provider_factories() {
                            ui.label(provider_factory.id);
                            if ui.add_enabled(!provider_factory.parameters.is_empty(), egui::Button::new("⚙"))
                                    .on_hover_text(format!("Open \"{}\" settings", provider_factory.id))
                                    .clicked() {
                                self.window_provider_parameters = Some(
                                    WindowProviderParameters::new(provider_factory, &self.state),
                                );
                            }
                            ui.end_row();
                        }
                    });

                    ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                        if ui.add_enabled(window.drg_pak_path_err.is_none(), egui::Button::new("save")).clicked() {
                            try_save = true;
                        }
                        if let Some(error) = &window.drg_pak_path_err {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                    });

                });
            if try_save {
                let folder_text = window.local_mod_folder.trim();
                let folder_result: std::io::Result<Option<PathBuf>> = if folder_text.is_empty() {
                    Ok(None)
                } else {
                    std::fs::canonicalize(folder_text).and_then(|path| {
                        if path.is_dir() {
                            Ok(Some(path))
                        } else {
                            Err(std::io::Error::other("Local mods path is not a folder"))
                        }
                    })
                };
                if let Err(error) = &folder_result {
                    window.local_mod_folder_err = Some(error.to_string());
                } else {
                    if let Err(e) = is_drg_pak(&window.drg_pak_path) {
                        window.drg_pak_path_err = Some(e.to_string());
                    } else {
                        self.state.config.drg_pak_path = Some(PathBuf::from(
                            self.settings_window.take().unwrap().drg_pak_path,
                        ));
                        self.state.config.local_mod_folder = folder_result.ok().flatten();
                        rescan_folder = true;
                        self.state.config.save().unwrap();
                    }
                }
            } else if !open {
                self.settings_window = None;
            }
        }
        // Folder selection is independent of the DRG pak Save button.
        // Save immediately for valid paths (or clearing the field), and on Browse.
        if let Some((folder, committed)) = auto_save_local_folder {
            if folder.trim().is_empty() || PathBuf::from(folder.trim()).is_dir() {
                match folder_import::save_folder_setting(&mut self.state, &folder) {
                    Ok(changed) => {
                        if changed {
                            folder_import::reset_schedule(self);
                        }
                    }
                    Err(error) => {
                        if let Some(window) = &mut self.settings_window {
                            window.local_mod_folder_err = Some(error.clone());
                        }
                        self.last_action = Some(LastAction::failure(error));
                    }
                }
            } else if committed && let Some(window) = &mut self.settings_window {
                window.local_mod_folder_err = Some(format!("Not a folder: {}", folder.trim()));
            }
        }
        if rescan_folder {
            folder_import::reset_schedule(self);
        }
    }
}
