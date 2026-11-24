use super::*;

#[test]
fn edge_scroll_reaches_2400_pixels_per_second_near_edges() {
    let rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(200.0, 400.0));
    let dt = 1.0 / 60.0;
    let upward = edge_scroll_delta(rect, egui::pos2(100.0, 1.0), dt);
    let downward = edge_scroll_delta(rect, egui::pos2(100.0, 399.0), dt);
    let expected = 2_400.0 * dt * (79.0_f32 / 80.0).powi(2);
    assert!((upward - expected).abs() < 0.001);
    assert!((downward + expected).abs() < 0.001);
}

#[test]
fn drag_preview_stays_with_pointer_when_scrolling_down() {
    let press = egui::pos2(100.0, 150.0);
    let pointer = egui::pos2(105.0, 220.0);
    let initial_row = egui::pos2(30.0, 130.0);
    let scrolled_row = egui::pos2(30.0, -70.0);
    let translation = drag_paint_translation(pointer, press, initial_row, scrolled_row);
    assert_eq!(scrolled_row + translation, egui::pos2(35.0, 200.0));
}

#[test]
fn drag_preview_stays_with_pointer_when_scrolling_up() {
    let press = egui::pos2(100.0, 150.0);
    let pointer = egui::pos2(105.0, 80.0);
    let initial_row = egui::pos2(30.0, 130.0);
    let scrolled_row = egui::pos2(30.0, 330.0);
    let translation = drag_paint_translation(pointer, press, initial_row, scrolled_row);
    assert_eq!(scrolled_row + translation, egui::pos2(35.0, 60.0));
}

#[test]
fn drag_preview_does_not_drift_when_pointer_is_stationary() {
    let press = egui::pos2(100.0, 150.0);
    let initial_row = egui::pos2(30.0, 130.0);
    for y in [-170.0, 80.0, 130.0, 270.0] {
        let current_row = egui::pos2(30.0, y);
        let delta = drag_paint_translation(press, press, initial_row, current_row);
        assert_eq!(current_row + delta, initial_row);
    }
}

#[test]
fn hover_requires_dwell_and_resets_after_leaving_or_missing_frames() {
    let mut timer = HoverOpen::default();
    assert!(!timer.update(1.0, true));
    for i in 1..5 {
        assert!(!timer.update(1.0 + i as f64 * 0.1, true));
    }
    assert!(timer.update(1.51, true));
    assert!(!timer.update(1.52, false));
    assert!(!timer.update(1.53, true));
    assert!(!timer.update(5.0, true));
    assert!(!timer.update(5.1, true));
}

#[test]
fn edge_scroll_continues_past_top_and_bottom_but_not_left_or_right() {
    let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(210.0, 420.0));
    let dt = 1.0 / 60.0;
    assert_eq!(edge_scroll_delta(rect, egui::pos2(100.0, -20.0), dt), 2_400.0 * dt);
    assert_eq!(edge_scroll_delta(rect, egui::pos2(100.0, 460.0), dt), -2_400.0 * dt);
    assert_eq!(edge_scroll_delta(rect, egui::pos2(9.0, -20.0), dt), 0.0);
    assert_eq!(edge_scroll_delta(rect, egui::pos2(211.0, 460.0), dt), 0.0);
}

#[test]
fn edge_scroll_has_direction_speed_limit_and_no_motion_outside_view() {
    let rect = egui::Rect::from_min_max(egui::pos2(10.0, 20.0), egui::pos2(210.0, 420.0));
    assert!(edge_scroll_delta(rect, egui::pos2(50.0, 21.0), 0.016) > 0.0);
    assert!(edge_scroll_delta(rect, egui::pos2(50.0, 419.0), 0.016) < 0.0);
    assert_eq!(edge_scroll_delta(rect, rect.center(), 0.016), 0.0);
    assert_eq!(edge_scroll_delta(rect, egui::pos2(9.0, 21.0), 0.016), 0.0);
    assert!(edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 10.0) <= 120.0);
    assert_eq!(
        edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 0.032),
        2.0 * edge_scroll_delta(rect, egui::pos2(50.0, 20.0), 0.016)
    );
}
