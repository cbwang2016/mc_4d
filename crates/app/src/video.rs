//! Scripted introductory video: a camera tour with captions, rendered headlessly and piped
//! to ffmpeg (`--video out.mp4`), or one still per shot for previewing (`--video-stills DIR`).

use crate::app::anyhow_lite::Result;
use crate::frame::{FrameInput, FrameRenderer, read_texture, write_png};
use crate::game::{Game, GameMode};
use crate::i18n::Language;
use crate::input::{Bindings, InputState};
use crate::settings::Settings;
use crate::tutorial::Tutorial;
use crate::ui::{Screen, UiState};
use crate::{HeadlessGpu, headless_gpu};
use mc4d_core::block::{AIR, BRICK, GLOW};
use mc4d_core::camera::Orientation;
use mc4d_core::chunk::chunk_of;
use mc4d_core::glam::{IVec4, Vec4};
use mc4d_core::physics::EYE_HEIGHT;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoKind {
    #[default]
    Intro,
    Tutorial,
}

pub struct VideoArgs {
    pub kind: VideoKind,
    pub language: Language,
    pub out: Option<PathBuf>,
    pub stills: Option<PathBuf>,
    pub ffmpeg: String,
    pub size: (u32, u32),
    pub fps: u32,
    pub seed: u32,
    /// Encode losslessly (RGB, for a master to derive other formats from).
    pub lossless: bool,
}

/// Camera pose: eye position and orientation angles in degrees.
#[derive(Clone, Copy)]
struct Pose {
    eye: Vec4,
    yaw: f32,
    pitch: f32,
    ana: f32,
    right_ana: f32,
}

/// Fixed points of the tour, found once from the generated world.
struct Places {
    /// A high viewpoint over the terrain.
    overlook: Vec4,
    /// Integer center of the hollow tesseract built for the demo.
    tesseract: IVec4,
    /// Eye position on the ground for the creature shot.
    meadow: Vec4,
    /// Low eye position on a sandy shore and the horizontal direction (x, z)
    /// from the sand toward the water.
    shore: Vec4,
    shore_dir: Vec4,
    /// A straight sandy walk from `shore` along the beach, the heading to look
    /// while walking it (slightly toward the water), and a W offset at the end
    /// of the walk whose column stays at beach level the whole way.
    shore_walk: Vec4,
    shore_yaw: f32,
    shore_w: f32,
}

#[derive(Clone, Copy)]
struct Shot {
    secs: f32,
    title: &'static str,
    sub: &'static str,
    /// Keycaps shown on screen (default bindings).
    keys: &'static [&'static str],
    /// Which keycaps are down at raw progress `t` (bit i = keys[i]).
    pressed: fn(f32) -> u32,
    /// Fraction of the shot to wait (e.g. for a key tap) before the camera starts moving.
    delay: f32,
    /// Pose at eased progress `t` in 0..=1.
    pose: fn(f32, &Places) -> Pose,
}

const HALF_TESSERACT: i32 = 2;

/// Viewpoint above and to the side of the tesseract, so its hollowness is visible.
const TESS_VIEW: Vec4 = Vec4::new(-5.5, 2.5, -8.5, 0.0);
const TESS_YAW: f32 = 32.0;
const TESS_PITCH: f32 = -12.0;

fn tess_eye(p: &Places, w_offset: f32) -> Vec4 {
    p.tesseract.as_vec4() + Vec4::splat(0.5) + TESS_VIEW + Vec4::W * w_offset
}

/// A slow quarter turn around the torch-lit tesseract at midnight.
fn night_orbit(t: f32, p: &Places) -> Pose {
    let radius = Vec4::new(TESS_VIEW.x, 0.0, TESS_VIEW.z, 0.0).length() * 1.15;
    let heading = TESS_YAW + 150.0 + 70.0 * t;
    let (s, c) = heading.to_radians().sin_cos();
    Pose { eye: p.tesseract.as_vec4() + Vec4::splat(0.5) + Vec4::new(-radius * s, 2.0, -radius * c, 0.0), yaw: heading, pitch: -8.0, ana: 0.0, right_ana: 0.0 }
}

/// Torches on the tesseract's top edge in the visible W layer (night shot).
fn place_night_torches(game: &mut Game, center: IVec4) {
    let r = HALF_TESSERACT;
    for (x, z) in [(-r, -r), (r, -r), (-r, r), (r, r), (0, -r), (0, r), (-r, 0), (r, 0)] {
        game.world.set_block_state(center + IVec4::new(x, r + 1, z, 0), mc4d_core::block::BlockState::new(mc4d_core::block::TORCH, 5));
    }
}

/// A full walk around the 5×5×5×5 tesseract through W, always facing its
/// centre. Facing diagonally between X and Z, the slice normal sweeps the
/// (X+Z, W) plane; whenever it points along the X, Z, W body diagonal the main
/// view cuts the tesseract into a hexagonal prism, while the side view sees an
/// ordinary circuit around a hollow box.
fn tess_orbit(t: f32, p: &Places) -> Pose {
    const YAW: f32 = 45.0;
    let radius = Vec4::new(TESS_VIEW.x, 0.0, TESS_VIEW.z, 0.0).length();
    let diagonal = Vec4::new(YAW.to_radians().sin(), 0.0, YAW.to_radians().cos(), 0.0);
    let angle = 40.0 + 360.0 * t;
    let (s, c) = angle.to_radians().sin_cos();
    Pose {
        // The sideways nudge keeps the side slice (normal = the main view's
        // right) off the voxel grid's diagonal, where block edges would speckle.
        eye: p.tesseract.as_vec4() + Vec4::splat(0.5) + Vec4::new(0.15, 5.5, -0.15, 0.0) - (diagonal * c + Vec4::W * s) * radius,
        yaw: YAW,
        pitch: -26.0,
        ana: angle,
        right_ana: 0.0,
    }
}

/// Held for the middle of the shot.
fn held(t: f32) -> bool {
    (0.04..0.96).contains(&t)
}

/// A short tap at the start of the shot.
fn tap(t: f32) -> bool {
    (0.03..0.11).contains(&t)
}

/// Repeated wheel clicks.
fn clicking(t: f32) -> bool {
    held(t) && (t * 22.0).fract() < 0.45
}

const SHOT_NIGHT: usize = 8;
const SHOT_GHOSTS: usize = 9;
const SHOT_CREATURES: usize = 10;

const SHOTS: &[Shot] = &[
    Shot {
        secs: 10.0,
        title: "Walk around a 5×5×5×5 tesseract",
        sub: "Side view (bottom right): you circle it · main view: its 3D cross-section transforms",
        keys: &[],
        pressed: |_| 0,
        delay: 0.0,
        pose: tess_orbit,
    },
    Shot {
        secs: 4.0,
        title: "mc_4d",
        sub: "Minecraft, but the world has four spatial dimensions",
        keys: &[],
        pressed: |_| 0,
        delay: 0.0,
        pose: |t, p| Pose {
            eye: p.shore - p.shore_dir * 3.0 + Vec4::Y * 1.0,
            yaw: heading_of(p.shore_dir) - 35.0 + 25.0 * t,
            pitch: -9.0,
            ana: 0.0,
            right_ana: 0.0,
        },
    },
    Shot {
        secs: 4.5,
        title: "You see a 3D slice of a 4D world",
        sub: "Walk around inside it just like in Minecraft",
        keys: &["W", "A", "S", "D"],
        pressed: |t| held(t) as u32,
        delay: 0.0,
        pose: |t, p| Pose {
            eye: p.shore + p.shore_walk * t,
            yaw: p.shore_yaw,
            pitch: -8.0,
            ana: 0.0,
            right_ana: 0.0,
        },
    },
    Shot {
        secs: 5.0,
        title: "Step into the 4th dimension",
        sub: "Z / C move along W: you are now looking at a neighbouring slice",
        keys: &["Z", "C"],
        pressed: |t| (held(t) as u32) << 1,
        delay: 0.0,
        pose: |t, p| Pose {
            eye: p.shore + p.shore_walk + Vec4::W * (p.shore_w * t),
            yaw: p.shore_yaw,
            pitch: -8.0,
            ana: 0.0,
            right_ana: 0.0,
        },
    },
    Shot {
        secs: 5.5,
        title: "Turn your view toward W",
        sub: "Alt + mouse wheel: the cubes become cross-sections of tesseracts",
        keys: &["Alt", "Wheel ▲"],
        pressed: |t| if clicking(t) { 0b11 } else { 0 },
        delay: 0.0,
        pose: |t, p| Pose {
            eye: p.shore + p.shore_walk + Vec4::W * p.shore_w,
            yaw: p.shore_yaw,
            pitch: -8.0,
            ana: 55.0 * t,
            right_ana: 0.0,
        },
    },
    Shot {
        secs: 3.5,
        title: "Snap back to the axes",
        sub: "R straightens the slice and keeps the direction you face",
        keys: &["R"],
        pressed: |t| tap(t) as u32,
        delay: 0.12,
        pose: |t, p| Pose {
            eye: p.shore + p.shore_walk + Vec4::W * p.shore_w,
            yaw: p.shore_yaw,
            pitch: -8.0,
            ana: 55.0 * (1.0 - t),
            right_ana: 0.0,
        },
    },
    Shot {
        secs: 7.0,
        title: "A hollow tesseract, built from blocks",
        sub: "Moving through it along W: nothing → solid face → hollow cube → solid face → nothing",
        keys: &["Z", "C"],
        pressed: |t| (held(t) as u32) << 1,
        delay: 0.0,
        pose: |t, p| Pose {
            eye: tess_eye(p, -3.4 + 6.8 * t),
            yaw: TESS_YAW,
            pitch: TESS_PITCH,
            ana: 0.0,
            right_ana: 0.0,
        },
    },
    Shot {
        secs: 6.0,
        title: "Rotate the slice",
        sub: "Hold Alt and move the mouse: the same tesseract, cut at an angle",
        keys: &["Alt", "+", "Mouse ←→"],
        pressed: |t| if held(t) { 0b101 } else { 0 },
        delay: 0.0,
        pose: |t, p| {
            let a = (t * std::f32::consts::PI).sin();
            Pose {
                eye: tess_eye(p, 0.0),
                yaw: TESS_YAW,
                pitch: TESS_PITCH,
                ana: 0.0,
                right_ana: 50.0 * a,
            }
        },
    },
    Shot {
        secs: 6.0,
        title: "Night in four dimensions",
        sub: "Torches, glowstone and smoke: Beta lighting, propagated through 4D space",
        keys: &[],
        pressed: |_| 0,
        delay: 0.0,
        pose: night_orbit,
    },
    Shot {
        secs: 5.0,
        title: "Ghost layers",
        sub: "G: red marks blocks just ana of your slice, blue just kata",
        keys: &["G", "W"],
        pressed: |t| tap(t) as u32 | ((t > 0.15 && t < 0.96) as u32) << 1,
        delay: 0.15,
        pose: |t, p| Pose {
            eye: p.overlook + Vec4::new(12.0 * t * 10f32.to_radians().sin(), 0.0, 12.0 * t * 10f32.to_radians().cos(), 0.0),
            yaw: 10.0,
            pitch: -14.0,
            ana: 0.0,
            right_ana: 0.0,
        },
    },
    Shot {
        secs: 6.0,
        title: "Life beyond your slice",
        sub: "Creatures can enter and leave your slice; 4D glasses (B) reveal nearby outlines",
        keys: &["B"],
        pressed: |t| tap(t) as u32,
        delay: 0.15,
        pose: |t, p| Pose {
            eye: p.meadow,
            yaw: -12.0 + 24.0 * t,
            pitch: -10.0,
            ana: 0.0,
            right_ana: 0.0,
        },
    },
    Shot {
        secs: 4.5,
        title: "Explore a world beyond three dimensions",
        sub: "Choose the 4D Tutorial from the main screen · every control can be rebound",
        keys: &[],
        pressed: |_| 0,
        delay: 0.0,
        pose: |t, p| Pose {
            eye: p.shore + p.shore_dir * (2.0 + 10.0 * t) + Vec4::Y * (1.0 * t),
            yaw: heading_of(p.shore_dir) + 30.0 * t,
            pitch: -6.0,
            ana: 0.0,
            right_ana: 0.0,
        },
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scene {
    World(u32),
    Concept(usize),
    Inventory(GameMode),
    Survival,
    Options,
    Ghosts,
    Creatures,
    Night,
}
struct StoryShot {
    shot: Shot,
    scene: Scene,
    zh: (&'static str, &'static str),
}
fn overlook_pose(t: f32, p: &Places) -> Pose {
    Pose {
        eye: p.overlook,
        yaw: 5.0 + 8.0 * t,
        pitch: -12.0,
        ana: 0.0,
        right_ana: 0.0,
    }
}
fn rotate_tesseract(t: f32, p: &Places) -> Pose {
    Pose {
        eye: tess_eye(p, 0.0),
        yaw: TESS_YAW,
        pitch: TESS_PITCH,
        ana: 35.0 * (t * std::f32::consts::PI).sin(),
        right_ana: 0.0,
    }
}
#[allow(clippy::too_many_arguments)]
fn lesson(
    secs: f32,
    title: &'static str,
    sub: &'static str,
    zh_title: &'static str,
    zh_sub: &'static str,
    scene: Scene,
    keys: &'static [&'static str],
    pose: fn(f32, &Places) -> Pose,
) -> StoryShot {
    StoryShot {
        shot: Shot {
            secs,
            title,
            sub,
            keys,
            pressed: |t| held(t) as u32,
            delay: 0.0,
            pose,
        },
        scene,
        zh: (zh_title, zh_sub),
    }
}
fn storyboard(kind: VideoKind) -> Vec<StoryShot> {
    if kind == VideoKind::Intro {
        let zh = [
            ("环绕 5×5×5×5 超立方体", "侧视图（右下）：你绕着它走一圈 · 主视图：它的三维截面不断变化"),
            ("mc_4d", "熟悉的方块世界，多了一个空间维度"),
            (
                "你看到的是四维世界的三维截面",
                "在这个截面中，像在 Minecraft 中一样自由探索",
            ),
            ("迈向第四个方向", "Z / C 沿 W 移动，让你看到相邻的截面"),
            (
                "把视线转向 W",
                "Alt + 鼠标滚轮：旋转截面，看到超立方体的不同交集",
            ),
            ("回到坐标轴方向", "R 将截面对齐坐标轴，并保持面朝方向"),
            (
                "用方块搭建的空心超立方体",
                "沿 W 穿过它：空无一物 → 实心胞 → 空心立方体 → 实心胞 → 空无一物",
            ),
            ("旋转截面", "按住 Alt 并移动鼠标：倾斜地切过同一个超立方体"),
            ("四维空间的夜晚", "火把、萤石与烟雾：Beta 光照在四维空间中传播"),
            (
                "幽灵层",
                "G：红色表示 ana 一侧的相邻方块，蓝色表示 kata 一侧",
            ),
            (
                "截面之外的生命",
                "生物会进入或离开你的截面；B 可显示附近轮廓",
            ),
            (
                "探索三维之外的世界",
                "在主菜单选择四维教程；所有按键均可重新绑定",
            ),
        ];
        return SHOTS
            .iter()
            .copied()
            .enumerate()
            .map(|(i, shot)| StoryShot {
                shot,
                scene: if i == SHOT_NIGHT {
                    Scene::Night
                } else if i == SHOT_GHOSTS {
                    Scene::Ghosts
                } else if i == SHOT_CREATURES {
                    Scene::Creatures
                } else {
                    Scene::World(2)
                },
                zh: zh[i],
            })
            .collect();
    }
    let mut shots = Vec::new();
    // Concepts use the same animated mathematical diagrams and bilingual text as
    // the interactive tutorial, before introducing movement or game modes.
    for (step, (secs, title, zh)) in [
        (18.0, "Square, cube, tesseract", "正方形、立方体、超立方体"),
        (
            18.0,
            "16 vertices, 32 edges, 24 square faces",
            "16 个顶点、32 条边、24 个正方形面",
        ),
        (22.0, "Eight cubic boundary cells", "8 个立方体边界胞"),
        (22.0, "Projection versus slice", "投影与截面的区别"),
        (
            20.0,
            "One object, many cross-sections",
            "同一个物体，不同的截面",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        shots.push(lesson(
            secs,
            title,
            "",
            zh,
            "",
            Scene::Concept(step),
            &[],
            overlook_pose,
        ));
    }
    shots.extend([
        lesson(10.0,"Main view + auxiliary view","Each view has its own compass. X switches the main view to the side orientation; four presses restore it.","主视图与辅助视图","每个视图都有自己的罗盘。X 将侧视图朝向切换为主视图；按四次恢复原朝向。",Scene::World(2),&[],overlook_pose),
        lesson(10.0,"Walk, then step through W","WASD moves inside the slice. Z / C move perpendicular to it; Y stays up.","先在截面内行走，再迈向 W","WASD 在截面内移动；Z / C 沿截面法线移动；Y 始终向上。",Scene::World(2),&["Z","C"],|t,p|Pose{eye:p.overlook+Vec4::W*(t*10.0),..overlook_pose(0.0,p)}),
        lesson(12.0,"Slide the slice, or rotate it","Alt + mouse / Alt + wheel rotate the slice. R aligns it with the axes again.","平移截面，或旋转截面","Alt + 鼠标或 Alt + 滚轮旋转截面；R 重新对齐坐标轴。",Scene::World(2),&["Alt","Mouse"],rotate_tesseract),
        lesson(10.0,"Creative: build and explore","Double-tap Space to fly. Hold Space to rise; Shift to descend.","创造模式：搭建与探索","双击空格开始飞行；按住空格上升，Shift 下降。",Scene::World(2),&["Space ×2"],|t,p|Pose{eye:p.overlook+Vec4::Y*(t*8.0),..overlook_pose(0.0,p)}),
        lesson(12.0,"Creative inventory","E opens the catalogue. Choose a block; the wheel selects a hotbar slot. Q drops it.","创造模式物品栏","E 打开物品目录；选取方块后，滚轮切换快捷栏；Q 丢弃物品。",Scene::Inventory(GameMode::Creative),&["E"],overlook_pose),
        lesson(12.0,"Survival: health, resources and crafting","Gather materials, craft tools and shelter before night. Hearts and air matter.","生存模式：生命、资源与合成","收集材料、制作工具，并在夜晚前建好庇护所；留意生命和氧气。",Scene::Survival,&[],overlook_pose),
        lesson(12.0,"Survival inventory","Your inventory holds collected items. Use the 2 × 2 grid, or a workbench for 3 × 3 recipes.","生存模式物品栏","物品栏保存你收集的物品；使用 2 × 2 合成格，或用工作台进行 3 × 3 合成。",Scene::Inventory(GameMode::Survival),&["E"],overlook_pose),
        lesson(12.0,"Optional view layouts","Esc → Options → 4D Options: choose the view count. Main + auxiliary is the default.","可选的视图布局","Esc → 选项 → 四维选项：选择视图数量；默认使用主视图加辅助视图。",Scene::Options,&["Esc"],overlook_pose),
        lesson(12.0,"Four views: compare four slices","All views share your position. Only the primary view aims and interacts; the others are references.","四视图：比较四个截面","所有视图共享你的位置；仅主视图用于瞄准和交互，其余视图用于观察。",Scene::World(4),&[],rotate_tesseract),
        lesson(12.0,"Six views: inspect more directions","Compare the same landmark across differently oriented slices. Labels identify each slice.","六视图：观察更多方向","从不同朝向的截面比较同一地标；每幅画面的标签说明截面方向。",Scene::World(6),&[],rotate_tesseract),
        lesson(12.0,"Nine views: a broader overview","More slices can help you survey a survival area; each view is smaller. Choose what you can read comfortably.","九视图：更广的观察范围","更多截面有助于勘察生存区域，但每幅画面更小；选择你能轻松辨认的布局。",Scene::World(9),&[],rotate_tesseract),
        lesson(10.0,"Ghost layers reveal nearby blocks","G shows neighbouring layers: red toward ana, blue toward kata. They are hints, not solid blocks in this slice.","幽灵层揭示附近方块","G 显示相邻层：红色朝向 ana，蓝色朝向 kata；它们是提示，不是当前截面中的实体。",Scene::Ghosts,&["G"],overlook_pose),
        lesson(10.0,"Return to the familiar layout","Set two views for main + auxiliary. V toggles the auxiliary view; F3 toggles debug information.","回到熟悉的布局","设为双视图，恢复主视图加辅助视图；V 开关辅助视图；F3 开关调试信息。",Scene::World(2),&["V","F3"],overlook_pose),
    ]);
    shots
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn apply_pose(game: &mut Game, pose: &Pose) {
    let mut o = Orientation::default();
    o.yaw(pose.yaw.to_radians());
    o.add_pitch(pose.pitch.to_radians());
    o.rotate_forward_ana(pose.ana.to_radians());
    o.rotate_right_ana(pose.right_ana.to_radians());
    game.orient = o;
    game.body.pos = pose.eye - Vec4::Y * EYE_HEIGHT;
    game.body.vel = Vec4::ZERO;
}

/// Build a 5⁴ hollow tesseract: its 3D cells are bricks, its lower-dimensional faces glow.
fn build_tesseract(game: &mut Game, center: IVec4) {
    let r = HALF_TESSERACT;
    // The orbit circles through W, so level a meadow around the tesseract in
    // the nearby W layers too: nothing above its base, grass just below it.
    use mc4d_core::block::{DIRT, GRASS, WATER};
    for w in -18..=18 {
        for z in -20..=20 {
            for x in -20..=20 {
                for y in -r - 4..=-r + 16 {
                    let p = center + IVec4::new(x, y, z, w);
                    game.world.load_now(chunk_of(p), None);
                    let current = game.world.block(p);
                    let want = match y + r {
                        0.. => AIR,
                        -1 => GRASS,
                        _ if matches!(current, Some(AIR | WATER)) => DIRT,
                        _ => continue,
                    };
                    if current != Some(want) {
                        game.world.set_block(p, want);
                    }
                }
            }
        }
    }
    for w in -r - 1..=r + 1 {
        for z in -r - 1..=r + 1 {
            for y in -r - 1..=r + 1 {
                for x in -r - 1..=r + 1 {
                    let p = center + IVec4::new(x, y, z, w);
                    game.world.load_now(chunk_of(p), None);
                    let d = IVec4::new(x, y, z, w).abs();
                    let on_boundary = d.cmpeq(IVec4::splat(r)).bitmask().count_ones();
                    let block = if d.max_element() > r {
                        AIR
                    } else {
                        match on_boundary {
                            0 => AIR,
                            1 => BRICK,
                            _ => GLOW,
                        }
                    };
                    game.world.set_block(p, block);
                }
            }
        }
    }
}

fn find_places(game: &mut Game) -> Places {
    let spawn = game.body.pos;
    let terrain = game.world.terrain.clone();
    let ground = |x: f32, z: f32, w: f32| {
        terrain.height(x.floor() as i32, z.floor() as i32, w.floor() as i32) as f32
    };
    let overlook = Vec4::new(
        spawn.x,
        ground(spawn.x, spawn.z, spawn.w).max(44.0) + 6.0,
        spawn.z - 6.0,
        spawn.w,
    );
    let (tx, tz, tw) = (spawn.x + 2.0, spawn.z + 14.0, spawn.w + 40.0);
    let t = IVec4::new(
        tx as i32,
        ground(tx, tz, tw).max(44.0) as i32 + 1 + HALF_TESSERACT,
        tz as i32,
        tw as i32,
    );
    let (mx, mz) = find_clearing(game, spawn);
    let meadow = Vec4::new(
        mx + 0.5,
        ground(mx, mz, spawn.w) + 1.0 + EYE_HEIGHT + 1.2,
        mz + 0.5,
        spawn.w,
    );
    let shore = find_shore(&terrain, spawn).unwrap_or_else(|| {
        eprintln!("video: no shore found near spawn");
        Shore { eye: overlook, dir: Vec4::Z, walk: Vec4::new(0.0, 0.0, 12.0, 0.0), yaw: 0.0, w: 14.0 }
    });
    Places {
        overlook,
        tesseract: t,
        meadow,
        shore: shore.eye,
        shore_dir: shore.dir,
        shore_walk: shore.walk,
        shore_yaw: shore.yaw,
        shore_w: shore.w,
    }
}

struct Shore {
    eye: Vec4,
    dir: Vec4,
    walk: Vec4,
    yaw: f32,
    w: f32,
}

/// A beach near spawn in the spawn's W layer: the eye stands where grass meets
/// sand, with dry grass behind and open water ahead. From there a straight
/// strip of sand runs along the shore, and the beach continues along W at its
/// end. The narrowest beach wins so that grass, sand and water are all close.
fn find_shore(terrain: &mc4d_core::terrain::TerrainGen, spawn: Vec4) -> Option<Shore> {
    use mc4d_core::terrain::{Biome, SEA_LEVEL as SEA};
    const WALK: i32 = 12;
    let w0 = spawn.w.floor() as i32;
    let height = |x: i32, z: i32, w: i32| terrain.height(x, z, w);
    let (sx, sz) = (spawn.x.floor() as i32, spawn.z.floor() as i32);
    let mut best: Option<((i32, i32), Shore)> = None;
    for x in sx - 160..=sx + 160 {
        for z in sz - 160..=sz + 160 {
            // The first sand column next to the grass.
            if !matches!(height(x, z, w0) - SEA, 0 | 1) || terrain.biome(x, z, w0) == Biome::Swamp {
                continue;
            }
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let at = |d: i32| height(x + dx * d, z + dz * d, w0);
                if !(1..=5).all(|d| (SEA + 2..=SEA + 4).contains(&at(-d))) {
                    continue;
                }
                let Some(edge) = (1..=14).find(|&d| at(d) < SEA) else { continue };
                let score = (edge, (x - sx).abs() + (z - sz).abs());
                if best.as_ref().is_some_and(|(b, _)| *b <= score) {
                    continue;
                }
                let beach = (1..edge).all(|d| (SEA..=SEA + 1).contains(&at(d)));
                let sea = (edge..edge + 12).all(|d| at(d) < SEA);
                if !beach || !sea {
                    continue;
                }
                for side in [1, -1] {
                    let (px, pz) = (side * dz, -side * dx);
                    let sand = |d: i32, w: i32| (SEA..=SEA + 1).contains(&height(x + px * d, z + pz * d, w));
                    if !(0..=WALK).all(|d| sand(d, w0)) {
                        continue;
                    }
                    let (ex, ez) = (x + px * WALK, z + pz * WALK);
                    let low = |w: i32| (0..=3).all(|d| height(ex + px * d, ez + pz * d, w) <= SEA + 1);
                    let step = (6..=14i32).rev().flat_map(|n| [n, -n]).find(|&n| {
                        (0..=n.abs()).all(|k| low(w0 + k * n.signum()))
                    });
                    let Some(step) = step else { continue };
                    let dir = Vec4::new(dx as f32, 0.0, dz as f32, 0.0).normalize();
                    let walk = Vec4::new((px * WALK) as f32, 0.0, (pz * WALK) as f32, 0.0);
                    let eye = Vec4::new(x as f32 + 0.5, (SEA + 2) as f32 + EYE_HEIGHT, z as f32 + 0.5, spawn.w);
                    let yaw = heading_of(walk) - side as f32 * 15.0;
                    best = Some((score, Shore { eye, dir, walk, yaw, w: step as f32 }));
                    break;
                }
            }
        }
    }
    best.map(|(_, shore)| shore)
}

fn heading_of(dir: Vec4) -> f32 { dir.x.atan2(dir.z).to_degrees() }

/// A dry-land spot for the creature shot; trees in view ahead (+Z) are removed.
fn find_clearing(game: &mut Game, spawn: Vec4) -> (f32, f32) {
    let w = spawn.w.floor() as i32;
    let (mut x, mut z) = (spawn.x.floor() as i32 + 30, spawn.z.floor() as i32 - 20);
    for i in 0..40 {
        let (cx, cz) = (x + 9 * (i % 6), z + 9 * (i / 6));
        if game.world.terrain.height(cx, cz, w) > mc4d_core::terrain::SEA_LEVEL + 2 {
            (x, z) = (cx, cz);
            break;
        }
    }
    let h = game.world.terrain.height(x, z, w);
    for dw in -3..=3 {
        for dz in -4..=16 {
            for dx in -10..=10 {
                for dy in 0..=12 {
                    let p = IVec4::new(x + dx, h + dy, z + dz, w + dw);
                    game.world.load_now(chunk_of(p), None);
                    if matches!(
                        game.world.block(p),
                        Some(mc4d_core::block::LEAVES | mc4d_core::block::LOG)
                    ) {
                        game.world.set_block(p, AIR);
                    }
                }
            }
        }
    }
    (x as f32, z as f32)
}

fn spawn_meadow_creatures(game: &mut Game, p: &Places) {
    game.creatures.clear();
    let terrain = game.world.terrain.clone();
    // (sideways offset, distance ahead, offset along W, radius, color)
    let specs: [(f32, f32, f32, f32, [f32; 3]); 7] = [
        (-2.5, 6.0, 0.0, 0.6, [0.95, 0.55, 0.65]),
        (1.5, 7.5, 0.0, 0.55, [0.55, 0.85, 0.95]),
        (3.5, 5.5, 0.35, 0.6, [0.95, 0.85, 0.45]),
        (-0.5, 9.0, 0.5, 0.65, [0.6, 0.95, 0.6]),
        (-4.5, 8.5, 1.6, 0.6, [0.8, 0.6, 0.95]),
        (5.0, 9.5, -2.0, 0.6, [0.95, 0.6, 0.4]),
        (0.5, 4.5, -0.25, 0.5, [0.7, 0.7, 0.95]),
    ];
    for (index, (side, ahead, w, _r, _color)) in specs.into_iter().enumerate() {
        let x = p.meadow.x + side;
        let z = p.meadow.z + ahead;
        let wp = p.meadow.w + w;
        let h = terrain.height(x.floor() as i32, z.floor() as i32, wp.floor() as i32) as f32 + 1.0;
        let kind = [
            crate::game::CreatureKind::Pig,
            crate::game::CreatureKind::Cow,
            crate::game::CreatureKind::Sheep,
            crate::game::CreatureKind::Chicken,
            crate::game::CreatureKind::Wolf,
            crate::game::CreatureKind::Pig,
            crate::game::CreatureKind::Sheep,
        ][index];
        game.spawn_creature(kind, Vec4::new(x, h, z, wp));
    }
}

pub fn run(args: &VideoArgs) -> Result<()> {
    record(args, None)
}

fn record(args: &VideoArgs, selection: Option<&[usize]>) -> Result<()> {
    if args.out.is_some() && args.stills.is_some() {
        return Err("choose a video output or preview stills, not both".into());
    }
    if args.fps == 0 || args.size.0 == 0 || args.size.1 == 0 {
        return Err("video dimensions and frame rate must be positive".into());
    }
    let size = args.size;
    let HeadlessGpu {
        device,
        queue,
        target,
        adapter_name,
    } = headless_gpu(size)?;
    let mut fr = FrameRenderer::new(&device, target.format(), size, adapter_name);
    // Lay the UI out as at 720p and scale it to the output resolution.
    let ppp = size.1 as f32 / 720.0;
    fr.egui_ctx.set_zoom_factor(ppp);
    let points = ((size.0 as f32 / ppp) as u32, 720);
    let mut game = Game::new(None, args.seed, GameMode::Creative);
    game.flying = true;
    let places = find_places(&mut game);
    build_tesseract(&mut game, places.tesseract);

    let mut settings = Settings {
        language: args.language,
        gui_scale: 2,
        show_hints: false,
        render_distance: 6,
        ..Settings::default()
    };
    let mut bindings = Bindings::default();
    let input = InputState::default();
    let mut ui = UiState {
        show_debug: false,
        video_presentation: true,
        video_compass: true,
        ..UiState::default()
    };
    let shots = storyboard(args.kind);
    ui.dt = 1.0 / args.fps as f32;
    let dt = 1.0 / args.fps as f32;

    let mut ffmpeg = match &args.out {
        Some(out) => {
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let child = Command::new(&args.ffmpeg)
                .args([
                    "-y",
                    "-loglevel",
                    "error",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgba",
                ])
                .args([
                    "-s",
                    &format!("{}x{}", size.0, size.1),
                    "-r",
                    &args.fps.to_string(),
                    "-i",
                    "-",
                ])
                .args(if args.lossless {
                    &["-c:v", "libx264rgb", "-preset", "veryfast", "-qp", "0"][..]
                } else {
                    &[
                        "-c:v",
                        "libx264",
                        "-preset",
                        "slow",
                        "-crf",
                        "16",
                        "-pix_fmt",
                        "yuv420p",
                        "-movflags",
                        "+faststart",
                    ][..]
                })
                .arg(out)
                .stdin(Stdio::piped())
                .spawn()
                .map_err(|e| format!("could not start ffmpeg ({}): {e}", args.ffmpeg))?;
            Some(child)
        }
        None => None,
    };

    let mut render = |game: &mut Game, settings: &mut Settings, ui: &mut UiState| {
        game.target = None;
        fr.render(
            &device,
            &queue,
            &target,
            FrameInput {
                game,
                settings,
                bindings: &mut bindings,
                input: &input,
                ui,
                raw_input: crate::raw_input(points),
                size,
                pixels_per_point: ppp,
            },
        );
        !ui.shows_world()
            || (fr
                .renderer
                .store
                .missing_dense_count(&game.world, &game.gpu_wanted)
                == 0
                && fr.renderer.missing_lighting() == 0
                && game.world.lighting_ready(&game.gpu_wanted))
    };

    let mut frames = 0u32;
    for (i, story) in shots.iter().enumerate() {
        if selection.is_some_and(|indices| !indices.contains(&i)) {
            continue;
        }
        let shot = &story.shot;
        let creatures = story.scene == Scene::Creatures;
        let night = story.scene == Scene::Night;
        // Midnight with torches for the night shot; the ordinary daytime otherwise.
        game.environment.world_ticks = if night { 18000 } else { 6000 };
        if night { place_night_torches(&mut game, places.tesseract); }
        settings.view_count = match story.scene {
            Scene::World(count) => count,
            _ => 2,
        };
        settings.inset = true;
        ui.screen = Screen::Playing;
        ui.tutorial = None;
        ui.caption = None;
        ui.keys.clear();
        game.set_mode(match story.scene {
            Scene::Survival | Scene::Inventory(GameMode::Survival) => GameMode::Survival,
            _ => GameMode::Creative,
        });
        game.flying = true;
        if let Scene::Inventory(mode) = story.scene {
            ui.screen = Screen::Inventory;
            ui.inventory_catalog = mode == GameMode::Creative;
            if mode == GameMode::Survival {
                game.hotbar = [None; 9];
                game.hotbar[0] = Some(crate::game::Slot::new(mc4d_core::item::WOOD_PICKAXE, 1));
                game.hotbar[1] = Some(crate::game::Slot::new(mc4d_core::block::PLANKS, 12));
                game.hotbar[2] = Some(crate::game::Slot::new(mc4d_core::item::BREAD, 2));
                game.inventory[0] = Some(crate::game::Slot::new(mc4d_core::block::LOG, 16));
                game.inventory[1] = Some(crate::game::Slot::new(mc4d_core::item::COAL, 8));
                game.crafting_grid[0] = Some(crate::game::Slot::new(mc4d_core::block::LOG, 1));
            }
        }
        if let Scene::Concept(step) = story.scene {
            let mut tutorial = Tutorial::new(&game, &settings);
            tutorial.skip_to(step, &game, &settings);
            ui.tutorial = Some(tutorial);
        }
        if creatures {
            spawn_meadow_creatures(&mut game, &places);
        } else {
            game.creatures.clear();
        }
        // Toggles start off and switch on when their key is tapped.
        settings.ghosts = false;
        settings.glasses = false;
        // Warm up: stream in chunks for the start of the shot and let uploads finish.
        apply_pose(&mut game, &(shot.pose)(0.0, &places));
        settle(&mut game, &settings)?;
        let warmup = std::time::Instant::now();
        while !render(&mut game, &mut settings, &mut ui) {
            if warmup.elapsed().as_secs() > 60 {
                return Err("video GPU uploads did not settle; refusing an incomplete shot".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let n = (shot.secs * args.fps as f32).round() as u32;
        let range: Vec<u32> = if args.stills.is_some() {
            vec![n / 2]
        } else {
            (0..n).collect()
        };
        for f in range {
            let t = f as f32 / (n - 1).max(1) as f32;
            let motion = smooth((t - shot.delay) / (1.0 - shot.delay));
            let pose = (shot.pose)(motion, &places);
            if story.scene == Scene::Ghosts {
                settings.ghosts = t > 0.08;
            }
            if creatures {
                settings.glasses = t > 0.08;
            }
            apply_pose(&mut game, &pose);
            if creatures || night {
                // Let creatures hop and torch smoke rise; the camera stays put.
                game.update(dt, None, &settings);
                if night { game.environment.world_ticks = 18000; }
                apply_pose(&mut game, &pose);
            }
            game.update_streaming(&settings, false);
            settle(&mut game, &settings)?;
            let secs = f as f32 * dt;
            let alpha = (secs / 0.35).min((shot.secs - secs) / 0.35).clamp(0.0, 1.0);
            ui.caption = if matches!(story.scene, Scene::Concept(_)) {
                None
            } else {
                let (title, sub) = if args.language == Language::Chinese {
                    story.zh
                } else {
                    (shot.title, shot.sub)
                };
                Some((title.into(), sub.into(), alpha))
            };
            if story.scene == Scene::Options {
                ui.screen = if t < 0.5 {
                    Screen::Playing
                } else {
                    Screen::FourDOptions
                };
                if t >= 0.5 {
                    ui.caption = None;
                }
            }
            if args.stills.is_some()
                && let Some(tutorial) = ui.tutorial.as_mut()
            {
                for _ in 0..(secs / 0.25) as usize {
                    tutorial.update(0.25, &game, &settings, false);
                }
            }
            let down = (shot.pressed)(t);
            ui.keys = shot
                .keys
                .iter()
                .enumerate()
                .map(|(k, label)| (label.to_string(), down & (1 << k) != 0))
                .collect();
            let ready_start = std::time::Instant::now();
            while !render(&mut game, &mut settings, &mut ui) {
                if ready_start.elapsed().as_secs() > 60 {
                    return Err("video frame has stale GPU geometry or lighting; refusing incomplete output".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }

            if let Some(dir) = &args.stills {
                render(&mut game, &mut settings, &mut ui); // uploads triggered by the first render
                let rgba = read_texture(&device, &queue, &target);
                let path = dir.join(format!("shot{:02}.png", i + 1));
                write_png(&path, size.0, size.1, &rgba)?;
                println!("wrote {}", path.display());
            }
            if let Some(child) = &mut ffmpeg {
                let rgba = read_texture(&device, &queue, &target);
                child.stdin.as_mut().unwrap().write_all(&rgba)?;
                frames += 1;
            }
        }
        eprintln!("shot {}/{} done: {}", i + 1, shots.len(), shot.title);
    }

    if let Some(mut child) = ffmpeg {
        drop(child.stdin.take());
        let status = child.wait()?;
        if !status.success() {
            return Err(format!("ffmpeg failed: {status}").into());
        }
        let out = args.out.as_deref().unwrap_or(Path::new("?"));
        println!(
            "wrote {} ({frames} frames, {:.1}s)",
            out.display(),
            frames as f32 / args.fps as f32
        );
    }
    Ok(())
}

/// Block until the chunks around the current pose are generated (bounded wait).
fn settle(game: &mut Game, settings: &Settings) -> Result<()> {
    let start = std::time::Instant::now();
    loop {
        game.update_streaming(settings, false);
        let ready = game.world.pending_count() == 0
            && game.gpu_wanted.iter().all(|c| game.world.is_loaded(*c))
            && game.world.lighting_ready(&game.gpu_wanted);
        if ready {
            return Ok(());
        }
        if start.elapsed().as_secs() > 60 {
            return Err(format!("video scene did not settle: {} loading chunks, {} lighting tasks; refusing incomplete output",game.world.pending_count(),game.world.lighting_pending()).into());
        }
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intro_stays_main_plus_aux_and_does_not_advertise_grid_modes() {
        let shots = storyboard(VideoKind::Intro);
        assert!(!shots.is_empty());
        for story in shots {
            assert!(matches!(
                story.scene,
                Scene::World(2) | Scene::Ghosts | Scene::Creatures | Scene::Night
            ));
            let text = format!("{} {}", story.shot.title, story.shot.sub).to_lowercase();
            assert!(
                !text.contains("four views")
                    && !text.contains("six views")
                    && !text.contains("nine views")
            );
        }
    }
    #[test]
    fn tutorial_teaches_geometry_before_controls_and_covers_every_layout_and_game_mode() {
        let shots = storyboard(VideoKind::Tutorial);
        for (step, shot) in shots.iter().take(5).enumerate() {
            assert_eq!(shot.scene, Scene::Concept(step));
        }
        for count in [2, 4, 6, 9] {
            assert!(shots.iter().any(|s| s.scene == Scene::World(count)));
        }
        for mode in [GameMode::Creative, GameMode::Survival] {
            assert!(shots.iter().any(|s| s.scene == Scene::Inventory(mode)));
        }
        assert!(shots.iter().any(|s| s.scene == Scene::Survival));
        assert_eq!(shots.last().unwrap().scene, Scene::World(2));
        let places = Places {
            overlook: Vec4::new(0.0, 50.0, 0.0, 0.0),
            tesseract: IVec4::new(0, 65, 12, 40),
            meadow: Vec4::new(0.0, 32.0, 0.0, 0.0),
            shore: Vec4::new(0.0, 47.6, 0.0, 0.0),
            shore_dir: Vec4::Z,
            shore_walk: Vec4::new(12.0, 0.0, 0.0, 0.0),
            shore_yaw: 80.0,
            shore_w: 14.0,
        };
        for story in shots {
            assert!(!story.zh.0.is_empty());
            assert!(story.shot.secs >= 10.0);
            for t in [0.0, 0.5, 1.0] {
                let p = (story.shot.pose)(t, &places);
                assert!(p.eye.is_finite());
                assert!(
                    p.yaw.is_finite()
                        && p.pitch.is_finite()
                        && p.ana.is_finite()
                        && p.right_ana.is_finite()
                );
            }
        }
    }
    #[test]
    #[ignore = "requires GPU; captures 1080p tutorial concepts and all optional layouts without publishing a video"]
    fn tutorial_storyboard_preview_frames_are_complete() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tutorial-video-review");
        let args = VideoArgs {
            kind: VideoKind::Tutorial,
            language: Language::English,
            out: None,
            stills: Some(root),
            ffmpeg: "ffmpeg".into(),
            size: (1920, 1080),
            fps: 30,
            seed: 1337,
            lossless: false,
        };
        record(&args, Some(&[0, 2, 9, 11, 12, 13, 14, 15])).unwrap();
    }
}
