//! Compact mod-list rows and optional user-selected group colors.
//! Tint selection is stored with each ModGroup; no automatic per-name colors.
use eframe::egui::{self, Color32, Ui};

pub(super) const COLOR_OPTIONS: &[(&str, &str)] = &[
    ("red", "Red"),
    ("orange", "Orange"),
    ("yellow", "Yellow"),
    ("green", "Green"),
    ("teal", "Teal"),
    ("blue", "Blue"),
    ("purple", "Purple"),
    ("pink", "Pink"),
];

fn accent(color: &str) -> Option<Color32> {
    Some(match color {
        "red" => Color32::from_rgb(217, 94, 95),
        "orange" => Color32::from_rgb(213, 139, 70),
        "yellow" => Color32::from_rgb(189, 169, 76),
        "green" => Color32::from_rgb(93, 178, 120),
        "teal" => Color32::from_rgb(83, 171, 179),
        "blue" => Color32::from_rgb(92, 142, 216),
        "purple" => Color32::from_rgb(155, 121, 211),
        "pink" => Color32::from_rgb(201, 112, 160),
        _ => return None,
    })
}

pub(super) fn swatch_color(color: Option<&str>, dark: bool) -> Color32 {
    color.and_then(accent).unwrap_or(if dark {
        Color32::from_gray(120)
    } else {
        Color32::from_gray(130)
    })
}

fn mix(base: Color32, tint: Color32, percent: u8) -> Color32 {
    let tint_part = percent as u16;
    let base_part = 100u16 - tint_part;
    Color32::from_rgb(
        ((base.r() as u16 * base_part + tint.r() as u16 * tint_part) / 100) as u8,
        ((base.g() as u16 * base_part + tint.g() as u16 * tint_part) / 100) as u8,
        ((base.b() as u16 * base_part + tint.b() as u16 * tint_part) / 100) as u8,
    )
}

/// Readable group-header text and arrow accent, blended with the active theme.
/// Uncolored groups use a quieter neutral gray instead of the bright theme foreground.
pub(super) fn group_header_text_color(ui: &Ui, selected: Option<&str>) -> Option<Color32> {
    Some(match selected.and_then(accent) {
        Some(accent) => mix(
            ui.visuals().text_color(),
            accent,
            if ui.visuals().dark_mode { 58 } else { 38 },
        ),
        None if ui.visuals().dark_mode => Color32::from_rgb(170, 173, 180),
        None => Color32::from_rgb(93, 98, 108),
    })
}

/// Only matching selected colors can be striped; other colors remain uniform.
pub(super) fn same_selected_color(previous: Option<&str>, current: Option<&str>) -> bool {
    matches!((previous, current), (Some(a), Some(b)) if a == b)
}

#[cfg(test)]
mod matching_color_stripe_tests {
    use super::same_selected_color;

    #[test]
    fn stripes_allowed_only_for_identical_selected_colors() {
        assert!(same_selected_color(Some("red"), Some("red")));
        assert!(!same_selected_color(Some("red"), Some("blue")));
        assert!(!same_selected_color(Some("red"), None));
        assert!(!same_selected_color(None, Some("red")));
        assert!(!same_selected_color(None, None));
    }
}

pub(super) fn row_frame(
    ui: &Ui,
    visual_index: usize,
    is_group: bool,
    selected_color: Option<&str>,
    is_group_child: bool,
    same_color_as_previous: bool,
) -> egui::Frame {
    let dark = ui.visuals().dark_mode;
    let alternate = visual_index % 2 == 1;
    // Light mode zebra stripes for uncolored rows; selected tints stay uniform.
    // Keep dark mode's original compact shades unchanged. In light mode,
    // derive both stripes from the active theme's panel background rather than
    // using near-identical hard-coded whites (previously only 3 RGB apart).
    let plain = if dark {
        if alternate {
            Color32::from_rgb(31, 32, 34)
        } else {
            Color32::from_rgb(29, 30, 32)
        }
    } else {
        let panel = ui.visuals().panel_fill;
        if alternate {
            mix(panel, Color32::from_rgb(80, 100, 125), 9)
        } else {
            panel
        }
    };
    let group_base = if dark {
        if alternate {
            Color32::from_rgb(36, 38, 42)
        } else {
            Color32::from_rgb(34, 36, 40)
        }
    } else {
        // Slightly differentiated headers without introducing outlines or
        // vertical padding; color-selected headers still blend over this base.
        let base = mix(ui.visuals().panel_fill, Color32::from_rgb(93, 111, 132), 9);
        if alternate {
            mix(base, Color32::from_rgb(80, 100, 125), 7)
        } else {
            base
        }
    };
    let tint = selected_color.and_then(accent);
    // Uniform colored group backgrounds (v6.6): no zebra striping except
    // where adjacent groups share the SAME user-selected tint.
    // Children of colored groups receive a uniform faint tint.
    let fill = if is_group {
        Some(tint.map_or(group_base, |color| {
            let unstriped = if dark {
                Color32::from_rgb(34, 36, 40)
            } else {
                mix(ui.visuals().panel_fill, Color32::from_rgb(93, 111, 132), 9)
            };
            let base = mix(unstriped, color, if dark { 18 } else { 20 });
            if same_color_as_previous {
                // Normal, subtle alternation within a same-color group run.
                mix(base, if dark { Color32::WHITE } else { Color32::BLACK }, 2)
            } else {
                base
            }
        }))
    } else if is_group_child {
        Some(tint.map_or(plain, |color| {
            // Group member tint v6.9: one shade per colored group. Members
            // use the same hue as their header, at about 70% of its tint
            // strength. No child-row alternation for selected colors.
            let unstriped = if dark {
                Color32::from_rgb(29, 30, 32)
            } else {
                ui.visuals().panel_fill
            };
            mix(unstriped, color, if dark { 15 } else { 17 })
        }))
    } else if alternate || !dark {
        // Light mode paints both shades (including the even row) so the
        // selected theme background and tinted child rows align consistently.
        Some(plain)
    } else {
        None
    };

    // Horizontal inset only. Preserve the exact compact row height from v4.
    let mut frame = egui::Frame::NONE.inner_margin(egui::Margin::symmetric(2, 0));
    if let Some(fill) = fill {
        frame = frame.fill(fill);
    }
    frame
}

/// Softly tint only the version selector and load-priority control of a
/// mod inside a manually colored group. Call from a `ui.scope` so other
/// widgets (toggles, icons, popup controls, standalone mods) are unaffected.
pub(super) fn tint_group_member_controls(ui: &mut Ui, selected: Option<Color32>) {
    let Some(accent) = selected else {
        return;
    };
    let dark = ui.visuals().dark_mode;
    let (strength, edit_strength) = if dark { (18, 12) } else { (17, 11) };
    let visuals = ui.visuals_mut();

    // Inactive/open handles the visible ComboBox field. Hovered and active
    // provide matching transitions; DragValue uses those widget visuals too.
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.weak_bg_fill = mix(widget.weak_bg_fill, accent, strength);
        widget.bg_fill = mix(widget.bg_fill, accent, strength);
        // Covers the ComboBox chevron and DragValue foreground.
        widget.fg_stroke.color = mix(widget.fg_stroke.color, accent, if dark { 26 } else { 22 });
    }
    // DragValue becomes an editable text field when clicked. Match its
    // background without tinting its priority-specific red/green text.
    // The version label and zero-priority dash use a matching soft tint.
    // Existing red/green priority overrides are set later and stay intact.
    let text = visuals.text_color();
    visuals.override_text_color = Some(mix(text, accent, if dark { 26 } else { 22 }));
    visuals.extreme_bg_color = mix(visuals.extreme_bg_color, accent, edit_strength);
}

// Draw an entire palette entry as one click target. The filled swatch and
// colored text use the existing group accent; selection is shown on the right.
// No group settings or save behavior are changed here.
pub(super) fn color_menu_option(
    ui: &mut Ui,
    color_key: Option<&str>,
    label: &str,
    selected: bool,
) -> bool {
    let dark = ui.visuals().dark_mode;
    let foreground = color_key
        .map(|key| swatch_color(Some(key), dark))
        .unwrap_or_else(|| ui.visuals().text_color());
    let size = egui::vec2(92.0, 18.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let background = if selected {
            ui.visuals().selection.bg_fill
        } else if response.hovered() {
            ui.visuals().widgets.hovered.weak_bg_fill
        } else {
            Color32::TRANSPARENT
        };
        if background != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, 3.0, background);
        }

        let center_y = rect.center().y;
        if color_key.is_some() {
            // Solid, small rectangular swatch (rather than an outlined glyph).
            let swatch = egui::Rect::from_center_size(
                egui::pos2(rect.left() + 12.0, center_y),
                egui::vec2(14.0, 10.0),
            );
            ui.painter().rect_filled(swatch, 2.0, foreground);
        }
        let font = egui::TextStyle::Body.resolve(ui.style());
        ui.painter().text(
            egui::pos2(rect.left() + 26.0, center_y),
            egui::Align2::LEFT_CENTER,
            label,
            font.clone(),
            foreground,
        );
        if selected {
            // Draw the check mark ourselves: the UI font may not contain U+2713.
            let color = ui.visuals().text_color();
            let center = egui::pos2(rect.right() - 9.0, center_y);
            let stroke = egui::Stroke::new(1.5_f32, color);
            ui.painter().line_segment(
                [
                    center + egui::vec2(-4.0, 0.0),
                    center + egui::vec2(-1.0, 3.0),
                ],
                stroke,
            );
            ui.painter().line_segment(
                [
                    center + egui::vec2(-1.0, 3.0),
                    center + egui::vec2(4.0, -3.0),
                ],
                stroke,
            );
        }
    }
    response.clicked()
}
