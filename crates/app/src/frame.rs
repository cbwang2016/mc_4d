//! Renders one frame (world + egui HUD/menus) to any texture view. Shared by the window
//! and headless (screenshot / benchmark) modes.

use crate::game::Game;
use crate::input::{Bindings, InputState};
use crate::settings::Settings;
use crate::ui::{self, DebugInfo, UiOutput, UiState};
use mc4d_render::{FrameParams, Renderer, flags};

pub struct FrameRenderer {
    pub renderer: Renderer,
    pub egui_ctx: egui::Context,
    egui_renderer: egui_wgpu::Renderer,
    pub adapter_name: String,
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
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: (u32, u32), adapter_name: String) -> Self {
        let egui_ctx = egui::Context::default();
        egui_ctx.set_visuals(egui::Visuals::dark());
        egui_ctx.set_fonts(fonts());
        Self {
            renderer: Renderer::new(device, format, size),
            // egui blends in gamma space, so it draws through a non-sRGB view of the target.
            egui_renderer: egui_wgpu::Renderer::new(device, format.remove_srgb_suffix(), egui_wgpu::RendererOptions::default()),
            egui_ctx,
            adapter_name,
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
        let world_view = target.create_view(&Default::default());
        let ui_view = target.create_view(&wgpu::TextureViewDescriptor {
            format: Some(target.format().remove_srgb_suffix()),
            ..Default::default()
        });
        self.renderer.resize(device, size, settings.render_scale);
        self.renderer.sync_world(queue, &game.world, game.eye(), &game.gpu_wanted);

        // UI first, so the inset rect and any setting changes apply this frame.
        let debug = DebugInfo {
            chunks_loaded: game.world.chunks.len(),
            chunks_pending: game.world.pending_count(),
            gpu_resident: self.renderer.store.resident_count(),
            gpu_slots: self.renderer.store.slot_count(),
            gpu_wanted: game.gpu_wanted.len(),
            adapter: self.adapter_name.clone(),
        };
        let mut ui_out = UiOutput::default();
        let full = self.egui_ctx.run_pass(raw_input, |ctx| {
            ui_out = ui::draw(ctx, ui, game, settings, bindings, input, &debug);
        });
        let screen_points = egui::vec2(size.0 as f32, size.1 as f32) / pixels_per_point;
        let inset = ui::inset_rect(screen_points);

        let mut fl = 0;
        for (on, bit) in [
            (settings.ghosts, flags::GHOSTS),
            (settings.inset, flags::INSET),
            (settings.shadows, flags::SHADOWS),
            (settings.glasses, flags::GLASSES),
            (settings.edges, flags::EDGES),
        ] {
            if on {
                fl |= bit;
            }
        }
        let entities = game.entity_draws();
        let params = FrameParams {
            eye: game.eye(),
            basis: game.basis(),
            fov_y_deg: settings.fov,
            max_dist: settings.render_distance as f32 * 16.0,
            target: game.target_block(),
            entities: &entities,
            flags: fl,
            ghost_offset: settings.ghost_offset,
            glasses_range: 8.0,
            time: game.time,
            inset: [
                inset.min.x * pixels_per_point,
                inset.min.y * pixels_per_point,
                inset.width() * pixels_per_point,
                inset.height() * pixels_per_point,
            ],
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        self.renderer.render(queue, &mut encoder, &world_view, &params);

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
        for id in &full.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        full.textures_delta.clear();
        (ui_out, full.platform_output)
    }
}

/// Bundled DejaVu fonts (egui 0.37's own font bundle isn't published on crates.io).
fn fonts() -> egui::FontDefinitions {
    use std::sync::Arc;
    let mut defs = egui::FontDefinitions::empty();
    defs.font_data
        .insert("sans".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/DejaVuSans.ttf"))));
    defs.font_data
        .insert("mono".into(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/DejaVuSansMono.ttf"))));
    defs.families.insert(egui::FontFamily::Proportional, vec!["sans".into(), "mono".into()]);
    defs.families.insert(egui::FontFamily::Monospace, vec!["mono".into(), "sans".into()]);
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
