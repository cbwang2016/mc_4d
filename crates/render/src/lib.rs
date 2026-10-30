//! wgpu renderer: ray-casts the 3D slice of the 4D world into an offscreen image at
//! `render_scale` resolution, then upscales it onto the output target.

mod store;
mod fine_occupancy;
mod block_shapes;
mod metadata;
mod light_store;
mod water_fx;
mod thermal_fx;
mod climate;
mod first_person;
mod weather_pass;
pub use first_person::FirstPersonDraw;
mod particles;
pub use particles::ParticleDraw;
pub mod atlas;
pub mod atlas_icons;
pub mod instrument_fx;
#[cfg(test)]
mod gpu_tests;
#[cfg(test)]
mod entity_gpu_tests;
#[cfg(test)]
mod chest_gpu_tests;

pub use store::ChunkStore;

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use mc4d_core::block::{BLOCK_COUNT, BLOCKS};
use mc4d_core::camera::Basis;
use mc4d_core::chunk::CHUNK;
use mc4d_core::glam::{IVec4, Vec4};
use mc4d_core::world::World;
use mc4d_core::views::MAX_VIEWS;

pub const MAX_ENTITIES: usize = 128;
pub const MAX_SIGN_TEXTURES: usize = 64;
pub const SIGN_TEXTURE_WIDTH: u32 = 192;
pub const SIGN_TEXTURE_HEIGHT: u32 = 96;
const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub mod flags {
    pub const GHOSTS: u32 = 1;
    pub const INSET: u32 = 2;
    pub const SHADOWS: u32 = 4;
    pub const GLASSES: u32 = 8;
    pub const EDGES: u32 = 16;
    pub const SMOOTH_LIGHTING: u32 = 32;
    pub const FANCY: u32 = 64;
    pub const ANAGLYPH: u32 = 128;
    pub const THUNDER: u32 = 256;
    pub const BURNING: u32 = 512;
    /// Set by the renderer when the beam prepass supplied ray start distances.
    #[doc(hidden)]
    pub const BEAM: u32 = 0x0800_0000;
    #[doc(hidden)]
    #[doc(hidden)]
    pub const REFERENCE_FUSED_WEATHER:u32=0x2000_0000;
    pub const REFERENCE_VOXEL_STEPS: u32 = 0x4000_0000;
    pub const DISABLE_CULLING: u32 = 0x8000_0000;
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
    palette: [[f32; 4]; BLOCK_COUNT],
    entities: [[f32; 4]; MAX_ENTITIES * 6],
    cameras: [[f32; 4]; MAX_VIEWS * 5],
    weather_origin: [i32; 4],
    gameplay: [f32;4],
    climate_origin: [i32;4],
}

/// Camera and output-pixel rectangle for one visible slice.
pub struct ViewDraw {
    pub basis: Basis,
    pub rect: [f32; 4],
}

/// A 4D entity: Gloom spheres, skinned multipart mob boxes, arrows or dropped items.
#[derive(Clone, Copy, Debug)]
pub struct EntityDraw {
    pub center: Vec4,
    pub radius: f32,
    pub color: [f32; 3],
    /// 0 Gloom; 1..11 ordinary mobs; 12 arrow; 13 dropped item; 14 falling block;
    /// 15/16 standing/wall sign; 17 primed TNT.
    pub kind: u32,
    pub forward: Vec4,
    pub item: mc4d_core::item::ItemId,
    pub item_variant: u8,
    /// Sheared, tamed, sitting, fusing, angry, hurt bits.
    pub state: u32,
}

/// Everything the renderer needs for one frame.
pub struct FrameParams<'a> {
    pub particles: &'a [ParticleDraw],
    pub first_person: Option<FirstPersonDraw>,
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
    pub break_progress: f32,
    /// Daylight, precipitation, snow fraction, lightning flash.
    pub atmosphere: [f32; 4],
    pub sun: Vec4,
    pub views: &'a [ViewDraw],
}

pub struct Renderer {
    raycast: wgpu::RenderPipeline,
    raycast_reference: Option<wgpu::RenderPipeline>,
    /// Entity-free variants (full screen) and the full shader limited to
    /// entity screen rectangles.
    raycast_light: wgpu::RenderPipeline,
    raycast_weather_light: wgpu::RenderPipeline,
    raycast_rects: wgpu::RenderPipeline,
    raycast_weather_rects: wgpu::RenderPipeline,
    entity_rects: wgpu::Buffer,
    /// Kept to record the per-band raycast submissions.
    device: wgpu::Device,
    /// Beam prepass (see cs_beam in raycast.wgsl): per-tile safe ray starts.
    beam: wgpu::ComputePipeline,
    beam_layout: wgpu::BindGroupLayout,
    beam_group: wgpu::BindGroup,
    ray_starts: wgpu::Buffer,
    bind_dirty: bool,
    fine_occupancy: fine_occupancy::FineOccupancy,
    weather_pass: weather_pass::WeatherPass,
    blit: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    block_tiles: wgpu::Buffer,
    block_shapes: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    atlas: wgpu::Texture,
    sign_atlas: wgpu::Texture,
    atlas_sampler: wgpu::Sampler,
    bound_metadata_generation: u64,
    lighting: light_store::LightStore,
    bound_lighting_generation: u64,
    pending_atlas: Option<Vec<u8>>,
    instrument_tiles: Option<[Vec<u8>;2]>,
    weather_field: wgpu::Texture,
    weather_lighting: wgpu::Texture,
    stars: wgpu::Texture,
    pending_stars: Option<Vec<u8>>,
    sky_temperature:f32,
    weather_origin: [i32; 4],
    water_fx: water_fx::WaterFx,
    thermal_fx: thermal_fx::ThermalFx,
    first_person: first_person::FirstPerson,
    particles: particles::ParticlePass,
    fire_overlay: fire_overlay::FireOverlay,
    climate: climate::Climate,
    blit_layout: wgpu::BindGroupLayout,
    blit_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    offscreen: wgpu::TextureView,
    pub store: ChunkStore,
    out_size: (u32, u32),
    render_scale: f32,
    pools_initialized: bool,
    /// Ray-casting pass GPU time, when the device supports timestamp queries.
    pub gpu_timer: Option<gpu_timer::GpuTimer>,
    frame: u32,
}

fn block_texture_table() -> Vec<i32> {
    (0..BLOCK_COUNT).flat_map(|id| {
        mc4d_core::block::definition(id as u8).tiles.into_iter().flatten().map(i32::from)
    }).collect()
}

impl Renderer {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat, size: (u32, u32)) -> Self {
        let store = ChunkStore::new(device);
        let lighting = light_store::LightStore::new(device);
        let climate = climate::Climate::new(device);
        let climate_view = climate.texture.create_view(&Default::default());
        let raycast_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("voxel scene without weather"),source:wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(weather_pass::raycast_source(false)))});
        let blit_module = device.create_shader_module(wgpu::include_wgsl!("../shaders/blit.wgsl"));

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let block_tiles = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("classic block face and metadata tiles"),
            contents: bytemuck::cast_slice(&block_texture_table()),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let block_shapes = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shared block shape geometry"),
            contents: &block_shapes::encode(),
            usage: wgpu::BufferUsages::STORAGE,
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
                storage(3),
                storage(6),
                storage(10),
                storage(8),
                storage(17),
                storage(18),
                storage(9),
                wgpu::BindGroupLayoutEntry {
                    binding:16,visibility:wgpu::ShaderStages::FRAGMENT,
                    ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::Cube,multisampled:false},count:None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 15, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {sample_type:wgpu::TextureSampleType::Float {filterable:false},view_dimension:wgpu::TextureViewDimension::D2Array,multisampled:false},count:None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Sint,
                        view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 12, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 13, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None,
                },
            ],
        });
        let weather_field=device.create_texture(&wgpu::TextureDescriptor {
            label:Some("precipitation column heights"), size:wgpu::Extent3d {width:25,height:25,depth_or_array_layers:25},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D3,format:wgpu::TextureFormat::Rgba32Sint,
            usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[],
        });
        let weather_view=weather_field.create_view(&Default::default());
        let weather_lighting=device.create_texture(&wgpu::TextureDescriptor {
            label:Some("original rain brightness levels"),size:wgpu::Extent3d {width:weather_colors::WIDTH,height:weather_colors::HEIGHT,depth_or_array_layers:weather_colors::LEVELS},
            mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba32Float,
            usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[],
        });
        let weather_lighting_view=weather_lighting.create_view(&Default::default());
        let stars=device.create_texture(&wgpu::TextureDescriptor {label:Some("native Beta star quads"),size:wgpu::Extent3d{width:stars::SIDE,height:stars::SIDE,depth_or_array_layers:6},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::R8Unorm,usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
        let star_view=stars.create_view(&wgpu::TextureViewDescriptor{dimension:Some(wgpu::TextureViewDimension::Cube),..Default::default()});
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Minecraft Beta terrain and skins"),
            size: wgpu::Extent3d { width: atlas::SIDE, height: atlas::SIDE, depth_or_array_layers: atlas::LAYERS },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
        });
        let sign_atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("world sign text cache"),
            size: wgpu::Extent3d { width: SIGN_TEXTURE_WIDTH, height: SIGN_TEXTURE_HEIGHT, depth_or_array_layers: MAX_SIGN_TEXTURES as u32 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let sign_view = sign_atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
        });
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("classic pixel textures"), mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest, mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let ray_starts = Self::ray_start_buffer(device, size);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raycast"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: store.window.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: store.pool.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: store.occupancy.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: store.metadata.buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: lighting.buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: block_tiles.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: block_shapes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&weather_view) },
                wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::TextureView(&weather_lighting_view) },
                wgpu::BindGroupEntry { binding:16,resource:wgpu::BindingResource::TextureView(&star_view)},
                wgpu::BindGroupEntry { binding:17,resource:store.fine.as_entire_binding()},
                wgpu::BindGroupEntry { binding:18,resource:ray_starts.as_entire_binding()},
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&atlas_sampler) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&sign_view) },
                wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&climate_view) },
                wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::Sampler(&climate.sampler) },
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

        let pipeline = |label, module: &wgpu::ShaderModule, layout: &wgpu::BindGroupLayout, formats:&[Option<wgpu::ColorTargetState>], entry:&str| {
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
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: formats,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let color=|format|Some(wgpu::ColorTargetState{format,blend:None,write_mask:wgpu::ColorWrites::ALL});
        let raycast=pipeline("raycast",&raycast_module,&bind_layout,&[color(OFFSCREEN_FORMAT)],"fs_color");
        let light_module=device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("voxel scene without entities"),source:wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(weather_pass::light_source()))});
        let rect_pipeline=|label,module:&wgpu::ShaderModule,formats:&[Option<wgpu::ColorTargetState>],entry:&str| {
            let pl=device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some(label),bind_group_layouts:&[Some(&bind_layout)],immediate_size:0});
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some(label),layout:Some(&pl),
                vertex:wgpu::VertexState{module,entry_point:Some("vs_rects"),compilation_options:Default::default(),
                    buffers:&[Some(wgpu::VertexBufferLayout{array_stride:16,step_mode:wgpu::VertexStepMode::Instance,attributes:&wgpu::vertex_attr_array![0=>Float32x4]})]},
                primitive:wgpu::PrimitiveState::default(),depth_stencil:None,multisample:wgpu::MultisampleState::default(),
                fragment:Some(wgpu::FragmentState{module,entry_point:Some(entry),compilation_options:Default::default(),targets:formats}),
                multiview_mask:None,cache:None})
        };
        let raycast_rects=rect_pipeline("raycast entity tiles",&raycast_module,&[color(OFFSCREEN_FORMAT)],"fs_color");
        let raycast_weather_rects=rect_pipeline("raycast weather entity tiles",&raycast_module,&[color(weather_pass::SCENE_FORMAT),color(weather_pass::DISTANCE_FORMAT)],"fs_main");
        let raycast_light=rect_pipeline("raycast light tiles",&light_module,&[color(OFFSCREEN_FORMAT)],"fs_color");
        let raycast_weather_light=rect_pipeline("raycast weather light tiles",&light_module,&[color(weather_pass::SCENE_FORMAT),color(weather_pass::DISTANCE_FORMAT)],"fs_main");
        let entity_rects=device.create_buffer(&wgpu::BufferDescriptor{label:Some("entity screen rects"),size:(4096*64*16) as u64,usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        #[cfg(not(test))]
        let raycast_reference=None;
        #[cfg(test)]
        let raycast_reference={
            let module=device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("fused precipitation reference"),source:wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(weather_pass::raycast_source(true)))});
            Some(pipeline("fused weather reference",&module,&bind_layout,&[color(OFFSCREEN_FORMAT)],"fs_color"))
        };
        let blit=pipeline("blit",&blit_module,&blit_layout,&[color(target_format)],"fs_main");

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let (offscreen, blit_group) = Self::make_offscreen(device, &blit_layout, &sampler, size);
        let weather_pass=weather_pass::WeatherPass::new(device,&bind_layout,&blit_layout,&sampler,target_format,size);
        let fine_occupancy=fine_occupancy::FineOccupancy::new(device,&store.pool,&store.fine);
        let beam_entry=|binding,ty|wgpu::BindGroupLayoutEntry{binding,visibility:wgpu::ShaderStages::COMPUTE,ty,count:None};
        let storage_ty=|read_only|wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only},has_dynamic_offset:false,min_binding_size:None};
        let beam_layout=device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("beam prepass"),entries:&[
            beam_entry(0,wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None}),
            beam_entry(1,storage_ty(true)),beam_entry(3,storage_ty(true)),beam_entry(19,storage_ty(false))]});
        let beam=device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor{label:Some("beam prepass"),
            layout:Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some("beam prepass"),bind_group_layouts:&[Some(&beam_layout)],immediate_size:0})),
            module:&raycast_module,entry_point:Some("cs_beam"),compilation_options:Default::default(),cache:None});
        let beam_group=Self::beam_group(device,&beam_layout,&globals,&store,&ray_starts);
        let first_person = first_person::FirstPerson::new(device, target_format, size, &atlas_view, &atlas_sampler);
        let fire_overlay=fire_overlay::FireOverlay::new(device,target_format,&atlas_view,&atlas_sampler);
        let particles=particles::ParticlePass::new(device,target_format,&atlas_view,&atlas_sampler,&weather_pass.distances);

        Self {
            raycast,
            raycast_reference,
            raycast_light,
            raycast_weather_light,
            raycast_rects,
            raycast_weather_rects,
            entity_rects,
            device: device.clone(),
            beam,
            beam_layout,
            beam_group,
            ray_starts,
            bind_dirty: false,
            fine_occupancy,
            weather_pass,
            blit,
            globals,
            block_tiles,
            block_shapes,
            bind_group,
            atlas,
            sign_atlas,
            atlas_sampler,
            bound_metadata_generation: 0,
            lighting,
            bound_lighting_generation: 0,
            pending_atlas: Some(atlas::load(None).expect("bundled Minecraft textures decode")),
            instrument_tiles:None,
            weather_field,
            weather_lighting,
            stars,
            pending_stars:Some(stars::pixels()),
            sky_temperature:0.5,
            weather_origin: [0;4],
            water_fx: water_fx::WaterFx::default(),
            thermal_fx: thermal_fx::ThermalFx::default(),
            first_person,
            particles,
            fire_overlay,
            climate,
            blit_layout,
            blit_group,
            sampler,
            offscreen,
            store,
            out_size: size,
            render_scale: 1.0,
            pools_initialized: false,
            gpu_timer: None,
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

    pub fn render_scale(&self) -> f32 { self.render_scale }

    fn ray_start_buffer(device: &wgpu::Device, size: (u32, u32)) -> wgpu::Buffer {
        let tiles = size.0.div_ceil(16).max(1) as u64 * size.1.div_ceil(16).max(1) as u64;
        device.create_buffer(&wgpu::BufferDescriptor { label: Some("beam ray starts"), size: tiles * 8, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false })
    }

    fn beam_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, globals: &wgpu::Buffer, store: &ChunkStore, starts: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("beam prepass"), layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: store.window.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: store.occupancy.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 19, resource: starts.as_entire_binding() }] })
    }

    /// wgpu zero-fills never-written buffers lazily in the first submission that
    /// reads them: ~110 ms for the pools. Windowed play calls this before the
    /// first frame is presented; otherwise the first world sync pays it.
    pub fn initialize_pools(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.pools_initialized { return; }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("initialize pools") });
        for buffer in [&self.store.pool, &self.store.occupancy, &self.store.fine, &self.store.metadata.buffer, &self.lighting.buffer] {
            encoder.clear_buffer(buffer, 0, None);
        }
        queue.submit([encoder.finish()]);
        self.pools_initialized = true;
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
        if size != self.out_size { self.first_person.resize(device, size); }
        self.out_size = size;
        self.render_scale = render_scale;
        let (view, group) = Self::make_offscreen(device, &self.blit_layout, &self.sampler, self.scaled_size());
        self.offscreen = view;
        self.blit_group = group;
        self.weather_pass.resize(device,&self.blit_layout,&self.sampler,self.scaled_size());
        self.ray_starts=Self::ray_start_buffer(device,self.scaled_size());
        self.beam_group=Self::beam_group(device,&self.beam_layout,&self.globals,&self.store,&self.ray_starts);
        self.bind_dirty=true;
        let atlas_view=self.atlas.create_view(&Default::default());
        self.particles.rebind(device,&atlas_view,&self.atlas_sampler,&self.weather_pass.distances);
    }

    /// Upload chunk changes for the chunks in `wanted` around `eye`.
    pub fn sync_world(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, world: &World, eye: Vec4, wanted: &[IVec4]) {
        self.initialize_pools(device, queue);
        self.climate.sync(queue, &world.terrain, eye);
        let point=eye.floor().as_ivec4();self.sky_temperature=world.terrain.climate(point.x,point.z,point.w)[0];
        let center = mc4d_core::chunk::chunk_of(eye.floor().as_ivec4());
        self.store.sync(device, queue, world, center, wanted);
        if !self.store.fine_dirty.is_empty() { self.fine_occupancy.update(device, queue, &mut self.store.fine_dirty); }
        self.lighting.sync(device, queue, world, center, wanted, self.store.uploads_per_frame);
        if self.bind_dirty || self.bound_metadata_generation != self.store.metadata.generation || self.bound_lighting_generation != self.lighting.generation {
            self.bind_dirty=false;
            let atlas_view = self.atlas.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
            });
            let weather_view = self.weather_field.create_view(&Default::default());
            let weather_lighting_view=self.weather_lighting.create_view(&Default::default());
            let star_view=self.stars.create_view(&wgpu::TextureViewDescriptor{dimension:Some(wgpu::TextureViewDimension::Cube),..Default::default()});
            let climate_view = self.climate.texture.create_view(&Default::default());
            let sign_view = self.sign_atlas.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default()
            });
            self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("raycast with resized metadata"), layout: &self.raycast.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.globals.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: self.store.window.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: self.store.pool.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: self.store.occupancy.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.atlas_sampler) },
                    wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&sign_view) },
                    wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(&climate_view) },
                    wgpu::BindGroupEntry { binding: 13, resource: wgpu::BindingResource::Sampler(&self.climate.sampler) },
                    wgpu::BindGroupEntry { binding: 6, resource: self.store.metadata.buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 10, resource: self.lighting.buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&weather_view) },
                wgpu::BindGroupEntry { binding: 15, resource: wgpu::BindingResource::TextureView(&weather_lighting_view) },
                wgpu::BindGroupEntry { binding:16,resource:wgpu::BindingResource::TextureView(&star_view)},
                wgpu::BindGroupEntry { binding:17,resource:self.store.fine.as_entire_binding()},
                wgpu::BindGroupEntry { binding:18,resource:self.ray_starts.as_entire_binding()},
                    wgpu::BindGroupEntry { binding: 8, resource: self.block_tiles.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 9, resource: self.block_shapes.as_entire_binding() },
                ],
            });
            self.bound_metadata_generation = self.store.metadata.generation;
            self.bound_lighting_generation = self.lighting.generation;
        }
    }

    pub fn missing_lighting(&self) -> usize { self.lighting.missing }
    pub fn lighting_bytes(&self) -> u64 { self.lighting.bytes() }

    /// A bounded application-owned text cache supplies already-rasterized signs.
    /// Font bytes remain in the app crate; the renderer only owns GPU pixels.
    pub fn upload_sign_texture(&self, queue: &wgpu::Queue, layer: u32, rgba: &[u8]) {
        assert!(layer < MAX_SIGN_TEXTURES as u32);
        assert_eq!(rgba.len(), (SIGN_TEXTURE_WIDTH * SIGN_TEXTURE_HEIGHT * 4) as usize);
        queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &self.sign_atlas, mip_level: 0,
            origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect: wgpu::TextureAspect::All }, rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SIGN_TEXTURE_WIDTH * 4), rows_per_image: Some(SIGN_TEXTURE_HEIGHT) },
            wgpu::Extent3d { width: SIGN_TEXTURE_WIDTH, height: SIGN_TEXTURE_HEIGHT, depth_or_array_layers: 1 });
    }

    /// Replace any supplied classic terrain/items/mob PNGs; missing files retain
    /// bundled originals. A failed decode leaves the active pack unchanged.
    pub fn set_weather_field(&mut self, queue: &wgpu::Queue, origin: [i32;3], heights: &[[i32;4]]) {
        assert_eq!(heights.len(),25*25*25);
        queue.write_texture(self.weather_field.as_image_copy(),bytemuck::cast_slice(heights),
            wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(25*16),rows_per_image:Some(25)},
            wgpu::Extent3d {width:25,height:25,depth_or_array_layers:25});
        let snow=heights.iter().any(|p|p[0] & (1<<16)!=0 && p[0] & (1<<17)==0);
        let rain=heights.iter().any(|p|p[0] & ((1<<16)|(1<<17))==0);
        self.weather_origin=[origin[0],origin[1],origin[2],25 | if snow {1<<16}else{0} | if rain {1<<17}else{0}];
    }

    pub fn set_texture_pack(&mut self, folder: &std::path::Path) -> Result<(), String> {
        self.pending_atlas = Some(atlas::load(Some(folder))?);
        Ok(())
    }

    pub fn reset_texture_pack(&mut self) -> Result<(), String> {
        self.pending_atlas = Some(atlas::load(None)?);
        Ok(())
    }

    pub fn set_instruments(&mut self,tiles:[Vec<u8>;2]) {self.instrument_tiles=Some(tiles);}

    pub fn render(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, p: &FrameParams) {
        if let Some(pixels)=self.pending_stars.take(){queue.write_texture(self.stars.as_image_copy(),&pixels,wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(stars::SIDE),rows_per_image:Some(stars::SIDE)},wgpu::Extent3d{width:stars::SIDE,height:stars::SIDE,depth_or_array_layers:6});}
        if let Some(pixels) = self.pending_atlas.take() {
            self.first_person.set_atlas(&pixels);
            let rain=weather_colors::rain_layers(&pixels);
            queue.write_texture(self.weather_lighting.as_image_copy(),bytemuck::cast_slice(&rain),
                wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(weather_colors::WIDTH*16),rows_per_image:Some(weather_colors::HEIGHT)},
                wgpu::Extent3d {width:weather_colors::WIDTH,height:weather_colors::HEIGHT,depth_or_array_layers:weather_colors::LEVELS});
            self.water_fx.force_upload();
            self.thermal_fx.force_upload();
            queue.write_texture(self.atlas.as_image_copy(), &pixels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(atlas::SIDE * 4), rows_per_image: Some(atlas::SIDE) },
                wgpu::Extent3d { width: atlas::SIDE, height: atlas::SIDE, depth_or_array_layers: atlas::LAYERS });
        }
        if let Some(tiles)=self.instrument_tiles.take(){
            for (tile,pixels) in [54u32,70u32].into_iter().zip(tiles){
                queue.write_texture(wgpu::TexelCopyTextureInfo{texture:&self.atlas,mip_level:0,origin:wgpu::Origin3d{x:tile%16*16,y:tile/16*16,z:1},aspect:wgpu::TextureAspect::All},&pixels,wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(64),rows_per_image:Some(16)},wgpu::Extent3d{width:16,height:16,depth_or_array_layers:1});
            }
        }
        if let Some(tiles) = self.water_fx.update(p.time) {
            for (tile, pixels) in [205u32, 206u32].into_iter().zip(tiles) {
                queue.write_texture(wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas, mip_level: 0,
                    origin: wgpu::Origin3d { x: (tile % 16) * 16, y: (tile / 16) * 16, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                }, &pixels, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(64), rows_per_image: Some(16) },
                    wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: 1 });
            }
        }
        if let Some(tiles) = self.thermal_fx.update(p.time) {
            for (tile, pixels) in [237u32, 238u32, 31u32, 47u32].into_iter().zip(tiles) {
                queue.write_texture(wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas, mip_level: 0,
                    origin: wgpu::Origin3d { x: (tile % 16) * 16, y: (tile / 16) * 16, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                }, &pixels, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(64), rows_per_image: Some(16) },
                    wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: 1 });
            }
        }
        self.frame = self.frame.wrapping_add(1);
        let center = self.store.center();
        let origin = (center * CHUNK).as_vec4();
        let v4 = |v: Vec4| v.to_array();
        let (sw, sh) = self.scaled_size();
        let scale = sw as f32 / self.out_size.0 as f32;

        let mut palette = [[0.0f32; 4]; BLOCK_COUNT];
        for (i, b) in BLOCKS.iter().enumerate().take(BLOCK_COUNT) {
            palette[i] = [b.color[0], b.color[1], b.color[2], block_shapes::flags(i as u8) as f32];
        }
        let mut entities = [[0.0f32; 4]; MAX_ENTITIES * 6];
        // Choose nearby entities from every visible slice before imposing the
        // capacity limit. Distant mobs cannot hide nearby arrows or dropped items.
        let mut visible: Vec<_> = p.entities.iter().filter(|e| {
            let delta = e.center - p.eye;
            let bound = e.radius * if e.kind == 0 { 1.0 } else if e.kind==21 {3.0} else { 2.5 };
            let margin = if p.flags & flags::GLASSES != 0 { p.glasses_range.max(bound) } else { bound };
            delta.length_squared() < (p.max_dist + bound).powi(2)
                && (p.views.iter().any(|v| delta.dot(v.basis.ana).abs() <= margin)
                    || p.views.is_empty() && delta.dot(p.basis.ana).abs() <= margin)
        }).collect();
        visible.sort_by(|a, b| (b.kind==22).cmp(&(a.kind==22)).then_with(|| (a.center - p.eye).length_squared().total_cmp(&(b.center - p.eye).length_squared())));
        let n = visible.len().min(MAX_ENTITIES);
        // Shader-space bounds of the uploaded entities, for the entity rectangles.
        let entity_bounds:Vec<(Vec4,f32,u32)>=visible.iter().take(n).map(|e|(e.center,e.radius,e.kind)).collect();
        for (i, e) in visible.into_iter().take(n).enumerate() {
            let forward = Vec4::new(e.forward.x, 0.0, e.forward.z, e.forward.w).normalize_or_zero();
            let forward = if forward.length_squared() < 0.5 { -Vec4::Z } else { forward };
            let horizontal = glam::Vec3::new(forward.x, forward.z, forward.w);
            let reference = if horizontal.z.abs() < 0.9 { glam::Vec3::Z } else { glam::Vec3::X };
            let right = if e.kind==21 {
                let axis=(e.state>>5)&3;let sign=if e.state&128!=0{-1.0}else{1.0};
                match axis {0=>glam::Vec3::X*sign,2=>glam::Vec3::Y*sign,_=>glam::Vec3::Z*sign}
            } else if e.kind==22 {
                // The local player's full orientation is known. Deriving right
                // from forward alone introduces a roll discontinuity near W.
                glam::Vec3::new(p.basis.right.x,p.basis.right.z,p.basis.right.w).normalize()
            } else {horizontal.cross(reference).normalize()};
            let ana = right.cross(horizontal).normalize();
            let icon = atlas_icons::item_icon_stack(mc4d_core::item::Stack::with_variant(e.item, 1, e.item_variant));
            entities[i * 6] = v4(e.center - origin);
            entities[i * 6 + 1] = [e.radius, e.color[0], e.color[1], e.color[2]];
            entities[i * 6 + 2] = if e.kind==24 {v4(e.forward)} else {v4(forward)};
            entities[i * 6 + 3] = [right.x, 0.0, right.y, right.z];
            entities[i * 6 + 4] = [ana.x, 0.0, ana.y, ana.z];
            // The local player carries armour in state bits 16..28; f32 is exact
            // only to 24 bits, so it travels in the unused icon-layer field.
            entities[i * 6 + 5] = if e.kind==22 {[22.0, (e.state>>16) as f32, icon.map_or(0.0, |i| (i.layer * 256 + i.tile + 1) as f32), (e.state&0xffff) as f32]} else {[e.kind as f32, icon.map_or(0, |i| i.layer) as f32,
                icon.map_or(1, |i| i.tile) as f32, e.state as f32]};
        }
        let target_cell = p.target.map(|t| t - center * CHUNK);
        let mut cameras = [[0.0; 4]; MAX_VIEWS * 5];
        for (i, view) in p.views.iter().take(MAX_VIEWS).enumerate() {
            cameras[i * 5] = v4(view.basis.right);
            cameras[i * 5 + 1] = v4(view.basis.up);
            cameras[i * 5 + 2] = v4(view.basis.forward);
            cameras[i * 5 + 3] = v4(view.basis.ana);
            cameras[i * 5 + 4] = view.rect.map(|v| v * scale);
        }
        let (sky_top,sky_horizon)=sky_colors::colors(self.sky_temperature,p.atmosphere[0],p.atmosphere[1],if p.flags & flags::THUNDER!=0{p.atmosphere[1]}else{0.0},p.atmosphere[3]);
        // Exact traversal references (tests) keep starting every ray at the eye.
        let beam = p.flags & (flags::REFERENCE_VOXEL_STEPS|flags::DISABLE_CULLING)==0;
        let g = Globals {
            eye: v4(p.eye - origin),
            right: v4(p.basis.right),
            up: v4(p.basis.up),
            forward: v4(p.basis.forward),
            ana: v4(p.basis.ana),
            center: center.to_array(),
            sun: v4(p.sun),
            sky_top,
            sky_horizon,
            target_cell: target_cell.unwrap_or(IVec4::ZERO).to_array(),
            info: [target_cell.is_some() as u32, p.flags | if beam {flags::BEAM} else {0}, n as u32, self.frame],
            view: [sw as f32, sh as f32, (p.fov_y_deg.to_radians() * 0.5).tan(), p.max_dist],
            inset: p.atmosphere,
            misc: [p.ghost_offset, p.time, p.glasses_range, p.views.len().min(MAX_VIEWS) as f32],
            palette,
            entities,
            cameras,
            weather_origin: self.weather_origin,
            climate_origin: self.climate.origin,
            gameplay: [p.break_progress.clamp(0.0,1.0),0.0,0.0,0.0],
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));

        let reference=p.flags & flags::REFERENCE_FUSED_WEATHER!=0 && self.raycast_reference.is_some();
        let timed;
        let (spans,bands)=tile_spans(&entity_screen_rects(p,&entity_bounds,(sw,sh),scale),(sw,sh));
        queue.write_buffer(&self.entity_rects,0,bytemuck::cast_slice(&spans));
        let weather=!reference && ((p.atmosphere[1]>0.001 && self.weather_origin[3]!=0)||!p.particles.is_empty());
        {
            let color=|view|Some(wgpu::RenderPassColorAttachment {view,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color::BLACK),store:wgpu::StoreOp::Store}});
            let attachments=if weather {vec![color(&self.weather_pass.scene),color(&self.weather_pass.distances)]}else{vec![color(&self.offscreen)]};
            let timestamps=self.gpu_timer.as_ref().and_then(|t|t.writes());
            timed=timestamps.is_some();
            if reference {
                if beam {
                    let mut compute=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor{label:Some("beam prepass"),timestamp_writes:None});
                    compute.set_pipeline(&self.beam);compute.set_bind_group(0,&self.beam_group,&[]);
                    compute.dispatch_workgroups(sw.div_ceil(16).div_ceil(8),sh.div_ceil(16).div_ceil(8),1);
                }
                let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("raycast"),color_attachments:&attachments,depth_stencil_attachment:None,timestamp_writes:timestamps,occlusion_query_set:None,multiview_mask:None});
                pass.set_bind_group(0,&self.bind_group,&[]);
                pass.set_pipeline(self.raycast_reference.as_ref().unwrap());pass.draw(0..3,0..1);
            } else {
                let (query_set,begin,end)=timestamps.map_or((None,None,None),|t|(Some(t.query_set),t.beginning_of_pass_write_index,t.end_of_pass_write_index));
                for (index,&(start,heavy,total)) in bands.iter().enumerate() {
                    let last=index+1==bands.len();
                    let load=|op:wgpu::LoadOp<wgpu::Color>|attachments.iter().map(|a|a.clone().map(|mut a|{a.ops.load=op;a})).collect::<Vec<_>>();
                    let band_attachments=load(if index==0 {wgpu::LoadOp::Clear(wgpu::Color::BLACK)} else {wgpu::LoadOp::Load});
                    let writes=query_set.filter(|_|index==0||last).map(|query_set|wgpu::RenderPassTimestampWrites{query_set,beginning_of_pass_write_index:if index==0{begin}else{None},end_of_pass_write_index:if last{end}else{None}});
                    let mut own=(!last).then(||self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor{label:Some("raycast band")}));
                    {
                        let target_encoder=own.as_mut().unwrap_or(&mut *encoder);
                        if index==0 && beam {
                            let mut compute=target_encoder.begin_compute_pass(&wgpu::ComputePassDescriptor{label:Some("beam prepass"),timestamp_writes:None});
                            compute.set_pipeline(&self.beam);compute.set_bind_group(0,&self.beam_group,&[]);
                            compute.dispatch_workgroups(sw.div_ceil(16).div_ceil(8),sh.div_ceil(16).div_ceil(8),1);
                        }
                        let mut pass=target_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("raycast band"),color_attachments:&band_attachments,depth_stencil_attachment:None,timestamp_writes:writes,occlusion_query_set:None,multiview_mask:None});
                        pass.set_bind_group(0,&self.bind_group,&[]);
                        pass.set_vertex_buffer(0,self.entity_rects.slice(..(spans.len().max(1)*16) as u64));
                        if heavy>0 {pass.set_pipeline(if weather{&self.raycast_weather_rects}else{&self.raycast_rects});pass.draw(0..6,start..start+heavy);}
                        if total>heavy {pass.set_pipeline(if weather{&self.raycast_weather_light}else{&self.raycast_light});pass.draw(0..6,start+heavy..start+total);}
                    }
                    if let Some(own)=own {queue.submit([own.finish()]);}
                }
            }
        }
        if let Some(timer)=&mut self.gpu_timer {timer.resolve(encoder,timed);}
        let scaled=self.scaled_size()!=self.out_size;
        if weather {self.weather_pass.render(encoder,&self.bind_group,target,scaled);}
        if !weather || scaled {
            let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {label:Some("blit"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:target,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Clear(wgpu::Color::BLACK),store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
            pass.set_pipeline(&self.blit);pass.set_bind_group(0,if weather{&self.weather_pass.scaled_blit}else{&self.blit_group},&[]);pass.draw(0..3,0..1);
        }
        if !reference {self.particles.render(queue,encoder,target,self.out_size,(sw,sh),p);}
        self.first_person.render(queue, encoder, target, self.out_size, p);
        self.fire_overlay.render(queue,encoder,target,self.out_size,p);
    }
}

const TILE: u32 = 32;
/// Separate GPU submissions per frame: a single long job delayed the
/// compositor's work behind it, which held swapchain images (35-55 ms acquire
/// waits with only 10-15 ms of ray casting, measured).
const BANDS: u32 = 4;

/// Split the frame into non-overlapping row spans of 32-pixel tiles: those an
/// entity rectangle touches (full shader) and the rest (entity-free shader), so
/// every pixel is shaded exactly once however many entities overlap.
fn tile_spans(rects: &[[f32; 4]], size: (u32, u32)) -> (Vec<[f32; 4]>, Vec<(u32, u32, u32)>) {
    let (tw, th) = (size.0.div_ceil(TILE), size.1.div_ceil(TILE));
    let mut covered = vec![false; (tw * th) as usize];
    for r in rects {
        let (x0, x1) = ((r[0].max(0.0) as u32 / TILE).min(tw - 1), ((r[2].max(0.0) as u32).saturating_sub(1) / TILE).min(tw - 1));
        let (y0, y1) = ((r[1].max(0.0) as u32 / TILE).min(th - 1), ((r[3].max(0.0) as u32).saturating_sub(1) / TILE).min(th - 1));
        for y in y0..=y1 { for x in x0..=x1 { covered[(y * tw + x) as usize] = true; } }
    }
    let ndc = |x0: u32, y0: u32, x1: u32, y1: u32| {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let (x1, y1) = ((x1 * TILE).min(size.0) as f32, (y1 * TILE).min(size.1) as f32);
        [(x0 * TILE) as f32 / w * 2.0 - 1.0, 1.0 - (y0 * TILE) as f32 / h * 2.0, x1 / w * 2.0 - 1.0, 1.0 - y1 / h * 2.0]
    };
    // Rows are grouped into bands, each submitted separately (see render()).
    let (mut spans, mut bands) = (Vec::new(), Vec::new());
    for band in 0..BANDS {
        let (r0, r1) = (th * band / BANDS, th * (band + 1) / BANDS);
        let start = spans.len() as u32;
        for pass_heavy in [true, false] {
            for y in r0..r1 {
                let mut x = 0;
                while x < tw {
                    let state = covered[(y * tw + x) as usize];
                    let begin = x;
                    while x < tw && covered[(y * tw + x) as usize] == state { x += 1; }
                    if state == pass_heavy { spans.push(ndc(begin, y, x, y + 1)); }
                }
            }
            if pass_heavy { bands.push((start, spans.len() as u32 - start, 0)); }
        }
        bands.last_mut().unwrap().2 = spans.len() as u32 - start;
    }
    (spans, bands)
}

/// Conservative screen rectangles (internal pixels: x0, y0, x1, y1) that may
/// show any uploaded entity in any view, from the same bounds the shader tests:
/// the 4D bounding sphere cut by each view's slice (a 3D sphere), and for 4D
/// glasses the outline ring around the slice projection.
fn entity_screen_rects(p: &FrameParams, bounds: &[(Vec4, f32, u32)], size: (u32, u32), scale: f32) -> Vec<[f32; 4]> {
    let mut rects = Vec::new();
    if bounds.is_empty() { return rects; }
    let full = [[0.0, 0.0, size.0 as f32, size.1 as f32]];
    let views: Vec<(mc4d_core::camera::Basis, [f32; 4])> = if p.views.is_empty() { vec![(p.basis, full[0])] }
        else { p.views.iter().take(MAX_VIEWS).map(|v| (v.basis, v.rect.map(|c| c * scale))).collect() };
    let th = (p.fov_y_deg.to_radians() * 0.5).tan();
    let stereo = if p.flags & flags::ANAGLYPH != 0 { 0.035 } else { 0.0 };
    let glasses = p.flags & flags::GLASSES != 0;
    for (index, (basis, rect)) in views.iter().enumerate() {
        let aspect = rect[2] / rect[3];
        for &(center, radius, kind) in bounds {
            let bound = radius * if kind == 0 { 1.0 } else if kind == 21 { 3.0 } else { 2.5 };
            let rel = center - p.eye;
            let off = rel.dot(basis.ana);
            let ring = glasses && index == 0 && kind < 12 && off.abs() < p.glasses_range;
            let sphere = (bound * bound - off * off).max(0.0).sqrt();
            let rs = if ring { sphere.max(radius) } else if off.abs() < bound { sphere } else { continue } + stereo + 0.05;
            let cp = rel - basis.ana * off;
            let (x, y, z) = (cp.dot(basis.right), cp.dot(basis.up), cp.dot(basis.forward));
            // Entirely behind the eye: rays only travel forward.
            if z + rs <= 0.0 { continue; }
            let (mut x0, mut y0, mut x1, mut y1) = (rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]);
            if z - rs > 0.05 {
                // The sphere lies in the box x±rs, y±rs, z±rs; its projection lies
                // within the extreme projections of the box corners.
                let (mut qx0, mut qx1, mut qy0, mut qy1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
                for zz in [z - rs, z + rs] { for xx in [x - rs, x + rs] { qx0 = qx0.min(xx / zz); qx1 = qx1.max(xx / zz); }
                    for yy in [y - rs, y + rs] { qy0 = qy0.min(yy / zz); qy1 = qy1.max(yy / zz); } }
                let px = |q: f32| rect[0] + (q / (th * aspect) + 1.0) * 0.5 * rect[2] - 0.5;
                let py = |q: f32| rect[1] + (1.0 - q / th) * 0.5 * rect[3] - 0.5;
                x0 = x0.max(px(qx0) - 3.0); x1 = x1.min(px(qx1) + 3.0);
                y0 = y0.max(py(qy1) - 3.0); y1 = y1.min(py(qy0) + 3.0);
            }
            if x1 <= x0 || y1 <= y0 { continue; }
            rects.push([x0, y0, x1, y1]);
        }
    }
    rects
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
    fn projection_weights_are_continuous_finite_and_normalized_near_axes() {
        // Matches texture_projection_weights in WGSL; the GPU fixtures also
        // verify the resulting rendered texture has variation on both axes.
        let weights = |v: [f32; 3]| {
            let v = v.map(f32::abs);
            let scale = v.into_iter().fold(0.0_f32, f32::max);
            if scale == 0.0 { return [1.0, 0.0, 0.0]; }
            let q = v.map(|x| (x / scale).powi(4));
            let sum: f32 = q.into_iter().sum();
            q.map(|x| x / sum)
        };
        for v in [[0.0; 3], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0],
            [1e-30, 1e-30, 0.0], [0.0, 1e-30, -2e-30], [0.5, 0.5, 0.5]] {
            let w = weights(v);
            assert!(w.iter().all(|x| x.is_finite() && (0.0..=1.0).contains(x)));
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        }
        let at = |a: f32| weights([a.cos(), a.sin(), 0.0]);
        for degrees in [0.0_f32, 45.0, 90.0] {
            let before = at((degrees - 0.0001).to_radians());
            let after = at((degrees + 0.0001).to_radians());
            assert!(before.iter().zip(after).all(|(a, b)| (a - b).abs() < 0.0001));
        }
    }

    #[test]
    fn uploaded_tile_table_matches_every_registered_face_and_variant() {
        let table = super::block_texture_table();
        assert_eq!(table.len(), mc4d_core::block::BLOCK_COUNT * 96);
        for &id in mc4d_core::block::ALL_BLOCKS {
            for metadata in 0..16u8 { for face in 0..6usize {
                let encoded = table[id as usize * 96 + metadata as usize * 6 + face];
                let state = mc4d_core::block::BlockState::new(id, metadata);
                assert_eq!(encoded.unsigned_abs() as u16, mc4d_core::block::texture_tile(state, face));
                assert_eq!(encoded < 0, mc4d_core::block::texture_mirrored(state, face));
            }}
        }
    }

    #[test]
    fn globals_size_matches_shader() {
        let module = naga::front::wgsl::parse_str(include_str!("../shaders/raycast.wgsl")).expect("WGSL parses");
        let shader_size = module.types.iter().find_map(|(_, ty)| {
            if ty.name.as_deref() == Some("Globals") {
                if let naga::TypeInner::Struct { span, .. } = ty.inner { return Some(span as usize); }
            }
            None
        }).expect("Globals uniform exists");
        assert_eq!(size_of::<super::Globals>(), shader_size, "Rust and WGSL uniform layout must agree");
    }
}

#[cfg(test)]
mod weather_gpu_tests;

mod weather_colors;

#[cfg(test)]
mod vehicle_gpu_tests;

mod stars;

pub mod pack;

pub mod gpu_timer;

#[cfg(test)]
mod sky_gpu_tests;

mod sky_colors;

mod fire_overlay;

#[cfg(test)]mod environment_gpu_tests;

#[cfg(test)]
mod partial_light_gpu_tests;

#[cfg(test)]
mod particle_gpu_tests;
