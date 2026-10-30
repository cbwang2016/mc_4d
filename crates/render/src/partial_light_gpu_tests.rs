//! Regression for native cn.n: inset door faces use the door cell's brightness,
//! not the opaque block behind the observer in the neighboring whole voxel.
use super::*;
use mc4d_core::{block,chunk::Chunk};
#[test]
#[ignore="requires GPU; captures inset-door face lighting with and without an opaque neighboring cell"]
fn inset_door_face_keeps_its_own_skylight(){
 let instance=wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
 let adapter=pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions{power_preference:wgpu::PowerPreference::HighPerformance,compatible_surface:None,force_fallback_adapter:false,apply_limit_buckets:false})).unwrap();
 let(device,queue)=pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor{required_limits:adapter.limits(),..Default::default()})).unwrap();
 let size=256u32;let format=wgpu::TextureFormat::Rgba8UnormSrgb;
 let target=device.create_texture(&wgpu::TextureDescriptor{label:Some("inset face light proof"),size:wgpu::Extent3d{width:size,height:size,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});let view=target.create_view(&Default::default());
 let read=device.create_buffer(&wgpu::BufferDescriptor{label:None,size:(size*size*4)as u64,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
 let mut world=World::new(5,None);world.insert_chunk(IVec4::ZERO,Chunk::uniform(block::AIR));
 let door=IVec4::new(8,1,8,8);world.set_block(door-IVec4::Y,block::STONE);world.set_block_state(door,block::BlockState::new(block::IRON_DOOR,0));world.set_block_state(door+IVec4::Y,block::BlockState::new(block::IRON_DOOR,8));world.set_block(door+IVec4::X,block::STONE);world.set_block(door+IVec4::X+IVec4::Y,block::STONE);
 let eye=Vec4::new(8.6,1.35,8.5,8.5);let basis=Basis{right:Vec4::Z,up:Vec4::Y,forward:Vec4::NEG_X,forward_h:Vec4::NEG_X,ana:Vec4::W};let views=[ViewDraw{basis,rect:[0.0,0.0,size as f32,size as f32]}];
 let mut params=FrameParams{ particles:&[],first_person:None,eye,basis,fov_y_deg:25.0,max_dist:16.0,target:None,entities:&[],flags:0,ghost_offset:1.0,glasses_range:8.0,time:0.0,break_progress:0.0,atmosphere:[1.0,0.0,0.0,0.0],sun:Vec4::Y,views:&views};let mut renderer=Renderer::new(&device,format,(size,size));renderer.store.uploads_per_frame=64;
 let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");std::fs::create_dir_all(&root).unwrap();
 let draw=|renderer:&mut Renderer,world:&mut World,params:&FrameParams,name:&str|{
  crate::gpu_tests::settle_lighting(world,&[IVec4::ZERO]);assert_eq!(world.light_at(door).unwrap().1,15);renderer.sync_world(&device,&queue,world,eye,&[IVec4::ZERO]);assert_eq!(renderer.missing_lighting(),0);
  let mut encoder=device.create_command_encoder(&Default::default());renderer.render(&queue,&mut encoder,&view,params);encoder.copy_texture_to_buffer(target.as_image_copy(),wgpu::TexelCopyBufferInfo{buffer:&read,layout:wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(size*4),rows_per_image:Some(size)}},target.size());queue.submit([encoder.finish()]);read.slice(..).map_async(wgpu::MapMode::Read,|r|r.unwrap());device.poll(wgpu::PollType::wait_indefinitely()).unwrap();let map=read.slice(..).get_mapped_range().unwrap();let image=map.to_vec();drop(map);read.unmap();let mut png=png::Encoder::new(std::fs::File::create(root.join(name)).unwrap(),size,size);png.set_color(png::ColorType::Rgba);png.set_depth(png::BitDepth::Eight);png.write_header().unwrap().write_image_data(&image).unwrap();image
 };
 let wall=draw(&mut renderer,&mut world,&params,"lighting-inset-door-wall.png");world.set_block(door+IVec4::X,block::AIR);world.set_block(door+IVec4::X+IVec4::Y,block::AIR);let clear=draw(&mut renderer,&mut world,&params,"lighting-inset-door-clear.png");
 let mean=|p:&[u8]|p.chunks_exact(4).map(|v|v[0]as f64+v[1]as f64+v[2]as f64).sum::<f64>()/(size*size*3)as f64;
 assert!((mean(&wall)-mean(&clear)).abs()<1.0,"inset face incorrectly samples dark neighbor: wall={} clear={}",mean(&wall),mean(&clear));
 params.flags=flags::SMOOTH_LIGHTING;
 world.set_block(door+IVec4::X,block::STONE);world.set_block(door+IVec4::X+IVec4::Y,block::STONE);
 let smooth=draw(&mut renderer,&mut world,&params,"lighting-inset-door-smooth.png");
 assert!((mean(&smooth)-mean(&clear)).abs()<1.0,"native door rendering does not add full-voxel ambient occlusion");

}
