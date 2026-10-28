//! Deterministic 4D terrain generation. Safe to call from many threads at once.

use crate::block::*;
use crate::chunk::{CHUNK, CHUNK_VOLUME, Chunk, WORLD_HEIGHT, local_index};
use glam::IVec4;
use noise::{Fbm, MultiFractal, NoiseFn, OpenSimplex};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const SEA_LEVEL: i32 = 44;
const SNOW_LINE: i32 = 92;
/// Leaves can stick out this far from a tree's trunk column.
const TREE_REACH: i32 = 2;
const TREE_MAX_HEIGHT: i32 = 9;

pub struct TerrainGen {
    seed: u32,
    hills: Fbm<OpenSimplex>,
    detail: Fbm<OpenSimplex>,
    mountains: Fbm<OpenSimplex>,
    caves: Fbm<OpenSimplex>,
    /// Column heights per chunk column (x, z, w), shared by the 16 chunks stacked in it.
    columns: Mutex<HashMap<(i32, i32, i32), Arc<ColumnHeights>>>,
}

struct ColumnHeights {
    heights: Vec<i32>,
    max: i32,
}

const M: i32 = TREE_REACH;
const SIDE: i32 = CHUNK + 2 * M;

fn col(x: i32, z: i32, w: i32) -> usize {
    ((x + M) + SIDE * ((z + M) + SIDE * (w + M))) as usize
}

/// Cheap integer hash → [0, 1).
pub fn hash4(seed: u32, p: IVec4, salt: u32) -> f32 {
    let mut h = (seed as u64) ^ ((salt as u64) << 32);
    for v in [p.x, p.y, p.z, p.w] {
        h ^= v as u32 as u64;
        h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 29;
    }
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

impl TerrainGen {
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            hills: Fbm::new(seed).set_octaves(4).set_frequency(1.0 / 160.0),
            detail: Fbm::new(seed.wrapping_add(1)).set_octaves(3).set_frequency(1.0 / 48.0),
            mountains: Fbm::new(seed.wrapping_add(2)).set_octaves(3).set_frequency(1.0 / 400.0),
            caves: Fbm::new(seed.wrapping_add(3)).set_octaves(2).set_frequency(1.0 / 36.0),
            columns: Mutex::new(HashMap::new()),
        }
    }

    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// Surface height (the y of the top solid block) of the column at horizontal (x, z, w).
    pub fn height(&self, x: i32, z: i32, w: i32) -> i32 {
        let p = [x as f64, z as f64, w as f64];
        let hills = self.hills.get(p);
        let detail = self.detail.get(p);
        let m = self.mountains.get(p);
        let mountain = if m > 0.15 { (m - 0.15) * 160.0 } else { 0.0 };
        (50.0 + hills * 22.0 + detail * 6.0 + mountain).round().clamp(4.0, WORLD_HEIGHT as f64 - 20.0) as i32
    }

    fn has_tree(&self, x: i32, z: i32, w: i32, height: i32) -> bool {
        height > SEA_LEVEL + 1
            && height < SNOW_LINE - 6
            && hash4(self.seed, IVec4::new(x, 0, z, w), 7) < 0.004
    }

    fn tree_height(&self, x: i32, z: i32, w: i32) -> i32 {
        4 + (hash4(self.seed, IVec4::new(x, 0, z, w), 8) * 3.0) as i32
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
                    for y in 0..CHUNK {
                        let wy = origin.y + y;
                        let l = IVec4::new(x, y, z, w);
                        let world = origin + l;
                        let b = if wy == 0 || (wy <= 2 && hash4(self.seed, world, 1) < 0.5) {
                            BEDROCK
                        } else if wy <= h {
                            let cave = wy > 3 && wy < h - 2 && cave_sample(&caves, l) > 0.42;
                            if cave {
                                AIR
                            } else if wy == h {
                                if h <= SEA_LEVEL + 1 {
                                    SAND
                                } else if h >= SNOW_LINE {
                                    SNOW
                                } else {
                                    GRASS
                                }
                            } else if wy > h - 4 {
                                if h <= SEA_LEVEL + 1 { SAND } else { DIRT }
                            } else if hash4(self.seed, world, 2) < 0.012 {
                                COAL_ORE
                            } else {
                                STONE
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

        self.place_trees(origin, heights, &mut blocks);
        Chunk::from_dense(blocks)
    }

    fn place_trees(
        &self,
        origin: IVec4,
        heights: &[i32],
        blocks: &mut [BlockId],
    ) {
        let r = TREE_REACH;
        for tw in -r..CHUNK + r {
            for tz in -r..CHUNK + r {
                for tx in -r..CHUNK + r {
                    let h = heights[col(tx, tz, tw)];
                    let (gx, gz, gw) = (origin.x + tx, origin.z + tz, origin.w + tw);
                    if !self.has_tree(gx, gz, gw, h) {
                        continue;
                    }
                    let th = self.tree_height(gx, gz, gw);
                    let top = h + th;
                    if top + 2 < origin.y || h + 1 >= origin.y + CHUNK + r {
                        continue;
                    }
                    // Leaves: a 4D ball around the top of the trunk.
                    for dw in -r..=r {
                        for dz in -r..=r {
                            for dx in -r..=r {
                                for dy in -2..=1 {
                                    let d2 = dx * dx + dz * dz + dw * dw + dy * dy;
                                    if d2 > 5 {
                                        continue;
                                    }
                                    let l = IVec4::new(tx + dx, top + dy - origin.y, tz + dz, tw + dw);
                                    if in_chunk(l) && blocks[local_index(l)] == AIR {
                                        blocks[local_index(l)] = LEAVES;
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
                        }
                    }
                    let below = IVec4::new(tx, h - origin.y, tz, tw);
                    if in_chunk(below) {
                        blocks[local_index(below)] = DIRT;
                    }
                }
            }
        }
    }

    /// Heights for the chunk column of `c`, with a margin so trees from neighbouring
    /// columns can reach in. Cached because all 16 chunks of a column need the same data.
    fn column(&self, c: IVec4) -> Arc<ColumnHeights> {
        let key = (c.x, c.z, c.w);
        if let Some(h) = self.columns.lock().unwrap().get(&key) {
            return h.clone();
        }
        let origin = c * CHUNK;
        let mut heights = vec![0i32; (SIDE * SIDE * SIDE) as usize];
        let mut max = i32::MIN;
        for w in -M..CHUNK + M {
            for z in -M..CHUNK + M {
                for x in -M..CHUNK + M {
                    let h = self.height(origin.x + x, origin.z + z, origin.w + w);
                    heights[col(x, z, w)] = h;
                    max = max.max(h);
                }
            }
        }
        let column = Arc::new(ColumnHeights { heights, max });
        let mut cache = self.columns.lock().unwrap();
        if cache.len() > 8192 {
            cache.clear();
        }
        cache.insert(key, column.clone());
        column
    }

    /// 4D cave noise sampled every 4 blocks over the chunk (5⁴ samples), interpolated later.
    fn cave_grid(&self, origin: IVec4) -> [f32; 625] {
        let mut g = [0.0f32; 625];
        for i in 0..625 {
            let (x, y, z, w) = (i % 5, (i / 5) % 5, (i / 25) % 5, i / 125);
            let p = origin + IVec4::new(x as i32, y as i32, z as i32, w as i32) * 4;
            g[i] = self.caves.get([p.x as f64, p.y as f64 * 1.6, p.z as f64, p.w as f64]) as f32;
        }
        g
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
        let o = IVec4::new(corner & 1, (corner >> 1) & 1, (corner >> 2) & 1, (corner >> 3) & 1);
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
    fn deterministic_and_layered() {
        let g = TerrainGen::new(42);
        let h = g.height(0, 0, 0);
        assert_eq!(h, TerrainGen::new(42).height(0, 0, 0));
        let c = g.generate(IVec4::new(0, h / CHUNK, 0, 0));
        assert!(c.uniform_id().is_none(), "surface chunk should be mixed");
        assert_eq!(g.generate(IVec4::new(0, 15, 0, 0)).uniform_id(), Some(AIR));
        assert_eq!(g.generate(IVec4::new(0, -1, 0, 0)).uniform_id(), Some(BEDROCK));
    }

    #[test]
    fn trees_cross_chunk_borders_consistently() {
        // Leaves near a chunk border are placed by both chunks from the same trunk, so
        // the blocks on each side must match a single generation of the 4D neighbourhood.
        let g = TerrainGen::new(5);
        let mut checked = 0;
        'search: for x in 0..400 {
            for w in 0..8 {
                let h = g.height(x * 16 + 15, 0, w);
                if g.has_tree(x * 16 + 15, 0, w, h) {
                    let top = h + g.tree_height(x * 16 + 15, 0, w);
                    let right = g.generate(IVec4::new(x + 1, top.div_euclid(CHUNK), 0, w.div_euclid(CHUNK)));
                    let l = IVec4::new(0, top.rem_euclid(CHUNK), 0, w.rem_euclid(CHUNK));
                    assert_eq!(right.get(l), LEAVES);
                    checked += 1;
                    break 'search;
                }
            }
        }
        assert_eq!(checked, 1, "no tree found at a chunk border");
    }
}
