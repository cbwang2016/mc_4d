//! Opt-in pixel comparisons against the original voxel-step traversal.
use super::*;
use mc4d_core::{block, camera::Orientation, chunk::Chunk};

/// Synthetic GPU fixtures must provide a known vertical sky boundary rather
/// than relying on the old unlit-world fallback to make every surface bright.
pub(super) fn settle_lighting(world: &mut World, wanted: &[IVec4]) {
    let columns: std::collections::HashSet<_> = wanted.iter().map(|c| (c.x, c.z, c.w)).collect();
    for (x, z, w) in columns {
        for y in 0..mc4d_core::chunk::WORLD_CHUNKS_Y {
            let c = IVec4::new(x, y, z, w);
            if !world.chunks.contains_key(&c) { world.insert_chunk(c, Chunk::uniform(block::AIR)); }
        }
    }
    let register: Vec<_> = world.chunks.iter().filter(|(c, chunk)| world.light_chunk(**c).is_none_or(|light| !light.matches(chunk)))
        .map(|(&c, chunk)| (c, chunk.clone())).collect();
    for (c, chunk) in register { world.insert_chunk(c, chunk); }
    let start = std::time::Instant::now();
    while !world.lighting_ready(wanted) {
        world.update_lighting(wanted, 128);
        assert!(start.elapsed().as_secs() < 30, "fixture lighting did not settle: {} pending, {} missing", world.lighting_pending(), world.missing_lighting(wanted));
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
#[ignore = "requires GPU; validates actual dark caves, torch removal, skylight and night lighting"]
fn propagated_light_changes_rendered_caves_and_survives_source_removal() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: None,
        force_fallback_adapter: false, apply_limit_buckets: false,
    })).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(), ..Default::default()
    })).unwrap();
    let size = 256u32; let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lighting proof"), size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: Some("lighting readback"),
        size: (size * size * 4) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut world = World::new(21, None);
    let c = IVec4::ZERO;
    let mut cave = Chunk::uniform(block::STONE);
    for w in 2..14 { for z in 2..14 { for y in 2..8 { for x in 2..14 { cave.set(IVec4::new(x, y, z, w), block::AIR); }}}}
    world.insert_chunk(c, cave);
    let wanted = [c]; let eye = Vec4::new(8.0, 4.0, 4.0, 8.0);
    let basis = Orientation::default().basis(); let views = [ViewDraw { basis, rect: [0.0, 0.0, size as f32, size as f32] }];
    let mut params = FrameParams { particles:&[], first_person: None, eye, basis, fov_y_deg: 70.0, max_dist: 32.0, target: None, break_progress: 0.0, entities: &[], flags: 0,
        ghost_offset: 1.0, glasses_range: 8.0, time: 0.0, atmosphere: [1.0, 0.0, 0.0, 0.0],
        sun: Vec4::new(0.4, 0.8, 0.3, 0.1).normalize(), views: &views };
    let mut renderer = Renderer::new(&device, format, (size, size));
    let draw = |renderer: &mut Renderer, world: &mut World, params: &FrameParams| {
        settle_lighting(world, &wanted);
        renderer.sync_world(&device, &queue, world, eye, &wanted);
        assert_eq!(renderer.missing_lighting(), 0);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&queue, &mut encoder, &view, params);
        encoder.copy_texture_to_buffer(target.as_image_copy(), wgpu::TexelCopyBufferInfo {
            buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size * 4), rows_per_image: Some(size) },
        }, target.size());
        queue.submit([encoder.finish()]); readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixels = readback.slice(..).get_mapped_range().unwrap().to_vec(); readback.unmap(); pixels
    };
    let dark = draw(&mut renderer, &mut world, &params);
    let torch = IVec4::new(6, 4, 8, 8);
    world.set_block(torch, block::TORCH);
    let lit = draw(&mut renderer, &mut world, &params);
    let mean = |pixels: &[u8]| pixels.chunks_exact(4).map(|p| (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0).sum::<f64>() / (size * size) as f64;
    assert!(mean(&dark) < 12.0, "closed caves must stay dark, got {}", mean(&dark));
    assert!(mean(&lit) > mean(&dark) + 8.0, "placed torch must light surrounding geometry, not just its own sprite");
    params.atmosphere[0] = 0.0;
    assert_eq!(draw(&mut renderer, &mut world, &params), lit, "torch-lit sealed caves must not dim at night");
    params.atmosphere[0] = 1.0;
    world.set_block(torch, block::AIR);
    let removed = draw(&mut renderer, &mut world, &params);
    assert_eq!(removed, dark, "removing the source must remove propagated light from the rendered cave");
    params.atmosphere = [0.0, 1.0, 0.0, 1.0];
    let flash = draw(&mut renderer, &mut world, &params);
    assert_eq!(flash, dark, "outdoor lightning must not globally illuminate a sealed cave");
    for y in 8..16 { world.set_block(IVec4::new(8, y, 8, 8), block::AIR); }
    params.atmosphere = [1.0, 0.0, 0.0, 0.0];
    let daylight = draw(&mut renderer, &mut world, &params);
    params.atmosphere[0] = 0.0;
    let night = draw(&mut renderer, &mut world, &params);
    assert!(mean(&daylight) > mean(&night) + 8.0, "day/night must modulate propagated skylight independently");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
    std::fs::create_dir_all(&root).unwrap();
    for (name, pixels) in [("lighting-cave-dark", dark), ("lighting-cave-torch", lit), ("lighting-cave-source-removed", removed), ("lighting-cave-skylight", daylight), ("lighting-cave-night", night)] {
        let file = std::fs::File::create(root.join(format!("{name}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, size, size); encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
    }
}

#[test]
#[ignore = "requires GPU; verifies sparse metadata allocation, resize/rebind, reloads and variant pixels"]
fn metadata_states_survive_sparse_pool_growth_and_chunk_reload() {
    use mc4d_core::chunk::{CHUNK_VOLUME, local_index};
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: None,
        force_fallback_adapter: false, apply_limit_buckets: false,
    })).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(), ..Default::default()
    })).unwrap();
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("metadata variant pixels"), size: wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("metadata pixels readback"), size: 64 * 64 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
    });
    let mut renderer = Renderer::new(&device, format, (64, 64));
    let mut world = World::new(17, None);
    let mut wanted = Vec::new();
    for w in -1..=1 { for z in -1..=1 { for x in -1..=1 {
        let c = IVec4::new(x, 0, z, w);
        wanted.push(c); world.chunks.insert(c, Chunk::uniform(block::AIR));
    }}}
    let center = IVec4::ZERO;
    world.chunks.insert(center, Chunk::uniform(block::WOOL));
    world.revision += 1;
    let eye = Vec4::new(8.0, 8.0, -4.0, 8.0);
    let basis = Orientation::default().basis();
    let views = [ViewDraw { basis, rect: [0.0, 0.0, 64.0, 64.0] }];
    let params = FrameParams { particles:&[], first_person: None, break_progress:0.0, eye, basis, fov_y_deg: 70.0, max_dist: 32.0, target: None, entities: &[], flags: 0,
        ghost_offset: 1.0, glasses_range: 8.0, time: 0.0, atmosphere: [1.0, 0.0, 0.0, 0.0],
        sun: Vec4::new(0.0, 1.0, -1.0, 0.0).normalize(), views: &views };
    let render = |renderer: &mut Renderer, world: &mut World, wanted: &[IVec4]| {
        settle_lighting(world, wanted);
        renderer.sync_world(&device, &queue, world, eye, wanted);
        assert_eq!(renderer.store.missing_dense_count(world, wanted), 0);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&queue, &mut encoder, &view, &params);
        encoder.copy_texture_to_buffer(target.as_image_copy(), wgpu::TexelCopyBufferInfo {
            buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(64) },
        }, target.size());
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixels = readback.slice(..).get_mapped_range().unwrap().to_vec();
        readback.unmap(); pixels
    };
    let white = render(&mut renderer, &mut world, &wanted);
    assert_eq!(renderer.store.metadata_bytes(), crate::metadata::HEADER_BYTES);
    let state_chunk = |variant| {
        let mut metadata = vec![0; CHUNK_VOLUME];
        for y in 0..16 { for x in 0..16 { metadata[local_index(IVec4::new(x, y, 0, 8))] = variant; }}
        Chunk::from_planes(vec![block::WOOL; CHUNK_VOLUME], Some(metadata)).unwrap()
    };
    world.chunks.insert(center, state_chunk(14)); world.revision += 1;
    let red = render(&mut renderer, &mut world, &wanted);
    assert!(red.chunks_exact(4).filter(|p| p[0] as u16 > p[1] as u16 + 20).count() > 3000, "metadata14 must select red wool");
    assert_ne!(red, white);
    assert!(renderer.store.metadata_bytes() <= crate::metadata::HEADER_BYTES + 16 * CHUNK_VOLUME as u64);
    // Both reloads have version0: identity, not just edit version, must invalidate
    // the cached GPU metadata when the saved chunk is reloaded with new contents.
    world.chunks.insert(center, state_chunk(11)); world.revision += 1;
    let blue = render(&mut renderer, &mut world, &wanted);
    assert!(blue.chunks_exact(4).filter(|p| p[2] as u16 > p[0] as u16 + 20).count() > 3000, "reloaded metadata11 must select blue wool");
    for y in 0..16 { for x in 0..16 { world.set_block_state(IVec4::new(x, y, 0, 8), block::BlockState::new(block::WOOL, 5)); }}
    assert_eq!(world.chunks[&center].data_version, 0, "metadata-only changes do not mutate the ID plane");
    let green = render(&mut renderer, &mut world, &wanted);
    assert_ne!(green, blue);
    let old_bytes = renderer.store.metadata_bytes();
    for &c in &wanted {
        if c != center { world.set_block_state(c * CHUNK, block::BlockState::new(block::AIR, 1)); }
    }
    let grown = render(&mut renderer, &mut world, &wanted);
    assert!(renderer.store.metadata_bytes() > old_bytes, "more than16 stateful chunks must grow the sparse pool");
    assert!(renderer.store.metadata_bytes() <= crate::metadata::HEADER_BYTES + 32 * CHUNK_VOLUME as u64);
    assert_eq!(grown, green, "growth/rebinding must preserve old chunk metadata and atlas resources");
    for y in 0..16 { for x in 0..16 { world.set_block(IVec4::new(x, y, 0, 8), block::WOOL); }}
    assert!(world.chunks[&center].metadata_plane().is_none());
    assert_eq!(render(&mut renderer, &mut world, &wanted), white, "clearing the last metadata cell restores default-state rendering");
    // Reloaded dense ID planes likewise must not reuse a stale version0 slot.
    let dense = |id| {
        let mut ids = vec![block::AIR; CHUNK_VOLUME];
        for y in 0..16 { for x in 0..16 { ids[local_index(IVec4::new(x, y, 0, 8))] = id; }}
        Chunk::from_dense(ids)
    };
    world.chunks.insert(center, dense(block::BRICK)); world.revision += 1;
    let brick = render(&mut renderer, &mut world, &[center]);
    world.chunks.insert(center, dense(block::GLOW)); world.revision += 1;
    let glow = render(&mut renderer, &mut world, &[center]);
    assert_ne!(brick, glow, "fresh dense chunk identity must invalidate cached IDs even with equal versions");
    let mut replacement = World::new(17, None);
    for &c in world.chunks.keys() { replacement.insert_chunk(c, Chunk::uniform(block::AIR)); }
    replacement.insert_chunk(center, dense(block::BRICK));
    settle_lighting(&mut replacement, &[center]);
    replacement.revision = world.revision;
    replacement.lighting_revision = world.lighting_revision;
    assert_eq!(render(&mut renderer, &mut replacement, &[center]), brick,
        "world switches with equal revisions and camera neighborhoods must not reuse another world's GPU data");
}

#[test]
#[ignore = "requires a GPU; run explicitly with repository-local driver caches"]
fn macrocell_skips_match_reference_pixels_in_rotated_transparent_scenes() {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .unwrap();
    let size = (64, 64);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("differential reference"),
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
        label: Some("differential readback"),
        size: 64 * 64 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut renderer = Renderer::new(&device, format, size);
    let mut world = World::new(11, None);
    let mut wanted = Vec::new();
    for w in -1..=1 {
        for z in -1..=1 {
            for x in -1..=1 {
                let c = IVec4::new(x, 0, z, w);
                world.chunks.insert(c, Chunk::uniform(block::AIR));
                wanted.push(c);
            }
        }
    }
    // Sparse opaque walls cross macrocell boundaries; transparent cells must
    // still contribute tint, and shallow lava must retain its fractional top.
    for w in -10..=10 {
        for z in -10..=10 {
            for x in -10..=10 {
                world.set_block(IVec4::new(x, 2, z, w), block::STONE);
                if (x + z + w).rem_euclid(7) == 0 {
                    world.set_block(IVec4::new(x, 5, z, w), block::GLASS);
                    world.set_block(IVec4::new(x, 4, z, w), block::WATER);
                }
                if x == 8 || w == -7 {
                    for y in 3..9 {
                        world.set_block(IVec4::new(x, y, z, w), block::BRICK);
                    }
                }
            }
        }
    }
    for x in -4..=4 {
        for w in -4..=4 {
            world.set_block(IVec4::new(x, 3, 5, w), block::FluidKind::Lava.flowing(5));
            world.set_block(IVec4::new(x, 4, -5, w), block::FluidKind::Water.flowing(3));
        }
    }
    let eye = Vec4::new(0.35, 7.3, 0.65, 0.15);
    settle_lighting(&mut world, &wanted);
    renderer.sync_world(&device, &queue, &world, eye, &wanted);
    assert_eq!(renderer.store.missing_dense_count(&world, &wanted), 0);
    for (scene, (yaw, ana, right_ana)) in [(0.0, 0.0, 0.0), (0.71, 0.43, -0.27), (-0.3, 1.1, 0.6)]
        .into_iter()
        .enumerate()
    {
        if scene == 1 {
            // Exercise resident-slot edits, including newly occupied regions;
            // a stale occupancy upload would hide these from accelerated rays.
            for w in -2..=2 {
                for x in -3..=3 {
                    world.set_block(IVec4::new(x, 5, -4, w), block::GLOW);
                    world.set_block(IVec4::new(x, 6, -4, w), block::GLOW);
                }
            }
            settle_lighting(&mut world, &wanted);
            renderer.store.uploads_per_frame = 1;
            renderer.sync_world(&device, &queue, &world, eye, &wanted);
            assert!(
                renderer.store.residency_counts().1 > 0,
                "dirty chunks must obey the same upload budget as newly resident chunks"
            );
            assert_eq!(
                renderer.store.residency_counts().1,
                renderer.store.missing_dense_count(&world, &wanted)
            );
            for _ in 0..8 {
                renderer.sync_world(&device, &queue, &world, eye, &wanted);
            }
            assert_eq!(renderer.store.missing_dense_count(&world, &wanted), 0);
            assert_eq!(renderer.store.residency_counts(), (0, 0));
            renderer.store.uploads_per_frame = 64;
            renderer.sync_world(&device, &queue, &world, eye, &wanted);
            assert_eq!(renderer.missing_lighting(), 0);
        }
        let mut orientation = Orientation::default();
        orientation.yaw(yaw);
        orientation.rotate_forward_ana(ana);
        orientation.rotate_right_ana(right_ana);
        orientation.add_pitch(-0.35);
        let basis = orientation.basis();
        let views = [ViewDraw {
            basis,
            rect: [0.0, 0.0, 64.0, 64.0],
        }];
        let mut params = FrameParams { particles:&[], first_person: None, break_progress:0.0,
            eye,
            basis,
            fov_y_deg: 75.0,
            max_dist: 32.0,
            target: None,
            entities: &[],
            flags: flags::GHOSTS | flags::SHADOWS | flags::EDGES,
            ghost_offset: 1.0,
            glasses_range: 8.0,
            time: 0.0,
            atmosphere: [1.0, 0.0, 0.0, 0.0],
            sun: Vec4::new(0.4, 0.8, 0.3, 0.1).normalize(),
            views: &views,
        };
        let mut outputs = Vec::new();
        for reference in [false, true] {
            if reference {
                params.flags |= flags::REFERENCE_VOXEL_STEPS;
            }
            let mut encoder = device.create_command_encoder(&Default::default());
            renderer.render(&queue, &mut encoder, &view, &params);
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(64),
                    },
                },
                target.size(),
            );
            queue.submit([encoder.finish()]);
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            outputs.push(readback.slice(..).get_mapped_range().unwrap().to_vec());
            readback.unmap();
        }
        // Crossing an exact texel boundary after a longer DDA step can change a
        // handful of nearest-sampled texels due to f32 rounding. Large regions
        // must not differ: missing surfaces/tints fail this comparison.
        let different = outputs[0]
            .chunks_exact(4)
            .zip(outputs[1].chunks_exact(4))
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(&a, &b)| a.abs_diff(b) > 3))
            .count();
        assert!(
            different <= 20,
            "{different}/4096 pixels differ for yaw={yaw}, ana={ana}, right_ana={right_ana}"
        );
    }
}
