//! One compact SVG delete button shared by the mod-list row renderers.
//! A chosen group color softly tints both glyph and background.
use eframe::egui::{self, Color32, Ui};

use super::widget_style::blend;

/// Returns the usual egui response so existing click/confirmation code stays unchanged.
/// Always-red destructive control. Group colors intentionally do not apply.
pub(in crate::gui) fn delete_button(ui: &mut Ui, _group_color: Option<Color32>) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let (background, hovered_background, active_background, icon_color, hovered_icon) = if dark {
        (
            Color32::from_rgb(58, 41, 44),
            Color32::from_rgb(84, 44, 49),
            Color32::from_rgb(103, 48, 54),
            Color32::from_rgb(217, 107, 112),
            Color32::from_rgb(240, 90, 96),
        )
    } else {
        (
            Color32::from_rgb(247, 231, 233),
            Color32::from_rgb(243, 206, 210),
            Color32::from_rgb(237, 189, 194),
            Color32::from_rgb(184, 65, 73),
            Color32::from_rgb(215, 51, 63),
        )
    };

    ui.scope(|ui| {
        // Unlike Button::fill(), scoped widget visuals preserve hover styling.
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.inactive.weak_bg_fill = background;
        widgets.hovered.weak_bg_fill = hovered_background;
        widgets.active.weak_bg_fill = active_background;
        widgets.inactive.bg_stroke = egui::Stroke::NONE;
        widgets.hovered.bg_stroke = egui::Stroke::NONE;
        widgets.active.bg_stroke = egui::Stroke::NONE;

        // The icon responds to hover as well as the button surface. A button
        // is at least 20x18 px; the icon is centered in its row as before.
        let likely_button_rect =
            egui::Rect::from_min_size(ui.next_widget_position(), egui::vec2(20.0, 18.0));
        let icon_tint = if ui
            .ctx()
            .pointer_hover_pos()
            .is_some_and(|position| likely_button_rect.contains(position))
        {
            hovered_icon
        } else {
            icon_color
        };

        let image = egui::Image::new(egui::include_image!("../../../assets/icons/delete.svg"))
            .fit_to_exact_size(egui::vec2(14.0, 14.0))
            .tint(icon_tint);
        ui.add(egui::Button::image(image).min_size(egui::vec2(20.0, 18.0)))
    })
    .inner
}
/// Copy a single mod URL. Shares the delete button's compact dimensions and
/// tinting, without changing the existing clipboard handler or tooltip.
pub(in crate::gui) fn copy_url_button(ui: &mut Ui, group_color: Option<Color32>) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let base_bg = if dark {
        Color32::from_rgb(46, 47, 50)
    } else {
        Color32::from_rgb(231, 233, 237)
    };
    let base_icon = ui.visuals().text_color();
    let bg = group_color.map_or(base_bg, |accent| blend(base_bg, accent, 14));
    let glyph = group_color.map_or(base_icon, |accent| blend(base_icon, accent, 30));
    let image = egui::Image::new(egui::include_image!("../../../assets/icons/copy_url.svg"))
        .fit_to_exact_size(egui::vec2(14.0, 14.0))
        .tint(glyph);
    ui.add(
        egui::Button::image(image)
            .fill(bg)
            .min_size(egui::vec2(20.0, 18.0)),
    )
}
/// Warning indicator for duplicate mods and missing dependencies.
/// Keep its glyph a fixed, vivid orange even inside colored or disabled groups.
/// Only the button background is softly influenced by the selected group tint.
/// Always-orange warning indicator, even inside tinted groups.
pub(in crate::gui) fn warning_button(ui: &mut Ui, _group_color: Option<Color32>) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let (background, hovered_background, active_background) = if dark {
        (
            Color32::from_rgb(65, 49, 29),
            Color32::from_rgb(90, 64, 32),
            Color32::from_rgb(111, 77, 34),
        )
    } else {
        (
            Color32::from_rgb(251, 240, 217),
            Color32::from_rgb(253, 226, 180),
            Color32::from_rgb(248, 213, 159),
        )
    };

    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.inactive.weak_bg_fill = background;
        widgets.hovered.weak_bg_fill = hovered_background;
        widgets.active.weak_bg_fill = active_background;
        widgets.inactive.bg_stroke = egui::Stroke::NONE;
        widgets.hovered.bg_stroke = egui::Stroke::NONE;
        widgets.active.bg_stroke = egui::Stroke::NONE;

        let image = egui::Image::new(egui::include_image!("../../../assets/icons/warning.svg"))
            .fit_to_exact_size(egui::vec2(14.0, 14.0))
            .tint(Color32::from_rgb(255, 159, 39));
        ui.add(egui::Button::image(image).min_size(egui::vec2(20.0, 18.0)))
    })
    .inner
}

/// Non-clickable error marker for unavailable or otherwise problematic mods.
/// Styled like other 20x18 mod-row SVG controls, without a click action.
fn mod_error_indicator(ui: &mut Ui, sense: egui::Sense) -> egui::Response {
    // Shared passive/clickable red error indicator. Match the delete icon's
    // background and hover palette in dark and light modes.
    let (background, hovered_background, glyph, hovered_glyph) = if ui.visuals().dark_mode {
        (
            Color32::from_rgb(58, 41, 44),
            Color32::from_rgb(84, 44, 49),
            Color32::from_rgb(217, 107, 112),
            Color32::from_rgb(240, 90, 96),
        )
    } else {
        (
            Color32::from_rgb(247, 231, 233),
            Color32::from_rgb(243, 206, 210),
            Color32::from_rgb(184, 65, 73),
            Color32::from_rgb(215, 51, 63),
        )
    };

    let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 18.0), sense);
    let hovered = response.hovered();
    ui.painter().rect_filled(
        rect,
        2.0,
        if hovered {
            hovered_background
        } else {
            background
        },
    );

    // Painting the image separately avoids a second interactable widget.
    egui::Image::new(egui::include_image!("../../../assets/icons/mod_error.svg"))
        .fit_to_exact_size(egui::vec2(14.0, 14.0))
        .tint(if hovered { hovered_glyph } else { glyph })
        .paint_at(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(14.0, 14.0)),
        );

    response
}

/// Passive error marker for unavailable or otherwise problematic mod rows,
/// including the unavailable-member indicator on group headers.
pub(in crate::gui) fn mod_error_badge(ui: &mut Ui) -> egui::Response {
    mod_error_indicator(ui, egui::Sense::hover())
}

/// Clickable error marker used to open the global unavailable-mod overview.
pub(in crate::gui) fn mod_error_button(ui: &mut Ui) -> egui::Response {
    mod_error_indicator(ui, egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Non-interactive provider icon for local-file mods.
/// Matches the compact mod-row controls without adding borders or row padding.
/// Passive file-type marker: not a clickable control and no button-style fill.
pub(in crate::gui) fn folder_badge(ui: &mut Ui, group_color: Option<Color32>) {
    let base_icon = ui.visuals().text_color();
    let glyph = group_color.map_or(base_icon, |accent| blend(base_icon, accent, 35));

    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());

    ui.put(
        rect.shrink(2.0),
        egui::Image::new(egui::include_image!("../../../assets/icons/folder.svg"))
            .fit_to_exact_size(egui::vec2(14.0, 14.0))
            .tint(glyph),
    );
}
