//! User preferences, persisted between runs.

use eframe::egui::{self, Modifiers};
use serde::{Deserialize, Serialize};

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
        }
    }
}

impl Settings {
    pub const STORAGE_KEY: &'static str = "settings";

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Brush size shortcut");
        egui::Grid::new("resize_settings")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Hold key");
                key_combo(ui, "resize_modifier", &mut self.resize_modifier);
                ui.end_row();

                ui.label("Change size by");
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.resize_with_drag, "Dragging left / right");
                    ui.checkbox(&mut self.resize_with_wheel, "Mouse wheel");
                });
                ui.end_row();

                ui.label("Sensitivity");
                ui.add(
                    egui::Slider::new(&mut self.resize_sensitivity, 0.2..=5.0).logarithmic(true),
                );
                ui.end_row();
            });
        if cfg!(target_os = "linux") && self.resize_modifier == ModifierKey::Alt {
            ui.small("Note: some Linux desktops use Alt + drag to move windows.");
        }

        ui.separator();
        ui.heading("Rectangles");
        ui.horizontal(|ui| {
            ui.label("Snap to square while holding");
            key_combo(ui, "square_modifier", &mut self.square_modifier);
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

        ui.separator();
        ui.heading("Paste");
        ui.checkbox(
            &mut self.paste_secondary_transparent,
            "Treat color 2 as transparent",
        );
        ui.separator();
        if ui.button("Restore defaults").clicked() {
            *self = Self::default();
        }
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
