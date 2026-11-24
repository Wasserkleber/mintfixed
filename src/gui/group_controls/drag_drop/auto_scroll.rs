use super::*;

/// Hover timers are scoped to the rendered group header, including its window.
/// Requiring a recent observation prevents an old timer surviving a profile switch.
#[derive(Clone, Copy, Default)]
pub(super) struct HoverOpen {
    started: Option<f64>,
    last_seen: f64,
}

impl HoverOpen {
    pub(super) fn update(&mut self, now: f64, active: bool) -> bool {
        if !active {
            self.started = None;
            self.last_seen = now;
            return false;
        }
        if now < self.last_seen || now - self.last_seen > 0.2 {
            self.started = None;
        }
        self.last_seen = now;
        let start = *self.started.get_or_insert(now);
        now - start >= 0.5
    }
}

pub(in crate::gui) fn hover_open(
    ui: &Ui,
    header: &egui::Response,
    header_drop_area: egui::Rect,
    closed: bool,
    profile: &str,
    manual: bool,
) -> bool {
    let internal = manual
        && egui::DragAndDrop::payload::<Drag>(ui.ctx())
            .is_some_and(|drag| !drag.is_group && drag.profile == profile);
    let external = ui.input(|i| !i.raw.hovered_files.is_empty());
    let active = closed && (internal || external) && ui.rect_contains_pointer(header_drop_area);
    let id = header.id.with("drag-hover-open");
    let now = ui.input(|i| i.time);
    let open = ui.ctx().data_mut(|data| {
        let mut timer = data.get_temp::<HoverOpen>(id).unwrap_or_default();
        let open = timer.update(now, active);
        data.insert_temp(id, timer);
        open
    });
    if active {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
    open
}

const EDGE_SCROLL_ZONE: f32 = 80.0;
const EDGE_SCROLL_MAX_SPEED: f32 = 2_400.0;

pub(super) fn edge_scroll_delta(view: egui::Rect, pointer: egui::Pos2, dt: f32) -> f32 {
    if view.height() <= 0.0 || pointer.x < view.left() || pointer.x > view.right() {
        return 0.0;
    }
    let band = EDGE_SCROLL_ZONE.min(view.height() * 0.45);
    let top = ((view.top() + band - pointer.y) / band).clamp(0.0, 1.0);
    let bottom = ((pointer.y - (view.bottom() - band)) / band).clamp(0.0, 1.0);
    // Positive moves content down (scroll up); negative scrolls down.
    (top * top - bottom * bottom) * EDGE_SCROLL_MAX_SPEED * dt.clamp(0.0, 0.05)
}

// egui-winit reports PointerGone at the native window boundary. On Windows,
// read the real cursor position while a mod drag is active so that edge
// scrolling does not depend on a stale or missing egui hover position.
#[cfg(target_os = "windows")]
mod window_pointer {
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetActiveWindow() -> isize;
        fn GetAsyncKeyState(key: i32) -> i16;
        fn GetCursorPos(point: *mut Point) -> i32;
        fn ScreenToClient(window: isize, point: *mut Point) -> i32;
    }

    pub(super) fn cursor_in_window_points(pixels_per_point: f32) -> Option<(bool, egui::Pos2)> {
        // All calls are read-only Win32 queries, executed on the GUI thread.
        let window = unsafe { GetActiveWindow() };
        if window == 0 {
            return None;
        }
        let mut point = Point { x: 0, y: 0 };
        let (down, ok) = unsafe {
            let down = (GetAsyncKeyState(0x01) as u16 & 0x8000) != 0;
            let ok = GetCursorPos(&mut point) != 0 && ScreenToClient(window, &mut point) != 0;
            (down, ok)
        };
        if !ok {
            return None;
        }
        let scale = pixels_per_point.max(0.01);
        Some((down, egui::pos2(point.x as f32 / scale, point.y as f32 / scale)))
    }
}

pub(in crate::gui) fn auto_scroll(ui: &Ui, profile: &str, manual: bool) {
    let internal = manual
        && egui::DragAndDrop::payload::<Drag>(ui.ctx()).is_some_and(|drag| drag.profile == profile);
    let external = ui.input(|i| !i.raw.hovered_files.is_empty());
    if !internal && !external {
        return;
    }
    // clip_rect is the visible part of the scroll area, not its full content.
    let view = ui.clip_rect();
    let (hover, dragging, focused, dt, _scale) = ui.input(|i| {
        (
            i.pointer.hover_pos(),
            i.pointer.primary_down(),
            i.focused,
            i.stable_dt,
            i.pixels_per_point(),
        )
    });
    #[cfg(target_os = "windows")]
    let mut pointer = hover;
    #[cfg(not(target_os = "windows"))]
    let pointer = hover;
    // Only use the OS fallback for an existing internal drag, never for an
    // ordinary hover or an external file drop.
    if internal && dragging && focused && hover.is_none() {
        #[cfg(target_os = "windows")]
        if let Some((still_down, position)) = window_pointer::cursor_in_window_points(_scale) {
            if !still_down {
                // A release outside the window might not reach egui-winit.
                egui::DragAndDrop::clear_payload(ui.ctx());
                return;
            }
            pointer = Some(position);
        }
    }
    let delta = pointer.map_or(0.0, |pos| edge_scroll_delta(view, pos, dt));
    if delta != 0.0 {
        ui.scroll_with_delta(egui::vec2(0.0, delta));
    }
    if delta != 0.0 || (internal && hover.is_none() && pointer.is_some()) {
        // Keep polling only while a drag is outside the window. The normal
        // idle repaint budget is unchanged.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
}
