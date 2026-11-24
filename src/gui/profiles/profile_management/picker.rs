//! Inline profile selection without nested popups.
use crate::gui::named_combobox::NamedEntries;

// Inline, scrollable picker: a nested egui popup would close Profile Management.
fn profile_option_row(ui: &mut egui::Ui, name: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Sense::click(),
    );
    if ui.is_rect_visible(rect) {
        let highlighted = selected || response.hovered() || response.has_focus();
        if highlighted {
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(2),
                ui.visuals().selection.bg_fill,
            );
        }
        let text_color = if highlighted {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            egui::pos2(rect.left() + 6.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::TextStyle::Button.resolve(ui.style()),
            text_color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub(super) fn choose_profile<E, N>(
    ui: &mut egui::Ui,
    entries: &mut N,
    target: &mut String,
    picker_open: &mut bool,
) -> bool
where
    N: NamedEntries<E>,
{
    if !target.is_empty() && !entries.contains(target) {
        target.clear();
    }
    let response = ui.add(
        egui::Button::new(if target.is_empty() {
            "Choose profile"
        } else {
            target.as_str()
        })
        .min_size(egui::vec2(135.0, ui.spacing().interact_size.y)),
    );
    if response.clicked() {
        *picker_open = !*picker_open;
    }
    crate::gui::widget_style::paint_picker_arrow(
        ui,
        response.rect,
        *picker_open,
        ui.visuals().text_color(),
    );
    if !*picker_open {
        return false;
    }
    let mut chosen = None;
    crate::gui::widget_style::picker_scroll()
        .show(ui, |ui| {
            for (name, _) in entries.entries() {
                if profile_option_row(ui, name, target.as_str() == name.as_str()).clicked() {
                    chosen = Some(name.clone());
                }
            }
        });
    if let Some(name) = chosen {
        *target = name;
        *picker_open = false;
        return true;
    }
    false
}
