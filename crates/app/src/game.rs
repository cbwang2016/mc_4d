//! Game state and per-frame simulation. Independent of windowing so it can also drive
//! headless screenshots.

use crate::input::{Action, Bindings, InputState};
use crate::settings::Settings;
use mc4d_core::block::{self, AIR, BlockId, WATER};
use mc4d_core::camera::{Basis, Orientation};
use mc4d_core::chunk::{CHUNK, chunk_of};
use mc4d_core::glam::{IVec4, Quat, Vec4};
use mc4d_core::physics::{Body, GRAVITY, JUMP_SPEED};
use mc4d_core::raycast::{Hit, raycast};
use mc4d_core::save::SaveDir;
use mc4d_core::terrain::SEA_LEVEL;
use mc4d_core::world::{ChunkSource, World, chunks_near_slice};
use mc4d_render::EntityDraw;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

const REACH: f32 = 6.0;
const MAX_CREATURES: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum GameMode {
    Survival,
    Creative,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub block: BlockId,
    pub count: u32,
}

const CREATIVE_HOTBAR: [BlockId; 9] = [
    block::GRASS,
    block::DIRT,
    block::STONE,
    block::COBBLE,
    block::PLANKS,
    block::LOG,
    block::GLASS,
    block::BRICK,
    block::GLOW,
];

/// A 4D hypersphere creature ("gloom") that hops around.
pub struct Creature {
    pub body: Body,
    pub radius: f32,
    pub color: [f32; 3],
    hop_timer: f32,
    pub hurt: f32,
    pub health: i32,
}

impl Creature {
    pub fn center(&self) -> Vec4 {
        self.body.pos + Vec4::Y * self.radius
    }
}

/// What the crosshair points at.
#[derive(Clone, Copy, Debug)]
pub enum Target {
    Block(Hit),
    Creature(usize),
}

/// Running totals of what the player has done (drives the tutorial).
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Radians of mouse look.
    pub looked: f32,
    /// Blocks walked within the slice (excluding vertical and ana motion).
    pub moved_slice: f32,
    /// Blocks moved along ana/kata.
    pub moved_ana: f32,
    /// Radians turned in the forward↔ana plane.
    pub turned_forward_ana: f32,
    /// Radians turned in the right↔ana plane.
    pub turned_right_ana: f32,
    pub snaps: u32,
    pub flips: u32,
    pub broken: u32,
    pub placed: u32,
    pub creatures_hit: u32,
}

/// Saved per-world state (`level.toml`).
#[derive(Serialize, Deserialize)]
struct Level {
    seed: u32,
    mode: GameMode,
    pos: [f32; 4],
    heading: [f32; 4],
    pitch: f32,
    flying: bool,
    selected: usize,
    /// Empty slots are stored with count 0 (TOML has no null).
    hotbar: Vec<Slot>,
}

pub struct Game {
    pub world: World,
    save: Option<Arc<SaveDir>>,
    pub seed: u32,
    pub body: Body,
    pub orient: Orientation,
    pub mode: GameMode,
    pub flying: bool,
    pub hotbar: [Option<Slot>; 9],
    pub selected: usize,
    pub creatures: Vec<Creature>,
    pub target: Option<Target>,
    pub time: f32,
    pub stats: Stats,
    break_cooldown: f32,
    place_cooldown: f32,
    rng: u64,
    spawn: Vec4,
    /// Chunks the renderer should have, nearest first.
    pub gpu_wanted: Vec<IVec4>,
    load_key: Option<(IVec4, [i32; 4], u32, i32)>,
    load_wanted: Vec<IVec4>,
    unload_timer: f32,
    autosave_timer: f32,
    /// Short on-screen messages with remaining time.
    pub toasts: Vec<(String, f32)>,
}

impl Game {
    /// Load a world from `dir` (or create it with `seed`). `dir = None` is an unsaved world.
    pub fn new(dir: Option<PathBuf>, seed: u32, mode: GameMode) -> Self {
        let save = dir.and_then(|d| SaveDir::new(d).map_err(|e| log::error!("save dir: {e}")).ok()).map(Arc::new);
        let level: Option<Level> = save
            .as_ref()
            .and_then(|s| std::fs::read_to_string(s.root().join("level.toml")).ok())
            .and_then(|t| toml::from_str(&t).map_err(|e| log::warn!("level.toml: {e}")).ok());
        let seed = level.as_ref().map_or(seed, |l| l.seed);
        let world = World::new(seed, save.clone().map(|s| s as Arc<dyn ChunkSource>));
        let spawn = find_spawn(&world);

        let mut game = Self {
            world,
            save,
            seed,
            body: Body::new(spawn),
            orient: Orientation::default(),
            mode,
            flying: false,
            hotbar: [None; 9],
            selected: 0,
            creatures: Vec::new(),
            target: None,
            time: 0.0,
            stats: Stats::default(),
            break_cooldown: 0.0,
            place_cooldown: 0.0,
            rng: 0x2545_F491_4F6C_DD1D ^ seed as u64,
            spawn,
            gpu_wanted: Vec::new(),
            load_key: None,
            load_wanted: Vec::new(),
            unload_timer: 0.0,
            autosave_timer: 0.0,
            toasts: Vec::new(),
        };
        if let Some(l) = level {
            game.mode = l.mode;
            game.body.pos = Vec4::from_array(l.pos);
            game.orient.heading = Quat::from_array(l.heading).normalize();
            game.orient.pitch = l.pitch;
            game.flying = l.flying && l.mode == GameMode::Creative;
            game.selected = l.selected.min(8);
            for (i, s) in l.hotbar.into_iter().take(9).enumerate() {
                game.hotbar[i] = (s.count > 0).then_some(s);
            }
        } else if mode == GameMode::Creative {
            game.fill_creative_hotbar();
        }
        game.load_around_now(24.0);
        // Never start inside a tree or a block placed since the last save.
        for _ in 0..64 {
            let world = &game.world;
            if !game.body.is_stuck(&|p| world.block(p)) {
                break;
            }
            game.body.pos.y += 1.0;
        }
        game
    }

    fn fill_creative_hotbar(&mut self) {
        for (i, b) in CREATIVE_HOTBAR.iter().enumerate() {
            if self.hotbar[i].is_none() {
                self.hotbar[i] = Some(Slot { block: *b, count: 1 });
            }
        }
    }

    pub fn set_mode(&mut self, mode: GameMode) {
        self.mode = mode;
        if mode == GameMode::Creative {
            self.fill_creative_hotbar();
        } else {
            self.flying = false;
        }
    }

    pub fn eye(&self) -> Vec4 {
        self.body.eye()
    }

    pub fn basis(&self) -> Basis {
        self.orient.basis()
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toasts.push((msg.into(), 2.5));
    }

    /// Synchronously generate the chunks within `radius` of the player in the slice.
    pub fn load_around_now(&mut self, radius: f32) {
        let b = self.basis();
        for c in chunks_near_slice(self.eye(), b.ana, radius, 2.0) {
            self.world.load_now(c, self.save.as_deref().map(|s| s as &dyn ChunkSource));
        }
        self.update_streaming(&Settings::default(), true);
    }

    /// Advance the simulation by `dt` seconds. `input = None` means no player control (menu open).
    pub fn update(&mut self, dt: f32, input: Option<(&InputState, &Bindings)>, settings: &Settings) {
        let dt = dt.min(0.1);
        self.time += dt;
        self.toasts.retain_mut(|t| {
            t.1 -= dt;
            t.1 > 0.0
        });

        if let Some((input, bindings)) = input {
            self.handle_look(dt, input, bindings, settings);
            self.handle_actions(input, bindings, settings);
        }
        self.orient.update(dt, settings.auto_snap);

        let empty = InputState::default();
        let bindings_default;
        let (input, bindings) = match input {
            Some(x) => x,
            None => {
                bindings_default = Bindings::default();
                (&empty, &bindings_default)
            }
        };
        self.move_player(dt, input, bindings);
        self.update_target();
        self.interact(dt, input, bindings);
        self.update_creatures(dt);
        self.update_streaming(settings, false);

        self.autosave_timer += dt;
        if self.autosave_timer > 120.0 {
            self.autosave_timer = 0.0;
            self.save_all();
        }
    }

    fn handle_look(&mut self, dt: f32, input: &InputState, b: &Bindings, s: &Settings) {
        let sens = 0.0022 * s.mouse_sensitivity;
        let (dx, dy) = (input.mouse_delta.0 as f32, input.mouse_delta.1 as f32);
        if input.is_down(b, Action::SliceModifier) {
            let k = sens * s.slice_mouse_sensitivity;
            if dx != 0.0 {
                self.orient.rotate_right_ana(dx * k);
                self.stats.turned_right_ana += (dx * k).abs();
            }
            if dy != 0.0 {
                self.orient.rotate_forward_ana(-dy * k);
                self.stats.turned_forward_ana += (dy * k).abs();
            }
        } else {
            self.orient.yaw(dx * sens);
            let inv = if s.invert_y { -1.0 } else { 1.0 };
            self.orient.add_pitch(-dy * sens * inv);
            self.stats.looked += (dx.abs() + dy.abs()) * sens;
        }

        let step = s.wheel_step_deg.to_radians();
        let rate = s.key_rotate_speed.to_radians() * dt;
        let axis = |pos: Action, neg: Action| {
            let wheel = input.wheel_steps(b, pos) as f32 - input.wheel_steps(b, neg) as f32;
            let held = input.held(b, pos) as i32 as f32 - input.held(b, neg) as i32 as f32;
            wheel * step + held * rate
        };
        let fa = axis(Action::RotateAnaPos, Action::RotateAnaNeg);
        if fa != 0.0 {
            self.orient.rotate_forward_ana(fa);
            self.stats.turned_forward_ana += fa.abs();
        }
        let ra = axis(Action::RotateRightAnaPos, Action::RotateRightAnaNeg);
        if ra != 0.0 {
            self.orient.rotate_right_ana(ra);
            self.stats.turned_right_ana += ra.abs();
        }
    }

    fn handle_actions(&mut self, input: &InputState, b: &Bindings, _s: &Settings) {
        if input.just_pressed(b, Action::SnapSlice) {
            self.orient.snap();
            self.stats.snaps += 1;
        }
        if input.just_pressed(b, Action::FlipAna) {
            self.orient.flip_ana();
            self.stats.flips += 1;
        }
        if input.just_pressed(b, Action::ToggleFly) {
            if self.mode == GameMode::Creative {
                self.flying = !self.flying;
                self.toast(if self.flying { "Flying" } else { "Walking" });
            } else {
                self.toast("Flying is only available in Creative mode");
            }
        }
        for &a in Action::ALL {
            if let Some(i) = a.hotbar_index()
                && input.just_pressed(b, a) {
                    self.selected = i;
                }
        }
        let next = input.presses(b, Action::HotbarNext) as i32 - input.presses(b, Action::HotbarPrev) as i32;
        self.selected = (self.selected as i32 + next).rem_euclid(9) as usize;
    }

    fn move_player(&mut self, dt: f32, input: &InputState, b: &Bindings) {
        let basis = self.basis();
        let axis = |pos: Action, neg: Action| input.is_down(b, pos) as i32 as f32 - input.is_down(b, neg) as i32 as f32;
        let mut wish = basis.forward_h * axis(Action::MoveForward, Action::MoveBack)
            + basis.right * axis(Action::MoveRight, Action::MoveLeft)
            + basis.ana * axis(Action::MoveAna, Action::MoveKata);
        if wish.length_squared() > 1.0 {
            wish = wish.normalize();
        }
        let sprint = input.is_down(b, Action::Sprint);
        let sneak = input.is_down(b, Action::Sneak);
        let jump = input.is_down(b, Action::Jump);

        // Freeze until the ground under the player exists.
        if !self.world.is_loaded(chunk_of(self.body.pos.floor().as_ivec4())) {
            return;
        }

        if self.flying {
            let speed = if sprint { 22.0 } else { 11.0 };
            let target = wish * speed + Vec4::Y * (jump as i32 - sneak as i32) as f32 * speed * 0.8;
            self.body.vel = self.body.vel.lerp(target, (dt * 10.0).min(1.0));
        } else {
            let speed = if sneak {
                1.4
            } else if sprint {
                5.8
            } else {
                4.4
            };
            let speed = if self.body.in_water { speed * 0.55 } else { speed };
            let accel = if self.body.on_ground || self.body.in_water { 14.0 } else { 3.0 };
            let target = wish * speed;
            let horiz = Vec4::new(self.body.vel.x, 0.0, self.body.vel.z, self.body.vel.w);
            let new_h = horiz.lerp(target, (dt * accel).min(1.0));
            self.body.vel = Vec4::new(new_h.x, self.body.vel.y, new_h.z, new_h.w);
            if self.body.in_water {
                self.body.vel.y -= GRAVITY * 0.25 * dt;
                self.body.vel.y = self.body.vel.y.max(-3.0);
                if jump {
                    self.body.vel.y = 3.2;
                }
            } else {
                self.body.vel.y -= GRAVITY * dt;
                self.body.vel.y = self.body.vel.y.max(-60.0);
                if jump && self.body.on_ground {
                    self.body.vel.y = JUMP_SPEED;
                }
            }
        }
        let world = &self.world;
        let before = self.body.pos;
        self.body.step(dt, &|p| world.block(p));
        let delta = self.body.pos - before;
        let along_ana = delta.dot(basis.ana);
        let in_slice = delta - basis.ana * along_ana;
        self.stats.moved_ana += along_ana.abs();
        self.stats.moved_slice += Vec4::new(in_slice.x, 0.0, in_slice.z, in_slice.w).length();
        if self.body.on_ground && self.flying && sneak {
            self.flying = false;
        }
        if self.body.pos.y < -32.0 {
            self.body = Body::new(self.spawn);
        }
    }

    fn update_target(&mut self) {
        let eye = self.eye();
        let fwd = self.basis().forward;
        let world = &self.world;
        let hit = raycast(eye, fwd, REACH, |p| world.block(p).is_some_and(|b| block::is_visible(b) && b != WATER));
        let block_dist = hit.map_or(REACH, |h| h.dist);
        let mut best: Option<(usize, f32)> = None;
        for (i, c) in self.creatures.iter().enumerate() {
            if let Some(t) = ray_sphere(eye, fwd, c.center(), c.radius)
                && t < block_dist && best.is_none_or(|(_, bt)| t < bt) {
                    best = Some((i, t));
                }
        }
        self.target = match (best, hit) {
            (Some((i, _)), _) => Some(Target::Creature(i)),
            (None, Some(h)) => Some(Target::Block(h)),
            _ => None,
        };
    }

    fn interact(&mut self, dt: f32, input: &InputState, b: &Bindings) {
        self.break_cooldown -= dt;
        self.place_cooldown -= dt;
        let creative = self.mode == GameMode::Creative;

        if input.just_pressed(b, Action::Break) {
            self.break_cooldown = 0.0;
        }
        if input.is_down(b, Action::Break) && self.break_cooldown <= 0.0 {
            match self.target {
                Some(Target::Creature(i)) => {
                    let push = self.basis().forward_h * 6.0 + Vec4::Y * 5.0;
                    let c = &mut self.creatures[i];
                    c.body.vel += push;
                    c.hurt = 0.3;
                    c.health -= 1;
                    self.break_cooldown = 0.4;
                    self.stats.creatures_hit += 1;
                }
                Some(Target::Block(h)) => {
                    let id = self.world.block(h.block).unwrap_or(AIR);
                    if block::info(id).breakable && self.world.set_block(h.block, AIR) {
                        if !creative {
                            self.add_item(if id == block::GRASS { block::DIRT } else if id == block::STONE { block::COBBLE } else { id });
                        }
                        self.break_cooldown = if creative { 0.18 } else { 0.3 };
                        self.stats.broken += 1;
                    }
                }
                None => {}
            }
        }
        self.creatures.retain(|c| c.health > 0);

        if input.just_pressed(b, Action::Place) {
            self.place_cooldown = 0.0;
        }
        if input.is_down(b, Action::Place) && self.place_cooldown <= 0.0
            && let (Some(Target::Block(h)), Some(slot)) = (self.target, self.hotbar[self.selected]) {
                let p = h.block + h.normal;
                let free = matches!(self.world.block(p), Some(AIR) | Some(WATER));
                let blocked_by_creature = self.creatures.iter().any(|c| c.body.overlaps_block(p));
                if h.normal != IVec4::ZERO && free && !self.body.overlaps_block(p) && !blocked_by_creature
                    && self.world.set_block(p, slot.block) {
                        if !creative {
                            self.take_selected();
                        }
                        self.place_cooldown = 0.22;
                        self.stats.placed += 1;
                    }
            }

        if input.just_pressed(b, Action::PickBlock)
            && let Some(Target::Block(h)) = self.target
                && let Some(id) = self.world.block(h.block) {
                    if let Some(i) = self.hotbar.iter().position(|s| s.is_some_and(|s| s.block == id)) {
                        self.selected = i;
                    } else if creative {
                        self.hotbar[self.selected] = Some(Slot { block: id, count: 1 });
                    }
                }
    }

    fn add_item(&mut self, id: BlockId) {
        if let Some(s) = self.hotbar.iter_mut().flatten().find(|s| s.block == id && s.count < 64) {
            s.count += 1;
        } else if let Some(s) = self.hotbar.iter_mut().find(|s| s.is_none()) {
            *s = Some(Slot { block: id, count: 1 });
        }
    }

    fn take_selected(&mut self) {
        if let Some(s) = &mut self.hotbar[self.selected] {
            s.count -= 1;
            if s.count == 0 {
                self.hotbar[self.selected] = None;
            }
        }
    }

    fn update_creatures(&mut self, dt: f32) {
        let player = self.body.pos;
        // Despawn far creatures, spawn new ones around the player in all four directions.
        self.creatures.retain(|c| (c.body.pos - player).length() < 56.0);
        if self.creatures.len() < MAX_CREATURES && self.rand() < dt * 0.5 {
            self.try_spawn_creature();
        }
        for i in 0..self.creatures.len() {
            let r1 = self.rand();
            let r2 = self.rand();
            let r3 = self.rand();
            let r4 = self.rand();
            let c = &mut self.creatures[i];
            c.hurt = (c.hurt - dt).max(0.0);
            c.hop_timer -= dt;
            if c.body.on_ground {
                let damp = (1.0 - dt * 8.0).max(0.0);
                c.body.vel.x *= damp;
                c.body.vel.z *= damp;
                c.body.vel.w *= damp;
                if c.hop_timer <= 0.0 {
                    let dir = Vec4::new(r1 - 0.5, 0.0, r2 - 0.5, r3 - 0.5).normalize_or_zero();
                    // Sometimes hop toward the player out of curiosity.
                    let to_player = Vec4::new(player.x - c.body.pos.x, 0.0, player.z - c.body.pos.z, player.w - c.body.pos.w)
                        .normalize_or_zero();
                    let dir = if r4 < 0.3 { to_player } else { dir };
                    c.body.vel = dir * 3.0 + Vec4::Y * 7.0;
                    c.hop_timer = 1.0 + r4 * 2.5;
                }
            }
            if c.body.in_water {
                c.body.vel.y += GRAVITY * 0.6 * dt;
            }
            c.body.vel.y -= GRAVITY * dt;
            let world = &self.world;
            if world.is_loaded(chunk_of(c.body.pos.floor().as_ivec4())) {
                c.body.step(dt, &|p| world.block(p));
            }
        }
    }

    /// Place a creature with its feet at `pos` (used by the scripted intro video).
    pub fn spawn_creature_at(&mut self, pos: Vec4, radius: f32, color: [f32; 3]) {
        self.creatures.push(Creature {
            body: Body::with_size(pos, radius * 0.9, radius * 2.0),
            radius,
            color,
            hop_timer: 0.5,
            hurt: 0.0,
            health: 3,
        });
    }

    fn try_spawn_creature(&mut self) {
        let basis = self.basis();
        let angle = self.rand() * std::f32::consts::TAU;
        let dist = 8.0 + self.rand() * 20.0;
        // Half of the creatures spawn near the slice so they are visible directly; the rest
        // anywhere in 4D (visible through 4D glasses).
        let off = if self.rand() < 0.5 { (self.rand() - 0.5) * 1.5 } else { (self.rand() - 0.5) * 16.0 };
        let p = self.body.pos + (basis.forward_h * angle.cos() + basis.right * angle.sin()) * dist + basis.ana * off;
        let (x, z, w) = (p.x.floor() as i32, p.z.floor() as i32, p.w.floor() as i32);
        let h = self.world.terrain.height(x, z, w);
        if h <= SEA_LEVEL || self.world.block(IVec4::new(x, h + 1, z, w)) != Some(AIR) {
            return;
        }
        let radius = 0.35 + self.rand() * 0.25;
        let hue = self.rand();
        let color = hsv(hue, 0.55, 0.95);
        let pos = Vec4::new(x as f32 + 0.5, h as f32 + 1.0, z as f32 + 0.5, w as f32 + 0.5);
        let hop_timer = self.rand() * 2.0;
        self.creatures.push(Creature {
            body: Body::with_size(pos, radius * 0.9, radius * 2.0),
            radius,
            color,
            hop_timer,
            hurt: 0.0,
            health: 3,
        });
    }

    /// Request generation of chunks near the slice and work out what the GPU should hold.
    pub fn update_streaming(&mut self, settings: &Settings, force: bool) {
        for _ in self.world.receive() {}
        let eye = self.eye();
        let ana = self.basis().ana;
        let radius = settings.render_distance as f32 * CHUNK as f32;
        // Recompute the sets only when the eye or slice moved noticeably.
        let key = (
            (eye / 4.0).floor().as_ivec4(),
            (ana * 50.0).round().as_ivec4().to_array(),
            settings.render_distance,
            settings.ghosts as i32,
        );
        if force || self.load_key != Some(key) {
            self.load_key = Some(key);
            self.load_wanted = chunks_near_slice(eye, ana, radius, 20.0);
            let margin = if settings.ghosts { settings.ghost_offset + 2.0 } else { 2.0 };
            self.gpu_wanted = chunks_near_slice(eye, ana, radius, margin);
        }
        self.world.request(self.load_wanted.iter().copied(), 64);

        self.unload_timer += 1.0 / 60.0;
        if self.unload_timer > 3.0 {
            self.unload_timer = 0.0;
            let keep: std::collections::HashSet<IVec4> = chunks_near_slice(eye, ana, radius + 32.0, 48.0).into_iter().collect();
            let save = self.save.clone();
            self.world.unload(
                |c| keep.contains(&c),
                |c, chunk| {
                    if let Some(s) = &save
                        && let Err(e) = s.save_chunk(c, chunk) {
                            log::error!("saving chunk {c}: {e}");
                        }
                },
            );
        }
    }

    pub fn entity_draws(&self) -> Vec<EntityDraw> {
        self.creatures
            .iter()
            .map(|c| EntityDraw {
                center: c.center(),
                radius: c.radius,
                color: if c.hurt > 0.0 { [1.0, 0.3, 0.3] } else { c.color },
            })
            .collect()
    }

    pub fn target_block(&self) -> Option<IVec4> {
        match self.target {
            Some(Target::Block(h)) => Some(h.block),
            _ => None,
        }
    }

    /// Save modified chunks and the level file.
    pub fn save_all(&mut self) {
        let Some(save) = &self.save else { return };
        for (&c, chunk) in &self.world.chunks {
            if chunk.modified
                && let Err(e) = save.save_chunk(c, chunk) {
                    log::error!("saving chunk {c}: {e}");
                }
        }
        let level = Level {
            seed: self.seed,
            mode: self.mode,
            pos: self.body.pos.to_array(),
            heading: self.orient.heading.to_array(),
            pitch: self.orient.pitch,
            flying: self.flying,
            selected: self.selected,
            hotbar: self.hotbar.iter().map(|s| s.unwrap_or(Slot { block: AIR, count: 0 })).collect(),
        };
        match toml::to_string_pretty(&level) {
            Ok(t) => {
                if let Err(e) = std::fs::write(save.root().join("level.toml"), t) {
                    log::error!("saving level: {e}");
                }
            }
            Err(e) => log::error!("encoding level: {e}"),
        }
    }
}

/// First dry-land column found spiralling out from the origin.
fn find_spawn(world: &World) -> Vec4 {
    for r in 0..200 {
        for (x, w) in [(r * 7, 0), (0, r * 7), (-r * 7, 0), (0, -r * 7), (r * 5, r * 5)] {
            if world.terrain.height(x, 0, w) > SEA_LEVEL + 1 {
                return world.spawn_point(x, 0, w);
            }
        }
    }
    world.spawn_point(0, 0, 0)
}

fn ray_sphere(o: Vec4, d: Vec4, c: Vec4, r: f32) -> Option<f32> {
    let oc = o - c;
    let b = oc.dot(d);
    let disc = b * b - (oc.dot(oc) - r * r);
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t > 0.0 && t < REACH).then_some(t)
}

fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    match i as i32 % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Binding;
    use winit::keyboard::KeyCode;

    fn settle(game: &mut Game, frames: usize, input: Option<(&InputState, &Bindings)>) {
        let s = Settings::default();
        for _ in 0..frames {
            game.update(1.0 / 60.0, input, &s);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn player_lands_and_walks_along_w() {
        let mut g = Game::new(None, 7, GameMode::Survival);
        settle(&mut g, 120, None);
        assert!(g.body.on_ground, "player should be standing at {:?}", g.body.pos);
        let w0 = g.body.pos.w;
        let b = Bindings::default();
        let mut input = InputState::default();
        input.press(Binding::Key(KeyCode::KeyE));
        settle(&mut g, 30, Some((&input, &b)));
        assert!(g.body.pos.w > w0 + 0.3 || g.body.vel.w.abs() < 1e-3, "E should move along +ana (w)");
    }

    #[test]
    fn break_and_place_in_survival() {
        let mut g = Game::new(None, 7, GameMode::Survival);
        settle(&mut g, 60, None);
        g.orient.add_pitch(-1.5); // look at the ground
        let b = Bindings::default();
        let mut input = InputState::default();
        g.update(1.0 / 60.0, Some((&input, &b)), &Settings::default());
        let Some(Target::Block(hit)) = g.target else { panic!("no target") };
        input.press(Binding::Mouse(crate::input::MouseBtn::Left));
        g.update(1.0 / 60.0, Some((&input, &b)), &Settings::default());
        assert_eq!(g.world.block(hit.block), Some(AIR));
        assert!(g.hotbar.iter().flatten().any(|s| s.count == 1), "broken block collected");
    }
}
