//! Bounded transient Beta effects. Positions and velocities are four-dimensional;
//! renderer clips their finite thickness into each displayed slice.
use super::*;
use std::collections::VecDeque;
const LIMIT:usize=1024;
#[derive(Clone,Copy,PartialEq,Eq,Debug)]enum Kind{Debris,Explosion,Splash,Smoke,Flame,Lava,RedDust}
/// Beta samples 1000 random cells per tick within 16 blocks (World.randomDisplayUpdates).
/// Here they are drawn from the displayed slice (half a block either side along
/// ana), so a torch emits about as often as in the 3D original.
const DISPLAY_SAMPLES:usize=1000;
struct Particle{body:Body,previous:Vec4,age:u16,life:u16,radius:f32,uv:[f32;4],tile:u16,tint:[f32;3],kind:Kind}
pub(super)struct Particles{items:VecDeque<Particle>,fraction:f32,rng:u64,was_water:bool}
impl Default for Particles{fn default()->Self{Self{items:VecDeque::new(),fraction:0.0,rng:0x4d7061727469636c,was_water:false}}}
impl Particles{
 fn random(&mut self)->f32{self.rng^=self.rng<<13;self.rng^=self.rng>>7;self.rng^=self.rng<<17;(self.rng as u32)as f32/u32::MAX as f32}
 fn noise(&mut self)->Vec4{Vec4::new(self.random()*2.0-1.0,self.random()*2.0-1.0,self.random()*2.0-1.0,self.random()*2.0-1.0)}
 fn smoke(&mut self,pos:Vec4,scale:f32,tint:Option<[f32;3]>){
  let mut body=Body::with_size(pos,0.01,0.02);body.vel=self.noise()*0.02;
  let gray=self.random()*0.3;let life=((8.0/(self.random()*0.8+0.2))*scale)as u16;
  let radius=0.075*(self.random()*0.5+0.5)*2.0*scale;
  let (kind,tint)=match tint{Some(t)=>(Kind::RedDust,t),None=>(Kind::Smoke,[gray;3])};
  self.push(Particle{previous:pos,body,age:0,life:life.max(2),radius,uv:[0.0;4],tile:7,tint,kind});
 }
 fn flame(&mut self,pos:Vec4){
  let mut body=Body::with_size(pos,0.01,0.02);body.vel=self.noise()*0.01;
  let life=(8.0/(self.random()*0.8+0.2))as u16+4;let radius=0.1*(self.random()*0.5+0.5);
  self.push(Particle{previous:pos,body,age:0,life,radius,uv:[0.0;4],tile:48,tint:[1.0;3],kind:Kind::Flame});
 }
 fn lava(&mut self,pos:Vec4){
  let mut body=Body::with_size(pos,0.01,0.02);let n=self.noise();
  body.vel=Vec4::new(n.x,0.0,n.z,n.w)*1.6+Vec4::Y*(self.random()*0.4+0.05)*20.0;
  let life=(16.0/(self.random()*0.8+0.2))as u16;let radius=0.1*(self.random()*2.0+0.2);
  self.push(Particle{previous:pos,body,age:0,life,radius,uv:[0.0;4],tile:49,tint:[1.0;3],kind:Kind::Lava});
 }
 fn push(&mut self,p:Particle){if self.items.len()==LIMIT{self.items.pop_front();}self.items.push_back(p);}
 fn splash(&mut self,position:Vec4,velocity:Vec4,count:usize){for _ in 0..count.min(32){
  let offset=self.noise()*Vec4::new(0.5,0.0,0.5,0.5);let mut body=Body::with_size(position+offset,0.01,0.02);
  let direction=self.noise().normalize_or_zero();let speed=(self.random()+self.random()+1.0)*0.36;
  body.vel=direction*Vec4::new(speed,0.0,speed,speed)+Vec4::Y*(2.0+self.random()*4.0);
  if velocity.y==0.0 && Vec4::new(velocity.x,0.0,velocity.z,velocity.w).length_squared()>0.0 {body.vel=velocity+Vec4::Y*2.0;}
  let life=(8.0/(self.random()*0.8+0.2))as u16;let radius=0.1+self.random()*0.1;let tile=20+(self.random()*4.0)as u16%4;
  self.push(Particle{previous:body.pos,body,age:0,life,radius,uv:[0.0;4],tile,tint:[1.0;3],kind:Kind::Splash});
 }}
}
fn surface(world:&World,p:Vec4)->Option<f32>{let cell=p.floor().as_ivec4();for dy in [0,-1,-2]{let q=cell+IVec4::Y*dy;if let Some(id)=world.block(q).filter(|&id|block::is_water(id)){return Some(q.y as f32+block::fluid_height(id,world.block(q+IVec4::Y)));}}None}
impl Game{
 pub(super)fn emit_block_particles(&mut self,cell:IVec4,state:block::BlockState,burst:bool){
  if state.id==AIR||block::is_fluid(state.id){return;}
  let count=if burst{64}else{4};let basis=self.basis();let center=cell.as_vec4()+Vec4::splat(0.5);
  let center=if burst{center-basis.ana*(center-self.eye()).dot(basis.ana)}else{center};
  for i in 0..count{
   let grid=Vec4::new(((i%4)as f32+0.5)/4.0-0.5,(((i/4)%4)as f32+0.5)/4.0-0.5,(((i/16)%4)as f32+0.5)/4.0-0.5,0.0);
   let pos=(center+basis.right*grid.x+Vec4::Y*grid.y+basis.forward_h*grid.z).clamp(cell.as_vec4()+Vec4::splat(0.01),cell.as_vec4()+Vec4::splat(0.99));
   let direction=(pos-center+self.particles.noise()*0.4).normalize_or_zero();let speed=(self.particles.random()+self.particles.random()+1.0)*1.2;
   let mut body=Body::with_size(pos,0.03,0.06);body.vel=direction*speed+Vec4::Y*2.0;
   let tile=block::texture_tile(state,2);let u=(tile%16)as f32*16.0+self.particles.random()*12.0;let v=(tile/16)as f32*16.0+self.particles.random()*12.0;
   let life=(4.0/(self.particles.random()*0.9+0.1))as u16;let radius=0.05+self.particles.random()*0.05;
   let tint=if state.id==block::LEAVES{match state.metadata&3{1=>[0.6*0.38,0.6*0.6,0.6*0.38],2=>[0.6*0.5,0.6*0.66,0.6*0.3],_=>[0.6*0.4,0.6*0.7,0.6*0.25]}}else{[0.6;3]};
   self.particles.push(Particle{previous:pos,body,age:0,life,radius,uv:[u/256.0,v/256.0,(u+3.996)/256.0,(v+3.996)/256.0],tile,tint,kind:Kind::Debris});
  }
 }
 pub(super)fn emit_explosion_particles(&mut self,center:Vec4,radius:u8){for _ in 0..64{
  let direction=self.particles.noise().normalize_or_zero();let offset=direction*self.particles.random()*radius as f32;
  let mut body=Body::with_size(center+offset*0.5,0.03,0.06);body.vel=direction*(2.0+self.particles.random()*5.0);
  let gray=0.7+self.particles.random()*0.3;let size=0.1*(1.0+self.particles.random()*self.particles.random()*6.0);let life=(16.0/(self.particles.random()*0.8+0.2))as u16+2;
  self.particles.push(Particle{previous:body.pos,body,age:0,life,radius:size,uv:[0.0;4],tile:7,tint:[gray;3],kind:Kind::Explosion});
 }}
 /// Block.randomDisplayTick equivalents for torches, fire, lit furnaces, lava
 /// and powered redstone, sampled around the displayed slice.
 fn display_ticks(&mut self){
  let basis=self.basis();let eye=self.eye();
  for _ in 0..DISPLAY_SAMPLES{
   let r=&mut self.particles;
   let offset=basis.right*(r.random()*33.0-16.5)+Vec4::Y*(r.random()*33.0-16.5)+basis.forward_h*(r.random()*33.0-16.5)+basis.ana*(r.random()-0.5);
   let cell=(eye+offset).floor().as_ivec4();let Some(state)=self.world.block_state(cell) else{continue};
   let base=cell.as_vec4();
   match state.id{
    block::TORCH|block::REDSTONE_TORCH=>{
     // Wall torches lean away from their support: 0.27 sideways, 0.22 higher.
     let support=mc4d_core::shapes::torch_support(state.metadata);let mut p=base+Vec4::new(0.5,0.7,0.5,0.5);
     if support.y==0{p+=Vec4::Y*0.22-support.as_vec4()*0.27;}
     if state.id==block::TORCH{self.particles.smoke(p,1.0,None);self.particles.flame(p);}
     else{let n=self.particles.noise()*0.1;self.particles.smoke(p+n,0.75,Some([1.0,0.0,0.0]));}
    }
    block::FIRE=>{for _ in 0..3{let n=Vec4::new(self.particles.random(),self.particles.random()*0.5+0.5,self.particles.random(),self.particles.random());self.particles.smoke(base+n,2.5,None);}}
    block::LIT_FURNACE=>{
     let front=-mc4d_core::shapes::wall_support(state.metadata).as_vec4();if front==Vec4::ZERO{continue;}
     let side=if front.x!=0.0{Vec4::Z}else{Vec4::X};let along=self.particles.random()*0.6-0.3;let height=self.particles.random()*6.0/16.0;
     let p=base+Vec4::new(0.5,0.0,0.5,0.5)+front*0.52+side*along+Vec4::Y*height;
     self.particles.smoke(p,1.0,None);self.particles.flame(p);
    }
    block::REDSTONE_WIRE if state.metadata>0=>{
     if self.particles.random()*15.0<state.metadata as f32{
      let n=Vec4::new(self.particles.random(),0.06,self.particles.random(),self.particles.random());let f=state.metadata as f32/15.0;
      self.particles.smoke(base+n,0.75,Some([f*0.6+0.4,0.0,0.0]));
     }
    }
    id if block::fluid(id).is_some_and(|(kind,_)|kind==block::FluidKind::Lava)=>{
     if self.world.block(cell+IVec4::Y)==Some(AIR)&&self.particles.random()<0.01{
      let n=Vec4::new(self.particles.random(),1.0,self.particles.random(),self.particles.random());self.particles.lava(base+n);
     }
    }
    _=>{}
   }
  }
 }
 pub(super)fn update_particles(&mut self,dt:f32){
  if self.body.in_water&&!self.particles.was_water{if let Some(y)=surface(&self.world,self.body.pos){let mut p=self.body.pos;p.y=y+0.02;self.particles.splash(p,self.body.vel,16);}}
  self.particles.was_water=self.body.in_water;
  self.particles.fraction+=dt.clamp(0.0,0.1)*20.0;
  while self.particles.fraction>=1.0 {self.particles.fraction-=1.0;
   self.display_ticks();
   // Native fo wake is a trail of splash particles, emitted only while moving in water.
   for boat in self.vehicles.saved.entities.iter().filter(|v|v.kind==super::vehicles::VehicleKind::Boat){let speed=Vec4::new(boat.velocity.x,0.0,boat.velocity.z,boat.velocity.w).length();if speed>0.15{if let Some(y)=surface(&self.world,boat.position){let mut p=boat.position-boat.forward*0.7;p.y=y+0.02;self.particles.splash(p,boat.velocity*20.0,(1.0+speed*60.0)as usize);}}}
   for p in &mut self.particles.items {
    p.previous=p.body.pos;p.age=p.age.saturating_add(1);if p.age>=p.life{continue;}
    // Per-tick Beta motion in blocks/second: smoke rises 0.004/tick², lava falls 0.03.
    p.body.vel.y+=match p.kind{Kind::Explosion|Kind::Smoke|Kind::RedDust=>0.08,Kind::Flame=>0.0,Kind::Lava=>-0.6,_=>-0.8};
    p.body.step_states_options(0.05,&|q|self.world.block_state(q),0.0,false);
    p.body.vel*=match p.kind{Kind::Explosion=>0.9,Kind::Smoke|Kind::RedDust|Kind::Flame=>0.96,Kind::Lava=>0.999,_=>0.98};
    if p.body.on_ground{p.body.vel*=Vec4::new(0.7,1.0,0.7,0.7);if p.kind==Kind::Splash{p.age=p.life;}}
    if p.kind==Kind::Splash&&p.age>2&&p.body.vel.y<0.0{if let Some(y)=surface(&self.world,p.body.pos){if p.body.pos.y<y{p.age=p.life;}}}
   }
   // Falling lava sparks leave occasional smoke (EntityLavaFX).
   let sparks:Vec<Vec4>=self.particles.items.iter().filter(|p|p.kind==Kind::Lava&&p.age<p.life).map(|p|p.body.pos).collect();
   for pos in sparks{if self.particles.random()>0.85{self.particles.smoke(pos,1.0,None);}}
   self.particles.items.retain(|p|p.age<p.life&&p.body.pos.is_finite());
  }
 }
 pub fn particle_draws(&self)->Vec<mc4d_render::ParticleDraw>{self.particles.items.iter().map(|p|{
  let position=p.previous.lerp(p.body.pos,self.particles.fraction);let light=self.world.light_at(position.floor().as_ivec4()).unwrap_or((0,0));
  let tile=match p.kind{Kind::Explosion|Kind::Smoke|Kind::RedDust=>7u16.saturating_sub(p.age*8/p.life.max(1)),_=>p.tile};
  // Flame and lava particles are self-lit (getEntityBrightness = 1).
  let light=if matches!(p.kind,Kind::Flame|Kind::Lava){(15,15)}else{light};
  let age=p.age as f32/p.life.max(1) as f32;
  let radius=match p.kind{Kind::Flame=>p.radius*(1.0-age*age*0.5),Kind::Lava=>p.radius*(1.0-age*age),_=>p.radius};
  let uv=if p.kind==Kind::Debris{p.uv}else{let u=(tile%16)as f32*8.0;let v=(tile/16)as f32*8.0;[u/256.0,v/256.0,(u+7.99)/256.0,(v+7.99)/256.0]};
  mc4d_render::ParticleDraw{position,radius,thickness:radius.max(0.2),uv,color:p.tint,layer:if p.kind==Kind::Debris{0}else{28},light:[light.0,light.1]}
 }).collect()}
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn real_block_break_and_explosion_emit_bounded_transient_effects(){let mut game=Game::new(None,7,GameMode::Creative);let cell=game.body.pos.floor().as_ivec4()+IVec4::Z*2;game.world.set_block(cell,block::BRICK);game.mine_block(Hit{block:cell,normal:-IVec4::Z,dist:2.0},0.1);assert_eq!(game.particles.items.len(),64);assert!(game.particle_draws().iter().all(|p|p.layer==0&&p.uv[2]>p.uv[0]));game.explode(cell.as_vec4());assert!(game.particles.items.iter().any(|p|p.kind==Kind::Explosion));for _ in 0..100{game.emit_explosion_particles(cell.as_vec4(),4);}assert_eq!(game.particles.items.len(),LIMIT);for _ in 0..100{game.update_particles(0.05);}assert!(game.particles.items.is_empty());}
 #[test]fn moving_boat_emits_a_water_wake_but_idle_or_dry_boat_does_not(){
  let mut game=Game::menu_preview(7);game.body.pos=Vec4::new(8.5,66.0,4.5,8.5);game.world.insert_chunk(IVec4::new(0,4,0,0),mc4d_core::chunk::Chunk::uniform(AIR));
  for w in 4..12{for z in 4..12{for x in 4..12{game.world.set_block(IVec4::new(x,65,z,w),block::WATER);}}}
  game.hotbar[0]=Some(Slot::new(item::BOAT,1));game.orient.add_pitch(-0.65);assert!(game.use_vehicle_item());game.update_particles(0.05);assert!(game.particles.items.is_empty());
  game.vehicles.saved.entities[0].velocity=Vec4::Z*0.2;game.update_vehicles(0.05);game.update_particles(0.05);assert!(game.particles.items.iter().any(|p|p.kind==Kind::Splash));
  let count=game.particles.items.len();game.vehicles.saved.entities[0].position.y+=10.0;game.update_particles(0.05);assert_eq!(game.particles.items.len(),count);
 }
 #[test]fn torches_furnaces_fire_and_lava_emit_beta_display_particles(){
  let mut game=Game::menu_preview(7);game.body.pos=Vec4::new(8.5,66.0,4.5,8.5);
  for y in 3..6{game.world.insert_chunk(IVec4::new(0,y,0,0),mc4d_core::chunk::Chunk::uniform(AIR));}
  let eye=game.eye().floor().as_ivec4();let forward=game.basis().forward_h.round().as_ivec4();
  let torch=eye+forward*3;game.world.set_block_state(torch,block::BlockState::new(block::TORCH,5));
  let count=|game:&Game,kind:Kind|game.particles.items.iter().filter(|p|p.kind==kind).count();
  for _ in 0..200{game.update_particles(0.05);if count(&game,Kind::Flame)>0&&count(&game,Kind::Smoke)>0{break;}}
  assert!(count(&game,Kind::Flame)>0&&count(&game,Kind::Smoke)>0,"a standing torch emits smoke and a flame");
  let flame=game.particles.items.iter().find(|p|p.kind==Kind::Flame).unwrap().body.pos;
  assert!((flame-torch.as_vec4()).y>0.6&&(flame-torch.as_vec4()).y<0.8,"flame sits at the torch tip");
  assert!(game.particle_draws().iter().any(|d|d.light==[15,15]),"flames are self-lit");
  game.world.set_block(torch,AIR);game.particles.items.clear();
  let pool=eye+forward*2-IVec4::Y*3;for dx in -1..=1{for dw in -1..=1{game.world.set_block(pool+IVec4::new(dx,0,0,dw),block::LAVA);}}
  for _ in 0..2000{game.update_particles(0.05);if count(&game,Kind::Lava)>0{break;}}
  assert!(count(&game,Kind::Lava)>0,"open lava pops sparks");
  for _ in 0..400{game.update_particles(0.05);}
  for dx in -1..=1{for dw in -1..=1{game.world.set_block(pool+IVec4::new(dx,0,0,dw),AIR);}}
  for _ in 0..400{game.update_particles(0.05);}
  assert!(game.particles.items.is_empty(),"display particles expire");
 }
 #[test]fn entering_real_water_emits_splash_and_particles_expire(){let mut game=Game::new(None,7,GameMode::Creative);let cell=game.body.pos.floor().as_ivec4();game.world.set_block(cell,block::WATER);game.body.in_water=true;game.update_particles(0.01);assert!(game.particles.items.iter().any(|p|p.kind==Kind::Splash));let count=game.particles.items.len();game.update_particles(0.01);assert_eq!(game.particles.items.len(),count);for _ in 0..100{game.update_particles(0.05);}assert!(game.particles.items.is_empty());}
}
