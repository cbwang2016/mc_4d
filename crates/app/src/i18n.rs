//! UI language selection, independent of the operating system locale.
use serde::{Deserialize, Serialize};
use std::{collections::HashMap,sync::OnceLock};
static CATALOG_ZH: OnceLock<HashMap<String,String>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    Chinese,
}

pub fn text(language: Language, english: &str) -> &str {
    if language == Language::English { return english; }
    match english {
        "Your tool broke" => "工具损坏了",
        "The bed is obstructed" => "床被阻挡了",
        "You can only sleep at night" => "只能在夜晚睡觉",
        "Respawn point set" => "重生点已设置",
        "You may not rest now; monsters are nearby" => "附近有怪物，现在无法休息",
        "Your home bed was missing or obstructed" => "你的床已不存在或被阻挡",
        "Edit sign message" => "编辑告示牌文字",
        "Building terrain and preparing the slices…" => "正在生成地形并准备截面……",
        "4D Tutorial" => "四维教程",
        "Generating level" => "正在生成世界",
        "Click: bind | Shift: alternate | Right-click: clear | Ctrl: reset" => "点击：绑定 | Shift：备用绑定 | 右键：清除 | Ctrl：重置",
        "Press a key or mouse button; Esc cancels." => "请按键盘或鼠标按键；Esc 取消。",
        "Jukebox: original procedural arrangement" => "唱片机：原创配乐",
        "Record inserted" => "已放入唱片",
        "A world beyond three dimensions" => "探索三维之外的世界",
        "Explore, build, and discover what lies along W." => "探索、建造，发现 W 方向上的世界。",
        "Play World" => "进入世界",
        "Quit Game" => "退出游戏",
        "Back to Game" => "返回游戏",
        "Options…" | "Options" => "选项",
        "Controls…" | "Controls" => "控制设置",
        "Inventory" => "物品栏",
        "Hotbar" => "快捷栏",
        "Creative" => "创造模式",
        "Creative (flying)" => "创造模式（飞行）",
        "Survival" => "生存模式",
        "Creative inventory — choose a block for the selected hotbar slot" => "创造模式物品栏：点击方块放入当前快捷栏位置",
        "Left-click: move stack · Right-click: split / place one · Shift-click: quick transfer" => "左键：移动整组 · 右键：分半／放置一个 · Shift＋点击：快速移动",
        "Game Menu" => "游戏菜单",
        "Game saved" => "游戏已保存",
        "Save Game" => "保存游戏",
        "Save and Quit" => "保存并退出",
        "Tutorial" => "教程",
        "Stop Tutorial" => "停止教程",
        "Field of view" => "视野",
        "Render distance" => "渲染距离",
        "Render scale" => "渲染比例",
        "Sun shadows" => "阳光阴影",
        "Cross-section edges" => "截面边缘",
        "Multiple slice views" => "多截面视图",
        "Slice layout" => "视图布局",
        "Ghost layers (red = ana, blue = kata)" => "相邻截面（红色＝ana，蓝色＝kata）",
        "Ghost layer distance" => "相邻截面距离",
        "4D glasses (creature outlines)" => "四维眼镜（生物轮廓）",
        "Auto-snap slice to axes" => "自动将截面对齐坐标轴",
        "Show control hints" => "显示操作提示",
        "Reset to defaults" => "恢复默认设置",
        "Done" => "完成",
        "Mouse sensitivity" => "鼠标灵敏度",
        "Invert mouse Y" => "反转鼠标 Y 轴",
        "Slice rotation: wheel step" => "截面旋转：滚轮步长",
        "Slice rotation: key speed" => "截面旋转：按键速度",
        "Slice rotation: modifier + mouse" => "截面旋转：修饰键＋鼠标",
        "Press a key, mouse button or scroll the wheel… (Esc to cancel)" => "按下按键、鼠标键或滚动滚轮……（Esc 取消）",
        "Click a binding to change it. Conflicting bindings are shown in red." => "点击按键绑定进行修改。冲突的绑定显示为红色。",
        "Right-click a binding to clear it." => "右键点击以清除绑定。",
        "Reset" => "重置",
        "Reset All Keys" => "重置所有按键",
        "Objective: try it now" => "目标：现在试一试",
        "Press Enter to continue" => "按 Enter 继续",
        "Esc → Stop Tutorial to skip" => "Esc → 停止教程，即可跳过",
        "✔ Nice!" => "✔ 完成！",
        "Movement" => "移动",
        "4D Navigation" => "四维导航",
        "Gameplay" => "游戏操作",
        "Interface" => "界面",
        "Air" => "空气", "Stone" => "石头", "Dirt" => "泥土", "Grass" => "草方块",
        "Sand" => "沙子", "Log" => "原木", "Leaves" => "树叶", "Planks" => "木板",
        "Cobblestone" => "圆石", "Coal Ore" => "煤矿石", "Glass" => "玻璃", "Brick" => "砖块",
        "Snow" => "雪", "Water" => "水", "Bedrock" => "基岩", "Glowstone" => "荧石",
        "Forward" => "前进",
        "Back" => "后退",
        "Strafe Left" => "向左平移",
        "Strafe Right" => "向右平移",
        "Move Kata (−ana)" => "向 kata 移动（−ana）",
        "Move Ana (+ana)" => "向 ana 移动（+ana）",
        "Jump / Fly Up" => "跳跃／向上飞行",
        "Sneak / Fly Down" => "潜行／向下飞行",
        "Sprint" => "疾跑",
        "Toggle Flying (creative)" => "切换飞行（创造模式）",
        "Turn Forward → Ana" => "将前方转向 ana",
        "Turn Forward → Kata" => "将前方转向 kata",
        "Swing Right → Ana" => "将右方转向 ana",
        "Swing Right → Kata" => "将右方转向 kata",
        "Mouse Rotates Slice (hold)" => "鼠标旋转截面（按住）",
        "Snap Slice to Axes" => "将截面对齐坐标轴",
        "Switch to Side View" => "切换到侧视图",
        "Flip 90° into W" => "向第四维翻转 90°",
        "Toggle Ghost Layers" => "切换相邻截面提示",
        "Toggle Side View" => "切换多截面视图",
        "Toggle 4D Glasses" => "切换四维眼镜",
        "Break Block / Attack" => "破坏方块／攻击",
        "Place Block" => "放置方块",
        "Drop Item" => "丢出物品",
        "Pick Block" => "选取方块",
        "Next Hotbar Slot" => "下一个快捷栏位置",
        "Previous Hotbar Slot" => "上一个快捷栏位置",
        "Hotbar Slot 1" => "快捷栏 1", "Hotbar Slot 2" => "快捷栏 2",
        "Hotbar Slot 3" => "快捷栏 3", "Hotbar Slot 4" => "快捷栏 4",
        "Hotbar Slot 5" => "快捷栏 5", "Hotbar Slot 6" => "快捷栏 6",
        "Hotbar Slot 7" => "快捷栏 7", "Hotbar Slot 8" => "快捷栏 8",
        "Hotbar Slot 9" => "快捷栏 9",
        "Toggle HUD" => "切换游戏界面",
        "Third-Person View" => "第三人称视角",
        "Debug Overlay" => "调试信息",
        "Screenshot" => "截图",
        "Left Button" => "鼠标左键", "Right Button" => "鼠标右键", "Middle Button" => "鼠标中键",
        "Mouse Back" => "鼠标后退键", "Mouse Forward" => "鼠标前进键",
        "Wheel Up" => "滚轮向上", "Wheel Down" => "滚轮向下",
        "Space" => "空格", "Left Shift" => "左 Shift", "Right Shift" => "右 Shift",
        "Left Ctrl" => "左 Ctrl", "Right Ctrl" => "右 Ctrl",
        "Left Alt" => "左 Alt", "Right Alt" => "右 Alt",
        "Left Super" => "左 Super", "Right Super" => "右 Super",
        "unbound" => "未绑定",
        "1: Play" => "1：操作视图", "2: Side" => "2：侧面截面",
        "3: Forward / W" => "3：前方／第四维截面",
        "4: Side +45°" => "4：侧向 +45°", "5: Side −45°" => "5：侧向 −45°",
        "6: Forward +45°" => "6：前向 +45°", "7: Forward −45°" => "7：前向 −45°",
        "8: Diagonal +45°" => "8：斜向 +45°", "9: Diagonal −45°" => "9：斜向 −45°",
        "Lava" => "岩浆",
        "Obsidian" => "黑曜石",
        "Crafting Table" => "工作台",
        "Chest" => "箱子",
        "Large chest" => "大箱子",
        "Furnace" => "熔炉",
        "Iron Ore" => "铁矿石",
        "Gold Ore" => "金矿石",
        "Stick" => "木棍",
        "Coal" => "煤炭",
        "Charcoal" => "木炭",
        "Iron Ingot" => "铁锭",
        "Gold Ingot" => "金锭",
        "Clay Ball" => "黏土球",
        "Brick Item" => "红砖",
        "Raw Porkchop" => "生猪肉",
        "Cooked Porkchop" => "熟猪肉",
        "Raw Fish" => "生鱼",
        "Cooked Fish" => "熟鱼",
        "Bucket" => "桶",
        "Water Bucket" => "水桶",
        "Lava Bucket" => "岩浆桶",
        "Wheat" => "小麦",
        "Bread" => "面包",
        "Sugar Cane" => "甘蔗",
        "Paper" => "纸",
        "Book" => "书",
        "Bone" => "骨头",
        "Bone Meal" => "骨粉",
        "Sugar" => "糖",
        "Crafting" => "合成",
        "Input · Fuel · Output" => "原料 · 燃料 · 产物",
        "Smelting" => "烧炼进度",
        "Fuel" => "燃料",
        "Armor" => "盔甲",
        "Chain Helmet" => "锁链头盔",
        "Chain Chestplate" => "锁链胸甲",
        "Chain Leggings" => "锁链护腿",
        "Chain Boots" => "锁链靴子",
        "Iron Helmet" => "铁头盔",
        "Iron Chestplate" => "铁胸甲",
        "Iron Leggings" => "铁护腿",
        "Iron Boots" => "铁靴子",
        "Gold Helmet" => "金头盔",
        "Gold Chestplate" => "金胸甲",
        "Gold Leggings" => "金护腿",
        "Gold Boots" => "金靴子",
        "Diamond Helmet" => "钻石头盔",
        "Diamond Chestplate" => "钻石胸甲",
        "Diamond Leggings" => "钻石护腿",
        "Diamond Boots" => "钻石靴子",
        "Leather Cap" => "皮革帽子",
        "Leather Tunic" => "皮革外套",
        "Leather Pants" => "皮革裤子",
        "Leather Boots" => "皮革靴子",
        "You died!" => "你死了！",
        "Your items remain where you fell. Return to collect them." => "物品留在你倒下的地方。返回那里可将它们拾回。",
        "Respawn" => "重生",
        "Fall" => "从高处跌落", "Drowned" => "溺水", "Suffocated" => "窒息",
        "Void" => "坠入虚空", "Creature" => "被生物攻击",
        "Time of day" => "世界时间",
        "Weather" => "天气",
        "Clear" => "晴天",
        "Rain" => "下雨",
        "Thunderstorm" => "雷暴",
        "Diamond" => "钻石",
        "Diamond Ore" => "钻石矿石",
        "Farmland" => "耕地",
        "Wool" => "羊毛",
        "Leather" => "皮革",
        "Feather" => "羽毛",
        "Arrow" => "箭",
        "String" => "线",
        "Gunpowder" => "火药",
        "Slimeball" => "黏液球",
        "Ink Sac" => "墨囊",
        "Egg" => "鸡蛋",
        "Seeds" => "种子",
        "Wooden Pickaxe" => "木镐",
        "Wooden Shovel" => "木锹",
        "Wooden Axe" => "木斧",
        "Wooden Hoe" => "木锄",
        "Wooden Sword" => "木剑",
        "Stone Pickaxe" => "石镐",
        "Stone Shovel" => "石锹",
        "Stone Axe" => "石斧",
        "Stone Hoe" => "石锄",
        "Stone Sword" => "石剑",
        "Iron Pickaxe" => "铁镐",
        "Iron Shovel" => "铁锹",
        "Iron Axe" => "铁斧",
        "Iron Hoe" => "铁锄",
        "Iron Sword" => "铁剑",
        "Golden Pickaxe" => "金镐",
        "Golden Shovel" => "金锹",
        "Golden Axe" => "金斧",
        "Golden Hoe" => "金锄",
        "Golden Sword" => "金剑",
        "Diamond Pickaxe" => "钻石镐",
        "Diamond Shovel" => "钻石锹",
        "Diamond Axe" => "钻石斧",
        "Diamond Hoe" => "钻石锄",
        "Diamond Sword" => "钻石剑",
        "Singleplayer" => "单人游戏",
        "Select World" => "选择世界",
        "Saved worlds" => "已保存的世界",
        "World name" => "世界名称",
        "Seed for a new world" => "新世界的种子",
        "Choose a saved world or enter a new name and seed." => "选择已有世界，或输入新的名称和种子。",
        "Cancel" => "取消",
        "Now in four dimensions!" => "现在有四个空间维度！",
        "Texture Packs" => "材质包",
        "Texture pack folder" => "材质包文件夹",
        "Choose Texture Pack…" => "选择材质包……",
        "Leave empty for the classic textures." => "留空以使用经典材质。",
        "Texture pack error" => "材质包错误",
        "Classic Beta textures" => "经典 Beta 材质",
        "The original pixel textures, with subtle fourth-axis shading." => "原版像素材质，仅对第四维表面添加轻微色调。",
        "Use Classic Pack" => "使用经典材质包",
        "Apply Texture Pack" => "应用材质包",
        "Folder layout: terrain.png, gui/items.png, mob/*.png" => "文件结构：terrain.png、gui/items.png、mob/*.png",
        "Choose a simple world name without path characters." => "请输入不含路径字符的有效世界名称。",
        "Flying" => "飞行模式",
        "Walking" => "行走模式",
        "Flying is only available in Creative mode" => "仅创造模式可飞行",
        "Gloom" => "幽球",
        "Pig" => "猪",
        "Cow" => "牛",
        "Sheep" => "羊",
        "Chicken" => "鸡",
        "Squid" => "鱿鱼",
        "Zombie" => "僵尸",
        "Skeleton" => "骷髅",
        "Spider" => "蜘蛛",
        "Creeper" => "苦力怕",
        "Slime" => "史莱姆",
        "Wolf" => "狼",
        "Plains" => "平原",
        "Forest" => "森林",
        "Desert" => "沙漠",
        "Taiga" => "针叶林",
        "Mountains" => "山地",
        "Swamp" => "沼泽",
        "Ocean" => "海洋",
        "Save and Quit to Title" => "保存并返回标题画面",
        "Chat / Commands" => "聊天／命令",
        "Type /help for commands" => "输入 /help 查看命令",
        "ON" => "开",
        "OFF" => "关",
        "Music" => "音乐",
        "Sound" => "音效",
        "Invert Mouse" => "反转鼠标",
        "Sensitivity" => "灵敏度",
        "Difficulty" => "难度",
        "Peaceful" => "和平",
        "Easy" => "简单",
        "Normal" => "普通",
        "Hard" => "困难",
        "Video Settings" => "视频设置",
        "Video Settings…" => "视频设置……",
        "4D Settings" => "四维设置",
        "4D Settings…" => "四维设置……",
        "Graphics" => "图像品质",
        "Fancy" => "精致",
        "Fast" => "流畅",
        "Render Distance" => "渲染距离",
        "Tiny" => "极近",
        "Short" => "近",
        "Far" => "远",
        "Limit Framerate" => "限制帧率",
        "Fullscreen" => "全屏",
        "No texture packs found. Put folders or .zip files here." => "未找到材质包。请将文件夹或 .zip 文件放在这里。",
        "Folder or .zip: terrain.png, gui/items.png, mob/*.png" => "文件夹或 .zip：terrain.png、gui/items.png、mob/*.png",
        "Toggle Fullscreen" => "切换全屏",
        "3D Anaglyph" => "红蓝立体",
        "View Bobbing" => "视角摇晃",
        "Smooth Lighting" => "平滑光照",
        "GUI Scale" => "界面大小",
        "Small" => "小",
        "Large" => "大",
        "Largest" => "特大",
        "Auto" => "自动",
        "Chunk Culling" => "区块剔除",
        "Auxiliary View" => "辅助视图",
        "Main + Auxiliary" => "主视图＋辅助视图",
        "Ghost Layers" => "相邻截面",
        "4D Glasses" => "四维眼镜",
        "Auto Snap" => "自动对齐",
        "Wheel rotation" => "滚轮旋转",
        "Key rotation" => "按键旋转",
        "4D mouse speed" => "四维鼠标速度",
        "Toggle Fog" => "切换渲染距离",
        "Game could not be saved" => "无法保存游戏",
        "Wolf tamed" => "狼已被驯服",
        "The wolf is still wary" => "狼仍然很警惕",
        "Lightning" => "闪电",
        "Ghost layers" => "相邻截面",
        "Side view" => "辅助视图",
        "4D glasses" => "四维眼镜",
        "on" => "开",
        "off" => "关",
        "Saved screenshot" => "截图已保存至",
        "Screenshot failed" => "截图失败",
        "This world could not be loaded. Its files have been preserved." => "无法载入此世界，其文件已保留。",
        "world no longer exists" => "该世界已不存在",
        "invalid world folder" => "无效的世界文件夹",
        "world must be a real save directory" => "世界必须是有效的存档文件夹",
        "world metadata missing" => "缺少世界信息文件",
        "invalid world name" => "无效的世界名称",
        _ => CATALOG_ZH.get_or_init(|| toml::from_str(include_str!("../assets/lang/catalog-zh.toml")).expect("valid catalog translations")).get(english).map(String::as_str).unwrap_or(english),
    }
}

/// Text shown under "You died!" for a damage cause recorded by the game (`Game::death_reason`).
pub fn death_reason(language: Language, cause: &str) -> &str {
    if language == Language::English { return cause; }
    match cause {
        "Lava" => "试图在岩浆里游泳",
        "Fire" => "被烧死",
        "Lightning" => "被闪电击中",
        "Arrow" => "被箭射杀",
        _ => text(language, cause),
    }
}

/// Fixed toast, status and menu strings that must have a Chinese entry. Keys that are translated
/// at display time (toasts are passed through `text` in `ui.rs`) or used as dynamic message parts.
#[cfg(test)]
pub(crate) const MESSAGE_KEYS: &[&str] = &[
    // Toasts raised from gameplay code and translated when displayed.
    "Your tool broke", "The bed is obstructed", "You can only sleep at night", "Respawn point set",
    "You may not rest now; monsters are nearby", "Your home bed was missing or obstructed",
    "Record inserted", "Jukebox: original procedural arrangement", "Flying", "Walking",
    "Flying is only available in Creative mode", "Game saved", "Game could not be saved",
    "Wolf tamed", "The wolf is still wary",
    // F-key toggles, screenshots and save failures (app.rs).
    "Ghost layers", "Side view", "4D glasses", "on", "off", "Render distance",
    "Saved screenshot", "Screenshot failed",
    // World selection errors shown on the world screen.
    "This world could not be loaded. Its files have been preserved.",
    "Choose a simple world name without path characters.", "world no longer exists",
    "invalid world folder", "world must be a real save directory", "world metadata missing", "invalid world name",
    // Death screen.
    "You died!", "Your items remain where you fell. Return to collect them.", "Respawn", "Save and Quit to Title",
    "Fall", "Drowned", "Suffocated", "Void", "Creature", "Lava", "Fire", "Lightning", "Arrow",
    // Menus.
    "Game Menu", "Back to Game", "Options…", "Controls…", "Tutorial", "Stop Tutorial", "Save Game",
    "Creative", "Survival", "Creative (flying)", "Singleplayer", "Texture Packs", "4D Tutorial", "Quit Game",
    "Classic Beta textures", "The original pixel textures, with subtle fourth-axis shading.", "Use Classic Pack",
    "Texture pack folder", "Folder or .zip: terrain.png, gui/items.png, mob/*.png", "No texture packs found. Put folders or .zip files here.", "Apply Texture Pack",
    "Texture pack error", "Done", "Reset", "Controls", "Edit sign message", "Generating level",
    "Building terrain and preparing the slices…", "Type /help for commands", "Now in four dimensions!",
    "Press a key or mouse button; Esc cancels.", "Click: bind | Shift: alternate | Right-click: clear | Ctrl: reset",
    // Options pages.
    "Options", "Video Settings", "4D Settings", "Video Settings…", "4D Settings…", "Music", "Sound", "Invert Mouse",
    "Sensitivity", "Difficulty", "Peaceful", "Easy", "Normal", "Hard", "Graphics", "Fancy", "Fast", "Render Distance",
    "Tiny", "Short", "Far", "Limit Framerate", "3D Anaglyph", "View Bobbing", "Smooth Lighting", "GUI Scale", "Small",
    "Large", "Largest", "Auto", "Chunk Culling", "Field of view", "Render scale", "Fullscreen",
    "Auxiliary View", "Slice layout", "Main + Auxiliary", "Ghost Layers", "4D Glasses", "Auto Snap",
    "Cross-section edges", "Show control hints", "Sun shadows", "Ghost layer distance", "Wheel rotation",
    "Key rotation", "4D mouse speed", "ON", "OFF",
    // Containers.
    "Inventory", "Crafting", "Chest", "Large chest", "Furnace", "Dispenser", "Crafting Table",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playable_items_and_creatures_have_chinese_names() {
        for id in mc4d_core::block::ALL_BLOCKS.iter().copied().map(mc4d_core::item::ItemId::from).chain(mc4d_core::item::ALL_ITEMS.iter().copied()) {
            let name=mc4d_core::item::info(id).name;
            if name=="TNT" {continue;} // Official Chinese name keeps this international abbreviation.
            assert_ne!(text(Language::Chinese,name),name,"missing item translation: {name}");
        }
        for kind in crate::game::CreatureKind::ALL {
            assert_ne!(text(Language::Chinese,kind.name()),kind.name());
        }
    }

    #[test]
    fn toast_and_menu_messages_have_chinese_translations() {
        let missing: Vec<_> = MESSAGE_KEYS.iter().filter(|&&key| text(Language::Chinese, key) == key).collect();
        assert!(missing.is_empty(), "missing Chinese UI/toast translations: {missing:?}");
        for cause in ["Fall", "Drowned", "Suffocated", "Void", "Creature", "Lava", "Fire", "Lightning", "Arrow"] {
            assert_ne!(death_reason(Language::Chinese, cause), cause, "death reason {cause}");
            assert_eq!(death_reason(Language::English, cause), cause);
        }
        for category in crate::input::Category::ALL {
            assert_ne!(text(Language::Chinese, category.name()), category.name());
        }
        for vehicle in ["Boat", "Minecart", "Minecart with Chest", "Minecart with Furnace"] {
            assert_ne!(text(Language::Chinese, vehicle), vehicle);
        }
        // Classic 150-px option buttons fit about 15 CJK glyphs at 9 px; keep labels well under that.
        for key in MESSAGE_KEYS.iter().take_while(|&&k| k != "Inventory").skip_while(|&&k| k != "Options") {
            assert!(text(Language::Chinese, key).chars().count() <= 10, "Chinese option label too long: {key}");
        }
    }

    #[test]
    fn creative_stack_names_have_chinese_translations() {
        let mut missing: Vec<_> = mc4d_core::item::creative_stacks().into_iter().map(mc4d_core::item::stack_name)
            .filter(|&name| name != "TNT" && text(Language::Chinese, name) == name).collect();
        missing.dedup();
        assert!(missing.is_empty(), "missing stack translations: {missing:?}");
    }

    #[test]
    fn all_actions_and_slice_labels_have_chinese_translations() {
        for action in crate::input::Action::ALL {
            assert_ne!(text(Language::Chinese, action.name()), action.name(), "{}", action.id());
        }
        for label in mc4d_core::views::VIEW_LABELS {
            assert_ne!(text(Language::Chinese, label), label);
        }
    }
}
