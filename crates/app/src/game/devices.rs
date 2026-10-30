//! Powered devices, primed TNT and the original instant-use projectiles.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct PrimedTnt {
    pos: Vec4,
    velocity: Vec4,
    fuse: u16,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Thrown {
    pos: Vec4,
    velocity: Vec4,
    item: item::ItemId,
    age: f32,
    from_player: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Devices {
    tnt: Vec<PrimedTnt>,
    thrown: Vec<Thrown>,
    dispensers: Vec<(IVec4, u64)>,
    tick: u64,
    fraction: f32,
}
impl Devices {
    pub fn sanitize(&mut self) {
        self.tnt
            .retain(|t| t.pos.is_finite() && t.velocity.is_finite() && t.fuse <= 80);
        self.thrown.retain(|t| {
            t.pos.is_finite()
                && t.velocity.is_finite()
                && t.age.is_finite()
                && t.age < 60.0
                && matches!(t.item, item::EGG | item::SNOWBALL)
        });
        self.fraction = if self.fraction.is_finite() {
            self.fraction.clamp(0.0, 0.999)
        } else {
            0.0
        };
    }
}
fn note_instrument(id: u8) -> &'static str {
    // Original sj tile entity compares material identity, not visual texture.
    match block::definition(id).reference_id {
        1 | 4 | 7 | 14 | 15 | 16 | 21 | 22 | 23 | 24 | 43 | 44 | 45 | 48 | 49 | 52 | 56 | 61
        | 62 | 67 | 70 | 72 | 73 | 74 | 87 => "note.bd",
        12 | 13 | 88 => "note.snare",
        20 | 89 => "note.hat",
        5 | 17 | 25 | 47 | 53 | 54 | 58 | 63 | 64 | 68 | 84 | 85 | 95 => "note.bassattack",
        _ => "note.harp",
    }
}
impl Game {
    pub(super) fn play_note(&mut self, p: IVec4) {
        let Some(state) = self
            .world
            .block_state(p)
            .filter(|s| s.id == block::NOTE_BLOCK)
        else {
            return;
        };
        if self.world.block(p + IVec4::Y) != Some(AIR) {
            return;
        }
        let instrument = note_instrument(self.world.block(p - IVec4::Y).unwrap_or(AIR));
        self.emit_sound(
            instrument,
            p.as_vec4() + Vec4::splat(0.5),
            3.0,
            2.0f32.powf((f32::from(state.metadata.min(24)) - 12.0) / 12.0),
        );
    }
    pub fn prime_tnt(&mut self, p: IVec4, chain: bool) -> bool {
        if self.world.block(p) != Some(block::TNT) || self.devices.tnt.len() >= 128 {
            return false;
        }
        if !self.world.set_block(p, AIR) {
            return false;
        }
        self.spawn_primed_tnt(p, chain);
        true
    }
    pub(super) fn spawn_primed_tnt(&mut self, p: IVec4, chain: bool) {
        let fuse = if chain {
            10 + (self.rand() * 20.0) as u16
        } else {
            80
        };
        // The original constructor multiplies a random angle in radians by pi/180;
        // preserve its tiny, biased initial horizontal nudge rather than modernizing it.
        let angle = self.rand() * std::f32::consts::TAU * std::f32::consts::PI / 180.0;
        self.devices.tnt.push(PrimedTnt {
            pos: p.as_vec4() + Vec4::new(0.5, 0.01, 0.5, 0.5),
            velocity: Vec4::new(-angle.sin() * 0.4, 4.0, -angle.cos() * 0.4, 0.0),
            fuse,
        });
        if !chain {
            self.emit_sound("random.fuse", p.as_vec4(), 1.0, 1.0);
        }
    }
    pub(super) fn gaussian(&mut self) -> f32 {
        let u = self.rand().max(f32::EPSILON);
        let v = self.rand();
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
    fn launch_projectile(
        &mut self,
        item: item::ItemId,
        pos: Vec4,
        direction: Vec4,
        speed: f32,
        spread: f32,
        from_player: bool,
    ) -> bool {
        if item == item::ARROW && self.arrows.len() >= 64
            || item != item::ARROW && self.devices.thrown.len() >= 128
        {
            return false;
        }
        let noise = Vec4::new(
            self.gaussian(),
            self.gaussian(),
            self.gaussian(),
            self.gaussian(),
        ) * spread;
        let velocity = (direction.normalize_or_zero() + noise) * speed;
        if item == item::ARROW {
            self.arrows.push(Arrow {
                pos,
                velocity,
                age: 0.0,
                stuck: false,
                from_player,
                from_living:from_player,
                ground_time: 0.0,
            });
        } else {
            self.devices.thrown.push(Thrown {
                pos,
                velocity,
                item,
                age: 0.0,
                from_player,
            });
        }
        true
    }
    pub(super) fn use_held_projectile(&mut self) -> bool {
        let Some(held) = self.hotbar[self.selected] else {
            return false;
        };
        let projectile = match held.block {
            item::BOW => item::ARROW,
            item::EGG => item::EGG,
            item::SNOWBALL => item::SNOWBALL,
            _ => return false,
        };
        let ammo = if held.block == item::BOW && (self.mode == GameMode::Survival) {
            let Some(index) = (0..36).find(|&i| {
                self.inventory_slot(i)
                    .is_some_and(|s| s.block == item::ARROW)
            }) else {
                return true;
            };
            Some(index)
        } else {
            None
        };
        let direction = self.basis().forward;
        let origin = self.eye() + direction * 0.3;
        if !self.launch_projectile(projectile, origin, direction, 30.0, 0.0075, true) {
            return true;
        }
        if self.mode == GameMode::Survival {
            if let Some(index) = ammo {
                let slot = self.slot_mut(index).unwrap();
                let stack = slot.as_mut().unwrap();
                stack.count -= 1;
                if stack.count == 0 {
                    *slot = None;
                }
            } else {
                self.take_selected();
            }
        }
        self.start_hand_use();
        self.emit_sound("random.bow", origin, 0.5, 1.0);
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .item(held.block)
            .used += 1;
        true
    }
    fn dispense(&mut self, p: IVec4) {
        let Some(state) = self
            .world
            .block_state(p)
            .filter(|s| s.id == block::DISPENSER)
        else {
            return;
        };
        let Some(index) = self
            .stations
            .iter()
            .position(|s| s.pos == p.to_array() && s.kind == StationKind::Dispenser)
        else {
            self.emit_sound("random.click", p.as_vec4(), 1.0, 1.2);
            return;
        };
        let occupied: Vec<_> = self.stations[index]
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| (s.count > 0).then_some(i))
            .collect();
        if occupied.is_empty() {
            self.emit_sound("random.click", p.as_vec4(), 1.0, 1.2);
            return;
        }
        let slot_index = occupied[(self.rand() * occupied.len() as f32) as usize % occupied.len()];
        let stack = self.stations[index].slots[slot_index];
        let forward = -mc4d_core::shapes::wall_support(state.metadata).as_vec4();
        let origin = p.as_vec4() + Vec4::splat(0.5) + forward * 0.5;
        if matches!(stack.block, item::ARROW | item::EGG | item::SNOWBALL) {
            if !self.launch_projectile(
                stack.block,
                origin,
                forward + Vec4::Y * 0.1,
                22.0,
                0.045,
                false,
            ) {
                return;
            }
            self.emit_sound("random.bow", origin, 1.0, 1.2);
        } else {
            let speed = 4.0 + self.rand() * 2.0;
            let velocity = forward * speed
                + Vec4::Y * 4.0
                + Vec4::new(
                    self.gaussian(),
                    self.gaussian(),
                    self.gaussian(),
                    self.gaussian(),
                ) * 0.9;
            self.spawn_drop(
                Slot { count: 1, ..stack },
                origin - Vec4::Y * 0.3,
                velocity,
                0.5,
            );
            self.emit_sound("random.click", origin, 1.0, 1.0);
        }
        self.stations[index].slots[slot_index].count -= 1;
        if self.stations[index].slots[slot_index].count == 0 {
            self.stations[index].slots[slot_index] = Slot::EMPTY;
        }
    }
    pub(super) fn update_devices(&mut self, dt: f32) {
        self.devices.fraction += dt * 20.0;
        let ticks = self.devices.fraction.floor() as u64;
        self.devices.fraction -= ticks as f32;
        self.devices.tick = self.devices.tick.saturating_add(ticks);
        for p in self.take_powered_events() {
            match self.world.block(p) {
                Some(block::TNT) => {
                    self.prime_tnt(p, false);
                }
                Some(block::NOTE_BLOCK) => self.play_note(p),
                Some(block::DISPENSER) => {
                    if !self.devices.dispensers.iter().any(|(q, _)| *q == p) {
                        self.devices.dispensers.push((p, self.devices.tick + 4));
                    }
                }
                _ => {}
            }
        }
        let pending = std::mem::take(&mut self.devices.dispensers);
        for (p, due) in pending {
            if due > self.devices.tick || !self.world.is_loaded(chunk_of(p)) {
                self.devices.dispensers.push((p, due));
            } else if self.block_powered(p) || self.block_powered(p + IVec4::Y) {
                self.dispense(p);
            }
        }
        let mut tnt = std::mem::take(&mut self.devices.tnt);
        let mut detonations = Vec::new();
        tnt.retain_mut(|t| {
            if !self.world.is_loaded(chunk_of(t.pos.floor().as_ivec4())) {
                return true;
            }
            for _ in 0..ticks {
                let mut body = Body::with_size(t.pos, 0.49, 0.98);
                t.velocity.y -= 0.8;
                body.vel = t.velocity;
                let incoming = t.velocity.y;
                body.step_states_options(0.05, &|p| self.world.block_state(p), 0.0, false);
                t.pos = body.pos;
                t.velocity = body.vel * 0.98;
                if body.on_ground {
                    t.velocity.x *= 0.7;
                    t.velocity.z *= 0.7;
                    t.velocity.w *= 0.7;
                    t.velocity.y = -incoming * 0.49;
                }
                if t.fuse == 0 {
                    detonations.push(t.pos + Vec4::Y * 0.49);
                    return false;
                }
                t.fuse -= 1;
            }
            true
        });
        self.devices.tnt = tnt;
        for p in detonations {
            self.explode_with_radius(p, 4);
        }
        self.update_thrown(dt);
    }
    fn update_thrown(&mut self, dt: f32) {
        let mut thrown = std::mem::take(&mut self.devices.thrown);
        let mut hatches = Vec::new();
        thrown.retain_mut(|t| {
            if !self.world.is_loaded(chunk_of(t.pos.floor().as_ivec4())) {
                return true;
            }
            t.age += dt;
            let delta = t.velocity * dt;
            let length = delta.length();
            let direction = delta.normalize_or_zero();
            let wall = raycast_states(
                t.pos,
                direction,
                length,
                &|p| {
                    Some(
                        self.world
                            .block_state(p)
                            .unwrap_or_else(|| block::BEDROCK.into()),
                    )
                },
                false,
            );
            let hit_mob = self
                .creatures
                .iter()
                .enumerate()
                .filter(|(_, c)| c.health > 0)
                .filter_map(|(i, c)| {
                    c.body
                        .bounds()
                        .ray(t.pos, direction, length)
                        .map(|(d, _)| (i, d))
                })
                .filter(|(_, d)| wall.is_none_or(|w| *d < w.dist))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let player = if !t.from_player || t.age > 0.25 {
                self.body
                    .bounds()
                    .ray(t.pos, direction, length)
                    .map(|(d, _)| d)
            } else {
                None
            };
            let impact_distance = wall
                .map_or(f32::INFINITY, |h| h.dist)
                .min(hit_mob.map_or(f32::INFINITY, |(_, d)| d))
                .min(player.unwrap_or(f32::INFINITY));
            if let Some((i, d)) = hit_mob
                && d <= impact_distance
            {
                self.creatures[i].body.vel += direction * 2.0;
                self.creatures[i].hurt = 0.2;
            }
            if impact_distance.is_finite() {
                if t.item == item::EGG {
                    hatches.push(t.pos + direction * impact_distance);
                }
                return false;
            }
            t.pos += delta;
            let drag: f32 = if self
                .world
                .block(t.pos.floor().as_ivec4())
                .is_some_and(block::is_water)
            {
                0.8
            } else {
                0.99
            };
            t.velocity *= drag.powf(dt * 20.0);
            t.velocity.y -= 12.0 * dt;
            t.age < 60.0
        });
        self.devices.thrown = thrown;
        for p in hatches {
            if self.rand() < 0.125 {
                let count = if self.rand() < 1.0 / 32.0 { 4 } else { 1 };
                for _ in 0..count {
                    self.spawn_creature(CreatureKind::Chicken, p);
                }
            }
        }
    }
    pub(super) fn device_draws(&self) -> Vec<EntityDraw> {
        self.devices
            .tnt
            .iter()
            .map(|t| EntityDraw {
                center: t.pos + Vec4::Y * 0.49,
                radius: 0.49,
                color: [1.0; 3],
                kind: 17,
                forward: Vec4::Z,
                item: block::TNT.into(),
                item_variant: 0,
                state: if (t.fuse / 5) % 2 == 0 { 64 } else { 0 },
            })
            .chain(self.devices.thrown.iter().map(|t| EntityDraw {
                center: t.pos,
                radius: 0.12,
                color: [1.0; 3],
                kind: 13,
                forward: t.velocity,
                item: t.item,
                item_variant: 0,
                state: 0,
            }))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc4d_core::{
        block::BlockState,
        chunk::{CHUNK_VOLUME, Chunk},
    };
    fn arena() -> (Game, IVec4) {
        let mut g = Game::menu_preview(3);
        let cells = (0..CHUNK_VOLUME)
            .map(|i| {
                if (i >> 4) & 15 == 7 {
                    block::STONE
                } else {
                    AIR
                }
            })
            .collect();
        g.world.insert_chunk(IVec4::ZERO, Chunk::from_dense(cells));
        g.body.pos = Vec4::new(2.0, 8.0, 2.0, 2.0);
        (g, IVec4::splat(8))
    }
    fn step(g: &mut Game, n: usize) {
        for _ in 0..n {
            g.update_redstone(0.05);
            g.update_devices(0.05);
        }
    }
    #[test]
    fn powered_tnt_has_persisted_fuse_and_detonates_after_original_post_decrement() {
        let (mut g, p) = arena();
        g.world.set_block(p, block::TNT);
        g.world
            .set_block_state(p + IVec4::W, BlockState::new(block::LEVER, 5 | 8));
        g.update_redstone(0.0);
        g.update_devices(0.0);
        assert_eq!(g.world.block(p), Some(AIR));
        assert_eq!(g.devices.tnt.len(), 1);
        assert_eq!(g.devices.tnt[0].fuse, 80);
        let text = toml::to_string(&g.devices).unwrap();
        let mut restored: Devices = toml::from_str(&text).unwrap();
        restored.sanitize();
        assert_eq!(restored.tnt[0].fuse, 80);
        g.devices = restored;
        step(&mut g, 80);
        assert_eq!(g.devices.tnt.len(), 1);
        assert!(g.explosions.is_empty());
        step(&mut g, 1);
        assert!(g.devices.tnt.is_empty());
        assert_eq!(g.explosions.len(), 1);
    }
    #[test]
    fn note_tuning_material_and_rising_power_produce_real_sound_events() {
        let (mut g, p) = arena();
        g.world.set_block(p, block::NOTE_BLOCK);
        g.target = Some(Target::Block(Hit {
            block: p,
            normal: IVec4::Y,
            dist: 2.0,
        }));
        assert!(g.use_target_building());
        assert_eq!(g.world.block_state(p).unwrap().metadata, 1);
        assert_eq!(g.sounds.last().unwrap().name, "note.bd");
        assert!((g.sounds.last().unwrap().pitch - 2.0f32.powf(-11.0 / 12.0)).abs() < 1e-6);
        g.sounds.clear();
        g.world
            .set_block_state(p + IVec4::W, BlockState::new(block::LEVER, 5 | 8));
        g.update_redstone(0.0);
        g.update_devices(0.0);
        assert_eq!(g.sounds.iter().filter(|s| s.name == "note.bd").count(), 1);
        g.sounds.clear();
        step(&mut g, 10);
        assert!(g.sounds.is_empty(), "steady power must not repeat notes");
        g.world.set_block(p + IVec4::Y, block::GLASS);
        g.play_note(p);
        assert!(
            g.sounds.is_empty(),
            "any material above silences original note blocks"
        );
    }
    #[test]
    fn dispenser_waits_four_ticks_and_conserves_ammo_on_every_pulse() {
        let (mut g, p) = arena();
        g.world
            .set_block_state(p, BlockState::new(block::DISPENSER, 5));
        let mut station = Station::new(p, StationKind::Dispenser);
        station.slots[0] = Slot::new(block::DIRT, 3);
        g.stations.push(station);
        let lever = p + IVec4::W;
        g.world
            .set_block_state(lever, BlockState::new(block::LEVER, 5 | 8));
        g.update_redstone(0.0);
        g.update_devices(0.0);
        step(&mut g, 3);
        assert!(g.dropped_items.is_empty());
        step(&mut g, 1);
        assert_eq!(g.dropped_items.len(), 1);
        assert_eq!(g.stations[0].slots[0].count, 2);
        step(&mut g, 20);
        assert_eq!(g.dropped_items.len(), 1, "held power fires once");
        g.use_redstone(lever, g.world.block_state(lever).unwrap());
        step(&mut g, 1);
        g.stations[0].slots[0] = Slot::new(item::ARROW, 2);
        g.use_redstone(lever, g.world.block_state(lever).unwrap());
        g.update_redstone(0.0);
        g.update_devices(0.0);
        step(&mut g, 4);
        assert_eq!(g.arrows.len(), 1);
        assert_eq!(g.stations[0].slots[0].count, 1);
        assert!(g.arrows[0].velocity.x > 15.0);
        let restored: Station = toml::from_str(&toml::to_string(&g.stations[0]).unwrap()).unwrap();
        assert_eq!(restored.slots.len(), 9);
        assert_eq!(restored.slots[0].count, 1);
    }
    #[test]
    fn instant_bow_uses_inventory_ammo_without_consuming_bow() {
        let (mut g, _) = arena();
        g.hotbar[0] = Some(Slot::new(item::BOW, 1));
        g.inventory[4] = Some(Slot::new(item::ARROW, 2));
        assert!(g.use_held_projectile());
        assert_eq!(g.arrows.len(), 1);
        assert!(g.arrows[0].from_player);
        assert_eq!(g.inventory[4].unwrap().count, 1);
        assert_eq!(g.hotbar[0].unwrap().block, item::BOW);
        g.hotbar[0] = Some(Slot::new(item::SNOWBALL, 2));
        assert!(g.use_held_projectile());
        assert_eq!(g.devices.thrown.len(), 1);
        assert_eq!(g.hotbar[0].unwrap().count, 1);
    }
}
