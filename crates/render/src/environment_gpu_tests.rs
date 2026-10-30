use super::*;
use mc4d_core::{block, camera::Orientation, chunk::Chunk};
#[test]
#[ignore = "requires GPU; validates bolt occlusion/4D clipping and native fire overlay"]
fn lightning_and_fire_are_visible_without_painting_reference_views() {
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
    let size = (512, 384);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(&device, format, size);
    let mut world = World::new(7, None);
    let mut wanted = Vec::new();
    for x in -1..=0 {
        for z in -1..=0 {
            for w in -1..=0 {
                let c = IVec4::new(x, 0, z, w);
                world.insert_chunk(c, Chunk::uniform(block::AIR));
                wanted.push(c);
            }
        }
    }
    let eye = Vec4::new(0.5, 8.0, -12.0, 0.5);
    let basis = Orientation::default().basis();
    renderer.store.uploads_per_frame = 64;
    renderer.sync_world(&device, &queue, &world, eye, &wanted);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("environment effects"),
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
    let view = target.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (size.0 * size.1 * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let views = [
        ViewDraw {
            basis,
            rect: [0.0, 0.0, size.0 as f32, size.1 as f32],
        },
        ViewDraw {
            basis,
            rect: [352.0, 264.0, 160.0, 120.0],
        },
    ];
    let bolt = EntityDraw {
        center: Vec4::new(0.5, 8.0, 0.0, 0.5),
        radius: 8.5,
        color: [0.45, 0.45, 0.5],
        kind: 24,
        forward: Vec4::Y * 16.0,
        item: 0,
        item_variant: 0,
        state: 0,
    };
    let render = |renderer: &mut Renderer, entities: &[EntityDraw], flags: u32, time: f32| {
        let params = FrameParams { particles:&[],
            eye,
            basis,
            fov_y_deg: 65.0,
            max_dist: 32.0,
            target: None,
            entities,
            flags,
            ghost_offset: 1.0,
            glasses_range: 8.0,
            time,
            break_progress: 0.0,
            atmosphere: [1.0, 0.0, 0.0, 0.0],
            sun: Vec4::Y,
            views: &views,
            first_person: None,
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&queue, &mut encoder, &view, &params);
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
        result
    };
    let clear = render(&mut renderer, &[], 0, 0.0);
    let lit = render(&mut renderer, &[bolt], 0, 0.0);
    assert!(
        lit.chunks_exact(4)
            .zip(clear.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count()
            > 1000
    );
    let mut offslice = bolt;
    offslice.center.w += 4.0;
    assert_eq!(clear, render(&mut renderer, &[offslice], 0, 0.0));
    for x in -12..=12 {
        for y in 0..16 {
            for w in -4..=4 {
                world.set_block(IVec4::new(x, y, -4, w), block::STONE);
            }
        }
    }
    renderer.sync_world(&device, &queue, &world, eye, &wanted);
    let wall = render(&mut renderer, &[], 0, 0.0);
    assert_eq!(
        wall,
        render(&mut renderer, &[bolt], 0, 0.0),
        "solid terrain occludes the bolt"
    );
    for tick in 0..40 {
        render(&mut renderer, &[], 0, tick as f32 / 20.0);
    }
    let base = render(&mut renderer, &[], 0, 2.0);
    let fire = render(&mut renderer, &[], flags::BURNING, 2.0);
    assert!(
        fire.chunks_exact(4)
            .zip(base.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count()
            > 1000
    );
    for y in 264..384usize {
        for x in 352..512usize {
            let i = (y * 512 + x) * 4;
            assert_eq!(
                &base[i..i + 4],
                &fire[i..i + 4],
                "fire overlay must not obscure auxiliary view"
            );
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
    std::fs::create_dir_all(&root).unwrap();
    for (name, pixels) in [("lightning-native.png", lit), ("burning-overlay.png", fire)] {
        let file = std::fs::File::create(root.join(name)).unwrap();
        let mut png = png::Encoder::new(file, size.0, size.1);
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
}
