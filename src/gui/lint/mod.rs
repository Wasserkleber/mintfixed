//! Lint UI and scan orchestration. Low-level lint checks live in crate::mod_lints.
mod report;
#[cfg(test)]
mod tests;
mod ui;
mod workflow;

pub(super) use report::save_txt;
pub(super) use workflow::{LintRun, LintScope, choose_mods, focus_only};

pub(super) struct LintOptions {
    archive_with_multiple_paks: bool,
    archive_with_only_non_pak_files: bool,
    asset_register_bin: bool,
    conflicting: bool,
    empty_archive: bool,
    outdated_pak_version: bool,
    shader_files: bool,
    non_asset_files: bool,
    split_asset_pairs: bool,
    unmodified_game_assets: bool,
}

impl Default for LintOptions {
    fn default() -> Self {
        Self {
            archive_with_multiple_paks: true,
            archive_with_only_non_pak_files: true,
            asset_register_bin: true,
            conflicting: true,
            empty_archive: true,
            outdated_pak_version: true,
            shader_files: true,
            non_asset_files: true,
            split_asset_pairs: true,
            // Potentially costly: opt in when a game pak has been configured.
            unmodified_game_assets: false,
        }
    }
}
