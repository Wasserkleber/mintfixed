// settings and provider configuration windows; methods moved intact from gui/mod.rs.
use crate::gui::*;

impl App {
    pub(in crate::gui) fn show_provider_parameters(&mut self, ctx: &egui::Context) {
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
                            // Validate and save when the user finishes editing (Enter, Tab,
                            // or clicking elsewhere); never persist an unchecked token.
                            if res.lost_focus()
                                && !window.parameters.values().all(|value| value.is_empty())
                                && self.state.config.provider_parameters.get(window.factory.id)
                                    != Some(&window.parameters)
                            {
                                check = true;
                            }
                            ui.end_row();
                        }
                    });

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width().max(0.0), ui.spacing().interact_size.y),
                        Layout::right_to_left(Align::Center),
                        |ui| {
                            if ui.add(crate::gui::mint_text_button("Save")).clicked() {
                                check = true;
                            }
                            if window.check_rid.is_some() {
                                ui.spinner();
                            }
                            if let Some(error) = &window.check_error {
                                ui.colored_label(ui.visuals().error_fg_color, error);
                            }
                        },
                    );
                });
            });
        if check {
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
        } else if !open {
            self.window_provider_parameters = None;
        }
    }

    pub(in crate::gui) fn show_settings(&mut self, ctx: &egui::Context) {
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
                            if res.lost_focus() {
                                try_save = true;
                            }
                            if ui.add(crate::gui::mint_text_button("browse")).clicked()
                                && let Some(fsd_pak) = rfd::FileDialog::new()
                                    .add_filter("DRG Pak", &["pak"])
                                    .pick_file()
                                {
                                    window.drg_pak_path = fsd_pak.to_string_lossy().to_string();
                                    window.drg_pak_path_err = None;
                                try_save = true;
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
                            if ui.add(crate::gui::mint_text_button("browse")).clicked()
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

                        // settings_modio_config_row_v7: keep the existing token dialog,
                        // but expose only the provider users need to configure.
                        ui.label("mod.io:");
                        if let Some(provider_factory) = ModStore::get_provider_factories()
                            .find(|factory| factory.id == "modio")
                        {
                            if ui
                                .add_enabled(
                                    !provider_factory.parameters.is_empty(),
                                    egui::Button::new("⚙"),
                                )
                                .on_hover_text("Configure mod.io access token")
                                .clicked()
                            {
                                self.window_provider_parameters = Some(
                                    WindowProviderParameters::new(provider_factory, &self.state),
                                );
                            }
                        } else {
                            ui.label("mod.io provider unavailable");
                        }
                        ui.end_row();

                        let config_dir = &self.state.dirs.config_dir;
                        let cache_dir = &self.state.dirs.cache_dir;
                        let data_dir = &self.state.dirs.data_dir;
                        // Determine Steam/WinGDK paths from the configured game pak.
                        let installation = self
                            .state
                            .config
                            .drg_pak_path
                            .as_deref()
                            .and_then(|pak| mint_lib::DRGInstallation::from_pak_path(pak).ok());

                        ui.label("Logs & diagnostics:");
                        let logs_button = ui.add(crate::gui::mint_text_button("Open diagnostics"));
                        let logs_popup_id = ui.make_persistent_id("settings_logs_popup");
                        if logs_button.clicked() {
                            ui.memory_mut(|memory| memory.toggle_popup(logs_popup_id));
                        }
                        egui::popup::popup_below_widget(
                            ui,
                            logs_popup_id,
                            &logs_button,
                            egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                            |ui| {
                                egui::Grid::new("settings_log_shortcuts_grid")
                                    .num_columns(2)
                                    .show(ui, |ui| {
                                        path_menu_entry(ui, "mint.log", &data_dir.join("mint.log"), false);
                                        if let Some(installation) = &installation {
                                            let bin = installation.binaries_directory();
                                            let logs = installation.root.join("Saved").join("Logs");
                                            let reporter = bin.join("mintfixed-bpcrash");
                                            path_menu_entry(ui, "FSD.log", &logs.join("FSD.log"), false);
                                            path_menu_entry(ui, "mint_hook.log", &bin.join("mint_hook.log"), false);
                                            path_menu_entry(ui, "bpcrash-loader.log", &reporter.join("bpcrash-loader.log"), false);
                                            path_menu_entry(ui, "bpcrash_status.txt", &reporter.join("bpcrash_status.txt"), false);
                                            path_menu_entry(ui, "crash_context.json", &reporter.join("crash_context.json"), false);
                                        }
                                        path_menu_entry(ui, "mod_data.json", &config_dir.join("mod_data.json"), false);
                                        path_menu_entry(ui, "cache.json", &cache_dir.join("cache.json"), false);
                                        path_menu_entry(ui, "deprecated_mods.json", &cache_dir.join("deprecated_mods.json"), false);
                                        // settings_lint_exports_v8: most recent completed lint run across profiles.
                                        if let Some(report) = latest_lint_report(&data_dir.join("lint-reports")) {
                                            path_menu_entry(ui, "lint_*.txt", &report, false);
                                        } else {
                                            ui.add_enabled(false, crate::gui::mint_text_button("lint_*.txt"))
                                                .on_disabled_hover_text("No saved lint reports yet");
                                            ui.end_row();
                                        }
                                        if installation.is_none() {
                                            ui.label("Configure a DRG pak to show game files");
                                            ui.end_row();
                                        }
                                    });
                            },
                        );
                        ui.end_row();

                        ui.label("Directories:");
                        let directories_button = ui.add(crate::gui::mint_text_button("Open directories"));
                        let directories_popup_id = ui.make_persistent_id("settings_directories_popup");
                        if directories_button.clicked() {
                            ui.memory_mut(|memory| memory.toggle_popup(directories_popup_id));
                        }
                        egui::popup::popup_below_widget(
                            ui,
                            directories_popup_id,
                            &directories_button,
                            egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                            |ui| {
                                egui::Grid::new("settings_directory_shortcuts_grid")
                                    .num_columns(2)
                                    .show(ui, |ui| {
                                        path_menu_entry(ui, "Config", config_dir, true);
                                        path_menu_entry(ui, "Cache", cache_dir, true);
                                        path_menu_entry(ui, "Data", data_dir, true);
                                        path_menu_entry(ui, "Lint reports", &data_dir.join("lint-reports"), true);
                                        // Group/profile exports are saved next to the running executable.
                                        if let Some(exports_dir) = std::env::current_exe()
                                            .ok()
                                            .and_then(|exe| exe.parent().map(|parent| parent.join("mintfixed exports")))
                                            .filter(|directory| directory.is_dir())
                                        {
                                            path_menu_entry(ui, "Exports", &exports_dir, true);
                                        }

                                        if let Some(installation) = &installation {
                                            let reporter = installation.binaries_directory().join("mintfixed-bpcrash");
                                            let logs = installation.root.join("Saved").join("Logs");
                                            path_menu_entry(ui, "DRG logs", &logs, true);
                                            path_menu_entry(ui, "Crash reports", &reporter, true);
                                        }
                                    });
                            },
                        );
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

                    });

                    ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {

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
                        self.state.config.drg_pak_path =
                            Some(PathBuf::from(window.drg_pak_path.clone()));
                        self.state.config.local_mod_folder = folder_result.ok().flatten();
                        rescan_folder = true;
                        self.state.config.save().unwrap();
                    }
                }
            }
            if !open {
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
                        self.status_state.last_action = Some(LastAction::failure(error));
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

// Open only existing log files. Paths can be copied even for absent files.
fn path_menu_entry(ui: &mut egui::Ui, label: &str, path: &std::path::Path, directory: bool) {
    let available = if directory {
        path.is_dir()
    } else {
        path.is_file()
    };
    let path_text = path.display().to_string();
    let response = ui
        .add_enabled(available, egui::Button::new(label))
        .on_hover_text(path_text.as_str())
        .on_disabled_hover_text(format!("Not available: {path_text}"));

    if response.clicked() {
        let result = if directory {
            opener::open(path).map_err(|error| std::io::Error::other(error.to_string()))
        } else {
            open_log_file(path)
        };
        if let Err(error) = result {
            tracing::warn!("Could not open {}: {error}", path.display());
        }
    }

    if ui
        .add(crate::gui::mint_text_button("Copy path"))
        .on_hover_text(path_text.as_str())
        .clicked()
    {
        ui.ctx().copy_text(path_text);
    }
    ui.end_row();
}

fn open_log_file(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        // .log, .txt, and .json do not always have working associations.
        std::process::Command::new("notepad.exe")
            .arg(path)
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(target_os = "windows"))]
    {
        opener::open(path).map_err(|error| std::io::Error::other(error.to_string()))
    }
}

// Find the most recently *created* lint report, regardless of profile name.
// Filenames are lint_<profile>_<unix_milliseconds>_<counter>.txt.
fn latest_lint_report(directory: &std::path::Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if !entry.file_type().ok()?.is_file() {
                return None;
            }
            let path = entry.path();
            let filename = path.file_name()?.to_str()?;
            let trimmed = filename.strip_prefix("lint_")?.strip_suffix(".txt")?;
            let (prefix, index) = trimmed.rsplit_once('_')?;
            let (_, timestamp) = prefix.rsplit_once('_')?;
            Some((
                (
                    timestamp.parse::<u128>().ok()?,
                    index.parse::<usize>().ok()?,
                ),
                path,
            ))
        })
        .max_by_key(|(sort_key, _)| *sort_key)
        .map(|(_, path)| path)
}
