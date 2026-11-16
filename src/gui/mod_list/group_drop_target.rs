// Full-width group header hit testing, hover-open and drop feedback.
// Kept separate from group title/appearance rendering.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_group_drop_target(
    ui: &mut Ui,
    header_response: &egui::Response,
    body_response: Option<&egui::Response>,
    fully_closed: bool,
    profile_name: &str,
    group_name: &str,
    row_index: usize,
    manual_order: bool,
    groups_to_open: &mut HashSet<(String, String)>,
    file_group: &mut Option<(String, bool)>,
    drag: &mut Option<(Drag, Target)>,
) {
    // Give the group header a full-width drop target, but leave
    // narrow bands for inserting a mod *outside* the group.
    let mut header_row = header_response.rect;
    header_row.min.x = ui.max_rect().left();
    header_row.max.x = ui.max_rect().right();
    let edge = 5.0_f32.min(header_row.height() * 0.2);
    // An expanded group's members immediately follow the header,
    // so the bottom edge should drop into the group, not after it.
    let bottom_edge = if fully_closed { edge } else { 0.0 };
    let mut header_drop_area = header_row;
    header_drop_area.min.y += edge;
    header_drop_area.max.y -= bottom_edge;
    let above_group = egui::Rect::from_min_max(
        header_row.min,
        egui::pos2(header_row.max.x, header_drop_area.min.y),
    );
    let below_group = egui::Rect::from_min_max(
        egui::pos2(header_row.min.x, header_drop_area.max.y),
        header_row.max,
    );
    if group_controls::hover_open(
        ui,
        header_response,
        header_drop_area,
        fully_closed,
        profile_name,
        manual_order,
    ) {
        groups_to_open.insert((profile_name.to_owned(), group_name.to_owned()));
        ui.ctx().request_repaint();
    }
    let mut area = header_drop_area;
    if let Some(body) = &body_response {
        area = area.union(body.rect);
    }
    if ui.rect_contains_pointer(area) {
        *file_group = Some((group_name.to_owned(), fully_closed));
    }
    let over_group_header = ui.rect_contains_pointer(header_drop_area);
    let over_above_group = ui.rect_contains_pointer(above_group);
    let over_below_group = fully_closed && ui.rect_contains_pointer(below_group);
    let dragging_mod = egui::DragAndDrop::payload::<Drag>(ui.ctx()).is_some_and(|d| !d.is_group);
    // Highlight the destination without drawing lines over the
    // group name. A slim rounded bar indicates a profile-level
    // insertion; a soft tint indicates a drop into the group.
    let accent = ui.visuals().selection.stroke.color;
    let group_fill = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 42);
    if manual_order && dragging_mod {
        if over_above_group || over_below_group {
            let y = if over_above_group {
                above_group.center().y
            } else {
                below_group.center().y
            };
            let marker = egui::Rect::from_center_size(
                egui::pos2(header_row.center().x, y),
                egui::vec2((header_row.width() - 16.0).max(0.0), 3.0),
            );
            ui.painter()
                .rect_filled(marker, egui::CornerRadius::same(2), accent);
        } else if over_group_header {
            ui.painter().rect_filled(
                header_drop_area.shrink2(egui::vec2(3.0, 0.0)),
                egui::CornerRadius::same(4),
                group_fill,
            );
        }
    } else if file_group
        .as_ref()
        .is_some_and(|(name, _)| name == group_name)
        && ui.input(|i| !i.raw.hovered_files.is_empty())
    {
        // Use the same drop-into-group feedback for local .pak files.
        ui.painter().rect_filled(
            header_drop_area.shrink2(egui::vec2(3.0, 0.0)),
            egui::CornerRadius::same(4),
            group_fill,
        );
    }
    if manual_order
        && dragging_mod
        && (over_above_group || over_group_header || over_below_group)
        && ui.input(|i| i.pointer.any_released())
        && drag.is_none()
        && let Some(source) = egui::DragAndDrop::payload::<Drag>(ui.ctx())
    {
        let target = if over_above_group {
            Target {
                group: None,
                slot: row_index,
            }
        } else if over_below_group {
            Target {
                group: None,
                slot: row_index + 1,
            }
        } else {
            // Dropping on the header always prepends to the group,
            // whether it was already open or expanded on hover.
            Target {
                group: Some(group_name.to_owned()),
                slot: 0,
            }
        };
        *drag = Some(((*source).clone(), target));
    }
}
