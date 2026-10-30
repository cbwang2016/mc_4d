//! Original m.f() star quad generator (java.util.Random seed10842), rasterized
//! once to a cube map so every sky ray avoids a1500-element search.
use mc4d_core::glam::DVec3;
pub const SIDE: u32 = 1024;
struct JavaRandom(u64);
impl JavaRandom {
    fn new(seed: u64) -> Self {
        Self((seed ^ 0x5deece66d) & ((1 << 48) - 1))
    }
    fn bits(&mut self, n: u32) -> u32 {
        self.0 = (self.0.wrapping_mul(0x5deece66d).wrapping_add(11)) & ((1 << 48) - 1);
        (self.0 >> (48 - n)) as u32
    }
    fn float(&mut self) -> f64 {
        self.bits(24) as f64 / (1u32 << 24) as f64
    }
    fn double(&mut self) -> f64 {
        (((self.bits(26) as u64) << 27) + self.bits(27) as u64) as f64 / (1u64 << 53) as f64
    }
}
pub fn quads() -> Vec<[DVec3; 4]> {
    let mut rng = JavaRandom::new(10842);
    let mut result = Vec::new();
    for _ in 0..1500 {
        let mut d = DVec3::new(
            (rng.float() as f32 * 2.0 - 1.0) as f64,
            (rng.float() as f32 * 2.0 - 1.0) as f64,
            (rng.float() as f32 * 2.0 - 1.0) as f64,
        );
        let size = (0.25_f32 + rng.float() as f32 * 0.25) as f64;
        let length = d.length_squared();
        if !(0.01..1.0).contains(&length) {
            continue;
        }
        d /= length.sqrt();
        let center = d * 100.0;
        let yaw = d.x.atan2(d.z);
        let pitch = (d.x * d.x + d.z * d.z).sqrt().atan2(d.y);
        let angle = rng.double() * std::f64::consts::TAU;
        result.push(std::array::from_fn(|i| {
            let a = ((i & 2) as f64 - 1.0) * size;
            let b = (((i + 1) & 2) as f64 - 1.0) * size;
            let q = a * angle.cos() - b * angle.sin();
            let r = b * angle.cos() + a * angle.sin();
            let y = q * pitch.sin();
            let z = -q * pitch.cos();
            center
                + DVec3::new(
                    z * yaw.sin() - r * yaw.cos(),
                    y,
                    r * yaw.sin() + z * yaw.cos(),
                )
        }));
    }
    result
}
fn project(p: DVec3, face: usize) -> (f64, f64, f64) {
    match face {
        0 => (-p.z, -p.y, p.x),
        1 => (p.z, -p.y, -p.x),
        2 => (p.x, p.z, p.y),
        3 => (p.x, -p.z, -p.y),
        4 => (p.x, -p.y, p.z),
        _ => (-p.x, -p.y, -p.z),
    }
}
fn edge(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> f64 {
    (p.0 - a.0) * (b.1 - a.1) - (p.1 - a.1) * (b.0 - a.0)
}
pub fn pixels() -> Vec<u8> {
    let side = SIDE as usize;
    let mut image = vec![0; side * side * 6];
    for quad in quads() {
        let center = quad.iter().sum::<DVec3>() / 4.0;
        let dominant = center.abs().max_element();
        for face in 0..6 {
            if project(center, face).2 < dominant * 0.985 {
                continue;
            }
            let p = quad.map(|v| {
                let (x, y, z) = project(v, face);
                (
                    (x / z * 0.5 + 0.5) * SIDE as f64,
                    (y / z * 0.5 + 0.5) * SIDE as f64,
                )
            });
            let minx = p
                .iter()
                .map(|p| p.0)
                .fold(f64::INFINITY, f64::min)
                .floor()
                .max(0.0) as usize;
            let maxx = p
                .iter()
                .map(|p| p.0)
                .fold(f64::NEG_INFINITY, f64::max)
                .ceil()
                .min(SIDE as f64 - 1.0) as usize;
            let miny = p
                .iter()
                .map(|p| p.1)
                .fold(f64::INFINITY, f64::min)
                .floor()
                .max(0.0) as usize;
            let maxy = p
                .iter()
                .map(|p| p.1)
                .fold(f64::NEG_INFINITY, f64::max)
                .ceil()
                .min(SIDE as f64 - 1.0) as usize;
            for y in miny..=maxy {
                for x in minx..=maxx {
                    let sample = (x as f64 + 0.5, y as f64 + 0.5);
                    let inside = [(0, 1, 2), (0, 2, 3)].iter().any(|&(a, b, c)| {
                        let e = [
                            edge(p[a], p[b], sample),
                            edge(p[b], p[c], sample),
                            edge(p[c], p[a], sample),
                        ];
                        e.iter().all(|&v| v >= 0.0) || e.iter().all(|&v| v <= 0.0)
                    });
                    if inside {
                        image[face * side * side + y * side + x] = 255;
                    }
                }
            }
        }
    }
    image
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_stars_are_deterministic_and_sparse() {
        let q = quads();
        assert_eq!(q.len(), 780);
        let reference = DVec3::new(-53.544398699078286, 69.795714043366, 47.557135428659606);
        assert!(q[0][0].distance(reference) < 1e-10);
        assert_eq!(q, quads());
        assert!(
            q.iter()
                .flatten()
                .all(|v| (v.length() - 100.0).abs() < 0.01)
        );
        let p = pixels();
        for face in p.chunks_exact((SIDE * SIDE) as usize) {
            let lit = face.iter().filter(|&&v| v > 0).count();
            assert!((500..30000).contains(&lit), "{lit}");
        }
    }
}
