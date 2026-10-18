//! sol_img: image decoding (PNG/GIF/BMP) and PNG encoding.
//! `Image` is a straight RGBA8 buffer.

pub mod bmp;
pub mod gif;
pub mod png;

pub const MAX_DIMENSION: u32 = 16384;
pub const MAX_PIXELS: usize = 16 * 1024 * 1024;
pub const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn dimensions(w: u32, h: u32) -> Result<usize, String> {
    if w == 0 || h == 0 || w > MAX_DIMENSION || h > MAX_DIMENSION {
        return Err("invalid image dimensions".into());
    }
    // File headers control both dimensions. Checking their product before any
    // allocation prevents overflow from turning a large declared image into a
    // small buffer that later scanline writes would incorrectly trust.
    let pixels = (w as usize)
        .checked_mul(h as usize)
        .ok_or("image dimensions overflow")?;
    if pixels > MAX_PIXELS {
        return Err("image pixel limit exceeded".into());
    }
    Ok(pixels)
}

pub(crate) fn input(data: &[u8]) -> Result<(), String> {
    if data.len() > MAX_INPUT_BYTES {
        return Err("image input limit exceeded".into());
    }
    Ok(())
}

pub(crate) fn zeroed(len: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|e| format!("image allocation: {e}"))?;
    bytes.resize(len, 0);
    Ok(bytes)
}

#[derive(Clone, Debug)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    /// RGBA8, row-major, top-left origin
    pub px: Vec<u8>,
}

impl Image {
    pub fn new(w: u32, h: u32) -> Image {
        Self::try_new(w, h).expect("valid image dimensions and available memory")
    }
    pub fn try_new(w: u32, h: u32) -> Result<Image, String> {
        let pixels = dimensions(w, h)?;
        Ok(Image {
            w,
            h,
            px: zeroed(pixels * 4)?,
        })
    }
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, c: [u8; 4]) {
        if x < self.w && y < self.h {
            let i = (y * self.w + x) as usize * 4;
            self.px[i..i + 4].copy_from_slice(&c);
        }
    }
    #[inline]
    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        if x < self.w && y < self.h {
            let i = (y * self.w + x) as usize * 4;
            self.px[i..i + 4].try_into().unwrap()
        } else {
            [0, 0, 0, 0]
        }
    }
}

pub fn decode(data: &[u8]) -> Result<Image, String> {
    input(data)?;
    if data.starts_with(b"\x89PNG") {
        png::decode(data)
    } else if data.starts_with(b"GIF8") {
        gif::decode(data)
    } else if data.starts_with(b"BM") {
        bmp::decode(data)
    } else {
        Err("unknown image format".into())
    }
}

/// PNG encode — used for screenshots.
pub fn encode_png(img: &Image) -> Vec<u8> {
    png::encode(img)
}
