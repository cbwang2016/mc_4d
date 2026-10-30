//! Pixel-coordinate inventory screens using the original Beta GUI atlases.
//! The player, crafting table, furnace and chest layouts keep their original
//! 176-pixel width and 18-pixel slot spacing. Creative uses a paged extension
//! with the same slots and frame, rather than an unrelated widget dashboard.
use super::{UiState, UiOutput};
use crate::game::{Game, GameMode, Slot, StationKind};
use crate::i18n::{Language, text as tr};
use crate::settings::Settings;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};
use mc4d_core::{block, item};
use mc4d_render::atlas_icons::{self, AtlasIcon, BlockFace};

use super::classic_ui::{self, Assets, assets, classic_text, text_width, region};

fn quad(p: &egui::Painter, texture: egui::TextureId, icon: AtlasIcon, positions: [Pos2; 4], face_uv: [[f32; 2]; 4], brightness: f32) {
    let uv = icon.uv();
    let tint = Color32::from_rgb((icon.tint[0] * brightness * 255.0) as u8,
        (icon.tint[1] * brightness * 255.0) as u8, (icon.tint[2] * brightness * 255.0) as u8);
    let uv = face_uv.map(|f| Pos2::new(uv[0][0] + (uv[1][0] - uv[0][0]) * f[0],
        uv[0][1] + (uv[1][1] - uv[0][1]) * f[1]));
    let mut mesh = egui::Mesh::with_texture(texture);
    for i in 0..4 { mesh.vertices.push(egui::epaint::Vertex { pos: positions[i], uv: uv[i], color: tint }); }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]); p.add(egui::Shape::mesh(mesh));
}

pub(super) fn paint_stack_icon(ctx: &egui::Context, painter: &egui::Painter, rect: Rect, stack: Slot) {
    let textures = ctx.data(|data| data.get_temp::<[egui::TextureId; 2]>(egui::Id::unique("inventory-atlas-ids")));
    let (Some(textures), Some(icon)) = (textures, atlas_icons::item_icon_stack(stack)) else { return };
    let cube_icon = item::placement(stack).is_some_and(|state| matches!(block::definition(state.id).shape,
        block::BlockShape::Cube | block::BlockShape::Slab | block::BlockShape::Stairs | block::BlockShape::Bed
            | block::BlockShape::Cactus | block::BlockShape::Fence | block::BlockShape::Cake
            | block::BlockShape::SnowLayer | block::BlockShape::PressurePlate));
    if icon.layer == 0 && cube_icon {
        let top = atlas_icons::block_icon_stack(stack, BlockFace::Top).unwrap_or(icon);
        let side = atlas_icons::block_icon_stack(stack, BlockFace::Side).unwrap_or(icon);
        let front = atlas_icons::block_icon_stack(stack, BlockFace::Front).unwrap_or(icon);
        let state = item::placement(stack).expect("block icon has a placement");
        let connections = if block::definition(state.id).shape == block::BlockShape::Fence { 3 } else { 0 };
        let mut boxes = mc4d_core::shapes::visual_boxes(state, connections).as_slice().to_vec();
        boxes.retain(|b| b.min.w <= 0.5 && b.max.w >= 0.5);
        boxes.sort_by(|a, b| (a.min.x + a.max.x + a.min.y + a.max.y + a.min.z + a.max.z)
            .total_cmp(&(b.min.x + b.max.x + b.min.y + b.max.y + b.min.z + b.max.z)));
        let v = |x: f32, y: f32, z: f32| rect.min + rect.size() * Vec2::new(0.5 + (x - z) * 0.5, (x + z) * 0.25 + (1.0 - y) * 0.5);
        for b in boxes {
            let (x0, x1, y0, y1, z0, z1) = (b.min.x, b.max.x, b.min.y, b.max.y, b.min.z, b.max.z);
            quad(painter, textures[0], top, [v(x0,y1,z0),v(x1,y1,z0),v(x1,y1,z1),v(x0,y1,z1)],
                [[x0,z0],[x1,z0],[x1,z1],[x0,z1]], 1.0);
            quad(painter, textures[0], side, [v(x0,y1,z1),v(x1,y1,z1),v(x1,y0,z1),v(x0,y0,z1)],
                [[x0,1.0-y1],[x1,1.0-y1],[x1,1.0-y0],[x0,1.0-y0]], 0.65);
            quad(painter, textures[0], front, [v(x1,y1,z1),v(x1,y1,z0),v(x1,y0,z0),v(x1,y0,z1)],
                [[1.0-z1,1.0-y1],[1.0-z0,1.0-y1],[1.0-z0,1.0-y0],[1.0-z1,1.0-y0]], 0.85);
        }
    } else {
        let uv = icon.uv();
        painter.image(textures[icon.layer as usize], rect, Rect::from_min_max(Pos2::from(uv[0]), Pos2::from(uv[1])), Color32::WHITE);
    }
}

fn stack(p: &egui::Painter, a: &Assets, rect: Rect, value: Slot, scale: f32) {
    paint_stack_icon(p.ctx(), p, rect, value);
    if value.count > 1 {
        let text = value.count.to_string();
        classic_text(p, a, &text, Pos2::new(rect.max.x - text_width(a, &text, scale) + scale, rect.max.y - 7.0 * scale), scale, Color32::WHITE, true);
    }
    let maximum = item::max_durability(value.block);
    if maximum > 0 && value.damage > 0 {
        let fraction = 1.0 - value.damage as f32 / maximum as f32;
        let r = Rect::from_min_size(rect.min + Vec2::new(2.0, 13.0) * scale, Vec2::new(13.0, 2.0) * scale);
        p.rect_filled(r, 0.0, Color32::BLACK);
        p.rect_filled(Rect::from_min_size(r.min, Vec2::new((fraction * 13.0).round().max(0.0) * scale, scale)), 0.0,
            Color32::from_rgb((255.0 * (1.0 - fraction)) as u8, (255.0 * fraction) as u8, 0));
    }
}

fn small_button(ui: &mut egui::Ui, a: &Assets, rect: Rect, id: &str, label: &str, scale: f32) -> bool {
    classic_ui::button_at(ui, a, rect, egui::Id::unique(id), label, scale).clicked()
}

#[derive(Clone, Copy)]
enum SlotAddress { Inventory(usize), Crafting(usize), Output, Station(usize), Catalog(Slot) }
fn slot(ui: &mut egui::Ui, a: &Assets, rect: Rect, address: SlotAddress, state: &mut UiState,
    game: &mut Game, scale: f32, language: Language, tooltip: &mut Option<String>) {
    let (key, value) = match address {
        SlotAddress::Inventory(i) => (format!("inventory-{i}"), game.inventory_slot(i)),
        SlotAddress::Crafting(i) => (format!("craft-{i}"), game.crafting_grid[i]),
        SlotAddress::Station(i) => (format!("station-{i}"), game.station_slot(i)),
        SlotAddress::Output => ("craft-result".into(), game.crafting_output()),
        SlotAddress::Catalog(s) => (format!("catalog-{}-{}", s.block, s.variant), Some(s)),
    };
    let response = ui.interact(rect, egui::Id::unique(key), Sense::click());
    if let Some(s) = value { stack(ui.painter(), a, rect, s, scale); }
    if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, Color32::from_white_alpha(80));
        if state.inventory_cursor.is_none() && let Some(s) = value { *tooltip = Some(tr(language, &item::stack_name(s)).into()); }
    }
    if !response.clicked() && !response.secondary_clicked() { return; }
    let split = response.secondary_clicked(); let shift = ui.input(|i| i.modifiers.shift);
    match address {
        SlotAddress::Inventory(i) => {
            if shift {
                if game.station_kind().is_some() && game.station_slot_count() > 0 { game.quick_transfer_to_station(i); }
                else { game.quick_transfer(i); }
            } else { game.click_inventory_slot(i, &mut state.inventory_cursor, split); }
        }
        SlotAddress::Crafting(i) => {
            if shift { game.quick_transfer_crafting(i); }
            else { game.click_crafting_slot(i, &mut state.inventory_cursor, split); }
        }
        SlotAddress::Output => {
            if shift { for _ in 0..64 { if !game.craft_to_inventory() { break; } } }
            else { game.take_crafting_output(&mut state.inventory_cursor); }
        }
        SlotAddress::Station(i) => {
            if shift { game.quick_transfer_station(i); }
            else { game.click_station_slot(i, &mut state.inventory_cursor, split); }
        }
        SlotAddress::Catalog(mut s) => {
            s.count = if split { 1 } else { item::stack_limit(s.block) };
            if shift { game.hotbar[game.selected] = Some(s); }
            else { state.inventory_cursor = Some(s); }
        }
    }
}

fn player_portrait(p: &egui::Painter, a: &Assets, origin: Pos2, scale: f32) {
    // A small orthographic skinned player model, with the classic foot anchor
    // (51,75) and mouse-following head/body, rather than a flat avatar sticker.
    let anchor = origin + Vec2::new(51.0, 75.0) * scale;
    let pointer = p.ctx().input(|i| i.pointer.hover_pos()).unwrap_or(p.ctx().content_rect().center());
    let dx = (pointer.x - anchor.x) / (40.0 * scale);
    let dy = (pointer.y - (origin.y + 35.0 * scale)) / (40.0 * scale);
    let yaw = dx.atan() * 0.4;
    let head_yaw = yaw + dx.atan() * 0.3;
    let head_pitch = dy.atan() * 0.3;
    let pieces = [
        ([0.0, 28.0, 0.0], [8.0, 8.0, 8.0], [0.0, 0.0], true),
        ([0.0, 18.0, 0.0], [8.0, 12.0, 4.0], [16.0, 16.0], false),
        ([-6.0, 18.0, 0.0], [4.0, 12.0, 4.0], [40.0, 16.0], false),
        ([6.0, 18.0, 0.0], [4.0, 12.0, 4.0], [40.0, 16.0], false),
        ([-2.0, 6.0, 0.0], [4.0, 12.0, 4.0], [0.0, 16.0], false),
        ([2.0, 6.0, 0.0], [4.0, 12.0, 4.0], [0.0, 16.0], false),
    ];
    let mut faces: Vec<(f32, [Pos2; 4], [[f32; 2]; 4], Color32)> = Vec::new();
    for (center, size, skin, head) in pieces {
        let angle = if head { head_yaw } else { yaw };
        let pitch = if head { head_pitch } else { 0.0 };
        let rotate = |v: [f32; 3]| {
            let y = v[1] * pitch.cos() - v[2] * pitch.sin();
            let z = v[1] * pitch.sin() + v[2] * pitch.cos();
            [v[0] * angle.cos() + z * angle.sin(), y, -v[0] * angle.sin() + z * angle.cos()]
        };
        for axis in 0..3 { for sign in [-1.0, 1.0] {
            let mut normal = [0.0; 3]; normal[axis] = sign;
            let normal = rotate(normal);
            if normal[2] <= 0.0 { continue; }
            let (u_axis, v_axis) = if axis == 0 { (2, 1) } else if axis == 1 { (0, 2) } else { (0, 1) };
            let mut points = [Pos2::ZERO; 4]; let mut depth = 0.0;
            for (i, (u, v)) in [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)].into_iter().enumerate() {
                let mut local = [0.0; 3]; local[axis] = sign * size[axis] * 0.5;
                local[u_axis] = u * size[u_axis] * 0.5; local[v_axis] = v * size[v_axis] * 0.5;
                let local = rotate(local);
                let center_rotated = if head { [center[0], center[1], center[2]] }
                    else { [center[0] * yaw.cos() + center[2] * yaw.sin(), center[1], -center[0] * yaw.sin() + center[2] * yaw.cos()] };
                let world = [local[0] + center_rotated[0], local[1] + center_rotated[1], local[2] + center_rotated[2]];
                points[i] = anchor + Vec2::new(world[0], -world[1]) * (1.7 * scale);
                depth += world[2];
            }
            let (mut u, mut v, w, h) = (skin[0] + size[2], skin[1] + size[2], size[u_axis], size[v_axis]);
            if axis == 0 { u = skin[0] + if sign > 0.0 { size[2] + size[0] } else { 0.0 }; }
            else if axis == 1 { u = skin[0] + size[2] + if sign > 0.0 { 0.0 } else { size[0] }; v = skin[1]; }
            else if sign < 0.0 { u += size[0] + size[2]; }
            let uv = [[u / 64.0, v / 32.0], [(u + w) / 64.0, v / 32.0], [(u + w) / 64.0, (v + h) / 32.0], [u / 64.0, (v + h) / 32.0]];
            let lighting = (0.65 + normal[1].max(0.0) * 0.3 + normal[2].max(0.0) * 0.2).min(1.0);
            faces.push((depth, points, uv, Color32::from_gray((lighting * 255.0) as u8)));
        }}
    }
    faces.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, points, uv, color) in faces {
        let mut mesh = egui::Mesh::with_texture(a.textures[6].id());
        for i in 0..4 { mesh.vertices.push(egui::epaint::Vertex { pos: points[i], uv: Pos2::from(uv[i]), color }); }
        mesh.indices.extend([0, 1, 2, 0, 2, 3]); p.add(egui::Shape::mesh(mesh));
    }
}

pub(super) fn draw(ctx: &egui::Context, state: &mut UiState, game: &mut Game, settings: &Settings, _out: &mut UiOutput) {
    let language = settings.language; let a = assets(ctx, &settings.texture_pack);
    let screen = ctx.content_rect();
    let scale = classic_ui::gui_scale(ctx);
    let station = game.station_kind();
    let catalog = game.mode == GameMode::Creative && station.is_none() && state.inventory_catalog;
    let chest = station == Some(StationKind::Chest);
    let chest_rows = if chest { game.station_slot_count().div_ceil(9).max(3) as f32 } else { 3.0 };
    let height = if chest { 114.0 + chest_rows * 18.0 } else { 166.0 };
    let origin = Pos2::new((screen.center().x - 88.0 * scale).floor(), (screen.center().y - height * scale * 0.5).floor());
    let panel = Rect::from_min_size(origin, Vec2::new(176.0, height) * scale);
    let r = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(origin + Vec2::new(x, y) * scale, Vec2::new(w, h) * scale);
    let mut tooltip = None;
    let header = if game.mode == GameMode::Creative && station.is_none() { 15.0 * scale } else { 0.0 };
    egui::Area::new(egui::Id::unique("classic-inventory"))
        .fixed_pos(origin - Vec2::new(0.0, header)).fade_in(false).order(egui::Order::Foreground).show(ctx, |ui| {
            ui.set_min_size(panel.size() + Vec2::new(0.0, header));
            if catalog {
                region(ui.painter(), &a.textures[3], r(0.0, 0.0, 176.0, 125.0), [0.0, 0.0, 176.0, 125.0]);
                region(ui.painter(), &a.textures[0], r(0.0, 125.0, 176.0, 41.0), [0.0, 125.0, 176.0, 41.0]);
                // Cover the unused storage-row remnants with the original flat panel gray.
                ui.painter().rect_filled(r(7.0, 125.0, 162.0, 15.0), 0.0, Color32::from_rgb(198, 198, 198));
                classic_text(ui.painter(), &a, tr(language, "Creative"), r(8.0, 6.0, 1.0, 1.0).min, scale, Color32::from_gray(64), false);
                let entries = item::creative_stacks();
                let pages = entries.len().div_ceil(54).max(1);
                if ui.rect_contains_pointer(panel) {
                    let scroll = ui.input(|i| i.raw.events.iter().filter_map(|event| {
                        if let egui::Event::MouseWheel { delta, .. } = event { Some(delta.y) } else { None }
                    }).sum::<f32>());
                    if scroll < -1.0 { state.inventory_page = (state.inventory_page + 1).min(pages - 1); }
                    else if scroll > 1.0 { state.inventory_page = state.inventory_page.saturating_sub(1); }
                }
                state.inventory_page = state.inventory_page.min(pages - 1);
                for (index, entry) in entries.iter().skip(state.inventory_page * 54).take(54).enumerate() {
                    slot(ui, &a, r(8.0 + (index % 9) as f32 * 18.0, 18.0 + (index / 9) as f32 * 18.0, 16.0, 16.0),
                        SlotAddress::Catalog(*entry), state, game, scale, language, &mut tooltip);
                }
                if small_button(ui, &a, r(8.0, 126.0, 18.0, 12.0), "catalog-prev", "<", scale) { state.inventory_page = state.inventory_page.saturating_sub(1); }
                if small_button(ui, &a, r(150.0, 126.0, 18.0, 12.0), "catalog-next", ">", scale) { state.inventory_page = (state.inventory_page + 1).min(pages - 1); }
                let page = format!("{} / {}", state.inventory_page + 1, pages);
                classic_text(ui.painter(), &a, &page, Pos2::new(panel.center().x - text_width(&a, &page, scale) * 0.5, origin.y + 128.0 * scale), scale, Color32::from_gray(64), false);
            } else if chest {
                let top = chest_rows * 18.0 + 17.0;
                region(ui.painter(), &a.textures[3], r(0.0, 0.0, 176.0, top), [0.0, 0.0, 176.0, top]);
                region(ui.painter(), &a.textures[3], r(0.0, top, 176.0, 96.0), [0.0, 126.0, 176.0, 96.0]);
                classic_text(ui.painter(), &a, tr(language,game.station_title().unwrap_or("Chest")), r(8.0, 6.0, 1.0, 1.0).min, scale, Color32::from_gray(64), false);
                classic_text(ui.painter(), &a, tr(language, "Inventory"), r(8.0, chest_rows * 18.0 + 19.0, 1.0, 1.0).min, scale, Color32::from_gray(64), false);
                for index in 0..game.station_slot_count() {
                    slot(ui, &a, r(8.0 + (index % 9) as f32 * 18.0, 18.0 + (index / 9) as f32 * 18.0, 16.0, 16.0), SlotAddress::Station(index), state, game, scale, language, &mut tooltip);
                }
            } else {
                let image = match station { Some(StationKind::CraftingTable) => 1, Some(StationKind::Furnace) => 2, Some(StationKind::Dispenser)=>11, _ => 0 };
                region(ui.painter(), &a.textures[image], panel, [0.0, 0.0, 176.0, 166.0]);
                if station==Some(StationKind::Dispenser) {
                    classic_text(ui.painter(),&a,tr(language,"Dispenser"),r(60.0,6.0,1.0,1.0).min,scale,Color32::from_gray(64),false);
                    classic_text(ui.painter(),&a,tr(language,"Inventory"),r(8.0,73.0,1.0,1.0).min,scale,Color32::from_gray(64),false);
                    for index in 0..9 {slot(ui,&a,r(62.0+(index%3)as f32*18.0,17.0+(index/3)as f32*18.0,16.0,16.0),SlotAddress::Station(index),state,game,scale,language,&mut tooltip);}
                } else if station == Some(StationKind::Furnace) {
                    classic_text(ui.painter(), &a, tr(language, "Furnace"), r(60.0, 6.0, 1.0, 1.0).min, scale, Color32::from_gray(64), false);
                    classic_text(ui.painter(), &a, tr(language, "Inventory"), r(8.0, 73.0, 1.0, 1.0).min, scale, Color32::from_gray(64), false);
                    for (index, (x, y)) in [(56.0, 17.0), (56.0, 53.0), (116.0, 35.0)].into_iter().enumerate() {
                        slot(ui, &a, r(x, y, 16.0, 16.0), SlotAddress::Station(index), state, game, scale, language, &mut tooltip);
                    }
                    if let Some((cook, burn)) = game.furnace_progress() {
                        let flame = (burn * 12.0).floor().clamp(0.0, 12.0);
                        if burn > 0.0 { region(ui.painter(), &a.textures[2], r(56.0, 48.0 - flame, 14.0, flame + 2.0), [176.0, 12.0 - flame, 14.0, flame + 2.0]); }
                        let progress = (cook * 24.0).floor().clamp(0.0, 24.0);
                        if progress > 0.0 { region(ui.painter(), &a.textures[2], r(79.0, 34.0, progress + 1.0, 16.0), [176.0, 14.0, progress + 1.0, 16.0]); }
                    }
                } else {
                    let table = station == Some(StationKind::CraftingTable);
                    classic_text(ui.painter(), &a, tr(language, "Crafting"), r(if table { 28.0 } else { 86.0 }, 6.0 + if table { 0.0 } else { 10.0 }, 1.0, 1.0).min, scale, Color32::from_gray(64), false);
                    if table { classic_text(ui.painter(), &a, tr(language, "Inventory"), r(8.0, 73.0, 1.0, 1.0).min, scale, Color32::from_gray(64), false); }
                    else {
                        player_portrait(ui.painter(), &a, origin, scale);
                        for index in 0..4 { slot(ui, &a, r(8.0, 8.0 + index as f32 * 18.0, 16.0, 16.0), SlotAddress::Inventory(36 + index), state, game, scale, language, &mut tooltip); }
                    }
                    let width = if table { 3 } else { 2 };
                    for index in 0..width * width {
                        let x = if table { 30.0 } else { 88.0 } + (index % width) as f32 * 18.0;
                        let y = if table { 17.0 } else { 26.0 } + (index / width) as f32 * 18.0;
                        slot(ui, &a, r(x, y, 16.0, 16.0), SlotAddress::Crafting(index), state, game, scale, language, &mut tooltip);
                    }
                    slot(ui, &a, r(if table { 124.0 } else { 144.0 }, if table { 35.0 } else { 36.0 }, 16.0, 16.0), SlotAddress::Output, state, game, scale, language, &mut tooltip);
                }
            }
            if !catalog {
                for index in 9..36 {
                    slot(ui, &a, r(8.0 + ((index - 9) % 9) as f32 * 18.0, if chest { chest_rows * 18.0 + 31.0 } else { 84.0 } + ((index - 9) / 9) as f32 * 18.0, 16.0, 16.0), SlotAddress::Inventory(index), state, game, scale, language, &mut tooltip);
                }
            }
            for index in 0..9 {
                slot(ui, &a, r(8.0 + index as f32 * 18.0, if chest { chest_rows * 18.0 + 89.0 } else { 142.0 }, 16.0, 16.0), SlotAddress::Inventory(index), state, game, scale, language, &mut tooltip);
            }
            if game.mode == GameMode::Creative && station.is_none() {
                let label = tr(language, if catalog { "Inventory" } else { "Creative" });
                if small_button(ui, &a, r(104.0, -15.0, 72.0, 14.0), "inventory-catalog-tab", label, scale) { state.inventory_catalog = !state.inventory_catalog; }
            }
        });
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    if let Some(pointer) = pointer {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("inventory-cursor")));
        if let Some(value) = state.inventory_cursor {
            stack(&painter, &a, Rect::from_center_size(pointer, Vec2::splat(16.0 * scale)), value, scale);
            let clickable = if header > 0.0 { panel.union(r(104.0, -15.0, 72.0, 14.0)) } else { panel };
            if !clickable.contains(pointer) {
                let (left, right) = ctx.input(|i| (i.pointer.primary_clicked(), i.pointer.secondary_clicked()));
                if left || right { game.drop_cursor_stack(&mut state.inventory_cursor, right); }
            }
        } else if let Some(text) = tooltip {
            let size = Vec2::new(text_width(&a, &text, scale) + 6.0 * scale, 14.0 * scale);
            let pos = Pos2::new((pointer.x + 12.0).min(screen.max.x - size.x - 4.0), (pointer.y - 12.0).min(screen.max.y - size.y - 4.0));
            let rect = Rect::from_min_size(pos, size);
            painter.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(16, 0, 16, 245));
            painter.rect_stroke(rect.shrink(scale), 0.0, Stroke::new(scale, Color32::from_rgb(80, 0, 160)), egui::StrokeKind::Inside);
            classic_text(&painter, &a, &text, pos + Vec2::splat(3.0 * scale), scale, Color32::WHITE, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{FrameInput, FrameRenderer};
    use crate::input::{Bindings, InputState};
    use crate::ui::Screen;
    use mc4d_core::{block, glam::IVec4};

    #[test]
    #[ignore = "requires GPU; captures original-layout inventory/table/chest/furnace/creative panels"]
    fn classic_panels_render_and_storage_click_moves_the_real_stack() {
        let size = (1280, 800);
        let gpu = crate::headless_gpu(size).unwrap();
        let mut frame = FrameRenderer::new(&gpu.device, gpu.target.format(), size, gpu.adapter_name);
        let mut game = Game::new(None, 7, GameMode::Survival);
        let mut settings = Settings::default();
        settings.language = Language::English;
        let mut bindings = Bindings::default(); let input = InputState::default();
        let mut state = UiState { screen: Screen::Inventory, show_hud: false, show_debug: false,
            inventory_catalog: false, ..Default::default() };
        game.inventory[0] = Some(Slot::new(block::STONE, 64));
        game.inventory[1] = Some(Slot::new(item::IRON_INGOT, 12));
        game.hotbar[0] = Some(Slot::new(item::IRON_PICKAXE, 1));
        game.hotbar[1] = Some(Slot::new(block::PLANKS, 32));
        game.hotbar[2] = Some(Slot::new(item::BREAD, 1));
        game.crafting_grid[0] = Some(Slot::new(block::LOG, 3));
        let mut render = |game: &mut Game, state: &mut UiState, events: Vec<egui::Event>| {
            let mut raw_input = crate::raw_input(size); raw_input.events = events;
            frame.render(&gpu.device, &gpu.queue, &gpu.target, FrameInput { game, settings: &mut settings,
                bindings: &mut bindings, input: &input, ui: state, raw_input, size, pixels_per_point: 1.0 });
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
        std::fs::create_dir_all(&root).unwrap();
        let capture = |name: &str| {
            let rgba = crate::frame::read_texture(&gpu.device, &gpu.queue, &gpu.target);
            crate::frame::write_png(&root.join(format!("inventory-{name}.png")), size.0, size.1, &rgba).unwrap();
        };
        render(&mut game, &mut state, vec![]); render(&mut game, &mut state, vec![]);
        capture("survival");
        // At 1280x800 the classic GUI is 3x: origin (376,151), first storage
        // icon center (8+8,84+8). Verify these pixels operate the real inventory.
        let pointer = Pos2::new(424.0, 427.0);
        render(&mut game, &mut state, vec![egui::Event::PointerMoved(pointer), egui::Event::PointerButton {
            pos: pointer, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::default() }]);
        render(&mut game, &mut state, vec![egui::Event::PointerButton {
            pos: pointer, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::default() }]);
        assert_eq!(state.inventory_cursor, Some(Slot::new(block::STONE, 64)));
        assert!(game.inventory[0].is_none());
        game.return_cursor(&mut state.inventory_cursor);
        for (offset, (name, id)) in [("crafting", block::CRAFTING_TABLE), ("chest", block::CHEST), ("furnace", block::FURNACE), ("dispenser",block::DISPENSER)].into_iter().enumerate() {
            let pos = game.body.pos.floor().as_ivec4() + IVec4::X * (offset as i32 + 2);
            game.world.set_block(pos, id);
            game.target = Some(crate::game::Target::Block(mc4d_core::raycast::Hit { block: pos, normal: IVec4::Y, dist: 2.0 }));
            assert!(game.open_target_station());
            if name == "crafting" { game.crafting_grid[0] = Some(Slot::new(block::LOG, 1)); }
            else if name == "chest" {
                let mut cursor = Some(Slot::new(block::COBBLE, 64)); game.click_station_slot(0, &mut cursor, false);
            } else if name=="dispenser" {
                let mut cursor=Some(Slot::new(item::ARROW,16));game.click_station_slot(0,&mut cursor,false);
            } else {
                let mut cursor = Some(Slot::new(block::IRON_ORE, 8)); game.click_station_slot(0, &mut cursor, false);
                let mut cursor = Some(Slot::new(item::COAL, 4)); game.click_station_slot(1, &mut cursor, false);
            }
            render(&mut game, &mut state, vec![egui::Event::PointerGone]);
            render(&mut game, &mut state, vec![]); capture(name);
            if name=="chest" {
                let other=pos+IVec4::W;game.world.set_block(other,block::CHEST);game.close_station();
                game.target=Some(crate::game::Target::Block(mc4d_core::raycast::Hit{block:other,normal:IVec4::Y,dist:2.0}));assert!(game.open_target_station());assert_eq!(game.station_slot_count(),54);
                let mut cursor=Some(Slot::new(item::DIAMOND,2));game.click_station_slot(53,&mut cursor,false);
                render(&mut game,&mut state,vec![]);render(&mut game,&mut state,vec![]);capture("large-chest");
                let pointer=Pos2::new(856.0,415.0);
                render(&mut game,&mut state,vec![egui::Event::PointerMoved(pointer),egui::Event::PointerButton{pos:pointer,button:egui::PointerButton::Primary,pressed:true,modifiers:egui::Modifiers::default()}]);
                render(&mut game,&mut state,vec![egui::Event::PointerButton{pos:pointer,button:egui::PointerButton::Primary,pressed:false,modifiers:egui::Modifiers::default()}]);
                assert_eq!(state.inventory_cursor,Some(Slot::new(item::DIAMOND,2)));assert!(game.station_slot(53).is_none());game.return_cursor(&mut state.inventory_cursor);
            }
            game.close_station();
        }
        game.set_mode(GameMode::Creative); state.inventory_catalog = true;
        render(&mut game, &mut state, vec![]); render(&mut game, &mut state, vec![]); capture("creative");
    }
}
