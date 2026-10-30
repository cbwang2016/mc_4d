//! Survival health, hazards, food, death and respawn. Food restores health directly;
//! there is no hunger meter in the Beta-era rules used here.
use super::*;
use mc4d_core::item;

pub(super) fn full_health() -> u8 { 20 }

impl Game {
    pub fn is_alive(&self) -> bool { self.health > 0 }

    pub fn armor_points(&self) -> u8 {
        let mut defense = 0;
        let mut remaining = 0u32;
        let mut maximum = 0u32;
        for stack in self.armor.iter().flatten() {
            let max = item::max_durability(stack.block);
            defense += item::armor_defense(stack.block) as u32;
            remaining += max.saturating_sub(stack.damage) as u32;
            maximum += max as u32;
        }
        if maximum == 0 { 0 } else { (((defense.saturating_sub(1) * remaining) / maximum) + 1).min(20) as u8 }
    }

    /// Armor slots are helmet, chestplate, leggings, boots. A cursor may remove
    /// an item or insert a matching armor type; mismatches leave both unchanged.
    pub fn click_armor_slot(&mut self, index: usize, cursor: &mut Option<Slot>) {
        if index >= 4 || cursor.is_some_and(|s| item::armor_slot(s.block) != Some(index)) { return; }
        item::click_slot(&mut self.armor[index], cursor, false);
    }

    /// Returns true when the selected item is food, even at full health, so the
    /// same click cannot also place a block behind the UI or consume extra food.
    pub fn use_selected_food(&mut self) -> bool {
        let Some(stack) = self.hotbar[self.selected] else { return false };
        let Some(healing) = item::food_heal(stack.block) else { return false };
        // Beta 1.5 ItemFood always eats, wasting food at full health.
        if self.is_alive() {
            self.start_hand_use();
            self.health = self.health.saturating_add(healing).min(20);
            if self.mode == GameMode::Survival {
                self.take_selected();
                // Beta's soup use action returns its bowl, independently of
                // the crafting-container remainder mechanism.
                if stack.block == item::MUSHROOM_STEW {
                    self.hotbar[self.selected] = Some(Slot::new(item::BOWL, 1));
                }
            }
        }
        true
    }

    pub fn damage(&mut self, amount: u8, cause: &str) {
        if self.mode == GameMode::Creative || !self.is_alive() || self.invulnerability > 0.0 || amount == 0 { return; }
        let amount=if matches!(cause,"Creature"|"Arrow"|"Explosion") {
            match self.difficulty {crate::settings::Difficulty::Peaceful=>0,crate::settings::Difficulty::Easy=>amount/2+1,
                crate::settings::Difficulty::Normal=>amount,crate::settings::Difficulty::Hard=>amount.saturating_mul(3)/2}
        } else {amount};
        if amount==0 {return;}
        // Fall, drowning and suffocation bypass armor; direct impact and lava do not.
        let armor_applies = !matches!(cause, "Fall" | "Drowned" | "Suffocated" | "Void");
        let armor = if armor_applies { self.armor_points() } else { 0 };
        let scaled = amount as u16 * (25 - armor as u16) + self.damage_remainder;
        let dealt = (scaled / 25).min(255) as u8;
        self.damage_remainder = scaled % 25;
        if armor_applies {
            for slot in &mut self.armor {
                if let Some(stack) = slot {
                    item::apply_wear(stack, amount as u16);
                    if stack.count == 0 { *slot = None; }
                }
            }
        }
        self.health = self.health.saturating_sub(dealt);
        self.record_counter("damage_taken",dealt as u64);
        self.emit_sound("random.hurt",self.eye(),0.8,1.0);
        self.invulnerability = 0.5;
        if self.health == 0 { self.die(cause); }
    }

    fn die(&mut self, cause: &str) {
        self.vehicles.saved.riding=None;self.vehicles.saved.rail_origin=None;
        self.record_counter("deaths",1);
        self.health = 0;
        self.death_reason = cause.into();
        self.body.vel = Vec4::ZERO;
        self.target = None;
        self.close_station();
        let carried: Vec<_> = self.hotbar.iter_mut().chain(self.inventory.iter_mut()).chain(self.armor.iter_mut())
            .filter_map(Option::take).collect();
        for (i, stack) in carried.into_iter().enumerate() {
            let angle = i as f32 * 2.399_963;
            let vel = Vec4::new(angle.cos() * 2.0, 3.0, angle.sin() * 2.0, (angle * 0.7).sin());
            self.spawn_drop(stack, self.body.pos + Vec4::Y * 0.5, vel, 1.0);
        }
    }

    pub fn respawn(&mut self) {
        self.reset_burning();
        self.vehicles.saved.riding=None;self.vehicles.saved.rail_origin=None;
        self.extinguish();
        self.health = 20;
        self.air = 15.0;
        self.death_reason.clear();
        self.invulnerability = 2.0;
        self.hazard_cooldown = 0.0;
        self.fall_distance = 0.0;
        self.damage_remainder = 0;
        self.flying = false;
        self.body = Body::new(self.spawn);
        self.load_around_now(24.0);
        self.validate_bed_respawn();
        self.body = Body::new(self.spawn);
        self.load_around_now(24.0);
        for _ in 0..64 {
            if !self.body.is_stuck_states(&|p| self.world.block_state(p)) { break; }
            self.body.pos.y += 1.0;
        }
    }

    /// Explicit spawnpoint commands replace any bed-linked respawn point.
    pub fn set_spawn(&mut self, pos: Vec4) {
        if pos.is_finite() { self.spawn = pos; self.building.bed_spawn = None; }
    }

    pub(super) fn update_survival(&mut self, dt: f32, previous_y: f32) {
        if self.difficulty==crate::settings::Difficulty::Peaceful && self.health>0 && self.health<20 {
            self.peaceful_heal_time+=dt;
            if self.peaceful_heal_time>=1.0 {self.health+=1;self.peaceful_heal_time-=1.0;}
        } else {self.peaceful_heal_time=0.0;}
        self.invulnerability = (self.invulnerability - dt).max(0.0);
        self.hazard_cooldown = (self.hazard_cooldown - dt).max(0.0);
        if self.mode == GameMode::Creative {
            self.air = 15.0;
            self.fall_distance = 0.0;
            if self.body.pos.y < -32.0 { self.body = Body::new(self.spawn); }
            return;
        }
        if self.body.pos.y < -32.0 { self.die("Void"); return; }
        if self.body.in_water || self.flying { self.fall_distance = 0.0; }
        else {
            self.fall_distance += (previous_y - self.body.pos.y).max(0.0);
            if self.body.on_ground {
                let damage = (self.fall_distance - 3.0).ceil().clamp(0.0, 255.0) as u8;
                self.fall_distance = 0.0;
                if damage > 0 { self.damage(damage, "Fall"); }
            }
        }
        if !self.is_alive() { return; }
        let eye = self.eye();
        let eye_block = self.world.block(eye.floor().as_ivec4());
        let eye_cell = eye.floor().as_ivec4();
        let submerged = eye_block.is_some_and(|id| block::is_water(id)
            && eye.y - eye.y.floor() < block::fluid_height(id, self.world.block(eye_cell + IVec4::Y)));
        self.air = if submerged { (self.air - dt).max(0.0) } else { 15.0 };
        let lava=self.body.touches_fluid(block::FluidKind::Lava,&|p|self.world.block_state(p));
        if self.hazard_cooldown <= 0.0 {
            if lava { self.damage(4, "Lava"); self.hazard_cooldown = 0.5; }
            else if self.air == 0.0 { self.damage(2, "Drowned"); self.hazard_cooldown = 1.0; }
            else if eye_block.is_some_and(|id|block::definition(id).opaque) { self.damage(1, "Suffocated"); self.hazard_cooldown = 0.5; }
        }

    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_immunity_food_and_armor_wear_are_persistent_item_state() {
        let mut game = Game::new(None, 7, GameMode::Survival);
        game.damage(4, "Creature");
        assert_eq!(game.health, 16);
        game.damage(4, "Creature");
        assert_eq!(game.health, 16, "contact cannot damage every render frame");
        game.hotbar[0] = Some(Slot::new(item::COOKED_PORK, 1));
        assert!(game.use_selected_food());
        assert_eq!(game.health, 20);
        assert!(game.hotbar[0].is_none());
        game.hotbar[0] = Some(Slot::new(item::BREAD, 1));
        game.use_selected_food();
        assert!(game.hotbar[0].is_none(), "Beta 1.5 eats even at full health");
        assert_eq!(game.health, 20);
        for (slot, id) in [item::IRON_HELMET, item::IRON_CHESTPLATE, item::IRON_LEGGINGS, item::IRON_BOOTS].into_iter().enumerate() {
            let mut cursor = Some(Slot::new(id, 1));
            game.click_inventory_slot(36 + slot, &mut cursor, false);
            assert!(cursor.is_none());
        }
        assert_eq!(game.armor_points(), 20, "new Beta armor gives full protection independent of material");
        game.invulnerability = 0.0;
        game.damage(10, "Creature");
        assert_eq!(game.health, 18);
        assert_eq!(game.armor[0].unwrap().damage, 10);
        let mut cursor = None;
        game.click_inventory_slot(36, &mut cursor, false);
        game.hotbar[0] = None;
        game.click_inventory_slot(0, &mut cursor, false);
        game.drop_selected();
        assert_eq!(game.dropped_items[0].stack.damage, 10, "unequipping and dropping must not repair armor");
        game.set_mode(GameMode::Creative);
        game.invulnerability = 0.0;
        game.damage(20, "Lava");
        assert_eq!(game.health, 18);
    }

    #[test]
    fn eating_mushroom_stew_returns_its_bowl() {
        let mut game = Game::new(None, 7, GameMode::Survival);
        game.health = 8;
        game.hotbar[0] = Some(Slot::new(item::MUSHROOM_STEW, 1));
        assert!(game.use_selected_food());
        assert_eq!(game.health, 18);
        assert_eq!(game.hotbar[0], Some(Slot::new(item::BOWL, 1)));
    }

    #[test]
    fn death_drops_inventory_crafting_and_cursor_once_then_respawns() {
        let mut game = Game::new(None, 7, GameMode::Survival);
        game.hotbar[0] = Some(Slot::new(block::DIRT, 2));
        game.inventory[0] = Some(Slot::new(block::STONE, 5));
        game.crafting_grid[0] = Some(Slot::new(block::PLANKS, 3));
        game.damage(20, "Creature");
        assert!(!game.is_alive());
        assert!(game.hotbar.iter().chain(game.inventory.iter()).all(Option::is_none));
        assert_eq!(game.dropped_items.iter().map(|d| d.stack.count).sum::<u32>(), 10);
        let mut cursor = Some(Slot::new(item::BUCKET, 1));
        game.return_cursor(&mut cursor);
        game.damage(20, "Creature");
        assert!(cursor.is_none());
        assert_eq!(game.dropped_items.iter().map(|d| d.stack.count).sum::<u32>(), 11);
        game.respawn();
        assert_eq!(game.health, 20);
        assert!(game.death_reason.is_empty());
        assert!(game.hotbar.iter().all(Option::is_none));
    }

    #[test]
    fn drowning_lava_falls_and_void_have_real_damage() {
        let mut game = Game::new(None, 7, GameMode::Survival);
        game.body.pos.y = game.body.pos.y.floor() + 0.37; // eye is near the top of a voxel
        let eye = game.eye().floor().as_ivec4();
        game.world.set_block(eye, block::WATER);
        game.world.set_block(eye + IVec4::Y, block::WATER);
        for _ in 0..16 { game.update_survival(1.0, game.body.pos.y); }
        assert!(game.health < 20, "continuous water column has no breathable voxel seams");
        game.world.set_block(eye, AIR);
        game.world.set_block(eye + IVec4::Y, AIR);
        game.update_survival(1.0, game.body.pos.y);
        assert_eq!(game.air, 15.0);
        let before = game.health;
        game.world.set_block(game.body.pos.floor().as_ivec4(), block::LAVA);
        game.update_survival(1.0, game.body.pos.y);
        assert!(game.health < before);
        game.world.set_block(game.body.pos.floor().as_ivec4(), AIR);
        game.health = 20;
        game.body.on_ground = true;
        game.update_survival(1.0, game.body.pos.y + 7.0);
        assert_eq!(game.health, 16);
        game.body.pos.y = -40.0;
        game.update_survival(1.0, -40.0);
        assert_eq!(game.health, 0);
        assert_eq!(game.death_reason, "Void");
        assert_eq!(game.body.pos.y, -40.0, "death must wait for explicit respawn");
    }

    #[test]
    fn saved_armor_keeps_wear_and_old_saves_start_healthy() {
        let old = r#"
seed=7
mode="Survival"
pos=[0.0,64.0,0.0,0.0]
heading=[0.0,0.0,0.0,1.0]
pitch=0.0
flying=false
selected=0
hotbar=[]
"#;
        let mut level: Level = toml::from_str(old).unwrap();
        assert_eq!(level.health, 20);
        level.health = 7;
        level.armor.push(Slot { variant: 0, block: item::IRON_HELMET, count: 1, damage: 21 });
        let restored: Level = toml::from_str(&toml::to_string(&level).unwrap()).unwrap();
        assert_eq!(restored.health, 7);
        assert_eq!(restored.armor[0].damage, 21);
    }
    #[test]
    fn open_door_airspace_does_not_suffocate_and_shallow_w_edge_lava_damages(){
        let mut game=Game::menu_preview(7);game.world.insert_chunk(IVec4::ZERO,mc4d_core::chunk::Chunk::uniform(block::AIR));game.body.pos=Vec4::new(8.5,4.0,8.5,8.5);
        game.world.set_block_state(game.eye().floor().as_ivec4(),block::BlockState::new(block::WOOD_DOOR,4));game.update_survival(0.05,4.0);assert_eq!(game.health,20);
        game.world.set_block(game.eye().floor().as_ivec4(),block::AIR);game.body.pos=Vec4::new(8.5,4.08,8.5,8.9);game.world.set_block(IVec4::new(8,4,8,9),block::FluidKind::Lava.flowing(7));game.update_survival(0.05,4.08);assert_eq!(game.health,16);
    }

}
