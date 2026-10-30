//! Bounded background sampling of the world's continuous XZW climate fields.
//! A 16-block lattice spans 512 blocks; hardware interpolation avoids biome seams.
use mc4d_core::{glam::{IVec3,Vec4}, terrain::TerrainGen};
use std::sync::{Arc,mpsc};
pub const SIDE:u32=33;
pub const STEP:i32=16;
struct Request {origin:IVec3,terrain:Arc<TerrainGen>}
struct Result {origin:IVec3,seed:u64,bytes:Vec<u8>}
pub struct Climate {
    pub texture:wgpu::Texture,pub sampler:wgpu::Sampler,pub origin:[i32;4],
    sender:Option<mpsc::SyncSender<Request>>,receiver:mpsc::Receiver<Result>,
    worker:Option<std::thread::JoinHandle<()>>,requested:Option<(IVec3,u64)>,
}
impl Climate {
    pub fn new(device:&wgpu::Device)->Self {
        let texture=device.create_texture(&wgpu::TextureDescriptor {label:Some("continuous biome climate"),
            size:wgpu::Extent3d {width:SIDE,height:SIDE,depth_or_array_layers:SIDE},mip_level_count:1,sample_count:1,
            dimension:wgpu::TextureDimension::D3,format:wgpu::TextureFormat::Rg8Unorm,
            usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[]});
        let sampler=device.create_sampler(&wgpu::SamplerDescriptor {label:Some("climate interpolation"),
            mag_filter:wgpu::FilterMode::Linear,min_filter:wgpu::FilterMode::Linear,..Default::default()});
        let (sender,requests)=mpsc::sync_channel::<Request>(1);let (results,receiver)=mpsc::channel();
        let worker=std::thread::spawn(move ||while let Ok(mut request)=requests.recv(){
            while let Ok(newer)=requests.try_recv(){request=newer;}
            let mut bytes=Vec::with_capacity((SIDE*SIDE*SIDE*2) as usize);
            for w in 0..SIDE {for z in 0..SIDE {for x in 0..SIDE {
                let p=request.origin+IVec3::new(x as i32,z as i32,w as i32)*STEP;
                for v in request.terrain.climate(p.x,p.y,p.z){bytes.push((v*255.0).round() as u8);}
            }}}
            if results.send(Result {origin:request.origin,seed:request.terrain.seed(),bytes}).is_err(){break;}
        });
        Self {texture,sampler,origin:[0;4],sender:Some(sender),receiver,worker:Some(worker),requested:None}
    }
    pub fn sync(&mut self,queue:&wgpu::Queue,terrain:&Arc<TerrainGen>,eye:Vec4){
        let p=eye.floor().as_ivec4();
        let origin=IVec3::new(p.x.div_euclid(64)*64-256,p.z.div_euclid(64)*64-256,p.w.div_euclid(64)*64-256);
        let key=(origin,terrain.seed());
        if self.requested.is_some_and(|r|r.1!=key.1){self.origin[3]=0;}
        if self.requested!=Some(key) && self.sender.as_ref().is_some_and(|s|s.try_send(Request {origin,terrain:terrain.clone()}).is_ok()){self.requested=Some(key);}
        if let Some(result)=self.receiver.try_iter().last().filter(|r|r.seed==terrain.seed()) {
            queue.write_texture(self.texture.as_image_copy(),&result.bytes,wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(SIDE*2),rows_per_image:Some(SIDE)},self.texture.size());
            self.origin=[result.origin.x,result.origin.y,result.origin.z,STEP];
        }
    }
}
impl Drop for Climate {fn drop(&mut self){self.sender=None;if let Some(worker)=self.worker.take(){let _=worker.join();}}}
