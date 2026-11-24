// Adapted from
// <https://github.com/emilk/egui/blob/master/crates/egui_demo_lib/src/demo/toggle_switch.rs>.

fn toggle_ui(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let desired_size = ui.spacing().interact_size.y * egui::vec2(2.0, 1.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, "")
    });

    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool(response.id, *on);
        let dark = ui.visuals().dark_mode;
        let (off, active, knob) = if dark {
            (
                egui::Color32::from_rgb(65, 68, 72),
                egui::Color32::from_rgb(49, 126, 89),
                egui::Color32::from_rgb(224, 231, 227),
            )
        } else {
            (
                egui::Color32::from_rgb(137, 142, 146),
                egui::Color32::from_rgb(45, 119, 82),
                egui::Color32::from_rgb(248, 250, 249),
            )
        };
        // Keep the original hit target and row alignment; only shrink the paint area.
        let track = rect.shrink2(rect.size() * egui::vec2(0.08, 0.12));
        let height = track.height();
        let mut fill = egui::Color32::from_rgb(
            egui::lerp(off.r() as f32..=active.r() as f32, how_on) as u8,
            egui::lerp(off.g() as f32..=active.g() as f32, how_on) as u8,
            egui::lerp(off.b() as f32..=active.b() as f32, how_on) as u8,
        );
        if ui.is_enabled() && response.hovered() {
            fill = fill.gamma_multiply(1.12);
        }
        let stroke = if response.has_focus() {
            ui.visuals().selection.stroke
        } else {
            egui::Stroke::NONE
        };
        ui.painter().rect(
            track,
            (height * 0.25).round() as u8,
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );
        let x = egui::lerp(
            (track.left() + height * 0.5)..=(track.right() - height * 0.5),
            how_on,
        );
        let thumb = egui::Rect::from_center_size(
            egui::pos2(x, track.center().y),
            egui::Vec2::splat(height * 0.68),
        );
        ui.painter()
            .rect_filled(thumb, (height * 0.16).round() as u8, knob);
    }

    response
}

#[allow(clippy::needless_pass_by_ref_mut)]
pub fn toggle_switch(on: &mut bool) -> impl egui::Widget + '_ {
    move |ui: &mut egui::Ui| toggle_ui(ui, on)
}
