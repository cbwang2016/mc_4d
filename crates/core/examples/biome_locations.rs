//! Print reproducible biome camera fixtures for screenshots and performance probes.
use mc4d_core::terrain::{Biome, TerrainGen, SEA_LEVEL};
use std::collections::HashMap;

fn main() {
    let terrain = TerrainGen::new(1337);
    let mut found = HashMap::new();
    for w in [0,-128,128,-384,384] {
        for z in (-1536_i32..=1536).step_by(48) {
            for x in (-1536_i32..=1536).step_by(48) {
                let biome = terrain.biome(x,z,w);
                if ![(32,0,0),(-32,0,0),(0,32,0),(0,-32,0),(0,0,32),(0,0,-32)]
                    .into_iter().all(|(dx,dz,dw)|terrain.biome(x+dx,z+dz,w+dw)==biome) { continue; }
                let ground = terrain.height(x,z,w);
                let distance=x*x+z*z+w*w;
                if found.get(&biome).is_none_or(|&(d,_,_,_,_)|distance<d) { found.insert(biome,(distance,x,ground,z,w)); }
            }
        }
    }
    println!("[");
    let biomes=[Biome::Plains,Biome::Forest,Biome::Desert,Biome::Taiga,Biome::Mountains,Biome::Swamp,Biome::Ocean];
    for (index,biome) in biomes.into_iter().enumerate() {
        let (_,x,ground,z,w)=found[&biome];
        let y=ground.max(SEA_LEVEL)+3;
        println!("  {{\"biome\":\"{}\",\"seed\":1337,\"pos\":[{}, {}, {}, {}],\"ground_y\":{},\"yaw\":45,\"pitch\":-15}}{}",
            biome.name(),x as f32+0.5,y as f32,z as f32+0.5,w as f32+0.5,ground,if index==6 { "" } else { "," });
    }
    println!("]");
}
