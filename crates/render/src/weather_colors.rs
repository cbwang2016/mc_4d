//! Beta's rain vertex brightness baked into sixteen texture layers. This keeps
//! original pack pixels/alpha and avoids repeated sRGB powers inside every ray.
use crate::atlas;
pub const WIDTH: u32 = 64;
pub const HEIGHT: u32 = 256;
pub const LEVELS: u32 = 16;
pub fn rain_layers(atlas_pixels: &[u8]) -> Vec<[f32; 4]> {
    let mut result = Vec::with_capacity((WIDTH * HEIGHT * LEVELS) as usize);
    for level in 0..LEVELS {
        let light = level as f32 / 15.0;
        let brightness = (light / ((1.0 - light) * 3.0 + 1.0) * 0.95 + 0.05) * 0.85 + 0.15;
        let mut channel = [0.0; 256];
        for (value, out) in channel.iter_mut().enumerate() {
            let encoded = value as f32 / 255.0 * brightness;
            *out = if encoded <= 0.04045 {
                encoded / 12.92
            } else {
                ((encoded + 0.055) / 1.055).powf(2.4)
            };
        }
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let offset = ((16 * atlas::SIDE * atlas::SIDE + y * atlas::SIDE + x) * 4) as usize;
                let p = &atlas_pixels[offset..offset + 4];
                result.push([
                    channel[p[0] as usize],
                    channel[p[1] as usize],
                    channel[p[2] as usize],
                    p[3] as f32 / 255.0,
                ]);
            }
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_alpha_and_pack_colors_survive_all_brightness_levels() {
        let mut atlas = vec![0; (atlas::SIDE * atlas::SIDE * atlas::LAYERS * 4) as usize];
        let offset = (16 * atlas::SIDE * atlas::SIDE * 4) as usize;
        atlas[offset..offset + 4].copy_from_slice(&[70, 103, 195, 80]);
        let layers = rain_layers(&atlas);
        let stride = (WIDTH * HEIGHT) as usize;
        for level in 0..16 {
            let p = layers[level * stride];
            assert_eq!(p[3], 80.0 / 255.0);
            assert!(p.iter().all(|v| v.is_finite()));
            if level > 0 {
                assert!(p[2] > layers[(level - 1) * stride][2]);
            }
        }
        let daylight = layers[15 * stride];
        let expected = ((195.0_f32 / 255.0 + 0.055) / 1.055).powf(2.4);
        assert!((daylight[2] - expected).abs() < 1e-6);
        assert_eq!(layers[1], [0.0; 4]);
    }
}
