use std::collections::HashMap;

use eframe::egui::{
    self, Align2, Color32, ColorImage, Key, KeyboardShortcut, Modifiers, PointerButton, Pos2, Rect,
    Sense, Stroke, TextureFilter, TextureHandle, TextureOptions, TextureWrapMode, Vec2,
};

use crate::canvas::{self, IRect, Layer, LayerId, PaintMode, PaintOp, TILE, TileKey};
use crate::history::{Action, History};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Brush,
    Eraser,
    Fill,
    Circle,
}

impl Tool {
    const ALL: [(Tool, &'static str, Key); 4] = [
        (Tool::Brush, "Brush", Key::B),
        (Tool::Eraser, "Eraser", Key::E),
        (Tool::Fill, "Fill", Key::G),
        (Tool::Circle, "Circle", Key::C),
    ];
}

/// An in-progress interaction with the canvas.
enum Gesture {
    Stroke { op: PaintOp, last: [f32; 2] },
    Circle { center: [f32; 2] },
    Pan { button: PointerButton, last: Pos2 },
}

const PALETTE: [Color32; 12] = [
    Color32::BLACK,
    Color32::from_gray(128),
    Color32::WHITE,
    Color32::from_rgb(220, 40, 40),
    Color32::from_rgb(245, 140, 30),
    Color32::from_rgb(250, 215, 40),
    Color32::from_rgb(60, 180, 75),
    Color32::from_rgb(40, 190, 200),
    Color32::from_rgb(40, 90, 220),
    Color32::from_rgb(140, 60, 200),
    Color32::from_rgb(235, 100, 170),
    Color32::from_rgb(120, 75, 40),
];

const TEXTURE_OPTIONS: TextureOptions = TextureOptions {
    magnification: TextureFilter::Nearest,
    minification: TextureFilter::Linear,
    wrap_mode: TextureWrapMode::ClampToEdge,
    mipmap_mode: None,
};

pub struct PaintApp {
    layers: Vec<Layer>,
    /// Index into `layers` (0 = bottom).
    active: usize,
    next_layer_id: LayerId,
    history: History,

    tool: Tool,
    color: Color32,
    size: f32,
    opacity: f32,
    fill_tolerance: u8,
    fill_shapes: bool,

    /// Canvas coordinate shown at the top-left corner of the canvas area.
    offset: Vec2,
    zoom: f32,

    gesture: Option<Gesture>,
    textures: HashMap<(LayerId, TileKey), TextureHandle>,
}

impl Default for PaintApp {
    fn default() -> Self {
        Self {
            layers: vec![Layer::new(1, "Layer 1")],
            active: 0,
            next_layer_id: 2,
            history: History::new(200),
            tool: Tool::Brush,
            color: Color32::BLACK,
            size: 8.0,
            opacity: 1.0,
            fill_tolerance: 32,
            fill_shapes: false,
            offset: Vec2::ZERO,
            zoom: 1.0,
            gesture: None,
            textures: HashMap::new(),
        }
    }
}

impl eframe::App for PaintApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas_area(ui));
        self.brush_window(&ctx);
        self.layers_window(&ctx);
    }
}

impl PaintApp {
    fn active_layer_id(&self) -> LayerId {
        self.layers[self.active].id
    }

    fn undo(&mut self) {
        self.finish_gesture();
        self.history.undo(&mut self.layers);
        self.clamp_active();
    }

    fn redo(&mut self) {
        self.finish_gesture();
        self.history.redo(&mut self.layers);
        self.clamp_active();
    }

    fn clamp_active(&mut self) {
        self.active = self.active.min(self.layers.len().saturating_sub(1));
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let redo_a = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        let redo_b = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let (do_redo, do_undo) = ctx.input_mut(|i| {
            let redo = i.consume_shortcut(&redo_a) || i.consume_shortcut(&redo_b);
            (redo, i.consume_shortcut(&undo))
        });
        if do_redo {
            self.redo();
        } else if do_undo {
            self.undo();
        }
        for (tool, _, key) in Tool::ALL {
            if ctx.input(|i| i.modifiers.is_none() && i.key_pressed(key)) {
                self.tool = tool;
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tool, name, key) in Tool::ALL {
                ui.selectable_value(&mut self.tool, tool, name)
                    .on_hover_text(format!("{name} ({})", key.name()));
            }
            ui.separator();

            egui::color_picker::color_edit_button_srgba(
                ui,
                &mut self.color,
                egui::color_picker::Alpha::Opaque,
            );
            for c in PALETTE {
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                let selected = c == self.color;
                ui.painter().rect_filled(rect, 3.0, c);
                ui.painter().rect_stroke(
                    rect,
                    3.0,
                    if selected {
                        Stroke::new(2.0, ui.visuals().selection.bg_fill)
                    } else {
                        Stroke::new(1.0, Color32::from_gray(90))
                    },
                    egui::StrokeKind::Inside,
                );
                if resp.clicked() {
                    self.color = c;
                }
            }
            ui.separator();

            if ui
                .add_enabled(self.history.can_undo(), egui::Button::new("Undo"))
                .on_hover_text("Ctrl+Z")
                .clicked()
            {
                self.undo();
            }
            if ui
                .add_enabled(self.history.can_redo(), egui::Button::new("Redo"))
                .on_hover_text("Ctrl+Shift+Z / Ctrl+Y")
                .clicked()
            {
                self.redo();
            }
            ui.separator();

            ui.label(format!("{:.0}%", self.zoom * 100.0));
            if ui.button("Reset view").clicked() {
                self.offset = Vec2::ZERO;
                self.zoom = 1.0;
            }
        });
    }

    fn brush_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("Brush")
            .title_bar(false)
            .resizable(false)
            .anchor(Align2::LEFT_CENTER, [10.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label("Size");
                        ui.add(
                            egui::Slider::new(&mut self.size, 1.0..=300.0)
                                .vertical()
                                .logarithmic(true)
                                .integer(),
                        );
                    });
                    ui.vertical(|ui| {
                        ui.label("Opacity");
                        let mut pct = self.opacity * 100.0;
                        ui.add(
                            egui::Slider::new(&mut pct, 1.0..=100.0)
                                .vertical()
                                .integer()
                                .suffix("%"),
                        );
                        self.opacity = pct / 100.0;
                    });
                });
                match self.tool {
                    Tool::Fill => {
                        ui.separator();
                        ui.label("Tolerance");
                        ui.add(egui::Slider::new(&mut self.fill_tolerance, 0..=255));
                    }
                    Tool::Circle => {
                        ui.separator();
                        ui.checkbox(&mut self.fill_shapes, "Filled");
                    }
                    _ => {}
                }
            });
    }

    fn layers_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("Layers")
            .resizable(false)
            .collapsible(true)
            .anchor(Align2::RIGHT_TOP, [-10.0, 40.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("+").on_hover_text("New layer").clicked() {
                        self.add_layer();
                    }
                    if ui
                        .add_enabled(self.layers.len() > 1, egui::Button::new("−"))
                        .on_hover_text("Delete layer")
                        .clicked()
                    {
                        self.history
                            .perform(Action::RemoveLayer { index: self.active }, &mut self.layers);
                        self.clamp_active();
                    }
                    let top = self.layers.len() - 1;
                    if ui
                        .add_enabled(self.active < top, egui::Button::new("⬆"))
                        .on_hover_text("Move up")
                        .clicked()
                    {
                        self.move_layer(self.active + 1);
                    }
                    if ui
                        .add_enabled(self.active > 0, egui::Button::new("⬇"))
                        .on_hover_text("Move down")
                        .clicked()
                    {
                        self.move_layer(self.active - 1);
                    }
                });
                ui.separator();

                // Topmost layer first.
                for i in (0..self.layers.len()).rev() {
                    ui.horizontal(|ui| {
                        let layer = &mut self.layers[i];
                        ui.checkbox(&mut layer.visible, "").on_hover_text("Visible");
                        if ui
                            .selectable_label(i == self.active, layer.name.as_str())
                            .clicked()
                        {
                            self.active = i;
                        }
                    });
                }
                ui.separator();

                let layer = &mut self.layers[self.active];
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut layer.name).desired_width(120.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Opacity");
                    ui.add(egui::Slider::new(&mut layer.opacity, 0.0..=1.0));
                });
            });
    }

    fn add_layer(&mut self) {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        let index = self.active + 1;
        let layer = Box::new(Layer::new(id, format!("Layer {id}")));
        self.history
            .perform(Action::InsertLayer { index, layer }, &mut self.layers);
        self.active = index;
    }

    fn move_layer(&mut self, to: usize) {
        self.history.perform(
            Action::MoveLayer {
                from: self.active,
                to,
            },
            &mut self.layers,
        );
        self.active = to;
    }

    fn to_canvas(&self, rect: Rect, p: Pos2) -> [f32; 2] {
        let v = (p - rect.min) / self.zoom + self.offset;
        [v.x, v.y]
    }

    fn to_screen(&self, rect: Rect, p: [f32; 2]) -> Pos2 {
        rect.min + (Vec2::new(p[0], p[1]) - self.offset) * self.zoom
    }

    fn visible_canvas_rect(&self, rect: Rect) -> IRect {
        let a = self.to_canvas(rect, rect.min);
        let b = self.to_canvas(rect, rect.max);
        IRect {
            x0: a[0].floor() as i32,
            y0: a[1].floor() as i32,
            x1: b[0].ceil() as i32,
            y1: b[1].ceil() as i32,
        }
    }

    fn canvas_area(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        self.handle_view_input(ui, rect, &response);
        self.handle_tool_input(ui, rect);
        self.sync_textures(ui.ctx());
        self.paint_canvas(ui, rect, &response);
    }

    fn handle_view_input(&mut self, ui: &egui::Ui, rect: Rect, response: &egui::Response) {
        if !response.hovered() {
            return;
        }
        let (scroll, pinch, pointer) =
            ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
        let factor = pinch * (scroll.y * 0.002).exp();
        if factor != 1.0 {
            let anchor = pointer.unwrap_or(rect.center());
            let before = self.to_canvas(rect, anchor);
            self.zoom = (self.zoom * factor).clamp(0.02, 64.0);
            let after = self.to_canvas(rect, anchor);
            self.offset += Vec2::new(before[0] - after[0], before[1] - after[1]);
        }
    }

    fn handle_tool_input(&mut self, ui: &egui::Ui, rect: Rect) {
        // Walk the raw events rather than the end-of-frame pointer state so
        // every motion sample lands in the stroke and quick press/release
        // pairs within one frame aren't lost.
        let (events, space) = ui.input(|i| (i.events.clone(), i.key_down(Key::Space)));
        let ctx = ui.ctx();
        for event in events {
            match event {
                egui::Event::PointerMoved(pos) => self.pointer_moved(rect, pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    ..
                } => {
                    let on_canvas =
                        rect.contains(pos) && ctx.layer_id_at(pos) == Some(ui.layer_id());
                    if self.gesture.is_some() || !on_canvas {
                        continue;
                    }
                    match button {
                        PointerButton::Middle => {
                            self.gesture = Some(Gesture::Pan { button, last: pos })
                        }
                        PointerButton::Primary if space => {
                            self.gesture = Some(Gesture::Pan { button, last: pos })
                        }
                        PointerButton::Primary => self.begin_tool(rect, pos),
                        _ => {}
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: false,
                    ..
                } => {
                    let ends = match &self.gesture {
                        Some(Gesture::Pan { button: b, .. }) => *b == button,
                        Some(_) => button == PointerButton::Primary,
                        None => false,
                    };
                    if ends {
                        self.pointer_moved(rect, pos);
                        self.finish_gesture_at(rect, Some(pos));
                    }
                }
                _ => {}
            }
        }
    }

    fn pointer_moved(&mut self, rect: Rect, pos: Pos2) {
        let p = self.to_canvas(rect, pos);
        match &mut self.gesture {
            Some(Gesture::Pan { last, .. }) => {
                self.offset -= (pos - *last) / self.zoom;
                *last = pos;
            }
            Some(Gesture::Stroke { op, last }) if p != *last => {
                if let Some(layer) = self.layers.iter_mut().find(|l| l.id == op.layer_id) {
                    op.stroke_segment(layer, *last, p, self.size / 2.0);
                }
                *last = p;
            }
            _ => {}
        }
    }

    fn paint_mode(&self) -> PaintMode {
        match self.tool {
            Tool::Eraser => PaintMode::Erase {
                strength: self.opacity,
            },
            _ => PaintMode::Paint {
                color: self.color.to_array(),
                opacity: self.opacity,
            },
        }
    }

    fn begin_tool(&mut self, rect: Rect, pos: Pos2) {
        let p = self.to_canvas(rect, pos);
        if !self.layers[self.active].visible {
            // Painting on a hidden layer would be invisible and confusing.
            return;
        }
        match self.tool {
            Tool::Brush | Tool::Eraser => {
                let mut op = PaintOp::new(self.active_layer_id(), self.paint_mode());
                op.stroke_segment(&mut self.layers[self.active], p, p, self.size / 2.0);
                self.gesture = Some(Gesture::Stroke { op, last: p });
            }
            Tool::Circle => self.gesture = Some(Gesture::Circle { center: p }),
            Tool::Fill => {
                let mut limit = self.visible_canvas_rect(rect);
                let layer = &mut self.layers[self.active];
                if let Some(b) = layer.bounds() {
                    limit = limit.union(&b);
                }
                let (x, y) = (p[0].floor() as i32, p[1].floor() as i32);
                let color = self.color.to_array();
                if let Some(tiles) =
                    canvas::flood_fill(layer, x, y, limit, self.fill_tolerance, color, self.opacity)
                {
                    self.history.push(Action::Tiles {
                        layer_id: layer.id,
                        tiles,
                    });
                }
            }
        }
    }

    fn finish_gesture(&mut self) {
        // Commit without a final pointer position (e.g. undo pressed mid-stroke).
        if let Some(Gesture::Stroke { op, .. }) = self.gesture.take() {
            self.commit(op);
        }
    }

    fn finish_gesture_at(&mut self, rect: Rect, pos: Option<Pos2>) {
        match self.gesture.take() {
            Some(Gesture::Stroke { op, .. }) => self.commit(op),
            Some(Gesture::Circle { center }) => {
                let Some(pos) = pos else { return };
                let p = self.to_canvas(rect, pos);
                let radius = ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)).sqrt();
                if radius < 0.5 {
                    return;
                }
                let mut op = PaintOp::new(self.active_layer_id(), self.paint_mode());
                op.circle(
                    &mut self.layers[self.active],
                    center,
                    radius,
                    self.size,
                    self.fill_shapes,
                );
                self.commit(op);
            }
            Some(Gesture::Pan { .. }) | None => {}
        }
    }

    fn commit(&mut self, op: PaintOp) {
        let layer_id = op.layer_id;
        if let Some(tiles) = op.finish() {
            self.history.push(Action::Tiles { layer_id, tiles });
        }
    }

    /// Uploads changed tiles to the GPU and drops textures of deleted layers/tiles.
    fn sync_textures(&mut self, ctx: &egui::Context) {
        for layer in &mut self.layers {
            for key in layer.dirty.drain() {
                let tex_key = (layer.id, key);
                match layer.tiles.get(&key) {
                    Some(tile) => {
                        let image = ColorImage::from_rgba_unmultiplied(
                            [TILE as usize, TILE as usize],
                            tile.as_flattened(),
                        );
                        match self.textures.get_mut(&tex_key) {
                            Some(tex) => tex.set(image, TEXTURE_OPTIONS),
                            None => {
                                let name = format!("tile-{}-{}-{}", layer.id, key.0, key.1);
                                let tex = ctx.load_texture(name, image, TEXTURE_OPTIONS);
                                self.textures.insert(tex_key, tex);
                            }
                        }
                    }
                    None => {
                        self.textures.remove(&tex_key);
                    }
                }
            }
        }
        let layers = &self.layers;
        self.textures.retain(|(id, key), _| {
            layers
                .iter()
                .any(|l| l.id == *id && l.tiles.contains_key(key))
        });
    }

    fn paint_canvas(&self, ui: &egui::Ui, rect: Rect, response: &egui::Response) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::WHITE);

        let view = self.visible_canvas_rect(rect);
        let (tx0, ty0) = canvas::tile_key(view.x0, view.y0);
        let (tx1, ty1) = canvas::tile_key(view.x1, view.y1);
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        for layer in self.layers.iter().filter(|l| l.visible && l.opacity > 0.0) {
            let tint = Color32::from_white_alpha((layer.opacity * 255.0).round() as u8);
            for &key in layer.tiles.keys() {
                if key.0 < tx0 || key.0 > tx1 || key.1 < ty0 || key.1 > ty1 {
                    continue;
                }
                let Some(tex) = self.textures.get(&(layer.id, key)) else {
                    continue;
                };
                let min = [(key.0 * TILE) as f32, (key.1 * TILE) as f32];
                let max = [min[0] + TILE as f32, min[1] + TILE as f32];
                let screen =
                    Rect::from_min_max(self.to_screen(rect, min), self.to_screen(rect, max));
                painter.image(tex.id(), screen, uv, tint);
            }
        }

        let outline = Stroke::new(1.0, Color32::from_gray(120));
        // Circle preview.
        if let Some(Gesture::Circle { center }) = &self.gesture
            && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        {
            let c = self.to_screen(rect, *center);
            let r = c.distance(pos);
            let color = self.color.gamma_multiply(self.opacity);
            if self.fill_shapes {
                painter.circle_filled(c, r, color);
            } else {
                painter.circle_stroke(c, r, Stroke::new(self.size * self.zoom, color));
            }
        }

        // Brush outline under the cursor.
        if let Some(pos) = response.hover_pos() {
            match self.tool {
                Tool::Brush | Tool::Eraser => {
                    painter.circle_stroke(pos, (self.size / 2.0 * self.zoom).max(1.0), outline);
                }
                Tool::Circle | Tool::Fill => {
                    painter.line_segment([pos - Vec2::X * 6.0, pos + Vec2::X * 6.0], outline);
                    painter.line_segment([pos - Vec2::Y * 6.0, pos + Vec2::Y * 6.0], outline);
                }
            }
        }
    }
}
