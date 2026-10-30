//! Builds the 2x2x2x2 occupancy level (see shaders/fine_occupancy.wgsl) for the
//! chunk slots uploaded since the last frame. Rays inside a partly filled 4^4
//! macrocell then skip empty 16-voxel groups instead of stepping voxel by voxel.
pub const WORDS_PER_SLOT: u64 = 128;
const BATCH: usize = 256;
/// Rebuilds per frame: a burst of 64 uploads (flight) cost ~30 ms of GPU time.
const PER_FRAME: usize = 16;

pub struct FineOccupancy {
    pipeline: wgpu::ComputePipeline,
    slots: wgpu::Buffer,
    group: wgpu::BindGroup,
}

impl FineOccupancy {
    pub fn new(device: &wgpu::Device, pool: &wgpu::Buffer, fine: &wgpu::Buffer) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("fine occupancy"), source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fine_occupancy.wgsl").into()) });
        let entry = |binding, read_only| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fine occupancy"), entries: &[entry(0, true), entry(1, false), entry(2, true)] });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("fine occupancy"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("fine occupancy"), layout: Some(&pl), module: &module,
            entry_point: Some("main"), compilation_options: Default::default(), cache: None });
        let slots = device.create_buffer(&wgpu::BufferDescriptor { label: Some("fine occupancy slots"), size: (BATCH * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("fine occupancy"), layout: &layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: pool.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: fine.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: slots.as_entire_binding() }] });
        Self { pipeline, slots, group }
    }

    /// Refine up to PER_FRAME of the `dirty` slots (oldest first). Slots still
    /// waiting keep conservative all-occupied bits, which render correctly.
    pub fn update(&self, device: &wgpu::Device, queue: &wgpu::Queue, dirty: &mut Vec<u32>) {
        let mut seen = std::collections::HashSet::new();
        dirty.retain(|slot| seen.insert(*slot));
        let take = dirty.len().min(PER_FRAME);
        let now: Vec<u32> = dirty.drain(..take).collect();
        for batch in now.chunks(BATCH) {
            queue.write_buffer(&self.slots, 0, bytemuck::cast_slice(batch));
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("fine occupancy") });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("fine occupancy"), timestamp_writes: None });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.group, &[]);
                pass.dispatch_workgroups(batch.len() as u32, 1, 1);
            }
            // Submitted per batch: the slot list buffer is rewritten for the next one.
            queue.submit([encoder.finish()]);
        }
    }
}
