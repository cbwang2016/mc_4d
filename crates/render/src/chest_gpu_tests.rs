//! Native double-chest seam/front tests, including the additional W adjacency.
use super::*;
use mc4d_core::{block,chunk::Chunk,camera::Basis};
#[test]
#[ignore="requires GPU; verifies native double-chest latch halves across X/Z/W and wall-selected fronts"]
fn double_chest_latches_join_at_the_shared_seam(){
    let instance=wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter=pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {power_preference:wgpu::PowerPreference::HighPerformance,compatible_surface:None,force_fallback_adapter:false,apply_limit_buckets:false})).unwrap();
    let (device,queue)=pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {required_limits:adapter.limits(),..Default::default()})).unwrap();
    let size=(384,256);let format=wgpu::TextureFormat::Rgba8UnormSrgb;let mut renderer=Renderer::new(&device,format,size);
    renderer.store.uploads_per_frame=64;
    let target=device.create_texture(&wgpu::TextureDescriptor {label:Some("double chest review"),size:wgpu::Extent3d {width:size.0,height:size.1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});
    let view=target.create_view(&Default::default());
    let readback=device.create_buffer(&wgpu::BufferDescriptor {label:None,size:(size.0*size.1*4) as u64,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
    for (name,axis,flip) in [("x",0,false),("z",2,false),("w",3,false),("x-wall-flip",0,true)] {
        let mut world=World::new(7,None);let mut wanted=Vec::new();
        for x in -1..=0 {for z in -1..=0 {for w in -1..=0 {let c=IVec4::new(x,0,z,w);world.insert_chunk(c,Chunk::uniform(block::AIR));wanted.push(c);}}}
        let high=IVec4::new(0,1,0,0);let mut low=high;low[axis]-=1;
        world.set_block(low,block::CHEST);world.set_block(high,block::CHEST);
        if flip {world.set_block(low+IVec4::Z,block::STONE);world.set_block(high+IVec4::Z,block::STONE);}
        let (eye,right,forward,ana)=match axis {
            2=>(Vec4::new(4.0,1.5,0.0,0.5),Vec4::Z,-Vec4::X,Vec4::W),
            3=>(Vec4::new(0.5,1.5,4.0,0.0),Vec4::W,-Vec4::Z,Vec4::X),
            _ if flip=>(Vec4::new(0.0,1.5,-3.0,0.5),Vec4::X,Vec4::Z,Vec4::W),
            _=>(Vec4::new(0.0,1.5,4.0,0.5),-Vec4::X,-Vec4::Z,Vec4::W),
        };
        let basis=Basis {right,up:Vec4::Y,forward,ana,forward_h:forward};
        crate::gpu_tests::settle_lighting(&mut world,&wanted);renderer.sync_world(&device,&queue,&world,eye,&wanted);
        let views=[ViewDraw {basis,rect:[0.0,0.0,size.0 as f32,size.1 as f32]}];
        let params=FrameParams { particles:&[],first_person:None,eye,basis,fov_y_deg:35.0,max_dist:12.0,target:None,entities:&[],flags:0,ghost_offset:1.0,glasses_range:8.0,time:0.0,break_progress:0.0,atmosphere:[1.0,0.0,0.0,0.0],sun:Vec4::Y,views:&views};
        let mut encoder=device.create_command_encoder(&Default::default());renderer.render(&queue,&mut encoder,&view,&params);
        encoder.copy_texture_to_buffer(target.as_image_copy(),wgpu::TexelCopyBufferInfo {buffer:&readback,layout:wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(size.0*4),rows_per_image:Some(size.1)}},target.size());queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read,|r|r.unwrap());device.poll(wgpu::PollType::wait_indefinitely()).unwrap();let mapped=readback.slice(..).get_mapped_range().unwrap();let pixels=mapped.to_vec();drop(mapped);readback.unmap();
        let mut seam=0;let mut outside=0;
        // The fixture's fallback bedrock is gray too; inspect only the
        // projected chest face (Y64..192), excluding the ground below it.
        for (i,p) in pixels.chunks_exact(4).enumerate(){let y=i as u32/size.0;if !(64..192).contains(&y){continue;}let lo=*p[..3].iter().min().unwrap();let hi=*p[..3].iter().max().unwrap();
            if lo>45 && hi-lo<14 {let x=i as u32%size.0;if x.abs_diff(size.0/2)<=14 {seam+=1;}else{outside+=1;}}
        }
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");std::fs::create_dir_all(&root).unwrap();
        let file=std::fs::File::create(root.join(format!("chest-double-{name}.png"))).unwrap();let mut png=png::Encoder::new(file,size.0,size.1);png.set_color(png::ColorType::Rgba);png.set_depth(png::BitDepth::Eight);png.write_header().unwrap().write_image_data(&pixels).unwrap();
        assert!(seam>30,"{name}: latch missing from shared seam ({seam} pixels, {outside} elsewhere)");
        assert!(outside<seam/20+4,"{name}: latch halves placed at outside edges ({outside} pixels)");
    }
}
