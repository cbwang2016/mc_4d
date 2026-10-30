//! Shared classic GUI sprites, pixel-font labels, buttons and sliders.
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, TextureHandle, Vec2};

const FILES: [&str; 12] = ["gui/inventory.png", "gui/crafting.png", "gui/furnace.png", "gui/container.png",
    "gui/gui.png", "font/default.png", "mob/char.png", "gui/icons.png", "item/sign.png", "achievement/bg.png", "terrain.png", "gui/trap.png"];
const DEFAULTS: [&[u8]; 12] = [
    include_bytes!("../../render/assets/minecraft-beta-1.5/gui/inventory.png"),
    include_bytes!("../../render/assets/minecraft-beta-1.5/gui/crafting.png"),
    include_bytes!("../../render/assets/minecraft-beta-1.5/gui/furnace.png"),
    include_bytes!("../../render/assets/minecraft-beta-1.5/gui/container.png"),
    include_bytes!("../../render/assets/minecraft-beta-1.5/gui/gui.png"),
    include_bytes!("../../render/assets/minecraft-beta-1.5/font/default.png"),
    include_bytes!("../../render/assets/minecraft-beta-1.5/mob/char.png"),
    include_bytes!("../../render/assets/minecraft-beta-1.5/gui/icons.png"),
    include_bytes!("../../render/assets/minecraft-beta-1.5/item/sign.png"),
    include_bytes!("../../render/assets/minecraft-beta-1.5/achievement/bg.png"),
    include_bytes!("../../render/assets/minecraft-beta-1.5/terrain.png"),
    include_bytes!("../../render/assets/minecraft-beta-1.5/gui/trap.png"),
];

#[derive(Clone)]
pub(crate) struct Assets { pack: String, pub textures: Vec<TextureHandle>, pub font_widths: [u8; 256] }

pub(crate) fn assets(ctx: &egui::Context, pack: &str) -> Assets {
    let key = egui::Id::unique("classic-gui-assets");
    if let Some(cached) = ctx.data(|d| d.get_temp::<Assets>(key)) && cached.pack == pack { return cached; }
    let mut textures = Vec::new(); let mut font_widths = [6; 256];
    for (index, path) in FILES.iter().enumerate() {
        let external = (!pack.is_empty()).then(|| mc4d_render::pack::read(std::path::Path::new(pack), path)).flatten();
        let decoded = external.as_deref().and_then(|bytes| mc4d_render::atlas::decode(bytes).ok())
            .unwrap_or_else(|| mc4d_render::atlas::decode(DEFAULTS[index]).expect("bundled classic GUI"));
        let (w, h, rgba) = decoded;
        if index == 5 && w == 128 && h == 128 {
            for code in 0..256 {
                let mut width = 0;
                for x in 0..8 { for y in 0..8 {
                    let i = (((code / 16 * 8 + y) * 128) + code % 16 * 8 + x) * 4;
                    if rgba[i + 3] > 0 && rgba[i..i + 3].iter().any(|&v| v > 0) { width = width.max(x + 1); }
                }}
                font_widths[code] = (width + 1).clamp(2, 8) as u8;
            }
            font_widths[32] = 4;
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
        textures.push(ctx.load_texture(format!("classic-{path}"), image, egui::TextureOptions::NEAREST));
    }
    let out = Assets { pack: pack.into(), textures, font_widths };
    ctx.data_mut(|d| d.insert_temp(key, out.clone())); out
}

pub(crate) fn classic_text(p: &egui::Painter, a: &Assets, text: &str, pos: Pos2, scale: f32, color: Color32, shadow: bool) {
    if !text.is_ascii() {
        if shadow { p.text(pos + Vec2::splat(scale), Align2::LEFT_TOP, text, FontId::proportional(9.0 * scale), Color32::from_black_alpha(190)); }
        p.text(pos, Align2::LEFT_TOP, text, FontId::proportional(9.0 * scale), color); return;
    }
    let mut x = pos.x; let mut y = pos.y;
    for c in text.bytes() {
        if c == b'\n' { x = pos.x; y += 9.0 * scale; continue; }
        if c != b' ' {
            let uv = Rect::from_min_max(Pos2::new((c % 16) as f32 / 16.0, (c / 16) as f32 / 16.0),
                Pos2::new((c % 16 + 1) as f32 / 16.0, (c / 16 + 1) as f32 / 16.0));
            let r = Rect::from_min_size(Pos2::new(x, y), Vec2::splat(8.0 * scale));
            if shadow { p.image(a.textures[5].id(), r.translate(Vec2::splat(scale)), uv, Color32::from_rgb(63, 63, 63)); }
            p.image(a.textures[5].id(), r, uv, color);
        }
        x += a.font_widths[c as usize] as f32 * scale;
    }
}
pub(crate) fn text_width(a: &Assets, text: &str, scale: f32) -> f32 {
    text.lines().map(|line| line.chars().map(|c| if c.is_ascii() {
        a.font_widths[c as usize] as f32 * scale
    } else { 9.0 * scale }).sum::<f32>()).fold(0.0, f32::max)
}

pub(crate) fn region(p: &egui::Painter, texture: &TextureHandle, dest: Rect, source: [f32; 4]) {
    let size = texture.size_vec2();
    let uv = Rect::from_min_max(Pos2::new(source[0] / size.x, source[1] / size.y),
        Pos2::new((source[0] + source[2]) / size.x, (source[1] + source[3]) / size.y));
    p.image(texture.id(), dest, uv, Color32::WHITE);
}


pub(crate) fn gui_scale(ctx: &egui::Context) -> f32 {
    let screen = ctx.content_rect();
    let maximum = ((screen.width() / 320.0).min(screen.height() / 240.0).floor()).clamp(1.0, 4.0) as u32;
    let selected = ctx.data(|d| d.get_temp::<u32>(egui::Id::unique("game-gui-scale"))).unwrap_or(0);
    if selected == 0 { maximum as f32 } else { selected.clamp(1, maximum) as f32 }
}

pub(crate) fn set_pack(ctx: &egui::Context, pack: &str) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("classic-gui-pack"), pack.to_owned()));
}
pub(crate) fn current_assets(ctx: &egui::Context) -> Assets {
    let pack = ctx.data(|d| d.get_temp::<String>(egui::Id::unique("classic-gui-pack"))).unwrap_or_default();
    assets(ctx, &pack)
}
pub(crate) fn text(painter: &egui::Painter, text: &str, pos: Pos2, scale: f32, color: Color32, shadow: bool) {
    classic_text(painter, &current_assets(painter.ctx()), text, pos, scale, color, shadow);
}

pub(crate) fn button_at(ui: &mut egui::Ui, a: &Assets, rect: Rect, id: egui::Id, label: &str, scale: f32) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    let source_y = if !ui.is_enabled() { 46.0 } else if response.hovered() { 86.0 } else { 66.0 };
    button_background(ui.painter(), a, rect, source_y, scale);
    let color = if !ui.is_enabled() { Color32::from_gray(160) } else if response.hovered() { Color32::from_rgb(255, 255, 160) } else { Color32::from_gray(224) };
    classic_text(ui.painter(), a, label, Pos2::new(rect.center().x - text_width(a, label, scale) * 0.5,
        rect.center().y - 4.0 * scale), scale, color, true);
    if response.clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("classic-ui-click"), true));
    }
    response
}
fn button_background(p: &egui::Painter, a: &Assets, rect: Rect, source_y: f32, scale: f32) {
    let edge = 2.0 * scale;
    region(p, &a.textures[4], Rect::from_min_size(rect.min, Vec2::new(edge, rect.height())), [0.0, source_y, 2.0, 20.0]);
    region(p, &a.textures[4], Rect::from_min_max(rect.min + Vec2::new(edge, 0.0), rect.max - Vec2::new(edge, 0.0)), [2.0, source_y, 196.0, 20.0]);
    region(p, &a.textures[4], Rect::from_min_size(Pos2::new(rect.max.x - edge, rect.min.y), Vec2::new(edge, rect.height())), [198.0, source_y, 2.0, 20.0]);
}

pub(crate) fn button(ui: &mut egui::Ui, label: &str, width: f32, height: f32) -> egui::Response {
    let (id, rect) = ui.allocate_space(Vec2::new(width, height));
    let a = current_assets(ui.ctx());
    button_at(ui, &a, rect, id, label, height / 20.0)
}

pub(crate) fn slider(ui: &mut egui::Ui, label: &str, width: f32, height: f32, value: &mut f32,
    range: std::ops::RangeInclusive<f32>) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let scale = height / 20.0; let a = current_assets(ui.ctx());
    let (lo, hi) = (*range.start(), *range.end());
    if (response.dragged() || response.clicked()) && let Some(pointer) = response.interact_pointer_pos() {
        let fraction = ((pointer.x - rect.min.x - 4.0 * scale) / (width - 8.0 * scale).max(1.0)).clamp(0.0, 1.0);
        let next = lo + fraction * (hi - lo);
        if *value != next { *value = next; response.mark_changed(); }
    }
    button_background(ui.painter(), &a, rect, 46.0, scale);
    let fraction = if hi > lo { ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    let x = rect.min.x + fraction * (width - 8.0 * scale);
    region(ui.painter(), &a.textures[4], Rect::from_min_size(Pos2::new(x, rect.min.y), Vec2::new(4.0 * scale, height)), [0.0, 66.0, 4.0, 20.0]);
    region(ui.painter(), &a.textures[4], Rect::from_min_size(Pos2::new(x + 4.0 * scale, rect.min.y), Vec2::new(4.0 * scale, height)), [196.0, 66.0, 4.0, 20.0]);
    classic_text(ui.painter(), &a, label, Pos2::new(rect.center().x - text_width(&a, label, scale) * 0.5,
        rect.center().y - 4.0 * scale), scale, Color32::from_gray(224), true);
    response
}
