//! Beta's draggable achievement map, statistics lists and earned-achievement popup.
use super::{Screen, UiState, classic_ui as c, paint_stack_icon};
use crate::{
    game::{Game, Slot},
    i18n::{Language, text as tr},
    input::{Action, Bindings},
    progress::ACHIEVEMENTS,
};
use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, Vec2};

pub struct State {
    scroll: Vec2,
    tab: usize,
    toast: Option<(u8, f32)>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            scroll: Vec2::new(-82.0, -70.0),
            tab: 0,
            toast: None,
        }
    }
}
fn translated<'a>(language: Language, en: &'a str, zh: &'a str) -> &'a str {
    if language == Language::Chinese {
        zh
    } else {
        en
    }
}
fn node_position(origin: Pos2, scroll: Vec2, index: usize, scale: f32) -> Pos2 {
    let a = &ACHIEVEMENTS[index];
    origin + (Vec2::new(a.x as f32 * 24.0, a.y as f32 * 24.0) - scroll) * scale
}

// wc.b(IIF), using java.util.Random's 48-bit generator: the stone/ore
// wallpaper is a stable part of the achievement map, not fresh visual noise.
fn background_tile(column: i32, row: i32) -> (u32, u8) {
    let mut seed = ((1234_i64 + column as i64) as u64 ^ 0x5deece66d) & ((1 << 48) - 1);
    let mut next = |bits: u32| {
        seed = (seed.wrapping_mul(0x5deece66d).wrapping_add(11)) & ((1 << 48) - 1);
        (seed >> (48 - bits)) as u32
    };
    next(32);
    let bound = (row + 1).max(1) as u32;
    let depth = loop {
        let bits = next(31);
        if bound.is_power_of_two() {
            break ((bound as u64 * bits as u64) >> 31) as i32;
        }
        let value = bits % bound;
        if bits.wrapping_sub(value).wrapping_add(bound - 1) < 0x80000000 {
            break value as i32;
        }
    } + row / 2;
    let tile = if depth > 37 || row == 35 {
        17
    } else if depth == 22 {
        if next(31) >> 30 == 0 { 50 } else { 51 }
    } else if depth == 10 {
        33
    } else if depth == 8 {
        34
    } else if depth > 4 {
        1
    } else if depth > 0 {
        2
    } else {
        18
    };
    (
        tile,
        ((0.6 - row as f32 / 25.0 * 0.3).clamp(0.0, 1.0) * 255.0) as u8,
    )
}

pub fn achievements(
    ctx: &egui::Context,
    state: &mut UiState,
    game: &Game,
    language: Language,
    bindings: &Bindings,
) {
    let scale = c::gui_scale(ctx);
    let assets = c::current_assets(ctx);
    let origin = ctx.content_rect().center() - Vec2::new(128.0, 101.0) * scale;
    let viewport = Rect::from_min_size(
        origin + Vec2::new(16.0, 17.0) * scale,
        Vec2::new(224.0, 155.0) * scale,
    );
    let progress = game
        .progress
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    egui::Area::new(Id::unique("achievement-map"))
        .movable(false)
        .fade_in(false)
        .fixed_pos(origin)
        .show(ctx, |ui| {
            ui.set_min_size(Vec2::new(256.0, 202.0) * scale);
            let response = ui.interact(viewport, Id::unique("achievement-map-pan"), Sense::drag());
            if response.dragged() {
                state.progress_ui.scroll -= ctx.input(|i| i.pointer.delta()) / scale;
            }
            // Original graph bounds, with enough margin to inspect edge nodes and tooltips.
            state.progress_ui.scroll = state
                .progress_ui
                .scroll
                .clamp(Vec2::new(-136.0, -232.0), Vec2::new(192.0, 144.0));
            let p = ui.painter().with_clip_rect(viewport);
            p.rect_filled(viewport, 0.0, Color32::from_gray(20));
            // Deterministic underground terrain scrolls together with the achievement graph.
            let scroll = state.progress_ui.scroll;
            for y in (scroll.y / 16.0).floor() as i32..=(scroll.y / 16.0).floor() as i32 + 11 {
                for x in (scroll.x / 16.0).floor() as i32..=(scroll.x / 16.0).floor() as i32 + 15 {
                    let (tile, brightness) = background_tile(x + 18, y + 18);
                    let dest = Rect::from_min_size(
                        viewport.min
                            + (Vec2::new(x as f32 * 16.0, y as f32 * 16.0) - scroll) * scale,
                        Vec2::splat(16.0 * scale),
                    );
                    let uv = Rect::from_min_size(
                        Pos2::new((tile % 16) as f32 / 16.0, (tile / 16) as f32 / 16.0),
                        Vec2::splat(1.0 / 16.0),
                    );
                    p.image(
                        assets.textures[10].id(),
                        dest,
                        uv,
                        Color32::from_gray(brightness),
                    );
                }
            }
            for (index, a) in ACHIEVEMENTS.iter().enumerate() {
                if let Some(parent) = a.parent {
                    let from = node_position(viewport.min, scroll, parent, scale)
                        + Vec2::splat(11.0 * scale);
                    let to = node_position(viewport.min, scroll, index, scale)
                        + Vec2::splat(11.0 * scale);
                    let color = if progress.unlocked.contains(&(index as u8)) {
                        Color32::from_rgb(110, 110, 110)
                    } else if progress.available(index) {
                        Color32::from_rgb(0, 120, 0)
                    } else {
                        Color32::BLACK
                    };
                    p.line_segment([from, Pos2::new(from.x, to.y)], Stroke::new(scale, color));
                    p.line_segment([Pos2::new(from.x, to.y), to], Stroke::new(scale, color));
                }
            }
            let mut hovered = None;
            for (index, a) in ACHIEVEMENTS.iter().enumerate() {
                let rect = Rect::from_min_size(
                    node_position(viewport.min, scroll, index, scale),
                    Vec2::splat(22.0 * scale),
                );
                if !rect.intersects(viewport) {
                    continue;
                }
                c::region(
                    &p,
                    &assets.textures[9],
                    rect.expand(2.0 * scale),
                    [if a.special { 26.0 } else { 0.0 }, 202.0, 26.0, 26.0],
                );
                paint_stack_icon(
                    ctx,
                    &p,
                    Rect::from_min_size(
                        rect.min + Vec2::splat(3.0 * scale),
                        Vec2::splat(16.0 * scale),
                    ),
                    a.icon(),
                );
                if !progress.available(index) {
                    p.rect_filled(
                        rect.expand(2.0 * scale),
                        0.0,
                        Color32::from_black_alpha(180),
                    );
                } else if !progress.unlocked.contains(&(index as u8)) {
                    p.rect_filled(rect.expand(2.0 * scale), 0.0, Color32::from_black_alpha(35));
                }
                if ctx
                    .pointer_hover_pos()
                    .is_some_and(|pos| rect.contains(pos) && viewport.contains(pos))
                {
                    hovered = Some(index);
                }
            }
            c::region(
                ui.painter(),
                &assets.textures[9],
                Rect::from_min_size(origin, Vec2::new(256.0, 202.0) * scale),
                [0.0, 0.0, 256.0, 202.0],
            );
            c::classic_text(
                ui.painter(),
                &assets,
                translated(language, "Achievements", "成就"),
                origin + Vec2::new(15.0, 6.0) * scale,
                scale,
                Color32::from_gray(64),
                false,
            );
            let done = Rect::from_min_size(
                origin + Vec2::new(152.0, 175.0) * scale,
                Vec2::new(80.0, 20.0) * scale,
            );
            if c::button_at(
                ui,
                &assets,
                done,
                Id::unique("achievement-done"),
                tr(language, "Done"),
                scale,
            )
            .clicked()
            {
                state.screen = Screen::Pause;
            }
            if let Some(index) = hovered {
                let a = &ACHIEVEMENTS[index];
                let available = progress.available(index);
                let inventory_key = bindings.get(Action::Inventory)[0].label(&Default::default());
                let detail = if available {
                    a.description(language).replace("%1$s", &inventory_key)
                } else {
                    format!(
                        "{} {}",
                        translated(language, "Requires:", "需要："),
                        a.parent
                            .map(|p| ACHIEVEMENTS[p].title(language))
                            .unwrap_or("")
                    )
                };
                let title = a.title(language);
                let pointer = ctx.pointer_hover_pos().unwrap() + Vec2::new(12.0, -12.0) * scale;
                // Tooltips are screen-clamped and wrapped independently of the map clip.
                egui::Area::new(Id::unique("achievement-tooltip"))
                    .order(egui::Order::Tooltip)
                    .fade_in(false)
                    .fixed_pos(pointer)
                    .show(ctx, |ui| {
                        egui::Frame::new()
                            .fill(Color32::from_black_alpha(235))
                            .inner_margin(4)
                            .show(ui, |ui| {
                                ui.set_max_width(180.0 * scale);
                                ui.label(egui::RichText::new(title).size(9.0 * scale).color(
                                    if available {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_rgb(160, 160, 0)
                                    },
                                ));
                                ui.label(egui::RichText::new(detail).size(8.0 * scale).color(
                                    if available {
                                        Color32::from_gray(180)
                                    } else {
                                        Color32::from_rgb(140, 100, 100)
                                    },
                                ));
                                if progress.unlocked.contains(&(index as u8)) {
                                    ui.label(
                                        egui::RichText::new(translated(
                                            language,
                                            "Achievement taken!",
                                            "已获得成就！",
                                        ))
                                        .size(8.0 * scale)
                                        .color(Color32::from_gray(160)),
                                    );
                                }
                            });
                    });
            }
        });
}

const GENERAL: &[(&str, &str, &str, bool)] = &[
    ("sessions", "Worlds played", "游玩世界数", false),
    ("saves_loaded", "Saves loaded", "载入存档数", false),
    ("games_quit", "Games quit", "退出游戏数", false),
    ("play_ticks", "Time played", "游戏时间", false),
    ("walk_cm", "Distance Walked", "步行距离", true),
    ("swim_cm", "Distance Swum", "游泳距离", true),
    ("fall_cm", "Distance Fallen", "下落距离", true),
    ("climb_cm", "Distance Climbed", "攀爬距离", true),
    ("fly_cm", "Distance Flown", "飞行距离", true),
    ("dive_cm", "Distance Dove", "潜水距离", true),
    ("minecart_cm", "Distance by Minecart", "矿车行驶距离", true),
    ("boat_cm", "Distance by Boat", "乘船距离", true),
    ("pig_cm", "Distance by Pig", "骑猪距离", true),
    ("jumps", "Jumps", "跳跃次数", false),
    ("drops", "Drops", "丢弃次数", false),
    ("damage_dealt", "Damage Dealt", "造成伤害", false),
    ("damage_taken", "Damage Taken", "受到伤害", false),
    ("deaths", "Number of Deaths", "死亡次数", false),
    ("mob_kills", "Mob Kills", "生物击杀数", false),
    ("player_kills", "Player Kills", "玩家击杀数", false),
    ("fish_caught", "Fish Caught", "钓鱼数", false),
];
fn distance(cm: u64) -> String {
    if cm >= 100_000 {
        format!("{:.2} km", cm as f64 / 100_000.0)
    } else if cm >= 100 {
        format!("{:.2} m", cm as f64 / 100.0)
    } else {
        format!("{cm} cm")
    }
}
pub fn statistics(ctx: &egui::Context, state: &mut UiState, game: &Game, language: Language) {
    let scale = c::gui_scale(ctx);
    let screen = ctx.content_rect();
    let assets = c::current_assets(ctx);
    let snapshot = game
        .progress
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let p = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Middle,
        Id::unique("statistics-title"),
    ));
    let title = translated(language, "Statistics", "统计信息");
    c::classic_text(
        &p,
        &assets,
        title,
        Pos2::new(
            screen.center().x - c::text_width(&assets, title, scale) / 2.0,
            8.0 * scale,
        ),
        scale,
        Color32::WHITE,
        true,
    );
    let list = Rect::from_min_max(
        Pos2::new(screen.center().x - 110.0 * scale, 32.0 * scale),
        Pos2::new(
            screen.center().x + 130.0 * scale,
            screen.bottom() - 64.0 * scale,
        ),
    );
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        Id::unique("title-background"),
    ))
    .rect_filled(
        Rect::from_min_max(
            Pos2::new(screen.left(), list.top()),
            Pos2::new(screen.right(), list.bottom()),
        ),
        0.0,
        Color32::from_black_alpha(150),
    );
    egui::Area::new(Id::unique("statistics-list"))
        .fade_in(false)
        .fixed_pos(list.min)
        .show(ctx, |ui| {
            ui.set_width(list.width());
            ui.set_max_height(list.height());
            egui::ScrollArea::vertical()
                .id_salt(("statistics-scroll", state.progress_ui.tab))
                .max_height(list.height())
                .show(ui, |ui| {
                    if state.progress_ui.tab == 0 {
                        for (index, (key, en, zh, is_distance)) in GENERAL.iter().enumerate() {
                            let value = snapshot.counter(key);
                            let value = if *is_distance {
                                distance(value)
                            } else if *key == "play_ticks" {
                                format!("{:.2} m", value as f64 / 1200.0)
                            } else {
                                value.to_string()
                            };
                            let (_, rect) =
                                ui.allocate_space(Vec2::new(220.0 * scale, 10.0 * scale));
                            let color = if index % 2 == 0 {
                                Color32::WHITE
                            } else {
                                Color32::from_gray(160)
                            };
                            c::classic_text(
                                ui.painter(),
                                &assets,
                                translated(language, en, zh),
                                rect.min,
                                scale,
                                color,
                                true,
                            );
                            c::classic_text(
                                ui.painter(),
                                &assets,
                                &value,
                                Pos2::new(
                                    rect.right() - c::text_width(&assets, &value, scale),
                                    rect.top(),
                                ),
                                scale,
                                color,
                                true,
                            );
                        }
                    } else {
                        let blocks = state.progress_ui.tab == 1;
                        let labels = if blocks {
                            translated(language, "Mined / Crafted / Used", "挖掘 / 合成 / 使用")
                        } else {
                            translated(language, "Depleted / Crafted / Used", "耗尽 / 合成 / 使用")
                        };
                        c::text(
                            ui.painter(),
                            labels,
                            ui.cursor().min,
                            scale,
                            Color32::WHITE,
                            true,
                        );
                        ui.add_space(14.0 * scale);
                        for (id, count) in &snapshot.items {
                            let Ok(id) = id.parse::<u16>() else {
                                continue;
                            };
                            if (id < 256) != blocks {
                                continue;
                            }
                            if count.mined + count.crafted + count.used + count.depleted == 0 {
                                continue;
                            }
                            let stack = Slot::new(id, 1);
                            let (_, rect) =
                                ui.allocate_space(Vec2::new(220.0 * scale, 24.0 * scale));
                            paint_stack_icon(
                                ctx,
                                ui.painter(),
                                Rect::from_min_size(rect.min, Vec2::splat(18.0 * scale)),
                                stack,
                            );
                            c::text(
                                ui.painter(),
                                tr(language, mc4d_core::item::stack_name(stack)),
                                rect.min + Vec2::new(22.0, 0.0) * scale,
                                scale,
                                Color32::WHITE,
                                true,
                            );
                            let text = format!(
                                "{} / {} / {}",
                                if blocks { count.mined } else { count.depleted },
                                count.crafted,
                                count.used
                            );
                            c::text(
                                ui.painter(),
                                &text,
                                rect.min + Vec2::new(22.0, 10.0) * scale,
                                scale,
                                Color32::from_gray(160),
                                true,
                            );
                        }
                    }
                });
        });
    egui::Area::new(Id::unique("statistics-buttons"))
        .fade_in(false)
        .fixed_pos(Pos2::new(
            screen.center().x - 155.0 * scale,
            screen.bottom() - 52.0 * scale,
        ))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0 * scale, 4.0 * scale);
            ui.horizontal(|ui| {
                for (index, (en, zh)) in
                    [("General", "常规"), ("Blocks", "方块"), ("Items", "物品")]
                        .into_iter()
                        .enumerate()
                {
                    if c::button(
                        ui,
                        translated(language, en, zh),
                        100.0 * scale,
                        20.0 * scale,
                    )
                    .clicked()
                    {
                        state.progress_ui.tab = index;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(105.0 * scale);
                if c::button(ui, tr(language, "Done"), 150.0 * scale, 20.0 * scale).clicked() {
                    state.screen = Screen::Pause;
                }
            });
        });
}

pub fn toast(ctx: &egui::Context, state: &mut UiState, game: &mut Game, language: Language) {
    if let Some((_, elapsed)) = &mut state.progress_ui.toast {
        *elapsed += state.dt.max(0.0);
        if *elapsed >= 3.0 {
            state.progress_ui.toast = None;
        }
    }
    if state.progress_ui.toast.is_none() && !game.achievement_notifications.is_empty() {
        state.progress_ui.toast = Some((game.achievement_notifications.remove(0), 0.0));
    }
    let Some((index, elapsed)) = state.progress_ui.toast else {
        return;
    };
    let Some(a) = ACHIEVEMENTS.get(index as usize) else {
        return;
    };
    let scale = c::gui_scale(ctx);
    let assets = c::current_assets(ctx);
    let slide = if elapsed < 0.3 {
        1.0 - elapsed / 0.3
    } else if elapsed > 2.7 {
        (elapsed - 2.7) / 0.3
    } else {
        0.0
    };
    let rect = Rect::from_min_size(
        Pos2::new(
            ctx.content_rect().right() - 160.0 * scale,
            -32.0 * scale * slide,
        ),
        Vec2::new(160.0, 32.0) * scale,
    );
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        Id::unique("achievement-toast"),
    ));
    c::region(
        &painter,
        &assets.textures[9],
        rect,
        [96.0, 202.0, 160.0, 32.0],
    );
    paint_stack_icon(
        ctx,
        &painter,
        Rect::from_min_size(
            rect.min + Vec2::new(8.0, 8.0) * scale,
            Vec2::splat(16.0 * scale),
        ),
        a.icon(),
    );
    c::classic_text(
        &painter,
        &assets,
        translated(language, "Achievement get!", "获得成就！"),
        rect.min + Vec2::new(30.0, 7.0) * scale,
        scale,
        Color32::from_rgb(255, 255, 0),
        false,
    );
    c::classic_text(
        &painter,
        &assets,
        a.title(language),
        rect.min + Vec2::new(30.0, 18.0) * scale,
        scale,
        Color32::WHITE,
        false,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_map_places_inventory_and_wood_inside_original_viewport() {
        let state = State::default();
        let origin = Pos2::ZERO;
        assert_eq!(
            node_position(origin, state.scroll, 0, 1.0),
            Pos2::new(82.0, 70.0)
        );
        assert_eq!(
            node_position(origin, state.scroll, 1, 1.0),
            Pos2::new(130.0, 94.0)
        );
        for i in 0..ACHIEVEMENTS.len() {
            let point = node_position(origin, state.scroll, i, 1.0);
            assert!(point.x.is_finite() && point.y.is_finite());
        }
    }
    #[test]
    fn statistic_distance_units_match_centimetre_storage() {
        assert_eq!(distance(4), "4 cm");
        assert_eq!(distance(150), "1.50 m");
        assert_eq!(distance(125000), "1.25 km");
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::{
        frame::{FrameInput, FrameRenderer},
        input::InputState,
        settings::Settings,
    };
    #[test]
    #[ignore = "requires GPU; verifies achievement-map interaction and bilingual progress captures"]
    fn original_progress_screens_render_and_map_drags() {
        let size = (1280, 800);
        let gpu = crate::headless_gpu(size).unwrap();
        let mut frame =
            FrameRenderer::new(&gpu.device, gpu.target.format(), size, gpu.adapter_name);
        let mut game = Game::menu_preview(7);
        let mut settings = Settings::default();
        let mut bindings = Bindings::default();
        let input = InputState::default();
        let mut state = UiState {
            screen: Screen::Achievements,
            show_hud: false,
            ..Default::default()
        };
        {
            let mut p = game.progress.lock().unwrap();
            p.award(0);
            p.award(1);
            p.bump("walk_cm", 125000);
            p.bump("play_ticks", 72000);
            p.item(mc4d_core::block::LOG.into()).mined = 32;
        }
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
        std::fs::create_dir_all(&root).unwrap();
        let mut render = |game: &mut Game,
                          state: &mut UiState,
                          settings: &mut Settings,
                          events: Vec<egui::Event>| {
            let mut raw_input = crate::raw_input(size);
            raw_input.events = events;
            frame.render(
                &gpu.device,
                &gpu.queue,
                &gpu.target,
                FrameInput {
                    game,
                    settings,
                    bindings: &mut bindings,
                    input: &input,
                    ui: state,
                    raw_input,
                    size,
                    pixels_per_point: 1.0,
                },
            );
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
        };
        for language in [Language::English, Language::Chinese] {
            settings.language = language;
            for (screen, name) in [
                (Screen::Achievements, "achievements"),
                (Screen::Statistics, "statistics"),
            ] {
                state.screen = screen;
                render(&mut game, &mut state, &mut settings, vec![]);
                render(&mut game, &mut state, &mut settings, vec![]);
                let rgba = crate::frame::read_texture(&gpu.device, &gpu.queue, &gpu.target);
                crate::frame::write_png(
                    &root.join(format!(
                        "{name}-{}.png",
                        if language == Language::English {
                            "en"
                        } else {
                            "zh"
                        }
                    )),
                    size.0,
                    size.1,
                    &rgba,
                )
                .unwrap();
            }
        }
        state.screen = Screen::Achievements;
        render(&mut game, &mut state, &mut settings, vec![]);
        let before = state.progress_ui.scroll;
        let start = Pos2::new(600.0, 400.0);
        let end = start + Vec2::new(60.0, 30.0);
        render(
            &mut game,
            &mut state,
            &mut settings,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        render(
            &mut game,
            &mut state,
            &mut settings,
            vec![egui::Event::PointerMoved(end)],
        );
        render(
            &mut game,
            &mut state,
            &mut settings,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        assert!(
            state.progress_ui.scroll.x < before.x,
            "drag moves the map rather than the window"
        );
        state.progress_ui.toast = Some((1, 1.0));
        render(
            &mut game,
            &mut state,
            &mut settings,
            vec![egui::Event::PointerGone],
        );
        let rgba = crate::frame::read_texture(&gpu.device, &gpu.queue, &gpu.target);
        crate::frame::write_png(
            &root.join("achievement-toast-zh.png"),
            size.0,
            size.1,
            &rgba,
        )
        .unwrap();
    }
}
