//! Renders one frame (world + egui HUD/menus) to any texture view. Shared by the window
//! and headless (screenshot / benchmark) modes.

use crate::game::Game;
use crate::input::{Bindings, InputState};
use crate::settings::Settings;
use crate::ui::{self, DebugInfo, UiOutput, UiState};
use mc4d_render::{FrameParams, Renderer, ViewDraw, flags};
use mc4d_core::glam::Vec4;

#[path = "sign_textures.rs"]
mod sign_textures;
pub(crate) const CJK_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-Regular.ttf");
pub(crate) const CLASSIC_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/ClassicPixel.ttf");

pub struct FrameRenderer {
    pub renderer: Renderer,
    pub egui_ctx: egui::Context,
    egui_renderer: egui_wgpu::Renderer,
    pub adapter_name: String,
    sign_textures: sign_textures::SignTextures,
    icon_textures: [egui::TextureHandle; 2],
    applied_texture_pack: String,
    instruments:mc4d_render::instrument_fx::InstrumentFx,
    weather: crate::weather::WeatherField,
    /// Latest ray-casting pass GPU time (timestamp queries), for F3 and reports.
    pub gpu_ms: Option<f32>,
}

pub struct FrameInput<'a> {
    pub game: &'a mut Game,
    pub settings: &'a mut Settings,
    pub bindings: &'a mut Bindings,
    pub input: &'a InputState,
    pub ui: &'a mut UiState,
    pub raw_input: egui::RawInput,
    pub size: (u32, u32),
    pub pixels_per_point: f32,
}

impl FrameRenderer {
    /// The same readiness gate is used by the window loader and its GPU test.
    /// Loading screens synchronize the world without displaying unfinished rays.
    pub fn world_ready(&self,game:&Game)->bool {
        !game.gpu_wanted.is_empty()
            && game.gpu_wanted.iter().all(|&c|game.world.is_loaded(c))
            && self.renderer.store.missing_dense_count(&game.world,&game.gpu_wanted)==0
            && game.world.lighting_ready(&game.gpu_wanted)
            && self.renderer.missing_lighting()==0
    }

    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32), adapter_name: String) -> Self {
        let egui_ctx = egui::Context::default();
        egui_ctx.set_visuals(egui::Visuals::dark());
        egui_ctx.style_mut_of(egui::Theme::Dark,|style| style.animation_time=0.0);
        egui_ctx.set_fonts(fonts());
        let icon_textures = icon_textures(&egui_ctx, &mc4d_render::atlas::load(None).expect("bundled atlas"));
        let renderer=Renderer::new(device, format, size);
        Self {
            renderer,
            // egui blends in gamma space, so it draws through a non-sRGB view of the target.
            egui_renderer: egui_wgpu::Renderer::new(device, format.remove_srgb_suffix(), egui_wgpu::RendererOptions::default()),
            egui_ctx,
            adapter_name,
            sign_textures: sign_textures::SignTextures::new(),
            icon_textures,
            instruments:mc4d_render::instrument_fx::InstrumentFx::new(&mc4d_render::atlas::load(None).expect("original items")),
            applied_texture_pack: String::new(),
            weather: crate::weather::WeatherField::new(),
            gpu_ms: None,
        }
    }

    /// Draw the world and UI into `target`, which must allow a view in its non-sRGB format
    /// (see `view_formats`). Returns the UI's requests and egui's platform
    /// output (cursor, clipboard...).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::Texture,
        f: FrameInput,
    ) -> (UiOutput, egui::PlatformOutput) {
        let FrameInput { game, settings, bindings, input, ui, raw_input, size, pixels_per_point } = f;
        if self.applied_texture_pack != settings.texture_pack {
            let folder = (!settings.texture_pack.is_empty()).then(|| std::path::Path::new(&settings.texture_pack));
            match mc4d_render::atlas::load(folder) {
                Ok(pixels) => {
                    self.icon_textures = icon_textures(&self.egui_ctx, &pixels);
                    self.instruments=mc4d_render::instrument_fx::InstrumentFx::new(&pixels);
                    // The renderer validates the same folder and retains the old pack on error.
                    if let Some(folder) = folder {
                        ui.texture_error = self.renderer.set_texture_pack(folder).err();
                    } else {
                        ui.texture_error = self.renderer.reset_texture_pack().err();
                    }
                }
                Err(error) => ui.texture_error = Some(error),
            }
            self.applied_texture_pack = settings.texture_pack.clone();
        }
        if let Some(tiles)=self.instruments.update(game.time,game.compass_angle(),game.environment.celestial_angle()) {
            for (tile,pixels) in [54usize,70].into_iter().zip(&tiles){self.icon_textures[1].set_partial([tile%16*16,tile/16*16],egui::ColorImage::from_rgba_unmultiplied([16,16],pixels),egui::TextureOptions::NEAREST);}
            self.renderer.set_instruments(tiles);
        }
        if let Some(columns)=self.weather.update(&game.world,game.camera_eye(),game.environment.rain>0.001 || game.environment.raining) {
            self.renderer.set_weather_field(queue,columns.origin.to_array(),&columns.values);
        }
        let world_view = target.create_view(&Default::default());
        let ui_view = target.create_view(&wgpu::TextureViewDescriptor {
            format: Some(target.format().remove_srgb_suffix()),
            ..Default::default()
        });
        let render_scale=settings.render_scale;
        self.renderer.resize(device, size, render_scale);
        if ui.shows_world() || ui.screen == crate::ui::Screen::Loading {
        self.renderer.sync_world(device, queue, &game.world, game.camera_eye(), &game.gpu_wanted);
        }

        // UI first, so the inset rect and any setting changes apply this frame.
        let debug = DebugInfo {
            chunks_loaded: game.world.chunks.len(),
            chunks_pending: game.world.pending_count(),
            lighting_pending: game.world.lighting_pending(),
            lighting_missing: self.renderer.missing_lighting(),
            gpu_resident: self.renderer.store.resident_count(),
            gpu_slots: self.renderer.store.slot_count(),
            gpu_wanted: game.gpu_wanted.len(),
            adapter: self.adapter_name.clone(),
            render_scale,
            gpu_ms: self.gpu_ms,
        };
        let mut ui_out = UiOutput::default();
        let full = self.egui_ctx.run_pass(raw_input, |ctx| {
            ui_out = ui::draw(ctx, ui, game, settings, bindings, input, &debug);
        });
        let screen_points = egui::vec2(size.0 as f32, size.1 as f32) / pixels_per_point;
        let count = settings.active_view_count();
        let views: Vec<_> = mc4d_core::views::slice_bases(game.basis(), count).into_iter()
            .zip(mc4d_core::views::view_rects([screen_points.x, screen_points.y], count))
            .map(|(basis, rect)| ViewDraw { basis, rect: rect.map(|v| v * pixels_per_point) })
            .collect();

        let mut fl = 0;
        for (on, bit) in [
            (settings.ghosts, flags::GHOSTS),
            (settings.inset, flags::INSET),
            (settings.shadows, flags::SHADOWS),
            (settings.glasses, flags::GLASSES),
            (settings.edges, flags::EDGES),
            (settings.smooth_lighting, flags::SMOOTH_LIGHTING),
            (settings.fancy_graphics, flags::FANCY),
            (settings.anaglyph, flags::ANAGLYPH),
            (game.environment.thundering, flags::THUNDER),
            (game.burning() && !game.third_person,flags::BURNING),
            (!settings.chunk_culling, flags::DISABLE_CULLING),
        ] {
            if on {
                fl |= bit;
            }
        }
        let mut entities = game.entity_draws();
        if game.third_person {
            entities.push(mc4d_render::EntityDraw {center:game.body.pos,radius:0.9375,color:[1.0;3],kind:22,forward:game.basis().forward_h,item:game.hotbar[game.selected].map_or(0,|s|s.block),item_variant:game.hotbar[game.selected].map_or(0,|s|s.variant),state:player_armor_bits(&game.armor) | (if Vec4::new(game.body.vel.x,0.0,game.body.vel.z,game.body.vel.w).length_squared()>0.1{64}else{0}) | (((128.0+game.orient.pitch/std::f32::consts::FRAC_PI_2*127.0).round().clamp(1.0,255.0)as u32)<<8)});
        }
        entities.extend(self.sign_textures.sync(queue, &self.renderer, &game.world, game.sign_texts(),
            game.camera_eye(), &views, settings.render_distance as f32 * 16.0, &settings.texture_pack));
        let precipitation = game.environment.rain;
        let phase=(game.stats.moved_slice+game.stats.moved_ana)*4.5;
        let speed=Vec4::new(game.body.vel.x,0.0,game.body.vel.z,game.body.vel.w).length().min(4.4)/4.4;
        let bob=if settings.view_bobbing && game.body.on_ground && !game.flying {
            Vec4::Y*(-phase.sin().abs()*0.04*speed)+game.basis().right*(phase.cos()*0.025*speed)
        } else {Vec4::ZERO};
        let (held_stack,equip,swing)=game.first_person_hand();
        let light=game.world.light_at(game.eye().floor().as_ivec4()).unwrap_or((0,15));
        let hand=(!game.third_person && ui.show_hud && game.is_alive() && ui.shows_world()
            && !ui.tutorial.as_ref().is_some_and(|t| t.is_conceptual())).then_some(mc4d_render::FirstPersonDraw {
            stack:held_stack,equip,swing,light:[light.0,light.1],bob:[bob.dot(game.basis().right),bob.y],
        });
        let particle_draws=game.particle_draws();
        let params = FrameParams { particles:&particle_draws,
            first_person: hand,
            eye: game.camera_eye()+if game.third_person {Vec4::ZERO}else{bob},
            basis: game.basis(),
            fov_y_deg: settings.fov,
            max_dist: settings.render_distance as f32 * 16.0,
            target: game.target_block(),
            entities: &entities,
            flags: fl,
            ghost_offset: settings.ghost_offset,
            glasses_range: 8.0,
            time: game.time,
            break_progress: game.mining_progress,
            sun: game.environment.sun(),
            atmosphere: [game.environment.daylight(), precipitation, if game.body.pos.y >= 92.0 { 1.0 } else { 0.0 }, game.environment.lightning],
            views: &views,
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        if ui.shows_world() {self.renderer.render(queue, &mut encoder, &world_view, &params);}

        let mut full = full;
        let prims = self.egui_ctx.tessellate(std::mem::take(&mut full.shapes), full.pixels_per_point);
        for (id, deltas) in &full.textures_delta.set {
            for delta in deltas {
                self.egui_renderer.update_texture(device, queue, *id, delta);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [size.0, size.1], pixels_per_point: full.pixels_per_point };
        let extra = self.egui_renderer.update_buffers(device, queue, &mut encoder, &prims, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ui_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.egui_renderer.render(&mut pass.forget_lifetime(), &prims, &screen);
        }
        queue.submit(extra.into_iter().chain([encoder.finish()]));
        if let Some(timer)=&mut self.renderer.gpu_timer {
            timer.after_submit(device);
            if let Some(ms)=timer.take() {self.gpu_ms=Some(ms);}
        }
        for id in &full.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        full.textures_delta.clear();
        (ui_out, full.platform_output)
    }
}

fn icon_textures(ctx: &egui::Context, pixels: &[u8]) -> [egui::TextureHandle; 2] {
    let layer_bytes = 256 * 256 * 4;
    let handles = std::array::from_fn(|layer| ctx.load_texture(format!("inventory-atlas-{layer}"),
        egui::ColorImage::from_rgba_unmultiplied([256, 256], &pixels[layer * layer_bytes..(layer + 1) * layer_bytes]),
        egui::TextureOptions::NEAREST));
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("inventory-atlas-ids"), [handles[0].id(), handles[1].id()]));
    handles
}

/// Bundled DejaVu fonts (egui 0.37's own font bundle isn't published on crates.io).
fn fonts() -> egui::FontDefinitions {
    use std::sync::Arc;
    let mut defs = egui::FontDefinitions::empty();
    defs.font_data.insert("classic".into(), Arc::new(egui::FontData::from_static(CLASSIC_FONT_BYTES)));
    defs.font_data
        .insert("sans".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/DejaVuSans.ttf"))));
    defs.font_data
        .insert("mono".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/DejaVuSansMono.ttf"))));
    defs.font_data
        .insert("chinese".into(), Arc::new(egui::FontData::from_static(CJK_FONT_BYTES)));
    defs.families.insert(egui::FontFamily::Proportional, vec!["classic".into(), "chinese".into(), "sans".into(), "mono".into()]);
    defs.families.insert(egui::FontFamily::Monospace, vec!["classic".into(), "chinese".into(), "mono".into(), "sans".into()]);
    defs
}

/// Copy a texture to CPU memory as tightly packed RGBA8 rows.
pub fn read_texture(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture) -> Vec<u8> {
    let (w, h) = (tex.width(), tex.height());
    let row = w * 4;
    let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
        },
        tex.size(),
    );
    queue.submit([enc.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    let data = buf.slice(..).get_mapped_range().expect("mapped range");
    let bgra = matches!(tex.format(), wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
    let mut out = Vec::with_capacity((row * h) as usize);
    for y in 0..h {
        let start = (y * padded) as usize;
        let line = &data[start..start + row as usize];
        if bgra {
            for px in line.as_chunks::<4>().0 {
                out.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        } else {
            out.extend(line.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]));
        }
    }
    out
}

pub fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(rgba).map_err(std::io::Error::other)?;
    Ok(())
}

#[cfg(test)]
mod loader_tests {
    use super::*;
    use crate::game::GameMode;
    #[test]
    #[ignore="requires GPU; actual async world load must upload and become ready behind the Loading screen"]
    fn opaque_loading_screen_uploads_a_real_world_before_playing() {
        let size=(640,480);let gpu=crate::headless_gpu(size).unwrap();
        let mut frame=FrameRenderer::new(&gpu.device,gpu.target.format(),size,gpu.adapter_name);
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/loader-review");
        let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let world_path=root.join(format!("world-{stamp}"));std::fs::create_dir_all(&world_path).unwrap();
        let (sender,receiver)=std::sync::mpsc::channel();
        let worker=std::thread::spawn(move||{
            let result=Game::try_load(Some(world_path),1337,GameMode::Creative).and_then(|mut game|{
                game.save_checked().map_err(|e|e.to_string())?;Ok(game)
            });
            sender.send(result).unwrap();
        });
        let mut game=Game::menu_preview(1337);
        let mut settings=Settings{render_distance:2,..Default::default()};
        let mut state=UiState{screen:ui::Screen::Loading,..Default::default()};
        let mut bindings=Bindings::default();let input=InputState::default();
        let start=std::time::Instant::now();let mut loaded=false;let mut loading_frames=0;
        assert!(!state.shows_world());assert!(!frame.world_ready(&game));
        loop {
            if !loaded && let Ok(result)=receiver.try_recv(){game=result.unwrap();loaded=true;}
            if loaded {game.update_streaming(&settings,false);}
            frame.render(&gpu.device,&gpu.queue,&gpu.target,FrameInput{game:&mut game,settings:&mut settings,
                bindings:&mut bindings,input:&input,ui:&mut state,raw_input:crate::raw_input(size),size,pixels_per_point:1.0});
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();loading_frames+=1;
            assert_eq!(state.screen,ui::Screen::Loading);
            if loaded && frame.world_ready(&game){break;}
            assert!(start.elapsed().as_secs()<60,"loading never reached readiness: {} pending geometry, {} pending light, {} GPU light",game.world.pending_count(),game.world.lighting_pending(),frame.renderer.missing_lighting());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        worker.join().unwrap();assert!(loading_frames>0);
        state.screen=ui::Screen::Playing;
        frame.render(&gpu.device,&gpu.queue,&gpu.target,FrameInput{game:&mut game,settings:&mut settings,
            bindings:&mut bindings,input:&input,ui:&mut state,raw_input:crate::raw_input(size),size,pixels_per_point:1.0});
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();assert!(frame.world_ready(&game));
        write_png(&root.join("first-complete-frame.png"),size.0,size.1,&read_texture(&gpu.device,&gpu.queue,&gpu.target)).unwrap();
    }
}

/// Worn armour for the third-person model: 3 bits per slot (helmet, chest,
/// legs, boots), 0 = none, 1..5 = cloth, chain, iron, diamond, gold (item id order).
fn player_armor_bits(armor:&[Option<mc4d_core::item::Stack>;4])->u32 {
    armor.iter().enumerate().fold(0,|bits,(i,slot)|{
        let material=slot.filter(|s|(mc4d_core::item::LEATHER_HELMET..=mc4d_core::item::LEATHER_HELMET+19).contains(&s.block))
            .map_or(0,|s|(s.block-mc4d_core::item::LEATHER_HELMET)/4+1) as u32;
        bits | material<<(16+3*i)
    })
}

#[cfg(test)]
mod armor_gpu_tests {
    use super::*;
    use crate::game::{Game, Slot};
    use crate::input::{Bindings, InputState};
    use crate::settings::Settings;
    use crate::ui::{Screen, UiState};
    #[test]
    fn armour_bits_follow_item_material_order() {
        let mut armor = [None; 4];
        assert_eq!(player_armor_bits(&armor), 0);
        armor[0] = Some(Slot::new(mc4d_core::item::DIAMOND_HELMET, 1));
        armor[3] = Some(Slot::new(mc4d_core::item::LEATHER_BOOTS, 1));
        armor[2] = Some(Slot::new(mc4d_core::item::GOLD_HELMET + 2, 1));
        assert_eq!(player_armor_bits(&armor), (4 << 16) | (5 << 22) | (1 << 25));
    }
    #[test]
    #[ignore = "requires GPU; renders the third-person player with and without armour into target/render-review"]
    fn third_person_player_shows_worn_armour() {
        let size = (480, 360);
        let gpu = crate::headless_gpu(size).unwrap();
        let mut frame = FrameRenderer::new(&gpu.device, gpu.target.format(), size, gpu.adapter_name);
        let mut game = Game::menu_preview(7);
        game.third_person = true;
        let mut settings = Settings { show_hints: false, inset: false, ..Settings::default() };
        let mut bindings = Bindings::default();
        let input = InputState::default();
        let mut ui = UiState { screen: Screen::Playing, show_hud: false, show_debug: false, ..Default::default() };
        let mut render = |game: &mut Game| {
            for _ in 0..600 {
                game.update_streaming(&settings, false);
                frame.render(&gpu.device, &gpu.queue, &gpu.target, FrameInput { game, settings: &mut settings, bindings: &mut bindings, input: &input, ui: &mut ui, raw_input: crate::raw_input(size), size, pixels_per_point: 1.0 });
                gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                if frame.world_ready(game) { break; }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            frame.render(&gpu.device, &gpu.queue, &gpu.target, FrameInput { game, settings: &mut settings, bindings: &mut bindings, input: &input, ui: &mut ui, raw_input: crate::raw_input(size), size, pixels_per_point: 1.0 });
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            read_texture(&gpu.device, &gpu.queue, &gpu.target)
        };
        let plain = render(&mut game);
        game.armor = [mc4d_core::item::DIAMOND_HELMET, mc4d_core::item::DIAMOND_HELMET + 1, mc4d_core::item::DIAMOND_HELMET + 2, mc4d_core::item::DIAMOND_HELMET + 3].map(|id| Some(Slot::new(id, 1)));
        let armoured = render(&mut game);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
        std::fs::create_dir_all(&root).unwrap();
        write_png(&root.join("third-person-plain.png"), size.0, size.1, &plain).unwrap();
        write_png(&root.join("third-person-diamond.png"), size.0, size.1, &armoured).unwrap();
        let cyan = |rgba: &[u8]| rgba.chunks_exact(4).filter(|p| p[2] as i32 > p[0] as i32 + 40 && p[1] as i32 > p[0] as i32 + 30).count();
        let changed = plain.chunks_exact(4).zip(armoured.chunks_exact(4)).filter(|(a, b)| a != b).count();
        assert!(changed > 800, "armour must visibly change the player model: {changed} pixels");
        assert!(cyan(&armoured) > cyan(&plain) + 300, "diamond armour adds its cyan texture");
        // Viewed from above the shoulder: straight behind, the arm hides the hand.
        game.orient.add_pitch(-0.9);
        let empty_hand = render(&mut game);
        game.hotbar[game.selected] = Some(Slot::new(mc4d_core::item::DIAMOND_SWORD, 1));
        let holding = render(&mut game);
        write_png(&root.join("third-person-held-sword.png"), size.0, size.1, &holding).unwrap();
        let diff: Vec<usize> = holding.chunks_exact(4).zip(empty_hand.chunks_exact(4)).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect();
        let (xs, ys): (Vec<_>, Vec<_>) = diff.iter().map(|i| (i % size.0 as usize, i / size.0 as usize)).unzip();
        eprintln!("held item pixels {} x {:?}..{:?} y {:?}..{:?}", diff.len(), xs.iter().min(), xs.iter().max(), ys.iter().min(), ys.iter().max());
        assert!(diff.len() > 25, "the held item is drawn in the hand");
    }
}
