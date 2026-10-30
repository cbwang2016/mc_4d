//! Deterministic 4D terrain generation. Safe to call from many threads at once.

use crate::block::*;
use crate::chunk::{CHUNK, CHUNK_VOLUME, Chunk, WORLD_HEIGHT, local_index};
use glam::IVec4;
use noise::{Fbm, MultiFractal, NoiseFn, OpenSimplex};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub const SEA_LEVEL: i32 = 44;
const SNOW_LINE: i32 = 92;
/// Leaves can stick out this far from a tree's trunk column.
const TREE_REACH: i32 = 2;
const TREE_MAX_HEIGHT: i32 = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Biome {
    Plains,
    Forest,
    Desert,
    Taiga,
    Mountains,
    Swamp,
    Ocean,
}

impl Biome {
    pub fn name(self) -> &'static str {
        match self {
            Self::Plains => "Plains",
            Self::Forest => "Forest",
            Self::Desert => "Desert",
            Self::Taiga => "Taiga",
            Self::Mountains => "Mountains",
            Self::Swamp => "Swamp",
            Self::Ocean => "Ocean",
        }
    }
    fn tree_density(self) -> f32 {
        match self {
            Self::Plains => 0.00007,
            Self::Forest => 0.0014,
            Self::Taiga => 0.0009,
            Self::Swamp => 0.0005,
            Self::Mountains => 0.00004,
            Self::Desert | Self::Ocean => 0.0,
        }
    }
}

pub struct TerrainGen {
    seed: u64,
    hills: Fbm<OpenSimplex>,
    detail: Fbm<OpenSimplex>,
    mountains: Fbm<OpenSimplex>,
    temperature: OpenSimplex,
    moisture: OpenSimplex,
    caves: Fbm<OpenSimplex>,
    /// Column heights per chunk column (x, z, w), shared by the 16 chunks stacked in it.
    columns: Mutex<HashMap<(i32, i32, i32), Arc<OnceLock<Arc<ColumnHeights>>>>>,
    dungeons: Mutex<HashMap<IVec4,Arc<OnceLock<Option<Arc<crate::dungeon::Plan>>>>>>,
}

struct ColumnHeights {
    heights: Vec<i32>,
    biomes: Vec<Biome>,
    max: i32,
}

const M: i32 = TREE_REACH;
const SIDE: i32 = CHUNK + 2 * M;

fn col(x: i32, z: i32, w: i32) -> usize {
    ((x + M) + SIDE * ((z + M) + SIDE * (w + M))) as usize
}

/// Cheap integer hash → [0, 1).
pub fn hash4(seed: u32, p: IVec4, salt: u32) -> f32 {
    hash_seed64(seed as u64,p,salt)
}

fn hash_seed64(seed:u64,p:IVec4,salt:u32)->f32 {
    let mut h = seed ^ ((salt as u64) << 32);
    for v in [p.x, p.y, p.z, p.w] {
        h ^= v as u32 as u64;
        h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 29;
    }
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

// Independent noise domains consume high bits; low unsigned32 seeds are unchanged.
fn seed_key(seed:u64,domain:u32)->u32 {
    let low=(seed as u32).wrapping_add(domain);
    let high=seed>>32;
    if high==0 {return low;}
    let mut mixed=high ^ (domain as u64).wrapping_mul(0x9e3779b97f4a7c15);
    mixed=(mixed^(mixed>>30)).wrapping_mul(0xbf58476d1ce4e5b9);
    mixed=(mixed^(mixed>>27)).wrapping_mul(0x94d049bb133111eb);
    low ^ (mixed^(mixed>>31)) as u32
}

impl TerrainGen {
    pub fn new(seed: u32) -> Self { Self::versioned(seed as i64,0) }

    pub fn versioned(seed:i64,version:u32)->Self {
        assert!(version<=1,"unsupported terrain generator");
        let seed=if version==0 {seed as u32 as u64}else{seed as u64};
        Self {
            seed,
            hills: Fbm::new(seed_key(seed,0)).set_octaves(4).set_frequency(1.0 / 160.0),
            detail: Fbm::new(seed_key(seed,1))
                .set_octaves(3)
                .set_frequency(1.0 / 48.0),
            mountains: Fbm::new(seed_key(seed,2))
                .set_octaves(3)
                .set_frequency(1.0 / 400.0),
            temperature: OpenSimplex::new(seed_key(seed,31)),
            moisture: OpenSimplex::new(seed_key(seed,32)),
            caves: Fbm::new(seed_key(seed,3))
                .set_octaves(2)
                .set_frequency(1.0 / 36.0),
            columns: Mutex::new(HashMap::new()),
            dungeons: Mutex::new(HashMap::new()),
        }
    }

    /// Full 64-bit generation identity (legacy worlds keep their unsigned 32-bit value).
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Surface height (the y of the top solid block) of the column at horizontal (x, z, w).
    pub fn height(&self, x: i32, z: i32, w: i32) -> i32 {
        self.sample(x, z, w).0
    }

    pub fn biome(&self, x: i32, z: i32, w: i32) -> Biome {
        self.sample(x, z, w).1
    }

    /// Continuous climate for the classic grass/foliage color lookup tables.
    /// Uses the same noise fields that select biomes, extended into W.
    pub fn climate(&self, x: i32, z: i32, w: i32) -> [f32; 2] {
        let p = [x as f64, z as f64, w as f64];
        [
            (self.temperature.get(p.map(|v| v / 520.0)) * 1.5 + 0.5).clamp(0.0, 1.0) as f32,
            (self.moisture.get(p.map(|v| v / 370.0)) * 1.5 + 0.5).clamp(0.0, 1.0) as f32,
        ]
    }

    fn sample(&self, x: i32, z: i32, w: i32) -> (i32, Biome) {
        let p = [x as f64, z as f64, w as f64];
        let hills = self.hills.get(p);
        let detail = self.detail.get(p);
        let continent = self.mountains.get(p);
        let temperature = self.temperature.get(p.map(|v| v / 520.0));
        let moisture = self.moisture.get(p.map(|v| v / 370.0));
        let biome = if continent < -0.20 {
            Biome::Ocean
        } else if continent > 0.23 {
            Biome::Mountains
        } else if temperature > 0.18 && moisture < 0.06 {
            Biome::Desert
        } else if temperature < -0.18 {
            Biome::Taiga
        } else if moisture > 0.25 && continent < 0.06 {
            Biome::Swamp
        } else if moisture > 0.02 {
            Biome::Forest
        } else {
            Biome::Plains
        };
        // Smooth climate weights avoid walls at biome borders. Surface materials and
        // vegetation may change at a boundary, while the underlying land joins up.
        let mountain = ((continent - 0.12) / 0.28).clamp(0.0, 1.0);
        let wetland = ((moisture - 0.20) / 0.20).clamp(0.0, 1.0)
            * ((0.10 - continent) / 0.15).clamp(0.0, 1.0)
            * ((continent + 0.20) / 0.10).clamp(0.0, 1.0);
        let base = 54.0 + continent * 60.0 + hills * 8.0 + detail * 2.0;
        let highlands = base + mountain * (35.0 + hills.abs() * 80.0 + detail * 8.0);
        let height = highlands * (1.0 - wetland) + (SEA_LEVEL as f64 + detail * 3.0) * wetland;
        (
            height.round().clamp(4.0, WORLD_HEIGHT as f64 - 20.0) as i32,
            biome,
        )
    }

    #[cfg(test)]
    fn has_tree(&self, x: i32, z: i32, w: i32, height: i32) -> bool {
        self.tree_at(x, z, w, height, self.biome(x, z, w))
    }

    fn tree_at(&self, x: i32, z: i32, w: i32, height: i32, biome: Biome) -> bool {
        height
            > (if biome == Biome::Swamp {
                SEA_LEVEL - 1
            } else {
                SEA_LEVEL + 1
            })
            && height
                < (if biome == Biome::Taiga {
                    112
                } else {
                    SNOW_LINE - 6
                })
            && hash_seed64(self.seed, IVec4::new(x, 0, z, w), 7) < biome.tree_density()
    }

    fn tree_height(&self, x: i32, z: i32, w: i32) -> i32 {
        let random = hash_seed64(self.seed, IVec4::new(x, 0, z, w), 8);
        match self.biome(x, z, w) {
            Biome::Taiga => 6 + (random * 4.0) as i32,
            Biome::Swamp => 3 + (random * 2.0) as i32,
            _ => 4 + (random * 3.0) as i32,
        }
    }

    /// Generate the chunk at chunk coordinate `c`.
    pub fn generate(&self, c: IVec4) -> Chunk {
        let origin = c * CHUNK;
        if origin.y >= WORLD_HEIGHT {
            return Chunk::uniform(AIR);
        }
        if origin.y + CHUNK <= 0 {
            return Chunk::uniform(BEDROCK);
        }

        let column = self.column(c);
        let heights = &column.heights;
        let max_h = column.max;
        let top = max_h.max(SEA_LEVEL) + TREE_MAX_HEIGHT + 2;
        if origin.y > top {
            return Chunk::uniform(AIR);
        }

        let caves = self.cave_grid(origin);
        let mut blocks = vec![AIR; CHUNK_VOLUME];
        for w in 0..CHUNK {
            for z in 0..CHUNK {
                for x in 0..CHUNK {
                    let h = heights[col(x, z, w)];
                    let biome = column.biomes[col(x, z, w)];
                    for y in 0..CHUNK {
                        let wy = origin.y + y;
                        let l = IVec4::new(x, y, z, w);
                        let world = origin + l;
                        let b = if wy == 0 || (wy <= 2 && hash_seed64(self.seed, world, 1) < 0.5) {
                            BEDROCK
                        } else if wy <= h {
                            let cave = wy > 3 && wy < h - 2 && cave_sample(&caves, l) > 0.35;
                            if cave {
                                if wy < 10 { LAVA } else { AIR }
                            } else if wy>h-3 && h<SEA_LEVEL && biome!=Biome::Swamp && hash_seed64(self.seed,IVec4::new(world.x.div_euclid(4),0,world.z.div_euclid(4),world.w.div_euclid(4)),40)<0.025 {
                                CLAY
                            } else if wy == h {
                                if biome == Biome::Desert
                                    || biome != Biome::Swamp && h <= SEA_LEVEL + 1
                                {
                                    SAND
                                } else if biome == Biome::Taiga || h >= 112 {
                                    SNOW
                                } else if biome == Biome::Mountains && h >= 80 {
                                    STONE
                                } else {
                                    GRASS
                                }
                            } else if wy > h - 4 {
                                if biome == Biome::Desert
                                    || biome != Biome::Swamp && h <= SEA_LEVEL + 1
                                {
                                    SAND
                                } else if biome == Biome::Mountains && h >= 80 {
                                    STONE
                                } else {
                                    DIRT
                                }
                            } else {
                                ore_at(self.seed, world)
                            }
                        } else if wy <= SEA_LEVEL {
                            WATER
                        } else {
                            AIR
                        };
                        blocks[local_index(l)] = b;
                    }
                }
            }
        }

        let mut metadata = self.place_trees(origin, &column, &mut blocks);
        let mut chests=std::collections::BTreeMap::new();
        for w in c.w-1..=c.w+1 {for z in c.z-1..=c.z+1 {for x in c.x-1..=c.x+1 {for attempt in 0..8 {
            let key=IVec4::new(x,attempt,z,w);let candidate=crate::dungeon::candidate(seed_key(self.seed,0),key,attempt as u8);
            if !candidate.intersects_chunk(c){continue;}
            let Some(plan)=self.dungeon_plan(key,&candidate)else{continue};
            for &(p,id) in &plan.blocks {let local=p-origin;if in_chunk(local){let index=local_index(local);blocks[index]=id;if let Some(metadata)=&mut metadata{metadata[index]=0;}}}
            for (p,slots) in &plan.chests {let local=*p-origin;if in_chunk(local){chests.insert(local_index(local)as u16,slots.clone());}}
        }}}}
        self.place_decorations(origin,&column,&mut blocks);
        chests.retain(|index,_|blocks[*index as usize]==CHEST);
        let mut result=Chunk::from_planes(blocks, metadata).expect("generated planes have chunk volume");result.generated_chests=chests.into_iter().collect();result
    }

    fn place_decorations(&self,origin:IVec4,column:&ColumnHeights,blocks:&mut[BlockId]) {
        for w in 0..16 {for z in 0..16 {for x in 0..16 {
            let height=column.heights[col(x,z,w)];let biome=column.biomes[col(x,z,w)];let root=IVec4::new(origin.x+x,height+1,origin.z+z,origin.w+w);
            let chance=hash_seed64(self.seed,root,41);
            let mut plant=None;
            if biome==Biome::Desert && height>SEA_LEVEL+1 && chance<0.003 {
                let clear=crate::shapes::HORIZONTAL_DIRECTIONS.into_iter().all(|d|column.heights[col(x+d.x,z+d.z,w+d.w)]<=height);
                if clear {let bound=1+(hash_seed64(self.seed,root,44)*3.0)as i32;plant=Some((CACTUS,1+(hash_seed64(self.seed,root,45)*bound as f32)as i32));}
            } else if biome==Biome::Swamp && height==SEA_LEVEL && chance<0.08 {
                let water=crate::shapes::HORIZONTAL_DIRECTIONS.into_iter().any(|d|column.heights[col(x+d.x,z+d.z,w+d.w)]<SEA_LEVEL);
                if water {let bound=1+(hash_seed64(self.seed,root,44)*3.0)as i32;plant=Some((REEDS,2+(hash_seed64(self.seed,root,45)*bound as f32)as i32));}
            } else if height>SEA_LEVEL+1 && height<112 && !matches!(biome,Biome::Desert|Biome::Taiga) && !(biome==Biome::Mountains&&height>=80) && chance<0.002 {
                plant=Some((if hash_seed64(self.seed,root,43)<0.8{DANDELION}else{ROSE},1));
            }
            if let Some((id,length))=plant {for y in 0..length {let p=root+IVec4::Y*y-origin;if in_chunk(p)&&blocks[local_index(p)]==AIR {blocks[local_index(p)]=id;}}}
            for y in 1..16 {
                let p=origin+IVec4::new(x,y,z,w);
                if p.y>12 && p.y<height-4 && blocks[local_index(IVec4::new(x,y,z,w))]==AIR && crate::block::definition(blocks[local_index(IVec4::new(x,y-1,z,w))]).opaque && hash_seed64(self.seed,p,46)<0.003 {
                    blocks[local_index(IVec4::new(x,y,z,w))]=if hash_seed64(self.seed,p,47)<0.67{BROWN_MUSHROOM}else{RED_MUSHROOM};
                }
            }
        }}}
    }

    fn dungeon_plan(&self,key:IVec4,candidate:&crate::dungeon::Candidate)->Option<Arc<crate::dungeon::Plan>> {
        let slot={let mut plans=self.dungeons.lock().unwrap_or_else(|e|e.into_inner());plans.entry(key).or_insert_with(||Arc::new(OnceLock::new())).clone()};
        slot.get_or_init(||{
            let mut grids=HashMap::new();
            crate::dungeon::plan(candidate,&mut|p|{
                if p.y<0||p.y>=128{return AIR;}if p.y<=3{return STONE;}
                let c=crate::chunk::chunk_of(p);let l=crate::chunk::local_of(p);let column=self.column(c);let height=column.heights[col(l.x,l.z,l.w)];
                if p.y>height{return if p.y<=SEA_LEVEL{WATER}else{AIR};}
                if p.y<height-2 {
                    let grid=grids.entry(c).or_insert_with(||self.cave_grid(c*16));
                    if cave_sample(grid,l)>0.35{return if p.y<10{LAVA}else{AIR};}
                }
                STONE
            }).map(Arc::new)
        }).clone()
    }

    fn place_trees(
        &self,
        origin: IVec4,
        column: &ColumnHeights,
        blocks: &mut [BlockId],
    ) -> Option<Vec<u8>> {
        let mut metadata: Option<Vec<u8>> = None;
        let mut set_species = |index: usize, species: u8| {
            if species != 0 && metadata.is_none() {
                metadata = Some(vec![0; CHUNK_VOLUME]);
            }
            if let Some(plane) = &mut metadata {
                plane[index] = species;
            }
        };
        let r = TREE_REACH;
        for tw in -r..CHUNK + r {
            for tz in -r..CHUNK + r {
                for tx in -r..CHUNK + r {
                    let h = column.heights[col(tx, tz, tw)];
                    let biome = column.biomes[col(tx, tz, tw)];
                    let (gx, gz, gw) = (origin.x + tx, origin.z + tz, origin.w + tw);
                    if !self.tree_at(gx, gz, gw, h, biome) {
                        continue;
                    }
                    let species = if biome == Biome::Taiga { 1 } else { 0 };
                    let th = self.tree_height(gx, gz, gw);
                    let top = h + th;
                    if top + 2 < origin.y || h + 1 >= origin.y + CHUNK + r {
                        continue;
                    }
                    // Leaves: a 4D ball around the top of the trunk.
                    for dw in -r..=r {
                        for dz in -r..=r {
                            for dx in -r..=r {
                                for dy in (if biome == Biome::Taiga { -5 } else { -2 })..=1 {
                                    let d2 = dx * dx + dz * dz + dw * dw + dy * dy;
                                    let outside = if biome == Biome::Taiga {
                                        let radius = if dy >= 0 {
                                            0
                                        } else if dy >= -2 {
                                            1
                                        } else {
                                            2
                                        };
                                        dx * dx + dz * dz + dw * dw > radius * radius + 1
                                    } else {
                                        d2 > 5
                                    };
                                    if outside {
                                        continue;
                                    }
                                    let l =
                                        IVec4::new(tx + dx, top + dy - origin.y, tz + dz, tw + dw);
                                    if in_chunk(l) && blocks[local_index(l)] == AIR {
                                        blocks[local_index(l)] = LEAVES;
                                        set_species(local_index(l), species);
                                    }
                                }
                            }
                        }
                    }
                    // Trunk.
                    for y in h + 1..=top {
                        let l = IVec4::new(tx, y - origin.y, tz, tw);
                        if in_chunk(l) {
                            blocks[local_index(l)] = LOG;
                            set_species(local_index(l), species);
                        }
                    }
                    let below = IVec4::new(tx, h - origin.y, tz, tw);
                    if in_chunk(below) {
                        blocks[local_index(below)] = DIRT;
                        set_species(local_index(below), 0);
                    }
                }
            }
        }
        metadata
    }

    /// Heights for the chunk column of `c`, with a margin so trees from neighbouring
    /// columns can reach in. Cached because all 16 chunks of a column need the same data.
    fn column(&self, c: IVec4) -> Arc<ColumnHeights> {
        let key = (c.x, c.z, c.w);
        let slot = {
            let mut cache = self.columns.lock().unwrap();
            if cache.len() > 8192 {
                cache.clear();
            }
            cache
                .entry(key)
                .or_insert_with(|| Arc::new(OnceLock::new()))
                .clone()
        };
        slot.get_or_init(|| {
            let origin = c * CHUNK;
            let mut heights = vec![0i32; (SIDE * SIDE * SIDE) as usize];
            let mut biomes = vec![Biome::Plains; heights.len()];
            let mut max = i32::MIN;
            for w in -M..CHUNK + M {
                for z in -M..CHUNK + M {
                    for x in -M..CHUNK + M {
                        let (h, biome) = self.sample(origin.x + x, origin.z + z, origin.w + w);
                        heights[col(x, z, w)] = h;
                        biomes[col(x, z, w)] = biome;
                        max = max.max(h);
                    }
                }
            }
            Arc::new(ColumnHeights {
                heights,
                biomes,
                max,
            })
        })
        .clone()
    }

    /// 4D cave noise sampled every 4 blocks over the chunk (5⁴ samples), interpolated later.
    fn cave_grid(&self, origin: IVec4) -> [f32; 625] {
        let mut g = [0.0f32; 625];
        for i in 0..625 {
            let (x, y, z, w) = (i % 5, (i / 5) % 5, (i / 25) % 5, i / 125);
            let p = origin + IVec4::new(x as i32, y as i32, z as i32, w as i32) * 4;
            g[i] = self
                .caves
                .get([p.x as f64, p.y as f64 * 1.6, p.z as f64, p.w as f64])
                as f32;
        }
        g
    }
}

/// Small deterministic 4D ore veins. World-space grouping lets veins cross chunk
/// boundaries without generation-order dependencies. Height limits retain the
/// progression from coal/iron near the surface to gold/diamond deeper down.
fn ore_at(seed: u64, pos: IVec4) -> u8 {
    if pos.y>=128{return STONE;}
    let cell = IVec4::new(
        pos.x.div_euclid(2),
        pos.y.div_euclid(2),
        pos.z.div_euclid(2),
        pos.w.div_euclid(2),
    );
    if hash_seed64(seed, pos, 28) >= 0.65 {
        return STONE;
    }
    if hash_seed64(seed,cell,29)<0.025 {return GRAVEL;}
    // Original population uses nextInt(16)+nextInt(16) for lapis height.
    let lapis_weight=(1.0-(pos.y as f32-15.0).abs()/16.0).max(0.0);
    if pos.y<31 && hash_seed64(seed,cell,30)<0.01*lapis_weight{return LAPIS_ORE;}
    if pos.y<16 && hash_seed64(seed,cell,31)<0.025{return REDSTONE_ORE;}
    if pos.y < 16 && hash_seed64(seed, cell, 24) < 0.006 {
        DIAMOND_ORE
    } else if pos.y < 32 && hash_seed64(seed, cell, 25) < 0.012 {
        GOLD_ORE
    } else if pos.y < 64 && hash_seed64(seed, cell, 26) < 0.035 {
        IRON_ORE
    } else if hash_seed64(seed, cell, 27) < 0.06 {
        COAL_ORE
    } else {
        STONE
    }
}

fn in_chunk(l: IVec4) -> bool {
    l.cmpge(IVec4::ZERO).all() && l.cmplt(IVec4::splat(CHUNK)).all()
}

/// Quadrilinear interpolation of the cave grid at a local block position.
fn cave_sample(g: &[f32; 625], l: IVec4) -> f32 {
    let cell = l / 4;
    let f = (l % 4).as_vec4() / 4.0;
    let mut acc = 0.0;
    for corner in 0..16 {
        let o = IVec4::new(
            corner & 1,
            (corner >> 1) & 1,
            (corner >> 2) & 1,
            (corner >> 3) & 1,
        );
        let p = cell + o;
        let idx = (p.x + 5 * (p.y + 5 * (p.z + 5 * p.w))) as usize;
        let wgt = (0..4).fold(1.0, |a, i| a * if o[i] == 1 { f[i] } else { 1.0 - f[i] });
        acc += g[idx] * wgt;
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_dungeon_is_deterministic_across_chunk_order_and_carries_loot(){
        let generator=TerrainGen::new(1337);let c=IVec4::new(-2,2,3,-2);
        let chunk=generator.generate(c);assert!(!chunk.generated_chests.is_empty());
        let fresh=TerrainGen::new(1337);fresh.generate(c+IVec4::Y);let other=fresh.generate(c);
        for i in 0..CHUNK_VOLUME {let p=IVec4::new((i&15)as i32,((i>>4)&15)as i32,((i>>8)&15)as i32,((i>>12)&15)as i32);assert_eq!(chunk.get_state(p),other.get_state(p));}
        assert_eq!(chunk.generated_chests,other.generated_chests);
        for (i,slots) in &chunk.generated_chests {let i=*i as i32;assert_eq!(chunk.get(IVec4::new(i&15,(i>>4)&15,(i>>8)&15,(i>>12)&15)),CHEST);assert_eq!(slots.len(),27);assert!(slots.iter().any(|s|s.count>0));}
    }

    #[test]
    fn generated_materials_support_survival_without_creative_injection(){
        let g=TerrainGen::new(1337);
        for (c,ids) in [(IVec4::new(-3,3,0,0),vec![DANDELION,ROSE]),(IVec4::new(-6,3,-6,0),vec![CACTUS]),(IVec4::new(11,2,-3,3),vec![REEDS]),(IVec4::new(-12,2,21,-24),vec![CLAY]),(IVec4::new(0,2,0,0),vec![BROWN_MUSHROOM,RED_MUSHROOM]),(IVec4::new(-12,0,21,-24),vec![LAPIS_ORE,REDSTONE_ORE,GRAVEL])] {
            let chunk=g.generate(c);for id in ids{assert!(chunk.may_contain(id),"missing generated{} at{c:?}",crate::block::info(id).name);}
        }
    }

    #[test]
    #[ignore="search a deterministic natural reeds fixture"]
    fn natural_reeds_fixture_search(){
        let g=TerrainGen::new(1337);for w in -16..=16 {for z in -16..=16 {for x in -16..=16 {
            let wx=x*16+8;let wz=z*16+8;let ww=w*16+8;
            if g.biome(wx,wz,ww)!=Biome::Swamp||(g.height(wx,wz,ww)-SEA_LEVEL).abs()>2{continue;}
            let c=IVec4::new(x,SEA_LEVEL/16,z,w);let chunk=g.generate(c);if chunk.may_contain(REEDS){println!("natural reeds {c:?}");return;}
        }}}panic!("no reeds fixture found");
    }

    #[test]
    #[ignore="search deterministic generated-resource fixtures"]
    fn generated_resource_fixture_search(){
        let g=TerrainGen::new(1337);
        for c in [IVec4::new(-3,3,0,0),IVec4::new(-6,3,-6,0),IVec4::new(-6,4,-6,0),IVec4::new(6,2,-3,8),IVec4::new(-12,0,21,-24),IVec4::new(-12,1,21,-24),IVec4::new(-12,2,21,-24),IVec4::new(0,1,0,0),IVec4::new(1,1,0,0),IVec4::new(0,2,0,0)] {
            let chunk=g.generate(c);println!("{c:?}: {:?}",[DANDELION,ROSE,CACTUS,REEDS,BROWN_MUSHROOM,RED_MUSHROOM,CLAY,GRAVEL,LAPIS_ORE,REDSTONE_ORE].into_iter().filter(|&id|chunk.may_contain(id)).map(|id|crate::block::info(id).name).collect::<Vec<_>>());
        }
    }

    #[test]
    #[ignore="reference search for a deterministic natural dungeon fixture"]
    fn find_natural_dungeon_fixture(){
        let generator=TerrainGen::new(1337);
        for w in -3..=3 {for z in -3..=3 {for x in -3..=3 {for attempt in 0..8 {
            let key=IVec4::new(x,attempt,z,w);let candidate=crate::dungeon::candidate(1337,key,attempt as u8);
            if let Some(plan)=generator.dungeon_plan(key,&candidate) {
                println!("dungeon key={key:?} center={:?} blocks={} chests={}",candidate.center,plan.blocks.len(),plan.chests.len());
                if !plan.chests.is_empty(){return;}
            }
        }}}}
        panic!("no natural dungeon chest found in fixture search");
    }

    #[test]
    fn broad_world_samples_include_distinct_landforms_and_all_biomes() {
        let mut seen = std::collections::HashSet::new();
        let mut min_height = WORLD_HEIGHT;
        let mut max_height = 0;
        for seed in [7, 42, 1337] {
            let generator = TerrainGen::new(seed);
            for w in [-512, 0, 512] {
                for z in (-1536..=1536).step_by(192) {
                    for x in (-1536..=1536).step_by(192) {
                        let (height, biome) = generator.sample(x, z, w);
                        assert_eq!((height, biome), TerrainGen::new(seed).sample(x, z, w));
                        seen.insert(biome);
                        min_height = min_height.min(height);
                        max_height = max_height.max(height);
                    }
                }
            }
        }
        assert_eq!(seen.len(), 7, "missing biome: {seen:?}");
        assert!(min_height < SEA_LEVEL - 8, "oceans need meaningful depth");
        assert!(max_height > 100, "mountains must rise above ordinary hills");
    }

    #[test]
    fn biome_and_height_cache_agree_across_negative_chunk_seams() {
        let generator = TerrainGen::new(1337);
        let left = generator.column(IVec4::new(-1, 0, 0, 0));
        let right = generator.column(IVec4::ZERO);
        for x in -2..=1 {
            for z in 0..16 {
                for w in 0..16 {
                    assert_eq!(left.heights[col(x + 16, z, w)], right.heights[col(x, z, w)]);
                    assert_eq!(left.biomes[col(x + 16, z, w)], right.biomes[col(x, z, w)]);
                    assert_eq!(right.heights[col(x, z, w)], generator.height(x, z, w));
                }
            }
        }
    }

    #[test]
    fn ores_are_repeatable_and_respect_depth_limits() {
        let mut seen = [false; 6];
        for w in -12..12 {
            for x in -12..12 {
                for y in 4..80 {
                    let p = IVec4::new(x, y, 3, w);
                    let ore = ore_at(17, p);
                    assert_eq!(ore, ore_at(17, p));
                    match ore {
                        COAL_ORE => seen[0] = true,
                        IRON_ORE => {
                            assert!(y < 64);
                            seen[1] = true;
                        }
                        GOLD_ORE => {
                            assert!(y < 32);
                            seen[2] = true;
                        }
                        DIAMOND_ORE => {
                            assert!(y < 16);
                            seen[3] = true;
                        }
                        LAPIS_ORE=>{assert!(y<31);seen[4]=true;}
                        REDSTONE_ORE=>{assert!(y<16);seen[5]=true;}
                        STONE|GRAVEL => {}
                        _ => panic!("not an ore"),
                    }
                }
            }
        }
        assert!(
            seen.iter().all(|v| *v),
            "every ore has a reachable depth band"
        );
    }

    #[test]
    fn deterministic_and_layered() {
        let g = TerrainGen::new(42);
        let h = g.height(0, 0, 0);
        assert_eq!(h, TerrainGen::new(42).height(0, 0, 0));
        let c = g.generate(IVec4::new(0, h / CHUNK, 0, 0));
        assert!(c.uniform_id().is_none(), "surface chunk should be mixed");
        assert_eq!(g.generate(IVec4::new(0, 15, 0, 0)).uniform_id(), Some(AIR));
        assert_eq!(
            g.generate(IVec4::new(0, -1, 0, 0)).uniform_id(),
            Some(BEDROCK)
        );
    }

    #[test]
    fn generated_conifers_keep_spruce_metadata_and_species_loot() {
        let g = TerrainGen::new(5);
        for x in (-3000..3000).step_by(7) {
            for w in 0..8 {
                if g.biome(x, 0, w) != Biome::Taiga {
                    continue;
                }
                let h = g.height(x, 0, w);
                if !g.has_tree(x, 0, w, h) {
                    continue;
                }
                let p = IVec4::new(x, h + 1, 0, w);
                let chunk = g.generate(crate::chunk::chunk_of(p));
                assert_eq!(
                    chunk.get_state(crate::chunk::local_of(p)),
                    BlockState::new(LOG, 1)
                );
                let top = IVec4::new(x, h + g.tree_height(x, 0, w) + 1, 0, w);
                let crown = g.generate(crate::chunk::chunk_of(top));
                let leaves = crown.get_state(crate::chunk::local_of(top));
                assert_eq!(leaves, BlockState::new(LEAVES, 1));
                let drops = crate::loot::drops(leaves, crate::loot::DropCause::Natural, |_| 0);
                assert_eq!(drops, vec![crate::item::Stack::with_variant(SAPLING, 1, 1)]);
                return;
            }
        }
        panic!("no conifer fixture found");
    }

    #[test]
    fn trees_cross_chunk_borders_consistently() {
        // Leaves near a chunk border are placed by both chunks from the same trunk, so
        // the blocks on each side must match a single generation of the 4D neighbourhood.
        let g = TerrainGen::new(5);
        let mut checked = 0;
        'search: for x in 0..4000 {
            for w in 0..8 {
                let h = g.height(x * 16 + 15, 0, w);
                if g.has_tree(x * 16 + 15, 0, w, h) {
                    let top = h + g.tree_height(x * 16 + 15, 0, w);
                    let right = g.generate(IVec4::new(
                        x + 1,
                        top.div_euclid(CHUNK),
                        0,
                        w.div_euclid(CHUNK),
                    ));
                    let l = IVec4::new(0, top.rem_euclid(CHUNK), 0, w.rem_euclid(CHUNK));
                    assert_eq!(right.get(l), LEAVES);
                    checked += 1;
                    break 'search;
                }
            }
        }
        assert_eq!(checked, 1, "no tree found at a chunk border");
    }

    fn fingerprint(generator: &TerrainGen) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for c in [IVec4::new(0, 3, 0, 0), IVec4::new(-1, 0, 2, -3), IVec4::new(5, 4, -7, 1), IVec4::new(-9, 1, -2, 6)] {
            let chunk = generator.generate(c);
            for w in 0..16 { for z in 0..16 { for y in 0..16 { for x in 0..16 {
                let s = chunk.get_state(IVec4::new(x, y, z, w));
                for b in [s.id, s.metadata] { h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3); }
            }}}}
        }
        h
    }

    /// Golden values from the unsigned-32-bit generator in commit a6e61df.
    const LEGACY: [(u32, u64); 5] = [
        (0, 0xd8b6ca7eefe1bcce),
        (7, 0x95cbd8d9d3b4df49),
        (1337, 0x7d022d3fd910c9c9),
        (0x8000_0000, 0x2dd9f5f099e65553),
        (u32::MAX, 0xeb8daf939bbed793),
    ];

    #[test]
    fn unsigned_32_bit_worlds_keep_their_original_terrain() {
        for (seed, expected) in LEGACY {
            assert_eq!(fingerprint(&TerrainGen::new(seed)), expected, "legacy seed {seed}");
            // A new-format world with the same non-negative value is the same world.
            assert_eq!(fingerprint(&TerrainGen::versioned(seed as i64, 1)), expected, "version 1 seed {seed}");
        }
    }

    #[test]
    fn upper_seed_bits_and_sign_select_distinct_worlds() {
        let base = fingerprint(&TerrainGen::versioned(1337, 1));
        let mut seen = std::collections::HashSet::from([base]);
        for seed in [1337 | (1i64 << 32), 1337 | (1i64 << 62), 1337 - (1i64 << 32), -1337, i64::MIN, i64::MAX] {
            let generator = TerrainGen::versioned(seed, 1);
            assert_eq!(generator.seed(), seed as u64, "cache identity keeps every bit");
            assert!(seen.insert(fingerprint(&generator)), "seed {seed} repeated another world");
        }
        // Version 0 is defined by the low 32 bits only.
        assert_eq!(TerrainGen::versioned(1337 | (1i64 << 40), 0).seed(), 1337);
    }
}
