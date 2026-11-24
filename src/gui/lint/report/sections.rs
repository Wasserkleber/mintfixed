//! Render each category of lint findings.
use super::super::*;

pub(super) fn show(ui: &mut egui::Ui, report: &LintReport) {
    const AMBER: Color32 = Color32::from_rgb(255, 191, 0);

    if let Some(conflicting_mods) = &report.conflicting_mods
        && !conflicting_mods.is_empty()
    {
        CollapsingHeader::new(
            RichText::new("⚠ Mods(s) with conflicting asset modifications detected").color(AMBER),
        )
        .default_open(true)
        .show(ui, |ui| {
            conflicting_mods.iter().for_each(|(path, mods)| {
                CollapsingHeader::new(
                    RichText::new(format!("⚠ Conflicting modification of asset `{path}`"))
                        .color(AMBER),
                )
                .show(ui, |ui| {
                    mods.iter().for_each(|mod_spec| {
                        ui.label(&mod_spec.url);
                    });
                });
            });
        });
    }

    if let Some(asset_register_bin_mods) = &report.asset_register_bin_mods
        && !asset_register_bin_mods.is_empty()
    {
        CollapsingHeader::new(
            RichText::new("ℹ Mod(s) with `AssetRegistry.bin` included detected")
                .color(Color32::LIGHT_BLUE),
        )
        .default_open(true)
        .show(ui, |ui| {
            asset_register_bin_mods.iter().for_each(|(r#mod, paths)| {
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
            });
        });
    }

    if let Some(shader_file_mods) = &report.shader_file_mods
        && !shader_file_mods.is_empty()
    {
        CollapsingHeader::new(
            RichText::new("⚠ Mods(s) with shader files included detected").color(AMBER),
        )
        .default_open(true)
        .show(ui, |ui| {
            shader_file_mods.iter().for_each(|(r#mod, shader_files)| {
                CollapsingHeader::new(
                    RichText::new(format!("⚠ {} includes one or more shader files", r#mod.url))
                        .color(AMBER),
                )
                .show(ui, |ui| {
                    shader_files.iter().for_each(|shader_file| {
                        ui.label(shader_file);
                    });
                });
            });
        });
    }

    if let Some(outdated_pak_version_mods) = &report.outdated_pak_version_mods
        && !outdated_pak_version_mods.is_empty()
    {
        CollapsingHeader::new(
            RichText::new("⚠ Mod(s) with outdated pak version detected").color(AMBER),
        )
        .default_open(true)
        .show(ui, |ui| {
            outdated_pak_version_mods
                .iter()
                .for_each(|(r#mod, version)| {
                    ui.label(
                        RichText::new(format!(
                            "⚠ {} includes outdated pak version {}",
                            r#mod.url, version
                        ))
                        .color(AMBER),
                    );
                });
        });
    }

    if let Some(empty_archive_mods) = &report.empty_archive_mods
        && !empty_archive_mods.is_empty()
    {
        CollapsingHeader::new(RichText::new("⚠ Mod(s) with empty archives detected").color(AMBER))
            .default_open(true)
            .show(ui, |ui| {
                empty_archive_mods.iter().for_each(|r#mod| {
                    ui.label(
                        RichText::new(format!("⚠ {} contains an empty archive", r#mod.url))
                            .color(AMBER),
                    );
                });
            });
    }

    if let Some(archive_with_only_non_pak_files_mods) = &report.archive_with_only_non_pak_files_mods
        && !archive_with_only_non_pak_files_mods.is_empty()
    {
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
        && !archive_with_multiple_paks_mods.is_empty()
    {
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
        && !non_asset_file_mods.is_empty()
    {
        CollapsingHeader::new(RichText::new("⚠ Mod(s) with non-asset files detected").color(AMBER))
            .default_open(true)
            .show(ui, |ui| {
                non_asset_file_mods.iter().for_each(|(r#mod, files)| {
                    CollapsingHeader::new(
                        RichText::new(format!("⚠ {} includes non-asset files", r#mod.url))
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
        && !split_asset_pairs_mods.is_empty()
    {
        CollapsingHeader::new(
            RichText::new("⚠ Mod(s) with split {uexp, uasset} pairs detected").color(AMBER),
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
                    files.iter().for_each(|(file, kind)| match kind {
                        SplitAssetPair::MissingUasset => {
                            ui.label(format!("`{file}` missing matching .uasset file"));
                        }
                        SplitAssetPair::MissingUexp => {
                            ui.label(format!("`{file}` missing matching .uexp file"));
                        }
                    });
                });
            });
        });
    }

    if let Some(unmodified_game_assets_mods) = &report.unmodified_game_assets_mods
        && !unmodified_game_assets_mods.is_empty()
    {
        CollapsingHeader::new(
            RichText::new("⚠ Mod(s) with unmodified game assets detected").color(AMBER),
        )
        .default_open(true)
        .show(ui, |ui| {
            unmodified_game_assets_mods
                .iter()
                .for_each(|(r#mod, files)| {
                    CollapsingHeader::new(
                        RichText::new(format!("⚠ {} includes unmodified game assets", r#mod.url))
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
}
