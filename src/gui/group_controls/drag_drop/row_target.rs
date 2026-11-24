use super::*;

pub(in crate::gui) fn row_target(
    ui: &Ui,
    response: &egui::Response,
    drag: &mut Option<(Drag, Target)>,
    group: Option<&str>,
    index: usize,
) {
    // A member or header gets first choice for mod drops. Whole groups always
    // use the enclosing profile row, even over the members of an expanded group.
    if drag.is_some() {
        return;
    }
    let Some(source) = egui::DragAndDrop::payload::<Drag>(ui.ctx()) else {
        return;
    };
    if group.is_some() && source.is_group {
        return;
    }
    let mut rect = response.rect;
    rect.max.x = rect.max.x.max(ui.max_rect().right());
    // Use the space between profile rows as an insertion target, so dragging
    // a mod between two adjacent groups does not drop it into either group.
    // Keep the drawn marker on the actual row edge, not in the gap.
    let mut hit_rect = rect;
    if group.is_none() {
        let half_gap = ui.spacing().item_spacing.y * 0.5;
        hit_rect.min.y -= half_gap;
        hit_rect.max.y += half_gap;
    }
    if !ui.rect_contains_pointer(hit_rect) {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    let after = pointer.y > rect.center().y;
    let y = if after { rect.bottom() } else { rect.top() };
    // A short rounded insertion marker is clearer than a hard full-row line.
    let marker = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, y),
        egui::vec2((rect.width() - 16.0).max(0.0), 3.0),
    );
    ui.painter().rect_filled(
        marker,
        egui::CornerRadius::same(2),
        ui.visuals().selection.stroke.color,
    );
    if ui.input(|input| input.pointer.any_released()) {
        *drag = Some((
            (*source).clone(),
            Target {
                group: group.map(str::to_owned),
                slot: index + usize::from(after),
            },
        ));
    }
}
