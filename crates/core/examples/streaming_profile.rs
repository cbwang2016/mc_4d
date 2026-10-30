//! CPU-only footprint timings; does not certify gameplay FPS.
use mc4d_core::{
    camera::Orientation,
    glam::Vec4,
    world::{
        SliceNeighborhood, chunks_near_slices, chunks_visible_in_slices, skylight_column_order,
    },
};
use std::{hint::black_box, time::Instant};
fn main() {
    let eye = Vec4::new(3.7, 61.0, -7.4, 25.1);
    let mut o = Orientation::default();
    o.rotate_forward_ana(0.73);
    o.rotate_right_ana(0.36);
    for count in [2, 4, 6, 9] {
        let normals: Vec<_> = mc4d_core::views::slice_bases(o.basis(), count)
            .iter()
            .map(|b| b.ana)
            .collect();
        for _ in 0..3 {
            let t = Instant::now();
            let prefetch = chunks_near_slices(eye, &normals, 80.0, 20.0);
            let a = t.elapsed();
            let t = Instant::now();
            let visible = chunks_visible_in_slices(eye, &normals, 80.0, 2.0);
            let b = t.elapsed();
            let t = Instant::now();
            let order = skylight_column_order(&visible, &prefetch);
            let c = t.elapsed();
            let t = Instant::now();
            let keep = chunks_near_slices(eye, &normals, 112.0, 48.0);
            let d = t.elapsed();
            let t = Instant::now();
            let region = SliceNeighborhood::new(eye, &normals, 112.0, 48.0);
            let retained = prefetch.iter().filter(|&&c| region.contains(c)).count();
            let e = t.elapsed();
            println!(
                "views={count} prefetch={:.3}ms visible={:.3}ms order={:.3}ms keep={:.3}ms predicate={:.3}ms counts={}/{}/{}/{}",
                a.as_secs_f64() * 1000.0,
                b.as_secs_f64() * 1000.0,
                c.as_secs_f64() * 1000.0,
                d.as_secs_f64() * 1000.0,
                e.as_secs_f64() * 1000.0,
                prefetch.len(),
                visible.len(),
                order.len(),
                keep.len()
            );
            black_box((prefetch, visible, order, keep, retained));
        }
    }
}
