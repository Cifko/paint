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

pub fn empty_tile() -> Vec<Rgba> {
    vec![[0; 4]; TILE_PIXELS]
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
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
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
    /// Tiles changed since the renderer last uploaded them.
    pub dirty: HashSet<TileKey>,
}

impl Layer {
    pub fn new(id: LayerId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            visible: true,
            opacity: 1.0,
            tiles: HashMap::new(),
            dirty: HashSet::new(),
        }
    }

    pub fn pixel(&self, x: i32, y: i32) -> Rgba {
        self.tiles
            .get(&tile_key(x, y))
            .map_or([0; 4], |t| t[local_index(x, y)])
    }

    /// Bounding rectangle of all allocated tiles.
    pub fn bounds(&self) -> Option<IRect> {
        self.tiles.keys().fold(None, |acc, &(tx, ty)| {
            let r = IRect {
                x0: tx * TILE,
                y0: ty * TILE,
                x1: (tx + 1) * TILE,
                y1: (ty + 1) * TILE,
            };
            Some(acc.map_or(r, |a: IRect| a.union(&r)))
        })
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
        // Erasing never needs to allocate a tile that doesn't exist yet.
        if erasing && !layer.tiles.contains_key(&key) && !self.before.contains_key(&key) {
            return;
        }
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
                let old = before.as_ref().map_or([0; 4], |t| t[i]);
                let new = match self.mode {
                    PaintMode::Paint { opacity, .. } => blend_over(old, color, c * opacity),
                    PaintMode::Erase { strength } => erase(old, c * strength),
                };
                layer.tiles.entry(key).or_insert_with(empty_tile)[i] = new;
                touched = true;
            }
        }
        if touched {
            layer.dirty.insert(key);
        }
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

    /// Draws a circle around `center`, either as an outline of width `width` or filled.
    pub fn circle(
        &mut self,
        layer: &mut Layer,
        center: [f32; 2],
        radius: f32,
        width: f32,
        filled: bool,
    ) {
        let half = (width / 2.0).max(0.5);
        let outer = radius + if filled { 0.0 } else { half };
        let area = bbox(
            center[0] - outer,
            center[1] - outer,
            center[0] + outer,
            center[1] + outer,
        );
        self.apply(layer, area, |x, y| {
            let d = ((x - center[0]).powi(2) + (y - center[1]).powi(2)).sqrt();
            if filled {
                coverage(radius - d)
            } else {
                coverage(half - (d - radius).abs())
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

/// Tight bounds of the non-transparent pixels in a tile, in canvas pixels.
pub fn tile_content_bounds(key: TileKey, tile: &[Rgba]) -> Option<IRect> {
    let mut b: Option<IRect> = None;
    for (i, p) in tile.iter().enumerate() {
        if p[3] == 0 {
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

/// Flood fills the contiguous region around (`x`, `y`) on `layer`.
///
/// The canvas is infinite, so the fill is limited to `limit` (typically the
/// visible area combined with the layer's painted bounds).
pub fn flood_fill(
    layer: &mut Layer,
    x: i32,
    y: i32,
    limit: IRect,
    tolerance: u8,
    color: Rgba,
    opacity: f32,
) -> Option<Vec<(TileKey, Option<Vec<Rgba>>)>> {
    if !limit.contains(x, y) {
        return None;
    }
    let target = layer.pixel(x, y);
    let (w, h) = (limit.width() as usize, limit.height() as usize);
    let mut mask = vec![false; w * h];
    let idx = |x: i32, y: i32| (y - limit.y0) as usize * w + (x - limit.x0) as usize;
    let matches = |x: i32, y: i32| color_matches(layer.pixel(x, y), target, tolerance);

    // Scanline fill.
    let mut stack = vec![(x, y)];
    while let Some((sx, sy)) = stack.pop() {
        if mask[idx(sx, sy)] || !matches(sx, sy) {
            continue;
        }
        let mut lx = sx;
        while lx > limit.x0 && !mask[idx(lx - 1, sy)] && matches(lx - 1, sy) {
            lx -= 1;
        }
        let mut rx = sx;
        while rx + 1 < limit.x1 && !mask[idx(rx + 1, sy)] && matches(rx + 1, sy) {
            rx += 1;
        }
        for px in lx..=rx {
            mask[idx(px, sy)] = true;
        }
        for ny in [sy - 1, sy + 1] {
            if ny < limit.y0 || ny >= limit.y1 {
                continue;
            }
            let mut px = lx;
            while px <= rx {
                if !mask[idx(px, ny)] && matches(px, ny) {
                    stack.push((px, ny));
                    // Skip the rest of this run; it'll be filled from the pushed seed.
                    while px <= rx && matches(px, ny) {
                        px += 1;
                    }
                } else {
                    px += 1;
                }
            }
        }
    }

    // Grow the region by one pixel so it tucks under anti-aliased edges
    // instead of leaving a light seam between the fill and the outline.
    let filled = |x: i32, y: i32| limit.contains(x, y) && mask[idx(x, y)];
    let mut op = PaintOp::new(layer.id, PaintMode::Paint { color, opacity });
    op.apply(layer, limit, |fx, fy| {
        let (px, py) = (fx.floor() as i32, fy.floor() as i32);
        let hit = filled(px, py)
            || filled(px - 1, py)
            || filled(px + 1, py)
            || filled(px, py - 1)
            || filled(px, py + 1);
        if hit { 1.0 } else { 0.0 }
    });
    op.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba = [255, 0, 0, 255];

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
        op.circle(&mut layer, [0.0, 0.0], 20.0, 3.0, false);
        let limit = IRect {
            x0: -50,
            y0: -50,
            x1: 50,
            y1: 50,
        };
        let blue = [0, 0, 255, 255];
        flood_fill(&mut layer, 0, 0, limit, 32, blue, 1.0).unwrap();
        assert_eq!(layer.pixel(0, 0), blue);
        assert_eq!(layer.pixel(10, -5), blue);
        assert_eq!(
            layer.pixel(40, 40),
            [0; 4],
            "fill leaked outside the circle"
        );
    }

    #[test]
    fn flood_fill_empty_layer_fills_limit() {
        let mut layer = Layer::new(1, "L");
        let limit = IRect {
            x0: -10,
            y0: -10,
            x1: 10,
            y1: 10,
        };
        flood_fill(&mut layer, 0, 0, limit, 0, RED, 1.0).unwrap();
        assert_eq!(layer.pixel(-10, -10), RED);
        assert_eq!(layer.pixel(9, 9), RED);
        assert_eq!(layer.pixel(10, 10), [0; 4]);
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
            .filter_map(|(k, t)| tile_content_bounds(*k, t))
            .reduce(|a, b| a.union(&b))
            .unwrap();
        assert!(
            b.x0 >= -23 && b.x1 <= 23 && b.y0 >= 7 && b.y1 <= 13,
            "{b:?}"
        );
        let thumb = thumbnail(&layer, b, 16);
        assert!(thumb.iter().any(|p| p[0] == 255 && p[3] > 0));
    }
}
