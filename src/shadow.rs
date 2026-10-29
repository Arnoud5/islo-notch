//! The island's soft shadow, drawn once at startup.
//!
//! Slint's software renderer can't blur, so we make the blurred shadow
//! ourselves: a rounded rectangle, Gaussian-blurred, with a little noise
//! ("dithering") so the fade has no visible steps (banding). Slint then
//! stretches it around the island as a nine-slice image: the four corners are
//! drawn 1:1 and the 2 px straight middle is stretched to the island's size.
//!
//! Cost: two small images (62 px and 94 px square, ~50 KB together), made
//! once and shared by every island window.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// How far the shadow reaches past the island, in px. Must match `sh-pad` in island.slint.
pub const PAD: usize = 16;
/// Blur radius (Gaussian sigma), in px.
const SIGMA: f32 = 5.0;
/// Darkness straight under the island (alpha 0..1). The island's edge gets about half.
const STRENGTH: f32 = 0.42;
/// Samples per pixel along each axis, for a smooth rounded corner.
const SS: usize = 4;

thread_local! {
    // `thread_local!` = one copy per thread; the UI runs on one thread, so this
    // is simply "make the images the first time someone asks, then reuse them".
    static CACHE: (Image, Image) = (make(14), make(30));
}

/// The (compact-corner, open-corner) shadow images: corner radius 14 and 30 px.
pub fn images() -> (Image, Image) {
    // Image is reference-counted, so clone() is cheap (no pixel copy).
    CACHE.with(|c| (c.0.clone(), c.1.clone()))
}

/// The shadow of a rounded rectangle with corner radius `r` px.
fn make(r: usize) -> Image {
    let alpha = alpha_map(r, 0x15_1A0 + r as u64);
    let size = 2 * (PAD + r) + 2;
    let mut buf = SharedPixelBuffer::<Rgba8Pixel>::new(size as u32, size as u32);
    for (px, &a) in buf.make_mut_slice().iter_mut().zip(&alpha) {
        // black, premultiplied: the colour channels stay 0
        *px = Rgba8Pixel { r: 0, g: 0, b: 0, a };
    }
    let mut img = Image::from_rgba8_premultiplied(buf);
    let edge = (PAD + r) as u16;
    img.set_nine_slice_edges(edge, edge, edge, edge);
    img
}

/// The shadow's alpha values, row by row (size x size, size = 2 * (PAD + r) + 2).
fn alpha_map(r: usize, seed: u64) -> Vec<u8> {
    let size = 2 * (PAD + r) + 2;

    // 1. Coverage of the rounded rect [PAD, size - PAD] (0..1 per pixel).
    let (lo, hi) = ((PAD + r) as f32, (size - PAD - r) as f32);
    let mut cover = vec![0f32; size * size];
    for y in 0..size {
        for x in 0..size {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let fy = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    let dx = (lo - fx).max(fx - hi).max(0.0);
                    let dy = (lo - fy).max(fy - hi).max(0.0);
                    if dx * dx + dy * dy <= (r * r) as f32 {
                        hits += 1;
                    }
                }
            }
            cover[y * size + x] = hits as f32 / (SS * SS) as f32;
        }
    }

    // 2. Gaussian blur, first across then down ("separable": two cheap 1-D passes
    //    instead of one expensive 2-D one). Outside the image counts as empty.
    let kernel: Vec<f32> = {
        let k: Vec<f32> = (-(PAD as i32)..=PAD as i32)
            .map(|i| (-((i * i) as f32) / (2.0 * SIGMA * SIGMA)).exp())
            .collect();
        let sum: f32 = k.iter().sum();
        k.into_iter().map(|v| v / sum).collect()
    };
    let blur = |src: &[f32], step: (usize, usize)| -> Vec<f32> {
        // step = (dx, dy): which direction this pass blurs in
        let mut out = vec![0f32; size * size];
        for y in 0..size {
            for x in 0..size {
                let mut acc = 0.0;
                for (j, w) in kernel.iter().enumerate() {
                    let o = j as i32 - PAD as i32;
                    let (sx, sy) = (x as i32 + o * step.0 as i32, y as i32 + o * step.1 as i32);
                    if sx >= 0 && sy >= 0 && (sx as usize) < size && (sy as usize) < size {
                        acc += w * src[sy as usize * size + sx as usize];
                    }
                }
                out[y * size + x] = acc;
            }
        }
        out
    };
    let blurred = blur(&blur(&cover, (1, 0)), (0, 1));

    // 3. Quantise to 8 bits with a tiny triangular dither (+-1 step), so the
    //    fade never shows steps. A small xorshift PRNG keeps it dependency-free
    //    and the same on every run.
    let mut state = seed | 1;
    let mut rand = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 24) as f32 // 0..1
    };
    blurred
        .iter()
        .map(|&v| {
            let a = v * STRENGTH * 255.0;
            // no noise in the empty margin: those pixels must stay exactly 0 so
            // clicks pass through them
            let noise = if a > 1.0 { rand() - rand() } else { 0.0 };
            (a + noise).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_and_click_through() {
        for r in [14, 30] {
            let size = 2 * (PAD + r) + 2;
            let a = alpha_map(r, 7);
            // the outermost pixels are fully transparent (click-through)
            assert!(a[..size].iter().all(|&v| v == 0), "top row should be empty");
            // down the middle column the fade rises gently: no jumps (no banding)
            let col: Vec<i32> = (0..PAD + 4).map(|y| a[y * size + size / 2] as i32).collect();
            for w in col.windows(2) {
                assert!((w[1] - w[0]).abs() <= 12, "step too big: {col:?}");
            }
            // about 20% dark at the island's edge
            let edge = col[PAD];
            assert!((40..=65).contains(&edge), "edge alpha {edge}");
        }
    }
}
