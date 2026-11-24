//! Shared colour and geometry primitives for compact GUI widgets.
use eframe::egui::{self, Color32, Ui};

/// Blend RGB channels using the same integer rounding as existing group tints.
pub(in crate::gui) fn blend(base: Color32, tint: Color32, percent: u16) -> Color32 {
    let percent = percent.min(100);
    let rest = 100 - percent;
    Color32::from_rgb(
        ((base.r() as u16 * rest + tint.r() as u16 * percent) / 100) as u8,
        ((base.g() as u16 * rest + tint.g() as u16 * percent) / 100) as u8,
        ((base.b() as u16 * rest + tint.b() as u16 * percent) / 100) as u8,
    )
}

/// Draw the original inline picker arrow without allocating another widget.
pub(in crate::gui) fn paint_picker_arrow(ui: &Ui, rect: egui::Rect, open: bool, color: Color32) {
    let x = rect.right() - 12.0;
    let y = rect.center().y;
    let arrow = if open {
        vec![
            egui::pos2(x - 4.0, y + 2.0),
            egui::pos2(x + 4.0, y + 2.0),
            egui::pos2(x, y - 2.0),
        ]
    } else {
        vec![
            egui::pos2(x - 4.0, y - 2.0),
            egui::pos2(x + 4.0, y - 2.0),
            egui::pos2(x, y + 2.0),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        arrow,
        color,
        egui::Stroke::NONE,
    ));
}

pub(in crate::gui) fn picker_scroll() -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .max_height(140.0)
        .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysVisible)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_preserves_rgb_rounding_and_bounds() {
        let base = Color32::from_rgb(10, 21, 30);
        let tint = Color32::from_rgb(101, 122, 240);
        assert_eq!(blend(base, tint, 0), base);
        assert_eq!(blend(base, tint, 100), tint);
        assert_eq!(blend(base, tint, 50), Color32::from_rgb(55, 71, 135));
        assert_eq!(blend(base, tint, 150), tint);
    }
}
