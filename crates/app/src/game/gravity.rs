//! Original three-tick sand/gravel activation and0.98-sized falling entities.
//! Scans, starts and physics are bounded; no synchronous terrain generation.
use super::*;
use mc4d_core::{block::BlockState, chunk::CHUNK_VOLUME};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::mpsc::Receiver,
};
#[derive(Clone, Serialize, Deserialize)]
struct Falling {
    position: Vec4,
    velocity: Vec4,
    state: BlockState,
    ticks: u16,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct SavedGravity {
    entities: Vec<Falling>,
    pending: Vec<(IVec4, u64)>,
}
pub(super) struct GravityState {
    changes: Receiver<mc4d_core::world::BlockChange>,
    entities: Vec<Falling>,
    timers: BTreeMap<u64, Vec<IVec4>>,
    scheduled: HashMap<IVec4, u64>,
    scans: VecDeque<(IVec4, u64, usize)>,
    chunk_events: Receiver<mc4d_core::world::ChunkLifecycle>,
    tick: u64,
    fraction: f32,
}
fn falls(id: u8) -> bool {
    matches!(id, block::SAND | block::GRAVEL)
}
fn through(id: u8) -> bool {
    id == AIR || id == block::FIRE || block::is_fluid(id)
}
impl GravityState {
    pub fn new(world: &mut World) -> Self {
        Self {
            changes: world.subscribe_changes(),
            entities: Vec::new(),
            timers: BTreeMap::new(),
            scheduled: HashMap::new(),
            scans: VecDeque::new(),
            chunk_events: world.subscribe_chunk_lifecycle(),
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
    pub fn snapshot(&self) -> SavedGravity {
        SavedGravity {
            entities: self.entities.clone(),
            pending: self
                .scheduled
                .iter()
                .map(|(&p, &due)| (p, due.saturating_sub(self.tick)))
                .collect(),
        }
    }
    pub fn restore(&mut self, saved: SavedGravity) {
        self.entities = saved
            .entities
            .into_iter()
            .filter(|e| {
                falls(e.state.id)
                    && e.position.is_finite()
                    && e.velocity.is_finite()
                    && e.ticks <= 100
            })
            .collect();
        for (p, delay) in saved.pending {
            self.schedule(p, delay.min(100));
        }
    }
}
impl Game {
    pub(super) fn update_gravity(&mut self, dt: f32) {
        self.gravity.fraction += dt * 20.0;
        let ticks = self.gravity.fraction.floor() as u64;
        self.gravity.fraction -= ticks as f32;
        self.gravity.tick = self.gravity.tick.saturating_add(ticks);
        // Activation is bounded independently of world size. Block changes
        // continue through their existing queues and keep original timer debt.
        for _ in 0..256 {
            let Ok(event) = self.gravity.chunk_events.try_recv() else {
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
            if ch.may_contain_any(&[block::SAND, block::GRAVEL]) {
                self.gravity.scans.push_back((c, event.identity, 0));
            }
        }
        for _ in 0..1024 {
            let Ok(change) = self.gravity.changes.try_recv() else {
                break;
            };
            for p in [change.position, change.position + IVec4::Y] {
                if self.world.block(p).is_some_and(falls) {
                    self.gravity.schedule(p, 3);
                }
            }
        }
        for _ in 0..512 {
            let Some((c, identity, index)) = self.gravity.scans.front().copied() else {
                break;
            };
            let Some(ch) = self
                .world
                .chunks
                .get(&c)
                .filter(|ch| ch.identity == identity)
            else {
                self.gravity.scans.pop_front();
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
                self.gravity.scans.pop_front();
            } else {
                self.gravity.scans.front_mut().unwrap().2 += 1;
            }
            if falls(state.id) && self.world.block(p - IVec4::Y).is_some_and(through) {
                self.gravity.schedule(p, 3);
            }
        }
        for _ in 0..64 {
            let Some(p) = self.gravity.next() else { break };
            let Some(state) = self.world.block_state(p).filter(|s| falls(s.id)) else {
                continue;
            };
            let Some(below) = self.world.block(p - IVec4::Y) else {
                self.gravity.schedule(p, 20);
                continue;
            };
            if !through(below) {
                continue;
            }
            if self.gravity.entities.len() >= 256 {
                self.gravity.schedule(p, 3);
                continue;
            }
            if self.world.set_block(p, AIR) {
                self.gravity.entities.push(Falling {
                    position: p.as_vec4() + Vec4::new(0.5, 0.01, 0.5, 0.5),
                    velocity: Vec4::ZERO,
                    state,
                    ticks: 0,
                });
            }
        }
        let mut entities = std::mem::take(&mut self.gravity.entities);
        let mut placements = Vec::new();
        let mut drops = Vec::new();
        entities.retain_mut(|e| {
            if !self
                .world
                .is_loaded(chunk_of(e.position.floor().as_ivec4()))
            {
                return true;
            }
            for _ in 0..ticks {
                e.ticks += 1;
                e.velocity.y -= 0.8;
                let mut body = Body::with_size(e.position, 0.49, 0.98);
                body.vel = e.velocity;
                body.step_states_options(0.05, &|p| self.world.block_state(p), 0.0, false);
                e.position = body.pos;
                e.velocity = body.vel * 0.98;
                if body.on_ground {
                    let p = (e.position + Vec4::Y * 0.49).floor().as_ivec4();
                    placements.push((p, e.state, e.position));
                    return false;
                }
                if e.ticks > 100 {
                    drops.push((e.position, e.state));
                    return false;
                }
            }
            true
        });
        self.gravity.entities = entities;
        for (p, state, position) in placements {
            if self.world.block(p).is_some_and(through) && self.world.set_block_state(p, state) {
            } else {
                drops.push((position, state));
            }
        }
        for (pos, state) in drops {
            let stack = item::from_block_state(state);
            self.spawn_drop(stack, pos, Vec4::ZERO, 0.5);
        }
    }
    pub(super) fn gravity_draws(&self) -> Vec<EntityDraw> {
        self.gravity
            .entities
            .iter()
            .map(|e| EntityDraw {
                center: e.position + Vec4::Y * 0.49,
                radius: 0.49,
                color: [1.0; 3],
                kind: 14,
                forward: Vec4::Z,
                item: e.state.id.into(),
                item_variant: e.state.metadata,
                state: 0,
            })
            .collect()
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
        (g, IVec4::new(8, 9, 8, 8))
    }
    #[test]
    fn unsupported_sand_waits_three_ticks_falls_and_places_without_duplication() {
        let (mut g, p) = arena();
        g.world.set_block(p, block::SAND);
        g.update_gravity(0.0);
        for _ in 0..2 {
            g.update_gravity(0.05);
        }
        assert_eq!(g.world.block(p), Some(block::SAND));
        g.update_gravity(0.05);
        assert_eq!(g.world.block(p), Some(AIR));
        assert_eq!(g.gravity.entities.len(), 1);
        for _ in 0..60 {
            g.update_gravity(0.05);
        }
        assert!(g.gravity.entities.is_empty());
        assert_eq!(g.world.block(IVec4::new(8, 4, 8, 8)), Some(block::SAND));
        assert!(g.dropped_items.is_empty());
    }
    #[test]
    fn falling_gravel_over_torch_becomes_item_and_midfall_save_keeps_one_entity() {
        let (mut g, p) = arena();
        let ground = IVec4::new(8, 4, 8, 8);
        g.world
            .set_block_state(ground, BlockState::new(block::TORCH, 5));
        g.world.set_block(p, block::GRAVEL);
        g.update_gravity(0.0);
        for _ in 0..5 {
            g.update_gravity(0.05);
        }
        let saved: SavedGravity =
            toml::from_str(&toml::to_string(&g.gravity.snapshot()).unwrap()).unwrap();
        assert_eq!(saved.entities.len(), 1);
        g.gravity.entities.clear();
        g.gravity.restore(saved);
        for _ in 0..60 {
            g.update_gravity(0.05);
        }
        assert_eq!(g.world.block(ground), Some(block::TORCH));
        assert!(g.gravity.entities.is_empty());
        assert_eq!(g.dropped_items.len(), 1);
        assert_eq!(
            g.dropped_items[0].stack.block,
            item::ItemId::from(block::GRAVEL)
        );
    }
    #[test]
    fn busy_edit_frames_do_not_delay_sand_activation() {
        let (mut g, p) = arena();
        g.world.set_block(p - IVec4::Y, block::STONE);
        g.world.set_block(p, block::SAND);
        g.update_gravity(0.0);
        for frame in 0..14 {
            // Unrelated edits, e.g. fluids alone may change128 cells a frame.
            for i in 0..256 {
                let q = IVec4::new(i % 16, 12, i / 16, 0);
                g.world
                    .set_block(q, if frame % 2 == 0 { block::STONE } else { AIR });
            }
            if frame == 10 {
                g.world.set_block(p - IVec4::Y, AIR);
            }
            g.update_gravity(0.05);
        }
        assert_eq!(g.world.block(p), Some(AIR));
        assert_eq!(g.gravity.entities.len(), 1);
    }
}
