//! Twenty-Hz boats and rail vehicles. Original constants come from fo/xb.
use super::*;
use mc4d_core::{block::BlockState, rails};
use std::{
    collections::{HashSet, VecDeque},
    sync::mpsc::Receiver,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VehicleKind {
    Boat,
    Minecart,
    ChestCart,
    FurnaceCart,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Vehicle {
    pub kind: VehicleKind,
    pub position: Vec4,
    pub velocity: Vec4,
    pub forward: Vec4,
    pub cargo: Vec<Slot>,
    pub fuel: u32,
    pub push: Vec4,
    pub damage: u16,
    #[serde(skip)]
    previous_position: Option<Vec4>,
    #[serde(skip)]
    previous_forward: Option<Vec4>,
}
impl Vehicle {
    fn draw_forward(&self, fraction: f32) -> Vec4 {
        self.previous_forward
            .unwrap_or(self.forward)
            .lerp(self.forward, fraction.clamp(0.0, 1.0))
            .normalize_or_zero()
    }
    fn draw_position(&self, fraction: f32) -> Vec4 {
        self.previous_position
            .unwrap_or(self.position)
            .lerp(self.position, fraction.clamp(0.0, 1.0))
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct SavedVehicles {
    pub entities: Vec<Vehicle>,
    pub riding: Option<usize>,
    pub rail_origin: Option<Vec4>,
    #[serde(default)]
    pub detectors: Vec<(IVec4, u8)>,
}
pub(super) struct Vehicles {
    pub saved: SavedVehicles,
    pub active_cargo: Option<usize>,
    changes: Receiver<mc4d_core::world::BlockChange>,
    chunk_events: Receiver<mc4d_core::world::ChunkLifecycle>,
    scans:VecDeque<(IVec4,u64,usize)>,
    queue: VecDeque<IVec4>,
    queued: HashSet<IVec4>,
    fraction: f32,
    rider_wish: Vec4,
}
fn vehicle_pair_impulse(a:&mut Vehicle,b:&mut Vehicle){
    let raw=b.position-a.position;let delta=Vec4::new(raw.x,0.0,raw.z,raw.w);let distance=delta.length();if distance<0.01{return;}
    if a.kind==VehicleKind::Boat {let magnitude=delta.abs().max_element().sqrt();let push=delta/magnitude*(1.0/magnitude).min(1.0)*0.05;a.velocity-=push;b.velocity+=push;return;}
    let push=delta/distance*(1.0/distance).min(1.0)*0.05;let va=Vec4::new(a.velocity.x,0.0,a.velocity.z,a.velocity.w);let vb=Vec4::new(b.velocity.x,0.0,b.velocity.z,b.velocity.w);
    let(na,nb)=if a.kind==VehicleKind::FurnaceCart&&b.kind!=VehicleKind::FurnaceCart{(va*0.7,vb*0.2+va+push)}else if b.kind==VehicleKind::FurnaceCart&&a.kind!=VehicleKind::FurnaceCart{(va*0.2+vb-push,vb*0.7)}else{let average=(va+vb)*0.5;(va*0.2+average-push,vb*0.2+average+push)};
    a.velocity=na+Vec4::Y*a.velocity.y;b.velocity=nb+Vec4::Y*b.velocity.y;
}
impl Vehicles {
    pub fn new(world: &mut World) -> Self {
        Self {
            saved: SavedVehicles::default(),
            active_cargo: None,
            changes: world.subscribe_changes(),
            chunk_events:world.subscribe_chunk_lifecycle(),
            scans:VecDeque::new(),
            queue: VecDeque::new(),
            queued: HashSet::new(),
            fraction: 0.0,
            rider_wish: Vec4::ZERO,
        }
    }
    pub fn restore(&mut self, mut saved: SavedVehicles) {
        saved
            .entities
            .retain(|v| v.position.is_finite() && v.velocity.is_finite() && v.forward.is_finite());
        saved.entities.truncate(64);
        for v in &mut saved.entities {
            v.velocity = v.velocity.clamp(Vec4::splat(-2.0), Vec4::splat(2.0));
            v.cargo.resize(
                if v.kind == VehicleKind::ChestCart {
                    27
                } else {
                    0
                },
                Slot::EMPTY,
            );
            for s in &mut v.cargo {
                *s = s.normalized().unwrap_or(Slot::EMPTY);
            }
            v.fuel = v.fuel.min(32000);
            v.damage = v.damage.min(100);
        }
        if saved.riding.is_some_and(|i| i >= saved.entities.len()) {
            saved.riding = None;
        }
        self.saved = saved;
    }
    fn schedule(&mut self, p: IVec4) {
        if self.queued.insert(p) {
            self.queue.push_back(p);
        }
    }
}
fn track_axis(s: BlockState) -> usize {
    match rails::shape(s) {
        1 | 2 | 3 => 0,
        16 | 17 | 18 => 3,
        _ => 2,
    }
}
impl Game {
    pub fn targeted_vehicle_name(&self) -> Option<&'static str> {
        let Some(Target::Vehicle(i)) = self.target else {
            return None;
        };
        self.vehicles.saved.entities.get(i).map(|v| match v.kind {
            VehicleKind::Boat => "Boat",
            VehicleKind::Minecart => "Minecart",
            VehicleKind::ChestCart => "Minecart with Chest",
            VehicleKind::FurnaceCart => "Minecart with Furnace",
        })
    }

    pub(super) fn use_vehicle_item(&mut self) -> bool {
        let Some(stack) = self.hotbar[self.selected] else {
            return false;
        };
        let kind = match stack.block {
            item::BOAT => VehicleKind::Boat,
            item::MINECART => VehicleKind::Minecart,
            item::CHEST_MINECART => VehicleKind::ChestCart,
            item::FURNACE_MINECART => VehicleKind::FurnaceCart,
            _ => return false,
        };
        if self.vehicles.saved.entities.len() >= 64 {
            return false;
        }
        let hit = if kind == VehicleKind::Boat {
            raycast_states(
                self.eye(),
                self.basis().forward,
                5.0,
                &|p| self.world.block_state(p),
                true,
            )
        } else {
            match self.target {
                Some(Target::Block(h)) => Some(h),
                _ => None,
            }
        };
        let Some(hit) = hit else { return false };
        let Some(state) = self.world.block_state(hit.block) else {
            return false;
        };
        let position = if kind == VehicleKind::Boat {
            if !block::is_water(state.id) || hit.normal != IVec4::Y {
                return false;
            }
            hit.block.as_vec4() + Vec4::new(0.5, 0.5, 0.5, 0.5)
        } else {
            if !rails::is_rail(state.id) {
                return false;
            }
            rails::project(
                hit.block,
                rails::shape(state),
                hit.block.as_vec4() + Vec4::splat(0.5),
            )
            .0
        };
        self.vehicles.saved.entities.push(Vehicle {
            kind,
            position,
            velocity: Vec4::ZERO,
            forward: self.basis().forward_h,
            cargo: vec![
                Slot::EMPTY;
                if kind == VehicleKind::ChestCart {
                    27
                } else {
                    0
                }
            ],
            fuel: 0,
            push: Vec4::ZERO,
            damage: 0,
            previous_position: None,
            previous_forward: None,
        });
        if self.mode == GameMode::Survival {
            self.take_selected();
        }
        self.start_hand_use();
        true
    }
    pub(super) fn aimed_vehicle(&self) -> Option<usize> {
        if self.vehicles.saved.entities.is_empty() {
            return None;
        }
        let origin = self.eye();
        let ray = self.basis().forward;
        let wall = raycast_states(origin, ray, REACH, &|p| self.world.block_state(p), false)
            .map_or(REACH, |h| h.dist);
        let limit = match self.target {
            Some(Target::Block(h)) => h.dist,
            Some(Target::Painting(i)) => self.painting_target_distance(i).unwrap_or(REACH),
            Some(Target::Creature(i)) => self
                .creatures
                .get(i)
                .and_then(|c| c.target_distance(origin, ray, REACH))
                .unwrap_or(REACH),
            _ => REACH,
        }
        .min(wall);
        self.vehicles
            .saved
            .entities
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                let width = if v.kind == VehicleKind::Boat {
                    0.75
                } else {
                    0.49
                };
                let lo =
                    v.draw_position(self.vehicles.fraction) - Vec4::new(width, 0.0, width, width);
                let hi =
                    v.draw_position(self.vehicles.fraction) + Vec4::new(width, 0.7, width, width);
                let mut near = 0.0_f32;
                let mut far = limit;
                for axis in 0..4 {
                    if ray[axis].abs() < 1e-7 {
                        if origin[axis] < lo[axis] || origin[axis] > hi[axis] {
                            return None;
                        }
                    } else {
                        let a = (lo[axis] - origin[axis]) / ray[axis];
                        let b = (hi[axis] - origin[axis]) / ray[axis];
                        near = near.max(a.min(b));
                        far = far.min(a.max(b));
                    }
                }
                (near < far && far > 0.0).then_some((i, near))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
    pub(super) fn interact_vehicle(&mut self, attack: bool) -> bool {
        let Some(index) = self.aimed_vehicle() else {
            return false;
        };
        if attack {
            let v = &mut self.vehicles.saved.entities[index];
            v.damage = v.damage.saturating_add(
                (item::attack_damage(self.hotbar[self.selected].map_or(0, |s| s.block)).max(1)
                    as u16)
                    * 10,
            );
            if self.mode == GameMode::Creative || v.damage > 40 {
                self.break_vehicle(index, false);
            }
            return true;
        }
        let kind = self.vehicles.saved.entities[index].kind;
        if kind == VehicleKind::ChestCart {
            self.close_station();
            self.vehicles.active_cargo = Some(index);
            return true;
        }
        if kind == VehicleKind::FurnaceCart {
            if self.hotbar[self.selected].is_some_and(|s| s.block == item::COAL) {
                if self.mode == GameMode::Survival {
                    self.take_selected();
                }
                // xb.a: each coal adds 1200 ticks of fuel in Beta 1.5.
                self.vehicles.saved.entities[index].fuel = self.vehicles.saved.entities[index]
                    .fuel
                    .saturating_add(1200)
                    .min(32000);
            }
            let delta = self.vehicles.saved.entities[index].position - self.body.pos;
            self.vehicles.saved.entities[index].push =
                Vec4::new(delta.x, 0.0, delta.z, delta.w).normalize_or_zero();
            return true;
        }
        if self.vehicles.saved.riding == Some(index) {
            self.vehicles.saved.riding = None;
            self.vehicles.saved.rail_origin = None;
            self.vehicles.rider_wish = Vec4::ZERO;
            return true;
        }
        if self.vehicles.saved.riding.is_none() {
            self.vehicles.saved.riding = Some(index);
            self.vehicles.saved.rail_origin = (kind == VehicleKind::Minecart)
                .then_some(self.vehicles.saved.entities[index].position);
            self.flying = false;
        }
        true
    }
    fn break_vehicle(&mut self, index: usize, _impact: bool) {
        let vehicle = self.vehicles.saved.entities.remove(index);
        let pos = vehicle.position;
        let mut drops = vehicle.cargo;
        match vehicle.kind {
            VehicleKind::Boat => {
                drops.push(Slot::new(block::PLANKS, 3));
                drops.push(Slot::new(item::STICK, 2));
            }
            kind => {
                drops.push(Slot::new(item::MINECART, 1));
                if kind == VehicleKind::ChestCart {
                    drops.push(Slot::new(block::CHEST, 1));
                }
                if kind == VehicleKind::FurnaceCart {
                    drops.push(Slot::new(block::FURNACE, 1));
                }
            }
        }
        for stack in drops.into_iter().filter_map(Slot::normalized) {
            self.spawn_drop(stack, pos, Vec4::Y * 2.0, 0.5);
        }
        for selected in [
            &mut self.vehicles.saved.riding,
            &mut self.vehicles.active_cargo,
        ] {
            if *selected == Some(index) {
                *selected = None;
            } else if let Some(i) = selected {
                if *i > index {
                    *i -= 1;
                }
            }
        }
    }
    pub(super) fn move_mounted_player(&mut self, input: &InputState, b: &Bindings) -> bool {
        let Some(index) = self.vehicles.saved.riding else {
            self.vehicles.rider_wish = Vec4::ZERO;
            return false;
        };
        if !self.is_alive() || input.just_pressed(b, Action::Sneak) {
            self.vehicles.saved.riding = None;
            self.vehicles.saved.rail_origin = None;
            self.body.vel = Vec4::ZERO;
            return false;
        }
        self.flying = false;
        let basis = self.basis();
        let axis = |p, n| input.is_down(b, p) as i32 as f32 - input.is_down(b, n) as i32 as f32;
        let wish = basis.forward_h * axis(Action::MoveForward, Action::MoveBack)
            + basis.right * axis(Action::MoveRight, Action::MoveLeft)
            + basis.ana * axis(Action::MoveAna, Action::MoveKata);
        let Some(v) = self.vehicles.saved.entities.get_mut(index) else {
            self.vehicles.saved.riding = None;
            return false;
        };
        self.vehicles.rider_wish = wish.clamp_length_max(1.0);
        self.body.pos = v.draw_position(self.vehicles.fraction)
            + Vec4::Y
                * if v.kind == VehicleKind::Boat {
                    0.0
                } else {
                    0.05
                };
        self.body.vel = v.velocity * 20.0;
        self.body.on_ground = false;
        true
    }
    fn powered_track_source(&self, start: IVec4) -> bool {
        let mut queue = VecDeque::from([(start, 0u8)]);
        let mut seen = HashSet::new();
        while let Some((p, distance)) = queue.pop_front() {
            if !seen.insert(p) {
                continue;
            }
            if self.block_powered(p) {
                return true;
            }
            if distance == 8 {
                continue;
            }
            let Some(s) = self.world.block_state(p) else {
                continue;
            };
            let axis = track_axis(s);
            for sign in [-1, 1] {
                let mut d = IVec4::ZERO;
                d[axis] = sign;
                if let Some(q) = rails::neighbor(p, d, &|p| self.world.block_state(p)) {
                    if self
                        .world
                        .block_state(q)
                        .is_some_and(|t| t.id == block::POWERED_RAIL && track_axis(t) == axis)
                    {
                        queue.push_back((q, distance + 1));
                    }
                }
            }
        }
        false
    }
    pub(super) fn update_vehicles(&mut self, dt: f32) {
        for _ in 0..256 {let Ok(event)=self.vehicles.chunk_events.try_recv()else{break};if !event.loaded{continue;}if self.world.chunks.get(&event.coordinate).is_some_and(|c|c.identity==event.identity&&c.may_contain_any(&[block::RAIL,block::POWERED_RAIL,block::DETECTOR_RAIL])){self.vehicles.scans.push_back((event.coordinate,event.identity,0));}}
        for _ in 0..1024 {let Some((c,identity,index))=self.vehicles.scans.front().copied()else{break};let Some(chunk)=self.world.chunks.get(&c).filter(|c|c.identity==identity)else{self.vehicles.scans.pop_front();continue};
            let local=IVec4::new((index&15)as i32,((index>>4)&15)as i32,((index>>8)&15)as i32,((index>>12)&15)as i32);let state=chunk.get_state(local);if index+1==mc4d_core::chunk::CHUNK_VOLUME{self.vehicles.scans.pop_front();}else{self.vehicles.scans.front_mut().unwrap().2+=1;}
            if rails::is_rail(state.id){let p=c*16+local;self.vehicles.schedule(p);if state.id==block::DETECTOR_RAIL&&rails::powered(state)&&!self.vehicles.saved.detectors.iter().any(|t|t.0==p){self.vehicles.saved.detectors.push((p,20));}}
        }
        for change in self
            .vehicles
            .changes
            .try_iter()
            .take(512)
            .collect::<Vec<_>>()
        {
            for d in std::iter::once(IVec4::ZERO).chain(rails::DIRECTIONS) {
                for y in -1..=1 {
                    let p = change.position + d + IVec4::Y * y;
                    if self.world.chunks.get(&chunk_of(p)).is_some_and(|c| {
                        c.may_contain_any(&[block::RAIL, block::POWERED_RAIL, block::DETECTOR_RAIL])
                    }) && self.world.block(p).is_some_and(rails::is_rail)
                    {
                        self.vehicles.schedule(p);
                    }
                }
            }
        }
        for _ in 0..128 {
            let Some(p) = self.vehicles.queue.pop_front() else {
                break;
            };
            self.vehicles.queued.remove(&p);
            let Some(state) = self.world.block_state(p) else {
                continue;
            };
            if !rails::is_rail(state.id) {
                continue;
            }
            let upper = match rails::shape(state) {
                2 => IVec4::X,
                3 => -IVec4::X,
                4 => -IVec4::Z,
                5 => IVec4::Z,
                17 => IVec4::W,
                18 => -IVec4::W,
                _ => IVec4::ZERO,
            };
            if (self.world.block(p - IVec4::Y).is_some() && !self.full_support(p - IVec4::Y))
                || (upper != IVec4::ZERO
                    && self.world.block(p + upper).is_some()
                    && !self.full_support(p + upper))
            {
                self.world.set_block(p, AIR);
                self.spawn_drop(
                    Slot::new(state.id, 1),
                    p.as_vec4() + Vec4::splat(0.5),
                    Vec4::ZERO,
                    0.5,
                );
                continue;
            }
            let shape = rails::connected_shape_powered(p, state, self.block_powered(p), &|q| self.world.block_state(q));
            let next = BlockState::new(
                state.id,
                if state.id == block::RAIL {
                    shape
                } else {
                    rails::with_power(
                        BlockState::new(state.id, shape),
                        if state.id == block::POWERED_RAIL {
                            self.powered_track_source(p)
                        } else {
                            rails::powered(state)
                        },
                    )
                    .metadata
                },
            );
            if next != state {
                self.world.set_block_state(p, next);
            }
        }
        self.vehicles.fraction += dt.min(0.1) * 20.0;
        while self.vehicles.fraction >= 1.0 {
            self.vehicles.fraction -= 1.0;
            self.vehicle_tick();
        }
        if let Some(i) = self.vehicles.saved.riding {
            if let Some(v) = self.vehicles.saved.entities.get(i) {
                self.body.pos = v.draw_position(self.vehicles.fraction)
                    + Vec4::Y
                        * if v.kind == VehicleKind::Boat {
                            0.0
                        } else {
                            0.05
                        };
                self.body.vel = v.velocity * 20.0;
            }
        }
    }
    fn vehicle_tick(&mut self) {
        let mut broken = Vec::new();
        let mut traveled = Vec::new();
        for index in 0..self.vehicles.saved.entities.len() {
            let v=&mut self.vehicles.saved.entities[index];
            // Like entities in an unloaded vanilla chunk, distant vehicles wait.
            // An unknown chunk boundary is not a solid wall which breaks boats.
            let width = if v.kind == VehicleKind::Boat {
                0.75
            } else {
                0.49
            };
            let lo = chunk_of(
                (v.position.min(v.position + v.velocity) - Vec4::new(width, 0.0, width, width)
                    + Vec4::splat(0.0001))
                .floor()
                .as_ivec4(),
            );
            let hi = chunk_of(
                (v.position.max(v.position + v.velocity) + Vec4::new(width, 0.7, width, width)
                    - Vec4::splat(0.0001))
                .floor()
                .as_ivec4(),
            );
            let mut loaded = true;
            for mask in 0..16 {
                let c = IVec4::new(
                    if mask & 1 == 0 { lo.x } else { hi.x },
                    if mask & 2 == 0 { lo.y } else { hi.y },
                    if mask & 4 == 0 { lo.z } else { hi.z },
                    if mask & 8 == 0 { lo.w } else { hi.w },
                );
                if c.y >= 0 && c.y < mc4d_core::chunk::WORLD_CHUNKS_Y && !self.world.is_loaded(c) {
                    loaded = false;
                    break;
                }
            }
            if !loaded {
                v.previous_position = Some(v.position);
                v.previous_forward = Some(v.forward);
                continue;
            }
            v.damage = v.damage.saturating_sub(1);
            let before = v.position;
            v.previous_position = Some(v.position);
            v.previous_forward = Some(v.forward);
            if self.vehicles.saved.riding == Some(index) {
                if v.kind == VehicleKind::Boat {
                    v.velocity += self.vehicles.rider_wish * 0.008;
                } else if v.velocity.length_squared() < 0.0001 {
                    v.velocity += self.vehicles.rider_wish * 0.02;
                }
            }
            if v.kind == VehicleKind::Boat {
                let mut immersed = 0.0;
                for layer in 0..5 {
                    let slice=Body::with_size(v.position+Vec4::Y*(layer as f32*0.12-0.125),0.75,0.12);
                    if slice.touches_fluid(block::FluidKind::Water,&|p|self.world.block_state(p)){immersed+=0.2;}
                }
                v.velocity.y += 0.04 * (immersed * 2.0 - 1.0);
                let speed = Vec4::new(v.velocity.x, 0.0, v.velocity.z, v.velocity.w).length();
                v.velocity.x = v.velocity.x.clamp(-0.4, 0.4);
                v.velocity.z = v.velocity.z.clamp(-0.4, 0.4);
                v.velocity.w = v.velocity.w.clamp(-0.4, 0.4);
                let direction =
                    Vec4::new(v.velocity.x, 0.0, v.velocity.z, v.velocity.w).normalize_or_zero();
                if direction.length_squared() > 0.5 {
                    let current = v.forward.normalize_or_zero();
                    let angle = current.dot(direction).clamp(-1.0, 1.0).acos();
                    if angle < 0.001 {
                        v.forward = direction;
                    } else {
                        let mut tangent =
                            (direction - current * current.dot(direction)).normalize_or_zero();
                        if tangent.length_squared() < 0.5 {
                            let reference = if current.x.abs() < 0.9 {
                                Vec4::X
                            } else {
                                Vec4::Z
                            };
                            tangent = (reference - current * current.dot(reference)).normalize();
                        }
                        let turn = angle.min(20.0_f32.to_radians());
                        v.forward =
                            (current * turn.cos() + tangent * turn.sin()).normalize_or_zero();
                    }
                }
                let mut body = Body::with_size(v.position, 0.75, 0.6);
                body.vel = v.velocity * 20.0;
                body.step_states_options(0.05, &|p| self.world.block_state(p), 0.0, false);
                let blocked = (body.vel.x.abs() + body.vel.z.abs() + body.vel.w.abs())
                    < (v.velocity.x.abs() + v.velocity.z.abs() + v.velocity.w.abs()) * 10.0;
                v.position = body.pos;
                v.velocity = body.vel / 20.0;
                if blocked && speed > 0.15 {
                    broken.push(index);
                } else {
                    if body.on_ground {
                        v.velocity *= 0.5;
                    }
                    v.velocity *= Vec4::new(0.99, 0.95, 0.99, 0.99);
                }
            } else {
                let mut cell = v.position.floor().as_ivec4();
                if self
                    .world
                    .block(cell - IVec4::Y)
                    .is_some_and(rails::is_rail)
                {
                    cell -= IVec4::Y;
                }
                if let Some(state) = self
                    .world
                    .block_state(cell)
                    .filter(|s| rails::is_rail(s.id))
                {
                    let (projected, tangent) =
                        rails::project(cell, rails::shape(state), v.position);
                    let horizontal = Vec4::new(tangent.x, 0.0, tangent.z, tangent.w);
                    let horizontal_length = horizontal.length();
                    let direction = horizontal / horizontal_length;
                    let moving = Vec4::new(v.velocity.x, 0.0, v.velocity.z, v.velocity.w);
                    let mut speed = moving.length()
                        * if moving.dot(direction) < 0.0 {
                            -1.0
                        } else {
                            1.0
                        };
                    speed -= if tangent.y > 0.0 {
                        0.0078125
                    } else if tangent.y < 0.0 {
                        -0.0078125
                    } else {
                        0.0
                    };
                    if state.id == block::POWERED_RAIL {
                        if rails::powered(state) {
                            if speed.abs() > 0.01 {
                                speed += speed.signum() * 0.06;
                            } else {
                                let axis = track_axis(state);
                                let mut step = IVec4::ZERO;
                                step[axis] = 1;
                                if self
                                    .world
                                    .block(cell - step)
                                    .is_some_and(|id| block::definition(id).opaque)
                                {
                                    speed = 0.02;
                                } else if self
                                    .world
                                    .block(cell + step)
                                    .is_some_and(|id| block::definition(id).opaque)
                                {
                                    speed = -0.02;
                                } else {
                                    speed += v.push.dot(direction) * 0.02;
                                }
                            }
                        } else if speed.abs() < 0.03 {
                            speed = 0.0;
                        } else {
                            speed *= 0.5;
                        }
                    }
                    if v.kind == VehicleKind::FurnaceCart && v.fuel > 0 {
                        v.fuel -= 1;
                        speed = speed * 0.8 + v.push.dot(direction) * 0.04;
                    }
                    let limit = 0.4 / direction.abs().max_element().max(0.00001);
                    speed = speed.clamp(-limit, limit);
                    let desired = projected + tangent * (speed / horizontal_length);
                    if self
                        .world
                        .block(desired.floor().as_ivec4())
                        .is_some_and(block::is_solid)
                    {
                        speed = 0.0;
                        v.position = projected;
                    } else {
                        v.position = desired;
                    }
                    v.velocity = tangent
                        * (speed / horizontal_length)
                        * if self.vehicles.saved.riding == Some(index) {
                            0.99699998
                        } else {
                            0.96
                        };
                    v.forward = if speed < 0.0 { -tangent } else { tangent };
                } else {
                    v.velocity.y -= 0.04;
                    let mut body = Body::with_size(v.position, 0.49, 0.7);
                    body.vel = v.velocity.clamp(Vec4::splat(-0.4), Vec4::splat(0.4)) * 20.0;
                    body.step_states_options(0.05, &|p| self.world.block_state(p), 0.0, false);
                    v.position = body.pos;
                    v.velocity = body.vel / 20.0 * if body.on_ground { 0.5 } else { 0.95 };
                }
            }
            if v.position.y < -64.0 {
                broken.push(index);
            }
            let displacement = (v.position - before).length();
            if self.vehicles.saved.riding == Some(index) {
                traveled.push((v.kind, displacement, v.position));
            }
            if !broken.contains(&index){self.collide_vehicle(index);}
        }
        for (kind, distance, pos) in traveled {
            self.record_distance(
                if kind == VehicleKind::Boat {
                    "boat_cm"
                } else {
                    "minecart_cm"
                },
                distance,
            );
            if kind != VehicleKind::Boat
                && self
                    .vehicles
                    .saved
                    .rail_origin
                    .is_some_and(|origin| (pos - origin).length_squared() >= 1_000_000.0)
            {
                self.award_achievement(11);
            }
        }
        let occupied: HashSet<_> = self
            .vehicles
            .saved
            .entities
            .iter()
            .filter(|v| v.kind != VehicleKind::Boat)
            .filter_map(|v| {
                let p = v.position.floor().as_ivec4();
                let p = if self.world.block(p - IVec4::Y) == Some(block::DETECTOR_RAIL) {
                    p - IVec4::Y
                } else {
                    p
                };
                (self.world.block(p) == Some(block::DETECTOR_RAIL)
                    && v.position.y < p.y as f32 + 0.25)
                    .then_some(p)
            })
            .collect();
        for timer in &mut self.vehicles.saved.detectors {
            if self.world.block_state(timer.0).is_some() {
                timer.1 = timer.1.saturating_sub(1);
            }
        }
        for &p in &occupied {
            if !self.vehicles.saved.detectors.iter().any(|t| t.0 == p) {
                self.vehicles.saved.detectors.push((p, 20));
                if let Some(s) = self.world.block_state(p) {
                    self.world.set_block_state(p, rails::with_power(s, true));
                }
            }
        }
        for timer in &mut self.vehicles.saved.detectors {
            if timer.1 == 0 && occupied.contains(&timer.0) {
                timer.1 = 20;
            }
        }
        let released: Vec<_> = self
            .vehicles
            .saved
            .detectors
            .iter()
            .filter(|t| t.1 == 0)
            .map(|t| t.0)
            .collect();
        self.vehicles.saved.detectors.retain(|t| t.1 > 0);
        for p in released {
            if let Some(s) = self
                .world
                .block_state(p)
                .filter(|s| s.id == block::DETECTOR_RAIL)
            {
                self.world.set_block_state(p, rails::with_power(s, false));
            }
        }
        broken.sort_unstable();
        broken.dedup();
        for index in broken.into_iter().rev() {
            self.break_vehicle(index, true);
        }
        if let Some(i) = self.vehicles.active_cargo {
            if self
                .vehicles
                .saved
                .entities
                .get(i)
                .is_none_or(|v| (v.position - self.body.pos).length_squared() > 64.0)
            {
                self.close_station();
            }
        }
    }
    fn vehicle_bounds(v:&Vehicle)->mc4d_core::shapes::Aabb4 {Body::with_size(v.position,if v.kind==VehicleKind::Boat{0.75}else{0.49},if v.kind==VehicleKind::Boat{0.6}else{0.7}).bounds()}
    fn collide_vehicle(&mut self,index:usize){
        let bounds=Self::vehicle_bounds(&self.vehicles.saved.entities[index]);let margin=Vec4::new(0.2,0.0,0.2,0.2);let query=mc4d_core::shapes::Aabb4::new(bounds.min-margin,bounds.max+margin);
        for other in 0..self.vehicles.saved.entities.len(){if other==index{continue;}let a=&self.vehicles.saved.entities[index];let b=&self.vehicles.saved.entities[other];if (a.kind==VehicleKind::Boat)!=(b.kind==VehicleKind::Boat)||!self.world.is_loaded(chunk_of(b.position.floor().as_ivec4()))||!query.overlaps(Self::vehicle_bounds(b)){continue;}
            let (a,b)=if index<other{let(left,right)=self.vehicles.saved.entities.split_at_mut(other);(&mut left[index],&mut right[0])}else{let(left,right)=self.vehicles.saved.entities.split_at_mut(index);(&mut right[0],&mut left[other])};
            vehicle_pair_impulse(a,b);
        }
        if self.vehicles.saved.riding.is_none()&&query.overlaps(self.body.bounds()){
            let v=&mut self.vehicles.saved.entities[index];let delta=self.body.pos-v.position;let delta=Vec4::new(delta.x,0.0,delta.z,delta.w);let length=delta.length();if length>0.01{let push=delta/length*(1.0/length).min(1.0)*0.05;v.velocity-=push;self.body.vel+=push*if v.kind==VehicleKind::Boat{20.0}else{5.0};}
        }
    }
    pub(super) fn damage_vehicle(&mut self,index:usize,amount:u8){if amount==0||index>=self.vehicles.saved.entities.len(){return;}let v=&mut self.vehicles.saved.entities[index];v.damage=v.damage.saturating_add(u16::from(amount)*10);if v.damage>40{self.break_vehicle(index,false);}}
    pub(super) fn vehicles_blast(&mut self,center:Vec4,radius:u8,get:&impl Fn(IVec4)->Option<BlockState>){
        let impacts:Vec<_>=self.vehicles.saved.entities.iter().enumerate().filter_map(|(i,v)|{let bounds=Self::vehicle_bounds(v);let delta=(bounds.min+bounds.max)*0.5-center;let distance=delta.length();if distance>=f32::from(radius)*2.0{return None;}let exposure=mc4d_core::explosion::exposure(center,bounds,get);Some((i,mc4d_core::explosion::damage(radius,distance,exposure),delta.normalize_or_zero()*(1.0-distance/(f32::from(radius)*2.0))*exposure))}).collect();
        for(i,damage,push)in impacts.into_iter().rev(){self.vehicles.saved.entities[i].velocity+=push;self.damage_vehicle(i,damage);}
    }
    pub(super) fn vehicles_lightning(&mut self,min:Vec4,max:Vec4){let area=mc4d_core::shapes::Aabb4::new(min,max);let indices:Vec<_>=self.vehicles.saved.entities.iter().enumerate().filter(|(_,v)|area.overlaps(Self::vehicle_bounds(v))).map(|(i,_)|i).collect();for i in indices.into_iter().rev(){self.damage_vehicle(i,5);}}

    pub(super) fn vehicle_draws(&self) -> impl Iterator<Item = EntityDraw> + '_ {
        self.vehicles.saved.entities.iter().map(|v| EntityDraw {
            center: v.draw_position(self.vehicles.fraction),
            radius: 1.0,
            color: [1.0; 3],
            kind: if v.kind == VehicleKind::Boat { 18 } else { 19 },
            forward: v.draw_forward(self.vehicles.fraction),
            item: match v.kind {
                VehicleKind::ChestCart => block::CHEST.into(),
                VehicleKind::FurnaceCart => block::FURNACE.into(),
                _ => 0,
            },
            item_variant: 0,
            state: v.damage as u32
                | ((match v.kind {
                    VehicleKind::ChestCart => 1,
                    VehicleKind::FurnaceCart => 2,
                    _ => 0,
                }) << 8),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc4d_core::chunk::Chunk;
    fn scene() -> Game {
        let mut g = Game::menu_preview(7);
        g.body.pos = Vec4::new(0.5, 66.0, 0.5, 0.5);
        for x in -1..=1 {
            for z in -1..=1 {
                for w in -1..=1 {
                    for y in 3..=4 {
                        g.world
                            .chunks
                            .insert(IVec4::new(x, y, z, w), Chunk::uniform(AIR));
                    }
                }
            }
        }
        g
    }
    fn cart(kind: VehicleKind, pos: Vec4, velocity: Vec4) -> Vehicle {
        Vehicle {
            kind,
            position: pos,
            velocity,
            forward: Vec4::Z,
            cargo: vec![
                Slot::EMPTY;
                if kind == VehicleKind::ChestCart {
                    27
                } else {
                    0
                }
            ],
            fuel: 0,
            push: Vec4::ZERO,
            damage: 0,
            previous_position: None,
            previous_forward: None,
        }
    }
    #[test]
    fn cart_moves_on_real_w_rails_and_unpowered_rail_brakes() {
        let mut g = scene();
        for w in 0..14 {
            g.world.set_block(IVec4::new(0, 65, 0, w), block::STONE);
            g.world
                .set_block_state(IVec4::new(0, 66, 0, w), BlockState::new(block::RAIL, 16));
        }
        g.vehicles.saved.entities.push(cart(
            VehicleKind::Minecart,
            Vec4::new(0.5, 66.0625, 0.5, 1.5),
            Vec4::W * 0.2,
        ));
        for _ in 0..10 {
            g.update_vehicles(0.05);
        }
        assert!(g.vehicles.saved.entities[0].position.w > 2.8);
        let pos = g.vehicles.saved.entities[0].position.floor().as_ivec4();
        g.world
            .set_block_state(pos, BlockState::new(block::POWERED_RAIL, 16));
        g.vehicle_tick();
        assert!(g.vehicles.saved.entities[0].velocity.w < 0.1);
    }
    #[test]
    fn boat_placement_uses_water_hit_and_consumes_only_success() {
        let mut g = scene();
        for x in 0..8 {
            for z in 0..8 {
                for w in 0..4 {
                    g.world.set_block(IVec4::new(x, 65, z, w), block::WATER);
                }
            }
        }
        g.hotbar[0] = Some(Slot::new(item::BOAT, 1));
        g.orient.add_pitch(-0.65);
        assert!(g.use_vehicle_item());
        assert!(g.hotbar[0].is_none());
        assert_eq!(g.vehicles.saved.entities.len(), 1);
        let before = g.vehicles.saved.entities[0].position;
        for _ in 0..200 {
            g.vehicle_tick();
        }
        let after = g.vehicles.saved.entities[0].position;
        assert!(
            (after.y - before.y).abs() < 0.5,
            "boat remains at water surface: {after:?}"
        );
    }
    #[test]
    fn cart_cargo_survives_actual_save_reload_and_break_without_duplication() {
        let mut g = scene();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-vehicles")
            .join(format!("{}-{stamp}", std::process::id()));
        g.save = Some(Arc::new(SaveDir::new(root.clone()).unwrap()));
        let mut v = cart(
            VehicleKind::ChestCart,
            Vec4::new(2.5, 66.0, 2.5, 0.5),
            Vec4::W * 0.15,
        );
        v.cargo[26] = Slot::with_variant(block::WOOL, 37, 14);
        g.vehicles.saved.entities.push(v);
        g.save_checked().unwrap();
        let mut restored = Game::try_load(Some(root.clone()), 7, GameMode::Survival).unwrap();
        assert_eq!(
            restored.vehicles.saved.entities[0].cargo[26],
            Slot::with_variant(block::WOOL, 37, 14)
        );
        restored.vehicles.active_cargo = Some(0);
        restored.quick_transfer_station(26);
        assert_eq!(restored.station_slot(26), None);
        assert_eq!(restored.hotbar[0].unwrap().count, 37);
        restored.break_vehicle(0, false);
        assert_eq!(
            restored
                .dropped_items
                .iter()
                .filter(|d| d.stack.block == item::MINECART)
                .map(|d| d.stack.count)
                .sum::<u32>(),
            1
        );
        assert_eq!(
            restored
                .dropped_items
                .iter()
                .filter(|d| d.stack.block == u16::from(block::CHEST))
                .map(|d| d.stack.count)
                .sum::<u32>(),
            1
        );
        restored.save_checked().unwrap();
        let again = Game::try_load(Some(root.clone()), 7, GameMode::Survival).unwrap();
        assert!(again.vehicles.saved.entities.is_empty());
        assert_eq!(again.hotbar[0].unwrap().count, 37);
        drop(again);
        drop(restored);
        drop(g);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn broken_boat_on_impact_returns_original_planks_and_sticks() {
        let mut g = scene();
        g.vehicles.saved.entities.push(cart(
            VehicleKind::Boat,
            Vec4::new(1.0, 66.0, 1.0, 1.0),
            Vec4::ZERO,
        ));
        g.break_vehicle(0, true);
        assert_eq!(
            g.dropped_items
                .iter()
                .find(|d| d.stack.block == u16::from(block::PLANKS))
                .unwrap()
                .stack
                .count,
            3
        );
        assert_eq!(
            g.dropped_items
                .iter()
                .find(|d| d.stack.block == item::STICK)
                .unwrap()
                .stack
                .count,
            2
        );
    }
    #[test]
    fn powered_rail_signal_stops_after_eight_neighbors_and_detector_releases_after_twenty_ticks() {
        let mut g = scene();
        for x in 0..12 {
            g.world.set_block(IVec4::new(x, 65, 0, 0), block::STONE);
            g.world.set_block_state(
                IVec4::new(x, 66, 0, 0),
                BlockState::new(block::POWERED_RAIL, 1),
            );
        }
        g.world.set_block_state(
            IVec4::new(0, 66, 1, 0),
            BlockState::new(block::REDSTONE_TORCH, 5),
        );
        assert!(g.powered_track_source(IVec4::new(8, 66, 0, 0)));
        assert!(!g.powered_track_source(IVec4::new(9, 66, 0, 0)));
        let cell = IVec4::new(4, 66, 0, 0);
        g.world
            .set_block_state(cell, BlockState::new(block::DETECTOR_RAIL, 1));
        g.vehicles.saved.entities.push(cart(
            VehicleKind::Minecart,
            cell.as_vec4() + Vec4::new(0.5, 0.0625, 0.5, 0.5),
            Vec4::ZERO,
        ));
        g.vehicle_tick();
        assert!(rails::powered(g.world.block_state(cell).unwrap()));
        assert!(g.block_powered(cell + IVec4::Z));
        g.vehicles.saved.entities.clear();
        for _ in 0..19 {
            g.vehicle_tick();
        }
        assert!(rails::powered(g.world.block_state(cell).unwrap()));
        g.vehicle_tick();
        assert!(!rails::powered(g.world.block_state(cell).unwrap()));
    }
    #[test]
    fn vehicle_target_respects_walls_and_existing_boat_mounts_before_item_placement() {
        let mut g = scene();
        for z in 0..8 {
            g.world.set_block(IVec4::new(0, 65, z, 0), block::WATER);
        }
        g.vehicles.saved.entities.push(cart(
            VehicleKind::Boat,
            Vec4::new(0.5, 65.5, 3.5, 0.5),
            Vec4::ZERO,
        ));
        g.hotbar[0] = Some(Slot::new(item::BOAT, 1));
        g.orient.add_pitch(-0.5);
        g.update_target();
        assert!(matches!(g.target, Some(Target::Vehicle(0))));
        let bindings = Bindings::default();
        let mut input = InputState::default();
        input.press(bindings.get(Action::Place)[0]);
        g.interact(0.05, &input, &bindings);
        assert_eq!(g.vehicles.saved.riding, Some(0));
        assert_eq!(g.vehicles.saved.entities.len(), 1);
        assert_eq!(g.hotbar[0].unwrap().count, 1);
        assert!(g.interact_vehicle(false));
        assert_eq!(g.vehicles.saved.riding, None);
        g.world.set_block(IVec4::new(0, 66, 2, 0), block::STONE);
        g.world.set_block(IVec4::new(0, 67, 2, 0), block::STONE);
        g.update_target();
        assert!(matches!(g.target, Some(Target::Block(_))));
    }
    #[test]
    fn fluid_edits_without_rails_do_not_accumulate_unrelated_rail_work() {
        let mut g = scene();
        for i in 0..512 {
            g.world
                .set_block(IVec4::new(i % 16, 65, (i / 16) % 16, 0), block::WATER);
        }
        g.update_vehicles(0.05);
        assert!(g.vehicles.queue.is_empty());
        assert!(g.vehicles.queued.is_empty());
    }
    #[test]
    fn beta_attacked_boats_also_drop_planks_and_sticks_not_a_reusable_boat() {
        let mut g = scene();
        g.vehicles.saved.entities.push(cart(
            VehicleKind::Boat,
            Vec4::new(1.0, 66.0, 1.0, 1.0),
            Vec4::ZERO,
        ));
        g.break_vehicle(0, false);
        assert_eq!(
            g.dropped_items.iter().map(|d| d.stack.count).sum::<u32>(),
            5
        );
        assert!(!g.dropped_items.iter().any(|d| d.stack.block == item::BOAT));
    }
    #[test]
    fn unloaded_vehicles_wait_without_breaking_or_spending_fuel() {
        let mut g = scene();
        let pos = Vec4::new(1000.0, 66.0, 1.0, 1.0);
        g.vehicles
            .saved
            .entities
            .push(cart(VehicleKind::Boat, pos, Vec4::X * 0.4));
        let mut furnace = cart(VehicleKind::FurnaceCart, pos + Vec4::Z * 3.0, Vec4::X * 0.3);
        furnace.fuel = 1000;
        g.vehicles.saved.entities.push(furnace);
        for _ in 0..40 {
            g.vehicle_tick();
        }
        assert_eq!(g.vehicles.saved.entities.len(), 2);
        assert_eq!(g.vehicles.saved.entities[0].position, pos);
        assert_eq!(g.vehicles.saved.entities[1].fuel, 1000);
        assert!(g.dropped_items.is_empty());
    }
    #[test]
    fn rails_require_a_full_opaque_floor_and_slopes_keep_their_upper_support() {
        let mut g = scene();
        let p = IVec4::new(4, 66, 4, 0);
        g.world.set_block(p - IVec4::Y, block::STONE);
        g.world.set_block_state(p, BlockState::new(block::RAIL, 0));
        g.update_vehicles(0.05);
        assert_eq!(g.world.block(p), Some(block::RAIL));
        g.world.set_block(p - IVec4::Y, block::CACTUS);
        g.update_vehicles(0.05);
        assert_eq!(g.world.block(p), Some(AIR));
        g.world.set_block(p - IVec4::Y, block::STONE);
        g.world.set_block_state(p, BlockState::new(block::RAIL, 2));
        g.world.set_block(p + IVec4::X, block::STONE);
        g.world
            .set_block_state(p + IVec4::X + IVec4::Y, BlockState::new(block::RAIL, 1));
        g.update_vehicles(0.05);
        assert_eq!(rails::shape(g.world.block_state(p).unwrap()), 2);
        g.world.set_block(p + IVec4::X, AIR);
        g.update_vehicles(0.05);
        assert_eq!(g.world.block(p), Some(AIR));
    }
    #[test]
    fn mounted_camera_and_vehicle_render_interpolate_between_native_ticks() {
        let mut g = scene();
        for w in 0..14 {
            g.world.set_block(IVec4::new(0, 65, 0, w), block::STONE);
            g.world
                .set_block_state(IVec4::new(0, 66, 0, w), BlockState::new(block::RAIL, 16));
        }
        let v = cart(
            VehicleKind::Minecart,
            Vec4::new(0.5, 66.0625, 0.5, 1.5),
            Vec4::W * 0.3,
        );
        g.body.pos = v.position + Vec4::Y * 0.05;
        g.vehicles.saved.entities.push(v);
        g.vehicles.saved.riding = Some(0);
        for _ in 0..12 {
            let before = g.body.pos;
            g.update_vehicles(1.0 / 60.0);
            let draw = g.vehicle_draws().next().unwrap();
            assert!((g.body.pos - (draw.center + Vec4::Y * 0.05)).length() < 1e-5);
            assert!(
                (g.body.pos - before).length() < 0.2,
                "camera should not jump a full20Hz tick"
            );
        }
    }
    #[test]
    fn native_curves_preserve_horizontal_speed_and_flat_carts_do_not_drift() {
        let mut g = scene();
        let p = IVec4::new(4, 66, 4, 0);
        g.world.set_block(p - IVec4::Y, block::STONE);
        g.world.set_block_state(p, BlockState::new(block::RAIL, 6));
        g.vehicles.saved.entities.push(cart(
            VehicleKind::Minecart,
            p.as_vec4() + Vec4::new(0.75, 0.0625, 0.75, 0.5),
            -Vec4::Z * 0.3,
        ));
        g.vehicle_tick();
        assert!((g.vehicles.saved.entities[0].velocity.length() - 0.288).abs() < 1e-5);
        g.vehicles.saved.entities.clear();
        g.world.set_block_state(p, BlockState::new(block::RAIL, 0));
        let position = p.as_vec4() + Vec4::new(0.5, 0.0625, 0.5, 0.5);
        g.vehicles
            .saved
            .entities
            .push(cart(VehicleKind::Minecart, position, Vec4::ZERO));
        for _ in 0..20 {
            g.vehicle_tick();
        }
        assert_eq!(g.vehicles.saved.entities[0].position, position);
    }
    #[test]
    fn powered_rail_launches_a_stationary_cart_away_from_a_wall() {
        let mut g = scene();
        let p = IVec4::new(4, 66, 4, 0);
        g.world.set_block(p - IVec4::Y, block::STONE);
        g.world.set_block(p - IVec4::X, block::STONE);
        g.world
            .set_block_state(p, BlockState::new(block::POWERED_RAIL, 9));
        let position = p.as_vec4() + Vec4::new(0.5, 0.0625, 0.5, 0.5);
        g.vehicles
            .saved
            .entities
            .push(cart(VehicleKind::Minecart, position, Vec4::ZERO));
        g.vehicle_tick();
        assert!(g.vehicles.saved.entities[0].position.x > position.x);
        assert!(g.vehicles.saved.entities[0].velocity.x > 0.0);
    }
    #[test]
    fn rider_acceleration_is_independent_of_render_frame_rate() {
        let mut results = Vec::new();
        for fps in [30, 60, 120] {
            let mut g = scene();
            for x in -4..8 {
                for z in -4..10 {
                    for w in -2..=2 {
                        g.world.set_block(IVec4::new(x, 65, z, w), block::WATER);
                    }
                }
            }
            g.vehicles.saved.entities.push(cart(
                VehicleKind::Boat,
                Vec4::new(0.5, 65.5, 0.5, 0.5),
                Vec4::ZERO,
            ));
            g.vehicles.saved.riding = Some(0);
            let bindings = Bindings::default();
            let mut input = InputState::default();
            input.press(bindings.get(Action::MoveForward)[0]);
            for _ in 0..fps {
                g.move_mounted_player(&input, &bindings);
                g.update_vehicles(1.0 / fps as f32);
            }
            results.push(g.vehicles.saved.entities[0].position);
        }
        assert!((results[0] - results[1]).length() < 1e-4);
        assert!((results[1] - results[2]).length() < 1e-4);
    }
    #[test]
    fn riding_a_real_powered_track_one_kilometre_in_w_awards_on_a_rail() {
        let mut g = scene();
        for w in 0..=1010 {
            let c = IVec4::new(0, 4, 0, w / 16);
            g.world
                .chunks
                .entry(c)
                .or_insert_with(|| Chunk::uniform(AIR));
            g.world.set_block(IVec4::new(0, 65, 0, w), block::STONE);
            g.world.set_block_state(
                IVec4::new(0, 66, 0, w),
                BlockState::new(block::POWERED_RAIL, 144),
            );
            if w % 8 == 0 {
                g.world.set_block(IVec4::new(0, 65, 1, w), block::STONE);
                g.world.set_block_state(
                    IVec4::new(0, 66, 1, w),
                    BlockState::new(block::REDSTONE_TORCH, 5),
                );
            }
        }
        for index in 0..=5 {
            g.award_achievement(index);
        }
        let start = Vec4::new(0.5, 66.0625, 0.5, 0.5);
        g.vehicles
            .saved
            .entities
            .push(cart(VehicleKind::Minecart, start, Vec4::W * 0.4));
        g.vehicles.saved.riding = Some(0);
        g.vehicles.saved.rail_origin = Some(start);
        for _ in 0..2502 {
            g.update_vehicles(0.05);
        }
        assert!((g.vehicles.saved.entities[0].position - start).length() >= 1000.0);
        assert!(g.progress.lock().unwrap().unlocked.contains(&11));
    }
    #[test]
    fn native_cart_impulses_transfer_motion_and_furnace_pushes_other_carts_in_w(){
        let mut a=cart(VehicleKind::Minecart,Vec4::ZERO,Vec4::W*0.2);let mut b=cart(VehicleKind::Minecart,Vec4::W*0.8,Vec4::ZERO);vehicle_pair_impulse(&mut a,&mut b);assert!((a.velocity.w-0.09).abs()<1e-6);assert!((b.velocity.w-0.15).abs()<1e-6);
        let mut a=cart(VehicleKind::FurnaceCart,Vec4::ZERO,Vec4::W*0.2);let mut b=cart(VehicleKind::ChestCart,Vec4::W*0.8,Vec4::ZERO);vehicle_pair_impulse(&mut a,&mut b);assert!((a.velocity.w-0.14).abs()<1e-6);assert!((b.velocity.w-0.25).abs()<1e-6);
    }
    #[test]
    fn actual_vehicle_tick_pushes_overlapping_carts_without_spending_inventory(){
        let mut g=scene();let a=Vec4::new(4.5,66.0625,4.5,4.5);let b=a+Vec4::W*0.8;
        for w in 0..10 {g.world.set_block(IVec4::new(4,65,4,w),block::STONE);g.world.set_block_state(IVec4::new(4,66,4,w),BlockState::new(block::RAIL,16));}
        g.vehicles.saved.entities.push(cart(VehicleKind::Minecart,a,Vec4::W*0.2));g.vehicles.saved.entities.push(cart(VehicleKind::Minecart,b,Vec4::ZERO));g.vehicle_tick();assert!(g.vehicles.saved.entities[1].velocity.w>0.05);assert_eq!(g.vehicles.saved.entities.len(),2);assert!(g.dropped_items.is_empty());
    }
    #[test]
    fn reloaded_powered_and_detector_rails_are_revalidated_without_a_neighbor_edit(){
        let mut g=scene();let c=IVec4::new(0,4,0,0);let mut chunk=Chunk::uniform(AIR);chunk.set(IVec4::new(8,0,8,8),block::STONE);chunk.set_state(IVec4::new(8,1,8,8),BlockState::new(block::POWERED_RAIL,8));chunk.set(IVec4::new(10,0,8,8),block::STONE);chunk.set_state(IVec4::new(10,1,8,8),BlockState::new(block::DETECTOR_RAIL,8));g.world.insert_chunk(c,chunk);
        for _ in 0..100{g.update_vehicles(0.05);}assert!(!rails::powered(g.world.block_state(IVec4::new(8,65,8,8)).unwrap()));assert!(!rails::powered(g.world.block_state(IVec4::new(10,65,8,8)).unwrap()));
    }
    #[test]
    fn blast_and_lightning_destroy_vehicle_and_preserve_new_cargo_drops_once(){
        let mut g=scene();let pos=Vec4::new(8.5,66.0,8.5,8.5);let mut vehicle=cart(VehicleKind::ChestCart,pos,Vec4::ZERO);vehicle.cargo[0]=Slot::new(item::IRON_INGOT,7);g.vehicles.saved.entities.push(vehicle);g.explode_with_radius(pos,4);assert!(g.vehicles.saved.entities.is_empty());assert_eq!(g.dropped_items.iter().filter(|d|d.stack.block==item::IRON_INGOT).map(|d|d.stack.count).sum::<u32>(),7);
        g.vehicles.saved.entities.push(cart(VehicleKind::Boat,pos,Vec4::ZERO));g.vehicles_lightning(pos-Vec4::splat(3.0),pos+Vec4::splat(3.0));assert!(g.vehicles.saved.entities.is_empty());assert_eq!(g.dropped_items.iter().filter(|d|d.stack.block==item::ItemId::from(block::PLANKS)).map(|d|d.stack.count).sum::<u32>(),3);
    }

}
