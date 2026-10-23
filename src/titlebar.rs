//! The toolbar doubles as the window's title bar, like in VS Code: it can be
//! dragged to move the window, double-clicked to maximize, and on Windows and
//! Linux it has its own minimize / maximize / close buttons and resize edges.
//! On macOS the native traffic lights stay, drawn over the toolbar's left end.

use eframe::egui::{
    self, Align, Color32, CursorIcon, Layout, Pos2, Rect, ResizeDirection, Sense, Stroke,
    UiBuilder, Vec2, ViewportCommand,
};

/// Space left free for the macOS traffic lights.
const MAC_TRAFFIC_LIGHTS: f32 = 72.0;
const BUTTON_WIDTH: f32 = 40.0;
/// How far from the window edge the resize cursor appears.
const RESIZE_MARGIN: f32 = 5.0;

/// Whether this platform draws its own window controls (instead of keeping
/// the native ones, as on macOS).
pub const DRAWS_CONTROLS: bool = !cfg!(target_os = "macos");

/// Sets up the window so the toolbar can take over the title bar.
pub fn configure(builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    if DRAWS_CONTROLS {
        builder.with_decorations(false)
    } else {
        builder
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false)
    }
}

/// Lays out a title bar around `content` (the toolbar). With `custom` off the
/// toolbar is shown as-is, below the system title bar.
pub fn show(ui: &mut egui::Ui, custom: bool, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    if !custom {
        content(ui);
        return;
    }
    // As tall as the toolbar's icon buttons.
    let row_height = 28.0;
    // Senses below the toolbar's own widgets: only the empty parts drag.
    let bar = ui.scope_builder(
        UiBuilder::new()
            .sense(Sense::click_and_drag())
            .layout(Layout::right_to_left(Align::Min)),
        |ui| {
            ui.set_min_height(row_height);
            let buttons = DRAWS_CONTROLS.then(|| window_buttons(ui, row_height));
            let tools = ui
                .with_layout(Layout::left_to_right(Align::Min), |ui| {
                    if DRAWS_CONTROLS {
                        app_icon(ui, row_height);
                    } else {
                        ui.add_space(MAC_TRAFFIC_LIGHTS);
                    }
                    content(ui);
                })
                .response
                .rect;
            (tools, buttons)
        },
    );
    let (tools, buttons) = bar.inner;
    let bar_rect = bar.response.rect;
    paint_title(
        ui,
        title,
        bar_rect,
        tools.right(),
        buttons.map_or(bar_rect.right(), |b| b.left()),
    );

    let response = bar.response;
    if response.double_clicked() {
        toggle_maximized(ui.ctx());
    } else if response.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// The app's icon at the start of the bar, like in VS Code.
fn app_icon(ui: &mut egui::Ui, height: f32) {
    let id = egui::Id::unique("app_icon_texture");
    let texture = ui.data_mut(|d| d.get_temp::<egui::TextureHandle>(id));
    let texture = texture.unwrap_or_else(|| {
        let png = include_bytes!("../assets/icons/64.png");
        let image = image::load_from_memory(png)
            .expect("bundled icon is a valid PNG")
            .into_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        let texture = ui
            .ctx()
            .load_texture("app_icon", image, egui::TextureOptions::LINEAR);
        ui.data_mut(|d| d.insert_temp(id, texture.clone()));
        texture
    });
    let (rect, _) = ui.allocate_exact_size(Vec2::new(height + 4.0, height), Sense::hover());
    let icon = Rect::from_center_size(rect.center(), Vec2::splat(20.0));
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    ui.painter().image(texture.id(), icon, uv, Color32::WHITE);
}

/// Draws the title centered on the bar, or in the free space between the
/// tools and the buttons, or not at all when it doesn't fit.
fn paint_title(ui: &egui::Ui, title: &str, bar: Rect, free_left: f32, free_right: f32) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().weak_text_color();
    let galley = ui.painter().layout_no_wrap(title.to_owned(), font, color);
    let width = galley.size().x;
    let (left, right) = (free_left + 16.0, free_right - 16.0);
    if right - left < width {
        return;
    }
    let centered = bar.center().x - width / 2.0;
    let x = centered.clamp(left, right - width);
    let pos = Pos2::new(x, bar.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, color);
}

fn is_maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

fn toggle_maximized(ctx: &egui::Context) {
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_maximized(ctx)));
}

#[derive(Clone, Copy)]
enum Control {
    Minimize,
    Maximize,
    Close,
}

/// Close, maximize and minimize, laid out right to left. Returns their area.
fn window_buttons(ui: &mut egui::Ui, height: f32) -> Rect {
    let spacing = std::mem::take(&mut ui.spacing_mut().item_spacing.x);
    let ctx = ui.ctx().clone();
    let maximized = is_maximized(&ctx);
    let mut area = Rect::NOTHING;
    for (control, tip) in [
        (Control::Close, "Close"),
        (
            Control::Maximize,
            if maximized { "Restore" } else { "Maximize" },
        ),
        (Control::Minimize, "Minimize"),
    ] {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(BUTTON_WIDTH, height), Sense::click());
        area = area.union(rect);
        let visuals = ui.visuals();
        let hovered = response.hovered();
        if hovered {
            let fill = match control {
                Control::Close => Color32::from_rgb(196, 43, 28),
                _ => visuals.widgets.hovered.weak_bg_fill,
            };
            ui.painter().rect_filled(rect, 0.0, fill);
        }
        let color = if hovered && matches!(control, Control::Close) {
            Color32::WHITE
        } else {
            visuals.text_color()
        };
        paint_control(
            ui.painter(),
            rect.center(),
            control,
            maximized,
            Stroke::new(1.0, color),
        );
        if response.on_hover_text(tip).clicked() {
            match control {
                Control::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
                Control::Maximize => toggle_maximized(&ctx),
                Control::Minimize => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
            }
        }
    }
    ui.spacing_mut().item_spacing.x = spacing;
    area
}

fn paint_control(
    painter: &egui::Painter,
    c: Pos2,
    control: Control,
    maximized: bool,
    stroke: Stroke,
) {
    let r = 5.0;
    match control {
        Control::Minimize => {
            painter.line_segment([c + Vec2::new(-r, 0.5), c + Vec2::new(r, 0.5)], stroke);
        }
        Control::Maximize if maximized => {
            // Two overlapping windows.
            let back = Rect::from_min_max(c + Vec2::new(-r + 2.0, -r), c + Vec2::new(r, r - 2.0));
            let front = back.translate(Vec2::new(-2.0, 2.0));
            painter.line_segment([back.left_top(), back.right_top()], stroke);
            painter.line_segment([back.right_top(), back.right_bottom()], stroke);
            painter.rect_stroke(front, 0.0, stroke, egui::StrokeKind::Middle);
        }
        Control::Maximize => {
            let rect = Rect::from_center_size(c, Vec2::splat(2.0 * r));
            painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Middle);
        }
        Control::Close => {
            painter.line_segment([c + Vec2::new(-r, -r), c + Vec2::new(r, r)], stroke);
            painter.line_segment([c + Vec2::new(-r, r), c + Vec2::new(r, -r)], stroke);
        }
    }
}

/// Without system decorations the window has no borders to resize it by, so
/// the outermost pixels of the window act as them. Returns the edge at `pos`.
pub fn edge_at(ctx: &egui::Context, pos: Pos2) -> Option<ResizeDirection> {
    let fixed = ctx.input(|i| {
        i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
    });
    let screen = ctx.content_rect();
    if fixed || !screen.contains(pos) {
        return None;
    }
    // Corners reach further along the edges so they're easy to grab.
    let near = |d: f32, margin: f32| d < margin;
    let (left, right) = (pos.x - screen.left(), screen.right() - pos.x);
    let (top, bottom) = (pos.y - screen.top(), screen.bottom() - pos.y);
    let corner = 3.0 * RESIZE_MARGIN;
    let (w, e) = (near(left, RESIZE_MARGIN), near(right, RESIZE_MARGIN));
    let (n, s) = (near(top, RESIZE_MARGIN), near(bottom, RESIZE_MARGIN));
    let (wc, ec) = (near(left, corner), near(right, corner));
    let (nc, sc) = (near(top, corner), near(bottom, corner));
    use ResizeDirection::*;
    Some(match () {
        _ if (n && wc) || (w && nc) => NorthWest,
        _ if (n && ec) || (e && nc) => NorthEast,
        _ if (s && wc) || (w && sc) => SouthWest,
        _ if (s && ec) || (e && sc) => SouthEast,
        _ if n => North,
        _ if s => South,
        _ if w => West,
        _ if e => East,
        _ => return None,
    })
}

/// Shows the resize cursor on the window edges and starts resizing when one
/// is pressed. Called last so the cursor wins over the widgets underneath.
pub fn resize_edges(ctx: &egui::Context) {
    let (hover, presses) = ctx.input(|i| {
        let presses: Vec<Pos2> = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => Some(*pos),
                _ => None,
            })
            .collect();
        (i.pointer.hover_pos(), presses)
    });
    if let Some(direction) = hover.and_then(|pos| edge_at(ctx, pos)) {
        ctx.set_cursor_icon(cursor(direction));
    }
    if let Some(direction) = presses.into_iter().find_map(|pos| edge_at(ctx, pos)) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}

fn cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    }
}
