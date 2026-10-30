//! Shared block-local geometry for collision, selection and rendering.
//! Original Beta dimensions/metadata conventions are retained in X/Y/Z. Bit16
//! extends horizontal facing to +/-W without stealing original open/head bits.
use crate::block::{self, BlockId, BlockShape, BlockState};
use glam::{IVec4,Vec4};

#[derive(Clone,Copy,Debug,PartialEq)]
pub struct Aabb4 {pub min:Vec4,pub max:Vec4}
impl Aabb4 {
 pub const ZERO:Self=Self{min:Vec4::ZERO,max:Vec4::ZERO};
 pub const UNIT:Self=Self{min:Vec4::ZERO,max:Vec4::ONE};
 pub fn new(min:Vec4,max:Vec4)->Self{Self{min,max}}
 pub fn translated(self,offset:Vec4)->Self{Self{min:self.min+offset,max:self.max+offset}}
 pub fn overlaps(self,other:Self)->bool{self.min.cmplt(other.max).all()&&self.max.cmpgt(other.min).all()}
 pub fn ray(self,origin:Vec4,dir:Vec4,max_dist:f32)->Option<(f32,IVec4)>{
  let(mut near,mut far)=(0.0,max_dist);let mut normal=IVec4::ZERO;
  for axis in 0..4{
   if dir[axis].abs()<1e-8{if origin[axis]<self.min[axis]||origin[axis]>self.max[axis]{return None}continue}
   let a=(self.min[axis]-origin[axis])/dir[axis];let b=(self.max[axis]-origin[axis])/dir[axis];
   let(lo,hi)=if a<b{(a,b)}else{(b,a)};
   if lo>near{near=lo;normal=IVec4::ZERO;normal[axis]=if dir[axis]>0.0{-1}else{1};}
   far=far.min(hi);if near>far+1e-6{return None}
  }
  (far>=0.0&&near<=max_dist).then_some((near.max(0.0),normal))
 }
}

#[derive(Clone,Copy,Debug)]
pub struct ShapeBoxes {boxes:[Aabb4;16],len:usize}
impl ShapeBoxes {
 pub const fn empty()->Self{Self{boxes:[Aabb4::ZERO;16],len:0}}
 pub fn one(aabb:Aabb4)->Self{let mut out=Self::empty();out.push(aabb);out}
 pub fn push(&mut self,aabb:Aabb4){assert!(self.len<self.boxes.len());self.boxes[self.len]=aabb;self.len+=1;}
 pub fn as_slice(&self)->&[Aabb4]{&self.boxes[..self.len]}
 pub fn len(&self)->usize{self.len}
 pub fn is_empty(&self)->bool{self.len==0}
}

pub const HORIZONTAL_DIRECTIONS:[IVec4;6]=[IVec4::X,IVec4::NEG_X,IVec4::Z,IVec4::NEG_Z,IVec4::W,IVec4::NEG_W];
pub fn dominant_horizontal(forward:Vec4)->IVec4{
 let axis=if forward.x.abs()>=forward.z.abs()&&forward.x.abs()>=forward.w.abs(){0}else if forward.z.abs()>=forward.w.abs(){2}else{3};
 let mut out=IVec4::ZERO;out[axis]=if forward[axis]<0.0{-1}else{1};out
}
pub fn stair_direction(metadata:u8)->IVec4{
 if metadata&16!=0{if metadata&1==0{IVec4::W}else{IVec4::NEG_W}}
 else{[IVec4::X,IVec4::NEG_X,IVec4::Z,IVec4::NEG_Z][(metadata&3)as usize]}
}
pub fn bed_direction(metadata:u8)->IVec4{
 if metadata&16!=0{if metadata&1==0{IVec4::W}else{IVec4::NEG_W}}
 else{[IVec4::Z,IVec4::NEG_X,IVec4::NEG_Z,IVec4::X][(metadata&3)as usize]}
}
pub fn door_direction(metadata:u8)->IVec4{
 if metadata&16!=0{if metadata&1==0{IVec4::W}else{IVec4::NEG_W}}
 else{[IVec4::X,IVec4::Z,IVec4::NEG_X,IVec4::NEG_Z][(metadata&3)as usize]}
}
pub fn facing_metadata(id:BlockId,direction:IVec4)->u8{
 if direction.w!=0{return 16+u8::from(direction.w<0)}
 match block::definition(id).shape{
  BlockShape::Bed=>if direction.z>0{0}else if direction.x<0{1}else if direction.z<0{2}else{3},
  BlockShape::Door=>if direction.x>0{0}else if direction.z>0{1}else if direction.x<0{2}else{3},
  BlockShape::Stairs=>if direction.x>0{0}else if direction.x<0{1}else if direction.z>0{2}else{3},
  _=>wall_metadata(direction),
 }
}
pub fn wall_metadata(normal:IVec4)->u8{
 if normal.z<0{2}else if normal.z>0{3}else if normal.x<0{4}else if normal.x>0{5}else if normal.w>0{16}else{17}
}
pub fn wall_support(metadata:u8)->IVec4{
 match metadata{2=>IVec4::Z,3=>IVec4::NEG_Z,4=>IVec4::X,5=>IVec4::NEG_X,16=>IVec4::NEG_W,17=>IVec4::W,_=>IVec4::ZERO}
}
pub fn torch_metadata(normal:IVec4)->Option<u8>{
 if normal.x>0{Some(1)}else if normal.x<0{Some(2)}else if normal.z>0{Some(3)}else if normal.z<0{Some(4)}else if normal.y>0{Some(5)}else if normal.w>0{Some(6)}else if normal.w<0{Some(7)}else{None}
}
pub fn torch_support(metadata:u8)->IVec4{match metadata&7{1=>IVec4::NEG_X,2=>IVec4::X,3=>IVec4::NEG_Z,4=>IVec4::Z,6=>IVec4::NEG_W,7=>IVec4::W,_=>IVec4::NEG_Y}}
pub fn connections(pos:IVec4,get:&impl Fn(IVec4)->Option<BlockState>)->u8{
 HORIZONTAL_DIRECTIONS.iter().enumerate().fold(0,|mask,(i,&d)|if get(pos+d).is_some_and(|s|s.id==block::FENCE){mask|1<<i}else{mask})
}
fn height(h:f32)->Aabb4{Aabb4::new(Vec4::ZERO,Vec4::new(1.0,h,1.0,1.0))}
fn wall_box(metadata:u8,thickness:f32,low_y:f32,high_y:f32)->Aabb4{
 let mut b=height(high_y);b.min.y=low_y;
 let support=wall_support(metadata);let axis=if support.x!=0{0}else if support.z!=0{2}else{3};
 if support[axis]>0{b.min[axis]=1.0-thickness}else{b.max[axis]=thickness}b
}
fn door_box(metadata:u8)->Aabb4{
 let t=3.0/16.0;
 if metadata&16!=0{
  let mut b=Aabb4::UNIT;
  let axis=if metadata&4==0{3}else{0};
  if metadata&1==0{b.max[axis]=t}else{b.min[axis]=1.0-t}return b
 }
 let plane=if metadata&4==0{metadata.wrapping_sub(1)&3}else{metadata&3};
 match plane{0=>{let mut b=Aabb4::UNIT;b.max.z=t;b},1=>{let mut b=Aabb4::UNIT;b.min.x=1.0-t;b},2=>{let mut b=Aabb4::UNIT;b.min.z=1.0-t;b},_=>{let mut b=Aabb4::UNIT;b.max.x=t;b}}
}

/// Visible boxes are also the conservative selection geometry. Thin cutout
/// plants/torches/sign text can be refined by the renderer within these bounds.
pub fn visual_boxes(state:BlockState,connections:u8)->ShapeBoxes{
 use block::*;let id=state.id;let m=state.metadata;
 if id==AIR{return ShapeBoxes::empty()}
 if id==FARMLAND{return ShapeBoxes::one(height(15.0/16.0))}
 match definition(id).shape{
  BlockShape::Cube|BlockShape::Web|BlockShape::Portal=>ShapeBoxes::one(Aabb4::UNIT),
  BlockShape::Fluid=>ShapeBoxes::one(height(fluid_height(id,None))),
  BlockShape::Slab=>ShapeBoxes::one(height(0.5)),
  BlockShape::Stairs=>{let mut out=ShapeBoxes::one(height(0.5));let dir=stair_direction(m);let axis=if dir.x!=0{0}else if dir.z!=0{2}else{3};let mut top=Aabb4::UNIT;top.min.y=0.5;if dir[axis]>0{top.min[axis]=0.5}else{top.max[axis]=0.5}out.push(top);out},
  BlockShape::Door=>ShapeBoxes::one(door_box(m)),
  BlockShape::Bed=>ShapeBoxes::one(height(9.0/16.0)),
  BlockShape::Ladder=>ShapeBoxes::one(wall_box(m,0.125,0.0,1.0)),
  BlockShape::SnowLayer=>ShapeBoxes::one(height(((m&7)+1)as f32/8.0)),
  BlockShape::Cactus=>ShapeBoxes::one(Aabb4::new(Vec4::new(0.0625,0.0,0.0625,0.0625),Vec4::new(0.9375,1.0,0.9375,0.9375))),
  BlockShape::Cake=>ShapeBoxes::one(Aabb4::new(Vec4::new((1+2*m.min(5))as f32/16.0,0.0,0.0625,0.0625),Vec4::new(0.9375,0.5,0.9375,0.9375))),
  BlockShape::Fence=>{let mut out=ShapeBoxes::one(Aabb4::new(Vec4::new(0.375,0.0,0.375,0.375),Vec4::new(0.625,1.0,0.625,0.625)));for(i,dir)in HORIZONTAL_DIRECTIONS.into_iter().enumerate(){if connections&(1<<i)==0{continue}let axis=if dir.x!=0{0}else if dir.z!=0{2}else{3};for y in [0.375,0.75]{let mut arm=Aabb4::new(Vec4::new(0.4375,y,0.4375,0.4375),Vec4::new(0.5625,y+0.1875,0.5625,0.5625));if dir[axis]>0{arm.min[axis]=0.5;arm.max[axis]=1.0}else{arm.min[axis]=0.0;arm.max[axis]=0.5}out.push(arm);}}out},
  BlockShape::Sign=>{if id==WALL_SIGN{ShapeBoxes::one(wall_box(m,0.125,0.28125,0.78125))}else{let mut out=ShapeBoxes::one(Aabb4::new(Vec4::new(0.4375,0.0,0.4375,0.4375),Vec4::new(0.5625,0.5625,0.5625,0.5625)));out.push(if m&16!=0{Aabb4::new(Vec4::new(0.0,0.5,0.0,0.4375),Vec4::new(1.0,1.0,1.0,0.5625))}else{Aabb4::new(Vec4::new(0.0,0.5,0.4375,0.0),Vec4::new(1.0,1.0,0.5625,1.0))});out}},
  BlockShape::Rail=>ShapeBoxes::one(height(if matches!(crate::rails::shape(state),2..=5|17|18){0.625}else{0.125})),
  BlockShape::Wire=>ShapeBoxes::one(height(1.0/16.0)),
  BlockShape::Repeater=>ShapeBoxes::one(height(0.125)),
  BlockShape::PressurePlate=>ShapeBoxes::one(Aabb4::new(Vec4::new(0.0625,0.0,0.0625,0.0625),Vec4::new(0.9375,if m&1==0{0.0625}else{0.03125},0.9375,0.9375))),
  BlockShape::Torch=>{let support=torch_support(m);if support.y<0{ShapeBoxes::one(Aabb4::new(Vec4::new(0.4375,0.0,0.4375,0.4375),Vec4::new(0.5625,0.625,0.5625,0.5625)))}else{let mut b=Aabb4::new(Vec4::new(0.35,0.2,0.35,0.35),Vec4::new(0.65,0.8,0.65,0.65));let axis=if support.x!=0{0}else if support.z!=0{2}else{3};if support[axis]>0{b.min[axis]=0.7;b.max[axis]=1.0}else{b.min[axis]=0.0;b.max[axis]=0.3}ShapeBoxes::one(b)}},
  BlockShape::Button|BlockShape::Lever=>ShapeBoxes::one(Aabb4::new(Vec4::new(0.25,0.25,0.25,0.25),Vec4::new(0.75,0.75,0.75,0.75))),
  BlockShape::Plant=>ShapeBoxes::one(Aabb4::UNIT),
  BlockShape::Fire=>ShapeBoxes::empty(),
 }
}
pub fn selection_boxes(state:BlockState,connections:u8)->ShapeBoxes{
 if state.id==block::SIGN_POST{return ShapeBoxes::one(Aabb4::new(Vec4::new(0.25,0.0,0.25,0.25),Vec4::new(0.75,1.0,0.75,0.75)))}
 if state.id==block::CROPS{return ShapeBoxes::one(height(0.25))}
 if block::definition(state.id).shape==BlockShape::Plant{
  let(radius,h)=if state.id==block::SAPLING{(0.4,0.8)}else if state.id==block::REEDS{(0.375,1.0)}else if matches!(state.id,block::BROWN_MUSHROOM|block::RED_MUSHROOM){(0.2,0.4)}else{(0.2,0.6)};
  return ShapeBoxes::one(Aabb4::new(Vec4::new(0.5-radius,0.0,0.5-radius,0.5-radius),Vec4::new(0.5+radius,h,0.5+radius,0.5+radius)))
 }
 visual_boxes(state,connections)
}
pub fn collision_boxes(state:BlockState,connections:u8)->ShapeBoxes{
 use block::*;
 if state.id==AIR{return ShapeBoxes::empty()}
 if state.id==FARMLAND{return ShapeBoxes::one(Aabb4::UNIT)} // verified ue.d: full-height collision
 if state.id==SOUL_SAND{return ShapeBoxes::one(height(0.875))}
 match definition(state.id).shape{
  BlockShape::Fence=>ShapeBoxes::one(height(1.5)), // verified Beta jc.d: full footprint, not modern connected collision
  BlockShape::SnowLayer=>if state.metadata&7>=3{ShapeBoxes::one(height(0.5))}else{ShapeBoxes::empty()},
  BlockShape::Fluid|BlockShape::Plant|BlockShape::Rail|BlockShape::Wire|BlockShape::Torch|BlockShape::Fire|BlockShape::Sign|BlockShape::Button|BlockShape::Lever|BlockShape::PressurePlate|BlockShape::Web|BlockShape::Portal=>ShapeBoxes::empty(),
  _=>visual_boxes(state,connections),
 }
}

#[cfg(test)]mod tests{
 use super::*;
 #[test]fn beta_dimensions_and_door_open_state(){
  assert_eq!(visual_boxes(BlockState::new(block::SLAB,0),0).as_slice()[0].max.y,0.5);
  assert_eq!(collision_boxes(BlockState::new(block::DOUBLE_SLAB,0),0).as_slice()[0],Aabb4::UNIT);
  assert_eq!(visual_boxes(BlockState::new(block::BED,0),0).as_slice()[0].max.y,0.5625);
  assert_eq!(collision_boxes(BlockState::new(block::FENCE,0),0).as_slice()[0].max.y,1.5);
  assert_eq!(collision_boxes(BlockState::new(block::FARMLAND,0),0).as_slice()[0].max.y,1.0);
  let closed=collision_boxes(BlockState::new(block::WOOD_DOOR,0),0).as_slice()[0];
  let open=collision_boxes(BlockState::new(block::WOOD_DOOR,4),0).as_slice()[0];
  assert_eq!(closed.max.x,0.1875);assert_eq!(open.max.z,0.1875);
 }
 #[test]fn stairs_lift_to_w_and_boxes_remain_bounded(){
  let s=visual_boxes(BlockState::new(block::WOOD_STAIRS,16),0);assert_eq!(s.len(),2);assert_eq!(s.as_slice()[1].min.w,0.5);
  for &id in block::ALL_BLOCKS{for metadata in 0..32{for b in visual_boxes(BlockState::new(id,metadata),63).as_slice(){assert!(b.min.cmpge(Vec4::ZERO).all());assert!(b.max.cmple(Vec4::ONE).all());assert!(b.min.cmple(b.max).all());}}}
 }
 #[test]fn rays_miss_empty_half_of_slab_and_hit_exact_surface(){
  let b=visual_boxes(BlockState::new(block::SLAB,0),0).as_slice()[0];
  assert!(b.ray(Vec4::new(-1.0,0.75,0.5,0.5),Vec4::X,5.0).is_none());
  let(t,n)=b.ray(Vec4::new(0.5,2.0,0.5,0.5),-Vec4::Y,5.0).unwrap();assert_eq!(t,1.5);assert_eq!(n,IVec4::Y);
 }
}
