// Shared group-header presentation for the profile mod list.
// Keep data changes in profile_ui.rs; these helpers only render the header.
use crate::gui::*;

pub(in crate::gui) fn title(
    ui: &Ui,
    group_name: &str,
    group_header_color: Option<Color32>,
    uncolored: bool,
    collapsed_match: bool,
) -> RichText {
    // Highlight matching members when their containing group is collapsed.
    let group_title = if collapsed_match {
        RichText::new(group_name)
            .strong()
            .background_color(Color32::YELLOW)
            .color(Color32::BLACK)
    } else {
        let mut title = RichText::new(group_name).strong();
        if let Some(color) = group_header_color {
            title = title.color(color);
        }
        title
    };

    // Neutral group titles are deliberately softer than the default white.
    if uncolored && !collapsed_match {
        group_title.color(if ui.visuals().dark_mode {
            Color32::from_gray(164)
        } else {
            Color32::from_gray(92)
        })
    } else {
        group_title
    }
}

// A double Option distinguishes "no selection" from the "None" color option.
pub(in crate::gui) fn render_contents(
    ui: &mut Ui,
    group_title: RichText,
    group_member_count: usize,
    unavailable_group_tooltip: Option<&str>,
    selected_color: Option<&str>,
    group_description: Option<&str>,
) -> (bool, Option<Option<String>>, bool) {
    let title_clicked = ui
        .add(egui::Label::new(group_title).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked();
    ui.label(
        RichText::new(format!(
            "{} mod{}",
            group_member_count,
            if group_member_count == 1 { "" } else { "s" }
        ))
        .small()
        .weak(),
    );
    if let Some(tooltip) = unavailable_group_tooltip {
        icon_buttons::mod_error_badge(ui).on_hover_text_at_pointer(tooltip);
    }

    let mut requested_color: Option<Option<String>> = None;
    let mut remove_description = false;
    // Right-aligned palette and help icon, preserving the original order.
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.scope(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(2.0, 0.0);
            ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
            ui.menu_image_button(
                egui::Image::new(egui::include_image!(
                    "../../../../assets/icons/group_palette.svg"
                ))
                .fit_to_exact_size(egui::vec2(15.0, 15.0))
                .tint(mod_list_style::swatch_color(
                    selected_color,
                    ui.visuals().dark_mode,
                )),
                |ui| {
                    if mod_list_style::color_menu_option(ui, None, "None", selected_color.is_none())
                    {
                        requested_color = Some(None);
                        ui.close_menu();
                    }
                    for &(key, label) in mod_list_style::COLOR_OPTIONS {
                        if mod_list_style::color_menu_option(
                            ui,
                            Some(key),
                            label,
                            selected_color == Some(key),
                        ) {
                            requested_color = Some(Some(key.to_owned()));
                            ui.close_menu();
                        }
                    }
                },
            )
            .response
            .on_hover_text("Group color");
        });
        if let Some(description) = group_description {
            let group_tint = selected_color
                .map(|color| mod_list_style::swatch_color(Some(color), ui.visuals().dark_mode));
            remove_description = description_help_button(ui, description, group_tint);
        }
    });
    (title_clicked, requested_color, remove_description)
}
