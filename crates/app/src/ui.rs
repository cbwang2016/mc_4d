//! HUD and menus (egui): crosshair, hotbar, 4D compass, pause menu, options and the
//! Minecraft-style Controls screen.

#[path = "classic_ui.rs"]
pub(crate) mod classic_ui;
#[path = "inventory_ui.rs"]
mod inventory_ui;
#[path = "progress_ui.rs"]
mod progress_ui;
#[path = "world_ui.rs"]
mod world_ui;
use inventory_ui::paint_stack_icon;

use crate::game::{Game, GameMode, Target};
use crate::i18n::{Language, text as tr};
use crate::input::{Action, Bindings, Category, InputState};
use crate::settings::Settings;
use crate::tutorial::Tutorial;
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2};
use mc4d_core::{block, item};
use mc4d_core::glam::Vec4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Playing,
    Title,
    Worlds,
    CreateWorld,
    RenameWorld,
    DeleteWorld,
    Loading,
    TexturePacks,
    Inventory,
    Chat,
    SignEditor,
    Death,
    Pause,
    Achievements,
    Statistics,
    Options,
    VideoOptions,
    FourDOptions,
    Controls,
}

pub struct UiState {
    pub screen: Screen,
    pub menu_parent: Screen,
    pub world_name: String,
    pub world_seed: String,
    pub world_mode: GameMode,
    pub loading_progress: f32,
    pub worlds: Vec<crate::worlds::Entry>,
    pub world_rename: String,
    pub world_error: Option<String>,
    pub inventory_catalog: bool,
    pub inventory_page: usize,
    pub chat_input: String,
    pub chat_history: Vec<String>,
    pub chat_focus: bool,
    pub sign_editor: Option<(mc4d_core::glam::IVec4,[String;4])>,
    pub sign_focus: bool,
    pub texture_error: Option<String>,
    pub texture_pack_draft: String,
    /// Folder scanned for texture pack folders and .zip files.
    pub texture_pack_root: Option<std::path::PathBuf>,
    pub inventory_cursor: Option<crate::game::Slot>,
    /// Waiting for an input to bind to (action, slot).
    pub capturing: Option<(Action, usize)>,
    pub controls_category: usize,
    pub progress_ui: progress_ui::State,
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
    /// Recording omits prompts which require a live player to press a key.
    pub video_presentation: bool,
    /// Draw the per-view compasses even when the F3 overlay is hidden.
    pub video_compass: bool,
    /// Video keystroke overlay: key labels and whether each is pressed.
    pub keys: Vec<(String, bool)>,
}

impl UiState {
    pub fn shows_world(&self)->bool {
        !matches!(self.screen,Screen::Title|Screen::Worlds|Screen::CreateWorld|Screen::RenameWorld|Screen::DeleteWorld|Screen::Loading|Screen::Achievements|Screen::Statistics)
            && !(self.menu_parent==Screen::Title && matches!(self.screen,Screen::Options|Screen::VideoOptions|Screen::FourDOptions|Screen::Controls|Screen::TexturePacks))
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            screen: Screen::Playing,
            menu_parent: Screen::Pause,
            world_name: "world".into(),
            world_seed: "1337".into(),
            world_mode: GameMode::Survival,
            loading_progress: 0.0,
            worlds: Vec::new(),
            world_rename: String::new(),
            world_error: None,
            inventory_catalog: true,
            inventory_page: 0,
            chat_input: String::new(),
            chat_history: Vec::new(),
            chat_focus: false,
            sign_editor: None,
            sign_focus: false,
            texture_error: None,
            texture_pack_draft: String::new(),
            texture_pack_root: None,
            inventory_cursor: None,
            capturing: None,
            controls_category: 0,
            progress_ui: Default::default(),
            show_hud: true,
            show_debug: true,
            fps: 0.0,
            tutorial: None,
            enter_pressed: false,
            dt: 0.0,
            timings: [0.0; 4],
            caption: None,
            video_presentation: false,
            video_compass: false,
            keys: Vec::new(),
        }
    }
}

pub struct WorldRequest { pub name: String, pub seed: i64, pub mode: GameMode, pub create: bool }

#[derive(Default)]
pub struct UiOutput {
    pub load_world: Option<WorldRequest>,
    pub refresh_worlds: bool,
    pub rename_world: Option<(String,String)>,
    pub delete_world: Option<String>,
    pub start_tutorial: bool,
    pub resume: bool,
    pub respawn: bool,
    pub quit: bool,
    pub return_to_title: bool,
    pub settings_changed: bool,
    pub bindings_changed: bool,
}

pub struct DebugInfo {
    pub chunks_loaded: usize,
    pub chunks_pending: usize,
    pub lighting_pending: usize,
    pub lighting_missing: usize,
    pub gpu_resident: usize,
    pub gpu_slots: u32,
    pub gpu_wanted: usize,
    pub adapter: String,
    /// Internal render scale this frame.
    pub render_scale: f32,
    pub gpu_ms: Option<f32>,
}

fn slice_rects(size: Vec2, count: u32) -> Vec<Rect> {
    mc4d_core::views::view_rects([size.x, size.y], count).into_iter()
        .map(|[x, y, w, h]| Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)))
        .collect()
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
    classic_ui::set_pack(ctx, &settings.texture_pack);
    let mut out = UiOutput::default();
    let screen = ctx.content_rect();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("game-gui-scale"),settings.gui_scale));
    if matches!(ui.screen, Screen::Title | Screen::Worlds | Screen::CreateWorld | Screen::RenameWorld | Screen::DeleteWorld | Screen::Loading | Screen::Achievements | Screen::Statistics) || (ui.menu_parent == Screen::Title && matches!(ui.screen, Screen::Options | Screen::VideoOptions | Screen::FourDOptions | Screen::Controls | Screen::TexturePacks)) {
        title_background(ctx, screen);
    }
    if let Some(t) = &mut ui.tutorial {
        let enter = std::mem::take(&mut ui.enter_pressed);
        if ui.screen == Screen::Playing && t.update(ui.dt, game, settings, enter) {
            ui.tutorial = None;
            settings.tutorial_done = true;
            out.settings_changed = true;
        }
    }
    if ui.screen == Screen::Playing {
        if let Some(t) = &ui.tutorial {
            tutorial_panel(ctx, t, bindings, input, settings.language, ui.video_presentation);
        }
    }
    if let Some((title, sub, alpha)) = &ui.caption {
        caption(ctx, title, sub, *alpha);
    }
    if !ui.keys.is_empty() {
        keycaps(ctx, &ui.keys, screen);
    }
    if ui.show_hud && ui.screen == Screen::Playing && !ui.tutorial.as_ref().is_some_and(|t| t.is_conceptual()) {
        hud(ctx, ui, game, settings, bindings, input, debug, screen);
    }
    match ui.screen {
        Screen::Playing => {}
        Screen::Chat => command_panel(ctx, ui, game, settings.language, &mut out),
        Screen::SignEditor => sign_editor(ctx,ui,game,settings.language,&mut out),
        Screen::Title => title_menu(ctx, ui, settings, &mut out),
        Screen::TexturePacks => texture_pack_menu(ctx, ui, settings, &mut out),
        Screen::Worlds | Screen::CreateWorld | Screen::RenameWorld | Screen::DeleteWorld => world_ui::draw(ctx, ui, settings.language, &mut out),
        Screen::Loading => loading_screen(ctx,ui,settings.language),
        Screen::Inventory => inventory_ui::draw(ctx, ui, game, settings, &mut out),
        Screen::Death => death_menu(ctx, game, settings.language, &mut out),
        Screen::Achievements => progress_ui::achievements(ctx,ui,game,settings.language,bindings),
        Screen::Statistics => progress_ui::statistics(ctx,ui,game,settings.language),
        Screen::Pause => pause_menu(ctx, ui, game, settings, &mut out),
        Screen::Options | Screen::VideoOptions | Screen::FourDOptions => crate::options_ui::draw(ctx,ui,settings,&mut out),
        Screen::Controls => controls_menu(ctx, ui, settings, bindings, input, &mut out),
    }
    if !matches!(ui.screen, Screen::Playing | Screen::Chat) {
        // Dim the world behind menus.
        ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("dim")))
            .rect_filled(screen, 0.0, Color32::from_black_alpha(110));
    }
    progress_ui::toast(ctx, ui, game, settings.language);
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
    let language = settings.language;
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("hud")));
    let center = screen.center();
    let shadow = Color32::from_black_alpha(160);
    let text = |pos: Pos2, align: Align2, s: &str, size: f32, color: Color32| {
        painter.text(pos + Vec2::new(1.0, 1.0), align, s, FontId::monospace(size), shadow);
        painter.text(pos, align, s, FontId::monospace(size), color);
    };

    // Crosshair.
    let view_rects = slice_rects(screen.size(), settings.active_view_count());
    let aim_center = view_rects[0].center() + screen.min.to_vec2();
    if ui.screen == Screen::Playing {
        for (a, b) in [(Vec2::new(-9.0, 0.0), Vec2::new(9.0, 0.0)), (Vec2::new(0.0, -9.0), Vec2::new(0.0, 9.0))] {
            painter.line_segment([aim_center + a, aim_center + b], Stroke::new(3.5, Color32::from_black_alpha(140)));
            painter.line_segment([aim_center + a, aim_center + b], Stroke::new(1.5, Color32::WHITE));
        }
    }

    // Original Beta HUD atlas: nine hotbar slots, health, armor and air (no hunger).
    let assets=classic_ui::current_assets(ctx);let scale=classic_ui::gui_scale(ctx);
    let start=Pos2::new(center.x-91.0*scale,screen.bottom()-22.0*scale);
    classic_ui::region(&painter,&assets.textures[4],Rect::from_min_size(start,Vec2::new(182.0*scale,22.0*scale)),[0.0,0.0,182.0,22.0]);
    classic_ui::region(&painter,&assets.textures[4],Rect::from_min_size(start+Vec2::new((game.selected as f32*20.0-1.0)*scale,-scale),Vec2::splat(24.0*scale)),[0.0,22.0,24.0,24.0]);
    for (index,slot) in game.hotbar.iter().enumerate() {
        if let Some(stack)=slot {
            let r=Rect::from_min_size(start+Vec2::new((3.0+index as f32*20.0)*scale,3.0*scale),Vec2::splat(16.0*scale));
            paint_stack_icon(ctx,&painter,r,*stack);
            if stack.count>1 {
                let count=stack.count.to_string();
                classic_ui::classic_text(&painter,&assets,&count,r.right_bottom()-Vec2::new(classic_ui::text_width(&assets,&count,scale),7.0*scale),scale,Color32::WHITE,true);
            }
            let maximum=item::max_durability(stack.block);
            if maximum>0 && stack.damage>0 {
                let remaining=(1.0-stack.damage as f32/maximum as f32).clamp(0.0,1.0);
                let bar=Rect::from_min_size(r.left_bottom()+Vec2::new(2.0*scale,-2.0*scale),Vec2::new(13.0*scale,2.0*scale));
                painter.rect_filled(bar,0.0,Color32::BLACK);
                painter.rect_filled(Rect::from_min_size(bar.min,Vec2::new((13.0*remaining).round()*scale,scale)),0.0,
                    Color32::from_rgb((255.0*(1.0-remaining)) as u8,(255.0*remaining) as u8,0));
            }
        }
    }
    if let Some(stack)=game.hotbar[game.selected].filter(|_|settings.show_hints) {
        let name=tr(language,item::stack_name(stack));
        classic_ui::classic_text(&painter,&assets,name,Pos2::new(center.x-classic_ui::text_width(&assets,name,scale)/2.0,
            start.y-if game.mode==GameMode::Survival {23.0*scale}else{12.0*scale}),scale,Color32::WHITE,true);
    }
    if game.mode==GameMode::Survival {
        for i in 0..10u8 {
            let r=Rect::from_min_size(start+Vec2::new(i as f32*8.0*scale,-10.0*scale),Vec2::splat(9.0*scale));
            classic_ui::region(&painter,&assets.textures[7],r,[16.0,0.0,9.0,9.0]);
            let units=game.health.saturating_sub(i*2).min(2);
            if units>0 {classic_ui::region(&painter,&assets.textures[7],r,[if units==2 {52.0}else{61.0},0.0,9.0,9.0]);}
            let armor=game.armor_points();
            if armor>0 {
                let r=Rect::from_min_size(start+Vec2::new((173.0-i as f32*8.0)*scale,-10.0*scale),Vec2::splat(9.0*scale));
                let units=armor.saturating_sub(i*2).min(2);
                classic_ui::region(&painter,&assets.textures[7],r,[match units{2=>34.0,1=>25.0,_=>16.0},9.0,9.0,9.0]);
            }
            if game.air<15.0 {
                let r=Rect::from_min_size(start+Vec2::new((173.0-i as f32*8.0)*scale,-20.0*scale),Vec2::splat(9.0*scale));
                classic_ui::region(&painter,&assets.textures[7],r,[if game.air/1.5>i as f32{16.0}else{25.0},18.0,9.0,9.0]);
            }
        }
    }

    // Coordinates and orientation are part of F3, like the remaining diagnostics.
    if ui.show_debug {
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
        format!("{} {}", if language == Language::Chinese { "截面法向（ana）" } else { "slice normal (ana)" }, fmt4(b.ana)),
        tr(language, mode).to_string(),
    ];
    let misalign = game.orient.misalignment().to_degrees();
    if misalign > 0.5 {
        lines.push(if language == Language::Chinese {
            format!("截面偏离坐标轴 {misalign:.0}°（{} 对齐）", label(bindings, input, language, Action::SnapSlice))
        } else { format!("slice tilted {misalign:.0}° from axes ({} to snap)", label(bindings, input, language, Action::SnapSlice)) });
    }
    if ui.show_debug {
        lines.push(String::new());
        lines.push(format!("{:.0} fps, {} {:.0}%{}", ui.fps, if language==Language::Chinese {"渲染比例"} else {"render scale"}, debug.render_scale*100.0, debug.gpu_ms.map_or(String::new(),|ms|format!(", GPU {ms:.1} ms"))));
        let t = ui.timings;
        lines.push(format!("cpu ms: update {:.1} acquire {:.1} render {:.1} present {:.1}", t[0], t[1], t[2], t[3]));
        lines.push(format!("forward {}", fmt4(b.forward)));
        lines.push(format!("right   {}", fmt4(b.right)));
        lines.push(format!("chunks: {} loaded, {} generating", debug.chunks_loaded, debug.chunks_pending));
        lines.push(format!("lighting: {} queued, {} GPU chunks unready", debug.lighting_pending, debug.lighting_missing));
        lines.push(format!(
            "gpu: {} / {} slots, {} chunks near slice",
            debug.gpu_resident, debug.gpu_slots, debug.gpu_wanted
        ));
        lines.push(format!("creatures: {}", game.creatures.len()));
        if let Some(name)=game.targeted_vehicle_name(){lines.push(format!("{}: {}",if language==Language::Chinese{"载具"}else{"vehicle"},tr(language,name)));}
        if let Some(Target::Block(h)) = game.target {
            let id = game.world.block(h.block).unwrap_or(0);
            lines.push(format!("target {} {} face {}", item::info(id).name, h.block, h.normal));
        }
        lines.push(debug.adapter.clone());
    }
    for (i, l) in lines.iter().enumerate() {
        text(Pos2::new(10.0, 10.0 + i as f32 * 17.0), Align2::LEFT_TOP, l, 14.0, Color32::WHITE);
    }

    }

    // Every slice view carries its own compass in its top-right corner (with F3,
    // or in videos, which show compasses without the diagnostic text).
    if ui.show_debug || ui.video_compass {
    let bases=mc4d_core::views::slice_bases(game.basis(),settings.active_view_count());
    let shown=if settings.inset {view_rects.len().min(bases.len())} else {1};
    for (i,rect) in view_rects.iter().enumerate().take(shown) {
        // In grid layouts the play view is one equal cell: compact like the rest.
        if let Some(layout)=compass_layout(rect.translate(screen.min.to_vec2()),scale,i>0 || shown>2) {
            compass(&painter,bases[i],layout,language);
        }
    }
    }

    // Labels identify the primary interaction view and the other slices.
    if settings.inset {
        for (i, r) in view_rects.iter().enumerate() {
            let label = tr(settings.language, mc4d_core::views::VIEW_LABELS[i]);
            text(r.right_top() + screen.min.to_vec2() + Vec2::new(-8.0, 5.0), Align2::RIGHT_TOP, label, 12.0, Color32::WHITE);
        }
    }

    // Toasts.
    for (i, (msg, t)) in game.toasts.iter().enumerate() {
        let a = (t.min(1.0) * 255.0) as u8;
        text(
            Pos2::new(center.x, screen.max.y - 110.0 - i as f32 * 20.0),
            Align2::CENTER_BOTTOM,
            tr(language, msg),
            16.0,
            Color32::from_rgba_unmultiplied(255, 255, 210, a),
        );
    }

    // Control hints.
    if settings.show_hints && ui.screen == Screen::Playing && ui.tutorial.is_none() {
        let hints = if language == Language::English { [
            format!(
                "{} / {}: turn into the 4th dimension",
                label(bindings, input, language, Action::RotateAnaPos),
                label(bindings, input, language, Action::RotateAnaNeg)
            ),
            format!("hold {} + mouse: rotate the slice", label(bindings, input, language, Action::SliceModifier)),
            format!(
                "{} / {}: move along ana / kata",
                label(bindings, input, language, Action::MoveAna),
                label(bindings, input, language, Action::MoveKata)
            ),
            format!(
                "{}: snap   {}: flip 90° into W",
                label(bindings, input, language, Action::SnapSlice),
                label(bindings, input, language, Action::FlipAna)
            ),
            format!("{}: switch main to side view",label(bindings,input,language,Action::SwitchToSideView)),
            format!(
                "{}: ghosts   {}: side view   {}: 4D glasses",
                label(bindings, input, language, Action::ToggleGhosts),
                label(bindings, input, language, Action::ToggleInset),
                label(bindings, input, language, Action::ToggleGlasses)
            ),
            "Esc: menu (Options → Controls to rebind, hide these hints)".to_string(),
        ] } else { [
            format!("{} / {}：向第四维旋转", label(bindings, input, language, Action::RotateAnaPos), label(bindings, input, language, Action::RotateAnaNeg)),
            format!("按住 {} ＋鼠标：自由旋转截面", label(bindings, input, language, Action::SliceModifier)),
            format!("{} / {}：沿 ana／kata 移动", label(bindings, input, language, Action::MoveAna), label(bindings, input, language, Action::MoveKata)),
            format!("{}：对齐坐标轴   {}：向 W 翻转 90°", label(bindings, input, language, Action::SnapSlice), label(bindings, input, language, Action::FlipAna)),
            format!("{}：主视图切换到侧视图",label(bindings,input,language,Action::SwitchToSideView)),
            format!("{}：相邻截面   {}：多视图   {}：四维眼镜", label(bindings, input, language, Action::ToggleGhosts), label(bindings, input, language, Action::ToggleInset), label(bindings, input, language, Action::ToggleGlasses)),
            "Esc：菜单（选项／控制设置可修改按键和隐藏提示）".into(),
        ] };
        let y0 = screen.max.y - 80.0 - hints.len() as f32 * 16.0;
        for (i, h) in hints.iter().enumerate() {
            text(Pos2::new(10.0, y0 + i as f32 * 16.0), Align2::LEFT_TOP, h, 13.0, Color32::from_gray(225));
        }
    }
}

fn fmt4(v: Vec4) -> String {
    format!("({:+.2} {:+.2} {:+.2} {:+.2})", v.x, v.y, v.z, v.w)
}

fn label(bindings: &Bindings, input: &InputState, language: Language, a: Action) -> String {
    let [p, s] = bindings.get(a);
    let key = |b: crate::input::Binding| tr(language, &b.label(&input.layout_names)).to_string();
    match (p.is_bound(), s.is_bound()) {
        (true, true) => format!("{} / {}", key(p), key(s)),
        (true, false) => key(p),
        (false, true) => key(s),
        _ => tr(language, "unbound").into(),
    }
}

/// A compass uses the same logical-point viewport as FrameRenderer. Egui applies
/// native DPI scaling; the fitted layout leaves the existing view-label row clear.
#[derive(Clone,Copy)]
struct CompassLayout {center:Pos2,radius:f32,factor:f32,text_factor:f32,bar_step:f32,bar_gap:f32,bounds:Rect}
fn compass_layout(view:Rect,gui_scale:f32,compact:bool)->Option<CompassLayout>{
    if view.width()<120.0 || view.height()<92.0 {return None;}
    let preferred=if compact {22.0}else{42.0}*(gui_scale.max(1.0)/2.0).sqrt();
    let radius=preferred.min(view.width()*0.2-12.0).min((view.height()-36.0)/3.7);
    let factor=radius/58.0;let text_factor=factor.max(0.7);
    let bar_step=(10.0*factor).max(8.0);let bar_gap=(16.0*factor).max(8.0);
    let pad=(8.0*factor).max(4.0);let label=(9.0*factor).max(4.0)+6.5*text_factor;
    let half=50.0*factor;
    let left=(radius+pad).max(radius+label).max(half+18.0*text_factor);
    let right=(radius+pad).max(radius+label).max(half+30.0*text_factor);
    let top=radius+pad+10.0*text_factor;let bottom=radius+bar_gap+2.0*bar_step+5.0*text_factor;
    let margin=6.0*text_factor;
    let center=Pos2::new(view.right()-right-margin,view.top()+22.0+top);
    let bounds=Rect::from_min_max(center-Vec2::new(left,top),center+Vec2::new(right,bottom));
    if bounds.bottom()>view.bottom()-margin || bounds.left()<view.left()+margin{return None;}
    Some(CompassLayout{center,radius,factor,text_factor,bar_step,bar_gap,bounds})
}
fn compass_projection(b:mc4d_core::camera::Basis,axis:Vec4)->(Vec2,f32){
    (Vec2::new(axis.dot(b.right),-axis.dot(b.forward_h)),axis.dot(b.ana))
}
/// World X/Z/W projected into this particular view, including their normal parts.
fn compass(painter:&egui::Painter,b:mc4d_core::camera::Basis,layout:CompassLayout,language:Language){
    let CompassLayout{center:c,radius:r,factor:s,text_factor:ts,bar_step,bar_gap,bounds}=layout;
    let painter=painter.with_clip_rect(bounds.expand(0.5));
    painter.rect_filled(bounds.expand(1.0),4.0,Color32::from_black_alpha(105));
    painter.circle_stroke(c,r,Stroke::new((s).max(0.75),Color32::from_gray(140)));
    let axes=[("X",Vec4::X,Color32::from_rgb(240,90,80)),("Z",Vec4::Z,Color32::from_rgb(90,160,250)),("W",Vec4::W,Color32::from_rgb(200,120,250))];
    for(i,(name,axis,color))in axes.iter().enumerate(){
        let (projected,normal)=compass_projection(b,*axis);let tip=c+projected*r;
        painter.line_segment([c,tip],Stroke::new((2.5*s).max(1.5),*color));
        painter.line_segment([c,c-projected*r],Stroke::new((s).max(0.75),color.gamma_multiply(0.35)));
        if projected.length_sq()>0.02 {painter.text(tip+projected*(9.0*s).max(4.0),Align2::CENTER_CENTER,name,FontId::monospace(13.0*ts),*color);}
        let y=c.y+r+bar_gap+i as f32*bar_step;let half=50.0*s;
        painter.line_segment([Pos2::new(c.x-half,y),Pos2::new(c.x+half,y)],Stroke::new((s).max(0.75),Color32::from_gray(90)));
        painter.line_segment([Pos2::new(c.x,y),Pos2::new(c.x+normal*half,y)],Stroke::new((5.0*s).max(2.0),*color));
        painter.text(Pos2::new(c.x-half-6.0*ts,y),Align2::RIGHT_CENTER,name,FontId::monospace(10.0*ts),*color);
    }
    painter.text(c+Vec2::new(0.0,-r*0.5),Align2::CENTER_CENTER,if language==Language::Chinese{"前"}else{"fwd"},FontId::monospace(10.0*ts),Color32::from_gray(210));
    painter.text(Pos2::new(c.x+50.0*s+6.0*ts,c.y+r+bar_gap+bar_step),Align2::LEFT_CENTER,"ana",FontId::monospace(10.0*ts),Color32::from_gray(210));
}

fn menu_window(title: &str) -> egui::Window<'_> {
    egui::Window::new(title)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .collapsible(false)
        .resizable(false)
        .title_bar(true)
}

fn big_button(ui: &mut egui::Ui, text: &str) -> bool {
    let scale=classic_ui::gui_scale(ui.ctx());
    classic_ui::button(ui,text,200.0*scale,20.0*scale).clicked()
}

fn title_background(ctx: &egui::Context, screen: Rect) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("title-background")));
    painter.rect_filled(screen,0.0,Color32::from_rgb(40,30,22));
    if let Some(textures) = ctx.data(|data| data.get_temp::<[egui::TextureId;2]>(egui::Id::unique("inventory-atlas-ids"))) {
        let uv = mc4d_render::atlas_icons::block_icon(block::DIRT,mc4d_render::atlas_icons::BlockFace::Side).unwrap().uv();
        for y in 0..(screen.height()/64.0).ceil() as i32 {
            for x in 0..(screen.width()/64.0).ceil() as i32 {
                let rect=Rect::from_min_size(screen.min+Vec2::new(x as f32*64.0,y as f32*64.0),Vec2::splat(64.0));
                painter.image(textures[0],rect,Rect::from_min_max(uv[0].into(),uv[1].into()),Color32::from_gray(95));
            }
        }
    }
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
    egui::Area::new(egui::Id::unique("caption")).fade_in(false)
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

fn tutorial_panel(ctx: &egui::Context, t: &Tutorial, bindings: &Bindings, input: &InputState, language: Language, presentation: bool) {
    let v = t.view(bindings, input, language);
    let conceptual = v.diagram.is_some();
    let bounds = ctx.content_rect();
    let width = (if conceptual { 900.0_f32 } else { 560.0 }).min((bounds.width()-40.0).max(180.0));
    egui::Area::new(egui::Id::unique("tutorial")).fade_in(false)
        .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 14.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(12, 14, 24, if conceptual { 245 } else { 215 }))
                .stroke(Stroke::new(1.0, Color32::from_rgb(150, 120, 230)))
                .corner_radius(6.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.label(
                        RichText::new(if presentation {v.title.to_owned()} else {format!("{} {}/{} · {}", tr(language, "Tutorial"), v.index + 1, v.count, v.title)})
                            .strong()
                            .size(17.0)
                            .color(Color32::from_rgb(210, 190, 255)),
                    );
                    ui.add_space(4.0);
                    if let Some(diagram) = v.diagram {
                        let diagram_height = ((bounds.height()-280.0)*0.60).clamp(150.0,300.0);
                        crate::tutorial_visual::draw(ui, diagram, v.animation_time, language, diagram_height);
                        ui.add_space(10.0);
                    }
                    ui.label(RichText::new(&v.text).size(15.0).color(Color32::WHITE));
                    if !presentation {
                    ui.add_space(6.0);
                    let footer = if v.completed {
                        RichText::new(tr(language, "✔ Nice!")).color(Color32::from_rgb(120, 230, 120)).strong()
                    } else if v.needs_enter {
                        RichText::new(tr(language, "Press Enter to continue")).color(Color32::YELLOW)
                    } else {
                        RichText::new(tr(language, "Objective: try it now")).color(Color32::from_gray(190))
                    };
                    ui.horizontal(|ui| {
                        ui.label(footer.size(14.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(tr(language, "Esc → Stop Tutorial to skip")).color(Color32::from_gray(170)).size(12.0));
                        });
                    });
                    }
                });
        });
}

fn command_panel(ctx: &egui::Context, state: &mut UiState, game: &mut Game, language: Language, out: &mut UiOutput) {
    egui::Area::new(egui::Id::unique("command-panel")).fade_in(false).anchor(Align2::LEFT_BOTTOM, Vec2::ZERO).show(ctx, |ui| {
        ui.set_width(ctx.content_rect().width()-16.0);
        egui::Frame::new().fill(Color32::from_black_alpha(180)).inner_margin(8.0).show(ui, |ui| {
        for line in state.chat_history.iter().rev().take(8).collect::<Vec<_>>().into_iter().rev() {
            ui.label(RichText::new(line).color(Color32::WHITE));
        }
        let response = ui.add(egui::TextEdit::singleline(&mut state.chat_input).desired_width(f32::INFINITY)
            .hint_text(tr(language,"Type /help for commands")));
        if state.chat_focus { response.request_focus(); state.chat_focus = false; }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let line = state.chat_input.trim().to_owned();
            if !line.is_empty() {
                state.chat_history.push(format!("> {line}"));
                let result = game.command(&line, language).unwrap_or_else(|e| e);
                state.chat_history.push(result.clone());
                game.toast(result);
                if state.chat_history.len() > 100 { state.chat_history.drain(..state.chat_history.len()-100); }
            }
            state.chat_input.clear();
            out.resume = true;
        }
        });
    });
}

fn sign_editor(ctx:&egui::Context,state:&mut UiState,game:&mut Game,language:Language,out:&mut UiOutput) {
    let Some((position,lines))=state.sign_editor.as_mut() else {out.resume=true;return;};
    let position=*position;let scale=classic_ui::gui_scale(ctx);let screen=ctx.content_rect();
    let assets=classic_ui::current_assets(ctx);let painter=ctx.layer_painter(egui::LayerId::new(egui::Order::Middle,egui::Id::unique("sign-editor-preview")));
    let title=tr(language,"Edit sign message");
    classic_ui::classic_text(&painter,&assets,title,Pos2::new(screen.center().x-classic_ui::text_width(&assets,title,scale)/2.0,20.0*scale),scale,Color32::WHITE,true);
    let board=Rect::from_center_size(screen.center()-Vec2::new(0.0,25.0*scale),Vec2::new(128.0*scale,64.0*scale));
    classic_ui::region(&painter,&assets.textures[8],Rect::from_min_size(board.center_bottom()-Vec2::new(4.0*scale,0.0),Vec2::new(8.0*scale,30.0*scale)),[2.0,16.0,2.0,14.0]);
    classic_ui::region(&painter,&assets.textures[8],board,[2.0,2.0,24.0,12.0]);
    egui::Area::new(egui::Id::unique("sign-editor-lines")).fade_in(false).fixed_pos(board.min+Vec2::new(8.0*scale,8.0*scale)).show(ctx,|ui| {
        ui.set_width(112.0*scale);ui.spacing_mut().item_spacing.y=2.0*scale;
        for (i,line) in lines.iter_mut().enumerate() {
            let response=ui.add(egui::TextEdit::singleline(line).id(egui::Id::unique(("sign-line",i)))
                .char_limit(15).frame(egui::Frame::new()).margin(egui::Margin::same(0)).font(FontId::proportional(8.0*scale)).text_color(Color32::BLACK)
                .align(Align2::CENTER_CENTER).desired_width(112.0*scale));
            if state.sign_focus && i==0 {response.request_focus();state.sign_focus=false;}
            if response.lost_focus() && ui.input(|input|input.key_pressed(egui::Key::Enter)) {
                ui.memory_mut(|memory|memory.request_focus(egui::Id::unique(("sign-line",(i+1)%4))));
            }
        }
    });
    let mut done=ctx.input(|i|i.key_pressed(egui::Key::Escape));
    egui::Area::new(egui::Id::unique("sign-editor-done")).fade_in(false).fixed_pos(Pos2::new(screen.center().x-100.0*scale,screen.bottom()-28.0*scale)).show(ctx,|ui| {
        done|=classic_ui::button(ui,tr(language,"Done"),200.0*scale,20.0*scale).clicked();
    });
    if done {
        let lines=lines.clone();game.save_sign_text(position,lines);state.sign_editor=None;out.resume=true;
    }
}

fn death_menu(ctx: &egui::Context, game: &Game, language: Language, out: &mut UiOutput) {
    menu_window(tr(language, "You died!")).show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.label(crate::i18n::death_reason(language, &game.death_reason));
            ui.label(tr(language, "Your items remain where you fell. Return to collect them."));
            if big_button(ui, tr(language, "Respawn")) { out.respawn = true; }
            if big_button(ui, tr(language, "Save and Quit to Title")) { out.return_to_title = true; }
        });
    });
}

fn loading_screen(ctx:&egui::Context,state:&UiState,language:Language) {
    let scale=classic_ui::gui_scale(ctx);let screen=ctx.content_rect();let assets=classic_ui::current_assets(ctx);
    let painter=ctx.layer_painter(egui::LayerId::new(egui::Order::Middle,egui::Id::unique("loading-world")));
    let title=tr(language,"Generating level");let subtitle=tr(language,"Building terrain and preparing the slices…");
    for (text,y) in [(title,-30.0),(subtitle,-12.0)] {
        classic_ui::classic_text(&painter,&assets,text,screen.center()+Vec2::new(-classic_ui::text_width(&assets,text,scale)/2.0,y*scale),scale,Color32::WHITE,true);
    }
    let bar=Rect::from_min_size(screen.center()+Vec2::new(-50.0*scale,8.0*scale),Vec2::new(100.0*scale,2.0*scale));
    painter.rect_filled(bar,0.0,Color32::from_gray(80));
    painter.rect_filled(Rect::from_min_size(bar.min,Vec2::new(bar.width()*state.loading_progress.clamp(0.0,1.0),bar.height())),0.0,Color32::from_rgb(128,255,128));
}

fn title_menu(ctx: &egui::Context, state: &mut UiState, settings: &Settings, out: &mut UiOutput) {
    let language=settings.language;let screen=ctx.content_rect();let scale=classic_ui::gui_scale(ctx);
    let assets=classic_ui::current_assets(ctx);
    let painter=ctx.layer_painter(egui::LayerId::new(egui::Order::Middle,egui::Id::unique("title-logo")));
    let logo_scale=scale*5.0;
    let logo_width=classic_ui::text_width(&assets,"MC_4D",logo_scale);
    let logo=Pos2::new(screen.center().x-logo_width/2.0,30.0*scale);
    for depth in (1..=5).rev() {
        classic_ui::classic_text(&painter,&assets,"MC_4D",logo+Vec2::new(depth as f32*scale,depth as f32*scale),logo_scale,Color32::from_gray(35+depth*8),false);
    }
    classic_ui::classic_text(&painter,&assets,"MC_4D",logo,logo_scale,Color32::from_gray(210),true);
    classic_ui::classic_text(&painter,&assets,tr(language,"Now in four dimensions!"),Pos2::new(screen.center().x+20.0*scale,76.0*scale),scale,Color32::YELLOW,true);
    classic_ui::classic_text(&painter,&assets,"mc_4d",screen.min+Vec2::splat(2.0*scale),scale,Color32::from_gray(150),true);
    let top=screen.height()/4.0+48.0*scale;
    egui::Area::new(egui::Id::unique("title-buttons")).fade_in(false).fixed_pos(Pos2::new(screen.center().x-100.0*scale,top)).show(ctx,|ui| {
        ui.spacing_mut().item_spacing=Vec2::new(4.0*scale,4.0*scale);
        if classic_ui::button(ui,tr(language,"Singleplayer"),200.0*scale,20.0*scale).clicked(){state.screen=Screen::Worlds;out.refresh_worlds=true;}
        if classic_ui::button(ui,tr(language,"Texture Packs"),200.0*scale,20.0*scale).clicked(){state.menu_parent=Screen::Title;state.texture_pack_draft=settings.texture_pack.clone();state.screen=Screen::TexturePacks;}
        if classic_ui::button(ui,tr(language,"4D Tutorial"),200.0*scale,20.0*scale).clicked(){out.start_tutorial=true;}
        ui.add_space(12.0*scale);
        ui.horizontal(|ui| {
            if classic_ui::button(ui,tr(language,"Options…"),98.0*scale,20.0*scale).clicked(){state.menu_parent=Screen::Title;state.screen=Screen::Options;}
            if classic_ui::button(ui,tr(language,"Quit Game"),98.0*scale,20.0*scale).clicked(){out.quit=true;}
        });
    });
}

fn texture_pack_menu(ctx: &egui::Context, state: &mut UiState, settings: &mut Settings, out: &mut UiOutput) {
    let language=settings.language;
    menu_window(tr(language,"Texture Packs")).show(ctx,|ui| {
        ui.set_min_width(420.0);
        ui.heading(tr(language,"Classic Beta textures"));
        ui.label(tr(language,"The original pixel textures, with subtle fourth-axis shading."));
        if big_button(ui,tr(language,"Use Classic Pack")) { settings.texture_pack.clear(); state.texture_pack_draft.clear(); out.settings_changed=true; }
        // Beta lists every folder or .zip in the texturepacks directory.
        if let Some(root)=&state.texture_pack_root {
            ui.separator();
            ui.label(format!("{}: {}",tr(language,"Texture pack folder"),root.display()));
            let packs=mc4d_render::pack::list(root);
            if packs.is_empty() { ui.label(tr(language,"No texture packs found. Put folders or .zip files here.")); }
            for pack in packs {
                let name=pack.file_name().and_then(|n|n.to_str()).unwrap_or("?").to_owned();
                let selected=settings.texture_pack==pack.to_string_lossy();
                if big_button(ui,&if selected {format!("> {name} <")} else {name}) {
                    settings.texture_pack=pack.to_string_lossy().into_owned();state.texture_pack_draft=settings.texture_pack.clone();out.settings_changed=true;
                }
            }
        }
        ui.separator();
        ui.label(tr(language,"Texture pack folder"));
        ui.text_edit_singleline(&mut state.texture_pack_draft);
        ui.label(tr(language,"Folder or .zip: terrain.png, gui/items.png, mob/*.png"));
        if big_button(ui,tr(language,"Apply Texture Pack")) { settings.texture_pack=state.texture_pack_draft.trim().to_owned(); out.settings_changed=true; }
        if let Some(error)=&state.texture_error { ui.colored_label(Color32::LIGHT_RED,format!("{}: {error}",tr(language,"Texture pack error"))); }
        ui.add_space(12.0);
        if big_button(ui,tr(language,"Done")) { state.screen=state.menu_parent; }
    });
}

fn pause_menu(ctx: &egui::Context, ui_state: &mut UiState, game: &mut Game, settings: &mut Settings, out: &mut UiOutput) {
    let language = settings.language;
    menu_window(tr(language, "Game Menu")).show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            if big_button(ui, tr(language, "Back to Game")) {
                out.resume = true;
            }
            ui.horizontal(|ui| {
                let s=classic_ui::gui_scale(ctx);
                if classic_ui::button(ui,if language==Language::Chinese {"成就"}else{"Achievements"},98.0*s,20.0*s).clicked(){ui_state.screen=Screen::Achievements;}
                if classic_ui::button(ui,if language==Language::Chinese {"统计信息"}else{"Statistics"},98.0*s,20.0*s).clicked(){ui_state.screen=Screen::Statistics;}
            });
            if big_button(ui, tr(language, "Options…")) {
                ui_state.screen = Screen::Options;
            }
            if big_button(ui, tr(language, "Controls…")) {
                ui_state.screen = Screen::Controls;
            }
            if ui_state.tutorial.is_some() {
                if big_button(ui, tr(language, "Stop Tutorial")) {
                    ui_state.tutorial = None;
                    settings.tutorial_done = true;
                    out.settings_changed = true;
                }
            } else if big_button(ui, tr(language, "Tutorial")) {
                ui_state.tutorial = Some(Tutorial::new(game, settings));
                out.resume = true;
            }
            let other = match game.mode {
                GameMode::Survival => GameMode::Creative,
                GameMode::Creative => GameMode::Survival,
            };
            if big_button(ui, &format!("{}: {}", if language == Language::Chinese { "游戏模式" } else { "Game Mode" }, tr(language, if game.mode == GameMode::Creative { "Creative" } else { "Survival" }))) {
                game.set_mode(other);
            }
            if big_button(ui, tr(language, "Save Game")) {
                game.save_all();
                game.toast("Game saved");
            }
            if big_button(ui, tr(language, "Save and Quit to Title")) {
                out.return_to_title = true;
            }
        });
    });
}

fn controls_menu(ctx:&egui::Context,state:&mut UiState,s:&mut Settings,bindings:&mut Bindings,input:&InputState,out:&mut UiOutput) {
    let scale=classic_ui::gui_scale(ctx);let language=s.language;let screen=ctx.content_rect();
    let assets=classic_ui::current_assets(ctx);
    let painter=ctx.layer_painter(egui::LayerId::new(egui::Order::Middle,egui::Id::unique("controls-title")));
    let title=tr(language,"Controls");
    classic_ui::classic_text(&painter,&assets,title,Pos2::new(screen.center().x-classic_ui::text_width(&assets,title,scale)/2.0,20.0*scale),scale,Color32::WHITE,true);
    egui::Area::new(egui::Id::unique("controls-tabs")).fade_in(false).anchor(Align2::CENTER_TOP,Vec2::new(0.0,40.0*scale)).show(ctx,|ui| {
        ui.spacing_mut().item_spacing=Vec2::new(2.0*scale,4.0*scale);
        ui.horizontal(|ui| {
            for (index,category) in Category::ALL.into_iter().enumerate() {
                let label=if category==Category::FourD {"4D"}else{tr(language,category.name())};
                if classic_ui::button(ui,label,60.0*scale,20.0*scale).clicked(){state.controls_category=index;}
            }
            if classic_ui::button(ui,tr(language,"Reset"),60.0*scale,20.0*scale).clicked(){*bindings=Bindings::default();out.bindings_changed=true;}
        });
    });
    egui::Area::new(egui::Id::unique("classic-controls")).fade_in(false).anchor(Align2::CENTER_TOP,Vec2::new(0.0,68.0*scale)).show(ctx,|ui| {
        ui.set_width(310.0*scale);
        egui::ScrollArea::vertical().max_height((screen.height()-108.0*scale).max(24.0*scale)).show(ui,|ui| {
            let category=Category::ALL[state.controls_category.min(3)];
            egui::Grid::new("classic-bindings").num_columns(2).spacing([10.0*scale,4.0*scale]).show(ui,|ui| {
                let mut index=0;
                for &action in Action::ALL.iter().filter(|a|a.category()==category) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing=Vec2::new(5.0*scale,0.0);
                        let current=bindings.get(action);
                        let capturing=state.capturing.is_some_and(|(a,_)|a==action);
                        let label=if capturing {"> ? <".into()}else{
                            let first=tr(language,&current[0].label(&input.layout_names)).to_owned();
                            if current[1].is_bound(){format!("{} / {}",first,tr(language,&current[1].label(&input.layout_names)))}else{first}
                        };
                        let button=classic_ui::button(ui,&label,70.0*scale,20.0*scale);
                        let alternate=ui.input(|i|i.modifiers.shift) as usize;
                        if button.clicked(){
                            if ui.input(|i|i.modifiers.ctrl){bindings.reset(action);out.bindings_changed=true;}
                            else {state.capturing=Some((action,alternate));}
                        }
                        if button.secondary_clicked(){bindings.set(action,alternate,crate::input::Binding::Unbound);out.bindings_changed=true;}
                        let conflict=!bindings.conflicts(action).is_empty();
                        if conflict {ui.painter().rect_stroke(button.rect,0.0,Stroke::new(2.0,Color32::RED),egui::StrokeKind::Inside);}
                        button.on_hover_text(tr(language,action.name()));
                        let (rect,response)=ui.allocate_exact_size(Vec2::new(75.0*scale,20.0*scale),egui::Sense::hover());
                        let name=tr(language,action.name());
                        let width=classic_ui::text_width(&assets,name,scale);
                        let label_scale=scale*(rect.width()/width.max(1.0)).min(1.0).max(0.65);
                        classic_ui::classic_text(ui.painter(),&assets,name,rect.left_center()-Vec2::new(0.0,4.0*label_scale),label_scale,
                            if conflict{Color32::LIGHT_RED}else{Color32::WHITE},true);
                        response.on_hover_text(name);
                    });
                    index+=1;if index%2==0 {ui.end_row();}
                }
            });
        });
    });
    let hint=if state.capturing.is_some(){tr(language,"Press a key or mouse button; Esc cancels.")}else{tr(language,"Click: bind | Shift: alternate | Right-click: clear | Ctrl: reset")};
    let small=0.75*scale;
    classic_ui::classic_text(&painter,&assets,hint,Pos2::new(screen.center().x-classic_ui::text_width(&assets,hint,small)/2.0,screen.bottom()-42.0*scale),small,Color32::from_gray(210),true);
    egui::Area::new(egui::Id::unique("controls-done")).fade_in(false).fixed_pos(Pos2::new(screen.center().x-100.0*scale,screen.bottom()-28.0*scale)).show(ctx,|ui| {
        if classic_ui::button(ui,tr(language,"Done"),200.0*scale,20.0*scale).clicked(){state.capturing=None;state.screen=Screen::Options;}
    });
}

#[cfg(test)]mod compass_tests {
    use super::*;
    #[test]fn side_compass_projects_actual_side_axes_and_normal(){
        let basis=mc4d_core::camera::Orientation::default().basis();let side=mc4d_core::views::slice_bases(basis,2)[1];
        let (main_x,_)=compass_projection(basis,Vec4::X);let(side_x,normal_x)=compass_projection(side,Vec4::X);let(side_w,normal_w)=compass_projection(side,Vec4::W);
        assert_eq!(main_x,Vec2::X);assert_eq!(side_x,Vec2::ZERO);assert_eq!(normal_x,-1.0);assert_eq!(side_w,Vec2::X);assert_eq!(normal_w,0.0);
    }
    #[test]fn compass_stays_inside_actual_inset_at_gui_and_native_dpi_scales(){
        for size in [[1280.0,800.0],[1920.0,1080.0],[2560.0,1440.0]] {for ppp in [1.0,1.25,2.0] {
            let logical=Vec2::new(size[0]/ppp,size[1]/ppp);let ui_rects=slice_rects(logical,2);
            // FrameRenderer clamps in logical points, then scales each rectangle.
            let physical=mc4d_core::views::view_rects([logical.x,logical.y],2)[1].map(|v|v*ppp);
            let actual=Rect::from_min_size(Pos2::new(physical[0]/ppp,physical[1]/ppp),Vec2::new(physical[2]/ppp,physical[3]/ppp));
            assert!((actual.min-ui_rects[1].min).length()<0.001);assert!((actual.max-ui_rects[1].max).length()<0.001);
            for gui in [1.0,2.0,3.0,4.0]{let layout=compass_layout(actual,gui,true).expect("side compass fits");assert!(actual.contains_rect(layout.bounds));assert!(layout.bounds.top()>=actual.top()+21.0);}
        }}
    }
    #[test]fn every_view_of_every_grid_layout_fits_its_own_compass(){
        for size in [[1280.0,800.0],[1920.0,1080.0],[2560.0,1440.0]] {for ppp in [1.0,1.25,2.0] {for views in [2,4,6,9] {
            let logical=Vec2::new(size[0]/ppp,size[1]/ppp);
            for (i,rect) in slice_rects(logical,views).iter().enumerate() {for gui in [1.0,2.0,3.0,4.0] {
                let layout=compass_layout(*rect,gui,i>0).unwrap_or_else(||panic!("{views} views at {size:?}/{ppp}: view {i} lacks a compass"));
                assert!(rect.contains_rect(layout.bounds));
            }}
        }}}
    }
}

#[cfg(test)]mod side_compass_gpu_tests {
    use super::*;
    use crate::frame::{FrameInput,FrameRenderer};
    #[test]
    #[ignore="requires GPU; verifies side compass visibility and captures normal/high-DPI HUD layouts"]
    fn side_compass_uses_side_frame_and_fits_high_dpi_inset(){
        let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");std::fs::create_dir_all(&root).unwrap();
        for (size,ppp,gui) in [((1280,800),1.0_f32,0),((1920,1080),2.0,1)] {
            let gpu=crate::headless_gpu(size).unwrap();let mut frame=FrameRenderer::new(&gpu.device,gpu.target.format(),size,gpu.adapter_name);
            let mut game=Game::menu_preview(7);let mut settings=Settings::default();settings.gui_scale=gui;settings.show_hints=false;
            let mut bindings=Bindings::default();let input=InputState::default();let mut state=UiState{screen:Screen::Playing,show_hud:true,show_debug:false,..Default::default()};
            let mut render=|game:&mut Game,state:&mut UiState|{
                let mut raw=crate::raw_input(size);raw.screen_rect=Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(size.0 as f32/ppp,size.1 as f32/ppp)));
                raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point=Some(ppp);
                frame.render(&gpu.device,&gpu.queue,&gpu.target,FrameInput{game,settings:&mut settings,bindings:&mut bindings,input:&input,ui:state,raw_input:raw,size,pixels_per_point:ppp});
                gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();crate::frame::read_texture(&gpu.device,&gpu.queue,&gpu.target)
            };
            render(&mut game,&mut state);let hidden=render(&mut game,&mut state);state.show_debug=true;let shown=render(&mut game,&mut state);
            let logical=Vec2::new(size.0 as f32/ppp,size.1 as f32/ppp);let inset=slice_rects(logical,2)[1];
            let scale=if gui==0{((logical.x/320.0).min(logical.y/240.0).floor()).clamp(1.0,4.0)}else{gui as f32};let bounds=compass_layout(inset,scale,true).unwrap().bounds;
            let mut changed=0;for y in (bounds.top()*ppp).ceil()as usize..(bounds.bottom()*ppp).floor()as usize{for x in (bounds.left()*ppp).ceil()as usize..(bounds.right()*ppp).floor()as usize{let i=(y*size.0 as usize+x)*4;if hidden[i..i+3]!=shown[i..i+3]{changed+=1;}}}
            assert!(changed>100,"side compass must be visibly drawn only with F3: {changed}");
            crate::frame::write_png(&root.join(format!("side-compass-dpi{ppp}.png")),size.0,size.1,&shown).unwrap();
            game.switch_to_side_view();let switched=render(&mut game,&mut state);assert_ne!(shown,switched);
            crate::frame::write_png(&root.join(format!("side-compass-switched-dpi{ppp}.png")),size.0,size.1,&switched).unwrap();
        }
    }
}
