use super::*;
use mc4d_core::camera::Orientation;
#[test]
#[ignore = "requires GPU; verifies rain visibility, animation and roof/dry-column occlusion"]
fn rain_planes_are_visible_animated_and_stop_below_roofs() {
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
    let size = (256, 256);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(&device, format, size);
    let world = World::new(7, None);
    let eye = Vec4::new(0.5, 4.5, 0.5, 0.5);
    renderer.sync_world(&device, &queue, &world, eye, &[]);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rain planes validation"),
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
    let mut render = |basis, time, height: i32| {
        renderer.set_weather_field(
            &queue,
            [-12, -12, -12],
            &vec![[height, 0, 0, 7]; 25 * 25 * 25],
        );
        let views = [ViewDraw {
            basis,
            rect: [0.0, 0.0, size.0 as f32, size.1 as f32],
        }];
        let params = FrameParams { particles:&[],
            first_person: None,
            eye,
            basis,
            fov_y_deg: 65.0,
            max_dist: 32.0,
            target: None,
            entities: &[],
            flags: 64,
            ghost_offset: 1.0,
            glasses_range: 8.0,
            time,
            break_progress: 0.0,
            atmosphere: [1.0, 1.0, 0.0, 0.0],
            sun: Vec4::Y,
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
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let result = mapped.to_vec();
        drop(mapped);
        readback.unmap();
        result
    };
    for angle in [0.0_f32, 37.0] {
        let mut orientation = Orientation::default();
        orientation.rotate_forward_ana(angle.to_radians());
        let basis = orientation.basis();
        let dry = render(basis, 0.0, 1 << 17);
        let roof = render(basis, 0.0, 100);
        assert_eq!(
            dry, roof,
            "covered columns cannot rain beneath roof at{angle}"
        );
        let rain = render(basis, 0.0, 0);
        let animated = render(basis, 0.4, 0);
        let changed = rain
            .chunks_exact(4)
            .zip(dry.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > 1000,
            "rain must be visible, not sparse invented cylinders: {changed}"
        );
        assert_ne!(rain, animated, "original rain texture scrolls over time");
    }
}

#[test]
#[ignore = "requires GPU; compares separated weather composition with the original fused equations"]
fn weather_composition_matches_fused_reference_across_views_stereo_and_scale() {
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
    let size = (256, 256);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(&device, format, size);
    let world = World::new(7, None);
    let eye = Vec4::new(0.5, 4.5, 0.5, 0.5);
    renderer.sync_world(&device, &queue, &world, eye, &[]);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("weather composition differential"),
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
    let entities = [EntityDraw {
        center: Vec4::new(0.5, 4.3, 7.0, 0.5),
        radius: 1.35,
        color: [0.7, 0.25, 0.9],
        kind: 0,
        forward: Vec4::Z,
        item: 0,
        item_variant: 0,
        state: 0,
    }];
    let mut render = |count, angle: f32, scale, weather: f32, column: i32, reference| {
        renderer.resize(&device, size, scale);
        renderer.set_weather_field(
            &queue,
            [-12, -12, -12],
            &vec![[column, 0x7fff7fff, 0x40007fff, 7]; 25 * 25 * 25],
        );
        let mut orientation = Orientation::default();
        orientation.rotate_forward_ana(angle.to_radians());
        orientation.add_pitch(-0.13);
        let basis = orientation.basis();
        let bases = mc4d_core::views::slice_bases(basis, count);
        let views: Vec<_> = bases
            .into_iter()
            .zip(mc4d_core::views::view_rects(
                [size.0 as f32, size.1 as f32],
                count,
            ))
            .map(|(basis, rect)| ViewDraw { basis, rect })
            .collect();
        let flags = flags::FANCY
            | flags::GHOSTS
            | if count == 6 { flags::ANAGLYPH } else { 0 }
            | if reference {
                flags::REFERENCE_FUSED_WEATHER
            } else {
                0
            };
        let params = FrameParams { particles:&[],
            first_person: None,
            eye,
            basis,
            fov_y_deg: 65.0,
            max_dist: 32.0,
            target: None,
            entities: &entities,
            flags,
            ghost_offset: 1.0,
            glasses_range: 8.0,
            time: 3.2,
            break_progress: 0.0,
            atmosphere: [1.0, weather, 0.0, 0.0],
            sun: Vec4::Y,
            views: &views,
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&queue, &mut encoder, &target_view, &params);
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
    for (count, angle, scale, weather, column) in [
        (1, 0.0, 1.0, 0.0, 0),
        (2, 37.0, 0.5, 0.0, 0),
        (1, 0.0, 1.0, 1.0, 0),
        (2, 37.0, 1.0, 1.0, 0),
        (4, 37.0, 0.5, 1.0, 0),
        (6, 23.0, 1.0, 1.0, 0),
        (9, 89.9, 0.75, 1.0, 0),
        (2, 37.0, 1.0, 1.0, 100),
        (2, 0.0, 0.5, 1.0, 1 << 16),
    ] {
        let fused = render(count, angle, scale, weather, column, true);
        let separate = render(count, angle, scale, weather, column, false);
        if weather == 0.0 {
            assert_eq!(
                separate, fused,
                "no-weather pixels must be unchanged count{count} scale{scale}"
            );
            continue;
        }
        let max = fused
            .iter()
            .zip(&separate)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        let mean = fused
            .iter()
            .zip(&separate)
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / fused.len() as f64;
        assert!(
            max <= 3 && mean < 0.15,
            "FP16 composition quantization exceeded tolerance: count{count} angle{angle} scale{scale} column{column}: max{max} mean{mean}"
        );
    }
}
