//! Actual native vehicle meshes/skins, viewed in XZ and XW slices.
use super::*;
use mc4d_core::{block,camera::Basis,chunk::Chunk};
#[test]
#[ignore="requires GPU; captures native boat/cart/cargo silhouettes in XZ and XW"]
fn native_vehicles_and_cargo_render_in_primary_and_w_slices(){
 let instance=wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());let adapter=pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions{power_preference:wgpu::PowerPreference::HighPerformance,compatible_surface:None,force_fallback_adapter:false,apply_limit_buckets:false})).unwrap();
 let(device,queue)=pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor{required_limits:adapter.limits(),..Default::default()})).unwrap();
 let size=(1024,512);let format=wgpu::TextureFormat::Rgba8UnormSrgb;let mut renderer=Renderer::new(&device,format,size);renderer.store.uploads_per_frame=64;
 let mut world=World::new(7,None);let mut wanted=Vec::new();for x in -1..=0{for z in -1..=0{for w in -1..=0{let c=IVec4::new(x,0,z,w);world.chunks.insert(c,Chunk::uniform(block::AIR));wanted.push(c);}}}
 for x in -8..=8{for z in -4..=4{for w in -4..=4{world.set_block(IVec4::new(x,0,z,w),block::PLANKS);}}}
 crate::gpu_tests::settle_lighting(&mut world,&wanted);
 let target=device.create_texture(&wgpu::TextureDescriptor{label:Some("native vehicles"),size:wgpu::Extent3d{width:size.0,height:size.1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]});let view=target.create_view(&Default::default());let readback=device.create_buffer(&wgpu::BufferDescriptor{label:None,size:(size.0*size.1*4)as u64,usage:wgpu::BufferUsages::COPY_DST|wgpu::BufferUsages::MAP_READ,mapped_at_creation:false});
 let entities:Vec<_>=(0..4).map(|i|EntityDraw{center:Vec4::new((i as f32-1.5)*2.5,1.0,0.0,0.0),radius:1.0,color:[1.0;3],kind:if i==0{18}else{19},forward:Vec4::Z,item:if i==2{block::CHEST.into()}else if i==3{block::FURNACE.into()}else{0},item_variant:0,state:if i>=2{(i-1)<<8}else{0}}).collect();
 let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");std::fs::create_dir_all(&root).unwrap();
 for (name,eye,fh,ana) in [("xz",Vec4::new(0.0,3.0,9.0,0.0),-Vec4::Z,Vec4::W),("xw",Vec4::new(0.0,3.0,0.0,9.0),-Vec4::W,Vec4::Z)]{
  let pitch=-0.15_f32;let basis=Basis{right:Vec4::X,forward_h:fh,ana,forward:fh*pitch.cos()+Vec4::Y*pitch.sin(),up:-fh*pitch.sin()+Vec4::Y*pitch.cos()};let views=[ViewDraw{basis,rect:[0.0,0.0,size.0 as f32,size.1 as f32]}];renderer.sync_world(&device,&queue,&world,eye,&wanted);
  let mut outputs=Vec::new();for show in [false,true]{let params=FrameParams{ particles:&[],first_person:None,eye,basis,fov_y_deg:65.0,max_dist:24.0,target:None,entities:if show{&entities}else{&[]},flags:0,ghost_offset:1.0,glasses_range:8.0,time:0.0,break_progress:0.0,atmosphere:[1.0,0.0,0.0,0.0],sun:Vec4::Y,views:&views};let mut encoder=device.create_command_encoder(&Default::default());renderer.render(&queue,&mut encoder,&view,&params);encoder.copy_texture_to_buffer(target.as_image_copy(),wgpu::TexelCopyBufferInfo{buffer:&readback,layout:wgpu::TexelCopyBufferLayout{offset:0,bytes_per_row:Some(size.0*4),rows_per_image:Some(size.1)}},target.size());queue.submit([encoder.finish()]);readback.slice(..).map_async(wgpu::MapMode::Read,|r|r.unwrap());device.poll(wgpu::PollType::wait_indefinitely()).unwrap();let mapped=readback.slice(..).get_mapped_range().unwrap();outputs.push(mapped.to_vec());drop(mapped);readback.unmap();}
  for (index,range) in [(0,200..400),(1,370..515),(2,510..660),(3,650..830)]{let changed=(170..370).flat_map(|y|range.clone().map(move|x|(y*size.0 as usize+x)*4)).filter(|&p|outputs[0][p..p+3]!=outputs[1][p..p+3]).count();assert!(changed>120,"vehicle{index} missing from {name}: {changed}");}
  let file=std::fs::File::create(root.join(format!("vehicles-native-{name}.png"))).unwrap();let mut png=png::Encoder::new(file,size.0,size.1);png.set_color(png::ColorType::Rgba);png.set_depth(png::BitDepth::Eight);png.write_header().unwrap().write_image_data(&outputs[1]).unwrap();
 }
}
