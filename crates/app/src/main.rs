//! mc_4d: a Minecraft-like voxel sandbox in four spatial dimensions.
// Release builds on Windows open no console window when double-clicked.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod headless_route;
mod audio;
mod audio_assets;
mod frame;
mod game;
mod input;
mod perf;
mod progress;
mod i18n;
mod settings;
mod tutorial;
mod tutorial_visual;
mod ui;
mod options_ui;
mod video;
mod weather;
mod worlds;

use app::anyhow_lite::Result;
use frame::{FrameInput, FrameRenderer, read_texture, write_png};
use game::{Game, GameMode};
use input::{Bindings, InputState};
use mc4d_core::glam::Vec4;
use settings::{Paths, Settings};
use std::path::PathBuf;
use std::time::Instant;

const HELP: &str = "\
mc4d — a four-dimensional voxel sandbox

USAGE: mc4d [DEVELOPMENT OPTIONS]

Start without arguments to choose worlds, seeds and game mode on the main screen.
Language and slice views are configured in Options.

OPTIONS:
  --data-dir DIR        keep settings, controls and saves under DIR
  --smoke-test SECS     open the window without grabbing the mouse, screenshot after SECS, exit
  --benchmark-gameplay FILE.toml
                        record a windowed gameplay route in a new scratch world (requires --data-dir)
  --headless-route      run the --benchmark-gameplay route offscreen (no window/compositor)
  --performance-test SECS
                        route duration for --benchmark-gameplay (default 60; minimum 9)

HEADLESS (renders without a window, then exits):
  --screenshot FILE     render one frame to a PNG
  --bench N             measure N static frames (mean, p95/p99, worst, minimum FPS)
  --size WxH            output size (default 1280x800; video 1920x1080)
  --pos X,Y,Z,W         put the camera's feet here (default: spawn)
  --yaw DEG --pitch DEG --ana DEG --right-ana DEG
                        orientation: ana = turn forward into +W, right-ana = swing right into +W
  --ghosts --no-inset --no-hud --debug --no-debug
                        F3 debug overlay is on by default
  --tutorial N          show tutorial step N (1-based)
  --screen NAME         screenshot UI: playing, title, worlds, textures, inventory, options, controls, achievements, statistics
  --command CMD         execute a local command in the headless fixture (repeatable)

INTRO VIDEO (headless):
  --video FILE.mp4      render the scripted intro video (needs ffmpeg)
  --video-stills DIR    render one preview still per shot instead
  --ffmpeg PATH         ffmpeg executable (default: ffmpeg)
  --fps N               video frame rate (default 30)
  --tutorial-video FILE record the separate detailed tutorial (1080p default)
  --tutorial-video-stills DIR  tutorial storyboard stills
  --video-language en|zh video captions language
  --lossless            encode the video losslessly (a master for other formats)
";

struct Args {
    data_dir: Option<PathBuf>,
    screenshot: Option<PathBuf>,
    screen: ui::Screen,
    commands: Vec<String>,
    bench: Option<u32>,
    size: (u32, u32),
    pos: Option<Vec4>,
    yaw: f32,
    pitch: f32,
    ana: f32,
    right_ana: f32,
    ghosts: bool,
    no_inset: bool,
    no_hud: bool,
    debug: bool,
    tutorial: Option<usize>,
    smoke_test: Option<f32>,
    gameplay_report: Option<PathBuf>,
    headless_route: bool,
    performance_seconds: f32,
    video: Option<PathBuf>,
    video_kind: video::VideoKind,
    video_language: i18n::Language,
    video_stills: Option<PathBuf>,
    ffmpeg: String,
    fps: u32,
    size_given: bool,
    lossless: bool,
}

fn parse_args() -> std::result::Result<Args, String> {
    let mut a = Args {
        data_dir: None,
        screenshot: None,
        screen: ui::Screen::Playing,
        commands: Vec::new(),
        bench: None,
        size: (1280, 800),
        pos: None,
        yaw: 0.0,
        pitch: 0.0,
        ana: 0.0,
        right_ana: 0.0,
        ghosts: false,
        no_inset: false,
        no_hud: false,
        debug: true,
        tutorial: None,
        smoke_test: None,
        gameplay_report: None,
        headless_route: false,
        performance_seconds: 60.0,
        video: None,
        video_kind: video::VideoKind::Intro,
        video_language: i18n::Language::English,
        video_stills: None,
        ffmpeg: "ffmpeg".into(),
        fps: 30,
        size_given: false,
        lossless: false,
    };
    let mut it = std::env::args().skip(1);
    let num = |v: Option<String>, name: &str| -> std::result::Result<f32, String> {
        v.ok_or(format!("{name} needs a value"))?.parse().map_err(|_| format!("bad value for {name}"))
    };
    let mut video_output_requested=false;
    while let Some(arg) = it.next() {
        if matches!(arg.as_str(),"--video"|"--video-stills"|"--tutorial-video"|"--tutorial-video-stills") {
            if video_output_requested {return Err("choose exactly one intro/tutorial video or stills output".into());}
            video_output_requested=true;
        }
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--data-dir" => a.data_dir = Some(it.next().ok_or("--data-dir needs a path")?.into()),
            "--screenshot" => a.screenshot = Some(it.next().ok_or("--screenshot needs a file")?.into()),
            "--command" => a.commands.push(it.next().ok_or("--command needs a command string")?),
            "--screen" => a.screen = match it.next().as_deref() {
                Some("playing") => ui::Screen::Playing,
                Some("title") => ui::Screen::Title,
                Some("worlds") => ui::Screen::Worlds,
                Some("create-world") => ui::Screen::CreateWorld,
                Some("textures") => ui::Screen::TexturePacks,
                Some("inventory") => ui::Screen::Inventory,
                Some("achievements") => ui::Screen::Achievements,
                Some("statistics") => ui::Screen::Statistics,
                Some("options") => ui::Screen::Options,
                Some("video-options") => ui::Screen::VideoOptions,
                Some("4d-options") => ui::Screen::FourDOptions,
                Some("controls") => ui::Screen::Controls,
                _ => return Err("unknown --screen (playing, title, inventory, options, controls)".into()),
            },
            "--bench" => a.bench = Some(num(it.next(), "--bench")? as u32),
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size needs WxH")?;
                a.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
                a.size_given = true;
            }
            "--pos" => {
                let v = it.next().ok_or("--pos needs X,Y,Z,W")?;
                let p: Vec<f32> = v.split(',').map(|s| s.trim().parse().map_err(|_| "bad --pos")).collect::<std::result::Result<_, _>>()?;
                if p.len() != 4 {
                    return Err("--pos needs 4 numbers".into());
                }
                a.pos = Some(Vec4::new(p[0], p[1], p[2], p[3]));
            }
            "--yaw" => a.yaw = num(it.next(), "--yaw")?,
            "--pitch" => a.pitch = num(it.next(), "--pitch")?,
            "--ana" => a.ana = num(it.next(), "--ana")?,
            "--right-ana" => a.right_ana = num(it.next(), "--right-ana")?,
            "--ghosts" => a.ghosts = true,
            "--no-inset" => a.no_inset = true,
            "--no-hud" => a.no_hud = true,
            "--debug" => a.debug = true,
            "--no-debug" => a.debug = false,
            "--video" => a.video = Some(it.next().ok_or("--video needs a file")?.into()),
            "--tutorial-video" => {a.video_kind=video::VideoKind::Tutorial;a.video=Some(it.next().ok_or("--tutorial-video needs a file")?.into());},
            "--tutorial-video-stills" => {a.video_kind=video::VideoKind::Tutorial;a.video_stills=Some(it.next().ok_or("--tutorial-video-stills needs a directory")?.into());},
            "--video-language" => a.video_language=match it.next().as_deref(){Some("en")=>i18n::Language::English,Some("zh")=>i18n::Language::Chinese,_=>return Err("--video-language expects en or zh".into())},
            "--video-stills" => a.video_stills = Some(it.next().ok_or("--video-stills needs a directory")?.into()),
            "--ffmpeg" => a.ffmpeg = it.next().ok_or("--ffmpeg needs a path")?,
            "--lossless" => a.lossless = true,
            "--fps" => a.fps = num(it.next(), "--fps")? as u32,
            "--headless-route" => a.headless_route = true,
            "--smoke-test" => a.smoke_test = Some(num(it.next(), "--smoke-test")?),
            "--benchmark-gameplay" => a.gameplay_report = Some(it.next().ok_or("--benchmark-gameplay needs a report file")?.into()),
            "--performance-test" => a.performance_seconds = num(it.next(), "--performance-test")?,
            "--tutorial" => a.tutorial = Some(num(it.next(), "--tutorial")? as usize),
            other => return Err(format!("unknown argument `{other}` (see --help)")),
        }
    }
    if !a.commands.is_empty() && a.screenshot.is_none() && a.bench.is_none() {
        return Err("--command is only for headless screenshots and benchmarks; use in-game chat during play".into());
    }
    if a.gameplay_report.is_some() {
        if a.data_dir.is_none() { return Err("--benchmark-gameplay requires an explicit --data-dir for its scratch world".into()); }
        if !a.performance_seconds.is_finite() || a.performance_seconds < 9.0 {
            return Err("--performance-test must be a finite duration of at least 9 seconds".into());
        }
        if a.video.is_some() || a.video_stills.is_some() || a.screenshot.is_some() || a.bench.is_some() || a.smoke_test.is_some() {
            return Err("--benchmark-gameplay cannot be combined with another rendering/test mode".into());
        }
    }
    Ok(a)
}

fn main() {
    // A GUI-subsystem executable started from a terminal still reports
    // --help, benchmark results and errors there.
    #[cfg(windows)]
    unsafe { windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS); }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"))
        .init();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    let paths = args.data_dir.clone().map(Paths::under).unwrap_or_else(Paths::new);
    let mode = GameMode::Survival;
    let result = if args.video.is_some() || args.video_stills.is_some() {
        video::run(&video::VideoArgs {
            kind: args.video_kind,
            language: args.video_language,
            out: args.video.clone(),
            stills: args.video_stills.clone(),
            ffmpeg: args.ffmpeg.clone(),
            size: if args.size_given { args.size } else { (1920, 1080) },
            fps: args.fps.max(1),
            seed: 1337,
            lossless: args.lossless,
        })
    } else if args.headless_route && args.gameplay_report.is_some() {
        let data = args.data_dir.clone().expect("validated --data-dir");
        headless_route::run(perf::Config { report: args.gameplay_report.clone().unwrap(), duration: args.performance_seconds,
            size: if args.size_given { args.size } else { (1920, 1080) } }, &data)
    } else if args.screenshot.is_some() || args.bench.is_some() {
        headless(&args, paths, mode)
    } else {
        let (paths, performance) = if let Some(report) = args.gameplay_report {
            let root = match perf::scratch_directory(&paths.config) {
                Ok(root) => root,
                Err(e) => {
                    eprintln!("cannot create isolated performance world: {e}");
                    std::process::exit(1);
                }
            };
            (Paths::under(root), Some(perf::Config { report, duration: args.performance_seconds,
                size: if args.size_given { args.size } else { (1920, 1080) } }))
        } else { (paths, None) };
        let world_dir = paths.saves.join(if performance.is_some() { "benchmark" } else { "world" });
        log::info!("world: {}", world_dir.display());
        app::run(app::Options { world_dir, seed: 1337, mode, paths, smoke_test: args.smoke_test, performance })
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// Render without a window: for screenshots, benchmarks and automated checks.
/// A GPU device and an RGBA8 sRGB render target of `size`, without a window.
pub struct HeadlessGpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub target: wgpu::Texture,
    pub adapter_name: String,
}

pub fn headless_gpu(size: (u32, u32)) -> Result<HeadlessGpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))?;
    let info = adapter.get_info();
    let adapter_name = format!("{} ({:?})", info.name, info.backend);
    eprintln!("adapter: {adapter_name}");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("mc4d headless"),
        required_limits: adapter.limits(),
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))?;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless target"),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
    });
    Ok(HeadlessGpu { device, queue, target, adapter_name })
}

fn headless(args: &Args, paths: Paths, mode: GameMode) -> Result<()> {
    let size = args.size;
    let HeadlessGpu { device, queue, target, adapter_name } = headless_gpu(size)?;
    let format = target.format();
    let mut fr = FrameRenderer::new(&device, format, size, adapter_name);
    fr.renderer.gpu_timer = mc4d_render::gpu_timer::GpuTimer::new(&device, &queue);

    // Headless runs never touch the user's saves.
    let mut game = Game::new(None, 1337, if args.screen == ui::Screen::Inventory || args.bench.is_some() { GameMode::Creative } else { mode });
    if let Some(p) = args.pos {
        game.body.pos = p;
        game.flying = true;
    }
    game.orient.yaw(args.yaw.to_radians());
    game.orient.add_pitch(args.pitch.to_radians());
    game.orient.rotate_forward_ana(args.ana.to_radians());
    game.orient.rotate_right_ana(args.right_ana.to_radians());
    let mut settings: Settings = paths.load_settings();
    settings.ghosts |= args.ghosts;
    settings.inset &= !args.no_inset;
    settings.show_hints = false;
    for command in &args.commands {
        game.command(command, settings.language).map_err(|e| std::io::Error::other(e))?;
    }
    // Static benchmark fixtures represent the requested settled weather without
    // advancing simulation/time while background generation and lighting finish.
    if args.bench.is_some() { game.environment.rain = if game.environment.raining { 1.0 } else { 0.0 }; }
    let mut bindings: Bindings = paths.load_bindings();
    let input = InputState::default();
    let mut ui = ui::UiState { screen: args.screen, menu_parent: if matches!(args.screen,ui::Screen::Options|ui::Screen::VideoOptions|ui::Screen::FourDOptions|ui::Screen::Controls|ui::Screen::TexturePacks) {ui::Screen::Title}else{ui::Screen::Pause}, show_hud: !args.no_hud, show_debug: args.debug, ..Default::default() };
    if let Some(n) = args.tutorial {
        let mut t = tutorial::Tutorial::new(&game, &settings);
        t.skip_to(n.saturating_sub(1), &game, &settings);
        ui.tutorial = Some(t);
    }

    // Let generation and uploads finish (frozen in place: no player input, flying).
    let start = Instant::now();
    let was_flying = game.flying;
    game.flying = true;
    loop {
        game.body.vel = Vec4::ZERO;
        if args.bench.is_some() { game.update_streaming(&settings, false); }
        else { game.update(1.0 / 60.0, None, &settings); }
        let mut frame = |game: &mut Game, settings: &mut Settings, ui: &mut ui::UiState, bindings: &mut Bindings| {
            fr.render(
                &device,
                &queue,
                &target,
                FrameInput {
                    game,
                    settings,
                    bindings,
                    input: &input,
                    ui,
                    raw_input: raw_input(size),
                    size,
                    pixels_per_point: 1.0,
                },
            );
            fr.renderer.missing_lighting()
        };
        let missing_lighting=frame(&mut game, &mut settings, &mut ui, &mut bindings);
        if !ui.shows_world() {
            for _ in 0..3 {frame(&mut game,&mut settings,&mut ui,&mut bindings);}
            break;
        }
        // Diagnostic: measure while lighting is still streaming, as during travel.
        let early=std::env::var_os("MC4D_BENCH_EARLY").is_some();
        let geometry = game.world.pending_count() == 0 && game.gpu_wanted.iter().all(|c| game.world.is_loaded(*c));
        if early && geometry {eprintln!("early bench: {} lighting tasks pending, {missing_lighting} light chunks missing on GPU",game.world.lighting_pending());break;}
        let all_loaded = geometry
            && game.world.lighting_ready(&game.gpu_wanted) && missing_lighting == 0;
        if all_loaded || start.elapsed().as_secs() > 60 {
            // A few more frames so every upload lands.
            for _ in 0..40 {
                if args.bench.is_some() { game.update_streaming(&settings, false); }
                else { game.update(1.0 / 60.0, None, &settings); }
                frame(&mut game, &mut settings, &mut ui, &mut bindings);
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    game.flying = was_flying;
    eprintln!(
        "scene preparation ended after {:.1}s: {} chunks loaded, {} on GPU",
        start.elapsed().as_secs_f32(),
        game.world.chunks.len(),
        fr.renderer.store.resident_count()
    );

    if let Some(n) = args.bench {
        if n == 0 {
            return Err("--bench needs at least one frame".into());
        }
        let unloaded = game.gpu_wanted.iter().filter(|&&c| !game.world.is_loaded(c)).count();
        let missing_gpu = fr.renderer.store.missing_dense_count(&game.world, &game.gpu_wanted);
        let missing_light = fr.renderer.missing_lighting();
        let light_pending = game.world.lighting_pending();
        if std::env::var_os("MC4D_BENCH_EARLY").is_none() && ui.shows_world() && (unloaded != 0 || missing_gpu != 0 || missing_light != 0 || !game.world.lighting_ready(&game.gpu_wanted)) {
            return Err(format!(
                "benchmark scene is incomplete: {unloaded} chunks unloaded, {missing_gpu} block/state chunks stale on GPU, {missing_light} light chunks unready, {light_pending} lighting tasks pending; no performance result certified"
            ).into());
        }
        device.poll(wgpu::PollType::wait_indefinitely())?;
        let mut frame_ms = Vec::with_capacity(n as usize);
        let mut submission_ms = Vec::with_capacity(n as usize);
        let mut raycast_ms: Vec<f64> = Vec::new();
        for _ in 0..n {
            if std::env::var_os("MC4D_BENCH_EARLY").is_some() { game.update_streaming(&settings, false); }
            let t0 = Instant::now();
            fr.render(
                &device,
                &queue,
                &target,
                FrameInput {
                    game: &mut game,
                    settings: &mut settings,
                    bindings: &mut bindings,
                    input: &input,
                    ui: &mut ui,
                    raw_input: raw_input(size),
                    size,
                    pixels_per_point: 1.0,
                },
            );
            submission_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
            if let Some(ms) = fr.gpu_ms.take() { raycast_ms.push(ms as f64); }
            device.poll(wgpu::PollType::wait_indefinitely())?;
            frame_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        let ms = frame_ms.iter().sum::<f64>() / frame_ms.len() as f64;
        let worst_index = frame_ms.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        let worst_cpu = submission_ms[worst_index];
        let worst_wait = frame_ms[worst_index] - worst_cpu;
        let mean_cpu = submission_ms.iter().sum::<f64>() / submission_ms.len() as f64;
        frame_ms.sort_by(f64::total_cmp);
        let percentile = |p: f64| frame_ms[((frame_ms.len() as f64 * p).ceil() as usize).saturating_sub(1)];
        let worst = frame_ms[frame_ms.len() - 1];
        let slow = frame_ms.iter().filter(|&&ms| ms > perf::FRAME_BUDGET_MS).count();
        println!("Static headless render benchmark: {n} frames, {}x{}, scale {:.2}, distance {} chunks, ghosts {}, views {}, shadows {}",
            size.0, size.1, settings.render_scale, settings.render_distance, settings.ghosts, settings.active_view_count(), settings.shadows);
        let feet = game.body.pos;
        let biome = game.world.terrain.biome(feet.x.floor() as i32, feet.z.floor() as i32, feet.w.floor() as i32);
        println!("Scene biome: {}; feet: {:.2},{:.2},{:.2},{:.2}; mode: {:?}", biome.name(), feet.x, feet.y, feet.z, feet.w, game.mode);
        println!("Scene environment: ticks {}, rain {:.2}, thunder {}, daylight {:.2}; lighting ready {}, light GPU bytes {}",
            game.environment.world_ticks, game.environment.rain, game.environment.thundering, game.environment.daylight(),
            game.world.lighting_ready(&game.gpu_wanted), fr.renderer.lighting_bytes());
        println!("mean {ms:.2} ms ({:.1} FPS), p95 {:.2} ms, p99 {:.2} ms, worst {worst:.2} ms (minimum {:.1} FPS)",
            1000.0 / ms, percentile(0.95), percentile(0.99), 1000.0 / worst);
        if !raycast_ms.is_empty() { println!("Ray-casting pass GPU time: mean {:.2} ms over {} timed frames", raycast_ms.iter().sum::<f64>() / raycast_ms.len() as f64, raycast_ms.len()); }
        println!("Frames above the {:.2} ms / {:.0} FPS budget: {slow}/{n}", perf::FRAME_BUDGET_MS, perf::MINIMUM_FPS);
        println!("Mean CPU submission {mean_cpu:.2} ms; slowest frame CPU submission {worst_cpu:.2} ms, completion wait {worst_wait:.2} ms (wait includes driver/OS scheduling, not a GPU timestamp)");
        println!("Includes CPU frame submission and GPU completion; excludes simulation, moving-world streaming, window presentation and startup. This does not certify a minimum gameplay FPS.");
    }

    if let Some(path) = &args.screenshot {
        let rgba = read_texture(&device, &queue, &target);
        write_png(path, size.0, size.1, &rgba)?;
        let p = game.body.pos;
        println!("wrote {} (feet at {:.1},{:.1},{:.1},{:.1})", path.display(), p.x, p.y, p.z, p.w);
    }
    Ok(())
}

pub fn raw_input(size: (u32, u32)) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0 as f32, size.1 as f32))),
        ..Default::default()
    }
}

#[cfg(test)]
mod lighting_motion_tests;
