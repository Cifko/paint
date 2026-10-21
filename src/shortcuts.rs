//! Rebindable keyboard shortcuts.

use eframe::egui::{Event, Key, Modifiers};
use serde::{Deserialize, Serialize};

/// Everything that can be bound to a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Action {
    ToolBrush,
    ToolEraser,
    ToolFill,
    ToolShape,
    ToolGrid,
    ToolSelect,
    ToolText,
    ShapeRectangle,
    ShapeEllipse,
    ShapeNGon,
    ShapeLine,
    ShapePolyline,
    Undo,
    Redo,
    RedoAlternative,
    SelectAll,
    CopyMerged,
    SwapColors,
    FitToWindow,
    ResetView,
    New,
    Open,
    Save,
    SaveAs,
}

impl Action {
    /// All actions with the group and name shown in the settings.
    pub const ALL: [(Action, &'static str, &'static str); 24] = [
        (Self::ToolBrush, "Tools", "Brush"),
        (Self::ToolEraser, "Tools", "Eraser"),
        (Self::ToolFill, "Tools", "Fill"),
        (Self::ToolShape, "Tools", "Shape (current)"),
        (Self::ToolGrid, "Tools", "Grid"),
        (Self::ToolSelect, "Tools", "Select"),
        (Self::ToolText, "Tools", "Text"),
        (Self::ShapeRectangle, "Shapes", "Rectangle"),
        (Self::ShapeEllipse, "Shapes", "Ellipse"),
        (Self::ShapeNGon, "Shapes", "N-gon"),
        (Self::ShapeLine, "Shapes", "Line"),
        (Self::ShapePolyline, "Shapes", "Polyline"),
        (Self::Undo, "Edit", "Undo"),
        (Self::Redo, "Edit", "Redo"),
        (Self::RedoAlternative, "Edit", "Redo (alternative)"),
        (Self::SelectAll, "Edit", "Select all"),
        (Self::CopyMerged, "Edit", "Copy merged"),
        (Self::SwapColors, "Edit", "Swap colors"),
        (Self::FitToWindow, "View", "Fit to window"),
        (Self::ResetView, "View", "Reset view (100%)"),
        (Self::New, "File", "New"),
        (Self::Open, "File", "Open"),
        (Self::Save, "File", "Save"),
        (Self::SaveAs, "File", "Save as"),
    ];

    pub fn default_shortcut(self) -> Option<Shortcut> {
        use Key::*;
        let s = match self {
            Self::ToolBrush => Shortcut::key(B),
            Self::ToolEraser => Shortcut::key(E),
            Self::ToolFill => Shortcut::key(G),
            Self::ToolShape => Shortcut::key(S),
            Self::ToolGrid => Shortcut::key(H),
            Self::ToolSelect => Shortcut::key(M),
            Self::ToolText => Shortcut::key(T),
            Self::ShapeRectangle => Shortcut::key(R),
            Self::ShapeEllipse => Shortcut::key(C),
            Self::ShapeNGon => Shortcut::key(N),
            Self::ShapeLine => Shortcut::key(L),
            Self::ShapePolyline => Shortcut::key(P),
            Self::Undo => Shortcut::command(Z),
            Self::Redo => Shortcut::command_shift(Z),
            Self::RedoAlternative => Shortcut::command(Y),
            Self::SelectAll => Shortcut::command(A),
            Self::CopyMerged => Shortcut::command_shift(C),
            Self::SwapColors => Shortcut::key(X),
            Self::FitToWindow => Shortcut::command(Num0),
            Self::ResetView => Shortcut::command(Num1),
            Self::New => Shortcut::command(N),
            Self::Open => Shortcut::command(O),
            Self::Save => Shortcut::command(S),
            Self::SaveAs => Shortcut::command_shift(S),
        };
        Some(s)
    }
}

/// A key plus modifiers. `command` is Ctrl on Windows/Linux and ⌘ on macOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    pub key: Key,
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Shortcut {
    pub const fn key(key: Key) -> Self {
        Self {
            key,
            command: false,
            shift: false,
            alt: false,
        }
    }

    pub const fn command(key: Key) -> Self {
        Self {
            command: true,
            ..Self::key(key)
        }
    }

    pub const fn command_shift(key: Key) -> Self {
        Self {
            shift: true,
            ..Self::command(key)
        }
    }

    pub fn from_press(key: Key, m: Modifiers) -> Self {
        Self {
            key,
            command: m.command,
            shift: m.shift,
            alt: m.alt,
        }
    }

    /// Whether `key` with exactly these modifiers is this shortcut.
    pub fn matches(&self, key: Key, m: Modifiers) -> bool {
        key == self.key && m.command == self.command && m.shift == self.shift && m.alt == self.alt
    }

    /// Whether this shortcut was pressed among `events`.
    pub fn pressed(&self, events: &[Event]) -> bool {
        events.iter().any(|e| {
            matches!(e, Event::Key { key, pressed: true, modifiers, .. }
                if self.matches(*key, *modifiers))
        })
    }

    /// Human-readable form, e.g. "Ctrl+Shift+Z" (or "⌘⇧Z" on macOS).
    pub fn label(&self) -> String {
        let mac = cfg!(target_os = "macos");
        let mut parts = Vec::new();
        if self.command {
            parts.push(if mac { "⌘" } else { "Ctrl" });
        }
        if self.alt {
            parts.push(if mac { "⌥" } else { "Alt" });
        }
        if self.shift {
            parts.push(if mac { "⇧" } else { "Shift" });
        }
        parts.push(self.key.symbol_or_name());
        parts.join(if mac { "" } else { "+" })
    }
}

/// Keys that can't be bound on their own (they're used to cancel, clear or
/// confirm while choosing a shortcut, or are needed elsewhere).
pub fn is_reserved(key: Key) -> bool {
    matches!(
        key,
        Key::Escape | Key::Enter | Key::Backspace | Key::Delete | Key::Space | Key::Tab
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn modifiers_must_match_exactly() {
        let undo = Shortcut::command(Key::Z);
        let redo = Shortcut::command_shift(Key::Z);
        let ctrl_z = [press(Key::Z, Modifiers::COMMAND)];
        let ctrl_shift_z = [press(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)];
        assert!(undo.pressed(&ctrl_z) && !redo.pressed(&ctrl_z));
        assert!(redo.pressed(&ctrl_shift_z) && !undo.pressed(&ctrl_shift_z));
        assert!(!Shortcut::key(Key::Z).pressed(&ctrl_z));
    }

    #[test]
    fn defaults_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (action, _, _) in Action::ALL {
            let s = action.default_shortcut().unwrap();
            assert!(
                seen.insert((s.key, s.command, s.shift, s.alt)),
                "{action:?} clashes"
            );
        }
    }

    #[test]
    fn labels() {
        if !cfg!(target_os = "macos") {
            assert_eq!(Shortcut::command_shift(Key::Z).label(), "Ctrl+Shift+Z");
            assert_eq!(Shortcut::key(Key::B).label(), "B");
        }
    }
}
