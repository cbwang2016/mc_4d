//! Event-driven circuit updates with bounded work and delayed Beta devices.
use super::*;
use mc4d_core::{block::BlockState, chunk::CHUNK_VOLUME, redstone as power, shapes};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::mpsc::Receiver,
};
pub(super) struct RedstoneState {
    changes: Receiver<mc4d_core::world::BlockChange>,
    dirty: VecDeque<IVec4>,
    queued: HashSet<IVec4>,
    timers: BTreeMap<u64, Vec<(IVec4, u8)>>,
    scheduled: HashMap<IVec4, (u64, u8)>,
    chunk_events: Receiver<mc4d_core::world::ChunkLifecycle>,
    scans: VecDeque<(IVec4, u64, usize)>,
    refresh: f32,
    pub tick: u64,
    fraction: f32,
    plates: HashSet<IVec4>,
    plate_cursor: usize,
    burnout: HashMap<IVec4, VecDeque<u64>>,
    powered: HashMap<IVec4, bool>,
    events: Vec<IVec4>,
}
impl RedstoneState {
    pub fn new(world: &mut World) -> Self {
        Self {
            changes: world.subscribe_changes(),
            dirty: VecDeque::new(),
            queued: HashSet::new(),
            timers: BTreeMap::new(),
            scheduled: HashMap::new(),
            chunk_events: world.subscribe_chunk_lifecycle(),
            scans: VecDeque::new(),
            refresh: 0.0,
            tick: 0,
            fraction: 0.0,
            plates: HashSet::new(),
            plate_cursor: 0,
            burnout: HashMap::new(),
            powered: HashMap::new(),
            events: Vec::new(),
        }
    }
    fn queue(&mut self, p: IVec4) {
        if self.queued.insert(p) {
            self.dirty.push_back(p)
        }
    }
    fn schedule(&mut self, p: IVec4, id: u8, delay: u64) {
        if self.scheduled.contains_key(&p) {
            return;
        }
        let due = self.tick.saturating_add(delay);
        self.scheduled.insert(p, (due, id));
        self.timers.entry(due).or_default().push((p, id));
    }
    fn burnt_out(&mut self, p: IVec4, record: bool) -> bool {
        let toggles = self.burnout.entry(p).or_default();
        while toggles
            .front()
            .is_some_and(|&tick| self.tick.saturating_sub(tick) > 100)
        {
            toggles.pop_front();
        }
        if record {
            toggles.push_back(self.tick);
        }
        toggles.len() >= 8
    }
    fn next_timer(&mut self) -> Option<(IVec4, u8)> {
        loop {
            let mut first = self.timers.first_entry()?;
            if *first.key() > self.tick {
                return None;
            }
            let due = *first.key();
            let (p, id) = first.get_mut().pop().unwrap();
            if first.get().is_empty() {
                first.remove_entry();
            }
            if self.scheduled.get(&p) == Some(&(due, id)) {
                self.scheduled.remove(&p);
                return Some((p, id));
            }
        }
    }
}
impl Game {
    pub(super) fn use_redstone(&mut self, p: IVec4, state: BlockState) -> bool {
        let metadata = match state.id {
            block::LEVER => state.metadata ^ 8,
            block::BUTTON => {
                if state.metadata & 8 != 0 {
                    return true;
                }
                self.redstone.schedule(p, state.id, 20);
                state.metadata | 8
            }
            block::REPEATER | block::REPEATER_OFF => {
                (state.metadata & !12) | ((((state.metadata >> 2) + 1) & 3) << 2)
            }
            _ => return false,
        };
        self.start_hand_use();
        self.world
            .set_block_state(p, BlockState::new(state.id, metadata));
        self.emit_sound(
            "random.click",
            p.as_vec4(),
            0.3,
            if metadata & 8 != 0 { 0.6 } else { 0.5 },
        );
        true
    }
    pub fn block_powered(&self, p: IVec4) -> bool {
        power::input_power(p, true, &|p| self.world.block_state(p)) > 0
    }
    pub fn take_powered_events(&mut self) -> Vec<IVec4> {
        std::mem::take(&mut self.redstone.events)
    }
    fn plate_occupied(&self, p: IVec4, id: u8) -> bool {
        let zone = shapes::Aabb4::new(
            Vec4::new(0.125, 0.0, 0.125, 0.125),
            Vec4::new(0.875, 0.25, 0.875, 0.875),
        )
        .translated(p.as_vec4());
        self.is_alive() && zone.overlaps(self.body.bounds())
            || self
                .creatures
                .iter()
                .any(|c| c.health > 0 && zone.overlaps(c.body.bounds()))
            || id == block::WOOD_PRESSURE_PLATE
                && self
                    .dropped_items
                    .iter()
                    .any(|d| zone.overlaps(d.body.bounds()))
    }
    fn update_plate(&mut self, p: IVec4, state: BlockState) {
        let pressed = self.plate_occupied(p, state.id);
        let old = state.metadata != 0;
        if pressed != old {
            self.world
                .set_block_state(p, BlockState::new(state.id, pressed as u8));
            self.emit_sound(
                "random.click",
                p.as_vec4(),
                0.3,
                if pressed { 0.6 } else { 0.5 },
            );
        }
        if pressed {
            self.redstone.schedule(p, state.id, 20);
        }
    }
    pub(super) fn update_redstone(&mut self, dt: f32) {
        self.redstone.fraction += dt * 20.0;
        let ticks = self.redstone.fraction.floor() as u64;
        self.redstone.fraction -= ticks as f32;
        self.redstone.tick = self.redstone.tick.saturating_add(ticks);
        // Activation is bounded independently of world size. Block changes
        // continue through their existing queues and keep original timer debt.
        for _ in 0..256 {
            let Ok(event) = self.redstone.chunk_events.try_recv() else {
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
            if ch.may_contain_any(power::COMPONENTS) {
                self.redstone.scans.push_back((c, event.identity, 0));
            }
        }
        self.redstone.refresh -= dt;
        if self.redstone.refresh <= 0.0 {
            self.redstone.refresh = 1.0;
            self.redstone.plates.retain(|p| {
                self.world.block(*p).is_some_and(|id| {
                    matches!(id, block::WOOD_PRESSURE_PLATE | block::STONE_PRESSURE_PLATE)
                })
            });
            self.redstone
                .powered
                .retain(|p, _| self.world.is_loaded(chunk_of(*p)));
            self.redstone.burnout.retain(|p, times| {
                self.world.is_loaded(chunk_of(*p))
                    && times
                        .back()
                        .is_some_and(|t| self.redstone.tick.saturating_sub(*t) <= 100)
            });
        }
        for _ in 0..1024 {
            let Some((c, id, index)) = self.redstone.scans.front().copied() else {
                break;
            };
            let Some(chunk) = self.world.chunks.get(&c).filter(|ch| ch.identity == id) else {
                self.redstone.scans.pop_front();
                continue;
            };
            let local = IVec4::new(
                (index & 15) as i32,
                ((index >> 4) & 15) as i32,
                ((index >> 8) & 15) as i32,
                ((index >> 12) & 15) as i32,
            );
            let state = chunk.get_state(local);
            if index + 1 == CHUNK_VOLUME {
                self.redstone.scans.pop_front();
            } else {
                self.redstone.scans.front_mut().unwrap().2 += 1;
            }
            if power::COMPONENTS.contains(&state.id) {
                let p = c * 16 + local;
                // A device that was already powered when saved must not fire
                // again merely because its chunk was loaded.
                if matches!(state.id, block::DISPENSER | block::NOTE_BLOCK | block::TNT) {
                    let powered = self.block_powered(p)
                        || (state.id == block::DISPENSER && self.block_powered(p + IVec4::Y));
                    self.redstone.powered.insert(p, powered);
                }
                self.redstone.queue(p);
            }
        }
        // Contacts press immediately; releasing a plate is checked on its 20-tick timer.
        let plates: Vec<_> = self.redstone.plates.iter().copied().collect();
        if !plates.is_empty() {
            for offset in 0..plates.len().min(64) {
                let p = plates[(self.redstone.plate_cursor + offset) % plates.len()];
                if let Some(s) = self.world.block_state(p)
                    && s.metadata == 0
                {
                    self.update_plate(p, s);
                }
            }
            self.redstone.plate_cursor = (self.redstone.plate_cursor + 64) % plates.len();
        }
        for _ in 0..64 {
            let Some((p, id)) = self.redstone.next_timer() else {
                break;
            };
            let Some(s) = self.world.block_state(p).filter(|s| s.id == id) else {
                continue;
            };
            match id {
                block::BUTTON => {
                    self.world
                        .set_block_state(p, BlockState::new(id, s.metadata & !8));
                    self.emit_sound("random.click", p.as_vec4(), 0.3, 0.5);
                }
                block::WOOD_PRESSURE_PLATE | block::STONE_PRESSURE_PLATE => self.update_plate(p, s),
                block::REDSTONE_TORCH | block::REDSTONE_TORCH_OFF => {
                    let input = power::torch_input(p, s, &|p| self.world.block_state(p));
                    if id == block::REDSTONE_TORCH && input {
                        self.world.set_block_state(
                            p,
                            BlockState::new(block::REDSTONE_TORCH_OFF, s.metadata),
                        );
                        if self.redstone.burnt_out(p, true) {
                            self.redstone.schedule(p, block::REDSTONE_TORCH_OFF, 102);
                        }
                    } else if id == block::REDSTONE_TORCH_OFF && !input {
                        if !self.redstone.burnt_out(p, false) {
                            self.world.set_block_state(
                                p,
                                BlockState::new(block::REDSTONE_TORCH, s.metadata),
                            );
                        } else {
                            self.redstone.schedule(p, id, 102);
                        }
                    }
                }
                block::REPEATER | block::REPEATER_OFF => {
                    let input = power::repeater_input(p, s, &|p| self.world.block_state(p));
                    if id == block::REPEATER_OFF {
                        self.world
                            .set_block_state(p, BlockState::new(block::REPEATER, s.metadata));
                        if !input {
                            self.redstone.schedule(
                                p,
                                block::REPEATER,
                                power::repeater_delay(s.metadata),
                            );
                        }
                    } else if !input {
                        self.world
                            .set_block_state(p, BlockState::new(block::REPEATER_OFF, s.metadata));
                    }
                }
                _ => {}
            }
        }
        // Torch, dust and device reactions settle within the same frame instead
        // of costing one rendered frame per hop. Work stays bounded per frame.
        for _ in 0..4 {
            if self.drain_redstone_changes() + self.process_redstone_dirty() == 0 {
                break;
            }
        }
    }
    fn drain_redstone_changes(&mut self) -> usize {
        let mut work = 0;
        for _ in 0..128 {
            let Ok(change) = self.redstone.changes.try_recv() else {
                break;
            };
            work += 1;
            if self
                .redstone
                .scheduled
                .get(&change.position)
                .is_some_and(|(_, id)| *id != change.after.id)
            {
                self.redstone.scheduled.remove(&change.position);
            }
            for a in std::iter::once(IVec4::ZERO).chain(power::NEIGHBORS) {
                for b in std::iter::once(IVec4::ZERO).chain(power::NEIGHBORS) {
                    let p = change.position + a + b;
                    if self
                        .world
                        .block(p)
                        .is_some_and(|id| power::COMPONENTS.contains(&id))
                    {
                        self.redstone.queue(p);
                    }
                }
            }
        }
        work
    }
    fn process_redstone_dirty(&mut self) -> usize {
        let mut work = 0;
        for _ in 0..128 {
            let Some(p) = self.redstone.dirty.pop_front() else {
                break;
            };
            work += 1;
            self.redstone.queued.remove(&p);
            let Some(s) = self.world.block_state(p) else {
                continue;
            };
            match s.id {
                block::REDSTONE_WIRE => {
                    let changed = power::wire_network(p, 4096, &|p| self.world.block_state(p));
                    for (q, next) in changed {
                        self.world.set_block_state(q, BlockState::new(s.id, next));
                    }
                }
                block::REDSTONE_TORCH | block::REDSTONE_TORCH_OFF => {
                    let input = power::torch_input(p, s, &|p| self.world.block_state(p));
                    if (s.id == block::REDSTONE_TORCH) == input {
                        self.redstone.schedule(p, s.id, 2);
                    }
                }
                block::REPEATER | block::REPEATER_OFF => {
                    let input = power::repeater_input(p, s, &|p| self.world.block_state(p));
                    if (s.id == block::REPEATER) != input {
                        self.redstone
                            .schedule(p, s.id, power::repeater_delay(s.metadata));
                    }
                }
                block::BUTTON => {
                    if s.metadata & 8 != 0 {
                        self.redstone.schedule(p, s.id, 20);
                    }
                }
                block::WOOD_PRESSURE_PLATE | block::STONE_PRESSURE_PLATE => {
                    self.redstone.plates.insert(p);
                    self.update_plate(p, s);
                }
                block::WOOD_DOOR | block::IRON_DOOR => {
                    let bottom = if s.metadata & 8 == 0 { p } else { p - IVec4::Y };
                    let powered =
                        self.block_powered(bottom) || self.block_powered(bottom + IVec4::Y);
                    let previous = self.redstone.powered.insert(bottom, powered);
                    if powered || previous == Some(true) || s.id == block::IRON_DOOR {
                        self.set_door_open(bottom, powered);
                    }
                }
                block::DISPENSER | block::NOTE_BLOCK | block::TNT => {
                    let powered = self.block_powered(p)
                        || (s.id == block::DISPENSER && self.block_powered(p + IVec4::Y));
                    if powered && self.redstone.powered.get(&p) != Some(&true) {
                        self.redstone.events.push(p);
                    }
                    self.redstone.powered.insert(p, powered);
                }
                _ => {}
            }
        }
        work
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc4d_core::chunk::{CHUNK_VOLUME, Chunk};
    fn circuit() -> (Game, IVec4) {
        let mut g = Game::menu_preview(7);
        let mut cells = vec![AIR; CHUNK_VOLUME];
        for i in 0..CHUNK_VOLUME {
            if (i >> 4) & 15 == 7 {
                cells[i] = block::STONE;
            }
        }
        g.world.insert_chunk(IVec4::ZERO, Chunk::from_dense(cells));
        g.body.pos = Vec4::new(2.0, 8.0, 2.0, 2.0);
        (g, IVec4::splat(8))
    }
    fn steps(g: &mut Game, n: usize) {
        for _ in 0..n {
            g.update_redstone(0.05);
        }
    }
    #[test]
    fn lever_wire_and_iron_door_work_across_w_and_turn_off() {
        let (mut g, p) = circuit();
        g.world.set_block_state(p, BlockState::new(block::LEVER, 5));
        for n in 1..=4 {
            g.world.set_block(p + IVec4::W * n, block::REDSTONE_WIRE);
        }
        let door = p + IVec4::W * 5;
        g.world
            .set_block_state(door, BlockState::new(block::IRON_DOOR, 0));
        g.world
            .set_block_state(door + IVec4::Y, BlockState::new(block::IRON_DOOR, 8));
        g.use_redstone(p, g.world.block_state(p).unwrap());
        steps(&mut g, 12);
        for n in 1..=4 {
            assert_eq!(
                g.world.block_state(p + IVec4::W * n).unwrap().metadata,
                16 - n as u8
            );
        }
        assert_ne!(g.world.block_state(door).unwrap().metadata & 4, 0);
        g.use_redstone(p, g.world.block_state(p).unwrap());
        steps(&mut g, 40);
        assert_eq!(g.world.block_state(door).unwrap().metadata & 4, 0);
        assert_eq!(g.world.block_state(p + IVec4::W).unwrap().metadata, 0);
    }
    #[test]
    fn torch_inverts_powered_support_after_two_ticks() {
        let (mut g, p) = circuit();
        g.world.set_block(p, block::STONE);
        let lever = p + IVec4::Y;
        let torch = p + IVec4::X;
        g.world
            .set_block_state(lever, BlockState::new(block::LEVER, 5));
        g.world
            .set_block_state(torch, BlockState::new(block::REDSTONE_TORCH, 1));
        g.use_redstone(lever, g.world.block_state(lever).unwrap());
        g.update_redstone(0.0);
        g.update_redstone(0.05);
        assert_eq!(g.world.block(torch), Some(block::REDSTONE_TORCH));
        g.update_redstone(0.05);
        assert_eq!(g.world.block(torch), Some(block::REDSTONE_TORCH_OFF));
        g.use_redstone(lever, g.world.block_state(lever).unwrap());
        g.update_redstone(0.0);
        steps(&mut g, 2);
        assert_eq!(g.world.block(torch), Some(block::REDSTONE_TORCH));
    }
    #[test]
    fn repeater_preserves_short_pulse_and_selected_four_tick_delay() {
        let (mut g, p) = circuit();
        let lever = p - IVec4::W;
        g.world
            .set_block_state(lever, BlockState::new(block::LEVER, 5 | 8));
        g.world
            .set_block_state(p, BlockState::new(block::REPEATER_OFF, 16 | 12));
        g.update_redstone(0.0);
        steps(&mut g, 3);
        g.use_redstone(lever, g.world.block_state(lever).unwrap());
        steps(&mut g, 4);
        assert_eq!(g.world.block(p), Some(block::REPEATER_OFF));
        steps(&mut g, 1);
        assert_eq!(g.world.block(p), Some(block::REPEATER));
        steps(&mut g, 7);
        assert_eq!(g.world.block(p), Some(block::REPEATER));
        steps(&mut g, 1);
        assert_eq!(g.world.block(p), Some(block::REPEATER_OFF));
    }
    #[test]
    fn button_releases_after_one_second_and_wood_plate_accepts_items() {
        let (mut g, p) = circuit();
        g.world
            .set_block_state(p, BlockState::new(block::BUTTON, 1));
        g.use_redstone(p, g.world.block_state(p).unwrap());
        steps(&mut g, 19);
        assert_ne!(g.world.block_state(p).unwrap().metadata & 8, 0);
        steps(&mut g, 1);
        assert_eq!(g.world.block_state(p).unwrap().metadata & 8, 0);
        g.world.set_block(p, block::WOOD_PRESSURE_PLATE);
        g.world
            .set_block(p + IVec4::X * 3, block::STONE_PRESSURE_PLATE);
        for q in [p, p + IVec4::X * 3] {
            g.spawn_drop(
                Slot::new(block::DIRT, 1),
                q.as_vec4() + Vec4::new(0.5, 0.01, 0.5, 0.5),
                Vec4::ZERO,
                1.0,
            );
        }
        g.update_redstone(0.0);
        assert_eq!(g.world.block_state(p).unwrap().metadata, 1);
        assert_eq!(g.world.block_state(p + IVec4::X * 3).unwrap().metadata, 0);
        g.dropped_items.clear();
        steps(&mut g, 20);
        assert_eq!(g.world.block_state(p).unwrap().metadata, 0);
    }

    #[test]
    fn long_dust_lines_and_loops_settle_within_one_frame() {
        let (mut g, p) = circuit();
        let lever = p - IVec4::X * 6;
        g.world.set_block_state(lever, BlockState::new(block::LEVER, 5));
        // A U-shaped loop crossing X and W must neither lag nor self-sustain.
        let mut line = Vec::new();
        for x in -5..=5 { line.push(p + IVec4::X * x); }
        for w in 1..=2 { line.push(p + IVec4::X * 5 + IVec4::W * w); }
        for x in (-5..=5).rev() { line.push(p + IVec4::X * x + IVec4::W * 3); }
        line.push(p - IVec4::X * 5 + IVec4::W * 2);
        line.push(p - IVec4::X * 5 + IVec4::W);
        for &q in &line { g.world.set_block(q, block::REDSTONE_WIRE); }
        g.update_redstone(0.0);
        g.use_redstone(lever, g.world.block_state(lever).unwrap());
        g.update_redstone(0.0);
        assert_eq!(g.world.block_state(line[0]).unwrap().metadata, 15);
        assert_eq!(g.world.block_state(line[10]).unwrap().metadata, 5);
        assert_eq!(g.world.block_state(*line.last().unwrap()).unwrap().metadata, 14);
        g.use_redstone(lever, g.world.block_state(lever).unwrap());
        g.update_redstone(0.0);
        assert!(line.iter().all(|&q| g.world.block_state(q).unwrap().metadata == 0));
    }
    #[test]
    fn torch_clock_toggles_every_two_ticks_until_burnout() {
        let (mut g, p) = circuit();
        g.world.set_block(p, block::STONE);
        let torch = p + IVec4::X;
        g.world.set_block_state(torch, BlockState::new(block::REDSTONE_TORCH, 1));
        g.world.set_block(torch + IVec4::Y, block::REDSTONE_WIRE);
        g.world.set_block(p + IVec4::Y, block::REDSTONE_WIRE);
        g.update_redstone(0.0);
        let mut states = Vec::new();
        for _ in 0..12 {
            g.update_redstone(0.05);
            states.push(g.world.block(torch) == Some(block::REDSTONE_TORCH));
        }
        assert_eq!(states, [true, false, false, true, true, false, false, true, true, false, false, true]);
        steps(&mut g, 40);
        assert_eq!(g.world.block(torch), Some(block::REDSTONE_TORCH_OFF), "rapid toggling burns out");
    }
    #[test]
    fn dust_passing_a_repeater_side_still_runs_straight() {
        let (mut g, p) = circuit();
        g.world.set_block_state(p - IVec4::X, BlockState::new(block::LEVER, 5 | 8));
        g.world.set_block(p, block::REDSTONE_WIRE);
        // Repeater beside the dust, facing along X: not a dust connection.
        g.world.set_block_state(p + IVec4::W, BlockState::new(block::REPEATER_OFF, power::repeater_metadata(IVec4::X)));
        let door = p + IVec4::X;
        g.world.set_block_state(door, BlockState::new(block::IRON_DOOR, 0));
        g.world.set_block_state(door + IVec4::Y, BlockState::new(block::IRON_DOOR, 8));
        g.update_redstone(0.0);
        steps(&mut g, 2);
        assert_ne!(g.world.block_state(door).unwrap().metadata & 4, 0);
    }
    #[test]
    fn loading_an_already_powered_dispenser_does_not_fire_it() {
        let (mut g, _) = circuit();
        let mut chunk = Chunk::from_dense(vec![AIR; CHUNK_VOLUME]);
        let (lever, dispenser) = (IVec4::new(3, 3, 3, 3), IVec4::new(4, 3, 3, 3));
        chunk.set_state(lever, BlockState::new(block::LEVER, 5 | 8));
        chunk.set_state(dispenser, BlockState::new(block::DISPENSER, 0));
        let c = IVec4::new(0, 2, 0, 0);
        g.world.insert_chunk(c, chunk);
        for _ in 0..80 { g.update_redstone(0.05); }
        assert!(g.take_powered_events().is_empty());
        let lever = c * 16 + lever;
        g.use_redstone(lever, g.world.block_state(lever).unwrap());
        steps(&mut g, 2);
        g.use_redstone(lever, g.world.block_state(lever).unwrap());
        steps(&mut g, 2);
        assert_eq!(g.take_powered_events(), vec![c * 16 + dispenser]);
    }
}
