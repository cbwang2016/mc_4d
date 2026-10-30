//! Beta-style water texture simulation, reconstructed from observable behavior
//! and constants in the verified Beta 1.5 `ul` (still) / `nl` (flowing) effects.
//! A 16x16 periodic field diffuses with damped random impulses at twenty ticks/s.
//! Original RGB/alpha ramps are retained; 4D changes only the surface projection.

struct Field { height: [f32; 256], velocity: [f32; 256], impulse: [f32; 256], flowing: bool }
impl Field {
    fn new(flowing: bool) -> Self { Self { height: [0.0; 256], velocity: [0.0; 256], impulse: [0.0; 256], flowing } }
    fn step(&mut self, rng: &mut u64) {
        let mut next = [0.0; 256];
        for y in 0..16 { for x in 0..16 {
            let i = x + y * 16;
            let neighborhood: f32 = if self.flowing {
                (0..3).map(|dy| self.height[x + ((y + 16 - dy) & 15) * 16]).sum()
            } else { (0..3).map(|dx| self.height[((x + 15 + dx) & 15) + y * 16]).sum() };
            next[i] = neighborhood / if self.flowing { 3.2 } else { 3.3 } + self.velocity[i] * 0.8;
            self.velocity[i] = (self.velocity[i] + self.impulse[i] * 0.05).max(0.0);
            self.impulse[i] -= if self.flowing { 0.3 } else { 0.1 };
            *rng ^= *rng << 13; *rng ^= *rng >> 7; *rng ^= *rng << 17;
            let random = (*rng >> 40) as f32 / (1u32 << 24) as f32;
            if random < if self.flowing { 0.2 } else { 0.05 } { self.impulse[i] = 0.5; }
        } }
        self.height = next;
    }
    fn pixels(&self, tick: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(1024);
        for i in 0..256 {
            let j = if self.flowing { (i + 256 - ((tick as usize * 16) & 255)) & 255 } else { i };
            let wave = self.height[j].clamp(0.0, 1.0).powi(2);
            out.extend_from_slice(&[(32.0 + wave * 32.0) as u8, (50.0 + wave * 64.0) as u8,
                255, (146.0 + wave * 50.0) as u8]);
        }
        out
    }
}

pub struct WaterFx { still: Field, flow: Field, rng: u64, tick: u32, last_time_tick: Option<u32> }
impl Default for WaterFx {
    fn default() -> Self {
        let mut fx = Self { still: Field::new(false), flow: Field::new(true), rng: 0xCAFE_1234_ABCD_5678, tick: 0, last_time_tick: None };
        // Warm the field once so the very first visible surface has ripples.
        for _ in 0..80 { fx.step(); }
        fx
    }
}
impl WaterFx {
    fn step(&mut self) { self.still.step(&mut self.rng); self.flow.step(&mut self.rng); self.tick = self.tick.wrapping_add(1); }
    pub fn update(&mut self, seconds: f32) -> Option<[Vec<u8>; 2]> {
        let now = (seconds.max(0.0) * 20.0) as u32;
        let steps = self.last_time_tick.map_or(0, |last| now.saturating_sub(last).min(4));
        let changed = self.last_time_tick.is_none() || steps > 0;
        self.last_time_tick = Some(now);
        for _ in 0..steps { self.step(); }
        changed.then(|| [self.still.pixels(self.tick), self.flow.pixels(self.tick)])
    }
    pub fn force_upload(&mut self) { self.last_time_tick = None; }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blue_alpha_ramp_is_classic_and_animation_is_clocked_and_repeatable() {
        let mut a = WaterFx::default(); let mut b = WaterFx::default();
        let first = a.update(0.0).unwrap();
        assert_eq!(first, b.update(0.0).unwrap());
        for tile in &first { for p in tile.chunks_exact(4) {
            assert!((32..=64).contains(&p[0]) && (50..=114).contains(&p[1]));
            assert_eq!(p[2], 255); assert!((146..=196).contains(&p[3]));
        } }
        assert!(a.update(0.01).is_none(), "render frame rate must not drive simulation speed");
        assert_ne!(a.update(0.05).unwrap(), first);
        assert!(a.update(0.05).is_none());
    }
}
