//! Player body: an axis-aligned 4D box that never rotates.

use crate::block::{self, BlockId};
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
        let feet = self.pos.floor().as_ivec4();
        self.in_water = block_at(feet) == Some(block::WATER)
            || block_at(feet + IVec4::Y) == Some(block::WATER);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor_world(b: IVec4) -> Option<BlockId> {
        Some(if b.y < 10 || b.w == 5 { block::STONE } else { block::AIR })
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
}
