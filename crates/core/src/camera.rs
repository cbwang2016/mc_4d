//! Player orientation in 4D with gravity locked to world Y.
//!
//! The heading is a 3D rotation acting on the horizontal space `(X, Z, W)`, stored as a
//! quaternion over the embedding `h = (X, Z, W)`. Its local axes are
//! `Vec3::X` = right, `Vec3::Y` = forward, `Vec3::Z` = ana. Pitch is a separate scalar that
//! only tilts the view inside the slice.

use glam::{Mat3, Quat, Vec3, Vec4};
use std::f32::consts::FRAC_PI_2;

const MAX_PITCH: f32 = 89.0 * std::f32::consts::PI / 180.0;

/// Maps a horizontal vector `(X, Z, W)` into world space `(X, Y, Z, W)`.
#[inline]
pub fn horiz_to_world(h: Vec3) -> Vec4 {
    Vec4::new(h.x, 0.0, h.y, h.z)
}

/// Orthonormal camera frame in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Basis {
    pub right: Vec4,
    pub up: Vec4,
    pub forward: Vec4,
    /// Slice normal. The visible slice is `{p : (p - eye)·ana = 0}`.
    pub ana: Vec4,
    /// Horizontal forward (forward without pitch).
    pub forward_h: Vec4,
}

#[derive(Clone, Copy, Debug)]
struct SnapAnim {
    target: Quat,
    /// Radians per second.
    speed: f32,
}

#[derive(Clone, Debug)]
pub struct Orientation {
    pub heading: Quat,
    pub pitch: f32,
    anim: Option<SnapAnim>,
    /// Seconds since the last manual slice rotation (used by auto-snap).
    idle_slice: f32,
}

impl Default for Orientation {
    fn default() -> Self {
        Self { heading: Quat::IDENTITY, pitch: 0.0, anim: None, idle_slice: 0.0 }
    }
}

impl Orientation {
    pub fn basis(&self) -> Basis {
        let right_h = horiz_to_world(self.heading * Vec3::X);
        let forward_h = horiz_to_world(self.heading * Vec3::Y);
        let ana = horiz_to_world(self.heading * Vec3::Z);
        let (s, c) = self.pitch.sin_cos();
        Basis {
            right: right_h,
            up: -s * forward_h + c * Vec4::Y,
            forward: c * forward_h + s * Vec4::Y,
            ana,
            forward_h,
        }
    }

    fn rotate_local(&mut self, axis: Vec3, angle: f32) {
        self.heading = (self.heading * Quat::from_axis_angle(axis, angle)).normalize();
    }

    /// Turn forward toward right (positive = turn right).
    pub fn yaw(&mut self, angle: f32) {
        self.rotate_local(Vec3::Z, -angle);
    }

    /// Tilt the view up (positive) or down. Never changes the slice.
    pub fn add_pitch(&mut self, angle: f32) {
        self.pitch = (self.pitch + angle).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Rotate forward toward ana (positive = forward turns into +ana).
    pub fn rotate_forward_ana(&mut self, angle: f32) {
        self.anim = None;
        self.idle_slice = 0.0;
        self.rotate_local(Vec3::X, angle);
    }

    /// Rotate right toward ana (positive = right turns into +ana).
    pub fn rotate_right_ana(&mut self, angle: f32) {
        self.anim = None;
        self.idle_slice = 0.0;
        self.rotate_local(Vec3::Y, -angle);
    }

    /// Start an animated snap that aligns the slice with the world axes, turning the slice
    /// normal onto the nearest axis by the shortest rotation (so the facing direction is
    /// kept as far as possible).
    pub fn snap(&mut self) {
        let target = slice_aligned(self.heading);
        self.anim = Some(SnapAnim { target, speed: 10.0 });
    }

    /// Start an animated 90° rotation in the forward↔ana plane.
    pub fn flip_ana(&mut self) {
        let base = match self.anim {
            Some(a) => a.target,
            None => nearest_aligned(self.heading),
        };
        let target = (base * Quat::from_axis_angle(Vec3::X, FRAC_PI_2)).normalize();
        self.anim = Some(SnapAnim { target, speed: 8.0 });
    }

    pub fn is_animating(&self) -> bool {
        self.anim.is_some()
    }

    /// Angle (radians) between the slice normal and the nearest world axis.
    pub fn misalignment(&self) -> f32 {
        let ana = self.heading * Vec3::Z;
        ana.abs().max_element().clamp(-1.0, 1.0).acos()
    }

    /// Advance animations. `auto_snap` eases into alignment after a pause in slice rotation
    /// when the slice is already close to aligned.
    pub fn update(&mut self, dt: f32, auto_snap: bool) {
        self.idle_slice += dt;
        if auto_snap && self.anim.is_none() && self.idle_slice > 1.0 {
            let m = self.misalignment();
            if m > 1e-4 && m < 12f32.to_radians() {
                self.anim = Some(SnapAnim { target: slice_aligned(self.heading), speed: 2.0 });
            }
        }
        if let Some(anim) = self.anim {
            let mut target = anim.target;
            if self.heading.dot(target) < 0.0 {
                target = -target;
            }
            let angle = self.heading.angle_between(target);
            let step = anim.speed * dt;
            if angle <= step.max(1e-4) {
                self.heading = target;
                self.anim = None;
            } else {
                self.heading = self.heading.slerp(target, step / angle).normalize();
            }
        }
    }
}

/// The 24 rotations mapping the horizontal axes onto signed horizontal axes.
pub fn aligned_rotations() -> Vec<Quat> {
    const PERMS: [[usize; 3]; 6] = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    let mut out = Vec::with_capacity(24);
    for p in PERMS {
        for signs in 0..8u32 {
            let s = |i: u32| if signs & (1 << i) != 0 { -1.0 } else { 1.0 };
            let col = |i: usize, sign: f32| {
                let mut v = Vec3::ZERO;
                v[p[i]] = sign;
                v
            };
            let m = Mat3::from_cols(col(0, s(0)), col(1, s(1)), col(2, s(2)));
            if m.determinant() > 0.0 {
                out.push(Quat::from_mat3(&m));
            }
        }
    }
    out
}

/// `q` turned by the smallest rotation that puts its ana axis on a signed world axis.
pub fn slice_aligned(q: Quat) -> Quat {
    let ana = q * Vec3::Z;
    let i = ana.abs().max_position();
    let mut axis = Vec3::ZERO;
    axis[i] = ana[i].signum();
    (Quat::from_rotation_arc(ana, axis) * q).normalize()
}

/// The nearest of the 24 fully axis-aligned orientations (facing direction included).
pub fn nearest_aligned(q: Quat) -> Quat {
    aligned_rotations()
        .into_iter()
        .max_by(|a, b| a.dot(q).abs().total_cmp(&b.dot(q).abs()))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_orthonormal(b: &Basis) {
        let v = [b.right, b.up, b.forward, b.ana];
        for i in 0..4 {
            assert!((v[i].length() - 1.0).abs() < 1e-5, "axis {i} not unit");
            for j in (i + 1)..4 {
                assert!(v[i].dot(v[j]).abs() < 1e-5, "axes {i},{j} not orthogonal");
            }
        }
    }

    fn scrambled() -> Orientation {
        let mut o = Orientation::default();
        o.yaw(0.7);
        o.rotate_forward_ana(0.4);
        o.rotate_right_ana(-1.1);
        o.add_pitch(0.3);
        o
    }

    #[test]
    fn identity_frame() {
        let b = Orientation::default().basis();
        assert_eq!(b.right, Vec4::X);
        assert_eq!(b.up, Vec4::Y);
        assert_eq!(b.forward, Vec4::Z);
        assert_eq!(b.ana, Vec4::W);
    }

    #[test]
    fn stays_orthonormal() {
        let mut o = scrambled();
        for i in 0..1000 {
            o.yaw(0.01 * i as f32);
            o.rotate_forward_ana(0.003);
            o.rotate_right_ana(0.007);
        }
        assert_orthonormal(&o.basis());
    }

    #[test]
    fn pitch_does_not_change_slice() {
        let mut o = scrambled();
        let ana = o.basis().ana;
        o.add_pitch(-0.9);
        assert!((o.basis().ana - ana).length() < 1e-6);
    }

    #[test]
    fn slice_always_contains_up() {
        let o = scrambled();
        let b = o.basis();
        assert_eq!(b.ana.y, 0.0);
        assert_eq!(b.right.y, 0.0);
        assert_eq!(b.forward_h.y, 0.0);
    }

    #[test]
    fn rotation_directions() {
        let before = Orientation::default().basis();
        let mut o = Orientation::default();
        o.yaw(0.1);
        assert!(o.basis().forward.dot(before.right) > 0.0);
        let mut o = Orientation::default();
        o.rotate_forward_ana(0.1);
        assert!(o.basis().forward.dot(before.ana) > 0.0);
        let mut o = Orientation::default();
        o.rotate_right_ana(0.1);
        assert!(o.basis().right.dot(before.ana) > 0.0);
    }

    #[test]
    fn there_are_24_aligned_rotations() {
        let r = aligned_rotations();
        assert_eq!(r.len(), 24);
        for (i, a) in r.iter().enumerate() {
            for b in &r[i + 1..] {
                assert!(a.dot(*b).abs() < 0.99, "duplicate rotation");
            }
        }
    }

    #[test]
    fn snap_reaches_axis_aligned() {
        let mut o = scrambled();
        o.snap();
        for _ in 0..200 {
            o.update(1.0 / 60.0, false);
        }
        assert!(!o.is_animating());
        let ana = o.basis().ana;
        assert!((ana.abs().max_element() - 1.0).abs() < 1e-4, "slice not aligned: {ana:?}");
    }

    #[test]
    fn snap_keeps_facing_direction() {
        let mut o = Orientation::default();
        o.yaw(0.5);
        assert!(o.misalignment() < 1e-5, "yaw alone keeps the slice aligned");
        let forward_before = o.basis().forward_h;
        o.rotate_forward_ana(0.3);
        o.snap();
        for _ in 0..200 {
            o.update(1.0 / 60.0, false);
        }
        assert!(o.misalignment() < 1e-4);
        assert!((o.basis().forward_h - forward_before).length() < 1e-3, "{:?}", o.basis().forward_h);
    }

    #[test]
    fn flip_swaps_forward_and_ana() {
        let mut o = Orientation::default();
        o.flip_ana();
        for _ in 0..200 {
            o.update(1.0 / 60.0, false);
        }
        let b = o.basis();
        assert!((b.forward - Vec4::W).length() < 1e-4);
        assert!((b.ana + Vec4::Z).length() < 1e-4);
    }
}
