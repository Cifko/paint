//! User preferences, persisted between runs.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, Key, Modifiers};
use serde::{Deserialize, Serialize};

use crate::shortcuts::{self, Action, Shortcut};
use crate::text::FontFace;

/// A modifier key that can be assigned to an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModifierKey {
    Alt,
    Ctrl,
    Shift,
    Disabled,
}

impl ModifierKey {
    const ALL: [ModifierKey; 4] = [Self::Alt, Self::Ctrl, Self::Shift, Self::Disabled];

    pub fn label(self) -> &'static str {
        match self {
            Self::Alt if cfg!(target_os = "macos") => "Option (⌥)",
            Self::Alt => "Alt",
            Self::Ctrl => "Ctrl",
            Self::Shift => "Shift",
            Self::Disabled => "Disabled",
        }
    }

    pub fn is_held(self, m: Modifiers) -> bool {
        match self {
            Self::Alt => m.alt,
            Self::Ctrl => m.ctrl,
            Self::Shift => m.shift,
            Self::Disabled => false,
        }
    }
}

/// Values the tools start with when the app opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolDefaults {
    pub brush_size: f32,
    pub brush_opacity: f32,
    pub eraser_size: f32,
    pub eraser_opacity: f32,
    pub text_size: f32,
    pub text_opacity: f32,
    pub font: FontFace,
    pub fill_tolerance: u8,
    pub filled_shapes: bool,
    pub ngon_sides: u32,
    pub color1: [u8; 4],
    pub color2: [u8; 4],
    pub undo_steps: usize,
}

impl Default for ToolDefaults {
    fn default() -> Self {
        Self {
            brush_size: 8.0,
            brush_opacity: 1.0,
            eraser_size: 24.0,
            eraser_opacity: 1.0,
            text_size: 32.0,
            text_opacity: 1.0,
            font: FontFace::Sans,
            fill_tolerance: 32,
            filled_shapes: false,
            ngon_sides: 6,
            color1: [0, 0, 0, 255],
            color2: [255, 255, 255, 255],
            undo_steps: 200,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Held to change the brush size by dragging / scrolling.
    pub resize_modifier: ModifierKey,
    /// Resize by dragging left/right while the modifier is held.
    pub resize_with_drag: bool,
    /// Resize with the mouse wheel while the modifier is held.
    pub resize_with_wheel: bool,
    /// Multiplier for how fast the size changes.
    pub resize_sensitivity: f32,
    /// Held while drawing a rectangle (selection, shape, grid) to make it square.
    pub square_modifier: ModifierKey,
    /// Make pixels of color 2 in pasted images transparent.
    #[serde(alias = "paste_white_transparent")]
    pub paste_secondary_transparent: bool,
    pub tool_defaults: ToolDefaults,
    /// Shortcuts changed from their defaults (`None` = no shortcut).
    pub shortcuts: BTreeMap<Action, Option<Shortcut>>,
    /// The action waiting for a key press in the settings window.
    #[serde(skip)]
    pub capturing: Option<Action>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            resize_modifier: ModifierKey::Alt,
            resize_with_drag: true,
            resize_with_wheel: true,
            resize_sensitivity: 1.0,
            square_modifier: ModifierKey::Shift,
            paste_secondary_transparent: true,
            tool_defaults: ToolDefaults::default(),
            shortcuts: BTreeMap::new(),
            capturing: None,
        }
    }
}

/// Adds a small button that resets `value` to `default` (disabled when it already is).
fn reset<T: PartialEq + Clone>(ui: &mut egui::Ui, value: &mut T, default: T) {
    let changed = *value != default;
    if ui
        .add_enabled(changed, egui::Button::new("↺").small())
        .on_hover_text("Reset to default")
        .clicked()
    {
        *value = default;
    }
}

fn color_edit(ui: &mut egui::Ui, rgba: &mut [u8; 4]) {
    let [r, g, b, a] = *rgba;
    let mut c = Color32::from_rgba_unmultiplied(r, g, b, a);
    egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque);
    *rgba = c.to_srgba_unmultiplied();
}

/// Opacity 0..=1 edited as a percentage.
fn percent(ui: &mut egui::Ui, value: &mut f32) {
    let mut pct = *value * 100.0;
    ui.add(
        egui::Slider::new(&mut pct, 1.0..=100.0)
            .integer()
            .suffix("%"),
    );
    *value = pct / 100.0;
}

impl Settings {
    pub const STORAGE_KEY: &'static str = "settings";

    /// The key bound to `action`, if any.
    pub fn binding(&self, action: Action) -> Option<Shortcut> {
        match self.shortcuts.get(&action) {
            Some(custom) => *custom,
            None => action.default_shortcut(),
        }
    }

    /// Text for tooltips, e.g. "Brush (B)".
    pub fn with_shortcut(&self, name: &str, action: Action) -> String {
        match self.binding(action) {
            Some(s) => format!("{name} ({})", s.label()),
            None => name.to_owned(),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let defaults = Settings::default();
        egui::ScrollArea::vertical()
            .max_height(520.0)
            .show(ui, |ui| {
                egui::CollapsingHeader::new("Tool defaults")
                    .default_open(true)
                    .show(ui, |ui| self.tool_defaults_ui(ui));
                egui::CollapsingHeader::new("Keyboard shortcuts")
                    .default_open(false)
                    .show(ui, |ui| self.shortcuts_ui(ui));
                egui::CollapsingHeader::new("Brush size key")
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::Grid::new("resize_settings")
                            .num_columns(3)
                            .spacing([12.0, 6.0])
                            .show(ui, |ui| {
                                ui.label("Hold key");
                                key_combo(ui, "resize_modifier", &mut self.resize_modifier);
                                reset(ui, &mut self.resize_modifier, defaults.resize_modifier);
                                ui.end_row();

                                ui.label("Drag left / right");
                                ui.checkbox(&mut self.resize_with_drag, "");
                                reset(ui, &mut self.resize_with_drag, defaults.resize_with_drag);
                                ui.end_row();

                                ui.label("Mouse wheel");
                                ui.checkbox(&mut self.resize_with_wheel, "");
                                reset(ui, &mut self.resize_with_wheel, defaults.resize_with_wheel);
                                ui.end_row();

                                ui.label("Sensitivity");
                                ui.add(
                                    egui::Slider::new(&mut self.resize_sensitivity, 0.2..=5.0)
                                        .logarithmic(true),
                                );
                                reset(
                                    ui,
                                    &mut self.resize_sensitivity,
                                    defaults.resize_sensitivity,
                                );
                                ui.end_row();
                            });
                        if cfg!(target_os = "linux") && self.resize_modifier == ModifierKey::Alt {
                            ui.small("Note: some Linux desktops use Alt + drag to move windows.");
                        }
                    });
                egui::CollapsingHeader::new("Drawing & pasting")
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::Grid::new("other_settings")
                            .num_columns(3)
                            .spacing([12.0, 6.0])
                            .show(ui, |ui| {
                                ui.label("Snap to square / 15° key");
                                key_combo(ui, "square_modifier", &mut self.square_modifier);
                                reset(ui, &mut self.square_modifier, defaults.square_modifier);
                                ui.end_row();

                                ui.label("Pasting: color 2 is transparent");
                                ui.checkbox(&mut self.paste_secondary_transparent, "");
                                reset(
                                    ui,
                                    &mut self.paste_secondary_transparent,
                                    defaults.paste_secondary_transparent,
                                );
                                ui.end_row();
                            });
                        if self.square_modifier != ModifierKey::Disabled
                            && self.square_modifier == self.resize_modifier
                            && self.resize_with_drag
                        {
                            ui.colored_label(
                                ui.visuals().warn_fg_color,
                                "Same key as brush resizing: pressing it starts a resize instead.",
                            );
                        }
                    });
            });
        ui.separator();
        if ui.button("Restore all defaults").clicked() {
            *self = Self::default();
        }
    }

    fn tool_defaults_ui(&mut self, ui: &mut egui::Ui) {
        ui.small("Used when the app starts.");
        let d = ToolDefaults::default();
        let t = &mut self.tool_defaults;
        egui::Grid::new("tool_defaults")
            .num_columns(3)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Brush size");
                ui.add(
                    egui::DragValue::new(&mut t.brush_size)
                        .range(1.0..=300.0)
                        .max_decimals(0),
                );
                reset(ui, &mut t.brush_size, d.brush_size);
                ui.end_row();

                ui.label("Brush opacity");
                percent(ui, &mut t.brush_opacity);
                reset(ui, &mut t.brush_opacity, d.brush_opacity);
                ui.end_row();

                ui.label("Eraser size");
                ui.add(
                    egui::DragValue::new(&mut t.eraser_size)
                        .range(1.0..=300.0)
                        .max_decimals(0),
                );
                reset(ui, &mut t.eraser_size, d.eraser_size);
                ui.end_row();

                ui.label("Eraser opacity");
                percent(ui, &mut t.eraser_opacity);
                reset(ui, &mut t.eraser_opacity, d.eraser_opacity);
                ui.end_row();

                ui.label("Text size");
                ui.add(
                    egui::DragValue::new(&mut t.text_size)
                        .range(1.0..=300.0)
                        .max_decimals(0),
                );
                reset(ui, &mut t.text_size, d.text_size);
                ui.end_row();

                ui.label("Text opacity");
                percent(ui, &mut t.text_opacity);
                reset(ui, &mut t.text_opacity, d.text_opacity);
                ui.end_row();

                ui.label("Font");
                egui::ComboBox::from_id_salt("default_font")
                    .selected_text(t.font.label())
                    .show_ui(ui, |ui| {
                        for f in FontFace::ALL {
                            ui.selectable_value(&mut t.font, f, f.label());
                        }
                    });
                reset(ui, &mut t.font, d.font);
                ui.end_row();

                ui.label("Fill tolerance");
                ui.add(egui::DragValue::new(&mut t.fill_tolerance).range(0..=255));
                reset(ui, &mut t.fill_tolerance, d.fill_tolerance);
                ui.end_row();

                ui.label("Filled shapes");
                ui.checkbox(&mut t.filled_shapes, "");
                reset(ui, &mut t.filled_shapes, d.filled_shapes);
                ui.end_row();

                ui.label("N-gon sides");
                ui.add(egui::DragValue::new(&mut t.ngon_sides).range(3..=20));
                reset(ui, &mut t.ngon_sides, d.ngon_sides);
                ui.end_row();

                ui.label("Color 1");
                color_edit(ui, &mut t.color1);
                reset(ui, &mut t.color1, d.color1);
                ui.end_row();

                ui.label("Color 2");
                color_edit(ui, &mut t.color2);
                reset(ui, &mut t.color2, d.color2);
                ui.end_row();

                ui.label("Undo steps");
                ui.add(egui::DragValue::new(&mut t.undo_steps).range(10..=2000));
                reset(ui, &mut t.undo_steps, d.undo_steps);
                ui.end_row();
            });
    }

    fn shortcuts_ui(&mut self, ui: &mut egui::Ui) {
        self.capture_key(ui);
        ui.small("Click a shortcut, then press the new keys (Esc cancels, Backspace removes it).");
        // Shortcuts used by more than one action.
        let mut clashes = Vec::new();
        for (i, (a, _, name_a)) in Action::ALL.iter().enumerate() {
            for (b, _, name_b) in &Action::ALL[i + 1..] {
                if let (Some(sa), Some(sb)) = (self.binding(*a), self.binding(*b))
                    && sa == sb
                {
                    clashes.push((
                        *a,
                        *b,
                        format!("{} is used by both {name_a} and {name_b}", sa.label()),
                    ));
                }
            }
        }
        let mut group = "";
        egui::Grid::new("shortcuts")
            .num_columns(3)
            .spacing([12.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                for (action, g, name) in Action::ALL {
                    if g != group {
                        group = g;
                        ui.strong(g);
                        ui.end_row();
                    }
                    ui.label(name);
                    let text = if self.capturing == Some(action) {
                        "Press keys…".to_owned()
                    } else {
                        self.binding(action).map_or("—".to_owned(), |s| s.label())
                    };
                    let clashing = clashes.iter().any(|(a, b, _)| *a == action || *b == action);
                    let mut button = egui::Button::new(text).min_size(egui::vec2(110.0, 0.0));
                    if clashing {
                        button = button.fill(ui.visuals().warn_fg_color.gamma_multiply(0.35));
                    }
                    if ui.add(button).clicked() {
                        self.capturing = Some(action);
                    }
                    let customized = self.shortcuts.contains_key(&action);
                    if ui
                        .add_enabled(customized, egui::Button::new("↺").small())
                        .on_hover_text("Reset to default")
                        .clicked()
                    {
                        self.shortcuts.remove(&action);
                    }
                    ui.end_row();
                }
            });
        for (_, _, text) in &clashes {
            ui.colored_label(ui.visuals().warn_fg_color, text);
        }
        ui.small(
            "Fixed: Ctrl+C / Ctrl+X / Ctrl+V copy, cut and paste; Enter places; Esc \
             cancels; Delete removes the selection; Space + drag pans.",
        );
        if ui.button("Reset all shortcuts").clicked() {
            self.shortcuts.clear();
        }
    }

    /// While waiting for a shortcut, takes the next key press.
    fn capture_key(&mut self, ui: &mut egui::Ui) {
        let Some(action) = self.capturing else {
            return;
        };
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, modifiers)) = pressed else {
            return;
        };
        match key {
            Key::Escape => {}
            Key::Backspace | Key::Delete if modifiers.is_none() => {
                self.shortcuts.insert(action, None);
            }
            key if shortcuts::is_reserved(key) => return,
            key => {
                let shortcut = Shortcut::from_press(key, modifiers);
                if Some(shortcut) == action.default_shortcut() {
                    self.shortcuts.remove(&action);
                } else {
                    self.shortcuts.insert(action, Some(shortcut));
                }
            }
        }
        self.capturing = None;
    }
}

fn key_combo(ui: &mut egui::Ui, id: &str, key: &mut ModifierKey) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(key.label())
        .show_ui(ui, |ui| {
            for m in ModifierKey::ALL {
                ui.selectable_value(key, m, m.label());
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_fall_back_to_defaults() {
        let mut s = Settings::default();
        assert_eq!(s.binding(Action::Undo), Some(Shortcut::command(Key::Z)));
        s.shortcuts
            .insert(Action::Undo, Some(Shortcut::key(Key::U)));
        assert_eq!(s.binding(Action::Undo), Some(Shortcut::key(Key::U)));
        s.shortcuts.insert(Action::Undo, None);
        assert_eq!(s.binding(Action::Undo), None);
    }

    #[test]
    fn settings_survive_a_save_roundtrip() {
        // eframe stores settings as RON.
        let mut s = Settings::default();
        s.shortcuts
            .insert(Action::ToolBrush, Some(Shortcut::command_shift(Key::F5)));
        s.shortcuts.insert(Action::Undo, None);
        s.tool_defaults.brush_size = 42.0;
        s.tool_defaults.font = FontFace::Mono;
        let text = ron::to_string(&s).unwrap();
        let back: Settings = ron::from_str(&text).unwrap();
        assert_eq!(back, s);
        // Settings saved by older versions (missing fields) still load.
        let old: Settings = ron::from_str("(resize_sensitivity: 2.0)").unwrap();
        assert_eq!(old.resize_sensitivity, 2.0);
        assert_eq!(old.tool_defaults, ToolDefaults::default());
    }
}
