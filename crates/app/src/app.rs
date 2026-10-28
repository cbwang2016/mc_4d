//! Windowed game: winit event loop, input routing (including key-capture for rebinding),
//! menus and presentation.

use crate::frame::{FrameInput, FrameRenderer, read_texture, write_png};
use crate::game::{Game, GameMode};
use crate::input::{Action, Binding, Bindings, InputState};
use crate::settings::{Paths, Settings};
use crate::ui::{Screen, UiState};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

pub struct Options {
    pub world_dir: std::path::PathBuf,
    pub seed: u32,
    pub mode: GameMode,
    pub paths: Paths,
    /// Run for this many seconds without grabbing the cursor, save a screenshot, then exit.
    pub smoke_test: Option<f32>,
}

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    frame: FrameRenderer,
    egui_state: egui_winit::State,
}

struct App {
    opts: Options,
    gfx: Option<Gfx>,
    game: Option<Game>,
    settings: Settings,
    bindings: Bindings,
    input: InputState,
    ui: UiState,
    last_frame: Instant,
    fps_timer: (f32, u32),
    grabbed: bool,
    screenshot_requested: bool,
    started: Instant,
    smoke_shot_taken: bool,
    exiting: bool,
}

pub fn run(opts: Options) -> anyhow_lite::Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let settings = opts.paths.load_settings();
    let bindings = opts.paths.load_bindings();
    let mut app = App {
        opts,
        gfx: None,
        game: None,
        settings,
        bindings,
        input: InputState::default(),
        ui: UiState::default(),
        last_frame: Instant::now(),
        fps_timer: (0.0, 0),
        grabbed: false,
        screenshot_requested: false,
        started: Instant::now(),
        smoke_shot_taken: false,
        exiting: false,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

impl App {
    fn init(&mut self, event_loop: &ActiveEventLoop) -> anyhow_lite::Result<()> {
        let attrs = Window::default_attributes()
            .with_title("mc_4d — a four-dimensional voxel sandbox")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        let window = Arc::new(event_loop.create_window(attrs)?);
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))?;
        let info = adapter.get_info();
        let adapter_name = format!("{} ({:?})", info.name, info.backend);
        log::info!("adapter: {adapter_name}");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("mc4d"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            usage |= wgpu::TextureUsages::COPY_SRC;
        }
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let config = wgpu::SurfaceConfiguration {
            usage,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: if format.is_srgb() { vec![format.remove_srgb_suffix()] } else { vec![] },
            color_space: Default::default(),
        };
        surface.configure(&device, &config);
        let frame = FrameRenderer::new(&device, format, (config.width, config.height), adapter_name);
        let egui_state = egui_winit::State::new(
            frame.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        self.gfx = Some(Gfx { window, surface, device, queue, config, frame, egui_state });

        let game = Game::new(Some(self.opts.world_dir.clone()), self.opts.seed, self.opts.mode);
        if !self.settings.tutorial_done {
            self.ui.tutorial = Some(crate::tutorial::Tutorial::new(&game, &self.settings));
        }
        self.game = Some(game);
        self.set_playing(true);
        Ok(())
    }

    fn set_grab(&mut self, grab: bool) {
        let Some(gfx) = &self.gfx else { return };
        if self.opts.smoke_test.is_some() {
            return;
        }
        if grab {
            let ok = gfx
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| gfx.window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            gfx.window.set_cursor_visible(false);
            self.grabbed = ok;
        } else {
            let _ = gfx.window.set_cursor_grab(CursorGrabMode::None);
            gfx.window.set_cursor_visible(true);
            self.grabbed = false;
        }
    }

    fn set_playing(&mut self, playing: bool) {
        self.ui.screen = if playing { Screen::Playing } else { Screen::Pause };
        self.ui.capturing = None;
        self.input.release_all();
        self.set_grab(playing);
    }

    fn quit(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(game) = &mut self.game {
            game.save_all();
        }
        self.opts.paths.save_settings(&self.settings);
        // Release GPU and windowing state while the display connection is still alive.
        self.gfx = None;
        self.exiting = true;
        event_loop.exit();
    }

    /// While the Controls screen waits for an input, the next key/button/wheel is bound.
    fn capture(&mut self, b: Binding) -> bool {
        let Some((action, slot)) = self.ui.capturing.take() else { return false };
        if b != Binding::Key(KeyCode::Escape) {
            self.bindings.set(action, slot, b);
            self.opts.paths.save_bindings(&self.bindings);
        }
        true
    }

    fn on_key(&mut self, event: &KeyEvent, event_loop: &ActiveEventLoop) -> bool {
        let PhysicalKey::Code(code) = event.physical_key else { return false };
        if let Key::Character(s) = &event.logical_key {
            self.input.layout_names.entry(code).or_insert_with(|| s.to_uppercase());
        }
        let pressed = event.state == ElementState::Pressed;
        if pressed && self.capture(Binding::Key(code)) {
            return true;
        }
        if matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) && pressed && self.ui.screen == Screen::Playing {
            self.ui.enter_pressed = true;
            return true;
        }
        if code == KeyCode::Escape && pressed && !event.repeat {
            match self.ui.screen {
                Screen::Playing => self.set_playing(false),
                Screen::Pause => self.set_playing(true),
                Screen::Options | Screen::Controls => self.ui.screen = Screen::Pause,
            }
            return true;
        }
        if self.ui.screen == Screen::Playing {
            if pressed {
                if !event.repeat {
                    self.input.press(Binding::Key(code));
                }
            } else {
                self.input.release(Binding::Key(code));
            }
            return true;
        }
        let _ = event_loop;
        false
    }

    fn handle_toggles(&mut self) {
        let (i, b) = (&self.input, &self.bindings);
        let s = &mut self.settings;
        let mut changed = false;
        if i.just_pressed(b, Action::ToggleHud) {
            self.ui.show_hud = !self.ui.show_hud;
        }
        if i.just_pressed(b, Action::ToggleDebug) {
            self.ui.show_debug = !self.ui.show_debug;
        }
        let game = self.game.as_mut().unwrap();
        for (action, flag, name) in [
            (Action::ToggleGhosts, &mut s.ghosts, "Ghost layers"),
            (Action::ToggleInset, &mut s.inset, "Side view"),
            (Action::ToggleGlasses, &mut s.glasses, "4D glasses"),
        ] {
            if i.just_pressed(b, action) {
                *flag = !*flag;
                changed = true;
                game.toast(format!("{name}: {}", if *flag { "on" } else { "off" }));
            }
        }
        if i.just_pressed(b, Action::Screenshot) {
            self.screenshot_requested = true;
        }
        if changed {
            self.opts.paths.save_settings(&self.settings);
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.fps_timer.0 += dt;
        self.fps_timer.1 += 1;
        if self.fps_timer.0 > 0.5 {
            self.ui.fps = self.fps_timer.1 as f32 / self.fps_timer.0;
            self.fps_timer = (0.0, 0);
        }

        self.ui.dt = dt;
        let playing = self.ui.screen == Screen::Playing;
        if playing {
            self.handle_toggles();
        }
        {
            let game = self.game.as_mut().unwrap();
            let input = playing.then_some((&self.input, &self.bindings));
            game.update(dt, input, &self.settings);
        }
        let t_update = Instant::now();

        let gfx = self.gfx.as_mut().unwrap();
        let tex = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gfx.surface.configure(&gfx.device, &gfx.config);
                self.input.end_frame();
                return;
            }
            other => {
                log::debug!("skipping frame: {other:?}");
                self.input.end_frame();
                return;
            }
        };
        let raw_input = gfx.egui_state.take_egui_input(&gfx.window);
        let t_acquire = Instant::now();
        let (out, platform) = gfx.frame.render(
            &gfx.device,
            &gfx.queue,
            &tex.texture,
            FrameInput {
                game: self.game.as_mut().unwrap(),
                settings: &mut self.settings,
                bindings: &mut self.bindings,
                input: &self.input,
                ui: &mut self.ui,
                raw_input,
                size: (gfx.config.width, gfx.config.height),
                pixels_per_point: gfx.window.scale_factor() as f32,
            },
        );
        gfx.egui_state.handle_platform_output(&gfx.window, platform);
        if self.screenshot_requested {
            self.screenshot_requested = false;
            if gfx.config.usage.contains(wgpu::TextureUsages::COPY_SRC) {
                let rgba = read_texture(&gfx.device, &gfx.queue, &tex.texture);
                let name = format!("mc4d-{}.png", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
                let path = self.opts.paths.screenshots.join(name);
                let game = self.game.as_mut().unwrap();
                match write_png(&path, gfx.config.width, gfx.config.height, &rgba) {
                    Ok(()) => {
                        println!("screenshot: {}", path.display());
                        game.toast(format!("Saved screenshot {}", path.display()));
                    }
                    Err(e) => game.toast(format!("Screenshot failed: {e}")),
                }
            }
        }
        gfx.window.pre_present_notify();
        let t_render = Instant::now();
        gfx.queue.present(tex);
        let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1000.0;
        let t = [ms(now, t_update), ms(t_update, t_acquire), ms(t_acquire, t_render), ms(t_render, Instant::now())];
        for (avg, v) in self.ui.timings.iter_mut().zip(t) {
            *avg = *avg * 0.95 + v * 0.05;
        }

        if out.settings_changed {
            self.opts.paths.save_settings(&self.settings);
        }
        if out.bindings_changed {
            self.opts.paths.save_bindings(&self.bindings);
        }
        if out.resume {
            self.set_playing(true);
        }
        if out.quit {
            self.quit(event_loop);
            return;
        }
        if let Some(secs) = self.opts.smoke_test {
            let elapsed = self.started.elapsed().as_secs_f32();
            if self.smoke_shot_taken {
                let t = self.ui.timings;
                println!("timings ms: update {:.2}, acquire {:.2}, render {:.2}, present {:.2}", t[0], t[1], t[2], t[3]);
                println!("smoke test: {:.0} fps, {} chunks loaded", self.ui.fps, self.game.as_ref().map_or(0, |g| g.world.chunks.len()));
                self.quit(event_loop);
                return;
            } else if elapsed > secs {
                self.screenshot_requested = true;
                self.smoke_shot_taken = true;
            }
        }
        self.input.end_frame();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_none() && !self.exiting
            && let Err(e) = self.init(event_loop) {
                log::error!("startup failed: {e}");
                event_loop.exit();
            }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let capturing = self.ui.capturing.is_some();
        let playing = self.ui.screen == Screen::Playing;
        {
            let Some(gfx) = self.gfx.as_mut() else { return };
            // egui sees events while a menu is open (but not the input being captured for a binding).
            let is_input =
                matches!(event, WindowEvent::KeyboardInput { .. } | WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. });
            if !(capturing && is_input) && !(playing && is_input) {
                let _ = gfx.egui_state.on_window_event(&gfx.window, &event);
            }
        }

        match event {
            WindowEvent::CloseRequested => self.quit(event_loop),
            WindowEvent::Resized(size) => {
                let gfx = self.gfx.as_mut().unwrap();
                gfx.config.width = size.width.max(1);
                gfx.config.height = size.height.max(1);
                gfx.surface.configure(&gfx.device, &gfx.config);
            }
            WindowEvent::Focused(false) => {
                self.input.release_all();
                if self.ui.screen == Screen::Playing {
                    self.set_playing(false);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.on_key(&event, event_loop);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let b = Binding::Mouse(button.into());
                if state == ElementState::Pressed && self.capture(b) {
                    return;
                }
                if self.ui.screen == Screen::Playing {
                    if !self.grabbed && state == ElementState::Pressed {
                        self.set_grab(true);
                    }
                    match state {
                        ElementState::Pressed => self.input.press(b),
                        ElementState::Released => self.input.release(b),
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                if lines != 0.0 && self.capture(if lines > 0.0 { Binding::WheelUp } else { Binding::WheelDown }) {
                    return;
                }
                if self.ui.screen == Screen::Playing {
                    self.input.wheel(lines);
                }
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event
            && self.ui.screen == Screen::Playing && self.grabbed {
                self.input.mouse_delta.0 += delta.0;
                self.input.mouse_delta.1 += delta.1;
            }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // A smoke test must end even if the compositor never lets the window draw
        // (Wayland sends no frame callbacks to hidden windows).
        if let Some(secs) = self.opts.smoke_test {
            if !self.exiting && !self.smoke_shot_taken && self.started.elapsed().as_secs_f32() > secs + 10.0 {
                println!("smoke test: no frame was drawn (window hidden, or screen locked?)");
                self.quit(event_loop);
                return;
            }
        }
        if let Some(gfx) = &self.gfx {
            gfx.window.request_redraw();
        }
    }
}

/// Minimal boxed-error result so `?` works across winit/wgpu error types.
pub mod anyhow_lite {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}
