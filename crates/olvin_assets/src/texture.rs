//! Image decoding (PNG, JPEG, TGA, BMP, WebP → RGBA8; HDR, EXR → RGBA32F),
//! downscaling and editor thumbnails with a small LRU cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use image::{DynamicImage, ImageFormat as FileFormat, imageops::FilterType};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// 4 bytes per pixel, colour data (decode sRGB → linear on the GPU).
    Rgba8Srgb,
    /// 4 bytes per pixel, raw data (normal maps, masks).
    Rgba8Linear,
    /// 16 bytes per pixel, native-endian `f32` (HDR panoramas).
    Rgba32Float,
}

impl ImageFormat {
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            ImageFormat::Rgba8Srgb | ImageFormat::Rgba8Linear => 4,
            ImageFormat::Rgba32Float => 16,
        }
    }
}

/// Decoded pixels, rows top to bottom, tightly packed.
#[derive(Clone, PartialEq)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for ImageData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ImageData({}x{} {:?}, {} bytes)",
            self.width,
            self.height,
            self.format,
            self.pixels.len()
        )
    }
}

impl ImageData {
    /// Pixels as `f32` (only for [`ImageFormat::Rgba32Float`]).
    pub fn floats(&self) -> Vec<f32> {
        self.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_ne_bytes(*b))
            .collect()
    }

    fn from_dynamic(img: DynamicImage, float: bool, srgb: bool) -> Self {
        let (width, height) = (img.width(), img.height());
        if float {
            let pixels = img
                .into_rgba32f()
                .into_raw()
                .into_iter()
                .flat_map(f32::to_ne_bytes)
                .collect();
            Self {
                width,
                height,
                format: ImageFormat::Rgba32Float,
                pixels,
            }
        } else {
            Self {
                width,
                height,
                format: if srgb {
                    ImageFormat::Rgba8Srgb
                } else {
                    ImageFormat::Rgba8Linear
                },
                pixels: img.into_rgba8().into_raw(),
            }
        }
    }

    fn to_dynamic(&self) -> DynamicImage {
        // invariant: `pixels` is always `width * height` RGBA texels in the
        // `format` (every constructor fills it from a decoded image).
        match self.format {
            ImageFormat::Rgba32Float => DynamicImage::ImageRgba32F(
                image::Rgba32FImage::from_raw(self.width, self.height, self.floats())
                    .expect("size"),
            ),
            _ => DynamicImage::ImageRgba8(
                image::RgbaImage::from_raw(self.width, self.height, self.pixels.clone())
                    .expect("size"),
            ),
        }
    }

    /// Resizes (Triangle filter) so both sides are ≤ `max_size`,
    /// keeping the aspect. 0 = no limit.
    pub fn downscale(&mut self, max_size: u32) {
        if max_size == 0 || (self.width <= max_size && self.height <= max_size) {
            return;
        }
        let k = max_size as f32 / self.width.max(self.height) as f32;
        let w = ((self.width as f32 * k).round() as u32).clamp(1, max_size);
        let h = ((self.height as f32 * k).round() as u32).clamp(1, max_size);
        let resized = self.to_dynamic().resize_exact(w, h, FilterType::Triangle);
        *self = Self::from_dynamic(
            resized,
            self.format == ImageFormat::Rgba32Float,
            self.format == ImageFormat::Rgba8Srgb,
        );
    }
}

fn file_format(path: &Path) -> Option<FileFormat> {
    FileFormat::from_path(path).ok()
}

fn is_float_format(path: &Path, img: &DynamicImage) -> bool {
    matches!(
        file_format(path),
        Some(FileFormat::Hdr | FileFormat::OpenExr)
    ) || matches!(
        img,
        DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_)
    )
}

fn decode(path: &Path) -> Result<DynamicImage, String> {
    image::ImageReader::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .with_guessed_format()
        .map_err(|e| format!("{}: {e}", path.display()))?
        .decode()
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Decodes an image to RGBA. HDR/EXR become [`ImageFormat::Rgba32Float`];
/// everything else RGBA8 tagged sRGB or linear by `srgb`.
pub fn load_image(path: impl AsRef<Path>, srgb: bool) -> Result<ImageData, String> {
    load_image_sized(path, srgb, 0)
}

/// [`load_image`] then [`ImageData::downscale`] to `max_size` (0 = keep).
pub fn load_image_sized(
    path: impl AsRef<Path>,
    srgb: bool,
    max_size: u32,
) -> Result<ImageData, String> {
    let path = path.as_ref();
    let img = decode(path)?;
    let float = is_float_format(path, &img);
    let mut data = ImageData::from_dynamic(img, float, srgb);
    data.downscale(max_size);
    Ok(data)
}

/// RGBA8 sRGB preview fitting a `size`×`size` square (aspect kept). HDR is
/// tonemapped (Reinhard). `None` for non-images or decode errors.
pub fn thumbnail(path: impl AsRef<Path>, size: u32) -> Option<ImageData> {
    let path = path.as_ref();
    if crate::AssetKind::of(path) != crate::AssetKind::Texture || size == 0 {
        return None;
    }
    let img = decode(path).ok()?;
    let float = is_float_format(path, &img);
    // Tonemap first: `thumbnail` on float images is biased in image 0.25.
    let rgba = if float {
        let mut f = img.into_rgba32f();
        for p in f.pixels_mut() {
            for c in &mut p.0[..3] {
                let x = *c / (1.0 + *c);
                *c = if x <= 0.003_130_8 {
                    x * 12.92
                } else {
                    1.055 * x.powf(1.0 / 2.4) - 0.055
                };
            }
        }
        DynamicImage::ImageRgba32F(f).into_rgba8()
    } else {
        img.into_rgba8()
    };
    let rgba = DynamicImage::ImageRgba8(rgba)
        .thumbnail(size, size)
        .into_rgba8();
    Some(ImageData {
        width: rgba.width(),
        height: rgba.height(),
        format: ImageFormat::Rgba8Srgb,
        pixels: rgba.into_raw(),
    })
}

/// (file, its modification time, thumbnail size).
type ThumbKey = (PathBuf, SystemTime, u32);

/// In-memory LRU of thumbnails keyed by (path, mtime, size): an edited file
/// gets a fresh thumbnail, an unchanged one is decoded once.
pub struct ThumbnailCache {
    capacity: usize,
    tick: u64,
    entries: HashMap<ThumbKey, (Option<Arc<ImageData>>, u64)>,
}

impl ThumbnailCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            tick: 0,
            entries: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Cached thumbnail, decoding on a miss (failures are cached too).
    pub fn get(&mut self, path: impl AsRef<Path>, size: u32) -> Option<Arc<ImageData>> {
        let path = path.as_ref();
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
        self.tick += 1;
        let key = (path.to_path_buf(), mtime, size);
        if let Some((img, used)) = self.entries.get_mut(&key) {
            *used = self.tick;
            return img.clone();
        }
        // Drop stale versions of the same file/size, then the least recently used.
        self.entries
            .retain(|(p, _, s), _| !(p == path && *s == size));
        while self.entries.len() >= self.capacity {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => self.entries.remove(&k),
                None => break,
            };
        }
        let img = thumbnail(path, size).map(Arc::new);
        self.entries.insert(key, (img.clone(), self.tick));
        img
    }
}

impl Default for ThumbnailCache {
    fn default() -> Self {
        Self::new(256)
    }
}
