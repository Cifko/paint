use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, ColorImage, CursorIcon, Key, KeyboardShortcut, Modifiers, PointerButton,
    Pos2, Rect, Sense, Shape, Stroke, TextureFilter, TextureHandle, TextureOptions,
    TextureWrapMode, Vec2,
};

use crate::canvas::{
    self, GridKind, IRect, Image, Layer, LayerId, PaintMode, PaintOp, ShapeKind, TILE, TileKey,
};
use crate::file;
use crate::floating::{Floating, Frame, Handle};
use crate::history::{Action, History};
use crate::icons::{self, Icon};
use crate::settings::Settings;
use crate::text::{self, FontFace, TextStyle};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Brush,
    Eraser,
    Fill,
    Shape,
    Grid,
    Select,
    Text,
}

impl Tool {
    const ALL: [(Tool, &'static str, Key, Icon); 7] = [
        (Tool::Brush, "Brush", Key::B, Icon::Brush),
        (Tool::Eraser, "Eraser", Key::E, Icon::Eraser),
        (Tool::Fill, "Fill", Key::G, Icon::Fill),
        (Tool::Shape, "Shape", Key::S, Icon::Rectangle),
        (Tool::Grid, "Grid", Key::H, Icon::Grid),
        (Tool::Select, "Select", Key::M, Icon::Select),
        (Tool::Text, "Text", Key::T, Icon::Text),
    ];
}

#[derive(Clone, Copy, Debug)]
struct Brush {
    /// Pen width, or font size for text, in canvas pixels.
    size: f32,
    opacity: f32,
}

/// Text being typed with the Text tool, not yet placed onto the layer.
struct TextEdit {
    /// Top-left corner of the first line.
    origin: [i32; 2],
    buffer: text::Buffer,
}

/// What the preview layer currently shows.
#[derive(Clone, Debug, PartialEq)]
enum PreviewKey {
    Shape {
        shape: ShapeSpec,
        color: [u8; 4],
        opacity: f32,
    },
    Text {
        origin: [i32; 2],
        text: String,
        style: TextStyle,
        color: [u8; 4],
        opacity: f32,
    },
}

/// What a file dialog was opened for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DialogPurpose {
    Open,
    SaveAs,
}

/// A native file dialog running on a background thread, so the UI never
/// blocks on it (on Linux it goes through the desktop portal, which may be
/// slow or missing).
struct FileDialog {
    purpose: DialogPurpose,
    result: std::sync::mpsc::Receiver<Option<PathBuf>>,
}

/// Something that would discard unsaved changes, waiting for confirmation.
#[derive(Clone, Debug)]
enum PendingAction {
    New,
    Open(PathBuf),
}

/// Shortcut keys that pick a shape (and the Shape tool).
const SHAPE_KEYS: [(Key, ShapeKind); 5] = [
    (Key::R, ShapeKind::Rectangle),
    (Key::C, ShapeKind::Ellipse),
    (Key::N, ShapeKind::NGon),
    (Key::L, ShapeKind::Line),
    (Key::P, ShapeKind::Polyline),
];

/// A polyline being clicked out point by point.
struct Polyline {
    points: Vec<[f32; 2]>,
}

/// A shape rendered into the preview layer.
#[derive(Clone, Debug, PartialEq)]
enum ShapeSpec {
    Ellipse(IRect, f32, bool),
    /// A convex polygon (triangle, n-gon).
    Polygon(Vec<[f32; 2]>, f32, bool),
    Polyline(Vec<[f32; 2]>, f32),
}

impl ShapeSpec {
    fn draw(&self, op: &mut PaintOp, layer: &mut Layer) {
        match self {
            Self::Ellipse(r, size, filled) => {
                if r.width() > 0 && r.height() > 0 {
                    op.ellipse(layer, *r, *size, *filled);
                }
            }
            Self::Polygon(points, size, filled) => op.polygon(layer, points, *size, *filled),
            Self::Polyline(points, size) => op.polyline(layer, points, *size),
        }
    }
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
    /// Dragging out a rectangle, ellipse or triangle.
    Shape {
        start: [i32; 2],
    },
    Line {
        start: [f32; 2],
    },
    /// Placing an n-gon: `start` is vertex 0, `press` where the mouse went down.
    NGon {
        start: [f32; 2],
        press: Pos2,
    },
    Select {
        start: [i32; 2],
    },
    /// Dragging a transform handle of the floating piece.
    Transform {
        handle: Handle,
        start: [f32; 2],
        frame: Frame,
    },
    /// Dragging the floating piece; `grab` is the pointer's offset from its center.
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
    /// Color 1 (left mouse button) and color 2 (right mouse button).
    primary: Color32,
    secondary: Color32,
    /// Whether the last canvas click used the right button, i.e. color 2.
    use_secondary: bool,
    /// Mouse button that started the current gesture.
    gesture_button: PointerButton,
    /// Size and opacity, kept separately for painting tools, the eraser and text.
    paint_brush: Brush,
    eraser_brush: Brush,
    text_brush: Brush,
    fill_tolerance: u8,
    fill_shapes: bool,

    settings: Settings,
    settings_open: bool,

    selection: Option<IRect>,
    floating: Option<Floating>,
    text_edit: Option<TextEdit>,
    text_face: FontFace,
    grid: Option<GridState>,
    grid_kind: GridKind,
    shape_kind: ShapeKind,
    /// Number of sides of the n-gon shape.
    ngon_sides: u32,
    /// Vertex 0 of an n-gon waiting for the click that places its far vertex.
    ngon_start: Option<[f32; 2]>,
    polyline: Option<Polyline>,

    /// Where the image was last opened from / saved to.
    file_path: Option<PathBuf>,
    /// History version at the last save or open, to detect unsaved changes.
    saved_version: u64,
    pending: Option<PendingAction>,
    /// Action to run once a save asked for by the confirmation dialog is done.
    after_save: Option<PendingAction>,
    dialog: Option<FileDialog>,
    /// A short notice shown at the bottom of the canvas.
    message: Option<(String, Instant)>,
    /// Window title last sent to the OS.
    title: String,
    /// Canvas area to bring into view once the canvas size is known.
    center_on: Option<IRect>,
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

    /// Scratch layer showing the shape being dragged out, rendered with the
    /// same code as the final shape so the preview matches it exactly.
    preview: Layer,
    preview_textures: HashMap<TileKey, TextureHandle>,
    /// Inputs of the current preview, to skip re-rendering when unchanged.
    preview_key: Option<PreviewKey>,

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
            primary: Color32::BLACK,
            secondary: Color32::WHITE,
            use_secondary: false,
            gesture_button: PointerButton::Primary,
            paint_brush: Brush {
                size: 8.0,
                opacity: 1.0,
            },
            eraser_brush: Brush {
                size: 24.0,
                opacity: 1.0,
            },
            text_brush: Brush {
                size: 32.0,
                opacity: 1.0,
            },
            fill_tolerance: 32,
            fill_shapes: false,
            settings,
            settings_open: false,
            selection: None,
            floating: None,
            text_edit: None,
            text_face: FontFace::Sans,
            grid: None,
            grid_kind: GridKind::Rectangle,
            shape_kind: ShapeKind::Rectangle,
            ngon_sides: 6,
            ngon_start: None,
            polyline: None,
            file_path: None,
            saved_version: 0,
            pending: None,
            after_save: None,
            dialog: None,
            message: None,
            title: String::new(),
            center_on: None,
            clipboard: None,
            offset: Vec2::ZERO,
            zoom: 1.0,
            pixels_per_point: cc.egui_ctx.pixels_per_point(),
            modifiers: Modifiers::NONE,
            canvas_rect: Rect::NOTHING,
            canvas_hover: None,
            gesture: None,
            textures: HashMap::new(),
            preview: Layer::new(0, "preview"),
            preview_textures: HashMap::new(),
            preview_key: None,
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
        if self.tool != Tool::Text {
            self.commit_text();
        }
        if self.tool != Tool::Shape || self.shape_kind != ShapeKind::Polyline {
            self.finish_polyline();
        }
        if self.tool != Tool::Shape || self.shape_kind != ShapeKind::NGon {
            self.ngon_start = None;
        }
        self.shortcuts(&ctx);
        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            self.request(PendingAction::Open(path));
        }

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas_area(ui));
        self.brush_window(&ctx);
        self.layers_window(&ctx);
        self.settings_window(&ctx);
        self.poll_dialog(&ctx);
        self.transform_bar(&ctx);
        self.confirm_window(&ctx);
        self.message_popup(&ctx);
        self.update_title(&ctx);
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
        // While clicking out a polyline, undo takes back its last point.
        if let Some(poly) = &mut self.polyline {
            poly.points.pop();
            if poly.points.is_empty() {
                self.polyline = None;
            }
            return;
        }
        // Undoing while typing drops the text that isn't placed yet.
        if self.text_edit.take().is_some() {
            return;
        }
        // Undoing while a paste or lifted selection floats drops it.
        if self.cancel_floating() {
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
        let shortcut = |m: Modifiers, k: Key| KeyboardShortcut::new(m, k);
        let (save_as, save, open, new) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&shortcut(Modifiers::COMMAND | Modifiers::SHIFT, Key::S)),
                i.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::S)),
                i.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::O)),
                i.consume_shortcut(&shortcut(Modifiers::COMMAND, Key::N)),
            )
        });
        if save_as || save {
            self.save(save_as);
        }
        if open {
            self.open_dialog();
        }
        if new {
            self.request(PendingAction::New);
        }
        if self.text_edit.is_some() {
            // While typing, the keyboard belongs to the text.
            self.text_keys(ctx);
            return;
        }
        let redo_a = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        let redo_b = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let copy_merged = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::C);
        let select_all = KeyboardShortcut::new(Modifiers::COMMAND, Key::A);
        let (do_redo, do_undo, do_copy_merged, do_select_all) = ctx.input_mut(|i| {
            let redo = i.consume_shortcut(&redo_a) || i.consume_shortcut(&redo_b);
            (
                redo,
                i.consume_shortcut(&undo),
                i.consume_shortcut(&copy_merged),
                i.consume_shortcut(&select_all),
            )
        });
        if do_select_all {
            self.select_all();
        }
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
        for (key, kind) in SHAPE_KEYS {
            if ctx.input(|i| i.modifiers.is_none() && i.key_pressed(key)) {
                self.tool = Tool::Shape;
                self.shape_kind = kind;
            }
        }

        if ctx.input(|i| i.modifiers.is_none() && i.key_pressed(Key::X)) {
            std::mem::swap(&mut self.primary, &mut self.secondary);
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
            self.finish_polyline();
        }
        // Esc cancels an unfinished grid, polyline or paste first, otherwise
        // clears the selection.
        if escape
            && self.grid.take().is_none()
            && self.polyline.take().is_none()
            && self.ngon_start.take().is_none()
            && !self.cancel_floating()
        {
            self.selection = None;
        }
        if delete && !self.discard_floating() {
            self.delete_selection();
        }

        // egui-winit turns the platform copy/cut/paste shortcuts into events.
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            match event {
                egui::Event::Copy => self.copy(ctx, false),
                egui::Event::Cut => {
                    self.copy(ctx, false);
                    if !self.discard_floating() {
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

    fn text_keys(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let Some(edit) = &mut self.text_edit else {
                return;
            };
            let b = &mut edit.buffer;
            match event {
                egui::Event::Text(s) | egui::Event::Paste(s) => {
                    let s: String = s
                        .chars()
                        .filter(|c| !c.is_control() || *c == '\n')
                        .collect();
                    b.insert(&s);
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => match key {
                    Key::Enter => b.insert("\n"),
                    Key::Tab => b.insert("    "),
                    Key::Backspace => b.backspace(),
                    Key::Delete => b.delete(),
                    Key::ArrowLeft => b.left(),
                    Key::ArrowRight => b.right(),
                    Key::ArrowUp => b.vertical(false),
                    Key::ArrowDown => b.vertical(true),
                    Key::Home => b.home(),
                    Key::End => b.end(),
                    Key::Escape => self.commit_text(),
                    Key::Z if modifiers.command => self.text_edit = None,
                    _ => {}
                },
                _ => {}
            }
        }
    }

    /// Places the text being typed onto the active layer.
    fn commit_text(&mut self) {
        let Some(edit) = self.text_edit.take() else {
            return;
        };
        let style = self.text_style();
        let mode = self.color_mode(self.text_brush.opacity);
        let mut op = PaintOp::new(self.active_layer_id(), mode);
        draw_text(
            &mut op,
            &mut self.layers[self.active],
            edit.origin,
            style,
            &edit.buffer.text,
        );
        self.commit(op);
    }

    /// Color 1 in front, color 2 behind it, like in MS Paint. Clicking either
    /// opens its color picker; the arrow swaps them.
    fn color_pair(&mut self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(46.0, 26.0), Sense::hover());
        let size = Vec2::splat(18.0);
        let front = Rect::from_min_size(rect.min, size);
        let back = Rect::from_min_size(rect.min + Vec2::new(10.0, 8.0), size);
        let swap = Rect::from_min_size(rect.min + Vec2::new(31.0, 0.0), Vec2::splat(14.0));

        // Register the back swatch first so the front one wins where they overlap.
        let back_resp = ui
            .interact(back, ui.scope_id().with("color2"), Sense::click())
            .on_hover_text("Color 2 (right mouse button). Click to change.");
        let front_resp = ui
            .interact(front, ui.scope_id().with("color1"), Sense::click())
            .on_hover_text("Color 1 (left mouse button). Click to change.");
        let swap_resp = ui
            .interact(swap, ui.scope_id().with("swap_colors"), Sense::click())
            .on_hover_text("Swap colors (X)");

        let painter = ui.painter();
        let frame = Stroke::new(1.0, Color32::from_gray(150));
        swatch(painter, back, self.secondary);
        painter.rect_stroke(back, 2.0, frame, egui::StrokeKind::Outside);
        swatch(painter, front, self.primary);
        painter.rect_stroke(front, 2.0, frame, egui::StrokeKind::Outside);
        let arrow = if swap_resp.hovered() {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        painter.text(
            swap.center(),
            Align2::CENTER_CENTER,
            "⇄",
            egui::FontId::proportional(13.0),
            arrow,
        );
        if swap_resp.clicked() {
            std::mem::swap(&mut self.primary, &mut self.secondary);
        }

        for (resp, color) in [
            (front_resp, &mut self.primary),
            (back_resp, &mut self.secondary),
        ] {
            egui::Popup::from_toggle_button_response(&resp)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    // The picker can't show "clear"; start from white instead.
                    if *color == Color32::TRANSPARENT {
                        *color = Color32::WHITE;
                    }
                    egui::color_picker::color_picker_color32(
                        ui,
                        color,
                        egui::color_picker::Alpha::Opaque,
                    );
                });
        }
    }

    // ----- Files -----

    fn unsaved(&self) -> bool {
        self.history.version() != self.saved_version
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), Instant::now()));
    }

    /// Runs `action` now, or asks first if it would discard unsaved changes.
    fn request(&mut self, action: PendingAction) {
        if self.unsaved() {
            self.pending = Some(action);
        } else {
            self.run(action);
        }
    }

    fn run(&mut self, action: PendingAction) {
        match action {
            PendingAction::New => self.reset_document(),
            PendingAction::Open(path) => self.open_path(path),
        }
    }

    /// Starts over with one empty layer.
    fn reset_document(&mut self) {
        self.gesture = None;
        self.floating = None;
        self.text_edit = None;
        self.grid = None;
        self.selection = None;
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        self.layers = vec![Layer::new(id, "Layer 1")];
        self.active = 0;
        self.history = History::new(200);
        self.saved_version = self.history.version();
        self.file_path = None;
        self.offset = Vec2::ZERO;
        self.zoom = 1.0;
    }

    /// Opens an image as a new document, its top-left corner at the origin.
    pub fn open_path(&mut self, path: PathBuf) {
        let image = match file::load(&path) {
            Ok(image) => image,
            Err(e) => {
                let name = path.file_name().unwrap_or(path.as_os_str());
                self.notify(format!("Couldn't open {}: {e}", name.to_string_lossy()));
                return;
            }
        };
        self.reset_document();
        let layer = &mut self.layers[0];
        if let Some(name) = path.file_name() {
            layer.name = name.to_string_lossy().into_owned();
        }
        let mode = PaintMode::Paint {
            color: [0; 4],
            opacity: 1.0,
        };
        PaintOp::new(layer.id, mode).blit(layer, &image, 0, 0);
        self.center_on = Some(IRect {
            x0: 0,
            y0: 0,
            x1: image.width as i32,
            y1: image.height as i32,
        });
        // Only remember formats we can write back to.
        self.file_path = file::Format::from_path(&path).map(|_| path);
    }

    fn open_dialog(&mut self) {
        let dialog = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp"]);
        self.show_dialog(DialogPurpose::Open, move || dialog.pick_file());
    }

    fn show_dialog(
        &mut self,
        purpose: DialogPurpose,
        run: impl FnOnce() -> Option<PathBuf> + Send + 'static,
    ) {
        if self.dialog.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        if cfg!(target_os = "macos") {
            // macOS only allows dialogs on the main thread; they can't hang there.
            tx.send(run()).ok();
        } else {
            std::thread::spawn(move || tx.send(run()).ok());
        }
        self.dialog = Some(FileDialog {
            purpose,
            result: rx,
        });
    }

    /// Acts on a file dialog's answer once it arrives.
    fn poll_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.dialog else {
            return;
        };
        let answer = match dialog.result.try_recv() {
            Ok(answer) => answer,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        };
        let purpose = dialog.purpose;
        self.dialog = None;
        match (purpose, answer) {
            (DialogPurpose::Open, Some(path)) => self.request(PendingAction::Open(path)),
            (DialogPurpose::SaveAs, Some(path)) => self.save_to(file::with_image_extension(path)),
            (_, None) => self.after_save = None,
        }
    }

    /// The visible layers flattened, cropped to what's drawn.
    fn export_image(&self) -> Option<Image> {
        let visible = |id: LayerId| self.layers.iter().any(|l| l.id == id && l.visible);
        let drawn = self
            .content_bounds
            .iter()
            .filter(|((id, _), _)| visible(*id))
            .map(|(_, b)| *b)
            .reduce(|a, b| a.union(&b));
        // With only a filled background there's no edge; save the view.
        let area = drawn.or_else(|| {
            let filled = self.layers.iter().any(|l| l.visible && l.default[3] > 0);
            filled.then(|| self.visible_canvas_rect(self.canvas_rect))
        })?;
        Some(canvas::copy_region_merged(&self.layers, area))
    }

    /// Saves to the current file, or asks where with `save_as` or for a new image.
    fn save(&mut self, save_as: bool) {
        match (&self.file_path, save_as) {
            (Some(path), false) => self.save_to(path.clone()),
            _ => {
                let name = self
                    .file_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map_or("untitled.png".into(), |n| n.to_string_lossy().into_owned());
                let mut dialog = rfd::FileDialog::new().set_file_name(name);
                for (label, extensions) in file::FILTERS {
                    dialog = dialog.add_filter(label, extensions);
                }
                self.show_dialog(DialogPurpose::SaveAs, move || dialog.save_file());
            }
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        self.commit_text();
        self.place_floating();
        let Some(image) = self.export_image() else {
            self.after_save = None;
            self.notify("Nothing to save yet: the canvas is empty.");
            return;
        };
        let name = path.file_name().map_or(path.display().to_string(), |n| {
            n.to_string_lossy().into_owned()
        });
        match file::save(&path, &image) {
            Ok(()) => {
                self.notify(format!("Saved {name} ({} × {})", image.width, image.height));
                self.file_path = Some(path);
                self.saved_version = self.history.version();
                if let Some(action) = self.after_save.take() {
                    self.run(action);
                }
            }
            Err(e) => {
                self.after_save = None;
                self.notify(format!("Couldn't save {name}: {e}"));
            }
        }
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending.clone() else {
            return;
        };
        let mut choice = None;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("The image has unsaved changes. Save them first?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some("save");
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some("discard");
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some("cancel");
                    }
                });
            });
        match choice {
            Some("save") => {
                // Runs the action once saving finishes (possibly after a dialog).
                self.pending = None;
                self.after_save = Some(action);
                self.save(false);
            }
            Some("discard") => {
                self.pending = None;
                self.run(action);
            }
            Some(_) => self.pending = None,
            None => {}
        }
    }

    fn message_popup(&mut self, ctx: &egui::Context) {
        let Some((text, at)) = &self.message else {
            return;
        };
        const SHOW_FOR: Duration = Duration::from_secs(4);
        if at.elapsed() > SHOW_FOR {
            self.message = None;
            return;
        }
        ctx.request_repaint_after(SHOW_FOR - at.elapsed());
        egui::Window::new("message")
            .title_bar(false)
            .resizable(false)
            .anchor(Align2::CENTER_BOTTOM, [0.0, -16.0])
            .show(ctx, |ui| ui.label(text.as_str()));
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("Untitled".into(), |n| n.to_string_lossy().into_owned());
        let title = format!("{}{name} – Paint", if self.unsaved() { "*" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New").on_hover_text("Ctrl+N").clicked() {
                    self.request(PendingAction::New);
                    ui.close();
                }
                if ui.button("Open…").on_hover_text("Ctrl+O").clicked() {
                    ui.close();
                    self.open_dialog();
                }
                if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
                    ui.close();
                    self.save(false);
                }
                if ui
                    .button("Save As…")
                    .on_hover_text("Ctrl+Shift+S")
                    .clicked()
                {
                    ui.close();
                    self.save(true);
                }
            });
            ui.separator();
            for (tool, name, key, icon) in Tool::ALL {
                // The grid and shape buttons show the chosen kind.
                let icon = match tool {
                    Tool::Grid => grid_icon(self.grid_kind),
                    Tool::Shape => shape_icon(self.shape_kind),
                    _ => icon,
                };
                let title = format!("{name} ({})", key.name());
                let button = icon_button(ui, icon, self.tool == tool);
                // Tools with kinds show a menu on hover instead of a tooltip.
                let has_menu = matches!(tool, Tool::Grid | Tool::Shape);
                let button = if has_menu {
                    button
                } else {
                    button.on_hover_text(&title)
                };
                if button.clicked() {
                    self.tool = tool;
                }
                if tool == Tool::Grid
                    && let Some(kind) = hover_menu(
                        ui,
                        &button,
                        &title,
                        &GridKind::ALL,
                        self.grid_kind,
                        grid_icon,
                        GridKind::label,
                    )
                {
                    if self.grid_kind != kind {
                        self.grid = None;
                    }
                    self.grid_kind = kind;
                    self.tool = Tool::Grid;
                }
                if tool == Tool::Shape
                    && let Some(kind) = hover_menu(
                        ui,
                        &button,
                        &title,
                        &ShapeKind::ALL,
                        self.shape_kind,
                        shape_icon,
                        ShapeKind::label,
                    )
                {
                    self.shape_kind = kind;
                    self.tool = Tool::Shape;
                }
            }
            ui.separator();

            self.color_pair(ui);
            for c in PALETTE.into_iter().chain([Color32::TRANSPARENT]) {
                let (rect, mut resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                swatch(ui.painter(), rect, c);
                let marked = if c == self.primary {
                    Some(ui.visuals().selection.bg_fill)
                } else if c == self.secondary {
                    Some(Color32::from_gray(200))
                } else {
                    None
                };
                if let Some(mark) = marked {
                    ui.painter().rect_stroke(
                        rect,
                        3.0,
                        Stroke::new(2.0, mark),
                        egui::StrokeKind::Inside,
                    );
                }
                if c == Color32::TRANSPARENT {
                    resp = resp.on_hover_text(
                        "Clear: fill, brush, shapes and text erase instead of painting",
                    );
                }
                if resp.clicked() {
                    self.primary = c;
                }
                if resp.secondary_clicked() {
                    self.secondary = c;
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

            let can_copy = self.active_selection().is_some() || self.floating.is_some();
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
                // The slider's built-in value box would pull it to the left, so
                // the value gets its own box below the slider.
                let brush = match self.tool {
                    Tool::Eraser => &mut self.eraser_brush,
                    Tool::Text => &mut self.text_brush,
                    _ => &mut self.paint_brush,
                };
                ui.vertical_centered(|ui| {
                    ui.label("Size");
                    centered_slider(
                        ui,
                        egui::Slider::new(&mut brush.size, MIN_SIZE..=MAX_SIZE)
                            .vertical()
                            .logarithmic(true)
                            .show_value(false),
                    );
                    ui.add(
                        egui::DragValue::new(&mut brush.size)
                            .range(MIN_SIZE..=MAX_SIZE)
                            .max_decimals(0),
                    );
                    ui.add_space(8.0);
                    ui.label("Opacity");
                    let mut pct = brush.opacity * 100.0;
                    centered_slider(
                        ui,
                        egui::Slider::new(&mut pct, 1.0..=100.0)
                            .vertical()
                            .show_value(false),
                    );
                    ui.add(
                        egui::DragValue::new(&mut pct)
                            .range(1.0..=100.0)
                            .max_decimals(0)
                            .suffix("%"),
                    );
                    brush.opacity = pct / 100.0;
                    if self.tool == Tool::Shape && self.shape_kind == ShapeKind::NGon {
                        ui.add_space(8.0);
                        ui.label("Sides");
                        centered_slider(
                            ui,
                            egui::Slider::new(&mut self.ngon_sides, 3..=20)
                                .vertical()
                                .show_value(false),
                        );
                        ui.add(egui::DragValue::new(&mut self.ngon_sides).range(3..=20));
                    }
                });
                match self.tool {
                    Tool::Fill => {
                        ui.separator();
                        ui.label("Tolerance");
                        ui.add(egui::DragValue::new(&mut self.fill_tolerance).range(0..=255));
                    }
                    Tool::Shape if self.shape_kind.can_fill() => {
                        ui.separator();
                        ui.checkbox(&mut self.fill_shapes, "Filled");
                    }
                    Tool::Shape if self.shape_kind == ShapeKind::Polyline => {
                        ui.separator();
                        ui.small(
                            "Click to add points. Double-click or Enter to finish; \
                             click the first point to close.",
                        );
                    }
                    Tool::Text => {
                        ui.separator();
                        ui.label("Font");
                        egui::ComboBox::from_id_salt("font")
                            .width(52.0)
                            .selected_text(self.text_face.label())
                            .show_ui(ui, |ui| {
                                for f in FontFace::ALL {
                                    ui.selectable_value(&mut self.text_face, f, f.label());
                                }
                            });
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
                    ui.checkbox(
                        &mut self.settings.paste_secondary_transparent,
                        "Color 2 → clear",
                    )
                    .on_hover_text("Treat pixels of color 2 in the pasted image as transparent");
                    ui.vertical_centered_justified(|ui| {
                        if ui.button("Place").on_hover_text("Enter").clicked() {
                            self.place_floating();
                        }
                        if ui.button("Cancel").on_hover_text("Esc").clicked() {
                            self.cancel_floating();
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
        let image = match (&self.floating, self.active_selection()) {
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
        let Some(sel) = self.active_selection() else {
            return;
        };
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
        let key = self.paste_transparent_color();
        self.floating = Some(Floating::pasted(image, x, y, key));
        self.selection = None;
    }

    /// Commits the floating piece onto the active layer. For a lifted
    /// selection, lifting and placing become one undo step.
    fn place_floating(&mut self) {
        let Some(f) = self.floating.take() else {
            return;
        };
        let layer_id = self.active_layer_id();
        let mode = PaintMode::Paint {
            color: [0; 4],
            opacity: 1.0,
        };
        let mut op = PaintOp::new(layer_id, mode);
        op.blit(&mut self.layers[self.active], &f.image, f.x(), f.y());
        let mut tiles = op.finish().unwrap_or_default();
        let placed = f.rect();
        match f.lifted {
            Some((lifted_from, lifted)) if lifted_from == layer_id => {
                // The tiles' state from before the lift wins over the
                // in-between state the blit recorded.
                let keys: std::collections::HashSet<TileKey> =
                    lifted.iter().map(|(k, _)| *k).collect();
                tiles.retain(|(k, _)| !keys.contains(k));
                tiles.extend(lifted);
                self.push_tiles(layer_id, tiles);
            }
            Some((lifted_from, lifted)) => {
                self.push_tiles(lifted_from, lifted);
                self.push_tiles(layer_id, tiles);
            }
            None => self.push_tiles(layer_id, tiles),
        }
        self.selection = Some(placed);
    }

    fn push_tiles(&mut self, layer_id: LayerId, tiles: Vec<(TileKey, Option<Vec<canvas::Rgba>>)>) {
        if !tiles.is_empty() {
            self.history.push(Action::Tiles {
                layer_id,
                tiles,
                default: None,
            });
        }
    }

    /// Drops the floating piece, putting a lifted selection back where it was.
    fn cancel_floating(&mut self) -> bool {
        let Some(f) = self.floating.take() else {
            return false;
        };
        if let Some((layer_id, lifted)) = f.lifted
            && let Some(layer) = self.layers.iter_mut().find(|l| l.id == layer_id)
        {
            for (key, tile) in lifted {
                layer.swap_tile(key, tile);
            }
        }
        true
    }

    /// Drops the floating piece for good (Delete / Cut): a lifted selection
    /// stays removed from its layer, as an undoable edit.
    fn discard_floating(&mut self) -> bool {
        let Some(f) = self.floating.take() else {
            return false;
        };
        if let Some((layer_id, lifted)) = f.lifted {
            self.push_tiles(layer_id, lifted);
        }
        true
    }

    /// Cuts the active selection out of its layer into a floating piece, so
    /// it can be moved and transformed.
    fn lift_selection(&mut self) -> bool {
        let Some(sel) = self.active_selection() else {
            return false;
        };
        let layer = &mut self.layers[self.active];
        let image = canvas::copy_region(layer, sel);
        let mut op = PaintOp::new(layer.id, PaintMode::Erase { strength: 1.0 });
        op.apply(layer, sel, |_, _| 1.0);
        let undo = op.finish().unwrap_or_default();
        self.floating = Some(Floating::lifted(image, sel.x0, sel.y0, layer.id, undo));
        self.selection = None;
        true
    }

    /// The box the transform handles sit on: the floating piece, or the
    /// selection while the Select tool is active.
    fn transform_frame(&self) -> Option<Frame> {
        match (&self.floating, self.active_selection()) {
            (Some(f), _) => Some(f.frame()),
            (None, Some(sel)) if sel.width() > 0 && sel.height() > 0 => Some(Frame::of_rect(sel)),
            _ => None,
        }
    }

    /// The transform handle under screen point `pos`, if any.
    fn handle_at(&self, rect: Rect, pos: Pos2) -> Option<Handle> {
        let frame = self.transform_frame()?;
        frame
            .handles(self.scale())
            .into_iter()
            .find(|(_, at)| self.to_screen(rect, *at).distance(pos) <= 8.0)
            .map(|(h, _)| h)
    }

    /// Applies `m` (in screen orientation) on top of the current transform,
    /// lifting the selection first if nothing floats yet.
    fn transform_by(&mut self, m: canvas::Matrix) {
        if self.floating.is_none() && !self.lift_selection() {
            return;
        }
        if let Some(f) = &mut self.floating {
            f.set_matrix(canvas::mat_mul(m, f.matrix));
        }
    }

    /// Mirror and quarter-turn buttons under the floating piece or selection.
    fn transform_bar(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.transform_frame() else {
            return;
        };
        if matches!(
            self.gesture,
            Some(Gesture::Transform { .. } | Gesture::MoveFloating { .. } | Gesture::Select { .. })
        ) {
            return;
        }
        let rect = self.canvas_rect;
        let corners = frame.corners().map(|c| self.to_screen(rect, c));
        let bottom = corners.iter().map(|c| c.y).fold(f32::MIN, f32::max);
        let cx = corners.iter().map(|c| c.x).sum::<f32>() / 4.0;
        let quarter = std::f32::consts::FRAC_PI_2;
        let actions: [(Icon, &str, canvas::Matrix); 4] = [
            (
                Icon::FlipHorizontal,
                "Mirror left ↔ right",
                [-1.0, 0.0, 0.0, 1.0],
            ),
            (
                Icon::FlipVertical,
                "Mirror top ↕ bottom",
                [1.0, 0.0, 0.0, -1.0],
            ),
            (
                Icon::RotateLeft,
                "Rotate 90° left",
                canvas::rotation(-quarter),
            ),
            (
                Icon::RotateRight,
                "Rotate 90° right",
                canvas::rotation(quarter),
            ),
        ];
        let mut picked = None;
        egui::Area::new(egui::Id::unique("transform_bar"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(Pos2::new(cx, bottom + 10.0))
            .show(ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (icon, tip, m) in actions {
                            if icon_button(ui, icon, false).on_hover_text(tip).clicked() {
                                picked = Some(m);
                            }
                        }
                    });
                });
            });
        if let Some(m) = picked {
            self.transform_by(m);
        }
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

    /// The selection, which only exists for the user while the Select tool is
    /// active: other tools neither show it nor act on it.
    fn active_selection(&self) -> Option<IRect> {
        self.selection.filter(|_| self.tool == Tool::Select)
    }

    /// Selects everything painted on any layer.
    fn select_all(&mut self) {
        self.place_floating();
        let all = self
            .content_bounds
            .values()
            .copied()
            .reduce(|a, b| a.union(&b));
        if all.is_some() {
            self.tool = Tool::Select;
            self.selection = all;
        }
    }

    /// Size and opacity for the current tool.
    fn brush(&self) -> Brush {
        match self.tool {
            Tool::Eraser => self.eraser_brush,
            Tool::Text => self.text_brush,
            _ => self.paint_brush,
        }
    }

    fn brush_mut(&mut self) -> &mut Brush {
        match self.tool {
            Tool::Eraser => &mut self.eraser_brush,
            Tool::Text => &mut self.text_brush,
            _ => &mut self.paint_brush,
        }
    }

    fn text_style(&self) -> TextStyle {
        TextStyle {
            face: self.text_face,
            size: self.text_brush.size,
        }
    }

    /// Corner of a grid cell under `p`; with the snap key held the cell
    /// becomes regular (square, equilateral triangle, regular hexagon).
    fn cell_end(&self, origin: [i32; 2], p: [f32; 2]) -> [i32; 2] {
        self.regular_end(origin, p, self.grid_kind.regular_ratio())
    }

    /// Corner of a rectangle, ellipse or triangle's box; with the snap key
    /// held it becomes a square, circle or equilateral triangle.
    fn shape_end(&self, origin: [i32; 2], p: [f32; 2]) -> [i32; 2] {
        self.regular_end(origin, p, self.shape_kind.regular_ratio())
    }

    /// The pixel corner under `p`; with the snap key held, moved so the box
    /// from `origin` has height / width `ratio`.
    fn regular_end(&self, origin: [i32; 2], p: [f32; 2], ratio: f32) -> [i32; 2] {
        let end = [p[0].round() as i32, p[1].round() as i32];
        if !self.square_held() {
            return end;
        }
        let (dx, dy) = (end[0] - origin[0], end[1] - origin[1]);
        let w = (dx.abs() as f32).max(dy.abs() as f32 / ratio);
        let sign = |v: i32| if v < 0 { -1.0 } else { 1.0 };
        [
            origin[0] + (sign(dx) * w).round() as i32,
            origin[1] + (sign(dy) * w * ratio).round() as i32,
        ]
    }

    /// Corners of the triangle or n-gon filling `r`.
    fn polygon_points(&self, r: IRect) -> Vec<[f32; 2]> {
        if r.width() <= 0 || r.height() <= 0 {
            return Vec::new();
        }
        canvas::triangle_points(r).to_vec()
    }

    /// Where an n-gon's far vertex goes for pointer `p`; the snap key turns
    /// it in 15° steps around vertex 0.
    fn ngon_end(&self, v0: [f32; 2], p: [f32; 2]) -> [f32; 2] {
        if self.square_held() {
            canvas::snap_rotation(v0, p, 15.0)
        } else {
            p
        }
    }

    fn commit_ngon(&mut self, v0: [f32; 2], vk: [f32; 2]) {
        let pts = canvas::ngon_from_vertices(v0, vk, self.ngon_sides);
        if pts.is_empty() {
            return;
        }
        let mut op = PaintOp::new(self.active_layer_id(), self.paint_mode());
        let (size, filled) = (self.paint_brush.size, self.fill_shapes);
        op.polygon(&mut self.layers[self.active], &pts, size, filled);
        self.commit(op);
    }

    /// End of a line from `start`, snapped to 45° steps with the snap key.
    fn line_end(&self, start: [f32; 2], p: [f32; 2]) -> [f32; 2] {
        let end = pixel_center(p);
        if self.square_held() {
            canvas::snap_angle(start, end)
        } else {
            end
        }
    }

    /// Adds a polyline point; clicking the last point again (a double click)
    /// finishes it, clicking the first point closes it.
    fn polyline_click(&mut self, rect: Rect, p: [f32; 2]) {
        let Some(poly) = &self.polyline else {
            self.polyline = Some(Polyline {
                points: vec![pixel_center(p)],
            });
            return;
        };
        let last = *poly.points.last().expect("polylines have a point");
        let first = poly.points[0];
        let point = self.line_end(last, p);
        let near = |a: [f32; 2]| {
            self.to_screen(rect, a)
                .distance(self.to_screen(rect, point))
                < 6.0
        };
        if near(last) {
            self.finish_polyline();
        } else if poly.points.len() >= 3 && near(first) {
            if let Some(poly) = &mut self.polyline {
                poly.points.push(first);
            }
            self.finish_polyline();
        } else if let Some(poly) = &mut self.polyline {
            poly.points.push(point);
        }
    }

    fn finish_polyline(&mut self) {
        let Some(poly) = self.polyline.take() else {
            return;
        };
        if poly.points.len() < 2 {
            return;
        }
        let mut op = PaintOp::new(
            self.active_layer_id(),
            self.color_mode(self.paint_brush.opacity),
        );
        op.polyline(
            &mut self.layers[self.active],
            &poly.points,
            self.paint_brush.size,
        );
        self.commit(op);
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
        if let Some(r) = self.center_on.take() {
            // Fit an opened image into the view (never above 100%).
            let view = rect.size() * self.pixels_per_point;
            let fit = (view.x / r.width() as f32).min(view.y / r.height() as f32) * 0.9;
            self.zoom = fit.min(1.0);
            let center = Vec2::new((r.x0 + r.x1) as f32 / 2.0, (r.y0 + r.y1) as f32 / 2.0);
            self.offset = center - rect.size() / self.scale() / 2.0;
        }
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        self.canvas_hover = response.hover_pos();
        self.handle_view_input(ui, rect, &response);
        self.handle_tool_input(ui, rect);
        self.update_preview(ui, rect);
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
            let brush = self.brush_mut();
            brush.size = (brush.size * f).clamp(MIN_SIZE, MAX_SIZE);
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
                            start_size: self.brush().size,
                        }),
                        PointerButton::Primary if space => Some(Gesture::Pan { button, last: pos }),
                        PointerButton::Primary | PointerButton::Secondary => {
                            // Like MS Paint: the right button paints with color 2.
                            // Shapes built from several clicks (polyline, grid) keep
                            // the color of their first click.
                            if self.polyline.is_none()
                                && self.grid.is_none()
                                && self.ngon_start.is_none()
                            {
                                self.use_secondary = button == PointerButton::Secondary;
                            }
                            self.gesture_button = button;
                            self.begin_tool(rect, pos);
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
                        Some(_) => button == self.gesture_button,
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
        let size = self.brush().size;
        let snap = self.square_held();
        let mut resized = None;
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
                    op.stroke_segment(layer, *last, p, size / 2.0);
                }
                *last = p;
            }
            Some(Gesture::Select { start }) => {
                let end = select_end.unwrap_or(*start);
                self.selection = Some(canvas::rect_from_corners(*start, end));
            }
            Some(Gesture::MoveFloating { grab }) => {
                if let Some(f) = &mut self.floating {
                    f.center = [p[0] - grab[0], p[1] - grab[1]];
                }
            }
            Some(Gesture::Transform {
                handle,
                start,
                frame,
            }) => {
                let m = frame.drag(*handle, *start, p, snap);
                if let Some(f) = &mut self.floating {
                    f.set_matrix(m);
                }
            }
            Some(Gesture::Resize { anchor, start_size }) => {
                // Doubles/halves the size every 100 px of horizontal movement.
                let dx = (pos.x - anchor.x) / 100.0 * self.settings.resize_sensitivity;
                resized = Some((*start_size * dx.exp2()).clamp(MIN_SIZE, MAX_SIZE));
            }
            _ => {}
        }
        if let Some(size) = resized {
            self.brush_mut().size = size;
        }
    }

    fn paint_mode(&self) -> PaintMode {
        match self.tool {
            Tool::Eraser => PaintMode::Erase {
                strength: self.brush().opacity,
            },
            _ => self.color_mode(self.brush().opacity),
        }
    }

    /// The color keyed out of pasted images: color 2, if enabled and not Clear.
    fn paste_transparent_color(&self) -> Option<[u8; 3]> {
        let [r, g, b, a] = self.secondary.to_array();
        (self.settings.paste_secondary_transparent && a > 0).then_some([r, g, b])
    }

    /// The color for the current click: color 2 after a right click.
    fn color(&self) -> Color32 {
        if self.use_secondary {
            self.secondary
        } else {
            self.primary
        }
    }

    /// Painting with the current color; the clear color erases instead.
    fn color_mode(&self, opacity: f32) -> PaintMode {
        if self.color() == Color32::TRANSPARENT {
            PaintMode::Erase { strength: opacity }
        } else {
            PaintMode::Paint {
                color: self.color().to_array(),
                opacity,
            }
        }
    }

    /// Color for on-screen previews; erasing shapes preview in gray.
    fn preview_color(&self) -> Color32 {
        if self.color() == Color32::TRANSPARENT {
            Color32::from_gray(128).gamma_multiply(0.6)
        } else {
            self.color().gamma_multiply(self.brush().opacity)
        }
    }

    fn begin_tool(&mut self, rect: Rect, pos: Pos2) {
        let brush = self.brush();
        let p = self.to_canvas(rect, pos);
        if let Some(handle) = self.handle_at(rect, pos) {
            if self.floating.is_some() || self.lift_selection() {
                let frame = self.floating.as_ref().expect("lifted above").frame();
                self.gesture = Some(Gesture::Transform {
                    handle,
                    start: p,
                    frame,
                });
            }
            return;
        }
        if let Some(f) = &self.floating {
            if f.frame().contains(p) {
                let grab = [p[0] - f.center[0], p[1] - f.center[1]];
                self.gesture = Some(Gesture::MoveFloating { grab });
            } else {
                // Clicking elsewhere drops the floating piece in place.
                self.place_floating();
            }
            return;
        }
        if self.tool == Tool::Select
            && self
                .active_selection()
                .is_some_and(|sel| Frame::of_rect(sel).contains(p))
        {
            // Dragging the selection moves its pixels.
            self.lift_selection();
            if let Some(f) = &self.floating {
                let grab = [p[0] - f.center[0], p[1] - f.center[1]];
                self.gesture = Some(Gesture::MoveFloating { grab });
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
                op.stroke_segment(&mut self.layers[self.active], p, p, brush.size / 2.0);
                self.gesture = Some(Gesture::Stroke { op, last: p });
            }
            Tool::Shape => match self.shape_kind {
                ShapeKind::Line => {
                    let start = pixel_center(p);
                    self.gesture = Some(Gesture::Line { start });
                }
                ShapeKind::Polyline => self.polyline_click(rect, p),
                // Vertex 0 here, the far vertex on release after a drag or on
                // the next click.
                ShapeKind::NGon => match self.ngon_start.take() {
                    Some(v0) => self.commit_ngon(v0, self.ngon_end(v0, p)),
                    None => {
                        self.gesture = Some(Gesture::NGon {
                            start: p,
                            press: pos,
                        })
                    }
                },
                _ => {
                    let start = [p[0].round() as i32, p[1].round() as i32];
                    self.gesture = Some(Gesture::Shape { start });
                }
            },
            Tool::Grid => self.grid_click(p),
            Tool::Text => {
                // A click finishes any text being typed and starts new text there.
                // The I-beam cursor points with its middle, so the first line is
                // centered vertically on the click rather than hanging below it.
                self.commit_text();
                let half_line = self.text_style().caret_height() / 2.0;
                self.text_edit = Some(TextEdit {
                    origin: [p[0].round() as i32, (p[1] - half_line).round() as i32],
                    buffer: text::Buffer::default(),
                });
            }
            Tool::Fill => {
                let mode = self.color_mode(brush.opacity);
                let layer = &mut self.layers[self.active];
                let (x, y) = (p[0].floor() as i32, p[1].floor() as i32);
                if let Some(fill) = canvas::flood_fill(layer, x, y, self.fill_tolerance, mode) {
                    self.history.push(Action::Tiles {
                        layer_id: layer.id,
                        tiles: fill.tiles,
                        default: fill.old_default,
                    });
                }
            }
            Tool::Select => unreachable!(),
        }
    }

    /// Advances the grid tool: start the cell, fix the cell, then draw the grid.
    fn grid_click(&mut self, p: [f32; 2]) {
        let brush = self.brush();
        self.grid = match self.grid {
            None => Some(GridState::Cell {
                origin: [p[0].round() as i32, p[1].round() as i32],
            }),
            Some(GridState::Cell { origin }) => {
                let end = self.cell_end(origin, p);
                let cell = [(end[0] - origin[0]).abs(), (end[1] - origin[1]).abs()];
                if cell[0] > 0 && cell[1] > 0 {
                    Some(GridState::Grid { origin, cell })
                } else {
                    self.grid
                }
            }
            Some(GridState::Grid { origin, cell }) => {
                let end = self.rect_end(origin, p);
                let kind = self.grid_kind;
                let (top_left, cols, rows) = kind.layout(origin, cell, end);
                if cols == 0 || rows == 0 {
                    return;
                }
                let mut op = PaintOp::new(self.active_layer_id(), self.paint_mode());
                let layer = &mut self.layers[self.active];
                if kind == GridKind::Rectangle {
                    // Pixel-aligned, with square corners.
                    let tl = [top_left[0] as i32, top_left[1] as i32];
                    op.grid(layer, tl, cell, cols, rows, brush.size);
                } else {
                    for [a, b] in kind.segments(top_left, cell, cols, rows) {
                        op.stroke_segment(layer, a, b, brush.size / 2.0);
                    }
                }
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
        let brush = self.brush();
        match self.gesture.take() {
            Some(Gesture::Stroke { op, .. }) => self.commit(op),
            Some(Gesture::Shape { start }) => {
                let Some(pos) = pos else { return };
                let r = canvas::rect_from_corners(
                    start,
                    self.shape_end(start, self.to_canvas(rect, pos)),
                );
                if r.width() <= 0 || r.height() <= 0 {
                    return;
                }
                let mut op = PaintOp::new(self.active_layer_id(), self.paint_mode());
                let layer = &mut self.layers[self.active];
                match self.shape_kind {
                    ShapeKind::Rectangle => op.rectangle(layer, r, brush.size, self.fill_shapes),
                    ShapeKind::Ellipse => op.ellipse(layer, r, brush.size, self.fill_shapes),
                    _ => {
                        let pts = canvas::triangle_points(r);
                        op.polygon(layer, &pts, brush.size, self.fill_shapes);
                    }
                }
                self.commit(op);
            }
            Some(Gesture::NGon { start, press }) => {
                let Some(pos) = pos else { return };
                if press.distance(pos) >= 5.0 {
                    self.commit_ngon(start, self.ngon_end(start, self.to_canvas(rect, pos)));
                } else {
                    // Just a click: wait for the second one.
                    self.ngon_start = Some(start);
                }
            }
            Some(Gesture::Line { start }) => {
                let Some(pos) = pos else { return };
                let end = self.line_end(start, self.to_canvas(rect, pos));
                let mut op = PaintOp::new(self.active_layer_id(), self.paint_mode());
                op.polyline(&mut self.layers[self.active], &[start, end], brush.size);
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
            Some(
                Gesture::MoveFloating { .. }
                | Gesture::Transform { .. }
                | Gesture::Resize { .. }
                | Gesture::Pan { .. },
            )
            | None => {}
        }
    }

    fn commit(&mut self, op: PaintOp) {
        let layer_id = op.layer_id;
        if let Some(tiles) = op.finish() {
            self.history.push(Action::Tiles {
                layer_id,
                tiles,
                default: None,
            });
        }
    }

    // ----- Rendering -----

    /// Uploads changed tiles to the GPU, drops textures of deleted layers/tiles,
    /// and refreshes layer thumbnails.
    fn sync_textures(&mut self, ctx: &egui::Context) {
        for layer in &mut self.layers {
            if std::mem::take(&mut layer.default_changed) {
                // Content is whatever differs from the default color, so every
                // tile's bounds change with it.
                self.thumbs_dirty = true;
                for (&key, tile) in &layer.tiles {
                    match canvas::tile_content_bounds(key, tile, layer.default) {
                        Some(b) => self.content_bounds.insert((layer.id, key), b),
                        None => self.content_bounds.remove(&(layer.id, key)),
                    };
                }
            }
            for key in layer.dirty.drain() {
                let tex_key = (layer.id, key);
                self.thumbs_dirty = true;
                match layer.tiles.get(&key) {
                    Some(tile) => {
                        match canvas::tile_content_bounds(key, tile, layer.default) {
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

        let key = self.paste_transparent_color();
        if let Some(f) = &mut self.floating {
            // Follows the setting and color 2 live while the paste is floating.
            f.set_transparent(key);
            if f.chunks.is_empty() {
                f.chunks = image_chunks(ctx, &f.image);
            }
        }
    }

    fn ngon_preview(&self, v0: [f32; 2], p: [f32; 2]) -> PreviewKey {
        let pts = canvas::ngon_from_vertices(v0, self.ngon_end(v0, p), self.ngon_sides);
        self.shape_preview(ShapeSpec::Polygon(pts, self.brush().size, self.fill_shapes))
    }

    fn shape_preview(&self, shape: ShapeSpec) -> PreviewKey {
        PreviewKey::Shape {
            shape,
            color: self.color().to_array(),
            opacity: self.brush().opacity,
        }
    }

    /// Renders the shape or text being drawn into the preview layer.
    fn update_preview(&mut self, ui: &egui::Ui, rect: Rect) {
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let key = match (&self.gesture, pointer, &self.text_edit) {
            (Some(Gesture::Shape { start }), Some(pos), _)
                if self.shape_kind != ShapeKind::Rectangle =>
            {
                let r = canvas::rect_from_corners(
                    *start,
                    self.shape_end(*start, self.to_canvas(rect, pos)),
                );
                let (size, filled) = (self.brush().size, self.fill_shapes);
                let shape = match self.shape_kind {
                    ShapeKind::Ellipse => ShapeSpec::Ellipse(r, size, filled),
                    _ => ShapeSpec::Polygon(self.polygon_points(r), size, filled),
                };
                Some(self.shape_preview(shape))
            }
            (Some(Gesture::Line { start }), Some(pos), _) => {
                let end = self.line_end(*start, self.to_canvas(rect, pos));
                let shape = ShapeSpec::Polyline(vec![*start, end], self.brush().size);
                Some(self.shape_preview(shape))
            }
            (Some(Gesture::NGon { start, .. }), Some(pos), _) => {
                Some(self.ngon_preview(*start, self.to_canvas(rect, pos)))
            }
            (None, Some(pos), _) if self.ngon_start.is_some() => {
                let start = self.ngon_start.expect("checked above");
                Some(self.ngon_preview(start, self.to_canvas(rect, pos)))
            }
            (None, pointer, _) if self.polyline.is_some() => {
                let poly = self.polyline.as_ref().expect("checked above");
                let mut points = poly.points.clone();
                if let Some(pos) = pointer {
                    let last = *points.last().expect("polylines have a point");
                    points.push(self.line_end(last, self.to_canvas(rect, pos)));
                }
                Some(self.shape_preview(ShapeSpec::Polyline(points, self.brush().size)))
            }
            (_, _, Some(edit)) => Some(PreviewKey::Text {
                origin: edit.origin,
                text: edit.buffer.text.clone(),
                style: self.text_style(),
                color: self.color().to_array(),
                opacity: self.brush().opacity,
            }),
            _ => None,
        };
        if key == self.preview_key {
            return;
        }
        self.preview.tiles.clear();
        let [r, g, b, a] = self.preview_color().to_srgba_unmultiplied();
        let preview_mode = PaintMode::Paint {
            color: [r, g, b, 255],
            opacity: a as f32 / 255.0,
        };
        let mut op = PaintOp::new(self.preview.id, preview_mode);
        match &key {
            Some(PreviewKey::Shape { shape, .. }) => shape.draw(&mut op, &mut self.preview),
            Some(PreviewKey::Text {
                origin,
                text,
                style,
                ..
            }) => draw_text(&mut op, &mut self.preview, *origin, *style, text),
            _ => {}
        }
        self.preview_key = key;
        self.preview.dirty.clear();
        let ctx = ui.ctx();
        self.preview_textures
            .retain(|k, _| self.preview.tiles.contains_key(k));
        for (key, tile) in &self.preview.tiles {
            let image = ColorImage::from_rgba_unmultiplied(
                [TILE as usize, TILE as usize],
                tile.as_flattened(),
            );
            match self.preview_textures.get_mut(key) {
                Some(tex) => tex.set(image, TEXTURE_OPTIONS),
                None => {
                    let tex = ctx.load_texture("preview", image, TEXTURE_OPTIONS);
                    self.preview_textures.insert(*key, tex);
                }
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
                None => vec![layer.default; THUMB * THUMB],
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
                self.paint_layer_default(&painter, rect, layer, (tx0, ty0, tx1, ty1));
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
            // Shape previews and pasted images show where they will land:
            // right above the active layer.
            if index == self.active && layer.visible {
                let tint = Color32::from_white_alpha((layer.opacity * 255.0).round() as u8);
                for (key, tex) in &self.preview_textures {
                    let r = IRect {
                        x0: key.0 * TILE,
                        y0: key.1 * TILE,
                        x1: (key.0 + 1) * TILE,
                        y1: (key.1 + 1) * TILE,
                    };
                    painter.image(tex.id(), self.screen_rect_of(rect, r), uv, tint);
                }
            }
            if index == self.active
                && let Some(f) = &self.floating
            {
                let (fx, fy) = (f.x(), f.y());
                for (chunk, tex) in &f.chunks {
                    let r = IRect {
                        x0: fx + chunk.x0,
                        y0: fy + chunk.y0,
                        x1: fx + chunk.x1,
                        y1: fy + chunk.y1,
                    };
                    painter.image(tex.id(), self.screen_rect_of(rect, r), uv, Color32::WHITE);
                }
            }
        }

        if let Some(frame) = self.transform_frame() {
            let corners = frame.corners().map(|c| self.to_screen(rect, c));
            marching_ants_polygon(&painter, &corners);
            let selecting = matches!(self.gesture, Some(Gesture::Select { .. }));
            if !selecting {
                paint_handles(
                    &painter,
                    &frame
                        .handles(self.scale())
                        .map(|(h, at)| (h, self.to_screen(rect, at))),
                );
            }
        }

        let outline = Stroke::new(1.0, Color32::from_gray(120));
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let color = self.preview_color();
        let pen = Stroke::new(self.brush().size * self.scale(), color);
        // Rectangle preview, and the size of whatever rectangle is being dragged.
        if let Some(pos) = pointer {
            let p = self.to_canvas(rect, pos);
            match &self.gesture {
                Some(Gesture::Shape { start }) => {
                    let r = canvas::rect_from_corners(*start, self.shape_end(*start, p));
                    if self.shape_kind == ShapeKind::Rectangle {
                        let sr = self.screen_rect_of(rect, r);
                        if self.fill_shapes {
                            painter.rect_filled(sr, 0.0, color);
                        } else {
                            painter.rect_stroke(sr, 0.0, pen, egui::StrokeKind::Middle);
                        }
                    }
                    info_label(&painter, pos, format!("{} × {}", r.width(), r.height()));
                }
                Some(Gesture::Line { start }) => {
                    info_label(&painter, pos, line_info(*start, self.line_end(*start, p)));
                }
                Some(Gesture::NGon { start, .. }) => {
                    info_label(&painter, pos, line_info(*start, self.ngon_end(*start, p)));
                }
                None if self.ngon_start.is_some() => {
                    let start = self.ngon_start.expect("checked above");
                    info_label(&painter, pos, line_info(start, self.ngon_end(start, p)));
                }
                None if self.polyline.is_some() => {
                    let poly = self.polyline.as_ref().expect("checked above");
                    let last = *poly.points.last().expect("polylines have a point");
                    info_label(&painter, pos, line_info(last, self.line_end(last, p)));
                }
                Some(Gesture::Select { .. }) => {
                    if let Some(sel) = self.selection {
                        info_label(&painter, pos, format!("{} × {}", sel.width(), sel.height()));
                    }
                }
                _ => {}
            }
        }

        // Text caret.
        if let Some(edit) = &self.text_edit {
            let style = self.text_style();
            let (line, before) = edit.buffer.caret_line();
            let x = edit.origin[0] as f32 + style.advance(before);
            let y = edit.origin[1] as f32 + line as f32 * style.line_height();
            let top = self.to_screen(rect, [x, y]);
            let bottom = self.to_screen(rect, [x, y + style.caret_height()]);
            painter.line_segment([top, bottom], Stroke::new(3.0, Color32::WHITE));
            painter.line_segment([top, bottom], Stroke::new(1.5, Color32::BLACK));
        }

        // Grid preview.
        if let (Some(grid), Some(pos)) = (self.grid, pointer) {
            let p = self.to_canvas(rect, pos);
            match grid {
                GridState::Cell { origin } => {
                    let r = canvas::rect_from_corners(origin, self.cell_end(origin, p));
                    let cell = [r.width(), r.height()];
                    let tl = [r.x0 as f32, r.y0 as f32];
                    let segments = self.grid_kind.segments(tl, cell, 1, 1);
                    self.paint_segments(&painter, rect, &segments, pen);
                    info_label(
                        &painter,
                        pos,
                        format!("cell {} × {}", r.width(), r.height()),
                    );
                }
                GridState::Grid { origin, cell } => {
                    let end = self.rect_end(origin, p);
                    let kind = self.grid_kind;
                    let (top_left, cols, rows) = kind.layout(origin, cell, end);
                    // The area being dragged out, and the cells that fit in it.
                    marching_ants(
                        &painter,
                        self.screen_rect_of(rect, canvas::rect_from_corners(origin, end)),
                    );
                    // Huge grids would just be a solid blob on screen; cap the line count.
                    if cols.saturating_mul(rows) <= 20_000 {
                        let segments = kind.segments(top_left, cell, cols, rows);
                        self.paint_segments(&painter, rect, &segments, pen);
                    }
                    let what = kind.label().to_lowercase();
                    info_label(&painter, pos, format!("{cols} × {rows} {what}"));
                }
            }
        }

        // While resizing, show the brush at the spot where the drag started.
        if let Some(Gesture::Resize { anchor, .. }) = &self.gesture {
            let r = (self.brush().size / 2.0 * self.scale()).max(1.0);
            painter.circle_stroke(*anchor, r, Stroke::new(2.0, Color32::WHITE));
            painter.circle_stroke(*anchor, r, Stroke::new(1.0, Color32::BLACK));
            painter.text(
                *anchor + Vec2::new(r + 8.0, 0.0),
                Align2::LEFT_CENTER,
                format!("{:.0} px", self.brush().size),
                egui::FontId::proportional(14.0),
                Color32::BLACK,
            );
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            return;
        }

        // Cursor under the pointer.
        if let Some(pos) = response.hover_pos() {
            let p = self.to_canvas(rect, pos);
            let handle = match &self.gesture {
                Some(Gesture::Transform { handle, .. }) => Some(*handle),
                None => self.handle_at(rect, pos),
                _ => None,
            };
            if let Some(handle) = handle {
                ui.ctx().set_cursor_icon(match handle {
                    Handle::Rotate => CursorIcon::Grab,
                    Handle::SkewTop | Handle::SkewBottom => CursorIcon::ResizeHorizontal,
                    Handle::SkewLeft | Handle::SkewRight => CursorIcon::ResizeVertical,
                });
                return;
            }
            let over_piece = self
                .floating
                .as_ref()
                .is_some_and(|f| f.frame().contains(p))
                || (self.tool == Tool::Select
                    && self.gesture.is_none()
                    && self
                        .active_selection()
                        .is_some_and(|sel| Frame::of_rect(sel).contains(p)));
            if over_piece || matches!(self.gesture, Some(Gesture::MoveFloating { .. })) {
                ui.ctx().set_cursor_icon(CursorIcon::Move);
                return;
            }
            match self.tool {
                Tool::Brush | Tool::Eraser => {
                    painter.circle_stroke(
                        pos,
                        (self.brush().size / 2.0 * self.scale()).max(1.0),
                        outline,
                    );
                }
                Tool::Text => ui.ctx().set_cursor_icon(CursorIcon::Text),
                Tool::Shape | Tool::Fill | Tool::Select | Tool::Grid => {
                    painter.line_segment([pos - Vec2::X * 6.0, pos + Vec2::X * 6.0], outline);
                    painter.line_segment([pos - Vec2::Y * 6.0, pos + Vec2::Y * 6.0], outline);
                }
            }
        }
    }
}

impl PaintApp {
    /// Paints a layer's default color everywhere in view that has no tile;
    /// tiles carry their own pixels. Gaps are merged into one rectangle per
    /// run of missing tiles in each tile row.
    fn paint_layer_default(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        layer: &Layer,
        (tx0, ty0, tx1, ty1): (i32, i32, i32, i32),
    ) {
        if layer.default[3] == 0 {
            return;
        }
        let [r, g, b, a] = layer.default;
        let color = Color32::from_rgba_unmultiplied(r, g, b, a).gamma_multiply(layer.opacity);
        let mut rows: HashMap<i32, Vec<i32>> = HashMap::new();
        for &(tx, ty) in layer.tiles.keys() {
            if (tx0..=tx1).contains(&tx) && (ty0..=ty1).contains(&ty) {
                rows.entry(ty).or_default().push(tx);
            }
        }
        for ty in ty0..=ty1 {
            let mut present = rows.remove(&ty).unwrap_or_default();
            present.sort_unstable();
            let mut start = tx0;
            for tx in present.into_iter().chain([tx1 + 1]) {
                if tx > start {
                    let gap = IRect {
                        x0: start * TILE,
                        y0: ty * TILE,
                        x1: tx * TILE,
                        y1: (ty + 1) * TILE,
                    };
                    painter.rect_filled(self.screen_rect_of(rect, gap), 0.0, color);
                }
                start = tx + 1;
            }
        }
    }

    fn paint_segments(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        segments: &[canvas::Segment],
        pen: Stroke,
    ) {
        for [a, b] in segments {
            painter.line_segment([self.to_screen(rect, *a), self.to_screen(rect, *b)], pen);
        }
    }
}

/// Adds a vertical slider so a centered parent layout can center it (the
/// slider itself always lays out left-aligned within whatever it is given).
fn centered_slider(ui: &mut egui::Ui, slider: egui::Slider<'_>) -> egui::Response {
    let thickness = ui
        .text_style_height(&egui::TextStyle::Body)
        .max(ui.spacing().interact_size.y);
    let size = Vec2::new(thickness, ui.spacing().slider_width);
    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.add(slider)
    })
    .inner
}

/// Renders `text` with its first line's top-left corner at `origin`.
fn draw_text(op: &mut PaintOp, layer: &mut Layer, origin: [i32; 2], style: TextStyle, text: &str) {
    let Some(mask) = text::render(style, text) else {
        return;
    };
    let (ox, oy) = (origin[0], origin[1]);
    let area = IRect {
        x0: ox + mask.x,
        y0: oy + mask.y,
        x1: ox + mask.x + mask.width as i32,
        y1: oy + mask.y + mask.height as i32,
    };
    op.apply(layer, area, |fx, fy| {
        mask.at(fx.floor() as i32 - ox, fy.floor() as i32 - oy)
    });
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

/// A list of kinds (shapes, grid shapes) for a tool button. It opens while
/// the pointer is over the button and stays open while the pointer is over
/// the list. Returns the kind picked.
fn hover_menu<K: Copy + PartialEq>(
    ui: &mut egui::Ui,
    button: &egui::Response,
    title: &str,
    kinds: &[K],
    current: K,
    icon: fn(K) -> Icon,
    label: fn(K) -> &'static str,
) -> Option<K> {
    let ctx = ui.ctx().clone();
    let id = button.id.with("hover_menu");
    let rect_id = id.with("rect");
    let mut open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let menu_rect = ctx.data(|d| d.get_temp::<Rect>(rect_id));
    let anchor = button.rect;
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let over = |r: Rect| pointer.is_some_and(|p| r.expand(4.0).contains(p));
    if button.hovered() {
        open = true;
    } else if !over(anchor) && !(open && menu_rect.is_some_and(over)) {
        open = false;
    }

    let mut picked = None;
    if open {
        let area = egui::Area::new(id)
            .order(egui::Order::Foreground)
            .fixed_pos(anchor.left_bottom())
            .show(&ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.label(egui::RichText::new(title).small().weak());
                    for &kind in kinds {
                        let (row, resp) =
                            ui.allocate_exact_size(Vec2::new(140.0, 26.0), Sense::click());
                        let visuals = ui.visuals();
                        if kind == current || resp.hovered() {
                            let fill = if kind == current {
                                visuals.selection.bg_fill
                            } else {
                                visuals.widgets.hovered.weak_bg_fill
                            };
                            ui.painter().rect_filled(row, 4.0, fill);
                        }
                        let text_color = visuals.text_color();
                        let icon_rect =
                            Rect::from_min_size(row.min + Vec2::new(4.0, 3.0), Vec2::splat(20.0));
                        icons::paint(ui.painter(), icon_rect, icon(kind), text_color);
                        ui.painter().text(
                            row.left_center() + Vec2::new(30.0, 0.0),
                            Align2::LEFT_CENTER,
                            label(kind),
                            egui::FontId::proportional(14.0),
                            text_color,
                        );
                        if resp.clicked() {
                            picked = Some(kind);
                        }
                    }
                });
            });
        ctx.data_mut(|d| d.insert_temp(rect_id, area.response.rect));
    }
    if picked.is_some() {
        open = false;
    }
    ctx.data_mut(|d| d.insert_temp(id, open));
    picked
}

fn shape_icon(kind: ShapeKind) -> Icon {
    match kind {
        ShapeKind::Rectangle => Icon::Rectangle,
        ShapeKind::Ellipse => Icon::Ellipse,
        ShapeKind::Triangle => Icon::Triangle,
        ShapeKind::NGon => Icon::NGon,
        ShapeKind::Line => Icon::Line,
        ShapeKind::Polyline => Icon::Polyline,
    }
}

/// Center of the pixel under `p`, so 1 px lines land on whole pixels.
fn pixel_center(p: [f32; 2]) -> [f32; 2] {
    [p[0].floor() + 0.5, p[1].floor() + 0.5]
}

/// Length and angle of a line, for the label next to the pointer.
fn line_info(a: [f32; 2], b: [f32; 2]) -> String {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    // Angles count counter-clockwise from the right, as on paper.
    // Round first so 359.6° shows as 0°, and `+ 0.0` turns -0 into 0.
    let angle = (-dy).atan2(dx).to_degrees().round().rem_euclid(360.0) + 0.0;
    format!("{:.0} px, {angle:.0}°", dx.hypot(dy))
}

fn grid_icon(kind: GridKind) -> Icon {
    match kind {
        GridKind::Rectangle => Icon::Grid,
        GridKind::Triangle => Icon::TriangleGrid,
        GridKind::Hexagon => Icon::HexagonGrid,
    }
}

/// Fills `rect` with `color`; the clear color is shown as white with a red slash.
fn swatch(painter: &egui::Painter, rect: Rect, color: Color32) {
    if color == Color32::TRANSPARENT {
        painter.rect_filled(rect, 2.0, Color32::WHITE);
        painter.line_segment(
            [rect.left_bottom(), rect.right_top()],
            Stroke::new(2.0, Color32::from_rgb(220, 40, 40)),
        );
    } else {
        painter.rect_filled(rect, 2.0, color);
    }
}

/// Draws a closed selection outline through screen points.
fn marching_ants_polygon(painter: &egui::Painter, corners: &[Pos2]) {
    let mut pts = corners.to_vec();
    pts.push(corners[0]);
    painter.add(Shape::line(pts.clone(), Stroke::new(1.0, Color32::WHITE)));
    painter.extend(Shape::dashed_line(
        &pts,
        Stroke::new(1.0, Color32::BLACK),
        4.0,
        4.0,
    ));
}

/// Draws transform handles: a round rotate handle on a stalk, diamonds for skew.
fn paint_handles(painter: &egui::Painter, handles: &[(Handle, Pos2)]) {
    let fill = Color32::WHITE;
    let line = Stroke::new(1.0, Color32::from_gray(60));
    let top = handles
        .iter()
        .find(|(h, _)| *h == Handle::SkewTop)
        .map(|(_, p)| *p);
    for &(handle, at) in handles {
        if handle == Handle::Rotate {
            if let Some(top) = top {
                painter.line_segment([top, at], line);
            }
            painter.circle(at, 5.5, fill, line);
        } else {
            let r = 5.0;
            painter.add(Shape::convex_polygon(
                vec![
                    at + Vec2::new(0.0, -r),
                    at + Vec2::new(r, 0.0),
                    at + Vec2::new(0.0, r),
                    at + Vec2::new(-r, 0.0),
                ],
                fill,
                line,
            ));
        }
    }
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
