//! Engine-independent core of mc_4d: math, world data, terrain, ray-casting and physics.

pub mod block;
pub mod environment;
pub mod explosion;
pub mod entities;
pub mod item;
pub mod loot;
pub mod farming;
pub mod crafting;
pub mod containers;
pub mod camera;
pub mod chunk;
pub mod fluid;
pub mod fire;
pub mod physics;
pub mod shapes;
pub mod raycast;
pub mod rails;
pub mod redstone;
pub mod save;
pub mod terrain;
pub mod world;
pub mod lighting;
pub mod views;

pub use glam;

pub mod ecology;

pub mod paintings;

pub mod dungeon;

/// Fraction of wall time each background worker (generation, lighting) may
/// compute. Integrated GPUs share package power with the CPU; saturated
/// workers measurably slowed ray casting threefold. `MC4D_BACKGROUND_DUTY`
/// (0..=1) overrides it for measurements.
///
/// Throttling applies only while the frontend reports an interactive, visible
/// world; loading screens run the workers at full speed.
/// With the 20 FPS budget (50 ms) full-speed workers measured at 0.03% slow
/// frames while settling a third more visible lighting; 0.5 was needed only
/// under the earlier 30 FPS target.
const DEFAULT_DUTY: f32 = 1.0;

pub fn background_duty() -> f32 {
    if !BACKGROUND_THROTTLE.load(std::sync::atomic::Ordering::Relaxed) { return 1.0; }
    static DUTY: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *DUTY.get_or_init(|| {
        std::env::var("MC4D_BACKGROUND_DUTY").ok().and_then(|v| v.parse::<f32>().ok())
            .filter(|d| *d > 0.0 && *d <= 1.0).unwrap_or(DEFAULT_DUTY)
    })
}

static BACKGROUND_THROTTLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called by the frontend: true while gameplay frames are being presented.
pub fn set_background_throttle(on: bool) {
    BACKGROUND_THROTTLE.store(on, std::sync::atomic::Ordering::Relaxed);
}
