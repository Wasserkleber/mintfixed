//! Common text buttons, description help and bounded status labels.
use eframe::egui::{self, Button, Color32, RichText, Ui};

pub(in crate::gui) fn description_help_button(ui: &mut Ui, description: &str, group_tint: Option<Color32>) {
    // Use the supplied group accent directly, not inherited widget text visuals.
    // The same hover-only icon is used by headers and both mod-row paths.
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    let glyph = group_tint.unwrap_or_else(|| {
        if response.hovered() {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        }
    });
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().circle_filled(
                rect.center(),
                9.0,
                ui.visuals().widgets.hovered.weak_bg_fill,
            );
        }
        egui::Image::new(egui::include_image!(
            "../../../assets/icons/description_help.svg"
        ))
        .tint(glyph)
        .paint_at(ui, rect.shrink(1.5));
    }
    response
        .on_hover_ui_at_pointer(|ui| {
            ui.set_max_width(320.0);
            ui.label(RichText::new("Description").strong());
            ui.label(description);
        });
}

// A single-line, width-bounded status label. Never let diagnostics reserve toolbar space.
// Hover exposes the unabridged message; clicking opens the copyable details window.
pub(in crate::gui) fn status_message(ui: &mut Ui, message: &str) -> bool {
    let width = ui.available_width();
    if width < 18.0 {
        return false;
    }
    // A frameless Button keeps the text left-aligned and truncates without
    // Label's automatic tooltip (egui 0.31), avoiding duplicate hover popups.
    ui.add(
        Button::new(RichText::new(message).text_style(egui::TextStyle::Body))
            .frame(false)
            .truncate(),
    )
    .on_hover_ui_at_pointer(|ui| {
        ui.set_max_width(500.0);
        ui.label(message);
        ui.small("Click to open and copy the complete message");
    })
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .clicked()
}

// Shared styling for ordinary text buttons; icon buttons retain their own rendering.
pub(in crate::gui) fn mint_text_button(text: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(text).corner_radius(4_u8)
}
