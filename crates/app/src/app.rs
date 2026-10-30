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
    pub performance: Option<crate::perf::Config>,
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
    profile: crate::progress::ProfileStore,
    profile_save_timer: f32,
    gpu_probe: Vec<(f32,f32,f32)>,
    /// MC4D_GPU_PROBE: diagnostic that serialises CPU/GPU and logs GPU completion time.
    gpu_probe_enabled: bool,
    window_focused: bool,
    window_hidden: bool,
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
    performance: Option<crate::perf::GameplayPerf>,
    audio: Option<crate::audio::Audio>,
    pending_load: Option<std::sync::mpsc::Receiver<Result<Game,String>>>,
    streaming_load: bool,
    tutorial_after_load: bool,
}

pub fn run(mut opts: Options) -> anyhow_lite::Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut settings = opts.paths.load_settings();
    let mut bindings = opts.paths.load_bindings();
    if opts.performance.is_some() {
        settings = Settings::default();
        settings.tutorial_done = true;
        bindings = Bindings::default();
    }
    let performance = opts.performance.take().map(crate::perf::GameplayPerf::new);
    let ui = UiState {
        world_name: opts.world_dir.file_name().and_then(|s| s.to_str()).unwrap_or("world").into(),
        world_seed: opts.seed.to_string(),
        worlds: crate::worlds::list(&opts.paths.saves),
        texture_pack_root: {let root=opts.paths.texture_packs();let _=std::fs::create_dir_all(&root);Some(root)},
        ..Default::default()
    };
    let mut app = App {
        profile_save_timer: 0.0,
        gpu_probe: Vec::new(),
        gpu_probe_enabled: std::env::var_os("MC4D_GPU_PROBE").is_some(),
        window_focused: true,
        window_hidden: false,
        profile: crate::progress::ProfileStore::load(opts.paths.config.join("statistics.toml")),
        opts,
        gfx: None,
        game: None,
        settings,
        bindings,
        input: InputState::default(),
        ui,
        last_frame: Instant::now(),
        fps_timer: (0.0, 0),
        grabbed: false,
        screenshot_requested: false,
        started: Instant::now(),
        smoke_shot_taken: false,
        exiting: false,
        performance,
        audio: None,
        pending_load: None,
        streaming_load: false,
        tutorial_after_load: false,
    };
    event_loop.run_app(&mut app)?;
    if let Some(perf) = &app.performance { perf.result()?; }
    Ok(())
}

impl App {
    fn init(&mut self, event_loop: &ActiveEventLoop) -> anyhow_lite::Result<()> {
        let mut attrs = Window::default_attributes()
            .with_title("mc_4d — a four-dimensional voxel sandbox")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        if let Some(perf) = &self.performance {
            attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(perf.config.size.0, perf.config.size.1));
        }
        if self.performance.is_some() && std::env::var_os("MC4D_PERF_FULLSCREEN").is_some() { self.settings.fullscreen = true; }
        if self.settings.fullscreen && (self.performance.is_none() || std::env::var_os("MC4D_PERF_FULLSCREEN").is_some()) {
            // Benchmarks measure the supported target: the largest monitor up to 1920x1200.
            let monitor = self.performance.as_ref().and_then(|_| event_loop.available_monitors()
                .filter(|m| m.size().width <= 1920 && m.size().height <= 1200).max_by_key(|m| m.size().width * m.size().height));
            attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(monitor)));
        }
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
            // Ray-casting GPU time for F3 and benchmark reports.
            required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            ..Default::default()
        }))?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            usage |= wgpu::TextureUsages::COPY_SRC;
        }
        // Controlled development experiment; ordinary play retains the existing
        // tearing-free mode until the complete route has verified the alternative.
        let test_immediate=self.performance.is_some()
            && std::env::var_os("MC4D_PERF_IMMEDIATE").is_some();
        let present_mode = if test_immediate && caps.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else if std::env::var("MC4D_PRESENT").as_deref()==Ok("fifo") {
            wgpu::PresentMode::Fifo
        } else if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        // Unpaced Mailbox rendering with only two images blocked swapchain acquire
        // for 35-58 ms whenever the compositor still held one (measured); a third
        // image keeps a free target available.
        let frame_latency=if test_immediate && present_mode==wgpu::PresentMode::Immediate {1}else{3};
        log::info!("presentation: {present_mode:?}, maximum frame latency: {frame_latency}");
        let config = wgpu::SurfaceConfiguration {
            usage,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: frame_latency,
            alpha_mode: caps.alpha_modes[0],
            view_formats: if format.is_srgb() { vec![format.remove_srgb_suffix()] } else { vec![] },
            color_space: Default::default(),
        };
        surface.configure(&device, &config);
        let mut frame = FrameRenderer::new(&device, format, (config.width, config.height), adapter_name);
        frame.renderer.initialize_pools(&device, &queue);
        frame.renderer.gpu_timer = mc4d_render::gpu_timer::GpuTimer::new(&device, &queue);
        log::info!("GPU pass timing: {}", if frame.renderer.gpu_timer.is_some() {"timestamp queries"} else {"unavailable"});
        // Part of startup, not of the first presented frame.
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let egui_state = egui_winit::State::new(
            frame.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        self.gfx = Some(Gfx { window, surface, device, queue, config, frame, egui_state });

        let game = Game::menu_preview(self.opts.seed);
        if !self.settings.tutorial_done {
            self.ui.tutorial = Some(crate::tutorial::Tutorial::new(&game, &self.settings));
        }
        self.game = Some(game);
        self.audio = crate::audio::Audio::new(self.performance.is_some() || self.opts.smoke_test.is_some());
        self.set_playing(false);
        self.ui.screen = Screen::Title;
        if self.performance.is_some() {
            self.ui.tutorial = None;
            self.begin_world_load(crate::ui::WorldRequest {name:"benchmark".into(),seed:self.opts.seed.into(),mode:self.opts.mode,create:true},false);
        }
        Ok(())
    }

    fn set_grab(&mut self, grab: bool) {
        let Some(gfx) = &self.gfx else { return };
        if self.opts.smoke_test.is_some() || self.performance.is_some() {
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
        if self.ui.screen == Screen::Inventory {
            if let Some(game) = &mut self.game {
                game.return_cursor(&mut self.ui.inventory_cursor);
            }
        }
        self.ui.screen = if playing { Screen::Playing } else { Screen::Pause };
        self.ui.menu_parent = Screen::Pause;
        self.ui.capturing = None;
        self.input.release_all();
        if let Some(game)=&mut self.game{game.release_controls();}
        self.set_grab(playing);
    }

    fn quit(&mut self, event_loop: &ActiveEventLoop) {
        self.write_performance_report(false);
        if let Some(game) = &mut self.game {
            if let Some((pos,lines))=self.ui.sign_editor.take(){game.save_sign_text(pos,lines);}
            game.return_cursor(&mut self.ui.inventory_cursor);
            game.save_all();
        }
        if let Err(error)=self.profile.save(){log::error!("saving statistics: {error}");}
        self.opts.paths.save_settings(&self.settings);
        // Release GPU and windowing state while the display connection is still alive.
        self.gfx = None;
        self.exiting = true;
        event_loop.exit();
    }

    fn write_performance_report(&mut self, completed: bool) {
        if let (Some(perf), Some(gfx), Some(game)) = (&mut self.performance, &self.gfx, &self.game) {
            if let Err(e) = perf.write(completed, &gfx.frame.adapter_name, (gfx.config.width, gfx.config.height), game, &self.settings) {
                log::error!("writing gameplay performance report: {e}");
            }
        }
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
                Screen::Pause | Screen::Inventory | Screen::Chat => self.set_playing(true),
                Screen::Title | Screen::Death | Screen::SignEditor => {},
                Screen::Worlds => self.ui.screen = Screen::Title,
                Screen::CreateWorld | Screen::RenameWorld | Screen::DeleteWorld => {self.ui.world_error=None;self.ui.screen=Screen::Worlds;},
                Screen::Achievements | Screen::Statistics => self.ui.screen = Screen::Pause,
                Screen::Loading => {self.pending_load=None;self.streaming_load=false;self.ui.screen=Screen::Title;}
                Screen::Options | Screen::TexturePacks => self.ui.screen = self.ui.menu_parent,
                Screen::VideoOptions | Screen::FourDOptions | Screen::Controls => self.ui.screen = Screen::Options,
            }
            return true;
        }
        if pressed && !event.repeat && self.ui.screen == Screen::Playing
            && self.bindings.get(Action::Chat).contains(&Binding::Key(code)) {
            self.set_playing(false);
            self.ui.screen = Screen::Chat;
            self.ui.chat_input = if code == KeyCode::Slash { "/".into() } else { String::new() };
            self.ui.chat_focus = true;
            return true;
        }
        if pressed && !event.repeat
            && matches!(self.ui.screen, Screen::Playing | Screen::Inventory)
            && self.bindings.get(Action::Inventory).contains(&Binding::Key(code))
            && !(self.ui.screen == Screen::Inventory && self.gfx.as_ref().is_some_and(|g| g.frame.egui_ctx.egui_wants_keyboard_input())) {
            if self.ui.screen == Screen::Inventory {
                self.set_playing(true);
            } else {
                self.set_playing(false);
                self.ui.screen = Screen::Inventory;
                self.game.as_mut().unwrap().award_achievement(0);
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
        let language = s.language;
        if i.just_pressed(b, Action::ToggleHud) {
            self.ui.show_hud = !self.ui.show_hud;
        }
        if i.just_pressed(b, Action::ToggleDebug) {
            self.ui.show_debug = !self.ui.show_debug;
        }
        let game = self.game.as_mut().unwrap();
        if i.just_pressed(b, Action::TogglePerspective) {game.third_person = !game.third_person;}
        for (action, flag, name) in [
            (Action::ToggleGhosts, &mut s.ghosts, "Ghost layers"),
            (Action::ToggleInset, &mut s.inset, "Side view"),
            (Action::ToggleGlasses, &mut s.glasses, "4D glasses"),
        ] {
            if i.just_pressed(b, action) {
                *flag = !*flag;
                changed = true;
                game.toast(format!("{}: {}", crate::i18n::text(language, name), crate::i18n::text(language, if *flag { "on" } else { "off" })));
            }
        }
        if i.just_pressed(b,Action::CycleRenderDistance) {
            s.render_distance=match s.render_distance {0..=2=>3,3=>4,4=>6,_=>2};changed=true;
            game.toast(format!("{}: {}",crate::i18n::text(s.language,"Render distance"),s.render_distance));
        }
        if i.just_pressed(b, Action::Screenshot) {
            self.screenshot_requested = true;
        }
        if changed {
            self.opts.paths.save_settings(&self.settings);
        }
    }

    fn begin_world_load(&mut self, request: crate::ui::WorldRequest, tutorial:bool) {
        if !crate::settings::valid_world_name(&request.name) {self.ui.world_error=Some("Choose a simple world name without path characters.".into());return;}
        if let Err(error)=self.game.as_mut().unwrap().save_checked() {
            log::error!("save before world switch: {error}");self.ui.world_error=Some("Game could not be saved".into());return;
        }
        self.opts.world_dir=self.opts.paths.saves.join(&request.name);
        let directory=if tutorial {None}else{Some(self.opts.world_dir.clone())};
        if let Some(path)=&directory {
            let result=if request.create {std::fs::create_dir_all(&self.opts.paths.saves).and_then(|_|std::fs::create_dir(path))} else if path.join("level.toml").is_file(){Ok(())}else{Err(std::io::Error::new(std::io::ErrorKind::NotFound,"world no longer exists"))};
            if let Err(error)=result {self.ui.world_error=Some(error.to_string());return;}
        }
        let (sender,receiver)=std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result=Game::try_load(directory,request.seed,request.mode).and_then(|mut game| {
                game.save_checked().map_err(|e|e.to_string())?;Ok(game)
            });
            let _=sender.send(result);
        });
        self.set_playing(false);
        self.ui.screen=Screen::Loading;self.ui.loading_progress=0.0;
        self.pending_load=Some(receiver);self.streaming_load=false;self.tutorial_after_load=tutorial;
        self.ui.tutorial=None;
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.profile_save_timer+=dt;
        if self.profile_save_timer>=120.0 {
            self.profile_save_timer=0.0;
            if let Err(error)=self.profile.save(){log::error!("saving statistics: {error}");}
        }
        let loaded=self.pending_load.as_ref().and_then(|receiver|match receiver.try_recv(){
            Ok(result)=>Some(result),Err(std::sync::mpsc::TryRecvError::Empty)=>None,
            Err(std::sync::mpsc::TryRecvError::Disconnected)=>Some(Err("world loading stopped".into())),
        });
        if let Some(result)=loaded {
            self.pending_load=None;
            match result {
                Ok(mut game)=>{if self.performance.is_some(){game.body.pos.y+=6.0;game.flying=true;}if !self.tutorial_after_load {game.progress=self.profile.state.clone();game.record_counter("sessions",1);self.ui.worlds=crate::worlds::list(&self.opts.paths.saves);}game.update_streaming(&self.settings,true);self.game=Some(game);self.streaming_load=true;}
                Err(error)=>{log::error!("world load: {error}");self.ui.world_error=Some("This world could not be loaded. Its files have been preserved.".into());self.ui.screen=Screen::Worlds;}
            }
        }
        if let Some(perf) = &mut self.performance {
            if perf.begin(now, dt, self.started.elapsed().as_secs_f64() * 1000.0) {
                self.write_performance_report(true);
                self.quit(event_loop);
                return;
            }
            if self.ui.screen==Screen::Playing {
                perf.drive(dt, self.game.as_mut().unwrap(), &mut self.settings, &mut self.input, &self.bindings);
            }else{self.input.release_all();self.input.end_frame();}
        }
        self.fps_timer.0 += dt;
        self.fps_timer.1 += 1;
        if self.fps_timer.0 > 0.5 {
            self.ui.fps = self.fps_timer.1 as f32 / self.fps_timer.0;
            self.fps_timer = (0.0, 0);
        }

        self.ui.dt = dt;
        if self.input.just_pressed(&self.bindings, Action::ToggleFullscreen) {
            self.settings.fullscreen = !self.settings.fullscreen;
            self.opts.paths.save_settings(&self.settings);
        }
        if let Some(gfx) = &self.gfx {
            // Options and F11 share one setting; apply whenever they disagree.
            if gfx.window.fullscreen().is_some() != self.settings.fullscreen && self.performance.is_none() {
                gfx.window.set_fullscreen(self.settings.fullscreen.then_some(winit::window::Fullscreen::Borderless(None)));
            }
        }
        if self.ui.screen == Screen::Playing
            && self.input.just_pressed(&self.bindings, Action::Place)
            && !self.input.is_down(&self.bindings, Action::Sneak)
            && self.game.as_mut().is_some_and(|game| game.open_target_station()) {
            self.set_playing(false);
            self.ui.screen = Screen::Inventory;
        }
        let playing = self.ui.screen == Screen::Playing;
        if playing {
            self.handle_toggles();
        }
        {
            let game = self.game.as_mut().unwrap();
            let input = playing.then_some((&self.input, &self.bindings));
            if matches!(self.ui.screen, Screen::Playing | Screen::Inventory | Screen::Chat) {
                game.update(dt, input, &self.settings);
            } else if self.ui.shows_world() || self.streaming_load {
                game.update_streaming(&self.settings, false);
            }
        }
        if self.ui.screen==Screen::Playing {
            if let Some(editor)=self.game.as_mut().unwrap().take_sign_editor_request() {
                if self.game.as_ref().unwrap().is_alive() {
                    self.set_playing(false);self.ui.sign_editor=Some(editor);self.ui.sign_focus=true;self.ui.screen=Screen::SignEditor;
                }
            }
        }
        if matches!(self.ui.screen,Screen::Playing|Screen::Inventory|Screen::Chat) && self.game.as_ref().is_some_and(|game| !game.is_alive()) {
            self.game.as_mut().unwrap().return_cursor(&mut self.ui.inventory_cursor);
            self.set_playing(false);
            self.ui.screen = Screen::Death;
        }
        if self.streaming_load {
            let game=self.game.as_ref().unwrap();
            let loaded=game.gpu_wanted.iter().filter(|&&c|game.world.is_loaded(c)).count();
            self.ui.loading_progress=0.1+0.8*loaded as f32/game.gpu_wanted.len().max(1) as f32;
        }
        let t_update = Instant::now();

        let gfx = self.gfx.as_mut().unwrap();
        let tex = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                gfx.surface.configure(&gfx.device, &gfx.config);
                if let Some(perf) = &mut self.performance {
                    let game = self.game.as_ref().unwrap();
                    let counts = (game.gpu_wanted.iter().filter(|&&c| !game.world.is_loaded(c)).count(),
                        gfx.frame.renderer.store.missing_dense_count(&game.world, &game.gpu_wanted));
                    perf.observe(game, counts, [(t_update - now).as_secs_f32() * 1000.0, 0.0, 0.0, 0.0], false, gfx.frame.renderer.missing_lighting(),self.ui.shows_world());
                }
                self.input.end_frame();
                return;
            }
            other => {
                log::debug!("skipping frame: {other:?}");
                if let Some(perf) = &mut self.performance {
                    let game = self.game.as_ref().unwrap();
                    let counts = (game.gpu_wanted.iter().filter(|&&c| !game.world.is_loaded(c)).count(),
                        gfx.frame.renderer.store.missing_dense_count(&game.world, &game.gpu_wanted));
                    perf.observe(game, counts, [(t_update - now).as_secs_f32() * 1000.0, 0.0, 0.0, 0.0], false, gfx.frame.renderer.missing_lighting(),self.ui.shows_world());
                }
                self.input.end_frame();
                return;
            }
        };
        mc4d_core::set_background_throttle(self.ui.shows_world() && self.ui.screen!=Screen::Loading);
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
                        game.toast(format!("{} {}", crate::i18n::text(self.settings.language, "Saved screenshot"), path.display()));
                    }
                    Err(e) => game.toast(format!("{}: {e}", crate::i18n::text(self.settings.language, "Screenshot failed"))),
                }
            }
        }
        // pre_present_notify makes winit wait for the compositor's frame callback
        // (Wayland) before the next redraw. That bursty 60 Hz load let the iGPU
        // drop to 400-900 MHz mid-frame (measured), slowing every frame. Unlimited
        // play renders continuously with Mailbox; Limit Framerate keeps pacing.
        if self.settings.limit_framerate { gfx.window.pre_present_notify(); }
        let t_render = Instant::now();
        gfx.queue.present(tex);
        let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1000.0;
        let t = [ms(now, t_update), ms(t_update, t_acquire), ms(t_acquire, t_render), ms(t_render, Instant::now())];
        if self.gpu_probe_enabled {
            // Diagnostic only: serialises CPU and GPU to expose GPU completion time.
            let t_wait=Instant::now();
            let _=gfx.device.poll(wgpu::PollType::wait_indefinitely());
            let gpu=ms(t_wait,Instant::now());
            self.gpu_probe.push((gpu,dt*1000.0,gfx.frame.renderer.render_scale()));
            if self.gpu_probe.len()==120 {
                let n=self.gpu_probe.len() as f32;let mean=|f:fn(&(f32,f32,f32))->f32|self.gpu_probe.iter().map(f).sum::<f32>()/n;
                let worst=self.gpu_probe.iter().map(|p|p.0).fold(0.0,f32::max);
                eprintln!("gpu probe: wait mean {:.1} worst {:.1} ms; interval mean {:.1} ms; scale {:.2}; views {}",mean(|p|p.0),worst,mean(|p|p.1),mean(|p|p.2),self.settings.active_view_count());
                self.gpu_probe.clear();
            }
        }
        if let Some(perf) = &mut self.performance {
            let game = self.game.as_ref().unwrap();
            perf.observe(game, gfx.frame.renderer.store.residency_counts(), t, true, gfx.frame.renderer.missing_lighting(),self.ui.shows_world());
            perf.note_render_scale(gfx.frame.renderer.render_scale(), gfx.frame.gpu_ms);
            perf.note_window(self.window_focused && !self.window_hidden);
        }
        for (avg, v) in self.ui.timings.iter_mut().zip(t) {
            *avg = *avg * 0.95 + v * 0.05;
        }

        let clicked=gfx.frame.egui_ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique("classic-ui-click"))).unwrap_or(false);
        if let Some(audio)=&mut self.audio {
            audio.update(self.game.as_mut().unwrap(),&self.settings,dt,clicked,self.ui.screen==Screen::Playing);
        } else { self.game.as_mut().unwrap().sounds.clear(); }
        if out.settings_changed {
            self.opts.paths.save_settings(&self.settings);
        }
        if out.bindings_changed {
            self.opts.paths.save_bindings(&self.bindings);
        }
        if out.refresh_worlds {self.ui.worlds=crate::worlds::list(&self.opts.paths.saves);}
        if let Some((folder,name))=out.rename_world {
            match crate::worlds::rename(&self.opts.paths.saves,&folder,&name){Ok(())=>{self.ui.worlds=crate::worlds::list(&self.opts.paths.saves);self.ui.world_error=None;self.ui.screen=Screen::Worlds;},Err(e)=>self.ui.world_error=Some(e.to_string())}
        }
        if let Some(folder)=out.delete_world {
            match crate::worlds::delete(&self.opts.paths.saves,&folder){Ok(())=>{self.ui.worlds=crate::worlds::list(&self.opts.paths.saves);self.ui.world_error=None;self.ui.screen=Screen::Worlds;},Err(e)=>self.ui.world_error=Some(e.to_string())}
        }
        if out.start_tutorial {
            self.begin_world_load(crate::ui::WorldRequest {name:"tutorial".into(),seed:1337,mode:GameMode::Creative,create:false},true);
        }
        if let Some(request)=out.load_world {
            self.begin_world_load(request,false);
        }
        if self.streaming_load {
            let game=self.game.as_ref().unwrap();
            let complete=self.gfx.as_ref().unwrap().frame.world_ready(game);
            if complete {
                if let Some(perf)=&mut self.performance {perf.loading_completed(Instant::now());}
                self.streaming_load=false;
                if self.tutorial_after_load || !self.settings.tutorial_done {
                    self.ui.tutorial=Some(crate::tutorial::Tutorial::new(game,&self.settings));
                }
                self.set_playing(true);
            }
        }
        if out.respawn {
            self.game.as_mut().unwrap().respawn();
            self.set_playing(true);
        }
        if out.resume {
            self.set_playing(true);
        }
        if out.return_to_title {
            let game = self.game.as_mut().unwrap();
            game.return_cursor(&mut self.ui.inventory_cursor);
            if let Err(error)=game.save_checked(){game.toast(format!("{}: {error}", crate::i18n::text(self.settings.language, "Game could not be saved")));return;}
            game.record_counter("games_quit",1);
            if let Err(error)=self.profile.save(){log::error!("saving statistics: {error}");}
            self.set_playing(false);
            self.ui.screen=Screen::Title;
            self.ui.tutorial=None;
            self.game=Some(Game::menu_preview(self.opts.seed));
            self.ui.worlds=crate::worlds::list(&self.opts.paths.saves);
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
            WindowEvent::Occluded(hidden) => self.window_hidden = hidden,
            WindowEvent::Focused(focused) => {
                self.window_focused = focused;
                if focused { return; }
                self.input.release_all();
                // The scripted benchmark keeps driving; it records focus instead.
                if self.ui.screen == Screen::Playing && self.performance.is_none() {
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
        // Loading can legitimately extend the route. This watchdog only detects
        // a compositor that stops producing redraws; the harness owns its route
        // and loading deadlines and records every visible frame.
        if self.performance.is_some() && self.last_frame.elapsed().as_secs_f32() > 30.0 {
            log::error!("gameplay benchmark timed out before completing its route (window hidden or display unavailable)");
            self.quit(event_loop);
            return;
        }
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
            if self.settings.limit_framerate && self.performance.is_none() {
                let next=self.last_frame+std::time::Duration::from_secs_f64(1.0/60.0);
                event_loop.set_control_flow(ControlFlow::WaitUntil(next));
                if Instant::now()>=next {gfx.window.request_redraw();}
            } else {event_loop.set_control_flow(ControlFlow::Poll);gfx.window.request_redraw();}
        }
    }
}

/// Minimal boxed-error result so `?` works across winit/wgpu error types.
pub mod anyhow_lite {
    pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
}
