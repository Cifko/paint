//! The color picker: a saturation/value square and a hue strip, with the
//! numbers (RGB, HSV or hex) next to it.

use eframe::egui::{self, Color32, Mesh, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2};
use serde::{Deserialize, Serialize};

/// How the color's numbers are shown and typed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorMode {
    #[default]
    Rgb,
    Hsv,
    Hex,
}

impl ColorMode {
    const ALL: [(ColorMode, &'static str); 3] =
        [(Self::Rgb, "RGB"), (Self::Hsv, "HSV"), (Self::Hex, "Hex")];
}

const SQUARE: f32 = 180.0;
const STRIP_HEIGHT: f32 = 16.0;
/// Width of every number field, so they don't change size with their value.
const FIELD_WIDTH: f32 = 56.0;

/// Hue, saturation and value, all 0..=1, in sRGB space like other apps show
/// them. Kept between frames so the hue survives gray and black.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hsv {
    h: f32,
    s: f32,
    v: f32,
}

impl Hsv {
    fn from_color(c: Color32) -> Self {
        let [r, g, b] = [c.r(), c.g(), c.b()].map(|x| x as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d == 0.0 {
            0.0
        } else if max == r {
            ((g - b) / d).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / d + 2.0) / 6.0
        } else {
            ((r - g) / d + 4.0) / 6.0
        };
        let s = if max == 0.0 { 0.0 } else { d / max };
        Self { h, s, v: max }
    }

    fn to_color(self) -> Color32 {
        let h = (self.h.rem_euclid(1.0)) * 6.0;
        let c = self.v * self.s;
        let x = c * (1.0 - (h % 2.0 - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = self.v - c;
        let byte = |x: f32| ((x + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Color32::from_rgb(byte(r), byte(g), byte(b))
    }
}

/// What the picker remembers between frames.
#[derive(Clone, Default)]
struct State {
    /// The HSV for `color`, unless the color was changed from elsewhere.
    hsv: Option<(Color32, Hsv)>,
    /// The hex text being typed.
    hex: String,
}

/// Shows the picker. Returns whether the color changed.
pub fn show(ui: &mut egui::Ui, color: &mut Color32, mode: &mut ColorMode) -> bool {
    let id = ui.scope_id().with("color_picker");
    let mut state: State = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let mut hsv = match state.hsv {
        Some((c, hsv)) if c == *color => hsv,
        _ => Hsv::from_color(*color),
    };
    let before = hsv;
    let mut typed = None;

    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            sv_square(ui, &mut hsv);
            hue_strip(ui, &mut hsv);
        });
        ui.vertical(|ui| {
            ui.set_width(3.0 * FIELD_WIDTH);
            ui.horizontal(|ui| {
                for (m, label) in ColorMode::ALL {
                    ui.selectable_value(mode, m, label);
                }
            });
            ui.add_space(4.0);
            ui.spacing_mut().interact_size.x = FIELD_WIDTH;
            match mode {
                ColorMode::Rgb => typed = rgb_fields(ui, hsv.to_color()),
                ColorMode::Hsv => hsv_fields(ui, &mut hsv),
                ColorMode::Hex => typed = hex_field(ui, &mut state.hex, hsv.to_color()),
            }
            ui.add_space(4.0);
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), 2.0 * STRIP_HEIGHT),
                Sense::hover(),
            );
            ui.painter().rect_filled(rect, 3.0, hsv.to_color());
            ui.painter().rect_stroke(
                rect,
                3.0,
                Stroke::new(1.0, Color32::from_gray(120)),
                StrokeKind::Inside,
            );
        });
    });

    let changed = if let Some(c) = typed.filter(|c| c != color) {
        hsv = Hsv::from_color(c);
        *color = c;
        true
    } else if hsv != before {
        *color = hsv.to_color();
        true
    } else {
        false
    };
    state.hsv = Some((*color, hsv));
    ui.data_mut(|d| d.insert_temp(id, state));
    changed
}

/// Saturation left to right, value bottom to top.
fn sv_square(ui: &mut egui::Ui, hsv: &mut Hsv) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(SQUARE), Sense::click_and_drag());
    if let Some(pos) = response.interact_pointer_pos() {
        hsv.s = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        hsv.v = 1.0 - ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
    }
    const N: u32 = 24;
    let mut mesh = Mesh::default();
    for row in 0..=N {
        for col in 0..=N {
            let (s, v) = (col as f32 / N as f32, 1.0 - row as f32 / N as f32);
            let pos = rect.lerp_inside(Vec2::new(col as f32, row as f32) / N as f32);
            mesh.colored_vertex(pos, Hsv { s, v, ..*hsv }.to_color());
        }
    }
    for row in 0..N {
        for col in 0..N {
            let i = row * (N + 1) + col;
            mesh.add_triangle(i, i + 1, i + N + 1);
            mesh.add_triangle(i + 1, i + N + 2, i + N + 1);
        }
    }
    ui.painter().add(Shape::mesh(mesh));
    let marker = rect.lerp_inside(Vec2::new(hsv.s, 1.0 - hsv.v));
    let contrast = if hsv.v > 0.5 && hsv.s < 0.5 {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    ui.painter()
        .circle_stroke(marker, 5.0, Stroke::new(1.5, contrast));
}

fn hue_strip(ui: &mut egui::Ui, hsv: &mut Hsv) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(SQUARE, STRIP_HEIGHT), Sense::click_and_drag());
    if let Some(pos) = response.interact_pointer_pos() {
        hsv.h = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    }
    const N: u32 = 36;
    let mut mesh = Mesh::default();
    for i in 0..=N {
        let t = i as f32 / N as f32;
        let color = Hsv {
            h: t,
            s: 1.0,
            v: 1.0,
        }
        .to_color();
        mesh.colored_vertex(
            Pos2::new(rect.lerp_inside(Vec2::new(t, 0.0)).x, rect.top()),
            color,
        );
        mesh.colored_vertex(
            Pos2::new(rect.lerp_inside(Vec2::new(t, 0.0)).x, rect.bottom()),
            color,
        );
        if i < N {
            let k = 2 * i;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 3, k + 2);
        }
    }
    ui.painter().add(Shape::mesh(mesh));
    let x = rect.lerp_inside(Vec2::new(hsv.h, 0.0)).x;
    let marker = Rect::from_center_size(
        Pos2::new(x, rect.center().y),
        Vec2::new(4.0, rect.height() + 4.0),
    );
    ui.painter().rect_stroke(
        marker,
        1.0,
        Stroke::new(1.5, Color32::WHITE),
        StrokeKind::Middle,
    );
    ui.painter().rect_stroke(
        marker.expand(1.5),
        1.0,
        Stroke::new(1.0, Color32::BLACK),
        StrokeKind::Middle,
    );
}

/// A labelled number field of the fixed width.
fn field(ui: &mut egui::Ui, label: &str, drag: egui::DragValue<'_>) -> bool {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 18.0), Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        ui.add_sized([FIELD_WIDTH, ui.spacing().interact_size.y], drag)
            .changed()
    })
    .inner
}

fn rgb_fields(ui: &mut egui::Ui, color: Color32) -> Option<Color32> {
    let mut rgb = [color.r(), color.g(), color.b()];
    let mut changed = false;
    for (label, value) in ["R", "G", "B"].into_iter().zip(&mut rgb) {
        changed |= field(ui, label, egui::DragValue::new(value).range(0..=255));
    }
    changed.then(|| Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
}

/// A whole-number field from 0 to `max`.
fn whole<'a>(value: &'a mut f32, max: f32, suffix: &'static str) -> egui::DragValue<'a> {
    egui::DragValue::new(value)
        .range(0.0..=max)
        .max_decimals(0)
        .suffix(suffix)
}

fn hsv_fields(ui: &mut egui::Ui, hsv: &mut Hsv) {
    let mut h = hsv.h * 360.0;
    let mut s = hsv.s * 100.0;
    let mut v = hsv.v * 100.0;
    if field(ui, "H", whole(&mut h, 360.0, "°")) {
        hsv.h = h / 360.0;
    }
    if field(ui, "S", whole(&mut s, 100.0, "%")) {
        hsv.s = s / 100.0;
    }
    if field(ui, "V", whole(&mut v, 100.0, "%")) {
        hsv.v = v / 100.0;
    }
}

/// `#RRGGBB`, accepting `RGB` / `RRGGBB` with or without `#` while typing.
fn hex_field(ui: &mut egui::Ui, text: &mut String, color: Color32) -> Option<Color32> {
    let current = format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b());
    let id = ui.scope_id().with("hex");
    let editing = ui.memory(|m| m.has_focus(id));
    if !editing {
        *text = current;
    }
    let response = ui.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .char_limit(7)
            .desired_width(2.0 * FIELD_WIDTH)
            .font(egui::TextStyle::Monospace),
    );
    response.changed().then(|| parse_hex(text)).flatten()
}

fn parse_hex(text: &str) -> Option<Color32> {
    let digits = text.trim().trim_start_matches('#');
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => {
            let [_, r, g, b] = value.to_be_bytes();
            Some(Color32::from_rgb(r, g, b))
        }
        3 => {
            let nibble = |shift: u32| ((value >> shift) & 0xF) as u8 * 17;
            Some(Color32::from_rgb(nibble(8), nibble(4), nibble(0)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips() {
        for c in [
            Color32::from_rgb(255, 0, 0),
            Color32::from_rgb(12, 200, 99),
            Color32::from_rgb(40, 40, 40),
            Color32::from_rgb(1, 2, 254),
            Color32::WHITE,
            Color32::BLACK,
        ] {
            assert_eq!(Hsv::from_color(c).to_color(), c);
        }
        let hsv = Hsv::from_color(Color32::from_rgb(0, 255, 255));
        assert!((hsv.h - 0.5).abs() < 1e-6 && hsv.s == 1.0 && hsv.v == 1.0);
    }

    #[test]
    fn parses_hex() {
        assert_eq!(parse_hex("#FF8000"), Some(Color32::from_rgb(255, 128, 0)));
        assert_eq!(parse_hex("ff8000"), Some(Color32::from_rgb(255, 128, 0)));
        assert_eq!(parse_hex("#f80"), Some(Color32::from_rgb(255, 136, 0)));
        assert_eq!(parse_hex("#FF80"), None);
        assert_eq!(parse_hex("#GG0000"), None);
    }
}
