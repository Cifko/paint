//! User preferences, persisted between runs.

use eframe::egui::{self, Modifiers};
use serde::{Deserialize, Serialize};

/// Modifier key that, while held, turns mouse drag / wheel into brush resizing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResizeModifier {
    Alt,
    Ctrl,
    Shift,
    Disabled,
}

impl ResizeModifier {
    const ALL: [ResizeModifier; 4] = [Self::Alt, Self::Ctrl, Self::Shift, Self::Disabled];

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
    pub resize_modifier: ResizeModifier,
    /// Resize by dragging left/right while the modifier is held.
    pub resize_with_drag: bool,
    /// Resize with the mouse wheel while the modifier is held.
    pub resize_with_wheel: bool,
    /// Multiplier for how fast the size changes.
    pub resize_sensitivity: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            resize_modifier: ResizeModifier::Alt,
            resize_with_drag: true,
            resize_with_wheel: true,
            resize_sensitivity: 1.0,
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
                egui::ComboBox::from_id_salt("resize_modifier")
                    .selected_text(self.resize_modifier.label())
                    .show_ui(ui, |ui| {
                        for m in ResizeModifier::ALL {
                            ui.selectable_value(&mut self.resize_modifier, m, m.label());
                        }
                    });
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
        if cfg!(target_os = "linux") && self.resize_modifier == ResizeModifier::Alt {
            ui.small("Note: some Linux desktops use Alt + drag to move windows.");
        }
        ui.separator();
        if ui.button("Restore defaults").clicked() {
            *self = Self::default();
        }
    }
}
