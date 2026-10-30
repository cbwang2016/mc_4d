//! Opt-in windowed performance route, isolated from normal worlds by the launcher.
use crate::game::{Game, GameMode};
use crate::input::{Action, Bindings, InputState};
use crate::settings::Settings;
use mc4d_core::{block, glam::IVec4};
use serde::Serialize;
use std::{path::PathBuf, time::Instant};

/// A fresh directory for every invocation; never reuse the named playable world.
pub fn scratch_directory(base: &std::path::Path) -> std::io::Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let root = base.join(format!("performance-run-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(base)?;
    std::fs::create_dir(&root)?;
    Ok(root)
}

/// User target (2026-10-31): at least 20 FPS in every scene on the reference
/// iGPU, at native resolution up to 1920x1200, with the GPU cooled down.
pub const MINIMUM_FPS: f64 = 20.0;
pub const FRAME_BUDGET_MS: f64 = 1000.0 / MINIMUM_FPS;
/// Share of gameplay frames allowed above the budget.
pub const SLOW_FRAME_ALLOWANCE: f64 = 0.01;

pub const PHASES: [&str; 9] = [
    "loading_idle",
    "w_flight",
    "continuous_rotation",
    "four_views",
    "six_views",
    "nine_views",
    "ghosts",
    "fluid_and_edits",
    "save",
];

pub struct Config {
    pub report: PathBuf,
    pub duration: f32,
    pub size: (u32, u32),
}

#[derive(Serialize)]
struct Sample {
    phase: String,
    elapsed_seconds: f64,
    interval_ms: f64,
    cpu_stages_ms: [f32; 4],
    streaming_stages_ms: [f32;5],
    update_stages_ms:[f32;11],
    chunks_pending: usize,
    requested_unloaded: usize,
    missing_gpu_dense: usize,
    lighting_pending: usize,
    lighting_queues: [usize; 4],
    missing_gpu_light: usize,
    unsettled_visible_light: usize,
    presented: bool,
    world_visible: bool,
    position: [f32; 4],
    render_scale: f32,
    /// Window focused and not occluded; compositor-paced intervals of a
    /// hidden window are reported separately, never silently dropped.
    window_active: bool,
    /// Ray-casting pass GPU time from timestamp queries (0 when unavailable).
    gpu_ms: f32,
}

struct Pending {
    phase: usize,
    elapsed: f64,
    cpu: [f32; 4],
    streaming: [f32;5],
    update:[f32;11],
    pending: usize,
    unloaded: usize,
    missing: usize,
    lighting_pending: usize,
    lighting_queues: [usize; 4],
    missing_light: usize,
    unsettled_light: usize,
    presented: bool,
    world_visible: bool,
    position: [f32; 4],
    render_scale: f32,
    /// Window focused and not occluded; compositor-paced intervals of a
    /// hidden window are reported separately, never silently dropped.
    window_active: bool,
    /// Ray-casting pass GPU time from timestamp queries (0 when unavailable).
    gpu_ms: f32,
}

#[derive(Serialize)]
struct Summary {
    phase: String,
    frames: usize,
    visible_frames: usize,
    mean_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    worst_ms: f64,
    minimum_fps: f64,
    frames_over_budget: usize,
    incomplete_frames: usize,
    presentation_failures: usize,
    /// Internal render scale (1.0 unless the player lowered it).
    minimum_render_scale: f32,
    mean_render_scale: f32,
    mean_gpu_ms: f32,
    worst_gpu_ms: f32,
    /// Frames while the window was unfocused or occluded (excluded from
    /// frames_over_budget) and how many of those exceeded the budget.
    inactive_frames: usize,
    inactive_over_budget: usize,
}

#[derive(Serialize)]
struct Report<'a> {
    streaming_stage_labels: [&'static str;5],
    update_stage_labels: [&'static str;11],
    scope: &'static str,
    lighting_queue_labels: [&'static str; 4],
    minimum_fps_target: f64,
    frame_budget_ms: f64,
    completed: bool,
    adapter: &'a str,
    size: [u32; 2],
    seed: i64,
    generation_workers: usize,
    requested_duration_seconds: f32,
    startup_ms: f64,
    loading_ready_seconds: Option<f64>,
    observed_duration_seconds: f64,
    edits_applied: usize,
    saves_triggered: usize,
    all_phases_sampled: bool,
    meets_frame_budget: bool,
    all_frames_resident: bool,
    final_settings: &'a Settings,
    phases: Vec<Summary>,
    samples: &'a [Sample],
}

pub struct GameplayPerf {
    pub config: Config,
    origin: Option<Instant>,
    startup_ms: f64,
    loading_ready: Option<f64>,
    phase: usize,
    pending: Option<Pending>,
    samples: Vec<Sample>,
    last_edit: f64,
    edits: usize,
    saves: usize,
    written: bool,
    passed: bool,
}

impl GameplayPerf {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            origin: None,
            startup_ms: 0.0,
            loading_ready: None,
            phase: 0,
            pending: None,
            samples: Vec::new(),
            last_edit: -1.0,
            edits: 0,
            saves: 0,
            written: false,
            passed: false,
        }
    }

    pub fn result(&self) -> Result<(), String> {
        if !self.written {
            return Err("gameplay benchmark did not produce a report".into());
        }
        if !self.passed {
            return Err(format!(
                "gameplay benchmark incomplete or exceeded the {:.2} ms / {:.0} FPS frame budget; see {}",
                FRAME_BUDGET_MS,
                MINIMUM_FPS,
                self.config.report.display()
            ));
        }
        Ok(())
    }

    /// Called at redraw entry. The interval belongs to the previous frame and
    /// includes its update, acquire, rendering, presentation and event-loop wait.
    pub fn begin(&mut self, now: Instant, dt: f32, startup_ms: f64) -> bool {
        if self.origin.is_none() {
            self.origin = Some(now);
            self.startup_ms = startup_ms;
        }
        if let Some(p) = self.pending.take() {
            self.samples.push(Sample {
                phase: PHASES[p.phase].into(),
                elapsed_seconds: p.elapsed,
                interval_ms: dt as f64 * 1000.0,
                cpu_stages_ms: p.cpu,
                streaming_stages_ms:p.streaming,
                update_stages_ms:p.update,
                chunks_pending: p.pending,
                requested_unloaded: p.unloaded,
                missing_gpu_dense: p.missing,
                lighting_pending: p.lighting_pending,
                lighting_queues: p.lighting_queues,
                missing_gpu_light: p.missing_light,
                unsettled_visible_light: p.unsettled_light,
                presented: p.presented,
                world_visible: p.world_visible,
                position: p.position,
                render_scale: p.render_scale,
                window_active: p.window_active,
                gpu_ms: p.gpu_ms,
            });
        }
        let elapsed = now.duration_since(self.origin.unwrap()).as_secs_f64();
        let segment = self.config.duration as f64 / PHASES.len() as f64;
        if let Some(ready) = self.loading_ready {
            // Every route phase keeps its requested duration. Slow loading
            // extends the total run instead of skipping later route phases.
            let loading_end = ready.max(segment);
            if elapsed >= loading_end + segment * (PHASES.len() - 1) as f64 {
                return true;
            }
            self.phase = if elapsed < loading_end {
                0
            } else {
                1 + ((elapsed - loading_end) / segment) as usize
            };
            self.phase = self.phase.min(PHASES.len() - 1);
        } else {
            self.phase = 0;
            if elapsed > self.config.duration.max(60.0) as f64 * 2.0 {
                return true;
            }
        }
        self.pending = Some(Pending {
            phase: self.phase,
            elapsed,
            cpu: [0.0; 4],
            streaming:[0.0;5],
            update:[0.0;11],
            pending: 0,
            unloaded: 0,
            missing: 0,
            lighting_pending: 0,
            lighting_queues: [0; 4],
            missing_light: 0,
            unsettled_light: 0,
            presented: false,
            world_visible: false,
            position: [0.0; 4],
            render_scale: 1.0,
            window_active: true,
            gpu_ms: 0.0,
        });
        false
    }

    /// Called only after the normal window loader has passed full readiness.
    pub fn loading_completed(&mut self, now: Instant) {
        if self.loading_ready.is_none() {
            self.loading_ready = Some(
                self.origin
                    .map_or(0.0, |origin| now.duration_since(origin).as_secs_f64()),
            );
        }
    }

    pub fn drive(
        &mut self,
        dt: f32,
        game: &mut Game,
        settings: &mut Settings,
        input: &mut InputState,
        bindings: &Bindings,
    ) {
        input.release_all();
        input.end_frame();
        game.mode = GameMode::Creative;
        game.flying = true;
        settings.inset = true;
        settings.view_count = match self.phase {
            3 => 4,
            4 => 6,
            5 => 9,
            _ => 2,
        };
        settings.ghosts = self.phase == 6;
        if self.phase == 1 {
            for action in [Action::MoveAna, Action::Sprint] {
                input.press(bindings.get(action)[0]);
            }
        }
        if (2..=6).contains(&self.phase) {
            game.orient.rotate_forward_ana(dt.min(0.1) * 0.45);
            game.orient.rotate_right_ana(dt.min(0.1) * 0.23);
        }
        let elapsed = self.pending.as_ref().map_or(0.0, |p| p.elapsed);
        if self.phase == 7 && elapsed - self.last_edit >= 0.2 {
            self.last_edit = elapsed;
            let base = (game.body.pos + game.basis().forward_h * 4.0)
                .floor()
                .as_ivec4()
                + IVec4::Y * 2;
            for i in 0..12 {
                let p = base + IVec4::new(i % 4, 0, i / 4, (self.edits % 3) as i32 - 1);
                let id = match i % 4 {
                    0 => block::WATER,
                    1 => block::LAVA,
                    2 => block::BRICK,
                    _ => block::AIR,
                };
                self.edits += game.world.set_block(p, id) as usize;
            }
        }
        if self.phase == 8 && self.saves == 0 {
            game.save_all();
            self.saves += 1;
        }
    }

    pub fn note_window(&mut self, active: bool) {
        if let Some(p) = &mut self.pending { p.window_active = active; }
    }

    pub fn note_render_scale(&mut self, scale: f32, gpu_ms: Option<f32>) {
        if let Some(p) = &mut self.pending { p.render_scale = scale; p.gpu_ms = gpu_ms.unwrap_or(0.0); }
    }

    pub fn observe(
        &mut self,
        game: &Game,
        residency: (usize, usize),
        stages: [f32; 4],
        presented: bool,
        missing_light: usize,
        world_visible: bool,
    ) {
        if let Some(p) = &mut self.pending {
            p.pending = game.world.pending_count();
            p.unloaded = residency.0;
            p.missing = residency.1;
            p.lighting_pending = game.world.lighting_pending();
            p.lighting_queues = game.world.lighting_stages();
            p.missing_light = missing_light;
            p.unsettled_light = game.world.missing_lighting(&game.gpu_wanted);
            p.cpu = stages;
            p.streaming=game.streaming_timings;
            p.update=game.update_timings;
            p.presented = presented;
            p.world_visible = world_visible;
            p.position = game.body.pos.to_array();
        }
    }

    pub fn write(
        &mut self,
        completed: bool,
        adapter: &str,
        size: (u32, u32),
        game: &Game,
        settings: &Settings,
    ) -> std::io::Result<()> {
        if self.written {
            return Ok(());
        }
        let phases = PHASES
            .iter()
            .map(|name| {
                let samples: Vec<_> = self.samples.iter().filter(|s| s.phase == *name).collect();
                let inactive_frames = samples.iter().filter(|s| !s.window_active).count();
                let inactive_over_budget = samples.iter().filter(|s| !s.window_active && s.interval_ms > FRAME_BUDGET_MS).count();
                let mut times: Vec<_> = samples.iter().filter(|s| s.window_active).map(|s| s.interval_ms).collect();
                times.sort_by(f64::total_cmp);
                let percentile = |p: f64| {
                    times
                        .get(((times.len() as f64 * p).ceil() as usize).saturating_sub(1))
                        .copied()
                        .unwrap_or(0.0)
                };
                let worst = percentile(1.0);
                Summary {
                    phase: (*name).into(),
                    frames: times.len(),
                    visible_frames: samples.iter().filter(|s| s.world_visible).count(),
                    mean_ms: if times.is_empty() {
                        0.0
                    } else {
                        times.iter().sum::<f64>() / times.len() as f64
                    },
                    p95_ms: percentile(0.95),
                    p99_ms: percentile(0.99),
                    worst_ms: worst,
                    minimum_fps: if worst > 0.0 { 1000.0 / worst } else { 0.0 },
                    frames_over_budget: times.iter().filter(|&&t| t > FRAME_BUDGET_MS).count(),
                    incomplete_frames: samples
                        .iter()
                        .filter(|s| {
                            s.world_visible
                                && (s.requested_unloaded > 0
                                    || s.missing_gpu_dense > 0
                                    || s.unsettled_visible_light > 0
                                    || s.missing_gpu_light > 0)
                        })
                        .count(),
                    presentation_failures: samples.iter().filter(|s| !s.presented).count(),
                    minimum_render_scale: samples.iter().map(|s| s.render_scale).fold(1.0, f32::min),
                    inactive_frames,
                    mean_gpu_ms: { let g: Vec<f32> = samples.iter().map(|s| s.gpu_ms).filter(|&g| g > 0.0).collect(); if g.is_empty() {0.0} else {g.iter().sum::<f32>() / g.len() as f32} },
                    worst_gpu_ms: samples.iter().map(|s| s.gpu_ms).fold(0.0, f32::max),
                    inactive_over_budget,
                    mean_render_scale: if samples.is_empty() {1.0} else {samples.iter().map(|s| s.render_scale).sum::<f32>() / samples.len() as f32},
                }
            })
            .collect::<Vec<_>>();
        for p in &phases {
            println!(
                "gameplay {}: {} frames, worst {:.2} ms / min {:.1} FPS, {} over budget, {} incomplete, {} presentation failures, render scale min {:.2} mean {:.2}, {} inactive-window frames ({} slow), raycast GPU mean {:.1} worst {:.1} ms",
                p.phase,
                p.frames,
                p.worst_ms,
                p.minimum_fps,
                p.frames_over_budget,
                p.incomplete_frames,
                p.presentation_failures,
                p.minimum_render_scale,
                p.mean_render_scale,
                p.inactive_frames,
                p.inactive_over_budget,
                p.mean_gpu_ms,
                p.worst_gpu_ms
            );
        }
        let all_phases_sampled = phases
            .iter()
            .enumerate()
            .all(|(i, p)| p.frames > 0 && (i == 0 || p.visible_frames > 0));
        // User acceptance (2026-10-31): fewer than 1% of gameplay frames (after
        // loading, window focused and visible) slower than the budget.
        let gameplay = &phases[1..];
        let frames: usize = gameplay.iter().map(|p| p.frames).sum();
        let slow: usize = gameplay.iter().map(|p| p.frames_over_budget).sum();
        let meets_frame_budget = completed
            && all_phases_sampled
            && (slow as f64) < frames as f64 * SLOW_FRAME_ALLOWANCE
            && phases.iter().all(|p| p.presentation_failures == 0);
        let all_frames_resident =
            all_phases_sampled && phases.iter().all(|p| p.incomplete_frames == 0);
        let report = Report {
            streaming_stage_labels:["receive","footprints","request","lighting","eviction"],
            update_stage_labels:["controls","movement_interaction","creatures","drops","stations","fluids","building","farming","redstone","devices_gravity_fire","streaming"],
            lighting_queue_labels: [
                "columns",
                "seed_chunks",
                "propagation_voxels",
                "unpublished_chunks",
            ],
            minimum_fps_target: MINIMUM_FPS,
            frame_budget_ms: FRAME_BUDGET_MS,
            scope: "Actual redraw intervals including the normal asynchronous Loading screen, simulation, streaming, presentation and event-loop pacing; no loading samples excluded. World residency applies only while the world is visible; raw missing counts are retained for hidden loading frames. Slow loading extends total duration to preserve every route phase. Startup reported separately. Finite scripted route, not an all-scenes guarantee.",
            completed: completed && all_phases_sampled,
            adapter,
            size: [size.0, size.1],
            seed: game.seed,
            generation_workers: game.world.worker_count(),
            requested_duration_seconds: self.config.duration,
            startup_ms: self.startup_ms,
            loading_ready_seconds: self.loading_ready,
            observed_duration_seconds: self
                .samples
                .last()
                .map_or(0.0, |s| s.elapsed_seconds + s.interval_ms / 1000.0),
            edits_applied: self.edits,
            saves_triggered: self.saves,
            all_phases_sampled,
            meets_frame_budget,
            all_frames_resident,
            final_settings: settings,
            phases,
            samples: &self.samples,
        };
        if let Some(parent) = self.config.report.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            &self.config.report,
            toml::to_string_pretty(&report).map_err(std::io::Error::other)?,
        )?;
        println!(
            "Gameplay performance report: {}",
            self.config.report.display()
        );
        self.written = true;
        self.passed = meets_frame_budget && all_frames_resident;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redraw_stalls_and_phase_crossings_are_not_discarded() {
        let mut perf = GameplayPerf::new(Config {
            report: PathBuf::from("unused.toml"),
            duration: 9.0,
            size: (1920, 1080),
        });
        let start = Instant::now();
        assert!(!perf.begin(start, 1.0, 1000.0));
        perf.loading_completed(start);
        assert!(!perf.begin(start + std::time::Duration::from_millis(2500), 2.5, 0.0));
        assert_eq!(perf.samples[0].phase, "loading_idle");
        assert_eq!(perf.samples[0].interval_ms, 2500.0);
        assert_eq!(perf.phase, 2);
        assert!(perf.begin(start + std::time::Duration::from_secs(10), 7.5, 0.0));
        assert_eq!(perf.samples[1].phase, "continuous_rotation");
        assert_eq!(perf.samples[1].interval_ms, 7500.0);
    }

    #[test]
    fn slow_real_loading_is_counted_and_extends_rather_than_skips_the_route() {
        let mut perf = GameplayPerf::new(Config {
            report: PathBuf::from("unused.toml"),
            duration: 9.0,
            size: (640, 480),
        });
        let start = Instant::now();
        assert!(!perf.begin(start, 0.01, 20.0));
        assert!(!perf.begin(start + std::time::Duration::from_secs(4), 4.0, 0.0));
        assert_eq!(perf.phase, 0);
        assert_eq!(perf.samples[0].interval_ms, 4000.0);
        perf.loading_completed(start + std::time::Duration::from_millis(4500));
        assert!(!perf.begin(start + std::time::Duration::from_millis(4600), 0.6, 0.0));
        assert_eq!(perf.phase, 1);
        assert!(!perf.begin(start + std::time::Duration::from_millis(12400), 7.8, 0.0));
        assert_eq!(perf.phase, 8);
        assert!(perf.begin(start + std::time::Duration::from_millis(12600), 0.2, 0.0));
        assert_eq!(perf.loading_ready, Some(4.5));
    }

    #[test]
    fn hidden_loading_and_distant_work_keep_telemetry_without_invalidating_settled_visible_worlds()
    {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-performance-gates");
        std::fs::create_dir_all(&root).unwrap();
        let report = root.join(format!("real-loading-{}.toml", std::process::id()));
        let mut perf = GameplayPerf::new(Config {
            report: report.clone(),
            duration: 9.0,
            size: (640, 480),
        });
        for (index, phase) in PHASES.into_iter().enumerate() {
            perf.samples.push(Sample {
                phase: phase.into(),
                elapsed_seconds: index as f64,
                interval_ms: 10.0,
                cpu_stages_ms: [0.0; 4],
                streaming_stages_ms:[0.0;5],
                update_stages_ms:[0.0;11],
                chunks_pending: if index == 0 { 32 } else { 0 },
                requested_unloaded: if index == 0 { 1000 } else { 0 },
                missing_gpu_dense: 0,
                lighting_pending: 5000,
                lighting_queues: [0; 4],
                missing_gpu_light: if index == 0 { 1000 } else { 0 },
                unsettled_visible_light: if index == 0 { 1000 } else { 0 },
                presented: true,
                world_visible: index != 0,
                position: [0.0; 4],
                render_scale: 1.0,
                window_active: true,
                gpu_ms: 0.0,
            });
        }
        perf.loading_ready = Some(1.0);
        perf.write(
            true,
            "test adapter",
            (640, 480),
            &Game::menu_preview(7),
            &Settings::default(),
        )
        .unwrap();
        assert!(perf.result().is_ok());
        let saved: toml::Value =
            toml::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
        assert_eq!(
            saved["samples"][0]["requested_unloaded"].as_integer(),
            Some(1000)
        );
        assert_eq!(saved["samples"][0]["world_visible"].as_bool(), Some(false));
        assert_eq!(saved["phases"][0]["frames"].as_integer(), Some(1));
        assert_eq!(
            saved["phases"][0]["incomplete_frames"].as_integer(),
            Some(0)
        );
        std::fs::remove_file(report).unwrap();
    }

    #[test]
    fn scratch_runs_are_unique_and_do_not_touch_existing_worlds() {
        let base = scratch_directory(&std::env::temp_dir()).unwrap();
        let world = base.join("saves/world");
        std::fs::create_dir_all(&world).unwrap();
        std::fs::write(world.join("sentinel"), "existing save").unwrap();
        let a = scratch_directory(&base).unwrap();
        let b = scratch_directory(&base).unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with(&base) && b.starts_with(&base));
        assert_eq!(
            std::fs::read_to_string(world.join("sentinel")).unwrap(),
            "existing save"
        );
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn fast_frames_with_missing_lighting_cannot_pass_gameplay_gate() {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-performance-gates");
        std::fs::create_dir_all(&root).unwrap();
        let report = root.join(format!("missing-light-{}.toml", std::process::id()));
        let mut perf = GameplayPerf::new(Config {
            report: report.clone(),
            duration: 9.0,
            size: (1920, 1080),
        });
        for phase in PHASES {
            perf.samples.push(Sample {
                phase: (*phase).into(),
                elapsed_seconds: 1.0,
                interval_ms: 10.0,
                cpu_stages_ms: [0.0; 4],
                streaming_stages_ms:[0.0;5],
                update_stages_ms:[0.0;11],
                chunks_pending: 0,
                requested_unloaded: 0,
                missing_gpu_dense: 0,
                lighting_pending: 1,
                lighting_queues: [0; 4],
                missing_gpu_light: 0,
                unsettled_visible_light: 1,
                presented: true,
                world_visible: true,
                position: [0.0; 4],
                render_scale: 1.0,
                window_active: true,
                gpu_ms: 0.0,
            });
        }
        perf.write(
            true,
            "test adapter",
            (1920, 1080),
            &Game::menu_preview(7),
            &Settings::default(),
        )
        .unwrap();
        assert!(
            perf.result().is_err(),
            "100 FPS with unfinished light must not certify a complete scene"
        );
        let saved: toml::Value =
            toml::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
        assert_eq!(saved["meets_frame_budget"].as_bool(), Some(true));
        assert_eq!(saved["all_frames_resident"].as_bool(), Some(false));
        std::fs::remove_file(report).unwrap();
    }
}
