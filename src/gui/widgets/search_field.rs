//! Mod-list search field and its inline clear affordance.
use eframe::egui::{self, Align, Ui};

/// Render the field without changing the parent's UI scopes or widget IDs.
/// The caller owns the query, focus handling and scroll-to-match behaviour.
pub(super) fn show(
    ui: &mut Ui,
    search_string: &mut String,
    any_matches: bool,
    show_clear: bool,
) -> (egui::Response, bool) {
    // Keep egui's native text-field border and size. The right margin
    // reserves room for a clear icon without adding another layout row.
    let mut text_edit = egui::TextEdit::singleline(search_string)
        .hint_text("Search")
        .margin(egui::Margin {
            left: 4,
            right: 28,
            top: 2,
            bottom: 2,
        })
        .desired_width((ui.spacing().text_edit_width - 24.0).max(80.0));
    if !any_matches {
        text_edit = text_edit.text_color(ui.visuals().error_fg_color);
    }
    ui
        .scope_builder(
            egui::UiBuilder::new().layout(egui::Layout::bottom_up(Align::RIGHT)),
            |ui| {
                let res = ui.add(text_edit);
                // TextEdit returns its inner rect, excluding the reserved right
                // margin. Paint the icon inside that margin without affecting layout.
                let clear_rect = egui::Rect::from_center_size(
                    egui::pos2(res.rect.right() + 14.0, res.rect.center().y - 0.5),
                    egui::vec2(22.0, 20.0),
                );
                // The clear icon remains visible but does not accept clicks when
                // the search is empty (or the whole panel is disabled).
                let can_clear = show_clear && ui.is_enabled();
                let clear = can_clear.then(|| {
                    ui.interact(
                        clear_rect,
                        ui.id().with("mod_search_clear"),
                        egui::Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Clear search")
                });
                let color = match &clear {
                    Some(clear) if clear.hovered() => {
                        ui.visuals().widgets.hovered.fg_stroke.color
                    }
                    Some(_) => ui.visuals().weak_text_color(),
                    None => ui.visuals().weak_text_color().gamma_multiply(0.45),
                };
                // Draw the SVG directly in the original search field: unlike
                // ui.put(), Image::paint_at does not allocate another widget row.
                egui::Image::new(egui::include_image!("../../../assets/icons/search_clear.svg"))
                    .tint(color)
                    .paint_at(
                        ui,
                        egui::Rect::from_center_size(
                            clear_rect.center(),
                            egui::vec2(17.0, 17.0),
                        ),
                    );
                let clear_clicked = clear.is_some_and(|response| response.clicked());
                (res, clear_clicked)
            },
        )
        .inner
}
