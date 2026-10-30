//! Saved world clock and deterministic weather. One day is 24,000 game ticks.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Environment {
    pub world_ticks: u64,
    pub raining: bool,
    pub thundering: bool,
    pub weather_ticks: u64,
    pub rain: f32,
    #[serde(skip)]
    pub lightning: f32,
    #[serde(skip)]
    pub lightning_events: u64,
    fraction: f32,
    rng: u64,
}

impl Default for Environment {
    fn default() -> Self {
        Self { world_ticks: 6000, raining: false, thundering: false, weather_ticks: 12000,
            rain: 0.0, lightning: 0.0, lightning_events:0, fraction: 0.0, rng: 0x1e3779b97f4a7c15 }
    }
}

impl Environment {
    pub fn seeded(seed: u32) -> Self { Self { rng: Self::default().rng ^ seed as u64, ..Self::default() } }
    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng &= i64::MAX as u64;
        self.rng
    }
    pub fn is_night(&self) -> bool { (13000..23000).contains(&(self.world_ticks % 24000)) }
    /// Original world-provider easing of the daily sun angle (zero at noon).
    pub fn celestial_angle(&self) -> f32 {
        let phase=(((self.world_ticks%24000)as f32+self.fraction)/24000.0-0.25).rem_euclid(1.0);
        let eased=1.0-(((phase as f64*std::f64::consts::PI).cos()+1.0)/2.0)as f32;
        phase+(eased-phase)/3.0
    }
    pub fn daylight(&self) -> f32 {
        (self.celestial_angle().mul_add(std::f32::consts::TAU,0.0).cos()*2.0+0.5).clamp(0.0,1.0)
    }
    pub fn sun(&self) -> glam::Vec4 {
        let angle=self.celestial_angle()*std::f32::consts::TAU;
        glam::Vec4::new(0.0,angle.cos(),angle.sin(),0.0)
    }
    pub fn sleep_until_morning(&mut self) {
        self.world_ticks = (self.world_ticks / 24000 + 1) * 24000;
        self.raining = false;
        self.thundering = false;
        self.weather_ticks = 12000;
    }
    pub fn update(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
        self.rain = (self.rain + if self.raining { dt / 5.0 } else { -dt / 5.0 }).clamp(0.0, 1.0);
        self.lightning = (self.lightning - dt * 4.0).max(0.0);
        self.fraction += dt * 20.0;
        let ticks = self.fraction.floor() as u64;
        self.fraction -= ticks as f32;
        self.world_ticks = self.world_ticks.saturating_add(ticks);
        self.weather_ticks = self.weather_ticks.saturating_sub(ticks);
        if self.weather_ticks == 0 {
            self.raining = !self.raining;
            self.thundering = self.raining && self.random() % 4 == 0;
            self.weather_ticks = if self.raining { 12000 + self.random() % 12000 } else { 12000 + self.random() % 168000 };
        }
        if self.thundering && ticks > 0 && self.random() % 600 < ticks { self.lightning_events=self.lightning_events.wrapping_add(1); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn celestial_angle_matches_verified_provider_samples() {
        let mut e=Environment::default();
        for (tick,angle) in [(0,0.7845178_f32),(6000,0.0),(12000,0.21548219),(18000,0.5),(23999,0.7844746)] {
            e.world_ticks=tick;assert!((e.celestial_angle()-angle).abs()<1e-7);
            assert!((e.sun().length()-1.0).abs()<1e-6);
        }
    }
    #[test]
    fn clock_weather_survive_save_and_sleep() {
        let mut e = Environment::seeded(7);
        assert!(e.daylight() > 0.99);
        e.world_ticks = 18000;
        assert!(e.is_night());
        assert!(e.daylight() < 0.01);
        e.weather_ticks = 1;
        e.update(0.1);
        assert!(e.raining);
        let mut restored: Environment = toml::from_str(&toml::to_string(&e).unwrap()).unwrap();
        for _ in 0..50 { e.update(0.1); restored.update(0.1); }
        assert_eq!(e.world_ticks, restored.world_ticks);
        assert_eq!(e.weather_ticks, restored.weather_ticks);
        restored.sleep_until_morning();
        assert_eq!(restored.world_ticks % 24000, 0);
        assert!(!restored.raining);
    }
}
