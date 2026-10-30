//! Sparse derived lighting: uniform chunks live entirely in the window header;
//! only mixed light fields consume byte-plane slots.
use crate::store::{WINDOW_LEN, WINDOW_RADIUS, window_index};
use mc4d_core::{chunk::{CHUNK_VOLUME, WORLD_CHUNKS_Y}, glam::IVec4, world::World};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

const HEADER_BYTES: u64 = (WINDOW_LEN * 4) as u64;
const PLANE_BYTES: u64 = CHUNK_VOLUME as u64;
const INITIAL_PLANES: u32 = if cfg!(test) { 0 } else { 4096 };
const DENSE: u32 = 0x8000_0000;
const UNKNOWN: u32 = 0x4000_0000;

#[derive(Clone, Copy)]
struct Resident { slot: u32, version: u64, identity: u64 }

pub(crate) struct LightStore {
    pub buffer: wgpu::Buffer,
    pub generation: u64,
    capacity: u32,
    limit: u32,
    free: Vec<u32>,
    resident: HashMap<IVec4, Resident>,
    header: Vec<u32>,
    center: IVec4,
    wanted: Vec<IVec4>,
    wanted_set: HashSet<IVec4>,
    revision: u64,
    world_identity: u64,
    behind: bool,
    pub missing: usize,
}

impl LightStore {
    pub fn new(device: &wgpu::Device) -> Self {
        let limits = device.limits();
        let maximum = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let limit = (maximum.saturating_sub(HEADER_BYTES) / PLANE_BYTES).min(0x3fff_ffff) as u32;
        // Growing mid-flight zero-fills a new buffer and copies the old one on
        // the GPU (an 80 ms stall at 128 -> 256 MiB). Nine slices at the default
        // distance use 4096 planes, so reserve that up front.
        let capacity = INITIAL_PLANES.min(limit);
        Self { buffer: Self::buffer(device, capacity), generation: 0, capacity, limit,
            free: (0..capacity).rev().collect(), resident: HashMap::default(), header: vec![UNKNOWN; WINDOW_LEN],
            center: IVec4::splat(i32::MIN), wanted: Vec::new(), wanted_set: HashSet::default(),
            revision: u64::MAX, world_identity: 0, behind: false, missing: 0 }
    }
    fn buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor { label: Some("optional packed voxel lighting"),
            size: HEADER_BYTES + capacity as u64 * PLANE_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false })
    }
    pub fn bytes(&self) -> u64 { self.buffer.size() }

    pub fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, world: &World, center: IVec4, wanted: &[IVec4], budget: usize) {
        let center = IVec4::new(center.x, 0, center.z, center.w);
        if !self.behind && self.world_identity == world.identity && self.revision == world.lighting_revision && self.center == center && self.wanted == wanted { return; }
        let world_changed=self.world_identity!=world.identity;
        let changed = world_changed || self.center != center || self.wanted != wanted;
        if world_changed {
            // UNKNOWN permits continuity only within the same geometry identity.
            // A budgeted reload must never expose another world's light plane.
            self.free.extend(self.resident.drain().map(|(_,resident)|resident.slot));
        }
        self.world_identity = world.identity;
        self.center = center; self.revision = world.lighting_revision; self.behind = false;
        let inside = |c: IVec4| { let d = (c - center).abs(); c.y >= 0 && c.y < WORLD_CHUNKS_Y && d.x <= WINDOW_RADIUS && d.z <= WINDOW_RADIUS && d.w <= WINDOW_RADIUS };
        let mut header_changed = changed;
        if changed {
            self.wanted.clear(); self.wanted.extend_from_slice(wanted);
            self.wanted_set.clear(); self.wanted_set.extend(wanted.iter().copied().filter(|&c| inside(c)));
            self.header.fill(UNKNOWN);
            self.resident.retain(|c, r| { if self.wanted_set.contains(c) { true } else { self.free.push(r.slot); false } });
        }
        let needed = self.wanted_set.iter().filter(|&&c| world.light_chunk(c).is_some_and(|light| light.data.dense().is_some())).count() as u32;
        if needed > self.capacity {
            let capacity = needed.next_power_of_two().max(16).min(self.limit);
            if capacity > self.capacity {
                let buffer = Self::buffer(device, capacity);
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("grow lighting pool") });
                encoder.copy_buffer_to_buffer(&self.buffer, 0, &buffer, 0, self.buffer.size());
                queue.submit([encoder.finish()]);
                self.buffer = buffer; self.free.extend((self.capacity..capacity).rev()); self.capacity = capacity;
                self.generation = self.generation.wrapping_add(1);
            }
        }
        let mut uploads = 0; self.missing = 0;
        for &c in wanted {
            if !inside(c) { continue; }
            let source = world.chunks.get(&c).and_then(|chunk| world.light_chunk(c).filter(|light| light.geometry_identity == chunk.identity).map(|light| (chunk, light)));
            let entry = if let Some((_chunk, light)) = source {
                let ready = world.light_ready(c);
                if !ready { self.missing += 1; }
                if let Some(value) = light.data.uniform() {
                    if let Some(old) = self.resident.remove(&c) { self.free.push(old.slot); }
                    value as u32 | if ready { 0 } else { UNKNOWN }
                } else {
                    if self.resident.get(&c).is_some_and(|resident|resident.identity!=light.geometry_identity) {
                        if let Some(old)=self.resident.remove(&c){self.free.push(old.slot);}
                    }
                    let old = self.resident.get(&c).copied();
                    let current = old.is_some_and(|r| r.version == light.version && r.identity == light.geometry_identity);
                    if !current {
                        if uploads < budget {
                            let slot = old.map(|r| r.slot).or_else(|| self.free.pop());
                            if let Some(slot) = slot {
                                queue.write_buffer(&self.buffer, HEADER_BYTES + slot as u64 * PLANE_BYTES, light.data.dense().unwrap());
                                self.resident.insert(c, Resident { slot, version: light.version, identity: light.geometry_identity }); uploads += 1;
                            } else { self.behind = true; }
                        } else { self.behind = true; }
                    }
                    let resident = self.resident.get(&c);
                    let valid = resident.is_some_and(|r| r.version == light.version && r.identity == light.geometry_identity);
                    if !valid { self.missing += ready as usize; }
                    resident.map_or(UNKNOWN, |r| DENSE | r.slot | if valid && ready { 0 } else { UNKNOWN })
                }
            } else {
                self.missing += 1;
                if let Some(old) = self.resident.remove(&c) { self.free.push(old.slot); }
                UNKNOWN
            };
            let index = window_index(c);
            if self.header[index] != entry { self.header[index] = entry; header_changed = true; }
        }
        if header_changed { queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.header)); }
    }
}

#[cfg(test)]mod tests {
    use super::*;
    use mc4d_core::{block,chunk::Chunk};
    fn cave(torch:IVec4)->World {
        let mut world=World::new(1,None);let mut chunk=Chunk::uniform(block::STONE);
        for w in 2..14 {for z in 2..14 {for y in 2..10 {for x in 2..14 {chunk.set(IVec4::new(x,y,z,w),block::AIR);}}}}
        chunk.set(torch,block::TORCH);world.insert_chunk(IVec4::ZERO,chunk);
        crate::gpu_tests::settle_lighting(&mut world,&[IVec4::ZERO]);world
    }
    #[test]
    #[ignore="requires GPU; unrelated world light must never remain addressable during budgeted reload"]
    fn dense_light_identity_is_not_reused_when_reload_upload_is_deferred(){
        let instance=wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter=pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions{power_preference:wgpu::PowerPreference::HighPerformance,compatible_surface:None,force_fallback_adapter:false,apply_limit_buckets:false})).unwrap();
        let(device,queue)=pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor{required_limits:adapter.limits(),..Default::default()})).unwrap();
        let mut a=cave(IVec4::new(3,4,3,3));let b=cave(IVec4::new(12,4,12,12));let c=IVec4::ZERO;
        assert_ne!(a.light_chunk(c).unwrap().get(IVec4::new(4,4,3,3)),b.light_chunk(c).unwrap().get(IVec4::new(4,4,3,3)));
        let mut gpu=LightStore::new(&device);gpu.sync(&device,&queue,&a,c,&[c],64);
        assert_ne!(gpu.header[window_index(c)]&DENSE,0);
        let old_slot=gpu.resident[&c].slot;
        a.set_block(IVec4::new(3,4,3,3),block::AIR);a.set_block(IVec4::new(9,4,9,9),block::TORCH);
        crate::gpu_tests::settle_lighting(&mut a,&[c]);
        gpu.sync(&device,&queue,&a,c,&[c],0);
        assert_eq!(gpu.resident[&c].slot,old_slot);
        assert_eq!(gpu.header[window_index(c)]&(DENSE|UNKNOWN),DENSE|UNKNOWN,"same-identity stale light remains available while its replacement is queued");
        gpu.sync(&device,&queue,&a,c,&[c],64);
        gpu.sync(&device,&queue,&b,c,&[c],0);
        assert_eq!(gpu.header[window_index(c)]&DENSE,0,"UNKNOWN must not expose a dense plane from a different geometry identity");
        gpu.sync(&device,&queue,&b,c,&[c],64);
        assert_eq!(gpu.missing,0);assert_eq!(gpu.resident[&c].identity,b.light_chunk(c).unwrap().geometry_identity);
    }
}
