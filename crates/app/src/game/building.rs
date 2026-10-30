//! Stateful building interactions. Pair placement is preflighted before either
//! half changes; support checks are scheduled and bounded instead of world scans.
use super::*;
use mc4d_core::{block::BlockState, shapes};
use std::collections::{HashSet, VecDeque};
use std::sync::mpsc::Receiver;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignText {
    pub position: IVec4,
    pub lines: [String; 4],
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct SavedBuilding {
    #[serde(default)]
    signs: Vec<SignText>,
    #[serde(default)]
    bed_spawn: Option<IVec4>,
    #[serde(default)]
    pending: Vec<IVec4>,
}
pub(super) struct BuildingState {
    changes: Receiver<mc4d_core::world::BlockChange>,
    pending: VecDeque<IVec4>,
    queued: HashSet<IVec4>,
    signs: Vec<SignText>,
    pub bed_spawn: Option<IVec4>,
    edit_sign: Option<IVec4>,
}
impl BuildingState {
    pub fn new(world: &mut World) -> Self {
        Self {
            changes: world.subscribe_changes(),
            pending: VecDeque::new(),
            queued: HashSet::new(),
            signs: Vec::new(),
            bed_spawn: None,
            edit_sign: None,
        }
    }
    fn queue(&mut self, pos: IVec4) {
        if self.queued.insert(pos) {
            self.pending.push_back(pos)
        }
    }
    fn receive(&mut self, limit: usize) {
        for _ in 0..limit {
            let Ok(change) = self.changes.try_recv() else {
                break;
            };
            for d in [
                IVec4::ZERO,
                IVec4::X,
                IVec4::NEG_X,
                IVec4::Y,
                IVec4::NEG_Y,
                IVec4::Z,
                IVec4::NEG_Z,
                IVec4::W,
                IVec4::NEG_W,
            ] {
                self.queue(change.position + d)
            }
        }
    }
    pub fn snapshot(&mut self) -> SavedBuilding {
        self.receive(usize::MAX);
        SavedBuilding {
            signs: self.signs.clone(),
            bed_spawn: self.bed_spawn,
            pending: self.pending.iter().copied().collect(),
        }
    }
    pub fn restore(&mut self, saved: SavedBuilding) {
        self.signs = saved.signs;
        for sign in &mut self.signs {
            for line in &mut sign.lines {
                *line = clean_line(line)
            }
        }
        self.bed_spawn = saved.bed_spawn;
        for pos in saved.pending {
            self.queue(pos)
        }
    }
}
fn clean_line(line: &str) -> String {
    line.chars().filter(|c| !c.is_control()).take(15).collect()
}
fn is_door(id: u8) -> bool {
    matches!(id, block::WOOD_DOOR | block::IRON_DOOR)
}
fn is_sign(id: u8) -> bool {
    matches!(id, block::SIGN_POST | block::WALL_SIGN)
}
fn paired(pos: IVec4, state: BlockState) -> Option<IVec4> {
    if is_door(state.id) {
        Some(
            pos + if state.metadata & 8 == 0 {
                IVec4::Y
            } else {
                IVec4::NEG_Y
            },
        )
    } else if state.id == block::BED {
        Some(
            pos + shapes::bed_direction(state.metadata)
                * if state.metadata & 8 == 0 { 1 } else { -1 },
        )
    } else {
        None
    }
}

impl Game {
    /// Beta1.5 et.a(IIIIZ) only lets placement overwrite air, the four fluid
    /// blocks, fire and snow layers; flowers, saplings and crops obstruct it.
    fn replaceable(&self, pos: IVec4) -> bool {
        self.world.block_state(pos).is_some_and(|s| {
            s.id == AIR
                || block::is_fluid(s.id)
                || matches!(
                    block::definition(s.id).shape,
                    block::BlockShape::Fire | block::BlockShape::SnowLayer
                )
        })
    }
    pub(super) fn full_support(&self, pos: IVec4) -> bool {
        self.world.block_state(pos).is_some_and(|s| {
            block::definition(s.id).opaque
                && shapes::collision_boxes(s, 0)
                    .as_slice()
                    .iter()
                    .any(|b| b.min.cmple(Vec4::ZERO).all() && b.max.cmpge(Vec4::ONE).all())
        })
    }
    fn can_occupy(&self, pos: IVec4, state: BlockState) -> bool {
        self.world.is_loaded(chunk_of(pos))
            && self.replaceable(pos)
            && !self.body.overlaps_state(pos, state, 0)
            && !self
                .creatures
                .iter()
                .any(|c| c.body.overlaps_state(pos, state, 0))
    }

    /// Shared placement dispatcher: metadata and paired blocks are never flattened
    /// into a generic cube by the ordinary placement path.
    pub(super) fn place_building(&mut self, hit: Hit, stack: Slot) -> bool {
        let Some(mut state) = item::placement(stack) else {
            return false;
        };
        if hit.normal == IVec4::ZERO {
            return false;
        }
        let targeted = self.world.block(hit.block).unwrap_or(AIR);
        // ki.a: an iron door swallows the click, so nothing is built against it.
        if targeted == block::IRON_DOOR {
            return false;
        }
        if state.id == block::SLAB && targeted == block::SLAB && hit.normal == IVec4::Y {
            let old = self.world.block_state(hit.block).unwrap();
            if old.metadata & 3 == state.metadata & 3 {
                let double = BlockState::new(block::DOUBLE_SLAB, state.metadata & 3);
                return !self.body.overlaps_state(hit.block, double, 0)
                    && !self
                        .creatures
                        .iter()
                        .any(|c| c.body.overlaps_state(hit.block, double, 0))
                    && self.world.set_block_state(hit.block, double);
            }
        }
        // Item blocks redirect only a clicked snow layer into its own cell.
        let pos = if targeted == block::SNOW_LAYER {
            hit.block
        } else {
            hit.block + hit.normal
        };
        if state.id == block::SLAB
            && self
                .world
                .block_state(pos)
                .is_some_and(|s| s.id == block::SLAB && s.metadata & 3 == state.metadata & 3)
        {
            let double = BlockState::new(block::DOUBLE_SLAB, state.metadata & 3);
            return !self.body.overlaps_state(pos, double, 0)
                && !self
                    .creatures
                    .iter()
                    .any(|c| c.body.overlaps_state(pos, double, 0))
                && self.world.set_block_state(pos, double);
        }
        let forward = shapes::dominant_horizontal(self.basis().forward_h);
        if is_door(state.id) {
            if !self.full_support(pos - IVec4::Y) {
                return false;
            }
            state.metadata = shapes::facing_metadata(state.id, forward);
            let upper = BlockState::new(state.id, state.metadata | 8);
            if !self.can_occupy(pos, state) || !self.can_occupy(pos + IVec4::Y, upper) {
                return false;
            }
            self.world.set_block_state(pos, state);
            self.world.set_block_state(pos + IVec4::Y, upper);
            return true;
        }
        if state.id == block::BED {
            state.metadata = shapes::facing_metadata(state.id, forward);
            let other = pos + shapes::bed_direction(state.metadata);
            let head = BlockState::new(state.id, state.metadata | 8);
            // kt.a: top faces only, and both cells must be air.
            if hit.normal != IVec4::Y
                || self.world.block(pos) != Some(AIR)
                || self.world.block(other) != Some(AIR)
                || !self.full_support(pos - IVec4::Y)
                || !self.full_support(other - IVec4::Y)
                || !self.can_occupy(pos, state)
                || !self.can_occupy(other, head)
            {
                return false;
            }
            self.world.set_block_state(pos, state);
            self.world.set_block_state(other, head);
            return true;
        }
        match state.id {
            block::WOOD_STAIRS | block::STONE_STAIRS => {
                state.metadata = shapes::facing_metadata(state.id, forward)
            }
            block::CHEST => {
                if !mc4d_core::containers::can_place_chest(pos, &|p| self.world.block_state(p)) {
                    return false;
                }
                state.metadata = 0;
            }
            block::FURNACE | block::LIT_FURNACE | block::DISPENSER => {
                state.metadata = shapes::wall_metadata(-forward)
            }
            block::LEVER | block::BUTTON => {
                if state.id == block::BUTTON && hit.normal.y != 0 {
                    return false;
                }
                let Some(meta) = shapes::torch_metadata(hit.normal) else {
                    return false;
                };
                if !self.full_support(hit.block) {
                    return false;
                }
                state.metadata = meta
            }
            block::REPEATER | block::REPEATER_OFF => {
                if !self.full_support(pos - IVec4::Y) {
                    return false;
                }
                state.metadata = mc4d_core::redstone::repeater_metadata(forward)
            }
            block::LADDER => {
                if hit.normal.y != 0 || !self.full_support(hit.block) {
                    return false;
                }
                state.metadata = shapes::wall_metadata(hit.normal)
            }
            block::TORCH | block::REDSTONE_TORCH | block::REDSTONE_TORCH_OFF => {
                let Some(meta) = shapes::torch_metadata(hit.normal) else {
                    return false;
                };
                if !self.full_support(hit.block) {
                    return false;
                }
                state.metadata = meta
            }
            block::SIGN_POST | block::WALL_SIGN => {
                if hit.normal == IVec4::Y {
                    if !self.full_support(pos - IVec4::Y) {
                        return false;
                    }
                    state.id = block::SIGN_POST;
                    let f = self.basis().forward_h;
                    state.metadata = if f.w.abs() > f.x.abs().max(f.z.abs()) {
                        if f.w > 0.0 { 17 } else { 16 }
                    } else {
                        ((8.0 - f.x.atan2(f.z) / std::f32::consts::TAU * 16.0).round() as i32)
                            .rem_euclid(16) as u8
                    };
                } else if hit.normal.y == 0 && self.full_support(hit.block) {
                    state.id = block::WALL_SIGN;
                    state.metadata = shapes::wall_metadata(hit.normal)
                } else {
                    return false;
                }
            }
            block::CROPS => {
                // qj.a: seeds go on the top face of farmland into air only.
                if hit.normal != IVec4::Y
                    || targeted != block::FARMLAND
                    || self.world.block(pos) != Some(AIR)
                {
                    return false;
                }
                state.metadata = 0;
            }
            block::SAPLING | block::DANDELION | block::ROSE => {
                if !matches!(
                    self.world.block(pos - IVec4::Y),
                    Some(block::DIRT | block::GRASS | block::FARMLAND)
                ) {
                    return false;
                }
            }
            block::BROWN_MUSHROOM | block::RED_MUSHROOM => {
                if !self.full_support(pos - IVec4::Y) {
                    return false;
                }
            }
            block::REEDS => {
                let below = pos - IVec4::Y;
                if !matches!(
                    self.world.block(below),
                    Some(block::DIRT | block::GRASS | block::REEDS)
                ) {
                    return false;
                }
                if self.world.block(below) != Some(block::REEDS)
                    && !shapes::HORIZONTAL_DIRECTIONS
                        .iter()
                        .any(|&d| self.world.block(below + d).is_some_and(block::is_water))
                {
                    return false;
                }
            }
            block::CACTUS => {
                if !matches!(
                    self.world.block(pos - IVec4::Y),
                    Some(block::SAND | block::CACTUS)
                ) || shapes::HORIZONTAL_DIRECTIONS
                    .iter()
                    .any(|&d| self.world.block(pos + d).is_some_and(block::is_solid))
                {
                    return false;
                }
            }
            // jc.a: never directly on another fence; any solid material below.
            block::FENCE => {
                let below = self.world.block(pos - IVec4::Y).unwrap_or(AIR);
                if below == block::FENCE || !mc4d_core::ecology::material_blocks_movement(below) {
                    return false;
                }
            }
            // es.a/es.a(kw): a normal cube below; the face follows the
            // placer's yaw (front on the side being looked toward).
            block::PUMPKIN | block::JACK_O_LANTERN => {
                if !self.full_support(pos - IVec4::Y) {
                    return false;
                }
                let f = self.basis().forward_h;
                state.metadata = if f.z.abs() >= f.x.abs() {
                    if f.z >= 0.0 { 0 } else { 2 }
                } else if f.x < 0.0 {
                    1
                } else {
                    3
                };
            }
            block::CAKE
            | block::RAIL
            | block::POWERED_RAIL
            | block::DETECTOR_RAIL
            | block::REDSTONE_WIRE
            | block::WOOD_PRESSURE_PLATE
            | block::STONE_PRESSURE_PLATE => {
                if !self.full_support(pos - IVec4::Y) {
                    return false;
                }
            }
            _ => {}
        }
        if !self.can_occupy(pos, state) || !self.world.set_block_state(pos, state) {
            return false;
        }
        if is_sign(state.id) {
            self.building.signs.retain(|sign| sign.position != pos);
            self.building.signs.push(SignText {
                position: pos,
                lines: std::array::from_fn(|_| String::new()),
            });
            self.building.edit_sign = Some(pos);
        }
        true
    }

    pub fn set_door_open(&mut self, pos: IVec4, open: bool) -> bool {
        let Some(state) = self.world.block_state(pos) else {
            return false;
        };
        if !is_door(state.id) {
            return false;
        }
        let bottom = if state.metadata & 8 != 0 {
            pos - IVec4::Y
        } else {
            pos
        };
        let Some(lower) = self.world.block_state(bottom) else {
            return false;
        };
        if lower.id != state.id || self.world.block(bottom + IVec4::Y) != Some(state.id) {
            return false;
        }
        let metadata = (lower.metadata & !4) | if open { 4 } else { 0 };
        if metadata == lower.metadata {
            return true;
        }
        self.world
            .set_block_state(bottom, BlockState::new(state.id, metadata));
        self.world
            .set_block_state(bottom + IVec4::Y, BlockState::new(state.id, metadata | 8));
        self.emit_sound(
            if open {
                "random.door_open"
            } else {
                "random.door_close"
            },
            bottom.as_vec4(),
            1.0,
            1.0,
        );
        true
    }
    #[cfg(test)]
    pub fn set_block_powered(&mut self, pos: IVec4, powered: bool) -> bool {
        self.set_door_open(pos, powered)
    }

    pub(super) fn use_target_building(&mut self) -> bool {
        let Some(Target::Block(hit)) = self.target else {
            return false;
        };
        let Some(state) = self.world.block_state(hit.block) else {
            return false;
        };
        if self.use_redstone(hit.block, state) {
            return true;
        }
        if state.id == block::NOTE_BLOCK {
            self.start_hand_use();
            self.world.set_block_state(
                hit.block,
                BlockState::new(state.id, (state.metadata + 1) % 25),
            );
            self.play_note(hit.block);
            return true;
        }
        if state.id == block::WOOD_DOOR {
            return self.set_door_open(hit.block, state.metadata & 4 == 0);
        }
        if state.id == block::BED {
            self.sleep_in_bed(hit.block);
            return true;
        }
        if state.id == block::CAKE {
            self.eat_cake(hit.block);
            return true;
        }
        false
    }
    /// th.c: both right and left clicks eat a slice, only below full health.
    pub(super) fn eat_cake(&mut self, p: IVec4) {
        let Some(state) = self.world.block_state(p).filter(|s| s.id == block::CAKE) else {
            return;
        };
        if self.health < 20 {
            self.health = (self.health + 3).min(20);
            if state.metadata >= 5 {
                self.world.set_block(p, AIR);
            } else {
                self.world
                    .set_block_state(p, BlockState::new(state.id, state.metadata + 1));
            }
        }
    }

    fn bed_exit(&self, head: IVec4) -> Option<Vec4> {
        for distance in [1, 2] {
            for d in shapes::HORIZONTAL_DIRECTIONS {
                let cell = head + d * distance;
                let feet = cell.as_vec4() + Vec4::new(0.5, 0.0, 0.5, 0.5);
                if self.full_support(cell - IVec4::Y)
                    && !Body::new(feet).is_stuck_states(&|p| self.world.block_state(p))
                {
                    return Some(feet);
                }
            }
        }
        None
    }
    pub fn sleep_in_bed(&mut self, pos: IVec4) -> bool {
        let Some(state) = self.world.block_state(pos) else {
            return false;
        };
        if state.id != block::BED {
            return false;
        }
        let head = if state.metadata & 8 != 0 {
            pos
        } else {
            pos + shapes::bed_direction(state.metadata)
        };
        if self.world.block(head) != Some(block::BED) {
            return false;
        }
        if !self.environment.is_night() {
            self.toast("You can only sleep at night");
            return false;
        }
        // gh.b(III) TOO_FAR_AWAY: eye within 3 horizontally (here X, Z and W)
        // and 2 vertically of the head block; Beta shows no message for it.
        let reach = self.eye() - head.as_vec4();
        if reach.x.abs() > 3.0 || reach.y.abs() > 2.0 || reach.z.abs() > 3.0 || reach.w.abs() > 3.0 {
            return false;
        }
        // Beta 1.5 has no "monsters nearby" refusal (its sleep result enum
        // has five values and the language file no tile.bed.notSafe).
        let Some(exit) = self.bed_exit(head) else {
            self.toast("The bed is obstructed");
            return false;
        };
        self.spawn = exit;
        self.building.bed_spawn = Some(head);
        self.environment.sleep_until_morning();
        self.body.pos = exit;
        self.body.vel = Vec4::ZERO;
        self.fall_distance = 0.0;
        self.toast("Respawn point set");
        true
    }
    pub(super) fn validate_bed_respawn(&mut self) {
        let Some(head) = self.building.bed_spawn else {
            return;
        };
        let valid = self
            .world
            .block_state(head)
            .filter(|s| s.id == block::BED)
            .and_then(|s| paired(head, s))
            .is_some_and(|other| self.world.block(other) == Some(block::BED));
        if valid && let Some(exit) = self.bed_exit(head) {
            self.spawn = exit;
            return;
        }
        self.spawn = super::find_spawn(&self.world);
        self.building.bed_spawn = None;
        self.toast("Your home bed was missing or obstructed");
    }

    pub fn take_sign_editor_request(&mut self) -> Option<(IVec4, [String; 4])> {
        let pos = self.building.edit_sign.take()?;
        self.building
            .signs
            .iter()
            .find(|s| s.position == pos)
            .map(|s| (pos, s.lines.clone()))
    }
    pub fn save_sign_text(&mut self, pos: IVec4, lines: [String; 4]) -> bool {
        if !self.world.block(pos).is_some_and(is_sign) {
            return false;
        }
        let Some(sign) = self.building.signs.iter_mut().find(|s| s.position == pos) else {
            return false;
        };
        sign.lines = lines.map(|line| clean_line(&line));
        true
    }
    pub fn sign_texts(&self) -> &[SignText] {
        &self.building.signs
    }

    pub(super) fn break_building_pair(&mut self, pos: IVec4, state: BlockState) {
        if let Some(other) = paired(pos, state)
            && self.world.block(other) == Some(state.id)
        {
            self.world.set_block(other, AIR);
        }
        if is_sign(state.id) {
            self.building.signs.retain(|s| s.position != pos);
        }
    }
    pub(super) fn paired_loot_state(&self, pos: IVec4, mut state: BlockState) -> BlockState {
        if state.metadata & 8 != 0
            && paired(pos, state).is_some_and(|other| self.world.block(other) == Some(state.id))
        {
            state.metadata &= !8;
        }
        state
    }
    pub(super) fn update_building(&mut self) {
        self.building.receive(256);
        let count = self.building.pending.len().min(64);
        for _ in 0..count {
            let Some(pos) = self.building.pending.pop_front() else {
                break;
            };
            self.building.queued.remove(&pos);
            let Some(state) = self.world.block_state(pos) else {
                self.building.queue(pos);
                continue;
            };
            if self.stations.iter().any(|s| {
                s.pos == pos.to_array() && StationKind::from_block(state.id) != Some(s.kind)
            }) {
                self.break_station(pos);
            }
            if !is_sign(state.id) {
                self.building.signs.retain(|s| s.position != pos);
            }
            let supported = if is_door(state.id) {
                let bottom = if state.metadata & 8 == 0 {
                    pos
                } else {
                    pos - IVec4::Y
                };
                self.full_support(bottom - IVec4::Y)
                    && self.world.block(bottom) == Some(state.id)
                    && self.world.block(bottom + IVec4::Y) == Some(state.id)
            } else if state.id == block::BED {
                self.full_support(pos - IVec4::Y)
                    && paired(pos, state)
                        .is_some_and(|other| self.world.block(other) == Some(block::BED))
            } else if state.id == block::LADDER {
                self.full_support(pos + shapes::wall_support(state.metadata))
            } else if matches!(
                state.id,
                block::TORCH | block::REDSTONE_TORCH | block::REDSTONE_TORCH_OFF
            ) {
                self.full_support(pos + shapes::torch_support(state.metadata))
            } else if matches!(state.id, block::LEVER | block::BUTTON) {
                self.full_support(pos + shapes::torch_support(state.metadata))
            } else if matches!(
                state.id,
                block::REDSTONE_WIRE
                    | block::REPEATER
                    | block::REPEATER_OFF
                    | block::STONE_PRESSURE_PLATE
                    | block::WOOD_PRESSURE_PLATE
            ) {
                self.full_support(pos - IVec4::Y)
            } else if state.id == block::SIGN_POST {
                self.full_support(pos - IVec4::Y)
            } else if state.id == block::WALL_SIGN {
                self.full_support(pos + shapes::wall_support(state.metadata))
            } else {
                true
            };
            if !supported {
                let loot_state = self.paired_loot_state(pos, state);
                self.world.set_block(pos, AIR);
                self.break_building_pair(pos, state);
                self.spawn_block_loot(pos, loot_state, mc4d_core::loot::DropCause::Natural);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn arena() -> (Game, IVec4, Hit) {
        let mut game = Game::new(None, 7, GameMode::Survival);
        let base = game.body.pos.floor().as_ivec4();
        for w in -2..=3 {
            for z in -3..=4 {
                for x in -3..=4 {
                    for y in -1..=3 {
                        game.world.set_block(
                            base + IVec4::new(x, y, z, w),
                            if y == -1 { block::STONE } else { AIR },
                        );
                    }
                }
            }
        }
        game.creatures.clear();
        game.hotbar = [None; 9];
        game.inventory = [None; 27];
        game.body = Body::new(base.as_vec4() + Vec4::new(-2.0, 0.0, 0.5, 0.5));
        game.body.on_ground = true;
        game.building.receive(usize::MAX);
        game.building.pending.clear();
        game.building.queued.clear();
        let hit = Hit {
            block: base - IVec4::Y,
            normal: IVec4::Y,
            dist: 3.0,
        };
        (game, base, hit)
    }
    #[test]
    fn door_pair_consumes_one_item_and_breaking_either_half_drops_one() {
        for upper in [false, true] {
            let (mut game, base, hit) = arena();
            game.hotbar[0] = Some(Slot::new(item::WOOD_DOOR_ITEM, 1));
            game.target = Some(Target::Block(hit));
            let mut input = InputState::default();
            input.press(crate::input::Binding::Mouse(crate::input::MouseBtn::Right));
            game.interact(0.016, &input, &Bindings::default());
            assert!(game.hotbar[0].is_none());
            assert_eq!(game.world.block(base), Some(block::WOOD_DOOR));
            assert_eq!(game.world.block(base + IVec4::Y), Some(block::WOOD_DOOR));
            assert!(game.set_door_open(base + IVec4::Y, true));
            assert_eq!(game.world.block_state(base).unwrap().metadata & 4, 4);
            game.mine_block(
                Hit {
                    block: if upper { base + IVec4::Y } else { base },
                    normal: IVec4::NEG_Z,
                    dist: 3.0,
                },
                5.0,
            );
            game.update_building();
            assert_eq!(game.world.block(base), Some(AIR));
            assert_eq!(game.world.block(base + IVec4::Y), Some(AIR));
            assert!(game.hotbar.iter().all(Option::is_none));
            assert_eq!(
                game.dropped_items
                    .iter()
                    .filter(|d| d.stack.block == item::WOOD_DOOR_ITEM)
                    .map(|d| d.stack.count)
                    .sum::<u32>(),
                1
            );
            assert!(game.dropped_items[0].pickup_delay > 0.0);
        }
    }
    #[test]
    fn chest_placement_rejects_triples_and_blocked_use_cannot_place_through_ui() {
        let (mut game, base, hit) = arena();
        assert!(game.place_building(hit, Slot::new(block::CHEST, 1)));
        let second = Hit {
            block: base,
            normal: IVec4::W,
            dist: 2.0,
        };
        assert!(game.place_building(second, Slot::new(block::CHEST, 1)));
        assert!(!game.place_building(
            Hit {
                block: base + IVec4::W,
                normal: IVec4::W,
                dist: 2.0
            },
            Slot::new(block::CHEST, 1)
        ));
        assert_eq!(game.world.block(base + IVec4::W * 2), Some(AIR));
        assert!(!game.place_building(
            Hit {
                block: base,
                normal: IVec4::X,
                dist: 2.0
            },
            Slot::new(block::CHEST, 1)
        ));
        game.world
            .set_block(base + IVec4::W + IVec4::Y, block::STONE);
        game.hotbar[0] = Some(Slot::new(block::DIRT, 1));
        game.target = Some(Target::Block(Hit {
            block: base,
            normal: IVec4::X,
            dist: 2.0,
        }));
        let mut input = InputState::default();
        input.press(crate::input::Binding::Mouse(crate::input::MouseBtn::Right));
        game.interact(0.016, &input, &Bindings::default());
        assert!(game.active_station.is_none());
        assert_eq!(game.world.block(base + IVec4::X), Some(AIR));
        assert_eq!(game.hotbar[0], Some(Slot::new(block::DIRT, 1)));
    }
    #[test]
    fn pair_placement_is_atomic_and_iron_door_requires_power() {
        let (mut game, base, hit) = arena();
        game.world.set_block(base + IVec4::Y, block::STONE);
        assert!(!game.place_building(hit, Slot::new(item::WOOD_DOOR_ITEM, 1)));
        assert_eq!(game.world.block(base), Some(AIR));
        game.world.set_block(base + IVec4::Y, AIR);
        assert!(game.place_building(hit, Slot::new(item::IRON_DOOR_ITEM, 1)));
        game.target = Some(Target::Block(Hit {
            block: base,
            normal: IVec4::NEG_Z,
            dist: 2.0,
        }));
        assert!(!game.use_target_building());
        assert_eq!(game.world.block_state(base).unwrap().metadata & 4, 0);
        assert!(game.set_block_powered(base, true));
        assert_eq!(game.world.block_state(base).unwrap().metadata & 4, 4);
    }
    #[test]
    fn slabs_merge_by_material_and_never_inside_the_player() {
        let (mut game, base, hit) = arena();
        assert!(game.place_building(hit, Slot::with_variant(block::SLAB, 1, 2)));
        let upper = Hit {
            block: base,
            normal: IVec4::Y,
            dist: 2.0,
        };
        game.body.pos = base.as_vec4() + Vec4::splat(0.5);
        assert!(!game.place_building(upper, Slot::with_variant(block::SLAB, 1, 2)));
        game.body.pos.x -= 2.0;
        assert!(game.place_building(upper, Slot::with_variant(block::SLAB, 1, 2)));
        assert_eq!(
            game.world.block_state(base),
            Some(BlockState::new(block::DOUBLE_SLAB, 2))
        );
    }
    #[test]
    fn beds_sleep_set_spawn_and_fail_safely_when_broken() {
        let (mut game, base, hit) = arena();
        assert!(game.place_building(hit, Slot::new(item::BED_ITEM, 1)));
        assert!(!game.sleep_in_bed(base));
        game.environment.world_ticks = 18000;
        game.spawn_creature(
            CreatureKind::Zombie,
            base.as_vec4() + Vec4::new(3.0, 0.0, 0.0, 0.0),
        );
        assert!(game.sleep_in_bed(base), "Beta 1.5 lets you sleep beside monsters");
        assert_eq!(game.environment.world_ticks % 24000, 0);
        let head = game.building.bed_spawn.unwrap();
        assert_eq!(head, base + IVec4::Z);
        game.health = 0;
        game.respawn();
        assert_eq!(game.building.bed_spawn, Some(head));
        assert!(!game.body.is_stuck_states(&|p| game.world.block_state(p)));
        game.world.set_block(head, AIR);
        for _ in 0..8 {
            game.update_building();
        }
        game.health = 0;
        game.respawn();
        assert!(game.building.bed_spawn.is_none());
    }
    #[test]
    fn w_beds_and_supported_ladders_use_four_dimensional_neighbors() {
        let (mut game, base, hit) = arena();
        game.orient.rotate_forward_ana(std::f32::consts::FRAC_PI_2);
        assert!(game.place_building(hit, Slot::new(item::BED_ITEM, 1)));
        assert_eq!(game.world.block(base + IVec4::W), Some(block::BED));
        game.world.set_block(base, AIR);
        game.world.set_block(base + IVec4::W, AIR);
        game.world.set_block(base + IVec4::X, block::STONE);
        assert!(game.place_building(
            Hit {
                block: base + IVec4::X,
                normal: IVec4::NEG_X,
                dist: 2.0
            },
            Slot::new(block::LADDER, 1)
        ));
        assert_eq!(game.world.block_state(base).unwrap().metadata, 4);
        game.world.set_block(base + IVec4::X, AIR);
        for _ in 0..8 {
            game.update_building();
        }
        assert_eq!(game.world.block(base), Some(AIR));
        assert!(
            game.dropped_items
                .iter()
                .any(|d| d.stack.block == item::ItemId::from(block::LADDER))
        );
    }
    #[test]
    fn sign_editor_payload_and_text_persist_with_unicode_limits() {
        let (mut game, base, hit) = arena();
        assert!(game.place_building(hit, Slot::new(item::SIGN, 1)));
        assert_eq!(
            game.world.block_state(base).unwrap().metadata,
            8,
            "sign front faces the player, as original ItemSign adds180degrees"
        );
        let (pos, lines) = game.take_sign_editor_request().unwrap();
        assert_eq!(pos, base);
        assert!(lines.iter().all(String::is_empty));
        assert!(game.save_sign_text(
            pos,
            [
                "四维空间中的一块告示牌测试中文长度限制".into(),
                "hello\nworld".into(),
                "third".into(),
                "fourth".into()
            ]
        ));
        let saved = game.building.snapshot();
        let encoded = toml::to_string(&saved).unwrap();
        let decoded: SavedBuilding = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.signs[0].lines[0].chars().count(), 15);
        assert_eq!(decoded.signs[0].lines[1], "helloworld");
    }
}
