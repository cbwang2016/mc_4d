//! Opt-in visual fixture for the classic atlas and multipart 4D mob skins.
use super::*;
use mc4d_core::{block, camera::Orientation, chunk::Chunk};

#[test]
#[ignore = "requires GPU; writes the review image under repository target/render-review"]
fn classic_mob_parts_have_visible_textured_silhouettes() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: None,
        force_fallback_adapter: false, apply_limit_buckets: false,
    })).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(), ..Default::default()
    })).unwrap();
    let size = (1280, 800);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(&device, format, size);
    let mut world = World::new(7, None);
    let mut wanted = Vec::new();
    for x in -1..=0 { for z in -1..=0 { for w in -1..=0 {
        let c = IVec4::new(x, 0, z, w);
        world.chunks.insert(c, Chunk::uniform(block::AIR)); wanted.push(c);
    }}}
    for x in -12..=12 { for z in -12..=8 { for w in -2..=2 {
        world.set_block(IVec4::new(x, 0, z, w), block::PLANKS);
    }}}
    let eye = Vec4::new(0.0, 4.0, 11.0, 0.0);
    renderer.store.uploads_per_frame = 64;
    crate::gpu_tests::settle_lighting(&mut world, &wanted);
    renderer.sync_world(&device, &queue, &world, eye, &wanted);
    let mut orient = Orientation::default(); orient.yaw(std::f32::consts::PI); orient.add_pitch(-0.16);
    let basis = orient.basis();
    let views = [ViewDraw { basis, rect: [0.0, 0.0, size.0 as f32, size.1 as f32] }];
    let mut entities = Vec::new();
    for species in mc4d_core::entities::CreatureKind::ALL.into_iter().skip(1) {
        let kind = species as u32;
        let (x, z) = if kind <= 6 { ((kind as f32 - 3.5) * 2.1, 1.0) }
            else { ((kind as f32 - 9.0) * 2.1, -3.5) };
        entities.push(EntityDraw { center: Vec4::new(x, 1.0, z, 0.0), radius: species.render_scale(2),
            color: [1.0; 3], kind, forward: Vec4::Z, item: 0, item_variant: 0, state: 0 });
    }
    entities.push(EntityDraw { center: Vec4::new(6.5, 2.0, -3.5, 0.0), radius: 0.7,
        color: [0.6, 0.2, 0.9], kind: 0, forward: Vec4::Z, item: 0, item_variant: 0, state: 0 });
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("classic mob review"), size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (size.0 * size.1 * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut params = FrameParams { particles:&[], first_person: None, break_progress:0.0, eye, basis, fov_y_deg: 65.0, max_dist: 32.0, target: None,
        entities: &[], flags: 0, ghost_offset: 1.0, glasses_range: 8.0,
        time: 0.0, atmosphere: [1.0, 0.0, 0.0, 0.0], sun: Vec4::new(0.4, 0.8, 0.3, 0.1).normalize(), views: &views };
    let mut dyed=entities.clone();
    dyed.iter_mut().filter(|e|e.kind==3).for_each(|e|e.state=14<<8);
    let mut sheared=entities.clone();
    sheared.iter_mut().filter(|e|e.kind==3).for_each(|e|e.state=1);
    let mut sheared_dyed=sheared.clone();
    sheared_dyed.iter_mut().filter(|e|e.kind==3).for_each(|e|e.state|=14<<8);
    let mut outputs = Vec::new();
    for scene in [&[][..],&entities[..],&dyed[..],&sheared[..],&sheared_dyed[..]] {
        params.entities=scene;
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&queue, &mut encoder, &view, &params);
        encoder.copy_texture_to_buffer(target.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.0 * 4), rows_per_image: Some(size.1) } }, target.size());
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        outputs.push(mapped.to_vec()); drop(mapped); readback.unmap();
    }
    let changed = outputs[0].chunks_exact(4).zip(outputs[1].chunks_exact(4)).filter(|(a, b)| a != b).count();
    assert!(changed > 5000, "multipart creatures must contribute visible silhouettes, changed={changed}");
    let wool_changes=outputs[1].chunks_exact(4).zip(outputs[2].chunks_exact(4)).filter(|(a,b)|a!=b).count();
    assert!(wool_changes>100,"dyed wool must visibly use the original sheep palette");
    assert_eq!(outputs[3],outputs[4],"shearing must expose untinted skin regardless of saved wool color");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
    std::fs::create_dir_all(&root).unwrap();
    let file = std::fs::File::create(root.join("mobs-classic.png")).unwrap();
    let mut encoder = png::Encoder::new(file, size.0, size.1);
    encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&outputs[1]).unwrap();
    for (name,pixels) in [("mobs-dyed-sheep.png",&outputs[2]),("mobs-sheared-sheep.png",&outputs[3])] {
        let file=std::fs::File::create(root.join(name)).unwrap();let mut encoder=png::Encoder::new(file,size.0,size.1);
        encoder.set_color(png::ColorType::Rgba);encoder.set_depth(png::BitDepth::Eight);encoder.write_header().unwrap().write_image_data(pixels).unwrap();
    }
}

#[test]
#[ignore = "requires GPU; writes axis/near-axis texture review images under repository target/render-review"]
fn projected_classic_textures_stay_two_dimensional_at_slice_axes() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: None,
        force_fallback_adapter: false, apply_limit_buckets: false,
    })).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(), ..Default::default()
    })).unwrap();
    let size = (256, 256);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(&device, format, size);
    let mut world = World::new(7, None);
    let mut wanted = Vec::new();
    for x in -1..=0 { for z in -1..=0 { for w in -1..=0 {
        let c = IVec4::new(x, 0, z, w);
        world.chunks.insert(c, Chunk::uniform(block::AIR)); wanted.push(c);
    }}}
    for x in -15..=15 { for z in -15..=15 { for w in -15..=15 {
        world.set_block(IVec4::new(x, 0, z, w), block::GRASS);
    }}}
    let eye = Vec4::new(0.35, 4.0, 0.35, 0.35);
    renderer.store.uploads_per_frame = 64;
    crate::gpu_tests::settle_lighting(&mut world, &wanted);
    renderer.sync_world(&device, &queue, &world, eye, &wanted);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("projection review"), size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (size.0 * size.1 * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
    std::fs::create_dir_all(&root).unwrap();
    for (name, ana, right_ana) in [("neg0_1", -0.1_f32, 0.0_f32), ("0", 0.0, 0.0), ("0_1", 0.1, 0.0),
        ("45", 45.0, 0.0), ("89_9", 89.9, 0.0), ("90", 90.0, 0.0), ("normal-x", 0.0, 90.0)] {
        let mut orient = Orientation::default();
        orient.rotate_forward_ana(ana.to_radians()); orient.rotate_right_ana(right_ana.to_radians()); orient.add_pitch(-0.55);
        let basis = orient.basis();
        let views = [ViewDraw { basis, rect: [0.0, 0.0, size.0 as f32, size.1 as f32] }];
        let params = FrameParams { particles:&[], first_person: None, break_progress:0.0, eye, basis, fov_y_deg: 65.0, max_dist: 24.0, target: None, entities: &[], flags: 0,
            ghost_offset: 1.0, glasses_range: 8.0, time: 0.0, atmosphere: [1.0, 0.0, 0.0, 0.0],
            sun: Vec4::Y, views: &views };
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(&queue, &mut encoder, &view, &params);
        encoder.copy_texture_to_buffer(target.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.0 * 4), rows_per_image: Some(size.1) } }, target.size());
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        let rgba = mapped.to_vec(); drop(mapped); readback.unmap();
        // A degenerate projection repeats one texel across the entire row.
        let row: Vec<_> = (32..224).map(|x| rgba[(200 * 256 + x) * 4 + 1]).collect();
        let spread = row.iter().max().unwrap() - row.iter().min().unwrap();
        assert!(spread > 10, "texture collapsed into stripes at {name}: green-channel spread={spread}");
        let file = std::fs::File::create(root.join(format!("uv-ana-{name}.png"))).unwrap();
        let mut png = png::Encoder::new(file, size.0, size.1);
        png.set_color(png::ColorType::Rgba); png.set_depth(png::BitDepth::Eight);
        png.write_header().unwrap().write_image_data(&rgba).unwrap();
    }
}

#[test]
#[ignore = "requires GPU; writes animated water review images under repository target/render-review"]
fn classic_water_has_visible_animated_top_and_side_surfaces() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: None,
        force_fallback_adapter: false, apply_limit_buckets: false,
    })).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(), ..Default::default()
    })).unwrap();
    let size = (768, 512);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(&device, format, size);
    let mut world = World::new(7, None); let mut wanted = Vec::new();
    for x in -1..=0 { for z in -1..=0 { for w in -1..=0 {
        let c = IVec4::new(x, 0, z, w); world.chunks.insert(c, Chunk::uniform(block::AIR)); wanted.push(c);
    }}}
    for x in -12..=12 { for z in -12..=12 { for w in -6..=6 {
        world.set_block(IVec4::new(x, 0, z, w), block::SAND);
        if (-4..=2).contains(&x) && (-3..=3).contains(&z) && (-4..=4).contains(&w) {
            world.set_block(IVec4::new(x, 1, z, w), block::WATER);
        }
    }}}
    for w in -4..=4 { for z in -3..=3 {
        world.set_block(IVec4::new(3, 1, z, w), block::FluidKind::Water.flowing(3));
        world.set_block(IVec4::new(4, 1, z, w), block::FluidKind::Water.flowing(6));
    }}
    for y in 2..=4 { for w in -1..=1 { world.set_block(IVec4::new(-3, y, -1, w), block::FluidKind::Water.flowing(8)); }}
    let eye = Vec4::new(7.0, 5.0, 9.0, 0.2);
    renderer.store.uploads_per_frame = 64; crate::gpu_tests::settle_lighting(&mut world, &wanted);
    renderer.sync_world(&device, &queue, &world, eye, &wanted);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("water review"), size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (size.0 * size.1 * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
    std::fs::create_dir_all(&root).unwrap(); let mut outputs = Vec::new();
    for (name, time, underwater) in [("water-classic", 0.0, false), ("water-classic-next", 0.2, false), ("water-underwater", 0.2, true)] {
        let eye = if underwater { Vec4::new(0.2, 1.4, 0.2, 0.2) } else { eye };
        let mut orient = Orientation::default(); orient.yaw(std::f32::consts::PI + 0.6); orient.add_pitch(if underwater { 0.2 } else { -0.32 });
        let basis = orient.basis(); let views = [ViewDraw { basis, rect: [0.0, 0.0, size.0 as f32, size.1 as f32] }];
        let params = FrameParams { particles:&[], first_person: None, break_progress:0.0, eye, basis, fov_y_deg: 65.0, max_dist: 32.0, target: None, entities: &[], flags: 0,
            ghost_offset: 1.0, glasses_range: 8.0, time, atmosphere: [1.0, 0.0, 0.0, 0.0], sun: Vec4::Y, views: &views };
        let mut encoder = device.create_command_encoder(&Default::default()); renderer.render(&queue, &mut encoder, &view, &params);
        encoder.copy_texture_to_buffer(target.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.0 * 4), rows_per_image: Some(size.1) } }, target.size());
        queue.submit([encoder.finish()]); readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap(); let mapped = readback.slice(..).get_mapped_range().unwrap();
        let rgba = mapped.to_vec(); drop(mapped); readback.unmap();
        let file = std::fs::File::create(root.join(format!("{name}.png"))).unwrap(); let mut png = png::Encoder::new(file, size.0, size.1);
        png.set_color(png::ColorType::Rgba); png.set_depth(png::BitDepth::Eight);
        png.write_header().unwrap().write_image_data(&rgba).unwrap(); outputs.push(rgba);
    }
    let changed = outputs[0].chunks_exact(4).zip(outputs[1].chunks_exact(4)).filter(|(a, b)| a != b).count();
    assert!(changed > 100, "water skin must animate on screen, changed={changed}");
    let blue = outputs[0].chunks_exact(4).filter(|p| p[2] > p[0].saturating_add(50) && p[2] > p[1].saturating_add(30)).count();
    assert!(blue > 1000, "water surfaces must be visibly blue rather than an invisible medium");
}

#[test]
#[ignore = "requires GPU; verifies uniform slabs and opened X/W door gaps are truly transparent"]
fn uniform_partial_blocks_do_not_turn_into_solid_chunks() {
    use mc4d_core::chunk::CHUNK_VOLUME;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: None,
        force_fallback_adapter: false, apply_limit_buckets: false,
    })).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(), ..Default::default()
    })).unwrap();
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = Renderer::new(&device, format, (64, 64));
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None, size: wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 64 * 64 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut world = World::new(7, None); let wanted = [IVec4::ZERO];
    let mut render = |id, metadata, eye, basis| {
        world.chunks.insert(IVec4::ZERO, Chunk::from_planes(vec![id; CHUNK_VOLUME], Some(vec![metadata; CHUNK_VOLUME])).unwrap());
        world.revision += 1;
        crate::gpu_tests::settle_lighting(&mut world, &wanted);
    renderer.sync_world(&device, &queue, &world, eye, &wanted);
        let views = [ViewDraw { basis, rect: [0.0, 0.0, 64.0, 64.0] }];
        let params = FrameParams { particles:&[], first_person: None, break_progress:0.0, eye, basis, fov_y_deg: 2.0, max_dist: 8.0, target: None, entities: &[], flags: 0,
            ghost_offset: 1.0, glasses_range: 8.0, time: 0.0, atmosphere: [1.0, 0.0, 0.0, 0.0], sun: Vec4::Y, views: &views };
        let mut encoder = device.create_command_encoder(&Default::default()); renderer.render(&queue, &mut encoder, &view, &params);
        encoder.copy_texture_to_buffer(target.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(64) } }, target.size());
        queue.submit([encoder.finish()]); readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let pixels = readback.slice(..).get_mapped_range().unwrap().to_vec(); readback.unmap(); pixels
    };
    let eye = Vec4::new(8.5, 8.75, 2.5, 8.5);
    let basis = Orientation::default().basis();
    let sky = render(block::AIR, 0, eye, basis);
    assert_eq!(render(block::SLAB, 0, eye, basis), sky, "the air half of uniform slab chunks must remain visible");
    assert_ne!(render(block::DOUBLE_SLAB, 0, eye, basis), sky, "double slabs remain full cubes");
    let mut orient = Orientation::default(); orient.yaw(std::f32::consts::FRAC_PI_2);
    let eye = Vec4::new(2.5, 8.5, 8.5, 8.5); let basis = orient.basis();
    let sky = render(block::AIR, 0, eye, basis);
    assert_ne!(render(block::WOOD_DOOR, 0, eye, basis), sky);
    assert_eq!(render(block::WOOD_DOOR, 4, eye, basis), sky, "open doors rotate out of the X corridor");
    let mut orient = Orientation::default(); orient.rotate_forward_ana(std::f32::consts::FRAC_PI_2);
    let eye = Vec4::new(8.5, 8.5, 8.5, 2.5); let basis = orient.basis();
    let sky = render(block::AIR, 0, eye, basis);
    assert_ne!(render(block::WOOD_DOOR, 16, eye, basis), sky);
    assert_eq!(render(block::WOOD_DOOR, 20, eye, basis), sky, "W-facing open doors must leave the W corridor clear");
}

#[test]
#[ignore = "requires GPU; validates sign text on physical front faces in Z and W"]
fn signs_use_cached_text_only_on_front_faces() {
    let instance=wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter=pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference:wgpu::PowerPreference::HighPerformance,compatible_surface:None,
        force_fallback_adapter:false,apply_limit_buckets:false,
    })).unwrap();
    let (device,queue)=pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {required_limits:adapter.limits(),..Default::default()})).unwrap();
    let size=(256,256);let format=wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer=Renderer::new(&device,format,size);
    let mut world=World::new(7,None);let mut wanted=Vec::new();
    for x in -1..=0 {for z in -1..=0 {for w in -1..=0 {
        let c=IVec4::new(x,0,z,w);world.chunks.insert(c,Chunk::uniform(block::AIR));wanted.push(c);
    }}}
    crate::gpu_tests::settle_lighting(&mut world,&wanted);
    renderer.store.uploads_per_frame=64;
    renderer.sync_world(&device,&queue,&world,Vec4::splat(0.5),&wanted);
    // High-contrast cache content makes a missed or incorrectly mapped front
    // face distinguishable from the native wooden back and post.
    let mut pixels=vec![255u8;(SIGN_TEXTURE_WIDTH*SIGN_TEXTURE_HEIGHT*4) as usize];
    for y in 0..SIGN_TEXTURE_HEIGHT {for x in 0..SIGN_TEXTURE_WIDTH {
        let i=((y*SIGN_TEXTURE_WIDTH+x)*4) as usize;
        pixels[i..i+3].copy_from_slice(if x>30 && x<160 && y>25 && y<70 {&[0,0,0]}else{&[255,255,255]});
    }}
    renderer.upload_sign_texture(&queue,0,&pixels);
    let target=device.create_texture(&wgpu::TextureDescriptor {label:Some("sign front test"),
        size:wgpu::Extent3d {width:256,height:256,depth_or_array_layers:1},mip_level_count:1,sample_count:1,
        dimension:wgpu::TextureDimension::D2,format,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
    let view=target.create_view(&Default::default());
    let readback=device.create_buffer(&wgpu::BufferDescriptor {label:None,size:256*256*4,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    for forward in [Vec4::Z,Vec4::W] {for front in [true,false] {
        let direction=if front {forward}else{-forward};
        let center=Vec4::new(0.5,1.5,0.5,0.5);
        let eye=center+direction*3.0+Vec4::Y*0.3;
        let basis=mc4d_core::camera::Basis {right:-Vec4::X,up:Vec4::Y,forward:-direction,forward_h:-direction,ana:if forward==Vec4::Z {Vec4::W}else{Vec4::Z}};
        let views=[ViewDraw {basis,rect:[0.0,0.0,256.0,256.0]}];
        let mut entity=EntityDraw {center,radius:0.5,color:[1.0;3],kind:15,forward,item:0,item_variant:0,state:0};
        let mut outputs=Vec::new();
        for cached in [false,true] {
            entity.state=if cached {1<<8}else{0};let entities=[entity];
            let params=FrameParams { particles:&[], first_person: None,break_progress:0.0,eye,basis,fov_y_deg:60.0,max_dist:12.0,target:None,entities:&entities,flags:0,ghost_offset:1.0,glasses_range:8.0,time:0.0,atmosphere:[1.0,0.0,0.0,0.0],sun:Vec4::Y,views:&views};
            let mut encoder=device.create_command_encoder(&Default::default());renderer.render(&queue,&mut encoder,&view,&params);
            encoder.copy_texture_to_buffer(target.as_image_copy(),wgpu::TexelCopyBufferInfo {buffer:&readback,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(1024),rows_per_image:Some(256)}},target.size());
            queue.submit([encoder.finish()]);readback.slice(..).map_async(wgpu::MapMode::Read,|r|r.unwrap());device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let mapped=readback.slice(..).get_mapped_range().unwrap();outputs.push(mapped.to_vec());drop(mapped);readback.unmap();
        }
        let changed=outputs[0].chunks_exact(4).zip(outputs[1].chunks_exact(4)).filter(|(a,b)|a!=b).count();
        if front {assert!(changed>1000,"sign front cache missing for {forward:?}: {changed}");}
        else {assert_eq!(changed,0,"text leaked onto back of sign {forward:?}");}
    }}
}
