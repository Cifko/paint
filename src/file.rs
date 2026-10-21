//! Loading and saving images (PNG, JPEG, BMP).

use std::path::{Path, PathBuf};

use crate::canvas::Image;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Bmp,
}

impl Format {
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "bmp" => Some(Self::Bmp),
            _ => None,
        }
    }

    /// Whether the format keeps transparency.
    pub fn has_alpha(self) -> bool {
        self == Self::Png
    }
}

/// File-dialog filters for the formats we can save.
pub const FILTERS: [(&str, &[&str]); 3] = [
    ("PNG image", &["png"]),
    ("JPEG image", &["jpg", "jpeg"]),
    ("BMP image", &["bmp"]),
];

/// Adds `.png` to paths without a known image extension.
pub fn with_image_extension(path: PathBuf) -> PathBuf {
    if Format::from_path(&path).is_some() {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".png");
        name.into()
    }
}

/// Saves `image`. Formats without transparency get it flattened onto white.
pub fn save(path: &Path, image: &Image) -> Result<(), String> {
    let format = Format::from_path(path).ok_or("Unsupported file type")?;
    let (w, h) = (image.width as u32, image.height as u32);
    let result = if format.has_alpha() {
        let buf = image::RgbaImage::from_raw(w, h, image.pixels.as_flattened().to_vec())
            .ok_or("Invalid image size")?;
        buf.save(path)
    } else {
        let rgb: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(|p| {
                let a = p[3] as u32;
                [0, 1, 2].map(|c| ((p[c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8)
            })
            .collect();
        let buf = image::RgbImage::from_raw(w, h, rgb).ok_or("Invalid image size")?;
        buf.save(path)
    };
    result.map_err(|e| e.to_string())
}

pub fn load(path: &Path) -> Result<Image, String> {
    let img = image::open(path).map_err(|e| e.to_string())?.into_rgba8();
    let (w, h) = img.dimensions();
    Ok(Image {
        width: w as usize,
        height: h as usize,
        pixels: img
            .into_raw()
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Image {
        Image {
            width: 3,
            height: 2,
            pixels: vec![
                [255, 0, 0, 255],
                [0, 255, 0, 128],
                [0, 0, 0, 0],
                [10, 20, 30, 255],
                [255, 255, 255, 255],
                [0, 0, 255, 255],
            ],
        }
    }

    #[test]
    fn png_roundtrip_keeps_alpha() {
        let dir = std::env::temp_dir().join(format!("paint-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        save(&path, &sample()).unwrap();
        assert_eq!(load(&path).unwrap(), sample());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn bmp_flattens_onto_white() {
        let dir = std::env::temp_dir().join(format!("paint-test-bmp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.bmp");
        save(&path, &sample()).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back.pixels[0], [255, 0, 0, 255]);
        assert_eq!(back.pixels[1], [127, 255, 127, 255]);
        assert_eq!(back.pixels[2], [255, 255, 255, 255]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn extensions() {
        assert_eq!(Format::from_path(Path::new("x.JPG")), Some(Format::Jpeg));
        assert_eq!(Format::from_path(Path::new("x")), None);
        assert_eq!(with_image_extension("a/b".into()), PathBuf::from("a/b.png"));
        assert_eq!(with_image_extension("b.bmp".into()), PathBuf::from("b.bmp"));
    }
}
