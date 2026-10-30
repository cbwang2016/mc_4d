//! Small raster billboards with exact world-distance occlusion. Work scales with
//! bounded visible particles, never with particles times every screen pixel.
use super::{FrameParams,ViewDraw,flags};
use bytemuck::{Pod,Zeroable};
use mc4d_core::glam::Vec4;
use wgpu::util::DeviceExt;
pub const MAX_PARTICLES:usize=1024;
#[derive(Clone,Copy,Debug)]pub struct ParticleDraw{pub position:Vec4,pub radius:f32,pub thickness:f32,pub uv:[f32;4],pub color:[f32;3],pub layer:u32,pub light:[u8;2]}
#[repr(C)]#[derive(Clone,Copy,Pod,Zeroable)]struct Vertex{clip:[f32;4],delta:[f32;4],uv:[f32;2],color:[f32;4],view:u32,eye:u32,layer:u32}
#[repr(C)]#[derive(Clone,Copy,Pod,Zeroable)]struct Uniforms{size:[f32;4],rects:[[f32;4];9],info:[u32;4]}
pub struct ParticlePass{vertices:wgpu::Buffer,uniform:wgpu::Buffer,layout:wgpu::BindGroupLayout,group:wgpu::BindGroup,pipelines:[wgpu::RenderPipeline;3],scratch:Vec<Vertex>}
impl ParticlePass{
 pub fn new(device:&wgpu::Device,format:wgpu::TextureFormat,atlas:&wgpu::TextureView,sampler:&wgpu::Sampler,depth:&wgpu::TextureView)->Self{
  let layout=device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("bounded particles"),entries:&[
   wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,has_dynamic_offset:false,min_binding_size:None},count:None},
   wgpu::BindGroupLayoutEntry{binding:1,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},view_dimension:wgpu::TextureViewDimension::D2Array,multisampled:false},count:None},
   wgpu::BindGroupLayoutEntry{binding:2,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),count:None},
   wgpu::BindGroupLayoutEntry{binding:3,visibility:wgpu::ShaderStages::FRAGMENT,ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:false},view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
  ]});
  let uniform=device.create_buffer_init(&wgpu::util::BufferInitDescriptor{label:Some("particle views"),contents:bytemuck::bytes_of(&Uniforms::zeroed()),usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST});
  let group=Self::group(device,&layout,&uniform,atlas,sampler,depth);
  let shader=device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("raster particles"),source:wgpu::ShaderSource::Wgsl(include_str!("../shaders/particles.wgsl").into())});
  let pl=device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some("particles"),bind_group_layouts:&[Some(&layout)],immediate_size:0});
  let pipelines=std::array::from_fn(|i|device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some("particles"),layout:Some(&pl),vertex:wgpu::VertexState{module:&shader,entry_point:Some("vs_main"),compilation_options:Default::default(),buffers:&[Some(wgpu::VertexBufferLayout{array_stride:std::mem::size_of::<Vertex>()as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x2,3=>Float32x4,4=>Uint32,5=>Uint32,6=>Uint32]})]},fragment:Some(wgpu::FragmentState{module:&shader,entry_point:Some("fs_main"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format,blend:Some(wgpu::BlendState::ALPHA_BLENDING),write_mask:match i{1=>wgpu::ColorWrites::RED,2=>wgpu::ColorWrites::GREEN|wgpu::ColorWrites::BLUE,_=>wgpu::ColorWrites::ALL}})]}),primitive:wgpu::PrimitiveState{cull_mode:None,..Default::default()},depth_stencil:None,multisample:Default::default(),multiview_mask:None,cache:None}));
  let vertices=device.create_buffer(&wgpu::BufferDescriptor{label:Some("bounded particle vertices"),size:(MAX_PARTICLES*9*6*2*std::mem::size_of::<Vertex>())as u64,usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
  Self{vertices,uniform,layout,group,pipelines,scratch:Vec::new()}
 }
 fn group(device:&wgpu::Device,layout:&wgpu::BindGroupLayout,uniform:&wgpu::Buffer,atlas:&wgpu::TextureView,sampler:&wgpu::Sampler,depth:&wgpu::TextureView)->wgpu::BindGroup{device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("particle atlas and scene distance"),layout,entries:&[wgpu::BindGroupEntry{binding:0,resource:uniform.as_entire_binding()},wgpu::BindGroupEntry{binding:1,resource:wgpu::BindingResource::TextureView(atlas)},wgpu::BindGroupEntry{binding:2,resource:wgpu::BindingResource::Sampler(sampler)},wgpu::BindGroupEntry{binding:3,resource:wgpu::BindingResource::TextureView(depth)}]})}
 pub fn rebind(&mut self,device:&wgpu::Device,atlas:&wgpu::TextureView,sampler:&wgpu::Sampler,depth:&wgpu::TextureView){self.group=Self::group(device,&self.layout,&self.uniform,atlas,sampler,depth);}
 pub fn render(&mut self,queue:&wgpu::Queue,encoder:&mut wgpu::CommandEncoder,target:&wgpu::TextureView,size:(u32,u32),internal:(u32,u32),p:&FrameParams){
  if p.particles.is_empty(){return;}
  let fallback=[ViewDraw{basis:p.basis,rect:[0.0,0.0,size.0 as f32,size.1 as f32]}];let views=if p.views.is_empty(){&fallback[..]}else{&p.views[..p.views.len().min(9)]};
  let mut u=Uniforms::zeroed();u.size=[size.0 as f32,size.1 as f32,internal.0 as f32,internal.1 as f32];u.info[0]=views.len()as u32;for(i,v)in views.iter().enumerate(){u.rects[i]=v.rect;}queue.write_buffer(&self.uniform,0,bytemuck::bytes_of(&u));
  let stereo=p.flags&flags::ANAGLYPH!=0;let modes:&[usize]=if stereo{&[1,2]}else{&[0]};let mut ranges=Vec::new();self.scratch.clear();let tan=(p.fov_y_deg.to_radians()*0.5).tan();
  let weather=1.0-p.atmosphere[1].clamp(0.0,1.0)*5.0/16.0;let sky_loss=(11.0*(1.0-p.atmosphere[0]*weather*if p.flags&flags::THUNDER!=0{weather}else{1.0})*(1.0-p.atmosphere[3])).floor();
  for &mode in modes {let start=self.scratch.len()as u32;
   for(view_index,v)in views.iter().enumerate(){let eye=p.eye+v.basis.right*match mode{1=>-0.035,2=>0.035,_=>0.0};let mut visible:Vec<_>=p.particles.iter().take(MAX_PARTICLES).filter_map(|particle|{
    let delta=particle.position-eye;let off=delta.dot(v.basis.ana);if !delta.is_finite()||delta.length_squared()>p.max_dist.min(32.0).powi(2)||off.abs()>=particle.thickness{return None;}
    let projected=delta-v.basis.ana*off;let z=projected.dot(v.basis.forward);if z<=0.05{return None;}
    Some((particle,projected,z,particle.radius*(1.0-(off/particle.thickness).powi(2)).sqrt()))
   }).collect();visible.sort_by(|a,b|b.2.total_cmp(&a.2));
   for(particle,delta,z,radius)in visible {let level=(particle.light[0]as f32).max(particle.light[1]as f32-sky_loss).clamp(0.0,15.0)/15.0;let bright=level/((1.0-level)*3.0+1.0)*0.95+0.05;let color=[particle.color[0]*bright,particle.color[1]*bright,particle.color[2]*bright,1.0];
    for (x,y)in [(-1.0,-1.0),(1.0,-1.0),(1.0,1.0),(-1.0,-1.0),(1.0,1.0),(-1.0,1.0)]{
     let d=delta+v.basis.right*(x*radius)+v.basis.up*(y*radius);let nx=d.dot(v.basis.right)/(z*tan*(v.rect[2]/v.rect[3]));let ny=d.dot(v.basis.up)/(z*tan);
     let px=v.rect[0]+(nx+1.0)*0.5*v.rect[2]-0.5*size.0 as f32/internal.0 as f32;let py=v.rect[1]+(1.0-ny)*0.5*v.rect[3]-0.5*size.1 as f32/internal.1 as f32;
     self.scratch.push(Vertex{clip:[(px/size.0 as f32*2.0-1.0)*z,(1.0-py/size.1 as f32*2.0)*z,z*0.5,z],delta:d.to_array(),uv:[if x<0.0{particle.uv[0]}else{particle.uv[2]},if y>0.0{particle.uv[1]}else{particle.uv[3]}],color,view:view_index as u32,eye:mode as u32,layer:particle.layer});
    }
   }
  }ranges.push((mode,start..self.scratch.len()as u32));}
  if self.scratch.is_empty(){return;}queue.write_buffer(&self.vertices,0,bytemuck::cast_slice(&self.scratch));
  let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("bounded world particles"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:target,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});pass.set_bind_group(0,&self.group,&[]);pass.set_vertex_buffer(0,self.vertices.slice(..(self.scratch.len()*std::mem::size_of::<Vertex>())as u64));for(mode,range)in ranges{pass.set_pipeline(&self.pipelines[mode]);pass.draw(range,0..1);}
 }
}
#[cfg(test)]mod tests{#[test]fn particle_shader_is_valid(){let s=include_str!("../shaders/particles.wgsl");let module=naga::front::wgsl::parse_str(s).unwrap_or_else(|e|panic!("{}",e.emit_to_string(s)));naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::default()).validate(&module).unwrap();}}
