use super::*;

/// A full-width dropdown row with the name left-aligned.
/// The whole row receives hover highlighting and click input.
/// Use exactly the same saved tint and palette as the mod list.
fn management_group_accent(ui: &Ui, tint: Option<&str>) -> Option<egui::Color32> {
    let tint = tint?;
    if !mod_list_style::COLOR_OPTIONS
        .iter()
        .any(|(name, _)| *name == tint)
    {
        return None;
    }
    Some(mod_list_style::swatch_color(
        Some(tint),
        ui.visuals().dark_mode,
    ))
}

fn management_text(ui: &Ui, tint: egui::Color32) -> egui::Color32 {
    // Keep even yellow and other light colors legible in light themes.
    crate::gui::widget_style::blend(
        ui.visuals().text_color(),
        tint,
        if ui.visuals().dark_mode { 55 } else { 45 },
    )
}

/// Shared colored selector for Rename, Duplicate, Export, and Delete.
/// It styles the label, button fill, border, and chevron together.
pub(super) fn group_picker_button(
    ui: &mut Ui,
    selected: &str,
    tint: Option<&str>,
    open: &mut bool,
    tip: &str,
) -> egui::Response {
    let accent = management_group_accent(ui, tint);
    let label = format!(
        "{}    ",
        if selected.is_empty() {
            "Choose group"
        } else {
            selected
        }
    );
    let mut text = RichText::new(label);
    if let Some(tint) = accent {
        text = text.color(management_text(ui, tint));
    }
    let mut button = Button::new(text).min_size(egui::vec2(110.0, ui.spacing().interact_size.y));
    if let Some(tint) = accent {
        button = button
            .fill(crate::gui::widget_style::blend(
                ui.visuals().widgets.inactive.weak_bg_fill,
                tint,
                if ui.visuals().dark_mode { 23 } else { 16 },
            ))
            .stroke(egui::Stroke::new(
                1.0_f32,
                crate::gui::widget_style::blend(ui.visuals().widgets.inactive.bg_stroke.color, tint, 55),
            ));
    }
    let response = ui.add(button).on_hover_text(tip);
    if response.clicked() {
        *open = !*open;
    }
    crate::gui::widget_style::paint_picker_arrow(
        ui,
        response.rect,
        *open,
        accent.unwrap_or_else(|| ui.visuals().text_color()),
    );
    response
}

/// Action buttons retain normal theme styling, including disabled states.
pub(super) fn group_action_button(
    ui: &mut Ui,
    label: &str,
    enabled: bool,
) -> egui::Response {
    ui.add_enabled(enabled, Button::new(label))
}
/// Colored dropdown entries: subtle row tint, stronger selected/hover tint,
/// a matching swatch, and hue-aware but readable name text.
pub(super) fn group_option_row(ui: &mut Ui, name: &str, selected: bool, tint: Option<&str>) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let highlighted = selected || response.hovered() || response.has_focus();
        let accent = management_group_accent(ui, tint);
        if let Some(tint) = accent {
            let amount = if selected {
                27
            } else if highlighted {
                19
            } else if ui.visuals().dark_mode {
                12
            } else {
                8
            };
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(2),
                crate::gui::widget_style::blend(ui.visuals().widgets.inactive.weak_bg_fill, tint, amount),
            );
        } else if highlighted {
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(2),
                ui.visuals().selection.bg_fill,
            );
        }
        let dot = accent.unwrap_or_else(|| {
            if ui.visuals().dark_mode {
                egui::Color32::from_gray(120)
            } else {
                egui::Color32::from_gray(150)
            }
        });
        ui.painter()
            .circle_filled(egui::pos2(rect.left() + 10.0, rect.center().y), 4.0, dot);
        let text_color = if let Some(tint) = accent {
            management_text(ui, tint)
        } else if highlighted {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            egui::pos2(rect.left() + 20.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::TextStyle::Button.resolve(ui.style()),
            text_color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
/// Use the same compact, always-visible scrollbar for every group selector.
pub(super) fn group_picker_scroll() -> egui::ScrollArea {
    crate::gui::widget_style::picker_scroll()
}
