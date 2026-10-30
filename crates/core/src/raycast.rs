//! 4D DDA voxel traversal (Amanatides–Woo in four axes).
//! `crates/render/shaders/raycast.wgsl` implements the same algorithm on the GPU.

use glam::{IVec4, Vec4};
use crate::block::{self, BlockState};
use crate::shapes;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub block: IVec4,
    /// Unit axis vector pointing out of the face that was entered (zero if the ray starts
    /// inside a solid block).
    pub normal: IVec4,
    pub dist: f32,
}

/// Walks the grid from `origin` along `dir` (need not be normalized; distances are in units
/// of `|dir|`) until `solid` returns true or `max_dist` is exceeded.
pub fn raycast(origin: Vec4, dir: Vec4, max_dist: f32, mut solid: impl FnMut(IVec4) -> bool) -> Option<Hit> {
    let dir = dir.normalize_or_zero();
    if dir == Vec4::ZERO {
        return None;
    }
    let mut cell = origin.floor().as_ivec4();
    if solid(cell) {
        return Some(Hit { block: cell, normal: IVec4::ZERO, dist: 0.0 });
    }
    let step = IVec4::new(sgn(dir.x), sgn(dir.y), sgn(dir.z), sgn(dir.w));
    let inv = Vec4::new(inv(dir.x), inv(dir.y), inv(dir.z), inv(dir.w));
    let next_boundary = cell.as_vec4() + step.max(IVec4::ZERO).as_vec4();
    let mut t_max = Vec4::new(
        tmax(dir.x, next_boundary.x - origin.x, inv.x),
        tmax(dir.y, next_boundary.y - origin.y, inv.y),
        tmax(dir.z, next_boundary.z - origin.z, inv.z),
        tmax(dir.w, next_boundary.w - origin.w, inv.w),
    );
    let t_delta = inv.abs();
    loop {
        let axis = min_axis(t_max);
        let t = t_max[axis];
        if t > max_dist {
            return None;
        }
        cell[axis] += step[axis];
        t_max[axis] += t_delta[axis];
        if solid(cell) {
            let mut normal = IVec4::ZERO;
            normal[axis] = -step[axis];
            return Some(Hit { block: cell, normal, dist: t });
        }
    }
}

/// Ray against actual state-dependent selection geometry, not merely occupied
/// voxel cells. A ray can pass through the empty half of a slab or an open door.
pub fn raycast_states(origin:Vec4,dir:Vec4,max_dist:f32,get:&impl Fn(IVec4)->Option<BlockState>,target_fluids:bool)->Option<Hit>{
    if !origin.is_finite() || !dir.is_finite() || !max_dist.is_finite() || max_dist<0.0 { return None; }
    let dir=dir.normalize_or_zero();if dir==Vec4::ZERO{return None}
    let mut cell=origin.floor().as_ivec4();
    let step=IVec4::new(sgn(dir.x),sgn(dir.y),sgn(dir.z),sgn(dir.w));
    let inverse=Vec4::new(inv(dir.x),inv(dir.y),inv(dir.z),inv(dir.w));
    let boundary=cell.as_vec4()+step.max(IVec4::ZERO).as_vec4();
    let mut t_max=Vec4::new(tmax(dir.x,boundary.x-origin.x,inverse.x),tmax(dir.y,boundary.y-origin.y,inverse.y),tmax(dir.z,boundary.z-origin.z,inverse.z),tmax(dir.w,boundary.w-origin.w,inverse.w));
    let mut entered=0.0;
    loop{
        let axis=min_axis(t_max);let exit=t_max[axis];
        let state=get(cell)?;
        if state.id!=block::AIR && (target_fluids || !block::is_fluid(state.id)){
            let connected=if state.id==block::FENCE{shapes::connections(cell,get)}else{0};
            let boxes=shapes::selection_boxes(state,connected);let mut best:Option<Hit>=None;
            for bounds in boxes.as_slice(){
                if let Some((dist,normal))=bounds.translated(cell.as_vec4()).ray(origin,dir,max_dist)
                    && dist+1e-5>=entered && dist<=exit+1e-5 && best.is_none_or(|hit|dist<hit.dist){
                    best=Some(Hit{block:cell,normal,dist});
                }
            }
            if best.is_some(){return best}
        }
        if exit>max_dist{return None}
        cell[axis]+=step[axis];t_max[axis]+=inverse[axis].abs();entered=exit;
    }
}

fn sgn(v: f32) -> i32 {
    if v > 0.0 { 1 } else if v < 0.0 { -1 } else { 0 }
}

fn inv(v: f32) -> f32 {
    if v == 0.0 { f32::INFINITY } else { 1.0 / v }
}

fn tmax(d: f32, dist: f32, inv: f32) -> f32 {
    if d == 0.0 { f32::INFINITY } else { dist * inv }
}

fn min_axis(v: Vec4) -> usize {
    let mut a = 0;
    for i in 1..4 {
        if v[i] < v[a] {
            a = i;
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_wall_along_w() {
        let hit = raycast(Vec4::new(0.5, 0.5, 0.5, 0.5), Vec4::W, 10.0, |c| c.w == 3).unwrap();
        assert_eq!(hit.block, IVec4::new(0, 0, 0, 3));
        assert_eq!(hit.normal, IVec4::new(0, 0, 0, -1));
        assert!((hit.dist - 2.5).abs() < 1e-5);
    }

    #[test]
    fn diagonal_ray() {
        let d = Vec4::new(1.0, -1.0, 0.0, 1.0);
        let hit = raycast(Vec4::new(0.5, 5.5, 0.5, 0.5), d, 20.0, |c| c.y < 0).unwrap();
        assert_eq!(hit.block.y, -1);
        assert_eq!(hit.normal, IVec4::new(0, 1, 0, 0));
        let p = Vec4::new(0.5, 5.5, 0.5, 0.5) + d.normalize() * hit.dist;
        assert!(p.y.abs() < 1e-4);
    }

    #[test]
    fn misses_beyond_range() {
        assert!(raycast(Vec4::splat(0.5), Vec4::X, 5.0, |c| c.x == 10).is_none());
    }

    #[test]
    fn state_rays_select_slab_surface_and_pass_through_open_doorway() {
        let slab=|p:IVec4|Some(BlockState::new(if p==IVec4::ZERO{block::SLAB}else{block::AIR},0));
        assert!(raycast_states(Vec4::new(-1.0,0.75,0.5,0.5),Vec4::X,4.0,&slab,false).is_none());
        let hit=raycast_states(Vec4::new(0.5,2.0,0.5,0.5),-Vec4::Y,4.0,&slab,false).unwrap();
        assert!((hit.dist-1.5).abs()<1e-5);
        for (metadata,expected) in [(0,true),(4,false)] {
            let door=|p:IVec4|Some(BlockState::new(if p==IVec4::ZERO{block::WOOD_DOOR}else{block::AIR},metadata));
            assert_eq!(raycast_states(Vec4::new(-1.0,0.5,0.5,0.5),Vec4::X,4.0,&door,false).is_some(),expected);
        }
    }
}
