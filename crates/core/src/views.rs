//! Shared slice cameras and viewport layout for rendering, streaming and HUD placement.

use crate::camera::Basis;
use glam::Vec4;

pub const MAX_VIEWS: usize = 9;

pub fn valid_view_count(count: u32) -> u32 {
    match count { 1 | 2 | 4 | 6 | 9 => count, _ => 2 }
}

/// The primary slice followed by perpendicular and diagonal cuts through the same eye.
/// All rotations act in horizontal XZW space; every slice contains world Y.
pub fn slice_bases(b: Basis, count: u32) -> Vec<Basis> {
    let r = b.right;
    let f = b.forward_h;
    let a = b.ana;
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let frames = [
        (r, f, a), (a, f, -r), (r, a, -f),
        ((r + a) * k, f, (a - r) * k),
        ((r - a) * k, f, (a + r) * k),
        (r, (f + a) * k, (a - f) * k),
        (r, (f - a) * k, (a + f) * k),
        (a, (r + f) * k, (f - r) * k),
        (a, (f - r) * k, -(r + f) * k),
    ];
    let sin_pitch = b.forward.y;
    let cos_pitch = b.up.y;
    frames.into_iter().take(valid_view_count(count) as usize).map(|(right, forward_h, ana)| Basis {
        right, forward_h, ana,
        forward: forward_h * cos_pitch + Vec4::Y * sin_pitch,
        up: -forward_h * sin_pitch + Vec4::Y * cos_pitch,
    }).collect()
}

/// Rectangles in the caller's units. Two views preserve the original corner inset;
/// grids partition the output so each pixel traces exactly one view.
pub fn view_rects(size: [f32; 2], count: u32) -> Vec<[f32; 4]> {
    let [w, h] = size;
    let count = valid_view_count(count);
    if count <= 2 {
        let mut rects = vec![[0.0, 0.0, w, h]];
        if count == 2 {
            let iw = (w * 0.26).clamp(160.0, 420.0).min((w - 24.0).max(1.0));
            let ih = (iw * 0.75).min((h - 24.0).max(1.0));
            rects.push([(w - iw - 12.0).max(0.0), (h - ih - 12.0).max(0.0), iw, ih]);
        }
        return rects;
    }
    let cols = if count == 4 { 2 } else { 3 };
    let rows = count / cols;
    (0..count).map(|i| {
        let x = (i % cols) as f32 * w / cols as f32;
        let y = (i / cols) as f32 * h / rows as f32;
        [x, y, w / cols as f32, h / rows as f32]
    }).collect()
}

pub const VIEW_LABELS: [&str; MAX_VIEWS] = [
    "1: Play", "2: Side", "3: Forward / W", "4: Side +45°", "5: Side −45°",
    "6: Forward +45°", "7: Forward −45°", "8: Diagonal +45°", "9: Diagonal −45°",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Orientation;

    #[test]
    fn cameras_are_distinct_orthonormal_slices_after_rotation_and_pitch() {
        let mut o = Orientation::default();
        o.yaw(0.71);
        o.rotate_forward_ana(0.43);
        o.rotate_right_ana(-0.26);
        o.add_pitch(1.2);
        let views = slice_bases(o.basis(), 9);
        assert_eq!(views.len(), 9);
        for (i, b) in views.iter().enumerate() {
            let axes = [b.right, b.up, b.forward, b.ana];
            for (j, v) in axes.iter().enumerate() {
                assert!((v.length_squared() - 1.0).abs() < 1e-5);
                for other in axes.iter().skip(j + 1) { assert!(v.dot(*other).abs() < 1e-5); }
            }
            assert_eq!(b.ana.y, 0.0);
            for other in views.iter().skip(i + 1) { assert!(b.ana.dot(other.ana).abs() < 0.999); }
        }
        assert!((views[0].forward - o.basis().forward).length() < 1e-5);
        assert_eq!(views[1].right, o.basis().ana);
    }

    #[test]
    fn grid_covers_output_once_and_keeps_primary_at_top_left() {
        for count in [4, 6, 9] {
            let rects = view_rects([1920.0, 1080.0], count);
            assert_eq!(rects.len(), count as usize);
            assert_eq!(&rects[0][..2], &[0.0, 0.0]);
            for y in (0..1080).step_by(17) {
                for x in (0..1920).step_by(19) {
                    assert_eq!(rects.iter().filter(|r| x as f32 >= r[0] && (x as f32) < r[0]+r[2] && y as f32 >= r[1] && (y as f32) < r[1]+r[3]).count(), 1);
                }
            }
        }
    }
}
