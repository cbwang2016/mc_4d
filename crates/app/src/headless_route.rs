//! The scripted gameplay route of `--benchmark-gameplay`, rendered offscreen.
//! Runs while no window can be shown (locked screen, CI). Simulation,
//! streaming, lighting, uploads and GPU rendering are the real ones; window
//! presentation and compositor pacing are not included, so the windowed route
//! remains the acceptance measurement. Like a swapchain with frame latency 2,
//! the CPU may run at most two frames ahead of the GPU.
use crate::frame::{FrameInput, FrameRenderer};
use crate::game::{Game, GameMode};
use crate::input::{Bindings, InputState};
use crate::perf::{Config, GameplayPerf};
use crate::settings::Settings;
use crate::ui::{Screen, UiState};
use std::collections::VecDeque;
use std::path::Path;
use std::time::Instant;

pub fn run(config: Config, data: &Path) -> crate::app::anyhow_lite::Result<()> {
    let started = Instant::now();
    let size = config.size;
    let gpu = crate::headless_gpu(size)?;
    let mut frame = FrameRenderer::new(&gpu.device, gpu.target.format(), size, gpu.adapter_name.clone());
    frame.renderer.initialize_pools(&gpu.device, &gpu.queue);
    frame.renderer.gpu_timer = mc4d_render::gpu_timer::GpuTimer::new(&gpu.device, &gpu.queue);
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let root = crate::perf::scratch_directory(data)?;
    let mut game = Game::try_load(Some(root.join("benchmark")), 1337, GameMode::Creative).map_err(std::io::Error::other)?;
    game.body.pos.y += 6.0;
    game.flying = true;
    let mut settings = Settings { show_hints: false, ..Settings::default() };
    let mut bindings = Bindings::default();
    let mut input = InputState::default();
    let mut ui = UiState { screen: Screen::Loading, show_hud: true, show_debug: true, ..Default::default() };
    let mut perf = GameplayPerf::new(config);
    let mut in_flight = VecDeque::new();
    let mut last = Instant::now();
    loop {
        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;
        if perf.begin(now, dt, started.elapsed().as_secs_f64() * 1000.0) {
            break;
        }
        let playing = ui.screen == Screen::Playing;
        if playing {
            perf.drive(dt, &mut game, &mut settings, &mut input, &bindings);
            game.update(dt, Some((&input, &bindings)), &settings);
        } else {
            game.update_streaming(&settings, false);
        }
        let t_update = Instant::now();
        frame.render(&gpu.device, &gpu.queue, &gpu.target, FrameInput {
            game: &mut game, settings: &mut settings, bindings: &mut bindings, input: &input, ui: &mut ui,
            raw_input: crate::raw_input(size), size, pixels_per_point: 1.0,
        });
        let t_render = Instant::now();
        in_flight.push_back(gpu.queue.submit([]));
        if in_flight.len() > 2 {
            let oldest = in_flight.pop_front();
            gpu.device.poll(wgpu::PollType::Wait { submission_index: oldest, timeout: None })?;
        }
        if !playing && frame.world_ready(&game) {
            ui.screen = Screen::Playing;
            mc4d_core::set_background_throttle(true);
            perf.loading_completed(Instant::now());
        }
        let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1000.0;
        perf.observe(&game, frame.renderer.store.residency_counts(),
            [ms(now, t_update), 0.0, ms(t_update, t_render), ms(t_render, Instant::now())],
            true, frame.renderer.missing_lighting(), playing);
        perf.note_render_scale(frame.renderer.render_scale(), frame.gpu_ms);
    }
    gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
    perf.write(true, &format!("{} (headless route)", gpu.adapter_name), size, &game, &settings)?;
    perf.result().map_err(|e| std::io::Error::other(e).into())

}
