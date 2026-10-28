//! Loaded chunks, block access and background chunk generation.

use crate::block::{AIR, BEDROCK, BlockId};
use crate::chunk::{CHUNK, Chunk, WORLD_CHUNKS_Y, chunk_of, local_of};
use crate::terrain::TerrainGen;
use glam::{IVec4, Vec4};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

/// Source of chunks that are not in memory: saved edits first, then the generator.
pub trait ChunkSource: Send + Sync {
    fn load(&self, c: IVec4) -> Option<Chunk>;
}

pub struct World {
    pub terrain: Arc<TerrainGen>,
    pub chunks: HashMap<IVec4, Chunk>,
    /// Bumped whenever any chunk is added, removed or edited.
    pub revision: u64,
    pending: HashSet<IVec4>,
    to_workers: Option<Sender<IVec4>>,
    from_workers: Receiver<(IVec4, Chunk)>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl World {
    pub fn new(seed: u32, saved: Option<Arc<dyn ChunkSource>>) -> Self {
        let terrain = Arc::new(TerrainGen::new(seed));
        let (to_workers, jobs) = channel::<IVec4>();
        let (results, from_workers) = channel();
        let jobs = Arc::new(Mutex::new(jobs));
        let n = thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(2).max(1);
        let workers = (0..n)
            .map(|i| {
                let jobs = jobs.clone();
                let results = results.clone();
                let terrain = terrain.clone();
                let saved = saved.clone();
                thread::Builder::new()
                    .name(format!("chunkgen-{i}"))
                    .spawn(move || {
                        loop {
                            let job = jobs.lock().unwrap().recv();
                            let Ok(c) = job else { break };
                            let chunk = saved.as_ref().and_then(|s| s.load(c)).unwrap_or_else(|| terrain.generate(c));
                            if results.send((c, chunk)).is_err() {
                                break;
                            }
                        }
                    })
                    .expect("spawn chunk worker")
            })
            .collect();
        Self {
            terrain,
            chunks: HashMap::new(),
            revision: 0,
            pending: HashSet::new(),
            to_workers: Some(to_workers),
            from_workers,
            workers,
        }
    }

    /// Block at `p`, or `None` if its chunk isn't loaded. Outside the vertical range the
    /// world is bedrock below and air above.
    pub fn block(&self, p: IVec4) -> Option<BlockId> {
        let c = chunk_of(p);
        if c.y < 0 {
            return Some(BEDROCK);
        }
        if c.y >= WORLD_CHUNKS_Y {
            return Some(AIR);
        }
        self.chunks.get(&c).map(|ch| ch.get(local_of(p)))
    }

    /// Set a block. Returns false if its chunk isn't loaded or it's outside the world.
    pub fn set_block(&mut self, p: IVec4, id: BlockId) -> bool {
        let c = chunk_of(p);
        if c.y < 0 || c.y >= WORLD_CHUNKS_Y {
            return false;
        }
        match self.chunks.get_mut(&c) {
            Some(ch) => {
                ch.set(local_of(p), id);
                self.revision += 1;
                true
            }
            None => false,
        }
    }

    pub fn is_loaded(&self, c: IVec4) -> bool {
        self.chunks.contains_key(&c)
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Queue generation of `wanted` chunks (in priority order), keeping at most
    /// `max_in_flight` jobs outstanding.
    pub fn request(&mut self, wanted: impl IntoIterator<Item = IVec4>, max_in_flight: usize) {
        let Some(tx) = &self.to_workers else { return };
        for c in wanted {
            if self.pending.len() >= max_in_flight {
                break;
            }
            if c.y < 0 || c.y >= WORLD_CHUNKS_Y || self.chunks.contains_key(&c) || self.pending.contains(&c) {
                continue;
            }
            if tx.send(c).is_ok() {
                self.pending.insert(c);
            }
        }
    }

    /// Generate a chunk synchronously (used at spawn).
    pub fn load_now(&mut self, c: IVec4, saved: Option<&dyn ChunkSource>) {
        if !self.chunks.contains_key(&c) && c.y >= 0 && c.y < WORLD_CHUNKS_Y {
            let chunk = saved.and_then(|s| s.load(c)).unwrap_or_else(|| self.terrain.generate(c));
            self.chunks.insert(c, chunk);
            self.revision += 1;
        }
    }

    /// Move finished chunks into the world. Returns their coordinates.
    pub fn receive(&mut self) -> Vec<IVec4> {
        let mut got = Vec::new();
        while let Ok((c, chunk)) = self.from_workers.try_recv() {
            self.pending.remove(&c);
            self.chunks.entry(c).or_insert(chunk);
            got.push(c);
        }
        if !got.is_empty() {
            self.revision += 1;
        }
        got
    }

    /// Drop chunks that `keep` rejects. Modified chunks are handed to `on_evict_modified`
    /// (for saving) before being dropped.
    pub fn unload(&mut self, keep: impl Fn(IVec4) -> bool, mut on_evict_modified: impl FnMut(IVec4, &Chunk)) {
        let before = self.chunks.len();
        self.chunks.retain(|&c, chunk| {
            let k = keep(c);
            if !k && chunk.modified {
                on_evict_modified(c, chunk);
            }
            k
        });
        if self.chunks.len() != before {
            self.revision += 1;
        }
    }

    /// Surface spawn point for horizontal position (x, z, w).
    pub fn spawn_point(&self, x: i32, z: i32, w: i32) -> Vec4 {
        let h = self.terrain.height(x, z, w).max(crate::terrain::SEA_LEVEL);
        Vec4::new(x as f32 + 0.5, h as f32 + 1.0, z as f32 + 0.5, w as f32 + 0.5)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        self.to_workers = None;
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

/// Chunks that should be loaded around the slice through `eye` with normal `ana`:
/// within `radius` blocks of the eye horizontally (in-slice) and close enough to the slice
/// hyperplane to intersect it, plus `margin` blocks. Sorted nearest-first.
pub fn chunks_near_slice(eye: Vec4, ana: Vec4, radius: f32, margin: f32) -> Vec<IVec4> {
    let half = CHUNK as f32 / 2.0;
    // Support of a tesseract along `ana`: how far the chunk reaches from its center.
    let reach = half * (ana.x.abs() + ana.y.abs() + ana.z.abs() + ana.w.abs()) + margin;
    let horiz = radius + half * 2.0;
    let center = chunk_of(eye.floor().as_ivec4());
    let r = (horiz / CHUNK as f32).ceil() as i32 + 1;
    let mut out = Vec::new();
    for w in -r..=r {
        for z in -r..=r {
            for x in -r..=r {
                for cy in 0..WORLD_CHUNKS_Y {
                    let c = IVec4::new(center.x + x, cy, center.z + z, center.w + w);
                    let mid = (c * CHUNK).as_vec4() + Vec4::splat(half);
                    let rel = mid - eye;
                    let off = rel.dot(ana);
                    if off.abs() > reach {
                        continue;
                    }
                    let in_plane = rel - ana * off;
                    let horiz_dist = Vec4::new(in_plane.x, 0.0, in_plane.z, in_plane.w).length();
                    if horiz_dist > horiz {
                        continue;
                    }
                    out.push(c);
                }
            }
        }
    }
    out.sort_by(|a, b| {
        let da = ((*a * CHUNK).as_vec4() + Vec4::splat(half) - eye).length_squared();
        let db = ((*b * CHUNK).as_vec4() + Vec4::splat(half) - eye).length_squared();
        da.total_cmp(&db)
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_slice_is_one_chunk_thick() {
        let eye = Vec4::new(8.0, 60.0, 8.0, 8.0);
        let set = chunks_near_slice(eye, Vec4::W, 64.0, 0.0);
        assert!(!set.is_empty());
        assert!(set.iter().all(|c| c.w == 0));
    }

    #[test]
    fn world_loads_and_edits() {
        let mut w = World::new(1, None);
        let c = IVec4::new(0, 3, 0, 0);
        w.load_now(c, None);
        let p = IVec4::new(1, 50, 2, 3);
        assert!(w.set_block(p, crate::block::GLOW));
        assert_eq!(w.block(p), Some(crate::block::GLOW));
        assert_eq!(w.block(IVec4::new(100, 50, 0, 0)), None);
    }

    #[test]
    fn background_generation() {
        let mut w = World::new(1, None);
        w.request([IVec4::new(0, 3, 0, 0), IVec4::new(1, 3, 0, 0)], 8);
        let mut got = 0;
        for _ in 0..500 {
            got += w.receive().len();
            if got == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(got, 2);
    }
}
