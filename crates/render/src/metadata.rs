//! Optional metadata residency. Zero-state terrain allocates only an indirection
//! header; byte planes grow geometrically when stateful chunks become visible.
use crate::store::{WINDOW_LEN, window_index};
use mc4d_core::{chunk::{CHUNK_VOLUME, Chunk}, glam::IVec4, world::World};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

pub const HEADER_BYTES: u64 = (WINDOW_LEN * 4) as u64;
const PLANE_BYTES: u64 = CHUNK_VOLUME as u64;

#[derive(Clone, Copy)]
struct Resident {
    slot: u32,
    identity: u64,
    version: u32,
    published_entry: u32,
}

pub(crate) struct MetadataPool {
    pub buffer: wgpu::Buffer,
    pub generation: u64,
    capacity: u32,
    limit: u32,
    free: Vec<u32>,
    resident: HashMap<IVec4, Resident>,
    header: Vec<u32>,
    header_dirty: bool,
    header_active: usize,
}

impl MetadataPool {
    pub fn new(device: &wgpu::Device) -> Self {
        let limits = device.limits();
        let max_bytes = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let limit = ((max_bytes.saturating_sub(HEADER_BYTES)) / PLANE_BYTES).min(u32::MAX as u64) as u32;
        // Avoid early mid-game growth copies; stateful chunks are a minority.
        // Tests start empty to keep exercising the growth/rebind path.
        let capacity = if cfg!(test) { 0 } else { 512u32.min(limit) };
        Self {
            buffer: Self::buffer(device, capacity), generation: 0, capacity, limit,
            free: (0..capacity).rev().collect(), resident: HashMap::default(), header: vec![0; WINDOW_LEN], header_dirty: true, header_active: 0,
        }
    }

    fn buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("optional block state metadata"), size: HEADER_BYTES + capacity as u64 * PLANE_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, world: &World, wanted: &HashSet<IVec4>) {
        self.resident.retain(|c, resident| {
            if wanted.contains(c) { true } else { self.free.push(resident.slot); false }
        });
        let needed = wanted.iter().filter(|c| world.chunks.get(c).is_some_and(|chunk| chunk.metadata_plane().is_some())).count() as u32;
        if needed <= self.capacity { return; }
        let capacity = needed.next_power_of_two().max(16).min(self.limit);
        if capacity <= self.capacity { return; }
        let buffer = Self::buffer(device, capacity);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("grow state metadata") });
        encoder.copy_buffer_to_buffer(&self.buffer, 0, &buffer, 0, self.buffer.size());
        queue.submit([encoder.finish()]);
        self.buffer = buffer;
        self.free.extend((self.capacity..capacity).rev());
        self.capacity = capacity;
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn clear_window(&mut self) {
        if self.header_active > 0 { self.header.fill(0); self.header_dirty = true; self.header_active = 0; }
    }

    pub fn needs_update(&self, c: IVec4, chunk: &Chunk) -> bool {
        match (chunk.metadata_plane(), self.resident.get(&c)) {
            (None, None) => false,
            (Some(_), Some(r)) => r.identity != chunk.identity || r.version != chunk.metadata_version,
            _ => true,
        }
    }

    pub fn needs_upload(&self, c: IVec4, chunk: &Chunk) -> bool {
        chunk.metadata_plane().is_some() && self.needs_update(c, chunk)
    }

    pub fn can_accept(&self, c: IVec4, chunk: &Chunk) -> bool {
        chunk.metadata_plane().is_none() || self.resident.contains_key(&c) || !self.free.is_empty()
    }

    pub fn previous_entry(&self, c: IVec4) -> Option<u32> { self.resident.get(&c).map(|r| r.published_entry) }

    fn set_header(&mut self, c: IVec4, slot: u32) {
        let index = window_index(c);
        if self.header[index] != slot {
            self.header_active = self.header_active - (self.header[index] != 0) as usize + (slot != 0) as usize;
            self.header[index] = slot; self.header_dirty = true;
        }
    }

    pub fn publish_existing(&mut self, c: IVec4) {
        self.set_header(c, self.resident.get(&c).map_or(0, |r| r.slot + 1));
    }

    /// Called only after the caller reserves the complete chunk update budget,
    /// so IDs and metadata become visible together in the next submission.
    pub fn accept(&mut self, queue: &wgpu::Queue, c: IVec4, chunk: &Chunk, published_entry: u32) {
        if let Some(plane) = chunk.metadata_plane() {
            let previous = self.resident.get(&c).copied();
            let slot = previous.map_or_else(|| self.free.pop().expect("metadata capacity reserved"), |r| r.slot);
            if previous.is_none_or(|r| r.identity != chunk.identity || r.version != chunk.metadata_version) {
                queue.write_buffer(&self.buffer, HEADER_BYTES + slot as u64 * PLANE_BYTES, plane);
            }
            self.resident.insert(c, Resident { slot, identity: chunk.identity, version: chunk.metadata_version, published_entry });
            self.set_header(c, slot + 1);
        } else {
            if let Some(r) = self.resident.remove(&c) { self.free.push(r.slot); }
            self.set_header(c, 0);
        }
    }

    pub fn flush(&mut self, queue: &wgpu::Queue) -> bool {
        let changed = self.header_dirty;
        if self.header_dirty {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.header));
            self.header_dirty = false;
        }
        changed
    }

    pub fn allocated_bytes(&self) -> u64 { self.buffer.size() }
}
