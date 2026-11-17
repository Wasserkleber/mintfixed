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

fn management_mix(base: egui::Color32, tint: egui::Color32, amount: u16) -> egui::Color32 {
    let amount = amount.min(100);
    let rest = 100 - amount;
    egui::Color32::from_rgb(
        ((base.r() as u16 * rest + tint.r() as u16 * amount) / 100) as u8,
        ((base.g() as u16 * rest + tint.g() as u16 * amount) / 100) as u8,
        ((base.b() as u16 * rest + tint.b() as u16 * amount) / 100) as u8,
    )
}

fn management_text(ui: &Ui, tint: egui::Color32) -> egui::Color32 {
    // Keep even yellow and other light colors legible in light themes.
    management_mix(
        ui.visuals().text_color(),
        tint,
        if ui.visuals().dark_mode { 55 } else { 45 },
    )
}

/// Shared colored selector for Rename, Duplicate, Export, and Delete.
/// It styles the label, button fill, border, and chevron together.
fn group_picker_button(
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
            .fill(management_mix(
                ui.visuals().widgets.inactive.weak_bg_fill,
                tint,
                if ui.visuals().dark_mode { 23 } else { 16 },
            ))
            .stroke(egui::Stroke::new(
                1.0_f32,
                management_mix(ui.visuals().widgets.inactive.bg_stroke.color, tint, 55),
            ));
    }
    let response = ui.add(button).on_hover_text(tip);
    if response.clicked() {
        *open = !*open;
    }
    let x = response.rect.right() - 12.0;
    let y = response.rect.center().y;
    let arrow = if *open {
        vec![
            egui::pos2(x - 4.0, y + 2.0),
            egui::pos2(x + 4.0, y + 2.0),
            egui::pos2(x, y - 2.0),
        ]
    } else {
        vec![
            egui::pos2(x - 4.0, y - 2.0),
            egui::pos2(x + 4.0, y - 2.0),
            egui::pos2(x, y + 2.0),
        ]
    };
    ui.painter().add(egui::Shape::convex_polygon(
        arrow,
        accent.unwrap_or_else(|| ui.visuals().text_color()),
        egui::Stroke::NONE,
    ));
    response
}

/// Action buttons retain normal theme styling, including disabled states.
fn group_action_button(
    ui: &mut Ui,
    label: &str,
    _tint: Option<&str>,
    enabled: bool,
) -> egui::Response {
    ui.add_enabled(enabled, Button::new(label))
}
/// Colored dropdown entries: subtle row tint, stronger selected/hover tint,
/// a matching swatch, and hue-aware but readable name text.
fn group_option_row(ui: &mut Ui, name: &str, selected: bool, tint: Option<&str>) -> egui::Response {
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
                management_mix(ui.visuals().widgets.inactive.weak_bg_fill, tint, amount),
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
fn group_picker_scroll() -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .max_height(140.0)
        .scroll_bar_visibility(egui::containers::scroll_area::ScrollBarVisibility::AlwaysVisible)
}

