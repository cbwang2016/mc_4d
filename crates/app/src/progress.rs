//! Player-wide statistics and the original sixteen dependency-based achievements.
//! Progress is shared across worlds; tutorial and headless fixtures remain isolated.
use crate::{
    game::{Game, Slot},
    i18n::Language,
};
use mc4d_core::{block, item};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub struct Achievement {
    /// Original Beta 1.5 statistic identifier, kept with the verified table data.
    #[allow(dead_code)]
    pub key: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub zh_title: &'static str,
    pub zh_description: &'static str,
    pub x: i32,
    pub y: i32,
    pub parent: Option<usize>,
    pub icon_reference: u16,
    pub special: bool,
}
impl Achievement {
    pub fn title(&self, language: Language) -> &'static str {
        if language == Language::Chinese {
            self.zh_title
        } else {
            self.title
        }
    }
    pub fn description(&self, language: Language) -> &'static str {
        if language == Language::Chinese {
            self.zh_description
        } else {
            self.description
        }
    }
    pub fn icon(&self) -> Slot {
        let id = if self.icon_reference < 256 {
            block::from_reference_id(self.icon_reference)
                .map(item::ItemId::from)
                .unwrap_or(0)
        } else {
            self.icon_reference
        };
        Slot::new(id, 1)
    }
}
#[path = "achievement_data.rs"]
mod data;
pub use data::ACHIEVEMENTS;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemCounts {
    pub mined: u64,
    pub crafted: u64,
    pub used: u64,
    pub depleted: u64,
    pub picked_up: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
    pub version: u32,
    pub unlocked: BTreeSet<u8>,
    pub counters: BTreeMap<String, u64>,
    pub items: BTreeMap<String, ItemCounts>,
    remainders: BTreeMap<String, f64>,
    #[serde(skip)]
    revision: u64,
    #[serde(skip)]
    dirty: bool,
}
impl Default for Progress {
    fn default() -> Self {
        Self {
            version: 1,
            unlocked: BTreeSet::new(),
            counters: BTreeMap::new(),
            items: BTreeMap::new(),
            remainders: BTreeMap::new(),
            revision: 0,
            dirty: false,
        }
    }
}
impl Progress {
    fn changed(&mut self) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn counter(&self, name: &str) -> u64 {
        self.counters.get(name).copied().unwrap_or(0)
    }
    pub fn bump(&mut self, name: &str, amount: u64) {
        let counter = self.counters.entry(name.into()).or_default();
        *counter = counter.saturating_add(amount);
        self.changed();
    }
    pub fn accumulate(&mut self, name: &str, amount: f64) {
        if !amount.is_finite() || amount <= 0.0 {
            return;
        }
        let remainder = self.remainders.entry(name.into()).or_default();
        *remainder += amount;
        let whole = remainder.floor() as u64;
        *remainder -= whole as f64;
        if whole > 0 {
            self.bump(name, whole);
        } else {
            self.changed();
        }
    }
    pub fn available(&self, index: usize) -> bool {
        ACHIEVEMENTS
            .get(index)
            .is_some_and(|a| a.parent.is_none_or(|p| self.unlocked.contains(&(p as u8))))
    }
    pub fn award(&mut self, index: usize) -> bool {
        if !self.available(index) || !self.unlocked.insert(index as u8) {
            return false;
        }
        self.changed();
        true
    }
    pub fn item(&mut self, id: item::ItemId) -> &mut ItemCounts {
        self.changed();
        self.items.entry(id.to_string()).or_default()
    }
}

pub struct ProfileStore {
    path: PathBuf,
    writable: bool,
    pub state: Arc<Mutex<Progress>>,
}
impl ProfileStore {
    pub fn load(path: PathBuf) -> Self {
        let (state, writable) = match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Progress>(&text) {
                Ok(p) if p.version <= 1 => (p, true),
                _ => {
                    log::warn!(
                        "statistics file unreadable; preserving original and keeping session-only progress"
                    );
                    (Progress::default(), false)
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Progress::default(), true),
            Err(e) => {
                log::warn!("statistics load: {e}");
                (Progress::default(), false)
            }
        };
        Self {
            path,
            writable,
            state: Arc::new(Mutex::new(state)),
        }
    }
    pub fn save(&self) -> std::io::Result<()> {
        let snapshot = self.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if !self.writable || !snapshot.dirty {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(&snapshot).map_err(std::io::Error::other)?;
        let temp = self.path.with_extension("toml.tmp");
        std::fs::write(&temp, text)?;
        std::fs::rename(temp, &self.path)?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.revision == snapshot.revision {
            state.dirty = false;
        }
        Ok(())
    }
}

impl Game {
    pub fn award_achievement(&mut self, index: usize) {
        if self
            .progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .award(index)
        {
            self.achievement_notifications.push(index as u8);
        }
    }
    pub fn record_counter(&mut self, name: &str, amount: u64) {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bump(name, amount);
    }
    pub fn record_distance(&mut self, name: &str, meters: f32) {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .accumulate(name, meters as f64 * 100.0);
    }
    pub fn record_crafted(&mut self, stack: Slot) {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .item(stack.block)
            .crafted += stack.count as u64;
        let index = match stack.block {
            id if id == item::ItemId::from(block::CRAFTING_TABLE) => Some(2),
            item::WOOD_PICKAXE => Some(3),
            id if id == item::ItemId::from(block::FURNACE) => Some(4),
            item::WOOD_HOE => Some(6),
            item::BREAD => Some(7),
            item::CAKE_ITEM => Some(8),
            item::STONE_PICKAXE => Some(9),
            item::WOOD_SWORD => Some(12),
            _ => None,
        };
        if let Some(index) = index {
            self.award_achievement(index);
        }
    }
    pub fn record_smelted(&mut self, stack: Slot) {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .item(stack.block)
            .crafted += stack.count as u64;
        match stack.block {
            item::IRON_INGOT => self.award_achievement(5),
            item::COOKED_FISH => self.award_achievement(10),
            _ => {}
        }
    }
    pub fn record_pickup(&mut self, stack: Slot) {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .item(stack.block)
            .picked_up += stack.count as u64;
        if stack.block == item::ItemId::from(block::LOG) {
            self.award_achievement(1);
        }
        if stack.block == item::LEATHER {
            self.award_achievement(14);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dependency_rules_do_not_award_out_of_order_or_repeat_toasts() {
        let mut p = Progress::default();
        assert!(!p.award(1));
        assert!(p.award(0));
        assert!(!p.award(0));
        assert!(p.award(1));
        assert!(p.award(2));
        assert!(!p.award(5));
        let restored: Progress = toml::from_str(&toml::to_string(&p).unwrap()).unwrap();
        assert_eq!(restored.unlocked, p.unlocked);
    }
    #[test]
    fn fractional_time_and_distance_are_not_lost_at_high_frame_rates() {
        let mut p = Progress::default();
        for _ in 0..600 {
            p.accumulate("play_ticks", 20.0 / 60.0);
            p.accumulate("walk_cm", 0.25);
        }
        assert!((199..=200).contains(&p.counter("play_ticks")));
        assert_eq!(p.counter("walk_cm"), 150);
    }
    #[test]
    fn crafting_transaction_only_records_successful_output() {
        let mut game = Game::menu_preview(7);
        game.award_achievement(0);
        game.award_achievement(1);
        game.crafting_grid[0] = Some(Slot::new(block::PLANKS, 1));
        game.crafting_grid[1] = Some(Slot::new(block::PLANKS, 1));
        game.crafting_grid[2] = Some(Slot::new(block::PLANKS, 1));
        game.crafting_grid[3] = Some(Slot::new(block::PLANKS, 1));
        game.hotbar.fill(Some(Slot::new(block::STONE, 64)));
        game.inventory.fill(Some(Slot::new(block::STONE, 64)));
        assert!(!game.craft_to_inventory());
        assert!(!game.progress.lock().unwrap().unlocked.contains(&2));
        assert!(game.progress.lock().unwrap().items.is_empty());
        game.hotbar[0] = None;
        assert!(game.craft_to_inventory());
        let progress = game.progress.lock().unwrap();
        assert!(progress.unlocked.contains(&2));
        assert_eq!(
            progress.items[&block::CRAFTING_TABLE.to_string()].crafted,
            1
        );
    }

    #[test]
    fn profile_survives_restart_and_preserves_unreadable_original() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-profiles")
            .join(format!("{}-{unique}", std::process::id()));
        let path = root.join("statistics.toml");
        let profile = ProfileStore::load(path.clone());
        {
            let mut p = profile.state.lock().unwrap();
            p.award(0);
            p.bump("deaths", 2);
        }
        profile.save().unwrap();
        let restored = ProfileStore::load(path.clone());
        assert_eq!(restored.state.lock().unwrap().counter("deaths"), 2);
        assert!(restored.state.lock().unwrap().unlocked.contains(&0));
        std::fs::write(&path, "unreadable original = [").unwrap();
        let invalid = ProfileStore::load(path.clone());
        invalid.state.lock().unwrap().bump("deaths", 1);
        invalid.save().unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "unreadable original = ["
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
