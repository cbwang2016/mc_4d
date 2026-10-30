use super::*;
use mc4d_core::camera::Orientation;
#[test]
#[ignore = "requires GPU; native sky/day/night/cloud and third-person avatar captures"]
fn native_sky_and_player_render_in_rotated_slices() {
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
    let size = (512, 512);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(&device, format, size);
    let mut world = World::new(7, None);
    let mut wanted = Vec::new();
    for x in -1..=0 {
        for z in -1..=0 {
            for w in -1..=0 {
                let c = IVec4::new(x, 0, z, w);
                world.insert_chunk(c, mc4d_core::chunk::Chunk::uniform(mc4d_core::block::AIR));
                wanted.push(c);
            }
        }
    }
    for x in -8..8 {
        for z in -8..8 {
            for w in -8..8 {
                world.set_block(IVec4::new(x, 0, z, w), mc4d_core::block::PLANKS);
            }
        }
    }
    crate::gpu_tests::settle_lighting(&mut world, &wanted);
    renderer.store.uploads_per_frame = 64;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("native sky review"),
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
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
    std::fs::create_dir_all(&root).unwrap();
    let render = |renderer: &mut Renderer,
                  eye: Vec4,
                  basis: Basis,
                  day: f32,
                  fancy: bool,
                  entities: &[EntityDraw],
                  name: &str| {
        renderer.sync_world(&device, &queue, &world, eye, &wanted);
        let views = [ViewDraw {
            basis,
            rect: [0.0, 0.0, size.0 as f32, size.1 as f32],
        }];
        let params = FrameParams { particles:&[],
            first_person: None,
            eye,
            basis,
            fov_y_deg: 65.0,
            max_dist: 80.0,
            target: None,
            entities,
            flags: if fancy { 64 } else { 0 },
            ghost_offset: 1.0,
            glasses_range: 8.0,
            time: 0.0,
            break_progress: 0.0,
            atmosphere: [day, 0.0, 0.0, 0.0],
            sun: if name.contains("sunset") {
                Vec4::Z
            } else if day > 0.5 {
                Vec4::Y
            } else {
                -Vec4::Y
            },
            views: &views,
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
        let map = readback.slice(..).get_mapped_range().unwrap();
        let pixels = map.to_vec();
        drop(map);
        readback.unmap();
        if !name.is_empty() {
            let mut png = png::Encoder::new(
                std::fs::File::create(root.join(name)).unwrap(),
                size.0,
                size.1,
            );
            png.set_color(png::ColorType::Rgba);
            png.set_depth(png::BitDepth::Eight);
            png.write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
        pixels
    };
    let mut orient = Orientation::default();
    orient.add_pitch(1.25);
    let eye = Vec4::new(0.5, 130.0, 0.5, 0.5);
    let day = render(
        &mut renderer,
        eye,
        orient.basis(),
        1.0,
        true,
        &[],
        "sky-native-day.png",
    );
    let night = render(
        &mut renderer,
        eye,
        orient.basis(),
        0.0,
        true,
        &[],
        "sky-native-night.png",
    );
    assert_ne!(day, night);
    queue.write_texture(
        renderer.stars.as_image_copy(),
        &vec![0; (stars::SIDE * stars::SIDE * 6) as usize],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stars::SIDE),
            rows_per_image: Some(stars::SIDE),
        },
        wgpu::Extent3d {
            width: stars::SIDE,
            height: stars::SIDE,
            depth_or_array_layers: 6,
        },
    );
    let blank = render(&mut renderer, eye, orient.basis(), 0.0, true, &[], "");
    let stars = night
        .chunks_exact(4)
        .zip(blank.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(stars > 20 && stars < 10000, "native star pixels={stars}");
    let sunset = render(
        &mut renderer,
        eye,
        Orientation::default().basis(),
        0.5,
        true,
        &[],
        "sky-native-sunset.png",
    );
    assert_ne!(sunset, day);
    let mut diagonal=Orientation::default();diagonal.rotate_forward_ana(std::f32::consts::FRAC_PI_4);
    let tilted_sunset=render(&mut renderer,eye,diagonal.basis(),0.5,true,&[],"sky-native-sunset-w.png");
    assert!(tilted_sunset[(255*512+255)*4+2]>150,"negative solar plane offset must not produce NaN or erase the sun");
    let mut orient = Orientation::default();
    orient.add_pitch(0.25);
    let eye = Vec4::new(0.5, 100.0, 0.5, 0.5);
    let fast = render(
        &mut renderer,
        eye,
        orient.basis(),
        1.0,
        false,
        &[],
        "sky-clouds-fast.png",
    );
    let fancy = render(
        &mut renderer,
        eye,
        orient.basis(),
        1.0,
        true,
        &[],
        "sky-clouds-fancy.png",
    );
    assert_ne!(fast, fancy);
    for angle in [0.0_f32, 37.0, 89.9] {
        let mut o = Orientation::default();
        o.rotate_forward_ana(angle.to_radians());
        let basis = o.basis();
        let eye = Vec4::Y * 2.62 - basis.forward * 4.0;
        let plain = render(&mut renderer, eye, basis, 1.0, true, &[], "");
        let entity = EntityDraw {
            center: Vec4::Y,
            radius: 0.9375,
            color: [1.0; 3],
            kind: 22,
            forward: basis.forward_h,
            item: 0,
            item_variant: 0,
            state: 0,
        };
        let shown = render(
            &mut renderer,
            eye,
            basis,
            1.0,
            true,
            &[entity],
            &format!("third-person-{angle}.png"),
        );
        if angle==0.0 {
            let looking=EntityDraw {state:192<<8,..entity};
            let tilted=render(&mut renderer,eye,basis,1.0,true,&[looking],"third-person-head-pitch.png");
            assert_ne!(shown,tilted,"head and hat follow the actual look pitch");
        }
        let count = plain
            .chunks_exact(4)
            .zip(shown.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(count > 200, "player missing at{angle}:{count}");
        let xs: Vec<_> = plain
            .chunks_exact(4)
            .zip(shown.chunks_exact(4))
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, _)| i % size.0 as usize)
            .collect();
        assert!(
            xs.iter().max().unwrap() - xs.iter().min().unwrap() > 50,
            "primary avatar collapsed across W"
        );
    }
}
