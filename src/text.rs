//! Text tool support: fonts, layout, rasterization and caret editing.
//!
//! Text is laid out from its origin (the top-left of the first line) with no
//! wrapping or box: lines only break where the text has a newline.

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont, point};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontFace {
    Sans,
    SansBold,
    Serif,
    Mono,
}

impl FontFace {
    pub const ALL: [FontFace; 4] = [Self::Sans, Self::SansBold, Self::Serif, Self::Mono];

    pub fn label(self) -> &'static str {
        match self {
            Self::Sans => "Sans",
            Self::SansBold => "Sans Bold",
            Self::Serif => "Serif",
            Self::Mono => "Mono",
        }
    }

    fn font(self) -> FontRef<'static> {
        let bytes = match self {
            Self::Sans => dejavu::sans::regular(),
            Self::SansBold => dejavu::sans::bold(),
            Self::Serif => dejavu::serif::regular(),
            Self::Mono => dejavu::sans_mono::regular(),
        };
        FontRef::try_from_slice(bytes).expect("bundled font is valid")
    }
}

/// Font and size (in canvas pixels) of a piece of text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub face: FontFace,
    pub size: f32,
}

impl TextStyle {
    /// Height of a caret: the font's full ascent-to-descent extent.
    pub fn caret_height(&self) -> f32 {
        let font = self.face.font();
        let f = font.as_scaled(PxScale::from(self.size));
        f.ascent() - f.descent()
    }

    /// Distance between the tops of consecutive lines.
    pub fn line_height(&self) -> f32 {
        let font = self.face.font();
        let f = font.as_scaled(PxScale::from(self.size));
        f.ascent() - f.descent() + f.line_gap()
    }

    /// Width of `line` (which must not contain newlines).
    pub fn advance(&self, line: &str) -> f32 {
        let font = self.face.font();
        let f = font.as_scaled(PxScale::from(self.size));
        let mut x = 0.0;
        let mut prev: Option<GlyphId> = None;
        for c in line.chars() {
            let id = f.glyph_id(c);
            if let Some(p) = prev {
                x += f.kern(p, id);
            }
            x += f.h_advance(id);
            prev = Some(id);
        }
        x
    }
}

/// Anti-aliased coverage of rendered text, positioned relative to the text
/// origin (it can start left of or above it).
pub struct Mask {
    pub x: i32,
    pub y: i32,
    pub width: usize,
    pub height: usize,
    pub coverage: Vec<f32>,
}

impl Mask {
    pub fn at(&self, x: i32, y: i32) -> f32 {
        let (lx, ly) = (x - self.x, y - self.y);
        if lx < 0 || ly < 0 || lx as usize >= self.width || ly as usize >= self.height {
            0.0
        } else {
            self.coverage[ly as usize * self.width + lx as usize]
        }
    }
}

/// Rasterizes `text` into a coverage mask, or `None` if nothing is visible.
pub fn render(style: TextStyle, text: &str) -> Option<Mask> {
    let font = style.face.font();
    let scale = PxScale::from(style.size);
    let f = font.as_scaled(scale);
    let line_height = style.line_height();

    let mut glyphs = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        let baseline = f.ascent() + i as f32 * line_height;
        let mut x = 0.0;
        let mut prev: Option<GlyphId> = None;
        for c in line.chars() {
            let id = f.glyph_id(c);
            if let Some(p) = prev {
                x += f.kern(p, id);
            }
            let glyph = id.with_scale_and_position(scale, point(x, baseline));
            if let Some(outlined) = font.outline_glyph(glyph) {
                glyphs.push(outlined);
            }
            x += f.h_advance(id);
            prev = Some(id);
        }
    }

    let bounds = glyphs
        .iter()
        .map(|g| g.px_bounds())
        .reduce(|a, b| ab_glyph::Rect {
            min: point(a.min.x.min(b.min.x), a.min.y.min(b.min.y)),
            max: point(a.max.x.max(b.max.x), a.max.y.max(b.max.y)),
        })?;
    let x0 = bounds.min.x.floor() as i32;
    let y0 = bounds.min.y.floor() as i32;
    let width = (bounds.max.x.ceil() as i32 - x0).max(0) as usize;
    let height = (bounds.max.y.ceil() as i32 - y0).max(0) as usize;
    let mut coverage = vec![0.0f32; width * height];
    for g in &glyphs {
        let b = g.px_bounds();
        let (gx0, gy0) = (b.min.x as i32 - x0, b.min.y as i32 - y0);
        g.draw(|gx, gy, c| {
            let (px, py) = (gx0 + gx as i32, gy0 + gy as i32);
            if px >= 0 && py >= 0 && (px as usize) < width && (py as usize) < height {
                let cell = &mut coverage[py as usize * width + px as usize];
                // Overlapping glyphs (e.g. italics, kerning) shouldn't exceed full coverage.
                *cell = (*cell + c).min(1.0);
            }
        });
    }
    Some(Mask {
        x: x0,
        y: y0,
        width,
        height,
        coverage,
    })
}

/// Text being typed, with a caret (a byte index into `text`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Buffer {
    pub text: String,
    pub caret: usize,
}

impl Buffer {
    pub fn insert(&mut self, s: &str) {
        // Normalize Windows line endings from pasted text.
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        self.text.insert_str(self.caret, &s);
        self.caret += s.len();
    }

    pub fn backspace(&mut self) {
        if let Some(c) = self.text[..self.caret].chars().next_back() {
            self.caret -= c.len_utf8();
            self.text.remove(self.caret);
        }
    }

    pub fn delete(&mut self) {
        if self.caret < self.text.len() {
            self.text.remove(self.caret);
        }
    }

    pub fn left(&mut self) {
        if let Some(c) = self.text[..self.caret].chars().next_back() {
            self.caret -= c.len_utf8();
        }
    }

    pub fn right(&mut self) {
        if let Some(c) = self.text[self.caret..].chars().next() {
            self.caret += c.len_utf8();
        }
    }

    fn line_start(&self) -> usize {
        self.text[..self.caret].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.caret..]
            .find('\n')
            .map_or(self.text.len(), |i| self.caret + i)
    }

    pub fn home(&mut self) {
        self.caret = self.line_start();
    }

    pub fn end(&mut self) {
        self.caret = self.line_end();
    }

    /// Moves to the previous/next line, keeping the column in characters.
    pub fn vertical(&mut self, down: bool) {
        let start = self.line_start();
        let column = self.text[start..self.caret].chars().count();
        let target_start = if down {
            let end = self.line_end();
            if end == self.text.len() {
                return;
            }
            end + 1
        } else {
            if start == 0 {
                return;
            }
            self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1)
        };
        let line = self.text[target_start..].split('\n').next().unwrap_or("");
        let offset: usize = line.chars().take(column).map(char::len_utf8).sum();
        self.caret = target_start + offset;
    }

    /// Line index and the text before the caret on that line.
    pub fn caret_line(&self) -> (usize, &str) {
        let start = self.line_start();
        (
            self.text[..start].matches('\n').count(),
            &self.text[start..self.caret],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(text: &str, caret: usize) -> Buffer {
        Buffer {
            text: text.into(),
            caret,
        }
    }

    #[test]
    fn typing_and_newlines() {
        let mut b = Buffer::default();
        b.insert("hi");
        b.insert("\n");
        b.insert("yo");
        assert_eq!(b.text, "hi\nyo");
        assert_eq!(b.caret_line(), (1, "yo"));
        b.backspace();
        b.backspace();
        b.backspace();
        assert_eq!(b.text, "hi");
        b.insert("a\r\nb");
        assert_eq!(b.text, "hia\nb");
    }

    #[test]
    fn caret_movement_handles_unicode_and_lines() {
        let mut b = buffer("aé\nxyz", 0);
        b.right();
        b.right();
        assert_eq!(b.caret, 3, "é is two bytes");
        b.vertical(true);
        assert_eq!(b.caret_line(), (1, "xy"));
        b.end();
        assert_eq!(b.caret_line(), (1, "xyz"));
        b.vertical(false);
        assert_eq!(
            b.caret_line(),
            (0, "aé"),
            "column clamps to the shorter line"
        );
        b.home();
        assert_eq!(b.caret, 0);
        b.left();
        assert_eq!(b.caret, 0);
        b.delete();
        assert_eq!(b.text, "é\nxyz");
    }

    #[test]
    fn renders_lines_below_each_other() {
        let style = TextStyle {
            face: FontFace::Sans,
            size: 32.0,
        };
        let one = render(style, "Hello").unwrap();
        let two = render(style, "Hello\nHello").unwrap();
        assert!(one.coverage.iter().any(|&c| c > 0.9));
        assert_eq!(one.width, two.width);
        let lh = style.line_height();
        assert!((two.height as f32 - one.height as f32 - lh).abs() <= 2.0);
        assert!(
            render(style, " \n ").is_none(),
            "whitespace renders nothing"
        );
        assert!(style.advance("Hello") > style.advance("Hell"));
    }
}
