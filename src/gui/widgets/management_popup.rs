//! Shared viewport-aware popup layout for Group and Profile Management.
//! The egui 0.31 popup API positions an area at the button but does not
//! constrain its *contents* when a collapsing section becomes tall.

const EDGE_PADDING: f32 = 6.0;
const FRAME_ALLOWANCE: f32 = 10.0;
const PREFERRED_WIDTH: f32 = 330.0;
const COMFORTABLE_HEIGHT: f32 = 180.0;

/// Returns the preferred side and maximum inner content size in logical points.
fn popup_geometry(screen: egui::Rect, button: egui::Rect) -> (egui::AboveOrBelow, f32, f32) {
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
/// Shared Group/Profile popup, sized from the current pass's visible controls.
/// Keep the popup anchored while collapsing sections expand or contract.
pub(super) fn show<R>(
    parent_ui: &egui::Ui,
    id: egui::Id,
    button: &egui::Response,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    if !parent_ui.memory(|mem| mem.is_popup_open(id)) {
        return None;
    }

    let (side, max_height, width) = popup_geometry(parent_ui.ctx().screen_rect(), button.rect);
    let (mut anchor, pivot) = match side {
        egui::AboveOrBelow::Above => (button.rect.left_top(), egui::Align2::LEFT_BOTTOM),
        egui::AboveOrBelow::Below => (button.rect.left_bottom(), egui::Align2::LEFT_TOP),
    };
    if let Some(to_global) = parent_ui
        .ctx()
        .layer_transform_to_global(parent_ui.layer_id())
    {
        anchor = to_global * anchor;
    }

    let popup = egui::Area::new(id)
        .kind(egui::UiKind::Popup)
        .order(egui::Order::Foreground)
        .fixed_pos(anchor)
        .pivot(pivot)
        .show(parent_ui.ctx(), |ui| {
            egui::Frame::popup(parent_ui.style())
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;

                    // Use an unallocated child so the parent's cached Area size
                    // cannot force a stale popup height on the visible Frame.
                    let content_rect =
                        egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, max_height));
                    let mut body = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt("management-body-v8")
                            .max_rect(content_rect),
                    );
                    let output = egui::ScrollArea::vertical()
                        .id_salt("management-contents-v8")
                        .max_height(max_height)
                        .min_scrolled_height(0.0)
                        .auto_shrink([false, true])
                        .scroll_bar_visibility(
                            egui::containers::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                        )
                        .show(&mut body, |ui| {
                            ui.set_width(width);
                            let top = ui.cursor().top();
                            let value = contents(ui);
                            let bottom = ui.data(|data| {
                                data.get_temp::<f32>(id.with("management-last-bottom-v6"))
                            });
                            let measured = bottom.map(|end| (end - top + 4.0).max(1.0));
                            (value, measured)
                        });
                    let (value, measured) = output.inner;
                    let visible_height = measured
                        .filter(|height| height.is_finite())
                        .unwrap_or(output.content_size.y)
                        .clamp(1.0, max_height);

                    // Parent Frame gets its size *now*, not a frame later.
                    // Only the ScrollArea can use the additional clipped space.
                    ui.allocate_space(egui::vec2(width, visible_height));
                    (value, measured, visible_height)
                })
                .inner
        });

    let (value, measured_height, visible_height) = popup.inner;
    if std::env::var_os("MINTFIXED_POPUP_DEBUG").is_some() {
        let reported = measured_height.unwrap_or(0.0);
        let expected_outer_height = visible_height + 14.0;
        if (popup.response.rect.height() - expected_outer_height).abs() > 3.0 {
            eprintln!(
                "[mintfixed popup v8] content={reported:.0} visible={visible_height:.0} outer={:.0} max={max_height:.0}",
                popup.response.rect.height(),
            );
        }
    }

    // Match the original popup's Escape and click-outside semantics.
    if parent_ui.input(|input| input.key_pressed(egui::Key::Escape))
        || (button.clicked_elsewhere() && popup.response.clicked_elsewhere())
    {
        parent_ui.memory_mut(|mem| mem.close_popup());
    }
    Some(value)
}

/// Record the bottom of the last *actual* control in the popup.
/// This avoids mistaking empty space from an old popup layout for content.
pub(super) fn record_content_end(ui: &egui::Ui, id: egui::Id, bottom: f32) {
    ui.data_mut(|data| data.insert_temp(id.with("management-last-bottom-v6"), bottom));
}

/// Scroll only when a section is opened, without closing any other section.
pub(super) fn scroll_expanded_header(header: &egui::Response, expanded: bool) {
    if expanded && header.clicked() {
        header.scroll_to_me(None);
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
