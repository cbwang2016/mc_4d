//! Loaded chunks, block access and background chunk generation.

use crate::block::{self, AIR, BEDROCK, BlockId, BlockState};
use crate::chunk::{CHUNK, Chunk, WORLD_CHUNKS_Y, chunk_of, local_of};
use crate::fluid::{FluidBudget, FluidSimulator, FluidWork, FluidWorld};
use crate::lighting::{LightChunk, LightingEngine};
use crate::terrain::TerrainGen;
use glam::{IVec4, Vec4};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

static NEXT_WORLD_IDENTITY: AtomicU64 = AtomicU64::new(1);

/// Source of chunks that are not in memory: saved edits first, then the generator.
pub trait ChunkSource: Send + Sync {
    fn load(&self, c: IVec4) -> Option<Chunk>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockChange {
    pub position: IVec4,
    pub before: BlockState,
    pub after: BlockState,
}

/// Published once on insertion/replacement or eviction, never on ordinary block
/// edits. Consumers can activate chunks incrementally without rescanning the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkLifecycle {
    pub coordinate: IVec4,
    pub identity: u64,
    pub loaded: bool,
}

pub struct World {
    pub identity: u64,
    pub terrain: Arc<TerrainGen>,
    pub chunks: HashMap<IVec4, Chunk>,
    /// Bumped whenever any chunk is added, removed or edited.
    pub revision: u64,
    fluids: FluidSimulator,
    pending: HashSet<IVec4>,
    to_workers: Option<Sender<IVec4>>,
    from_workers: Receiver<(IVec4, Chunk)>,
    workers: Vec<thread::JoinHandle<()>>,
    change_subscribers: Vec<Sender<BlockChange>>,
    chunk_subscribers: Vec<Sender<ChunkLifecycle>>,
    lighting: LightingEngine,
    light_dirty: HashSet<IVec4>,
    light_unsubmitted: HashSet<IVec4>,
    light_submission_poll: VecDeque<IVec4>,
    light_waiting_columns: HashMap<IVec4, (usize, u64)>,
    light_blocked_columns: HashMap<IVec4, usize>,
    light_required_columns: HashMap<IVec4, u64>,
    light_chunks: HashMap<IVec4, LightChunk>,
    pub lighting_revision: u64,
}

impl World {
    pub fn new(seed: u32, saved: Option<Arc<dyn ChunkSource>>) -> Self {
        Self::versioned(seed as i64,0,saved)
    }

    pub fn versioned(seed:i64,version:u32,saved:Option<Arc<dyn ChunkSource>>)->Self {
        let terrain = Arc::new(TerrainGen::versioned(seed,version));
        let (to_workers, jobs) = channel::<IVec4>();
        let (results, from_workers) = channel();
        let jobs = Arc::new(Mutex::new(jobs));
        // Chunk generation shares memory bandwidth and package power with an
        // integrated GPU. Filling every CPU core causes visible rendering stalls
        // while travelling, even though generation itself is asynchronous.
        let n = thread::available_parallelism()
            .map_or(4, |n| n.get())
            .saturating_sub(2)
            .clamp(1, 4);
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
                            let started = std::time::Instant::now();
                            let chunk = saved
                                .as_ref()
                                .and_then(|s| s.load(c))
                                .unwrap_or_else(|| terrain.generate(c));
                            let duty = crate::background_duty();
                            if duty < 1.0 { thread::sleep(started.elapsed().mul_f32((1.0 - duty) / duty)); }
                            if results.send((c, chunk)).is_err() {
                                break;
                            }
                        }
                    })
                    .expect("spawn chunk worker")
            })
            .collect();
        Self {
            identity: NEXT_WORLD_IDENTITY.fetch_add(1, Ordering::Relaxed),
            terrain,
            chunks: HashMap::default(),
            revision: 0,
            fluids: FluidSimulator::default(),
            pending: HashSet::default(),
            to_workers: Some(to_workers),
            from_workers,
            workers,
            change_subscribers: Vec::new(),
            chunk_subscribers: Vec::new(),
            lighting: LightingEngine::new(),
            light_dirty: HashSet::default(),
            light_unsubmitted: HashSet::default(),
            light_submission_poll: VecDeque::new(),
            light_waiting_columns: HashMap::default(),
            light_blocked_columns: HashMap::default(),
            light_required_columns: HashMap::default(),
            light_chunks: HashMap::default(),
            lighting_revision: 0,
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

    pub fn block_state(&self, p: IVec4) -> Option<BlockState> {
        let c = chunk_of(p);
        if c.y < 0 {
            return Some(BEDROCK.into());
        }
        if c.y >= WORLD_CHUNKS_Y {
            return Some(AIR.into());
        }
        self.chunks
            .get(&c)
            .map(|chunk| chunk.get_state(local_of(p)))
    }

    /// Optional observers receive every actual edit, including fluid changes and
    /// metadata-only changes. With no subscribers there is no retained event queue.
    /// Includes currently loaded chunks once for a late subscriber. New chunks
    /// are sent in their actual insertion order; no periodic world scan is needed.
    pub fn subscribe_chunk_lifecycle(&mut self) -> Receiver<ChunkLifecycle> {
        let (sender, receiver) = channel();
        for (&coordinate, chunk) in &self.chunks {
            let _ = sender.send(ChunkLifecycle {
                coordinate,
                identity: chunk.identity,
                loaded: true,
            });
        }
        self.chunk_subscribers.push(sender);
        receiver
    }
    fn publish_chunk_lifecycle(&mut self, event: ChunkLifecycle) {
        self.chunk_subscribers
            .retain(|sender| sender.send(event).is_ok());
    }

    pub fn subscribe_changes(&mut self) -> Receiver<BlockChange> {
        let (sender, receiver) = channel();
        self.change_subscribers.push(sender);
        receiver
    }

    /// Set a block. Returns false if its chunk isn't loaded or it's outside the world.
    pub fn set_block(&mut self, p: IVec4, id: BlockId) -> bool {
        self.set_block_state(p, id.into())
    }

    pub fn set_block_state(&mut self, p: IVec4, state: BlockState) -> bool {
        let old_state = self.block_state(p);
        let old = old_state.map(|s| s.id);
        let id = state.id;
        let changed = self.set_state_without_notifications(p, state);
        let fluid_nearby = block::is_fluid(id)
            || old.is_some_and(block::is_fluid)
            || [
                IVec4::X,
                IVec4::NEG_X,
                IVec4::Y,
                IVec4::NEG_Y,
                IVec4::Z,
                IVec4::NEG_Z,
                IVec4::W,
                IVec4::NEG_W,
            ]
            .iter()
            .any(|&d| self.block(p + d).is_some_and(block::is_fluid));
        if changed && old_state != Some(state) && fluid_nearby {
            let delay = block::fluid(id)
                .or_else(|| old.and_then(block::fluid))
                .map_or(5, |(k, _)| k.tick_delay());
            self.fluids.notify_edit(p, delay);
        }
        changed
    }

    pub fn step_fluids(&mut self, dt: f32, budget: FluidBudget) -> FluidWork {
        let mut fluids = std::mem::take(&mut self.fluids);
        let work = fluids.advance(self, dt, budget);
        self.fluids = fluids;
        work
    }

    fn activate_fluid_chunks(&mut self, c: IVec4) {
        for d in [
            IVec4::ZERO,
            IVec4::X,
            IVec4::NEG_X,
            IVec4::Y,
            IVec4::NEG_Y,
            IVec4::Z,
            IVec4::NEG_Z,
            IVec4::W,
            IVec4::NEG_W,
        ] {
            let neighbor = c + d;
            if self
                .chunks
                .get(&neighbor)
                .is_some_and(|chunk| chunk.fluid_cells > 0)
            {
                self.fluids.activate_chunk(neighbor);
            }
        }
    }

    fn set_block_without_notifications(&mut self, p: IVec4, id: BlockId) -> bool {
        self.set_state_without_notifications(p, id.into())
    }

    fn set_state_without_notifications(&mut self, p: IVec4, state: BlockState) -> bool {
        let c = chunk_of(p);
        if c.y < 0 || c.y >= WORLD_CHUNKS_Y {
            return false;
        }
        match self.chunks.get_mut(&c) {
            Some(ch) => {
                let before = ch.get_state(local_of(p));
                let previous_light_version = ch.light_state_version;
                if ch.set_state(local_of(p), state) {
                    self.revision += 1;
                    if previous_light_version != ch.light_state_version {
                        self.mark_light_dirty(c);
                        self.lighting_revision = self.lighting_revision.wrapping_add(1);
                    }
                    let change = BlockChange {
                        position: p,
                        before,
                        after: state,
                    };
                    self.change_subscribers
                        .retain(|sender| sender.send(change).is_ok());
                }
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

    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    fn mark_light_dirty(&mut self, c: IVec4) {
        self.light_dirty.insert(c);
        if !self.light_unsubmitted.insert(c) {
            return;
        }
        self.light_submission_poll.push_back(c);
        let column = IVec4::new(c.x, 0, c.z, c.w);
        let waiting = self.light_waiting_columns.entry(column).or_default();
        waiting.0 += 1;
        if waiting.0 == 1 {
            // Keep dependencies for unloaded neighboring columns too: a column
            // loaded before this deferred snapshot is sent must not appear ready.
            for dw in -1..=1 {
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        *self
                            .light_blocked_columns
                            .entry(column + IVec4::new(dx, 0, dz, dw))
                            .or_default() += 1;
                    }
                }
            }
        }
        self.lighting_revision = self.lighting_revision.wrapping_add(1);
    }

    fn acknowledge_light_submissions(&mut self) {
        let count = self.light_submission_poll.len().min(512);
        for _ in 0..count {
            let Some(c) = self.light_submission_poll.pop_front() else {
                break;
            };
            let Some(serial) = self.lighting.submitted_serial(c, self.chunks.get(&c)) else {
                self.light_submission_poll.push_back(c);
                continue;
            };
            self.light_unsubmitted.remove(&c);
            if !self.chunks.contains_key(&c) {
                self.lighting.forget_removed_submission(c);
            }
            let column = IVec4::new(c.x, 0, c.z, c.w);
            let waiting = self
                .light_waiting_columns
                .get_mut(&column)
                .expect("pending light column");
            waiting.0 -= 1;
            waiting.1 = waiting.1.max(serial);
            if waiting.0 != 0 {
                continue;
            }
            let serial = waiting.1;
            self.light_waiting_columns.remove(&column);
            for y in 0..WORLD_CHUNKS_Y {
                let c = IVec4::new(column.x, y, column.z, column.w);
                if !self.chunks.contains_key(&c) {
                    self.lighting.forget_removed_submission(c);
                }
            }
            for dw in -1..=1 {
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let affected = column + IVec4::new(dx, 0, dz, dw);
                        if let Some(count) = self.light_blocked_columns.get_mut(&affected) {
                            *count -= 1;
                            if *count == 0 {
                                self.light_blocked_columns.remove(&affected);
                            }
                        }
                        let required = self.light_required_columns.entry(affected).or_default();
                        *required = (*required).max(serial);
                    }
                }
            }
            self.lighting_revision = self.lighting_revision.wrapping_add(1);
        }
    }

    /// Insert or replace loaded geometry, invalidating derived lighting. Tests and
    /// tools should use this rather than editing the public chunk map directly.
    pub fn insert_chunk(&mut self, c: IVec4, chunk: Chunk) {
        let invalidate_light = self
            .light_chunks
            .get(&c)
            .is_some_and(|light| !light.matches(&chunk));
        let identity = chunk.identity;
        self.chunks.insert(c, chunk);
        self.publish_chunk_lifecycle(ChunkLifecycle {
            coordinate: c,
            identity,
            loaded: true,
        });
        self.revision += 1;
        self.mark_light_dirty(c);
        if invalidate_light {
            self.light_chunks.remove(&c);
            self.lighting_revision += 1;
        }
        self.activate_fluid_chunks(c);
    }

    /// Submit a bounded number of cheap COW snapshots and receive bounded light
    /// results. This also runs while a world is loading or the game is paused.
    pub fn update_lighting(&mut self, priority: &[IVec4], result_budget: usize) {
        self.lighting.flush();
        self.acknowledge_light_submissions();
        let mut changed = false;
        for _ in 0..result_budget {
            let Some((c, light)) = self.lighting.poll() else {
                break;
            };
            if self
                .chunks
                .get(&c)
                .is_some_and(|chunk| light.matches(chunk))
                && self.light_chunks.get(&c).is_none_or(|old| {
                    old.version < light.version
                        || (!old.settled && light.settled)
                        || old.settled_serial < light.settled_serial
                        || old.geometry_identity != light.geometry_identity
                })
            {
                self.light_chunks.insert(c, light);
                changed = true;
            }
        }
        if changed {
            self.lighting_revision = self.lighting_revision.wrapping_add(1);
        }
        if self.light_dirty.is_empty() {
            return;
        }
        // A skylight column must be seen as one snapshot. In particular an
        // evicted top must not be submitted before its also-evicted terrain:
        // that transient column would manufacture a huge sky-removal wave.
        let mut columns = Vec::with_capacity(4);
        for &c in priority {
            if self.light_dirty.contains(&c) {
                let column = IVec4::new(c.x, 0, c.z, c.w);
                if !columns.contains(&column) {
                    columns.push(column);
                }
                if columns.len() == 4 {
                    break;
                }
            }
        }
        if columns.len() < 4 {
            for &c in &self.light_dirty {
                let column = IVec4::new(c.x, 0, c.z, c.w);
                if !columns.contains(&column) {
                    columns.push(column);
                }
                if columns.len() == 4 {
                    break;
                }
            }
        }
        let mut coordinates = Vec::with_capacity(columns.len() * WORLD_CHUNKS_Y as usize);
        for column in columns {
            for y in (0..WORLD_CHUNKS_Y).rev() {
                let c = IVec4::new(column.x, y, column.z, column.w);
                self.light_dirty.remove(&c);
                coordinates.push(c);
            }
        }
        self.lighting.submit(
            coordinates
                .into_iter()
                .map(|c| (c, self.chunks.get(&c).cloned()))
                .collect(),
        );
    }

    pub fn light_chunk(&self, c: IVec4) -> Option<&LightChunk> {
        self.light_chunks.get(&c)
    }
    pub fn light_at(&self, p: IVec4) -> Option<(u8, u8)> {
        if p.y < 0 {
            return Some((0, 0));
        }
        if p.y >= crate::chunk::WORLD_HEIGHT {
            return Some((0, 15));
        }
        self.light_chunks
            .get(&chunk_of(p))
            .map(|light| light.get(local_of(p)))
    }
    pub fn fresh_light_at(&self, p: IVec4) -> Option<(u8, u8)> {
        let c = chunk_of(p);
        if !self.light_ready(c) {
            return None;
        }
        self.light_chunks
            .get(&c)
            .map(|light| light.get(local_of(p)))
    }
    /// A local certificate requires both geometry provenance and quiescence of
    /// every pending light source that can reach this chunk. Global offscreen
    /// work remains observable through lighting_pending(), but is independent.
    pub fn light_ready(&self, c: IVec4) -> bool {
        let column = IVec4::new(c.x, 0, c.z, c.w);
        if self.light_blocked_columns.contains_key(&column) {
            return false;
        }
        let Some(chunk) = self.chunks.get(&c) else {
            return false;
        };
        self.light_chunks.get(&c).is_some_and(|light| {
            light.matches(chunk)
                && light.settled
                && light.sky_known
                && light.settled_serial
                    >= self
                        .light_required_columns
                        .get(&column)
                        .copied()
                        .unwrap_or(0)
        })
    }
    pub fn lighting_stages(&self) -> [usize; 4] {
        self.lighting.stages()
    }
    pub fn lighting_pending(&self) -> usize {
        self.light_dirty.len() + self.lighting.pending()
    }
    pub fn missing_lighting(&self, wanted: &[IVec4]) -> usize {
        wanted.iter().filter(|&&c| !self.light_ready(c)).count()
    }
    pub fn lighting_ready(&self, wanted: &[IVec4]) -> bool {
        self.missing_lighting(wanted) == 0
    }

    /// Queue generation of `wanted` chunks (in priority order), keeping at most
    /// `max_in_flight` jobs outstanding.
    pub fn request(&mut self, wanted: impl IntoIterator<Item = IVec4>, max_in_flight: usize) {
        let Some(tx) = &self.to_workers else { return };
        for c in wanted {
            if self.pending.len() >= max_in_flight {
                break;
            }
            if c.y < 0
                || c.y >= WORLD_CHUNKS_Y
                || self.chunks.contains_key(&c)
                || self.pending.contains(&c)
            {
                continue;
            }
            if tx.send(c).is_ok() {
                self.pending.insert(c);
            }
        }
    }

    /// Like `request`, resuming at `start`. Returns the new length of the
    /// already-resident prefix so per-frame calls avoid rescanning thousands
    /// of loaded chunks. Reset the cursor when the list or residency changes.
    pub fn request_from(&mut self, wanted: &[IVec4], start: usize, max_in_flight: usize) -> usize {
        let mut cursor = start.min(wanted.len());
        while cursor < wanted.len() {
            let c = wanted[cursor];
            if c.y < 0 || c.y >= WORLD_CHUNKS_Y || self.chunks.contains_key(&c) { cursor += 1; } else { break; }
        }
        self.request(wanted[cursor..].iter().copied(), max_in_flight);
        cursor
    }

    /// Generate a chunk synchronously (used at spawn).
    pub fn load_now(&mut self, c: IVec4, saved: Option<&dyn ChunkSource>) {
        if !self.chunks.contains_key(&c) && c.y >= 0 && c.y < WORLD_CHUNKS_Y {
            let chunk = saved
                .and_then(|s| s.load(c))
                .unwrap_or_else(|| self.terrain.generate(c));
            self.insert_chunk(c, chunk);
        }
    }

    /// Move finished chunks into the world. Returns their coordinates.
    pub fn receive(&mut self) -> Vec<IVec4> {
        let mut got = Vec::new();
        while let Ok((c, chunk)) = self.from_workers.try_recv() {
            self.pending.remove(&c);
            if !self.chunks.contains_key(&c) {
                self.insert_chunk(c, chunk);
            }
            got.push(c);
        }
        if !got.is_empty() {
            self.revision += 1;
        }
        got
    }

    /// Drop chunks that `keep` rejects. Modified chunks are handed to `on_evict_modified`
    /// (for saving) before being dropped. A false return preserves the dirty chunk.
    pub fn unload(
        &mut self,
        keep: impl Fn(IVec4) -> bool,
        mut on_evict_modified: impl FnMut(IVec4, &Chunk) -> bool,
    ) {
        let before = self.chunks.len();
        let mut removed = Vec::new();
        self.chunks.retain(|&c, chunk| {
            let k = keep(c);
            if !k && chunk.modified && !on_evict_modified(c, chunk) {
                // Failed persistence must not discard the only current copy.
                return true;
            }
            if !k {
                removed.push((c, chunk.identity));
            }
            k
        });
        for (c, identity) in removed {
            self.publish_chunk_lifecycle(ChunkLifecycle {
                coordinate: c,
                identity,
                loaded: false,
            });
            self.mark_light_dirty(c);
            if self.light_chunks.remove(&c).is_some() {
                self.lighting_revision += 1;
            }
        }
        if self.chunks.len() != before {
            // A completely unloaded column has no cached light to certify. A
            // later reload receives a newer FIFO input serial of its own.
            self.light_required_columns.retain(|c, _| {
                (0..WORLD_CHUNKS_Y).any(|y| self.chunks.contains_key(&IVec4::new(c.x, y, c.z, c.w)))
            });
            self.revision += 1;
        }
    }

    /// Surface spawn point for horizontal position (x, z, w).
    pub fn spawn_point(&self, x: i32, z: i32, w: i32) -> Vec4 {
        let h = self.terrain.height(x, z, w).max(crate::terrain::SEA_LEVEL);
        Vec4::new(
            x as f32 + 0.5,
            h as f32 + 1.0,
            z as f32 + 0.5,
            w as f32 + 0.5,
        )
    }
}

impl FluidWorld for World {
    fn fluid_chunk_loaded(&self, c: IVec4) -> bool {
        self.is_loaded(c)
    }
    fn fluid_block(&self, p: IVec4) -> Option<BlockId> {
        self.block(p)
    }
    fn set_fluid_block(&mut self, p: IVec4, id: BlockId) -> bool {
        self.set_block_without_notifications(p, id)
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
    chunks_near_slices(eye, &[ana], radius, margin)
}

/// Exact membership in the same finite region as `chunks_near_slices`.
/// Eviction needs only membership for existing chunks, not an allocated and
/// distance-sorted enumeration of every possible chunk in a larger region.
pub struct SliceNeighborhood {
    eye: Vec4,
    center: IVec4,
    scan_radius: i32,
    horizontal_radius_squared: f32,
    planes: Vec<(Vec4, f32)>,
}
impl SliceNeighborhood {
    pub fn new(eye: Vec4, normals: &[Vec4], radius: f32, margin: f32) -> Self {
        let half = CHUNK as f32 * 0.5;
        let horizontal_radius = radius + half * 2.0;
        Self {
            eye,
            center: chunk_of(eye.floor().as_ivec4()),
            scan_radius: (horizontal_radius / CHUNK as f32).ceil() as i32 + 1,
            horizontal_radius_squared: horizontal_radius * horizontal_radius,
            planes: normals
                .iter()
                .map(|&n| (n, half * n.abs().element_sum() + margin))
                .collect(),
        }
    }
    pub fn contains(&self, c: IVec4) -> bool {
        if !(0..WORLD_CHUNKS_Y).contains(&c.y) {
            return false;
        }
        let delta = (c - self.center).abs();
        if delta.x > self.scan_radius || delta.z > self.scan_radius || delta.w > self.scan_radius {
            return false;
        }
        let rel = (c * CHUNK).as_vec4() + Vec4::splat(CHUNK as f32 * 0.5) - self.eye;
        self.planes.iter().any(|&(normal, reach)| {
            let off = rel.dot(normal);
            if off.abs() > reach {
                return false;
            }
            let p = rel - normal * off;
            Vec4::new(p.x, 0.0, p.z, p.w).length_squared() <= self.horizontal_radius_squared
        })
    }
}

/// Union of the visible slice neighborhoods, computed in one spatial scan and sorted
/// nearest-first. No duplicate chunks or repeated scans per viewport.
pub fn chunks_near_slices(eye: Vec4, normals: &[Vec4], radius: f32, margin: f32) -> Vec<IVec4> {
    let half = CHUNK as f32 / 2.0;
    // Support of a tesseract along `ana`: how far the chunk reaches from its center.
    let planes: Vec<_> = normals
        .iter()
        .map(|&ana| (ana, half * ana.abs().element_sum() + margin))
        .collect();
    let horiz = radius + half * 2.0;
    let center = chunk_of(eye.floor().as_ivec4());
    let r = (horiz / CHUNK as f32).ceil() as i32 + 1;
    let mut out = Vec::new();
    let horizontal = planes.iter().all(|(ana, _)| ana.y == 0.0);
    let intersects = |rel: Vec4| {
        planes.iter().any(|&(ana, reach)| {
            let off = rel.dot(ana);
            if off.abs() > reach {
                return false;
            }
            let in_plane = rel - ana * off;
            Vec4::new(in_plane.x, 0.0, in_plane.z, in_plane.w).length_squared() <= horiz * horiz
        })
    };
    for w in -r..=r {
        for z in -r..=r {
            for x in -r..=r {
                // Gameplay slices retain vertical Y. Their plane and horizontal
                // distance tests are identical in every vertical chunk, so do
                // the expensive multi-plane test once for the entire column.
                if horizontal {
                    let c = IVec4::new(center.x + x, 0, center.z + z, center.w + w);
                    if !intersects((c * CHUNK).as_vec4() + Vec4::splat(half) - eye) {
                        continue;
                    }
                    for cy in 0..WORLD_CHUNKS_Y {
                        out.push(IVec4::new(c.x, cy, c.z, c.w));
                    }
                    continue;
                }
                for cy in 0..WORLD_CHUNKS_Y {
                    let c = IVec4::new(center.x + x, cy, center.z + z, center.w + w);
                    let mid = (c * CHUNK).as_vec4() + Vec4::splat(half);
                    let rel = mid - eye;
                    if !intersects(rel) {
                        continue;
                    }
                    out.push(c);
                }
            }
        }
    }
    // Squared distances are nonnegative, so their IEEE bit order is numeric.
    // Cache once instead of recomputing two distances for every sort comparison.
    out.sort_by_cached_key(|c| {
        ((*c * CHUNK).as_vec4() + Vec4::splat(half) - eye)
            .length_squared()
            .to_bits()
    });
    out
}

/// Visible rays stop at `radius`. A 16^4 chunk has circumsphere radius16;
/// the extra margin covers ghost origins and the streaming-cache guard. Full
/// vertical columns are still loaded separately for authoritative skylight.
pub fn chunks_visible_in_slices(
    eye: Vec4,
    normals: &[Vec4],
    radius: f32,
    margin: f32,
) -> Vec<IVec4> {
    let bound = radius + CHUNK as f32 + margin.max(0.0);
    chunks_near_slices(eye, normals, radius + margin.max(0.0), margin)
        .into_iter()
        .filter(|&c| {
            let center = (c * CHUNK).as_vec4() + Vec4::splat(CHUNK as f32 * 0.5);
            (center - eye).length_squared() <= bound * bound
        })
        .collect()
}

/// Maximum displacement of a current ray from a cached slice footprint. All
/// directions are unit vectors; |dot(ray, old_normal-new_normal)| is bounded by
/// ray_length * |old_normal-new_normal|. Include ghost-origin distance in range.
pub fn slice_cache_motion_bound(
    old_eye: Vec4,
    old_normals: &[Vec4],
    eye: Vec4,
    normals: &[Vec4],
    range: f32,
) -> f32 {
    if old_normals.len() != normals.len() || old_normals.is_empty() {
        return f32::INFINITY;
    }
    let angular = old_normals
        .iter()
        .zip(normals)
        .map(|(a, b)| (*a - *b).length())
        .fold(0.0_f32, f32::max);
    (eye - old_eye).length() + range * angular
}

/// Generate complete columns, nearest visible column first, then prefetch.
/// Sky-to-ground order prevents loading a broad field of terrain whose direct
/// sunlight must wait for thousands of distant upper-air chunks to arrive.
pub fn skylight_column_order(visible: &[IVec4], prefetch: &[IVec4]) -> Vec<IVec4> {
    let mut columns = HashSet::default();
    let mut ordered = Vec::new();
    for &c in visible.iter().chain(prefetch) {
        let key = IVec4::new(c.x, 0, c.z, c.w);
        if !columns.insert(key) {
            continue;
        }
        for y in (0..WORLD_CHUNKS_Y).rev() {
            ordered.push(IVec4::new(c.x, y, c.z, c.w));
        }
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_reports_bootstrap_replacement_and_real_eviction_but_not_edits() {
        let mut world = World::new(7, None);
        let c = IVec4::ZERO;
        let initial = Chunk::uniform(block::AIR);
        let first = initial.identity;
        world.insert_chunk(c, initial);
        let events = world.subscribe_chunk_lifecycle();
        assert_eq!(
            events.try_recv().unwrap(),
            ChunkLifecycle {
                coordinate: c,
                identity: first,
                loaded: true
            }
        );
        world.set_block(IVec4::ZERO, block::TORCH);
        assert!(
            events.try_recv().is_err(),
            "block updates must not trigger full chunk activation"
        );
        world.unload(|_| false, |_, _| false);
        assert!(
            events.try_recv().is_err(),
            "failed eviction keeps the loaded lifecycle intact"
        );
        let replacement = Chunk::uniform(block::STONE);
        let next = replacement.identity;
        world.insert_chunk(c, replacement);
        assert_eq!(
            events.try_recv().unwrap(),
            ChunkLifecycle {
                coordinate: c,
                identity: next,
                loaded: true
            }
        );
        world.unload(|_| false, |_, _| true);
        assert_eq!(
            events.try_recv().unwrap(),
            ChunkLifecycle {
                coordinate: c,
                identity: next,
                loaded: false
            }
        );
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn provisional_display_light_is_never_a_fresh_certificate() {
        let mut world = World::new(7, None);
        let c = IVec4::new(0, 3, 0, 0);
        let chunk = Chunk::uniform(AIR);
        let identity = chunk.identity;
        world.chunks.insert(c, chunk);
        world.light_chunks.insert(
            c,
            LightChunk {
                data: crate::lighting::LightData::Uniform(240),
                version: 1,
                geometry_identity: identity,
                geometry_version: 0,
                sky_known: true,
                settled_serial: 0,
                settled: false,
            },
        );
        assert_eq!(world.light_at(c * CHUNK + IVec4::splat(8)), Some((0, 15)));
        assert!(!world.light_ready(c));
        assert!(world.fresh_light_at(c * CHUNK + IVec4::splat(8)).is_none());
        world.light_chunks.get_mut(&c).unwrap().settled = true;
        assert!(world.light_ready(c));
    }
    #[test]
    fn visible_sphere_contains_sampled_rays_and_ghost_origins_for_all_layouts() {
        let eye = Vec4::new(-3.5, 61.0, 7.5, 0.5);
        let radius = 80.0;
        let margin = 3.0;
        for angle in [0.0_f32, 0.001, 0.7, 1.5707963] {
            let mut orientation = crate::camera::Orientation::default();
            orientation.rotate_forward_ana(angle);
            orientation.rotate_right_ana(angle * 0.37);
            for count in [2, 4, 6, 9] {
                let bases = crate::views::slice_bases(orientation.basis(), count);
                let normals: Vec<_> = bases.iter().map(|b| b.ana).collect();
                let visible: HashSet<_> = chunks_visible_in_slices(eye, &normals, radius, margin)
                    .into_iter()
                    .collect();
                let all = chunks_near_slices(eye, &normals, radius, margin);
                assert!(visible.len() < all.len());
                for basis in bases {
                    for u in [-1.5_f32, -0.5, 0.0, 0.5, 1.5] {
                        for v in [-1.0_f32, 0.0, 1.0] {
                            let direction =
                                (basis.forward + basis.right * u + Vec4::Y * v).normalize();
                            for offset in [-margin, 0.0, margin] {
                                for step in 0..=160 {
                                    let point =
                                        eye + basis.ana * offset + direction * (step as f32 * 0.5);
                                    let c = chunk_of(point.floor().as_ivec4());
                                    if (0..WORLD_CHUNKS_Y).contains(&c.y) {
                                        assert!(
                                            visible.contains(&c),
                                            "missing {c:?},layout{count},angle{angle}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn cached_footprints_contain_moved_rotated_ghost_and_bobbing_rays() {
        let eye = Vec4::new(0.1, 61.5, 0.1, 0.1);
        let range = 96.0;
        let ghost = 3.0;
        for angle in [0.0_f32, 0.001, 0.65, 1.5707963] {
            let mut original = crate::camera::Orientation::default();
            original.rotate_forward_ana(angle);
            original.rotate_right_ana(angle * 0.4);
            original.add_pitch(-0.3);
            let mut current = original.clone();
            current.rotate_forward_ana(0.003);
            current.rotate_right_ana(-0.002);
            for count in [1, 2, 4, 6, 9] {
                let old = crate::views::slice_bases(original.basis(), count);
                let new = crate::views::slice_bases(current.basis(), count);
                let normals: Vec<_> = old.iter().map(|b| b.ana).collect();
                let new_normals: Vec<_> = new.iter().map(|b| b.ana).collect();
                let moved = eye + Vec4::new(0.7, 0.0, 0.3, 0.2);
                assert!(
                    slice_cache_motion_bound(eye, &normals, moved, &new_normals, range + ghost)
                        <= 1.5
                );
                let wanted: HashSet<_> =
                    chunks_visible_in_slices(eye, &normals, range, ghost + 2.0)
                        .into_iter()
                        .collect();
                for basis in new {
                    for u in [-2.0_f32, -1.0, 0.0, 1.0, 2.0] {
                        for v in [-1.0_f32, 0.0, 1.0] {
                            let direction =
                                (basis.forward + basis.right * u + basis.up * v).normalize();
                            let bob = Vec4::Y * (-0.04) + basis.right * 0.025;
                            for offset in [-ghost, 0.0, ghost] {
                                for step in 0..=192 {
                                    let p = moved
                                        + bob
                                        + basis.ana * offset
                                        + direction * (step as f32 * 0.5);
                                    let c = chunk_of(p.floor().as_ivec4());
                                    if (0..WORLD_CHUNKS_Y).contains(&c.y) {
                                        assert!(
                                            wanted.contains(&c),
                                            "cached footprint lost {c:?},layout{count},angle{angle}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let moved = Vec4::new(3.9, 61.5, 3.9, 3.9);
        let normals = [Vec4::new(0.70710677, 0.0, 0.70710677, 0.0)];
        assert_eq!(
            (eye / 4.0).floor(),
            (moved / 4.0).floor(),
            "old cache bins would reuse this footprint"
        );
        assert!(slice_cache_motion_bound(eye, &normals, moved, &normals, range) > 1.5);
        assert!(slice_cache_motion_bound(eye, &normals, eye, &[], range).is_infinite());
    }

    #[test]
    fn local_light_rejects_unsent_neighbor_edits_but_not_unrelated_background_work() {
        let mut world = World::new(7, None);
        let mut wanted = Vec::new();
        for x in 0..2 {
            for y in 0..WORLD_CHUNKS_Y {
                let c = IVec4::new(x, y, 0, 0);
                world.insert_chunk(c, Chunk::uniform(block::AIR));
                wanted.push(c);
            }
        }
        let settle = |world: &mut World| {
            let start = std::time::Instant::now();
            loop {
                world.update_lighting(&wanted, 128);
                if world.lighting_ready(&wanted) {
                    return;
                }
                assert!(
                    start.elapsed().as_secs() < 10,
                    "local lighting did not settle: {}",
                    world.lighting_pending()
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        };
        settle(&mut world);
        let receiver = IVec4::new(15, 60, 8, 8);
        let receiver_chunk = chunk_of(receiver);
        assert_eq!(world.fresh_light_at(receiver).unwrap().0, 0);
        world.insert_chunk(IVec4::new(100, 0, 0, 0), Chunk::uniform(block::STONE));
        assert!(world.lighting_pending() > 0);
        assert!(world.lighting_ready(&wanted));
        // The receiving chunk's own geometry is unchanged, but its neighbor is
        // not yet submitted. Old geometry-only readiness would falsely pass.
        world.set_block(receiver + IVec4::X, block::TORCH);
        assert!(
            world
                .light_chunk(receiver_chunk)
                .unwrap()
                .matches(&world.chunks[&receiver_chunk])
        );
        assert!(!world.light_ready(receiver_chunk));
        assert!(world.fresh_light_at(receiver).is_none());
        settle(&mut world);
        assert_eq!(world.fresh_light_at(receiver).unwrap().0, 13);
        world.set_block(receiver + IVec4::X, block::AIR);
        assert!(!world.light_ready(receiver_chunk));
        settle(&mut world);
        assert_eq!(world.fresh_light_at(receiver).unwrap().0, 0);
    }

    #[test]
    fn streaming_prioritizes_complete_visible_columns_before_prefetch() {
        let a = IVec4::new(2, 3, 4, 5);
        let b = IVec4::new(3, 4, 5, 6);
        let c = IVec4::new(4, 5, 6, 7);
        let order = skylight_column_order(&[a, a + IVec4::Y, b], &[c, b, a]);
        assert_eq!(order.len(), 3 * WORLD_CHUNKS_Y as usize);
        for (column, source) in [a, b, c].into_iter().enumerate() {
            for y in 0..WORLD_CHUNKS_Y {
                assert_eq!(
                    order[column * WORLD_CHUNKS_Y as usize + y as usize],
                    IVec4::new(source.x, WORLD_CHUNKS_Y - 1 - y, source.z, source.w)
                );
            }
        }
    }

    #[test]
    fn aligned_slice_is_one_chunk_thick() {
        let eye = Vec4::new(8.0, 60.0, 8.0, 8.0);
        let set = chunks_near_slice(eye, Vec4::W, 64.0, 0.0);
        assert!(!set.is_empty());
        assert!(set.iter().all(|c| c.w == 0));
    }

    #[test]
    fn multiple_slices_equal_the_union_of_individual_slices() {
        let eye = Vec4::new(8.0, 60.0, 8.0, 8.0);
        let normals = [Vec4::W, Vec4::X, (Vec4::Z + Vec4::W).normalize()];
        let expected: HashSet<_> = normals
            .iter()
            .flat_map(|&n| chunks_near_slice(eye, n, 32.0, 2.0))
            .collect();
        let actual = chunks_near_slices(eye, &normals, 32.0, 2.0);
        let actual_set: HashSet<_> = actual.iter().copied().collect();
        assert_eq!(actual.len(), actual_set.len());
        assert_eq!(actual_set, expected);
        assert!(
            actual.iter().any(|c| c.w.abs() >= 2 && c.x == 0),
            "perpendicular plane must be loaded"
        );
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
    fn state_only_edits_notify_observers_and_default_edits_clear_metadata() {
        let mut world = World::new(1, None);
        let p = IVec4::new(0, 48, 0, 0);
        world.load_now(chunk_of(p), None);
        world.set_block(p, crate::block::WOOL);
        let events = world.subscribe_changes();
        let revision = world.revision;
        world.set_block_state(p, BlockState::new(crate::block::WOOL, 5));
        assert!(world.revision > revision);
        let event = events.try_recv().unwrap();
        assert_eq!(event.before, BlockState::new(crate::block::WOOL, 0));
        assert_eq!(event.after, BlockState::new(crate::block::WOOL, 5));
        assert_eq!(event.position, p);
        assert_eq!(world.block(p), Some(crate::block::WOOL));
        world.set_block_state(p, event.after);
        assert!(events.try_recv().is_err());
        world.set_block(p, crate::block::WOOL);
        assert_eq!(events.try_recv().unwrap().after.metadata, 0);
    }

    #[test]
    fn derived_lighting_worker_publishes_fresh_sources_without_saving_light_planes() {
        let mut world = World::new(3, None);
        let wanted: Vec<_> = (0..WORLD_CHUNKS_Y)
            .map(|y| IVec4::new(0, y, 0, 0))
            .collect();
        for &c in &wanted {
            world.insert_chunk(c, Chunk::uniform(AIR));
        }
        let settle = |world: &mut World| {
            for _ in 0..5000 {
                world.update_lighting(&wanted, 128);
                if world.lighting_ready(&wanted) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            panic!(
                "lighting worker failed to settle: {} pending, {} missing",
                world.lighting_pending(),
                world.missing_lighting(&wanted)
            );
        };
        settle(&mut world);
        let p = IVec4::new(8, 60, 8, 8);
        assert_eq!(world.fresh_light_at(p), Some((0, 15)));
        world.set_block(p, crate::block::TORCH);
        assert!(world.fresh_light_at(p).is_none());
        settle(&mut world);
        assert_eq!(world.fresh_light_at(p), Some((14, 15)));
        assert_eq!(world.fresh_light_at(p + IVec4::W), Some((13, 15)));
        let saved = crate::save::encode(&world.chunks[&chunk_of(p)]);
        let restored = crate::save::decode(&saved).unwrap();
        assert_eq!(restored.get(local_of(p)), crate::block::TORCH);
        assert_eq!(restored.light_state_version, 0);
    }

    #[test]
    fn failed_save_preserves_dirty_chunk_during_eviction() {
        let mut world = World::new(1, None);
        let c = IVec4::new(0, 3, 0, 0);
        world.load_now(c, None);
        world.set_block(c * CHUNK, crate::block::GLOW);
        world.unload(|_| false, |_, _| false);
        assert_eq!(world.block(c * CHUNK), Some(crate::block::GLOW));
        world.unload(|_| false, |_, _| true);
        assert!(!world.is_loaded(c));
    }

    #[test]
    fn horizontal_column_optimization_matches_brute_force_geometry_and_order() {
        for eye in [
            Vec4::new(7.3, 61.2, -14.8, 21.6),
            Vec4::new(-16.0, 9.0, 32.0, -8.0),
        ] {
            for normals in [
                vec![Vec4::W, Vec4::X],
                vec![(Vec4::X + Vec4::Z * 0.3 + Vec4::W).normalize()],
                vec![(Vec4::Y + Vec4::W).normalize()],
            ] {
                let (radius, margin) = (32.0, 2.0);
                let half = CHUNK as f32 / 2.0;
                let horiz = radius + half * 2.0;
                let center = chunk_of(eye.floor().as_ivec4());
                let r = (horiz / CHUNK as f32).ceil() as i32 + 1;
                let mut expected = Vec::new();
                for w in -r..=r {
                    for z in -r..=r {
                        for x in -r..=r {
                            for y in 0..WORLD_CHUNKS_Y {
                                let c = IVec4::new(center.x + x, y, center.z + z, center.w + w);
                                let rel = (c * CHUNK).as_vec4() + Vec4::splat(half) - eye;
                                if normals.iter().any(|&n| {
                                    let off = rel.dot(n);
                                    let p = rel - n * off;
                                    off.abs() <= half * n.abs().element_sum() + margin
                                        && Vec4::new(p.x, 0.0, p.z, p.w).length_squared()
                                            <= horiz * horiz
                                }) {
                                    expected.push(c);
                                }
                            }
                        }
                    }
                }
                expected.sort_by(|a, b| {
                    let distance = |c: IVec4| {
                        ((c * CHUNK).as_vec4() + Vec4::splat(half) - eye).length_squared()
                    };
                    distance(*a).total_cmp(&distance(*b))
                });
                assert_eq!(chunks_near_slices(eye, &normals, radius, margin), expected);
                let neighborhood = SliceNeighborhood::new(eye, &normals, radius, margin);
                let expected: HashSet<_> = expected.into_iter().collect();
                for w in -r - 1..=r + 1 {
                    for z in -r - 1..=r + 1 {
                        for x in -r - 1..=r + 1 {
                            for y in -1..=WORLD_CHUNKS_Y {
                                let c = IVec4::new(center.x + x, y, center.z + z, center.w + w);
                                assert_eq!(
                                    neighborhood.contains(c),
                                    expected.contains(&c),
                                    "retention membership differs at {c:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
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
