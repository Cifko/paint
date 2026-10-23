//! A floating piece – a pasted image or a lifted selection – that can be
//! moved, rotated, scaled, skewed and mirrored before it's placed onto a layer.

use eframe::egui::TextureHandle;

use crate::canvas::{self, IDENTITY, IRect, Image, LayerId, Matrix, Rgba, TileKey};

/// Tiles as they were before an edit, for undo.
pub type TileUndo = Vec<(TileKey, Option<Vec<Rgba>>)>;

/// Transform handles drawn around a floating piece or selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Rotate,
    SkewTop,
    SkewBottom,
    SkewLeft,
    SkewRight,
    /// A corner, by index into [`Frame::corners`]: scales with the opposite
    /// corner fixed.
    Scale(usize),
}

/// Local directions of the corners, in [`Frame::corners`] order.
const CORNER_SIGNS: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

/// Screen distance of the rotate handle beyond the top edge.
const ROTATE_HANDLE_OFFSET: f32 = 28.0;

pub struct Floating {
    /// The pixels as pasted or lifted.
    source: Image,
    /// `source` with the transparent color keyed out.
    keyed: Image,
    /// The color made transparent, if any.
    pub transparent: Option<[u8; 3]>,
    /// Pasted images can key out a color; lifted selections keep theirs.
    pub keyable: bool,
    /// Rotation, scale, skew and mirroring applied around the center.
    pub matrix: Matrix,
    /// What will be placed: `keyed` transformed by `matrix`.
    pub image: Image,
    /// Canvas position of the center.
    pub center: [f32; 2],
    /// GPU textures for `image`, split so no texture exceeds GPU limits.
    pub chunks: Vec<(IRect, TextureHandle)>,
    /// For a lifted selection: its layer and the pixels from before lifting,
    /// so cancelling can put them back and placing can be undone in one step.
    pub lifted: Option<(LayerId, TileUndo)>,
}

impl Floating {
    fn new(source: Image, x: i32, y: i32) -> Self {
        let center = [
            x as f32 + source.width as f32 / 2.0,
            y as f32 + source.height as f32 / 2.0,
        ];
        Self {
            keyed: source.clone(),
            image: source.clone(),
            source,
            transparent: None,
            keyable: false,
            matrix: IDENTITY,
            center,
            chunks: Vec::new(),
            lifted: None,
        }
    }

    pub fn pasted(image: Image, x: i32, y: i32, transparent: Option<[u8; 3]>) -> Self {
        let mut f = Self::new(image, x, y);
        f.keyable = true;
        f.set_transparent(transparent);
        f
    }

    pub fn lifted(image: Image, x: i32, y: i32, layer: LayerId, undo: TileUndo) -> Self {
        let mut f = Self::new(image, x, y);
        f.lifted = Some((layer, undo));
        f
    }

    pub fn set_transparent(&mut self, key: Option<[u8; 3]>) {
        if !self.keyable || key == self.transparent {
            return;
        }
        self.transparent = key;
        self.keyed = match key {
            Some(key) => canvas::color_to_transparent(&self.source, key),
            None => self.source.clone(),
        };
        self.rebuild();
    }

    pub fn set_matrix(&mut self, m: Matrix) {
        if canvas::mat_inverse(m).is_none() {
            return;
        }
        self.matrix = m;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.image = canvas::transform_image(&self.keyed, self.matrix);
        self.chunks.clear();
    }

    /// Top-left pixel of `image`.
    pub fn x(&self) -> i32 {
        (self.center[0] - self.image.width as f32 / 2.0).round() as i32
    }

    pub fn y(&self) -> i32 {
        (self.center[1] - self.image.height as f32 / 2.0).round() as i32
    }

    /// Bounding box of what will be placed.
    pub fn rect(&self) -> IRect {
        IRect {
            x0: self.x(),
            y0: self.y(),
            x1: self.x() + self.image.width as i32,
            y1: self.y() + self.image.height as i32,
        }
    }

    pub fn frame(&self) -> Frame {
        Frame {
            center: self.center,
            matrix: self.matrix,
            size: [self.source.width as f32, self.source.height as f32],
        }
    }
}

/// The (possibly rotated or skewed) box that transform handles sit on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub center: [f32; 2],
    pub matrix: Matrix,
    /// Untransformed width and height.
    pub size: [f32; 2],
}

impl Frame {
    /// The frame of an untransformed selection.
    pub fn of_rect(r: IRect) -> Self {
        Self {
            center: [(r.x0 + r.x1) as f32 / 2.0, (r.y0 + r.y1) as f32 / 2.0],
            matrix: IDENTITY,
            size: [r.width() as f32, r.height() as f32],
        }
    }

    /// Canvas position of a point given relative to the untransformed center.
    fn point(&self, local: [f32; 2]) -> [f32; 2] {
        let v = canvas::mat_apply(self.matrix, local);
        [self.center[0] + v[0], self.center[1] + v[1]]
    }

    /// Corners, clockwise from the top-left.
    pub fn corners(&self) -> [[f32; 2]; 4] {
        let [w, h] = [self.size[0] / 2.0, self.size[1] / 2.0];
        CORNER_SIGNS.map(|[sx, sy]| self.point([sx * w, sy * h]))
    }

    pub fn contains(&self, p: [f32; 2]) -> bool {
        let Some(inv) = canvas::mat_inverse(self.matrix) else {
            return false;
        };
        let l = canvas::mat_apply(inv, [p[0] - self.center[0], p[1] - self.center[1]]);
        l[0].abs() <= self.size[0] / 2.0 && l[1].abs() <= self.size[1] / 2.0
    }

    /// Where each handle is, in canvas coordinates. `scale` is screen points
    /// per canvas pixel, so the rotate handle keeps its distance on screen.
    pub fn handles(&self, scale: f32) -> [(Handle, [f32; 2]); 9] {
        let [w, h] = [self.size[0] / 2.0, self.size[1] / 2.0];
        let top = self.point([0.0, -h]);
        let (dx, dy) = (top[0] - self.center[0], top[1] - self.center[1]);
        let len = dx.hypot(dy);
        let dir = if len > 1e-3 {
            [dx / len, dy / len]
        } else {
            [0.0, -1.0]
        };
        let off = ROTATE_HANDLE_OFFSET / scale;
        let corners = self.corners();
        [
            (
                Handle::Rotate,
                [top[0] + dir[0] * off, top[1] + dir[1] * off],
            ),
            (Handle::SkewTop, top),
            (Handle::SkewBottom, self.point([0.0, h])),
            (Handle::SkewLeft, self.point([-w, 0.0])),
            (Handle::SkewRight, self.point([w, 0.0])),
            (Handle::Scale(0), corners[0]),
            (Handle::Scale(1), corners[1]),
            (Handle::Scale(2), corners[2]),
            (Handle::Scale(3), corners[3]),
        ]
    }

    /// The transform and center after dragging `handle` from `start` to
    /// `now` (canvas points), starting from this frame. With `snap`, rotation
    /// moves in 15° steps and scaling keeps the proportions.
    pub fn drag(
        &self,
        handle: Handle,
        start: [f32; 2],
        now: [f32; 2],
        snap: bool,
    ) -> (Matrix, [f32; 2]) {
        if let Handle::Scale(corner) = handle {
            return self.scale_corner(corner, now, snap);
        }
        (self.drag_matrix(handle, start, now, snap), self.center)
    }

    /// Scales so corner `corner` follows `now` while the opposite corner stays.
    fn scale_corner(&self, corner: usize, now: [f32; 2], keep_aspect: bool) -> (Matrix, [f32; 2]) {
        let m0 = self.matrix;
        let Some(inv) = canvas::mat_inverse(m0) else {
            return (m0, self.center);
        };
        let [sx, sy] = CORNER_SIGNS[corner];
        let [w, h] = [self.size[0].max(1.0) / 2.0, self.size[1].max(1.0) / 2.0];
        let opposite_local = [-sx * w, -sy * h];
        let fixed = self.point(opposite_local);
        // The dragged corner relative to the fixed one, in the box's own axes.
        let d = canvas::mat_apply(inv, [now[0] - fixed[0], now[1] - fixed[1]]);
        let span = [2.0 * sx * w, 2.0 * sy * h];
        let (mut kx, mut ky) = (d[0] / span[0], d[1] / span[1]);
        if keep_aspect {
            // Project onto the diagonal: one factor for both axes.
            let k = (d[0] * span[0] + d[1] * span[1]) / (span[0] * span[0] + span[1] * span[1]);
            (kx, ky) = (k, k);
        }
        // Keep at least a pixel; dragging past the fixed corner mirrors.
        let at_least = |k: f32, side: f32| {
            let min = 1.0 / side;
            if k.abs() < min { min.copysign(k) } else { k }
        };
        let (kx, ky) = (at_least(kx, 2.0 * w), at_least(ky, 2.0 * h));
        let m = canvas::mat_mul(m0, [kx, 0.0, 0.0, ky]);
        let v = canvas::mat_apply(m, opposite_local);
        (m, [fixed[0] - v[0], fixed[1] - v[1]])
    }

    fn drag_matrix(&self, handle: Handle, start: [f32; 2], now: [f32; 2], snap: bool) -> Matrix {
        let m0 = self.matrix;
        let c = self.center;
        let (dx, dy) = (now[0] - start[0], now[1] - start[1]);
        let [w, h] = [self.size[0].max(1.0) / 2.0, self.size[1].max(1.0) / 2.0];
        match handle {
            Handle::Rotate => {
                let a0 = (start[1] - c[1]).atan2(start[0] - c[0]);
                let a1 = (now[1] - c[1]).atan2(now[0] - c[0]);
                let mut delta = a1 - a0;
                if snap {
                    let step = 15f32.to_radians();
                    delta = (delta / step).round() * step;
                }
                canvas::mat_mul(canvas::rotation(delta), m0)
            }
            Handle::SkewTop | Handle::SkewBottom => {
                // Slide the edge along the box's own horizontal axis.
                let a = canvas::mat_apply(m0, [1.0, 0.0]);
                let along = (dx * a[0] + dy * a[1]) / (a[0] * a[0] + a[1] * a[1]).max(1e-6);
                let sign = if handle == Handle::SkewTop { -1.0 } else { 1.0 };
                canvas::mat_mul(m0, [1.0, sign * along / h, 0.0, 1.0])
            }
            Handle::SkewLeft | Handle::SkewRight => {
                let b = canvas::mat_apply(m0, [0.0, 1.0]);
                let along = (dx * b[0] + dy * b[1]) / (b[0] * b[0] + b[1] * b[1]).max(1e-6);
                let sign = if handle == Handle::SkewLeft {
                    -1.0
                } else {
                    1.0
                };
                canvas::mat_mul(m0, [1.0, 0.0, sign * along / w, 1.0])
            }
            Handle::Scale(_) => unreachable!("handled in drag"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Frame {
        Frame::of_rect(IRect {
            x0: 0,
            y0: 0,
            x1: 100,
            y1: 50,
        })
    }

    #[test]
    fn rotate_handle_turns_around_center() {
        let f = frame();
        let start = f.handles(1.0)[0].1;
        // Drag the handle (above the center) to the right of the center: 90°.
        let (m, _) = f.drag(Handle::Rotate, start, [150.0, 25.0], false);
        let r = canvas::rotation(std::f32::consts::FRAC_PI_2);
        assert!(m.iter().zip(r).all(|(a, b)| (a - b).abs() < 1e-4), "{m:?}");
        // Snapping rounds a ~5° turn to the nearest 15° step: none.
        let (m, _) = f.drag(Handle::Rotate, [50.0, -100.0], [60.0, -100.0], true);
        assert!(
            m.iter().zip(IDENTITY).all(|(a, b)| (a - b).abs() < 1e-4),
            "{m:?}"
        );
    }

    #[test]
    fn skew_handles_follow_the_pointer() {
        let f = frame();
        let top = f.handles(1.0)[1].1;
        let (m, _) = f.drag(Handle::SkewTop, top, [top[0] + 20.0, top[1]], false);
        let moved = Frame { matrix: m, ..f };
        let new_top = moved.handles(1.0)[1].1;
        assert!((new_top[0] - (top[0] + 20.0)).abs() < 1e-3, "{new_top:?}");
        // The bottom edge mirrors it, so the center stays put.
        let bottom = moved.handles(1.0)[2].1;
        assert!((bottom[0] - 30.0).abs() < 1e-3, "{bottom:?}");
        let right = f.handles(1.0)[4].1;
        let (m, _) = f.drag(Handle::SkewRight, right, [right[0], right[1] + 10.0], false);
        let moved = Frame { matrix: m, ..f };
        assert!((moved.handles(1.0)[4].1[1] - (right[1] + 10.0)).abs() < 1e-3);
    }

    #[test]
    fn corners_scale_with_the_opposite_corner_fixed() {
        let f = frame();
        // Drag the bottom-right corner from (100, 50) to (200, 75).
        let (m, center) = f.drag(Handle::Scale(2), [100.0, 50.0], [200.0, 75.0], false);
        let scaled = Frame {
            matrix: m,
            center,
            ..f
        };
        let c = scaled.corners();
        assert!((c[0][0]).abs() < 1e-3 && (c[0][1]).abs() < 1e-3, "{c:?}");
        assert!(
            (c[2][0] - 200.0).abs() < 1e-3 && (c[2][1] - 75.0).abs() < 1e-3,
            "{c:?}"
        );
        // Keeping the aspect ratio scales both axes by the same factor.
        let (m, _) = f.drag(Handle::Scale(2), [100.0, 50.0], [200.0, 60.0], true);
        assert!(
            (m[0] - m[3]).abs() < 1e-4 && m[1] == 0.0 && m[2] == 0.0,
            "{m:?}"
        );
        // The top-left corner works the same way, around the bottom-right.
        let (m, center) = f.drag(Handle::Scale(0), [0.0, 0.0], [50.0, 25.0], false);
        let c = Frame {
            matrix: m,
            center,
            ..f
        }
        .corners();
        assert!(
            (c[0][0] - 50.0).abs() < 1e-3 && (c[2][0] - 100.0).abs() < 1e-3,
            "{c:?}"
        );
    }

    #[test]
    fn contains_follows_the_transform() {
        let f = Frame {
            matrix: canvas::rotation(std::f32::consts::FRAC_PI_2),
            ..frame()
        };
        // Rotated a quarter turn the 100x50 box is 50 wide, 100 tall.
        assert!(f.contains([50.0, -20.0]));
        assert!(!f.contains([90.0, 25.0]));
    }
}
