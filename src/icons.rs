//! Toolbar icons, drawn as vector shapes so they stay crisp at any DPI.

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Brush,
    Eraser,
    Fill,
    Rectangle,
    Ellipse,
    Grid,
    Select,
    Text,
}

/// Draws `icon` inside `rect` using `color`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    // Icons are designed on a 0..1 square.
    let s = rect.width().min(rect.height());
    let origin = rect.center() - Vec2::splat(s / 2.0);
    let p = |x: f32, y: f32| -> Pos2 { origin + Vec2::new(x, y) * s };
    let thin = Stroke::new((s * 0.08).max(1.0), color);

    match icon {
        Icon::Brush => {
            // Handle, ferrule and a round bristle tip.
            painter.line_segment([p(0.88, 0.12), p(0.52, 0.48)], Stroke::new(s * 0.12, color));
            painter.add(Shape::convex_polygon(
                vec![p(0.46, 0.42), p(0.58, 0.54), p(0.44, 0.68), p(0.32, 0.56)],
                color,
                Stroke::NONE,
            ));
            painter.add(Shape::convex_polygon(
                vec![
                    p(0.32, 0.58),
                    p(0.42, 0.68),
                    p(0.30, 0.86),
                    p(0.12, 0.88),
                    p(0.16, 0.72),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Eraser => {
            // A tilted block with its rubber end filled.
            let body = [p(0.55, 0.15), p(0.88, 0.48), p(0.50, 0.86), p(0.17, 0.53)];
            painter.add(Shape::convex_polygon(
                vec![body[3], p(0.36, 0.34), p(0.69, 0.67), body[2]],
                color,
                Stroke::NONE,
            ));
            painter.add(Shape::closed_line(body.to_vec(), thin));
            painter.line_segment([p(0.30, 0.90), p(0.85, 0.90)], thin);
        }
        Icon::Fill => {
            // A tipped bucket with a drop of paint.
            let bucket = vec![p(0.40, 0.18), p(0.74, 0.52), p(0.46, 0.80), p(0.12, 0.46)];
            painter.add(Shape::closed_line(bucket, thin));
            painter.add(Shape::convex_polygon(
                vec![p(0.12, 0.46), p(0.74, 0.52), p(0.46, 0.80)],
                color,
                Stroke::NONE,
            ));
            painter.line_segment([p(0.40, 0.18), p(0.46, 0.06)], thin);
            painter.circle_filled(p(0.84, 0.74), s * 0.09, color);
            painter.add(Shape::convex_polygon(
                vec![p(0.84, 0.54), p(0.92, 0.72), p(0.76, 0.72)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Rectangle => {
            painter.rect_stroke(
                Rect::from_min_max(p(0.12, 0.24), p(0.88, 0.76)),
                0.0,
                thin,
                StrokeKind::Middle,
            );
        }
        Icon::Ellipse => {
            let pts: Vec<Pos2> = (0..=48)
                .map(|i| {
                    let t = i as f32 / 48.0 * std::f32::consts::TAU;
                    p(0.5 + 0.40 * t.cos(), 0.5 + 0.30 * t.sin())
                })
                .collect();
            painter.add(Shape::line(pts, thin));
        }
        Icon::Grid => {
            for i in 0..4 {
                let t = 0.14 + i as f32 * 0.24;
                painter.line_segment([p(t, 0.14), p(t, 0.86)], thin);
                painter.line_segment([p(0.14, t), p(0.86, t)], thin);
            }
        }
        Icon::Text => {
            let bold = Stroke::new((s * 0.14).max(1.5), color);
            painter.line_segment([p(0.18, 0.18), p(0.82, 0.18)], bold);
            painter.line_segment([p(0.5, 0.18), p(0.5, 0.86)], bold);
            painter.line_segment([p(0.36, 0.86), p(0.64, 0.86)], thin);
        }
        Icon::Select => {
            let pts = [
                p(0.14, 0.2),
                p(0.86, 0.2),
                p(0.86, 0.8),
                p(0.14, 0.8),
                p(0.14, 0.2),
            ];
            painter.extend(Shape::dashed_line(&pts, thin, s * 0.12, s * 0.08));
        }
    }
}
