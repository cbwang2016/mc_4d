//! Block types. Ids are `u8`; id 0 is air.

pub type BlockId = u8;

pub const AIR: BlockId = 0;
pub const STONE: BlockId = 1;
pub const DIRT: BlockId = 2;
pub const GRASS: BlockId = 3;
pub const SAND: BlockId = 4;
pub const LOG: BlockId = 5;
pub const LEAVES: BlockId = 6;
pub const PLANKS: BlockId = 7;
pub const COBBLE: BlockId = 8;
pub const COAL_ORE: BlockId = 9;
pub const GLASS: BlockId = 10;
pub const BRICK: BlockId = 11;
pub const SNOW: BlockId = 12;
pub const WATER: BlockId = 13;
pub const BEDROCK: BlockId = 14;
pub const GLOW: BlockId = 15;

pub const BLOCK_COUNT: usize = 16;

pub struct BlockInfo {
    pub name: &'static str,
    /// Linear RGB base color (used by the shader palette and the UI).
    pub color: [f32; 3],
    pub solid: bool,
    pub breakable: bool,
}

pub const BLOCKS: [BlockInfo; BLOCK_COUNT] = [
    BlockInfo { name: "Air", color: [0.0, 0.0, 0.0], solid: false, breakable: false },
    BlockInfo { name: "Stone", color: [0.42, 0.42, 0.44], solid: true, breakable: true },
    BlockInfo { name: "Dirt", color: [0.45, 0.30, 0.18], solid: true, breakable: true },
    BlockInfo { name: "Grass", color: [0.33, 0.62, 0.22], solid: true, breakable: true },
    BlockInfo { name: "Sand", color: [0.86, 0.80, 0.56], solid: true, breakable: true },
    BlockInfo { name: "Log", color: [0.40, 0.29, 0.16], solid: true, breakable: true },
    BlockInfo { name: "Leaves", color: [0.20, 0.48, 0.16], solid: true, breakable: true },
    BlockInfo { name: "Planks", color: [0.70, 0.55, 0.33], solid: true, breakable: true },
    BlockInfo { name: "Cobblestone", color: [0.36, 0.36, 0.37], solid: true, breakable: true },
    BlockInfo { name: "Coal Ore", color: [0.30, 0.30, 0.31], solid: true, breakable: true },
    BlockInfo { name: "Glass", color: [0.75, 0.90, 0.95], solid: true, breakable: true },
    BlockInfo { name: "Brick", color: [0.62, 0.27, 0.20], solid: true, breakable: true },
    BlockInfo { name: "Snow", color: [0.93, 0.95, 0.97], solid: true, breakable: true },
    BlockInfo { name: "Water", color: [0.15, 0.32, 0.70], solid: false, breakable: false },
    BlockInfo { name: "Bedrock", color: [0.12, 0.12, 0.13], solid: true, breakable: false },
    BlockInfo { name: "Glowstone", color: [0.98, 0.85, 0.45], solid: true, breakable: true },
];

#[inline]
pub fn info(id: BlockId) -> &'static BlockInfo {
    &BLOCKS[(id as usize).min(BLOCK_COUNT - 1)]
}

#[inline]
pub fn is_solid(id: BlockId) -> bool {
    info(id).solid
}

/// Blocks the ray-caster stops at (everything except air).
#[inline]
pub fn is_visible(id: BlockId) -> bool {
    id != AIR
}
