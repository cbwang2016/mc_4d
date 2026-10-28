//! mc_4d: a Minecraft-like voxel sandbox in four spatial dimensions.

mod app;
mod frame;
mod game;
mod input;
mod settings;
mod tutorial;
mod ui;
mod video;

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

USAGE: mc4d [OPTIONS]

OPTIONS:
  --world NAME          world save to load or create (default: world)
  --seed N              seed for a new world (default: 1337)
  --creative            start a new world in creative mode
  --data-dir DIR        keep settings, controls and saves under DIR
  --smoke-test SECS     open the window without grabbing the mouse, screenshot after SECS, exit

HEADLESS (renders without a window, then exits):
  --screenshot FILE     render one frame to a PNG
  --bench N             render N frames and print the average frame time
  --size WxH            output size (default 1280x800)
  --pos X,Y,Z,W         put the camera's feet here (default: spawn)
  --yaw DEG --pitch DEG --ana DEG --right-ana DEG
                        orientation: ana = turn forward into +W, right-ana = swing right into +W
  --ghosts --no-inset --no-hud --debug
  --tutorial N          show tutorial step N (1-based)

INTRO VIDEO (headless):
  --video FILE.mp4      render the scripted intro video (needs ffmpeg)
  --video-stills DIR    render one preview still per shot instead
  --ffmpeg PATH         ffmpeg executable (default: ffmpeg)
  --fps N               video frame rate (default 30)
";

struct Args {
    world: String,
    seed: u32,
    creative: bool,
    data_dir: Option<PathBuf>,
    screenshot: Option<PathBuf>,
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
    video: Option<PathBuf>,
    video_stills: Option<PathBuf>,
    ffmpeg: String,
    fps: u32,
    size_given: bool,
}

fn parse_args() -> std::result::Result<Args, String> {
    let mut a = Args {
        world: "world".into(),
        seed: 1337,
        creative: false,
        data_dir: None,
        screenshot: None,
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
        debug: false,
        tutorial: None,
        smoke_test: None,
        video: None,
        video_stills: None,
        ffmpeg: "ffmpeg".into(),
        fps: 30,
        size_given: false,
    };
    let mut it = std::env::args().skip(1);
    let num = |v: Option<String>, name: &str| -> std::result::Result<f32, String> {
        v.ok_or(format!("{name} needs a value"))?.parse().map_err(|_| format!("bad value for {name}"))
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--world" => a.world = it.next().ok_or("--world needs a name")?,
            "--seed" => a.seed = num(it.next(), "--seed")? as u32,
            "--creative" => a.creative = true,
            "--data-dir" => a.data_dir = Some(it.next().ok_or("--data-dir needs a path")?.into()),
            "--screenshot" => a.screenshot = Some(it.next().ok_or("--screenshot needs a file")?.into()),
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
            "--video" => a.video = Some(it.next().ok_or("--video needs a file")?.into()),
            "--video-stills" => a.video_stills = Some(it.next().ok_or("--video-stills needs a directory")?.into()),
            "--ffmpeg" => a.ffmpeg = it.next().ok_or("--ffmpeg needs a path")?,
            "--fps" => a.fps = num(it.next(), "--fps")? as u32,
            "--smoke-test" => a.smoke_test = Some(num(it.next(), "--smoke-test")?),
            "--tutorial" => a.tutorial = Some(num(it.next(), "--tutorial")? as usize),
            other => return Err(format!("unknown argument `{other}` (see --help)")),
        }
    }
    Ok(a)
}

fn main() {
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
    let mode = if args.creative { GameMode::Creative } else { GameMode::Survival };
    let result = if args.video.is_some() || args.video_stills.is_some() {
        video::run(&video::VideoArgs {
            out: args.video.clone(),
            stills: args.video_stills.clone(),
            ffmpeg: args.ffmpeg.clone(),
            size: if args.size_given { args.size } else { (1280, 720) },
            fps: args.fps.max(1),
            seed: args.seed,
        })
    } else if args.screenshot.is_some() || args.bench.is_some() {
        headless(&args, paths, mode)
    } else {
        let world_dir = paths.saves.join(&args.world);
        log::info!("world: {}", world_dir.display());
        app::run(app::Options { world_dir, seed: args.seed, mode, paths, smoke_test: args.smoke_test })
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

    // Headless runs never touch the user's saves.
    let mut game = Game::new(None, args.seed, mode);
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
    let mut bindings: Bindings = paths.load_bindings();
    let input = InputState::default();
    let mut ui = ui::UiState { show_hud: !args.no_hud, show_debug: args.debug, ..Default::default() };
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
        game.update(1.0 / 60.0, None, &settings);
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
        };
        frame(&mut game, &mut settings, &mut ui, &mut bindings);
        let all_loaded = game.world.pending_count() == 0 && game.gpu_wanted.iter().all(|c| game.world.is_loaded(*c));
        if all_loaded || start.elapsed().as_secs() > 60 {
            // A few more frames so every upload lands.
            for _ in 0..40 {
                game.update(1.0 / 60.0, None, &settings);
                frame(&mut game, &mut settings, &mut ui, &mut bindings);
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    game.flying = was_flying;
    eprintln!(
        "world ready in {:.1}s: {} chunks loaded, {} on GPU",
        start.elapsed().as_secs_f32(),
        game.world.chunks.len(),
        fr.renderer.store.resident_count()
    );

    if let Some(n) = args.bench {
        device.poll(wgpu::PollType::wait_indefinitely())?;
        let t0 = Instant::now();
        for _ in 0..n {
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
            device.poll(wgpu::PollType::wait_indefinitely())?;
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / n.max(1) as f64;
        println!("{}x{}: {ms:.2} ms/frame ({:.0} fps)", size.0, size.1, 1000.0 / ms);
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
