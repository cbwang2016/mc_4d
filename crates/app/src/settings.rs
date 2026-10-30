//! User settings and file locations.

use crate::input::Bindings;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="lowercase")]
pub enum Difficulty { Peaceful, Easy, #[default] Normal, Hard }
impl Difficulty {
    pub fn name(self)->&'static str {match self {Self::Peaceful=>"Peaceful",Self::Easy=>"Easy",Self::Normal=>"Normal",Self::Hard=>"Hard"}}
    pub fn next(self)->Self {match self {Self::Peaceful=>Self::Easy,Self::Easy=>Self::Normal,Self::Normal=>Self::Hard,Self::Hard=>Self::Peaceful}}
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub language: crate::i18n::Language,
    /// Empty selects the bundled classic pack; otherwise a folder containing terrain.png.
    pub texture_pack: String,
    pub difficulty: Difficulty,
    pub fancy_graphics: bool,
    pub view_bobbing: bool,
    pub smooth_lighting: bool,
    pub anaglyph: bool,
    pub limit_framerate: bool,
    /// Borderless fullscreen on the current monitor (F11).
    pub fullscreen: bool,
    pub gui_scale: u32,
    pub chunk_culling: bool,
    pub music_volume: f32,
    pub sound_volume: f32,
    pub fov: f32,
    /// In chunks (rays travel at most this far, in-slice).
    pub render_distance: u32,
    /// Internal render scale; 1.0 (native) by default and never changed automatically.
    pub render_scale: f32,
    pub mouse_sensitivity: f32,
    pub invert_y: bool,
    /// Degrees per wheel step for slice rotation.
    pub wheel_step_deg: f32,
    /// Degrees per second for keyboard slice rotation.
    pub key_rotate_speed: f32,
    /// Multiplier for slice rotation with modifier + mouse.
    pub slice_mouse_sensitivity: f32,
    pub auto_snap: bool,
    pub ghosts: bool,
    pub ghost_offset: f32,
    pub inset: bool,
    /// Number of simultaneous slices when the multi-view display is enabled.
    pub view_count: u32,
    pub glasses: bool,
    pub shadows: bool,
    pub edges: bool,
    pub show_hints: bool,
    /// Set once the tutorial has been finished or skipped; it starts automatically until then.
    pub tutorial_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: crate::i18n::Language::English,
            texture_pack: String::new(),
            difficulty: Difficulty::Normal,
            fancy_graphics: true,
            view_bobbing: true,
            smooth_lighting: true,
            anaglyph: false,
            limit_framerate: false,
            fullscreen: false,
            gui_scale: 0,
            chunk_culling: true,
            music_volume: 1.0,
            sound_volume: 1.0,
            fov: 75.0,
            render_distance: 5,
            render_scale: 1.0,
            mouse_sensitivity: 1.0,
            invert_y: false,
            wheel_step_deg: 6.0,
            key_rotate_speed: 60.0,
            slice_mouse_sensitivity: 1.0,
            auto_snap: false,
            ghosts: false,
            ghost_offset: 1.0,
            inset: true,
            view_count: 2,
            glasses: true,
            shadows: false,
            edges: false,
            show_hints: false,
            tutorial_done: false,
        }
    }
}

impl Settings {
    pub fn active_view_count(&self) -> u32 {
        if self.inset { mc4d_core::views::valid_view_count(self.view_count) } else { 1 }
    }
}

pub struct Paths {
    pub config: PathBuf,
    pub saves: PathBuf,
    pub screenshots: PathBuf,
}

impl Paths {
    /// Beta-style texturepacks directory beside the saves.
    pub fn texture_packs(&self) -> PathBuf { self.saves.parent().map_or_else(|| self.config.join("texturepacks"), |p| p.join("texturepacks")) }
}

impl Paths {
    pub fn new() -> Self {
        match directories::ProjectDirs::from("", "", "mc4d") {
            Some(d) => Self {
                config: d.config_dir().to_path_buf(),
                saves: d.data_dir().join("saves"),
                screenshots: d.data_dir().join("screenshots"),
            },
            None => Self {
                config: PathBuf::from("mc4d-data"),
                saves: PathBuf::from("mc4d-data/saves"),
                screenshots: PathBuf::from("mc4d-data/screenshots"),
            },
        }
    }

    /// Keep everything under one directory (used by tests and `--data-dir`).
    pub fn under(root: PathBuf) -> Self {
        Self { config: root.clone(), saves: root.join("saves"), screenshots: root.join("screenshots") }
    }

    fn controls_file(&self) -> PathBuf {
        self.config.join("controls.toml")
    }

    fn settings_file(&self) -> PathBuf {
        self.config.join("settings.toml")
    }

    pub fn load_bindings(&self) -> Bindings {
        std::fs::read_to_string(self.controls_file()).map(|s| Bindings::from_toml(&s)).unwrap_or_default()
    }

    pub fn save_bindings(&self, b: &Bindings) {
        write_file(&self.controls_file(), &b.to_toml());
    }

    pub fn load_settings(&self) -> Settings {
        std::fs::read_to_string(self.settings_file())
            .ok()
            .and_then(|s| toml::from_str(&s).map_err(|e| log::warn!("settings.toml: {e}")).ok())
            .unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) {
        write_file(&self.settings_file(), &toml::to_string_pretty(s).unwrap_or_default());
    }
}

fn write_file(path: &std::path::Path, contents: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, contents) {
        log::error!("could not write {}: {e}", path.display());
    }
}

/// Cross-platform world folder name; a title-screen name can never escape saves/.
pub fn valid_world_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 || name == "." || name == ".." || name.ends_with(['.', ' '])
        || name.chars().any(|c| c.is_control() || "/\\:<>\"|?*".contains(c)) { return false; }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

#[cfg(test)]
mod world_name_tests {
    use super::*;
    #[test]
    fn world_names_are_portable_and_cannot_escape_save_root() {
        for name in ["world", "My World", "四维世界"] { assert!(valid_world_name(name)); }
        for name in ["", ".", "..", "../other", "a/b", "a\\b", "C:world", "NUL", "COM1.txt", "bad.", "bad "] {
            assert!(!valid_world_name(name), "{name}");
        }
    }
}
