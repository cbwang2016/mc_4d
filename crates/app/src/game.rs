//! Game state and per-frame simulation. Independent of windowing so it can also drive
//! headless screenshots.

use crate::input::{Action, Bindings, InputState};
use crate::settings::Settings;
use mc4d_core::block::{self, AIR, BlockId};
use mc4d_core::camera::{Basis, Orientation};
use mc4d_core::chunk::{CHUNK, chunk_of};
use mc4d_core::glam::{IVec4, Quat, Vec4};
use mc4d_core::physics::{Body, GRAVITY, JUMP_SPEED};
use mc4d_core::raycast::{Hit, raycast_states};
use mc4d_core::save::SaveDir;
use mc4d_core::terrain::SEA_LEVEL;
use mc4d_core::world::{ChunkSource, World, SliceNeighborhood, chunks_near_slice, slice_cache_motion_bound};
use mc4d_render::EntityDraw;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

mod survival;
mod footprint;
mod commands;
mod mobs;
pub use mobs::{CreatureKind, Arrow};
mod mining;
mod hand;
mod building;
mod farming;
mod ecology;
mod lightning;
mod redstone;
mod devices;
mod gravity;
mod fire;
mod vehicles;
mod utilities;
pub use building::SignText;

const REACH: f32 = 6.0;
const MAX_CREATURES: usize = 24;
const DROP_RADIUS: f32 = 0.16;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum GameMode {
    Survival,
    Creative,
}

pub use mc4d_core::item::Stack as Slot;
use mc4d_core::item;
#[path = "workstations.rs"]
mod workstations;
pub use workstations::StationKind;
use workstations::Station;

/// A physical stack in the world; saved independently of player inventory.
pub struct DroppedItem {
    pub body: Body,
    pub stack: Slot,
    pub pickup_delay: f32,
    pub age: f32,
    pub health:u8,
    pub burn_ticks:u16,
}

fn default_drop_health()->u8{5}
#[derive(Serialize, Deserialize)]
struct SavedDrop {
    pos: [f32; 4],
    vel: [f32; 4],
    stack: Slot,
    pickup_delay: f32,
    age: f32,
    #[serde(default="default_drop_health")]health:u8,
    #[serde(default)]burn_ticks:u16,
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

/// A creature with species-specific collision; Glooms retain hypersphere visuals.
pub struct Creature {
    pub body: Body,
    pub radius: f32,
    pub slime_size:u8,
    pub burn_ticks:u16,
    pub color: [f32; 3],
    hop_timer: f32,
    pub hurt: f32,
    pub health: i32,
    /// Neutral creatures retaliate for a short time after being attacked.
    pub anger: f32,
    pub kind: CreatureKind,
    pub tamed: bool,
    pub sitting: bool,
    pub sheared: bool,
    pub wool_color:u8,
    pub fuse: f32,
    ai_timer: f32,
    attack_timer: f32,
    egg_timer: f32,
    walk_dir: Vec4,
}

impl Creature {
    pub fn center(&self) -> Vec4 {
        self.body.pos + Vec4::Y * (self.body.height*0.5)
    }
}

/// What the crosshair points at.
#[derive(Clone, Copy, Debug)]
pub enum Target {
    Block(Hit),
    Creature(usize),
    Vehicle(usize),
    Painting(usize),
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
    #[serde(default)]
    format_version: u32,
    #[serde(default)]
    building: building::SavedBuilding,
    #[serde(default)] vehicles:vehicles::SavedVehicles,
    #[serde(default)] devices:devices::Devices,
    #[serde(default)] utilities:utilities::SavedUtilities,
    #[serde(default)] gravity:gravity::SavedGravity,
    #[serde(default)] fire:fire::SavedFire,
    #[serde(default)]
    creatures: Vec<mobs::SavedCreature>,
    #[serde(default)]
    arrows: Vec<Arrow>,
    #[serde(default)]
    explosions: Vec<mobs::Explosion>,
    #[serde(default)]
    environment: mc4d_core::environment::Environment,
    #[serde(default = "survival::full_health")]
    health: u8,
    #[serde(default)]
    armor: Vec<Slot>,
    #[serde(default)]
    spawn: Option<[f32; 4]>,
    #[serde(default)] world_spawn:Option<[f32;4]>,
    #[serde(default)]
    death_reason: String,
    seed: i64,
    #[serde(default)]
    generator_version: u32,
    mode: GameMode,
    pos: [f32; 4],
    heading: [f32; 4],
    pitch: f32,
    flying: bool,
    selected: usize,
    /// Empty slots are stored with count 0 (TOML has no null).
    hotbar: Vec<Slot>,
    #[serde(default)]
    inventory: Vec<Slot>,
    #[serde(default)]
    dropped_items: Vec<SavedDrop>,
    #[serde(default)]
    stations: Vec<Station>,
    #[serde(default)]
    crafting: Vec<Slot>,
}

impl Level {
    fn upgrade_items(&mut self) {
        if self.format_version != 0 { return; }
        for stack in self.hotbar.iter_mut().chain(&mut self.inventory).chain(&mut self.armor).chain(&mut self.crafting) {
            *stack = item::migrate_legacy(*stack);
        }
        for drop in &mut self.dropped_items { drop.stack = item::migrate_legacy(drop.stack); }
        for station in &mut self.stations { station.migrate_legacy_items(); }
        self.format_version = 1;
    }
}

pub struct Game {
    particles: particles::Particles,
    pub third_person: bool,
    /// CPU milliseconds: receive, footprints, request, light, eviction.
    pub streaming_timings: [f32;5],
    /// CPU milliseconds: setup/control, movement/interaction, mobs, drops,
    /// stations, fluids, building, farming, redstone, devices/gravity/fire, streaming.
    pub update_timings: [f32;11],
    pub progress: std::sync::Arc<std::sync::Mutex<crate::progress::Progress>>,
    pub achievement_notifications: Vec<u8>,
    building: building::BuildingState,
    farming: farming::FarmingState,
    ecology: ecology::Ecology,
    lightning: lightning::Lightning,
    redstone: redstone::RedstoneState,
    vehicles: vehicles::Vehicles,
    utilities: utilities::Utilities,
    devices: devices::Devices,
    gravity:gravity::GravityState,
    fire:fire::FireState,
    pub sounds: Vec<crate::audio::SoundEvent>,
    pub difficulty: crate::settings::Difficulty,
    peaceful_heal_time: f32,
    pub environment: mc4d_core::environment::Environment,
    pub world: World,
    save: Option<Arc<SaveDir>>,
    pub seed: i64,
    generator_version:u32,
    pub body: Body,
    pub orient: Orientation,
    pub mode: GameMode,
    pub flying: bool,
    pub hotbar: [Option<Slot>; 9],
    pub inventory: [Option<Slot>; 27],
    pub dropped_items: Vec<DroppedItem>,
    pub health: u8,
    pub armor: [Option<Slot>; 4],
    pub air: f32,
    pub death_reason: String,
    invulnerability: f32,
    hazard_cooldown: f32,
    fall_distance: f32,
    damage_remainder: u16,
    stations: Vec<Station>,
    pub active_station: Option<IVec4>,
    pub active_station_secondary:Option<IVec4>,
    pub crafting_grid: [Option<Slot>; 9],
    pub crafting_width: usize,
    pub selected: usize,
    pub creatures: Vec<Creature>,
    pub arrows: Vec<Arrow>,
    explosions: Vec<mobs::Explosion>,
    pub target: Option<Target>,
    pub time: f32,
    pub stats: Stats,
    break_cooldown: f32,
    last_jump_tap: Option<f32>,
    mining_target: Option<(IVec4, u8, usize, item::ItemId, u16, u8)>,
    pub mining_progress: f32,
    hand: hand::HandState,
    dig_sound_cooldown: f32,
    place_cooldown: f32,
    rng: u64,
    spawn: Vec4,
    world_spawn:Vec4,
    /// Chunks the renderer should have, nearest first.
    pub gpu_wanted: Vec<IVec4>,
    load_key: Option<(u32,u32,u32)>,
    load_eye:Vec4,
    load_normals:Vec<Vec4>,
    load_wanted: Vec<IVec4>,
    /// Prefix of `load_wanted` already resident; requests resume after it.
    request_cursor: usize,
    footprints: footprint::FootprintWorker,
    unload_timer: f32,
    autosave_timer: f32,
    /// Short on-screen messages with remaining time.
    pub toasts: Vec<(String, f32)>,
}

impl Game {
    /// Load a world from `dir` (or create it with `seed`). `dir = None` is an unsaved world.
    pub fn new(dir: Option<PathBuf>, seed: impl Into<i64>, mode: GameMode) -> Self {
        Self::construct(dir,seed.into(),mode,true)
    }

    pub fn menu_preview(seed:impl Into<i64>)->Self {Self::construct(None,seed.into(),GameMode::Survival,false)}

    pub fn validate_level(text:&str)->Result<(),String> {
        let level:Level=toml::from_str(text).map_err(|e|e.to_string())?;
        if level.format_version>1 || level.generator_version>1
            || (level.generator_version==0 && !(0..=u32::MAX as i64).contains(&level.seed))
            || !level.pos.iter().chain(level.heading.iter()).all(|v|v.is_finite())
            || !level.pitch.is_finite() || level.heading.iter().map(|v|v*v).sum::<f32>()<0.0001
            || level.spawn.iter().chain(level.world_spawn.iter()).flatten().any(|v|!v.is_finite()) {
            return Err("unsupported or invalid world metadata".into());
        }
        Ok(())
    }

    pub fn try_load(dir:Option<PathBuf>,seed:impl Into<i64>,mode:GameMode)->Result<Self,String> {
        if let Some(root)=dir.as_ref() {
            let path=root.join("level.toml");
            match std::fs::read_to_string(&path) {
                Ok(text)=>Self::validate_level(&text).map_err(|error|format!("{}: {error}",path.display()))?,
                Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{
                    if root.join("chunks").is_dir() && std::fs::read_dir(root.join("chunks")).map_err(|e|e.to_string())?.next().is_some() {
                        return Err("World metadata missing; existing chunk files preserved".into());
                    }
                },
                Err(error)=>return Err(error.to_string()),
            }
            SaveDir::validate_existing(root).map_err(|e|e.to_string())?;
            SaveDir::new(root).map_err(|e|e.to_string())?;
        }
        Ok(Self::construct(dir,seed.into(),mode,true))
    }

    fn construct(dir: Option<PathBuf>, seed: i64, mode: GameMode, load:bool) -> Self {
        let save = dir.and_then(|d| SaveDir::new(d).map_err(|e| log::error!("save dir: {e}")).ok()).map(Arc::new);
        let mut level: Option<Level> = save
            .as_ref()
            .and_then(|s| std::fs::read_to_string(s.root().join("level.toml")).ok())
            .and_then(|t| toml::from_str(&t).map_err(|e| log::warn!("level.toml: {e}")).ok());
        if let Some(level) = &mut level { level.upgrade_items(); }
        let seed = level.as_ref().map_or(seed, |l| l.seed);
        let generator_version=level.as_ref().map_or(1,|l|l.generator_version);
        let mut world = World::versioned(seed,generator_version, save.clone().map(|s| s as Arc<dyn ChunkSource>));
        let spawn = find_spawn(&world);
        let building = building::BuildingState::new(&mut world);
        let farming = farming::FarmingState::new(&mut world);
        let redstone = redstone::RedstoneState::new(&mut world);
        let gravity=gravity::GravityState::new(&mut world);
        let fire=fire::FireState::new(&mut world);
        let vehicles=vehicles::Vehicles::new(&mut world);
        let utilities=utilities::Utilities::new(&mut world);

        let mut game = Self {
            particles: Default::default(),
            third_person: false,
            streaming_timings: [0.0;5],
            update_timings:[0.0;11],
            building,
            farming,
            ecology: Default::default(),
            lightning: Default::default(),
            redstone,
            devices:devices::Devices::default(),
            gravity,
            fire,
            vehicles,
            utilities,
            sounds: Vec::new(),
            progress: Default::default(),
            achievement_notifications: Vec::new(),
            difficulty: crate::settings::Difficulty::Normal,
            peaceful_heal_time: 0.0,
            environment: mc4d_core::environment::Environment::seeded((seed as u64 ^ ((seed as u64)>>32)) as u32),
            world,
            save,
            seed,
            generator_version,
            body: Body::new(spawn),
            orient: Orientation::default(),
            mode,
            flying: false,
            hotbar: [None; 9],
            inventory: [None; 27],
            dropped_items: Vec::new(),
            health: 20,
            armor: [None; 4],
            air: 15.0,
            death_reason: String::new(),
            invulnerability: 0.0,
            hazard_cooldown: 0.0,
            fall_distance: 0.0,
            damage_remainder: 0,
            stations: Vec::new(),
            active_station: None,
            active_station_secondary:None,
            crafting_grid: [None; 9],
            crafting_width: 2,
            selected: 0,
            creatures: Vec::new(),
            arrows: Vec::new(),
            explosions: Vec::new(),
            target: None,
            time: 0.0,
            stats: Stats::default(),
            break_cooldown: 0.0,
            last_jump_tap: None,
            mining_target: None,
            mining_progress: 0.0,
            hand: hand::HandState::default(),
            dig_sound_cooldown: 0.0,
            place_cooldown: 0.0,
            rng: 0x2545_F491_4F6C_DD1D ^ seed as u64,
            spawn,
            world_spawn:spawn,
            gpu_wanted: Vec::new(),
            load_key: None,
            load_eye:Vec4::ZERO,load_normals:Vec::new(),
            load_wanted: Vec::new(),
            request_cursor: 0,
            footprints: footprint::FootprintWorker::new(),
            unload_timer: 0.0,
            autosave_timer: 0.0,
            toasts: Vec::new(),
        };
        if let Some(l) = level {
            game.building.restore(l.building);
            game.vehicles.restore(l.vehicles);
            game.utilities.restore(l.utilities);
            game.devices=l.devices;game.devices.sanitize();
            game.gravity.restore(l.gravity);
            game.fire.restore(l.fire);
            game.stations = l.stations;
            for station in &mut game.stations { station.sanitize(); }
            for (i, stack) in l.crafting.into_iter().take(9).enumerate() { game.crafting_grid[i] = stack.normalized(); }
            if let Some(p)=l.world_spawn.filter(|p|p.iter().all(|v|v.is_finite())){game.world_spawn=Vec4::from_array(p);}
            game.environment = l.environment;
            game.mode = l.mode;
            game.creatures = l.creatures.into_iter().filter_map(Creature::restore).take(MAX_CREATURES).collect();
            game.arrows = l.arrows.into_iter().filter(|a| a.pos.is_finite() && a.velocity.is_finite() && a.age.is_finite() && a.ground_time.is_finite() && a.age < 120.0).take(64).collect();
            game.explosions = l.explosions.into_iter().filter(mobs::Explosion::valid).collect();
            game.body.pos = Vec4::from_array(l.pos);
            game.orient.heading = Quat::from_array(l.heading).normalize();
            game.orient.pitch = l.pitch;
            game.flying = l.flying && l.mode == GameMode::Creative;
            game.selected = l.selected.min(8);
            game.health = l.health.min(20);
            game.death_reason = l.death_reason;
            if let Some(spawn) = l.spawn.filter(|p| p.iter().all(|v| v.is_finite())) {
                game.spawn = Vec4::from_array(spawn);
            }
            for (i, s) in l.armor.into_iter().take(4).enumerate() {
                if mc4d_core::item::armor_slot(s.block) == Some(i) { game.armor[i] = valid_slot(s); }
            }
            for (i, s) in l.hotbar.into_iter().take(9).enumerate() {
                game.hotbar[i] = valid_slot(s);
            }
            for (i, s) in l.inventory.into_iter().take(27).enumerate() {
                game.inventory[i] = valid_slot(s);
            }
            game.dropped_items = l.dropped_items.into_iter().filter_map(|d| {
                let stack = valid_slot(d.stack)?;
                if !d.pos.iter().chain(d.vel.iter()).all(|v| v.is_finite())
                    || !d.age.is_finite() || !d.pickup_delay.is_finite() || d.health==0 { return None; }
                let mut body = Body::with_size(Vec4::from_array(d.pos), DROP_RADIUS, DROP_RADIUS * 2.0);
                body.vel = Vec4::from_array(d.vel);
                Some(DroppedItem { body, stack, pickup_delay: d.pickup_delay.clamp(0.0, 2.0), age: d.age.max(0.0),health:d.health.min(5),burn_ticks:d.burn_ticks.min(300) })
            }).collect();
        } else if mode == GameMode::Creative {
            game.fill_creative_hotbar();
        }
        game.close_station();
        if load {
        game.load_around_now(24.0);
        // Never start inside a tree or a block placed since the last save.
        for _ in 0..64 {
            let world = &game.world;
            if !game.body.is_stuck_states(&|p| world.block_state(p)) {
                break;
            }
            game.body.pos.y += 1.0;
        }
        }
        game
    }

    fn fill_creative_hotbar(&mut self) {
        for (i, b) in CREATIVE_HOTBAR.iter().enumerate() {
            if self.hotbar[i].is_none() {
                self.hotbar[i] = Some(Slot::new(*b, 1));
            }
        }
    }

    pub fn set_mode(&mut self, mode: GameMode) {
        self.last_jump_tap=None;
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

    pub fn release_controls(&mut self) {self.last_jump_tap=None;self.reset_mining();}

    pub fn emit_sound(&mut self, name: &'static str, pos: Vec4, volume: f32, pitch: f32) {
        if self.sounds.len()<64 {self.sounds.push(crate::audio::SoundEvent {name,pos,volume,pitch});}
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toasts.push((msg.into(), 2.5));
    }

    /// Synchronously generate the chunks within `radius` of the player in the slice.
    pub fn load_around_now(&mut self, radius: f32) {
        let b = self.basis();
        for c in chunks_near_slice(self.eye(), b.ana, radius, 2.0) {
            self.world.load_now(c, self.save.as_deref().map(|s| s as &dyn ChunkSource));
            self.materialize_chunk_chests(c);
        }
        self.update_streaming(&Settings::default(), true);
    }

    /// Advance the simulation by `dt` seconds. `input = None` means no player control (menu open).
    pub fn update(&mut self, dt: f32, input: Option<(&InputState, &Bindings)>, settings: &Settings) {
        let mut update_stage=std::time::Instant::now();
        self.update_timings.fill(0.0);
        let dt = dt.min(0.1);
        self.difficulty = settings.difficulty;
        self.time += dt;
        self.hand.update(dt, self.hotbar[self.selected]);
        self.progress.lock().unwrap_or_else(|e|e.into_inner()).accumulate("play_ticks",dt as f64*20.0);
        self.environment.update(dt);
        self.toasts.retain_mut(|t| {
            t.1 -= dt;
            t.1 > 0.0
        });

        if !self.is_alive() {
            self.update_streaming(settings, false);
            return;
        }
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
        self.update_timings[0]=update_stage.elapsed().as_secs_f32()*1000.0;
        update_stage=std::time::Instant::now();
        let previous_pos = self.body.pos;
        let previous_y = previous_pos.y;
        self.move_player(dt, input, bindings);
        self.ecological_step(previous_pos,input.is_down(bindings,Action::Sneak));
        let displacement=self.body.pos-previous_pos;
        let distance=(displacement.x*displacement.x+displacement.z*displacement.z+displacement.w*displacement.w).sqrt();
        self.record_distance(if self.body.in_water {"swim_cm"} else if self.flying {"fly_cm"} else {"walk_cm"},distance);
        self.update_survival(dt, previous_y);
        if self.is_alive() {
            self.update_target();
            self.interact(dt, input, bindings);
        }
        self.update_timings[1]=update_stage.elapsed().as_secs_f32()*1000.0;
        update_stage=std::time::Instant::now();
        self.update_creatures(dt);
        self.update_timings[2]=update_stage.elapsed().as_secs_f32()*1000.0;
        update_stage=std::time::Instant::now();
        self.update_dropped_items(dt);
        self.update_timings[3]=update_stage.elapsed().as_secs_f32()*1000.0;
        update_stage=std::time::Instant::now();
        self.update_stations(dt);
        self.update_timings[4]=update_stage.elapsed().as_secs_f32()*1000.0;
        update_stage=std::time::Instant::now();
        self.world.step_fluids(dt, mc4d_core::fluid::FluidBudget::default());
        self.update_timings[5]=update_stage.elapsed().as_secs_f32()*1000.0;
        update_stage=std::time::Instant::now();
        self.update_building();
        self.update_timings[6]=update_stage.elapsed().as_secs_f32()*1000.0;
        update_stage=std::time::Instant::now();
        self.update_farming(dt);
        self.update_ecology(dt);
        self.update_lightning(dt);
        self.update_timings[7]=update_stage.elapsed().as_secs_f32()*1000.0;
        update_stage=std::time::Instant::now();
        self.update_redstone(dt);
        self.update_timings[8]=update_stage.elapsed().as_secs_f32()*1000.0;
        update_stage=std::time::Instant::now();
        self.update_devices(dt);
        self.update_utilities(dt);
        self.update_vehicles(dt);
        self.update_gravity(dt);
        self.update_fire(dt);
        self.update_particles(dt);
        self.update_timings[9]=update_stage.elapsed().as_secs_f32()*1000.0;
        update_stage=std::time::Instant::now();
        self.update_streaming(settings, false);

        self.update_timings[10]=update_stage.elapsed().as_secs_f32()*1000.0;
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
        let modifier_wheel=if input.is_down(b,Action::SliceModifier) {
            (input.wheel_steps(b,Action::HotbarPrev) as f32-input.wheel_steps(b,Action::HotbarNext) as f32)*step
        } else {0.0};
        let fa = axis(Action::RotateAnaPos, Action::RotateAnaNeg)+modifier_wheel;
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
        if self.mode == GameMode::Creative && input.just_pressed(b, Action::Jump) {
            if self.last_jump_tap.is_some_and(|last| self.time - last <= 0.35) {
                self.flying = !self.flying;
                self.body.vel.y = 0.0;
                self.last_jump_tap = None;
                self.toast(if self.flying { "Flying" } else { "Walking" });
            } else { self.last_jump_tap = Some(self.time); }
        }
        if input.just_pressed(b, Action::Drop) {
            self.drop_selected();
        }
        if input.just_pressed(b, Action::SnapSlice) {
            self.orient.snap();
            self.stats.snaps += 1;
        }
        if input.just_pressed(b, Action::SwitchToSideView) {
            self.switch_to_side_view();
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
        let mut next = input.presses(b, Action::HotbarNext) as i32 - input.presses(b, Action::HotbarPrev) as i32;
        if input.is_down(b,Action::SliceModifier) {
            next-=input.wheel_steps(b,Action::HotbarNext) as i32-input.wheel_steps(b,Action::HotbarPrev) as i32;
        }
        self.selected = (self.selected as i32 + next).rem_euclid(9) as usize;
    }

    fn move_player(&mut self, dt: f32, input: &InputState, b: &Bindings) {
        if self.move_mounted_player(input,b){return;}
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
                    self.record_counter("jumps",1);
                    self.body.vel.y = JUMP_SPEED;
                }
            }
        }
        if !self.flying && let Some(ladder) = self.body.ladder(&|p| self.world.block_state(p)) {
            self.body.vel.y = self.body.vel.y.max(-3.0);
            if sneak { self.body.vel.y = self.body.vel.y.max(0.0); }
            let support = mc4d_core::shapes::wall_support(ladder.metadata).as_vec4();
            if jump || wish.dot(support) > 0.05 { self.body.vel.y = 4.0; }
        }
        let world = &self.world;
        let before = self.body.pos;
        self.body.step_states_options(dt, &|p| world.block_state(p), if self.flying { 0.0 } else { 0.5 }, sneak && !self.flying);
        let delta = self.body.pos - before;
        let along_ana = delta.dot(basis.ana);
        let in_slice = delta - basis.ana * along_ana;
        self.stats.moved_ana += along_ana.abs();
        self.stats.moved_slice += Vec4::new(in_slice.x, 0.0, in_slice.z, in_slice.w).length();
        if self.body.on_ground && self.flying && sneak {
            self.flying = false;
        }

    }

    fn update_target(&mut self) {
        let eye = self.eye();
        let fwd = self.basis().forward;
        let world = &self.world;
        let target_fluid = self.hotbar[self.selected].is_some_and(|s| s.block == item::BUCKET
            || self.mode == GameMode::Creative && matches!(item::placed_block(s.block), Some(block::WATER | block::LAVA)));
        let source_only = self.hotbar[self.selected].is_some_and(|s| s.block == item::BUCKET);
        let hit = raycast_states(eye, fwd, REACH, &|p| world.block_state(p).map(|s| {
            if source_only && block::fluid(s.id).is_some_and(|(_,level)|level!=0) { AIR.into() } else { s }
        }), target_fluid);
        let block_dist = hit.map_or(REACH, |h| h.dist);
        let mut best: Option<(usize, f32)> = None;
        for (i, c) in self.creatures.iter().enumerate() {
            if c.health<=0{continue}
            if let Some(t) = c.target_distance(eye,fwd,REACH)
                && t < block_dist && best.is_none_or(|(_, bt)| t < bt) {
                    best = Some((i, t));
                }
        }
        self.target = match (best, hit) {
            (Some((i, _)), _) => Some(Target::Creature(i)),
            (None, Some(h)) => Some(Target::Block(h)),
            _ => None,
        };
        if let Some(i)=self.aimed_painting(){self.target=Some(Target::Painting(i));}
        if let Some(i)=self.aimed_vehicle(){self.target=Some(Target::Vehicle(i));}
    }

    fn interact(&mut self, dt: f32, input: &InputState, b: &Bindings) {
        if input.just_pressed(b,Action::Place)&&!input.is_down(b,Action::Sneak)
            && matches!(self.target,Some(Target::Block(h)) if self.world.block(h.block).and_then(StationKind::from_block).is_some()) {
            self.open_target_station();return;
        }
        if input.just_pressed(b, Action::Break) { self.start_hand_swing();if self.break_aimed_painting(){return;} }
        if input.just_pressed(b,Action::Place) && (self.interact_vehicle(false) || self.use_vehicle_item()){return;}
        if input.just_pressed(b, Action::Place) && !input.is_down(b, Action::Sneak) && self.use_target_building() { return; }
        if input.just_pressed(b,Action::Place) && matches!(self.target,Some(Target::Block(h)) if self.world.block(h.block)==Some(block::IRON_DOOR)) {return;}
        if input.just_pressed(b,Action::Place) && self.use_utility_item(){return;}
        if input.just_pressed(b, Action::Place) && (self.interact_creature() || self.use_selected_food()) { return; }
        if input.just_pressed(b, Action::Place) && self.use_held_projectile(){return;}
        if input.just_pressed(b, Action::Place) && self.use_selected_tool() { return; }
        self.break_cooldown -= dt;
        self.dig_sound_cooldown-=dt;
        self.place_cooldown -= dt;
        let creative = self.mode == GameMode::Creative;
        if input.is_down(b,Action::Break) && self.break_cooldown<=0.0 && self.interact_vehicle(true){self.break_cooldown=0.4;return;}

        if !input.is_down(b, Action::Break) { self.reset_mining(); }

        if input.just_pressed(b, Action::Break) {
            self.break_cooldown = 0.0;
        }
        if input.is_down(b, Action::Break) && self.break_cooldown <= 0.0 {
            match self.target {
                Some(Target::Creature(i)) => {
                    self.on_creature_hit(i);
                    self.reset_mining();
                    let push = self.basis().forward_h * 6.0 + Vec4::Y * 5.0;
                    let held = self.hotbar[self.selected].map_or(0, |s| s.block);
                    let c = &mut self.creatures[i];
                    c.body.vel += push;
                    c.hurt = 0.3;
                    c.health -= item::attack_damage(held);
                    c.anger = 15.0;
                    let killed=c.health<=0;
                    let monster=matches!(c.kind,CreatureKind::Zombie|CreatureKind::Skeleton|CreatureKind::Spider|CreatureKind::Creeper);
                    if killed {self.record_counter("mob_kills",1);if monster {self.award_achievement(13);}}
                    self.record_counter("damage_dealt",item::attack_damage(held).max(0) as u64);
                    self.break_cooldown = 0.4;
                    self.stats.creatures_hit += 1;
                    let wear = item::tool(held).map_or(0, |t| match t.kind {
                        item::ToolKind::Sword => 1, item::ToolKind::Hoe => 0, _ => 2,
                    });
                    self.wear_selected(wear);
                    if let Some(c)=self.creatures.get(i) { if let Some(name)=crate::audio::creature_sound(c.kind,true,false) {self.emit_sound(name,c.center(),1.0,1.0);} }
                }
                Some(Target::Block(h)) => {
                    self.start_hand_swing();
                    self.mine_block(h, dt);
                }
                Some(Target::Vehicle(_)) | Some(Target::Painting(_)) | None => self.reset_mining(),
            }
        }
        self.collect_dead_creatures();

        if input.just_pressed(b, Action::Place) {
            self.place_cooldown = 0.0;
        }
        if input.is_down(b, Action::Place) && self.place_cooldown <= 0.0
            && let (Some(Target::Block(h)), Some(slot)) = (self.target, self.hotbar[self.selected]) {
                if self.place_building(h, slot) {
                        self.start_hand_swing();
                        if !creative {
                            self.take_selected();
                        }
                        self.place_cooldown = 0.22;
                        self.stats.placed += 1;
                        self.progress.lock().unwrap_or_else(|e|e.into_inner()).item(slot.block).used+=1;
                        if let Some(id)=item::placed_block(slot.block) { self.emit_sound(crate::audio::step_sound(id),h.block.as_vec4(),0.8,0.8); }
                    }
            }

        if input.just_pressed(b, Action::PickBlock)
            && let Some(Target::Block(h)) = self.target
                && let Some(state) = self.world.block_state(h.block) {
                    let picked = item::from_block_state(state);
                    if let Some(i) = self.hotbar.iter().position(|s| s.is_some_and(|s| s.same_kind(picked))) {
                        self.selected = i;
                    } else if creative {
                        self.hotbar[self.selected] = Some(picked);
                    }
                }
    }

    /// Unified slot numbers: hotbar 0..9, storage 9..36.
    pub fn inventory_slot(&self, index: usize) -> Option<Slot> {
        match index {
            0..9 => self.hotbar[index],
            9..36 => self.inventory[index - 9],
            36..40 => self.armor[index - 36],
            _ => None,
        }
    }

    fn slot_mut(&mut self, index: usize) -> Option<&mut Option<Slot>> {
        match index {
            0..9 => self.hotbar.get_mut(index),
            9..36 => self.inventory.get_mut(index - 9),
            36..40 => self.armor.get_mut(index - 36),
            _ => None,
        }
    }

    /// Left click picks up, merges or swaps; right click halves a stack or places one.
    pub fn click_inventory_slot(&mut self, index: usize, cursor: &mut Option<Slot>, split: bool) {
        if (36..40).contains(&index) { self.click_armor_slot(index - 36, cursor); return; }
        let Some(slot) = self.slot_mut(index) else { return };
        item::click_slot(slot, cursor, split);
    }

    /// Shift click transfers into the other inventory section, merging stacks first.
    pub fn quick_transfer(&mut self, index: usize) {
        let Some(stack) = self.inventory_slot(index) else { return };
        if index < 36 && let Some(armor) = item::armor_slot(stack.block) && self.armor[armor].is_none() {
            self.armor[armor] = Some(stack);
            *self.slot_mut(index).unwrap() = None;
            return;
        }
        let range = if index < 9 { 9..36 } else if index < 36 { 0..9 } else { 0..36 };
        let left = self.insert_stack_range(stack, range);
        if let Some(slot) = self.slot_mut(index) { *slot = left; }
    }

    fn insert_stack_range(&mut self, mut stack: Slot, range: std::ops::Range<usize>) -> Option<Slot> {
        for merge in [true, false] {
            for i in range.clone() {
                let slot = self.slot_mut(i).expect("inventory range");
                let room = match slot {
                    Some(s) if merge && s.same_kind(stack) => item::stack_limit(s.block).saturating_sub(s.count),
                    None if !merge => item::stack_limit(stack.block),
                    _ => 0,
                };
                let count = room.min(stack.count);
                if count > 0 {
                    let old_count = slot.map_or(0, |s| s.count);
                    *slot = Some(Slot { count: old_count + count, ..stack });
                    stack.count -= count;
                    if stack.count == 0 { return None; }
                }
            }
        }
        Some(stack)
    }

    /// Closing the UI must return its cursor stack before saving or losing focus.
    pub fn return_cursor(&mut self, cursor: &mut Option<Slot>) {
        self.close_station();
        if !self.is_alive() {
            if let Some(stack) = cursor.take() { self.spawn_drop(stack, self.eye(), Vec4::ZERO, 1.0); }
            return;
        }
        if let Some(stack) = cursor.take()
            && let Some(left) = self.insert_stack_range(stack, 0..36) {
                self.spawn_drop(left, self.eye(), Vec4::ZERO, 0.5);
            }
    }

    /// Clicking outside a container drops the carried stack (right click: one).
    pub fn drop_cursor_stack(&mut self, cursor: &mut Option<Slot>, one: bool) {
        let Some(mut held) = *cursor else { return };
        let mut dropped = held;
        dropped.count = if one { 1 } else { held.count };
        self.record_counter("drops",1);
        self.spawn_drop(dropped, self.eye(), self.basis().forward * 4.0 + Vec4::Y, 1.0);
        held.count -= dropped.count;
        *cursor = (held.count > 0).then_some(held);
    }

    fn spawn_block_loot(&mut self, position: IVec4, state: block::BlockState, cause: mc4d_core::loot::DropCause) {
        let stacks=mc4d_core::loot::drops(state,cause,|bound|(self.rand()*bound as f32)as u32);
        for stack in stacks {
            let offset=Vec4::new(self.rand(),self.rand(),self.rand(),self.rand())*0.7+Vec4::splat(0.15);
            let velocity=Vec4::new((self.rand()-0.5)*4.0,4.0,(self.rand()-0.5)*4.0,(self.rand()-0.5)*4.0);
            self.spawn_drop(stack,position.as_vec4()+offset,velocity,0.5);
        }
    }

    fn spawn_drop(&mut self, stack: Slot, pos: Vec4, vel: Vec4, pickup_delay: f32) {
        let mut body = Body::with_size(pos, DROP_RADIUS, DROP_RADIUS * 2.0);
        body.vel = vel;
        self.dropped_items.push(DroppedItem { body, stack, pickup_delay, age: 0.0,health:5,burn_ticks:0 });
    }

    pub fn drop_selected(&mut self) {
        let Some(stack) = self.hotbar[self.selected] else { return };
        // Start at the eye, inside the player's free body volume, so walls cannot
        // swallow an item spawned on the far side of their collision surface.
        self.record_counter("drops",1);
        self.spawn_drop(Slot { count: 1, ..stack }, self.eye(), self.basis().forward * 4.0 + Vec4::Y, 1.0);
        if self.mode == GameMode::Survival { self.take_selected(); }
    }

    fn update_dropped_items(&mut self, dt: f32) {
        let mut drops = std::mem::take(&mut self.dropped_items);
        for drop in &mut drops {
            if !self.world.is_loaded(chunk_of(drop.body.pos.floor().as_ivec4())) { continue; }
            drop.age += dt;
            drop.pickup_delay = (drop.pickup_delay - dt).max(0.0);
            drop.body.vel.y = (drop.body.vel.y - GRAVITY * dt).max(-30.0);
            let friction = if drop.body.on_ground { 8.0 } else { 0.5 };
            let damp = (-friction * dt).exp();
            drop.body.vel.x *= damp;
            drop.body.vel.z *= damp;
            drop.body.vel.w *= damp;
            drop.body.step_states(dt, &|p| self.world.block_state(p));
            let delta = drop.body.pos - (self.body.pos + Vec4::Y * 0.8);
            if drop.pickup_delay == 0.0 && delta.length_squared() < 1.8 * 1.8 {
                let previous=drop.stack.count;
                drop.stack.count = self.insert_stack_range(drop.stack, 0..36).map_or(0, |s| s.count);
                if drop.stack.count<previous {self.record_pickup(Slot { count: previous-drop.stack.count, ..drop.stack });self.emit_sound("random.pop",drop.body.pos,0.25,1.2);}
            }
        }
        drops.retain(|d| d.stack.count > 0 && d.age < 300.0 && d.body.pos.y > -32.0);
        self.dropped_items = drops;
    }

    fn take_selected(&mut self) {
        if let Some(s) = &mut self.hotbar[self.selected] {
            s.count -= 1;
            if s.count == 0 {
                self.hotbar[self.selected] = None;
            }
        }
    }

    fn try_spawn_creature(&mut self) {
        let basis = self.basis();
        let angle = self.rand() * std::f32::consts::TAU;
        let dist = 10.0 + self.rand() * 20.0;
        let off = if self.rand() < 0.5 { (self.rand() - 0.5) * 1.5 } else { (self.rand() - 0.5) * 16.0 };
        let p = self.body.pos + (basis.forward_h * angle.cos() + basis.right * angle.sin()) * dist + basis.ana * off;
        let (x, z, w) = (p.x.floor() as i32, p.z.floor() as i32, p.w.floor() as i32);
        let h = self.world.terrain.height(x, z, w);
        let roll = (self.rand() * 8.0) as usize;
        let kind = if h < SEA_LEVEL - 2 { CreatureKind::Squid }
        else if self.environment.is_night() && self.difficulty!=crate::settings::Difficulty::Peaceful {
            [CreatureKind::Zombie, CreatureKind::Skeleton, CreatureKind::Spider, CreatureKind::Creeper,
                CreatureKind::Slime, CreatureKind::Gloom, CreatureKind::Wolf, CreatureKind::Pig][roll.min(7)]
        } else {
            [CreatureKind::Pig, CreatureKind::Cow, CreatureKind::Sheep, CreatureKind::Chicken,
                CreatureKind::Wolf, CreatureKind::Gloom, CreatureKind::Pig, CreatureKind::Sheep][roll.min(7)]
        };
        let y = if kind == CreatureKind::Squid { SEA_LEVEL - 2 } else { h + 1 };
        let cell = IVec4::new(x, y, z, w);
        let free = self.world.block(cell).is_some_and(|id| if kind == CreatureKind::Squid { block::is_water(id) } else { id == AIR });
        if !free { return; }
        let pos = Vec4::new(x as f32 + 0.5, y as f32, z as f32 + 0.5, w as f32 + 0.5);
        let previous=self.creatures.len();
        self.spawn_creature(kind,pos);
        if self.creatures.len()>previous && self.creatures.last().is_some_and(|c|c.body.is_stuck_states(&|p|self.world.block_state(p))) {
            self.creatures.pop();
        }
        if self.creatures.len()>previous&&kind==CreatureKind::Sheep {
            let common=(self.rand()*100.0)as u32;let rare=(self.rand()*500.0)as u32;
            self.creatures.last_mut().unwrap().wool_color=mc4d_core::entities::natural_sheep_color(common,rare);
        }
    }

    /// Request generation of chunks near the slice and work out what the GPU should hold.
    pub fn update_streaming(&mut self, settings: &Settings, force: bool) {
        let mut stage=std::time::Instant::now();
        for c in self.world.receive() {self.materialize_chunk_chests(c);}
        self.streaming_timings[0]=stage.elapsed().as_secs_f32()*1000.0;
        stage=std::time::Instant::now();
        let eye = self.camera_eye();
        let basis = self.basis();
        let count = settings.active_view_count();
        let normals: Vec<_> = mc4d_core::views::slice_bases(basis, count).iter().map(|b| b.ana).collect();
        let radius = settings.render_distance as f32 * CHUNK as f32;
        // The footprint has a two-block guard. Reserve half a block for
        // view-bobbing; cached geometric motion may consume at most1.5 blocks.
        // Quantized position/angle bins do not provide this containment bound.
        let ghost=if settings.ghosts{settings.ghost_offset}else{0.0};
        let key=(settings.render_distance,count,if settings.ghosts{ghost.to_bits()}else{u32::MAX});
        let margin = if settings.ghosts { settings.ghost_offset + 2.0 } else { 2.0 };
        // Background results are accepted only while they still contain the
        // current view within the same motion bound as a synchronous footprint.
        if let Some(result)=self.footprints.receive() {
            let fresh=slice_cache_motion_bound(result.job.eye,&result.job.normals,eye,&normals,radius+ghost);
            let current=slice_cache_motion_bound(self.load_eye,&self.load_normals,eye,&normals,radius+ghost);
            if result.job.key==key && self.load_key==Some(key) && fresh<=1.5 && fresh<current {
                self.load_key=Some(key);self.load_eye=result.job.eye;self.load_normals=result.job.normals;
                self.gpu_wanted=result.gpu_wanted;self.load_wanted=result.load_wanted;self.request_cursor=0;
            }
        }
        let motion=slice_cache_motion_bound(self.load_eye,&self.load_normals,eye,&normals,radius+ghost);
        if force || self.load_key!=Some(key) || motion>1.5 {
            self.load_key=Some(key);self.load_eye=eye;self.load_normals=normals.clone();
            let job=footprint::Job {eye,normals:normals.clone(),radius,margin,key};
            let result=job.compute();
            self.gpu_wanted=result.gpu_wanted;self.load_wanted=result.load_wanted;self.request_cursor=0;
        } else if motion>0.75 {
            // Refresh ahead of need on a worker: rotation otherwise recomputes
            // multi-view footprints on the frame thread every few frames.
            self.footprints.submit(footprint::Job {eye,normals:normals.clone(),radius,margin,key});
        }
        // Populate what the player can currently see before spending worker time
        // on the wider movement/rotation prefetch margin. Keep queued work short
        // so a change of slice direction is reflected promptly.
        self.streaming_timings[1]=stage.elapsed().as_secs_f32()*1000.0;
        stage=std::time::Instant::now();
        self.request_cursor=self.world.request_from(&self.load_wanted, self.request_cursor, 32);
        self.streaming_timings[2]=stage.elapsed().as_secs_f32()*1000.0;
        stage=std::time::Instant::now();
        self.world.update_lighting(&self.load_wanted, 128);
        self.streaming_timings[3]=stage.elapsed().as_secs_f32()*1000.0;
        stage=std::time::Instant::now();

        self.unload_timer += 1.0 / 60.0;
        if self.unload_timer > 3.0 {
            self.unload_timer = 0.0;
            self.request_cursor = 0;
            let keep = SliceNeighborhood::new(eye, &normals, radius + 32.0, 48.0);
            let save = self.save.clone();
            self.world.unload(
                |c| keep.contains(c),
                |c, chunk| {
                    let Some(s) = &save else { return false };
                    match s.save_chunk(c, chunk) {
                        Ok(()) => true,
                        Err(e) => { log::error!("saving chunk {c}: {e}"); false }
                    }
                },
            );
        }
        self.streaming_timings[4]=stage.elapsed().as_secs_f32()*1000.0;
    }

    pub fn entity_draws(&self) -> Vec<EntityDraw> {
        self.creatures
            .iter()
            .map(|c| EntityDraw {
                center: if c.kind == CreatureKind::Gloom { c.center() } else { c.body.pos },
                radius: if c.kind==CreatureKind::Gloom {c.radius}else{c.kind.render_scale(c.slime_size)},
                color: if c.hurt > 0.0 { [1.0, 0.3, 0.3] } else { c.color },
                kind: c.kind as u32,
                forward: if c.body.vel.length_squared() > 0.1 { c.body.vel } else { c.walk_dir },
                item: AIR.into(), item_variant: 0,
                state: (c.sheared as u32) | ((c.tamed as u32) << 1) | ((c.sitting as u32) << 2)
                    | (((c.fuse > 0.0) as u32) << 3) | (((c.anger > 0.0) as u32) << 4) | (((c.hurt > 0.0) as u32) << 5) | (u32::from(c.wool_color&15)<<8),
            })
            .chain(self.arrows.iter().map(|a| EntityDraw {
                center: a.pos, radius: 0.08, color: [0.6, 0.45, 0.25],
                kind: 12, forward: a.velocity, item: item::ARROW, item_variant: 0, state: 0,
            }))
            .chain(self.dropped_items.iter().map(|d| EntityDraw {
                center: d.body.pos + Vec4::Y * DROP_RADIUS,
                radius: DROP_RADIUS,
                color: item::info(d.stack.block).color,
                kind: 13, forward: Vec4::Z, item: d.stack.block, item_variant: d.stack.variant, state: 0,
            }))
            .chain(self.device_draws())
            .chain(self.gravity_draws())
            .chain(self.lightning_draws())
            .chain(self.vehicle_draws())
            .chain(self.painting_draws())
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
        if let Err(error)=self.save_checked() {log::error!("saving game: {error}");self.toast("Game could not be saved");}
    }

    pub fn save_checked(&mut self)->std::io::Result<()> {
        let Some(save) = &self.save else { return Ok(()) };
        for (&c, chunk) in &mut self.world.chunks {
            if chunk.modified {
                save.save_chunk(c,chunk)?;
                chunk.modified=false;
            }
        }
        let level = Level {
            format_version: 1,
            building: self.building.snapshot(),
            vehicles:self.vehicles.saved.clone(),
            devices:self.devices.clone(),
            utilities:self.utilities.snapshot(),
            gravity:self.gravity.snapshot(),
            fire:self.fire.snapshot(),
            creatures: self.creatures.iter().map(Creature::save).collect(),
            arrows: self.arrows.clone(),
            explosions: self.explosions.clone(),
            environment: self.environment.clone(),
            seed: self.seed,
            generator_version:self.generator_version,
            mode: self.mode,
            pos: self.body.pos.to_array(),
            heading: self.orient.heading.to_array(),
            pitch: self.orient.pitch,
            flying: self.flying,
            selected: self.selected,
            health: self.health,
            armor: self.armor.iter().map(|s| s.unwrap_or(Slot::EMPTY)).collect(),
            spawn: Some(self.spawn.to_array()),
            world_spawn:Some(self.world_spawn.to_array()),
            death_reason: self.death_reason.clone(),
            hotbar: self.hotbar.iter().map(|s| s.unwrap_or(Slot::EMPTY)).collect(),
            inventory: self.inventory.iter().map(|s| s.unwrap_or(Slot::EMPTY)).collect(),
            stations: self.stations.clone(),
            crafting: self.crafting_grid.iter().map(|s| s.unwrap_or(Slot::EMPTY)).collect(),
            dropped_items: self.dropped_items.iter().map(|d| SavedDrop {
                pos: d.body.pos.to_array(), vel: d.body.vel.to_array(), stack: d.stack,
                pickup_delay: d.pickup_delay, age: d.age,health:d.health,burn_ticks:d.burn_ticks,
            }).collect(),
        };
        let text=toml::to_string_pretty(&level).map_err(std::io::Error::other)?;
        let temporary=save.root().join("level.toml.tmp");
        std::fs::write(&temporary,text)?;
        std::fs::rename(temporary,save.root().join("level.toml"))?;
        Ok(())
    }
}

fn valid_slot(s: Slot) -> Option<Slot> {
    s.normalized()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Binding;
    use winit::keyboard::KeyCode;

    fn seed_test_dir(name:&str)->PathBuf {
        let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/seed-save-tests").join(format!("{name}-{}-{stamp}",std::process::id()))
    }

    #[test]
    fn signed_extreme_seeds_survive_real_disk_save_and_reload(){
        for seed in [i64::MIN,-1,i64::MAX,1337|(1i64<<40)] {
            let dir=seed_test_dir("extreme");
            let mut game=Game::try_load(Some(dir.clone()),seed,GameMode::Creative).unwrap();
            assert_eq!((game.seed,game.generator_version),(seed,1));
            game.save_checked().unwrap();drop(game);
            // The stored seed wins over the launcher's argument.
            let restored=Game::try_load(Some(dir.clone()),0,GameMode::Creative).unwrap();
            assert_eq!((restored.seed,restored.generator_version),(seed,1));
            assert_eq!(restored.world.terrain.seed(),seed as u64);
            drop(restored);std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn unversioned_saves_load_as_generator_zero_and_unknown_versions_are_refused(){
        let base="mode=\"Creative\"\npos=[0.5,70.0,0.5,0.5]\nheading=[0.0,0.0,1.0,0.0]\npitch=0.0\nflying=true\nselected=0\nhotbar=[]\n";
        let dir=seed_test_dir("legacy");std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("level.toml"),format!("seed=4000000000\n{base}")).unwrap();
        let game=Game::try_load(Some(dir.clone()),1,GameMode::Creative).unwrap();
        assert_eq!((game.seed,game.generator_version,game.world.terrain.seed()),(4_000_000_000,0,4_000_000_000));
        drop(game);std::fs::remove_dir_all(dir).unwrap();
        assert!(Game::validate_level(&format!("seed=7\ngenerator_version=1\n{base}")).is_ok());
        assert!(Game::validate_level(&format!("seed=7\ngenerator_version=2\n{base}")).is_err());
        assert!(Game::validate_level(&format!("seed=-7\n{base}")).is_err(),"version 0 seeds were unsigned 32-bit");
        assert!(Game::validate_level(&format!("seed=-7\ngenerator_version=1\n{base}")).is_ok());
    }

    #[test]
    fn corrupt_saved_chunk_refuses_load_and_preserves_the_file(){
        let dir=seed_test_dir("corrupt");
        let mut game=Game::try_load(Some(dir.clone()),7,GameMode::Creative).unwrap();
        game.save_checked().unwrap();drop(game);
        std::fs::create_dir_all(dir.join("chunks")).unwrap();
        let damaged=dir.join("chunks").join("0_3_0_0.bin");
        std::fs::write(&damaged,b"not a chunk").unwrap();
        let error=Game::try_load(Some(dir.clone()),7,GameMode::Creative).err().expect("damaged chunk must refuse load");
        assert!(error.contains("Damaged saved chunk"),"{error}");
        assert_eq!(std::fs::read(&damaged).unwrap(),b"not a chunk");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_item_upgrade_covers_every_nested_stack_and_runs_once() {
        let source = r#"
seed=7
mode="Survival"
pos=[0.0,64.0,0.0,0.0]
heading=[0.0,0.0,0.0,1.0]
pitch=0.0
flying=false
selected=0
hotbar=[{block=64,count=2},{block=5,count=3}]
inventory=[{block=66,count=4}]
armor=[{block=104,count=1,damage=5}]
crafting=[{block=84,count=3}]
[[dropped_items]]
pos=[0.0,64.0,0.0,0.0]
vel=[0.0,0.0,0.0,0.0]
stack={block=116,count=1,damage=7}
pickup_delay=0.5
age=3.0
[[stations]]
pos=[1,63,1,1]
kind="Chest"
slots=[{block=84,count=8}]
"#;
        let mut level: Level = toml::from_str(source).unwrap();
        level.upgrade_items();
        assert_eq!(level.format_version, 1);
        assert_eq!(level.hotbar[0].block, item::STICK);
        assert_eq!(level.hotbar[1], Slot::new(block::LOG, 3));
        assert_eq!(level.inventory[0], Slot::with_variant(item::COAL, 4, 1));
        assert_eq!(level.armor[0].block, item::IRON_HELMET);
        assert_eq!(level.armor[0].damage, 5);
        assert_eq!(level.crafting[0], Slot::with_variant(item::DYE, 3, 15));
        assert_eq!(level.dropped_items[0].stack.block, item::WOOD_PICKAXE);
        assert_eq!(level.dropped_items[0].stack.damage, 7);
        let encoded: toml::Value = toml::from_str(&toml::to_string(&level).unwrap()).unwrap();
        assert_eq!(encoded["stations"][0]["slots"][0]["block"].as_integer(), Some(item::DYE.into()));
        assert_eq!(encoded["stations"][0]["slots"][0]["variant"].as_integer(), Some(15));
        level.hotbar[0] = Slot::new(64u16, 1); // now a genuine new block identity
        level.upgrade_items();
        assert_eq!(level.hotbar[0].block, 64);
    }

    fn settle(game: &mut Game, frames: usize, input: Option<(&InputState, &Bindings)>) {
        let s = Settings::default();
        for _ in 0..frames {
            game.update(1.0 / 60.0, input, &s);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn double_jump_toggles_creative_flight_but_single_or_held_jump_does_not() {
        let mut game = Game::new(None, 7, GameMode::Creative);
        let bindings = Bindings::default();
        let mut input = InputState::default();
        let jump = Binding::Key(KeyCode::Space);
        input.press(jump);
        game.handle_actions(&input, &bindings, &Settings::default());
        assert!(!game.flying);
        input.end_frame();
        game.time = 0.1;
        game.handle_actions(&input, &bindings, &Settings::default());
        assert!(!game.flying, "holding jump must not count as another tap");
        input.release(jump); input.press(jump);
        game.handle_actions(&input, &bindings, &Settings::default());
        assert!(game.flying);
        input.end_frame(); input.release(jump);
        game.time = 1.0; input.press(jump);
        game.handle_actions(&input, &bindings, &Settings::default());
        input.end_frame(); input.release(jump);
        game.time = 1.1; input.press(jump);
        game.handle_actions(&input, &bindings, &Settings::default());
        assert!(!game.flying);
        game.mode = GameMode::Survival;
        for time in [2.0, 2.1] {
            input.end_frame(); input.release(jump); input.press(jump); game.time = time;
            game.handle_actions(&input, &bindings, &Settings::default());
        }
        assert!(!game.flying);
    }

    #[test]
    fn inventory_click_and_transfer_conserve_items() {
        let mut g = Game::new(None, 7, GameMode::Survival);
        g.hotbar[0] = Some(Slot { variant: 0, damage: 0, block: (block::STONE).into(), count: 63 });
        g.inventory[0] = Some(Slot { variant: 0, damage: 0, block: (block::STONE).into(), count: 10 });
        g.quick_transfer(9);
        assert_eq!(g.hotbar[0].unwrap().count, 64);
        assert_eq!(g.hotbar[1].unwrap().count, 9);
        assert_eq!(g.inventory[0], None);
        let mut cursor = None;
        g.click_inventory_slot(1, &mut cursor, true);
        assert_eq!(cursor.unwrap().count, 5);
        assert_eq!(g.hotbar[1].unwrap().count, 4);
        g.click_inventory_slot(9, &mut cursor, true);
        assert_eq!(g.inventory[0].unwrap().count, 1);
        assert_eq!(cursor.unwrap().count, 4);
        g.return_cursor(&mut cursor);
        assert_eq!(cursor, None);
        assert_eq!(g.hotbar.iter().chain(g.inventory.iter()).flatten().map(|s| s.count).sum::<u32>(), 73);
        g.hotbar.fill(Some(Slot { variant: 0, damage: 0, block: (block::STONE).into(), count: 64 }));
        g.inventory.fill(Some(Slot { variant: 0, damage: 0, block: (block::STONE).into(), count: 64 }));
        let mut cursor = Some(Slot { variant: 0, damage: 0, block: (block::DIRT).into(), count: 12 });
        g.return_cursor(&mut cursor);
        assert_eq!(g.dropped_items[0].stack.count, 12, "closing a full inventory must not destroy the cursor");
    }

    #[test]
    fn drop_has_delay_and_pickup_preserves_full_inventory() {
        let mut g = Game::new(None, 7, GameMode::Survival);
        g.hotbar[0] = Some(Slot { variant: 0, damage: 0, block: (block::DIRT).into(), count: 2 });
        g.drop_selected();
        assert_eq!(g.hotbar[0].unwrap().count, 1);
        assert_eq!(g.dropped_items.len(), 1);
        g.dropped_items[0].body.pos = g.body.pos + Vec4::Y * 0.8;
        g.dropped_items[0].body.vel = Vec4::ZERO;
        g.update_dropped_items(0.0);
        assert_eq!(g.dropped_items.len(), 1, "pickup delay prevents instant recollection");
        g.dropped_items[0].pickup_delay = 0.0;
        g.hotbar.fill(Some(Slot { variant: 0, damage: 0, block: (block::STONE).into(), count: 64 }));
        g.inventory.fill(Some(Slot { variant: 0, damage: 0, block: (block::STONE).into(), count: 64 }));
        g.update_dropped_items(0.0);
        assert_eq!(g.dropped_items.len(), 1, "full inventory leaves item in world");
        g.inventory[0] = None;
        g.update_dropped_items(0.0);
        assert!(g.dropped_items.is_empty());
        assert_eq!(g.inventory[0], Some(Slot { variant: 0, damage: 0, block: (block::DIRT).into(), count: 1 }));
        g.set_mode(GameMode::Creative);
        let before = g.hotbar[0];
        g.drop_selected();
        assert_eq!(g.hotbar[0], before, "creative drops do not consume inventory");
    }

    #[test]
    fn creative_fluid_selection_targets_and_removes_source() {
        let mut g = Game::new(None, 7, GameMode::Creative);
        let cell = (g.eye() + g.basis().forward * 2.0).floor().as_ivec4();
        g.world.set_block(cell, block::WATER);
        g.hotbar[0] = Some(Slot { variant: 0, damage: 0, block: (block::WATER).into(), count: 1 });
        g.selected = 0;
        g.update_target();
        assert!(matches!(g.target, Some(Target::Block(h)) if h.block == cell));
        let mut input = InputState::default();
        input.press(Binding::Mouse(crate::input::MouseBtn::Left));
        g.interact(0.0, &input, &Bindings::default());
        assert_eq!(g.world.block(cell), Some(AIR));
    }

    #[test]
    fn legacy_level_defaults_inventory_and_drops() {
        let old = r#"
seed = 7
mode = "Survival"
pos = [0.0, 64.0, 0.0, 0.0]
heading = [0.0, 0.0, 0.0, 1.0]
pitch = 0.0
flying = false
selected = 0
hotbar = [{ block = 1, count = 3 }]
"#;
        let mut level: Level = toml::from_str(old).unwrap();
        assert!(level.inventory.is_empty());
        assert!(level.dropped_items.is_empty());
        level.inventory.push(Slot { variant: 0, damage: 0, block: (block::DIRT).into(), count: 19 });
        level.dropped_items.push(SavedDrop { pos: [1.0; 4], vel: [0.0; 4],
            stack: Slot { variant: 0, damage: 0, block: (block::PLANKS).into(), count: 5 }, pickup_delay: 0.5, age: 2.0,health:5,burn_ticks:0 });
        let restored: Level = toml::from_str(&toml::to_string(&level).unwrap()).unwrap();
        assert_eq!(restored.inventory, level.inventory);
        assert_eq!(restored.dropped_items[0].stack.count, 5);
    }

    #[test]
    fn player_lands_and_walks_along_w() {
        let mut g = Game::new(None, 7, GameMode::Survival);
        settle(&mut g, 120, None);
        assert!(g.body.on_ground, "player should be standing at {:?}", g.body.pos);
        let w0 = g.body.pos.w;
        let b = Bindings::default();
        let mut input = InputState::default();
        input.press(Binding::Key(KeyCode::KeyC));
        settle(&mut g, 30, Some((&input, &b)));
        assert!(g.body.pos.w > w0 + 0.3 || g.body.vel.w.abs() < 1e-3, "C should move along +ana (w)");
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
        for _ in 0..180 {
            g.update(1.0 / 60.0, Some((&input, &b)), &Settings::default());
            input.end_frame();
            if g.world.block(hit.block) == Some(AIR) { break; }
        }
        assert_eq!(g.world.block(hit.block), Some(AIR));
        assert!(g.hotbar.iter().all(Option::is_none), "mining should spawn loot rather than teleport it into inventory");
        assert_eq!(g.dropped_items.len(),1);
        let loot=g.dropped_items[0].stack;
        assert!(g.dropped_items[0].pickup_delay>0.0);
        g.body.pos=g.dropped_items[0].body.pos-Vec4::Y*0.8;
        g.update_dropped_items(0.0);
        assert_eq!(g.dropped_items.len(),1,"pickup delay is enforced even when touching the item");
        for _ in 0..6 {g.update_dropped_items(0.1);}
        if let Some(drop)=g.dropped_items.first(){g.body.pos=drop.body.pos-Vec4::Y*0.8;g.update_dropped_items(0.0);}
        assert!(g.dropped_items.is_empty());
        assert_eq!(g.hotbar[0],Some(loot));
        g.body.pos=hit.block.as_vec4()+Vec4::X*3.0;
        g.target=Some(Target::Block(Hit{block:hit.block-IVec4::Y,normal:IVec4::Y,dist:3.0}));
        let mut place=InputState::default();place.press(Binding::Mouse(crate::input::MouseBtn::Right));
        g.interact(0.016,&place,&b);
        assert_eq!(g.world.block(hit.block),item::placed_block(loot.block));
        assert!(g.hotbar[0].is_none(),"placing the collected block consumes it");
    }
}

#[path = "game/camera.rs"]
mod camera;

mod particles;
#[cfg(test)] mod progression_tests;
#[cfg(test)] mod item_use_tests;
