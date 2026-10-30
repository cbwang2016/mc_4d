//! Original block loot rules and harvest overrides, separate from tool gating.
//! Data comes from a(Random), a(metadata,Random), b(metadata), plus the snow and
//! crop overrides inspected in the reference JAR. No modern loot-table assumptions.
use crate::{block::{self,BlockState},item::{self,ItemId,Stack}};
#[derive(Clone,Copy)]
struct DropRule{count:u8,span:u8,chance:u8,alternate:ItemId,alternate_chance:u8,items:[ItemId;16],variants:[u8;16]}
impl DropRule{const EMPTY:Self=Self{count:0,span:0,chance:1,alternate:0,alternate_chance:1,items:[0;16],variants:[0;16]};}
#[path="loot_registry.rs"]mod registry;
const RULES:[DropRule;256]=registry::rules();
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum DropCause{Harvest,Natural}

/// Every returned stack is one physical dropped item, as in the original loop.
/// The caller checks tool eligibility before Harvest and supplies bounded RNG.
pub fn drops(state:BlockState,cause:DropCause,mut roll:impl FnMut(u32)->u32)->Vec<Stack>{
 if state.id==block::SNOW_LAYER && cause==DropCause::Harvest{return vec![Stack::new(item::SNOWBALL,1)]}
 let rule=RULES[block::material_id(state.id)as usize];let metadata=(state.metadata&15)as usize;
 let mut count=rule.count as u32;
 if rule.chance>1 && roll(rule.chance as u32)%rule.chance as u32!=0{count=0}
 if rule.span>0{count+=roll(rule.span as u32)%rule.span as u32}
 let mut out=Vec::new();
 for _ in 0..count{
  let id=if rule.alternate!=0 && roll(rule.alternate_chance as u32)%rule.alternate_chance as u32==0{rule.alternate}else{rule.items[metadata]};
  if id!=0{out.push(Stack::with_variant(id,1,rule.variants[metadata]));}
 }
 // mm.a(World,...): three independent seed rolls even when the crop is immature.
 if state.id==block::CROPS{for _ in 0..3{if roll(15)%15<=u32::from(state.metadata.min(7)){out.push(Stack::new(item::SEEDS,1));}}}
 out
}

#[cfg(test)]mod tests{
 use super::*;
 #[test]fn original_material_quantities_and_variants(){
  let zero=|_:u32|0;
  assert_eq!(drops(BlockState::new(block::CLAY,0),DropCause::Harvest,zero),vec![Stack::new(item::CLAY_BALL,1);4]);
  assert_eq!(drops(BlockState::new(block::GLOW,0),DropCause::Harvest,zero),vec![Stack::new(item::GLOWSTONE_DUST,1)]);
  assert!(drops(BlockState::new(block::BOOKSHELF,0),DropCause::Harvest,zero).is_empty());
  assert_eq!(drops(BlockState::new(block::DOUBLE_SLAB,2),DropCause::Harvest,zero),vec![Stack::with_variant(block::SLAB,1,2);2]);
  assert_eq!(drops(BlockState::new(block::LAPIS_ORE,0),DropCause::Harvest,|n|n-1),vec![Stack::with_variant(item::DYE,1,4);8]);
  assert_eq!(drops(BlockState::new(block::REDSTONE_ORE,0),DropCause::Harvest,|n|n-1),vec![Stack::new(item::REDSTONE,1);5]);
 }
 #[test]fn exact_gravel_and_leaf_chance_boundaries(){
  assert_eq!(drops(BlockState::new(block::GRAVEL,0),DropCause::Harvest,|_|0),vec![Stack::new(item::FLINT,1)]);
  assert_eq!(drops(BlockState::new(block::GRAVEL,0),DropCause::Harvest,|_|1),vec![Stack::new(block::GRAVEL,1)]);
  assert_eq!(drops(BlockState::new(block::LEAVES,10),DropCause::Natural,|_|0),vec![Stack::with_variant(block::SAPLING,1,2)]);
  for choice in 1..20{assert!(drops(BlockState::new(block::LEAVES,2),DropCause::Natural,|_|choice).is_empty());}
 }
 #[test]fn harvest_overrides_and_paired_half_rules(){
  assert!(drops(BlockState::new(block::SNOW_LAYER,0),DropCause::Natural,|_|0).is_empty());
  assert_eq!(drops(BlockState::new(block::SNOW_LAYER,0),DropCause::Harvest,|_|0),vec![Stack::new(item::SNOWBALL,1)]);
  assert!(drops(BlockState::new(block::BED,8),DropCause::Harvest,|_|0).is_empty());
  assert_eq!(drops(BlockState::new(block::WOOD_DOOR,0),DropCause::Harvest,|_|0),vec![Stack::new(item::WOOD_DOOR_ITEM,1)]);
  let mature=drops(BlockState::new(block::CROPS,7),DropCause::Harvest,|_|0);
  assert_eq!(mature.iter().filter(|s|s.block==item::WHEAT).count(),1);assert_eq!(mature.iter().filter(|s|s.block==item::SEEDS).count(),3);
  assert!(drops(BlockState::new(block::CROPS,0),DropCause::Harvest,|_|14).is_empty());
 }
}
