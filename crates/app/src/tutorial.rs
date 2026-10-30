//! Guided first-time tutorial. Each step has an objective that completes when the player
//! actually does it (tracked via `game::Stats`), or waits for Enter on purely explanatory
//! steps. Key names come from the current bindings, so rebinding updates the text.

use crate::game::{Game, Stats};
use crate::input::{Action, Bindings, InputState};
use crate::settings::Settings;
use crate::i18n::{self, Language};
use crate::tutorial_visual::Diagram;

enum Goal {
    /// Press Enter to continue.
    Continue,
    /// Objective met, given the stats gained since the step started.
    Check(fn(&Stats, &Game, &Settings, &Tutorial) -> bool),
}

struct Step {
    title: [&'static str; 2],
    text: [&'static str; 2],
    keys: &'static [(&'static str, Action)],
    goal: Goal,
}

const STEPS: &[Step] = &[
    Step {
        title: ["Build the idea: square → cube → tesseract", "先建立直觉：正方形 → 立方体 → 超立方体"],
        text: [
            "A square extends in two independent directions. Copy it and connect matching corners in a third direction: you get a cube. Do the same with a cube in W, a fourth direction perpendicular to all three: you get a tesseract, or 4D hypercube. W is space, not time. Watch the copies separate and their corners connect in the diagram.",
            "正方形沿两个独立方向展开。把它复制一份，沿第三个方向移开，再连接对应的顶点，就得到立方体。对立方体再做一次同样的事，沿同时垂直于前三个方向的第四个空间方向 W 展开，就得到四维超立方体。W 是空间方向，不是时间。观察图中的副本如何分开，并连接对应顶点。",
        ], keys: &[], goal: Goal::Continue,
    },
    Step {
        title: ["16 vertices, 32 edges, 24 square faces", "16 个顶点、32 条边、24 个正方形面"],
        text: [
            "A tesseract has 16 vertices and 32 edges. Its two-dimensional faces are 24 squares: the blue highlight cycles through them. These are ordinary square faces, not the three-dimensional boundary pieces. Rotation changes their projected appearance, but their counts and connections never change.",
            "超立方体有 16 个顶点和 32 条边。它的二维面是 24 个正方形；蓝色高亮会逐个展示它们。这里说的是普通的正方形面，并不是三维的边界部分。旋转会改变它们在屏幕上的投影形状，但数量和连接关系始终不变。",
        ], keys: &[], goal: Goal::Continue,
    },
    Step {
        title: ["Eight cubes form the boundary", "8 个立方体构成它的边界"],
        text: [
            "A cube is bounded by 6 squares. A tesseract is bounded by 8 cubic cells: its three-dimensional boundary pieces, highlighted in gold. Each cell has 8 vertices, 12 edges and 6 square faces. Every square is shared by two cells, so there are 8 × 6 ÷ 2 = 24 unique squares. The 8 cells are the boundary of one 4D solid, not eight interior volumes to add together.",
            "立方体的边界由 6 个正方形构成；超立方体的边界则由 8 个立方体胞构成，也可称为三维“体面”，图中以金色高亮。每个胞有 8 个顶点、12 条边、6 个正方形面。每个正方形由两个胞共享，因此独立的正方形面共有 8 × 6 ÷ 2＝24 个。这 8 个胞是同一个四维实体的边界，不能把它们当成内部的 8 块体积相加。",
        ], keys: &[], goal: Goal::Continue,
    },
    Step {
        title: ["A projection is different from a slice", "投影与截面是两回事"],
        text: [
            "The familiar small cube inside a larger cube is a projection: all 16 corners are mapped onto a lower-dimensional picture, so some overlap. It does not mean one physical cube sits inside another. Your game view instead shows an intersection: only the 3D slice through your position. On the right, the slice slides through the tesseract, showing a cube or nothing while the full object still exists.",
            "常见的“小立方体套在大立方体里”是投影：全部 16 个顶点被映射到低维图像上，因此会重叠。这并不表示一个真实立方体装在另一个里面。游戏视图显示的则是交集，也就是经过你当前位置的三维截面。右图让截面穿过超立方体：有时得到立方体，有时没有交集，但完整物体一直存在。",
        ], keys: &[], goal: Goal::Continue,
    },
    Step {
        title: ["One object, many cross-sections", "同一个物体，不同的截面"],
        text: [
            "Now the slicing direction tilts through 4D space. The right-hand solid changes shape as the plane meets different edges and boundary cells. It is a true 3D intersection shown from outside for learning, not the whole tesseract and not necessarily one of its 8 boundary cubes. In the game you stand inside this slice. Moving along W changes its position; rotating into W changes its direction.",
            "现在让截取方向在四维空间中倾斜。右边的实体会改变形状，因为截面经过了不同的边和边界胞。这是从外部观察的真实三维交集，便于理解；它既不是完整超立方体，也不一定是那 8 个边界立方体之一。游戏中，你站在这个截面里。沿 W 移动会改变截面位置，向 W 旋转则改变截面方向。",
        ], keys: &[], goal: Goal::Continue,
    },
    Step {
        title: ["A slice is not the whole object", "截面不是整个物体"],
        text: [
            "Imagine a flat sheet passing through a ball: its intersection is a circle that grows, then shrinks and disappears. Here, a 3D slice of a 4D ball is a sphere. As you move along W, the visible sphere can grow or vanish without the creature changing size. A block is a 4D cube: a tesseract.",
            "想象一张平面穿过一个球：交集是一个圆，先变大，再变小，最后消失。同样，四维球的三维截面是一个球。沿 W 移动时，眼前的球可能变大或消失，但生物本身并没有改变大小。这里的方块是四维立方体，也叫超立方体。",
        ], keys: &[], goal: Goal::Continue,
    },
    Step {
        title: ["Look around", "环顾四周"],
        text: ["Move the mouse to look around. Looking left/right or up/down changes where you face inside the current slice.", "移动鼠标环顾四周。普通的左右转头和抬头低头，只会改变你在当前截面中的朝向。"],
        keys: &[], goal: Goal::Check(|s, _, _, _| s.looked > 1.5),
    },
    Step {
        title: ["Walk", "行走"],
        text: ["Walk with $forward $left $back $right and jump with $jump. These directions lie inside the slice; Y is always up, even after rotating into W. In Creative, double-tap $jump to start or stop flying; hold it to rise and hold $down to descend.", "使用 $forward $left $back $right 行走，使用 $jump 跳跃。这些方向都在截面内；即使向 W 旋转，Y 方向仍然向上。在创造模式中，双击 $jump 开始或停止飞行；按住它上升，按住 $down 下降。"],
        keys: &[("$forward", Action::MoveForward), ("$left", Action::MoveLeft), ("$back", Action::MoveBack), ("$right", Action::MoveRight), ("$jump", Action::Jump), ("$down", Action::Sneak)],
        goal: Goal::Check(|s, _, _, _| s.moved_slice > 4.0),
    },
    Step {
        title: ["Step into W", "迈向 W 方向"],
        text: [
            "$ana / $kata move perpendicular to your slice, toward ana / kata. These names mean the two opposite directions along the slice normal. In the starting orientation they are +W / −W. Move a few blocks: you change position, revealing a neighboring slice. Returning along the same path restores what you saw.",
            "$ana／$kata 分别沿截面的垂直方向移动，也就是 ana／kata。这两个词表示截面法线上的相反方向；在初始朝向下，它们是 +W／−W。试着移动几格：你的位置改变了，因此看到了相邻截面。原路返回，就能再次看到原先的景象。",
        ], keys: &[("$ana", Action::MoveAna), ("$kata", Action::MoveKata)],
        goal: Goal::Check(|s, _, _, _| s.moved_ana > 3.0),
    },
    Step {
        title: ["Translation and rotation", "平移与旋转"],
        text: [
            "Moving along ana/kata slides the viewing plane to a new place. Rotating into W turns that plane through your current position instead. Think of sliding a knife through a loaf versus tilting the knife: each gives different cross-sections of the same object. Neither operation changes the blocks themselves.",
            "沿 ana／kata 移动，是把观察平面平移到新的位置；向 W 旋转，则是让平面绕你当前的位置转动。就像切面包时平移刀的位置，或者倾斜刀的角度：两者会得到同一物体的不同截面，都不会改变方块本身。",
        ], keys: &[], goal: Goal::Continue,
    },
    Step {
        title: ["Turn into W", "向 W 旋转"],
        text: [
            "Use $turn_plus / $turn_minus to rotate forward into ana/kata. The blocks can look slanted or change shape because the viewing plane cuts tesseracts at a new angle. Try turning at least 40°. After rotating, ana may combine X, Z and W rather than equal world W.",
            "使用 $turn_plus／$turn_minus，将前方转向 ana／kata。方块可能变得倾斜或改变形状，因为观察平面以新的角度切过超立方体。试着旋转至少 40°。旋转后，ana 可能同时含有 X、Z 和 W 方向的分量，而不再等于世界的 W 方向。",
        ], keys: &[("$turn_plus", Action::RotateAnaPos), ("$turn_minus", Action::RotateAnaNeg)],
        goal: Goal::Check(|s, _, _, _| s.turned_forward_ana > 40f32.to_radians()),
    },
    Step {
        title: ["Snap back", "重新对齐"],
        text: ["Press $snap to align the slice with the nearest world axis. This changes orientation, not your position. Use the XYZW coordinates to tell where you are.", "按 $snap，将截面对齐到最近的世界坐标轴。这会改变朝向，不会改变位置。可以通过 XYZW 坐标判断自己在哪里。"],
        keys: &[("$snap", Action::SnapSlice)],
        goal: Goal::Check(|s, g, _, _| s.snaps > 0 && !g.orient.is_animating() && g.orient.misalignment() < 0.02),
    },
    Step {
        title: ["Flip 90°", "翻转 90°"],
        text: ["Press $flip to rotate the forward/ana plane by 90°. The old hidden direction becomes visible and the old forward direction leaves the slice. Repeated flips cycle through axis-aligned directions.", "按 $flip，将前方与 ana 构成的平面旋转 90°。原先隐藏的方向变得可见，而原来的前方离开了截面。重复翻转可以在对齐坐标轴的朝向之间循环。"],
        keys: &[("$flip", Action::FlipAna)],
        goal: Goal::Check(|s, g, _, _| s.flips > 0 && !g.orient.is_animating()),
    },
    Step {
        title: ["Your 4D compass", "四维罗盘"],
        text: ["Each compass shows how world X, Z and W project into its own view; the side view has its own compass. The ana bars show how much each world axis points out of your slice. When an axis lies mostly outside the slice, movement along it can reveal terrain that was invisible a moment ago.", "每个罗盘显示世界 X、Z、W 轴在对应视图中的投影；侧视图也有自己的罗盘。ana 条显示各坐标轴有多少分量指向截面外。当某个轴主要位于截面外时，沿它移动就可能看到刚才不可见的地形。"],
        keys: &[], goal: Goal::Continue,
    },
    Step {
        title: ["Read several slices together", "同时理解多个截面"],
        text: [
            "Esc → Options offers 2, 4, 6 or 9 views. They share your eye position but cut different planes through XZW space. View 2 puts ana on its right. Other views add perpendicular and diagonal cuts. A creature may appear in one view and not another: that is different intersection geometry, not a different world. Only view 1 (Play) aims, breaks and places blocks.",
            "Esc → 选项可以选择 2、4、6 或 9 个视图。它们共享你的眼睛位置，但在 XZW 空间中截取不同的平面。视图 2 的右方是 ana；其他视图补充垂直和斜向截面。生物可能只在某些视图中出现，这是交集不同，并不是不同的世界。只有视图 1（操作视图）用于瞄准、破坏和放置方块。",
        ], keys: &[], goal: Goal::Continue,
    },
    Step {
        title: ["Compare the views", "比较不同视图"],
        text: ["Use $views to show/hide the extra views, or open Esc → Options and change the slice layout. Then return to the game. Compare one nearby landmark in the different cuts. Press $side to make the displayed side orientation your main view, keeping position and pitch. Four presses restore the original frame; aiming and movement follow the new main view.", "使用 $views 显示或隐藏额外视图，或者打开 Esc → 选项，改变视图布局，然后返回游戏。比较同一个附近地标在不同截面中的样子。按 $side 将侧视图的朝向切换为主视图，保持位置和俯仰角；按四次恢复原朝向，瞄准和移动跟随新的主视图。"],
        keys: &[("$views", Action::ToggleInset), ("$side", Action::SwitchToSideView)],
        goal: Goal::Check(|_, _, s, t| s.active_view_count() != t.views_at_start),
    },
    Step {
        title: ["Ghost layers", "相邻截面提示"],
        text: ["Press $ghosts to toggle ghost layers. Red shows blocks just ana of your slice; blue shows blocks just kata. Unlike the other viewports, these are parallel slices at nearby positions. Use them to plan your next step out of the current slice.", "按 $ghosts 切换相邻截面提示。红色显示截面 ana 侧附近的方块，蓝色显示 kata 侧附近的方块。与其他视图不同，这些是位于附近位置的平行截面。可以利用它们规划离开当前截面后的下一步。"],
        keys: &[("$ghosts", Action::ToggleGhosts)],
        goal: Goal::Check(|_, _, s, t| s.ghosts != t.ghosts_at_start),
    },
    Step {
        title: ["Rotate the slice freely", "自由旋转截面"],
        text: ["Hold $modifier and move the mouse. Sideways rotates right toward ana; up/down rotates forward toward ana. Your whole slice frame changes, including what ana/kata movement means. Use $snap whenever you need an axis-aligned frame again.", "按住 $modifier 并移动鼠标。左右移动会让右方转向 ana，上下移动会让前方转向 ana。整个截面坐标架都会改变，包括 ana／kata 移动所对应的方向。需要重新对齐坐标轴时，可以按 $snap。"],
        keys: &[("$modifier", Action::SliceModifier), ("$snap", Action::SnapSlice)],
        goal: Goal::Check(|s, _, _, _| s.turned_right_ana > 30f32.to_radians()),
    },
    Step {
        title: ["Build", "开始建造"],
        text: ["Break with $break and place with $place, aiming through view 1. W-facing surfaces have a violet tint. In Survival, mined blocks enter your inventory. Choose a hotbar slot with $first…$last. A block occupies all four dimensions, so it can intersect several views at once.", "通过视图 1 瞄准，用 $break 破坏，用 $place 放置。朝向 W 的表面带有紫色。生存模式中，挖下的方块会进入物品栏。用 $first…$last 选择快捷栏位置。方块占据四维空间，因此可能同时与多个视图相交。"],
        keys: &[("$break", Action::Break), ("$place", Action::Place), ("$first", Action::Hotbar1), ("$last", Action::Hotbar9)],
        goal: Goal::Check(|s, _, _, _| s.broken > 0 && s.placed > 0),
    },
    Step {
        title: ["Inventory and dropped items", "物品栏与掉落物"],
        text: ["Open the inventory with $inventory. In Creative, choose any available block; in Survival, left-click moves stacks, right-click splits or places one, and Shift-click transfers quickly. $drop drops one item from the selected slot. Walk close to a dropped item to pick it up; it must also be close along W.", "按 $inventory 打开物品栏。创造模式中可以选择所有可用方块；生存模式中，左键移动整组，右键分半或放置一个，Shift＋点击快速转移。按 $drop 丢出当前快捷栏中的一个物品。靠近掉落物即可拾取；W 方向上的距离也必须足够近。"],
        keys: &[("$inventory", Action::Inventory), ("$drop", Action::Drop)], goal: Goal::Continue,
    },
    Step {
        title: ["Creatures", "四维生物"],
        text: ["The bouncing glooms are hyperspheres: their visible spheres shrink as they leave a slice. With 4D glasses ($glasses), nearby out-of-slice creatures have outlines: red on the ana side, blue on the kata side. Compare the other views to understand where the complete creature is.", "跳动的生物是四维球：离开截面时，可见的球形截面会缩小。开启四维眼镜（$glasses）后，附近截面外的生物会显示轮廓：ana 侧是红色，kata 侧是蓝色。结合其他视图，可以判断完整生物所在的位置。"],
        keys: &[("$glasses", Action::ToggleGlasses)], goal: Goal::Continue,
    },
    Step {
        title: ["You're ready", "准备好出发了"],
        text: ["Explore a place, step out of its slice, and find it again using the compass and other views. You can replay this tutorial from Esc → Tutorial and rebind controls under Esc → Controls. Try building something that looks different in every slice!", "探索一个地方，走出它所在的截面，再利用罗盘和其他视图找到它。可以通过 Esc → 教程重播教程，在 Esc → 控制设置中修改按键。试着建造一个在每个截面中看起来都不同的作品吧！"],
        keys: &[], goal: Goal::Continue,
    },
];

pub struct Tutorial {
    step: usize,
    elapsed: f32,
    baseline: Stats,
    ghosts_at_start: bool,
    views_at_start: u32,
    /// Seconds left of the "done" flash before moving to the next step.
    completed: Option<f32>,
}

/// What the panel should show this frame.
pub struct View {
    pub index: usize,
    pub count: usize,
    pub title: &'static str,
    pub text: String,
    pub needs_enter: bool,
    pub completed: bool,
    pub diagram: Option<Diagram>,
    pub animation_time: f32,
}

impl Tutorial {
    pub fn new(game: &Game, settings: &Settings) -> Self {
        Self { step: 0, elapsed: 0.0, baseline: game.stats, ghosts_at_start: settings.ghosts, views_at_start: settings.active_view_count(), completed: None }
    }

    /// Jump to step `n` (used by the headless `--tutorial N` preview).
    pub fn skip_to(&mut self, n: usize, game: &Game, settings: &Settings) {
        self.step = n.min(STEPS.len() - 1);
        self.begin_step(game, settings);
    }

    fn begin_step(&mut self, game: &Game, settings: &Settings) {
        self.elapsed = 0.0;
        self.baseline = game.stats;
        self.ghosts_at_start = settings.ghosts;
        self.views_at_start = settings.active_view_count();
        self.completed = None;
    }

    /// Advance. Returns true when the tutorial has finished.
    pub fn update(&mut self, dt: f32, game: &Game, settings: &Settings, enter: bool) -> bool {
        self.elapsed += dt.clamp(0.0, 0.25);
        let step = &STEPS[self.step];
        match self.completed {
            Some(t) if t - dt > 0.0 => self.completed = Some(t - dt),
            Some(_) => return self.next(game, settings),
            None => match step.goal {
                Goal::Continue if enter => return self.next(game, settings),
                Goal::Continue => {}
                Goal::Check(done) => {
                    if done(&diff(&game.stats, &self.baseline), game, settings, self) {
                        self.completed = Some(1.2);
                    }
                }
            },
        }
        false
    }

    fn next(&mut self, game: &Game, settings: &Settings) -> bool {
        self.step += 1;
        if self.step >= STEPS.len() {
            return true;
        }
        self.begin_step(game, settings);
        false
    }

    pub fn is_conceptual(&self) -> bool { self.step < 5 }

    pub fn view(&self, bindings: &Bindings, input: &InputState, language: Language) -> View {
        let step = &STEPS[self.step];
        let key = |a: Action| {
            let [p, s] = bindings.get(a);
            let l = |b: crate::input::Binding| i18n::text(language, &b.label(&input.layout_names)).to_string();
            match (p.is_bound(), s.is_bound()) {
                (true, true) => format!("[{}]/[{}]", l(p), l(s)),
                (true, false) => format!("[{}]", l(p)),
                (false, true) => format!("[{}]", l(s)),
                _ => format!("[{}]", i18n::text(language, "unbound")),
            }
        };
        let lang = if language == Language::Chinese { 1 } else { 0 };
        let mut text = step.text[lang].to_string();
        for &(placeholder, action) in step.keys { text = text.replace(placeholder, &key(action)); }
        View {
            index: self.step,
            count: STEPS.len(),
            title: step.title[lang],
            text,
            needs_enter: matches!(step.goal, Goal::Continue),
            completed: self.completed.is_some(),
            diagram: [Diagram::Dimensions, Diagram::SquareFaces, Diagram::CubicCells, Diagram::ProjectionSlice, Diagram::RotatingSlice].get(self.step).copied(),
            animation_time: self.elapsed,
        }
    }
}

fn diff(now: &Stats, base: &Stats) -> Stats {
    Stats {
        looked: now.looked - base.looked,
        moved_slice: now.moved_slice - base.moved_slice,
        moved_ana: now.moved_ana - base.moved_ana,
        turned_forward_ana: now.turned_forward_ana - base.turned_forward_ana,
        turned_right_ana: now.turned_right_ana - base.turned_right_ana,
        snaps: now.snaps - base.snaps,
        flips: now.flips - base.flips,
        broken: now.broken - base.broken,
        placed: now.placed - base.placed,
        creatures_hit: now.creatures_hit - base.creatures_hit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameMode;

    #[test]
    fn steps_complete_from_stats_and_enter() {
        let mut game = Game::new(None, 3, GameMode::Creative);
        let settings = Settings::default();
        let mut t = Tutorial::new(&game, &settings);
        assert!(t.view(&Bindings::default(), &InputState::default(), Language::English).needs_enter);
        while STEPS[t.step].title[0] != "Look around" {
            assert!(!t.update(0.016, &game, &settings, true));
        }
        t.update(0.016, &game, &settings, false);
        assert!(t.completed.is_none());
        game.stats.looked += 2.0;
        t.update(0.016, &game, &settings, false);
        assert!(t.completed.is_some());
        t.update(2.0, &game, &settings, false);
        assert_eq!(STEPS[t.step].title[0], "Walk");
    }

    #[test]
    fn text_uses_current_bindings() {
        let game = Game::new(None, 3, GameMode::Creative);
        let mut t = Tutorial::new(&game, &Settings::default());
        t.step = STEPS.iter().position(|s| s.title[0] == "Step into W").unwrap();
        let mut b = Bindings::default();
        b.set(Action::MoveAna, 0, crate::input::Binding::Key(winit::keyboard::KeyCode::KeyO));
        assert!(t.view(&b, &InputState::default(), Language::English).text.contains("[O]"));
        assert!(t.view(&b, &InputState::default(), Language::Chinese).text.contains("[O]"));
    }

    #[test]
    fn every_step_is_bilingual_and_resolves_all_key_placeholders() {
        let game = Game::new(None, 3, GameMode::Creative);
        let mut tutorial = Tutorial::new(&game, &Settings::default());
        for index in 0..STEPS.len() {
            tutorial.step = index;
            let en = tutorial.view(&Bindings::default(), &InputState::default(), Language::English);
            let zh = tutorial.view(&Bindings::default(), &InputState::default(), Language::Chinese);
            assert!(!en.text.contains('$') && !zh.text.contains('$'), "unresolved binding at step {index}");
            assert_ne!(en.title, zh.title);
            assert_ne!(en.text, zh.text);
            assert!(zh.text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)));
        }
    }

    #[test]
    fn opening_diagrams_animate_but_wait_for_explicit_enter() {
        let game = Game::new(None, 3, GameMode::Creative);
        let settings = Settings::default();
        let mut tutorial = Tutorial::new(&game, &settings);
        for index in 0..5 {
            tutorial.skip_to(index,&game,&settings);
            assert!(tutorial.is_conceptual());
            let before = tutorial.view(&Bindings::default(), &InputState::default(), Language::English);
            assert!(before.diagram.is_some() && before.needs_enter);
            for _ in 0..20 { assert!(!tutorial.update(0.1,&game,&settings,false)); }
            assert_eq!(tutorial.step,index);
            assert!(tutorial.elapsed>1.9);
            assert!(!tutorial.update(0.016,&game,&settings,true));
            assert_eq!(tutorial.step,index+1);
        }
        assert!(!tutorial.is_conceptual());
    }

    #[test]
    fn changing_layout_completes_comparison_but_existing_layout_does_not() {
        let game = Game::new(None, 3, GameMode::Creative);
        let mut settings = Settings::default();
        let mut tutorial = Tutorial::new(&game, &settings);
        tutorial.skip_to(STEPS.iter().position(|s| s.title[0] == "Compare the views").unwrap(), &game, &settings);
        tutorial.update(0.016, &game, &settings, false);
        assert!(tutorial.completed.is_none());
        settings.view_count = 9;
        tutorial.update(0.016, &game, &settings, false);
        assert!(tutorial.completed.is_some());
    }
}
