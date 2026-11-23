//! Lint configuration and report windows. The rest of App remains in gui/mod.rs.
use super::super::*;

impl App {
    pub(in crate::gui) fn enabled_lint_specs(&self) -> Vec<ModSpecification> {
        let mut seen = BTreeSet::new();
        let mut specs = Vec::new();
        self.state
            .mod_data
            .for_each_enabled_mod(&self.state.mod_data.active_profile, |mc| {
                if seen.insert(mc.spec.clone()) {
                    specs.push(mc.spec.clone());
                }
            });
        specs
    }

    pub(in crate::gui) fn show_lints_toggle(&mut self, ctx: &egui::Context) {
        let available_mods = self.enabled_lint_specs();
        if let Some(_lints_toggle) = &self.lints_toggle_window {
            let mut open = true;

            egui::Window::new("Lint mods")
                .open(&mut open)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.label("What should be scanned?");
                        ui.horizontal_wrapped(|ui| {
                            ui.selectable_value(
                                &mut self.lint_scope,
                                super::LintScope::All,
                                "All enabled",
                            );
                            ui.selectable_value(
                                &mut self.lint_scope,
                                super::LintScope::Selected,
                                "Selected mods",
                            );
                            ui.selectable_value(
                                &mut self.lint_scope,
                                super::LintScope::Focus,
                                "Compare one mod",
                            );
                        });
                        match self.lint_scope {
                            super::LintScope::Selected => {
                                ui.horizontal(|ui| {
                                    if ui.small_button("Select all").clicked() {
                                        let filter = self.lint_filter.trim().to_lowercase();
                                        self.lint_selected_mods = available_mods
                                            .iter()
                                            .filter(|spec| {
                                                if filter.is_empty() {
                                                    return true;
                                                }
                                                let label = self
                                                    .state
                                                    .store
                                                    .get_mod_info(spec)
                                                    .map(|info| info.name)
                                                    .unwrap_or_else(|| spec.url.clone());
                                                label.to_lowercase().contains(&filter)
                                                    || spec.url.to_lowercase().contains(&filter)
                                            })
                                            .cloned()
                                            .collect();
                                    }
                                    if ui.small_button("Clear selection").clicked() {
                                        self.lint_selected_mods.clear();
                                    }
                                    ui.label(format!(
                                        "{} selected",
                                        available_mods
                                            .iter()
                                            .filter(|s| self.lint_selected_mods.contains(*s))
                                            .count()
                                    ));
                                });
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.lint_filter)
                                        .hint_text("Filter mod list..."),
                                );
                                egui::ScrollArea::vertical()
                                    .max_height(165.0)
                                    .show(ui, |ui| {
                                        for spec in &available_mods {
                                            let label = self
                                                .state
                                                .store
                                                .get_mod_info(spec)
                                                .map(|info| info.name)
                                                .unwrap_or_else(|| spec.url.clone());
                                            if !label
                                                .to_lowercase()
                                                .contains(&self.lint_filter.to_lowercase())
                                                && !spec
                                                    .url
                                                    .to_lowercase()
                                                    .contains(&self.lint_filter.to_lowercase())
                                            {
                                                continue;
                                            }
                                            let mut checked =
                                                self.lint_selected_mods.contains(spec);
                                            if ui
                                                .checkbox(&mut checked, label)
                                                .on_hover_text(&spec.url)
                                                .changed()
                                            {
                                                if checked {
                                                    self.lint_selected_mods.insert(spec.clone());
                                                } else {
                                                    self.lint_selected_mods.remove(spec);
                                                }
                                            }
                                        }
                                    });
                            }
                            super::LintScope::Focus => {
                                if self
                                    .lint_focus
                                    .as_ref()
                                    .is_none_or(|f| !available_mods.contains(f))
                                {
                                    self.lint_focus = available_mods.first().cloned();
                                }
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.lint_focus_filter)
                                            .hint_text("Search mod name or URL...")
                                            .desired_width(360.0),
                                    );
                                    if !self.lint_focus_filter.is_empty()
                                        && ui.small_button("Clear").clicked()
                                    {
                                        self.lint_focus_filter.clear();
                                    }
                                });
                                let filter = self.lint_focus_filter.trim().to_lowercase();
                                let selected_label = self
                                    .lint_focus
                                    .as_ref()
                                    .map(|spec| {
                                        self.state
                                            .store
                                            .get_mod_info(spec)
                                            .map(|info| info.name)
                                            .unwrap_or_else(|| spec.url.clone())
                                    })
                                    .unwrap_or_else(|| "No enabled mods".to_owned());

                                if filter.is_empty() {
                                    egui::ComboBox::from_id_salt("lint_focus_combo")
                                        .selected_text(selected_label)
                                        .width(ui.available_width().min(460.0))
                                        .show_ui(ui, |ui| {
                                            for spec in &available_mods {
                                                let label = self
                                                    .state
                                                    .store
                                                    .get_mod_info(spec)
                                                    .map(|info| info.name)
                                                    .unwrap_or_else(|| spec.url.clone());
                                                ui.selectable_value(
                                                    &mut self.lint_focus,
                                                    Some(spec.clone()),
                                                    label,
                                                )
                                                .on_hover_text(&spec.url);
                                            }
                                        });
                                } else {
                                    // Render matches inline: the regular ComboBox remains closed when typing.
                                    let matches: Vec<_> = available_mods
                                        .iter()
                                        .filter(|spec| {
                                            let label = self
                                                .state
                                                .store
                                                .get_mod_info(spec)
                                                .map(|info| info.name)
                                                .unwrap_or_else(|| spec.url.clone());
                                            label.to_lowercase().contains(&filter)
                                                || spec.url.to_lowercase().contains(&filter)
                                        })
                                        .collect();
                                    ui.small(format!(
                                        "{} matching mods - click one to select",
                                        matches.len()
                                    ));
                                    egui::ScrollArea::vertical()
                                        .id_salt("lint_focus_live_results")
                                        .max_height(165.0)
                                        .show(ui, |ui| {
                                            if matches.is_empty() {
                                                ui.label("No matching enabled mods");
                                            }
                                            for spec in matches {
                                                let label = self
                                                    .state
                                                    .store
                                                    .get_mod_info(spec)
                                                    .map(|info| info.name)
                                                    .unwrap_or_else(|| spec.url.clone());
                                                let selected =
                                                    self.lint_focus.as_ref() == Some(spec);
                                                if ui
                                                    .selectable_label(selected, label)
                                                    .on_hover_text(&spec.url)
                                                    .clicked()
                                                {
                                                    self.lint_focus = Some(spec.clone());
                                                    self.lint_focus_filter.clear();
                                                }
                                            }
                                        });
                                    ui.small(format!("Selected: {selected_label}"));
                                }
                                ui.small(
                                    "Compares shared asset paths against every other enabled mod.",
                                );
                            }
                            super::LintScope::All => {
                                ui.small(format!(
                                    "Scanning {} enabled mods in this profile",
                                    available_mods.len()
                                ));
                            }
                        }
                        ui.separator();
                        ui.label("Checks");

                        egui::Grid::new("lints-toggle-grid").show(ui, |ui| {
                            ui.heading("Lint");
                            ui.heading("Enabled?");
                            ui.end_row();

                            ui.label("Archive with multiple paks");
                            ui.add(toggle_switch(
                                &mut self.lint_options.archive_with_multiple_paks,
                            ));
                            ui.end_row();

                            ui.label("Archive with only non-pak files");
                            ui.add(toggle_switch(
                                &mut self.lint_options.archive_with_only_non_pak_files,
                            ));
                            ui.end_row();

                            ui.label("Mods containing AssetRegister.bin");
                            ui.add(toggle_switch(&mut self.lint_options.asset_register_bin));
                            ui.end_row();

                            ui.label("Mods containing conflicting files");
                            ui.add(toggle_switch(&mut self.lint_options.conflicting));
                            ui.end_row();

                            ui.label("Mods containing empty archives");
                            ui.add(toggle_switch(&mut self.lint_options.empty_archive));
                            ui.end_row();

                            ui.label("Mods containing oudated pak version");
                            ui.add(toggle_switch(&mut self.lint_options.outdated_pak_version));
                            ui.end_row();

                            ui.label("Mods containing shader files");
                            ui.add(toggle_switch(&mut self.lint_options.shader_files));
                            ui.end_row();

                            ui.label("Mods containing non-asset files");
                            ui.add(toggle_switch(&mut self.lint_options.non_asset_files));
                            ui.end_row();

                            ui.label("Mods containing split {uexp, uasset} pairs");
                            ui.add(toggle_switch(&mut self.lint_options.split_asset_pairs));
                            ui.end_row();

                            ui.label("Mods containing unmodified game assets");
                            ui.add_enabled(
                                self.state.config.drg_pak_path.is_some(),
                                toggle_switch(&mut self.lint_options.unmodified_game_assets),
                            )
                            .on_disabled_hover_text(
                                "This lint requires DRG pak path to be specified",
                            );
                            ui.end_row();
                        });
                    });

                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.lints_toggle_window = None;
                        }

                        let valid_scope = super::choose_mods(
                            &available_mods,
                            self.lint_scope,
                            &self.lint_selected_mods,
                            self.lint_focus.as_ref(),
                        );
                        let can_run = self.check_updates_rid.is_none()
                            && self.integrate_rid.is_none()
                            && self.lint_rid.is_none()
                            && valid_scope.is_ok();
                        if let Err(reason) = &valid_scope {
                            ui.small(*reason);
                        }
                        if ui
                            .add_enabled(can_run, egui::Button::new("Generate report"))
                            .clicked()
                        {
                            let lint_options = BTreeMap::from([
                                (
                                    LintId::ARCHIVE_WITH_MULTIPLE_PAKS,
                                    self.lint_options.archive_with_multiple_paks,
                                ),
                                (
                                    LintId::ARCHIVE_WITH_ONLY_NON_PAK_FILES,
                                    self.lint_options.archive_with_only_non_pak_files,
                                ),
                                (
                                    LintId::ASSET_REGISTRY_BIN,
                                    self.lint_options.asset_register_bin,
                                ),
                                (LintId::CONFLICTING, self.lint_options.conflicting),
                                (LintId::EMPTY_ARCHIVE, self.lint_options.empty_archive),
                                (
                                    LintId::OUTDATED_PAK_VERSION,
                                    self.lint_options.outdated_pak_version,
                                ),
                                (LintId::SHADER_FILES, self.lint_options.shader_files),
                                (LintId::NON_ASSET_FILES, self.lint_options.non_asset_files),
                                (
                                    LintId::SPLIT_ASSET_PAIRS,
                                    self.lint_options.split_asset_pairs,
                                ),
                                (
                                    LintId::UNMODIFIED_GAME_ASSETS,
                                    self.lint_options.unmodified_game_assets,
                                ),
                            ]);

                            trace!(?lint_options);

                            let (mods, focus) =
                                valid_scope.expect("disabled button prevents an invalid scope");
                            let enabled_lints: BTreeSet<_> = lint_options
                                .into_iter()
                                .filter_map(|(lint, enabled)| enabled.then_some(lint))
                                .collect();
                            self.lint_run = Some(super::LintRun {
                                profile: self.state.mod_data.active_profile.clone(),
                                scope: self.lint_scope,
                                mods: mods.clone(),
                                focus,
                                checks: enabled_lints.iter().map(|id| id.to_name_lower()).collect(),
                            });
                            self.lint_report = None;
                            self.lint_report_path = None;
                            self.lint_report_error = None;
                            self.lint_rid = Some(message::LintMods::send(
                                &mut self.request_counter,
                                self.state.store.clone(),
                                mods,
                                enabled_lints,
                                self.state.config.drg_pak_path.clone(),
                                self.tx.clone(),
                                ctx.clone(),
                            ));
                            self.problematic_mod_id = None;
                            self.lints_toggle_window = None;
                            self.lint_report_window = Some(WindowLintReport);
                        }
                    });
                });

            if !open {
                self.lints_toggle_window = None;
            }
        }
    }

    pub(in crate::gui) fn show_lint_report(&mut self, ctx: &egui::Context) {
        if self.lint_report_window.is_some() {
            let mut open = true;

            egui::Window::new("Lint results")
                .open(&mut open)
                .resizable(true)
                .show(ctx, |ui| {
                    if let Some(path) = self.lint_report_path.clone() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!("TXT saved: {}", path.display()));
                            if ui.button("Open TXT").clicked()
                                && let Err(error) = opener::open(&path) {
                                    self.last_action = Some(LastAction::failure(format!("Could not open lint report: {error}")));
                                }
                            if ui.button("Copy path").clicked() {
                                ui.ctx().copy_text(path.to_string_lossy().to_string());
                            }
                        });
                    }
                    if let Some(error) = &self.lint_report_error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                    if let Some(report) = &self.lint_report {
                        let scroll_height =
                            (ui.available_height() - 30.0).clamp(0.0, f32::INFINITY);
                        egui::ScrollArea::vertical()
                            .max_height(scroll_height)
                            .show(ui, |ui| {
                                const AMBER: Color32 = Color32::from_rgb(255, 191, 0);

                                if let Some(conflicting_mods) = &report.conflicting_mods
                                    && !conflicting_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new("⚠ Mods(s) with conflicting asset modifications detected")
                                                .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            conflicting_mods.iter().for_each(|(path, mods)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ Conflicting modification of asset `{path}`"
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(
                                                    ui,
                                                    |ui| {
                                                        mods.iter().for_each(|mod_spec| {
                                                            ui.label(&mod_spec.url);
                                                        });
                                                    },
                                                );
                                            });
                                        });
                                    }

                                if let Some(asset_register_bin_mods) = &report.asset_register_bin_mods
                                    && !asset_register_bin_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new("ℹ Mod(s) with `AssetRegistry.bin` included detected")
                                                .color(Color32::LIGHT_BLUE),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            asset_register_bin_mods.iter().for_each(
                                                |(r#mod, paths)| {
                                                    CollapsingHeader::new(
                                                        RichText::new(format!(
                                                        "ℹ {} includes one or more `AssetRegistry.bin`",
                                                        r#mod.url
                                                    ))
                                                        .color(Color32::LIGHT_BLUE),
                                                    )
                                                    .show(ui, |ui| {
                                                        paths.iter().for_each(|path| {
                                                            ui.label(path);
                                                        });
                                                    });
                                                },
                                            );
                                        });
                                    }

                                if let Some(shader_file_mods) = &report.shader_file_mods
                                    && !shader_file_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mods(s) with shader files included detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            shader_file_mods.iter().for_each(
                                                |(r#mod, shader_files)| {
                                                    CollapsingHeader::new(
                                                        RichText::new(format!(
                                                            "⚠ {} includes one or more shader files",
                                                            r#mod.url
                                                        ))
                                                        .color(AMBER),
                                                    )
                                                    .show(ui, |ui| {
                                                        shader_files.iter().for_each(|shader_file| {
                                                            ui.label(shader_file);
                                                        });
                                                    });
                                                },
                                            );
                                        });
                                    }

                                if let Some(outdated_pak_version_mods) = &report.outdated_pak_version_mods
                                    && !outdated_pak_version_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with outdated pak version detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            outdated_pak_version_mods.iter().for_each(
                                                |(r#mod, version)| {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "⚠ {} includes outdated pak version {}",
                                                            r#mod.url, version
                                                        ))
                                                        .color(AMBER),
                                                    );
                                                },
                                            );
                                        });
                                    }

                                if let Some(empty_archive_mods) = &report.empty_archive_mods
                                    && !empty_archive_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with empty archives detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            empty_archive_mods.iter().for_each(|r#mod| {
                                                ui.label(
                                                    RichText::new(format!(
                                                        "⚠ {} contains an empty archive",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                );
                                            });
                                        });
                                    }

                                if let Some(archive_with_only_non_pak_files_mods) = &report.archive_with_only_non_pak_files_mods
                                    && !archive_with_only_non_pak_files_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with only non-`.pak` files detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            archive_with_only_non_pak_files_mods.iter().for_each(|r#mod| {
                                                ui.label(
                                                    RichText::new(format!(
                                                        "⚠ {} contains only non-`.pak` files, perhaps the author forgot to pack it?",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                );
                                            });
                                        });
                                    }

                                if let Some(archive_with_multiple_paks_mods) = &report.archive_with_multiple_paks_mods
                                    && !archive_with_multiple_paks_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with multiple `.pak`s detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            archive_with_multiple_paks_mods.iter().for_each(|r#mod| {
                                                ui.label(RichText::new(format!(
                                                    "⚠ {} contains multiple `.pak`s, only the first encountered `.pak` will be loaded",
                                                    r#mod.url
                                                ))
                                                .color(AMBER));
                                            });
                                        });
                                    }

                                if let Some(non_asset_file_mods) = &report.non_asset_file_mods
                                    && !non_asset_file_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with non-asset files detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            non_asset_file_mods.iter().for_each(|(r#mod, files)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ {} includes non-asset files",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(ui, |ui| {
                                                    files.iter().for_each(|file| {
                                                        ui.label(file);
                                                    });
                                                });
                                            });
                                        });
                                    }

                                if let Some(split_asset_pairs_mods) = &report.split_asset_pairs_mods
                                    && !split_asset_pairs_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with split {uexp, uasset} pairs detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            split_asset_pairs_mods.iter().for_each(|(r#mod, files)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ {} includes split {{uexp, uasset}} pairs",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(ui, |ui| {
                                                    files.iter().for_each(|(file, kind)| {
                                                        match kind {
                                                            SplitAssetPair::MissingUasset => {
                                                                ui.label(format!("`{file}` missing matching .uasset file"));
                                                            },
                                                            SplitAssetPair::MissingUexp => {
                                                                ui.label(format!("`{file}` missing matching .uexp file"));
                                                            }
                                                        }
                                                    });
                                                });
                                            });
                                        });
                                    }

                                if let Some(unmodified_game_assets_mods) = &report.unmodified_game_assets_mods
                                    && !unmodified_game_assets_mods.is_empty() {
                                        CollapsingHeader::new(
                                            RichText::new(
                                                "⚠ Mod(s) with unmodified game assets detected",
                                            )
                                            .color(AMBER),
                                        )
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            unmodified_game_assets_mods.iter().for_each(|(r#mod, files)| {
                                                CollapsingHeader::new(
                                                    RichText::new(format!(
                                                        "⚠ {} includes unmodified game assets",
                                                        r#mod.url
                                                    ))
                                                    .color(AMBER),
                                                )
                                                .show(ui, |ui| {
                                                    files.iter().for_each(|file| {
                                                        ui.label(file);
                                                    });
                                                });
                                            });
                                        });
                                    }
                            });
                                        } else if self.lint_rid.is_some() {
                        ui.spinner();
                        ui.label("Lint report generating...");
                    } else {
                        ui.label("Lint failed. See the bottom status message for details.");
                    }
                });
            if !open {
                self.lint_report_window = None;
            }
        }
    }
}
