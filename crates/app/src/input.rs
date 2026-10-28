//! Action-based input with Minecraft-style rebindable controls.
//!
//! Game code asks about `Action`s; a user-editable `Bindings` table maps physical inputs
//! (keys by physical position, mouse buttons, wheel directions) to actions.

use serde::de::IntoDeserializer;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

macro_rules! actions {
    ($( $variant:ident => ($id:literal, $name:literal, $cat:ident, $primary:expr, $secondary:expr) ),* $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
        pub enum Action { $($variant),* }

        impl Action {
            pub const ALL: &'static [Action] = &[$(Action::$variant),*];

            /// Stable id used in `controls.toml`.
            pub fn id(self) -> &'static str {
                match self { $(Action::$variant => $id),* }
            }

            pub fn name(self) -> &'static str {
                match self { $(Action::$variant => $name),* }
            }

            pub fn category(self) -> Category {
                match self { $(Action::$variant => Category::$cat),* }
            }


            pub fn default_bindings(self) -> [Binding; 2] {
                match self { $(Action::$variant => [$primary, $secondary]),* }
            }
        }
    };
}

use Binding::{Key as K, Mouse as M, Unbound as U, WheelDown as WD, WheelUp as WU};
use KeyCode::*;
use MouseBtn::*;

actions! {
    MoveForward => ("move.forward", "Forward", Movement, K(KeyW), U),
    MoveBack => ("move.back", "Back", Movement, K(KeyS), U),
    MoveLeft => ("move.left", "Strafe Left", Movement, K(KeyA), U),
    MoveRight => ("move.right", "Strafe Right", Movement, K(KeyD), U),
    MoveKata => ("move.kata", "Move Kata (−ana)", Movement, K(KeyQ), U),
    MoveAna => ("move.ana", "Move Ana (+ana)", Movement, K(KeyE), U),
    Jump => ("move.jump", "Jump / Fly Up", Movement, K(Space), U),
    Sneak => ("move.sneak", "Sneak / Fly Down", Movement, K(ShiftLeft), U),
    Sprint => ("move.sprint", "Sprint", Movement, K(ControlLeft), U),
    ToggleFly => ("move.fly", "Toggle Flying (creative)", Movement, K(KeyF), U),

    RotateAnaPos => ("4d.rotate_fwd_ana_pos", "Turn Forward → Ana", FourD, WU, K(BracketRight)),
    RotateAnaNeg => ("4d.rotate_fwd_ana_neg", "Turn Forward → Kata", FourD, WD, K(BracketLeft)),
    RotateRightAnaPos => ("4d.rotate_right_ana_pos", "Swing Right → Ana", FourD, U, K(Quote)),
    RotateRightAnaNeg => ("4d.rotate_right_ana_neg", "Swing Right → Kata", FourD, U, K(Semicolon)),
    SliceModifier => ("4d.modifier", "Mouse Rotates Slice (hold)", FourD, K(AltLeft), U),
    SnapSlice => ("4d.snap", "Snap Slice to Axes", FourD, K(KeyR), U),
    FlipAna => ("4d.flip", "Flip 90° into W", FourD, K(Tab), U),
    ToggleGhosts => ("4d.ghosts", "Toggle Ghost Layers", FourD, K(KeyG), U),
    ToggleInset => ("4d.inset", "Toggle Side View", FourD, K(KeyV), U),
    ToggleGlasses => ("4d.glasses", "Toggle 4D Glasses", FourD, K(KeyB), U),

    Break => ("game.break", "Break Block / Attack", Gameplay, M(Left), U),
    Place => ("game.place", "Place Block", Gameplay, M(Right), U),
    PickBlock => ("game.pick", "Pick Block", Gameplay, M(Middle), U),
    HotbarNext => ("game.hotbar_next", "Next Hotbar Slot", Gameplay, U, U),
    HotbarPrev => ("game.hotbar_prev", "Previous Hotbar Slot", Gameplay, U, U),
    Hotbar1 => ("game.hotbar1", "Hotbar Slot 1", Gameplay, K(Digit1), U),
    Hotbar2 => ("game.hotbar2", "Hotbar Slot 2", Gameplay, K(Digit2), U),
    Hotbar3 => ("game.hotbar3", "Hotbar Slot 3", Gameplay, K(Digit3), U),
    Hotbar4 => ("game.hotbar4", "Hotbar Slot 4", Gameplay, K(Digit4), U),
    Hotbar5 => ("game.hotbar5", "Hotbar Slot 5", Gameplay, K(Digit5), U),
    Hotbar6 => ("game.hotbar6", "Hotbar Slot 6", Gameplay, K(Digit6), U),
    Hotbar7 => ("game.hotbar7", "Hotbar Slot 7", Gameplay, K(Digit7), U),
    Hotbar8 => ("game.hotbar8", "Hotbar Slot 8", Gameplay, K(Digit8), U),
    Hotbar9 => ("game.hotbar9", "Hotbar Slot 9", Gameplay, K(Digit9), U),

    ToggleHud => ("ui.hud", "Toggle HUD", Interface, K(F1), U),
    ToggleDebug => ("ui.debug", "Debug Overlay", Interface, K(F3), U),
    Screenshot => ("ui.screenshot", "Screenshot", Interface, K(F2), U),
}

impl Action {
    pub fn hotbar_index(self) -> Option<usize> {
        use Action::*;
        [Hotbar1, Hotbar2, Hotbar3, Hotbar4, Hotbar5, Hotbar6, Hotbar7, Hotbar8, Hotbar9]
            .iter()
            .position(|&a| a == self)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Movement,
    FourD,
    Gameplay,
    Interface,
}

impl Category {
    pub const ALL: [Category; 4] = [Category::Movement, Category::FourD, Category::Gameplay, Category::Interface];

    pub fn name(self) -> &'static str {
        match self {
            Category::Movement => "Movement",
            Category::FourD => "4D Navigation",
            Category::Gameplay => "Gameplay",
            Category::Interface => "Interface",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MouseBtn {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Other(u16),
}

impl From<MouseButton> for MouseBtn {
    fn from(b: MouseButton) -> Self {
        match b {
            MouseButton::Left => Left,
            MouseButton::Right => Right,
            MouseButton::Middle => Middle,
            MouseButton::Back => Back,
            MouseButton::Forward => Forward,
            MouseButton::Other(n) => Other(n),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Binding {
    Key(KeyCode),
    Mouse(MouseBtn),
    WheelUp,
    WheelDown,
    Unbound,
}

impl fmt::Display for Binding {
    /// Serialized form used in `controls.toml`: `key:KeyW`, `mouse:Left`, `wheel:up`, `none`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Binding::Key(k) => write!(f, "key:{k:?}"),
            Binding::Mouse(Other(n)) => write!(f, "mouse:{n}"),
            Binding::Mouse(b) => write!(f, "mouse:{b:?}"),
            Binding::WheelUp => write!(f, "wheel:up"),
            Binding::WheelDown => write!(f, "wheel:down"),
            Binding::Unbound => write!(f, "none"),
        }
    }
}

impl FromStr for Binding {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let bad = || format!("unknown binding `{s}`");
        Ok(match s.split_once(':') {
            None if s == "none" => Binding::Unbound,
            Some(("key", k)) => {
                let de: serde::de::value::StrDeserializer<serde::de::value::Error> = k.into_deserializer();
                Binding::Key(KeyCode::deserialize(de).map_err(|_| bad())?)
            }
            Some(("mouse", b)) => Binding::Mouse(match b {
                "Left" => Left,
                "Right" => Right,
                "Middle" => Middle,
                "Back" => Back,
                "Forward" => Forward,
                n => Other(n.parse().map_err(|_| bad())?),
            }),
            Some(("wheel", "up")) => Binding::WheelUp,
            Some(("wheel", "down")) => Binding::WheelDown,
            _ => return Err(bad()),
        })
    }
}

impl Binding {
    pub fn is_bound(self) -> bool {
        self != Binding::Unbound
    }

    /// Human-readable label. Keys use the character they produced on this keyboard layout
    /// when known (`layout_names`), else a name derived from the physical key.
    pub fn label(self, layout_names: &HashMap<KeyCode, String>) -> String {
        match self {
            Binding::Key(k) => layout_names.get(&k).cloned().unwrap_or_else(|| key_name(k)),
            Binding::Mouse(Left) => "Left Button".into(),
            Binding::Mouse(Right) => "Right Button".into(),
            Binding::Mouse(Middle) => "Middle Button".into(),
            Binding::Mouse(Back) => "Mouse Back".into(),
            Binding::Mouse(Forward) => "Mouse Forward".into(),
            Binding::Mouse(Other(n)) => format!("Mouse {n}"),
            Binding::WheelUp => "Wheel Up".into(),
            Binding::WheelDown => "Wheel Down".into(),
            Binding::Unbound => "—".into(),
        }
    }
}

pub fn key_name(k: KeyCode) -> String {
    let dbg = format!("{k:?}");
    if let Some(rest) = dbg.strip_prefix("Key") {
        return rest.to_string();
    }
    if let Some(rest) = dbg.strip_prefix("Digit") {
        return rest.to_string();
    }
    for (suffix, side) in [("Left", "Left "), ("Right", "Right ")] {
        if let Some(base) = dbg.strip_suffix(suffix)
            && !base.is_empty() && !base.starts_with("Arrow") {
                return format!("{side}{}", match base {
                    "Control" => "Ctrl",
                    "Super" => "Super",
                    b => b,
                });
            }
    }
    match k {
        Space => "Space".into(),
        BracketLeft => "[".into(),
        BracketRight => "]".into(),
        Semicolon => ";".into(),
        Quote => "'".into(),
        Comma => ",".into(),
        Period => ".".into(),
        Slash => "/".into(),
        Backslash => "\\".into(),
        Minus => "-".into(),
        Equal => "=".into(),
        Backquote => "`".into(),
        _ => dbg,
    }
}

/// The binding table, persisted as `controls.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Bindings {
    map: HashMap<Action, [Binding; 2]>,
}

impl Default for Bindings {
    fn default() -> Self {
        Self { map: Action::ALL.iter().map(|&a| (a, a.default_bindings())).collect() }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct BindingsFile {
    #[serde(default)]
    bindings: BTreeMap<String, Vec<String>>,
}

impl Bindings {
    pub fn get(&self, a: Action) -> [Binding; 2] {
        self.map.get(&a).copied().unwrap_or_else(|| a.default_bindings())
    }

    pub fn set(&mut self, a: Action, slot: usize, b: Binding) {
        let mut cur = self.get(a);
        cur[slot] = b;
        self.map.insert(a, cur);
    }

    pub fn reset(&mut self, a: Action) {
        self.map.insert(a, a.default_bindings());
    }

    pub fn is_default(&self, a: Action) -> bool {
        self.get(a) == a.default_bindings()
    }

    /// Other actions sharing any of `a`'s bindings. Conflicts are allowed but shown in red.
    pub fn conflicts(&self, a: Action) -> Vec<Action> {
        let mine = self.get(a);
        Action::ALL
            .iter()
            .copied()
            .filter(|&o| o != a)
            .filter(|&o| self.get(o).iter().any(|b| b.is_bound() && mine.contains(b)))
            .collect()
    }

    pub fn to_toml(&self) -> String {
        let mut file = BindingsFile::default();
        for &a in Action::ALL {
            let [p, s] = self.get(a);
            file.bindings.insert(a.id().to_string(), vec![p.to_string(), s.to_string()]);
        }
        toml::to_string_pretty(&file).unwrap_or_default()
    }

    /// Parse `controls.toml`. Unknown actions are ignored and missing ones keep defaults,
    /// so files from older or newer versions still load.
    pub fn from_toml(s: &str) -> Self {
        let mut out = Self::default();
        let Ok(file) = toml::from_str::<BindingsFile>(s) else {
            log::warn!("controls.toml is invalid; using default controls");
            return out;
        };
        let by_id: HashMap<&str, Action> = Action::ALL.iter().map(|&a| (a.id(), a)).collect();
        for (id, list) in file.bindings {
            let Some(&a) = by_id.get(id.as_str()) else { continue };
            let mut b = a.default_bindings();
            for (i, s) in list.iter().take(2).enumerate() {
                match s.parse() {
                    Ok(v) => b[i] = v,
                    Err(e) => log::warn!("controls.toml: {id}: {e}"),
                }
            }
            out.map.insert(a, b);
        }
        out
    }
}

/// Per-frame input state.
#[derive(Default)]
pub struct InputState {
    down: HashSet<Binding>,
    pressed: HashSet<Binding>,
    wheel_up: u32,
    wheel_down: u32,
    wheel_accum: f32,
    pub mouse_delta: (f64, f64),
    /// Characters produced by physical keys on the current layout, for labels.
    pub layout_names: HashMap<KeyCode, String>,
}

impl InputState {
    pub fn press(&mut self, b: Binding) {
        if self.down.insert(b) {
            self.pressed.insert(b);
        }
    }

    pub fn release(&mut self, b: Binding) {
        self.down.remove(&b);
    }

    /// Wheel motion in "lines" (positive = up). Pixel deltas should be pre-divided.
    pub fn wheel(&mut self, lines: f32) {
        self.wheel_accum += lines;
        while self.wheel_accum >= 1.0 {
            self.wheel_accum -= 1.0;
            self.wheel_up += 1;
        }
        while self.wheel_accum <= -1.0 {
            self.wheel_accum += 1.0;
            self.wheel_down += 1;
        }
    }

    pub fn release_all(&mut self) {
        self.down.clear();
        self.pressed.clear();
    }

    /// Call at the end of every frame.
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.wheel_up = 0;
        self.wheel_down = 0;
        self.mouse_delta = (0.0, 0.0);
    }

    fn wheel_count(&self, b: Binding) -> u32 {
        match b {
            Binding::WheelUp => self.wheel_up,
            Binding::WheelDown => self.wheel_down,
            _ => 0,
        }
    }

    pub fn is_down(&self, bindings: &Bindings, a: Action) -> bool {
        bindings.get(a).iter().any(|b| b.is_bound() && self.down.contains(b))
    }

    /// Number of times `a` was triggered this frame (key/button presses plus wheel steps).
    pub fn presses(&self, bindings: &Bindings, a: Action) -> u32 {
        bindings
            .get(a)
            .iter()
            .filter(|b| b.is_bound())
            .map(|&b| self.wheel_count(b) + self.pressed.contains(&b) as u32)
            .sum()
    }

    pub fn just_pressed(&self, bindings: &Bindings, a: Action) -> bool {
        self.presses(bindings, a) > 0
    }

    /// Wheel steps bound to `a` this frame (for analog actions like slice rotation).
    pub fn wheel_steps(&self, bindings: &Bindings, a: Action) -> u32 {
        bindings.get(a).iter().map(|&b| self.wheel_count(b)).sum()
    }

    /// Is `a` held via a non-wheel binding?
    pub fn held(&self, bindings: &Bindings, a: Action) -> bool {
        bindings
            .get(a)
            .iter()
            .any(|&b| !matches!(b, Binding::WheelUp | Binding::WheelDown | Binding::Unbound) && self.down.contains(&b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let ids: HashSet<_> = Action::ALL.iter().map(|a| a.id()).collect();
        assert_eq!(ids.len(), Action::ALL.len());
    }

    #[test]
    fn binding_strings_roundtrip() {
        for b in [K(KeyW), K(AltLeft), M(Left), M(Other(7)), WU, WD, U] {
            assert_eq!(b.to_string().parse::<Binding>().unwrap(), b);
        }
        assert!("key:NotAKey".parse::<Binding>().is_err());
    }

    #[test]
    fn toml_roundtrip_and_tolerance() {
        let mut b = Bindings::default();
        b.set(Action::Jump, 0, K(KeyJ));
        b.set(Action::SnapSlice, 1, M(Back));
        let parsed = Bindings::from_toml(&b.to_toml());
        assert_eq!(parsed, b);

        let partial = "[bindings]\n\"move.jump\" = [\"key:KeyK\"]\n\"no.such.action\" = [\"key:KeyZ\"]\n";
        let p = Bindings::from_toml(partial);
        assert_eq!(p.get(Action::Jump), [K(KeyK), U]);
        assert_eq!(p.get(Action::MoveForward), Action::MoveForward.default_bindings());
    }

    #[test]
    fn defaults_have_no_conflicts() {
        let b = Bindings::default();
        for &a in Action::ALL {
            assert!(b.conflicts(a).is_empty(), "{a:?} conflicts with {:?}", b.conflicts(a));
        }
        let mut b = b;
        b.set(Action::Jump, 0, K(KeyW));
        assert_eq!(b.conflicts(Action::Jump), vec![Action::MoveForward]);
    }

    #[test]
    fn wheel_and_key_presses() {
        let b = Bindings::default();
        let mut s = InputState::default();
        s.wheel(2.0);
        assert_eq!(s.wheel_steps(&b, Action::RotateAnaPos), 2);
        s.press(K(KeyR));
        assert!(s.just_pressed(&b, Action::SnapSlice));
        s.end_frame();
        assert!(!s.just_pressed(&b, Action::SnapSlice));
        assert!(s.is_down(&b, Action::SnapSlice));
    }
}
