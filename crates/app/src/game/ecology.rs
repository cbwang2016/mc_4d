//! Sparse ambient block ticks and walked-on soil. Schedules are reconstructed on
//! load, as random ticks have no per-block saved timer in the original game.
use super::*;
use mc4d_core::{
    block::BlockState,
    chunk::{chunk_of, local_index, local_of},
    ecology as rules,
    farming::{self, PlantLight, TickAction},
};
use std::collections::{HashMap, VecDeque};
struct ActiveChunk {
    identity: u64,
    cells: Vec<u16>,
    last_time: f64,
    credit: f64,
}
#[derive(Default)]
pub(super) struct Ecology {
    active: HashMap<IVec4, ActiveChunk>,
    queue: VecDeque<IVec4>,
    weather: HashMap<[i32; 3], (f64, f64)>,
    pub(super) weather_queue: VecDeque<[i32; 3]>,
    time: f64,
    walked: f32,
}
pub(super) fn eligible(id: u8) -> bool {
    matches!(
        id,
        block::GRASS | block::DANDELION | block::ROSE | block::BROWN_MUSHROOM | block::RED_MUSHROOM
    ) || id == block::SNOW_LAYER
        || block::fluid(id).is_some_and(|(k, _)| k == block::FluidKind::Lava)
}
impl Game {
    pub(super) fn register_ecology(&mut self, p: IVec4, state: BlockState) {
        let c = chunk_of(p);
        let Some(chunk) = self.world.chunks.get(&c) else {
            return;
        };
        let new = !self.ecology.active.contains_key(&c);
        let column = [c.x, c.z, c.w];
        if (eligible(state.id) || state.id == block::SNOW)
            && !self.ecology.weather.contains_key(&column)
        {
            self.ecology
                .weather
                .insert(column, (self.ecology.time, 0.0));
            self.ecology.weather_queue.push_back(column);
        }
        if !self.ecology.active.contains_key(&c) && !eligible(state.id) {
            return;
        }
        if !self
            .ecology
            .active
            .get(&c)
            .is_some_and(|a| a.identity == chunk.identity)
        {
            self.ecology.active.insert(
                c,
                ActiveChunk {
                    identity: chunk.identity,
                    cells: Vec::new(),
                    last_time: self.ecology.time,
                    credit: 0.0,
                },
            );
            if new {
                self.ecology.queue.push_back(c);
            }
        }
        let index = local_index(local_of(p)) as u16;
        let active = self.ecology.active.get_mut(&c).unwrap();
        if eligible(state.id) {
            if !active.cells.contains(&index) {
                active.cells.push(index);
            }
        } else if let Some(index) = active.cells.iter().position(|&i| i == index) {
            active.cells.swap_remove(index);
        }
    }
    pub(super) fn ecological_step(&mut self, previous: Vec4, sneaking: bool) {
        if self.flying || self.vehicles.saved.riding.is_some() || (sneaking && self.body.on_ground)
        {
            return;
        }
        let delta = self.body.pos - previous;
        let distance = Vec4::new(delta.x, 0.0, delta.z, delta.w).length();
        if distance > 3.0 {
            return;
        }
        self.ecology.walked += distance * 0.6;
        let p = (self.body.pos - Vec4::Y * 0.2).floor().as_ivec4();
        if self.ecology.walked > 1.0 && self.world.block(p).is_some_and(|id| id != AIR) {
            // rj.b accumulates horizontal distance even in air, then advances
            // its walked-on threshold by one when a block is actually below.
            self.ecology.walked -= 1.0;
            if self.world.block(p) == Some(block::FARMLAND) && self.rand() < 0.25 {
                self.world.set_block(p, block::DIRT);
            }
        }
    }
    pub(super) fn update_ecology(&mut self, dt: f32) {
        self.ecology.time += dt.max(0.0) as f64;
        self.update_snow_columns();
        let mut budget = 4096;
        let rounds = self.ecology.queue.len();
        for _ in 0..rounds {
            if budget == 0 {
                break;
            }
            let Some(c) = self.ecology.queue.pop_front() else {
                break;
            };
            let valid = self.ecology.active.get(&c).is_some_and(|a| {
                self.world
                    .chunks
                    .get(&c)
                    .is_some_and(|ch| ch.identity == a.identity)
            });
            if !valid {
                self.ecology.active.remove(&c);
                continue;
            }
            self.ecology.queue.push_back(c);
            let a = self.ecology.active.get_mut(&c).unwrap();
            a.credit += (self.ecology.time - a.last_time)
                * 20.0
                * farming::RANDOM_TICK_PROBABILITY
                * a.cells.len() as f64;
            a.last_time = self.ecology.time;
            let count = (a.credit.floor() as usize).min(64).min(budget);
            a.credit -= count as f64;
            budget -= count;
            for _ in 0..count {
                let len = self.ecology.active[&c].cells.len();
                if len == 0 {
                    break;
                }
                let which = (self.rand() * len as f32) as usize % len;
                let i = self.ecology.active[&c].cells[which] as usize;
                let p = c * 16
                    + IVec4::new(
                        (i & 15) as i32,
                        ((i >> 4) & 15) as i32,
                        ((i >> 8) & 15) as i32,
                        ((i >> 12) & 15) as i32,
                    );
                self.ambient_block_tick(p);
            }
        }
    }
    fn update_snow_columns(&mut self) {
        let rounds = self.ecology.weather_queue.len();
        let mut budget = 256;
        for _ in 0..rounds {
            if budget == 0 {
                break;
            }
            let Some(c) = self.ecology.weather_queue.pop_front() else {
                break;
            };
            if !(0..mc4d_core::chunk::WORLD_CHUNKS_Y)
                .any(|y| self.world.is_loaded(IVec4::new(c[0], y, c[1], c[2])))
            {
                self.ecology.weather.remove(&c);
                continue;
            }
            self.ecology.weather_queue.push_back(c);
            let state = self.ecology.weather.get_mut(&c).unwrap();
            if self.environment.raining {
                state.1 += (self.ecology.time - state.0) * 20.0;
            }
            state.0 = self.ecology.time;
            let attempts = (state.1.floor() as usize).min(8).min(budget);
            state.1 -= attempts as f64;
            budget -= attempts;
            for _ in 0..attempts {
                let x = c[0] * 16 + (self.rand() * 16.0) as i32;
                let z = c[1] * 16 + (self.rand() * 16.0) as i32;
                let w = c[2] * 16 + (self.rand() * 16.0) as i32;
                let Some(surface) = self.ecology_surface(x, z, w) else {
                    continue;
                };
                // Match the precipitation field's existing alpine extension as
                // well as the original snowy-biome rule.
                if self.world.terrain.biome(x, z, w) == mc4d_core::terrain::Biome::Taiga
                    || surface.y + 1 >= 92
                {
                    self.snow_surface(surface);
                }
            }
        }
    }
    pub(super) fn ecology_surface(&self, x: i32, z: i32, w: i32) -> Option<IVec4> {
        for cy in (0..mc4d_core::chunk::WORLD_CHUNKS_Y).rev() {
            let chunk = self.world.chunks.get(&IVec4::new(
                x.div_euclid(16),
                cy,
                z.div_euclid(16),
                w.div_euclid(16),
            ))?;
            if chunk.uniform_id() == Some(AIR) {
                continue;
            }
            for y in (0..16).rev() {
                let id = chunk.get(IVec4::new(
                    x.rem_euclid(16),
                    y,
                    z.rem_euclid(16),
                    w.rem_euclid(16),
                ));
                if block::is_solid(id) || block::is_fluid(id) {
                    return Some(IVec4::new(x, cy * 16 + y, z, w));
                }
            }
        }
        None
    }
    #[cfg(test)]
    fn snow_column(&mut self, x: i32, z: i32, w: i32) {
        let Some(below) = self.ecology_surface(x, z, w) else {
            return;
        };
        self.snow_surface(below);
    }
    fn snow_surface(&mut self, below: IVec4) {
        let p = below + IVec4::Y;
        let Some((light, _)) = self.world.fresh_light_at(p) else {
            return;
        };
        if light >= 10 {
            return;
        }
        let Some(below_state) = self.world.block_state(below) else {
            return;
        };
        let Some(here) = self.world.block_state(p) else {
            return;
        };
        if rules::snow_can_form(here, below_state, light) {
            self.world.set_block(p, block::SNOW_LAYER);
        }
        if below_state.id == block::WATER {
            self.world.set_block(below, block::ICE);
        }
    }
    fn ambient_block_tick(&mut self, p: IVec4) {
        let Some(state) = self.world.block_state(p) else {
            return;
        };
        if !eligible(state.id) {
            self.register_ecology(p, state);
            return;
        }
        if state.id == block::GRASS {
            let e = &self.environment;
            let light = |p| {
                self.world.fresh_light_at(p).map(|(b, s)| PlantLight {
                    level: mc4d_core::lighting::effective_level(
                        b,
                        s,
                        e.daylight(),
                        e.rain,
                        e.thundering,
                        e.lightning,
                    ),
                    sees_sky: s == 15,
                })
            };
            let mut rng = self.rng;
            let action = farming::tick(
                p,
                state,
                &|p| self.world.block_state(p),
                &light,
                e.raining,
                &mut |n| {
                    rng ^= rng << 13;
                    rng ^= rng >> 7;
                    rng ^= rng << 17;
                    ((rng >> 32) as u32) % n
                },
            );
            self.rng = rng;
            match action {
                TickAction::Replace(s) => {
                    self.world.set_block_state(p, s);
                }
                TickAction::Spread(targets) => {
                    for p in targets.into_iter().flatten() {
                        self.world.set_block(p, block::GRASS);
                    }
                }
                _ => {}
            }
        } else if matches!(
            state.id,
            block::DANDELION | block::ROSE | block::BROWN_MUSHROOM | block::RED_MUSHROOM
        ) {
            if let Some((b, sky)) = self.world.fresh_light_at(p) {
                let e = &self.environment;
                let light = mc4d_core::lighting::effective_level(
                    b,
                    sky,
                    e.daylight(),
                    e.rain,
                    e.thundering,
                    e.lightning,
                );
                let mushroom = matches!(state.id, block::BROWN_MUSHROOM | block::RED_MUSHROOM);
                if if mushroom {
                    light > 13
                } else {
                    light < 8 && sky < 15
                } {
                    self.world.set_block(p, AIR);
                    self.spawn_block_loot(p, state, mc4d_core::loot::DropCause::Natural);
                }
            }
        } else if block::fluid(state.id).is_some_and(|(k, _)| k == block::FluidKind::Lava) {
            let mut rng = self.rng;
            let target = rules::lava_ignition(p, &|p| self.world.block_state(p), &mut |n| {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                ((rng >> 32) as u32) % n
            });
            self.rng = rng;
            if let Some(target) = target {
                self.ignite(target);
            }
        } else if state.id == block::SNOW_LAYER
            && self
                .world
                .fresh_light_at(p)
                .is_some_and(|(light, _)| light > 11)
        {
            self.world.set_block(p, AIR);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn walking_tramples_soil_but_sneaking_preserves_it() {
        let mut g = Game::menu_preview(7);
        g.world
            .insert_chunk(IVec4::ZERO, mc4d_core::chunk::Chunk::uniform(AIR));
        let p = IVec4::new(8, 3, 8, 8);
        g.world.set_block(p, block::FARMLAND);
        g.body.pos = Vec4::new(8.5, 4.0, 8.5, 8.5);
        g.body.on_ground = true;
        g.rng = 0;
        g.ecological_step(g.body.pos - Vec4::X * 2.0, true);
        assert_eq!(g.world.block(p), Some(block::FARMLAND));
        g.ecological_step(g.body.pos - Vec4::W * 2.0, false);
        assert_eq!(g.world.block(p), Some(block::DIRT));
    }
    #[test]
    fn ambient_activation_is_compact_and_does_not_duplicate_edited_cells() {
        let mut g = Game::menu_preview(7);
        g.world
            .insert_chunk(IVec4::ZERO, mc4d_core::chunk::Chunk::uniform(AIR));
        let p = IVec4::splat(8);
        g.world.set_block(p, block::GRASS);
        g.register_ecology(p, block::GRASS.into());
        g.register_ecology(p, block::GRASS.into());
        assert_eq!(g.ecology.active[&IVec4::ZERO].cells.len(), 1);
        g.world.set_block(p, AIR);
        g.ambient_block_tick(p);
        assert!(g.ecology.active[&IVec4::ZERO].cells.is_empty());
    }
    fn outdoor() -> Game {
        let mut g = Game::menu_preview(7);
        for y in 0..mc4d_core::chunk::WORLD_CHUNKS_Y {
            g.world.insert_chunk(
                IVec4::new(0, y, 0, 0),
                mc4d_core::chunk::Chunk::uniform(AIR),
            );
        }
        g
    }
    fn settle(g: &mut Game) {
        let wanted: Vec<_> = (0..mc4d_core::chunk::WORLD_CHUNKS_Y)
            .map(|y| IVec4::new(0, y, 0, 0))
            .collect();
        let start = std::time::Instant::now();
        while !g.world.lighting_ready(&wanted) {
            g.world.update_lighting(&wanted, 128);
            assert!(start.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    fn actual_grass_spreads_to_lit_dirt_and_dies_under_cover() {
        let mut g = outdoor();
        let p = IVec4::splat(8);
        let target = p + IVec4::new(-1, -3, -1, -1);
        g.world.set_block(p, block::GRASS);
        g.world.set_block(target, block::DIRT);
        settle(&mut g);
        g.rng = 0;
        g.ambient_block_tick(p);
        assert_eq!(g.world.block(target), Some(block::GRASS));
        g.world.set_block(p + IVec4::Y, block::STONE);
        settle(&mut g);
        g.ambient_block_tick(p);
        assert_eq!(g.world.block(p), Some(block::DIRT));
    }
    #[test]
    fn snow_and_freezing_change_real_blocks_without_building_modern_snow_stacks() {
        let mut g = outdoor();
        let p = IVec4::new(8, 3, 8, 8);
        g.world.set_block(p, block::GRASS);
        settle(&mut g);
        g.snow_column(8, 8, 8);
        assert_eq!(g.world.block(p + IVec4::Y), Some(block::SNOW_LAYER));
        g.snow_column(8, 8, 8);
        assert_eq!(g.world.block_state(p + IVec4::Y).unwrap().metadata, 0);
        let water = p + IVec4::X * 2;
        g.world.set_block(water, block::WATER);
        settle(&mut g);
        g.snow_column(10, 8, 8);
        assert_eq!(g.world.block(water), Some(block::ICE));
    }
    #[test]
    fn horizontal_jump_distance_reaches_soil_callback_on_landing() {
        let mut g = Game::menu_preview(7);
        g.world
            .insert_chunk(IVec4::ZERO, mc4d_core::chunk::Chunk::uniform(AIR));
        let p = IVec4::new(8, 3, 8, 8);
        g.world.set_block(p, block::FARMLAND);
        g.body.pos = Vec4::new(8.5, 7.0, 8.5, 8.5);
        g.rng = 0;
        g.ecological_step(g.body.pos - Vec4::W * 2.0, false);
        assert_eq!(g.world.block(p), Some(block::FARMLAND));
        g.body.pos.y = 4.0;
        g.body.on_ground = true;
        g.ecological_step(g.body.pos, false);
        assert_eq!(g.world.block(p), Some(block::DIRT));
    }
    #[test]
    fn beta_mushrooms_survive_light_thirteen_but_drop_above_it() {
        let mut g = outdoor();
        g.environment.world_ticks = 18000;
        let p = IVec4::splat(8);
        g.world.set_block(p - IVec4::Y, block::STONE);
        g.world.set_block(p, block::BROWN_MUSHROOM);
        g.world.set_block(p + IVec4::X, block::TORCH);
        settle(&mut g);
        assert_eq!(g.world.fresh_light_at(p).unwrap().0, 13);
        g.ambient_block_tick(p);
        assert_eq!(g.world.block(p), Some(block::BROWN_MUSHROOM));
        g.world.set_block(p + IVec4::X, block::GLOW);
        settle(&mut g);
        assert_eq!(g.world.fresh_light_at(p).unwrap().0, 14);
        g.ambient_block_tick(p);
        assert_eq!(g.world.block(p), Some(AIR));
        assert_eq!(g.dropped_items.len(), 1);
    }
}
