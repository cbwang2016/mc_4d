//! HUD and menus (egui): crosshair, hotbar, 4D compass, pause menu, options and the
//! Minecraft-style Controls screen.

use crate::game::{Game, GameMode, Target};
use crate::input::{Action, Bindings, Category, InputState};
use crate::settings::Settings;
use crate::tutorial::Tutorial;
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2};
use mc4d_core::block;
use mc4d_core::glam::Vec4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Playing,
    Pause,
    Options,
    Controls,
}

pub struct UiState {
    pub screen: Screen,
    /// Waiting for an input to bind to (action, slot).
    pub capturing: Option<(Action, usize)>,
    pub show_hud: bool,
    pub show_debug: bool,
    pub fps: f32,
    pub tutorial: Option<Tutorial>,
    /// Enter was pressed this frame (advances explanatory tutorial steps).
    pub enter_pressed: bool,
    /// Frame time, for UI animations and the tutorial.
    pub dt: f32,
    /// Smoothed CPU milliseconds: update, acquire, render, present.
    pub timings: [f32; 4],
    /// Video caption: title, subtitle and opacity (0..1).
    pub caption: Option<(String, String, f32)>,
    /// Video keystroke overlay: key labels and whether each is pressed.
    pub keys: Vec<(String, bool)>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            screen: Screen::Playing,
            capturing: None,
            show_hud: true,
            show_debug: false,
            fps: 0.0,
            tutorial: None,
            enter_pressed: false,
            dt: 0.0,
            timings: [0.0; 4],
            caption: None,
            keys: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct UiOutput {
    pub resume: bool,
    pub quit: bool,
    pub settings_changed: bool,
    pub bindings_changed: bool,
}

pub struct DebugInfo {
    pub chunks_loaded: usize,
    pub chunks_pending: usize,
    pub gpu_resident: usize,
    pub gpu_slots: u32,
    pub gpu_wanted: usize,
    pub adapter: String,
}

/// Inset (side view) rect in points for a screen of `size` points.
pub fn inset_rect(size: Vec2) -> Rect {
    let w = (size.x * 0.26).clamp(160.0, 420.0);
    let h = w * 0.75;
    Rect::from_min_size(Pos2::new(size.x - w - 12.0, size.y - h - 12.0), Vec2::new(w, h))
}

fn block_color(id: block::BlockId) -> Color32 {
    let c = block::info(id).color;
    // Palette colors are linear; convert for display.
    Color32::from(egui::Rgba::from_rgb(c[0], c[1], c[2]))
}

pub fn draw(
    ctx: &egui::Context,
    ui: &mut UiState,
    game: &mut Game,
    settings: &mut Settings,
    bindings: &mut Bindings,
    input: &InputState,
    debug: &DebugInfo,
) -> UiOutput {
    let mut out = UiOutput::default();
    let screen = ctx.content_rect();
    if let Some(t) = &mut ui.tutorial {
        let enter = std::mem::take(&mut ui.enter_pressed);
        if ui.screen == Screen::Playing && t.update(ui.dt, game, settings, enter) {
            ui.tutorial = None;
            settings.tutorial_done = true;
            out.settings_changed = true;
        }
    }
    if let Some(t) = &ui.tutorial {
        tutorial_panel(ctx, t, bindings, input);
    }
    if let Some((title, sub, alpha)) = &ui.caption {
        caption(ctx, title, sub, *alpha);
    }
    if !ui.keys.is_empty() {
        keycaps(ctx, &ui.keys, screen);
    }
    if ui.show_hud {
        hud(ctx, ui, game, settings, bindings, input, debug, screen);
    }
    match ui.screen {
        Screen::Playing => {}
        Screen::Pause => pause_menu(ctx, ui, game, settings, &mut out),
        Screen::Options => options_menu(ctx, ui, game, settings, &mut out),
        Screen::Controls => controls_menu(ctx, ui, settings, bindings, input, &mut out),
    }
    if ui.screen != Screen::Playing {
        // Dim the world behind menus.
        ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("dim")))
            .rect_filled(screen, 0.0, Color32::from_black_alpha(110));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn hud(
    ctx: &egui::Context,
    ui: &UiState,
    game: &Game,
    settings: &Settings,
    bindings: &Bindings,
    input: &InputState,
    debug: &DebugInfo,
    screen: Rect,
) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("hud")));
    let center = screen.center();
    let shadow = Color32::from_black_alpha(160);
    let text = |pos: Pos2, align: Align2, s: &str, size: f32, color: Color32| {
        painter.text(pos + Vec2::new(1.0, 1.0), align, s, FontId::monospace(size), shadow);
        painter.text(pos, align, s, FontId::monospace(size), color);
    };

    // Crosshair.
    if ui.screen == Screen::Playing {
        for (a, b) in [(Vec2::new(-9.0, 0.0), Vec2::new(9.0, 0.0)), (Vec2::new(0.0, -9.0), Vec2::new(0.0, 9.0))] {
            painter.line_segment([center + a, center + b], Stroke::new(3.5, Color32::from_black_alpha(140)));
            painter.line_segment([center + a, center + b], Stroke::new(1.5, Color32::WHITE));
        }
    }

    // Hotbar.
    let slot = 46.0;
    let start = Pos2::new(center.x - slot * 4.5, screen.max.y - slot - 14.0);
    for i in 0..9 {
        let r = Rect::from_min_size(start + Vec2::new(i as f32 * slot, 0.0), Vec2::splat(slot - 4.0));
        painter.rect_filled(r, 3.0, Color32::from_black_alpha(130));
        if let Some(s) = game.hotbar[i] {
            painter.rect_filled(r.shrink(8.0), 2.0, block_color(s.block));
            if game.mode == GameMode::Survival && s.count > 1 {
                text(r.right_bottom() + Vec2::new(-3.0, -2.0), Align2::RIGHT_BOTTOM, &s.count.to_string(), 13.0, Color32::WHITE);
            }
        }
        let stroke = if i == game.selected { Stroke::new(3.0, Color32::WHITE) } else { Stroke::new(1.0, Color32::from_gray(90)) };
        painter.rect_stroke(r, 3.0, stroke, egui::StrokeKind::Outside);
    }
    if let Some(s) = game.hotbar[game.selected] {
        text(start + Vec2::new(slot * 4.5, -10.0), Align2::CENTER_BOTTOM, block::info(s.block).name, 15.0, Color32::WHITE);
    }

    // Position and orientation.
    let p = game.body.pos;
    let b = game.basis();
    let mode = match (game.mode, game.flying) {
        (GameMode::Creative, true) => "Creative (flying)",
        (GameMode::Creative, false) => "Creative",
        (GameMode::Survival, _) => "Survival",
    };
    let mut lines = vec![
        format!("X {:8.2}  Y {:7.2}", p.x, p.y),
        format!("Z {:8.2}  W {:7.2}", p.z, p.w),
        format!("slice normal (ana) {}", fmt4(b.ana)),
        mode.to_string(),
    ];
    let misalign = game.orient.misalignment().to_degrees();
    if misalign > 0.5 {
        lines.push(format!("slice tilted {misalign:.0}° from axes ({} to snap)", label(bindings, input, Action::SnapSlice)));
    }
    if ui.show_debug {
        lines.push(String::new());
        lines.push(format!("{:.0} fps", ui.fps));
        let t = ui.timings;
        lines.push(format!("cpu ms: update {:.1} acquire {:.1} render {:.1} present {:.1}", t[0], t[1], t[2], t[3]));
        lines.push(format!("forward {}", fmt4(b.forward)));
        lines.push(format!("right   {}", fmt4(b.right)));
        lines.push(format!("chunks: {} loaded, {} generating", debug.chunks_loaded, debug.chunks_pending));
        lines.push(format!(
            "gpu: {} / {} slots, {} chunks near slice",
            debug.gpu_resident, debug.gpu_slots, debug.gpu_wanted
        ));
        lines.push(format!("creatures: {}", game.creatures.len()));
        if let Some(Target::Block(h)) = game.target {
            let id = game.world.block(h.block).unwrap_or(0);
            lines.push(format!("target {} {} face {}", block::info(id).name, h.block, h.normal));
        }
        lines.push(debug.adapter.clone());
    }
    for (i, l) in lines.iter().enumerate() {
        text(Pos2::new(10.0, 10.0 + i as f32 * 17.0), Align2::LEFT_TOP, l, 14.0, Color32::WHITE);
    }

    compass(&painter, game, Pos2::new(screen.max.x - 90.0, 95.0));

    // Side view label.
    if settings.inset {
        let r = inset_rect(screen.size());
        text(r.left_top() + Vec2::new(6.0, 4.0), Align2::LEFT_TOP, "side view: → is ana", 12.0, Color32::WHITE);
    }

    // Toasts.
    for (i, (msg, t)) in game.toasts.iter().enumerate() {
        let a = (t.min(1.0) * 255.0) as u8;
        text(
            Pos2::new(center.x, screen.max.y - 110.0 - i as f32 * 20.0),
            Align2::CENTER_BOTTOM,
            msg,
            16.0,
            Color32::from_rgba_unmultiplied(255, 255, 210, a),
        );
    }

    // Control hints.
    if settings.show_hints && ui.screen == Screen::Playing && ui.tutorial.is_none() {
        let hints = [
            format!(
                "{} / {}: turn into the 4th dimension",
                label(bindings, input, Action::RotateAnaPos),
                label(bindings, input, Action::RotateAnaNeg)
            ),
            format!("hold {} + mouse: rotate the slice", label(bindings, input, Action::SliceModifier)),
            format!(
                "{} / {}: move along ana / kata",
                label(bindings, input, Action::MoveAna),
                label(bindings, input, Action::MoveKata)
            ),
            format!(
                "{}: snap   {}: flip 90° into W",
                label(bindings, input, Action::SnapSlice),
                label(bindings, input, Action::FlipAna)
            ),
            format!(
                "{}: ghosts   {}: side view   {}: 4D glasses",
                label(bindings, input, Action::ToggleGhosts),
                label(bindings, input, Action::ToggleInset),
                label(bindings, input, Action::ToggleGlasses)
            ),
            "Esc: menu (Options → Controls to rebind, hide these hints)".to_string(),
        ];
        let y0 = screen.max.y - 80.0 - hints.len() as f32 * 16.0;
        for (i, h) in hints.iter().enumerate() {
            text(Pos2::new(10.0, y0 + i as f32 * 16.0), Align2::LEFT_TOP, h, 13.0, Color32::from_gray(225));
        }
    }
}

fn fmt4(v: Vec4) -> String {
    format!("({:+.2} {:+.2} {:+.2} {:+.2})", v.x, v.y, v.z, v.w)
}

fn label(bindings: &Bindings, input: &InputState, a: Action) -> String {
    let [p, s] = bindings.get(a);
    match (p.is_bound(), s.is_bound()) {
        (true, true) => format!("{} or {}", p.label(&input.layout_names), s.label(&input.layout_names)),
        (true, false) => p.label(&input.layout_names),
        (false, true) => s.label(&input.layout_names),
        _ => "(unbound)".into(),
    }
}

/// 4D compass: the world's horizontal axes X, Z, W drawn in the camera's
/// (right, forward) plane, with a bar showing how much of each points along ana.
fn compass(painter: &egui::Painter, game: &Game, c: Pos2) {
    let b = game.basis();
    let r = 58.0;
    painter.circle_filled(c, r + 8.0, Color32::from_black_alpha(120));
    painter.circle_stroke(c, r, Stroke::new(1.0, Color32::from_gray(120)));
    let axes = [
        ("X", Vec4::X, Color32::from_rgb(240, 90, 80)),
        ("Z", Vec4::Z, Color32::from_rgb(90, 160, 250)),
        ("W", Vec4::W, Color32::from_rgb(200, 120, 250)),
    ];
    for (i, (name, axis, color)) in axes.iter().enumerate() {
        let x = axis.dot(b.right);
        let y = axis.dot(b.forward_h);
        let a = axis.dot(b.ana);
        let tip = c + Vec2::new(x, -y) * r;
        painter.line_segment([c, tip], Stroke::new(2.5, *color));
        // Negative direction, faint.
        painter.line_segment([c, c - Vec2::new(x, -y) * r], Stroke::new(1.0, color.gamma_multiply(0.35)));
        if (x * x + y * y) > 0.02 {
            painter.text(tip + Vec2::new(x, -y) * 9.0, Align2::CENTER_CENTER, name, FontId::monospace(13.0), *color);
        }
        // Ana component bar under the dial.
        let bar_y = c.y + r + 16.0 + i as f32 * 10.0;
        let half = 50.0;
        painter.line_segment([Pos2::new(c.x - half, bar_y), Pos2::new(c.x + half, bar_y)], Stroke::new(1.0, Color32::from_gray(80)));
        painter.line_segment([Pos2::new(c.x, bar_y), Pos2::new(c.x + a * half, bar_y)], Stroke::new(5.0, *color));
        painter.text(Pos2::new(c.x - half - 6.0, bar_y), Align2::RIGHT_CENTER, name, FontId::monospace(10.0), *color);
    }
    painter.text(c + Vec2::new(0.0, -r - 2.0), Align2::CENTER_BOTTOM, "fwd", FontId::monospace(10.0), Color32::from_gray(200));
    painter.text(
        Pos2::new(c.x + 56.0, c.y + r + 26.0),
        Align2::LEFT_CENTER,
        "ana",
        FontId::monospace(10.0),
        Color32::from_gray(200),
    );
}

fn menu_window(title: &str) -> egui::Window<'_> {
    egui::Window::new(title)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .collapsible(false)
        .resizable(false)
        .title_bar(true)
}

fn big_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add_sized([260.0, 34.0], egui::Button::new(RichText::new(text).size(17.0))).clicked()
}

/// Row of keycaps above the caption; pressed keys light up and sink slightly.
fn keycaps(ctx: &egui::Context, keys: &[(String, bool)], screen: Rect) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("keycaps")));
    let font = FontId::proportional(18.0);
    let widths: Vec<f32> = keys
        .iter()
        .map(|(k, _)| {
            let w = painter.layout_no_wrap(k.clone(), font.clone(), Color32::WHITE).size().x;
            (w + 26.0).max(46.0)
        })
        .collect();
    let gap = 8.0;
    let total: f32 = widths.iter().sum::<f32>() + gap * (keys.len() as f32 - 1.0);
    let mut x = screen.center().x - total / 2.0;
    let y = screen.max.y - 140.0;
    for ((label, down), w) in keys.iter().zip(widths) {
        let sink = if *down { 3.0 } else { 0.0 };
        let r = Rect::from_min_size(Pos2::new(x, y + sink), Vec2::new(w, 44.0));
        // Key side (3D edge), then the cap.
        painter.rect_filled(r.translate(Vec2::new(0.0, 5.0 - sink)), 7.0, Color32::from_rgba_unmultiplied(0, 0, 0, 150));
        let (fill, text, stroke) = if *down {
            (Color32::from_rgb(250, 230, 120), Color32::from_rgb(30, 25, 10), Color32::WHITE)
        } else {
            (Color32::from_rgba_unmultiplied(30, 32, 44, 210), Color32::from_gray(230), Color32::from_gray(120))
        };
        painter.rect_filled(r, 7.0, fill);
        painter.rect_stroke(r, 7.0, Stroke::new(1.5, stroke), egui::StrokeKind::Inside);
        painter.text(r.center(), Align2::CENTER_CENTER, label, font.clone(), text);
        x += w + gap;
    }
}

fn caption(ctx: &egui::Context, title: &str, sub: &str, alpha: f32) {
    let a = |v: u8| (v as f32 * alpha.clamp(0.0, 1.0)) as u8;
    egui::Area::new(egui::Id::unique("caption"))
        .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 16.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(10, 12, 22, a(205)))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(22, 12))
                .show(ui, |ui| {
                    ui.set_min_width(560.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(title).size(28.0).strong().color(Color32::from_rgba_unmultiplied(255, 255, 255, a(255))));
                        if !sub.is_empty() {
                            ui.label(RichText::new(sub).size(17.0).color(Color32::from_rgba_unmultiplied(215, 205, 255, a(255))));
                        }
                    });
                });
        });
}

fn tutorial_panel(ctx: &egui::Context, t: &Tutorial, bindings: &Bindings, input: &InputState) {
    let v = t.view(bindings, input);
    egui::Area::new(egui::Id::unique("tutorial"))
        .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 14.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(12, 14, 24, 215))
                .stroke(Stroke::new(1.0, Color32::from_rgb(150, 120, 230)))
                .corner_radius(6.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(560.0);
                    ui.label(
                        RichText::new(format!("Tutorial {}/{} · {}", v.index + 1, v.count, v.title))
                            .strong()
                            .size(17.0)
                            .color(Color32::from_rgb(210, 190, 255)),
                    );
                    ui.add_space(4.0);
                    ui.label(RichText::new(&v.text).size(15.0).color(Color32::WHITE));
                    ui.add_space(6.0);
                    let footer = if v.completed {
                        RichText::new("✔ Nice!").color(Color32::from_rgb(120, 230, 120)).strong()
                    } else if v.needs_enter {
                        RichText::new("Press Enter to continue").color(Color32::YELLOW)
                    } else {
                        RichText::new("Objective: try it now").color(Color32::from_gray(190))
                    };
                    ui.horizontal(|ui| {
                        ui.label(footer.size(14.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new("Esc → Stop Tutorial to skip").weak().size(12.0));
                        });
                    });
                });
        });
}

fn pause_menu(ctx: &egui::Context, ui_state: &mut UiState, game: &mut Game, settings: &mut Settings, out: &mut UiOutput) {
    menu_window("Game Menu").show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            if big_button(ui, "Back to Game") {
                out.resume = true;
            }
            if big_button(ui, "Options…") {
                ui_state.screen = Screen::Options;
            }
            if big_button(ui, "Controls…") {
                ui_state.screen = Screen::Controls;
            }
            if ui_state.tutorial.is_some() {
                if big_button(ui, "Stop Tutorial") {
                    ui_state.tutorial = None;
                    settings.tutorial_done = true;
                    out.settings_changed = true;
                }
            } else if big_button(ui, "Tutorial") {
                ui_state.tutorial = Some(Tutorial::new(game, settings));
                out.resume = true;
            }
            let other = match game.mode {
                GameMode::Survival => GameMode::Creative,
                GameMode::Creative => GameMode::Survival,
            };
            if big_button(ui, &format!("Game Mode: {:?}", game.mode)) {
                game.set_mode(other);
            }
            if big_button(ui, "Save Game") {
                game.save_all();
                game.toast("Game saved");
            }
            if big_button(ui, "Save and Quit") {
                out.quit = true;
            }
        });
    });
}

fn options_menu(ctx: &egui::Context, ui_state: &mut UiState, _game: &mut Game, s: &mut Settings, out: &mut UiOutput) {
    let before = s.clone();
    menu_window("Options").show(ctx, |ui| {
        egui::Grid::new("options").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
            ui.label("Field of view");
            ui.add(egui::Slider::new(&mut s.fov, 50.0..=110.0).suffix("°"));
            ui.end_row();
            ui.label("Render distance");
            ui.add(egui::Slider::new(&mut s.render_distance, 2..=6).suffix(" chunks"));
            ui.end_row();
            ui.label("Render scale");
            ui.add(egui::Slider::new(&mut s.render_scale, 0.25..=1.0));
            ui.end_row();
            ui.label("Sun shadows");
            ui.checkbox(&mut s.shadows, "");
            ui.end_row();
            ui.label("Cross-section edges");
            ui.checkbox(&mut s.edges, "");
            ui.end_row();
            ui.separator();
            ui.separator();
            ui.end_row();
            ui.label("Side view (perpendicular slice)");
            ui.checkbox(&mut s.inset, "");
            ui.end_row();
            ui.label("Ghost layers (red = ana, blue = kata)");
            ui.checkbox(&mut s.ghosts, "");
            ui.end_row();
            ui.label("Ghost layer distance");
            ui.add(egui::Slider::new(&mut s.ghost_offset, 0.5..=3.0).suffix(" blocks"));
            ui.end_row();
            ui.label("4D glasses (creature outlines)");
            ui.checkbox(&mut s.glasses, "");
            ui.end_row();
            ui.label("Auto-snap slice to axes");
            ui.checkbox(&mut s.auto_snap, "");
            ui.end_row();
            ui.label("Show control hints");
            ui.checkbox(&mut s.show_hints, "");
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Reset to defaults").clicked() {
                *s = Settings::default();
            }
            if ui.button("Done").clicked() {
                ui_state.screen = Screen::Pause;
            }
        });
    });
    if *s != before {
        out.settings_changed = true;
    }
}

fn controls_menu(
    ctx: &egui::Context,
    ui_state: &mut UiState,
    s: &mut Settings,
    bindings: &mut Bindings,
    input: &InputState,
    out: &mut UiOutput,
) {
    let before = s.clone();
    let names = &input.layout_names;
    menu_window("Controls").default_height(560.0).show(ctx, |ui| {
        egui::Grid::new("mouse").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
            ui.label("Mouse sensitivity");
            ui.add(egui::Slider::new(&mut s.mouse_sensitivity, 0.1..=4.0));
            ui.end_row();
            ui.label("Invert mouse Y");
            ui.checkbox(&mut s.invert_y, "");
            ui.end_row();
            ui.label("Slice rotation: wheel step");
            ui.add(egui::Slider::new(&mut s.wheel_step_deg, 1.0..=30.0).suffix("°"));
            ui.end_row();
            ui.label("Slice rotation: key speed");
            ui.add(egui::Slider::new(&mut s.key_rotate_speed, 10.0..=180.0).suffix("°/s"));
            ui.end_row();
            ui.label("Slice rotation: modifier + mouse");
            ui.add(egui::Slider::new(&mut s.slice_mouse_sensitivity, 0.1..=4.0));
            ui.end_row();
        });
        ui.separator();
        if ui_state.capturing.is_some() {
            ui.label(RichText::new("Press a key, mouse button or scroll the wheel… (Esc to cancel)").color(Color32::YELLOW));
        } else {
            ui.label("Click a binding to change it. Conflicting bindings are shown in red.");
        }
        egui::ScrollArea::vertical().max_height(430.0).show(ui, |ui| {
            for cat in Category::ALL {
                ui.add_space(6.0);
                ui.label(RichText::new(cat.name()).strong().size(16.0));
                egui::Grid::new(cat.name()).num_columns(4).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
                    for &a in Action::ALL.iter().filter(|a| a.category() == cat) {
                        ui.label(a.name());
                        let conflict = !bindings.conflicts(a).is_empty();
                        let current = bindings.get(a);
                        for slot in 0..2 {
                            let capturing = ui_state.capturing == Some((a, slot));
                            let mut text = if capturing { "> ? <".to_string() } else { current[slot].label(names) };
                            if !capturing && slot == 0 && !bindings.is_default(a) {
                                text = format!("{text} *");
                            }
                            let mut rt = RichText::new(text);
                            if conflict && current[slot].is_bound() && !capturing {
                                rt = rt.color(Color32::from_rgb(255, 90, 80));
                            }
                            let btn = ui.add_sized([130.0, 22.0], egui::Button::new(rt));
                            if btn.clicked() {
                                ui_state.capturing = Some((a, slot));
                            }
                            if btn.secondary_clicked() {
                                bindings.set(a, slot, crate::input::Binding::Unbound);
                                out.bindings_changed = true;
                            }
                            if conflict {
                                btn.on_hover_text(format!(
                                    "Also used by: {}",
                                    bindings.conflicts(a).iter().map(|c| c.name()).collect::<Vec<_>>().join(", ")
                                ));
                            }
                        }
                        let reset = ui.add_enabled(!bindings.is_default(a), egui::Button::new("Reset"));
                        if reset.clicked() {
                            bindings.reset(a);
                            out.bindings_changed = true;
                        }
                        ui.end_row();
                    }
                });
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Right-click a binding to clear it.").weak());
            if ui.button("Reset All Keys").clicked() {
                *bindings = Bindings::default();
                out.bindings_changed = true;
            }
            if ui.button("Done").clicked() {
                ui_state.capturing = None;
                ui_state.screen = Screen::Pause;
            }
        });
    });
    if *s != before {
        out.settings_changed = true;
    }
}
