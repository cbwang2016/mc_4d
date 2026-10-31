//! Bounded 4D creature simulation. Every species has behavior beyond its palette:
//! grazing animals flee, squid swim, wolves tame/follow/defend, skeletons shoot,
//! creepers fuse and damage terrain, and slimes hop and split.
use super::*;
use mc4d_core::item;

pub use mc4d_core::entities::CreatureKind;

#[derive(Serialize, Deserialize)]
pub(super) struct SavedCreature {
    kind: CreatureKind,
    pos: [f32; 4],
    vel: [f32; 4],
    radius: f32,
    #[serde(default)]
    slime_size: u8,
    #[serde(default)]
    burn_ticks: u16,
    color: [f32; 3],
    health: i32,
    anger: f32,
    tamed: bool,
    sitting: bool,
    sheared: bool,
    #[serde(default)]
    wool_color: u8,
    fuse: f32,
    egg_timer: f32,
}

impl Creature {
    pub fn set_slime_size(&mut self, size: u8) {
        if self.kind != CreatureKind::Slime {
            return;
        }
        self.slime_size = mc4d_core::entities::valid_slime_size(size);
        let (width, height) = self.kind.dimensions(self.slime_size);
        self.radius = width * 0.5;
        self.body.half_width = width * 0.5;
        self.body.height = height;
        self.health = i32::from(self.slime_size).pow(2);
    }
    pub fn target_distance(&self, eye: Vec4, dir: Vec4, reach: f32) -> Option<f32> {
        if self.kind == CreatureKind::Gloom {
            return super::ray_sphere(eye, dir, self.center(), self.radius).filter(|&t| t <= reach);
        }
        self.body.bounds().ray(eye, dir, reach).map(|(t, _)| t)
    }
    pub(super) fn save(&self) -> SavedCreature {
        SavedCreature {
            kind: self.kind,
            pos: self.body.pos.to_array(),
            vel: self.body.vel.to_array(),
            radius: self.radius,
            slime_size: self.slime_size,
            burn_ticks: self.burn_ticks,
            color: self.color,
            health: self.health,
            anger: self.anger,
            tamed: self.tamed,
            sitting: self.sitting,
            sheared: self.sheared,
            wool_color: self.wool_color,
            fuse: self.fuse,
            egg_timer: self.egg_timer,
        }
    }
    pub(super) fn restore(s: SavedCreature) -> Option<Self> {
        if !s
            .pos
            .iter()
            .chain(s.vel.iter())
            .chain(s.color.iter())
            .all(|v| v.is_finite())
            || !s.radius.is_finite()
            || s.health <= 0
        {
            return None;
        }
        let mut c = Self::new(s.kind, Vec4::from_array(s.pos));
        c.body.vel = Vec4::from_array(s.vel);
        if s.kind == CreatureKind::Gloom {
            c.radius = s.radius.clamp(0.15, 1.0);
            c.body.half_width = c.radius * 0.9;
            c.body.height = c.radius * 2.0;
        } else if s.kind == CreatureKind::Slime {
            let size = if s.slime_size == 0 {
                if s.radius <= 0.35 { 1 } else { 2 }
            } else {
                s.slime_size
            };
            c.set_slime_size(size);
        }
        c.color = s.color;
        c.burn_ticks = s.burn_ticks.min(300);
        c.tamed = s.tamed && s.kind == CreatureKind::Wolf;
        c.sitting = s.sitting && c.tamed;
        c.health = s.health.min(if c.tamed {
            20
        } else if c.kind == CreatureKind::Slime {
            i32::from(c.slime_size).pow(2)
        } else {
            c.kind.health()
        });
        c.anger = if s.anger.is_finite() {
            s.anger.clamp(0.0, 15.0)
        } else {
            0.0
        };
        c.fuse = if s.fuse.is_finite() {
            s.fuse.clamp(0.0, 1.5)
        } else {
            0.0
        };
        c.egg_timer = if s.egg_timer.is_finite() {
            s.egg_timer.clamp(0.0, 600.0)
        } else {
            300.0
        };
        c.sheared = s.sheared;
        c.wool_color = s.wool_color & 15;
        Some(c)
    }
    pub fn new(kind: CreatureKind, pos: Vec4) -> Self {
        let (width, height) = kind.dimensions(2);
        let radius = width * 0.5;
        Self {
            body: Body::with_size(pos, width * 0.5, height),
            radius,
            slime_size: 2,
            burn_ticks: 0,
            color: kind.color(),
            hop_timer: 0.5,
            hurt: 0.0,
            health: kind.health(),
            anger: 0.0,
            kind,
            tamed: false,
            sitting: false,
            sheared: false,
            wool_color: 0,
            fuse: 0.0,
            ai_timer: 0.0,
            attack_timer: 0.0,
            egg_timer: 300.0,
            walk_dir: Vec4::ZERO,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Arrow {
    pub pos: Vec4,
    pub velocity: Vec4,
    pub age: f32,
    pub stuck: bool,
    #[serde(default)]
    pub from_player: bool,
    #[serde(default)]
    pub from_living: bool,
    #[serde(default)]
    pub ground_time: f32,
}

fn default_blast_radius() -> u8 {
    3
}
#[path="blast_snapshot.rs"]
mod blast_snapshot_codec;
type BlastSnapshot = std::collections::HashMap<IVec4, mc4d_core::chunk::Chunk>;
fn snapshot_state(snapshot: &BlastSnapshot, p: IVec4) -> Option<block::BlockState> {
    if p.y < 0 {
        return Some(block::BEDROCK.into());
    }
    if p.y >= mc4d_core::chunk::WORLD_HEIGHT {
        return Some(AIR.into());
    }
    snapshot
        .get(&chunk_of(p))
        .map(|c| c.get_state(mc4d_core::chunk::local_of(p)))
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Explosion {
    center: Vec4,
    next_cell: usize,
    #[serde(default = "default_blast_radius")]
    radius: u8,
    #[serde(default)]
    seed: u64,
    #[serde(default)]
    next_ray: usize,
    #[serde(default)]
    planned: bool,
    #[serde(default)]
    affected: Vec<IVec4>,
    #[serde(default,with="blast_snapshot_codec")]
    snapshot: Option<BlastSnapshot>,
    #[serde(skip)]
    seen: std::collections::HashSet<IVec4>,
}
impl Explosion {
    pub(super) fn valid(&self) -> bool {
        self.center.is_finite()
            && (1..=8).contains(&self.radius)
            && self.next_ray <= mc4d_core::explosion::DIRECTION_CELLS
            && (!self.planned || self.next_cell <= self.affected.len())
    }
}
fn blast_snapshot(world: &World, center: Vec4, radius: u8) -> BlastSnapshot {
    let reach = Vec4::splat(f32::from(radius) * 2.0 + 4.0);
    let lo = chunk_of((center - reach).floor().as_ivec4());
    let hi = chunk_of((center + reach).floor().as_ivec4());
    let mut snapshot = BlastSnapshot::new();
    for w in lo.w..=hi.w {
        for z in lo.z..=hi.z {
            for y in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let c = IVec4::new(x, y, z, w);
                    let low=(c*16).as_vec4();let high=low+Vec4::splat(16.0);
                    let separation=(low-center).max(Vec4::ZERO)+(center-high).max(Vec4::ZERO);
                    if separation.length_squared()>reach.x*reach.x{continue;}
                    if let Some(chunk) = world.chunks.get(&c) {
                        snapshot.insert(c, chunk.clone());
                    }
                }
            }
        }
    }
    snapshot
}

impl Game {
    pub fn spawn_creature(&mut self, kind: CreatureKind, pos: Vec4) {
        if self.creatures.len() < MAX_CREATURES {
            let mut creature = Creature::new(kind, pos);
            if kind == CreatureKind::Slime {
                creature.set_slime_size(1u8 << ((self.rand() * 3.0) as u8).min(2));
            }
            self.creatures.push(creature);
        }
    }

    /// Right-click a wolf with bones to tame it, meat to heal it, or another item
    /// to toggle sitting. The configured place/use binding drives this action.
    pub(super) fn interact_creature(&mut self) -> bool {
        let Some(Target::Creature(index)) = self.target else {
            return false;
        };
        let Some(c) = self.creatures.get(index) else {
            return false;
        };
        if c.health <= 0 {
            return false;
        }
        let held = self.hotbar[self.selected];
        if c.kind == CreatureKind::Cow && held.is_some_and(|s| s.block == item::BUCKET) {
            self.hotbar[self.selected] = Some(Slot::new(item::MILK, 1));
            self.start_hand_use();
            return true;
        }
        if c.kind == CreatureKind::Sheep && held.is_some_and(|s| s.block == item::DYE) {
            let color = 15 - (held.unwrap().variant & 15);
            if !c.sheared && c.wool_color != color {
                self.creatures[index].wool_color = color;
                if self.mode == GameMode::Survival {
                    self.take_selected();
                }
                self.start_hand_use();
            }
            return true;
        }
        if c.kind != CreatureKind::Wolf {
            return false;
        }
        if !c.tamed && held.is_some_and(|s| s.block == item::BONE) {
            let success = self.rand() < 1.0 / 3.0;
            if self.mode == GameMode::Survival {
                self.take_selected();
            }
            let c = &mut self.creatures[index];
            if success {
                c.tamed = true;
                c.anger = 0.0;
                c.health = 20;
                c.sitting = false;
            }
            self.toast(if success {
                "Wolf tamed"
            } else {
                "The wolf is still wary"
            });
            return true;
        }
        if c.tamed {
            if let Some(stack) = held
                && matches!(stack.block, item::RAW_PORK | item::COOKED_PORK)
                && c.health < 20
            {
                self.creatures[index].health =
                    (c.health + item::food_heal(stack.block).unwrap_or(0) as i32).min(20);
                if self.mode == GameMode::Survival {
                    self.take_selected();
                }
            } else {
                self.creatures[index].sitting = !c.sitting;
            }
            return true;
        }
        false
    }

    pub(super) fn update_creatures(&mut self, dt: f32) {
        if self.difficulty == crate::settings::Difficulty::Peaceful {
            self.creatures.retain(|c| {
                !c.kind.hostile() || c.kind == CreatureKind::Slime && c.slime_size == 1
            });
        }
        let player = self.body.pos;
        self.creatures
            .retain(|c| c.tamed || (c.body.pos - player).length_squared() < 80.0 * 80.0);
        if self.creatures.len() < MAX_CREATURES && self.rand() < dt * 0.5 {
            self.try_spawn_creature();
        }
        let positions: Vec<_> = self
            .creatures
            .iter()
            .map(|c| (c.kind, c.body.pos, c.health > 0))
            .collect();
        let mut arrows = Vec::new();
        let mut explosions = Vec::new();
        let mut eggs = Vec::new();
        let mut bites = Vec::new();
        let mut contact_damage = 0;
        let night = self.environment.is_night();
        let daylight = self.environment.daylight();
        for i in 0..self.creatures.len() {
            let random = [self.rand(), self.rand(), self.rand(), self.rand()];
            let c = &mut self.creatures[i];
            if !self
                .world
                .is_loaded(chunk_of(c.body.pos.floor().as_ivec4()))
            {
                continue;
            }
            c.hurt = (c.hurt - dt).max(0.0);
            c.anger = (c.anger - dt).max(0.0);
            c.attack_timer = (c.attack_timer - dt).max(0.0);
            c.hop_timer -= dt;
            c.ai_timer -= dt;
            let delta = player - c.body.pos;
            let distance = delta.length();
            let to_player = Vec4::new(delta.x, 0.0, delta.z, delta.w).normalize_or_zero();
            let aggressive =
                (c.kind.hostile() && (c.kind != CreatureKind::Spider || night)) || c.anger > 0.0;
            let target_player =
                aggressive && !c.tamed && self.mode == GameMode::Survival && distance < 24.0;
            if c.ai_timer <= 0.0 {
                c.ai_timer = 0.75 + random[0] * 1.25;
                c.walk_dir = Vec4::new(random[1] - 0.5, 0.0, random[2] - 0.5, random[3] - 0.5)
                    .normalize_or_zero();
                if matches!(c.kind, CreatureKind::Zombie | CreatureKind::Skeleton)
                    && daylight > 0.65
                    && !self.environment.raining
                {
                    let p = c.body.pos.floor().as_ivec4();
                    let sky = ((p.y + 2)..mc4d_core::chunk::WORLD_HEIGHT).all(|y| {
                        self.world
                            .block(IVec4::new(p.x, y, p.z, p.w))
                            .is_some_and(|id| !block::is_solid(id))
                    });
                    if sky {
                        c.health -= 2;
                        c.hurt = 0.4;
                    }
                }
                if c.kind == CreatureKind::Squid && !c.body.in_water {
                    c.health -= 1;
                }
            }
            let mut wish = c.walk_dir;
            let mut speed = if c.kind.hostile() { 2.6 } else { 1.4 };
            if target_player {
                if matches!(
                    c.kind,
                    CreatureKind::Pig
                        | CreatureKind::Cow
                        | CreatureKind::Sheep
                        | CreatureKind::Chicken
                ) {
                    wish = -to_player;
                    speed = 3.4;
                } else {
                    wish = to_player;
                }
            }
            if c.tamed {
                if c.sitting {
                    wish = Vec4::ZERO;
                } else {
                    wish = if distance > 4.0 {
                        to_player
                    } else {
                        Vec4::ZERO
                    };
                    speed = 3.5;
                    if let Some((target, (_, pos, _))) = positions
                        .iter()
                        .enumerate()
                        .filter(|(_, (kind, pos, alive))| {
                            *alive
                                && kind.hostile()
                                && (*pos - c.body.pos).length_squared() < 12.0 * 12.0
                        })
                        .min_by(|(_, a), (_, b)| {
                            (a.1 - c.body.pos)
                                .length_squared()
                                .total_cmp(&(b.1 - c.body.pos).length_squared())
                        })
                    {
                        let toward = *pos - c.body.pos;
                        wish = Vec4::new(toward.x, 0.0, toward.z, toward.w).normalize_or_zero();
                        if toward.length() < 1.5 && c.attack_timer <= 0.0 {
                            bites.push((target, 4));
                            c.attack_timer = 1.0;
                        }
                    }
                }
            }
            if c.kind == CreatureKind::Squid && c.body.in_water {
                c.body.vel = Vec4::new(
                    wish.x,
                    (self.time * 0.7 + i as f32).sin() * 0.4,
                    wish.z,
                    wish.w,
                );
            } else {
                if matches!(c.kind, CreatureKind::Gloom | CreatureKind::Slime) {
                    if c.body.on_ground && c.hop_timer <= 0.0 {
                        c.body.vel = wish * speed + Vec4::Y * 7.0;
                        c.hop_timer = 0.8 + random[0] * 1.7;
                    }
                } else {
                    c.body.vel.x = wish.x * speed;
                    c.body.vel.z = wish.z * speed;
                    c.body.vel.w = wish.w * speed;
                    let ahead = (c.body.pos + wish * (c.radius + 0.5)).floor().as_ivec4();
                    if c.body.on_ground && self.world.block(ahead).is_some_and(block::is_solid) {
                        c.body.vel.y = 7.5;
                    }
                    if c.kind == CreatureKind::Spider
                        && target_player
                        && self.world.block(ahead).is_some_and(block::is_solid)
                    {
                        c.body.vel.y = 3.0;
                    }
                }
                c.body.vel.y -= GRAVITY * dt * if c.body.in_water { 0.2 } else { 1.0 };
                if c.body.in_water {
                    c.body.vel.y = c.body.vel.y.max(2.0);
                }
                if c.kind == CreatureKind::Chicken {
                    c.body.vel.y = c.body.vel.y.max(-1.5);
                }
            }
            c.body.step_states(dt, &|p| self.world.block_state(p));
            if target_player
                && c.kind == CreatureKind::Skeleton
                && distance < 16.0
                && c.attack_timer <= 0.0
            {
                let origin = c.center();
                let target = self.body.pos + Vec4::Y;
                let line = target - origin;
                let clear = raycast_states(
                    origin,
                    line.normalize_or_zero(),
                    line.length(),
                    &|p| {
                        Some(
                            self.world
                                .block_state(p)
                                .unwrap_or_else(|| block::BEDROCK.into()),
                        )
                    },
                    false,
                )
                .is_none();
                if clear {
                    arrows.push(Arrow {
                        pos: origin,
                        velocity: line.normalize_or_zero() * 12.0 + Vec4::Y * distance * 0.3,
                        age: 0.0,
                        stuck: false,
                        from_player: false,
                        from_living: true,
                        ground_time: 0.0,
                    });
                    c.attack_timer = 2.0;
                }
            }
            if c.kind == CreatureKind::Creeper {
                let toward = self.body.pos + Vec4::Y - c.center();
                let can_see = distance < 3.0
                    && raycast_states(
                        c.center(),
                        toward.normalize_or_zero(),
                        toward.length(),
                        &|p| {
                            Some(
                                self.world
                                    .block_state(p)
                                    .unwrap_or_else(|| block::BEDROCK.into()),
                            )
                        },
                        false,
                    )
                    .is_none();
                c.fuse = if target_player && can_see {
                    c.fuse + dt
                } else {
                    (c.fuse - dt * 2.0).max(0.0)
                };
                if c.fuse >= 1.5 {
                    explosions.push(c.body.pos);
                    c.health = 0;
                }
            } else if target_player
                && !matches!(
                    c.kind,
                    CreatureKind::Pig
                        | CreatureKind::Cow
                        | CreatureKind::Sheep
                        | CreatureKind::Chicken
                        | CreatureKind::Squid
                )
                && (c.kind != CreatureKind::Slime || c.slime_size > 1)
                && distance < c.radius + 0.8
                && c.attack_timer <= 0.0
            {
                contact_damage = contact_damage.max(if c.kind == CreatureKind::Slime {
                    c.slime_size
                } else if c.kind == CreatureKind::Zombie {
                    4
                } else {
                    2
                });
                c.attack_timer = 1.0;
            }
            if c.kind == CreatureKind::Chicken {
                c.egg_timer -= dt;
                if c.egg_timer <= 0.0 {
                    eggs.push(c.body.pos);
                    c.egg_timer = 300.0 + random[0] * 300.0;
                }
            }
        }
        for (i, damage) in bites {
            self.on_creature_hit(i);
            if let Some(c) = self.creatures.get_mut(i) {
                c.health -= damage;
                c.hurt = 0.3;
            }
        }
        if contact_damage > 0 {
            self.damage(contact_damage, "Creature");
        }
        for arrow in &arrows {
            self.emit_sound("random.bow", arrow.pos, 0.8, 1.0);
        }
        self.arrows.extend(
            arrows
                .into_iter()
                .take(64usize.saturating_sub(self.arrows.len())),
        );
        for pos in eggs {
            self.spawn_drop(Slot::new(item::EGG, 1), pos, Vec4::ZERO, 0.5);
        }
        for pos in explosions {
            self.explode(pos);
        }
        self.collect_dead_creatures();
        self.update_arrows(dt);
        self.update_explosions();
    }

    pub(super) fn on_creature_hit(&mut self, index: usize) {
        let Some(c) = self.creatures.get_mut(index) else {
            return;
        };
        if c.kind == CreatureKind::Sheep && !c.sheared {
            c.sheared = true;
            let pos = c.body.pos;
            let color = c.wool_color;
            let count = 1 + (self.rand() * 3.0) as u32;
            for _ in 0..count {
                self.spawn_drop(
                    Slot::with_variant(block::WOOL, 1, color),
                    pos + Vec4::Y,
                    Vec4::Y * 2.0,
                    0.5,
                );
            }
        }
    }

    pub(super) fn collect_dead_creatures(&mut self) {
        let old = std::mem::take(&mut self.creatures);
        let mut children = Vec::new();
        for c in old {
            if c.health > 0 {
                self.creatures.push(c);
                continue;
            }
            if let Some(name) = crate::audio::creature_sound(c.kind, false, true) {
                self.emit_sound(name, c.center(), 0.8, 1.0);
            }
            let count = (self.rand() * 3.0) as u32;
            let drop = match c.kind {
                CreatureKind::Pig => Some(if c.burn_ticks > 0 {
                    item::COOKED_PORK
                } else {
                    item::RAW_PORK
                }),
                CreatureKind::Cow => Some(item::LEATHER),
                // dd has no death-loot override: living attackers already release
                // fleece through a(rj,int); environmental death adds no extra wool.
                CreatureKind::Sheep => None,
                CreatureKind::Chicken | CreatureKind::Zombie => Some(item::FEATHER),
                CreatureKind::Skeleton => Some(item::ARROW),
                CreatureKind::Spider => Some(item::STRING),
                CreatureKind::Creeper if c.fuse < 1.5 => Some(item::GUNPOWDER),
                CreatureKind::Slime if c.slime_size == 1 => Some(item::SLIME_BALL),
                CreatureKind::Squid => Some(item::INK_SAC),
                _ => None,
            };
            if let Some(id) = drop {
                let count = match c.kind {
                    CreatureKind::Sheep => 1,
                    // Squid drops rand(3)+1 ink sacs, never none.
                    CreatureKind::Squid => count + 1,
                    _ => count,
                };
                for _ in 0..count {
                    self.spawn_drop(Slot::new(id, 1), c.body.pos, Vec4::Y * 2.0, 0.5);
                }
            }
            if c.kind == CreatureKind::Skeleton {
                // Bones are an independent rand(3) roll, not the arrow count.
                for _ in 0..(self.rand() * 3.0) as u32 {
                    self.spawn_drop(Slot::new(item::BONE, 1), c.body.pos, Vec4::Y * 2.0, 0.5);
                }
            }
            if c.kind == CreatureKind::Slime && c.slime_size > 1 {
                for i in 0..4 {
                    let offset = Vec4::new((i % 2) as f32 - 0.5, 0.0, (i / 2) as f32 - 0.5, 0.0)
                        * (c.slime_size as f32 / 4.0)
                        + Vec4::Y * 0.5;
                    let mut child = Creature::new(CreatureKind::Slime, c.body.pos + offset);
                    child.set_slime_size(c.slime_size / 2);
                    children.push(child);
                }
            }
        }
        let room = MAX_CREATURES.saturating_sub(self.creatures.len());
        self.creatures.extend(children.into_iter().take(room));
    }

    pub(super) fn update_arrows(&mut self, dt: f32) {
        let mut arrows = std::mem::take(&mut self.arrows);
        for arrow in &mut arrows {
            if !self.world.is_loaded(chunk_of(arrow.pos.floor().as_ivec4())) {
                continue;
            }
            arrow.age += dt;
            if arrow.stuck {
                arrow.ground_time += dt;
                if arrow.ground_time >= 60.0 {
                    arrow.age = 121.0;
                    continue;
                }
                if arrow.from_player
                    && arrow.ground_time >= 0.35
                    && (arrow.pos - (self.body.pos + Vec4::Y * 0.8)).length_squared() < 1.5
                {
                    if self
                        .insert_stack_range(Slot::new(item::ARROW, 1), 0..36)
                        .is_none()
                    {
                        arrow.age = 121.0;
                        self.record_pickup(Slot::new(item::ARROW, 1));
                        self.emit_sound("random.pop", arrow.pos, 0.2, 1.0);
                    }
                }
                continue;
            }
            let delta = arrow.velocity * dt;
            let length = delta.length();
            let direction = delta.normalize_or_zero();
            let wall = raycast_states(
                arrow.pos,
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
            let player_hit = if self.mode == GameMode::Survival
                && length > 0.0
                && (!arrow.from_player || arrow.age > 0.25)
            {
                self.body
                    .bounds()
                    .ray(arrow.pos, direction, length)
                    .map(|(t, _)| t)
            } else {
                None
            };
            let mob_hit = self
                .creatures
                .iter()
                .enumerate()
                .filter(|(_, c)| c.health > 0)
                .filter_map(|(i, c)| {
                    c.body
                        .bounds()
                        .ray(arrow.pos, direction, length)
                        .map(|(t, _)| (i, t))
                })
                .filter(|(_, t)| arrow.age > 0.25 || *t > 0.001)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let obstacle = wall.map_or(f32::INFINITY, |h| h.dist);
            if let Some((index, distance)) =
                mob_hit.filter(|(_, t)| *t < obstacle && player_hit.is_none_or(|p| *t < p))
            {
                if arrow.from_living {
                    self.on_creature_hit(index);
                }
                let c = &mut self.creatures[index];
                c.health -= 4;
                c.hurt = 0.3;
                c.anger = 15.0;
                let killed = c.health <= 0;
                let hostile = c.kind.hostile();
                if arrow.from_player {
                    self.record_counter("damage_dealt", 4);
                    if killed {
                        self.record_counter("mob_kills", 1);
                        if hostile {
                            self.award_achievement(13);
                        }
                    }
                }
                arrow.pos += direction * distance;
                arrow.age = 121.0;
            } else if let Some(distance) = player_hit.filter(|&t| t < obstacle) {
                self.damage(4, "Arrow");
                arrow.pos += direction * distance;
                arrow.age = 121.0;
            } else if let Some(hit) = wall {
                arrow.pos += direction * hit.dist;
                arrow.stuck = true;
                arrow.velocity = Vec4::ZERO;
            } else {
                arrow.pos += delta;
            }
            if !arrow.stuck {
                let drag: f32 = if self
                    .world
                    .block(arrow.pos.floor().as_ivec4())
                    .is_some_and(block::is_water)
                {
                    0.8
                } else {
                    0.99
                };
                arrow.velocity *= drag.powf(dt * 20.0);
                arrow.velocity.y -= 12.0 * dt;
            }
        }
        arrows.retain(|a| a.age < 120.0 && a.pos.y > -32.0);
        self.arrows = arrows;
    }

    pub fn explode(&mut self, center: Vec4) {
        self.explode_with_radius(center, 3);
    }
    pub(super) fn explode_with_radius(&mut self, center: Vec4, radius: u8) {
        self.emit_sound("random.explode", center, 1.0, 1.0);
        self.emit_explosion_particles(center,radius);
        let snapshot = blast_snapshot(&self.world, center, radius);
        let get = |p| snapshot_state(&snapshot, p);
        let delta = self.body.pos + Vec4::Y * 0.9 - center;
        let distance = delta.length();
        if distance < f32::from(radius) * 2.0 {
            let exposure = mc4d_core::explosion::exposure(center, self.body.bounds(), &get);
            self.damage(
                mc4d_core::explosion::damage(radius, distance, exposure),
                "Explosion",
            );
            self.body.vel += delta.normalize_or_zero()
                * ((1.0 - distance / (f32::from(radius) * 2.0)) * exposure * 20.0);
        }
        for c in &mut self.creatures {
            let distance = (c.center() - center).length();
            if distance >= f32::from(radius) * 2.0 {
                continue;
            }
            let exposure = mc4d_core::explosion::exposure(center, c.body.bounds(), &get);
            c.health -= i32::from(mc4d_core::explosion::damage(radius, distance, exposure));
        }
        self.dropped_items.retain_mut(|drop|{let delta=drop.body.pos+Vec4::Y*DROP_RADIUS-center;let distance=delta.length();if distance<f32::from(radius)*2.0 {let exposure=mc4d_core::explosion::exposure(center,drop.body.bounds(),&get);let damage=mc4d_core::explosion::damage(radius,distance,exposure);drop.health=drop.health.saturating_sub(damage);drop.body.vel+=delta.normalize_or_zero()*(1.0-distance/(f32::from(radius)*2.0))*exposure*20.0;}drop.health>0});
        self.vehicles_blast(center,radius,&get);
        self.paintings_blast(center,radius,&get);
        self.explosions.push(Explosion {
            center,
            next_cell: 0,
            radius,
            seed: self.rng & i64::MAX as u64,
            next_ray: 0,
            planned: false,
            affected: Vec::new(),
            snapshot: Some(snapshot),
            seen: Default::default(),
        });
    }

    fn update_explosions(&mut self) {
        let mut ray_budget = 512;
        let mut edit_budget = 128;
        while edit_budget > 0 {
            if self
                .explosions
                .first()
                .is_some_and(|e| !self.world.is_loaded(chunk_of(e.center.floor().as_ivec4())))
            {
                self.explosions.rotate_left(1);
                break;
            }
            let Some(e) = self.explosions.first_mut() else {
                break;
            };
            if !e.planned {
                if e.snapshot.is_none() {
                    e.snapshot = Some(blast_snapshot(&self.world, e.center, e.radius));
                    e.affected.clear();
                    e.seen.clear();
                    e.next_ray = 0;
                    e.next_cell = 0;
                }
                if e.seen.is_empty() && !e.affected.is_empty(){e.seen.extend(e.affected.iter().copied());}
                while ray_budget > 0 && e.next_ray < mc4d_core::explosion::DIRECTION_CELLS {
                    let index = e.next_ray;
                    e.next_ray += 1;
                    ray_budget -= 1;
                    if let Some(direction) = mc4d_core::explosion::direction(index) {
                        let strength = mc4d_core::explosion::strength(e.radius, e.seed, index);
                        let snapshot = e.snapshot.as_ref().unwrap();
                        let seen = &mut e.seen;
                        let affected = &mut e.affected;
                        mc4d_core::explosion::trace(
                            e.center,
                            direction,
                            strength,
                            &|p| snapshot_state(snapshot, p),
                            |p| {
                                if seen.insert(p) {
                                    affected.push(p);
                                }
                            },
                        );
                    }
                }
                if e.next_ray < mc4d_core::explosion::DIRECTION_CELLS {
                    break;
                }
                e.planned = true;
                e.snapshot = None;
                e.seen.clear();
            }
            if e.next_cell >= e.affected.len() {
                self.explosions.remove(0);
                continue;
            }
            let p = e.affected[e.next_cell];
            e.next_cell += 1;
            edit_budget -= 1;
            let Some(state) = self.world.block_state(p) else {
                continue;
            };
            if state.id == AIR || state.id == block::BEDROCK {
                continue;
            }
            if state.id == block::TNT {
                self.prime_tnt(p, true);
            } else {
                let loot_state = self.paired_loot_state(p, state);
                self.break_station(p);
                self.world.set_block(p, AIR);
                self.emit_block_particles(p,state,false);
                self.break_building_pair(p, state);
                // px calls the original block drop helper with a0.3 probability.
                let stacks = mc4d_core::loot::drops(
                    loot_state,
                    mc4d_core::loot::DropCause::Natural,
                    |bound| (self.rand() * bound as f32) as u32,
                );
                for stack in stacks {
                    if self.rand() < 0.3 {
                        self.spawn_drop(stack, p.as_vec4() + Vec4::splat(0.5), Vec4::Y * 2.0, 0.5);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena() -> Game {
        let mut g = Game::new(None, 7, GameMode::Survival);
        g.body.pos.y += 3.0;
        g.environment.world_ticks = 18_000;
        let base = g.body.pos.floor().as_ivec4();
        for x in -2..=9 {
            for y in -1..=3 {
                for z in -1..=1 {
                    for w in -1..=1 {
                        g.world.set_block(base + IVec4::new(x, y, z, w), AIR);
                    }
                }
            }
        }
        g.creatures.clear();
        g
    }

    #[test]
    fn incremental_blast_keeps_snapshot_shielding_after_near_wall_is_destroyed() {
        let mut g = Game::menu_preview(9);
        let mut cells = vec![AIR; mc4d_core::chunk::CHUNK_VOLUME];
        for (i, id) in cells.iter_mut().enumerate() {
            if i & 15 == 9 {
                *id = block::STONE;
            }
        }
        g.world
            .insert_chunk(IVec4::ZERO, mc4d_core::chunk::Chunk::from_dense(cells));
        g.body.pos = Vec4::splat(2.0);
        let target = IVec4::new(10, 8, 8, 8);
        g.world.set_block(target, block::PLANKS);
        g.explode_with_radius(Vec4::splat(8.5), 4);
        for _ in 0..100 {
            g.update_explosions();
            if g.explosions.is_empty() {
                break;
            }
        }
        assert!(
            g.explosions.is_empty(),
            "bounded planner and edit queue should finish"
        );
        assert_eq!(
            g.world.block(target),
            Some(block::PLANKS),
            "destroying shield cells must not let later blast rays pass through the original wall"
        );
        assert_eq!(
            g.world.block(IVec4::new(9, 8, 8, 8)),
            Some(AIR),
            "near wall actually took blast damage"
        );
    }

    #[test]
    fn actual_creatures_use_reference_collision_sizes_and_box_targeting() {
        for (kind, width, height) in [
            (CreatureKind::Zombie, 0.6, 1.8),
            (CreatureKind::Skeleton, 0.6, 1.8),
            (CreatureKind::Creeper, 0.6, 1.8),
            (CreatureKind::Pig, 0.9, 0.9),
            (CreatureKind::Cow, 0.9, 1.3),
            (CreatureKind::Sheep, 0.9, 1.3),
            (CreatureKind::Chicken, 0.3, 0.4),
            (CreatureKind::Spider, 1.4, 0.9),
            (CreatureKind::Wolf, 0.8, 0.8),
            (CreatureKind::Squid, 0.95, 0.95),
        ] {
            let c = Creature::new(kind, Vec4::ZERO);
            assert!((c.body.half_width * 2.0 - width).abs() < 1e-6, "{kind:?}");
            assert!((c.body.height - height).abs() < 1e-6, "{kind:?}");
            assert_eq!(c.center().y, height * 0.5);
        }
        let zombie = Creature::new(CreatureKind::Zombie, Vec4::ZERO);
        assert!(
            zombie
                .target_distance(Vec4::new(0.0, 1.7, -3.0, 0.0), Vec4::Z, 6.0)
                .is_some(),
            "head is targetable above old sphere"
        );
        assert!(
            zombie
                .target_distance(Vec4::new(0.5, 0.8, -3.0, 0.0), Vec4::Z, 6.0)
                .is_none(),
            "ray outside narrow body must miss"
        );
        assert!(
            zombie
                .target_distance(Vec4::new(0.0, 0.8, -3.0, 0.5), Vec4::Z, 6.0)
                .is_none(),
            "W has the same physical extent as X/Z"
        );
        assert!(
            zombie
                .target_distance(Vec4::new(0.0, 1.7, 0.0, -3.0), Vec4::W, 6.0)
                .is_some()
        );
    }
    #[test]
    fn arrows_hit_player_head_before_wall_and_miss_outside_w_bounds() {
        let mut g = Game::menu_preview(3);
        g.world
            .insert_chunk(IVec4::ZERO, mc4d_core::chunk::Chunk::uniform(AIR));
        g.body.pos = Vec4::splat(8.0);
        g.world.set_block(IVec4::new(9, 9, 8, 8), block::STONE);
        g.arrows.push(Arrow {
            pos: g.body.pos + Vec4::Y * 1.7 - Vec4::X * 2.0,
            velocity: Vec4::X * 40.0,
            age: 0.0,
            stuck: false,
            from_player: false,
            from_living: false,
            ground_time: 0.0,
        });
        g.update_arrows(0.1);
        assert_eq!(g.health, 16, "player head is closer than the wall");
        assert!(g.arrows.is_empty());
        g.health = 20;
        g.invulnerability = 0.0;
        g.world.set_block(IVec4::new(9, 9, 8, 8), AIR);
        g.arrows.push(Arrow {
            pos: g.body.pos + Vec4::Y * 0.9 + Vec4::W * 0.5 - Vec4::X * 2.0,
            velocity: Vec4::X * 40.0,
            age: 0.0,
            stuck: false,
            from_player: false,
            from_living: false,
            ground_time: 0.0,
        });
        g.update_arrows(0.1);
        assert_eq!(g.health, 20, "ray outside player's W width must miss");
    }

    #[test]
    fn restored_mobs_ignore_legacy_incorrect_body_radius_and_keep_slime_size() {
        let c = Creature::new(CreatureKind::Zombie, Vec4::ZERO);
        let mut saved = c.save();
        saved.radius = 0.65;
        let restored = Creature::restore(saved).unwrap();
        assert_eq!(restored.body.height, 1.8);
        assert_eq!(restored.body.half_width, 0.3);
        let mut slime = Creature::new(CreatureKind::Slime, Vec4::ZERO);
        slime.set_slime_size(4);
        slime.health = 12;
        let saved: SavedCreature =
            toml::from_str(&toml::to_string(&slime.save()).unwrap()).unwrap();
        let restored = Creature::restore(saved).unwrap();
        assert_eq!(restored.slime_size, 4);
        assert_eq!(restored.body.height, 2.4);
        assert_eq!(restored.health, 12);
    }

    #[test]
    fn cow_milk_replaces_one_bucket_and_supplies_real_cake_recipe() {
        let mut g = arena();
        g.spawn_creature(CreatureKind::Cow, g.body.pos + Vec4::X * 2.0);
        g.target = Some(Target::Creature(0));
        let health = g.creatures[0].health;
        for slot in 0..3 {
            g.hotbar[0] = Some(Slot::new(item::BUCKET, 1));
            assert!(g.interact_creature());
            assert_eq!(g.hotbar[0], Some(Slot::new(item::MILK, 1)));
            g.crafting_grid[slot] = g.hotbar[0].take();
        }
        g.crafting_width = 3;
        g.crafting_grid[3] = Some(Slot::new(item::SUGAR, 1));
        g.crafting_grid[4] = Some(Slot::new(item::EGG, 1));
        g.crafting_grid[5] = Some(Slot::new(item::SUGAR, 1));
        for slot in 6..9 {
            g.crafting_grid[slot] = Some(Slot::new(item::WHEAT, 1));
        }
        let mut output = None;
        assert!(g.take_crafting_output(&mut output));
        assert_eq!(output, Some(Slot::new(item::CAKE_ITEM, 1)));
        for slot in 0..3 {
            assert_eq!(g.crafting_grid[slot], Some(Slot::new(item::BUCKET, 1)));
        }
        assert_eq!(g.creatures[0].health, health);
        assert!(g.dropped_items.is_empty());
        g.hotbar[0] = Some(Slot::new(item::MILK, 1));
        g.target = None;
        assert!(!g.use_selected_tool());
        assert_eq!(g.hotbar[0], Some(Slot::new(item::MILK, 1)));
        let cell = g.body.pos.floor().as_ivec4();
        g.world.set_block(cell, block::STONE);
        g.target = Some(Target::Block(Hit {
            block: cell,
            normal: IVec4::Y,
            dist: 2.0,
        }));
        assert!(g.use_selected_tool());
        assert_eq!(g.hotbar[0], Some(Slot::new(item::BUCKET, 1)));
    }
    #[test]
    fn sheep_dye_consumes_only_on_change_and_fleece_keeps_saved_color() {
        let mut g = arena();
        g.spawn_creature(CreatureKind::Sheep, g.body.pos + Vec4::X * 2.0);
        g.target = Some(Target::Creature(0));
        g.hotbar[0] = Some(Slot::with_variant(item::DYE, 3, 1));
        assert!(g.interact_creature());
        assert_eq!(g.creatures[0].wool_color, 14);
        assert_eq!(g.hotbar[0].unwrap().count, 2);
        assert!(g.interact_creature());
        assert_eq!(
            g.hotbar[0].unwrap().count,
            2,
            "same color does not consume dye"
        );
        let saved: SavedCreature =
            toml::from_str(&toml::to_string(&g.creatures[0].save()).unwrap()).unwrap();
        g.creatures[0] = Creature::restore(saved).unwrap();
        assert_eq!((g.entity_draws()[0].state >> 8) & 15, 14);
        g.on_creature_hit(0);
        assert!(g.creatures[0].sheared);
        assert!(
            g.dropped_items
                .iter()
                .all(|d| d.stack.block == item::ItemId::from(block::WOOL)
                    && d.stack.variant == 14
                    && d.stack.count == 1)
        );
        assert!((1..=3).contains(&g.dropped_items.len()));
        g.hotbar[0] = Some(Slot::with_variant(item::DYE, 2, 4));
        g.interact_creature();
        assert_eq!(
            g.hotbar[0].unwrap().count,
            2,
            "sheared fleece cannot be dyed"
        );
        g.creatures[0].health = 0;
        let count = g.dropped_items.len();
        g.collect_dead_creatures();
        assert_eq!(g.dropped_items.len(), count);
        g.spawn_creature(CreatureKind::Sheep, g.body.pos + Vec4::X * 2.0);
        g.creatures[0].health = 0;
        g.collect_dead_creatures();
        assert_eq!(
            g.dropped_items.len(),
            count,
            "environmental death adds no fleece in Beta1.5"
        );
    }

    #[test]
    fn sheep_drop_wool_once_and_slimes_split_into_real_entities() {
        let mut g = arena();
        g.spawn_creature(CreatureKind::Sheep, g.body.pos + Vec4::X * 2.0);
        g.on_creature_hit(0);
        let fleece = g
            .dropped_items
            .iter()
            .filter(|d| d.stack.block == item::ItemId::from(block::WOOL))
            .map(|d| d.stack.count)
            .sum::<u32>();
        assert!((1..=3).contains(&fleece));
        g.on_creature_hit(0);
        g.creatures[0].health = 0;
        g.collect_dead_creatures();
        assert_eq!(
            g.dropped_items
                .iter()
                .filter(|d| d.stack.block == item::ItemId::from(block::WOOL))
                .map(|d| d.stack.count)
                .sum::<u32>(),
            fleece
        );
        g.spawn_creature(CreatureKind::Slime, g.body.pos + Vec4::X * 3.0);
        g.creatures[0].set_slime_size(2);
        g.creatures[0].health = 0;
        g.collect_dead_creatures();
        assert_eq!(g.creatures.len(), 4);
        assert!(g.creatures.iter().all(|c| c.kind == CreatureKind::Slime
            && c.slime_size == 1
            && c.body.height == 0.6
            && c.health == 1));
    }

    #[test]
    fn skeleton_projectiles_damage_player_and_creeper_fuse_detonates() {
        let mut g = arena();
        g.spawn_creature(CreatureKind::Skeleton, g.body.pos + Vec4::X * 6.0);
        g.update_creatures(0.0);
        assert_eq!(
            g.arrows.len(),
            1,
            "skeleton must create a moving projectile"
        );
        g.arrows = vec![Arrow {
            pos: g.body.pos + Vec4::Y * 0.9 - Vec4::X,
            velocity: Vec4::X * 20.0,
            age: 0.0,
            stuck: false,
            from_player: false,
            from_living: true,
            ground_time: 0.0,
        }];
        g.update_arrows(0.1);
        assert_eq!(g.health, 16);
        assert!(g.arrows.is_empty());
        g.creatures.clear();
        g.invulnerability = 0.0;
        g.spawn_creature(CreatureKind::Creeper, g.body.pos + Vec4::X * 2.0);
        g.creatures[0].fuse = 1.49;
        g.update_creatures(0.02);
        assert!(g.creatures.is_empty());
        assert!(!g.explosions.is_empty());
        assert_eq!(
            g.explosions[0].next_ray, 512,
            "blast planning work is bounded per frame"
        );
        assert_eq!(
            g.explosions[0].next_cell, 0,
            "no geometry changes until every ray saw the original snapshot"
        );
        assert!(
            g.health < 16,
            "nearby unobstructed explosion causes actual damage"
        );
    }

    #[test]
    fn wolves_tame_consume_bones_heal_and_sit() {
        let mut g = arena();
        g.spawn_creature(CreatureKind::Wolf, g.body.pos + Vec4::X * 2.0);
        g.target = Some(Target::Creature(0));
        g.hotbar[0] = Some(Slot::new(item::BONE, 64));
        for _ in 0..64 {
            g.interact_creature();
            if g.creatures[0].tamed {
                break;
            }
        }
        assert!(g.creatures[0].tamed);
        assert!(g.hotbar[0].map_or(0, |s| s.count) < 64);
        g.creatures[0].health = 10;
        g.hotbar[0] = Some(Slot::new(item::COOKED_PORK, 1));
        g.interact_creature();
        assert_eq!(g.creatures[0].health, 18);
        assert!(g.hotbar[0].is_none());
        g.interact_creature();
        assert!(g.creatures[0].sitting);
        let saved = toml::to_string(&g.creatures[0].save()).unwrap();
        let restored = Creature::restore(toml::from_str(&saved).unwrap()).unwrap();
        assert!(restored.tamed && restored.sitting);
        assert_eq!(restored.health, 18);
        assert_eq!(restored.kind, CreatureKind::Wolf);
    }

    #[test]
    fn squid_always_drop_ink_and_skeleton_bones_roll_separately() {
        let mut g = Game::menu_preview(7);
        let mut squid = Vec::new();
        let mut uncorrelated = false;
        for _ in 0..200 {
            g.dropped_items.clear();
            g.spawn_creature(CreatureKind::Squid, Vec4::splat(8.0));
            g.spawn_creature(CreatureKind::Skeleton, Vec4::splat(8.0));
            for c in &mut g.creatures {
                c.health = 0;
            }
            g.collect_dead_creatures();
            let count = |id| {
                g.dropped_items
                    .iter()
                    .filter(|d| d.stack.block == id)
                    .count()
            };
            squid.push(count(item::INK_SAC));
            uncorrelated |= count(item::ARROW) != count(item::BONE);
        }
        assert_eq!(squid.iter().min(), Some(&1));
        assert_eq!(squid.iter().max(), Some(&3));
        assert!(uncorrelated, "arrow and bone counts are independent rolls");
    }

    #[test]
    fn passive_animals_flee_tamed_wolves_follow_and_squid_swim() {
        let mut g = arena();
        g.spawn_creature(CreatureKind::Cow, g.body.pos + Vec4::X * 3.0);
        g.creatures[0].anger = 10.0;
        g.update_creatures(0.01);
        assert!(g.creatures[0].body.vel.x > 0.0, "hurt cow flees the player");
        g.creatures.clear();
        g.spawn_creature(CreatureKind::Wolf, g.body.pos + Vec4::X * 7.0);
        g.creatures[0].tamed = true;
        g.update_creatures(0.01);
        assert!(g.creatures[0].body.vel.x < 0.0, "tamed wolf follows owner");
        g.creatures[0].sitting = true;
        g.update_creatures(0.01);
        assert_eq!(g.creatures[0].body.vel.x, 0.0);
        g.creatures.clear();
        let pos = g.body.pos + Vec4::X * 3.0;
        let cell = pos.floor().as_ivec4();
        for x in -1..=1 {
            for y in 0..=2 {
                for z in -1..=1 {
                    for w in -1..=1 {
                        g.world
                            .set_block(cell + IVec4::new(x, y, z, w), block::WATER);
                    }
                }
            }
        }
        g.spawn_creature(CreatureKind::Squid, pos);
        g.update_creatures(0.0);
        g.update_creatures(0.01);
        assert!(g.creatures[0].body.in_water);
        assert!(
            g.creatures[0].body.vel.y.abs() < 0.5,
            "squid swims instead of receiving normal gravity"
        );
    }

    #[test]
    fn undead_burn_under_open_daylight_but_rain_protects_them() {
        let mut g = arena();
        let pos = g.body.pos + Vec4::X * 3.0;
        let cell = pos.floor().as_ivec4();
        for y in cell.y..mc4d_core::chunk::WORLD_HEIGHT {
            g.world
                .set_block(IVec4::new(cell.x, y, cell.z, cell.w), AIR);
        }
        g.environment.world_ticks = 6_000;
        g.environment.raining = false;
        g.spawn_creature(CreatureKind::Zombie, pos);
        g.update_creatures(0.0);
        assert_eq!(g.creatures[0].health, 18);
        g.environment.raining = true;
        g.creatures[0].ai_timer = 0.0;
        g.update_creatures(0.0);
        assert_eq!(g.creatures[0].health, 18);
    }

    #[test]
    fn chicken_lays_pickable_egg_and_blast_respects_bedrock() {
        let mut g = arena();
        g.spawn_creature(CreatureKind::Chicken, g.body.pos + Vec4::X * 3.0);
        g.creatures[0].egg_timer = 0.0;
        g.update_creatures(0.0);
        assert!(g.dropped_items.iter().any(|d| d.stack.block == item::EGG));
        let center = g.body.pos + Vec4::X * 6.0;
        let p = center.floor().as_ivec4();
        g.world.set_block(p, block::STONE);
        g.world.set_block(p + IVec4::X, block::BEDROCK);
        g.explode(center);
        for _ in 0..20 {
            g.update_explosions();
        }
        assert_eq!(g.world.block(p), Some(AIR));
        assert_eq!(g.world.block(p + IVec4::X), Some(block::BEDROCK));
    }
    #[test]
    fn in_progress_blast_keeps_original_shielding_and_progress_after_real_save_reload(){
        let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/blast-save-tests").join(format!("{}-{stamp}",std::process::id()));
        let mut game=Game::try_load(Some(dir.clone()),7,GameMode::Creative).unwrap();
        game.world.insert_chunk(IVec4::ZERO,mc4d_core::chunk::Chunk::uniform(AIR));
        for y in 0..16{for z in 0..16{for w in 0..16{game.world.set_block(IVec4::new(9,y,z,w),block::STONE);}}}
        let target=IVec4::new(10,8,8,8);game.world.set_block(target,block::PLANKS);game.body.pos=Vec4::splat(8.5);
        game.explode_with_radius(Vec4::splat(8.5),4);game.update_explosions();assert_eq!(game.explosions[0].next_ray,512);
        // Another action removes the shield after the blast has captured it.
        for y in 0..16{for z in 0..16{for w in 0..16{game.world.set_block(IVec4::new(9,y,z,w),AIR);}}}
        game.save_checked().unwrap();drop(game);
        let mut restored=Game::try_load(Some(dir.clone()),7,GameMode::Creative).unwrap();
        assert_eq!(restored.explosions[0].next_ray,512);assert!(restored.explosions[0].snapshot.is_some());
        for _ in 0..100{restored.update_explosions();if restored.explosions.is_empty(){break;}}
        assert!(restored.explosions.is_empty());assert_eq!(restored.world.block(target),Some(block::PLANKS),"resume must not recalculate blast rays through the removed wall");
        drop(restored);std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn planned_blast_serializes_without_snapshot_and_resumes_its_edit_cursor(){
        let mut game=Game::menu_preview(7);game.mode=GameMode::Creative;game.world.insert_chunk(IVec4::ZERO,mc4d_core::chunk::Chunk::uniform(block::LEAVES));game.explode_with_radius(Vec4::splat(8.5),4);
        for _ in 0..20 {game.update_explosions();if game.explosions.first().is_some_and(|e|e.planned){break;}}
        let e=game.explosions.first().expect("large soft-block blast has incremental edits");assert!(e.planned);assert!(e.next_cell>0);assert!(e.snapshot.is_none());
        let restored:Explosion=toml::from_str(&toml::to_string(e).unwrap()).unwrap();assert_eq!(restored.next_cell,e.next_cell);assert_eq!(restored.affected,e.affected);assert!(restored.snapshot.is_none());game.explosions[0]=restored;
        for _ in 0..200 {game.update_explosions();if game.explosions.is_empty(){break;}}assert!(game.explosions.is_empty());
    }

}
