//! Non-mob utility interactions. Jukebox content is the original block metadata:
//! 0 empty, 1 record13, 2 recordcat. Playback itself is transient, as in Beta.
use super::*;
use mc4d_core::{paintings::ARTS, shapes::Aabb4};
use std::sync::mpsc::Receiver;
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Painting {
    anchor: IVec4,
    normal: IVec4,
    #[serde(default)]
    right: IVec4,
    art: usize,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct SavedUtilities {
    paintings: Vec<Painting>,
}
fn right(normal: IVec4) -> IVec4 {
    let n = normal.as_vec4();
    let h = mc4d_core::glam::Vec3::new(n.x, n.z, n.w);
    let reference = if h.z.abs() < 0.9 {
        mc4d_core::glam::Vec3::Z
    } else {
        mc4d_core::glam::Vec3::X
    };
    let r = h.cross(reference);
    IVec4::new(r.x as i32, 0, r.y as i32, r.z as i32)
}
impl Painting {
    fn center(&self) -> Vec4 {
        let a = ARTS[self.art];
        let w = a.width as i32 / 16;
        let h = a.height as i32 / 16;
        self.anchor.as_vec4()
            + Vec4::splat(0.5)
            + self.normal.as_vec4() * (0.5 + 1.0 / 32.0)
            + self.right.as_vec4() * if w % 2 == 0 { 0.5 } else { 0.0 }
            + Vec4::Y * if h % 2 == 0 { 0.5 } else { 0.0 }
    }
    fn bounds(&self) -> Aabb4 {
        let a = ARTS[self.art];
        let n = self.normal.as_vec4().abs();
        let r = self.right.as_vec4().abs();
        let extra = Vec4::ONE - n - r - Vec4::Y;
        let half = r * (a.width as f32 / 32.0)
            + Vec4::Y * (a.height as f32 / 32.0)
            + n / 32.0
            + extra * 0.5;
        Aabb4::new(
            self.center() - half + Vec4::splat(0.00625),
            self.center() + half - Vec4::splat(0.00625),
        )
    }
}

#[derive(Clone, Copy)]
pub struct RecordPlayback {
    pub position: IVec4,
    pub record: item::ItemId,
    pub serial: u64,
    pub elapsed: f32,
}
pub(super) struct Utilities {
    changes: Receiver<mc4d_core::world::BlockChange>,
    pub records: Vec<RecordPlayback>,
    next_serial: u64,
    paintings: Vec<Painting>,
    painting_check: f32,
}
impl Utilities {
    pub fn snapshot(&self) -> SavedUtilities {
        SavedUtilities {
            paintings: self.paintings.clone(),
        }
    }
    pub fn restore(&mut self, saved: SavedUtilities) {
        self.paintings = saved
            .paintings
            .into_iter()
            .map(|mut p| {
                if p.right == IVec4::ZERO {
                    p.right = right(p.normal);
                }
                p
            })
            .filter(|p| {
                let cardinal = |v: IVec4| {
                    v.y == 0
                        && v.to_array().into_iter().all(|n| (-1..=1).contains(&n))
                        && v.abs().element_sum() == 1
                };
                p.art < ARTS.len()
                    && cardinal(p.normal)
                    && cardinal(p.right)
                    && p.right.dot(p.normal) == 0
                    && p.anchor.cmpge(IVec4::splat(-1_000_000)).all()
                    && p.anchor.cmple(IVec4::splat(1_000_000)).all()
            })
            .take(256)
            .collect();
    }

    pub fn new(world: &mut World) -> Self {
        Self {
            changes: world.subscribe_changes(),
            records: Vec::new(),
            next_serial: 0,
            paintings: Vec::new(),
            painting_check: 0.0,
        }
    }
}
fn disc(meta: u8) -> Option<item::ItemId> {
    match meta {
        1 => Some(item::RECORD_13),
        2 => Some(item::RECORD_CAT),
        _ => None,
    }
}
impl Game {
    /// Project the world-spawn bearing into the current horizontal slice. Beds
    /// change personal respawn, never the compass target.
    pub fn compass_angle(&self) -> f32 {
        let delta = self.world_spawn - self.body.pos;
        let basis = self.basis();
        delta.dot(basis.right).atan2(-delta.dot(basis.forward_h))
    }

    pub fn finish_record(&mut self, serial: u64) {
        self.utilities.records.retain(|r| r.serial != serial);
    }
    pub fn record_playback(&self) -> Option<RecordPlayback> {
        self.utilities
            .records
            .iter()
            .filter(|r| self.world.block(r.position) == Some(block::JUKEBOX))
            .min_by(|a, b| {
                a.position
                    .as_vec4()
                    .distance_squared(self.eye())
                    .total_cmp(&b.position.as_vec4().distance_squared(self.eye()))
            })
            .copied()
    }
    pub(super) fn use_utility_item(&mut self) -> bool {
        let Some(Target::Block(hit)) = self.target else {
            return false;
        };
        let Some(state) = self.world.block_state(hit.block) else {
            return false;
        };
        if self.hotbar[self.selected].is_some_and(|s| s.block == item::PAINTING)
            && (state.id != block::JUKEBOX || disc(state.metadata).is_none())
            && hit.normal.y == 0
            && hit.normal.abs().element_sum() == 1
        {
            let view_right = -self.basis().right;
            let axis = [0usize, 2, 3]
                .into_iter()
                .filter(|&axis| hit.normal[axis] == 0)
                .max_by(|&a, &b| view_right[a].abs().total_cmp(&view_right[b].abs()))
                .unwrap();
            let mut painting_right = IVec4::ZERO;
            painting_right[axis] = if view_right[axis] >= 0.0 { 1 } else { -1 };
            let fits: Vec<_> = (0..ARTS.len())
                .filter(|&art| {
                    self.painting_supported(&Painting {
                        anchor: hit.block,
                        normal: hit.normal,
                        right: painting_right,
                        art,
                    }) == Some(true)
                })
                .collect();
            if fits.is_empty() || self.utilities.paintings.len() >= 256 {
                return true;
            }
            let art = fits[((self.rand() * fits.len() as f32) as usize).min(fits.len() - 1)];
            self.utilities.paintings.push(Painting {
                anchor: hit.block,
                normal: hit.normal,
                right: painting_right,
                art,
            });
            if self.mode == GameMode::Survival {
                self.take_selected();
            }
            self.start_hand_use();
            return true;
        }
        if state.id != block::JUKEBOX {
            return false;
        }
        if let Some(record) = disc(state.metadata) {
            if self
                .world
                .set_block_state(hit.block, block::BlockState::new(block::JUKEBOX, 0))
            {
                self.utilities.records.clear();
                self.spawn_drop(
                    Slot::new(record, 1),
                    hit.block.as_vec4() + Vec4::new(0.5, 1.0, 0.5, 0.5),
                    Vec4::Y,
                    0.5,
                );
            }
            return true;
        }
        let Some(held) = self.hotbar[self.selected] else {
            return false;
        };
        let meta = match held.block {
            item::RECORD_13 => 1,
            item::RECORD_CAT => 2,
            _ => return false,
        };
        if !self
            .world
            .set_block_state(hit.block, block::BlockState::new(block::JUKEBOX, meta))
        {
            return false;
        }
        if self.mode == GameMode::Survival {
            self.take_selected();
        }
        self.utilities.next_serial = self.utilities.next_serial.wrapping_add(1);
        self.utilities.records.clear();
        self.utilities.records.push(RecordPlayback {
            position: hit.block,
            record: held.block,
            serial: self.utilities.next_serial,
            elapsed: 0.0,
        });
        self.start_hand_use();
        self.toast("Record inserted");
        true
    }
    fn painting_supported(&self, painting: &Painting) -> Option<bool> {
        let a = ARTS[painting.art];
        let w = a.width as i32 / 16;
        let h = a.height as i32 / 16;
        let r = painting.right;
        for y in -(h - 1) / 2..=h / 2 {
            for x in -(w - 1) / 2..=w / 2 {
                let p = painting.anchor + r * x + IVec4::Y * y;
                if !block::is_solid(self.world.block(p)?) {
                    return Some(false);
                }
                if block::is_solid(self.world.block(p + painting.normal)?) {
                    return Some(false);
                }
            }
        }
        Some(!self.utilities.paintings.iter().any(|other| {
            !std::ptr::eq(other, painting) && other.bounds().overlaps(painting.bounds())
        }))
    }
    pub(super) fn painting_target_distance(&self, index: usize) -> Option<f32> {
        self.utilities
            .paintings
            .get(index)?
            .bounds()
            .ray(self.eye(), self.basis().forward, REACH)
            .map(|(t, _)| t)
    }
    pub(super) fn aimed_painting(&self) -> Option<usize> {
        let eye = self.eye();
        let dir = self.basis().forward;
        let obstruction = mc4d_core::raycast::raycast_states(
            eye,
            dir,
            REACH,
            &|p| self.world.block_state(p),
            false,
        )
        .map_or(REACH, |h| h.dist);
        let obstruction = obstruction.min(match self.target {
            Some(Target::Creature(i)) => self
                .creatures
                .get(i)
                .and_then(|c| c.target_distance(eye, dir, REACH))
                .unwrap_or(REACH),
            _ => REACH,
        });
        self.utilities
            .paintings
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.bounds().ray(eye, dir, obstruction).map(|(t, _)| (i, t)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
    pub(super) fn break_aimed_painting(&mut self) -> bool {
        let Some(Target::Painting(i)) = self.target else {
            return false;
        };
        if i >= self.utilities.paintings.len() {
            return false;
        }
        let p = self.utilities.paintings.swap_remove(i);
        self.spawn_drop(Slot::new(item::PAINTING, 1), p.center(), Vec4::ZERO, 0.5);
        self.target = None;
        self.start_hand_swing();
        true
    }
    pub(super) fn painting_draws(&self) -> impl Iterator<Item = EntityDraw> + '_ {
        self.utilities.paintings.iter().map(|p| EntityDraw {
            center: p.center(),
            radius: 1.0,
            color: [1.0; 3],
            kind: 21,
            forward: p.normal.as_vec4(),
            item: 0,
            item_variant: 0,
            state: {
                let axis = (0..4).find(|&a| p.right[a] != 0).unwrap();
                p.art as u32 | ((axis as u32) << 5) | if p.right[axis] < 0 { 128 } else { 0 }
            },
        })
    }
    pub(super) fn paintings_lightning(&mut self,min:Vec4,max:Vec4){
        let area=Aabb4::new(min,max);let indices:Vec<_>=self.utilities.paintings.iter().enumerate().filter(|(_,p)|area.overlaps(p.bounds())).map(|(i,_)|i).collect();
        for i in indices.into_iter().rev(){let painting=self.utilities.paintings.swap_remove(i);self.spawn_drop(Slot::new(item::PAINTING,1),painting.center(),Vec4::Y,0.5);}
    }
    pub(super) fn paintings_blast(
        &mut self,
        center: Vec4,
        radius: u8,
        get: &impl Fn(IVec4) -> Option<block::BlockState>,
    ) {
        let affected: Vec<_> = self
            .utilities
            .paintings
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                let distance = p.center().distance(center);
                distance < f32::from(radius) * 2.0
                    && mc4d_core::explosion::damage(
                        radius,
                        distance,
                        mc4d_core::explosion::exposure(center, p.bounds(), get),
                    ) > 0
            })
            .map(|(i, _)| i)
            .collect();
        for i in affected.into_iter().rev() {
            let p = self.utilities.paintings.swap_remove(i);
            self.spawn_drop(Slot::new(item::PAINTING, 1), p.center(), Vec4::Y, 0.5);
        }
    }
    pub(super) fn update_utilities(&mut self, dt: f32) {
        self.utilities.painting_check += dt;
        if self.utilities.painting_check >= 5.0 {
            self.utilities.painting_check = 0.0;
            let remove: Vec<_> = self
                .utilities
                .paintings
                .iter()
                .enumerate()
                .filter(|(_, p)| self.painting_supported(p) == Some(false))
                .map(|(i, _)| i)
                .collect();
            for i in remove.into_iter().rev() {
                let p = self.utilities.paintings.swap_remove(i);
                self.spawn_drop(Slot::new(item::PAINTING, 1), p.center(), Vec4::ZERO, 0.5);
            }
        }

        for _ in 0..256 {
            let Ok(change) = self.utilities.changes.try_recv() else {
                break;
            };
            if change.before.id == block::JUKEBOX && change.after.id != block::JUKEBOX {
                if let Some(record) = disc(change.before.metadata) {
                    self.spawn_drop(
                        Slot::new(record, 1),
                        change.position.as_vec4() + Vec4::splat(0.5),
                        Vec4::Y,
                        0.5,
                    );
                }
                self.utilities.records.clear();
            }
        }
        for record in &mut self.utilities.records {
            record.elapsed += dt;
        }
        self.utilities.records.retain(|r| {
            self.world
                .block(r.position)
                .is_none_or(|id| id == block::JUKEBOX)
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_insert_eject_and_break_without_duplication() {
        let mut game = Game::new(None, 7, GameMode::Survival);
        let p = game.body.pos.floor().as_ivec4() + IVec4::Z * 2;
        game.world.set_block(p, block::JUKEBOX);
        game.target = Some(Target::Block(Hit {
            block: p,
            normal: -IVec4::Z,
            dist: 2.0,
        }));
        game.hotbar[0] = Some(Slot::new(item::RECORD_CAT, 1));
        game.selected = 0;
        assert!(game.use_utility_item());
        assert!(game.hotbar[0].is_none());
        assert_eq!(game.world.block_state(p).unwrap().metadata, 2);
        assert!(game.record_playback().is_some());
        assert!(game.use_utility_item());
        game.update_utilities(0.05);
        assert_eq!(game.dropped_items.len(), 1);
        assert!(game.record_playback().is_none());
        game.hotbar[0] = Some(Slot::new(item::RECORD_13, 1));
        assert!(game.use_utility_item());
        game.world.set_block(p, AIR);
        game.update_utilities(0.05);
        game.update_utilities(0.05);
        assert_eq!(
            game.dropped_items
                .iter()
                .map(|d| d.stack.count)
                .sum::<u32>(),
            2
        );
    }
    #[test]
    fn paintings_choose_fitting_art_persist_and_drop_once_when_support_breaks() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-paintings")
            .join(format!("{unique}"));
        let mut game = Game::new(Some(root.clone()), 7, GameMode::Survival);
        let anchor = game.body.pos.floor().as_ivec4() + IVec4::Z * 3 + IVec4::Y;
        for x in -2..=2 {
            for y in -1..=3 {
                let p = anchor + IVec4::X * x + IVec4::Y * y;
                game.world.set_block(p, block::STONE);
                game.world.set_block(p - IVec4::Z, AIR);
            }
        }
        game.hotbar[0] = Some(Slot::new(item::PAINTING, 1));
        game.selected = 0;
        game.target = Some(Target::Block(Hit {
            block: anchor,
            normal: -IVec4::Z,
            dist: 3.0,
        }));
        assert!(game.use_utility_item());
        assert_eq!(game.utilities.paintings.len(), 1);
        assert!(game.hotbar[0].is_none());
        let bounds = game.utilities.paintings[0].bounds();
        assert!(
            bounds
                .ray(bounds.min - Vec4::Z * 2.0 + Vec4::splat(0.01), Vec4::Z, 6.0)
                .is_some()
        );
        let art = game.utilities.paintings[0].art;
        game.save_checked().unwrap();
        drop(game);
        let mut loaded = Game::try_load(Some(root.clone()), 7, GameMode::Survival).unwrap();
        assert_eq!(loaded.utilities.paintings[0].art, art);
        loaded.world.set_block(anchor, AIR);
        loaded.update_utilities(5.1);
        loaded.update_utilities(5.1);
        assert!(loaded.utilities.paintings.is_empty());
        assert_eq!(
            loaded
                .dropped_items
                .iter()
                .filter(|d| d.stack.block == item::PAINTING)
                .map(|d| d.stack.count)
                .sum::<u32>(),
            1
        );
        drop(loaded);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn jukebox_uses_one_original_stream_and_does_not_consume_the_other_record() {
        let mut game = Game::new(None, 7, GameMode::Survival);
        let p = game.body.pos.floor().as_ivec4() + IVec4::Z * 2;
        for (offset, disc) in [(IVec4::ZERO, item::RECORD_13), (IVec4::X, item::RECORD_CAT)] {
            game.world.set_block(p + offset, block::JUKEBOX);
            game.hotbar[0] = Some(Slot::new(disc, 1));
            game.selected = 0;
            game.target = Some(Target::Block(Hit {
                block: p + offset,
                normal: -IVec4::Z,
                dist: 2.0,
            }));
            assert!(game.use_utility_item());
        }
        assert_eq!(game.utilities.records.len(), 1);
        assert_eq!(game.record_playback().unwrap().record, item::RECORD_CAT);
        assert_eq!(game.world.block_state(p).unwrap().metadata, 1);
        game.target = Some(Target::Block(Hit {
            block: p,
            normal: -IVec4::Z,
            dist: 2.0,
        }));
        assert!(game.use_utility_item());
        assert!(game.record_playback().is_none());
        assert_eq!(game.world.block_state(p + IVec4::X).unwrap().metadata, 2);
    }

    #[test]
    fn blast_destroys_a_visible_painting_and_drops_one_item() {
        let mut game = Game::new(None, 7, GameMode::Creative);
        let anchor = game.body.pos.floor().as_ivec4() + IVec4::Z * 3 + IVec4::Y;
        game.world.set_block(anchor, block::STONE);
        game.world.set_block(anchor - IVec4::Z, AIR);
        let painting = Painting {
            anchor,
            normal: -IVec4::Z,
            right: -IVec4::X,
            art: 0,
        };
        let center = painting.center() - Vec4::Z * 2.0;
        game.utilities.paintings.push(painting);
        game.explode(center);
        game.update_utilities(6.0);
        assert!(game.utilities.paintings.is_empty());
        assert_eq!(
            game.dropped_items
                .iter()
                .filter(|d| d.stack.block == item::PAINTING)
                .map(|d| d.stack.count)
                .sum::<u32>(),
            1
        );
    }
    #[test]
    fn compass_tracks_world_spawn_in_w_and_ignores_bed_spawn() {
        let mut game = Game::menu_preview(7);
        game.world_spawn = Vec4::new(0.0, 64.0, 0.0, 10.0);
        game.body.pos = Vec4::new(0.0, 64.0, 0.0, 0.0);
        game.orient.rotate_forward_ana(std::f32::consts::FRAC_PI_2);
        let before = game.compass_angle();
        game.spawn = Vec4::splat(200.0);
        assert_eq!(game.compass_angle(), before);
        assert!(before.is_finite());
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::{
        frame::{FrameInput, FrameRenderer},
        input::{Bindings, InputState},
        settings::Settings,
        ui::{Screen, UiState},
    };
    #[test]
    #[ignore = "requires GPU; verifies paintings on Z/W walls and actual animated HUD instrument textures"]
    fn paintings_and_instruments_render_on_real_game_state() {
        let size = (854, 480);
        let gpu = crate::headless_gpu(size).unwrap();
        let mut frame =
            FrameRenderer::new(&gpu.device, gpu.target.format(), size, gpu.adapter_name);
        let mut game = Game::new(None, 7, GameMode::Creative);
        let mut settings = Settings {
            render_distance: 2,
            inset: false,
            ..Default::default()
        };
        let mut bindings = Bindings::default();
        let input = InputState::default();
        let mut ui = UiState {
            screen: Screen::Playing,
            show_debug: false,
            ..Default::default()
        };
        game.hotbar[0] = Some(Slot::new(item::COMPASS, 1));
        game.hotbar[1] = Some(Slot::new(item::CLOCK, 1));
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
        std::fs::create_dir_all(&root).unwrap();
        let mut render = |game: &mut Game| {
            game.update_streaming(&settings, false);
            frame.render(
                &gpu.device,
                &gpu.queue,
                &gpu.target,
                FrameInput {
                    game,
                    settings: &mut settings,
                    bindings: &mut bindings,
                    input: &input,
                    ui: &mut ui,
                    raw_input: crate::raw_input(size),
                    size,
                    pixels_per_point: 1.0,
                },
            );
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            frame.world_ready(game)
        };
        for (name, normal, angle) in [
            ("z", -IVec4::Z, 0.0),
            ("w", -IVec4::W, std::f32::consts::FRAC_PI_2),
        ] {
            game.orient = mc4d_core::camera::Orientation::default();
            game.orient.rotate_forward_ana(angle);
            let anchor = game.body.pos.floor().as_ivec4() - normal * 3 + IVec4::Y;
            for x in -2..=2 {
                for y in -1..=2 {
                    let p = anchor + IVec4::X * x + IVec4::Y * y;
                    game.world.set_block(p, block::STONE);
                    game.world.set_block(p + normal, AIR);
                }
            }
            let start = std::time::Instant::now();
            while !render(&mut game) {
                assert!(
                    start.elapsed().as_secs() < 45,
                    "fixture lighting must settle"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            render(&mut game);
            let empty = crate::frame::read_texture(&gpu.device, &gpu.queue, &gpu.target);
            game.utilities.paintings.push(Painting {
                anchor,
                normal,
                right: -IVec4::X,
                art: 0,
            });
            render(&mut game);
            let painted = crate::frame::read_texture(&gpu.device, &gpu.queue, &gpu.target);
            let changed = painted
                .chunks_exact(4)
                .zip(empty.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                changed > 1000,
                "painting must display actual artwork on{name} wall: {changed}"
            );
            crate::frame::write_png(
                &root.join(format!("painting-{name}-instruments-day.png")),
                size.0,
                size.1,
                &painted,
            )
            .unwrap();
            game.utilities.paintings.clear();
        }
        render(&mut game);
        let day = crate::frame::read_texture(&gpu.device, &gpu.queue, &gpu.target);
        game.environment.world_ticks = 18000;
        game.world_spawn = game.body.pos - Vec4::X * 20.0;
        for i in 1..=100 {
            game.time = i as f32 * 0.05;
            render(&mut game);
        }
        let night = crate::frame::read_texture(&gpu.device, &gpu.queue, &gpu.target);
        let red_pixels = |image: &[u8]| {
            let mut points = Vec::new();
            for y in 442..474 {
                for x in 251..283 {
                    let p = &image[(y * 854 + x) * 4..][..4];
                    if p[0] > 180 && p[1] < 80 && p[2] < 80 {
                        points.push((x, y));
                    }
                }
            }
            points
        };
        assert_ne!(
            red_pixels(&day),
            red_pixels(&night),
            "the rendered GUI compass needle must move"
        );
        let atlas = mc4d_render::atlas::load(None).unwrap();
        let mut changed_dial = 0;
        for y in 0..16 {
            for x in 0..16 {
                let p = &atlas[((256 * 256) + (64 + y) * 256 + 96 + x) * 4..][..4];
                if p[0] == p[2] && p[1] == 0 && p[2] > 0 {
                    let at = ((442 + y * 2 + 1) * 854 + 291 + x * 2 + 1) * 4;
                    if day[at..at + 3] != night[at..at + 3] {
                        changed_dial += 1;
                    }
                }
            }
        }
        assert!(
            changed_dial > 10,
            "actual GUI clock dial must follow world time: {changed_dial}"
        );
        crate::frame::write_png(&root.join("instruments-night.png"), size.0, size.1, &night)
            .unwrap();
    }
}
