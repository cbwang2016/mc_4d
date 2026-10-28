//! User settings and file locations.

use crate::input::Bindings;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub fov: f32,
    /// In chunks (rays travel at most this far, in-slice).
    pub render_distance: u32,
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
            glasses: true,
            shadows: true,
            edges: true,
            show_hints: true,
            tutorial_done: false,
        }
    }
}

pub struct Paths {
    pub config: PathBuf,
    pub saves: PathBuf,
    pub screenshots: PathBuf,
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
