//! Scheduled fire, player/mob burning and weather extinguishing. Nether portal
//! creation is deliberately absent; ordinary netherrack still supports fire.
use super::*;
use mc4d_core::{
    block::BlockState,
    chunk::{CHUNK_VOLUME, WORLD_HEIGHT},
    fire as rules,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, VecDeque},
    sync::mpsc::Receiver,
};
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct SavedFire {
    pending: Vec<(IVec4, u64)>,
    burn_ticks: u16,
}
pub(super) struct FireState {
    changes: Receiver<mc4d_core::world::BlockChange>,
    timers: BTreeMap<u64, Vec<IVec4>>,
    scheduled: HashMap<IVec4, u64>,
    chunk_events: Receiver<mc4d_core::world::ChunkLifecycle>,
    scans: VecDeque<(IVec4, u64, usize)>,
    tick: u64,
    fraction: f32,
    burn_ticks: u16,
    rain_columns: RefCell<HashMap<[i32; 3], i32>>,
}
impl FireState {
    pub fn new(world: &mut World) -> Self {
        Self {
            changes: world.subscribe_changes(),
            timers: BTreeMap::new(),
            scheduled: HashMap::new(),
            chunk_events: world.subscribe_chunk_lifecycle(),
            scans: VecDeque::new(),
            tick: 0,
            fraction: 0.0,
            burn_ticks: 0,
            rain_columns: RefCell::new(HashMap::new()),
        }
    }
    fn schedule(&mut self, p: IVec4, delay: u64) {
        let due = self.tick.saturating_add(delay);
        if self.scheduled.get(&p).is_some_and(|&old| old <= due) {
            return;
        }
        self.scheduled.insert(p, due);
        self.timers.entry(due).or_default().push(p);
    }
    fn next(&mut self) -> Option<IVec4> {
        loop {
            let mut first = self.timers.first_entry()?;
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
    pub fn snapshot(&self) -> SavedFire {
        SavedFire {
            pending: self
                .scheduled
                .iter()
                .map(|(&p, &due)| (p, due.saturating_sub(self.tick)))
                .collect(),
            burn_ticks: self.burn_ticks,
        }
    }
    pub fn restore(&mut self, s: SavedFire) {
        self.burn_ticks = s.burn_ticks.min(600);
        for (p, delay) in s.pending {
            self.schedule(p, delay.min(10000));
        }
    }
}
impl rules::FireWorld for Game {
    fn get(&self, p: IVec4) -> Option<BlockState> {
        self.world.block_state(p)
    }
    fn set(&mut self, p: IVec4, s: BlockState) {
        self.world.set_block_state(p, s);
    }
    fn wet(&self, p: IVec4) -> bool {
        self.rain_at(p)
    }
    fn prime_tnt(&mut self, p: IVec4) {
        self.prime_tnt(p, false);
    }
}
impl Game {
    pub(super) fn ignite_player(&mut self) {
        self.fire.burn_ticks = if self.fire.burn_ticks == 0 {
            300
        } else {
            self.fire.burn_ticks.saturating_add(1)
        };
    }
    pub(super) fn reset_burning(&mut self) {
        self.fire.burn_ticks = 0;
    }
    pub fn burning(&self) -> bool {
        self.fire.burn_ticks > 0
    }
    pub(super) fn extinguish(&mut self) {
        if self.fire.burn_ticks > 0 {
            self.emit_sound("random.fizz", self.body.pos, 0.7, 1.6);
        }
        self.fire.burn_ticks = 0;
    }
    fn rain_at(&self, p: IVec4) -> bool {
        if !self.environment.raining {
            return false;
        }
        if matches!(
            self.world.terrain.biome(p.x, p.z, p.w),
            mc4d_core::terrain::Biome::Desert | mc4d_core::terrain::Biome::Taiga
        ) {
            return false;
        }
        if let Some((_, sky)) = self.world.fresh_light_at(p) {
            return sky == 15;
        }
        let key = [p.x, p.z, p.w];
        if let Some(&top) = self.fire.rain_columns.borrow().get(&key) {
            return p.y >= top;
        }
        let generated_top = self.world.terrain.height(p.x, p.z, p.w) + 10;
        let mut top = 0;
        for y in (0..WORLD_HEIGHT).rev() {
            match self.world.block_state(IVec4::new(p.x, y, p.z, p.w)) {
                Some(s) if block::light_opacity(s) > 0 => {
                    top = y + 1;
                    break;
                }
                None if y <= generated_top => {
                    top = generated_top + 1;
                    break;
                }
                _ => {}
            }
        }
        self.fire.rain_columns.borrow_mut().insert(key, top);
        p.y >= top
    }
    pub(super) fn ignite(&mut self, p: IVec4) -> bool {
        if self.world.block(p) != Some(AIR)
            || rules::supported(p, &|q| self.world.block_state(q)) != Some(true)
        {
            return false;
        }
        if self.world.set_block(p, block::FIRE) {
            self.fire.schedule(p, 10);
            true
        } else {
            false
        }
    }
    fn fire_contact(&self, body: &Body) -> (bool, bool) {
        let bounds = body.bounds();
        let lo = (bounds.min + Vec4::splat(0.001)).floor().as_ivec4();
        let hi = (bounds.max - Vec4::splat(0.001)).floor().as_ivec4();
        let (mut fire, mut lava) = (false, false);
        for w in lo.w..=hi.w {
            for z in lo.z..=hi.z {
                for y in lo.y..=hi.y {
                    for x in lo.x..=hi.x {
                        let p = IVec4::new(x, y, z, w);
                        let id = self.world.block(p).unwrap_or(AIR);
                        fire |= id == block::FIRE;
                        lava |= block::fluid(id)
                            .is_some_and(|(kind, _)| kind == block::FluidKind::Lava)
                            && bounds.min.y
                                < (y as f32
                                    + block::fluid_height(id, self.world.block(p + IVec4::Y)));
                    }
                }
            }
        }
        (fire, lava)
    }
    pub(super) fn update_fire(&mut self, dt: f32) {
        self.fire.rain_columns.borrow_mut().clear();
        self.fire.fraction += dt * 20.0;
        let ticks = self.fire.fraction.floor() as u64;
        self.fire.fraction -= ticks as f32;
        self.fire.tick = self.fire.tick.saturating_add(ticks);
        // Activation is bounded independently of world size. Block changes
        // continue through their existing queues and keep original timer debt.
        for _ in 0..256 {
            let Ok(event) = self.fire.chunk_events.try_recv() else {
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
            if ch.may_contain(block::FIRE) {
                self.fire.scans.push_back((c, event.identity, 0));
            }
        }
        for _ in 0..1024 {
            let Ok(change) = self.fire.changes.try_recv() else {
                break;
            };
            if change.after.id != block::FIRE {
                self.fire.scheduled.remove(&change.position);
            }
            for d in std::iter::once(IVec4::ZERO).chain(mc4d_core::redstone::NEIGHBORS) {
                let p = change.position + d;
                if self.world.block(p) != Some(block::FIRE) {
                    continue;
                }
                if rules::supported(p, &|q| self.world.block_state(q)) == Some(false) {
                    self.world.set_block(p, AIR);
                } else {
                    self.fire.schedule(p, 10);
                }
            }
        }
        for _ in 0..512 {
            let Some((c, identity, index)) = self.fire.scans.front().copied() else {
                break;
            };
            let Some(ch) = self
                .world
                .chunks
                .get(&c)
                .filter(|ch| ch.identity == identity)
            else {
                self.fire.scans.pop_front();
                continue;
            };
            let local = IVec4::new(
                (index & 15) as i32,
                ((index >> 4) & 15) as i32,
                ((index >> 8) & 15) as i32,
                ((index >> 12) & 15) as i32,
            );
            let state = ch.get_state(local);
            if index + 1 == CHUNK_VOLUME {
                self.fire.scans.pop_front();
            } else {
                self.fire.scans.front_mut().unwrap().2 += 1;
            }
            if state.id == block::FIRE {
                self.fire.schedule(c * 16 + local, 10);
            }
        }
        for _ in 0..64 {
            let Some(p) = self.fire.next() else { break };
            if self.world.block(p).is_none() {
                self.fire.schedule(p, 20);
                continue;
            }
            if self.world.block(p) != Some(block::FIRE) {
                continue;
            }
            let mut rng = self.rng;
            let mut roll = |n: u32| {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                ((rng >> 32) as u32) % n
            };
            let scheduled = rules::tick(p, self, &mut roll);
            self.rng = rng;
            if self.world.block(p) == Some(block::FIRE) {
                let random =
                    mc4d_core::farming::random_tick_delay((self.rand() as f64).max(f64::EPSILON));
                self.fire
                    .schedule(p, if scheduled { 10.min(random) } else { random });
            }
        }
        for offset in 0..ticks {
            let tick = self.fire.tick - ticks + offset + 1;
            if self.mode == GameMode::Creative
                || self.body.in_water
                || self.rain_at(self.body.pos.floor().as_ivec4())
            {
                self.extinguish();
            } else {
                let (contact, lava) = self.fire_contact(&self.body);
                if contact || lava {
                    self.fire.burn_ticks = 300;
                }
                if self.fire.burn_ticks > 0 {
                    if tick % 20 == 0 || contact && tick % 10 == 0 {
                        self.damage(1, "Fire");
                    }
                    self.fire.burn_ticks -= 1;
                }
            }
            for index in 0..self.creatures.len() {
                let wet = self.creatures[index].body.in_water
                    || self.rain_at(self.creatures[index].body.pos.floor().as_ivec4());
                let (contact, lava) = self.fire_contact(&self.creatures[index].body);
                let c = &mut self.creatures[index];
                if wet {
                    c.burn_ticks = 0;
                } else {
                    if contact || lava {
                        c.burn_ticks = 300;
                    }
                    if c.burn_ticks > 0 {
                        if tick % 20 == 0 || (contact || lava) && tick % 10 == 0 {
                            c.health -= if lava { 4 } else { 1 };
                            c.hurt = 0.3;
                        }
                        c.burn_ticks -= 1;
                    }
                }
            }
        }
        if ticks > 0 {
            let mut items = std::mem::take(&mut self.dropped_items);
            items.retain_mut(|d| {
                let (fire, lava) = self.fire_contact(&d.body);
                for offset in 0..ticks {
                    let tick = self.fire.tick - ticks + offset + 1;
                    if d.body.in_water {
                        d.burn_ticks = 0;
                    } else {
                        if fire || lava {
                            d.burn_ticks = 300;
                        }
                        let damage = if lava {
                            5
                        } else if fire || d.burn_ticks > 0 && tick % 20 == 0 {
                            1
                        } else {
                            0
                        };
                        d.health = d.health.saturating_sub(damage);
                        d.burn_ticks = d.burn_ticks.saturating_sub(1);
                    }
                }
                d.health > 0
            });
            self.dropped_items = items;
            self.collect_dead_creatures();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc4d_core::chunk::Chunk;
    fn arena() -> (Game, IVec4) {
        let mut g = Game::menu_preview(7);
        let cells = (0..CHUNK_VOLUME)
            .map(|i| {
                if (i >> 4) & 15 == 3 {
                    block::STONE
                } else {
                    AIR
                }
            })
            .collect();
        g.world.insert_chunk(IVec4::ZERO, Chunk::from_dense(cells));
        g.body.pos = Vec4::new(2.0, 4.0, 2.0, 2.0);
        (g, IVec4::new(8, 4, 8, 8))
    }
    #[test]
    fn flint_ignites_with_wear_and_original_scheduled_fire_ages() {
        let (mut g, p) = arena();
        g.hotbar[0] = Some(Slot::new(item::FLINT_AND_STEEL, 1));
        g.target = Some(Target::Block(Hit {
            block: p - IVec4::Y,
            normal: IVec4::Y,
            dist: 3.0,
        }));
        assert!(g.use_selected_tool());
        assert_eq!(g.world.block(p), Some(block::FIRE));
        assert_eq!(g.hotbar[0].unwrap().damage, 1);
        for _ in 0..10 {
            g.update_fire(0.05);
        }
        assert!(g.world.block_state(p).unwrap().metadata >= 1);
    }
    #[test]
    fn burning_state_persists_and_water_extinguishes_player_and_mobs() {
        let (mut g, p) = arena();
        g.world.set_block(p, block::FIRE);
        g.body.pos = p.as_vec4() + Vec4::new(0.5, 0.01, 0.5, 0.5);
        g.spawn_creature(CreatureKind::Pig, g.body.pos);
        for _ in 0..10 {
            g.update_fire(0.05);
        }
        assert!(g.burning());
        assert!(g.health < 20);
        assert!(g.creatures[0].burn_ticks > 0);
        let restored: SavedFire =
            toml::from_str(&toml::to_string(&g.fire.snapshot()).unwrap()).unwrap();
        assert!(restored.burn_ticks > 0);
        g.body.in_water = true;
        g.creatures[0].body.in_water = true;
        g.update_fire(0.05);
        assert!(!g.burning());
        assert_eq!(g.creatures[0].burn_ticks, 0);
    }
    #[test]
    fn old_fire_burn_stage_primes_tnt_in_w_and_item_heat_respects_lava_height() {
        let (mut g, p) = arena();
        g.world.set_block(p - IVec4::Y, block::NETHERRACK);
        g.world.set_block_state(p, BlockState::new(block::FIRE, 15));
        g.world.set_block(p + IVec4::W, block::TNT);
        g.fire.schedule(p, 0);
        g.update_fire(0.0);
        assert_ne!(g.world.block(p + IVec4::W), Some(block::TNT));
        assert_eq!(g.device_draws().iter().filter(|e| e.kind == 17).count(), 1);
        let lava = p + IVec4::X * 3;
        g.world.set_block(lava, block::FluidKind::Lava.flowing(7));
        g.spawn_drop(
            Slot::new(block::DIRT, 1),
            lava.as_vec4() + Vec4::new(0.5, 0.8, 0.5, 0.5),
            Vec4::ZERO,
            1.0,
        );
        g.update_fire(0.05);
        assert_eq!(
            g.dropped_items.len(),
            1,
            "an item above shallow lava must not burn"
        );
        g.dropped_items[0].body.pos.y = lava.y as f32 + 0.01;
        g.update_fire(0.05);
        assert!(g.dropped_items.is_empty());
    }
    #[test]
    fn respawning_does_not_carry_the_dead_players_burning_state() {
        let mut game = Game::menu_preview(7);
        game.ignite_player();
        assert!(game.burning());
        game.health = 0;
        game.respawn();
        assert!(!game.burning());
        assert_eq!(game.health, 20);
    }
    #[test]
    fn many_simultaneous_fires_keep_the_original_ten_tick_pace() {
        let (mut g, _) = arena();
        g.environment.raining = false;
        let mut fires = Vec::new();
        for x in 0..10 {
            for w in 0..10 {
                let p = IVec4::new(x, 4, 12, w);
                g.world.set_block(p, block::FIRE);
                fires.push(p);
            }
        }
        // Unfuelled fire on stone ages at10-tick intervals and is out by age4.
        for _ in 0..80 {
            g.update_fire(0.05);
        }
        assert!(fires.iter().all(|&p| g.world.block(p) == Some(AIR)));
    }
}
