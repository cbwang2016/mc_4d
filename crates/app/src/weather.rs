//! Nearby precipitation columns, rebuilt off the render thread from world snapshots.
//! Rain stops at roofs, leaves and water; climate selects rain, snow or dry columns.
use mc4d_core::{
    block,
    chunk::{CHUNK, Chunk, WORLD_CHUNKS_Y},
    glam::{IVec3, IVec4, Vec4},
    terrain::{Biome, TerrainGen},
    world::World,
};
use std::{
    collections::HashMap,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

pub const SIDE: i32 = 25;
pub const SNOW: i32 = 1 << 16;
pub const DRY: i32 = 1 << 17;
struct Request {
    origin: IVec3,
    chunks: HashMap<IVec4, Chunk>,
    terrain: Arc<TerrainGen>,
}
pub struct Columns {
    pub origin: IVec3,
    pub values: Vec<[i32; 4]>,
}
pub struct WeatherField {
    sender: Option<mpsc::SyncSender<Request>>,
    receiver: mpsc::Receiver<Columns>,
    worker: Option<thread::JoinHandle<()>>,
    requested: Option<(IVec3, u64)>,
    last_request: Instant,
}
impl WeatherField {
    pub fn new() -> Self {
        let (sender, requests) = mpsc::sync_channel::<Request>(1);
        let (results, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            while let Ok(request) = requests.recv() {
                if results.send(build(request)).is_err() {
                    break;
                }
            }
        });
        Self {
            sender: Some(sender),
            receiver,
            worker: Some(worker),
            requested: None,
            last_request: Instant::now() - Duration::from_secs(1),
        }
    }
    pub fn update(&mut self, world: &World, eye: Vec4, active: bool) -> Option<Columns> {
        let latest = self.receiver.try_iter().last();
        if !active || self.last_request.elapsed() < Duration::from_millis(250) {
            return latest;
        }
        let p = eye.floor().as_ivec4();
        let origin = IVec3::new(
            p.x.div_euclid(4) * 4 - 12,
            p.z.div_euclid(4) * 4 - 12,
            p.w.div_euclid(4) * 4 - 12,
        );
        if self.requested == Some((origin, world.revision)) {
            return latest;
        }
        let mut chunks = HashMap::new();
        for w in origin.z.div_euclid(CHUNK)..=(origin.z + SIDE - 1).div_euclid(CHUNK) {
            for z in origin.y.div_euclid(CHUNK)..=(origin.y + SIDE - 1).div_euclid(CHUNK) {
                for x in origin.x.div_euclid(CHUNK)..=(origin.x + SIDE - 1).div_euclid(CHUNK) {
                    for y in 0..WORLD_CHUNKS_Y {
                        let c = IVec4::new(x, y, z, w);
                        if let Some(chunk) = world.chunks.get(&c) {
                            chunks.insert(c, chunk.clone());
                        }
                    }
                }
            }
        }
        let request = Request {
            origin,
            chunks,
            terrain: world.terrain.clone(),
        };
        if self
            .sender
            .as_ref()
            .is_some_and(|sender| sender.try_send(request).is_ok())
        {
            self.requested = Some((origin, world.revision));
            self.last_request = Instant::now();
        }
        latest
    }
}
impl Drop for WeatherField {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn column_height(
    chunks: &HashMap<IVec4, Chunk>,
    terrain: &TerrainGen,
    x: i32,
    z: i32,
    w: i32,
) -> i32 {
    for cy in (0..WORLD_CHUNKS_Y).rev() {
        let c = IVec4::new(
            x.div_euclid(CHUNK),
            cy,
            z.div_euclid(CHUNK),
            w.div_euclid(CHUNK),
        );
        let Some(chunk) = chunks.get(&c) else {
            continue;
        };
        if chunk.uniform_id() == Some(block::AIR) {
            continue;
        }
        for y in (0..CHUNK).rev() {
            let id = chunk.get(IVec4::new(
                x.rem_euclid(CHUNK),
                y,
                z.rem_euclid(CHUNK),
                w.rem_euclid(CHUNK),
            ));
            if block::is_solid(id) || block::is_fluid(id) {
                return cy * CHUNK + y + 1;
            }
        }
    }
    terrain.height(x, z, w) + 1
}
fn build(request: Request) -> Columns {
    let mut values = Vec::with_capacity((SIDE * SIDE * SIDE) as usize);
    for w in 0..SIDE {
        for z in 0..SIDE {
            for x in 0..SIDE {
                let p = request.origin + IVec3::new(x, z, w);
                let height = column_height(&request.chunks, &request.terrain, p.x, p.y, p.z);
                let biome = request.terrain.biome(p.x, p.y, p.z);
                let flags = if biome == Biome::Desert {
                    DRY
                } else if biome == Biome::Taiga || height >= 92 {
                    SNOW
                } else {
                    0
                };
                let hash = |salt: u32| {
                    let mut h = (p.x as u32).wrapping_mul(73856093)
                        ^ (p.y as u32).wrapping_mul(83492791)
                        ^ (p.z as u32).wrapping_mul(2654435761)
                        ^ salt.wrapping_mul(374761393);
                    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
                    (h ^ (h >> 16)) & 65535
                };
                // Preserve the shader hash's exact sixteen-bit values, computed once per
                // column instead of four times per visited column of every screen ray.
                let phase =
                    (p.x.wrapping_mul(p.x)
                        .wrapping_mul(3121)
                        .wrapping_add(p.x.wrapping_mul(45238971))
                        .wrapping_add(p.y.wrapping_mul(p.y).wrapping_mul(418711))
                        .wrapping_add(p.y.wrapping_mul(13761))
                        .wrapping_add(p.z.wrapping_mul(p.z).wrapping_mul(72373))
                        .wrapping_add(p.z.wrapping_mul(87211)))
                        & 31;
                values.push([
                    height | flags,
                    (hash(31) | (hash(37) << 16)) as i32,
                    (hash(41) | (hash(53) << 16)) as i32,
                    phase,
                ]);
            }
        }
    }
    Columns {
        origin: request.origin,
        values,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precipitation_stops_on_player_roofs_and_water() {
        let terrain = TerrainGen::new(7);
        let mut chunks = HashMap::new();
        let mut chunk = Chunk::uniform(block::AIR);
        chunk.set(IVec4::new(1, 10, 2, 3), block::PLANKS);
        chunk.set(IVec4::new(2, 8, 2, 3), block::WATER);
        chunks.insert(IVec4::new(0, 5, 0, 0), chunk);
        assert_eq!(column_height(&chunks, &terrain, 1, 2, 3), 91);
        assert_eq!(column_height(&chunks, &terrain, 2, 2, 3), 89);
    }
}
