//! Read-only world-sign text cache. One changed sign is rasterized per frame;
//! original bitmap English and bundled Noto Chinese become physical textures.
use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use crate::game::SignText;
use mc4d_core::{block, glam::{IVec4, Vec4}, shapes, world::World};
use mc4d_render::{EntityDraw, Renderer, ViewDraw, MAX_SIGN_TEXTURES, SIGN_TEXTURE_HEIGHT, SIGN_TEXTURE_WIDTH};
use std::collections::{HashMap, HashSet};

const WOOD: &[u8] = include_bytes!("../../render/assets/minecraft-beta-1.5/item/sign.png");
const BITMAP: &[u8] = include_bytes!("../../render/assets/minecraft-beta-1.5/font/default.png");

struct CachedSign { slot: u32, lines: [String; 4], ready: bool }
pub(super) struct SignTextures {
    entries: HashMap<IVec4, CachedSign>, free: Vec<u32>, pack: String,
    rasterizer: Rasterizer,
}
impl SignTextures {
    pub fn new() -> Self {
        Self { entries: HashMap::new(), free: (0..MAX_SIGN_TEXTURES as u32).rev().collect(),
            pack: String::new(), rasterizer: Rasterizer::new("") }
    }
    pub fn sync(&mut self, queue: &wgpu::Queue, renderer: &Renderer, world: &World,
        records: &[SignText], eye: Vec4, views: &[ViewDraw], distance: f32, pack: &str) -> Vec<EntityDraw> {
        if self.pack != pack {
            self.pack = pack.into(); self.rasterizer = Rasterizer::new(pack);
            self.entries.clear(); self.free = (0..MAX_SIGN_TEXTURES as u32).rev().collect();
        }
        let mut seen = HashSet::new();
        let mut selected: Vec<_> = records.iter().filter(|sign| {
            if !seen.insert(sign.position) { return false; }
            let offset = sign.position.as_vec4() + Vec4::splat(0.5) - eye;
            offset.length_squared() <= distance * distance
                && (views.is_empty() || views.iter().any(|view| offset.dot(view.basis.ana).abs() <= 1.5))
                && world.block(sign.position).is_some_and(|id| matches!(id, block::SIGN_POST | block::WALL_SIGN))
        }).collect();
        selected.sort_by(|a, b| (a.position.as_vec4() - eye).length_squared().total_cmp(&(b.position.as_vec4() - eye).length_squared()));
        selected.truncate(MAX_SIGN_TEXTURES);
        let keep: HashSet<_> = selected.iter().map(|s| s.position).collect();
        self.entries.retain(|pos, cached| {
            if keep.contains(pos) { true } else { self.free.push(cached.slot); false }
        });
        let mut budget = 1;
        let mut draws = Vec::with_capacity(selected.len());
        for sign in selected {
            let cached = self.entries.entry(sign.position).or_insert_with(|| CachedSign {
                slot: self.free.pop().expect("bounded sign cache"), lines: std::array::from_fn(|_| String::new()), ready: false,
            });
            if !cached.ready || cached.lines != sign.lines {
                cached.ready = false;
                if budget > 0 {
                    let pixels = self.rasterizer.rasterize(&sign.lines);
                    renderer.upload_sign_texture(queue, cached.slot, &pixels);
                    cached.lines = sign.lines.clone(); cached.ready = true; budget -= 1;
                }
            }
            let state = world.block_state(sign.position).expect("selected loaded sign");
            let standing = state.id == block::SIGN_POST;
            let forward = if standing {
                if state.metadata == 16 { Vec4::W } else if state.metadata == 17 { -Vec4::W }
                else { let angle = (state.metadata & 15) as f32 * std::f32::consts::TAU / 16.0;
                    Vec4::new(-angle.sin(), 0.0, angle.cos(), 0.0) }
            } else { -shapes::wall_support(state.metadata).as_vec4() };
            let layer = if cached.ready { cached.slot + 1 } else { 0 };
            draws.push(EntityDraw { center: sign.position.as_vec4() + Vec4::splat(0.5), radius: 0.5,
                color: [1.0; 3], kind: if standing { 15 } else { 16 }, forward,
                item: 0, item_variant: 0, state: state.metadata as u32 | (layer << 8) });
        }
        draws
    }
}

struct Rasterizer { wood: Vec<u8>, bitmap: Vec<u8>, widths: [u8; 256], classic: FontArc, chinese: FontArc }
impl Rasterizer {
    fn new(pack: &str) -> Self {
        let load = |name: &str, fallback: &[u8]| {
            let external = (!pack.is_empty()).then(|| mc4d_render::pack::read(std::path::Path::new(pack), name)).flatten();
            external.as_deref().and_then(|b| mc4d_render::atlas::decode(b).ok())
                .unwrap_or_else(|| mc4d_render::atlas::decode(fallback).expect("bundled sign asset"))
        };
        let (w, h, wood) = load("item/sign.png", WOOD);
        let mut background = vec![255u8; (SIGN_TEXTURE_WIDTH * SIGN_TEXTURE_HEIGHT * 4) as usize];
        for y in 0..SIGN_TEXTURE_HEIGHT { for x in 0..SIGN_TEXTURE_WIDTH {
            let sx = (((2.0 + x as f32 / SIGN_TEXTURE_WIDTH as f32 * 24.0) / 64.0) * w as f32).floor() as u32;
            let sy = (((2.0 + y as f32 / SIGN_TEXTURE_HEIGHT as f32 * 12.0) / 32.0) * h as f32).floor() as u32;
            let src = ((sy.min(h - 1) * w + sx.min(w - 1)) * 4) as usize;
            let dst = ((y * SIGN_TEXTURE_WIDTH + x) * 4) as usize;
            background[dst..dst + 3].copy_from_slice(&wood[src..src + 3]);
        }}
        let (w, h, raw_font) = load("font/default.png", BITMAP);
        let mut bitmap = vec![0u8; 128 * 128 * 4];
        for y in 0..128 { for x in 0..128 {
            let src = (((y as u32 * h / 128) * w + x as u32 * w / 128) * 4) as usize;
            bitmap[(y * 128 + x) * 4..(y * 128 + x + 1) * 4].copy_from_slice(&raw_font[src..src + 4]);
        }}
        let mut widths = [6u8; 256];
        for code in 0..256 { let mut width = 0;
            for x in 0..8 { for y in 0..8 {
                let src = (((code / 16 * 8 + y) * 128 + code % 16 * 8 + x) * 4) as usize;
                if bitmap[src + 3] > 0 && bitmap[src..src + 3].iter().any(|&v| v > 0) { width = width.max(x + 1); }
            }} widths[code] = (width + 1).clamp(2, 8) as u8;
        }
        widths[32] = 4;
        Self { wood: background, bitmap, widths,
            classic: FontArc::try_from_slice(super::CLASSIC_FONT_BYTES).expect("classic sign font"),
            chinese: FontArc::try_from_slice(super::CJK_FONT_BYTES).expect("Chinese sign font") }
    }
    fn darken(pixels: &mut [u8], x: i32, y: i32, coverage: f32) {
        if x < 0 || y < 0 || x >= SIGN_TEXTURE_WIDTH as i32 || y >= SIGN_TEXTURE_HEIGHT as i32 { return; }
        let i = ((y as u32 * SIGN_TEXTURE_WIDTH + x as u32) * 4) as usize;
        for c in &mut pixels[i..i + 3] { *c = (*c as f32 * (1.0 - coverage.clamp(0.0, 1.0))).round() as u8; }
    }
    fn rasterize(&self, lines: &[String; 4]) -> Vec<u8> {
        let mut pixels = self.wood.clone();
        for (line_index, line) in lines.iter().enumerate() {
            let chars: Vec<_> = line.chars().filter(|c| !c.is_control()).take(15).collect();
            let bitmap_width: u32 = chars.iter().filter(|c| c.is_ascii()).map(|&c| self.widths[c as usize] as u32 * 2).sum();
            if chars.iter().all(|c| c.is_ascii()) && bitmap_width <= 180 {
                let width = bitmap_width;
                let mut x = (SIGN_TEXTURE_WIDTH as i32 - width as i32) / 2;
                let top = 10 + line_index as i32 * 20;
                for c in chars {
                    for y in 0..8 { for px in 0..8 {
                        let src = (((c as usize / 16 * 8 + y) * 128 + c as usize % 16 * 8 + px) * 4) as usize;
                        let coverage = self.bitmap[src + 3] as f32 / 255.0 * self.bitmap[src..src + 3].iter().copied().max().unwrap_or(0) as f32 / 255.0;
                        for dy in 0..2 { for dx in 0..2 { Self::darken(&mut pixels, x + px as i32 * 2 + dx, top + y as i32 * 2 + dy, coverage); }}
                    }}
                    x += self.widths[c as usize] as i32 * 2;
                }
            } else {
                let font = |c: char| if c.is_ascii() { &self.classic } else { &self.chinese };
                let base = 16.0;
                let width: f32 = chars.iter().map(|&c| { let f = font(c); f.as_scaled(base).h_advance(f.glyph_id(c)) }).sum();
                let scale = PxScale::from(base * (180.0 / width.max(1.0)).min(1.0));
                let width: f32 = chars.iter().map(|&c| { let f = font(c); f.as_scaled(scale).h_advance(f.glyph_id(c)) }).sum();
                let mut x = (SIGN_TEXTURE_WIDTH as f32 - width) * 0.5;
                let baseline = 24.0 + line_index as f32 * 20.0;
                for c in chars {
                    let f = font(c); let glyph_id = f.glyph_id(c);
                    let glyph = glyph_id.with_scale_and_position(scale, point(x.round(), baseline));
                    if let Some(outlined) = f.outline_glyph(glyph) {
                        let bounds = outlined.px_bounds();
                        outlined.draw(|gx, gy, coverage| Self::darken(&mut pixels, bounds.min.x as i32 + gx as i32,
                            bounds.min.y as i32 + gy as i32, coverage));
                    }
                    x += f.as_scaled(scale).h_advance(glyph_id);
                }
            }
        }
        pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_ascii_and_chinese_are_rasterized_not_just_saved() {
        let raster = Rasterizer::new("");
        assert_ne!(raster.chinese.glyph_id('维').0, 0);
        let blank = raster.rasterize(&std::array::from_fn(|_| String::new()));
        for text in ["Hello 4D", "你好四维世界"] {
            let lines = [text.into(), "".into(), "".into(), "".into()];
            let pixels = raster.rasterize(&lines);
            let changed = pixels.chunks_exact(4).zip(blank.chunks_exact(4)).filter(|(a, b)| a != b).count();
            assert!(changed > 50, "missing rendered glyphs for {text}: {changed} changed pixels; outline={}", raster.chinese.outline_glyph(raster.chinese.glyph_id('维').with_scale(16.0)).is_some());
        }
    }
}
