//! Player body: an axis-aligned 4D box that never rotates.

use crate::block::{self, BlockId, BlockState};
use crate::shapes::{self, Aabb4, ShapeBoxes};
use glam::{IVec4, Vec4};

pub const HALF_WIDTH: f32 = 0.3;
pub const HEIGHT: f32 = 1.8;
pub const EYE_HEIGHT: f32 = 1.62;
pub const GRAVITY: f32 = 28.0;
pub const JUMP_SPEED: f32 = 8.6;
const MAX_STEP: f32 = 0.25;
const EPS: f32 = 1e-3;

#[derive(Clone, Debug)]
pub struct Body {
    /// Center of the feet.
    pub pos: Vec4,
    pub vel: Vec4,
    /// Half extent along X, Z and W.
    pub half_width: f32,
    pub height: f32,
    pub on_ground: bool,
    pub in_water: bool,
}

impl Body {
    /// A player-sized body.
    pub fn new(pos: Vec4) -> Self {
        Self::with_size(pos, HALF_WIDTH, HEIGHT)
    }

    pub fn with_size(pos: Vec4, half_width: f32, height: f32) -> Self {
        Self { pos, vel: Vec4::ZERO, half_width, height, on_ground: false, in_water: false }
    }

    pub fn eye(&self) -> Vec4 {
        self.pos + Vec4::Y * EYE_HEIGHT
    }

    pub fn min(&self) -> Vec4 {
        self.pos - Vec4::new(self.half_width, 0.0, self.half_width, self.half_width)
    }

    pub fn max(&self) -> Vec4 {
        self.pos + Vec4::new(self.half_width, self.height, self.half_width, self.half_width)
    }

    /// Does the body overlap block `b`?
    pub fn overlaps_block(&self, b: IVec4) -> bool {
        let lo = b.as_vec4();
        let hi = lo + Vec4::ONE;
        (self.min() + EPS).cmplt(hi).all() && (self.max() - EPS).cmpgt(lo).all()
    }

    /// Move by `vel * dt` with collision. `block_at` returns `None` for unloaded space,
    /// which is treated as solid so the player can't fall out of the world.
    pub fn step(&mut self, dt: f32, block_at: &impl Fn(IVec4) -> Option<BlockId>) {
        let solid = |b: IVec4| block_at(b).is_none_or(block::is_solid);
        let delta = self.vel * dt;
        let n = (delta.abs().max_element() / MAX_STEP).ceil().max(1.0) as usize;
        let d = delta / n as f32;
        self.on_ground = false;
        for _ in 0..n {
            // Y first so walking off a ledge and landing resolve cleanly.
            for axis in [1, 0, 2, 3] {
                if d[axis] != 0.0 && self.move_axis(axis, d[axis], &solid) {
                    if axis == 1 && d[axis] < 0.0 {
                        self.on_ground = true;
                    }
                    self.vel[axis] = 0.0;
                }
            }
        }
        self.in_water = self.touches_fluid(block::FluidKind::Water,&|p|block_at(p).map(Into::into));
    }

    /// Returns true if movement along `axis` was blocked.
    fn move_axis(&mut self, axis: usize, d: f32, solid: &impl Fn(IVec4) -> bool) -> bool {
        self.pos[axis] += d;
        let lo = (self.min() + EPS).floor().as_ivec4();
        let hi = (self.max() - EPS).floor().as_ivec4();
        let mut blocked = false;
        for w in lo.w..=hi.w {
            for z in lo.z..=hi.z {
                for y in lo.y..=hi.y {
                    for x in lo.x..=hi.x {
                        if solid(IVec4::new(x, y, z, w)) {
                            blocked = true;
                        }
                    }
                }
            }
        }
        if blocked {
            // Snap flush against the face we ran into.
            let (lo_off, hi_off) = (self.pos[axis] - self.min()[axis], self.max()[axis] - self.pos[axis]);
            self.pos[axis] = if d > 0.0 {
                (self.max()[axis] - EPS).floor() - hi_off
            } else {
                (self.min()[axis] + EPS).floor() + 1.0 + lo_off
            };
        }
        blocked
    }

    /// Is the body intersecting any solid block right now?
    pub fn is_stuck(&self, block_at: &impl Fn(IVec4) -> Option<BlockId>) -> bool {
        let lo = (self.min() + EPS).floor().as_ivec4();
        let hi = (self.max() - EPS).floor().as_ivec4();
        for w in lo.w..=hi.w {
            for z in lo.z..=hi.z {
                for y in lo.y..=hi.y {
                    for x in lo.x..=hi.x {
                        if block_at(IVec4::new(x, y, z, w)).is_none_or(block::is_solid) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    pub fn bounds(&self) -> Aabb4 { Aabb4::new(self.min(), self.max()) }

    /// State-aware movement against shared partial-block shapes. Half-block
    /// stepping permits ordinary walking up slabs/stairs without climbing cubes.
    pub fn step_states(&mut self, dt: f32, get: &impl Fn(IVec4) -> Option<BlockState>) {
        self.step_states_options(dt, get, 0.5, false);
    }

    pub fn step_states_options(&mut self, dt: f32, get: &impl Fn(IVec4) -> Option<BlockState>, step_height: f32, avoid_edges: bool) {
        let grounded_before = self.on_ground;
        let delta = self.vel * dt;
        let n = (delta.abs().max_element() / MAX_STEP).ceil().max(1.0) as usize;
        let d = delta / n as f32;
        self.on_ground = false;
        for _ in 0..n {
            for axis in [1, 0, 2, 3] {
                let mut requested = d[axis];
                if requested == 0.0 { continue; }
                if axis != 1 && avoid_edges && (grounded_before || self.on_ground) {
                    let before = self.pos;
                    while requested.abs() > EPS {
                        self.pos[axis] = before[axis] + requested;
                        let supported = self.clip_axis_states(1, -0.08, get) > -0.079;
                        self.pos = before;
                        if supported { break; }
                        requested = requested.signum() * (requested.abs() - 0.025).max(0.0);
                    }
                }
                let before = self.pos;
                let mut movement = self.clip_axis_states(axis, requested, get);
                if axis != 1 && (movement-requested).abs()>EPS && step_height>0.0
                    && (grounded_before || self.on_ground) && self.vel.y<=0.0 {
                    let rise = self.clip_axis_states(1, step_height, get);
                    self.pos.y += rise;
                    let stepped = self.clip_axis_states(axis, requested, get);
                    if rise>EPS && stepped.abs()>movement.abs()+EPS {
                        self.pos[axis] += stepped;
                        let fall = self.clip_axis_states(1, -rise, get);
                        self.pos.y += fall;
                        self.on_ground = true;
                        movement = stepped;
                    } else {
                        self.pos = before;
                        self.pos[axis] += movement;
                    }
                } else { self.pos[axis] += movement; }
                if (movement-requested).abs()>EPS {
                    if axis==1 && requested<0.0 { self.on_ground=true; }
                    self.vel[axis]=0.0;
                }
            }
        }
        if self.vel.y<=0.0 && self.clip_axis_states(1,-0.004,get)>-0.003 { self.on_ground=true; }
        self.in_water = self.touches_fluid(block::FluidKind::Water,get);
    }

    /// Contact covers the entire4D body, including X/Z/W edges, and clips the
    /// liquid volume to its real shallow-flow surface rather than a full cube.
    pub fn touches_fluid(&self,kind:block::FluidKind,get:&impl Fn(IVec4)->Option<BlockState>)->bool {
        let bounds=self.bounds();let lo=(bounds.min+Vec4::splat(0.0001)).floor().as_ivec4();let hi=(bounds.max-Vec4::splat(0.0001)).floor().as_ivec4();
        for w in lo.w..=hi.w{for z in lo.z..=hi.z{for y in lo.y..=hi.y{for x in lo.x..=hi.x{
            let p=IVec4::new(x,y,z,w);let Some(s)=get(p)else{continue};
            if block::fluid(s.id).is_some_and(|(k,_)|k==kind)&&bounds.min.y<(y as f32+block::fluid_height(s.id,get(p+IVec4::Y).map(|s|s.id)))&&bounds.max.y>y as f32{return true;}
        }}}}false
    }

    fn clip_axis_states(&self, axis: usize, distance: f32, get: &impl Fn(IVec4) -> Option<BlockState>) -> f32 {
        if distance==0.0 { return 0.0; }
        let body=self.bounds();let mut area=body;
        if distance>0.0 { area.max[axis]+=distance; } else { area.min[axis]+=distance; }
        let mut clipped=distance;
        visit_obstacles(area,get,|obstacle| {
            if (0..4).any(|other|other!=axis && (body.max[other]<=obstacle.min[other]+EPS || body.min[other]>=obstacle.max[other]-EPS)) { return; }
            if distance>0.0 && body.max[axis]<=obstacle.min[axis]+EPS {
                clipped=clipped.min((obstacle.min[axis]-body.max[axis]).max(0.0));
            } else if distance<0.0 && body.min[axis]>=obstacle.max[axis]-EPS {
                clipped=clipped.max((obstacle.max[axis]-body.min[axis]).min(0.0));
            }
        });
        clipped
    }

    pub fn is_stuck_states(&self,get:&impl Fn(IVec4)->Option<BlockState>)->bool {
        let body=Aabb4::new(self.min()+EPS,self.max()-EPS);let mut hit=false;
        visit_obstacles(body,get,|obstacle|hit|=body.overlaps(obstacle));hit
    }

    pub fn overlaps_state(&self,pos:IVec4,state:BlockState,connections:u8)->bool {
        let body=Aabb4::new(self.min()+EPS,self.max()-EPS);
        shapes::collision_boxes(state,connections).as_slice().iter().any(|b|body.overlaps(b.translated(pos.as_vec4())))
    }

    pub fn ladder(&self,get:&impl Fn(IVec4)->Option<BlockState>)->Option<BlockState> {
        [0.05,0.8].into_iter().filter_map(|y|get((self.pos+Vec4::Y*y).floor().as_ivec4())).find(|s|s.id==block::LADDER)
    }
}

fn visit_obstacles(area:Aabb4,get:&impl Fn(IVec4)->Option<BlockState>,mut visit:impl FnMut(Aabb4)) {
    let mut lo=(area.min-EPS).floor().as_ivec4();let hi=(area.max+EPS).floor().as_ivec4();
    lo.y-=1; // Beta fences collide up to half a block above their cell.
    for w in lo.w..=hi.w {for z in lo.z..=hi.z {for y in lo.y..=hi.y {for x in lo.x..=hi.x {
        let pos=IVec4::new(x,y,z,w);
        let boxes=match get(pos) {
            None=>ShapeBoxes::one(Aabb4::UNIT),
            Some(state)=>shapes::collision_boxes(state,0),
        };
        for b in boxes.as_slice() { visit(b.translated(pos.as_vec4())); }
    }}}}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor_world(b: IVec4) -> Option<BlockId> {
        Some(if b.y < 10 || b.w == 5 { block::STONE } else { block::AIR })
    }

    #[test]
    fn shallow_water_and_lava_contact_include_body_edges_but_not_air_above_surface(){
        for kind in [block::FluidKind::Water,block::FluidKind::Lava] {for axis in [0,2,3] {
            let mut cell=IVec4::ZERO;cell[axis]=1;let get=|p|Some(BlockState::new(if p==cell{kind.flowing(7)}else{block::AIR},0));
            let mut body=Body::new(Vec4::new(0.5,0.08,0.5,0.5));body.pos[axis]=0.9;
            assert!(body.touches_fluid(kind,&get),"edge contact in axis{axis} must count");body.pos.y=0.13;assert!(!body.touches_fluid(kind,&get),"body above1/9-height flow stays dry");
        }}
    }

    #[test]
    fn lands_on_floor() {
        let mut body = Body::new(Vec4::new(0.5, 15.0, 0.5, 0.5));
        for _ in 0..200 {
            body.vel.y -= GRAVITY / 60.0;
            body.step(1.0 / 60.0, &floor_world);
        }
        assert!(body.on_ground);
        assert!((body.pos.y - 10.0).abs() < 1e-2, "y = {}", body.pos.y);
    }

    #[test]
    fn blocked_by_wall_in_w() {
        let mut body = Body::new(Vec4::new(0.5, 10.0, 0.5, 0.5));
        body.vel.w = 10.0;
        for _ in 0..60 {
            body.step(1.0 / 60.0, &floor_world);
        }
        assert!(body.max().w <= 5.0 + 1e-3, "w max = {}", body.max().w);
        assert!(!body.is_stuck(&floor_world));
    }

    #[test]
    fn walks_up_half_slabs_and_stairs_but_not_full_blocks() {
        for (id,expected) in [(block::SLAB,0.5),(block::WOOD_STAIRS,1.0),(block::STONE,0.0)] {
            let world=|p:IVec4|Some(BlockState::new(if p.y<0{block::STONE}else if p==IVec4::new(1,0,0,0){id}else{block::AIR},0));
            let mut body=Body::new(Vec4::new(0.5,0.0,0.5,0.5));body.on_ground=true;
            for _ in 0..36 {body.vel.x=2.0;body.vel.y-=GRAVITY/60.0;body.step_states(1.0/60.0,&world);}
            assert!((body.pos.y-expected).abs()<0.01,"block{id} at {:?}",body.pos);
            if id==block::STONE{assert!(body.max().x<=1.001)}else{assert!(body.pos.x>1.5)}
            assert!(!body.is_stuck_states(&world));
        }
    }

    #[test]
    fn opening_a_door_changes_collision_in_x_and_w() {
        for axis in [0,3] {for open in [false,true] {
            let metadata=if axis==3{16}else{0}|if open{4}else{0};
            let world=|p:IVec4|Some(BlockState::new(if p.y<0{block::STONE}else if p[axis]==1&&(0..2).contains(&p.y)&&p.z==0&&p[if axis==0{3}else{0}]==0{block::WOOD_DOOR}else{block::AIR},metadata));
            let mut body=Body::new(Vec4::new(0.5,0.0,0.5,0.5));body.on_ground=true;
            for _ in 0..45 {body.vel[axis]=2.0;body.vel.y-=GRAVITY/60.0;body.step_states(1.0/60.0,&world);}
            if open{assert!(body.pos[axis]>1.5)}else{assert!(body.max()[axis]<=1.001)}
        }}
    }

    #[test]
    fn sneaking_keeps_support_at_an_edge() {
        let world=|p:IVec4|Some(BlockState::new(if p.y<0&&p.x<1{block::STONE}else{block::AIR},0));
        let mut body=Body::new(Vec4::new(0.5,0.0,0.5,0.5));body.on_ground=true;
        for _ in 0..90 {body.vel.x=4.0;body.vel.y-=GRAVITY/60.0;body.step_states_options(1.0/60.0,&world,0.5,true);}
        assert!(body.min().x<1.0);assert!(body.pos.y>=-0.001);
    }
}
