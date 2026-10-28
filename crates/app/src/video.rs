//! Scripted introductory video: a camera tour with captions, rendered headlessly and piped
//! to ffmpeg (`--video out.mp4`), or one still per shot for previewing (`--video-stills DIR`).

use crate::app::anyhow_lite::Result;
use crate::frame::{FrameInput, FrameRenderer, read_texture, write_png};
use crate::game::{Game, GameMode};
use crate::input::{Bindings, InputState};
use crate::settings::Settings;
use crate::ui::UiState;
use crate::{HeadlessGpu, headless_gpu};
use mc4d_core::block::{AIR, BRICK, GLOW};
use mc4d_core::camera::Orientation;
use mc4d_core::chunk::chunk_of;
use mc4d_core::glam::{IVec4, Vec4};
use mc4d_core::physics::EYE_HEIGHT;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct VideoArgs {
    pub out: Option<PathBuf>,
    pub stills: Option<PathBuf>,
    pub ffmpeg: String,
    pub size: (u32, u32),
    pub fps: u32,
    pub seed: u32,
}

/// Camera pose: eye position and orientation angles in degrees.
#[derive(Clone, Copy)]
struct Pose {
    eye: Vec4,
    yaw: f32,
    pitch: f32,
    ana: f32,
    right_ana: f32,
}

/// Fixed points of the tour, found once from the generated world.
struct Places {
    /// A high viewpoint over the terrain.
    overlook: Vec4,
    /// Integer center of the hollow tesseract built for the demo.
    tesseract: IVec4,
    /// Eye position on the ground for the creature shot.
    meadow: Vec4,
}

struct Shot {
    secs: f32,
    title: &'static str,
    sub: &'static str,
    /// Keycaps shown on screen (default bindings).
    keys: &'static [&'static str],
    /// Which keycaps are down at raw progress `t` (bit i = keys[i]).
    pressed: fn(f32) -> u32,
    /// Fraction of the shot to wait (e.g. for a key tap) before the camera starts moving.
    delay: f32,
    /// Pose at eased progress `t` in 0..=1.
    pose: fn(f32, &Places) -> Pose,
}

const HALF_TESSERACT: i32 = 2;

/// Viewpoint above and to the side of the tesseract, so its hollowness is visible.
const TESS_VIEW: Vec4 = Vec4::new(-5.0, 5.0, -8.0, 0.0);
const TESS_YAW: f32 = 32.0;
const TESS_PITCH: f32 = -27.0;

fn tess_eye(p: &Places, w_offset: f32) -> Vec4 {
    p.tesseract.as_vec4() + Vec4::splat(0.5) + TESS_VIEW + Vec4::W * w_offset
}

/// Held for the middle of the shot.
fn held(t: f32) -> bool {
    (0.04..0.96).contains(&t)
}

/// A short tap at the start of the shot.
fn tap(t: f32) -> bool {
    (0.03..0.11).contains(&t)
}

/// Repeated wheel clicks.
fn clicking(t: f32) -> bool {
    held(t) && (t * 22.0).fract() < 0.45
}

const SHOT_GHOSTS: usize = 7;
const SHOT_CREATURES: usize = 8;

const SHOTS: &[Shot] = &[
    Shot {
        secs: 4.0,
        title: "mc_4d",
        sub: "Minecraft, but the world has four spatial dimensions",
        keys: &["Mouse ←→"],
        pressed: |t| held(t) as u32,
        delay: 0.0,
        pose: |t, p| Pose { eye: p.overlook, yaw: -20.0 + 25.0 * t, pitch: -20.0, ana: 0.0, right_ana: 0.0 },
    },
    Shot {
        secs: 4.5,
        title: "You see a 3D slice of a 4D world",
        sub: "Walk around inside it just like in Minecraft",
        keys: &["W", "A", "S", "D"],
        pressed: |t| held(t) as u32,
        delay: 0.0,
        pose: |t, p| Pose { eye: p.overlook + Vec4::new(0.6, 0.0, 7.0, 0.0) * t, yaw: 5.0, pitch: -20.0, ana: 0.0, right_ana: 0.0 },
    },
    Shot {
        secs: 5.0,
        title: "Step into the 4th dimension",
        sub: "Q / E move along W: you are now looking at a neighbouring slice",
        keys: &["Q", "E"],
        pressed: |t| (held(t) as u32) << 1,
        delay: 0.0,
        pose: |t, p| Pose { eye: p.overlook + Vec4::new(0.6, 0.0, 7.0, 14.0 * t), yaw: 5.0, pitch: -20.0, ana: 0.0, right_ana: 0.0 },
    },
    Shot {
        secs: 5.5,
        title: "Turn your view toward W",
        sub: "Mouse wheel: the cubes become cross-sections of tesseracts",
        keys: &["Wheel ▲", "Wheel ▼"],
        pressed: |t| clicking(t) as u32,
        delay: 0.0,
        pose: |t, p| Pose { eye: p.overlook + Vec4::new(0.6, 0.0, 7.0, 14.0), yaw: 5.0, pitch: -20.0, ana: 55.0 * t, right_ana: 0.0 },
    },
    Shot {
        secs: 3.5,
        title: "Snap back to the axes",
        sub: "R straightens the slice and keeps the direction you face",
        keys: &["R"],
        pressed: |t| tap(t) as u32,
        delay: 0.12,
        pose: |t, p| Pose {
            eye: p.overlook + Vec4::new(0.6, 0.0, 7.0, 14.0),
            yaw: 5.0,
            pitch: -20.0,
            ana: 55.0 * (1.0 - t),
            right_ana: 0.0,
        },
    },
    Shot {
        secs: 7.0,
        title: "A hollow tesseract, built from blocks",
        sub: "Moving through it along W: nothing → solid face → hollow cube → solid face → nothing",
        keys: &["Q", "E"],
        pressed: |t| (held(t) as u32) << 1,
        delay: 0.0,
        pose: |t, p| Pose { eye: tess_eye(p, -3.4 + 6.8 * t), yaw: TESS_YAW, pitch: TESS_PITCH, ana: 0.0, right_ana: 0.0 },
    },
    Shot {
        secs: 6.0,
        title: "Rotate the slice",
        sub: "Hold Alt and move the mouse: the same tesseract, cut at an angle",
        keys: &["Alt", "+", "Mouse ←→"],
        pressed: |t| if held(t) { 0b101 } else { 0 },
        delay: 0.0,
        pose: |t, p| {
            let a = (t * std::f32::consts::PI).sin();
            Pose { eye: tess_eye(p, 0.0), yaw: TESS_YAW, pitch: TESS_PITCH, ana: 0.0, right_ana: 50.0 * a }
        },
    },
    Shot {
        secs: 5.0,
        title: "Ghost layers",
        sub: "G: red marks blocks just ana of your slice, blue just kata",
        keys: &["G", "W"],
        pressed: |t| tap(t) as u32 | ((t > 0.15 && t < 0.96) as u32) << 1,
        delay: 0.15,
        pose: |t, p| Pose { eye: p.overlook + Vec4::new(0.0, 0.0, 4.0 * t, 0.0), yaw: 10.0, pitch: -24.0, ana: 0.0, right_ana: 0.0 },
    },
    Shot {
        secs: 6.0,
        title: "Hyperspherical creatures",
        sub: "They shrink as they drift out of your slice; 4D glasses (B) show their outlines",
        keys: &["B", "Mouse ←→"],
        pressed: |t| tap(t) as u32 | ((t > 0.15 && t < 0.96) as u32) << 1,
        delay: 0.15,
        pose: |t, p| Pose { eye: p.meadow, yaw: -12.0 + 24.0 * t, pitch: -20.0, ana: 0.0, right_ana: 0.0 },
    },
    Shot {
        secs: 4.5,
        title: "cargo run --release",
        sub: "A tutorial starts on first launch · every key can be rebound (Esc → Controls)",
        keys: &[],
        pressed: |_| 0,
        delay: 0.0,
        pose: |t, p| Pose { eye: p.overlook + Vec4::new(0.0, 3.0 * t, 0.0, 0.0), yaw: 20.0 + 30.0 * t, pitch: -22.0, ana: 0.0, right_ana: 0.0 },
    },
];

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn apply_pose(game: &mut Game, pose: &Pose) {
    let mut o = Orientation::default();
    o.yaw(pose.yaw.to_radians());
    o.add_pitch(pose.pitch.to_radians());
    o.rotate_forward_ana(pose.ana.to_radians());
    o.rotate_right_ana(pose.right_ana.to_radians());
    game.orient = o;
    game.body.pos = pose.eye - Vec4::Y * EYE_HEIGHT;
    game.body.vel = Vec4::ZERO;
}

/// Build a 5⁴ hollow tesseract: its 3D cells are bricks, its lower-dimensional faces glow.
fn build_tesseract(game: &mut Game, center: IVec4) {
    let r = HALF_TESSERACT;
    for w in -r - 1..=r + 1 {
        for z in -r - 1..=r + 1 {
            for y in -r - 1..=r + 1 {
                for x in -r - 1..=r + 1 {
                    let p = center + IVec4::new(x, y, z, w);
                    game.world.load_now(chunk_of(p), None);
                    let d = IVec4::new(x, y, z, w).abs();
                    let on_boundary = d.cmpeq(IVec4::splat(r)).bitmask().count_ones();
                    let block = if d.max_element() > r {
                        AIR
                    } else {
                        match on_boundary {
                            0 => AIR,
                            1 => BRICK,
                            _ => GLOW,
                        }
                    };
                    game.world.set_block(p, block);
                }
            }
        }
    }
}

fn find_places(game: &mut Game) -> Places {
    let spawn = game.body.pos;
    let terrain = game.world.terrain.clone();
    let ground = |x: f32, z: f32, w: f32| terrain.height(x.floor() as i32, z.floor() as i32, w.floor() as i32) as f32;
    let overlook = Vec4::new(spawn.x, ground(spawn.x, spawn.z, spawn.w).max(44.0) + 11.0, spawn.z - 6.0, spawn.w);
    let (tx, tz, tw) = (spawn.x + 2.0, spawn.z + 14.0, spawn.w + 40.0);
    let t = IVec4::new(tx as i32, ground(tx, tz, tw) as i32 + 12, tz as i32, tw as i32);
    let (mx, mz) = find_clearing(game, spawn);
    let meadow = Vec4::new(mx + 0.5, ground(mx, mz, spawn.w) + 1.0 + EYE_HEIGHT + 1.2, mz + 0.5, spawn.w);
    Places { overlook, tesseract: t, meadow }
}

/// A dry-land spot for the creature shot; trees in view ahead (+Z) are removed.
fn find_clearing(game: &mut Game, spawn: Vec4) -> (f32, f32) {
    let w = spawn.w.floor() as i32;
    let (mut x, mut z) = (spawn.x.floor() as i32 + 30, spawn.z.floor() as i32 - 20);
    for i in 0..40 {
        let (cx, cz) = (x + 9 * (i % 6), z + 9 * (i / 6));
        if game.world.terrain.height(cx, cz, w) > mc4d_core::terrain::SEA_LEVEL + 2 {
            (x, z) = (cx, cz);
            break;
        }
    }
    let h = game.world.terrain.height(x, z, w);
    for dw in -3..=3 {
        for dz in -4..=16 {
            for dx in -10..=10 {
                for dy in 0..=12 {
                    let p = IVec4::new(x + dx, h + dy, z + dz, w + dw);
                    game.world.load_now(chunk_of(p), None);
                    if matches!(game.world.block(p), Some(mc4d_core::block::LEAVES | mc4d_core::block::LOG)) {
                        game.world.set_block(p, AIR);
                    }
                }
            }
        }
    }
    (x as f32, z as f32)
}

fn spawn_meadow_creatures(game: &mut Game, p: &Places) {
    game.creatures.clear();
    let terrain = game.world.terrain.clone();
    // (sideways offset, distance ahead, offset along W, radius, color)
    let specs: [(f32, f32, f32, f32, [f32; 3]); 7] = [
        (-2.5, 6.0, 0.0, 0.6, [0.95, 0.55, 0.65]),
        (1.5, 7.5, 0.0, 0.55, [0.55, 0.85, 0.95]),
        (3.5, 5.5, 0.35, 0.6, [0.95, 0.85, 0.45]),
        (-0.5, 9.0, 0.5, 0.65, [0.6, 0.95, 0.6]),
        (-4.5, 8.5, 1.6, 0.6, [0.8, 0.6, 0.95]),
        (5.0, 9.5, -2.0, 0.6, [0.95, 0.6, 0.4]),
        (0.5, 4.5, -0.25, 0.5, [0.7, 0.7, 0.95]),
    ];
    for (side, ahead, w, r, color) in specs {
        let x = p.meadow.x + side;
        let z = p.meadow.z + ahead;
        let wp = p.meadow.w + w;
        let h = terrain.height(x.floor() as i32, z.floor() as i32, wp.floor() as i32) as f32 + 1.0;
        game.spawn_creature_at(Vec4::new(x, h, z, wp), r, color);
    }
}

pub fn run(args: &VideoArgs) -> Result<()> {
    let size = args.size;
    let HeadlessGpu { device, queue, target, adapter_name } = headless_gpu(size)?;
    let mut fr = FrameRenderer::new(&device, target.format(), size, adapter_name);
    let mut game = Game::new(None, args.seed, GameMode::Creative);
    game.flying = true;
    let places = find_places(&mut game);
    build_tesseract(&mut game, places.tesseract);

    let mut settings = Settings { show_hints: false, render_distance: 6, ..Settings::default() };
    let mut bindings = Bindings::default();
    let input = InputState::default();
    let mut ui = UiState::default();
    let dt = 1.0 / args.fps as f32;

    let mut ffmpeg = match &args.out {
        Some(out) => {
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let child = Command::new(&args.ffmpeg)
                .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba"])
                .args(["-s", &format!("{}x{}", size.0, size.1), "-r", &args.fps.to_string(), "-i", "-"])
                .args(["-c:v", "libx264", "-preset", "slow", "-crf", "20", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
                .arg(out)
                .stdin(Stdio::piped())
                .spawn()
                .map_err(|e| format!("could not start ffmpeg ({}): {e}", args.ffmpeg))?;
            Some(child)
        }
        None => None,
    };

    let mut render = |game: &mut Game, settings: &mut Settings, ui: &mut UiState| {
        game.target = None;
        fr.render(
            &device,
            &queue,
            &target,
            FrameInput {
                game,
                settings,
                bindings: &mut bindings,
                input: &input,
                ui,
                raw_input: crate::raw_input(size),
                size,
                pixels_per_point: 1.0,
            },
        );
    };

    let mut frames = 0u32;
    for (i, shot) in SHOTS.iter().enumerate() {
        let creatures = i == SHOT_CREATURES;
        if creatures {
            spawn_meadow_creatures(&mut game, &places);
        } else {
            game.creatures.clear();
        }
        // Toggles start off and switch on when their key is tapped.
        settings.ghosts = false;
        settings.glasses = !creatures;
        // Warm up: stream in chunks for the start of the shot and let uploads finish.
        apply_pose(&mut game, &(shot.pose)(0.0, &places));
        settle(&mut game, &settings);
        for _ in 0..40 {
            render(&mut game, &mut settings, &mut ui);
        }

        let n = (shot.secs * args.fps as f32).round() as u32;
        let range: Vec<u32> = if args.stills.is_some() { vec![n / 2] } else { (0..n).collect() };
        for f in range {
            let t = f as f32 / (n - 1).max(1) as f32;
            let motion = smooth((t - shot.delay) / (1.0 - shot.delay));
            let pose = (shot.pose)(motion, &places);
            if i == SHOT_GHOSTS {
                settings.ghosts = t > 0.08;
            }
            if creatures {
                settings.glasses = t > 0.08;
            }
            apply_pose(&mut game, &pose);
            if creatures {
                // Let the creatures hop; the camera stays put.
                game.update(dt, None, &settings);
                apply_pose(&mut game, &pose);
            }
            game.update_streaming(&settings, false);
            settle(&mut game, &settings);
            let secs = f as f32 * dt;
            let alpha = (secs / 0.35).min((shot.secs - secs) / 0.35).clamp(0.0, 1.0);
            ui.caption = Some((shot.title.into(), shot.sub.into(), alpha));
            let down = (shot.pressed)(t);
            ui.keys = shot.keys.iter().enumerate().map(|(k, label)| (label.to_string(), down & (1 << k) != 0)).collect();
            render(&mut game, &mut settings, &mut ui);

            if let Some(dir) = &args.stills {
                render(&mut game, &mut settings, &mut ui); // uploads triggered by the first render
                let rgba = read_texture(&device, &queue, &target);
                let path = dir.join(format!("shot{:02}.png", i + 1));
                write_png(&path, size.0, size.1, &rgba)?;
                println!("wrote {}", path.display());
            }
            if let Some(child) = &mut ffmpeg {
                let rgba = read_texture(&device, &queue, &target);
                child.stdin.as_mut().unwrap().write_all(&rgba)?;
                frames += 1;
            }
        }
        eprintln!("shot {}/{} done: {}", i + 1, SHOTS.len(), shot.title);
    }

    if let Some(mut child) = ffmpeg {
        drop(child.stdin.take());
        let status = child.wait()?;
        if !status.success() {
            return Err(format!("ffmpeg failed: {status}").into());
        }
        let out = args.out.as_deref().unwrap_or(Path::new("?"));
        println!("wrote {} ({frames} frames, {:.1}s)", out.display(), frames as f32 / args.fps as f32);
    }
    Ok(())
}

/// Block until the chunks around the current pose are generated (bounded wait).
fn settle(game: &mut Game, settings: &Settings) {
    let start = std::time::Instant::now();
    loop {
        game.update_streaming(settings, false);
        let ready = game.world.pending_count() == 0 && game.gpu_wanted.iter().all(|c| game.world.is_loaded(*c));
        if ready || start.elapsed().as_secs() > 20 {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
}
