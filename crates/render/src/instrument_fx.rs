//! Original ar/jy compass needle and rotating clock dial, with20Hz damping.
use crate::atlas;
const COMPASS: usize = 54;
const CLOCK: usize = 70;
pub struct InstrumentFx {
    bases: [Vec<u8>; 2],
    dial: Vec<u8>,
    angle: [f64; 2],
    velocity: [f64; 2],
    tick: Option<u64>,
}
impl InstrumentFx {
    pub fn new(pixels: &[u8]) -> Self {
        let mut bases = [vec![0; 1024], vec![0; 1024]];
        for (out, tile) in bases.iter_mut().zip([COMPASS, CLOCK]) {
            for y in 0..16 {
                let src = (256 * 256 + (tile / 16 * 16 + y) * 256 + tile % 16 * 16) * 4;
                out[y * 64..y * 64 + 64].copy_from_slice(&pixels[src..src + 64]);
            }
        }
        let (_, _, dial) =
            atlas::decode(include_bytes!("../assets/minecraft-beta-1.5/misc/dial.png"))
                .expect("original dial");
        Self {
            bases,
            dial,
            angle: [0.0; 2],
            velocity: [0.0; 2],
            tick: None,
        }
    }
    pub fn update(&mut self, time: f32, compass: f32, celestial: f32) -> Option<[Vec<u8>; 2]> {
        let tick = (time.max(0.0) * 20.0).floor() as u64;
        if self.tick == Some(tick) {
            return None;
        }
        let steps = self
            .tick
            .map_or(1, |old| tick.saturating_sub(old).clamp(1, 20));
        self.tick = Some(tick);
        for _ in 0..steps {
            for (i, target) in [
                compass as f64,
                f64::from(-celestial * std::f32::consts::PI * 2.0),
            ]
            .into_iter()
            .enumerate()
            {
                let delta = (target - self.angle[i] + std::f64::consts::PI)
                    .rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI;
                self.velocity[i] = (self.velocity[i] + delta.clamp(-1.0, 1.0) * 0.1) * 0.8;
                self.angle[i] += self.velocity[i];
            }
        }
        let mut output = self.bases.clone();
        let (s, c) = self.angle[0].sin_cos();
        for j in -4..=4 {
            let x = (8.5 + c * j as f64 * 0.3) as usize;
            let y = (7.5 - s * j as f64 * 0.3 * 0.5) as usize;
            output[0][(y * 16 + x) * 4..(y * 16 + x) * 4 + 4]
                .copy_from_slice(&[100, 100, 100, 255]);
        }
        for j in -8..=16 {
            let x = (8.5 + s * j as f64 * 0.3) as usize;
            let y = (7.5 + c * j as f64 * 0.3 * 0.5) as usize;
            let color = if j >= 0 {
                [255, 20, 20, 255]
            } else {
                [100, 100, 100, 255]
            };
            output[0][(y * 16 + x) * 4..(y * 16 + x) * 4 + 4].copy_from_slice(&color);
        }
        let (s, c) = self.angle[1].sin_cos();
        for i in 0..256 {
            let p = &self.bases[1][i * 4..i * 4 + 4];
            if p[0] != p[2] || p[1] != 0 || p[2] == 0 {
                continue;
            }
            let x = -((i % 16) as f64) / 15.0 + 0.5;
            let y = (i / 16) as f64 / 15.0 - 0.5;
            let u = ((x * c + y * s + 0.5) * 16.0) as i32 & 15;
            let v = ((y * c - x * s + 0.5) * 16.0) as i32 & 15;
            let d = &self.dial[(v * 16 + u) as usize * 4..][..4];
            for k in 0..3 {
                output[1][i * 4 + k] = (d[k] as u32 * p[0] as u32 / 255) as u8;
            }
            output[1][i * 4 + 3] = d[3];
        }
        Some(output)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn needles_and_dial_respond_to_bearing_and_day_without_changing_casing() {
        let atlas = atlas::load(None).unwrap();
        let mut a = InstrumentFx::new(&atlas);
        let mut b = InstrumentFx::new(&atlas);
        let mut left = None;
        let mut right = None;
        for i in 0..100 {
            left = a.update(i as f32 / 20.0, 0.0, 0.0);
            right = b.update(i as f32 / 20.0, std::f32::consts::PI, 0.5);
        }
        let (left, right) = (left.unwrap(), right.unwrap());
        assert_ne!(left[0], right[0]);
        assert_ne!(left[1], right[1]);
        for i in 0..256 {
            let p = &a.bases[1][i * 4..i * 4 + 4];
            if !(p[0] == p[2] && p[1] == 0 && p[2] > 0) {
                assert_eq!(&right[1][i * 4..i * 4 + 4], p);
            }
        }
    }
}
