//! Classic two-column option pages, using the original GUI sprites and pixel font.
use crate::{settings::Settings, ui::{Screen, UiOutput, UiState, classic_ui}, i18n::{Language, text as tr}};
use egui::{Align2, Color32, Pos2, Vec2};

fn header(ctx:&egui::Context,title:&str,scale:f32) {
    let painter=ctx.layer_painter(egui::LayerId::new(egui::Order::Middle,egui::Id::unique("options-header")));
    let assets=classic_ui::current_assets(ctx);
    let width=classic_ui::text_width(&assets,title,scale);
    classic_ui::classic_text(&painter,&assets,title,Pos2::new(ctx.content_rect().center().x-width/2.0,20.0*scale),scale,Color32::WHITE,true);
}
fn navigation(ctx:&egui::Context,id:&str,label:&str,y:f32,scale:f32)->bool {
    let mut clicked=false;
    egui::Area::new(egui::Id::unique(id)).fade_in(false).fixed_pos(Pos2::new(ctx.content_rect().center().x-100.0*scale,y))
        .show(ctx,|ui| {clicked=classic_ui::button(ui,label,200.0*scale,20.0*scale).clicked();});
    clicked
}
fn toggle(ui:&mut egui::Ui,name:&str,value:&mut bool,language:Language,scale:f32) {
    let text=format!("{}: {}",tr(language,name),tr(language,if *value {"ON"}else{"OFF"}));
    if classic_ui::button(ui,&text,150.0*scale,20.0*scale).clicked(){*value=!*value;}
}
pub fn draw(ctx:&egui::Context,state:&mut UiState,s:&mut Settings,out:&mut UiOutput) {
    let before=s.clone(); let scale=classic_ui::gui_scale(ctx); let language=s.language;
    let title=match state.screen {Screen::VideoOptions=>"Video Settings",Screen::FourDOptions=>"4D Settings",_=>"Options"};
    header(ctx,tr(language,title),scale);
    egui::Area::new(egui::Id::unique("classic-options-grid")).fade_in(false).anchor(Align2::CENTER_TOP,Vec2::new(0.0,40.0*scale))
        .show(ctx,|ui| {
            ui.set_width(310.0*scale);
            egui::Grid::new("classic-options-fields").num_columns(2).spacing([10.0*scale,4.0*scale]).show(ui,|ui| {
                match state.screen {
                    Screen::Options=>{
                        let label=format!("{}: {:.0}%",tr(language,"Music"),s.music_volume*100.0);
                        classic_ui::slider(ui,&label,150.0*scale,20.0*scale,&mut s.music_volume,0.0..=1.0);
                        let label=format!("{}: {:.0}%",tr(language,"Sound"),s.sound_volume*100.0);
                        classic_ui::slider(ui,&label,150.0*scale,20.0*scale,&mut s.sound_volume,0.0..=1.0);ui.end_row();
                        toggle(ui,"Invert Mouse",&mut s.invert_y,language,scale);
                        let label=format!("{}: {:.0}%",tr(language,"Sensitivity"),s.mouse_sensitivity*100.0);
                        classic_ui::slider(ui,&label,150.0*scale,20.0*scale,&mut s.mouse_sensitivity,0.05..=2.0);ui.end_row();
                        let label=format!("{}: {}",tr(language,"Difficulty"),tr(language,s.difficulty.name()));
                        if classic_ui::button(ui,&label,150.0*scale,20.0*scale).clicked(){s.difficulty=s.difficulty.next();}
                        let label=if language==Language::English {"Language: English"}else{"语言：简体中文"};
                        if classic_ui::button(ui,label,150.0*scale,20.0*scale).clicked(){s.language=if language==Language::English{Language::Chinese}else{Language::English};}ui.end_row();
                        if classic_ui::button(ui,tr(language,"Texture Packs"),150.0*scale,20.0*scale).clicked(){state.texture_pack_draft=s.texture_pack.clone();state.screen=Screen::TexturePacks;}
                        if classic_ui::button(ui,tr(language,"4D Settings…"),150.0*scale,20.0*scale).clicked(){state.screen=Screen::FourDOptions;}
                    }
                    Screen::VideoOptions=>{
                        let label=format!("{}: {}",tr(language,"Graphics"),tr(language,if s.fancy_graphics {"Fancy"}else{"Fast"}));
                        if classic_ui::button(ui,&label,150.0*scale,20.0*scale).clicked(){s.fancy_graphics=!s.fancy_graphics;}
                        let label=format!("{}: {}",tr(language,"Render Distance"),tr(language,match s.render_distance {0..=2=>"Tiny",3=>"Short",4=>"Normal",_=>"Far"}));
                        if classic_ui::button(ui,&label,150.0*scale,20.0*scale).clicked(){s.render_distance=match s.render_distance{0..=2=>3,3=>4,4=>6,_=>2};}ui.end_row();
                        toggle(ui,"Limit Framerate",&mut s.limit_framerate,language,scale);toggle(ui,"3D Anaglyph",&mut s.anaglyph,language,scale);ui.end_row();
                        toggle(ui,"View Bobbing",&mut s.view_bobbing,language,scale);toggle(ui,"Smooth Lighting",&mut s.smooth_lighting,language,scale);ui.end_row();
                        let label=format!("{}: {}",tr(language,"GUI Scale"),tr(language,match s.gui_scale{1=>"Small",2=>"Normal",3=>"Large",4=>"Largest",_=>"Auto"}));
                        if classic_ui::button(ui,&label,150.0*scale,20.0*scale).clicked(){s.gui_scale=(s.gui_scale+1)%5;}
                        toggle(ui,"Chunk Culling",&mut s.chunk_culling,language,scale);ui.end_row();
                        let label=format!("{}: {:.0}°",tr(language,"Field of view"),s.fov);
                        classic_ui::slider(ui,&label,150.0*scale,20.0*scale,&mut s.fov,50.0..=110.0);
                        let label=format!("{}: {:.0}%",tr(language,"Render scale"),s.render_scale*100.0);
                        classic_ui::slider(ui,&label,150.0*scale,20.0*scale,&mut s.render_scale,0.25..=1.0);ui.end_row();
                        toggle(ui,"Fullscreen",&mut s.fullscreen,language,scale);ui.end_row();
                    }
                    Screen::FourDOptions=>{
                        toggle(ui,"Auxiliary View",&mut s.inset,language,scale);
                        let label=format!("{}: {}",tr(language,"Slice layout"),if s.view_count==2{tr(language,"Main + Auxiliary").to_string()}else{format!("{}",s.view_count)});
                        if classic_ui::button(ui,&label,150.0*scale,20.0*scale).clicked(){s.view_count=match s.view_count{2=>4,4=>6,6=>9,_=>2};}ui.end_row();
                        toggle(ui,"Ghost Layers",&mut s.ghosts,language,scale);toggle(ui,"4D Glasses",&mut s.glasses,language,scale);ui.end_row();
                        toggle(ui,"Auto Snap",&mut s.auto_snap,language,scale);toggle(ui,"Cross-section edges",&mut s.edges,language,scale);ui.end_row();
                        toggle(ui,"Show control hints",&mut s.show_hints,language,scale);toggle(ui,"Sun shadows",&mut s.shadows,language,scale);ui.end_row();
                        let label=format!("{}: {:.1}",tr(language,"Ghost layer distance"),s.ghost_offset);
                        classic_ui::slider(ui,&label,150.0*scale,20.0*scale,&mut s.ghost_offset,0.5..=4.0);
                        let label=format!("{}: {:.0}°",tr(language,"Wheel rotation"),s.wheel_step_deg);
                        classic_ui::slider(ui,&label,150.0*scale,20.0*scale,&mut s.wheel_step_deg,1.0..=30.0);ui.end_row();
                        let label=format!("{}: {:.0}°/s",tr(language,"Key rotation"),s.key_rotate_speed);
                        classic_ui::slider(ui,&label,150.0*scale,20.0*scale,&mut s.key_rotate_speed,10.0..=180.0);
                        let label=format!("{}: {:.1}",tr(language,"4D mouse speed"),s.slice_mouse_sensitivity);
                        classic_ui::slider(ui,&label,150.0*scale,20.0*scale,&mut s.slice_mouse_sensitivity,0.1..=4.0);
                    }
                    _=>{}
                }
            });
        });
    if state.screen==Screen::Options {
        if navigation(ctx,"options-video",tr(language,"Video Settings…"),ctx.content_rect().bottom()-92.0*scale,scale){state.screen=Screen::VideoOptions;}
        if navigation(ctx,"options-controls",tr(language,"Controls…"),ctx.content_rect().bottom()-68.0*scale,scale){state.screen=Screen::Controls;}
    }
    if navigation(ctx,"options-done",tr(language,"Done"),ctx.content_rect().bottom()-32.0*scale,scale) {
        state.screen=if state.screen==Screen::Options{state.menu_parent}else{Screen::Options};
    }
    if *s!=before {out.settings_changed=true;}
}
