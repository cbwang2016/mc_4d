//! Exact icon/tile lookup from the verified Beta1.5 registry, including metadata.
use mc4d_core::{block::{self,BlockState},item::{self,ItemId,Stack}};
#[derive(Clone,Copy,Debug,PartialEq)]
pub struct AtlasIcon {pub layer:u32,pub tile:u32,pub tint:[f32;3],pub mirrored:bool}
impl AtlasIcon {pub fn uv(self)->[[f32;2];2]{let x=(self.tile%16)as f32*16.0;let y=(self.tile/16)as f32*16.0;let (u0,u1)=if self.mirrored{(x+15.5,x+0.5)}else{(x+0.5,x+15.5)};[[u0/256.0,(y+0.5)/256.0],[u1/256.0,(y+15.5)/256.0]]}}
#[derive(Clone,Copy,Debug)]pub enum BlockFace{Top,Bottom,Side,Front}
pub fn block_icon(id:u8,face:BlockFace)->Option<AtlasIcon>{block_state_icon(BlockState::new(id,0),face)}
pub fn block_icon_stack(stack:Stack,face:BlockFace)->Option<AtlasIcon>{block_state_icon(item::placement(stack)?,face)}
pub fn block_state_icon(state:BlockState,face:BlockFace)->Option<AtlasIcon>{
 if state.id==block::AIR||!block::registered(state.id){return None}
 let side=match face{BlockFace::Bottom=>0,BlockFace::Top=>1,BlockFace::Side=>2,BlockFace::Front=>3};
 let tile=block::texture_tile(state,side)as u32;
 let tint=if state.id==block::GRASS&&side==1{[0.48,0.72,0.28]}
 else if state.id==block::LEAVES{match state.metadata&3{1=>[0.25,0.45,0.20],2=>[0.5,0.60,0.25],_=>[0.30,0.60,0.18]}}
 else{[1.0;3]};Some(AtlasIcon{layer:0,tile,tint,mirrored:block::texture_mirrored(state,side)})
}
pub fn item_icon(id:ItemId)->Option<AtlasIcon>{item_icon_stack(Stack::new(id,1))}
pub fn item_icon_stack(stack:Stack)->Option<AtlasIcon>{
 if let Some(definition)=item::definition(stack.block){return Some(AtlasIcon{layer:1,tile:definition.tiles[(stack.variant&15)as usize]as u32,tint:[1.0;3],mirrored:false})}
 block_icon_stack(stack,BlockFace::Side)
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn every_creative_stack_has_a_finite_atlas_icon(){for stack in item::creative_stacks(){let icon=item_icon_stack(stack).unwrap_or_else(||panic!("missing{}:{}",stack.block,stack.variant));assert!(icon.tile<256);assert!(icon.uv().into_iter().flatten().all(|x|x.is_finite()&&(0.0..=1.0).contains(&x)));}}
 #[test]fn wool_and_dye_variants_use_distinct_original_tiles(){assert_ne!(item_icon_stack(Stack::with_variant(block::WOOL,1,0)).unwrap().tile,item_icon_stack(Stack::with_variant(block::WOOL,1,14)).unwrap().tile);assert_ne!(item_icon_stack(Stack::with_variant(item::DYE,1,0)).unwrap().tile,item_icon_stack(Stack::with_variant(item::DYE,1,15)).unwrap().tile);}
}
