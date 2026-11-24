// mod.io approval, requirement, and category badges displayed on mod rows.
use crate::gui::*;

pub(in crate::gui) fn render_mod_tags(
    ui: &mut Ui,
    info: &ModInfo,
    search_string: &str,
    scroll_to_match: &mut bool,
) {
    if let Some(ModioTags {
        qol,
        gameplay,
        audio,
        visual,
        framework,
        required_status,
        approval_status,
        versions: _,
    }) = info.modio_tags.as_ref()
    {
        let mut mk_searchable_modio_tag =
            |tag_str: &str, ui: &mut Ui, color: Option<egui::Color32>, hover_str: Option<&str>| {
                let search = searchable_text(tag_str, search_string, {
                    TextFormat {
                        color: if color.is_some() {
                            Color32::BLACK
                        } else {
                            Color32::GRAY
                        },

                        ..Default::default()
                    }
                });

                let button = if let Some(color) = color {
                    egui::Button::new(search.job)
                        .small()
                        .fill(color)
                        .stroke(egui::Stroke::NONE)
                } else {
                    egui::Button::new(search.job)
                        .small()
                        .stroke(egui::Stroke::NONE)
                };

                let res = if let Some(hover_str) = hover_str {
                    ui.add_enabled(false, button)
                        .on_disabled_hover_text(hover_str)
                } else {
                    ui.add_enabled(false, button)
                };

                if search.is_match && *scroll_to_match {
                    res.scroll_to_me(None);
                    *scroll_to_match = false;
                }
            };

        match approval_status {
            ApprovalStatus::Verified => {
                mk_searchable_modio_tag(
                    "Verified",
                    ui,
                    Some(egui::Color32::LIGHT_GREEN),
                    Some("Does not contain any gameplay affecting features or changes"),
                );
            }
            ApprovalStatus::Approved => {
                mk_searchable_modio_tag(
                    "Approved",
                    ui,
                    Some(egui::Color32::LIGHT_BLUE),
                    Some("Contains gameplay affecting features or changes"),
                );
            }
            ApprovalStatus::Sandbox => {
                mk_searchable_modio_tag(
                    "Sandbox",
                    ui,
                    Some(egui::Color32::LIGHT_YELLOW),
                    Some(
                        "Contains significant, possibly progression breaking, changes to gameplay",
                    ),
                );
            }
        }

        match required_status {
            RequiredStatus::RequiredByAll => {
                mk_searchable_modio_tag(
                    "RequiredByAll",
                    ui,
                    Some(egui::Color32::LIGHT_RED),
                    Some("All lobby members must use this mod for it to work correctly!"),
                );
            }
            RequiredStatus::Optional => {
                mk_searchable_modio_tag(
                    "Optional",
                    ui,
                    None,
                    Some("Clients are not required to install this mod to function"),
                );
            }
        }

        if *qol {
            mk_searchable_modio_tag("QoL", ui, None, None);
        }
        if *gameplay {
            mk_searchable_modio_tag("Gameplay", ui, None, None);
        }
        if *audio {
            mk_searchable_modio_tag("Audio", ui, None, None);
        }
        if *visual {
            mk_searchable_modio_tag("Visual", ui, None, None);
        }
        if *framework {
            mk_searchable_modio_tag("Framework", ui, None, None);
        }
    }
}
