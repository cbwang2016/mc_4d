//! GPU time of the ray-casting pass from timestamp queries. Unlike the frame
//! interval, it is not quantised by vsync or compositor pacing, so adaptive
//! resolution can tell real rendering cost from presentation waits.
//! Results arrive a few frames late; `None` when the device lacks the feature.
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering::{Acquire, Release}};

// Slot states: free, resolved this frame, mapping requested, mapped and readable.
const FREE: u8 = 0;
const RESOLVED: u8 = 1;
const MAPPING: u8 = 2;
const READY: u8 = 3;

const SLOTS: usize = 4;
/// Resolve offsets must be QUERY_RESOLVE_BUFFER_ALIGNMENT (256) aligned.
const STRIDE: u64 = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;

pub struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: Vec<wgpu::Buffer>,
    state: Vec<Arc<AtomicU8>>,
    period_ns: f32,
    frame: usize,
    latest: Option<f32>,
}

impl GpuTimer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) { return None; }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("raycast timestamps"), ty: wgpu::QueryType::Timestamp, count: 2 * SLOTS as u32 });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor { label: Some("timestamp resolve"), size: STRIDE * SLOTS as u64,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let readback = (0..SLOTS).map(|_| device.create_buffer(&wgpu::BufferDescriptor { label: Some("timestamp readback"), size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false })).collect();
        Some(Self { queries, resolve, readback, state: (0..SLOTS).map(|_| Arc::new(AtomicU8::new(FREE))).collect(),
            period_ns: queue.get_timestamp_period(), frame: 0, latest: None })
    }

    /// Timestamp writes for this frame's pass, or None while its slot is still being read.
    pub fn writes(&self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let slot = self.frame % SLOTS;
        if self.state[slot].load(Acquire) != FREE { return None; }
        Some(wgpu::RenderPassTimestampWrites { query_set: &self.queries, beginning_of_pass_write_index: Some(2 * slot as u32), end_of_pass_write_index: Some(2 * slot as u32 + 1) })
    }

    /// After the pass: resolve into this frame's readback buffer.
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder, written: bool) {
        let slot = self.frame % SLOTS;
        if written {
            encoder.resolve_query_set(&self.queries, 2 * slot as u32..2 * slot as u32 + 2, &self.resolve, STRIDE * slot as u64);
            encoder.copy_buffer_to_buffer(&self.resolve, STRIDE * slot as u64, &self.readback[slot], 0, 16);
            self.state[slot].store(RESOLVED, Release);
        }
    }

    /// After submission: request mapping of the frame just submitted and
    /// collect any finished earlier frame.
    pub fn after_submit(&mut self, device: &wgpu::Device) {
        let slot = self.frame % SLOTS;
        if self.state[slot].load(Acquire) == RESOLVED {
            self.state[slot].store(MAPPING, Release);
            let flag = self.state[slot].clone();
            self.readback[slot].slice(..).map_async(wgpu::MapMode::Read, move |result| {
                flag.store(if result.is_ok() { READY } else { FREE }, Release);
            });
        }
        let _ = device.poll(wgpu::PollType::Poll);
        for (i, buffer) in self.readback.iter().enumerate() {
            if self.state[i].load(Acquire) != READY { continue; }
            let Ok(view) = buffer.slice(..).get_mapped_range() else { continue };
            let t: [u64; 2] = bytemuck::pod_read_unaligned(&view[..16]);
            drop(view);
            buffer.unmap();
            self.state[i].store(FREE, Release);
            if t[1] > t[0] { self.latest = Some((t[1] - t[0]) as f32 * self.period_ns / 1.0e6); }
        }
        self.frame = self.frame.wrapping_add(1);
    }

    /// Most recent completed ray-casting pass time in milliseconds.
    pub fn take(&mut self) -> Option<f32> { self.latest.take() }
}
