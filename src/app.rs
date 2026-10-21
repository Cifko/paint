use std::collections::HashMap;

use eframe::egui::{
    self, Align2, Color32, ColorImage, CursorIcon, Key, KeyboardShortcut, Modifiers, PointerButton,
    Pos2, Rect, Sense, Shape, Stroke, TextureFilter, TextureHandle, TextureOptions,
    TextureWrapMode, Vec2,
};

use crate::canvas::{self, IRect, Image, Layer, LayerId, PaintMode, PaintOp, TILE, TileKey};
use crate::history::{Action, History};
use crate::icons::{self, Icon};
use crate::settings::Settings;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Brush,
    Eraser,
    Fill,
    Rectangle,
    Circle,
    Grid,
    Select,
}

impl Tool {
    const ALL: [(Tool, &'static str, Key, Icon); 7] = [
        (Tool::Brush, "Brush", Key::B, Icon::Brush),
        (Tool::Eraser, "Eraser", Key::E, Icon::Eraser),
        (Tool::Fill, "Fill", Key::G, Icon::Fill),
        (Tool::Rectangle, "Rectangle", Key::R, Icon::Rectangle),
        (Tool::Circle, "Circle", Key::C, Icon::Circle),
        (Tool::Grid, "Grid", Key::T, Icon::Grid),
        (Tool::Select, "Select", Key::M, Icon::Select),
    ];
}

/// Progress of the click-by-click grid tool.
#[derive(Clone, Copy, Debug)]
enum GridState {
    /// First click done; the pointer sizes the cell.
    Cell { origin: [i32; 2] },
    /// Cell fixed; the pointer sizes the whole grid.
    Grid { origin: [i32; 2], cell: [i32; 2] },
}

/// An in-progress interaction with the canvas.
enum Gesture {
    Stroke {
        op: PaintOp,
        last: [f32; 2],
    },
    Circle {
        center: [f32; 2],
    },
    Rectangle {
        start: [i32; 2],
    },
    Select {
        start: [i32; 2],
    },
    /// Dragging the floating pasted image; `grab` is the pointer's offset from its corner.
    MoveFloating {
        grab: [f32; 2],
    },
    /// Changing the brush size with the resize modifier held.
    Resize {
        anchor: Pos2,
        start_size: f32,
    },
    Pan {
        button: PointerButton,
        last: Pos2,
    },
}

/// A pasted image that hasn't been placed onto a layer yet.
struct Floating {
    /// The image as pasted.
    original: Image,
    /// What will be placed: `original`, possibly with white made transparent.
    image: Image,
    white_transparent: bool,
    x: i32,
    y: i32,
    /// GPU textures for `image`, split so no texture exceeds GPU limits.
    chunks: Vec<(IRect, TextureHandle)>,
}

impl Floating {
    fn new(original: Image, x: i32, y: i32, white_transparent: bool) -> Self {
        let mut f = Self {
            image: original.clone(),
            original,
            white_transparent: false,
            x,
            y,
            chunks: Vec::new(),
        };
        f.set_white_transparent(white_transparent);
        f
    }

    fn set_white_transparent(&mut self, on: bool) {
        if on == self.white_transparent {
            return;
        }
        self.white_transparent = on;
        self.image = if on {
            canvas::white_to_transparent(&self.original)
        } else {
            self.original.clone()
        };
        self.chunks.clear();
    }

    fn rect(&self) -> IRect {
        IRect {
            x0: self.x,
            y0: self.y,
            x1: self.x + self.image.width as i32,
            y1: self.y + self.image.height as i32,
        }
    }
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

const MIN_SIZE: f32 = 1.0;
const MAX_SIZE: f32 = 300.0;
/// Resolution of layer thumbnails in pixels.
const THUMB: usize = 64;
/// Max side of a floating-image texture chunk.
const CHUNK: usize = 2048;

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

    settings: Settings,
    settings_open: bool,

    selection: Option<IRect>,
    floating: Option<Floating>,
    grid: Option<GridState>,
    /// Last copied image, used if the system clipboard can't be read.
    clipboard: Option<Image>,

    /// Canvas coordinate shown at the top-left corner of the canvas area.
    offset: Vec2,
    /// Canvas pixels per physical screen pixel (1.0 = 100%, pixel for pixel).
    zoom: f32,
    /// Physical pixels per egui point (the OS display scaling).
    pixels_per_point: f32,
    /// Modifier keys held during the current frame.
    modifiers: Modifiers,
    /// Screen area of the canvas during the last frame.
    canvas_rect: Rect,
    /// Pointer position if it was over the canvas during the last frame.
    canvas_hover: Option<Pos2>,

    gesture: Option<Gesture>,
    textures: HashMap<(LayerId, TileKey), TextureHandle>,

    thumbs: HashMap<LayerId, TextureHandle>,
    /// Tight bounds of the painted pixels in each tile, for framing thumbnails.
    content_bounds: HashMap<(LayerId, TileKey), IRect>,
    thumbs_dirty: bool,
    thumbs_updated_at: f64,
}

impl PaintApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, Settings::STORAGE_KEY))
            .unwrap_or_default();
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
            settings,
            settings_open: false,
            selection: None,
            floating: None,
            grid: None,
            clipboard: None,
            offset: Vec2::ZERO,
            zoom: 1.0,
            pixels_per_point: cc.egui_ctx.pixels_per_point(),
            modifiers: Modifiers::NONE,
            canvas_rect: Rect::NOTHING,
            canvas_hover: None,
            gesture: None,
            textures: HashMap::new(),
            thumbs: HashMap::new(),
            content_bounds: HashMap::new(),
            thumbs_dirty: true,
            thumbs_updated_at: 0.0,
        }
    }
}

impl eframe::App for PaintApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pixels_per_point = ctx.pixels_per_point();
        if self.tool != Tool::Grid {
            self.grid = None;
        }
        self.shortcuts(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas_area(ui));
        self.brush_window(&ctx);
        self.layers_window(&ctx);
        self.settings_window(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, Settings::STORAGE_KEY, &self.settings);
    }
}

impl PaintApp {
    fn active_layer_id(&self) -> LayerId {
        self.layers[self.active].id
    }

    fn undo(&mut self) {
        // Undoing right after a paste just drops the pasted image.
        if self.floating.take().is_some() {
            return;
        }
        self.finish_gesture();
        self.history.undo(&mut self.layers);
        self.clamp_active();
    }

    fn redo(&mut self) {
        if self.floating.is_some() {
            return;
        }
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
        let copy_merged = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::C);
        let (do_redo, do_undo, do_copy_merged) = ctx.input_mut(|i| {
            let redo = i.consume_shortcut(&redo_a) || i.consume_shortcut(&redo_b);
            (
                redo,
                i.consume_shortcut(&undo),
                i.consume_shortcut(&copy_merged),
            )
        });
        if do_copy_merged {
            self.copy(ctx, true);
        }
        if do_redo {
            self.redo();
        } else if do_undo {
            self.undo();
        }
        for (tool, _, key, _) in Tool::ALL {
            if ctx.input(|i| i.modifiers.is_none() && i.key_pressed(key)) {
                self.tool = tool;
            }
        }

        let (enter, escape, delete) = ctx.input(|i| {
            (
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
            )
        });
        if enter {
            self.place_floating();
        }
        // Esc cancels an unfinished grid or paste first, otherwise clears the selection.
        if escape && self.grid.take().is_none() && self.floating.take().is_none() {
            self.selection = None;
        }
        if delete && self.floating.take().is_none() {
            self.delete_selection();
        }

        // egui-winit turns the platform copy/cut/paste shortcuts into events.
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            match event {
                egui::Event::Copy => self.copy(ctx, false),
                egui::Event::Cut => {
                    self.copy(ctx, false);
                    if self.floating.take().is_none() {
                        self.delete_selection();
                    }
                }
                egui::Event::PasteImage(image) => self.paste(image_from_egui(&image)),
                // The clipboard had text; it may still hold an image as well.
                egui::Event::Paste(_) => self.paste_from_clipboard(),
                _ => {}
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tool, name, key, icon) in Tool::ALL {
                if icon_button(ui, icon, self.tool == tool)
                    .on_hover_text(format!("{name} ({})", key.name()))
                    .clicked()
                {
                    self.tool = tool;
                }
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

            let can_copy = self.selection.is_some() || self.floating.is_some();
            if ui
                .add_enabled(can_copy, egui::Button::new("Copy"))
                .on_hover_text("Copy from the active layer (Ctrl+C)")
                .clicked()
            {
                self.copy(ui.ctx(), false);
            }
            if ui
                .add_enabled(can_copy, egui::Button::new("Copy merged"))
                .on_hover_text("Copy what's visible on all layers (Ctrl+Shift+C)")
                .clicked()
            {
                self.copy(ui.ctx(), true);
            }
            if ui.button("Paste").on_hover_text("Ctrl+V").clicked() {
                self.paste_from_clipboard();
            }
            ui.separator();

            ui.label(format!("{:.0}%", self.zoom * 100.0));
            if ui
                .button("Reset view")
                .on_hover_text("Back to the origin at 100% (1 image pixel = 1 screen pixel)")
                .clicked()
            {
                self.offset = Vec2::ZERO;
                self.zoom = 1.0;
            }
            ui.separator();
            if ui.button("Settings").clicked() {
                self.settings_open = !self.settings_open;
            }
        });
    }

    fn brush_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("Brush")
            .title_bar(false)
            .resizable(false)
            .anchor(Align2::LEFT_CENTER, [10.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(64.0);
                ui.vertical(|ui| {
                    ui.label("Size");
                    ui.add(
                        egui::Slider::new(&mut self.size, MIN_SIZE..=MAX_SIZE)
                            .vertical()
                            .logarithmic(true)
                            .integer(),
                    );
                    ui.add_space(8.0);
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
                match self.tool {
                    Tool::Fill => {
                        ui.separator();
                        ui.label("Tolerance");
                        ui.add(egui::DragValue::new(&mut self.fill_tolerance).range(0..=255));
                    }
                    Tool::Circle | Tool::Rectangle => {
                        ui.separator();
                        ui.checkbox(&mut self.fill_shapes, "Filled");
                    }
                    Tool::Grid => {
                        ui.separator();
                        ui.small(match self.grid {
                            None => "Click to start the first cell",
                            Some(GridState::Cell { .. }) => "Click to set the cell size",
                            Some(GridState::Grid { .. }) => "Click to set the grid size",
                        });
                    }
                    _ => {}
                }
                if self.floating.is_some() {
                    ui.separator();
                    ui.checkbox(&mut self.settings.paste_white_transparent, "White → clear")
                        .on_hover_text("Treat white pixels of the pasted image as transparent");
                    ui.vertical_centered_justified(|ui| {
                        if ui.button("Place").on_hover_text("Enter").clicked() {
                            self.place_floating();
                        }
                        if ui.button("Cancel").on_hover_text("Esc").clicked() {
                            self.floating = None;
                        }
                    });
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
                        let thumb = self.thumbs.get(&layer.id).map(|t| t.id());
                        let clicked_thumb = layer_thumbnail(ui, thumb, i == self.active).clicked();
                        let clicked_name = ui
                            .selectable_label(i == self.active, layer.name.as_str())
                            .clicked();
                        if clicked_thumb || clicked_name {
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

    fn settings_window(&mut self, ctx: &egui::Context) {
        let settings = &mut self.settings;
        egui::Window::new("Settings")
            .open(&mut self.settings_open)
            .collapsible(false)
            .resizable(false)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| settings.ui(ui));
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

    // ----- Selection & clipboard -----

    /// Copies the selection (or the pending paste) to the clipboard. With
    /// `merged`, copies what's visible across all layers instead of just the
    /// active one.
    fn copy(&mut self, ctx: &egui::Context, merged: bool) {
        let image = match (&self.floating, self.selection) {
            (Some(f), _) => f.image.clone(),
            (None, Some(sel)) if merged => canvas::copy_region_merged(&self.layers, sel),
            (None, Some(sel)) => canvas::copy_region(&self.layers[self.active], sel),
            (None, None) => return,
        };
        ctx.copy_image(ColorImage::from_rgba_unmultiplied(
            [image.width, image.height],
            image.pixels.as_flattened(),
        ));
        self.clipboard = Some(image);
    }

    fn delete_selection(&mut self) {
        let Some(sel) = self.selection else { return };
        let mut op = PaintOp::new(self.active_layer_id(), PaintMode::Erase { strength: 1.0 });
        op.apply(&mut self.layers[self.active], sel, |_, _| 1.0);
        self.commit(op);
    }

    fn paste_from_clipboard(&mut self) {
        let image = arboard::Clipboard::new()
            .and_then(|mut c| c.get_image())
            .ok()
            .map(|img| Image {
                width: img.width,
                height: img.height,
                pixels: img
                    .bytes
                    .chunks_exact(4)
                    .map(|p| [p[0], p[1], p[2], p[3]])
                    .collect(),
            })
            .or_else(|| self.clipboard.clone());
        if let Some(image) = image {
            self.paste(image);
        }
    }

    /// Shows `image` as a floating piece centered under the pointer (or the view).
    fn paste(&mut self, image: Image) {
        if image.width == 0 || image.height == 0 {
            return;
        }
        self.place_floating();
        let rect = self.canvas_rect;
        let center = self.to_canvas(rect, self.canvas_hover.unwrap_or(rect.center()));
        let x = (center[0] - image.width as f32 / 2.0).round() as i32;
        let y = (center[1] - image.height as f32 / 2.0).round() as i32;
        let white = self.settings.paste_white_transparent;
        self.floating = Some(Floating::new(image, x, y, white));
        self.selection = None;
    }

    /// Commits the floating image onto the active layer.
    fn place_floating(&mut self) {
        let Some(f) = self.floating.take() else {
            return;
        };
        let mut op = PaintOp::new(
            self.active_layer_id(),
            PaintMode::Paint {
                color: [0; 4],
                opacity: 1.0,
            },
        );
        op.blit(&mut self.layers[self.active], &f.image, f.x, f.y);
        self.commit(op);
        self.selection = Some(f.rect());
    }

    // ----- Canvas -----

    /// Screen points per canvas pixel. egui works in points, which are larger
    /// than physical pixels when the OS display scaling is above 100%.
    fn scale(&self) -> f32 {
        self.zoom / self.pixels_per_point
    }

    fn to_canvas(&self, rect: Rect, p: Pos2) -> [f32; 2] {
        let v = (p - rect.min) / self.scale() + self.offset;
        [v.x, v.y]
    }

    fn to_screen(&self, rect: Rect, p: [f32; 2]) -> Pos2 {
        rect.min + (Vec2::new(p[0], p[1]) - self.offset) * self.scale()
    }

    /// Screen rectangle of a canvas rectangle, snapped to physical pixels so
    /// images stay sharp at 100% and neighboring tiles meet exactly.
    fn screen_rect_of(&self, rect: Rect, r: IRect) -> Rect {
        let ppp = self.pixels_per_point;
        let snap = |p: Pos2| Pos2::new((p.x * ppp).round() / ppp, (p.y * ppp).round() / ppp);
        Rect::from_min_max(
            snap(self.to_screen(rect, [r.x0 as f32, r.y0 as f32])),
            snap(self.to_screen(rect, [r.x1 as f32, r.y1 as f32])),
        )
    }

    fn square_held(&self) -> bool {
        self.settings.square_modifier.is_held(self.modifiers)
    }

    /// Pixel corner under `p`, moved to make a square with `start` if the
    /// snap-to-square key is held.
    fn rect_end(&self, start: [i32; 2], p: [f32; 2]) -> [i32; 2] {
        let end = [p[0].round() as i32, p[1].round() as i32];
        if self.square_held() {
            canvas::square_end(start, end)
        } else {
            end
        }
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
        self.canvas_rect = rect;
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        self.canvas_hover = response.hover_pos();
        self.handle_view_input(ui, rect, &response);
        self.handle_tool_input(ui, rect);
        self.sync_textures(ui.ctx());
        self.paint_canvas(ui, rect, &response);
    }

    fn handle_view_input(&mut self, ui: &egui::Ui, rect: Rect, response: &egui::Response) {
        if !response.hovered() {
            return;
        }
        let (scroll, pinch, pointer, mods) = ui.input(|i| {
            (
                i.smooth_scroll_delta,
                i.zoom_delta(),
                i.pointer.hover_pos(),
                i.modifiers,
            )
        });
        let factor = pinch * ((scroll.x + scroll.y) * 0.002).exp();
        if factor == 1.0 {
            return;
        }
        let s = &self.settings;
        if s.resize_with_wheel && s.resize_modifier.is_held(mods) {
            let f = factor.powf(s.resize_sensitivity);
            self.size = (self.size * f).clamp(MIN_SIZE, MAX_SIZE);
            return;
        }
        let anchor = pointer.unwrap_or(rect.center());
        let before = self.to_canvas(rect, anchor);
        self.zoom = (self.zoom * factor).clamp(0.02, 64.0);
        let after = self.to_canvas(rect, anchor);
        self.offset += Vec2::new(before[0] - after[0], before[1] - after[1]);
    }

    fn handle_tool_input(&mut self, ui: &egui::Ui, rect: Rect) {
        // Walk the raw events rather than the end-of-frame pointer state so
        // every motion sample lands in the stroke and quick press/release
        // pairs within one frame aren't lost.
        let (events, space, modifiers) =
            ui.input(|i| (i.events.clone(), i.key_down(Key::Space), i.modifiers));
        self.modifiers = modifiers;
        let ctx = ui.ctx();
        for event in events {
            match event {
                egui::Event::PointerMoved(pos) => self.pointer_moved(rect, pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers,
                } => {
                    let on_canvas =
                        rect.contains(pos) && ctx.layer_id_at(pos) == Some(ui.layer_id());
                    if self.gesture.is_some() || !on_canvas {
                        continue;
                    }
                    // Clicking the canvas takes keyboard focus away from text
                    // fields so shortcuts like Ctrl+V reach the canvas again.
                    ctx.memory_mut(|m| m.stop_text_input());
                    let resize = self.settings.resize_with_drag
                        && self.settings.resize_modifier.is_held(modifiers);
                    let gesture = match button {
                        PointerButton::Middle => Some(Gesture::Pan { button, last: pos }),
                        PointerButton::Primary if resize => Some(Gesture::Resize {
                            anchor: pos,
                            start_size: self.size,
                        }),
                        PointerButton::Primary if space => Some(Gesture::Pan { button, last: pos }),
                        PointerButton::Primary => {
                            self.begin_tool(rect, pos);
                            None
                        }
                        PointerButton::Secondary => {
                            self.grid = None;
                            None
                        }
                        _ => None,
                    };
                    if gesture.is_some() {
                        self.gesture = gesture;
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
        let scale = self.scale();
        let select_end = match &self.gesture {
            Some(Gesture::Select { start }) => Some(self.rect_end(*start, p)),
            _ => None,
        };
        match &mut self.gesture {
            Some(Gesture::Pan { last, .. }) => {
                self.offset -= (pos - *last) / scale;
                *last = pos;
            }
            Some(Gesture::Stroke { op, last }) if p != *last => {
                if let Some(layer) = self.layers.iter_mut().find(|l| l.id == op.layer_id) {
                    op.stroke_segment(layer, *last, p, self.size / 2.0);
                }
                *last = p;
            }
            Some(Gesture::Select { start }) => {
                let end = select_end.unwrap_or(*start);
                self.selection = Some(canvas::rect_from_corners(*start, end));
            }
            Some(Gesture::MoveFloating { grab }) => {
                if let Some(f) = &mut self.floating {
                    f.x = (p[0] - grab[0]).round() as i32;
                    f.y = (p[1] - grab[1]).round() as i32;
                }
            }
            Some(Gesture::Resize { anchor, start_size }) => {
                // Doubles/halves the size every 100 px of horizontal movement.
                let dx = (pos.x - anchor.x) / 100.0 * self.settings.resize_sensitivity;
                self.size = (*start_size * dx.exp2()).clamp(MIN_SIZE, MAX_SIZE);
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
        if let Some(f) = &self.floating {
            let (x, y) = (p[0].floor() as i32, p[1].floor() as i32);
            if f.rect().contains(x, y) {
                let grab = [p[0] - f.x as f32, p[1] - f.y as f32];
                self.gesture = Some(Gesture::MoveFloating { grab });
            } else {
                // Clicking elsewhere drops the pasted image in place.
                self.place_floating();
            }
            return;
        }
        if self.tool == Tool::Select {
            self.selection = None;
            let start = [p[0].round() as i32, p[1].round() as i32];
            self.gesture = Some(Gesture::Select { start });
            return;
        }
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
            Tool::Rectangle => {
                let start = [p[0].round() as i32, p[1].round() as i32];
                self.gesture = Some(Gesture::Rectangle { start });
            }
            Tool::Grid => self.grid_click(p),
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
            Tool::Select => unreachable!(),
        }
    }

    /// Advances the grid tool: start the cell, fix the cell, then draw the grid.
    fn grid_click(&mut self, p: [f32; 2]) {
        self.grid = match self.grid {
            None => Some(GridState::Cell {
                origin: [p[0].round() as i32, p[1].round() as i32],
            }),
            Some(GridState::Cell { origin }) => {
                let end = self.rect_end(origin, p);
                let cell = [(end[0] - origin[0]).abs(), (end[1] - origin[1]).abs()];
                if cell[0] > 0 && cell[1] > 0 {
                    Some(GridState::Grid { origin, cell })
                } else {
                    self.grid
                }
            }
            Some(GridState::Grid { origin, cell }) => {
                let end = self.rect_end(origin, p);
                let (top_left, cols, rows) = canvas::grid_layout(origin, cell, end);
                if cols == 0 || rows == 0 {
                    return;
                }
                let mut op = PaintOp::new(self.active_layer_id(), self.paint_mode());
                op.grid(
                    &mut self.layers[self.active],
                    top_left,
                    cell,
                    cols,
                    rows,
                    self.size,
                );
                self.commit(op);
                None
            }
        };
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
            Some(Gesture::Rectangle { start }) => {
                let Some(pos) = pos else { return };
                let end = self.rect_end(start, self.to_canvas(rect, pos));
                let r = canvas::rect_from_corners(start, end);
                if r.width() <= 0 || r.height() <= 0 {
                    return;
                }
                let mut op = PaintOp::new(self.active_layer_id(), self.paint_mode());
                op.rectangle(
                    &mut self.layers[self.active],
                    r,
                    self.size,
                    self.fill_shapes,
                );
                self.commit(op);
            }
            Some(Gesture::Select { .. }) => {
                if self
                    .selection
                    .is_some_and(|s| s.width() <= 0 || s.height() <= 0)
                {
                    self.selection = None;
                }
            }
            Some(Gesture::MoveFloating { .. } | Gesture::Resize { .. } | Gesture::Pan { .. })
            | None => {}
        }
    }

    fn commit(&mut self, op: PaintOp) {
        let layer_id = op.layer_id;
        if let Some(tiles) = op.finish() {
            self.history.push(Action::Tiles { layer_id, tiles });
        }
    }

    // ----- Rendering -----

    /// Uploads changed tiles to the GPU, drops textures of deleted layers/tiles,
    /// and refreshes layer thumbnails.
    fn sync_textures(&mut self, ctx: &egui::Context) {
        for layer in &mut self.layers {
            for key in layer.dirty.drain() {
                let tex_key = (layer.id, key);
                self.thumbs_dirty = true;
                match layer.tiles.get(&key) {
                    Some(tile) => {
                        match canvas::tile_content_bounds(key, tile) {
                            Some(b) => self.content_bounds.insert(tex_key, b),
                            None => self.content_bounds.remove(&tex_key),
                        };
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
                        self.content_bounds.remove(&tex_key);
                    }
                }
            }
        }
        let layers = &self.layers;
        let exists = |id: &LayerId, key: &TileKey| {
            layers
                .iter()
                .any(|l| l.id == *id && l.tiles.contains_key(key))
        };
        self.textures.retain(|(id, key), _| exists(id, key));
        self.content_bounds.retain(|(id, key), _| exists(id, key));
        if self.thumbs.len() != layers.len()
            || layers.iter().any(|l| !self.thumbs.contains_key(&l.id))
        {
            self.thumbs_dirty = true;
        }

        // Thumbnails are cheap but not free; refresh at most a few times per
        // second while painting.
        let now = ctx.input(|i| i.time);
        if self.thumbs_dirty && (self.gesture.is_none() || now - self.thumbs_updated_at > 0.25) {
            self.update_thumbnails(ctx);
            self.thumbs_dirty = false;
            self.thumbs_updated_at = now;
        } else if self.thumbs_dirty {
            ctx.request_repaint();
        }

        if let Some(f) = &mut self.floating {
            f.set_white_transparent(self.settings.paste_white_transparent);
            if f.chunks.is_empty() {
                f.chunks = image_chunks(ctx, &f.image);
            }
        }
    }

    fn update_thumbnails(&mut self, ctx: &egui::Context) {
        // All layers share one frame so the previews line up with each other.
        let region = self
            .content_bounds
            .values()
            .copied()
            .reduce(|a, b| a.union(&b));
        let ids: Vec<LayerId> = self.layers.iter().map(|l| l.id).collect();
        self.thumbs.retain(|id, _| ids.contains(id));
        for layer in &self.layers {
            let pixels = match region {
                Some(r) => canvas::thumbnail(layer, r, THUMB),
                None => vec![[0; 4]; THUMB * THUMB],
            };
            let image = ColorImage::from_rgba_unmultiplied([THUMB, THUMB], pixels.as_flattened());
            match self.thumbs.get_mut(&layer.id) {
                Some(t) => t.set(image, TextureOptions::LINEAR),
                None => {
                    let t = ctx.load_texture(
                        format!("thumb-{}", layer.id),
                        image,
                        TextureOptions::LINEAR,
                    );
                    self.thumbs.insert(layer.id, t);
                }
            }
        }
    }

    fn paint_canvas(&self, ui: &egui::Ui, rect: Rect, response: &egui::Response) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::WHITE);

        let view = self.visible_canvas_rect(rect);
        let (tx0, ty0) = canvas::tile_key(view.x0, view.y0);
        let (tx1, ty1) = canvas::tile_key(view.x1, view.y1);
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.visible && layer.opacity > 0.0 {
                let tint = Color32::from_white_alpha((layer.opacity * 255.0).round() as u8);
                for &key in layer.tiles.keys() {
                    if key.0 < tx0 || key.0 > tx1 || key.1 < ty0 || key.1 > ty1 {
                        continue;
                    }
                    let Some(tex) = self.textures.get(&(layer.id, key)) else {
                        continue;
                    };
                    let r = IRect {
                        x0: key.0 * TILE,
                        y0: key.1 * TILE,
                        x1: (key.0 + 1) * TILE,
                        y1: (key.1 + 1) * TILE,
                    };
                    painter.image(tex.id(), self.screen_rect_of(rect, r), uv, tint);
                }
            }
            // The pasted image previews where it will land: right above the active layer.
            if index == self.active
                && let Some(f) = &self.floating
            {
                for (chunk, tex) in &f.chunks {
                    let r = IRect {
                        x0: f.x + chunk.x0,
                        y0: f.y + chunk.y0,
                        x1: f.x + chunk.x1,
                        y1: f.y + chunk.y1,
                    };
                    painter.image(tex.id(), self.screen_rect_of(rect, r), uv, Color32::WHITE);
                }
            }
        }

        if let Some(f) = &self.floating {
            marching_ants(&painter, self.screen_rect_of(rect, f.rect()));
        } else if let Some(sel) = self.selection {
            marching_ants(&painter, self.screen_rect_of(rect, sel));
        }

        let outline = Stroke::new(1.0, Color32::from_gray(120));
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let color = self.color.gamma_multiply(self.opacity);
        let pen = Stroke::new(self.size * self.scale(), color);
        // Circle preview.
        if let Some(Gesture::Circle { center }) = &self.gesture
            && let Some(pos) = pointer
        {
            let c = self.to_screen(rect, *center);
            let r = c.distance(pos);
            if self.fill_shapes {
                painter.circle_filled(c, r, color);
            } else {
                painter.circle_stroke(c, r, pen);
            }
        }

        // Rectangle preview, and the size of whatever rectangle is being dragged.
        if let Some(pos) = pointer {
            let p = self.to_canvas(rect, pos);
            match &self.gesture {
                Some(Gesture::Rectangle { start }) => {
                    let r = canvas::rect_from_corners(*start, self.rect_end(*start, p));
                    let sr = self.screen_rect_of(rect, r);
                    if self.fill_shapes {
                        painter.rect_filled(sr, 0.0, color);
                    } else {
                        painter.rect_stroke(sr, 0.0, pen, egui::StrokeKind::Middle);
                    }
                    info_label(&painter, pos, format!("{} × {}", r.width(), r.height()));
                }
                Some(Gesture::Select { .. }) => {
                    if let Some(sel) = self.selection {
                        info_label(&painter, pos, format!("{} × {}", sel.width(), sel.height()));
                    }
                }
                _ => {}
            }
        }

        // Grid preview.
        if let (Some(grid), Some(pos)) = (self.grid, pointer) {
            let p = self.to_canvas(rect, pos);
            match grid {
                GridState::Cell { origin } => {
                    let r = canvas::rect_from_corners(origin, self.rect_end(origin, p));
                    painter.rect_stroke(
                        self.screen_rect_of(rect, r),
                        0.0,
                        pen,
                        egui::StrokeKind::Middle,
                    );
                    info_label(
                        &painter,
                        pos,
                        format!("cell {} × {}", r.width(), r.height()),
                    );
                }
                GridState::Grid { origin, cell } => {
                    let end = self.rect_end(origin, p);
                    let (top_left, cols, rows) = canvas::grid_layout(origin, cell, end);
                    // The area being dragged out, and the cells that fit in it.
                    marching_ants(
                        &painter,
                        self.screen_rect_of(rect, canvas::rect_from_corners(origin, end)),
                    );
                    self.paint_grid_preview(&painter, rect, top_left, cell, cols, rows, pen);
                    info_label(&painter, pos, format!("{cols} × {rows} cells"));
                }
            }
        }

        // While resizing, show the brush at the spot where the drag started.
        if let Some(Gesture::Resize { anchor, .. }) = &self.gesture {
            let r = (self.size / 2.0 * self.scale()).max(1.0);
            painter.circle_stroke(*anchor, r, Stroke::new(2.0, Color32::WHITE));
            painter.circle_stroke(*anchor, r, Stroke::new(1.0, Color32::BLACK));
            painter.text(
                *anchor + Vec2::new(r + 8.0, 0.0),
                Align2::LEFT_CENTER,
                format!("{:.0} px", self.size),
                egui::FontId::proportional(14.0),
                Color32::BLACK,
            );
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            return;
        }

        // Cursor under the pointer.
        if let Some(pos) = response.hover_pos() {
            let p = self.to_canvas(rect, pos);
            let over_floating = self
                .floating
                .as_ref()
                .is_some_and(|f| f.rect().contains(p[0].floor() as i32, p[1].floor() as i32));
            if over_floating || matches!(self.gesture, Some(Gesture::MoveFloating { .. })) {
                ui.ctx().set_cursor_icon(CursorIcon::Move);
                return;
            }
            match self.tool {
                Tool::Brush | Tool::Eraser => {
                    painter.circle_stroke(pos, (self.size / 2.0 * self.scale()).max(1.0), outline);
                }
                Tool::Circle | Tool::Fill | Tool::Select | Tool::Rectangle | Tool::Grid => {
                    painter.line_segment([pos - Vec2::X * 6.0, pos + Vec2::X * 6.0], outline);
                    painter.line_segment([pos - Vec2::Y * 6.0, pos + Vec2::Y * 6.0], outline);
                }
            }
        }
    }
}

impl PaintApp {
    #[allow(clippy::too_many_arguments)]
    fn paint_grid_preview(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        top_left: [i32; 2],
        cell: [i32; 2],
        cols: i32,
        rows: i32,
        pen: Stroke,
    ) {
        if cols == 0 || rows == 0 {
            return;
        }
        let at = |c: i32, r: i32| {
            self.to_screen(
                rect,
                [
                    (top_left[0] + c * cell[0]) as f32,
                    (top_left[1] + r * cell[1]) as f32,
                ],
            )
        };
        // Huge grids would just be a solid blob on screen; cap the line count.
        if cols > 500 || rows > 500 {
            painter.rect_stroke(
                Rect::from_two_pos(at(0, 0), at(cols, rows)),
                0.0,
                pen,
                egui::StrokeKind::Middle,
            );
            return;
        }
        for c in 0..=cols {
            painter.line_segment([at(c, 0), at(c, rows)], pen);
        }
        for r in 0..=rows {
            painter.line_segment([at(0, r), at(cols, r)], pen);
        }
    }
}

/// Draws a small text label with a background next to the pointer.
fn info_label(painter: &egui::Painter, pointer: Pos2, text: String) {
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), Color32::WHITE);
    let pos = pointer + Vec2::new(16.0, 16.0);
    let bg = Rect::from_min_size(pos, galley.size()).expand(4.0);
    painter.rect_filled(bg, 4.0, Color32::from_black_alpha(190));
    painter.galley(pos, galley, Color32::WHITE);
}

/// A square toolbar button showing an icon.
fn icon_button(ui: &mut egui::Ui, icon: Icon, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let visuals = ui.visuals();
    let (bg, fg) = if selected {
        (visuals.selection.bg_fill, visuals.selection.stroke.color)
    } else if resp.hovered() {
        (
            visuals.widgets.hovered.weak_bg_fill,
            visuals.widgets.hovered.fg_stroke.color,
        )
    } else {
        (
            Color32::TRANSPARENT,
            visuals.widgets.inactive.fg_stroke.color,
        )
    };
    ui.painter().rect_filled(rect, 4.0, bg);
    icons::paint(ui.painter(), rect.shrink(5.0), icon, fg);
    resp
}

/// Draws a selection outline visible on both light and dark content.
fn marching_ants(painter: &egui::Painter, r: Rect) {
    let pts = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    painter.add(Shape::line(pts.to_vec(), Stroke::new(1.0, Color32::WHITE)));
    painter.extend(Shape::dashed_line(
        &pts,
        Stroke::new(1.0, Color32::BLACK),
        4.0,
        4.0,
    ));
}

/// Draws a layer preview over a checkerboard; returns its click response.
fn layer_thumbnail(
    ui: &mut egui::Ui,
    tex: Option<egui::TextureId>,
    active: bool,
) -> egui::Response {
    let size = 44.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, Color32::from_gray(200));
    let cell = size / 6.0;
    for y in 0..6 {
        for x in 0..6 {
            if (x + y) % 2 == 0 {
                let min = rect.min + Vec2::new(x as f32, y as f32) * cell;
                p.rect_filled(
                    Rect::from_min_size(min, Vec2::splat(cell)),
                    0.0,
                    Color32::from_gray(245),
                );
            }
        }
    }
    if let Some(tex) = tex {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        p.image(tex, rect, uv, Color32::WHITE);
    }
    let stroke = if active {
        Stroke::new(2.0, ui.visuals().selection.bg_fill)
    } else {
        Stroke::new(1.0, Color32::from_gray(90))
    };
    p.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
    resp
}

fn image_from_egui(img: &ColorImage) -> Image {
    Image {
        width: img.size[0],
        height: img.size[1],
        pixels: img
            .pixels
            .iter()
            .map(|c| c.to_srgba_unmultiplied())
            .collect(),
    }
}

/// Uploads an image as textures of at most `CHUNK`×`CHUNK` pixels.
fn image_chunks(ctx: &egui::Context, image: &Image) -> Vec<(IRect, TextureHandle)> {
    let mut chunks = Vec::new();
    for y0 in (0..image.height).step_by(CHUNK) {
        for x0 in (0..image.width).step_by(CHUNK) {
            let (w, h) = (CHUNK.min(image.width - x0), CHUNK.min(image.height - y0));
            let mut rgba = Vec::with_capacity(w * h * 4);
            for y in y0..y0 + h {
                let row = &image.pixels[y * image.width + x0..y * image.width + x0 + w];
                rgba.extend_from_slice(row.as_flattened());
            }
            let tex = ctx.load_texture(
                format!("paste-{x0}-{y0}"),
                ColorImage::from_rgba_unmultiplied([w, h], &rgba),
                TEXTURE_OPTIONS,
            );
            let r = IRect {
                x0: x0 as i32,
                y0: y0 as i32,
                x1: (x0 + w) as i32,
                y1: (y0 + h) as i32,
            };
            chunks.push((r, tex));
        }
    }
    chunks
}
