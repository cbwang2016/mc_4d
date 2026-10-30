//! Bounded plant activation and random ticks. The geometric waiting time is
//! memoryless, so loaded plants can reconstruct their schedule without saving
//! millions of individual timers. Player edits receive immediate support checks.
use super::*;
use mc4d_core::{
    block::BlockState,
    chunk::CHUNK_VOLUME,
    farming::{self, PlantLight, TickAction},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::mpsc::Receiver,
};

pub(super) struct FarmingState {
    changes: Receiver<mc4d_core::world::BlockChange>,
    due: BTreeMap<u64, Vec<IVec4>>,
    scheduled: HashMap<IVec4, u64>,
    scans: VecDeque<(IVec4, u64, usize)>,
    chunk_events: Receiver<mc4d_core::world::ChunkLifecycle>,
    supports: VecDeque<IVec4>,
    support_set: HashSet<IVec4>,
    leaf_scans: VecDeque<(IVec4, i32, usize)>,
    tick: u64,
    fraction: f32,
}
impl FarmingState {
    pub fn new(world: &mut World) -> Self {
        Self {
            changes: world.subscribe_changes(),
            due: BTreeMap::new(),
            scheduled: HashMap::new(),
            scans: VecDeque::new(),
            chunk_events: world.subscribe_chunk_lifecycle(),
            supports: VecDeque::new(),
            support_set: HashSet::new(),
            leaf_scans: VecDeque::new(),
            tick: 0,
            fraction: 0.0,
        }
    }
    fn schedule(&mut self, p: IVec4, delay: u64) {
        if self.scheduled.contains_key(&p) {
            return;
        }
        let due = self.tick.saturating_add(delay);
        self.scheduled.insert(p, due);
        self.due.entry(due).or_default().push(p);
    }
    fn support(&mut self, p: IVec4) {
        if self.support_set.insert(p) {
            self.supports.push_back(p)
        }
    }
    fn pop_due(&mut self) -> Option<IVec4> {
        loop {
            let mut first = self.due.first_entry()?;
            if *first.key() > self.tick {
                return None;
            }
            let due = *first.key();
            let p = first.get_mut().pop().unwrap();
            if first.get().is_empty() {
                first.remove_entry();
            }
            if self.scheduled.get(&p) == Some(&due) {
                self.scheduled.remove(&p);
                return Some(p);
            }
        }
    }
}
fn plant(id: u8) -> bool {
    matches!(block::definition(id).shape, block::BlockShape::Plant)
        | matches!(id, block::CACTUS | block::REEDS | block::SNOW_LAYER)
}
impl Game {
    fn schedule_plant(&mut self, p: IVec4) {
        let delay = farming::random_tick_delay((self.rand() as f64).max(f64::EPSILON));
        self.farming.schedule(p, delay);
    }
    fn plant_light(&self, p: IVec4) -> Option<PlantLight> {
        let (block, sky) = self.world.fresh_light_at(p)?;
        let e = &self.environment;
        Some(PlantLight {
            level: mc4d_core::lighting::effective_level(
                block,
                sky,
                e.daylight(),
                e.rain,
                e.thundering,
                e.lightning,
            ),
            sees_sky: sky == 15,
        })
    }
    pub(super) fn update_farming(&mut self, dt: f32) {
        self.farming.fraction += dt * 20.0;
        let ticks = self.farming.fraction.floor() as u64;
        self.farming.fraction -= ticks as f32;
        self.farming.tick = self.farming.tick.saturating_add(ticks);
        // Activation is bounded independently of world size. Block changes
        // continue through their existing queues and keep original timer debt.
        for _ in 0..256 {
            let Ok(event) = self.farming.chunk_events.try_recv() else {
                break;
            };
            if !event.loaded {
                continue;
            }
            let c = event.coordinate;
            let Some(ch) = self
                .world
                .chunks
                .get(&c)
                .filter(|ch| ch.identity == event.identity)
            else {
                continue;
            };
            if ch.may_contain_any(farming::CRITICAL_BLOCKS)
                || ch.may_contain_any(mc4d_core::ecology::ACTIVE_BLOCKS)
                || (ch.may_contain(block::LEAVES) && ch.may_have_metadata_bits(8))
            {
                self.farming.scans.push_back((c, event.identity, 0));
            }
        }
        for _ in 0..128 {
            let Ok(change) = self.farming.changes.try_recv() else {
                break;
            };
            self.register_ecology(change.position, change.after);
            if farming::needs_tick(change.after) {
                self.schedule_plant(change.position);
            }
            for d in [
                IVec4::ZERO,
                IVec4::Y,
                IVec4::X,
                IVec4::NEG_X,
                IVec4::Z,
                IVec4::NEG_Z,
                IVec4::W,
                IVec4::NEG_W,
            ] {
                self.farming.support(change.position + d);
            }
            if change.before.id == block::LOG && change.after.id != block::LOG {
                self.farming.leaf_scans.push_back((change.position, 4, 0));
            }
            if change.before.id == block::LEAVES && change.after.id != block::LEAVES {
                self.farming.leaf_scans.push_back((change.position, 1, 0));
            }
            if !block::is_fluid(change.before.id)
                && change.before.id != AIR
                && block::is_water(change.after.id)
            {
                self.spawn_block_loot(
                    change.position,
                    change.before,
                    mc4d_core::loot::DropCause::Natural,
                );
            }
        }
        // Initial loaded-chunk scans are incremental; ID masks skip empty/stone chunks.
        for _ in 0..2048 {
            let Some((c, identity, index)) = self.farming.scans.front().copied() else {
                break;
            };
            let Some(ch) = self
                .world
                .chunks
                .get(&c)
                .filter(|ch| ch.identity == identity)
            else {
                self.farming.scans.pop_front();
                continue;
            };
            let local = IVec4::new(
                (index & 15) as i32,
                ((index >> 4) & 15) as i32,
                ((index >> 8) & 15) as i32,
                ((index >> 12) & 15) as i32,
            );
            let state = ch.get_state(local);
            let p = c * 16 + local;
            if index + 1 == CHUNK_VOLUME {
                self.farming.scans.pop_front();
            } else {
                self.farming.scans.front_mut().unwrap().2 += 1;
            }
            if super::ecology::eligible(state.id) || state.id == block::SNOW {
                self.register_ecology(p, state);
            }
            if farming::needs_tick(state) {
                self.schedule_plant(p);
            }
        }
        for _ in 0..256 {
            let Some((center, r, index)) = self.farming.leaf_scans.front().copied() else {
                break;
            };
            let side = (r * 2 + 1) as usize;
            let p = center
                + IVec4::new(
                    (index % side) as i32 - r,
                    ((index / side) % side) as i32 - r,
                    ((index / side / side) % side) as i32 - r,
                    ((index / side / side / side) % side) as i32 - r,
                );
            if index + 1 == side.pow(4) {
                self.farming.leaf_scans.pop_front();
            } else {
                self.farming.leaf_scans.front_mut().unwrap().2 += 1;
            }
            if let Some(s) = self.world.block_state(p)
                && s.id == block::LEAVES
                && s.metadata & 8 == 0
            {
                self.world
                    .set_block_state(p, BlockState::new(s.id, s.metadata | 8));
            }
        }
        for _ in 0..64 {
            let Some(p) = self.farming.supports.pop_front() else {
                break;
            };
            self.farming.support_set.remove(&p);
            let Some(state) = self.world.block_state(p) else {
                continue;
            };
            if plant(state.id)
                && farming::supported(p, state, &|p| self.world.block_state(p)) == Some(false)
            {
                self.world.set_block(p, AIR);
                self.spawn_block_loot(p, state, mc4d_core::loot::DropCause::Natural);
            }
            if state.id == block::FARMLAND
                && self.world.block(p + IVec4::Y).is_some_and(block::is_solid)
            {
                self.world.set_block(p, block::DIRT);
            }
        }
        for _ in 0..32 {
            let Some(p) = self.farming.pop_due() else {
                break;
            };
            let Some(state) = self.world.block_state(p) else {
                continue;
            };
            if !farming::needs_tick(state) {
                continue;
            }
            let mut random = self.rng;
            let action = farming::tick(
                p,
                state,
                &|p| self.world.block_state(p),
                &|p| self.plant_light(p),
                self.environment.raining,
                &mut |bound| {
                    random ^= random << 13;
                    random ^= random >> 7;
                    random ^= random << 17;
                    ((random >> 32) as u32) % bound
                },
            );
            self.rng = random;
            match action {
                TickAction::Stay => {}
                TickAction::Defer => {
                    self.farming.schedule(p, 20);
                    continue;
                }
                TickAction::Replace(s) => {
                    self.world.set_block_state(p, s);
                }
                TickAction::Break => {
                    self.world.set_block(p, AIR);
                    self.spawn_block_loot(p, state, mc4d_core::loot::DropCause::Natural);
                }
                TickAction::GrowColumn => {
                    self.world
                        .set_block_state(p + IVec4::Y, BlockState::new(state.id, 0));
                    self.world.set_block_state(p, BlockState::new(state.id, 0));
                }
                TickAction::GrowTree(species) => {
                    self.grow_sapling(p, species);
                }
                TickAction::Spread(targets) => {
                    for p in targets.into_iter().flatten() {
                        self.world.set_block(p, block::GRASS);
                    }
                }
            }
            if self.world.block_state(p).is_some_and(farming::needs_tick) {
                self.schedule_plant(p);
            }
        }
    }
    /// Preflight all tree cells before changing anything, including at chunk edges.
    pub(super) fn grow_sapling(&mut self, pos: IVec4, species: u8) -> bool {
        if !matches!(
            self.world.block(pos - IVec4::Y),
            Some(block::DIRT | block::GRASS | block::FARMLAND)
        ) {
            return false;
        }
        let height = if species == 1 { 6 } else { 4 } + (self.rand() * 3.0) as i32;
        let mut leaves = Vec::new();
        for y in (if species == 1 { 2 } else { height - 3 })..=height {
            let radius = if species == 1 {
                ((height - y + 1) / 2).min(2)
            } else if y == height {
                1
            } else {
                2
            };
            for w in -radius..=radius {
                for z in -radius..=radius {
                    for x in -radius..=radius {
                        if x.abs() == radius && z.abs() == radius && w.abs() == radius && radius > 0
                        {
                            continue;
                        }
                        leaves.push(pos + IVec4::new(x, y, z, w));
                    }
                }
            }
        }
        let trunk: Vec<_> = (0..height).map(|y| pos + IVec4::Y * y).collect();
        if trunk.iter().chain(leaves.iter()).any(|&p| {
            self.world
                .block(p)
                .is_none_or(|id| !matches!(id, AIR | block::LEAVES | block::SAPLING))
        }) {
            return false;
        }
        self.world.set_block(pos - IVec4::Y, block::DIRT);
        for p in leaves {
            if self.world.block(p) == Some(AIR) {
                self.world
                    .set_block_state(p, BlockState::new(block::LEAVES, species));
            }
        }
        for p in trunk {
            self.world
                .set_block_state(p, BlockState::new(block::LOG, species));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc4d_core::chunk::Chunk;
    fn garden() -> (Game, IVec4) {
        let mut game = Game::menu_preview(7);
        game.world.insert_chunk(IVec4::ZERO, Chunk::uniform(AIR));
        game.body.pos = Vec4::splat(8.0);
        (game, IVec4::splat(8))
    }
    #[test]
    #[ignore = "paced CPU diagnostic: records real opacity/emission-changing simulation edits during load/move"]
    fn moving_world_light_invalidation_sources() {
        let mut game = Game::new(None, 1337, GameMode::Creative);
        game.body.pos.y += 6.0;
        game.flying = true;
        let settings = Settings::default();
        let bindings = Bindings::default();
        let warm = std::time::Instant::now();
        while warm.elapsed().as_secs_f32() < 30.0 {
            game.update_streaming(&settings, false);
            if game.gpu_wanted.iter().all(|&c| game.world.is_loaded(c))
                && game.world.lighting_ready(&game.gpu_wanted)
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let warm_seconds = warm.elapsed().as_secs_f64();
        let events = game.world.subscribe_changes();
        let lifecycle = game.world.subscribe_chunk_lifecycle();
        for _ in lifecycle.try_iter() {}
        let mut phases = Vec::new();
        for (phase, frames) in [
            ("idle", 120),
            ("w_flight", 180),
            ("rotation", 180),
            ("settling", 240),
        ] {
            let mut counts = std::collections::BTreeMap::<(u8, u8, bool), u64>::new();
            let mut loaded = 0;
            let mut unloaded = 0;
            let mut worst = 0.0_f64;
            for _ in 0..frames {
                let start = std::time::Instant::now();
                let mut input = InputState::default();
                if phase == "w_flight" {
                    input.press(bindings.get(Action::MoveAna)[0]);
                    input.press(bindings.get(Action::Sprint)[0]);
                }
                if phase == "rotation" {
                    game.orient.rotate_forward_ana(0.45 / 60.0);
                    game.orient.rotate_right_ana(0.23 / 60.0);
                }
                game.update(1.0 / 60.0, Some((&input, &bindings)), &settings);
                worst = worst.max(start.elapsed().as_secs_f64() * 1000.0);
                for event in events.try_iter() {
                    let optical = block::light_opacity(event.before)
                        != block::light_opacity(event.after)
                        || block::light_emission(event.before)
                            != block::light_emission(event.after);
                    *counts
                        .entry((event.before.id, event.after.id, optical))
                        .or_default() += 1;
                }
                for event in lifecycle.try_iter() {
                    if event.loaded {
                        loaded += 1;
                    } else {
                        unloaded += 1;
                    }
                }
                let remaining =
                    std::time::Duration::from_secs_f64(1.0 / 60.0).saturating_sub(start.elapsed());
                std::thread::sleep(remaining);
            }
            let edits: Vec<_> = counts
                .into_iter()
                .map(|((before, after, light), count)| {
                    format!("{before}-> {after}, changes_light={light}: {count}")
                })
                .collect();
            phases.push(format!("{phase}: new_chunks={loaded}, evicted_chunks={unloaded}, worst_cpu_update_ms={worst:.3}, pending_light={}, unsettled_visible={}\n{}",game.world.lighting_pending(),game.world.missing_lighting(&game.gpu_wanted),edits.join("\n")));
        }
        let report = format!(
            "CPU-only paced diagnostic; no GPU/frame-rate certification.\nWarm-up seconds: {:.3}\n{}\n",
            warm_seconds,
            phases.join("\n\n")
        );
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/performance");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("subsystem-light-audit.txt"), &report).unwrap();
        println!("{report}");
    }

    #[test]
    fn loaded_chunk_activation_is_bounded_and_does_not_rescan_idle_worlds() {
        let mut game = Game::menu_preview(7);
        for x in 0..1024 {
            game.world
                .insert_chunk(IVec4::new(x, 0, 0, 0), Chunk::uniform(AIR));
        }
        game.update_farming(0.0);
        assert_eq!(
            game.farming.chunk_events.try_iter().count(),
            768,
            "one update consumes only its256 lifecycle-event budget"
        );
        assert!(game.farming.scans.is_empty());
        game.update_farming(2.0);
        assert!(game.farming.chunk_events.try_recv().is_err());
        assert!(
            game.farming.scans.is_empty(),
            "elapsed time does not rescan the world map"
        );
    }

    #[test]
    fn scheduled_column_growth_runs_in_game_and_stops_at_three() {
        let (mut game, p) = garden();
        game.world.set_block(p - IVec4::Y, block::SAND);
        game.world
            .set_block_state(p, BlockState::new(block::CACTUS, 15));
        game.update_farming(0.0);
        game.farming.due.clear();
        game.farming.scheduled.clear();
        game.farming.schedule(p, 0);
        game.update_farming(0.0);
        assert_eq!(game.world.block(p + IVec4::Y), Some(block::CACTUS));
        assert_eq!(game.world.block_state(p).unwrap().metadata, 0);
        game.world.set_block(p + IVec4::Y * 2, block::CACTUS);
        game.world
            .set_block_state(p + IVec4::Y * 2, BlockState::new(block::CACTUS, 15));
        game.farming.due.clear();
        game.farming.scheduled.clear();
        game.farming.schedule(p + IVec4::Y * 2, 0);
        game.update_farming(0.0);
        assert_eq!(game.world.block(p + IVec4::Y * 3), Some(AIR));
    }
    #[test]
    fn support_changes_break_plants_and_water_displacement_produces_physical_loot() {
        let (mut game, p) = garden();
        game.world.set_block(p - IVec4::Y, block::DIRT);
        game.world.set_block(p, block::DANDELION);
        game.update_farming(0.0);
        game.world.set_block(p - IVec4::Y, AIR);
        game.update_farming(0.0);
        assert_eq!(game.world.block(p), Some(AIR));
        assert_eq!(game.dropped_items.len(), 1);
        game.dropped_items.clear();
        game.world.set_block(p - IVec4::Y, block::DIRT);
        game.world.set_block(p, block::TORCH);
        game.world.set_block(p, block::WATER);
        game.update_farming(0.0);
        assert_eq!(game.dropped_items.len(), 1);
        assert_eq!(
            game.dropped_items[0].stack.block,
            item::ItemId::from(block::TORCH)
        );
    }
    #[test]
    fn tree_growth_preflight_preserves_obstructions_and_species() {
        let (mut game, p) = garden();
        game.world.set_block(p - IVec4::Y, block::DIRT);
        game.world
            .set_block_state(p, BlockState::new(block::SAPLING, 2));
        game.world.set_block(p + IVec4::Y, block::STONE);
        assert!(!game.grow_sapling(p, 2));
        assert_eq!(game.world.block(p), Some(block::SAPLING));
        assert_eq!(game.world.block(p + IVec4::Y), Some(block::STONE));
        game.world.set_block(p + IVec4::Y, AIR);
        assert!(game.grow_sapling(p, 2));
        assert_eq!(
            game.world.block_state(p),
            Some(BlockState::new(block::LOG, 2))
        );
        assert!(game.world.chunks[&IVec4::ZERO].may_contain(block::LEAVES));
    }
    #[test]
    fn missing_chunks_do_not_destroy_edge_cactus() {
        let (mut game, p) = garden();
        let p = IVec4::new(15, p.y, p.z, p.w);
        game.world.set_block(p - IVec4::Y, block::SAND);
        game.world.set_block(p, block::CACTUS);
        game.update_farming(0.0);
        assert_eq!(game.world.block(p), Some(block::CACTUS));
    }
}
