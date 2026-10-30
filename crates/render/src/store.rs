//! GPU residency of chunks: a pool of dense chunk slots plus a toroidal 16×16×16×16
//! indirection window centered on the player's chunk.
//!
//! Window layout (must match `raycast.wgsl`): index = (x&15) + 16·(y + 16·((z&15) + 16·(w&15))),
//! with Y addressed directly (the world is 16 chunks tall).

use mc4d_core::chunk::{CHUNK_VOLUME, OCCUPANCY_WORDS, WORLD_CHUNKS_Y};
use mc4d_core::glam::IVec4;
use mc4d_core::world::World;
use crate::metadata::MetadataPool;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

pub const WINDOW_SIDE: i32 = 16;
pub const WINDOW_RADIUS: i32 = 7;
pub(crate) const WINDOW_LEN: usize = (WINDOW_SIDE * WINDOW_SIDE * WINDOW_SIDE * WINDOW_SIDE) as usize;
pub const ENTRY_DENSE: u32 = 0x8000_0000;
pub const ENTRY_UNLOADED: u32 = 0x4000_0000;
const CHUNK_BYTES: u64 = CHUNK_VOLUME as u64;

#[derive(Clone, Copy)]
struct Resident {
    slot: u32,
    version: u32,
    identity: u64,
}

pub struct ChunkStore {
    pub(crate) pool: wgpu::Buffer,
    pub(crate) window: wgpu::Buffer,
    pub(crate) occupancy: wgpu::Buffer,
    /// 2x2x2x2 occupancy bits per slot, rebuilt on the GPU for `fine_dirty` slots.
    pub(crate) fine: wgpu::Buffer,
    pub(crate) fine_dirty: Vec<u32>,
    pub(crate) metadata: MetadataPool,
    slots: u32,
    free: Vec<u32>,
    resident: HashMap<IVec4, Resident>,
    window_cpu: Vec<u32>,
    center: IVec4,
    last_wanted: Vec<IVec4>,
    wanted_set: HashSet<IVec4>,
    residency_counts: (usize, usize),
    last_revision: u64,
    last_world: u64,
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
        // Nine intersecting slices need substantially more dense chunks than one:
        // a default-distance terrain scene alone can exceed 3,600 slots. Reserve
        // enough for the supported multi-view distances without reallocating a
        // large GPU buffer during movement. Respect the adapter's binding limit.
        let slots = (max_bytes / CHUNK_BYTES).min(8192) as u32;
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
        let occupancy = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk macrocell occupancy"),
            size: slots as u64 * (OCCUPANCY_WORDS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let fine = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("chunk fine occupancy"),
            size: slots as u64 * crate::fine_occupancy::WORDS_PER_SLOT * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pool,
            window,
            occupancy,
            fine,
            fine_dirty: Vec::new(),
            metadata: MetadataPool::new(device),
            slots,
            free: (0..slots).rev().collect(),
            resident: HashMap::default(),
            window_cpu: vec![ENTRY_UNLOADED; WINDOW_LEN],
            center: IVec4::new(i32::MIN, 0, 0, 0),
            last_wanted: Vec::new(),
            wanted_set: HashSet::default(),
            residency_counts: (0, 0),
            last_revision: u64::MAX,
            last_world: 0,
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

    /// Unloaded requested chunks and absent/stale dense chunks at the latest
    /// sync. Useful for frame telemetry without rescanning thousands of chunks.
    pub fn residency_counts(&self) -> (usize, usize) { self.residency_counts }

    pub fn metadata_bytes(&self) -> u64 { self.metadata.allocated_bytes() }

    /// Requested dense chunks inside the addressable GPU window that are absent or
    /// stale. Benchmarks must check this before timing a fully populated scene.
    pub fn missing_dense_count(&self, world: &World, wanted: &[IVec4]) -> usize {
        wanted.iter().filter(|&&c| {
            let d = (c - self.center).abs();
            if c.y < 0 || c.y >= WORLD_CHUNKS_Y || d.x > WINDOW_RADIUS || d.z > WINDOW_RADIUS || d.w > WINDOW_RADIUS {
                return false;
            }
            world.chunks.get(&c).is_some_and(|chunk| {
                (chunk.dense().is_some() && self.resident.get(&c).is_none_or(|r| r.identity != chunk.identity || r.version != chunk.data_version))
                    || self.metadata.needs_update(c, chunk)
            })
        }).count()
    }

    /// Make the GPU mirror `world` for the chunks in `wanted` (nearest first) around
    /// `center`. Re-uploads chunks whose version changed. Returns true if anything changed.
    pub fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, world: &World, center: IVec4, wanted: &[IVec4]) -> bool {
        let center = IVec4::new(center.x, 0, center.z, center.w);
        if !self.behind && self.last_world == world.identity && center == self.center && world.revision == self.last_revision && wanted == self.last_wanted.as_slice() {
            return false;
        }
        let world_changed = self.last_world != world.identity;
        self.last_world = world.identity;
        self.last_revision = world.revision;
        self.behind = false;
        let wanted_changed = world_changed || center != self.center || wanted != self.last_wanted.as_slice();
        self.center = center;
        let in_window = |c: IVec4| {
            let d = (c - center).abs();
            c.y >= 0 && c.y < WORLD_CHUNKS_Y && d.x <= WINDOW_RADIUS && d.z <= WINDOW_RADIUS && d.w <= WINDOW_RADIUS
        };
        let mut mapping_changed = wanted_changed;
        let mut data_changed = false;
        if wanted_changed {
            self.last_wanted.clear();
            self.last_wanted.extend_from_slice(wanted);
            self.wanted_set.clear();
            self.wanted_set.extend(wanted.iter().copied().filter(|&c| in_window(c)));
            self.window_cpu.fill(ENTRY_UNLOADED);
            self.metadata.clear_window();
        }
        self.metadata.prepare(device, queue, world, &self.wanted_set);

        let mut uploads = 0;
        self.residency_counts = (0, 0);
        for &c in wanted {
            let chunk = world.chunks.get(&c);
            if chunk.is_none() { self.residency_counts.0 += 1; }
            if !in_window(c) { continue; }
            let entry = if let Some(chunk) = chunk {
                let current = self.resident.get(&c).copied();
                let upload_ids = chunk.dense().is_some() && current.is_none_or(|r| r.identity != chunk.identity || r.version != chunk.data_version);
                let upload_metadata = self.metadata.needs_upload(c, chunk);
                let upload = upload_ids || upload_metadata;
                let budget_available = !upload || uploads < self.uploads_per_frame;
                if upload_ids && current.is_none() && self.free.is_empty() { self.evict(); }
                let room = (!upload_ids || current.is_some() || !self.free.is_empty()) && self.metadata.can_accept(c, chunk);
                let entry = if !budget_available || !room {
                    self.behind = true;
                    // Keep the previous complete ID/state pair visible until the
                    // whole chunk update can be submitted, never mix its planes.
                    self.metadata.publish_existing(c);
                    current.map(|r| ENTRY_DENSE | r.slot).or_else(|| self.metadata.previous_entry(c)).unwrap_or(ENTRY_UNLOADED)
                } else {
                    let entry = if let Some(data) = chunk.dense() {
                        let slot = current.map_or_else(|| self.free.pop().expect("chunk capacity reserved"), |r| r.slot);
                        if upload_ids {
                            queue.write_buffer(&self.pool, slot as u64 * CHUNK_BYTES, data);
                            queue.write_buffer(&self.occupancy, slot as u64 * (OCCUPANCY_WORDS * 4) as u64,
                                bytemuck::cast_slice(&chunk.occupancy_masks()));
                            // Conservative "occupied" until the budgeted GPU rebuild refines it.
                            queue.write_buffer(&self.fine, slot as u64 * crate::fine_occupancy::WORDS_PER_SLOT * 4, &[0xff; (crate::fine_occupancy::WORDS_PER_SLOT * 4) as usize]);
                            self.fine_dirty.push(slot);
                            self.resident.insert(c, Resident { slot, identity: chunk.identity, version: chunk.data_version });
                        }
                        ENTRY_DENSE | slot
                    } else {
                        if let Some(r) = self.resident.remove(&c) { self.free.push(r.slot); }
                        chunk.uniform_id().unwrap() as u32
                    };
                    self.metadata.accept(queue, c, chunk, entry);
                    if upload { uploads += 1; data_changed = true; }
                    entry
                };
                if (chunk.dense().is_some() && self.resident.get(&c).is_none_or(|r| r.identity != chunk.identity || r.version != chunk.data_version))
                    || self.metadata.needs_update(c, chunk) { self.residency_counts.1 += 1; }
                entry
            } else { ENTRY_UNLOADED };
            let index = window_index(c);
            if self.window_cpu[index] != entry {
                self.window_cpu[index] = entry;
                mapping_changed = true;
            }
        }
        // A changed voxel in an existing slot does not alter indirection. Avoid
        // rebuilding/reuploading the 256 KiB window for every fluid update.
        if mapping_changed {
            queue.write_buffer(&self.window, 0, bytemuck::cast_slice(&self.window_cpu));
        }
        let metadata_changed = self.metadata.flush(queue);
        mapping_changed || data_changed || metadata_changed
    }

    /// Free the slots of resident chunks that are no longer wanted.
    fn evict(&mut self) {
        let wanted = &self.wanted_set;
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
