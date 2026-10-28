//! GPU residency of chunks: a pool of dense chunk slots plus a toroidal 16×16×16×16
//! indirection window centered on the player's chunk.
//!
//! Window layout (must match `raycast.wgsl`): index = (x&15) + 16·(y + 16·((z&15) + 16·(w&15))),
//! with Y addressed directly (the world is 16 chunks tall).

use mc4d_core::chunk::{CHUNK_VOLUME, WORLD_CHUNKS_Y};
use mc4d_core::glam::IVec4;
use mc4d_core::world::World;
use std::collections::{HashMap, HashSet};

pub const WINDOW_SIDE: i32 = 16;
pub const WINDOW_RADIUS: i32 = 7;
const WINDOW_LEN: usize = (WINDOW_SIDE * WINDOW_SIDE * WINDOW_SIDE * WINDOW_SIDE) as usize;
pub const ENTRY_DENSE: u32 = 0x8000_0000;
pub const ENTRY_UNLOADED: u32 = 0x4000_0000;
const CHUNK_BYTES: u64 = CHUNK_VOLUME as u64;

#[derive(Clone, Copy)]
struct Resident {
    slot: u32,
    version: u32,
}

pub struct ChunkStore {
    pub(crate) pool: wgpu::Buffer,
    pub(crate) window: wgpu::Buffer,
    slots: u32,
    free: Vec<u32>,
    resident: HashMap<IVec4, Resident>,
    window_cpu: Vec<u32>,
    center: IVec4,
    last_wanted: Vec<IVec4>,
    last_loaded: usize,
    last_revision: u64,
    /// The last sync hit the upload budget, so there is more to upload.
    behind: bool,
    /// Max chunk uploads per `sync` call (bounds the per-frame upload cost).
    pub uploads_per_frame: usize,
}

pub fn window_index(c: IVec4) -> usize {
    ((c.x & 15) + 16 * (c.y + 16 * ((c.z & 15) + 16 * (c.w & 15)))) as usize
}

impl ChunkStore {
    pub fn new(device: &wgpu::Device) -> Self {
        let limits = device.limits();
        let max_bytes = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let slots = (max_bytes / CHUNK_BYTES).min(2048) as u32;
        log::info!("chunk pool: {slots} slots ({} MiB)", (slots as u64 * CHUNK_BYTES) >> 20);
        let pool = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk pool"),
            size: slots as u64 * CHUNK_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let window = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk window"),
            size: (WINDOW_LEN * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pool,
            window,
            slots,
            free: (0..slots).rev().collect(),
            resident: HashMap::new(),
            window_cpu: vec![ENTRY_UNLOADED; WINDOW_LEN],
            center: IVec4::new(i32::MIN, 0, 0, 0),
            last_wanted: Vec::new(),
            last_loaded: 0,
            last_revision: u64::MAX,
            behind: false,
            uploads_per_frame: 64,
        }
    }

    pub fn center(&self) -> IVec4 {
        self.center
    }

    pub fn slot_count(&self) -> u32 {
        self.slots
    }

    pub fn resident_count(&self) -> usize {
        self.resident.len()
    }

    /// Make the GPU mirror `world` for the chunks in `wanted` (nearest first) around
    /// `center`. Re-uploads chunks whose version changed. Returns true if anything changed.
    pub fn sync(&mut self, queue: &wgpu::Queue, world: &World, center: IVec4, wanted: &[IVec4]) -> bool {
        let center = IVec4::new(center.x, 0, center.z, center.w);
        if !self.behind && center == self.center && world.revision == self.last_revision && wanted == self.last_wanted.as_slice() {
            return false;
        }
        self.last_revision = world.revision;
        self.behind = false;
        let mut changed = center != self.center || wanted != self.last_wanted.as_slice();
        self.center = center;
        if changed {
            self.last_wanted = wanted.to_vec();
        }

        let in_window = |c: IVec4| {
            let d = (c - center).abs();
            c.y >= 0 && c.y < WORLD_CHUNKS_Y && d.x <= WINDOW_RADIUS && d.z <= WINDOW_RADIUS && d.w <= WINDOW_RADIUS
        };

        let mut uploads = 0;
        let wanted_set: HashSet<IVec4> = wanted.iter().copied().filter(|&c| in_window(c)).collect();
        // Newly generated uniform chunks change the window without needing an upload.
        let loaded = wanted_set.iter().filter(|c| world.chunks.contains_key(c)).count();
        if loaded != self.last_loaded {
            self.last_loaded = loaded;
            changed = true;
        }
        for &c in wanted {
            if !wanted_set.contains(&c) {
                continue;
            }
            let Some(chunk) = world.chunks.get(&c) else { continue };
            let Some(data) = chunk.dense() else {
                // Uniform chunks need no pool slot.
                if let Some(r) = self.resident.remove(&c) {
                    self.free.push(r.slot);
                    changed = true;
                }
                continue;
            };
            match self.resident.get(&c).copied() {
                Some(r) if r.version == chunk.version => {}
                Some(r) => {
                    queue.write_buffer(&self.pool, r.slot as u64 * CHUNK_BYTES, data);
                    self.resident.insert(c, Resident { slot: r.slot, version: chunk.version });
                    changed = true;
                }
                None => {
                    if uploads >= self.uploads_per_frame {
                        self.behind = true;
                        continue;
                    }
                    if self.free.is_empty() {
                        self.evict(&wanted_set);
                    }
                    let Some(slot) = self.free.pop() else { continue };
                    queue.write_buffer(&self.pool, slot as u64 * CHUNK_BYTES, data);
                    self.resident.insert(c, Resident { slot, version: chunk.version });
                    uploads += 1;
                    changed = true;
                }
            }
        }

        if changed {
            self.window_cpu.fill(ENTRY_UNLOADED);
            for &c in &wanted_set {
                let entry = if let Some(r) = self.resident.get(&c) {
                    ENTRY_DENSE | r.slot
                } else if let Some(id) = world.chunks.get(&c).and_then(|ch| ch.uniform_id()) {
                    id as u32
                } else {
                    ENTRY_UNLOADED
                };
                self.window_cpu[window_index(c)] = entry;
            }
            queue.write_buffer(&self.window, 0, bytemuck::cast_slice(&self.window_cpu));
        }
        changed
    }

    /// Free the slots of resident chunks that are no longer wanted.
    fn evict(&mut self, wanted: &HashSet<IVec4>) {
        let free = &mut self.free;
        self.resident.retain(|c, r| {
            let keep = wanted.contains(c);
            if !keep {
                free.push(r.slot);
            }
            keep
        });
    }
}
