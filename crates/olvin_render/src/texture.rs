//! CPU textures and the [`Textures`] registry. Mipmaps are built on the CPU
//! (box filter, in linear space for sRGB data) when a texture is uploaded.

/// Pixel layout of [`TextureData::pixels`]; always four channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    /// 8-bit sRGB colour + linear alpha (albedo, emissive, LDR skies).
    Rgba8Srgb,
    /// 8-bit linear data (normal maps, ORM).
    Rgba8Linear,
    /// Half floats, little-endian (HDR skies).
    Rgba16Float,
    /// 32-bit floats, little-endian. Uploaded as `Rgba16Float`: 32-bit float
    /// textures aren't filterable on WebGL2-class hardware.
    Rgba32Float,
}

impl TextureFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8Srgb | Self::Rgba8Linear => 4,
            Self::Rgba16Float => 8,
            Self::Rgba32Float => 16,
        }
    }

    /// The GPU format it's uploaded as.
    pub(crate) fn gpu_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Rgba8Srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            Self::Rgba8Linear => wgpu::TextureFormat::Rgba8Unorm,
            Self::Rgba16Float | Self::Rgba32Float => wgpu::TextureFormat::Rgba16Float,
        }
    }
}

/// A 2D image, rows top to bottom, `width * height * bytes_per_pixel` bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub pixels: Vec<u8>,
}

impl TextureData {
    /// 8-bit RGBA; `srgb` for colour data, linear otherwise.
    pub fn rgba8(width: u32, height: u32, pixels: Vec<u8>, srgb: bool) -> Self {
        let format = if srgb {
            TextureFormat::Rgba8Srgb
        } else {
            TextureFormat::Rgba8Linear
        };
        Self {
            width,
            height,
            format,
            pixels,
        }
    }

    /// HDR from 32-bit float RGBA (e.g. a decoded `.hdr` / `.exr`).
    pub fn rgba32f(width: u32, height: u32, pixels: &[f32]) -> Self {
        Self {
            width,
            height,
            format: TextureFormat::Rgba32Float,
            pixels: bytemuck::cast_slice(pixels).to_vec(),
        }
    }

    /// A 1×1 texture.
    pub fn solid(rgba: [u8; 4], srgb: bool) -> Self {
        Self::rgba8(1, 1, rgba.to_vec(), srgb)
    }

    /// Size and pixel count agree.
    pub fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.pixels.len()
                == self.width as usize * self.height as usize * self.format.bytes_per_pixel()
    }

    /// Levels in a full mip chain down to 1×1.
    pub fn mip_count(width: u32, height: u32) -> u32 {
        32 - width.max(height).max(1).leading_zeros()
    }

    /// Converted to the format the GPU stores (`Rgba32Float` → `Rgba16Float`).
    pub(crate) fn to_gpu_layout(&self) -> TextureData {
        if self.format != TextureFormat::Rgba32Float {
            return self.clone();
        }
        let floats: &[f32] = bytemuck::cast_slice(&self.pixels);
        let pixels = floats
            .iter()
            .flat_map(|&f| f32_to_f16(f).to_le_bytes())
            .collect();
        TextureData {
            format: TextureFormat::Rgba16Float,
            pixels,
            ..*self
        }
    }

    /// The next mip level: half size (at least 1), 2×2 box filter (edge
    /// texels repeat for odd sizes), averaged in linear space.
    pub fn downsample(&self) -> TextureData {
        let (w, h) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let lut = srgb_lut();
        let read = |x: u32, y: u32| -> [f32; 4] {
            let (x, y) = (x.min(self.width - 1), y.min(self.height - 1));
            let i = (y * self.width + x) as usize;
            let bpp = self.format.bytes_per_pixel();
            let p = &self.pixels[i * bpp..(i + 1) * bpp];
            match self.format {
                TextureFormat::Rgba8Srgb => [
                    lut[p[0] as usize],
                    lut[p[1] as usize],
                    lut[p[2] as usize],
                    p[3] as f32 / 255.0,
                ],
                TextureFormat::Rgba8Linear => [0, 1, 2, 3].map(|c| p[c] as f32 / 255.0),
                TextureFormat::Rgba16Float => {
                    [0, 1, 2, 3].map(|c| f16_to_f32(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]])))
                }
                TextureFormat::Rgba32Float => [0, 1, 2, 3].map(|c| {
                    f32::from_le_bytes([p[c * 4], p[c * 4 + 1], p[c * 4 + 2], p[c * 4 + 3]])
                }),
            }
        };
        let mut pixels =
            Vec::with_capacity(w as usize * h as usize * self.format.bytes_per_pixel());
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = (x * 2, y * 2);
                let mut sum = [0.0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = read(sx + dx, sy + dy);
                    for c in 0..4 {
                        sum[c] += p[c] * 0.25;
                    }
                }
                match self.format {
                    TextureFormat::Rgba8Srgb => {
                        for c in sum.iter().take(3) {
                            pixels.push(unorm8(linear_to_srgb(*c)));
                        }
                        pixels.push(unorm8(sum[3]));
                    }
                    TextureFormat::Rgba8Linear => pixels.extend(sum.map(unorm8)),
                    TextureFormat::Rgba16Float => {
                        for c in sum {
                            pixels.extend(f32_to_f16(c).to_le_bytes());
                        }
                    }
                    TextureFormat::Rgba32Float => {
                        for c in sum {
                            pixels.extend(c.to_le_bytes());
                        }
                    }
                }
            }
        }
        TextureData {
            width: w,
            height: h,
            format: self.format,
            pixels,
        }
    }

    /// Mip levels 1.. down to 1×1 (level 0 is `self`).
    pub fn mip_chain(&self) -> Vec<TextureData> {
        let mut levels = Vec::new();
        let mut current = self;
        let mut next;
        while current.width > 1 || current.height > 1 {
            next = current.downsample();
            levels.push(next);
            current = levels.last().expect("just pushed");
        }
        levels
    }
}

fn unorm8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn srgb_lut() -> [f32; 256] {
    std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0))
}

/// IEEE half from f32, round to nearest even; overflow → ±inf.
pub(crate) fn f32_to_f16(value: f32) -> u16 {
    let x = value.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let mant = x & 0x007f_ffff;
    if exp == 0xff {
        // Inf / NaN (keep a quiet NaN bit).
        return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        // Subnormal half.
        let m = mant | 0x0080_0000;
        let shift = (14 - e) as u32;
        let half = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let mid = 1 << (shift - 1);
        let round = (rem > mid || (rem == mid && half & 1 == 1)) as u32;
        return sign | (half + round) as u16;
    }
    let half = ((e as u32) << 10) | (mant >> 13);
    let rem = mant & 0x1fff;
    let round = (rem > 0x1000 || (rem == 0x1000 && half & 1 == 1)) as u32;
    // A carry out of the mantissa correctly bumps the exponent (up to inf).
    sign | (half + round) as u16
}

pub(crate) fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let mant = (h & 0x3ff) as u32;
    let bits = match (exp, mant) {
        (0, 0) => sign,
        (0, m) => {
            // Subnormal: normalize.
            let mut e = 127 - 15 + 1;
            let mut m = m;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 0x3ff) << 13)
        }
        (0x1f, m) => sign | 0x7f80_0000 | (m << 13),
        (e, m) => sign | ((e + 127 - 15) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureHandle(pub(crate) u32);

/// Texture registry, like [`Meshes`](crate::Meshes): usable before the GPU
/// exists, uploaded (with mipmaps) by `RenderCore::upload_assets`.
#[derive(Default)]
pub struct Textures {
    next: u32,
    pub(crate) pending: Vec<(TextureHandle, TextureData)>,
}

impl Textures {
    pub fn add(&mut self, texture: TextureData) -> TextureHandle {
        let handle = TextureHandle(self.next);
        self.next += 1;
        self.pending.push((handle, texture));
        handle
    }

    /// Replaces a texture's pixels; materials and skies using it pick it up.
    pub fn set(&mut self, handle: TextureHandle, texture: TextureData) {
        self.pending.retain(|(h, _)| *h != handle);
        self.pending.push((handle, texture));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_sizes() {
        assert_eq!(TextureData::mip_count(1, 1), 1);
        assert_eq!(TextureData::mip_count(256, 256), 9);
        assert_eq!(TextureData::mip_count(8, 2), 4);
        assert_eq!(TextureData::mip_count(5, 3), 3);
        let t = TextureData::rgba8(8, 2, vec![255; 8 * 2 * 4], true);
        let sizes: Vec<_> = t.mip_chain().iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(sizes, [(4, 1), (2, 1), (1, 1)]);
        let odd = TextureData::rgba8(5, 3, vec![0; 5 * 3 * 4], false);
        let chain = odd.mip_chain();
        assert_eq!(chain.len() as u32 + 1, TextureData::mip_count(5, 3));
        assert!(chain.iter().all(TextureData::is_valid));
    }

    #[test]
    fn srgb_mips_average_in_linear_space() {
        // Black and white checker: the linear average is 0.5, i.e. sRGB ~188, not 128.
        let pixels = [
            0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255,
        ]
        .to_vec();
        let t = TextureData::rgba8(2, 2, pixels, true);
        let m = t.downsample();
        assert_eq!((m.width, m.height), (1, 1));
        assert!((186..=190).contains(&m.pixels[0]), "{:?}", m.pixels);
        assert_eq!(m.pixels[3], 255);
    }

    #[test]
    fn half_floats_round_trip() {
        for v in [0.0, 1.0, -2.5, 0.1, 65504.0, 1e-5, 3.0e-8] {
            let back = f16_to_f32(f32_to_f16(v));
            assert!((back - v).abs() <= v.abs() * 1e-3 + 6e-8, "{v} -> {back}");
        }
        assert_eq!(f16_to_f32(f32_to_f16(1e6)), f32::INFINITY);
        let hdr = TextureData::rgba32f(2, 1, &[4.0, 0.5, 0.25, 1.0, 8.0, 1.5, 0.75, 1.0]);
        let gpu = hdr.to_gpu_layout();
        assert!(gpu.is_valid() && gpu.format == TextureFormat::Rgba16Float);
        let m = gpu.downsample();
        assert_eq!(
            f16_to_f32(u16::from_le_bytes([m.pixels[0], m.pixels[1]])),
            6.0
        );
    }
}
