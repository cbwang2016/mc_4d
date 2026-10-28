//! wgpu renderer: ray-casts the 3D slice of the 4D world into an offscreen image at
//! `render_scale` resolution, then upscales it onto the output target.

mod store;

pub use store::ChunkStore;

use bytemuck::{Pod, Zeroable};
use mc4d_core::block::{BLOCK_COUNT, BLOCKS};
use mc4d_core::camera::Basis;
use mc4d_core::chunk::CHUNK;
use mc4d_core::glam::{IVec4, Vec4};
use mc4d_core::world::World;

pub const MAX_ENTITIES: usize = 32;
const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub mod flags {
    pub const GHOSTS: u32 = 1;
    pub const INSET: u32 = 2;
    pub const SHADOWS: u32 = 4;
    pub const GLASSES: u32 = 8;
    pub const EDGES: u32 = 16;
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    ana: [f32; 4],
    center: [i32; 4],
    sun: [f32; 4],
    sky_top: [f32; 4],
    sky_horizon: [f32; 4],
    target_cell: [i32; 4],
    info: [u32; 4],
    view: [f32; 4],
    inset: [f32; 4],
    misc: [f32; 4],
    palette: [[f32; 4]; 16],
    entities: [[f32; 4]; MAX_ENTITIES * 2],
}

/// A hypersphere entity to draw.
#[derive(Clone, Copy, Debug)]
pub struct EntityDraw {
    pub center: Vec4,
    pub radius: f32,
    pub color: [f32; 3],
}

/// Everything the renderer needs for one frame.
pub struct FrameParams<'a> {
    pub eye: Vec4,
    pub basis: Basis,
    pub fov_y_deg: f32,
    pub max_dist: f32,
    pub target: Option<IVec4>,
    pub entities: &'a [EntityDraw],
    pub flags: u32,
    pub ghost_offset: f32,
    pub glasses_range: f32,
    pub time: f32,
    /// Inset rect in output pixels (x, y, w, h).
    pub inset: [f32; 4],
}

pub struct Renderer {
    raycast: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    blit_layout: wgpu::BindGroupLayout,
    blit_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    offscreen: wgpu::TextureView,
    pub store: ChunkStore,
    out_size: (u32, u32),
    render_scale: f32,
    frame: u32,
}

impl Renderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, size: (u32, u32)) -> Self {
        let store = ChunkStore::new(device);
        let raycast_module = device.create_shader_module(wgpu::include_wgsl!("../shaders/raycast.wgsl"));
        let blit_module = device.create_shader_module(wgpu::include_wgsl!("../shaders/blit.wgsl"));

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raycast"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1),
                storage(2),
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raycast"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: store.window.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: store.pool.as_entire_binding() },
            ],
        });

        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline = |label, module: &wgpu::ShaderModule, layout: &wgpu::BindGroupLayout, format| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let raycast = pipeline("raycast", &raycast_module, &bind_layout, OFFSCREEN_FORMAT);
        let blit = pipeline("blit", &blit_module, &blit_layout, target_format);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let (offscreen, blit_group) = Self::make_offscreen(device, &blit_layout, &sampler, size);

        Self {
            raycast,
            blit,
            globals,
            bind_group,
            blit_layout,
            blit_group,
            sampler,
            offscreen,
            store,
            out_size: size,
            render_scale: 1.0,
            frame: 0,
        }
    }

    fn make_offscreen(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: (u32, u32),
    ) -> (wgpu::TextureView, wgpu::BindGroup) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("raycast target"),
            size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        });
        (view, group)
    }

    fn scaled_size(&self) -> (u32, u32) {
        (
            ((self.out_size.0 as f32 * self.render_scale) as u32).max(1),
            ((self.out_size.1 as f32 * self.render_scale) as u32).max(1),
        )
    }

    /// Resize the output and/or change the internal render scale (0.25..=1).
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32), render_scale: f32) {
        let render_scale = render_scale.clamp(0.25, 1.0);
        if size == self.out_size && render_scale == self.render_scale {
            return;
        }
        self.out_size = size;
        self.render_scale = render_scale;
        let (view, group) = Self::make_offscreen(device, &self.blit_layout, &self.sampler, self.scaled_size());
        self.offscreen = view;
        self.blit_group = group;
    }

    /// Upload chunk changes for the chunks in `wanted` around `eye`.
    pub fn sync_world(&mut self, queue: &wgpu::Queue, world: &World, eye: Vec4, wanted: &[IVec4]) {
        let center = mc4d_core::chunk::chunk_of(eye.floor().as_ivec4());
        self.store.sync(queue, world, center, wanted);
    }

    pub fn render(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, p: &FrameParams) {
        self.frame = self.frame.wrapping_add(1);
        let center = self.store.center();
        let origin = (center * CHUNK).as_vec4();
        let v4 = |v: Vec4| v.to_array();
        let (sw, sh) = self.scaled_size();
        let scale = sw as f32 / self.out_size.0 as f32;

        let mut palette = [[0.0f32; 4]; 16];
        for (i, b) in BLOCKS.iter().enumerate().take(BLOCK_COUNT) {
            palette[i] = [b.color[0], b.color[1], b.color[2], 1.0];
        }
        let mut entities = [[0.0f32; 4]; MAX_ENTITIES * 2];
        let n = p.entities.len().min(MAX_ENTITIES);
        for (i, e) in p.entities.iter().take(n).enumerate() {
            entities[i * 2] = v4(e.center - origin);
            entities[i * 2 + 1] = [e.radius, e.color[0], e.color[1], e.color[2]];
        }
        let target_cell = p.target.map(|t| t - center * CHUNK);
        let g = Globals {
            eye: v4(p.eye - origin),
            right: v4(p.basis.right),
            up: v4(p.basis.up),
            forward: v4(p.basis.forward),
            ana: v4(p.basis.ana),
            center: center.to_array(),
            sun: v4(Vec4::new(0.45, 0.8, 0.3, 0.25).normalize()),
            sky_top: [0.30, 0.52, 0.92, 1.0],
            sky_horizon: [0.70, 0.82, 0.96, 1.0],
            target_cell: target_cell.unwrap_or(IVec4::ZERO).to_array(),
            info: [target_cell.is_some() as u32, p.flags, n as u32, self.frame],
            view: [sw as f32, sh as f32, (p.fov_y_deg.to_radians() * 0.5).tan(), p.max_dist],
            inset: [p.inset[0] * scale, p.inset[1] * scale, p.inset[2] * scale, p.inset[3] * scale],
            misc: [p.ghost_offset, p.time, p.glasses_range, 0.0],
            palette,
            entities,
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("raycast"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.offscreen,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.raycast);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.blit);
            pass.set_bind_group(0, &self.blit_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    /// Parse and validate the shaders on the CPU, so WGSL errors show up in `cargo test`
    /// instead of at startup.
    fn validate(src: &str) {
        let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
    }

    #[test]
    fn raycast_shader_is_valid() {
        validate(include_str!("../shaders/raycast.wgsl"));
    }

    #[test]
    fn blit_shader_is_valid() {
        validate(include_str!("../shaders/blit.wgsl"));
    }

    #[test]
    fn globals_size_matches_shader() {
        // 14 vec4 fields + 16 palette entries + 64 entity vec4s.
        assert_eq!(size_of::<super::Globals>(), (14 + 16 + 64) * 16);
    }
}
