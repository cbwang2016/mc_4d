//! Canonical inventory identity is independent of byte-sized world block IDs.
//! Item-only IDs use the original Beta IDs >=256. Variant metadata is independent
//! of durability, and old prototype stacks migrate explicitly at the save boundary.
use crate::block::{self, BlockId, BlockInfo, BlockState};
use serde::{Deserialize, Serialize};
pub type ItemId = u16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stack {
    /// Historical field name retained in TOML for compatibility.
    pub block: ItemId,
    pub count: u32,
    #[serde(default)] pub damage: u16,
    #[serde(default)] pub variant: u8,
}
impl Stack {
    pub const EMPTY: Self = Self { block:0,count:0,damage:0,variant:0 };
    pub fn new(block: impl Into<ItemId>, count:u32) -> Self { Self { block:block.into(),count,damage:0,variant:0 } }
    pub fn with_variant(block: impl Into<ItemId>, count:u32, variant:u8) -> Self { Self {block:block.into(),count,damage:0,variant} }
    pub fn same_kind(self, other:Self) -> bool { self.block==other.block && self.variant==other.variant && self.damage==other.damage }
    pub fn normalized(self) -> Option<Self> {
        let durability=max_durability(self.block);
        (valid(self.block) && self.count>0 && valid_variant(self.block,self.variant) && (durability==0 || self.damage<=durability)).then(||Self {
            count:self.count.min(stack_limit(self.block)),damage:if durability==0 {0}else{self.damage},..self
        })
    }
}

#[derive(Clone,Copy,Debug)]
pub struct ItemDefinition {
    pub harvest_blocks: [u64;4],
    pub efficient_blocks: [u64;4],
    pub efficiency: f32,
    pub attack_damage: i32,
    pub id: ItemId,
    pub display: BlockInfo,
    pub max_stack: u32,
    pub max_damage: u16,
    pub subtypes: bool,
    pub container: Option<ItemId>,
    pub names: [&'static str;16],
    pub tiles: [u16;16],
}
#[path="item_registry.rs"] mod registry;
pub use registry::ALL_ITEMS;
pub fn definition(id:ItemId)->Option<&'static ItemDefinition> {
    registry::ITEM_DEFINITIONS.binary_search_by_key(&id,|d|d.id).ok().map(|i|&registry::ITEM_DEFINITIONS[i])
}

pub const STICK: ItemId = 280;
pub const COAL: ItemId = 263;
pub const CHARCOAL: ItemId = 263;
pub const IRON_INGOT: ItemId = 265;
pub const GOLD_INGOT: ItemId = 266;
pub const CLAY_BALL: ItemId = 337;
pub const BRICK_ITEM: ItemId = 336;
pub const RAW_PORK: ItemId = 319;
pub const COOKED_PORK: ItemId = 320;
pub const RAW_FISH: ItemId = 349;
pub const COOKED_FISH: ItemId = 350;
pub const BUCKET: ItemId = 325;
pub const WATER_BUCKET: ItemId = 326;
pub const LAVA_BUCKET: ItemId = 327;
pub const WHEAT: ItemId = 296;
pub const BREAD: ItemId = 297;
pub const SUGAR_CANE: ItemId = 338;
pub const PAPER: ItemId = 339;
pub const BOOK: ItemId = 340;
pub const BONE: ItemId = 352;
pub const BONE_MEAL: ItemId = 351;
pub const SUGAR: ItemId = 353;
pub const DIAMOND: ItemId = 264;
pub const LEATHER: ItemId = 334;
pub const FEATHER: ItemId = 288;
pub const ARROW: ItemId = 262;
pub const STRING: ItemId = 287;
pub const GUNPOWDER: ItemId = 289;
pub const SLIME_BALL: ItemId = 341;
pub const INK_SAC: ItemId = 351;
pub const EGG: ItemId = 344;
pub const SEEDS: ItemId = 295;
pub const LEATHER_HELMET: ItemId = 298;
pub const LEATHER_CHESTPLATE: ItemId = 299;
pub const LEATHER_LEGGINGS: ItemId = 300;
pub const LEATHER_BOOTS: ItemId = 301;
pub const CHAIN_HELMET: ItemId = 302;
pub const CHAIN_CHESTPLATE: ItemId = 303;
pub const CHAIN_LEGGINGS: ItemId = 304;
pub const CHAIN_BOOTS: ItemId = 305;
pub const IRON_HELMET: ItemId = 306;
pub const IRON_CHESTPLATE: ItemId = 307;
pub const IRON_LEGGINGS: ItemId = 308;
pub const IRON_BOOTS: ItemId = 309;
pub const GOLD_HELMET: ItemId = 314;
pub const GOLD_CHESTPLATE: ItemId = 315;
pub const GOLD_LEGGINGS: ItemId = 316;
pub const GOLD_BOOTS: ItemId = 317;
pub const DIAMOND_HELMET: ItemId = 310;
pub const DIAMOND_CHESTPLATE: ItemId = 311;
pub const DIAMOND_LEGGINGS: ItemId = 312;
pub const DIAMOND_BOOTS: ItemId = 313;
pub const WOOD_PICKAXE: ItemId = 270;
pub const WOOD_SHOVEL: ItemId = 269;
pub const WOOD_AXE: ItemId = 271;
pub const WOOD_HOE: ItemId = 290;
pub const WOOD_SWORD: ItemId = 268;
pub const STONE_PICKAXE: ItemId = 274;
pub const STONE_SHOVEL: ItemId = 273;
pub const STONE_AXE: ItemId = 275;
pub const STONE_HOE: ItemId = 291;
pub const STONE_SWORD: ItemId = 272;
pub const IRON_PICKAXE: ItemId = 257;
pub const IRON_SHOVEL: ItemId = 256;
pub const IRON_AXE: ItemId = 258;
pub const IRON_HOE: ItemId = 292;
pub const IRON_SWORD: ItemId = 267;
pub const GOLD_PICKAXE: ItemId = 285;
pub const GOLD_SHOVEL: ItemId = 284;
pub const GOLD_AXE: ItemId = 286;
pub const GOLD_HOE: ItemId = 294;
pub const GOLD_SWORD: ItemId = 283;
pub const DIAMOND_PICKAXE: ItemId = 278;
pub const DIAMOND_SHOVEL: ItemId = 277;
pub const DIAMOND_AXE: ItemId = 279;
pub const DIAMOND_HOE: ItemId = 293;
pub const DIAMOND_SWORD: ItemId = 276;
pub const FLINT_AND_STEEL: ItemId = 259;
pub const APPLE: ItemId = 260;
pub const BOW: ItemId = 261;
pub const BOWL: ItemId = 281;
pub const MUSHROOM_STEW: ItemId = 282;
pub const FLINT: ItemId = 318;
pub const PAINTING: ItemId = 321;
pub const GOLDEN_APPLE: ItemId = 322;
pub const SIGN: ItemId = 323;
pub const WOOD_DOOR_ITEM: ItemId = 324;
pub const MINECART: ItemId = 328;
pub const SADDLE: ItemId = 329;
pub const IRON_DOOR_ITEM: ItemId = 330;
pub const REDSTONE: ItemId = 331;
pub const SNOWBALL: ItemId = 332;
pub const BOAT: ItemId = 333;
pub const MILK: ItemId = 335;
pub const CHEST_MINECART: ItemId = 342;
pub const FURNACE_MINECART: ItemId = 343;
pub const COMPASS: ItemId = 345;
pub const FISHING_ROD: ItemId = 346;
pub const CLOCK: ItemId = 347;
pub const GLOWSTONE_DUST: ItemId = 348;
pub const DYE: ItemId = 351;
pub const CAKE_ITEM: ItemId = 354;
pub const BED_ITEM: ItemId = 355;
pub const REPEATER_ITEM: ItemId = 356;
pub const COOKIE: ItemId = 357;
pub const RECORD_13: ItemId = 2256;
pub const RECORD_CAT: ItemId = 2257;
// These compatibility names identify the dye/coal family. Use variants15/1.
pub const BONE_MEAL_VARIANT:u8=15;
pub const CHARCOAL_VARIANT:u8=1;

pub fn placed_block(id:ItemId)->Option<BlockId> {
    if let Ok(id)=BlockId::try_from(id) { return block::ALL_BLOCKS.contains(&id).then_some(id); }
    Some(match id {
        SIGN=>block::SIGN_POST,WOOD_DOOR_ITEM=>block::WOOD_DOOR,IRON_DOOR_ITEM=>block::IRON_DOOR,
        BED_ITEM=>block::BED,CAKE_ITEM=>block::CAKE,REPEATER_ITEM=>block::REPEATER_OFF,
        REDSTONE=>block::REDSTONE_WIRE,SEEDS=>block::CROPS,SUGAR_CANE=>block::REEDS,
        _=>return None,
    })
}
pub fn placement(stack:Stack)->Option<BlockState> {
    let id=placed_block(stack.block)?;
    let metadata=if matches!(id,block::FURNACE|block::LIT_FURNACE|block::DISPENSER|block::CHEST) {3}else{stack.variant};
    Some(BlockState::new(id,metadata))
}
pub fn from_block_state(state:BlockState)->Stack {
    let id=match state.id {
        block::BED=>BED_ITEM,block::CAKE=>CAKE_ITEM,block::SIGN_POST|block::WALL_SIGN=>SIGN,
        block::WOOD_DOOR=>WOOD_DOOR_ITEM,block::IRON_DOOR=>IRON_DOOR_ITEM,
        block::REDSTONE_WIRE=>REDSTONE,block::REPEATER_OFF|block::REPEATER=>REPEATER_ITEM,
        block::CROPS=>SEEDS,block::REEDS=>SUGAR_CANE,block::LIT_FURNACE=>block::FURNACE.into(),
        block::DOUBLE_SLAB=>block::SLAB.into(),_=>state.id.into(),
    };
    let variant=match state.id {
        block::WOOL=>state.metadata&15,block::LOG|block::LEAVES|block::SAPLING=>state.metadata&3,
        block::SLAB|block::DOUBLE_SLAB=>state.metadata&3,_=>0,
    };
    Stack::with_variant(id,1,variant)
}
pub fn is_placeable(id:ItemId)->bool { placed_block(id).is_some() }
pub fn valid(id:ItemId)->bool {
    definition(id).is_some() || BlockId::try_from(id).ok().is_some_and(|id|block::ALL_BLOCKS.contains(&id))
}
pub fn info(id:impl Into<ItemId>)->&'static BlockInfo {
    let id=id.into();
    if let Some(d)=definition(id) { &d.display }
    else if let Some(id)=placed_block(id) { block::info(id) }
    else { block::info(block::AIR) }
}
pub fn stack_name(stack:Stack)->&'static str {
    if let Some(d)=definition(stack.block) { return d.names[(stack.variant&15) as usize]; }
    let Some(id)=placed_block(stack.block) else{return "Unknown"};
    if id==block::WOOL { return ["Wool","Orange Wool","Magenta Wool","Light Blue Wool","Yellow Wool","Lime Wool","Pink Wool","Gray Wool","Light Gray Wool","Cyan Wool","Purple Wool","Blue Wool","Brown Wool","Green Wool","Red Wool","Black Wool"][(stack.variant&15) as usize]; }
    if id==block::LOG { return ["Wood","Spruce Wood","Birch Wood"][(stack.variant as usize).min(2)]; }
    if id==block::SAPLING { return ["Sapling","Spruce Sapling","Birch Sapling"][(stack.variant as usize).min(2)]; }
    if id==block::LEAVES { return ["Leaves","Spruce Leaves","Birch Leaves"][(stack.variant as usize).min(2)]; }
    if matches!(id,block::SLAB|block::DOUBLE_SLAB) { return ["Stone Slab","Sandstone Slab","Wooden Slab","Cobblestone Slab"][(stack.variant&3) as usize]; }
    block::info(id).name
}
pub fn stack_limit(id:ItemId)->u32 { definition(id).map_or(64,|d|d.max_stack) }
/// Original maximum damage counter. Beta ii.a breaks only when this is exceeded,
/// so a fresh maxDamage59 tool survives59 damage and breaks on the60th point.
pub fn max_durability(id:ItemId)->u16 { definition(id).map_or(0,|d|d.max_damage) }
pub fn apply_wear(stack:&mut Stack,amount:u16)->bool {
    let maximum=max_durability(stack.block);if maximum==0||amount==0{return false}
    stack.damage=stack.damage.saturating_add(amount);
    if stack.damage>maximum {stack.count=stack.count.saturating_sub(1);stack.damage=0;true}else{false}
}
pub fn variants(id:ItemId)->&'static [u8] {
    if id==DYE || id==ItemId::from(block::WOOL) { return &[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]; }
    if id==COAL { return &[0,1]; }
    if placed_block(id).is_some_and(|id|matches!(id,block::LOG|block::LEAVES|block::SAPLING)) { return &[0,1,2]; }
    if placed_block(id).is_some_and(|id|matches!(id,block::SLAB|block::DOUBLE_SLAB)) { return &[0,1,2,3]; }
    &[0]
}
pub fn valid_variant(id:ItemId,variant:u8)->bool { variants(id).contains(&variant) }
pub fn creative_stacks()->Vec<Stack> {
    block::ALL_BLOCKS.iter().filter(|&&id|block::definition(id).inventory_item).map(|&id|ItemId::from(id))
        .chain(ALL_ITEMS.iter().copied()).flat_map(|id|variants(id).iter().map(move|&v|Stack::with_variant(id,1,v))).collect()
}
pub fn container_remainder(id:ItemId)->Option<ItemId> { definition(id).and_then(|d|d.container) }
pub fn food_heal(id:ItemId)->Option<u8> {
    match id { APPLE=>Some(4),GOLDEN_APPLE=>Some(20),MUSHROOM_STEW=>Some(10),COOKIE=>Some(1),
        RAW_PORK=>Some(3),COOKED_PORK=>Some(8),RAW_FISH=>Some(2),COOKED_FISH|BREAD=>Some(5),_=>None }
}
pub fn armor_slot(id:ItemId)->Option<usize> { (298..=317).contains(&id).then_some((id.saturating_sub(298)%4) as usize) }
pub fn armor_defense(id:ItemId)->u8 { armor_slot(id).map_or(0,|slot|[3,8,6,3][slot]) }

/// Cursor operations preserve variant and wear; distinct metadata never merges.
pub fn click_slot(slot:&mut Option<Stack>,cursor:&mut Option<Stack>,split:bool) {
    match (*cursor,*slot) {
        (None,Some(mut stack))=>{let count=if split {stack.count.div_ceil(2)}else{stack.count};*cursor=Some(Stack{count,..stack});stack.count-=count;*slot=(stack.count>0).then_some(stack);}
        (Some(mut held),Some(mut target)) if held.same_kind(target)=>{let take=(if split{1}else{held.count}).min(stack_limit(target.block).saturating_sub(target.count));target.count+=take;held.count-=take;*slot=Some(target);*cursor=(held.count>0).then_some(held);}
        (Some(mut held),None)=>{let take=(if split{1}else{held.count}).min(stack_limit(held.block));*slot=Some(Stack{count:take,..held});held.count-=take;*cursor=(held.count>0).then_some(held);}
        (Some(_),Some(_))=>std::mem::swap(slot,cursor),
        (None,None)=>{},
    }
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)] pub enum ToolKind {Pickaxe,Shovel,Axe,Hoe,Sword}
#[derive(Clone,Copy,Debug)] pub struct Tool {pub kind:ToolKind,pub tier:u8,pub speed:f32,pub durability:u16,pub material:usize}
const TOOL_IDS:[[ItemId;5];5]=[
 [WOOD_PICKAXE,WOOD_SHOVEL,WOOD_AXE,WOOD_HOE,WOOD_SWORD],
 [STONE_PICKAXE,STONE_SHOVEL,STONE_AXE,STONE_HOE,STONE_SWORD],
 [IRON_PICKAXE,IRON_SHOVEL,IRON_AXE,IRON_HOE,IRON_SWORD],
 [GOLD_PICKAXE,GOLD_SHOVEL,GOLD_AXE,GOLD_HOE,GOLD_SWORD],
 [DIAMOND_PICKAXE,DIAMOND_SHOVEL,DIAMOND_AXE,DIAMOND_HOE,DIAMOND_SWORD],
];
pub fn tool(id:ItemId)->Option<Tool> {
 for (material,tools) in TOOL_IDS.iter().enumerate() {if let Some(kind)=tools.iter().position(|&i|i==id) {
 return Some(Tool{kind:[ToolKind::Pickaxe,ToolKind::Shovel,ToolKind::Axe,ToolKind::Hoe,ToolKind::Sword][kind],tier:[0,1,2,0,3][material],speed:[2.0,4.0,6.0,12.0,8.0][material],durability:max_durability(id),material});}}
 None
}
pub fn attack_damage(id:ItemId)->i32 {definition(id).map_or(1,|d|d.attack_damage)}
fn reference_bit(mask:[u64;4],reference_id:u8)->bool {mask[(reference_id/64)as usize]&(1u64<<(reference_id%64))!=0}
pub fn can_harvest(block_id:BlockId,held:ItemId)->bool {
    let block=block::definition(block_id);
    !block.requires_tool || definition(held).is_some_and(|d|reference_bit(d.harvest_blocks,block.reference_id))
}
pub fn hardness(id:BlockId)->f32 {block::definition(id).hardness}
pub fn mining_speed(block_id:BlockId,held:ItemId)->f32 {
    definition(held).filter(|d|reference_bit(d.efficient_blocks,block::definition(block_id).reference_id)).map_or(1.0,|d|d.efficiency)
}
pub fn mining_seconds(block_id:BlockId,held:ItemId)->f32 {
    let hard=hardness(block_id);
    if hard<0.0 {f32::INFINITY} else if !can_harvest(block_id,held) {hard*5.0} else {hard*1.5/mining_speed(block_id,held)}
}

pub fn migrate_legacy(mut stack:Stack)->Stack {
 let mapped=match stack.block {
 64=>Some((280,0)),
 65=>Some((263,0)),
 66=>Some((263,1)),
 67=>Some((265,0)),
 68=>Some((266,0)),
 69=>Some((337,0)),
 70=>Some((336,0)),
 71=>Some((319,0)),
 72=>Some((320,0)),
 73=>Some((349,0)),
 74=>Some((350,0)),
 75=>Some((325,0)),
 76=>Some((326,0)),
 77=>Some((327,0)),
 78=>Some((296,0)),
 79=>Some((297,0)),
 80=>Some((338,0)),
 81=>Some((339,0)),
 82=>Some((340,0)),
 83=>Some((352,0)),
 84=>Some((351,15)),
 85=>Some((353,0)),
 86=>Some((264,0)),
 87=>Some((334,0)),
 88=>Some((288,0)),
 89=>Some((262,0)),
 90=>Some((287,0)),
 91=>Some((289,0)),
 92=>Some((341,0)),
 93=>Some((351,0)),
 94=>Some((344,0)),
 95=>Some((295,0)),
 96=>Some((298,0)),
 97=>Some((299,0)),
 98=>Some((300,0)),
 99=>Some((301,0)),
 100=>Some((302,0)),
 101=>Some((303,0)),
 102=>Some((304,0)),
 103=>Some((305,0)),
 104=>Some((306,0)),
 105=>Some((307,0)),
 106=>Some((308,0)),
 107=>Some((309,0)),
 108=>Some((314,0)),
 109=>Some((315,0)),
 110=>Some((316,0)),
 111=>Some((317,0)),
 112=>Some((310,0)),
 113=>Some((311,0)),
 114=>Some((312,0)),
 115=>Some((313,0)),
 116=>Some((270,0)),
 117=>Some((269,0)),
 118=>Some((271,0)),
 119=>Some((290,0)),
 120=>Some((268,0)),
 121=>Some((274,0)),
 122=>Some((273,0)),
 123=>Some((275,0)),
 124=>Some((291,0)),
 125=>Some((272,0)),
 126=>Some((257,0)),
 127=>Some((256,0)),
 128=>Some((258,0)),
 129=>Some((292,0)),
 130=>Some((267,0)),
 131=>Some((285,0)),
 132=>Some((284,0)),
 133=>Some((286,0)),
 134=>Some((294,0)),
 135=>Some((283,0)),
 136=>Some((278,0)),
 137=>Some((277,0)),
 138=>Some((279,0)),
 139=>Some((293,0)),
 140=>Some((276,0)),
 _=>None,
 };
 if let Some((id,variant))=mapped {stack.block=id;stack.variant=variant;}
 stack
}

#[cfg(test)] mod tests {
 use super::*;
 #[test] fn legacy_metadata_migrates_without_confusing_new_block_codes(){
  assert_eq!(migrate_legacy(Stack::new(66u16,3)),Stack::with_variant(COAL,3,1));
  assert_eq!(migrate_legacy(Stack::new(84u16,3)),Stack::with_variant(DYE,3,15));
  assert_eq!(migrate_legacy(Stack::new(116u16,1)).block,WOOD_PICKAXE);
  assert_eq!(migrate_legacy(Stack::new(block::STONE,2)),Stack::new(block::STONE,2));
 }
 #[test] fn differently_colored_or_worn_stacks_never_merge(){
  let mut slot=Some(Stack::with_variant(block::WOOL,2,14));let mut cursor=Some(Stack::with_variant(block::WOOL,3,0));
  click_slot(&mut slot,&mut cursor,false);assert_eq!(slot.unwrap().variant,0);assert_eq!(cursor.unwrap().variant,14);
 }
 #[test] fn exact_registry_limits_and_variants(){
  assert_eq!(stack_limit(COOKIE),8);assert_eq!(stack_limit(SNOWBALL),16);assert_eq!(stack_limit(BOW),1);
  assert_eq!(max_durability(DIAMOND_PICKAXE),1561);assert_eq!(variants(DYE).len(),16);
  assert!(!valid(0));assert!(!is_placeable(WOOD_PICKAXE));assert_eq!(placed_block(WOOD_DOOR_ITEM),Some(block::WOOD_DOOR));
  assert_eq!(mining_seconds(block::OBSIDIAN,DIAMOND_PICKAXE),15.0);
 }
 #[test] fn original_itemstack_breaks_only_after_exceeding_maximum_damage(){
  let mut tool=Stack::new(WOOD_PICKAXE,1);
  for _ in 0..59 {assert!(!apply_wear(&mut tool,1));}
  assert_eq!(tool.damage,59);assert!(tool.normalized().is_some());
  assert!(apply_wear(&mut tool,1));assert_eq!((tool.count,tool.damage),(0,0));
  let mut rods=Stack::new(FISHING_ROD,2);rods.damage=64;
  assert!(apply_wear(&mut rods,1));assert_eq!((rods.count,rods.damage),(1,0));
 }
}
