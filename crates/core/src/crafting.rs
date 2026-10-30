//! Exact Beta1.5 recipe data with metadata-aware shaped/shapeless matching.
use crate::{block as b,item::{self as i,ItemId,Stack}};

#[derive(Clone,Copy,Debug)]
pub struct Ingredient {pub id:ItemId,pub variant:Option<u8>}
impl Ingredient {
 fn matches(self,stack:Stack)->bool {stack.count>0&&stack.block==self.id&&self.variant.is_none_or(|v|v==stack.variant)}
}
#[derive(Clone,Copy,Debug)]
pub struct Recipe {
 pub reference_index:usize,pub width:usize,pub height:usize,pub shaped:bool,
 pub ingredients:&'static [Option<Ingredient>],pub output:Stack,
}
#[path="recipes_generated.rs"]mod generated;
pub use generated::RECIPES;
pub struct RecipeMatch {pub output:Stack,pub ingredients:Vec<usize>,pub reference_index:usize}

pub fn recipe(grid:&[Option<Stack>],width:usize)->Option<RecipeMatch>{
 if !matches!(width,2|3)||grid.len()<width*width{return None}
 for r in RECIPES {
  if r.shaped {
   if r.width>width||r.height>width{continue}
   for y in 0..=width-r.height {for x in 0..=width-r.width {for mirror in [false,true]{
    let mut consumed=Vec::new();
    let matches=(0..width*width).all(|index|{
     let(cx,cy)=(index%width,index/width);
     let expected=if cx>=x&&cx<x+r.width&&cy>=y&&cy<y+r.height {
      let rx=if mirror{r.width-1-(cx-x)}else{cx-x};r.ingredients[(cy-y)*r.width+rx]
     }else{None};
     match(expected,grid[index].filter(|s|s.count>0)){
      (None,None)=>true,(Some(ingredient),Some(stack))if ingredient.matches(stack)=>{consumed.push(index);true},_=>false,
     }
    });
    if matches{return Some(RecipeMatch{output:r.output,ingredients:consumed,reference_index:r.reference_index})}
   }}}
  }else{
   let mut remaining:Vec<_>=r.ingredients.iter().flatten().copied().collect();let mut consumed=Vec::new();let mut matches=true;
   for(index,slot)in grid[..width*width].iter().enumerate(){if let Some(stack)=slot.filter(|s|s.count>0){
    if let Some(at)=remaining.iter().position(|i|i.matches(stack)){remaining.remove(at);consumed.push(index)}else{matches=false;break}
   }}
   if matches&&remaining.is_empty(){return Some(RecipeMatch{output:r.output,ingredients:consumed,reference_index:r.reference_index})}
  }
 }
 None
}

/// Output capacity is checked before changing ingredients. Original container
/// remainders (e.g. milk buckets in cake) replace their consumed grid cell.
pub fn take_output(grid:&mut[Option<Stack>],width:usize,cursor:&mut Option<Stack>)->bool{
 let Some(found)=recipe(grid,width)else{return false};
 let old=match cursor{None=>0,Some(s)if s.same_kind(found.output)=>s.count,_=>return false};
 if old+found.output.count>i::stack_limit(found.output.block){return false}
 *cursor=Some(Stack{count:old+found.output.count,..found.output});
 for index in found.ingredients{
  let mut input=grid[index].expect("matched input");input.count-=1;
  grid[index]=if let Some(container)=i::container_remainder(input.block){Some(Stack::new(container,1))}else{(input.count>0).then_some(input)};
 }
 true
}

pub fn smelting_result(id:ItemId)->Option<Stack>{
 if let Ok(block)=u8::try_from(id){
  let out=match block{
   b::COBBLE=>Stack::new(b::STONE,1),b::SAND=>Stack::new(b::GLASS,1),
   b::LOG=>Stack::with_variant(i::COAL,1,1),b::IRON_ORE=>Stack::new(i::IRON_INGOT,1),
   b::GOLD_ORE=>Stack::new(i::GOLD_INGOT,1),b::DIAMOND_ORE=>Stack::new(i::DIAMOND,1),
   b::CACTUS=>Stack::with_variant(i::DYE,1,2),_=>return None,
  };return Some(out)
 }
 Some(match id{i::CLAY_BALL=>Stack::new(i::BRICK_ITEM,1),i::RAW_PORK=>Stack::new(i::COOKED_PORK,1),i::RAW_FISH=>Stack::new(i::COOKED_FISH,1),_=>return None})
}
pub fn fuel_seconds(id:ItemId)->f32{
 match id{i::COAL=>80.0,i::STICK=>5.0,i::LAVA_BUCKET=>1000.0,_=>{
  let Ok(block)=u8::try_from(id)else{return 0.0};
  if block==b::SAPLING{return 5.0}
  if matches!(block,b::LOG|b::PLANKS|b::CRAFTING_TABLE|b::CHEST|b::BOOKSHELF|b::WOOD_STAIRS|b::NOTE_BLOCK|b::JUKEBOX|b::FENCE|b::WOOD_PRESSURE_PLATE){15.0}else{0.0}
 }}
}

#[cfg(test)]mod tests{
 use super::*;
 fn grid_for(r:&Recipe,width:usize,mirror:bool,x:usize,y:usize)->Vec<Option<Stack>>{
  let mut grid=vec![None;width*width];
  for(index,ingredient)in r.ingredients.iter().enumerate(){if let Some(i)=ingredient{
   let at=if r.shaped{(y+index/r.width)*width+x+if mirror{r.width-1-index%r.width}else{index%r.width}}else{index};
   grid[at]=Some(Stack::with_variant(i.id,1,i.variant.unwrap_or(0)));
  }}grid
 }
 #[test]fn all_146_reference_recipes_match_including_mirrors_and_offsets(){
  assert_eq!(RECIPES.len(),146);
  for r in RECIPES{
   let mut grid=grid_for(r,3,false,0,0);let found=recipe(&grid,3).unwrap_or_else(||panic!("recipe{}",r.reference_index));assert_eq!(found.output,r.output,"recipe{}",r.reference_index);
   if r.shaped{
    grid=grid_for(r,3,true,3-r.width,3-r.height);
    assert_eq!(recipe(&grid,3).unwrap().output,r.output,"mirrored recipe{}",r.reference_index);
   }else{grid.reverse();assert_eq!(recipe(&grid,3).unwrap().output,r.output,"shapeless recipe{}",r.reference_index)}
  }
 }
 #[test]fn output_capacity_and_container_remainders_are_transactional(){
  let cake=RECIPES.iter().find(|r|r.output.block==i::CAKE_ITEM).unwrap();let mut grid=grid_for(cake,3,false,0,0);let original=grid.clone();let mut cursor=Some(Stack::new(i::CAKE_ITEM,1));
  assert!(!take_output(&mut grid,3,&mut cursor));assert_eq!(grid,original);
  cursor=None;assert!(take_output(&mut grid,3,&mut cursor));assert_eq!(cursor,Some(cake.output));
  assert_eq!(grid.iter().flatten().filter(|s|s.block==i::BUCKET).count(),3);
 }
 #[test]fn metadata_variants_are_not_interchangeable(){
  let grid=vec![Some(Stack::new(b::WOOL,1)),Some(Stack::with_variant(i::DYE,1,1)),None,None];
  let red=recipe(&grid,2).unwrap().output;assert_eq!(red.block,ItemId::from(b::WOOL));assert_eq!(red.variant,14);
  let mut blue=grid;blue[1]=Some(Stack::with_variant(i::DYE,1,4));assert_eq!(recipe(&blue,2).unwrap().output.variant,11);
 }
 #[test]fn three_by_three_recipes_do_not_fit_hand_grid(){
  let chest=RECIPES.iter().find(|r|r.output.block==ItemId::from(b::CHEST)).unwrap();
  let grid=grid_for(chest,3,false,0,0);assert!(recipe(&grid[..4],2).is_none_or(|r|r.output.block!=ItemId::from(b::CHEST)));
 }
}
