//! Persistent containers and furnace simulation; child module of `game`.
use super::{Game, Slot, Target};
use mc4d_core::glam::{IVec4, Vec4};
use mc4d_core::{block, crafting, item};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StationKind {
    CraftingTable,
    Chest,
    Furnace,
    Dispenser,
}

impl StationKind {
    pub fn from_block(id: u8) -> Option<Self> {
        match id {
            block::CRAFTING_TABLE => Some(Self::CraftingTable),
            block::CHEST => Some(Self::Chest),
            block::DISPENSER => Some(Self::Dispenser),
            block::FURNACE | block::LIT_FURNACE => Some(Self::Furnace),
            _ => None,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::CraftingTable => "Crafting Table",
            Self::Chest => "Chest",
            Self::Furnace => "Furnace",
            Self::Dispenser => "Dispenser",
        }
    }
    fn slot_count(self) -> usize {
        match self {
            Self::CraftingTable => 0,
            Self::Chest => 27,
            Self::Furnace => 3,
            Self::Dispenser => 9,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Station {
    pub pos: [i32; 4],
    pub kind: StationKind,
    /// Empty slots use AIR/count0 so TOML arrays have no null values.
    pub(super) slots: Vec<Slot>,
    #[serde(default)]
    burn_ticks: u32,
    #[serde(default)]
    fuel_ticks: u32,
    #[serde(default)]
    cook_ticks: u32,
    #[serde(default)]
    tick_fraction: f32,
}

impl Station {
    pub fn new(pos: IVec4, kind: StationKind) -> Self {
        Self {
            pos: pos.to_array(),
            kind,
            slots: vec![Slot::EMPTY; kind.slot_count()],
            burn_ticks: 0,
            fuel_ticks: 0,
            cook_ticks: 0,
            tick_fraction: 0.0,
        }
    }
    pub fn sanitize(&mut self) {
        self.slots.resize(self.kind.slot_count(), Slot::EMPTY);
        self.slots.truncate(self.kind.slot_count());
        for s in &mut self.slots {
            *s = s.normalized().unwrap_or(Slot::EMPTY);
        }
        self.burn_ticks = self.burn_ticks.min(20_000);
        self.fuel_ticks = self.fuel_ticks.min(20_000);
        self.cook_ticks = self.cook_ticks.min(199);
        self.tick_fraction = if self.tick_fraction.is_finite() {
            self.tick_fraction.clamp(0.0, 0.999)
        } else {
            0.0
        };
    }
    pub(super) fn migrate_legacy_items(&mut self) {
        for stack in &mut self.slots {
            *stack = item::migrate_legacy(*stack);
        }
    }
    fn can_smelt(&self) -> bool {
        if self.kind != StationKind::Furnace || self.slots[0].count == 0 {
            return false;
        }
        let Some(output) = crafting::smelting_result(self.slots[0].block) else {
            return false;
        };
        let slot = self.slots[2];
        slot.count == 0 || (slot.same_kind(output) && slot.count < item::stack_limit(output.block))
    }
    pub fn update(&mut self, dt: f32) {
        if self.kind != StationKind::Furnace {
            return;
        }
        self.tick_fraction += dt.max(0.0).min(1.0) * 20.0;
        let ticks = (self.tick_fraction + 1e-5).floor() as u32;
        self.tick_fraction = (self.tick_fraction - ticks as f32).max(0.0);
        for _ in 0..ticks {
            self.burn_ticks = self.burn_ticks.saturating_sub(1);
            if self.burn_ticks == 0 && self.can_smelt() && self.slots[1].count > 0 {
                let duration = (crafting::fuel_seconds(self.slots[1].block) * 20.0) as u32;
                if duration > 0 {
                    self.burn_ticks = duration;
                    self.fuel_ticks = duration;
                    self.slots[1].count -= 1;
                    if self.slots[1].count == 0 {
                        self.slots[1] = Slot::EMPTY;
                    }
                }
            }
            if self.burn_ticks > 0 && self.can_smelt() {
                self.cook_ticks += 1;
                if self.cook_ticks >= 200 {
                    let output = crafting::smelting_result(self.slots[0].block).expect("smeltable");
                    if self.slots[2].count == 0 {
                        self.slots[2] = output;
                    } else {
                        self.slots[2].count += 1;
                    }
                    self.slots[0].count -= 1;
                    if self.slots[0].count == 0 {
                        self.slots[0] = Slot::EMPTY;
                    }
                    self.cook_ticks = 0;
                }
            } else {
                self.cook_ticks = 0;
            }
        }
    }
}

impl Game {
    pub(super) fn materialize_chunk_chests(&mut self,c:IVec4){
        let positions:Vec<_>=self.world.chunks.get(&c).into_iter().flat_map(|ch|ch.generated_chests.iter()).map(|(index,_)|{let i=*index as i32;c*16+IVec4::new(i&15,(i>>4)&15,(i>>8)&15,(i>>12)&15)}).collect();
        for pos in positions{self.materialize_generated_chest(pos);}
    }
    fn materialize_generated_chest(&mut self,pos:IVec4){
        let c=mc4d_core::chunk::chunk_of(pos);let local=mc4d_core::chunk::local_index(mc4d_core::chunk::local_of(pos))as u16;
        let slots=self.world.chunks.get_mut(&c).and_then(|chunk|chunk.generated_chests.iter().position(|(i,_)|*i==local).map(|i|chunk.generated_chests.swap_remove(i).1));
        if let Some(slots)=slots {if !self.stations.iter().any(|s|s.pos==pos.to_array()){let mut station=Station::new(pos,StationKind::Chest);station.slots=slots;self.stations.push(station);}}
    }
    /// Each half keeps its own27 saved slots. Opening only creates a canonical
    /// combined view; merging/breaking chests never moves or duplicates stacks.
    pub fn open_target_station(&mut self) -> bool {
        if self.aimed_vehicle().is_some_and(|i|self.vehicles.saved.entities[i].kind==super::vehicles::VehicleKind::ChestCart){return self.interact_vehicle(false);}
        let Some(Target::Block(hit)) = self.target else {
            return false;
        };
        let Some(kind) = self
            .world
            .block(hit.block)
            .and_then(StationKind::from_block)
        else {
            return false;
        };
        let (first, second) = if kind == StationKind::Chest {
            let Some(pair) =
                mc4d_core::containers::chest_pair(hit.block, &|p| self.world.block_state(p))
            else {
                return false;
            };
            if !mc4d_core::containers::can_open_chest(pair, &|p| self.world.block_state(p)) {
                return false;
            }
            (pair.first, pair.second)
        } else {
            (hit.block, None)
        };
        self.close_station();
        for pos in std::iter::once(first).chain(second) {
            if kind==StationKind::Chest{self.materialize_generated_chest(pos);}
            if self
                .stations
                .iter()
                .any(|s| s.pos == pos.to_array() && s.kind != kind)
            {
                self.break_station(pos);
            }
            if !self.stations.iter().any(|s| s.pos == pos.to_array()) {
                self.stations.push(Station::new(pos, kind));
            }
        }
        self.active_station = Some(first);
        self.active_station_secondary = second;
        self.crafting_width = if kind == StationKind::CraftingTable {
            3
        } else {
            2
        };
        true
    }
    fn station_indices(&self) -> [Option<usize>; 2] {
        let first = self
            .active_station
            .and_then(|p| self.stations.iter().position(|s| s.pos == p.to_array()));
        let second = self.active_station_secondary.and_then(|p| {
            self.stations
                .iter()
                .position(|s| s.pos == p.to_array() && s.kind == StationKind::Chest)
        });
        if first.is_none() || self.active_station_secondary.is_some() && second.is_none() {
            [None, None]
        } else {
            [first, second]
        }
    }
    fn station_index(&self) -> Option<usize> {
        self.station_indices()[0]
    }
    fn station_slot_address(&self, mut index: usize) -> Option<(usize, usize)> {
        for station in self.station_indices().into_iter().flatten() {
            let count = self.stations[station].slots.len();
            if index < count {
                return Some((station, index));
            }
            index -= count;
        }
        None
    }
    pub fn station_kind(&self) -> Option<StationKind> {
        if self.vehicles.active_cargo.is_some(){return Some(StationKind::Chest);}
        self.station_index().map(|i| self.stations[i].kind)
    }
    pub fn station_title(&self) -> Option<&'static str> {
        if self.active_station_secondary.is_some() {
            Some("Large chest")
        } else {
            self.station_kind().map(StationKind::title)
        }
    }
    pub fn station_slot_count(&self) -> usize {
        if self.vehicles.active_cargo.is_some(){return 27;}
        self.station_indices()
            .into_iter()
            .flatten()
            .map(|i| self.stations[i].slots.len())
            .sum()
    }
    pub fn station_slot(&self, index: usize) -> Option<Slot> {
        if let Some(i)=self.vehicles.active_cargo{return self.vehicles.saved.entities.get(i)?.cargo.get(index)?.normalized();}
        let (station, local) = self.station_slot_address(index)?;
        self.stations[station].slots[local].normalized()
    }
    pub fn click_station_slot(&mut self, index: usize, cursor: &mut Option<Slot>, split: bool) {
        if let Some(i)=self.vehicles.active_cargo{if let Some(raw)=self.vehicles.saved.entities.get_mut(i).and_then(|v|v.cargo.get_mut(index)){let mut slot=raw.normalized();item::click_slot(&mut slot,cursor,split);*raw=slot.unwrap_or(Slot::EMPTY);}return;}
        let Some((station, index)) = self.station_slot_address(index) else {
            return;
        };
        let kind = self.stations[station].kind;
        let raw = &mut self.stations[station].slots[index];
        let mut slot = raw.normalized();
        if kind == StationKind::Furnace {
            if index == 2 {
                // Output accepts extraction only, never depositing or swapping.
                let Some(mut output) = slot else {
                    return;
                };
                let held = cursor.map_or(0, |s| {
                    if s.same_kind(output) {
                        s.count
                    } else {
                        u32::MAX
                    }
                });
                let room = item::stack_limit(output.block).saturating_sub(held);
                let take = room.min(if split {
                    output.count.div_ceil(2)
                } else {
                    output.count
                });
                if take == 0 {
                    return;
                }
                *cursor = Some(Slot {
                    count: held + take,
                    ..output
                });
                output.count -= take;
                *raw = if output.count > 0 {
                    output
                } else {
                    Slot::EMPTY
                };
                self.record_smelted(Slot {
                    count: take,
                    ..output
                });
                return;
            }
            if let Some(held) = cursor {
                let allowed = if index == 0 {
                    crafting::smelting_result(held.block).is_some()
                } else {
                    crafting::fuel_seconds(held.block) > 0.0
                };
                if !allowed {
                    return;
                }
            }
        }
        item::click_slot(&mut slot, cursor, split);
        *raw = slot.unwrap_or(Slot::EMPTY);
    }
    pub fn furnace_progress(&self) -> Option<(f32, f32)> {
        let i = self.station_index()?;
        let s = &self.stations[i];
        (s.kind == StationKind::Furnace).then(|| {
            (
                s.cook_ticks as f32 / 200.0,
                if s.fuel_ticks == 0 {
                    0.0
                } else {
                    s.burn_ticks as f32 / s.fuel_ticks as f32
                },
            )
        })
    }
    pub fn quick_transfer_station(&mut self, index: usize) {
        if let Some(i)=self.vehicles.active_cargo{if let Some(stack)=self.station_slot(index){let left=self.insert_stack_range(stack,0..36);self.vehicles.saved.entities[i].cargo[index]=left.unwrap_or(Slot::EMPTY);}return;}
        let Some((station, local)) = self.station_slot_address(index) else {
            return;
        };
        let Some(stack) = self.stations[station].slots[local].normalized() else {
            return;
        };
        let left = self.insert_stack_range(stack, 0..36);
        self.stations[station].slots[local] = left.unwrap_or(Slot::EMPTY);
        let taken = stack.count - left.map_or(0, |s| s.count);
        if taken > 0 && local == 2 && self.stations[station].kind == StationKind::Furnace {
            self.record_smelted(Slot {
                count: taken,
                ..stack
            });
        }
    }
    pub fn quick_transfer_to_station(&mut self, inventory_index: usize) {
        if let Some(i)=self.vehicles.active_cargo{let Some(mut stack)=self.inventory_slot(inventory_index)else{return};for merge in [true,false]{for target in &mut self.vehicles.saved.entities[i].cargo{let room=if merge&&target.count>0&&target.same_kind(stack){item::stack_limit(stack.block).saturating_sub(target.count)}else if !merge&&target.count==0{item::stack_limit(stack.block)}else{0};let take=room.min(stack.count);if take>0{*target=Slot{count:target.count+take,..stack};stack.count-=take;}}}if let Some(source)=self.slot_mut(inventory_index){*source=(stack.count>0).then_some(stack);}return;}
        let Some(station) = self.station_index() else {
            return;
        };
        let Some(mut stack) = self.inventory_slot(inventory_index) else {
            return;
        };
        let kind = self.stations[station].kind;
        let slots: Vec<usize> = match kind {
            StationKind::Chest => (0..self.station_slot_count()).collect(),
            StationKind::Dispenser => (0..9).collect(),
            StationKind::Furnace if crafting::smelting_result(stack.block).is_some() => vec![0],
            StationKind::Furnace if crafting::fuel_seconds(stack.block) > 0.0 => vec![1],
            _ => return,
        };
        for merge in [true, false] {
            for &index in &slots {
                let Some((part, local)) = self.station_slot_address(index) else {
                    continue;
                };
                let target = &mut self.stations[part].slots[local];
                let room = if merge && target.count > 0 && target.same_kind(stack) {
                    item::stack_limit(stack.block).saturating_sub(target.count)
                } else if !merge && target.count == 0 {
                    item::stack_limit(stack.block)
                } else {
                    0
                };
                let take = room.min(stack.count);
                if take > 0 {
                    target.block = stack.block;
                    target.damage = stack.damage;
                    target.variant = stack.variant;
                    target.count += take;
                    stack.count -= take;
                }
            }
        }
        if let Some(source) = self.slot_mut(inventory_index) {
            *source = (stack.count > 0).then_some(stack);
        }
    }
    pub fn crafting_output(&self) -> Option<Slot> {
        crafting::recipe(&self.crafting_grid, self.crafting_width).map(|r| r.output)
    }
    pub fn click_crafting_slot(&mut self, index: usize, cursor: &mut Option<Slot>, split: bool) {
        if index < self.crafting_width * self.crafting_width {
            item::click_slot(&mut self.crafting_grid[index], cursor, split);
        }
    }
    pub fn quick_transfer_crafting(&mut self, index: usize) {
        let Some(stack) = self.crafting_grid.get(index).copied().flatten() else {
            return;
        };
        self.crafting_grid[index] = self.insert_stack_range(stack, 0..36);
    }

    pub fn take_crafting_output(&mut self, cursor: &mut Option<Slot>) -> bool {
        let output = self.crafting_output();
        let taken = crafting::take_output(&mut self.crafting_grid, self.crafting_width, cursor);
        if taken {
            if let Some(output) = output {
                self.record_crafted(output);
            }
        }
        taken
    }
    pub fn craft_to_inventory(&mut self) -> bool {
        let before = (self.hotbar, self.inventory, self.crafting_grid);
        let mut output = None;
        if !crafting::take_output(&mut self.crafting_grid, self.crafting_width, &mut output) {
            return false;
        }
        if self
            .insert_stack_range(output.expect("crafted output"), 0..36)
            .is_some()
        {
            (self.hotbar, self.inventory, self.crafting_grid) = before;
            return false;
        }
        self.record_crafted(output.expect("crafted output"));
        true
    }
    /// Return ingredients before changing containers, closing inventory or saving.
    pub fn close_station(&mut self) {
        let grid = std::mem::replace(&mut self.crafting_grid, [None; 9]);
        for stack in grid.into_iter().flatten() {
            if let Some(left) = self.insert_stack_range(stack, 0..36) {
                self.spawn_drop(left, self.eye(), Vec4::ZERO, 0.5);
            }
        }
        self.vehicles.active_cargo=None;
        self.active_station = None;
        self.active_station_secondary = None;
        self.crafting_width = 2;
    }
    pub(super) fn update_stations(&mut self, dt: f32) {
        if let Some(primary) = self.active_station {
            let kind = self.station_kind();
            let parts = std::iter::once(primary).chain(self.active_station_secondary);
            if parts.into_iter().any(|p| {
                self.world.block(p).and_then(StationKind::from_block) != kind
                    || (self.body.pos - (p.as_vec4() + Vec4::splat(0.5))).length_squared() > 64.0
            }) {
                self.close_station();
            }
        }
        for station in &mut self.stations {
            let pos = IVec4::from_array(station.pos);
            if let Some(state) = self.world.block_state(pos)
                && StationKind::from_block(state.id) == Some(station.kind)
            {
                station.update(dt);
                if station.kind == StationKind::Furnace {
                    let id = if station.burn_ticks > 0 {
                        block::LIT_FURNACE
                    } else {
                        block::FURNACE
                    };
                    if id != state.id {
                        self.world.set_block_state(
                            pos,
                            mc4d_core::block::BlockState::new(id, state.metadata),
                        );
                    }
                }
            }
        }
    }
    pub(super) fn break_station(&mut self, pos: IVec4) {
        self.materialize_generated_chest(pos);
        if self.active_station == Some(pos) || self.active_station_secondary == Some(pos) {
            self.close_station();
        }
        if let Some(index) = self.stations.iter().position(|s| s.pos == pos.to_array()) {
            let station = self.stations.swap_remove(index);
            for mut stack in station.slots.into_iter().filter_map(Slot::normalized) {
                if station.kind == StationKind::Chest {
                    let offset = Vec4::new(self.rand(), self.rand(), self.rand(), self.rand())
                        * 0.8
                        + Vec4::splat(0.1);
                    while stack.count > 0 {
                        let count = (10 + (self.rand() * 21.0) as u32).min(stack.count);
                        let velocity = Vec4::new(
                            self.gaussian(),
                            self.gaussian() + 4.0,
                            self.gaussian(),
                            self.gaussian(),
                        );
                        self.spawn_drop(
                            Slot { count, ..stack },
                            pos.as_vec4() + offset,
                            velocity,
                            0.5,
                        );
                        stack.count -= count;
                    }
                } else {
                    self.spawn_drop(stack, pos.as_vec4() + Vec4::splat(0.5), Vec4::Y * 2.0, 0.5);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stack(block: impl Into<item::ItemId>, count: u32) -> Slot {
        Slot::new(block, count)
    }
    #[test]
    fn furnace_requires_time_fuel_and_output_room() {
        let mut f = Station::new(IVec4::ZERO, StationKind::Furnace);
        f.slots[0] = stack(block::SAND, 2);
        for _ in 0..20 {
            f.update(1.0);
        }
        assert_eq!(f.slots[0].count, 2);
        f.slots[1] = stack(item::COAL, 1);
        for _ in 0..9 {
            f.update(1.0);
        }
        assert_eq!(f.slots[2].count, 0);
        f.update(1.0);
        assert_eq!(f.slots[2], stack(block::GLASS, 1));
        assert_eq!(f.slots[0].count, 1);
        assert_eq!(f.slots[1].count, 0);
        f.slots[2].count = 64;
        let burn = f.burn_ticks;
        f.update(1.0);
        assert_eq!(f.slots[0].count, 1);
        assert!(
            f.burn_ticks < burn,
            "lit fuel keeps burning with blocked output"
        );
    }
    #[test]
    fn furnace_state_roundtrip_preserves_progress() {
        let mut f = Station::new(IVec4::new(-4, 51, 2, 9), StationKind::Furnace);
        f.slots[0] = stack(block::IRON_ORE, 2);
        f.slots[1] = stack(item::COAL, 1);
        for _ in 0..4 {
            f.update(1.0);
        }
        let encoded = toml::to_string(&f).unwrap();
        let mut restored: Station = toml::from_str(&encoded).unwrap();
        restored.sanitize();
        for _ in 0..6 {
            restored.update(1.0);
        }
        assert_eq!(restored.slots[2], stack(item::IRON_INGOT, 1));
        assert_eq!(restored.pos, f.pos);
    }

    #[test]
    fn furnace_world_state_drives_light_and_preserves_facing() {
        let mut game = Game::new(None, 7, super::super::GameMode::Survival);
        let pos = game.body.pos.floor().as_ivec4() + IVec4::Z * 2;
        game.world
            .set_block_state(pos, mc4d_core::block::BlockState::new(block::FURNACE, 5));
        let mut furnace = Station::new(pos, StationKind::Furnace);
        furnace.slots[0] = stack(block::SAND, 1);
        furnace.slots[1] = stack(item::COAL, 1);
        game.stations.push(furnace);
        game.update_stations(1.0);
        let lit = game.world.block_state(pos).unwrap();
        assert_eq!(lit.id, block::LIT_FURNACE);
        assert_eq!(lit.metadata, 5);
        assert_eq!(block::light_emission(lit), 13);
        for _ in 0..80 {
            game.update_stations(1.0);
        }
        let cold = game.world.block_state(pos).unwrap();
        assert_eq!(cold.id, block::FURNACE);
        assert_eq!(cold.metadata, 5);
        assert_eq!(game.stations[0].slots[2], stack(block::GLASS, 1));
    }

    #[test]
    fn chest_transfer_and_break_conserve_stacks() {
        let mut game = Game::new(None, 7, super::super::GameMode::Survival);
        let pos = game.body.pos.floor().as_ivec4();
        game.stations.push(Station::new(pos, StationKind::Chest));
        game.active_station = Some(pos);
        game.hotbar[0] = Some(stack(block::PLANKS, 64));
        game.quick_transfer_to_station(0);
        assert_eq!(game.hotbar[0], None);
        assert_eq!(game.station_slot(0), Some(stack(block::PLANKS, 64)));
        let mut held = Some(stack(block::PLANKS, 63));
        game.click_station_slot(0, &mut held, false);
        assert_eq!(
            held,
            Some(stack(block::PLANKS, 63)),
            "full target cannot absorb more"
        );
        game.quick_transfer_station(0);
        assert_eq!(game.station_slot(0), None);
        assert_eq!(game.hotbar[0], Some(stack(block::PLANKS, 64)));
        game.quick_transfer_to_station(0);
        game.break_station(pos);
        assert!(game.stations.is_empty());
        assert_eq!(
            game.dropped_items
                .iter()
                .map(|d| d.stack.count)
                .sum::<u32>(),
            64
        );
    }

    #[test]
    fn furnace_output_rejects_insertion() {
        let mut game = Game::new(None, 7, super::super::GameMode::Survival);
        let pos = game.body.pos.floor().as_ivec4();
        game.stations.push(Station::new(pos, StationKind::Furnace));
        game.active_station = Some(pos);
        let mut cursor = Some(stack(block::GLASS, 1));
        game.click_station_slot(2, &mut cursor, false);
        assert_eq!(game.station_slot(2), None);
        assert_eq!(cursor, Some(stack(block::GLASS, 1)));
    }
    #[test]
    fn furnace_achievement_requires_actual_extraction_and_counts_transferred_items() {
        let mut game = Game::menu_preview(7);
        for index in [0, 1, 2, 3, 4] {
            game.award_achievement(index);
        }
        let pos = IVec4::ZERO;
        let mut station = Station::new(pos, StationKind::Furnace);
        station.slots[2] = stack(item::IRON_INGOT, 3);
        game.stations.push(station);
        game.active_station = Some(pos);
        game.hotbar.fill(Some(stack(block::STONE, 64)));
        game.inventory.fill(Some(stack(block::STONE, 64)));
        game.quick_transfer_station(2);
        assert!(!game.progress.lock().unwrap().unlocked.contains(&5));
        assert_eq!(game.station_slot(2).unwrap().count, 3);
        game.hotbar[0] = Some(stack(item::IRON_INGOT, 63));
        game.quick_transfer_station(2);
        assert_eq!(game.station_slot(2).unwrap().count, 2);
        assert!(game.progress.lock().unwrap().unlocked.contains(&5));
        assert_eq!(
            game.progress.lock().unwrap().items[&item::IRON_INGOT.to_string()].crafted,
            1
        );
        let mut cursor = None;
        game.click_station_slot(2, &mut cursor, true);
        assert_eq!(cursor.unwrap().count, 1);
        assert_eq!(
            game.progress.lock().unwrap().items[&item::IRON_INGOT.to_string()].crafted,
            2
        );
    }
}

#[cfg(test)]
mod double_chest_tests {
    use super::*;
    use crate::game::{GameMode, Target};
    use mc4d_core::{chunk::Chunk, raycast::Hit};
    fn pair(direction: IVec4) -> (Game, IVec4, IVec4) {
        let mut g = Game::menu_preview(7);
        g.world
            .insert_chunk(IVec4::ZERO, Chunk::uniform(block::AIR));
        let p = IVec4::new(7, 8, 7, 7);
        let q = p + direction;
        g.world.set_block(p, block::CHEST);
        g.world.set_block(q, block::CHEST);
        g.body.pos = p.as_vec4() + Vec4::Z * 3.0;
        g.mode = GameMode::Survival;
        (g, p, q)
    }
    fn open(g: &mut Game, p: IVec4) -> bool {
        g.target = Some(Target::Block(Hit {
            block: p,
            normal: IVec4::Y,
            dist: 3.0,
        }));
        g.open_target_station()
    }
    #[test]
    fn both_halves_have_stable_54_slot_view_and_transfer_across_x_z_w() {
        for axis in [IVec4::X, IVec4::Z, IVec4::W] {
            let (mut g, p, q) = pair(axis);
            assert!(open(&mut g, q));
            assert_eq!(g.station_slot_count(), 54);
            assert_eq!(g.active_station, Some(p));
            let mut held = Some(Slot::with_variant(block::WOOL, 8, 14));
            g.click_station_slot(53, &mut held, false);
            assert!(held.is_none());
            g.close_station();
            assert!(open(&mut g, p));
            assert_eq!(
                g.station_slot(53),
                Some(Slot::with_variant(block::WOOL, 8, 14))
            );
            g.quick_transfer_station(53);
            assert_eq!(g.hotbar[0], Some(Slot::with_variant(block::WOOL, 8, 14)));
            assert!(g.station_slot(53).is_none());
            let first = g.station_index().unwrap();
            g.stations[first].slots.fill(Slot::new(block::STONE, 64));
            g.quick_transfer_to_station(0);
            assert!(g.hotbar[0].is_none());
            assert_eq!(
                g.station_slot(27),
                Some(Slot::with_variant(block::WOOL, 8, 14))
            );
            let saved: Vec<_> = g
                .stations
                .iter()
                .map(|station| toml::to_string(station).unwrap())
                .collect();
            g.close_station();
            g.stations = saved
                .into_iter()
                .map(|text| toml::from_str::<Station>(&text).unwrap())
                .collect();
            assert!(open(&mut g, q));
            assert_eq!(
                g.station_slot(27),
                Some(Slot::with_variant(block::WOOL, 8, 14))
            );
            g.world.set_block(p, block::AIR);
            g.break_station(p);
            assert!(g.active_station.is_none());
            assert!(g.active_station_secondary.is_none());
            assert_eq!(
                g.dropped_items.iter().map(|d| d.stack.count).sum::<u32>(),
                27 * 64
            );
            assert!(
                g.dropped_items
                    .iter()
                    .all(|d| d.stack.block == item::ItemId::from(block::STONE)
                        && d.stack.count <= 30)
            );
            assert!(open(&mut g, q));
            assert_eq!(g.station_slot_count(), 27);
            assert_eq!(
                g.station_slot(0),
                Some(Slot::with_variant(block::WOOL, 8, 14))
            );
        }
    }
    #[test]
    fn opaque_roof_on_either_half_blocks_access_but_glass_does_not() {
        let (mut g, p, q) = pair(IVec4::W);
        g.world.set_block(q + IVec4::Y, block::STONE);
        assert!(!open(&mut g, p));
        assert!(g.active_station.is_none());
        g.world.set_block(q + IVec4::Y, block::GLASS);
        assert!(open(&mut g, p));
        g.close_station();
        g.world.set_block(p + IVec4::Y, block::STONE);
        assert!(!open(&mut g, q));
    }
}

#[cfg(test)]
mod chest_save_coverage {
    use super::*;
    use crate::game::GameMode;
    use mc4d_core::{chunk::Chunk, raycast::Hit};
    fn open(g: &mut Game, p: IVec4) {
        g.target = Some(Target::Block(Hit {
            block: p,
            normal: IVec4::Y,
            dist: 3.0,
        }));
        assert!(g.open_target_station());
    }
    #[test]
    fn every_catalogue_variant_and_worn_tool_roundtrips_through_second_half() {
        let mut g = Game::menu_preview(7);
        g.world
            .insert_chunk(IVec4::ZERO, Chunk::uniform(block::AIR));
        let p = IVec4::splat(8);
        g.world.set_block(p, block::CHEST);
        g.world.set_block(p + IVec4::W, block::CHEST);
        open(&mut g, p);
        for mut stack in item::creative_stacks() {
            stack.count = item::stack_limit(stack.block);
            let durability = item::max_durability(stack.block);
            if durability > 0 {
                stack.damage = durability / 2;
            }
            g.hotbar = [None; 9];
            g.inventory = [None; 27];
            for station in &mut g.stations {
                station.slots.fill(Slot::EMPTY);
            }
            let mut cursor = Some(stack);
            g.click_station_slot(53, &mut cursor, false);
            assert!(cursor.is_none(), "{stack:?}");
            let second = g.station_indices()[1].unwrap();
            let mut restored: Station =
                toml::from_str(&toml::to_string(&g.stations[second]).unwrap()).unwrap();
            restored.sanitize();
            g.stations[second] = restored;
            assert_eq!(g.station_slot(53), Some(stack));
            g.quick_transfer_station(53);
            assert_eq!(g.hotbar[0], Some(stack));
            g.quick_transfer_to_station(0);
            assert!(g.hotbar[0].is_none());
            assert_eq!(g.station_slot(0), Some(stack));
            g.click_station_slot(0, &mut cursor, false);
            assert_eq!(cursor, Some(stack));
            assert!(g.station_slot(0).is_none());
        }
    }
    #[test]
    fn real_world_save_load_and_half_break_preserve_both_inventories_and_drops() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/test-worlds/double-chest-{}-{unique}",
            std::process::id()
        ));
        let mut g = Game::new(Some(path.clone()), 7, GameMode::Survival);
        let p = g.body.pos.floor().as_ivec4() + IVec4::X * 2;
        let q = p + IVec4::W;
        for cell in [p, q] {
            assert!(g.world.set_block(cell, block::CHEST));
            g.world.set_block(cell + IVec4::Y, block::AIR);
        }
        open(&mut g, q);
        let mut first = Some(Slot::new(block::PLANKS, 37));
        g.click_station_slot(0, &mut first, false);
        let tool = Slot {
            damage: 17,
            ..Slot::new(item::IRON_PICKAXE, 1)
        };
        let mut second = Some(tool);
        g.click_station_slot(53, &mut second, false);
        assert!(first.is_none() && second.is_none());
        g.save_checked().unwrap();
        drop(g);
        let mut g = Game::try_load(Some(path.clone()), 0, GameMode::Creative).unwrap();
        open(&mut g, p);
        assert_eq!(g.station_slot_count(), 54);
        assert_eq!(g.station_slot(0), Some(Slot::new(block::PLANKS, 37)));
        assert_eq!(g.station_slot(53), Some(tool));
        g.mine_block(
            Hit {
                block: p,
                normal: IVec4::NEG_Z,
                dist: 3.0,
            },
            50.0,
        );
        assert_eq!(g.world.block(p), Some(block::AIR));
        assert!(g.active_station.is_none());
        assert_eq!(
            g.dropped_items
                .iter()
                .filter(|d| d.stack.block == item::ItemId::from(block::PLANKS))
                .map(|d| d.stack.count)
                .sum::<u32>(),
            37
        );
        g.save_checked().unwrap();
        drop(g);
        let mut g = Game::try_load(Some(path.clone()), 0, GameMode::Creative).unwrap();
        assert_eq!(g.world.block(p), Some(block::AIR));
        open(&mut g, q);
        assert_eq!(g.station_slot_count(), 27);
        assert_eq!(g.station_slot(26), Some(tool));
        assert_eq!(
            g.dropped_items
                .iter()
                .filter(|d| d.stack.block == item::ItemId::from(block::PLANKS))
                .map(|d| d.stack.count)
                .sum::<u32>(),
            37
        );
        drop(g);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn generated_chest_loot_survives_disk_save_and_cannot_be_recreated_by_replacement(){
        let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/dungeon-save-tests").join(format!("{}-{stamp}",std::process::id()));
        let mut game=Game::try_load(Some(root.clone()),1337,super::super::GameMode::Survival).unwrap();let c=IVec4::new(-2,2,3,-2);let chunk=game.world.terrain.generate(c);let i=chunk.generated_chests[0].0 as i32;let expected=chunk.generated_chests[0].1.clone();let pos=c*16+IVec4::new(i&15,(i>>4)&15,(i>>8)&15,(i>>12)&15);
        game.world.insert_chunk(c,chunk);game.materialize_chunk_chests(c);assert_eq!(game.stations.iter().find(|s|s.pos==pos.to_array()).unwrap().slots,expected);
        game.world.set_block(c*16,block::BRICK);game.save_checked().unwrap();drop(game);
        let mut restored=Game::try_load(Some(root.clone()),1337,super::super::GameMode::Survival).unwrap();restored.world.load_now(c,restored.save.as_deref().map(|s|s as &dyn mc4d_core::world::ChunkSource));restored.materialize_chunk_chests(c);
        assert_eq!(restored.stations.iter().find(|s|s.pos==pos.to_array()).unwrap().slots,expected);
        restored.break_station(pos);let drops=restored.dropped_items.iter().map(|d|d.stack.count).sum::<u32>();assert_eq!(drops,expected.iter().map(|s|s.count).sum::<u32>());
        restored.world.set_block(pos,block::CHEST);restored.materialize_chunk_chests(c);restored.break_station(pos);assert_eq!(restored.dropped_items.iter().map(|d|d.stack.count).sum::<u32>(),drops);
        drop(restored);std::fs::remove_dir_all(root).unwrap();
    }

}
