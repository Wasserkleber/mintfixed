/// Hover timers are scoped to the rendered group header, including its window.
/// Requiring a recent observation prevents an old timer surviving a profile switch.
#[derive(Clone, Copy, Default)]
struct HoverOpen {
    started: Option<f64>,
    last_seen: f64,
}

impl HoverOpen {
    fn update(&mut self, now: f64, active: bool) -> bool {
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

fn edge_scroll_delta(view: egui::Rect, pointer: egui::Pos2, dt: f32) -> f32 {
    if !view.contains(pointer) || view.height() <= 0.0 {
        return 0.0;
    }
    let band = EDGE_SCROLL_ZONE.min(view.height() * 0.45);
    let top = ((view.top() + band - pointer.y) / band).clamp(0.0, 1.0);
    let bottom = ((pointer.y - (view.bottom() - band)) / band).clamp(0.0, 1.0);
    // Positive moves content down (scroll up); negative scrolls down.
    (top * top - bottom * bottom) * EDGE_SCROLL_MAX_SPEED * dt.clamp(0.0, 0.05)
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
    let delta = ui.input(|i| {
        i.pointer
            .hover_pos()
            .map(|pointer| edge_scroll_delta(view, pointer, i.stable_dt))
            .unwrap_or(0.0)
    });
    if delta != 0.0 {
        ui.scroll_with_delta(egui::vec2(0.0, delta));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
}

