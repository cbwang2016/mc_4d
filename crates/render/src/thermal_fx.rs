//! Original Beta 1.5 lava (`bz`/`hq`) and fire (`ra`) texture equations.
//! Constants and update ordering are transcribed from the verified client bytecode.
//! Only the random source is deterministic, for reproducible captures.
fn random(seed: &mut u64) -> f32 {
    *seed ^= *seed << 13; *seed ^= *seed >> 7; *seed ^= *seed << 17;
    (*seed >> 40) as f32 / 16777216.0
}
struct Lava { heat: [f32; 256], velocity: [f32; 256], impulse: [f32; 256] }
impl Lava {
    fn new() -> Self { Self { heat: [0.0;256], velocity: [0.0;256], impulse: [0.0;256] } }
    fn step(&mut self, rng: &mut u64) {
        let mut next = [0.0;256];
        // x-major ordering matters: the original modifies velocity in place.
        for x in 0i32..16 { for y in 0i32..16 {
            let dx = ((y as f32 * std::f32::consts::TAU / 16.0).sin() * 1.2) as i32;
            let dy = ((x as f32 * std::f32::consts::TAU / 16.0).sin() * 1.2) as i32;
            let index = |a:i32,b:i32| ((a & 15) + (b & 15)*16) as usize;
            let mut sum = 0.0;
            for a in x-1..=x+1 { for b in y-1..=y+1 { sum += self.heat[index(a+dx,b+dy)]; } }
            let i = index(x,y);
            next[i] = sum / 10.0 + (self.velocity[index(x,y)] + self.velocity[index(x+1,y)]
                + self.velocity[index(x+1,y+1)] + self.velocity[index(x,y+1)]) / 4.0 * 0.8;
            self.velocity[i] = (self.velocity[i] + self.impulse[i]*0.01).max(0.0);
            self.impulse[i] -= 0.06;
            if random(rng) < 0.005 { self.impulse[i] = 1.5; }
        }}
        self.heat = next;
    }
    fn pixels(&self, offset:usize) -> Vec<u8> {
        let mut pixels = Vec::with_capacity(1024);
        for i in 0usize..256 {
            let heat = (self.heat[i.wrapping_sub(offset) & 255]*2.0).clamp(0.0,1.0);
            pixels.extend_from_slice(&[(heat*100.0+155.0) as u8,(heat*heat*255.0) as u8,
                (heat*heat*heat*heat*128.0) as u8,255]);
        }
        pixels
    }
}
struct Fire { heat:[f32;320] }
impl Fire {
    fn new()->Self {Self {heat:[0.0;320]}}
    fn step(&mut self,rng:&mut u64) {
        let mut next=[0.0;320];
        for x in 0i32..16 {for y in 0i32..20 {
            let mut sum=self.heat[(x+(y+1)%20*16) as usize]*18.0;
            for a in x-1..=x+1 {for b in y..=y+1 {
                if (0..16).contains(&a) && (0..20).contains(&b) {sum+=self.heat[(a+b*16) as usize];}
            }}
            next[(x+y*16) as usize]=if y==19 {
                random(rng)*random(rng)*random(rng)*4.0+random(rng)*0.1+0.2
            } else {sum/(24.0*1.06)};
        }}
        self.heat=next;
    }
    fn pixels(&self)->Vec<u8> {
        let mut pixels=Vec::with_capacity(1024);
        for value in &self.heat[..256] {
            let v=(value*1.8).clamp(0.0,1.0);
            pixels.extend_from_slice(&[(v*155.0+100.0) as u8,(v*v*255.0) as u8,
                (v.powi(10)*255.0) as u8,if v<0.5 {0}else{255}]);
        }
        pixels
    }
}
pub struct ThermalFx { lava:[Lava;2],fire:[Fire;2],rng:u64,tick:u32,last:Option<u32> }
impl Default for ThermalFx {
    fn default()->Self {
        let mut fx=Self {lava:[Lava::new(),Lava::new()],fire:[Fire::new(),Fire::new()],rng:0xB15F_1A4E_1234_ABCD,tick:0,last:None};
        for _ in 0..80 {fx.step();} fx
    }
}
impl ThermalFx {
    fn step(&mut self) {for lava in &mut self.lava {lava.step(&mut self.rng);} for fire in &mut self.fire {fire.step(&mut self.rng);} self.tick=self.tick.wrapping_add(1);}
    pub fn force_upload(&mut self){self.last=None;}
    pub fn update(&mut self,time:f32)->Option<[Vec<u8>;4]> {
        let now=(time.max(0.0)*20.0) as u32;
        let steps=self.last.map_or(0,|last|now.saturating_sub(last).min(4));
        let changed=self.last.is_none() || steps>0;self.last=Some(now);
        for _ in 0..steps {self.step();}
        changed.then(||[self.lava[0].pixels(0),self.lava[1].pixels((self.tick/3) as usize*16),self.fire[0].pixels(),self.fire[1].pixels()])
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn native_color_and_alpha_ranges_and_tick_clock() {
        let mut fx=ThermalFx::default();let first=fx.update(0.0).unwrap();
        for lava in &first[..2] {for p in lava.chunks_exact(4) {assert!(p[0]>=155);assert!(p[2]<=128);assert_eq!(p[3],255);}}
        for fire in &first[2..] {assert!(fire.chunks_exact(4).any(|p|p[3]==0));assert!(fire.chunks_exact(4).any(|p|p[3]==255));}
        assert!(fx.update(0.01).is_none());assert_ne!(first,fx.update(0.05).unwrap());
        assert_eq!(first,ThermalFx::default().update(0.0).unwrap());
    }
}
