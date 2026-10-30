//! Tool-aware mining and right-click tools. Mining progress belongs to a target and
//! selected tool, so switching blocks, releasing attack or changing tools resets it.
use super::{Game, GameMode, Slot, Target};
use mc4d_core::glam::{IVec4, Vec4};
use mc4d_core::{
    block::{self, AIR},
    item::{self, ToolKind},
    raycast::Hit,
};

impl Game {
    pub(super) fn reset_mining(&mut self) {
        self.mining_target = None;
        self.mining_progress = 0.0;
        self.dig_sound_cooldown = 0.0;
    }
    pub(super) fn wear_selected(&mut self, amount: u16) {
        if self.mode == GameMode::Creative || amount == 0 {
            return;
        }
        let Some(slot) = &mut self.hotbar[self.selected] else {
            return;
        };
        if item::apply_wear(slot, amount) {
            self.progress
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .item(slot.block)
                .depleted += 1;
            if slot.count == 0 {
                self.hotbar[self.selected] = None;
            }
            self.toast("Your tool broke");
        }
    }
    pub(super) fn mine_block(&mut self, hit: Hit, dt: f32) {
        let id = self.world.block(hit.block).unwrap_or(AIR);
        let creative = self.mode == GameMode::Creative;
        if id == AIR || (!creative && !block::info(id).breakable) {
            self.reset_mining();
            return;
        }
        let held = self.hotbar[self.selected].unwrap_or(Slot::EMPTY);
        let key = (
            hit.block,
            id,
            self.selected,
            held.block,
            held.damage,
            held.variant,
        );
        if self.mining_target != Some(key) {
            if id == block::NOTE_BLOCK {
                self.play_note(hit.block);
            }
            if id == block::CAKE && !creative {
                self.eat_cake(hit.block);
                if self.world.block(hit.block) != Some(id) {
                    self.reset_mining();
                    return;
                }
            }
            self.reset_mining();
            self.mining_target = Some(key);
        }
        if !creative {
            if self.dig_sound_cooldown <= 0.0 {
                self.emit_sound(crate::audio::step_sound(id), hit.block.as_vec4(), 0.3, 0.5);
                self.dig_sound_cooldown = 0.2;
            }
            let mut duration = item::mining_seconds(id, held.block);
            if self.body.in_water {
                duration *= 5.0;
            }
            if !self.body.on_ground {
                duration *= 5.0;
            }
            self.mining_progress += if duration <= 0.0 { 1.0 } else { dt / duration };
            if self.mining_progress + 1e-5 < 1.0 {
                return;
            }
        }
        let original_state = self
            .world
            .block_state(hit.block)
            .unwrap_or_else(|| id.into());
        let loot_state = self.paired_loot_state(hit.block, original_state);
        if self.world.set_block(hit.block, AIR) {
            self.emit_block_particles(hit.block,original_state,true);
            self.progress
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .item(id.into())
                .mined += 1;
            self.break_building_pair(hit.block, original_state);
            self.break_station(hit.block);
            if !creative {
                if id == block::TNT {
                    self.spawn_primed_tnt(hit.block, false);
                }
                if item::can_harvest(id, held.block) {
                    self.spawn_block_loot(
                        hit.block,
                        loot_state,
                        mc4d_core::loot::DropCause::Harvest,
                    );
                }
                let wear = item::tool(held.block).map_or(0, |t| match t.kind {
                    ToolKind::Sword => 2,
                    ToolKind::Hoe => 0,
                    _ => 1,
                });
                self.wear_selected(wear);
                if id == block::ICE
                    && self
                        .world
                        .block(hit.block - IVec4::Y)
                        .is_some_and(|below| block::is_solid(below) || block::is_fluid(below))
                {
                    self.world.set_block(hit.block, block::WATER);
                }
            }
            self.break_cooldown = if creative { 0.18 } else { 0.25 };
            self.stats.broken += 1;
            self.emit_sound(crate::audio::step_sound(id), hit.block.as_vec4(), 0.8, 0.8);
        }
        self.reset_mining();
    }
    pub fn use_selected_tool(&mut self) -> bool {
        let Some(held) = self.hotbar[self.selected] else {
            return false;
        };
        let Some(Target::Block(hit)) = self.target else {
            return false;
        };
        let Some(id) = self.world.block(hit.block) else {
            return false;
        };
        // aw.a returns an empty bucket for milk only after a block ray hit;
        // Beta1.5 has no potion effects or modern milk-drinking animation.
        if held.block == item::MILK {
            self.hotbar[self.selected] = Some(Slot::new(item::BUCKET, 1));
            self.start_hand_use();
            return true;
        }

        // ke.a: grass tills under any non-solid material (air, fluids, plants,
        // snow layers); dirt tills unconditionally, as in the original.
        if item::tool(held.block).is_some_and(|t| t.kind == ToolKind::Hoe)
            && (id == block::DIRT
                || id == block::GRASS
                    && self
                        .world
                        .block(hit.block + IVec4::Y)
                        .is_some_and(|above| !mc4d_core::ecology::material_blocks_movement(above)))
        {
            if self.world.set_block(hit.block, block::FARMLAND) {
                self.start_hand_swing();
                self.wear_selected(1);
                if id == block::GRASS && self.rand() < 0.125 {
                    self.spawn_drop(
                        Slot::new(item::SEEDS, 1),
                        hit.block.as_vec4() + Vec4::new(0.5, 1.2, 0.5, 0.5),
                        Vec4::Y * 2.0,
                        0.5,
                    );
                }
                return true;
            }
        }
        // wu.a: fire only on the clicked face's air cell (TNT is not primed
        // directly; the fire's burn table does that), and every use wears it.
        if held.block == item::FLINT_AND_STEEL {
            let pos = hit.block + hit.normal;
            if hit.normal != IVec4::ZERO && self.world.block(pos) == Some(AIR) {
                self.ignite(pos);
                self.emit_sound("fire.ignite", pos.as_vec4(), 1.0, 1.0);
            }
            self.start_hand_swing();
            self.wear_selected(1);
            return true;
        }
        if held.block == item::DYE && held.variant == 15 {
            let used = match id {
                // js.a consumes the dye even on ripe crops.
                block::CROPS => {
                    self.world
                        .set_block_state(hit.block, block::BlockState::new(block::CROPS, 7));
                    true
                }
                block::SAPLING => {
                    let species = self.world.block_state(hit.block).unwrap().metadata & 3;
                    self.grow_sapling(hit.block, species);
                    true
                }
                _ => false,
            };
            if used {
                self.start_hand_use();
                if self.mode == GameMode::Survival {
                    let stack = self.hotbar[self.selected].as_mut().unwrap();
                    stack.count -= 1;
                    if stack.count == 0 {
                        self.hotbar[self.selected] = None;
                    }
                }
                return true;
            }
        }
        if held.block == item::BUCKET {
            let filled = match block::fluid(id) {
                Some((block::FluidKind::Water, 0)) => item::WATER_BUCKET,
                Some((block::FluidKind::Lava, 0)) => item::LAVA_BUCKET,
                _ => return false,
            };
            if self.world.set_block(hit.block, AIR) {
                self.start_hand_use();
                if self.mode == GameMode::Survival {
                    self.hotbar[self.selected] = Some(Slot::new(filled, 1));
                }
                return true;
            }
        }
        let fluid = match held.block {
            item::WATER_BUCKET => block::WATER,
            item::LAVA_BUCKET => block::LAVA,
            _ => return false,
        };
        let pos = hit.block + hit.normal;
        if hit.normal != IVec4::ZERO
            // aw.a: any cell whose material is not solid (air, fluids, plants,
            // torches, fire, snow layers, redstone parts) is overwritten.
            && self
                .world
                .block(pos)
                .is_some_and(|id| !mc4d_core::ecology::material_blocks_movement(id))
            && self.world.set_block(pos, fluid)
        {
            self.start_hand_use();
            if self.mode == GameMode::Survival {
                self.hotbar[self.selected] = Some(Slot::new(item::BUCKET, 1));
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup(id: u8, held: item::ItemId) -> (Game, Hit) {
        let mut game = Game::new(None, 7, GameMode::Survival);
        game.body.on_ground = true;
        let pos = game.body.pos.floor().as_ivec4() + IVec4::Z * 2;
        assert!(game.world.set_block(pos, id));
        if held != 0 {
            game.hotbar[0] = Some(Slot::new(held, 1));
        }
        let hit = Hit {
            block: pos,
            normal: -IVec4::Z,
            dist: 2.0,
        };
        (game, hit)
    }
    #[test]
    fn mining_requires_hold_and_tool_tiers_control_drops() {
        let (mut game, hit) = setup(block::STONE, AIR.into());
        game.mine_block(hit, 0.1);
        assert_eq!(game.world.block(hit.block), Some(block::STONE));
        game.mine_block(hit, 7.5);
        assert_eq!(game.world.block(hit.block), Some(AIR));
        assert!(game.hotbar.iter().all(Option::is_none));
        let (mut game, hit) = setup(block::IRON_ORE, item::STONE_PICKAXE);
        game.mine_block(hit, 2.0);
        assert_eq!(game.world.block(hit.block), Some(AIR));
        assert!(
            game.dropped_items
                .iter()
                .any(|d| d.stack.block == item::ItemId::from(block::IRON_ORE))
        );
        assert_eq!(game.hotbar[0].unwrap().damage, 1);
    }
    #[test]
    fn releasing_or_changing_target_resets_and_worn_tool_breaks() {
        let (mut game, hit) = setup(block::STONE, item::WOOD_PICKAXE);
        game.mine_block(hit, 0.5);
        assert!(game.mining_progress > 0.0);
        game.reset_mining();
        assert_eq!(game.mining_progress, 0.0);
        game.hotbar[0].as_mut().unwrap().damage = item::max_durability(item::WOOD_PICKAXE);
        game.mine_block(hit, 2.0);
        assert!(
            game.hotbar
                .iter()
                .flatten()
                .all(|s| s.block != item::WOOD_PICKAXE)
        );
    }
    #[test]
    fn buckets_move_sources_and_hoe_tills_without_placing_items() {
        let (mut game, hit) = setup(block::WATER, item::BUCKET);
        game.target = Some(Target::Block(hit));
        assert!(game.use_selected_tool());
        assert_eq!(game.world.block(hit.block), Some(AIR));
        assert_eq!(game.hotbar[0].unwrap().block, item::WATER_BUCKET);
        game.world.set_block(hit.block, block::DIRT);
        game.hotbar[0] = Some(Slot::new(item::WOOD_HOE, 1));
        game.world.set_block(hit.block + IVec4::Y, AIR);
        assert!(game.use_selected_tool());
        assert_eq!(game.world.block(hit.block), Some(block::FARMLAND));
        assert_eq!(game.hotbar[0].unwrap().damage, 1);
    }
}
