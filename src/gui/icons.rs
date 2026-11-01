use egui::{Color32, Image, Response, Ui, Vec2};

pub const SIZE: f32 = 16.0;
const CORNER_RADIUS: u8 = 3;

#[derive(Clone, Copy)]
pub enum Icon {
    Add,
    Delete,
    Settings,
    Copy,
    Duplicate,
    Drag,
    Folder,
    Web,
    Warning,
    Error,
    Light,
    Dark,
    System,
}

impl Icon {
    pub fn image(self, ui: &Ui, color: Color32) -> Image<'static> {
        let bytes: &[u8] = match self {
            Self::Add => include_bytes!("../../assets/icons/add.svg"),
            Self::Delete => include_bytes!("../../assets/icons/delete.svg"),
            Self::Settings => include_bytes!("../../assets/icons/settings.svg"),
            Self::Copy => include_bytes!("../../assets/icons/content_copy.svg"),
            Self::Duplicate => include_bytes!("../../assets/icons/file_copy.svg"),
            Self::Drag => include_bytes!("../../assets/icons/drag_handle.svg"),
            Self::Folder => include_bytes!("../../assets/icons/folder.svg"),
            Self::Web => include_bytes!("../../assets/icons/language.svg"),
            Self::Warning => include_bytes!("../../assets/icons/warning.svg"),
            Self::Error => include_bytes!("../../assets/icons/error.svg"),
            Self::Light => include_bytes!("../../assets/icons/light_mode.svg"),
            Self::Dark => include_bytes!("../../assets/icons/dark_mode.svg"),
            Self::System => include_bytes!("../../assets/icons/desktop_windows.svg"),
        };
        let pixels = (SIZE * ui.ctx().pixels_per_point()).round().max(1.0) as u32;
        let key = egui::Id::new(("material-icon", self as u8));
        let cached = ui
            .ctx()
            .data(|data| data.get_temp::<(u32, egui::TextureHandle)>(key));
        let texture = if let Some((_, texture)) = cached.filter(|(size, _)| *size == pixels) {
            texture
        } else {
            let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default())
                .expect("Bundled icon must be valid SVG");
            let mut pixmap =
                resvg::tiny_skia::Pixmap::new(pixels, pixels).expect("Icon size must be nonzero");
            let transform = resvg::tiny_skia::Transform::from_scale(
                pixels as f32 / tree.size().width(),
                pixels as f32 / tree.size().height(),
            );
            resvg::render(&tree, transform, &mut pixmap.as_mut());
            let image = egui::ColorImage::from_rgba_premultiplied(
                [pixels as usize, pixels as usize],
                pixmap.data(),
            );
            let texture = ui.ctx().load_texture(
                format!("material-icon-{}", self as u8),
                image,
                egui::TextureOptions::LINEAR,
            );
            ui.ctx()
                .data_mut(|data| data.insert_temp(key, (pixels, texture.clone())));
            texture
        };
        Image::new((texture.id(), Vec2::splat(SIZE))).tint(color)
    }
}

pub fn button(ui: &mut Ui, icon: Icon, label: &str) -> Response {
    let width = match icon {
        Icon::Add | Icon::Delete => 24.0,
        _ => 20.0,
    };
    let response = ui.add_sized(
        Vec2::new(width, ui.spacing().interact_size.y),
        egui::Button::new("").frame(false),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    let visuals = ui.style().interact(&response);
    let dark = ui.visuals().dark_mode;
    let (foreground, hover, active) = match icon {
        Icon::Add => {
            if dark {
                (
                    Color32::from_rgb(170, 235, 180),
                    Color32::DARK_GREEN,
                    Color32::from_rgb(0, 80, 0),
                )
            } else {
                (
                    Color32::from_rgb(25, 105, 55),
                    Color32::from_rgb(201, 230, 210),
                    Color32::from_rgb(178, 213, 191),
                )
            }
        }
        Icon::Delete => {
            if dark {
                (
                    Color32::from_rgb(255, 185, 185),
                    Color32::DARK_RED,
                    Color32::from_rgb(110, 0, 0),
                )
            } else {
                (
                    Color32::from_rgb(170, 40, 45),
                    Color32::from_rgb(246, 208, 211),
                    Color32::from_rgb(235, 181, 186),
                )
            }
        }
        Icon::Warning | Icon::Error => (
            ui.visuals().warn_fg_color,
            visuals.weak_bg_fill,
            visuals.weak_bg_fill,
        ),
        _ => (
            visuals.text_color(),
            visuals.weak_bg_fill,
            visuals.weak_bg_fill,
        ),
    };
    let background = if response.is_pointer_button_down_on() {
        active
    } else if response.hovered() {
        hover
    } else {
        visuals.weak_bg_fill
    };
    ui.painter()
        .rect_filled(response.rect, CORNER_RADIUS, background);
    let color = if ui.is_enabled() {
        foreground
    } else {
        ui.visuals().weak_text_color()
    };
    icon.image(ui, color).paint_at(
        ui,
        egui::Rect::from_center_size(response.rect.center(), Vec2::splat(SIZE)),
    );
    paint_focus(ui, &response);
    response.on_hover_text(label)
}

pub fn theme_button(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    let response = ui
        .scope(|ui| {
            let dark = ui.visuals().dark_mode;
            ui.spacing_mut().button_padding = Vec2::new(4.0, 0.0);
            let visuals = ui.visuals_mut();
            visuals.selection.bg_fill = if dark {
                Color32::from_rgb(35, 65, 82)
            } else {
                Color32::from_rgb(198, 222, 240)
            };
            visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            visuals.widgets.hovered.expansion = 0.0;
            visuals.widgets.active.expansion = 0.0;
            ui.add(
                egui::Button::image_and_text(icon.image(ui, ui.visuals().text_color()), label)
                    .selected(selected)
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(CORNER_RADIUS),
            )
        })
        .inner;
    paint_focus(ui, &response);
    response
}

fn paint_focus(ui: &Ui, response: &Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            CORNER_RADIUS,
            egui::Stroke::new(1.0_f32, ui.visuals().selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }
}

pub fn show(ui: &mut Ui, icon: Icon) -> Response {
    let color = match icon {
        Icon::Warning => ui.visuals().warn_fg_color,
        Icon::Error => ui.visuals().error_fg_color,
        _ => ui.visuals().text_color(),
    };
    ui.add(icon.image(ui, color))
}
