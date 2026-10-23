//! Infinite, tiled raster canvas with layers.
//!
//! Pixels are stored per layer in square tiles that are allocated lazily the
//! first time something is painted into them, so the canvas has no fixed size.
//! Pixels are straight (non-premultiplied) RGBA8.

use std::collections::{HashMap, HashSet};

/// Side length of a tile in pixels.
pub const TILE: i32 = 256;
const TILE_PIXELS: usize = (TILE * TILE) as usize;

/// Tile coordinate (tile column, tile row).
pub type TileKey = (i32, i32);
pub type Rgba = [u8; 4];
pub type LayerId = u64;

pub fn tile_key(x: i32, y: i32) -> TileKey {
    (x.div_euclid(TILE), y.div_euclid(TILE))
}

fn local_index(x: i32, y: i32) -> usize {
    (y.rem_euclid(TILE) * TILE + x.rem_euclid(TILE)) as usize
}

/// Inclusive-exclusive integer rectangle in canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub fn width(&self) -> i32 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> i32 {
        self.y1 - self.y0
    }
    pub fn union(&self, o: &IRect) -> IRect {
        IRect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
}

pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    /// Layer opacity used when compositing, 0..=1.
    pub opacity: f32,
    pub tiles: HashMap<TileKey, Vec<Rgba>>,
    /// Color of every pixel outside the allocated tiles, i.e. of the infinite
    /// empty area around the drawing. Transparent unless a fill reached infinity.
    pub default: Rgba,
    /// Tiles changed since the renderer last uploaded them.
    pub dirty: HashSet<TileKey>,
    /// Set when `default` changes, for the renderer.
    pub default_changed: bool,
}

impl Layer {
    pub fn new(id: LayerId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            visible: true,
            opacity: 1.0,
            tiles: HashMap::new(),
            default: [0; 4],
            dirty: HashSet::new(),
            default_changed: false,
        }
    }

    pub fn pixel(&self, x: i32, y: i32) -> Rgba {
        self.tiles
            .get(&tile_key(x, y))
            .map_or(self.default, |t| t[local_index(x, y)])
    }

    /// Changes the color of the infinite empty area, returning the old one.
    pub fn set_default(&mut self, color: Rgba) -> Rgba {
        self.default_changed = true;
        std::mem::replace(&mut self.default, color)
    }

    /// Replaces a tile (or removes it with `None`), returning the previous one.
    pub fn swap_tile(&mut self, key: TileKey, tile: Option<Vec<Rgba>>) -> Option<Vec<Rgba>> {
        self.dirty.insert(key);
        match tile {
            Some(t) => self.tiles.insert(key, t),
            None => self.tiles.remove(&key),
        }
    }

    pub fn mark_all_dirty(&mut self) {
        self.dirty.extend(self.tiles.keys().copied());
    }
}

/// Composites `src` with coverage `alpha` (0..=1) over `dst` using straight alpha.
pub fn blend_over(dst: Rgba, src: Rgba, alpha: f32) -> Rgba {
    let sa = alpha.clamp(0.0, 1.0);
    let da = dst[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return [0; 4];
    }
    let mut out = [0u8; 4];
    for c in 0..3 {
        let v = (src[c] as f32 * sa + dst[c] as f32 * da * (1.0 - sa)) / oa;
        out[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    out[3] = (oa * 255.0).round() as u8;
    out
}

/// Removes `alpha` (0..=1) worth of opacity from `dst`.
pub fn erase(dst: Rgba, alpha: f32) -> Rgba {
    let a = (dst[3] as f32 * (1.0 - alpha.clamp(0.0, 1.0))).round() as u8;
    if a == 0 {
        [0; 4]
    } else {
        [dst[0], dst[1], dst[2], a]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaintMode {
    /// Paint `color` with the given opacity.
    Paint { color: Rgba, opacity: f32 },
    /// Erase with the given strength.
    Erase { strength: f32 },
}

/// A pixel edit in progress (one brush stroke, shape, or fill).
///
/// Coverage is tracked per pixel as the maximum over everything stamped so
/// far and always applied to the pixels as they were before the edit began,
/// so overlapping dabs of a semi-transparent stroke don't build up opacity.
pub struct PaintOp {
    pub layer_id: LayerId,
    mode: PaintMode,
    before: HashMap<TileKey, Option<Vec<Rgba>>>,
    coverage: HashMap<TileKey, Vec<f32>>,
}

impl PaintOp {
    pub fn new(layer_id: LayerId, mode: PaintMode) -> Self {
        Self {
            layer_id,
            mode,
            before: HashMap::new(),
            coverage: HashMap::new(),
        }
    }

    /// Applies `cov(x, y)` (sampled at pixel centers) to every pixel in `area`.
    pub fn apply(&mut self, layer: &mut Layer, area: IRect, cov: impl Fn(f32, f32) -> f32) {
        let color = match self.mode {
            PaintMode::Paint { color, .. } => color,
            PaintMode::Erase { .. } => [0; 4],
        };
        self.apply_src(layer, area, |x, y| {
            Some((cov(x as f32 + 0.5, y as f32 + 0.5), color))
        });
    }

    /// Like [`Self::apply`], but `src(x, y)` also chooses the color per pixel.
    fn apply_src(
        &mut self,
        layer: &mut Layer,
        area: IRect,
        src: impl Fn(i32, i32) -> Option<(f32, Rgba)>,
    ) {
        if area.width() <= 0 || area.height() <= 0 {
            return;
        }
        let (tx0, ty0) = tile_key(area.x0, area.y0);
        let (tx1, ty1) = tile_key(area.x1 - 1, area.y1 - 1);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let key = (tx, ty);
                let x0 = area.x0.max(tx * TILE);
                let x1 = area.x1.min((tx + 1) * TILE);
                let y0 = area.y0.max(ty * TILE);
                let y1 = area.y1.min((ty + 1) * TILE);
                self.apply_tile(layer, key, x0..x1, y0..y1, &src);
            }
        }
    }

    fn apply_tile(
        &mut self,
        layer: &mut Layer,
        key: TileKey,
        xs: std::ops::Range<i32>,
        ys: std::ops::Range<i32>,
        src: &impl Fn(i32, i32) -> Option<(f32, Rgba)>,
    ) {
        let erasing = matches!(self.mode, PaintMode::Erase { .. });
        // Erasing a transparent area never needs to allocate a tile.
        if erasing
            && layer.default[3] == 0
            && !layer.tiles.contains_key(&key)
            && !self.before.contains_key(&key)
        {
            return;
        }
        let default = layer.default;
        let mut touched = false;
        for y in ys {
            for x in xs.clone() {
                let Some((c, color)) = src(x, y) else {
                    continue;
                };
                if c <= 0.0 {
                    continue;
                }
                let i = local_index(x, y);
                let covs = self
                    .coverage
                    .entry(key)
                    .or_insert_with(|| vec![0.0; TILE_PIXELS]);
                if c <= covs[i] {
                    continue;
                }
                covs[i] = c;
                let before = self
                    .before
                    .entry(key)
                    .or_insert_with(|| layer.tiles.get(&key).cloned());
                let old = before.as_ref().map_or(default, |t| t[i]);
                let new = match self.mode {
                    PaintMode::Paint { opacity, .. } => blend_over(old, color, c * opacity),
                    PaintMode::Erase { strength } => erase(old, c * strength),
                };
                layer
                    .tiles
                    .entry(key)
                    .or_insert_with(|| vec![default; TILE_PIXELS])[i] = new;
                touched = true;
            }
        }
        if touched {
            layer.dirty.insert(key);
        }
    }

    /// Draws a pixel-aligned rectangle, either filled or outlined with a line
    /// of `width` centered on its edges.
    pub fn rectangle(&mut self, layer: &mut Layer, r: IRect, width: f32, filled: bool) {
        let half = (width / 2.0).max(0.5);
        let grow = if filled { 0.0 } else { half };
        let area = bbox(
            r.x0 as f32 - grow,
            r.y0 as f32 - grow,
            r.x1 as f32 + grow,
            r.y1 as f32 + grow,
        );
        let (cx, cy) = ((r.x0 + r.x1) as f32 / 2.0, (r.y0 + r.y1) as f32 / 2.0);
        let (hw, hh) = (r.width() as f32 / 2.0, r.height() as f32 / 2.0);
        self.apply(layer, area, |x, y| {
            // Chebyshev distance to the rectangle's edge keeps corners square.
            let d = ((x - cx).abs() - hw).max((y - cy).abs() - hh);
            if filled {
                coverage(-d)
            } else {
                coverage(half - d.abs())
            }
        });
    }

    /// Draws a convex polygon, filled or outlined with a line of `width`
    /// centered on its edges (with sharp corners).
    pub fn polygon(&mut self, layer: &mut Layer, pts: &[[f32; 2]], width: f32, filled: bool) {
        if pts.len() < 3 {
            return;
        }
        let half = (width / 2.0).max(0.5);
        // Clockwise or not, make every edge normal point outwards.
        let area2: f32 = (0..pts.len())
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum();
        let sign = if area2 > 0.0 { 1.0 } else { -1.0 };
        let edges: Vec<([f32; 2], [f32; 2])> = (0..pts.len())
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let len = dx.hypot(dy).max(1e-6);
                (a, [sign * dy / len, -sign * dx / len])
            })
            .collect();
        let grow = if filled { 1.0 } else { half * 3.0 };
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pts {
            (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
        }
        let area = bbox(x0 - grow, y0 - grow, x1 + grow, y1 + grow);
        self.apply(layer, area, |x, y| {
            // Signed distance to the nearest edge's line (negative inside);
            // outside a corner this gives a sharp, mitred tip.
            let d = edges
                .iter()
                .map(|(a, n)| (x - a[0]) * n[0] + (y - a[1]) * n[1])
                .fold(f32::MIN, f32::max);
            if filled {
                coverage(-d)
            } else {
                coverage(half - d.abs())
            }
        });
    }

    /// Draws connected line segments of `width` with round ends and joins.
    pub fn polyline(&mut self, layer: &mut Layer, pts: &[[f32; 2]], width: f32) {
        match pts {
            [] => {}
            [p] => self.stroke_segment(layer, *p, *p, width / 2.0),
            _ => {
                for w in pts.windows(2) {
                    self.stroke_segment(layer, w[0], w[1], width / 2.0);
                }
            }
        }
    }

    /// Draws the lines of a `cols`×`rows` grid of `cell`-sized cells whose
    /// top-left corner is `origin`.
    pub fn grid(
        &mut self,
        layer: &mut Layer,
        origin: [i32; 2],
        cell: [i32; 2],
        cols: i32,
        rows: i32,
        width: f32,
    ) {
        if cols <= 0 || rows <= 0 || cell[0] <= 0 || cell[1] <= 0 {
            return;
        }
        let half = (width / 2.0).max(0.5);
        let (x0, y0) = (origin[0] as f32, origin[1] as f32);
        let (cw, ch) = (cell[0] as f32, cell[1] as f32);
        let (x1, y1) = (x0 + cols as f32 * cw, y0 + rows as f32 * ch);
        let area = bbox(x0 - half, y0 - half, x1 + half, y1 + half);
        self.apply(layer, area, |x, y| {
            // Distance to the nearest vertical line, limited to the grid's height.
            let k = ((x - x0) / cw).round().clamp(0.0, cols as f32);
            let dv = (x - (x0 + k * cw)).abs().max((y0 - y).max(y - y1).max(0.0));
            let k = ((y - y0) / ch).round().clamp(0.0, rows as f32);
            let dh = (y - (y0 + k * ch)).abs().max((x0 - x).max(x - x1).max(0.0));
            coverage(half - dv.min(dh))
        });
    }

    /// Allocates a tile filled with the layer's current default color, so it
    /// keeps its look when the default changes.
    pub fn materialize(&mut self, layer: &mut Layer, key: TileKey) {
        if layer.tiles.contains_key(&key) {
            return;
        }
        self.before.entry(key).or_insert(None);
        layer.tiles.insert(key, vec![layer.default; TILE_PIXELS]);
        layer.dirty.insert(key);
    }

    /// Composites `image` onto the layer with its top-left corner at (`x`, `y`).
    pub fn blit(&mut self, layer: &mut Layer, image: &Image, x: i32, y: i32) {
        let area = IRect {
            x0: x,
            y0: y,
            x1: x + image.width as i32,
            y1: y + image.height as i32,
        };
        self.apply_src(layer, area, |px, py| {
            let p = image.pixels[(py - y) as usize * image.width + (px - x) as usize];
            (p[3] > 0).then(|| (p[3] as f32 / 255.0, p))
        });
    }

    /// Stamps a round brush along the segment `a`–`b`.
    pub fn stroke_segment(&mut self, layer: &mut Layer, a: [f32; 2], b: [f32; 2], radius: f32) {
        let r = radius.max(0.5);
        let area = bbox(
            a[0].min(b[0]) - r,
            a[1].min(b[1]) - r,
            a[0].max(b[0]) + r,
            a[1].max(b[1]) + r,
        );
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        self.apply(layer, area, |x, y| {
            let t = if len2 > 0.0 {
                (((x - a[0]) * dx + (y - a[1]) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (px, py) = (a[0] + t * dx - x, a[1] + t * dy - y);
            coverage(r - (px * px + py * py).sqrt())
        });
    }

    /// Draws the ellipse inscribed in the pixel-aligned rectangle `r`, either
    /// filled or outlined with a line of `width` centered on its edge.
    pub fn ellipse(&mut self, layer: &mut Layer, r: IRect, width: f32, filled: bool) {
        let half = (width / 2.0).max(0.5);
        let grow = if filled { 0.0 } else { half };
        let area = bbox(
            r.x0 as f32 - grow,
            r.y0 as f32 - grow,
            r.x1 as f32 + grow,
            r.y1 as f32 + grow,
        );
        let (cx, cy) = ((r.x0 + r.x1) as f32 / 2.0, (r.y0 + r.y1) as f32 / 2.0);
        let (a, b) = (r.width() as f32 / 2.0, r.height() as f32 / 2.0);
        self.apply(layer, area, |x, y| {
            let d = ellipse_distance(x - cx, y - cy, a, b);
            if filled {
                coverage(-d)
            } else {
                coverage(half - d.abs())
            }
        });
    }

    /// Returns the undo record, or `None` if nothing changed.
    pub fn finish(self) -> Option<Vec<(TileKey, Option<Vec<Rgba>>)>> {
        if self.before.is_empty() {
            None
        } else {
            Some(self.before.into_iter().collect())
        }
    }
}

/// Signed distance from a point (relative to the center) to an ellipse with
/// semi-axes `a` and `b`; negative inside. Uses a few Newton-like steps to
/// find the closest point on the curve, so thick outlines stay round.
fn ellipse_distance(px: f32, py: f32, a: f32, b: f32) -> f32 {
    let (px, py) = (px.abs(), py.abs());
    let (a, b) = (a.max(1e-3), b.max(1e-3));
    if px < 1e-4 && py < 1e-4 {
        // The center: the iteration below has no direction to work with.
        return -a.min(b);
    }
    let (mut tx, mut ty) = (
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
    );
    for _ in 0..4 {
        let (x, y) = (a * tx, b * ty);
        let ex = (a * a - b * b) * tx.powi(3) / a;
        let ey = (b * b - a * a) * ty.powi(3) / b;
        let (rx, ry) = (x - ex, y - ey);
        let (qx, qy) = (px - ex, py - ey);
        let r = rx.hypot(ry);
        let q = qx.hypot(qy).max(1e-6);
        tx = ((qx * r / q + ex) / a).clamp(0.0, 1.0);
        ty = ((qy * r / q + ey) / b).clamp(0.0, 1.0);
        let t = tx.hypot(ty).max(1e-6);
        tx /= t;
        ty /= t;
    }
    let d = (px - a * tx).hypot(py - b * ty);
    if (px / a).powi(2) + (py / b).powi(2) < 1.0 {
        -d
    } else {
        d
    }
}

/// Anti-aliased coverage from a signed distance to the shape edge (positive inside).
fn coverage(signed_dist: f32) -> f32 {
    (signed_dist + 0.5).clamp(0.0, 1.0)
}

fn bbox(x0: f32, y0: f32, x1: f32, y1: f32) -> IRect {
    IRect {
        x0: x0.floor() as i32 - 1,
        y0: y0.floor() as i32 - 1,
        x1: x1.ceil() as i32 + 1,
        y1: y1.ceil() as i32 + 1,
    }
}

/// A standalone straight-RGBA image (clipboard contents, pasted pieces).
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgba>,
}

/// Returns a copy of `image` with every pixel of color `key` made transparent.
pub fn color_to_transparent(image: &Image, key: [u8; 3]) -> Image {
    let pixels = image
        .pixels
        .iter()
        .map(|&p| if p[..3] == key { [0; 4] } else { p })
        .collect();
    Image {
        width: image.width,
        height: image.height,
        pixels,
    }
}

/// Moves `end` so the rectangle from `start` to it becomes a square
/// (the larger side wins), keeping the drag direction.
pub fn square_end(start: [i32; 2], end: [i32; 2]) -> [i32; 2] {
    let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
    let d = dx.abs().max(dy.abs());
    let sign = |v: i32| if v < 0 { -1 } else { 1 };
    [start[0] + sign(dx) * d, start[1] + sign(dy) * d]
}

/// Normalized rectangle spanned by two corners.
pub fn rect_from_corners(a: [i32; 2], b: [i32; 2]) -> IRect {
    IRect {
        x0: a[0].min(b[0]),
        y0: a[1].min(b[1]),
        x1: a[0].max(b[0]),
        y1: a[1].max(b[1]),
    }
}

/// The stroke path of an arrow from `tail` to `tip` drawn `width` wide:
/// the shaft, then each barb of the open head and back to the tip. The head
/// grows with the width but stays under half the arrow's length.
pub fn arrow_path(tail: [f32; 2], tip: [f32; 2], width: f32) -> Vec<[f32; 2]> {
    let (dx, dy) = (tip[0] - tail[0], tip[1] - tail[1]);
    let len = dx.hypot(dy);
    if len < 1e-3 {
        return vec![tail, tip];
    }
    let head = (12.0 + 3.0 * width).min(len * 0.45);
    let (ux, uy) = (dx / len, dy / len);
    let (sin, cos) = 30f32.to_radians().sin_cos();
    let barb = |side: f32| {
        // The shaft direction turned back by 180° ± 30°.
        let (bx, by) = (-(ux * cos - side * uy * sin), -(side * ux * sin + uy * cos));
        [tip[0] + bx * head, tip[1] + by * head]
    };
    vec![tail, tip, barb(1.0), tip, barb(-1.0)]
}

/// What the Shape tool draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Triangle,
    /// A regular polygon with a chosen number of sides.
    NGon,
    Line,
    /// A line with an open (V-shaped) head at its end.
    Arrow,
    Polyline,
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 7] = [
        Self::Rectangle,
        Self::Ellipse,
        Self::Triangle,
        Self::NGon,
        Self::Line,
        Self::Arrow,
        Self::Polyline,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Triangle => "Triangle",
            Self::NGon => "N-gon",
            Self::Line => "Line",
            Self::Arrow => "Arrow",
            Self::Polyline => "Polyline",
        }
    }

    /// Whether the shape is drawn by dragging out a bounding box.
    pub fn is_boxed(self) -> bool {
        matches!(self, Self::Rectangle | Self::Ellipse | Self::Triangle)
    }

    /// Whether the shape can be filled instead of outlined.
    pub fn can_fill(self) -> bool {
        self.is_boxed() || self == Self::NGon
    }

    /// Height / width when snapped to a regular shape (square, circle,
    /// equilateral triangle).
    pub fn regular_ratio(self) -> f32 {
        match self {
            Self::Triangle => 3f32.sqrt() / 2.0,
            _ => 1.0,
        }
    }
}

/// Corners of the triangle filling `r`: apex at the top middle, base along the bottom.
pub fn triangle_points(r: IRect) -> [[f32; 2]; 3] {
    let mid = (r.x0 + r.x1) as f32 / 2.0;
    [
        [mid, r.y0 as f32],
        [r.x1 as f32, r.y1 as f32],
        [r.x0 as f32, r.y1 as f32],
    ]
}

/// The vertex of a regular `sides`-gon that's placed together with vertex 0.
pub fn ngon_far_vertex(sides: u32) -> u32 {
    sides.max(3) / 2
}

/// Corners of the regular `sides`-gon whose vertex 0 is at `v0` and whose
/// vertex `sides / 2` (rounded down) is at `vk`. For odd counts the polygon
/// lies to the left of the direction from `v0` to `vk` (below it when that
/// points right), so a left-to-right drag gives an upright shape.
pub fn ngon_from_vertices(v0: [f32; 2], vk: [f32; 2], sides: u32) -> Vec<[f32; 2]> {
    let n = sides.max(3);
    let k = ngon_far_vertex(n);
    let (dx, dy) = (vk[0] - v0[0], vk[1] - v0[1]);
    let chord = dx.hypot(dy);
    if chord < 1e-3 {
        return Vec::new();
    }
    // The chord spans k of the n equal steps around the circumscribed circle.
    let half_angle = std::f32::consts::PI * k as f32 / n as f32;
    let radius = chord / 2.0 / half_angle.sin();
    let apothem = radius * half_angle.cos();
    let (ux, uy) = (dx / chord, dy / chord);
    // Perpendicular pointing "down" for a rightward chord (screen y grows down).
    let (px, py) = (-uy, ux);
    let center = [
        (v0[0] + vk[0]) / 2.0 + px * apothem,
        (v0[1] + vk[1]) / 2.0 + py * apothem,
    ];
    let start = (v0[1] - center[1]).atan2(v0[0] - center[0]);
    // Walk round the circle in the direction that lands vertex k on `vk`.
    let step = std::f32::consts::TAU / n as f32;
    let at = |dir: f32, i: u32| {
        let a = start + dir * step * i as f32;
        [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
    };
    let miss = |dir: f32| {
        let p = at(dir, k);
        (p[0] - vk[0]).hypot(p[1] - vk[1])
    };
    let dir = if miss(1.0) <= miss(-1.0) { 1.0 } else { -1.0 };
    (0..n).map(|i| at(dir, i)).collect()
}

/// Rotates `end` around `start` to the nearest multiple of `degrees`,
/// keeping its distance.
pub fn snap_rotation(start: [f32; 2], end: [f32; 2], degrees: f32) -> [f32; 2] {
    let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
    let step = degrees.to_radians();
    let angle = (dy.atan2(dx) / step).round() * step;
    let len = dx.hypot(dy);
    [start[0] + len * angle.cos(), start[1] + len * angle.sin()]
}

/// Moves `end` so the line from `start` points in a multiple of 45°.
pub fn snap_angle(start: [f32; 2], end: [f32; 2]) -> [f32; 2] {
    let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
    let step = std::f32::consts::FRAC_PI_4;
    let angle = (dy.atan2(dx) / step).round() * step;
    // Keep the drag's extent along the chosen direction.
    let len = if angle.cos().abs() > 0.9 {
        dx.abs()
    } else if angle.sin().abs() > 0.9 {
        dy.abs()
    } else {
        dx.abs().max(dy.abs()) * std::f32::consts::SQRT_2
    };
    [start[0] + len * angle.cos(), start[1] + len * angle.sin()]
}

/// Shape of the cells drawn by the grid tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridKind {
    Rectangle,
    /// Rows of alternating up/down triangles; the lines run straight through.
    Triangle,
    /// Flat-top hexagons; every other column is shifted down half a cell.
    Hexagon,
}

/// A line segment between two canvas points.
pub type Segment = [[f32; 2]; 2];

impl GridKind {
    pub const ALL: [GridKind; 3] = [Self::Rectangle, Self::Triangle, Self::Hexagon];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangles",
            Self::Triangle => "Triangles",
            Self::Hexagon => "Hexagons",
        }
    }

    /// Height / width of a regular cell (square, equilateral triangle,
    /// regular hexagon), used when snapping the cell.
    pub fn regular_ratio(self) -> f32 {
        match self {
            Self::Rectangle => 1.0,
            Self::Triangle | Self::Hexagon => 3f32.sqrt() / 2.0,
        }
    }

    /// How many columns and rows of `cell`-sized cells fit in `area` (w, h).
    pub fn fit(self, cell: [i32; 2], area: [i32; 2]) -> (i32, i32) {
        let (w, h) = (cell[0].max(1) as f32, cell[1].max(1) as f32);
        let (aw, ah) = (area[0] as f32, area[1] as f32);
        match self {
            Self::Rectangle => (area[0] / cell[0].max(1), area[1] / cell[1].max(1)),
            Self::Triangle => {
                // Neighboring triangles overlap by half a cell.
                let cols = if aw >= w {
                    (2.0 * aw / w).floor() as i32 - 1
                } else {
                    0
                };
                (cols, (ah / h).floor() as i32)
            }
            Self::Hexagon => {
                let cols = if aw >= w {
                    ((aw - w) / (0.75 * w)).floor() as i32 + 1
                } else {
                    0
                };
                let shift = if cols > 1 { h / 2.0 } else { 0.0 };
                (cols, ((ah - shift) / h).floor().max(0.0) as i32)
            }
        }
    }

    /// Width and height covered by a grid of this many cells.
    pub fn extent(self, cell: [i32; 2], cols: i32, rows: i32) -> [f32; 2] {
        let (w, h) = (cell[0] as f32, cell[1] as f32);
        let (c, r) = (cols as f32, rows as f32);
        match self {
            Self::Rectangle => [c * w, r * h],
            Self::Triangle => [(c + 1.0) * w / 2.0, r * h],
            Self::Hexagon => [
                w + (c - 1.0) * 0.75 * w,
                r * h + if cols > 1 { h / 2.0 } else { 0.0 },
            ],
        }
    }

    /// Lays out a grid from `origin` towards `end` (it grows in the direction
    /// dragged), fitting as many whole cells as possible. Returns the top-left
    /// corner and the column and row counts.
    pub fn layout(self, origin: [i32; 2], cell: [i32; 2], end: [i32; 2]) -> ([f32; 2], i32, i32) {
        let area = [(end[0] - origin[0]).abs(), (end[1] - origin[1]).abs()];
        let (cols, rows) = self.fit(cell, area);
        let [ew, eh] = self.extent(cell, cols, rows);
        let x0 = origin[0] as f32 - if end[0] < origin[0] { ew } else { 0.0 };
        let y0 = origin[1] as f32 - if end[1] < origin[1] { eh } else { 0.0 };
        ([x0, y0], cols, rows)
    }

    /// The line segments of a grid (edges shared by cells may repeat).
    pub fn segments(
        self,
        top_left: [f32; 2],
        cell: [i32; 2],
        cols: i32,
        rows: i32,
    ) -> Vec<Segment> {
        let (x0, y0) = (top_left[0], top_left[1]);
        let (w, h) = (cell[0] as f32, cell[1] as f32);
        let mut out = Vec::new();
        if cols <= 0 || rows <= 0 {
            return out;
        }
        let mut polygon = |pts: &[[f32; 2]]| {
            for i in 0..pts.len() {
                out.push([pts[i], pts[(i + 1) % pts.len()]]);
            }
        };
        match self {
            Self::Rectangle => {
                let [ew, eh] = self.extent(cell, cols, rows);
                for c in 0..=cols {
                    let x = x0 + c as f32 * w;
                    out.push([[x, y0], [x, y0 + eh]]);
                }
                for r in 0..=rows {
                    let y = y0 + r as f32 * h;
                    out.push([[x0, y], [x0 + ew, y]]);
                }
            }
            Self::Triangle => {
                for r in 0..rows {
                    for i in 0..cols {
                        let (x, y) = (x0 + i as f32 * w / 2.0, y0 + r as f32 * h);
                        if (i + r) % 2 == 0 {
                            polygon(&[[x, y + h], [x + w / 2.0, y], [x + w, y + h]]);
                        } else {
                            polygon(&[[x, y], [x + w, y], [x + w / 2.0, y + h]]);
                        }
                    }
                }
            }
            Self::Hexagon => {
                for c in 0..cols {
                    for r in 0..rows {
                        let x = x0 + c as f32 * 0.75 * w;
                        let y = y0 + r as f32 * h + if c % 2 == 1 { h / 2.0 } else { 0.0 };
                        polygon(&[
                            [x, y + h / 2.0],
                            [x + w / 4.0, y],
                            [x + 0.75 * w, y],
                            [x + w, y + h / 2.0],
                            [x + 0.75 * w, y + h],
                            [x + w / 4.0, y + h],
                        ]);
                    }
                }
            }
        }
        out
    }
}

/// A 2×2 linear transform `[a, b, c, d]`: x' = a·x + b·y, y' = c·x + d·y.
pub type Matrix = [f32; 4];

pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0];

/// `m · n` (apply `n` first, then `m`).
pub fn mat_mul(m: Matrix, n: Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
    ]
}

pub fn mat_apply(m: Matrix, v: [f32; 2]) -> [f32; 2] {
    [m[0] * v[0] + m[1] * v[1], m[2] * v[0] + m[3] * v[1]]
}

pub fn mat_inverse(m: Matrix) -> Option<Matrix> {
    let det = m[0] * m[3] - m[1] * m[2];
    (det.abs() > 1e-6).then(|| [m[3] / det, -m[1] / det, -m[2] / det, m[0] / det])
}

pub fn rotation(angle: f32) -> Matrix {
    let (s, c) = angle.sin_cos();
    [c, -s, s, c]
}

/// Whether `m` only flips and turns by multiples of 90° (pixels map 1:1).
fn is_pixel_exact(m: Matrix) -> bool {
    m.iter()
        .all(|v| v.abs() < 1e-4 || (v.abs() - 1.0).abs() < 1e-4)
}

/// Transforms `src` by `m` around its center. The result is the bounding box
/// of the transformed image, sharing its center. Flips and 90° turns copy
/// pixels exactly; anything else is resampled bilinearly.
pub fn transform_image(src: &Image, m: Matrix) -> Image {
    let (w, h) = (src.width as f32, src.height as f32);
    let Some(inv) = mat_inverse(m) else {
        return Image {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };
    };
    let corners =
        [[-w, -h], [w, -h], [w, h], [-w, h]].map(|c| mat_apply(m, [c[0] / 2.0, c[1] / 2.0]));
    let max_x = corners.iter().map(|c| c[0].abs()).fold(0.0, f32::max);
    let max_y = corners.iter().map(|c| c[1].abs()).fold(0.0, f32::max);
    let (ow, oh) = (
        (2.0 * max_x - 1e-3).ceil() as usize,
        (2.0 * max_y - 1e-3).ceil() as usize,
    );
    let exact = is_pixel_exact(m);
    let mut pixels = vec![[0u8; 4]; ow * oh];
    let get = |x: i32, y: i32| -> [f32; 4] {
        if x < 0 || y < 0 || x >= src.width as i32 || y >= src.height as i32 {
            return [0.0; 4];
        }
        let p = src.pixels[y as usize * src.width + x as usize];
        let a = p[3] as f32 / 255.0;
        [p[0] as f32 * a, p[1] as f32 * a, p[2] as f32 * a, a]
    };
    for oy in 0..oh {
        for ox in 0..ow {
            // Pixel center relative to the image center, back into the source.
            let q = [
                ox as f32 + 0.5 - ow as f32 / 2.0,
                oy as f32 + 0.5 - oh as f32 / 2.0,
            ];
            let l = mat_apply(inv, q);
            let (sx, sy) = (l[0] + w / 2.0 - 0.5, l[1] + h / 2.0 - 0.5);
            let px = if exact {
                get(sx.round() as i32, sy.round() as i32)
            } else {
                let (x0, y0) = (sx.floor(), sy.floor());
                let (fx, fy) = (sx - x0, sy - y0);
                let (x0, y0) = (x0 as i32, y0 as i32);
                let mut acc = [0.0f32; 4];
                for (dx, dy, wgt) in [
                    (0, 0, (1.0 - fx) * (1.0 - fy)),
                    (1, 0, fx * (1.0 - fy)),
                    (0, 1, (1.0 - fx) * fy),
                    (1, 1, fx * fy),
                ] {
                    let s = get(x0 + dx, y0 + dy);
                    for c in 0..4 {
                        acc[c] += s[c] * wgt;
                    }
                }
                acc
            };
            if px[3] > 0.001 {
                pixels[oy * ow + ox] = [
                    (px[0] / px[3]).round().clamp(0.0, 255.0) as u8,
                    (px[1] / px[3]).round().clamp(0.0, 255.0) as u8,
                    (px[2] / px[3]).round().clamp(0.0, 255.0) as u8,
                    (px[3] * 255.0).round() as u8,
                ];
            }
        }
    }
    Image {
        width: ow,
        height: oh,
        pixels,
    }
}

/// Copies the pixels of `layer` inside `area`.
pub fn copy_region(layer: &Layer, area: IRect) -> Image {
    let (w, h) = (area.width().max(0) as usize, area.height().max(0) as usize);
    let mut pixels = Vec::with_capacity(w * h);
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            pixels.push(layer.pixel(x, y));
        }
    }
    Image {
        width: w,
        height: h,
        pixels,
    }
}

/// Copies `area` as it appears with all visible layers composited.
pub fn copy_region_merged(layers: &[Layer], area: IRect) -> Image {
    let mut image = copy_region(&Layer::new(0, ""), area);
    for layer in layers.iter().filter(|l| l.visible) {
        let src = copy_region(layer, area);
        for (dst, s) in image.pixels.iter_mut().zip(&src.pixels) {
            if s[3] > 0 {
                *dst = blend_over(*dst, *s, s[3] as f32 / 255.0 * layer.opacity);
            }
        }
    }
    image
}

/// Tight bounds of the pixels in a tile that differ from the layer's
/// `background` (its infinite default color), in canvas pixels.
pub fn tile_content_bounds(key: TileKey, tile: &[Rgba], background: Rgba) -> Option<IRect> {
    let mut b: Option<IRect> = None;
    for (i, &p) in tile.iter().enumerate() {
        if p == background || (p[3] == 0 && background[3] == 0) {
            continue;
        }
        let x = key.0 * TILE + i as i32 % TILE;
        let y = key.1 * TILE + i as i32 / TILE;
        let r = IRect {
            x0: x,
            y0: y,
            x1: x + 1,
            y1: y + 1,
        };
        b = Some(b.map_or(r, |b| b.union(&r)));
    }
    b
}

/// Renders a `size`×`size` preview of `region` of the layer, fitted and
/// centered with its aspect ratio preserved. Each output pixel takes the
/// average color but the strongest alpha of a few samples, so thin strokes
/// stay visible when scaled down.
pub fn thumbnail(layer: &Layer, region: IRect, size: usize) -> Vec<Rgba> {
    let mut out = vec![[0u8; 4]; size * size];
    let (rw, rh) = (region.width().max(1) as f32, region.height().max(1) as f32);
    let scale = rw.max(rh) / size as f32;
    let off_x = (size as f32 - rw / scale) / 2.0;
    let off_y = (size as f32 - rh / scale) / 2.0;
    const N: usize = 3;
    for oy in 0..size {
        for ox in 0..size {
            let mut acc = [0f32; 4];
            let mut max_a = 0u8;
            for sy in 0..N {
                for sx in 0..N {
                    let fx = (ox as f32 - off_x + (sx as f32 + 0.5) / N as f32) * scale;
                    let fy = (oy as f32 - off_y + (sy as f32 + 0.5) / N as f32) * scale;
                    if fx < 0.0 || fy < 0.0 || fx >= rw || fy >= rh {
                        continue;
                    }
                    let p = layer.pixel(region.x0 + fx as i32, region.y0 + fy as i32);
                    let a = p[3] as f32;
                    max_a = max_a.max(p[3]);
                    acc[0] += p[0] as f32 * a;
                    acc[1] += p[1] as f32 * a;
                    acc[2] += p[2] as f32 * a;
                    acc[3] += a;
                }
            }
            if acc[3] > 0.0 {
                out[oy * size + ox] = [
                    (acc[0] / acc[3]) as u8,
                    (acc[1] / acc[3]) as u8,
                    (acc[2] / acc[3]) as u8,
                    max_a,
                ];
            }
        }
    }
    out
}

fn color_matches(a: Rgba, b: Rgba, tolerance: u8) -> bool {
    // All fully transparent pixels count as the same color.
    let norm = |c: Rgba| if c[3] == 0 { [0; 4] } else { c };
    let (a, b) = (norm(a), norm(b));
    (0..4).all(|i| a[i].abs_diff(b[i]) <= tolerance)
}

/// What a flood fill changed.
pub struct FillResult {
    /// Undo record for the changed tiles.
    pub tiles: Vec<(TileKey, Option<Vec<Rgba>>)>,
    /// The layer's previous default color, if the fill reached infinity and
    /// changed it.
    pub old_default: Option<Rgba>,
}

/// The pixels reached by a flood fill. Unallocated tiles are uniform, so they
/// are tracked whole instead of per pixel.
struct Region {
    /// Allocated tiles at the time of the fill.
    existing: HashSet<TileKey>,
    /// Range of allocated tiles (min x, min y, max x, max y), if any.
    tile_bounds: Option<(i32, i32, i32, i32)>,
    /// Reached pixels inside allocated tiles.
    pixels: HashMap<TileKey, Vec<bool>>,
    /// Reached unallocated tiles inside `tile_bounds`.
    tiles: HashSet<TileKey>,
    /// Whether the fill escaped past all allocated tiles, i.e. is unbounded.
    infinite: bool,
}

impl Region {
    fn within_bounds(&self, k: TileKey) -> bool {
        self.tile_bounds
            .is_some_and(|(x0, y0, x1, y1)| k.0 >= x0 && k.0 <= x1 && k.1 >= y0 && k.1 <= y1)
    }

    fn contains(&self, x: i32, y: i32) -> bool {
        let k = tile_key(x, y);
        if self.existing.contains(&k) {
            self.pixels.get(&k).is_some_and(|m| m[local_index(x, y)])
        } else if self.within_bounds(k) {
            self.tiles.contains(&k)
        } else {
            self.infinite
        }
    }

    /// Whether anything in tile `k` was reached.
    fn touches_tile(&self, k: TileKey) -> bool {
        if self.existing.contains(&k) {
            self.pixels.contains_key(&k)
        } else if self.within_bounds(k) {
            self.tiles.contains(&k)
        } else {
            self.infinite
        }
    }
}

/// The pixels of tile `to` that border the neighboring tile `from`.
fn edge_pixels(from: TileKey, to: TileKey) -> impl Iterator<Item = (i32, i32)> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    (0..TILE).map(move |i| match (dx, dy) {
        (1, _) => (to.0 * TILE, to.1 * TILE + i),
        (-1, _) => (to.0 * TILE + TILE - 1, to.1 * TILE + i),
        (_, 1) => (to.0 * TILE + i, to.1 * TILE),
        _ => (to.0 * TILE + i, to.1 * TILE + TILE - 1),
    })
}

fn find_region(layer: &Layer, x: i32, y: i32, matches: impl Fn(Rgba) -> bool) -> Region {
    let existing: HashSet<TileKey> = layer.tiles.keys().copied().collect();
    let tile_bounds = existing.iter().fold(None, |acc, &(tx, ty)| {
        Some(match acc {
            None => (tx, ty, tx, ty),
            Some((a, b, c, d)) => (tx.min(a), ty.min(b), tx.max(c), ty.max(d)),
        })
    });
    let mut region = Region {
        existing,
        tile_bounds,
        pixels: HashMap::new(),
        tiles: HashSet::new(),
        infinite: false,
    };

    enum Node {
        Pixel(i32, i32),
        Tile(TileKey),
        Infinity,
    }
    let node_for_tile = |region: &Region, k: TileKey| {
        if region.within_bounds(k) {
            Node::Tile(k)
        } else {
            Node::Infinity
        }
    };
    let node_at = |region: &Region, x: i32, y: i32| {
        let k = tile_key(x, y);
        if region.existing.contains(&k) {
            Node::Pixel(x, y)
        } else {
            node_for_tile(region, k)
        }
    };

    let mut stack = vec![node_at(&region, x, y)];
    while let Some(node) = stack.pop() {
        match node {
            Node::Pixel(x, y) => {
                let k = tile_key(x, y);
                let i = local_index(x, y);
                if region.pixels.get(&k).is_some_and(|m| m[i]) || !matches(layer.pixel(x, y)) {
                    continue;
                }
                region
                    .pixels
                    .entry(k)
                    .or_insert_with(|| vec![false; TILE_PIXELS])[i] = true;
                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                    stack.push(node_at(&region, nx, ny));
                }
            }
            Node::Tile(k) => {
                if region.tiles.contains(&k) || !matches(layer.default) {
                    continue;
                }
                region.tiles.insert(k);
                for nk in [
                    (k.0 - 1, k.1),
                    (k.0 + 1, k.1),
                    (k.0, k.1 - 1),
                    (k.0, k.1 + 1),
                ] {
                    if region.existing.contains(&nk) {
                        stack.extend(edge_pixels(k, nk).map(|(x, y)| Node::Pixel(x, y)));
                    } else {
                        stack.push(node_for_tile(&region, nk));
                    }
                }
            }
            Node::Infinity => {
                if region.infinite || !matches(layer.default) {
                    continue;
                }
                region.infinite = true;
                // Everything outside the allocated range is one connected
                // area; it borders the tiles on the edge of that range.
                let Some((x0, y0, x1, y1)) = region.tile_bounds else {
                    continue;
                };
                let mut border = Vec::new();
                for tx in x0..=x1 {
                    border.push(((tx, y0), (tx, y0 - 1)));
                    border.push(((tx, y1), (tx, y1 + 1)));
                }
                for ty in y0..=y1 {
                    border.push(((x0, ty), (x0 - 1, ty)));
                    border.push(((x1, ty), (x1 + 1, ty)));
                }
                for (k, outside) in border {
                    if region.existing.contains(&k) {
                        stack.extend(edge_pixels(outside, k).map(|(x, y)| Node::Pixel(x, y)));
                    } else {
                        stack.push(Node::Tile(k));
                    }
                }
            }
        }
    }
    region
}

/// Flood fills the contiguous region around (`x`, `y`) on `layer`.
///
/// If the region is enclosed, its pixels are painted (or erased, with an
/// erase `mode`). If it reaches past
/// everything drawn (out to infinity), the layer's default color changes to
/// the fill color instead, while enclosed areas keep their current look.
pub fn flood_fill(
    layer: &mut Layer,
    x: i32,
    y: i32,
    tolerance: u8,
    mode: PaintMode,
) -> Option<FillResult> {
    let target = layer.pixel(x, y);
    let region = find_region(layer, x, y, |p| color_matches(p, target, tolerance));

    let mut op = PaintOp::new(layer.id, mode);
    // Paint reached pixels in allocated tiles. When painting, grow the region
    // by one pixel so it tucks under anti-aliased edges instead of leaving a
    // seam; when erasing that would eat into the neighboring outline instead.
    let grow = matches!(mode, PaintMode::Paint { .. });
    let filled = |x: i32, y: i32| {
        region.contains(x, y)
            || (grow
                && (region.contains(x - 1, y)
                    || region.contains(x + 1, y)
                    || region.contains(x, y - 1)
                    || region.contains(x, y + 1)))
    };
    let mut keys: Vec<TileKey> = region.existing.iter().copied().collect();
    keys.sort_unstable();
    for k in keys {
        let near = [
            k,
            (k.0 - 1, k.1),
            (k.0 + 1, k.1),
            (k.0, k.1 - 1),
            (k.0, k.1 + 1),
        ];
        if !near.iter().any(|&n| region.touches_tile(n)) {
            continue;
        }
        let area = IRect {
            x0: k.0 * TILE,
            y0: k.1 * TILE,
            x1: (k.0 + 1) * TILE,
            y1: (k.1 + 1) * TILE,
        };
        op.apply(layer, area, |fx, fy| {
            if filled(fx.floor() as i32, fy.floor() as i32) {
                1.0
            } else {
                0.0
            }
        });
    }

    let mut old_default = None;
    if region.infinite {
        // Empty tiles the fill didn't reach are enclosed: pin their current
        // color before the default changes underneath them.
        if let Some((x0, y0, x1, y1)) = region.tile_bounds {
            for ty in y0..=y1 {
                for tx in x0..=x1 {
                    let k = (tx, ty);
                    if !region.existing.contains(&k) && !region.tiles.contains(&k) {
                        op.materialize(layer, k);
                    }
                }
            }
        }
        let new = match mode {
            PaintMode::Paint { color, opacity } => blend_over(layer.default, color, opacity),
            PaintMode::Erase { strength } => erase(layer.default, strength),
        };
        if new != layer.default {
            old_default = Some(layer.set_default(new));
        }
    } else {
        for &k in &region.tiles {
            let area = IRect {
                x0: k.0 * TILE,
                y0: k.1 * TILE,
                x1: (k.0 + 1) * TILE,
                y1: (k.1 + 1) * TILE,
            };
            op.apply(layer, area, |_, _| 1.0);
        }
    }

    let tiles = op.finish();
    if tiles.is_none() && old_default.is_none() {
        return None;
    }
    Some(FillResult {
        tiles: tiles.unwrap_or_default(),
        old_default,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_head_points_back_from_the_tip() {
        let path = arrow_path([0.0, 0.0], [100.0, 0.0], 4.0);
        assert_eq!(path.len(), 5);
        assert_eq!(
            (path[0], path[1], path[3]),
            ([0.0, 0.0], [100.0, 0.0], [100.0, 0.0])
        );
        // A 24 px head (12 + 3 × width) at 30° to the shaft, one barb each side.
        let (sin, cos) = 30f32.to_radians().sin_cos();
        for barb in [path[2], path[4]] {
            assert!((barb[0] - (100.0 - 24.0 * cos)).abs() < 1e-3, "{barb:?}");
            assert!((barb[1].abs() - 24.0 * sin).abs() < 1e-3, "{barb:?}");
        }
        assert!(path[2][1] * path[4][1] < 0.0, "barbs on opposite sides");
        // A short arrow keeps its head under half its length.
        let short = arrow_path([0.0, 0.0], [10.0, 0.0], 8.0);
        assert!(10.0 - short[2][0] < 5.0, "{short:?}");
    }

    const RED: Rgba = [255, 0, 0, 255];

    fn paint(color: Rgba) -> PaintMode {
        PaintMode::Paint {
            color,
            opacity: 1.0,
        }
    }

    #[test]
    fn negative_coordinates_map_to_tiles() {
        assert_eq!(tile_key(-1, -1), (-1, -1));
        assert_eq!(tile_key(0, 255), (0, 0));
        assert_eq!(tile_key(256, -257), (1, -2));
        assert_eq!(local_index(-1, 0), (TILE - 1) as usize);
    }

    #[test]
    fn blend_over_transparent_keeps_color() {
        assert_eq!(blend_over([0; 4], RED, 1.0), RED);
        assert_eq!(blend_over([0; 4], RED, 0.5), [255, 0, 0, 128]);
        assert_eq!(blend_over([0, 0, 255, 255], RED, 0.0), [0, 0, 255, 255]);
    }

    #[test]
    fn overlapping_stroke_does_not_accumulate_opacity() {
        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: RED,
                opacity: 0.5,
            },
        );
        for _ in 0..5 {
            op.stroke_segment(&mut layer, [10.0, 10.0], [20.0, 10.0], 4.0);
        }
        assert_eq!(layer.pixel(15, 10), [255, 0, 0, 128]);
    }

    #[test]
    fn stroke_across_tiles_and_undo_record() {
        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: RED,
                opacity: 1.0,
            },
        );
        op.stroke_segment(&mut layer, [-10.0, 5.0], [10.0, 5.0], 2.0);
        assert_eq!(layer.pixel(-5, 5), RED);
        assert_eq!(layer.pixel(5, 5), RED);
        let undo = op.finish().unwrap();
        assert_eq!(undo.len(), 2);
        // Tiles didn't exist before.
        assert!(undo.iter().all(|(_, t)| t.is_none()));
    }

    #[test]
    fn erase_removes_paint() {
        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: RED,
                opacity: 1.0,
            },
        );
        op.stroke_segment(&mut layer, [0.0, 0.0], [0.0, 0.0], 5.0);
        let mut op = PaintOp::new(1, PaintMode::Erase { strength: 1.0 });
        op.stroke_segment(&mut layer, [0.0, 0.0], [0.0, 0.0], 5.0);
        assert_eq!(layer.pixel(0, 0), [0; 4]);
    }

    #[test]
    fn flood_fill_stays_inside_circle() {
        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: [0, 0, 0, 255],
                opacity: 1.0,
            },
        );
        let circle = IRect {
            x0: -20,
            y0: -20,
            x1: 20,
            y1: 20,
        };
        op.ellipse(&mut layer, circle, 3.0, false);
        let blue = [0, 0, 255, 255];
        let r = flood_fill(&mut layer, 0, 0, 32, paint(blue)).unwrap();
        assert!(
            r.old_default.is_none(),
            "an enclosed fill must not change the default"
        );
        assert_eq!(layer.pixel(0, 0), blue);
        assert_eq!(layer.pixel(10, -5), blue);
        assert_eq!(
            layer.pixel(40, 40),
            [0; 4],
            "fill leaked outside the circle"
        );
    }

    #[test]
    fn flood_fill_empty_layer_changes_default() {
        let mut layer = Layer::new(1, "L");
        let r = flood_fill(&mut layer, 0, 0, 0, paint(RED)).unwrap();
        assert_eq!(r.old_default, Some([0; 4]));
        assert!(
            layer.tiles.is_empty(),
            "no pixels needed for an infinite fill"
        );
        assert_eq!(layer.pixel(-100_000, 50_000), RED);
    }

    #[test]
    fn copy_and_blit_roundtrip() {
        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: RED,
                opacity: 1.0,
            },
        );
        op.stroke_segment(&mut layer, [5.0, 5.0], [5.0, 5.0], 3.0);
        let img = copy_region(
            &layer,
            IRect {
                x0: 0,
                y0: 0,
                x1: 10,
                y1: 10,
            },
        );
        assert_eq!((img.width, img.height), (10, 10));
        assert_eq!(img.pixels[5 * 10 + 5], RED);

        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: [0; 4],
                opacity: 1.0,
            },
        );
        op.blit(&mut layer, &img, -300, 40);
        assert_eq!(layer.pixel(-295, 45), RED);
        assert_eq!(layer.pixel(-300, 40), [0; 4]);
        assert!(op.finish().is_some());
    }

    #[test]
    fn copy_merged_composites_visible_layers() {
        let area = IRect {
            x0: 0,
            y0: 0,
            x1: 4,
            y1: 4,
        };
        let mut bottom = Layer::new(1, "a");
        let mut top = Layer::new(2, "b");
        let mut hidden = Layer::new(3, "c");
        hidden.visible = false;
        for (layer, color) in [
            (&mut bottom, RED),
            (&mut top, [0, 0, 255, 255]),
            (&mut hidden, [0, 255, 0, 255]),
        ] {
            let mut op = PaintOp::new(
                layer.id,
                PaintMode::Paint {
                    color,
                    opacity: 1.0,
                },
            );
            op.apply(
                layer,
                IRect {
                    x0: 0,
                    y0: 0,
                    x1: if layer.id == 2 { 2 } else { 4 },
                    y1: 4,
                },
                |_, _| 1.0,
            );
        }
        let img = copy_region_merged(&[bottom, top, hidden], area);
        assert_eq!(img.pixels[0], [0, 0, 255, 255]);
        assert_eq!(img.pixels[3], RED);
    }

    #[test]
    fn content_bounds_and_thumbnail() {
        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: RED,
                opacity: 1.0,
            },
        );
        op.stroke_segment(&mut layer, [-20.0, 10.0], [20.0, 10.0], 2.0);
        let b = layer
            .tiles
            .iter()
            .filter_map(|(k, t)| tile_content_bounds(*k, t, layer.default))
            .reduce(|a, b| a.union(&b))
            .unwrap();
        assert!(
            b.x0 >= -23 && b.x1 <= 23 && b.y0 >= 7 && b.y1 <= 13,
            "{b:?}"
        );
        let thumb = thumbnail(&layer, b, 16);
        assert!(thumb.iter().any(|p| p[0] == 255 && p[3] > 0));
    }

    #[test]
    fn grid_layout_rounds_down() {
        let rect = GridKind::Rectangle;
        // 3x7 cells in a 10x20 area -> 3 columns, 2 rows.
        assert_eq!(rect.layout([0, 0], [3, 7], [10, 20]), ([0.0, 0.0], 3, 2));
        // Dragging up/left grows the grid the other way.
        assert_eq!(
            rect.layout([0, 0], [3, 7], [-10, -20]),
            ([-9.0, -14.0], 3, 2)
        );
        assert_eq!(rect.layout([5, 5], [3, 7], [6, 30]), ([5.0, 5.0], 0, 3));
    }

    #[test]
    fn square_snapping() {
        assert_eq!(square_end([0, 0], [10, 4]), [10, 10]);
        assert_eq!(square_end([0, 0], [-3, 8]), [-8, 8]);
        assert_eq!(square_end([5, 5], [5, 5]), [5, 5]);
    }

    #[test]
    fn rectangle_outline_and_fill() {
        let mut layer = Layer::new(1, "L");
        let r = IRect {
            x0: 0,
            y0: 0,
            x1: 20,
            y1: 10,
        };
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: RED,
                opacity: 1.0,
            },
        );
        op.rectangle(&mut layer, r, 2.0, false);
        assert_eq!(layer.pixel(10, 0), RED);
        assert_eq!(layer.pixel(19, 5), RED);
        assert_eq!(layer.pixel(10, 5), [0; 4], "outline must stay hollow");
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: RED,
                opacity: 1.0,
            },
        );
        op.rectangle(&mut layer, r, 2.0, true);
        assert_eq!(layer.pixel(10, 5), RED);
        assert_eq!(layer.pixel(21, 5), [0; 4], "nothing beyond the outline");
    }

    #[test]
    fn grid_draws_lines_not_cells() {
        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: RED,
                opacity: 1.0,
            },
        );
        op.grid(&mut layer, [0, 0], [10, 10], 3, 2, 2.0);
        for x in [0, 10, 20, 29] {
            assert_eq!(layer.pixel(x, 5), RED, "vertical line near x={x}");
        }
        assert_eq!(layer.pixel(15, 10), RED, "middle horizontal line");
        assert_eq!(layer.pixel(5, 5), [0; 4], "cell interior");
        assert_eq!(layer.pixel(35, 5), [0; 4], "beyond last column");
        assert_eq!(layer.pixel(5, 25), [0; 4], "beyond last row");
    }

    #[test]
    fn key_color_becomes_transparent() {
        let img = Image {
            width: 3,
            height: 1,
            pixels: vec![[255, 255, 255, 255], [250, 255, 255, 255], [0, 128, 0, 255]],
        };
        let out = color_to_transparent(&img, [255, 255, 255]);
        assert_eq!(out.pixels[..2], [[0; 4], [250, 255, 255, 255]]);
        let out = color_to_transparent(&img, [0, 128, 0]);
        assert_eq!(out.pixels[2], [0; 4]);
        assert_eq!(out.pixels[0], [255, 255, 255, 255]);
    }

    #[test]
    fn ellipse_distance_is_accurate() {
        // A circle: distance is just |p| - r.
        assert!((ellipse_distance(30.0, 40.0, 10.0, 10.0) - 40.0).abs() < 0.01);
        assert!((ellipse_distance(0.0, 0.0, 10.0, 10.0) + 10.0).abs() < 0.01);
        // On the axes of an elongated ellipse.
        assert!((ellipse_distance(25.0, 0.0, 20.0, 5.0) - 5.0).abs() < 0.01);
        assert!((ellipse_distance(0.0, 9.0, 20.0, 5.0) - 4.0).abs() < 0.01);
        assert!((ellipse_distance(0.0, 0.0, 20.0, 5.0) + 5.0).abs() < 0.01);
    }

    #[test]
    fn thick_ellipse_outline_is_round() {
        // A tiny circle with a huge pen: the outside must be a disc of radius r + w/2.
        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: RED,
                opacity: 1.0,
            },
        );
        let r = IRect {
            x0: -5,
            y0: -5,
            x1: 5,
            y1: 5,
        };
        op.ellipse(&mut layer, r, 200.0, false);
        // Radius 105 in every direction, including the diagonals.
        let inside = 103.0 / std::f32::consts::SQRT_2;
        let outside = 107.0 / std::f32::consts::SQRT_2;
        assert_eq!(layer.pixel(103, 0), RED);
        assert_eq!(layer.pixel(inside as i32, inside as i32), RED);
        assert_eq!(layer.pixel(outside as i32, outside as i32), [0; 4]);
        assert_eq!(layer.pixel(107, 0), [0; 4]);
    }

    #[test]
    fn ellipse_fits_its_rectangle() {
        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: RED,
                opacity: 1.0,
            },
        );
        let r = IRect {
            x0: 0,
            y0: 0,
            x1: 40,
            y1: 10,
        };
        op.ellipse(&mut layer, r, 1.0, true);
        assert_eq!(layer.pixel(20, 5), RED);
        assert_eq!(layer.pixel(1, 5), RED);
        assert_eq!(layer.pixel(40, 5), [0; 4]);
        assert_eq!(layer.pixel(1, 1), [0; 4], "corners of the box stay empty");
    }

    fn ring(layer: &mut Layer, r: IRect) {
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: [0, 0, 0, 255],
                opacity: 1.0,
            },
        );
        op.ellipse(layer, r, 3.0, false);
    }

    #[test]
    fn fill_outside_shape_changes_background_keeps_inside() {
        let mut layer = Layer::new(1, "L");
        ring(
            &mut layer,
            IRect {
                x0: -40,
                y0: -40,
                x1: 40,
                y1: 40,
            },
        );
        let r = flood_fill(&mut layer, 100, 0, 32, paint(RED)).unwrap();
        assert_eq!(r.old_default, Some([0; 4]));
        assert_eq!(layer.pixel(0, 0), [0; 4], "inside stays as it was");
        assert_eq!(layer.pixel(60, 0), RED, "outside next to the shape");
        assert_eq!(layer.pixel(5_000, -9_000), RED, "far away");
    }

    #[test]
    fn fill_outside_keeps_enclosed_empty_tiles() {
        // A ring big enough that whole tiles inside it were never allocated.
        let mut layer = Layer::new(1, "L");
        ring(
            &mut layer,
            IRect {
                x0: -900,
                y0: -900,
                x1: 900,
                y1: 900,
            },
        );
        assert!(!layer.tiles.contains_key(&(0, 0)));
        flood_fill(&mut layer, 2000, 0, 32, paint(RED)).unwrap();
        assert_eq!(
            layer.pixel(10, 10),
            [0; 4],
            "enclosed empty tile keeps its color"
        );
        assert_eq!(layer.pixel(2000, 2000), RED);
    }

    #[test]
    fn fill_inside_after_background_fill() {
        let mut layer = Layer::new(1, "L");
        ring(
            &mut layer,
            IRect {
                x0: -40,
                y0: -40,
                x1: 40,
                y1: 40,
            },
        );
        flood_fill(&mut layer, 100, 0, 32, paint(RED)).unwrap();
        let blue = [0, 0, 255, 255];
        let r = flood_fill(&mut layer, 0, 0, 32, paint(blue)).unwrap();
        assert!(r.old_default.is_none());
        assert_eq!(layer.pixel(0, 0), blue);
        assert_eq!(layer.pixel(100, 0), RED);
    }

    #[test]
    fn erase_on_filled_background_allocates() {
        let mut layer = Layer::new(1, "L");
        flood_fill(&mut layer, 0, 0, 0, paint(RED)).unwrap();
        let mut op = PaintOp::new(1, PaintMode::Erase { strength: 1.0 });
        op.stroke_segment(&mut layer, [0.0, 0.0], [0.0, 0.0], 3.0);
        assert_eq!(layer.pixel(0, 0), [0; 4]);
        assert_eq!(layer.pixel(10, 0), RED);
    }

    #[test]
    fn background_fill_does_not_grow_content_bounds() {
        let mut layer = Layer::new(1, "L");
        ring(
            &mut layer,
            IRect {
                x0: -40,
                y0: -40,
                x1: 40,
                y1: 40,
            },
        );
        let bounds = |layer: &Layer| {
            layer
                .tiles
                .iter()
                .filter_map(|(k, t)| tile_content_bounds(*k, t, layer.default))
                .reduce(|a, b| a.union(&b))
                .unwrap()
        };
        let before = bounds(&layer);
        flood_fill(&mut layer, 100, 0, 32, paint(RED)).unwrap();
        // The fill may cover the faintest anti-aliased edge pixels, but the
        // background around the circle must not count as content.
        let after = bounds(&layer);
        assert_eq!(
            after.union(&before),
            before,
            "{after:?} grew past {before:?}"
        );
        assert!(
            after.width() >= before.width() - 2,
            "{after:?} vs {before:?}"
        );
    }

    #[test]
    fn clear_fill_erases_region_and_background() {
        let mut layer = Layer::new(1, "L");
        ring(
            &mut layer,
            IRect {
                x0: -40,
                y0: -40,
                x1: 40,
                y1: 40,
            },
        );
        flood_fill(&mut layer, 0, 0, 32, paint(RED)).unwrap();
        flood_fill(&mut layer, 100, 0, 32, paint(RED)).unwrap();
        let clear = PaintMode::Erase { strength: 1.0 };
        // Clearing the inside erases just the enclosed red.
        let r = flood_fill(&mut layer, 0, 0, 32, clear).unwrap();
        assert!(r.old_default.is_none());
        assert_eq!(layer.pixel(0, 0), [0; 4]);
        assert_eq!(layer.pixel(100, 0), RED);
        // Clearing the outside makes the infinite background clear again.
        let r = flood_fill(&mut layer, 100, 0, 32, clear).unwrap();
        assert_eq!(r.old_default, Some(RED));
        assert_eq!(layer.pixel(100, 0), [0; 4]);
        assert_eq!(layer.pixel(9_000, 9_000), [0; 4]);
        assert_eq!(layer.pixel(40, 0)[3], 255, "the outline stays");
    }

    #[test]
    fn triangle_and_hexagon_grids_fit_and_draw() {
        // Triangles 10 wide overlap by half: 7 half-widths fit in 35 -> 6 triangles.
        assert_eq!(GridKind::Triangle.fit([10, 9], [35, 20]), (6, 2));
        assert_eq!(GridKind::Triangle.fit([10, 9], [9, 20]), (0, 2));
        // Hexagons 20 wide step by 15: 20 + 3*15 = 65 <= 70 -> 4 columns,
        // and the shifted columns need half a cell more height.
        assert_eq!(GridKind::Hexagon.fit([20, 17], [70, 40]), (4, 1));
        assert_eq!(GridKind::Hexagon.fit([20, 17], [20, 40]), (1, 2));
        for kind in GridKind::ALL {
            let (cols, rows) = kind.fit([12, 10], [100, 80]);
            let [ew, eh] = kind.extent([12, 10], cols, rows);
            assert!(ew <= 100.0 && eh <= 80.0, "{kind:?} overflows: {ew}x{eh}");
        }

        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(1, paint(RED));
        let segs = GridKind::Hexagon.segments([0.0, 0.0], [40, 34], 2, 1);
        assert_eq!(segs.len(), 12);
        for [a, b] in segs {
            op.stroke_segment(&mut layer, a, b, 1.0);
        }
        assert_eq!(layer.pixel(20, 0), RED, "top edge of the first hexagon");
        assert_eq!(layer.pixel(20, 17), [0; 4], "inside the first hexagon");
        assert_eq!(
            layer.pixel(50, 17),
            RED,
            "the second column starts half a cell lower"
        );
        assert_eq!(
            layer.pixel(50, 5),
            [0; 4],
            "nothing above the shifted column"
        );
    }

    #[test]
    fn grid_layout_grows_towards_the_drag() {
        let (tl, cols, rows) = GridKind::Triangle.layout([100, 100], [10, 9], [65, 80]);
        assert_eq!((cols, rows), (6, 2));
        assert_eq!(tl, [65.0, 82.0]);
    }

    #[test]
    fn triangle_outline_and_fill() {
        let r = IRect {
            x0: 0,
            y0: 0,
            x1: 40,
            y1: 30,
        };
        let pts = triangle_points(r);
        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(1, paint(RED));
        op.polygon(&mut layer, &pts, 2.0, false);
        assert_eq!(layer.pixel(20, 29), RED, "base");
        assert_eq!(layer.pixel(20, 0), RED, "apex");
        assert_eq!(layer.pixel(20, 20), [0; 4], "hollow inside");
        assert_eq!(layer.pixel(2, 2), [0; 4], "outside the slanted side");
        let mut op = PaintOp::new(1, paint(RED));
        op.polygon(&mut layer, &pts, 2.0, true);
        assert_eq!(layer.pixel(20, 20), RED);
        assert_eq!(layer.pixel(2, 2), [0; 4]);
    }

    #[test]
    fn polyline_joins_segments() {
        let mut layer = Layer::new(1, "L");
        let mut op = PaintOp::new(
            1,
            PaintMode::Paint {
                color: RED,
                opacity: 0.5,
            },
        );
        op.polyline(&mut layer, &[[0.5, 0.5], [20.5, 0.5], [20.5, 20.5]], 3.0);
        assert_eq!(layer.pixel(10, 0)[3], 128);
        assert_eq!(layer.pixel(20, 10)[3], 128);
        assert_eq!(layer.pixel(20, 0)[3], 128, "the joint isn't painted twice");
        assert_eq!(layer.pixel(10, 10), [0; 4]);
    }

    #[test]
    fn line_angle_snapping() {
        let s = snap_angle([0.0, 0.0], [100.0, 10.0]);
        assert!((s[0] - 100.0).abs() < 1e-3 && s[1].abs() < 1e-3);
        let s = snap_angle([0.0, 0.0], [50.0, 60.0]);
        assert!((s[0] - s[1]).abs() < 1e-3, "diagonal: {s:?}");
        let s = snap_angle([0.0, 0.0], [-3.0, -80.0]);
        assert!(s[0].abs() < 1e-3 && (s[1] + 80.0).abs() < 1e-3);
    }

    #[test]
    fn ngon_through_two_vertices() {
        for n in [3u32, 4, 5, 6, 7, 12, 20] {
            let (v0, vk) = ([10.0, 50.0], [110.0, 30.0]);
            let pts = ngon_from_vertices(v0, vk, n);
            assert_eq!(pts.len(), n as usize);
            let k = ngon_far_vertex(n) as usize;
            let close = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 0.01;
            assert!(close(pts[0], v0), "n={n}: vertex 0 at {:?}", pts[0]);
            assert!(close(pts[k], vk), "n={n}: vertex {k} at {:?}", pts[k]);
            let side = |i: usize| {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                (a[0] - b[0]).hypot(a[1] - b[1])
            };
            for i in 1..pts.len() {
                assert!((side(i) - side(0)).abs() < 0.01, "n={n}: uneven sides");
            }
        }
        // A left-to-right pentagon stands upright: its peak (vertex 1) is above.
        let pent = ngon_from_vertices([0.0, 0.0], [100.0, 0.0], 5);
        assert!(pent[1][1] < 0.0 && pent[3][1] > 0.0, "{pent:?}");
        // Even counts: the two points are opposite corners (a square's diagonal).
        let sq = ngon_from_vertices([0.0, 0.0], [10.0, 10.0], 4);
        assert!(
            sq.iter()
                .any(|p| (p[0] - 10.0).abs() < 0.01 && p[1].abs() < 0.01)
        );
        assert!(ngon_from_vertices([1.0, 1.0], [1.0, 1.0], 6).is_empty());
    }

    fn sample_image() -> Image {
        // 3x2: distinct colors to track where pixels go.
        let c = |i: u8| [i * 40, 0, 0, 255];
        Image {
            width: 3,
            height: 2,
            pixels: (0..6).map(c).collect(),
        }
    }

    #[test]
    fn flips_and_quarter_turns_are_exact() {
        let img = sample_image();
        assert_eq!(transform_image(&img, IDENTITY), img);
        let flipped = transform_image(&img, mat_mul([-1.0, 0.0, 0.0, 1.0], IDENTITY));
        assert_eq!(flipped.pixels[0], img.pixels[2]);
        assert_eq!(flipped.pixels[3], img.pixels[5]);
        let turned = transform_image(&img, rotation(std::f32::consts::FRAC_PI_2));
        assert_eq!((turned.width, turned.height), (2, 3));
        // Clockwise on screen (y down): the bottom-left pixel ends up top-left.
        assert_eq!(turned.pixels[0], img.pixels[3]);
        let back = transform_image(&turned, rotation(-std::f32::consts::FRAC_PI_2));
        assert_eq!(back, img);
    }

    #[test]
    fn rotation_grows_the_bounding_box() {
        let img = Image {
            width: 100,
            height: 100,
            pixels: vec![RED; 10_000],
        };
        let r = transform_image(&img, rotation(std::f32::consts::FRAC_PI_4));
        assert!((r.width as i32 - 142).abs() <= 1, "{}", r.width);
        assert_eq!(
            r.pixels[r.width * (r.height / 2) + r.width / 2],
            RED,
            "center"
        );
        assert_eq!(r.pixels[0][3], 0, "corners are empty");
        let m = mat_mul(rotation(0.3), [1.0, 0.5, 0.0, 1.0]);
        let inv = mat_inverse(m).unwrap();
        let p = mat_apply(inv, mat_apply(m, [3.0, -7.0]));
        assert!((p[0] - 3.0).abs() < 1e-4 && (p[1] + 7.0).abs() < 1e-4);
    }
}
