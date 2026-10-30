//! Local world commands. These never run shell commands or contact a server.
use super::*;
use crate::i18n::Language;
use mc4d_core::item;

impl Game {
    pub fn command(&mut self, line: &str, language: Language) -> Result<String, String> {
        let words: Vec<_> = line.trim().trim_start_matches('/').split_whitespace().collect();
        let say = |en: &str, zh: &str| if language == Language::Chinese { zh.to_string() } else { en.to_string() };
        let usage = || say("Invalid command arguments. Type /help.", "命令参数无效。输入 /help 查看帮助。");
        match words.as_slice() {
            ["help"] => Ok(say("/time set <day|noon|night|midnight|ticks> · /time add <ticks> · /weather <clear|rain|thunder> [seconds] · /gamemode <survival|creative> · /tp <x> <y> <z> <w> · /give <item> [count] [variant] [variant] · /summon <mob> · /spawnpoint · /seed · /save",
                "/time set <day|noon|night|midnight|刻数> · /time add <刻数> · /weather <clear|rain|thunder> [秒数] · /gamemode <survival|creative> · /tp <x> <y> <z> <w> · /give <物品> [数量] [变种] [变种] · /summon <生物> · /spawnpoint · /seed · /save")),
            ["time", "set", value] => {
                let ticks = match *value {
                    "day" => 1000, "noon" => 6000, "night" => 13000, "midnight" => 18000,
                    n => n.parse::<u64>().map_err(|_| usage())?,
                };
                if ticks > i64::MAX as u64 { return Err(usage()); }
                self.environment.world_ticks = ticks;
                Ok(format!("{} {ticks}", say("Time set to", "时间已设为")))
            }
            ["time", "add", value] => {
                let ticks: u64 = value.parse().map_err(|_| usage())?;
                self.environment.world_ticks = self.environment.world_ticks.checked_add(ticks)
                    .filter(|&v| v <= i64::MAX as u64).ok_or_else(usage)?;
                Ok(format!("{} {}", say("World time:", "世界时间："), self.environment.world_ticks))
            }
            ["time", "query", "daytime"] => Ok((self.environment.world_ticks % 24000).to_string()),
            ["weather", kind] | ["weather", kind, _] => {
                if !matches!(*kind, "clear" | "rain" | "thunder") { return Err(usage()); }
                let seconds = words.get(2).map_or(Ok(600u64), |s| s.parse().map_err(|_| usage()))?;
                if !(1..=1_000_000).contains(&seconds) { return Err(usage()); }
                self.environment.raining = *kind != "clear";
                self.environment.thundering = *kind == "thunder";
                self.environment.weather_ticks = seconds * 20;
                let kind = match *kind { "clear" => say("clear", "晴天"), "rain" => say("rain", "下雨"), _ => say("thunder", "雷暴") };
                Ok(format!("{} {kind} ({seconds}{})", say("Weather:", "天气："), say("s", " 秒")))
            }
            ["gamemode", mode] => {
                let mode = match *mode {
                    "creative" | "c" | "1" => GameMode::Creative,
                    "survival" | "s" | "0" => GameMode::Survival,
                    _ => return Err(usage()),
                };
                self.set_mode(mode);
                Ok(say(if mode == GameMode::Creative { "Creative mode" } else { "Survival mode" },
                    if mode == GameMode::Creative { "创造模式" } else { "生存模式" }))
            }
            ["tp", x, y, z, w] | ["teleport", x, y, z, w] => {
                let current = self.body.pos.to_array();
                let mut coordinates = [0.0; 4];
                for (i, value) in [x, y, z, w].into_iter().enumerate() {
                    coordinates[i] = if let Some(offset) = value.strip_prefix('~') {
                        current[i] + if offset.is_empty() { 0.0 } else { offset.parse::<f32>().map_err(|_| usage())? }
                    } else { value.parse::<f32>().map_err(|_| usage())? };
                    if !coordinates[i].is_finite() || coordinates[i].abs() > 30_000_000.0 { return Err(usage()); }
                }
                self.vehicles.saved.riding=None;self.vehicles.saved.rail_origin=None;
                self.body.pos = Vec4::from_array(coordinates);
                self.body.vel = Vec4::ZERO;
                self.update_streaming(&Settings::default(), true);
                Ok(format!("{} {}", say("Teleported to", "已传送至"), self.body.pos))
            }
            ["give", name] | ["give", name, _] | ["give", name, _, _] => {
                let name = name.strip_prefix("minecraft:").unwrap_or(name);
                let mut selected = if let Ok(id) = name.parse::<item::ItemId>() {
                    let id=if id<256 {block::from_reference_id(id).map(item::ItemId::from).ok_or_else(usage)?}else{id};
                    if !item::valid(id) { return Err(usage()); }
                    Slot::new(id, 1)
                } else {
                    item::creative_stacks().into_iter().find(|&stack|
                        item::stack_name(stack).to_ascii_lowercase().replace(' ', "_") == name)
                        .ok_or_else(usage)?
                };
                if let Some(value) = words.get(3) {
                    selected.variant = value.parse::<u8>().map_err(|_| usage())?;
                    if !item::valid_variant(selected.block, selected.variant) { return Err(usage()); }
                }
                let id = selected.block;
                let count = words.get(2).map_or(Ok(1u32), |n| n.parse().map_err(|_| usage()))?;
                if !(1..=64).contains(&count) { return Err(usage()); }
                let mut remaining = count;
                while remaining > 0 {
                    let n = remaining.min(item::stack_limit(id));
                    if let Some(left) = self.insert_stack_range(Slot { count: n, ..selected }, 0..36) {
                        self.spawn_drop(left, self.eye(), Vec4::ZERO, 0.5);
                    }
                    remaining -= n;
                }
                Ok(format!("{} {} ×{count}", say("Given", "已给予"), crate::i18n::text(language, item::stack_name(selected))))
            }
            ["summon", name] => {
                let kind = CreatureKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(name)).ok_or_else(usage)?;
                if self.creatures.len() >= MAX_CREATURES { return Err(say("Nearby creature limit reached.", "附近的生物数量已达上限。")); }
                self.spawn_creature(kind, self.body.pos + self.basis().forward_h * 3.0);
                Ok(format!("{} {}", say("Summoned", "已召唤"), crate::i18n::text(language, kind.name())))
            }
            ["spawnpoint"] => { self.set_spawn(self.body.pos); Ok(say("Spawn point set.", "重生点已设置。")) }
            ["seed"] => Ok(format!("{} {}", say("Seed:", "种子："), self.seed)),
            ["save"] => { self.save_all(); Ok(say("Game saved.", "游戏已保存。")) }
            _ => Err(usage()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn time_weather_and_mode_commands_change_real_world_state() {
        let mut g = Game::new(None, 7, GameMode::Survival);
        g.command("/time set midnight", Language::English).unwrap();
        assert_eq!(g.environment.world_ticks, 18000);
        g.command("/time add 20", Language::Chinese).unwrap();
        assert_eq!(g.environment.world_ticks, 18020);
        g.command("/weather thunder 90", Language::English).unwrap();
        assert!(g.environment.raining && g.environment.thundering);
        assert_eq!(g.environment.weather_ticks, 1800);
        g.command("/gamemode creative", Language::English).unwrap();
        assert_eq!(g.mode, GameMode::Creative);
        g.command("/weather clear", Language::English).unwrap();
        assert!(!g.environment.raining);
        assert!(g.command("/weather snow", Language::English).is_err());
        let before = g.body.pos;
        assert!(g.command("/tp NaN 0 0 0", Language::English).is_err());
        assert_eq!(g.body.pos, before);
        g.command("/tp ~1 ~ ~ ~-2", Language::English).unwrap();
        assert_eq!(g.body.pos, before + Vec4::new(1.0,0.0,0.0,-2.0));
        assert!(g.command("/give stone 999999999", Language::English).is_err());
    }
}
