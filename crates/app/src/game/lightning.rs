//! Short-lived storm bolts. Original c lifecycle: initial2 ticks,1–3 repeat
//! flashes and a random0–9-tick gap; no weather entity is persisted in level data.
use super::*;
#[derive(Clone)]
pub(super) struct Bolt {
    pos: IVec4,
    life: i32,
    flashes: u8,
    points: Vec<(Vec4, Vec4, u32)>,
}
#[derive(Default)]
pub(super) struct Lightning {
    bolts: Vec<Bolt>,
    fraction: f32,
    last_event: Option<u64>,
}
impl Game {
    fn bolt_points(&mut self, pos: IVec4) -> Vec<(Vec4, Vec4, u32)> {
        let base = pos.as_vec4() + Vec4::new(0.5, 0.0, 0.5, 0.5);
        let mut points = vec![base];
        let mut horizontal = Vec4::ZERO;
        for y in 1..=8 {
            horizontal += Vec4::new(
                (self.rand() * 11.0).floor() - 5.0,
                0.0,
                (self.rand() * 11.0).floor() - 5.0,
                (self.rand() * 11.0).floor() - 5.0,
            );
            points.push(base + horizontal + Vec4::Y * (y as f32 * 16.0));
        }
        let mut segments: Vec<_> = points
            .windows(2)
            .enumerate()
            .map(|(i, p)| (p[0], p[1], i as u32))
            .collect();
        for index in [7, 6] {
            let mut from = points[index];
            for _ in 0..3 {
                let to = from
                    + Vec4::new(
                        (self.rand() * 31.0).floor() - 15.0,
                        -16.0,
                        (self.rand() * 31.0).floor() - 15.0,
                        (self.rand() * 31.0).floor() - 15.0,
                    );
                segments.push((from, to, 16));
                from = to;
            }
        }
        segments
    }
    fn strike_area_loaded(&self, pos: IVec4) -> bool {
        let lo = chunk_of(pos - IVec4::splat(10));
        let hi = chunk_of(pos + IVec4::splat(10));
        for w in lo.w..=hi.w {
            for z in lo.z..=hi.z {
                for x in lo.x..=hi.x {
                    for y in lo.y.max(0)..=hi.y.min(mc4d_core::chunk::WORLD_CHUNKS_Y - 1) {
                        if !self.world.is_loaded(IVec4::new(x, y, z, w)) {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
    fn strike_fire(&mut self, pos: IVec4, initial: bool) {
        if !matches!(
            self.difficulty,
            crate::settings::Difficulty::Normal | crate::settings::Difficulty::Hard
        ) || !self.strike_area_loaded(pos)
        {
            return;
        }
        self.ignite(pos);
        if initial {
            for _ in 0..4 {
                let delta = IVec4::new(
                    (self.rand() * 3.0) as i32 - 1,
                    (self.rand() * 3.0) as i32 - 1,
                    (self.rand() * 3.0) as i32 - 1,
                    (self.rand() * 3.0) as i32 - 1,
                );
                self.ignite(pos + delta);
            }
        }
    }
    pub(super) fn spawn_lightning(&mut self, pos: IVec4) {
        if self.lightning.bolts.len() >= 4 {
            return;
        }
        let flashes = 1 + (self.rand() * 3.0) as u8;
        let points = self.bolt_points(pos);
        self.strike_fire(pos, true);
        let thunder_pitch = 0.8 + self.rand() * 0.2;
        self.emit_sound("ambient.weather.thunder", self.eye(), 1.0, thunder_pitch);
        let pitch = 0.5 + self.rand() * 0.2;
        self.emit_sound("random.explode", pos.as_vec4(), 2.0, pitch);
        self.environment.lightning = 1.0;
        self.lightning.bolts.push(Bolt {
            pos,
            life: 2,
            flashes,
            points,
        });
    }
    pub(super) fn update_lightning(&mut self, dt: f32) {
        let event = self.environment.lightning_events;
        if self
            .lightning
            .last_event
            .replace(event)
            .is_some_and(|old| old != event)
            && self.environment.raining
            && self.environment.thundering
        {
            for _ in 0..32 {
                let len = self.ecology.weather_queue.len();
                if len == 0 {
                    break;
                }
                let index = (self.rand() * len as f32) as usize % len;
                let c = self.ecology.weather_queue[index];
                let x = c[0] * 16 + (self.rand() * 16.0) as i32;
                let z = c[1] * 16 + (self.rand() * 16.0) as i32;
                let w = c[2] * 16 + (self.rand() * 16.0) as i32;
                if matches!(
                    self.world.terrain.biome(x, z, w),
                    mc4d_core::terrain::Biome::Desert | mc4d_core::terrain::Biome::Taiga
                ) {
                    continue;
                }
                if let Some(top) = self.ecology_surface(x, z, w) {
                    self.spawn_lightning(top + IVec4::Y);
                    break;
                }
            }
        }
        self.lightning.fraction += dt.max(0.0) * 20.0;
        let ticks = self.lightning.fraction.floor() as u32;
        self.lightning.fraction -= ticks as f32;
        for _ in 0..ticks {
            let mut bolts = std::mem::take(&mut self.lightning.bolts);
            bolts.retain_mut(|bolt| {
                bolt.life -= 1;
                if bolt.life < 0 {
                    if bolt.flashes == 0 {
                        return false;
                    }
                    if bolt.life < -(self.rand() * 10.0) as i32 {
                        bolt.flashes -= 1;
                        bolt.life = 1;
                        bolt.points = self.bolt_points(bolt.pos);
                        self.strike_fire(bolt.pos, false);
                    }
                }
                if bolt.life >= 0 {
                    self.environment.lightning = 1.0;
                    let bounds = self.body.bounds();
                    let min = bolt.pos.as_vec4() - Vec4::splat(3.0);
                    let max = bolt.pos.as_vec4() + Vec4::new(3.0, 9.0, 3.0, 3.0);
                    let area=mc4d_core::shapes::Aabb4::new(min,max);
                    self.dropped_items.retain_mut(|drop|{if area.overlaps(drop.body.bounds()){drop.health=drop.health.saturating_sub(5);}drop.health>0});
                    self.vehicles_lightning(min,max);self.paintings_lightning(min,max);
                    if bounds.min.cmple(max).all() && bounds.max.cmpge(min).all() {
                        self.damage(5, "Lightning");
                        if self.mode == GameMode::Survival {
                            self.ignite_player();
                        }
                    }
                }
                true
            });
            self.lightning.bolts = bolts;
        }
    }
    pub(super) fn lightning_draws(&self) -> impl Iterator<Item = EntityDraw> + '_ {
        self.lightning
            .bolts
            .iter()
            .filter(|b| b.life >= 0)
            .flat_map(|b| b.points.iter())
            .map(|ends| {
                let delta = ends.1 - ends.0;
                EntityDraw {
                    center: (ends.0 + ends.1) * 0.5,
                    radius: delta.length() * 0.5 + 0.5,
                    color: [0.45, 0.45, 0.5],
                    kind: 24,
                    forward: delta,
                    item: 0,
                    item_variant: 0,
                    state: ends.2,
                }
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bolt_flashes_damage_player_and_expire_without_touching_mobs() {
        let mut g = Game::menu_preview(7);
        g.body.pos = Vec4::new(8.5, 5.0, 8.5, 8.5);
        g.spawn_lightning(IVec4::new(8, 4, 8, 8));
        assert_eq!(g.lightning_draws().count(), 14);
        g.update_lightning(0.05);
        assert_eq!(g.health, 15);
        assert!(g.burning());
        for _ in 0..100 {
            g.update_lightning(0.05);
        }
        assert_eq!(g.lightning_draws().count(), 0);
        assert!(g.lightning.bolts.is_empty());
        assert_eq!(
            g.sounds
                .iter()
                .filter(|s| s.name == "ambient.weather.thunder")
                .count(),
            1,
            "repeat flashes do not replay initial thunder"
        );
    }
    #[test]
    fn strike_ignites_loaded_supported_ground_only_on_normal_or_hard() {
        let mut g = Game::menu_preview(7);
        for w in -1..=1 {
            for z in -1..=1 {
                for x in -1..=1 {
                    for y in 0..=1 {
                        g.world.insert_chunk(
                            IVec4::new(x, y, z, w),
                            mc4d_core::chunk::Chunk::uniform(AIR),
                        );
                    }
                }
            }
        }
        let p = IVec4::new(8, 4, 8, 8);
        g.world.set_block(p - IVec4::Y, block::STONE);
        g.difficulty = crate::settings::Difficulty::Easy;
        g.strike_fire(p, false);
        assert_eq!(g.world.block(p), Some(AIR));
        g.difficulty = crate::settings::Difficulty::Normal;
        g.strike_fire(p, false);
        assert_eq!(g.world.block(p), Some(block::FIRE));
    }
}
