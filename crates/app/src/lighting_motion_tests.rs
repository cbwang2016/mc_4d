//! Opt-in real-renderer moving-world evidence. This is not a windowed FPS claim.
use crate::{
    frame::{FrameInput, FrameRenderer, read_texture, write_png},
    game::{Game, GameMode},
    input::{Action, Bindings, InputState},
    settings::Settings,
    ui::{Screen, UiState},
};
use serde::Serialize;
#[derive(Serialize)]
struct Capture {
    name: String,
    wall_seconds: f64,
    position: [f32; 4],
    wanted: usize,
    loaded: usize,
    display_planes: usize,
    sky_known_planes: usize,
    certified_planes: usize,
    pending_light: usize,
    queues: [usize; 4],
}
#[test]
#[ignore = "requires GPU; captures actual moving/rotating default-distance terrain with computed-light availability"]
fn capture_moving_default_world_light_availability() {
    let size = (1280, 800);
    let gpu = crate::headless_gpu(size).unwrap();
    let mut frame = FrameRenderer::new(&gpu.device, gpu.target.format(), size, gpu.adapter_name);
    let mut game = Game::new(None, 1337, GameMode::Creative);
    game.body.pos.y += 6.0;
    game.flying = true;
    let mut settings = Settings::default();
    let mut bindings = Bindings::default();
    let mut input = InputState::default();
    let mut ui = UiState {
        screen: Screen::Loading,
        show_debug: true,
        ..Default::default()
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/lighting-motion-review");
    std::fs::create_dir_all(&root).unwrap();
    let started = std::time::Instant::now();
    let mut captures = Vec::new();
    let mut render = |game: &mut Game,
                      settings: &mut Settings,
                      bindings: &mut Bindings,
                      input: &InputState,
                      ui: &mut UiState| {
        frame.render(
            &gpu.device,
            &gpu.queue,
            &gpu.target,
            FrameInput {
                game,
                settings,
                bindings,
                input,
                ui,
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
    loop {
        game.update_streaming(&settings, false);
        if render(&mut game, &mut settings, &mut bindings, &input, &mut ui) {
            break;
        }
        assert!(
            started.elapsed().as_secs_f64() < 60.0,
            "initial default-distance world did not become fully ready"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    ui.screen = Screen::Playing;
    for (phase, steps) in [
        ("ready", 1),
        ("w-flight", 120),
        ("rotation", 120),
        ("settling", 120),
    ] {
        for step in 0..steps {
            input.release_all();
            input.end_frame();
            if phase == "w-flight" {
                input.press(bindings.get(Action::MoveAna)[0]);
                input.press(bindings.get(Action::Sprint)[0]);
            }
            if phase == "rotation" {
                game.orient.rotate_forward_ana(0.45 / 60.0);
                game.orient.rotate_right_ana(0.23 / 60.0);
            }
            game.update(1.0 / 60.0, Some((&input, &bindings)), &settings);
            ui.dt = 1.0 / 60.0;
            render(&mut game, &mut settings, &mut bindings, &input, &mut ui);
            if step == steps - 1 || (phase != "ready" && step == 29) {
                let name = format!("{phase}-{step:03}");
                let rgba = read_texture(&gpu.device, &gpu.queue, &gpu.target);
                write_png(&root.join(format!("{name}.png")), size.0, size.1, &rgba).unwrap();
                let wanted = &game.gpu_wanted;
                let record = Capture {
                    name,
                    wall_seconds: started.elapsed().as_secs_f64(),
                    position: game.body.pos.to_array(),
                    wanted: wanted.len(),
                    loaded: wanted.iter().filter(|&&c| game.world.is_loaded(c)).count(),
                    display_planes: wanted
                        .iter()
                        .filter(|&&c| {
                            game.world.light_chunk(c).is_some_and(|l| {
                                game.world.chunks.get(&c).is_some_and(|g| l.matches(g))
                            })
                        })
                        .count(),
                    sky_known_planes: wanted
                        .iter()
                        .filter(|&&c| {
                            game.world.light_chunk(c).is_some_and(|l| {
                                l.sky_known
                                    && game.world.chunks.get(&c).is_some_and(|g| l.matches(g))
                            })
                        })
                        .count(),
                    certified_planes: wanted
                        .iter()
                        .filter(|&&c| game.world.light_ready(c))
                        .count(),
                    pending_light: game.world.lighting_pending(),
                    queues: game.world.lighting_stages(),
                };
                eprintln!(
                    "{}: {}/{} computed display planes, {} certified",
                    record.name, record.display_planes, record.wanted, record.certified_planes
                );
                captures.push(record);
                #[derive(Serialize)] struct PartialReport<'a>{captures:&'a [Capture]}
                std::fs::write(root.join("captures.toml"),toml::to_string_pretty(&PartialReport{captures:&captures}).unwrap()).unwrap();
            }
        }
    }
    #[derive(Serialize)]
    struct Report {
        scope: &'static str,
        captures: Vec<Capture>,
    }
    std::fs::write(root.join("captures.toml"),toml::to_string_pretty(&Report{scope:"Actual Game updates and Radeon renderer captures; fixed simulationdt and no window presentation, therefore no FPS certification",captures}).unwrap()).unwrap();
}
