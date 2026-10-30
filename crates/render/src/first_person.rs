//! Native Beta first-person presentation (`qa`): original equip/swing matrices,
//! a skinned right arm, solid block geometry, and pixel-extruded item sprites.
//! This is a small depth-tested overlay pass, not part of the world ray marcher.
use crate::{FrameParams, atlas, atlas_icons};
use bytemuck::{Pod, Zeroable};
use mc4d_core::{
    block::{self},
    glam::{Mat4, Vec3},
    item::{self, Stack},
};
use wgpu::util::DeviceExt;
// Atlases normalize each item to a 16x16 tile. Even if every pixel were
// isolated, 256 pixels * 6 quad faces * 6 triangle vertices is a strict bound.
const CAPACITY: u64 = 16 * 16 * 6 * 6;
const SKIN_LAYER: u32 = 21;
#[path = "held_blocks.rs"]
mod held_blocks;
#[derive(Clone, Copy, Debug)]
pub struct FirstPersonDraw {
    pub stack: Option<Stack>,
    pub equip: f32,
    pub swing: f32,
    pub light: [u8; 2],
    pub bob: [f32; 2],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    tint: [f32; 3],
    layer: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    transform: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    light: [f32; 4],
    light0: [f32; 4],
    light1: [f32; 4],
    exclusions: [[f32; 4]; 8],
    info: [u32; 4],
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum MeshKind {
    Arm,
    Block,
    Sprite,
}
fn native_block(stack: Stack) -> Option<&'static held_blocks::HeldBlock> {
    if stack.block >= 256 {
        return None;
    }
    let state = item::placement(stack)?;
    held_blocks::BLOCKS.get(block::definition(state.id).reference_id as usize)
}
fn mesh_kind(stack: Option<Stack>) -> MeshKind {
    let Some(stack) = stack else {
        return MeshKind::Arm;
    };
    if native_block(stack).is_some_and(|b| matches!(b.render_type, 0 | 10 | 11 | 13)) {
        MeshKind::Block
    } else {
        MeshKind::Sprite
    }
}
fn translation(x: f32, y: f32, z: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(x, y, z))
}
fn rx(v: f32) -> Mat4 {
    Mat4::from_rotation_x(v.to_radians())
}
fn ry(v: f32) -> Mat4 {
    Mat4::from_rotation_y(v.to_radians())
}
fn rz(v: f32) -> Mat4 {
    Mat4::from_rotation_z(v.to_radians())
}
fn perspective(fov: f32, aspect: f32) -> Mat4 {
    let f = 1.0 / (fov * 0.5).tan();
    let (near, far) = (0.05, 10.0);
    Mat4::from_cols_array_2d(&[
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, far / (near - far), -1.0],
        [0.0, 0.0, near * far / (near - far), 0.0],
    ])
}
fn model_matrix(draw: FirstPersonDraw) -> Mat4 {
    let s = draw.swing.clamp(0.0, 1.0);
    let root = (s.sqrt() * std::f32::consts::PI).sin();
    let twice = (s.sqrt() * std::f32::consts::TAU).sin();
    let sin = (s * std::f32::consts::PI).sin();
    let square = (s * s * std::f32::consts::PI).sin();
    let lower = (1.0 - draw.equip.clamp(0.0, 1.0)) * 0.6;
    let bob = translation(draw.bob[0], draw.bob[1], 0.0);
    if draw.stack.is_none() {
        return bob
            * translation(-root * 0.3, twice * 0.4, -sin * 0.4)
            * translation(0.64, -0.6 - lower, -0.72)
            * ry(45.0)
            * ry(root * 70.0)
            * rz(-square * 20.0)
            * translation(-1.0, 3.6, 3.5)
            * rz(120.0)
            * rx(200.0)
            * ry(-135.0)
            * translation(5.6, 0.0, 0.0);
    }
    let mut m = bob
        * translation(-root * 0.4, twice * 0.2, -sin * 0.2)
        * translation(0.56, -0.52 - lower, -0.72)
        * ry(45.0)
        * ry(-square * 20.0)
        * rz(-root * 20.0)
        * rx(-root * 80.0)
        * Mat4::from_scale(Vec3::splat(0.4));
    // Reflected gb.c() is true only for the fishing rod (346) in Beta1.5.
    if draw.stack.is_some_and(|s| s.block == item::FISHING_ROD) {
        m *= ry(180.0);
    }
    if mesh_kind(draw.stack) == MeshKind::Sprite {
        m = m
            * translation(0.0, -0.3, 0.0)
            * Mat4::from_scale(Vec3::splat(1.5))
            * ry(50.0)
            * rz(335.0)
            * translation(-0.9375, -0.0625, 0.0);
    }
    m
}
fn quad(
    out: &mut Vec<Vertex>,
    p: [Vec3; 4],
    normal: Vec3,
    uv: [[f32; 2]; 4],
    layer: u32,
    tint: [f32; 3],
) {
    for i in [0, 1, 2, 0, 2, 3] {
        out.push(Vertex {
            position: p[i].to_array(),
            normal: normal.to_array(),
            uv: uv[i],
            tint,
            layer,
        });
    }
}
fn face_points(lo: Vec3, hi: Vec3, axis: usize, positive: bool) -> [Vec3; 4] {
    let a = (axis + 1) % 3;
    let b = (axis + 2) % 3;
    let mut p = [lo; 4];
    for (i, (u, v)) in [(false, false), (true, false), (true, true), (false, true)]
        .into_iter()
        .enumerate()
    {
        p[i][axis] = if positive { hi[axis] } else { lo[axis] };
        p[i][a] = if u { hi[a] } else { lo[a] };
        p[i][b] = if v { hi[b] } else { lo[b] };
    }
    p
}
fn uv_rect(origin: [f32; 2], size: [f32; 2], coordinates: [[f32; 2]; 4]) -> [[f32; 2]; 4] {
    coordinates.map(|p| {
        [
            (origin[0] + 0.1 + p[0].clamp(0.0, 1.0) * (size[0] - 0.2)) / 256.0,
            (origin[1] + 0.1 + p[1].clamp(0.0, 1.0) * (size[1] - 0.2)) / 256.0,
        ]
    })
}
fn arm_mesh() -> Vec<Vertex> {
    let mut out = Vec::new();
    let lo = Vec3::new(-8.0, 0.0, -2.0) / 16.0;
    let hi = Vec3::new(-4.0, 12.0, 2.0) / 16.0;
    for axis in 0..3 {
        for positive in [false, true] {
            let p = face_points(lo, hi, axis, positive);
            let mut n = Vec3::ZERO;
            n[axis] = if positive { 1.0 } else { -1.0 };
            let f = p.map(|p| (p - lo) / (hi - lo));
            let (origin, size, coords) = match axis {
                0 => (
                    [if positive { 48.0 } else { 40.0 }, 20.0],
                    [4.0, 12.0],
                    f.map(|f| [if positive { 1.0 - f.z } else { f.z }, f.y]),
                ),
                1 => (
                    [if positive { 48.0 } else { 44.0 }, 16.0],
                    [4.0, 4.0],
                    f.map(|f| [f.x, f.z]),
                ),
                _ => (
                    [if positive { 52.0 } else { 44.0 }, 20.0],
                    [4.0, 12.0],
                    f.map(|f| [if positive { 1.0 - f.x } else { f.x }, f.y]),
                ),
            };
            quad(
                &mut out,
                p,
                n,
                uv_rect(origin, size, coords),
                SKIN_LAYER,
                [1.0; 3],
            );
        }
    }
    out
}
fn block_mesh(stack: Stack, fancy: bool) -> Vec<Vertex> {
    let state = item::placement(stack).unwrap();
    let native = native_block(stack).unwrap();
    let mut out = Vec::new();
    let b = native.bounds;
    let boxes: Vec<(Vec3, Vec3)> = match native.render_type {
        // cn.a(to,int) uses a fixed two-piece inventory staircase.
        10 => vec![
            (Vec3::ZERO, Vec3::new(1.0, 1.0, 0.5)),
            (Vec3::new(0.0, 0.0, 0.5), Vec3::new(1.0, 0.5, 1.0)),
        ],
        // The held fence is two posts and two rails, not an isolated world post.
        11 => vec![
            (Vec3::new(0.375, 0.0, 0.0), Vec3::new(0.625, 1.0, 0.25)),
            (Vec3::new(0.375, 0.0, 0.75), Vec3::new(0.625, 1.0, 1.0)),
            (
                Vec3::new(0.4375, 0.8125, -0.125),
                Vec3::new(0.5625, 0.9375, 1.125),
            ),
            (
                Vec3::new(0.4375, 0.3125, -0.125),
                Vec3::new(0.5625, 0.4375, 1.125),
            ),
        ],
        13 => vec![(
            Vec3::new(0.0625, 0.0, 0.0625),
            Vec3::new(0.9375, 1.0, 0.9375),
        )],
        _ => vec![(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5]))],
    };
    for (lo, hi) in boxes {
        for axis in 0..3 {
            for positive in [false, true] {
                let p = face_points(lo, hi, axis, positive);
                let mut n = Vec3::ZERO;
                n[axis] = if positive { 1.0 } else { -1.0 };
                let face = match (axis, positive) {
                    (0, false) => 4,
                    (0, true) => 5,
                    (1, false) => 0,
                    (1, true) => 1,
                    (2, false) => 2,
                    _ => 3,
                };
                let encoded = native.tiles[(stack.variant & 15) as usize][face];
                let mut tile = encoded.unsigned_abs() as u32;
                if state.id == block::LEAVES && !fancy {
                    tile += 1;
                }
                let coords = p.map(|p| {
                    let mut uv = match axis {
                        0 => [p.z, 1.0 - p.y],
                        1 => [p.x, p.z],
                        _ => [p.x, 1.0 - p.y],
                    };
                    if encoded < 0 {
                        uv[0] = 1.0 - uv[0];
                    }
                    uv
                });
                // qa.a(float) sets only grayscale world brightness. Unlike world
                // grass/foliage, this old held-item path has no biome multiplier.
                quad(
                    &mut out,
                    p.map(|p| p - Vec3::splat(0.5)),
                    n,
                    uv_rect(
                        [(tile % 16 * 16) as f32, (tile / 16 * 16) as f32],
                        [16.0, 16.0],
                        coords,
                    ),
                    0,
                    [1.0; 3],
                );
            }
        }
    }
    out
}
fn sprite_mesh(stack: Stack, pixels: &[u8]) -> Vec<Vertex> {
    let Some(mut icon) = atlas_icons::item_icon_stack(stack) else {
        return Vec::new();
    };
    if let Some(native) = native_block(stack) {
        let tile = native.tiles[(stack.variant & 15) as usize][2];
        icon.tile = tile.unsigned_abs() as u32;
        icon.mirrored = tile < 0;
        icon.tint = [1.0; 3];
    }
    let mut out = Vec::new();
    let ox = icon.tile % 16 * 16;
    let oy = icon.tile / 16 * 16;
    let opaque = |x: i32, y: i32| -> bool {
        if !(0..16).contains(&x) || !(0..16).contains(&y) {
            return false;
        }
        // qa places the right texture edge at model X=0.
        let tx = if icon.mirrored { x } else { 15 - x };
        let at =
            ((icon.layer * 256 * 256 + (oy + y as u32) * 256 + ox + tx as u32) * 4 + 3) as usize;
        pixels.get(at).is_some_and(|a| *a >= 26)
    };
    for y in 0..16i32 {
        for x in 0..16i32 {
            if !opaque(x, y) {
                continue;
            }
            let lo = Vec3::new(x as f32 / 16.0, 1.0 - (y + 1) as f32 / 16.0, -0.0625);
            let hi = lo + Vec3::splat(0.0625);
            // qa places the right texture edge at model X=0.
            let tx = if icon.mirrored { x } else { 15 - x };
            let uv = [
                (ox as f32 + tx as f32 + 0.5) / 256.0,
                (oy as f32 + y as f32 + 0.5) / 256.0,
            ];
            for axis in 0..3 {
                for positive in [false, true] {
                    let neighbor = match (axis, positive) {
                        (0, false) => opaque(x - 1, y),
                        (0, true) => opaque(x + 1, y),
                        (1, false) => opaque(x, y + 1),
                        (1, true) => opaque(x, y - 1),
                        _ => false,
                    };
                    if neighbor {
                        continue;
                    }
                    let mut n = Vec3::ZERO;
                    n[axis] = if positive { 1.0 } else { -1.0 };
                    quad(
                        &mut out,
                        face_points(lo, hi, axis, positive),
                        n,
                        [uv; 4],
                        icon.layer,
                        icon.tint,
                    );
                }
            }
        }
    }
    out
}
pub(super) struct FirstPerson {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    depth: wgpu::TextureView,
    pixels: Vec<u8>,
    key: Option<(Option<(u16, u8)>, bool)>,
    count: u32,
}
impl FirstPerson {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        atlas: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> Self {
        let shader =
            device.create_shader_module(wgpu::include_wgsl!("../shaders/first_person.wgsl"));
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("first-person transforms"),
            contents: bytemuck::bytes_of(&Uniforms::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("held item geometry"),
            size: CAPACITY * size_of::<Vertex>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("first-person"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("first-person"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("first-person"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline=device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {label:Some("first-person"),layout:Some(&pipeline_layout),
            vertex:wgpu::VertexState {module:&shader,entry_point:Some("vs_main"),compilation_options:Default::default(),buffers:&[Some(wgpu::VertexBufferLayout {array_stride:size_of::<Vertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x3,4=>Uint32]})]},
            primitive:wgpu::PrimitiveState {cull_mode:None,..Default::default()},depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:Some(true),depth_compare:Some(wgpu::CompareFunction::Less),stencil:Default::default(),bias:Default::default()}),multisample:Default::default(),
            fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some("fs_main"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState {format,blend:Some(wgpu::BlendState::ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache:None});
        Self {
            pipeline,
            group,
            uniform,
            vertices,
            depth: Self::depth(device, size),
            pixels: Vec::new(),
            key: None,
            count: 0,
        }
    }
    fn depth(device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("first-person depth"),
                size: wgpu::Extent3d {
                    width: size.0.max(1),
                    height: size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        self.depth = Self::depth(device, size);
    }
    pub fn set_atlas(&mut self, pixels: &[u8]) {
        self.pixels = pixels[..(atlas::SIDE * atlas::SIDE * 4 * 2) as usize].to_vec();
        self.key = None;
    }
    pub fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        p: &FrameParams,
    ) {
        let Some(draw) = p.first_person else {
            return;
        };
        let fancy = p.flags & crate::flags::FANCY != 0;
        let key = (draw.stack.map(|s| (s.block, s.variant)), fancy);
        if self.key != Some(key) {
            let mesh = match mesh_kind(draw.stack) {
                MeshKind::Arm => arm_mesh(),
                MeshKind::Block => block_mesh(draw.stack.unwrap(), fancy),
                MeshKind::Sprite => sprite_mesh(draw.stack.unwrap(), &self.pixels),
            };
            assert!(mesh.len() as u64 <= CAPACITY);
            self.count = mesh.len() as u32;
            self.key = Some(key);
            queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&mesh));
        }
        if self.count == 0 {
            return;
        }
        let rect = p
            .views
            .first()
            .map_or([0.0, 0.0, size.0 as f32, size.1 as f32], |v| v.rect);
        if rect[2] <= 0.0 || rect[3] <= 0.0 {
            return;
        }
        let model = model_matrix(draw);
        let mut projection = perspective(p.fov_y_deg.to_radians(), rect[2] / rect[3]);
        // The default 4D auxiliary view occupies Minecraft's hand corner.
        // Preserve native size/pose while shifting its screen projection beside
        // that overlay; otherwise almost the entire hand would be concealed.
        if p.views.len() == 2 && (p.flags & crate::flags::INSET) != 0 {
            let aux = p.views[1].rect;
            if aux[0] > rect[0] + rect[2] * 0.5 && aux[1] > rect[1] + rect[3] * 0.5 {
                let pixels = (rect[0] + rect[2] - aux[0] + 8.0).max(0.0);
                projection = translation(-2.0 * pixels / rect[2], 0.0, 0.0) * projection;
            }
        }
        let mut exclusions = [[0.0; 4]; 8];
        let skip = p.views.len() == 2 && (p.flags & crate::flags::INSET) == 0;
        let count = if skip {
            0
        } else {
            p.views.len().saturating_sub(1).min(8)
        };
        for (i, v) in p.views.iter().skip(1).take(count).enumerate() {
            exclusions[i] = v.rect;
        }
        let rain = 1.0 - p.atmosphere[1].clamp(0.0, 1.0) * 5.0 / 16.0;
        let daylight = p.atmosphere[0].clamp(0.0, 1.0)
            * rain
            * if p.flags & crate::flags::THUNDER != 0 {
                rain
            } else {
                1.0
            };
        let dim = (11.0 * (1.0 - daylight) * (1.0 - p.atmosphere[3].clamp(0.0, 1.0))).floor();
        let level = (draw.light[0] as f32)
            .max(draw.light[1] as f32 - dim)
            .clamp(0.0, 15.0)
            / 15.0;
        let brightness = level / ((1.0 - level) * 3.0 + 1.0) * 0.95 + 0.05;
        let light_direction = |x: f32, z: f32| {
            let world = mc4d_core::glam::Vec4::new(x, 1.0, z, 0.0);
            [
                world.dot(p.basis.right),
                world.dot(p.basis.up),
                -world.dot(p.basis.forward),
                0.0,
            ]
        };
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&Uniforms {
                transform: (projection * model).to_cols_array_2d(),
                model: model.to_cols_array_2d(),
                light: [brightness, 0.0, 0.0, 0.0],
                light0: light_direction(0.2, -0.7),
                light1: light_direction(-0.2, 0.7),
                exclusions,
                info: [count as u32, 0, 0, 0],
            }),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("first-person arm/item"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(rect[0], rect[1], rect[2], rect[3], 0.0, 1.0);
        pass.set_scissor_rect(
            rect[0] as u32,
            rect[1] as u32,
            (rect[2] as u32).max(1),
            (rect[3] as u32).max(1),
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..self.count, 0..1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_tools_have_real_thickness_and_blocks_have_metadata_textures() {
        let pixels = atlas::load(None).unwrap();
        let tool = sprite_mesh(Stack::new(item::DIAMOND_PICKAXE, 1), &pixels);
        assert!(tool.len() > 36);
        assert!(tool.iter().any(|v| v.position[2] < 0.0));
        assert!(tool.iter().any(|v| v.normal[2] == 0.0));
        let white = block_mesh(Stack::with_variant(block::WOOL, 1, 0), true);
        let red = block_mesh(Stack::with_variant(block::WOOL, 1, 14), true);
        assert_ne!(white[0].uv, red[0].uv);
        assert_eq!(arm_mesh().len(), 36);
    }
    #[test]
    fn adversarial_alpha_boundaries_fit_without_dropping_geometry() {
        let stack = Stack::new(item::WOOD_SWORD, 1);
        let icon = atlas_icons::item_icon_stack(stack).unwrap();
        let mut pixels = vec![0u8; 2 * 256 * 256 * 4];
        for y in 0..16u32 {
            for x in 0..16u32 {
                if (x + y) % 2 == 0 {
                    let at = ((icon.layer * 256 * 256
                        + (icon.tile / 16 * 16 + y) * 256
                        + icon.tile % 16 * 16
                        + x)
                        * 4) as usize;
                    pixels[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
        let mesh = sprite_mesh(stack, &pixels);
        assert_eq!(
            mesh.len(),
            128 * 6 * 6,
            "all isolated pixels need front/back and four exposed side faces"
        );
        assert!(mesh.len() as u64 <= CAPACITY);
    }
    #[test]
    fn original_swing_and_equip_transforms_stay_finite() {
        for stack in [
            None,
            Some(Stack::new(block::STONE, 1)),
            Some(Stack::new(item::WOOD_SWORD, 1)),
        ] {
            for swing in [0.0, 0.1, 0.5, 0.99, 1.0] {
                let m = model_matrix(FirstPersonDraw {
                    stack,
                    equip: 1.0,
                    swing,
                    light: [0, 15],
                    bob: [0.0; 2],
                });
                assert!(m.is_finite());
                assert!(m.determinant().abs() > 0.001);
            }
        }
    }
    #[test]
    fn first_person_shader_is_valid() {
        let module =
            naga::front::wgsl::parse_str(include_str!("../shaders/first_person.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    #[test]
    #[ignore = "requires GPU; writes first-person arm/block/extruded-tool review images"]
    fn native_hand_items_swing_and_main_view_clipping_render() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .unwrap();
        let size = (640, 480);
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: atlas::LAYERS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let mut renderer = FirstPerson::new(&device, format, size, &texture_view, &sampler);
        let pixels = atlas::load(None).unwrap();
        renderer.set_atlas(&pixels);
        queue.write_texture(
            texture.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1024),
                rows_per_image: Some(256),
            },
            texture.size(),
        );
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (size.0 * size.1 * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let basis = mc4d_core::camera::Orientation::default().basis();
        let mut views = vec![crate::ViewDraw {
            basis,
            rect: [0.0, 0.0, 640.0, 480.0],
        }];
        let mut arm: Option<Vec<u8>> = None;
        for (name, stack, swing) in [
            ("arm", None, 0.0),
            ("stone", Some(Stack::new(block::STONE, 1)), 0.0),
            ("pickaxe", Some(Stack::new(item::DIAMOND_PICKAXE, 1)), 0.0),
            ("swing", None, 0.4),
            (
                "red-wool",
                Some(Stack::with_variant(block::WOOL, 1, 14)),
                0.0,
            ),
            ("fence", Some(Stack::new(block::FENCE, 1)), 0.0),
            ("slab", Some(Stack::new(block::SLAB, 1)), 0.0),
            ("torch", Some(Stack::new(block::TORCH, 1)), 0.0),
            ("grass", Some(Stack::new(block::GRASS, 1)), 0.0),
            ("inset", None, 0.0),
            ("grid", None, 0.0),
            ("hidden", None, 0.0),
        ] {
            if name == "inset" {
                views.push(crate::ViewDraw {
                    basis,
                    rect: [430.0, 315.0, 198.0, 153.0],
                });
            }
            if name == "grid" {
                views = mc4d_core::views::view_rects([640.0, 480.0], 4)
                    .into_iter()
                    .map(|rect| crate::ViewDraw { basis, rect })
                    .collect();
            }
            let params = FrameParams { particles:&[],
                first_person: if name == "hidden" {
                    None
                } else {
                    Some(FirstPersonDraw {
                        stack,
                        equip: 1.0,
                        swing,
                        light: [0, 15],
                        bob: [0.0; 2],
                    })
                },
                eye: mc4d_core::glam::Vec4::ZERO,
                basis,
                fov_y_deg: 70.0,
                max_dist: 16.0,
                target: None,
                entities: &[],
                flags: crate::flags::INSET,
                ghost_offset: 1.0,
                glasses_range: 8.0,
                time: 0.0,
                break_progress: 0.0,
                atmosphere: [1.0, 0.0, 0.0, 0.0],
                sun: mc4d_core::glam::Vec4::Y,
                views: &views,
            };
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.05,
                                g: 0.1,
                                b: 0.2,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
            renderer.render(&queue, &mut encoder, &target_view, size, &params);
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size.0 * 4),
                        rows_per_image: Some(size.1),
                    },
                },
                target.size(),
            );
            queue.submit([encoder.finish()]);
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let mapped = readback.slice(..).get_mapped_range().unwrap();
            let result = mapped.to_vec();
            drop(mapped);
            readback.unmap();
            let background = &result[..4];
            let changed = result.chunks_exact(4).filter(|p| *p != background).count();
            if name == "hidden" {
                assert_eq!(changed, 0, "hidden HUD must hide the hand pass");
            } else {
                assert!(changed > 100, "{name} geometry is invisible: {changed}");
            }
            if name == "inset" {
                let plain = arm.as_ref().unwrap();
                let full = plain.chunks_exact(4).filter(|p| *p != &plain[..4]).count();
                assert!(
                    changed > full / 2,
                    "auxiliary view must not conceal the default first-person hand"
                );
            }
            if name == "arm" {
                arm = Some(result.clone());
            }
            if name == "swing" {
                assert_ne!(arm.as_ref().unwrap(), &result);
            }
            if name == "inset" || name == "grid" {
                for rect in views.iter().skip(1).map(|v| v.rect) {
                    for y in rect[1] as u32..(rect[1] + rect[3]) as u32 {
                        for x in rect[0] as u32..(rect[0] + rect[2]) as u32 {
                            let i = ((y * size.0 + x) * 4) as usize;
                            assert_eq!(
                                &result[i..i + 4],
                                background,
                                "hand leaked into auxiliary view"
                            );
                        }
                    }
                }
            }
            let root =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
            std::fs::create_dir_all(&root).unwrap();
            let file =
                std::fs::File::create(root.join(format!("first-person-{name}.png"))).unwrap();
            let mut png = png::Encoder::new(file, size.0, size.1);
            png.set_color(png::ColorType::Rgba);
            png.set_depth(png::BitDepth::Eight);
            png.write_header()
                .unwrap()
                .write_image_data(&result)
                .unwrap();
        }
    }
}
