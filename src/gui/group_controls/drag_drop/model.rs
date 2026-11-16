#[derive(Clone)]
pub(in crate::gui) struct Drag {
    profile: String,
    group: Option<String>,
    index: usize,
    key: String,
    pub(in crate::gui) is_group: bool,
}

pub(in crate::gui) struct Target {
    pub group: Option<String>,
    pub slot: usize,
}

// The dragged row is still laid out inside the scrolling list. Keep its first
// painted position so the preview can cancel any later scroll displacement.
#[derive(Clone, Copy)]
struct DragPaintAnchor {
    press_origin: egui::Pos2,
    row_origin: egui::Pos2,
}

const DRAG_PAINT_ANCHOR_ID: &str = "mintfixed-drag-paint-anchor";

fn drag_paint_translation(
    pointer: egui::Pos2,
    press_origin: egui::Pos2,
    initial_row: egui::Pos2,
    current_row: egui::Pos2,
) -> egui::Vec2 {
    // Pointer movement moves the preview; scrolling must not move it twice.
    (pointer - press_origin) + (initial_row - current_row)
}

