//! Guided first-time tutorial. Each step has an objective that completes when the player
//! actually does it (tracked via `game::Stats`), or waits for Enter on purely explanatory
//! steps. Key names come from the current bindings, so rebinding updates the text.

use crate::game::{Game, Stats};
use crate::input::{Action, Bindings, InputState};
use crate::settings::Settings;

enum Goal {
    /// Press Enter to continue.
    Continue,
    /// Objective met, given the stats gained since the step started.
    Check(fn(&Stats, &Game, &Settings, &Tutorial) -> bool),
}

struct Step {
    title: &'static str,
    text: fn(&dyn Fn(Action) -> String) -> String,
    goal: Goal,
}

const STEPS: &[Step] = &[
    Step {
        title: "Welcome to the 4th dimension",
        text: |_| {
            "This world has four spatial dimensions: X, Y (up), Z and W. Your screen can only show 3D, \
             so you see a 3D slice of the 4D world through your body, the way a flat 2D creature would \
             see only a thin line of our world."
                .into()
        },
        goal: Goal::Continue,
    },
    Step {
        title: "Look around",
        text: |_| "Move the mouse to look around.".into(),
        goal: Goal::Check(|s, _, _, _| s.looked > 1.5),
    },
    Step {
        title: "Walk",
        text: |k| {
            format!(
                "Walk with {} {} {} {}, jump with {}. This is ordinary movement inside your slice.",
                k(Action::MoveForward),
                k(Action::MoveLeft),
                k(Action::MoveBack),
                k(Action::MoveRight),
                k(Action::Jump)
            )
        },
        goal: Goal::Check(|s, _, _, _| s.moved_slice > 4.0),
    },
    Step {
        title: "Step into W",
        text: |k| {
            format!(
                "{} and {} move you along the 4th direction (\"ana\" and \"kata\"), perpendicular to \
                 everything you can see. The terrain changes because you are now looking at a \
                 neighbouring slice.",
                k(Action::MoveAna),
                k(Action::MoveKata)
            )
        },
        goal: Goal::Check(|s, _, _, _| s.moved_ana > 3.0),
    },
    Step {
        title: "Turn into W",
        text: |k| {
            format!(
                "Scroll the mouse wheel ({} / {}) to turn your view toward the 4th dimension. Blocks \
                 seem to change shape: they are tesseracts, and your slice now cuts them at an angle.",
                k(Action::RotateAnaPos),
                k(Action::RotateAnaNeg)
            )
        },
        goal: Goal::Check(|s, _, _, _| s.turned_forward_ana > 40f32.to_radians()),
    },
    Step {
        title: "Snap back",
        text: |k| format!("Odd angles get confusing. Press {} to snap the slice back onto the axes.", k(Action::SnapSlice)),
        goal: Goal::Check(|s, g, _, _| s.snaps > 0 && !g.orient.is_animating() && g.orient.misalignment() < 0.02),
    },
    Step {
        title: "Flip 90°",
        text: |k| {
            format!(
                "Press {} to swing a full 90°. What was \"forward\" now points along W, and the old \
                 forward direction is now hidden from view. Four flips bring you back where you started.",
                k(Action::FlipAna)
            )
        },
        goal: Goal::Check(|s, g, _, _| s.flips > 0 && !g.orient.is_animating()),
    },
    Step {
        title: "Your 4D instruments",
        text: |_| {
            "Top right is the 4D compass. Its arrows show where world X, Z and W point on screen. \
             The bars below show how much of each axis points along ana, out of your slice. \
             Bottom right is the side view: the slice standing perpendicular to yours, with ana \
             pointing right."
                .into()
        },
        goal: Goal::Continue,
    },
    Step {
        title: "Ghost layers",
        text: |k| {
            format!(
                "Press {} to toggle ghost layers. Red tints show blocks just ana of your slice and blue \
                 tints show blocks just kata, so you can see what's hiding next to you in W.",
                k(Action::ToggleGhosts)
            )
        },
        goal: Goal::Check(|_, _, s, t| s.ghosts != t.ghosts_at_start),
    },
    Step {
        title: "Rotate the slice freely",
        text: |k| {
            format!(
                "Hold {} and move the mouse. Sideways swings your right hand into W, and up/down tilts \
                 forward into W. Snap with {} whenever you get lost.",
                k(Action::SliceModifier),
                k(Action::SnapSlice)
            )
        },
        goal: Goal::Check(|s, _, _, _| s.turned_right_ana > 30f32.to_radians()),
    },
    Step {
        title: "Build",
        text: |k| {
            format!(
                "Break a block with {} and place one with {}. Faces that point along W have a violet \
                 tint. In Survival, broken blocks go into your hotbar; choose a slot with {}…{}.",
                k(Action::Break),
                k(Action::Place),
                k(Action::Hotbar1),
                k(Action::Hotbar9)
            )
        },
        goal: Goal::Check(|s, _, _, _| s.broken > 0 && s.placed > 0),
    },
    Step {
        title: "Creatures",
        text: |k| {
            format!(
                "The bouncing glooms are hyperspheres. When one drifts out of your slice it shrinks \
                 and disappears. With 4D glasses ({}) you still see its outline: red when it's on the \
                 ana side, blue on the kata side.",
                k(Action::ToggleGlasses)
            )
        },
        goal: Goal::Continue,
    },
    Step {
        title: "You're ready",
        text: |_| {
            "Go explore! You can replay this tutorial from the pause menu (Esc). \
             Any key can be changed under Esc → Controls."
                .into()
        },
        goal: Goal::Continue,
    },
];

pub struct Tutorial {
    step: usize,
    baseline: Stats,
    ghosts_at_start: bool,
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
}

impl Tutorial {
    pub fn new(game: &Game, settings: &Settings) -> Self {
        Self { step: 0, baseline: game.stats, ghosts_at_start: settings.ghosts, completed: None }
    }

    /// Jump to step `n` (used by the headless `--tutorial N` preview).
    pub fn skip_to(&mut self, n: usize, game: &Game, settings: &Settings) {
        self.step = n.min(STEPS.len() - 1);
        self.begin_step(game, settings);
    }

    fn begin_step(&mut self, game: &Game, settings: &Settings) {
        self.baseline = game.stats;
        self.ghosts_at_start = settings.ghosts;
        self.completed = None;
    }

    /// Advance. Returns true when the tutorial has finished.
    pub fn update(&mut self, dt: f32, game: &Game, settings: &Settings, enter: bool) -> bool {
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

    pub fn view(&self, bindings: &Bindings, input: &InputState) -> View {
        let step = &STEPS[self.step];
        let key = |a: Action| {
            let [p, s] = bindings.get(a);
            let l = |b: crate::input::Binding| b.label(&input.layout_names);
            match (p.is_bound(), s.is_bound()) {
                (true, true) => format!("[{}]/[{}]", l(p), l(s)),
                (true, false) => format!("[{}]", l(p)),
                (false, true) => format!("[{}]", l(s)),
                _ => "[unbound]".into(),
            }
        };
        View {
            index: self.step,
            count: STEPS.len(),
            title: step.title,
            text: (step.text)(&key),
            needs_enter: matches!(step.goal, Goal::Continue),
            completed: self.completed.is_some(),
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
        assert!(t.view(&Bindings::default(), &InputState::default()).needs_enter);
        assert!(!t.update(0.016, &game, &settings, true)); // welcome → look
        assert_eq!(t.step, 1);
        t.update(0.016, &game, &settings, false);
        assert!(t.completed.is_none());
        game.stats.looked += 2.0;
        t.update(0.016, &game, &settings, false);
        assert!(t.completed.is_some());
        t.update(2.0, &game, &settings, false);
        assert_eq!(t.step, 2);
    }

    #[test]
    fn text_uses_current_bindings() {
        let game = Game::new(None, 3, GameMode::Creative);
        let mut t = Tutorial::new(&game, &Settings::default());
        t.step = 3;
        let mut b = Bindings::default();
        b.set(Action::MoveAna, 0, crate::input::Binding::Key(winit::keyboard::KeyCode::KeyO));
        assert!(t.view(&b, &InputState::default()).text.contains("[O]"));
    }
}
