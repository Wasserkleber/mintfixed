//! Lint configuration window and scan submission.
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
                        self.show_lint_scope(ui, &available_mods);
                        ui.separator();
                        self.show_lint_checks(ui);
                    });

                    self.show_lint_actions(ui, ctx, &available_mods);
                });

            if !open {
                self.lints_toggle_window = None;
            }
        }
    }

    fn show_lint_actions(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        available_mods: &[ModSpecification],
    ) {
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                self.lints_toggle_window = None;
            }

            let valid_scope = super::choose_mods(
                available_mods,
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
    }
}
