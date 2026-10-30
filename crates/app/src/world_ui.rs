//! Original-style separate world selection, creation, rename and confirmation screens.
use super::{Screen, UiOutput, UiState, WorldRequest, classic_ui as c};
use crate::{game::GameMode, i18n::Language};
use egui::{Color32, Id, Pos2, Rect, Vec2};
fn t(language: Language, en: &str, zh: &str) -> String {
    if language == Language::Chinese {
        zh
    } else {
        en
    }
    .into()
}
fn label(ui: &egui::Ui, text: &str, pos: Pos2, scale: f32, color: Color32) {
    let a = c::current_assets(ui.ctx());
    c::classic_text(ui.painter(), &a, text, pos, scale, color, true);
}
fn button(
    ui: &mut egui::Ui,
    id: &str,
    text: &str,
    pos: Pos2,
    width: f32,
    s: f32,
    enabled: bool,
) -> bool {
    let a = c::current_assets(ui.ctx());
    ui.add_enabled_ui(enabled, |ui| {
        c::button_at(
            ui,
            &a,
            Rect::from_min_size(pos, Vec2::new(width * s, 20.0 * s)),
            Id::unique(id),
            text,
            s,
        )
    })
    .inner
    .clicked()
}
fn field(ui: &mut egui::Ui, id: &str, value: &mut String, pos: Pos2, s: f32) {
    let rect = Rect::from_min_size(pos, Vec2::new(200.0 * s, 20.0 * s));
    ui.painter()
        .rect_filled(rect.expand(s), 0.0, Color32::from_gray(160));
    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
    ui.put(
        rect.shrink(4.0 * s),
        egui::TextEdit::singleline(value)
            .id(Id::unique(id))
            .char_limit(64)
            .font(egui::FontId::proportional(8.0 * s))
            .text_color(Color32::WHITE)
            .frame(egui::Frame::new()),
    );
}
pub fn draw(ctx: &egui::Context, state: &mut UiState, language: Language, out: &mut UiOutput) {
    let s = c::gui_scale(ctx);
    let screen = ctx.content_rect();
    let center = screen.center().x;
    let top = screen.top();
    let bottom = screen.bottom();
    let title = match state.screen {
        Screen::CreateWorld => t(language, "Create New World", "创建新的世界"),
        Screen::RenameWorld => t(language, "Rename World", "重命名世界"),
        Screen::DeleteWorld => t(language, "Delete World", "删除世界"),
        _ => t(language, "Select World", "选择世界"),
    };
    egui::Area::new(Id::unique("world-management"))
        .fade_in(false)
        .movable(false)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            let a = c::current_assets(ctx);
            label(
                ui,
                &title,
                Pos2::new(center - c::text_width(&a, &title, s) / 2.0, top + 20.0 * s),
                s,
                Color32::WHITE,
            );
            match state.screen {
                Screen::Worlds => {
                    let list = Rect::from_min_max(
                        Pos2::new(center - 150.0 * s, top + 34.0 * s),
                        Pos2::new(center + 150.0 * s, bottom - 64.0 * s),
                    );
                    ui.painter().rect_filled(
                        Rect::from_min_max(
                            Pos2::new(screen.left(), list.top()),
                            Pos2::new(screen.right(), list.bottom()),
                        ),
                        0.0,
                        Color32::from_black_alpha(150),
                    );
                    ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(list.height())
                            .show(ui, |ui| {
                                for entry in &state.worlds {
                                    let (rect, response) = ui.allocate_exact_size(
                                        Vec2::new(298.0 * s, 36.0 * s),
                                        egui::Sense::click(),
                                    );
                                    if entry.folder == state.world_name {
                                        ui.painter().rect_filled(
                                            rect,
                                            0.0,
                                            Color32::from_gray(128),
                                        );
                                        ui.painter().rect_filled(
                                            rect.shrink(s),
                                            0.0,
                                            Color32::BLACK,
                                        );
                                    }
                                    label(
                                        ui,
                                        &entry.name,
                                        rect.min + Vec2::new(3.0 * s, 3.0 * s),
                                        s,
                                        Color32::WHITE,
                                    );
                                    label(
                                        ui,
                                        &entry.folder,
                                        rect.min + Vec2::new(3.0 * s, 15.0 * s),
                                        s,
                                        Color32::from_gray(128),
                                    );
                                    if entry.valid {
                                        label(ui, &crate::worlds::details(entry), rect.min + Vec2::new(3.0*s,25.0*s), s, Color32::from_gray(128));
                                    }
                                    if !entry.valid {
                                        label(
                                            ui,
                                            &t(
                                                language,
                                                "Unreadable world metadata",
                                                "世界信息无法读取",
                                            ),
                                            rect.min + Vec2::new(3.0 * s, 25.0 * s),
                                            s,
                                            Color32::LIGHT_RED,
                                        );
                                    }
                                    if response.clicked() {
                                        state.world_name = entry.folder.clone();
                                    }
                                    if response.double_clicked() && entry.valid {
                                        out.load_world = Some(WorldRequest {
                                            name: entry.folder.clone(),
                                            seed: 1337,
                                            mode: GameMode::Survival,
                                            create: false,
                                        });
                                    }
                                }
                            });
                    });
                    let selected = state
                        .worlds
                        .iter()
                        .find(|e| e.folder == state.world_name)
                        .cloned();
                    if button(
                        ui,
                        "world-play",
                        &t(language, "Play Selected World", "游玩选中的世界"),
                        Pos2::new(center - 154.0 * s, bottom - 52.0 * s),
                        150.0,
                        s,
                        selected.as_ref().is_some_and(|e| e.valid),
                    ) {
                        out.load_world = Some(WorldRequest {
                            name: state.world_name.clone(),
                            seed: 1337,
                            mode: GameMode::Survival,
                            create: false,
                        });
                    }
                    if button(
                        ui,
                        "world-create",
                        &t(language, "Create New World", "创建新的世界"),
                        Pos2::new(center + 4.0 * s, bottom - 52.0 * s),
                        150.0,
                        s,
                        true,
                    ) {
                        state.world_name = t(language, "New World", "新的世界");
                        state.world_seed.clear();
                        state.world_error = None;
                        state.screen = Screen::CreateWorld;
                    }
                    if button(
                        ui,
                        "world-rename",
                        &t(language, "Rename", "重命名"),
                        Pos2::new(center - 154.0 * s, bottom - 28.0 * s),
                        70.0,
                        s,
                        selected.is_some(),
                    ) {
                        state.world_rename = selected.as_ref().unwrap().name.clone();
                        state.screen = Screen::RenameWorld;
                    }
                    if button(
                        ui,
                        "world-delete",
                        &t(language, "Delete", "删除"),
                        Pos2::new(center - 74.0 * s, bottom - 28.0 * s),
                        70.0,
                        s,
                        selected.is_some(),
                    ) {
                        state.screen = Screen::DeleteWorld;
                    }
                    if button(
                        ui,
                        "world-cancel",
                        &t(language, "Cancel", "取消"),
                        Pos2::new(center + 4.0 * s, bottom - 28.0 * s),
                        150.0,
                        s,
                        true,
                    ) {
                        state.screen = Screen::Title;
                    }
                }
                Screen::CreateWorld => {
                    label(
                        ui,
                        &t(language, "World Name", "世界名称"),
                        Pos2::new(center - 100.0 * s, top + 46.0 * s),
                        s,
                        Color32::from_gray(160),
                    );
                    field(
                        ui,
                        "new-world-name",
                        &mut state.world_name,
                        Pos2::new(center - 100.0 * s, top + 60.0 * s),
                        s,
                    );
                    label(
                        ui,
                        &t(language, "Seed for the World Generator", "世界生成器种子"),
                        Pos2::new(center - 100.0 * s, top + 96.0 * s),
                        s,
                        Color32::from_gray(160),
                    );
                    field(
                        ui,
                        "new-world-seed",
                        &mut state.world_seed,
                        Pos2::new(center - 100.0 * s, top + 110.0 * s),
                        s,
                    );
                    label(
                        ui,
                        &t(
                            language,
                            "Leave blank for a random seed",
                            "留空以使用随机种子",
                        ),
                        Pos2::new(center - 100.0 * s, top + 136.0 * s),
                        s,
                        Color32::from_gray(160),
                    );
                    let mode = if state.world_mode == GameMode::Creative {
                        t(language, "Game Mode: Creative", "游戏模式：创造")
                    } else {
                        t(language, "Game Mode: Survival", "游戏模式：生存")
                    };
                    if button(
                        ui,
                        "new-world-mode",
                        &mode,
                        Pos2::new(center - 100.0 * s, top + 151.0 * s),
                        200.0,
                        s,
                        true,
                    ) {
                        state.world_mode = if state.world_mode == GameMode::Creative {
                            GameMode::Survival
                        } else {
                            GameMode::Creative
                        };
                    }
                    if button(
                        ui,
                        "new-world-confirm",
                        &title,
                        Pos2::new(center - 100.0 * s, bottom - 52.0 * s),
                        200.0,
                        s,
                        crate::settings::valid_world_name(state.world_name.trim()),
                    ) {
                        let base = state.world_name.trim();
                        let mut name = base.to_owned();
                        let mut suffix = 1;
                        while state.worlds.iter().any(|e| e.folder == name) {
                            name = format!("{base}-{suffix}");
                            suffix += 1;
                        }
                        out.load_world = Some(WorldRequest {
                            name,
                            seed: crate::worlds::seed(&state.world_seed),
                            mode: state.world_mode,
                            create: true,
                        });
                    }
                    if button(
                        ui,
                        "new-world-cancel",
                        &t(language, "Cancel", "取消"),
                        Pos2::new(center - 100.0 * s, bottom - 28.0 * s),
                        200.0,
                        s,
                        true,
                    ) {
                        state.screen = Screen::Worlds;
                    }
                }
                Screen::RenameWorld => {
                    label(
                        ui,
                        &t(language, "Enter a new name:", "输入新名称："),
                        Pos2::new(center - 100.0 * s, top + 70.0 * s),
                        s,
                        Color32::from_gray(160),
                    );
                    field(
                        ui,
                        "rename-world-name",
                        &mut state.world_rename,
                        Pos2::new(center - 100.0 * s, top + 86.0 * s),
                        s,
                    );
                    if button(
                        ui,
                        "rename-world-confirm",
                        &t(language, "Rename", "重命名"),
                        Pos2::new(center - 100.0 * s, bottom - 52.0 * s),
                        200.0,
                        s,
                        !state.world_rename.trim().is_empty(),
                    ) {
                        out.rename_world =
                            Some((state.world_name.clone(), state.world_rename.clone()));
                    }
                    if button(
                        ui,
                        "rename-world-cancel",
                        &t(language, "Cancel", "取消"),
                        Pos2::new(center - 100.0 * s, bottom - 28.0 * s),
                        200.0,
                        s,
                        true,
                    ) {
                        state.screen = Screen::Worlds;
                    }
                }
                Screen::DeleteWorld => {
                    let question = t(
                        language,
                        "Are you sure you want to delete this world?",
                        "确定要删除这个世界吗？",
                    );
                    label(
                        ui,
                        &question,
                        Pos2::new(
                            center - c::text_width(&a, &question, s) / 2.0,
                            top + 75.0 * s,
                        ),
                        s,
                        Color32::WHITE,
                    );
                    let name = state
                        .worlds
                        .iter()
                        .find(|e| e.folder == state.world_name)
                        .map_or(state.world_name.as_str(), |e| e.name.as_str());
                    label(
                        ui,
                        name,
                        Pos2::new(center - c::text_width(&a, name, s) / 2.0, top + 95.0 * s),
                        s,
                        Color32::from_gray(160),
                    );
                    let warning = t(
                        language,
                        "It will be lost forever! (A long time!)",
                        "世界将永远消失！（真的很久！）",
                    );
                    label(
                        ui,
                        &warning,
                        Pos2::new(
                            center - c::text_width(&a, &warning, s) / 2.0,
                            top + 115.0 * s,
                        ),
                        s,
                        Color32::from_gray(160),
                    );
                    if button(
                        ui,
                        "delete-world-confirm",
                        &t(language, "Delete", "删除"),
                        Pos2::new(center - 154.0 * s, top + 145.0 * s),
                        150.0,
                        s,
                        true,
                    ) {
                        out.delete_world = Some(state.world_name.clone());
                    }
                    if button(
                        ui,
                        "delete-world-cancel",
                        &t(language, "Cancel", "取消"),
                        Pos2::new(center + 4.0 * s, top + 145.0 * s),
                        150.0,
                        s,
                        true,
                    ) {
                        state.screen = Screen::Worlds;
                    }
                }
                _ => {}
            }
            if let Some(error) = &state.world_error {
                let error = crate::i18n::text(language, error);
                label(
                    ui,
                    error,
                    Pos2::new(
                        center - c::text_width(&a, error, s) / 2.0,
                        bottom - 64.0 * s,
                    ),
                    s,
                    Color32::LIGHT_RED,
                );
            }
        });
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::{
        frame::{FrameInput, FrameRenderer},
        game::Game,
        input::{Bindings, InputState},
        settings::Settings,
    };
    #[test]
    #[ignore = "requires GPU; captures separate world screens and checks creation/confirmation clicks"]
    fn classic_world_screens_and_actions() {
        let size = (854, 480);
        let gpu = crate::headless_gpu(size).unwrap();
        let mut frame =
            FrameRenderer::new(&gpu.device, gpu.target.format(), size, gpu.adapter_name);
        let mut game = Game::menu_preview(7);
        let mut settings = Settings::default();
        let mut bindings = Bindings::default();
        let input = InputState::default();
        let mut state = UiState {
            screen: Screen::Worlds,
            show_hud: false,
            world_name: "world".into(),
            ..Default::default()
        };
        state.worlds = vec![crate::worlds::Entry {
            folder: "world".into(),
            name: "First World".into(),
            modified: 0,
            size_bytes: 0,
            valid: true,
        }];
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/render-review");
        std::fs::create_dir_all(&root).unwrap();
        let mut render =
            |state: &mut UiState, settings: &mut Settings, events: Vec<egui::Event>| {
                let mut raw_input = crate::raw_input(size);
                raw_input.events = events;
                let result = frame
                    .render(
                        &gpu.device,
                        &gpu.queue,
                        &gpu.target,
                        FrameInput {
                            game: &mut game,
                            settings,
                            bindings: &mut bindings,
                            input: &input,
                            ui: state,
                            raw_input,
                            size,
                            pixels_per_point: 1.0,
                        },
                    )
                    .0;
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .unwrap();
                result
            };
        for language in [Language::English, Language::Chinese] {
            settings.language = language;
            for (screen, name) in [
                (Screen::Worlds, "world-selection"),
                (Screen::CreateWorld, "create-world"),
                (Screen::RenameWorld, "rename-world"),
                (Screen::DeleteWorld, "delete-world"),
            ] {
                state.screen = screen;
                state.world_rename = "First World".into();
                render(&mut state, &mut settings, vec![]);
                render(&mut state, &mut settings, vec![]);
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
        state.screen = Screen::CreateWorld;
        state.world_name = "New Test".into();
        state.world_seed = "123".into();
        render(&mut state, &mut settings, vec![]);
        let click = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]
        };
        let pos = Pos2::new(427.0, 396.0);
        render(&mut state, &mut settings, click(pos, true));
        let output = render(&mut state, &mut settings, click(pos, false));
        let request = output
            .load_world
            .expect("create click generates explicit creation request");
        assert!(request.create);
        assert_eq!(request.seed, 123);
        assert_eq!(request.name, "New Test");
        state.screen = Screen::DeleteWorld;
        state.world_name = "world".into();
        render(&mut state, &mut settings, vec![]);
        let cancel = Pos2::new(585.0, 310.0);
        render(&mut state, &mut settings, click(cancel, true));
        let output = render(&mut state, &mut settings, click(cancel, false));
        assert!(output.delete_world.is_none());
        assert_eq!(
            state.screen,
            Screen::Worlds,
            "cancel never deletes the selected save"
        );
    }
}
