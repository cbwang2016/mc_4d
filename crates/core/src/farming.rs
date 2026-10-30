//! Beta1.5 plant rules lifted symmetrically into horizontal X/Z/W space.
//! The original world samples80 positions per16*128*16 column per20Hz tick.
use crate::{block::{self,BlockState},shapes::HORIZONTAL_DIRECTIONS};
use glam::IVec4;
use std::collections::{HashSet,VecDeque};

pub const RANDOM_TICK_PROBABILITY:f64=80.0/(16.0*128.0*16.0);
pub const CRITICAL_BLOCKS:&[u8]=&[block::FARMLAND,block::CROPS,block::SAPLING,block::CACTUS,block::REEDS];
#[derive(Clone,Copy,Debug)]pub struct PlantLight{pub level:u8,pub sees_sky:bool}
#[derive(Clone,Copy,Debug,PartialEq)]pub enum TickAction{Stay,Defer,Replace(BlockState),Break,GrowTree(u8),GrowColumn,Spread([Option<IVec4>;4])}
pub fn needs_tick(s:BlockState)->bool{CRITICAL_BLOCKS.contains(&s.id)||s.id==block::LEAVES&&s.metadata&8!=0}

/// Sparse schedules retain the original per-block mean without scanning every
/// empty4D voxel. Geometric intervals have mean409.6 game ticks.
pub fn random_tick_delay(uniform_open:f64)->u64{
 (uniform_open.clamp(f64::EPSILON,1.0-f64::EPSILON).ln()/(1.0-RANDOM_TICK_PROBABILITY).ln()).ceil().max(1.0)as u64
}
pub fn near_water(pos:IVec4,get:&impl Fn(IVec4)->Option<BlockState>)->Option<bool>{
 let mut unknown=false;
 for w in -4..=4{for z in -4..=4{for x in -4..=4{for y in 0..=1{
  match get(pos+IVec4::new(x,y,z,w)){Some(s)if block::is_water(s.id)=>return Some(true),None=>unknown=true,_=>{}}
 }}}}
 if unknown{None}else{Some(false)}
}
pub fn supported(pos:IVec4,s:BlockState,get:&impl Fn(IVec4)->Option<BlockState>)->Option<bool>{
 let below=get(pos-IVec4::Y)?.id;
 match s.id{
  block::CROPS=>Some(below==block::FARMLAND),
  block::SNOW_LAYER=>Some(block::definition(below).opaque&&crate::ecology::material_blocks_movement(below)),
  block::SAPLING|block::DANDELION|block::ROSE=>Some(matches!(below,block::DIRT|block::GRASS|block::FARMLAND)),
  block::BROWN_MUSHROOM|block::RED_MUSHROOM=>Some(block::definition(below).opaque),
  block::REEDS=>{
   if below==block::REEDS{return Some(true)}
   // Sand support was added afterBeta1.5: qh.a permits only grass or dirt.
   if !matches!(below,block::DIRT|block::GRASS){return Some(false)}
   let mut unknown=false;for d in HORIZONTAL_DIRECTIONS{match get(pos-IVec4::Y+d){Some(s)if block::is_water(s.id)=>return Some(true),None=>unknown=true,_=>{}}}
   if unknown{None}else{Some(false)}
  },
  block::CACTUS=>{
   if !matches!(below,block::SAND|block::CACTUS){return Some(false)}
   let mut unknown=false;for d in HORIZONTAL_DIRECTIONS{match get(pos+d){Some(s)if block::is_solid(s.id)=>return Some(false),None=>unknown=true,_=>{}}}if unknown{None}else{Some(true)}
  },
  _=>Some(true),
 }
}
pub fn crop_fertility(pos:IVec4,get:&impl Fn(IVec4)->Option<BlockState>)->f32{
 let mut fertility=1.0;let mut axes=[false;3];let mut diagonal=false;
 for w in -1..=1{for z in -1..=1{for x in -1..=1{
  let offset=IVec4::new(x,0,z,w);
  if let Some(soil)=get(pos+offset-IVec4::Y)&&soil.id==block::FARMLAND{
   let mut contribution=if soil.metadata>0{3.0}else{1.0};if offset!=IVec4::ZERO{contribution/=4.0}fertility+=contribution;
  }
  if offset!=IVec4::ZERO&&get(pos+offset).is_some_and(|s|s.id==block::CROPS){
   let count=(x!=0)as usize+(z!=0)as usize+(w!=0)as usize;
   if count>=2{diagonal=true}else if x!=0{axes[0]=true}else if z!=0{axes[1]=true}else{axes[2]=true}
  }
 }}}
 if diagonal||axes.iter().filter(|&&v|v).count()>=2{fertility/=2.0}fertility
}
fn leaf_connected(pos:IVec4,get:&impl Fn(IVec4)->Option<BlockState>)->Option<bool>{
 let mut queue=VecDeque::from([(pos,0u8)]);let mut seen=HashSet::from([pos]);let mut unknown=false;
 while let Some((p,distance))=queue.pop_front(){
  match get(p){
   Some(s)if s.id==block::LOG=>return Some(true),
   Some(s)if s.id==block::LEAVES&&distance<4=>{for offset in [IVec4::X,IVec4::NEG_X,IVec4::Y,IVec4::NEG_Y,IVec4::Z,IVec4::NEG_Z,IVec4::W,IVec4::NEG_W]{if seen.insert(p+offset){queue.push_back((p+offset,distance+1))}}},
   None=>unknown=true,_=>{},
  }
 }
 if unknown{None}else{Some(false)}
}
pub fn tick(pos:IVec4,state:BlockState,get:&impl Fn(IVec4)->Option<BlockState>,light:&impl Fn(IVec4)->Option<PlantLight>,raining:bool,roll:&mut impl FnMut(u32)->u32)->TickAction{
 use TickAction::*;
 if needs_tick(state)&&state.id!=block::FARMLAND&&state.id!=block::LEAVES{
  match supported(pos,state,get){Some(false)=>return Break,None=>return Defer,_=>{}}
 }
 match state.id{
  block::FARMLAND=>{
   if get(pos+IVec4::Y).is_some_and(|s|block::is_solid(s.id)){return Replace(block::DIRT.into())}
   if roll(5)%5!=0{return Stay}
   let water=near_water(pos,get);
   let wet=water==Some(true)||raining&&light(pos+IVec4::Y).is_some_and(|l|l.sees_sky);
   if wet{return Replace(BlockState::new(block::FARMLAND,7))}
   if water.is_none(){return Defer}
   if state.metadata>0{Replace(BlockState::new(block::FARMLAND,state.metadata-1))}
   else if get(pos+IVec4::Y).is_some_and(|s|s.id==block::CROPS){Stay}else{Replace(block::DIRT.into())}
  },
  block::CROPS|block::SAPLING=>{
   let Some(here)=light(pos)else{return Defer};if here.level<8&&!here.sees_sky{return Break}
   let Some(above)=light(pos+IVec4::Y)else{return Defer};if above.level<9{return Stay}
   if state.id==block::CROPS{
    if state.metadata>=7{return Stay}let chance=(100.0/crop_fertility(pos,get))as u32;
    if roll(chance.max(1))%chance.max(1)==0{Replace(BlockState::new(block::CROPS,state.metadata+1))}else{Stay}
   }else if roll(30)%30==0{
    if state.metadata&8==0{Replace(BlockState::new(block::SAPLING,state.metadata|8))}else{GrowTree(state.metadata&3)}
   }else{Stay}
  },
  block::CACTUS|block::REEDS=>{
   if get(pos+IVec4::Y).is_none(){return Defer}
   if get(pos+IVec4::Y).is_none_or(|s|s.id!=block::AIR){return Stay}
   let mut height=1;while height<3&&get(pos-IVec4::Y*height).is_some_and(|s|s.id==state.id){height+=1}
   if height>=3{Stay}else if state.metadata>=15{GrowColumn}else{Replace(BlockState::new(state.id,state.metadata+1))}
  },
  block::LEAVES if state.metadata&8!=0=>match leaf_connected(pos,get){Some(true)=>Replace(BlockState::new(block::LEAVES,state.metadata&!8)),Some(false)=>Break,None=>Defer},
  block::GRASS=>{
   let Some(above)=light(pos+IVec4::Y)else{return Defer};let Some(top)=get(pos+IVec4::Y)else{return Defer};
   if above.level<4&&block::light_opacity(top)>2{return Replace(block::DIRT.into())}
   if above.level<9{return Stay}
   let mut targets=[None;4];
   for target in &mut targets{let p=pos+IVec4::new(roll(3)as i32-1,roll(5)as i32-3,roll(3)as i32-1,roll(3)as i32-1);
    if get(p).is_some_and(|s|s.id==block::DIRT)&&get(p+IVec4::Y).is_some_and(|s|block::light_opacity(s)<=2)&&light(p+IVec4::Y).is_some_and(|l|l.level>=4){*target=Some(p)}
   }Spread(targets)
  },
  _=>Stay,
 }
}

#[cfg(test)]mod tests{
 use super::*;use std::collections::HashMap;
 #[test]fn original_tick_rate_and_farmland_water_range_include_w(){
  assert!((1.0/RANDOM_TICK_PROBABILITY-409.6).abs()<1e-6);
  let mut total=0u64;for i in 0..10000{total+=random_tick_delay((i as f64+0.5)/10000.0)}
  assert!((total as f64/10000.0-409.6).abs()<1.0);
  let water=|p:IVec4|Some(if p==IVec4::new(0,1,0,4){block::WATER.into()}else{block::AIR.into()});assert_eq!(near_water(IVec4::ZERO,&water),Some(true));
  assert_eq!(near_water(IVec4::new(0,0,0,-1),&water),Some(false));
 }
 #[test]fn hydration_drying_crop_light_and_growth_match_old_rules(){
  let mut map=HashMap::new();map.insert(IVec4::ZERO,BlockState::new(block::FARMLAND,0));map.insert(IVec4::X,block::WATER.into());map.insert(IVec4::Y,BlockState::new(block::CROPS,0));
  let get=|p|Some(*map.get(&p).unwrap_or(&block::AIR.into()));let bright=|_|Some(PlantLight{level:15,sees_sky:true});
  assert_eq!(tick(IVec4::ZERO,map[&IVec4::ZERO],&get,&bright,false,&mut|_|0),TickAction::Replace(BlockState::new(block::FARMLAND,7)));
  assert_eq!(tick(IVec4::Y,map[&IVec4::Y],&get,&bright,false,&mut|_|0),TickAction::Replace(BlockState::new(block::CROPS,1)));
  let night=|_|Some(PlantLight{level:4,sees_sky:true});assert_eq!(tick(IVec4::Y,map[&IVec4::Y],&get,&night,false,&mut|_|0),TickAction::Stay);
  let dark=|_|Some(PlantLight{level:0,sees_sky:false});assert_eq!(tick(IVec4::Y,map[&IVec4::Y],&get,&dark,false,&mut|_|0),TickAction::Break);
 }
 #[test]fn saplings_need_growth_mark_and_reeds_reject_later_sand_rule(){
  let get=|p:IVec4|Some(if p==IVec4::NEG_Y{block::DIRT.into()}else if p==IVec4::ZERO{block::SAPLING.into()}else{block::AIR.into()});let light=|_|Some(PlantLight{level:15,sees_sky:true});
  assert_eq!(tick(IVec4::ZERO,BlockState::new(block::SAPLING,2),&get,&light,false,&mut|_|0),TickAction::Replace(BlockState::new(block::SAPLING,10)));
  assert_eq!(tick(IVec4::ZERO,BlockState::new(block::SAPLING,10),&get,&light,false,&mut|_|0),TickAction::GrowTree(2));
  let sand=|p:IVec4|Some(if p==IVec4::NEG_Y{block::SAND.into()}else{block::WATER.into()});assert_eq!(supported(IVec4::ZERO,block::REEDS.into(),&sand),Some(false));
 }
 #[test]fn leaf_support_is_four_dimensional_and_decay_is_marked(){
  let get=|p:IVec4|Some(if p==IVec4::W{block::LOG.into()}else if p==IVec4::ZERO{BlockState::new(block::LEAVES,10)}else{block::AIR.into()});
  assert_eq!(tick(IVec4::ZERO,BlockState::new(block::LEAVES,10),&get,&|_|None,false,&mut|_|0),TickAction::Replace(BlockState::new(block::LEAVES,2)));
  let empty=|_|Some(block::AIR.into());assert_eq!(tick(IVec4::ZERO,BlockState::new(block::LEAVES,8),&empty,&|_|None,false,&mut|_|0),TickAction::Break);
 }
}
