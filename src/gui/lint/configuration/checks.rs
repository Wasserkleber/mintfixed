//! Lint check toggles and game-pak prerequisite.
use super::super::*;

impl App {
    pub(super) fn show_lint_checks(&mut self, ui: &mut egui::Ui) {
        ui.label("Checks");

        egui::Grid::new("lints-toggle-grid").show(ui, |ui| {
            ui.heading("Lint");
            ui.heading("Enabled?");
            ui.end_row();

            ui.label("Archive with multiple paks");
            ui.add(toggle_switch(
                &mut self.lint_state.lint_options.archive_with_multiple_paks,
            ));
            ui.end_row();

            ui.label("Archive with only non-pak files");
            ui.add(toggle_switch(
                &mut self.lint_state.lint_options.archive_with_only_non_pak_files,
            ));
            ui.end_row();

            ui.label("Mods containing AssetRegister.bin");
            ui.add(toggle_switch(&mut self.lint_state.lint_options.asset_register_bin));
            ui.end_row();

            ui.label("Mods containing conflicting files");
            ui.add(toggle_switch(&mut self.lint_state.lint_options.conflicting));
            ui.end_row();

            ui.label("Mods containing empty archives");
            ui.add(toggle_switch(&mut self.lint_state.lint_options.empty_archive));
            ui.end_row();

            ui.label("Mods containing oudated pak version");
            ui.add(toggle_switch(&mut self.lint_state.lint_options.outdated_pak_version));
            ui.end_row();

            ui.label("Mods containing shader files");
            ui.add(toggle_switch(&mut self.lint_state.lint_options.shader_files));
            ui.end_row();

            ui.label("Mods containing non-asset files");
            ui.add(toggle_switch(&mut self.lint_state.lint_options.non_asset_files));
            ui.end_row();

            ui.label("Mods containing split {uexp, uasset} pairs");
            ui.add(toggle_switch(&mut self.lint_state.lint_options.split_asset_pairs));
            ui.end_row();

            ui.label("Mods containing unmodified game assets");
            ui.add_enabled(
                self.state.config.drg_pak_path.is_some(),
                toggle_switch(&mut self.lint_state.lint_options.unmodified_game_assets),
            )
            .on_disabled_hover_text(
                "This lint requires DRG pak path to be specified",
            );
            ui.end_row();
        });
    }
}
