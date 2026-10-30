//! Precipitation composition using the exact camera and weather functions shared
//! with the fused reference shader. World rays write only scene color and distance.
use std::borrow::Cow;

pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DISTANCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
const SOURCE: &str = include_str!("../shaders/raycast.wgsl");
fn between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    let a = source.find(start).expect(start);
    let b = source[a..].find(end).expect(end) + a;
    &source[a..b]
}
fn function<'a>(source: &'a str, name: &str) -> &'a str {
    let a = source.find(&format!("fn {name}(")).expect(name);
    let open = source[a..].find('{').unwrap() + a;
    let mut depth = 0;
    for (offset, c) in source[open..].char_indices() {
        if c == '{' {
            depth += 1;
        }
        if c == '}' {
            depth -= 1;
            if depth == 0 {
                return &source[a..open + offset + 1];
            }
        }
    }
    panic!("unterminated WGSL function {name}")
}
pub fn raycast_source(fused: bool) -> String {
    if fused {
        return SOURCE.into();
    }
    let weather = between(SOURCE, "fn world_snow(", "struct ShadeResult");
    let reference = between(
        SOURCE,
        "    // BEGIN FUSED WEATHER REFERENCE",
        "    // END FUSED WEATHER REFERENCE",
    );
    SOURCE.replace(weather, "").replace(reference, "")
}
/// The scene shader without the entity models, drawn wherever no entity can be
/// visible; their code alone doubled register use for every pixel.
pub fn light_source() -> String {
    let source = raycast_source(false);
    let entities = between(&source, "    // BEGIN ENTITY PASS", "    // END ENTITY PASS").to_owned();
    source.replace(&entities, "")
}
pub fn composite_source() -> String {
    let globals = between(SOURCE, "struct Globals {", "@group(0) @binding(1)");
    let camera = between(SOURCE, "struct Camera {", "// Minecraft-like skin boxes");
    let pixel = between(
        SOURCE,
        "// BEGIN SHARED PIXEL VIEW",
        "// END SHARED PIXEL VIEW",
    );
    let weather = between(SOURCE, "fn world_snow(", "struct ShadeResult");
    format!(
        "const FLAG_FANCY:u32=64u;\nconst FLAG_ANAGLYPH:u32=128u;\n{globals}\n@group(0) @binding(4) var classic_atlas:texture_2d_array<f32>;\n@group(0) @binding(5) var pixel_sampler:sampler;\n@group(0) @binding(7) var precipitation_columns:texture_3d<i32>;\n@group(0) @binding(15) var rain_brightness:texture_2d_array<f32>;\n{}\n{camera}\n{pixel}\n{}\n{weather}\n{}",
        function(SOURCE, "safe_inv"),
        function(SOURCE, "sky_dimming"),
        include_str!("../shaders/weather_composite.wgsl")
    )
}

pub struct WeatherPass {
    pub scene: wgpu::TextureView,
    pub distances: wgpu::TextureView,
    composed: wgpu::TextureView,
    pub scaled_blit: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    group: wgpu::BindGroup,
    full_pipeline: wgpu::RenderPipeline,
    scaled_pipeline: wgpu::RenderPipeline,
}
fn target(
    device: &wgpu::Device,
    label: &str,
    size: (u32, u32),
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default())
}
impl WeatherPass {
    pub fn new(
        device: &wgpu::Device,
        world_layout: &wgpu::BindGroupLayout,
        blit_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        target_format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("weather scene inputs"),
            entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shared precipitation compositor"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(composite_source())),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("weather composition"),
            bind_group_layouts: &[Some(world_layout), Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("weather composition"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
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
        let full_pipeline = pipeline(target_format);
        let scaled_pipeline = pipeline(super::OFFSCREEN_FORMAT);
        let (scene, distances, composed, group, scaled_blit) =
            Self::targets(device, &layout, blit_layout, sampler, size);
        Self {
            scene,
            distances,
            composed,
            scaled_blit,
            layout,
            group,
            full_pipeline,
            scaled_pipeline,
        }
    }
    fn targets(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        blit_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: (u32, u32),
    ) -> (
        wgpu::TextureView,
        wgpu::TextureView,
        wgpu::TextureView,
        wgpu::BindGroup,
        wgpu::BindGroup,
    ) {
        let scene = target(device, "weather linear scene", size, SCENE_FORMAT);
        let distances = target(
            device,
            "weather stereo hit distances",
            size,
            DISTANCE_FORMAT,
        );
        let composed = target(
            device,
            "weather composed at internal resolution",
            size,
            super::OFFSCREEN_FORMAT,
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("weather scene inputs"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&scene),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&distances),
                },
            ],
        });
        let blit = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("weather scaled blit"),
            layout: blit_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&composed),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        (scene, distances, composed, group, blit)
    }
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        blit_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: (u32, u32),
    ) {
        let (scene, distances, composed, group, blit) =
            Self::targets(device, &self.layout, blit_layout, sampler, size);
        self.scene = scene;
        self.distances = distances;
        self.composed = composed;
        self.group = group;
        self.scaled_blit = blit;
    }
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        world_group: &wgpu::BindGroup,
        target: &wgpu::TextureView,
        scaled: bool,
    ) {
        let output = if scaled { &self.composed } else { target };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("weather composition"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(if scaled {
            &self.scaled_pipeline
        } else {
            &self.full_pipeline
        });
        pass.set_bind_group(0, world_group, &[]);
        pass.set_bind_group(1, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracted_weather_and_both_scene_variants_validate() {
        for (name, source) in [
            ("voxel", raycast_source(false)),
            ("fused reference", raycast_source(true)),
            ("composition", composite_source()),
        ] {
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::default(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        assert!(
            !raycast_source(false).contains("fn world_precipitation("),
            "weather must leave the large voxel shader entirely"
        );
        assert!(
            composite_source().contains(function(SOURCE, "world_precipitation")),
            "the compositor must use the exact shared precipitation equations"
        );
    }
}
