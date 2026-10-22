//! TGA (CC5's shader masks): uncompressed or RLE, true colour (24 / 32 bit) or
//! greyscale (8 bit). Decoded here so the `image` crate's features (shared with
//! Bevy's) stay as they are.

use anyhow::{bail, ensure, Result};
use image::DynamicImage;

pub fn decode(b: &[u8]) -> Result<DynamicImage> {
    ensure!(b.len() >= 18, "TGA: short header");
    let id_len = b[0] as usize;
    let cmap_type = b[1];
    let kind = b[2];
    let cmap_len = u16::from_le_bytes([b[5], b[6]]) as usize;
    let cmap_bits = b[7] as usize;
    let w = u16::from_le_bytes([b[12], b[13]]) as usize;
    let h = u16::from_le_bytes([b[14], b[15]]) as usize;
    let bpp = b[16] as usize;
    let desc = b[17];
    ensure!(cmap_type == 0, "TGA: colour-mapped images are not supported");
    let (rle, grey) = match kind {
        2 => (false, false),
        3 => (false, true),
        10 => (true, false),
        11 => (true, true),
        _ => bail!("TGA: image type {kind} not supported"),
    };
    let px = bpp / 8;
    ensure!(matches!((grey, px), (true, 1) | (false, 3) | (false, 4)), "TGA: {bpp} bit {} not supported", if grey { "greyscale" } else { "colour" });
    let mut at = 18 + id_len + cmap_len * cmap_bits.div_ceil(8);
    let n = w * h * px;
    let mut data = Vec::with_capacity(n);
    if rle {
        while data.len() < n {
            ensure!(at < b.len(), "TGA: truncated");
            let head = b[at] as usize;
            at += 1;
            let count = (head & 0x7f) + 1;
            if head & 0x80 != 0 {
                ensure!(at + px <= b.len(), "TGA: truncated");
                for _ in 0..count {
                    data.extend_from_slice(&b[at..at + px]);
                }
                at += px;
            } else {
                ensure!(at + count * px <= b.len(), "TGA: truncated");
                data.extend_from_slice(&b[at..at + count * px]);
                at += count * px;
            }
        }
        data.truncate(n);
    } else {
        ensure!(at + n <= b.len(), "TGA: truncated");
        data.extend_from_slice(&b[at..at + n]);
    }
    // BGR(A) -> RGB(A)
    if !grey {
        for p in data.chunks_exact_mut(px) {
            p.swap(0, 2);
        }
    }
    // rows are stored bottom-up unless the descriptor says top-down
    if desc & 0x20 == 0 {
        let row = w * px;
        let mut flipped = Vec::with_capacity(n);
        for y in (0..h).rev() {
            flipped.extend_from_slice(&data[y * row..(y + 1) * row]);
        }
        data = flipped;
    }
    let (w, h) = (w as u32, h as u32);
    Ok(match px {
        1 => DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, data).unwrap()),
        3 => DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, data).unwrap()),
        _ => DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, data).unwrap()),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn rle_bottom_up_bgr() {
        // 2x2, 24 bit, RLE: bottom row = 2x red (run), top row = blue, green (raw)
        let mut b = vec![0u8, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 24, 0];
        b.extend_from_slice(&[0x81, 0, 0, 255]);
        b.extend_from_slice(&[0x01, 255, 0, 0, 0, 255, 0]);
        let img = super::decode(&b).unwrap().to_rgb8();
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [0, 255, 0]);
        assert_eq!(img.get_pixel(0, 1).0, [255, 0, 0]);
        assert_eq!(img.get_pixel(1, 1).0, [255, 0, 0]);
    }
}
