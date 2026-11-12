//! One compact SVG delete button shared by the mod-list row renderers.
//! A chosen group color softly tints both glyph and background.
use eframe::egui::{self, Color32, Ui};

fn blend(base: Color32, accent: Color32, percent: u16) -> Color32 {
    let rest = 100 - percent;
    Color32::from_rgb(
        ((base.r() as u16 * rest + accent.r() as u16 * percent) / 100) as u8,
        ((base.g() as u16 * rest + accent.g() as u16 * percent) / 100) as u8,
        ((base.b() as u16 * rest + accent.b() as u16 * percent) / 100) as u8,
    )
}

/// Returns the usual egui response so existing click/confirmation code stays unchanged.
pub(super) fn delete_button(ui: &mut Ui, group_color: Option<Color32>) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let base_bg = if dark {
        Color32::from_rgb(46, 47, 50)
    } else {
        Color32::from_rgb(231, 233, 237)
    };
    let base_icon = ui.visuals().text_color();
    let bg = group_color.map_or(base_bg, |accent| blend(base_bg, accent, 12));
    let glyph = group_color.map_or(base_icon, |accent| blend(base_icon, accent, 32));

    let image = egui::Image::new(egui::include_image!("../../assets/icons/delete.svg"))
        .fit_to_exact_size(egui::vec2(14.0, 14.0))
        .tint(glyph);
    ui.add(
        egui::Button::image(image)
            .fill(bg)
            .min_size(egui::vec2(20.0, 18.0)),
    )
}
/// Copy a single mod URL. Shares the delete button's compact dimensions and
/// tinting, without changing the existing clipboard handler or tooltip.
pub(super) fn copy_url_button(ui: &mut Ui, group_color: Option<Color32>) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let base_bg = if dark {
        Color32::from_rgb(46, 47, 50)
    } else {
        Color32::from_rgb(231, 233, 237)
    };
    let base_icon = ui.visuals().text_color();
    let bg = group_color.map_or(base_bg, |accent| blend(base_bg, accent, 14));
    let glyph = group_color.map_or(base_icon, |accent| blend(base_icon, accent, 30));
    let image = egui::Image::new(egui::include_image!("../../assets/icons/copy_url.svg"))
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
pub(super) fn warning_button(ui: &mut Ui, group_color: Option<Color32>) -> egui::Response {
    let neutral_bg = if ui.visuals().dark_mode {
        Color32::from_rgb(46, 47, 50)
    } else {
        Color32::from_rgb(231, 233, 237)
    };
    let bg = group_color.map_or(neutral_bg, |accent| blend(neutral_bg, accent, 14));
    let image = egui::Image::new(egui::include_image!("../../assets/icons/warning.svg"))
        .fit_to_exact_size(egui::vec2(14.0, 14.0))
        .tint(Color32::from_rgb(255, 159, 39));
    ui.add(
        egui::Button::image(image)
            .fill(bg)
            .min_size(egui::vec2(20.0, 18.0)),
    )
}

/// Non-interactive provider icon for local-file mods.
/// Matches the compact mod-row controls without adding borders or row padding.
pub(super) fn folder_badge(ui: &mut Ui, group_color: Option<Color32>) {
    let neutral_bg = if ui.visuals().dark_mode {
        Color32::from_rgb(46, 47, 50)
    } else {
        Color32::from_rgb(231, 233, 237)
    };
    let bg = group_color.map_or(neutral_bg, |accent| blend(neutral_bg, accent, 14));
    let base_icon = ui.visuals().text_color();
    let glyph = group_color.map_or(base_icon, |accent| blend(base_icon, accent, 35));

    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, 2.0, bg);
    }
    ui.put(
        rect.shrink(2.0),
        egui::Image::new(egui::include_image!("../../assets/icons/folder.svg"))
            .fit_to_exact_size(egui::vec2(14.0, 14.0))
            .tint(glyph),
    );
}
