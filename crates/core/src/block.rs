//! Block types. Ids are `u8`; id 0 is air.

pub type BlockId = u8;

/// Stable byte block identity plus independent state metadata. Ordinary blocks
/// use metadata zero; orientation, growth, color and signal state are explicit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlockState {
    pub id: BlockId,
    pub metadata: u8,
}

impl BlockState {
    pub const fn new(id: BlockId, metadata: u8) -> Self { Self { id, metadata } }
}

impl From<BlockId> for BlockState {
    fn from(id: BlockId) -> Self { Self::new(id, 0) }
}

pub const AIR: BlockId = 0;
pub const STONE: BlockId = 1;
pub const GRASS: BlockId = 3;
pub const DIRT: BlockId = 2;
pub const COBBLE: BlockId = 8;
pub const PLANKS: BlockId = 7;
pub const SAPLING: BlockId = 26;
pub const BEDROCK: BlockId = 14;
pub const WATER: BlockId = 13;
pub const LAVA: BlockId = 16;
pub const SAND: BlockId = 4;
pub const GRAVEL: BlockId = 27;
pub const GOLD_ORE: BlockId = 22;
pub const IRON_ORE: BlockId = 21;
pub const COAL_ORE: BlockId = 9;
pub const LOG: BlockId = 5;
pub const LEAVES: BlockId = 6;
pub const SPONGE: BlockId = 28;
pub const GLASS: BlockId = 10;
pub const LAPIS_ORE: BlockId = 29;
pub const LAPIS_BLOCK: BlockId = 30;
pub const DISPENSER: BlockId = 31;
pub const SANDSTONE: BlockId = 48;
pub const NOTE_BLOCK: BlockId = 49;
pub const BED: BlockId = 50;
pub const POWERED_RAIL: BlockId = 51;
pub const DETECTOR_RAIL: BlockId = 52;
pub const WEB: BlockId = 53;
pub const WOOL: BlockId = 25;
pub const DANDELION: BlockId = 54;
pub const ROSE: BlockId = 55;
pub const BROWN_MUSHROOM: BlockId = 56;
pub const RED_MUSHROOM: BlockId = 57;
pub const GOLD_BLOCK: BlockId = 58;
pub const IRON_BLOCK: BlockId = 59;
pub const DOUBLE_SLAB: BlockId = 60;
pub const SLAB: BlockId = 61;
pub const BRICK: BlockId = 11;
pub const TNT: BlockId = 62;
pub const BOOKSHELF: BlockId = 63;
pub const MOSS_STONE: BlockId = 64;
pub const OBSIDIAN: BlockId = 17;
pub const TORCH: BlockId = 65;
pub const FIRE: BlockId = 66;
pub const SPAWNER: BlockId = 67;
pub const WOOD_STAIRS: BlockId = 68;
pub const CHEST: BlockId = 19;
pub const REDSTONE_WIRE: BlockId = 69;
pub const DIAMOND_ORE: BlockId = 23;
pub const DIAMOND_BLOCK: BlockId = 70;
pub const CRAFTING_TABLE: BlockId = 18;
pub const CROPS: BlockId = 71;
pub const FARMLAND: BlockId = 24;
pub const FURNACE: BlockId = 20;
pub const LIT_FURNACE: BlockId = 72;
pub const SIGN_POST: BlockId = 73;
pub const WOOD_DOOR: BlockId = 74;
pub const LADDER: BlockId = 75;
pub const RAIL: BlockId = 76;
pub const STONE_STAIRS: BlockId = 77;
pub const WALL_SIGN: BlockId = 78;
pub const LEVER: BlockId = 79;
pub const STONE_PRESSURE_PLATE: BlockId = 80;
pub const IRON_DOOR: BlockId = 81;
pub const WOOD_PRESSURE_PLATE: BlockId = 82;
pub const REDSTONE_ORE: BlockId = 83;
pub const LIT_REDSTONE_ORE: BlockId = 84;
pub const REDSTONE_TORCH_OFF: BlockId = 85;
pub const REDSTONE_TORCH: BlockId = 86;
pub const BUTTON: BlockId = 87;
pub const SNOW_LAYER: BlockId = 88;
pub const ICE: BlockId = 89;
pub const SNOW: BlockId = 12;
pub const CACTUS: BlockId = 90;
pub const CLAY: BlockId = 91;
pub const REEDS: BlockId = 92;
pub const JUKEBOX: BlockId = 93;
pub const FENCE: BlockId = 94;
pub const PUMPKIN: BlockId = 95;
pub const NETHERRACK: BlockId = 96;
pub const SOUL_SAND: BlockId = 97;
pub const GLOW: BlockId = 15;
pub const PORTAL: BlockId = 98;
pub const JACK_O_LANTERN: BlockId = 99;
pub const CAKE: BlockId = 100;
pub const REPEATER_OFF: BlockId = 101;
pub const REPEATER: BlockId = 102;
pub const LOCKED_CHEST: BlockId = 103;

/// Internal flowing states preserve the byte-per-block chunk format. Source water
/// remains ID 13, so existing terrain and saves retain their original meaning.
pub const WATER_FLOW_START: BlockId = 32;
pub const LAVA_FLOW_START: BlockId = 40;
pub const BLOCK_COUNT: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidKind { Water, Lava }

impl FluidKind {
    pub fn source(self) -> BlockId { match self { Self::Water => WATER, Self::Lava => LAVA } }
    pub fn tick_delay(self) -> u64 { match self { Self::Water => 5, Self::Lava => 30 } }
    pub fn horizontal_decay(self) -> u8 { match self { Self::Water => 1, Self::Lava => 2 } }
    pub fn flowing(self, level: u8) -> BlockId {
        debug_assert!((1..=8).contains(&level));
        (match self { Self::Water => WATER_FLOW_START, Self::Lava => LAVA_FLOW_START }) + level - 1
    }
}

/// Level 0 is an explicit source, 1..7 is horizontal flow, 8 is falling.
pub fn fluid(id: BlockId) -> Option<(FluidKind, u8)> {
    match id {
        WATER => Some((FluidKind::Water, 0)),
        LAVA => Some((FluidKind::Lava, 0)),
        WATER_FLOW_START..=39 => Some((FluidKind::Water, id - WATER_FLOW_START + 1)),
        LAVA_FLOW_START..=47 => Some((FluidKind::Lava, id - LAVA_FLOW_START + 1)),
        _ => None,
    }
}

pub fn is_water(id: BlockId) -> bool { fluid(id).is_some_and(|(kind, _)| kind == FluidKind::Water) }
pub fn is_fluid(id: BlockId) -> bool { fluid(id).is_some() }

pub fn material_id(id: BlockId) -> BlockId { fluid(id).map_or(id, |(kind, _)| kind.source()) }


#[derive(Clone, Copy, Debug)]
pub struct BlockInfo {
    pub name: &'static str,
    /// Linear RGB base color (used by the shader palette and the UI).
    pub color: [f32; 3],
    pub solid: bool,
    pub breakable: bool,
}

#[path = "block_registry.rs"]
mod registry;
pub use registry::ALL_BLOCKS;
pub const BLOCKS: [BlockInfo; BLOCK_COUNT] = registry::block_infos();
pub const DEFINITIONS: [BlockDefinition; BLOCK_COUNT] = registry::block_definitions();

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum BlockShape { Cube, Fluid, Plant, Bed, Rail, Web, Slab, Torch, Fire, Stairs, Wire, Sign, Door, Ladder, Lever, PressurePlate, Button, SnowLayer, Cactus, Fence, Portal, Cake, Repeater }
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum StateSchema { None, Sapling, Species, Leaves, Facing, Bed, PoweredRail, Color, Slab, Torch, Age, Power, Growth, Moisture, Rotation, Door, Rail, Switch, Pressed, Layers, Record, Bites, Repeater }
#[derive(Clone,Copy,Debug)]
pub struct BlockDefinition {
 pub registered: bool,
 pub requires_tool: bool,
 pub opaque: bool,
 pub light_opacity: u8,
 pub light_emission: u8,
 pub reference_id: u8,
 pub shape: BlockShape,
 pub state: StateSchema,
 pub hardness: f32,
 pub resistance: f32,
 pub inventory_item: bool,
 /// Original faces: bottom, top, north, south, west, east; indexed by metadata.
 pub tiles: [[i16;6];16],
}
impl BlockDefinition {
 const UNKNOWN: Self = Self { registered:false, requires_tool:false,opaque:false,light_opacity:0,light_emission:0,reference_id:0, shape:BlockShape::Cube,state:StateSchema::None,hardness:-1.0,resistance:0.0,inventory_item:false,tiles:[[0;6];16] };
}
pub fn definition(id: BlockId) -> &'static BlockDefinition { &DEFINITIONS[material_id(id) as usize] }
pub fn registered(id: BlockId) -> bool { id == AIR || definition(id).registered }
pub fn from_reference_id(id: u16) -> Option<BlockId> {
 let mapped=*registry::REFERENCE_TO_BLOCK.get(id as usize)?;
 u8::try_from(mapped).ok()
}
pub fn texture_tile(state: BlockState, face: usize) -> u16 { definition(state.id).tiles[(state.metadata & 15) as usize][face.min(5)].unsigned_abs() }
pub fn texture_mirrored(state: BlockState, face: usize) -> bool { definition(state.id).tiles[(state.metadata & 15) as usize][face.min(5)] < 0 }
/// Verified from Beta1.5 to.q and to.s; legacy fluid state IDs normalize to source material.
pub fn light_opacity(state: BlockState) -> u8 { definition(state.id).light_opacity }
pub fn light_emission(state: BlockState) -> u8 { definition(state.id).light_emission }

#[inline]
pub fn info(id: BlockId) -> &'static BlockInfo {
    &BLOCKS[material_id(id) as usize]
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

/// Fluid surface within a block, matching renderer traversal and body hazards.
/// Continuous columns have no artificial air slit between adjacent cells.
pub fn fluid_height(id: BlockId, above: Option<BlockId>) -> f32 {
    let Some((kind, level)) = fluid(id) else { return 0.0 };
    if level == 8 || above.and_then(fluid).is_some_and(|(other, _)| other == kind) { 1.0 }
    else { (8 - level) as f32 / 9.0 }
}
