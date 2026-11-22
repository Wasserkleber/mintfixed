//! Shared viewport-aware popup layout for Group and Profile Management.
//! The egui 0.31 popup API positions an area at the button but does not
//! constrain its *contents* when a collapsing section becomes tall.

const EDGE_PADDING: f32 = 8.0;
const FRAME_ALLOWANCE: f32 = 20.0;
const PREFERRED_WIDTH: f32 = 330.0;
const COMFORTABLE_HEIGHT: f32 = 300.0;

/// Returns the preferred side and maximum inner content size in logical points.
fn popup_geometry(
    screen: egui::Rect,
    button: egui::Rect,
) -> (egui::AboveOrBelow, f32, f32) {
    let space_below = (screen.bottom() - button.bottom() - EDGE_PADDING).max(0.0);
    let space_above = (button.top() - screen.top() - EDGE_PADDING).max(0.0);
    // Prefer opening downwards unless there is too little room to work with.
    let below = space_below >= COMFORTABLE_HEIGHT || space_below >= space_above;
    let side = if below {
        egui::AboveOrBelow::Below
    } else {
        egui::AboveOrBelow::Above
    };
    let space = if below { space_below } else { space_above };
    // Reserve room for the popup frame and for a small gap from the screen edge.
    let max_height = (space - FRAME_ALLOWANCE).max(1.0);
    let width = (screen.width() - 2.0 * EDGE_PADDING - FRAME_ALLOWANCE)
        .max(1.0)
        .min(PREFERRED_WIDTH);
    (side, max_height, width)
}

/// Share layout, positioning and scroll behavior while keeping the current
/// per-menu persistent IDs, actions and click-outside close behavior.
pub(super) fn show<R>(
    parent_ui: &egui::Ui,
    id: egui::Id,
    button: &egui::Response,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let (side, max_height, width) = popup_geometry(parent_ui.ctx().screen_rect(), button.rect);
    egui::popup_above_or_below_widget(
        parent_ui,
        id,
        button,
        side,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_width(width);
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing.y = 4.0;
            egui::ScrollArea::vertical()
                .id_salt("management-contents")
                .max_height(max_height)
                .auto_shrink([false, true])
                .scroll_bar_visibility(
                    egui::containers::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                )
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.set_max_width(width);
                    contents(ui)
                })
                .inner
        },
    )
}

/// Scroll only when a section is opened, without closing any other section.
pub(super) fn scroll_expanded_header(header: &egui::Response, expanded: bool) {
    if expanded && header.clicked() {
        header.scroll_to_me(Some(egui::Align::Min));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_below_with_room_near_top() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 500.0));
        let button = egui::Rect::from_min_size(egui::pos2(10.0, 30.0), egui::vec2(140.0, 25.0));
        let (side, height, width) = popup_geometry(screen, button);
        assert_eq!(side, egui::AboveOrBelow::Below);
        assert!(height < screen.height() && height > 0.0);
        assert_eq!(width, PREFERRED_WIDTH);
    }

    #[test]
    fn opens_upwards_near_bottom() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 500.0));
        let button = egui::Rect::from_min_size(egui::pos2(10.0, 450.0), egui::vec2(140.0, 25.0));
        assert_eq!(popup_geometry(screen, button).0, egui::AboveOrBelow::Above);
    }

    #[test]
    fn limits_width_and_height_on_small_windows() {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(240.0, 200.0));
        let button = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(110.0, 24.0));
        let (_, height, width) = popup_geometry(screen, button);
        assert!(height > 0.0 && height < screen.height());
        assert!(width > 0.0 && width < screen.width());
    }
}
