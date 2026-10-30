//! Verified Beta1.5 species identity and collision sizes, shared by gameplay and visual fixtures.
use serde::{Deserialize, Serialize};
pub fn valid_slime_size(size: u8) -> u8 {
    match size {
        1 => 1,
        4 => 4,
        _ => 2,
    }
}
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CreatureKind {
    Gloom,
    Pig,
    Cow,
    Sheep,
    Chicken,
    Squid,
    Zombie,
    Skeleton,
    Spider,
    Creeper,
    Slime,
    Wolf,
}

impl CreatureKind {
    pub const ALL: [Self; 12] = [
        Self::Gloom,
        Self::Pig,
        Self::Cow,
        Self::Sheep,
        Self::Chicken,
        Self::Squid,
        Self::Zombie,
        Self::Skeleton,
        Self::Spider,
        Self::Creeper,
        Self::Slime,
        Self::Wolf,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Gloom => "Gloom",
            Self::Pig => "Pig",
            Self::Cow => "Cow",
            Self::Sheep => "Sheep",
            Self::Chicken => "Chicken",
            Self::Squid => "Squid",
            Self::Zombie => "Zombie",
            Self::Skeleton => "Skeleton",
            Self::Spider => "Spider",
            Self::Creeper => "Creeper",
            Self::Slime => "Slime",
            Self::Wolf => "Wolf",
        }
    }
    pub fn hostile(self) -> bool {
        matches!(
            self,
            Self::Zombie | Self::Skeleton | Self::Spider | Self::Creeper | Self::Slime
        )
    }
    /// Original rj.be/rj.bf constructor values; horizontal width also extends W.
    pub fn dimensions(self, slime_size: u8) -> (f32, f32) {
        match self {
            Self::Gloom => (1.0, 1.0),
            Self::Pig => (0.9, 0.9),
            Self::Cow | Self::Sheep => (0.9, 1.3),
            Self::Chicken => (0.3, 0.4),
            Self::Squid => (0.95, 0.95),
            Self::Zombie | Self::Skeleton | Self::Creeper => (0.6, 1.8),
            Self::Spider => (1.4, 0.9),
            Self::Wolf => (0.8, 0.8),
            Self::Slime => {
                let width = 0.6 * valid_slime_size(slime_size) as f32;
                (width, width)
            }
        }
    }
    /// Visual model scale is independent of the collision box. Native parts
    /// are measured in sixteenths of a block, with their baseline at pixel24.
    pub fn render_scale(self, slime_size: u8) -> f32 {
        match self {
            Self::Gloom => 0.5,
            Self::Slime => valid_slime_size(slime_size) as f32,
            _ => 1.0,
        }
    }
    pub fn health(self) -> i32 {
        match self {
            Self::Gloom => 3,
            Self::Pig | Self::Cow | Self::Sheep | Self::Squid => 10,
            Self::Wolf => 8,
            Self::Chicken => 4,
            Self::Slime => 4,
            _ => 20,
        }
    }
    pub fn color(self) -> [f32; 3] {
        match self {
            Self::Gloom => [0.5, 0.3, 0.9],
            Self::Pig => [0.9, 0.4, 0.45],
            Self::Cow => [0.4, 0.25, 0.13],
            Self::Sheep => [0.9, 0.9, 0.85],
            Self::Chicken => [0.95, 0.9, 0.7],
            Self::Squid => [0.15, 0.25, 0.4],
            Self::Zombie => [0.25, 0.5, 0.2],
            Self::Skeleton => [0.8, 0.8, 0.72],
            Self::Spider => [0.25, 0.1, 0.08],
            Self::Creeper => [0.3, 0.65, 0.18],
            Self::Slime => [0.4, 0.9, 0.3],
            Self::Wolf => [0.7, 0.7, 0.68],
        }
    }
}

/// Original dd.a(Random) natural fleece distribution (pink is conditional1/500).
pub fn natural_sheep_color(common: u32, rare: u32) -> u8 {
    match common % 100 {
        0..=4 => 15,
        5..=9 => 7,
        10..=14 => 8,
        15..=17 => 12,
        _ => {
            if rare % 500 == 0 {
                6
            } else {
                0
            }
        }
    }
}

#[cfg(test)]
mod sheep_tests {
    use super::*;
    #[test]
    fn natural_fleece_uses_original_common_and_conditional_pink_thresholds() {
        for (roll, color) in [
            (0, 15),
            (4, 15),
            (5, 7),
            (9, 7),
            (10, 8),
            (14, 8),
            (15, 12),
            (17, 12),
            (18, 0),
            (99, 0),
        ] {
            assert_eq!(natural_sheep_color(roll, 1), color);
        }
        assert_eq!(natural_sheep_color(18, 0), 6);
        assert_eq!(natural_sheep_color(17, 0), 12);
    }
}
