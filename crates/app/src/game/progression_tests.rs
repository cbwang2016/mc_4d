//! End-to-end Survival progression walkthroughs (Beta 1.5 order) driven through
//! the same `Game` entry points the UI and input loop use: clicks on inventory,
//! crafting and container slots, held-button mining and right-click use.
use super::*;
use crate::input::{Binding, MouseBtn};
use mc4d_core::block::BlockState;
use mc4d_core::chunk::{Chunk, WORLD_CHUNKS_Y};
use mc4d_core::raycast::Hit;
use item::ItemId;

const FLOOR: i32 = 3;

/// Open-sky test column (chunk x=z=w=0, every y) with a solid floor whose top
/// layer is `top`. The player stands at the centre of the floor.
fn arena(top: u8) -> Game {
    let mut g = Game::menu_preview(7);
    for cy in 0..WORLD_CHUNKS_Y {
        let mut chunk = Chunk::uniform(AIR);
        if cy == 0 {
            for w in 0..16 { for z in 0..16 { for x in 0..16 { for y in 0..=FLOOR {
                chunk.set(IVec4::new(x, y, z, w), if y == FLOOR { top } else { block::STONE });
            }}}}
        }
        g.world.insert_chunk(IVec4::new(0, cy, 0, 0), chunk);
    }
    g.body = Body::new(Vec4::new(8.5, FLOOR as f32 + 1.0, 8.5, 8.5));
    g.body.on_ground = true;
    g.creatures.clear();
    g.environment.raining = false;
    g
}
fn column() -> Vec<IVec4> { (0..WORLD_CHUNKS_Y).map(|y| IVec4::new(0, y, 0, 0)).collect() }
fn settle_light(g: &mut Game) {
    let wanted = column();
    let start = std::time::Instant::now();
    while !g.world.lighting_ready(&wanted) {
        g.world.update_lighting(&wanted, 128);
        assert!(start.elapsed().as_secs() < 20, "lighting never settled");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
fn ground(x: i32, z: i32, w: i32) -> IVec4 { IVec4::new(x, FLOOR, z, w) }
fn aim(g: &mut Game, block: IVec4, normal: IVec4) {
    g.target = Some(Target::Block(Hit { block, normal, dist: 2.0 }));
}
fn left() -> Binding { Binding::Mouse(MouseBtn::Left) }
fn right() -> Binding { Binding::Mouse(MouseBtn::Right) }

/// Hold attack on `block` (20 Hz frames) until it breaks; returns elapsed seconds.
fn mine(g: &mut Game, block: IVec4) -> f32 {
    aim(g, block, IVec4::Y);
    let bindings = Bindings::default();
    let mut input = InputState::default();
    input.press(left());
    for frame in 0..4000 {
        g.interact(0.05, &input, &bindings);
        input.end_frame();
        if g.world.block(block) == Some(AIR) { return (frame + 1) as f32 * 0.05; }
    }
    panic!("{block} was never mined");
}
/// One right click on `block`'s `normal` face with the selected stack.
fn use_on(g: &mut Game, block: IVec4, normal: IVec4) {
    aim(g, block, normal);
    let mut input = InputState::default();
    input.press(right());
    g.interact(0.05, &input, &Bindings::default());
}
/// Let drops land and their pickup delay expire, then walk onto each one.
fn collect(g: &mut Game) {
    let home = g.body.pos;
    for _ in 0..30 { g.update_dropped_items(0.05); }
    for _ in 0..64 {
        let Some(d) = g.dropped_items.first() else { break };
        g.body.pos = d.body.pos - Vec4::Y * 0.8;
        g.update_dropped_items(0.0);
    }
    assert!(g.dropped_items.is_empty(), "uncollectable drops");
    g.body.pos = home;
}
fn count(g: &Game, id: impl Into<ItemId>) -> u32 {
    let id = id.into();
    g.hotbar.iter().chain(g.inventory.iter()).flatten().filter(|s| s.block == id).map(|s| s.count).sum()
}
fn find(g: &Game, id: impl Into<ItemId>) -> Option<usize> {
    let id = id.into();
    (0..36).find(|&i| g.inventory_slot(i).is_some_and(|s| s.block == id))
}
/// Select `id` on the hotbar, swapping it into the current hotbar slot with
/// cursor clicks when it is in storage.
fn select(g: &mut Game, id: impl Into<ItemId>) {
    let id = id.into();
    let index = find(g, id).unwrap_or_else(|| panic!("no item {id}"));
    if index < 9 { g.selected = index; return; }
    let mut cursor = None;
    for slot in [index, g.selected, index] { g.click_inventory_slot(slot, &mut cursor, false); }
    assert!(cursor.is_none() && g.hotbar[g.selected].is_some_and(|s| s.block == id));
}
/// Lay out `rows` in the current grid one item per cell with right clicks (as a
/// player does) and return the recipe preview.
fn lay_out(g: &mut Game, rows: &[&str], keys: &[(char, ItemId)]) -> Slot {
    let width = g.crafting_width;
    for (y, row) in rows.iter().enumerate() {
        for (x, key) in row.chars().enumerate() {
            if key == ' ' { continue; }
            let id = keys.iter().find(|k| k.0 == key).unwrap().1;
            let from = find(g, id).unwrap_or_else(|| panic!("missing ingredient {id} for {rows:?}"));
            let mut cursor = None;
            g.click_inventory_slot(from, &mut cursor, false);
            g.click_crafting_slot(y * width + x, &mut cursor, true);
            g.click_inventory_slot(from, &mut cursor, false);
            assert!(cursor.is_none());
        }
    }
    g.crafting_output().unwrap_or_else(|| panic!("no recipe for {rows:?}"))
}
/// Lay out a recipe, then shift-click the result into the inventory.
fn craft(g: &mut Game, rows: &[&str], keys: &[(char, ItemId)]) -> Slot {
    let output = lay_out(g, rows, keys);
    assert!(g.craft_to_inventory(), "crafting {rows:?}");
    assert!(g.crafting_grid.iter().all(Option::is_none), "one of each ingredient is consumed");
    output
}
fn open_station(g: &mut Game, block: IVec4) {
    aim(g, block, IVec4::Y);
    let mut input = InputState::default();
    input.press(right());
    g.interact(0.05, &input, &Bindings::default());
    assert!(g.active_station == Some(block) || g.active_station_secondary == Some(block), "right click opens the station");
}
fn temp_world(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/progression-tests").join(format!("{name}-{}-{stamp}", std::process::id()))
}

const P: ItemId = block::PLANKS as ItemId;
const S: ItemId = item::STICK;
const C: ItemId = block::COBBLE as ItemId;

/// Punch a tree, then build the wooden and stone tool tiers at a real table.
fn wood_and_stone_age(g: &mut Game) -> IVec4 {
    // A four-log trunk; punching it by hand takes Beta's hardness 2.0 * 1.5 = 3 s per log.
    for y in 1..=4 { g.world.set_block(ground(11, 8, 8) + IVec4::Y * y, block::LOG); }
    for y in (1..=4).rev() {
        let seconds = mine(g, ground(11, 8, 8) + IVec4::Y * y);
        assert!((2.9..=3.1).contains(&seconds), "bare-hand log took {seconds}s");
    }
    assert_eq!(g.dropped_items.len(), 4, "logs drop physically, not into the inventory");
    assert_eq!(count(g, block::LOG), 0);
    collect(g);
    assert_eq!(count(g, block::LOG), 4);
    // Inventory 2x2 grid: one log gives four planks.
    assert_eq!(g.crafting_width, 2);
    for _ in 0..4 { assert_eq!(craft(g, &["L"], &[('L', block::LOG.into())]), Slot::new(block::PLANKS, 4)); }
    assert_eq!(count(g, block::PLANKS), 16);
    assert_eq!(craft(g, &["P", "P"], &[('P', P)]), Slot::new(item::STICK, 4));
    assert_eq!(craft(g, &["PP", "PP"], &[('P', P)]), Slot::new(block::CRAFTING_TABLE, 1));
    // A 3x3 recipe cannot fit in the inventory grid.
    assert!(find(g, item::WOOD_PICKAXE).is_none());
    // Place the table on the floor two blocks away and open it with right click.
    select(g, block::CRAFTING_TABLE);
    use_on(g, ground(10, 9, 8), IVec4::Y);
    let table = ground(10, 9, 8) + IVec4::Y;
    assert_eq!(g.world.block(table), Some(block::CRAFTING_TABLE));
    assert_eq!(count(g, block::CRAFTING_TABLE), 0);
    open_station(g, table);
    assert_eq!(g.crafting_width, 3);
    assert_eq!(craft(g, &["PPP", " S ", " S "], &[('P', P), ('S', S)]), Slot::new(item::WOOD_PICKAXE, 1));
    g.close_station();
    assert_eq!(g.crafting_width, 2);
    // Stone mined by hand drops nothing; with the wooden pickaxe it yields cobblestone.
    // An exposed stone outcrop in the grass floor.
    for w in 5..=7 { for z in 5..=7 { for x in 4..=6 { g.world.set_block(ground(x, z, w), block::STONE); } } }
    g.selected = (0..9).find(|&i| g.hotbar[i].is_none()).unwrap();
    assert!((7.4..=7.6).contains(&mine(g, ground(6, 6, 6))), "bare-hand stone takes 7.5 s");
    collect(g);
    assert_eq!(count(g, block::COBBLE), 0, "Beta stone requires a pickaxe to drop");
    select(g, item::WOOD_PICKAXE);
    let mut mined = 0;
    for w in 5..=7 { for z in 5..=7 {
        if z == 6 && w == 6 { continue; }
        let seconds = mine(g, ground(5, z, w));
        assert!(seconds < 1.3, "wooden pickaxe is 2x faster than 7.5 s by hand: {seconds}");
        mined += 1;
    }}
    for z in 5..=7 { mine(g, ground(4, z, 6)); mined += 1; }
    collect(g);
    assert_eq!(count(g, block::COBBLE), mined);
    let pick = g.inventory_slot(find(g, item::WOOD_PICKAXE).unwrap()).unwrap();
    assert_eq!(pick.damage, mined as u16, "each mined block wears the pickaxe once");
    // Stone tier at the same table.
    open_station(g, table);
    while count(g, item::STICK) < 7 { craft(g, &["P", "P"], &[('P', P)]); }
    assert_eq!(craft(g, &["CCC", " S ", " S "], &[('C', C), ('S', S)]).block, item::STONE_PICKAXE);
    assert_eq!(craft(g, &["C", "C", "S"], &[('C', C), ('S', S)]).block, item::STONE_SWORD);
    assert_eq!(craft(g, &["CC", "CS", " S"], &[('C', C), ('S', S)]).block, item::STONE_AXE);
    assert_eq!(craft(g, &["C", "S", "S"], &[('C', C), ('S', S)]).block, item::STONE_SHOVEL);
    g.close_station();
    for id in [item::STONE_PICKAXE, item::STONE_SWORD, item::STONE_AXE, item::STONE_SHOVEL] {
        let tool = g.inventory_slot(find(g, id).unwrap()).unwrap();
        assert_eq!((tool.count, tool.damage), (1, 0));
        assert_eq!(item::max_durability(id), 131, "Beta stone tools have 131 uses");
    }
    table
}

#[test]
fn punch_tree_craft_table_and_wooden_then_stone_tools() {
    let mut g = arena(block::GRASS);
    wood_and_stone_age(&mut g);
    // The stone pickaxe is what unlocks iron; the wooden one mines it without loot.
    for (tool, drops) in [(item::WOOD_PICKAXE, false), (item::STONE_PICKAXE, true)] {
        let ore = ground(12, 12, 12);
        g.world.set_block(ore, block::IRON_ORE);
        select(&mut g, tool);
        mine(&mut g, ore);
        collect(&mut g);
        assert_eq!(count(&g, block::IRON_ORE) > 0, drops, "iron ore with {tool}");
    }
    // Axe and shovel are faster on their materials than bare hands.
    select(&mut g, item::STONE_AXE);
    g.world.set_block(ground(12, 4, 12) + IVec4::Y, block::LOG);
    assert!(mine(&mut g, ground(12, 4, 12) + IVec4::Y) < 1.0);
    select(&mut g, item::STONE_SHOVEL);
    g.world.set_block(ground(4, 12, 12), block::DIRT);
    assert!(mine(&mut g, ground(4, 12, 12)) < 0.25);
}

#[test]
fn furnace_smelts_iron_and_cooks_food_from_mined_coal() {
    let mut g = arena(block::GRASS);
    let table = wood_and_stone_age(&mut g);
    // Coal and iron ores in the floor.
    for z in 3..=4 { g.world.set_block(ground(13, z, 10), block::COAL_ORE); }
    for z in 3..=5 { g.world.set_block(ground(13, z, 12), block::IRON_ORE); }
    select(&mut g, item::WOOD_PICKAXE);
    for z in 3..=4 { mine(&mut g, ground(13, z, 10)); }
    select(&mut g, item::STONE_PICKAXE);
    for z in 3..=5 { mine(&mut g, ground(13, z, 12)); }
    // More cobblestone for the furnace ring.
    for z in 10..=13 { for w in [3, 4] { g.world.set_block(ground(3, z, w), block::STONE); mine(&mut g, ground(3, z, w)); } }
    collect(&mut g);
    assert_eq!(count(&g, item::COAL), 2, "coal ore drops coal items");
    assert_eq!(count(&g, block::IRON_ORE), 3, "Beta iron ore drops the ore block");
    assert!(count(&g, block::COBBLE) >= 8);
    open_station(&mut g, table);
    assert_eq!(craft(&mut g, &["CCC", "C C", "CCC"], &[('C', C)]), Slot::new(block::FURNACE, 1));
    g.close_station();
    select(&mut g, block::FURNACE);
    use_on(&mut g, ground(8, 11, 8), IVec4::Y);
    let furnace = ground(8, 11, 8) + IVec4::Y;
    assert_eq!(g.world.block(furnace), Some(block::FURNACE));
    // Kill a pig for raw pork (the drop count is Beta's nextInt(3), so try a few).
    select(&mut g, item::STONE_SWORD);
    for attempt in 0..20 {
        if count(&g, item::RAW_PORK) > 0 { break; }
        assert!(attempt < 19, "no pig ever dropped pork");
        g.spawn_creature(CreatureKind::Pig, Vec4::new(8.5, 4.0, 6.5, 8.5));
        let mut input = InputState::default();
        input.press(left());
        while g.creatures.iter().any(|c| c.kind == CreatureKind::Pig) {
            g.target = Some(Target::Creature(0));
            g.interact(0.1, &input, &Bindings::default());
            input.end_frame();
        }
        collect(&mut g);
    }
    // Open the furnace: ore in the top slot, coal below.
    open_station(&mut g, furnace);
    assert_eq!(g.station_kind(), Some(StationKind::Furnace));
    let mut cursor = None;
    g.click_inventory_slot(find(&g, block::IRON_ORE).unwrap(), &mut cursor, false);
    let mut refused = cursor;
    g.click_station_slot(1, &mut refused, false);
    assert_eq!(refused, cursor, "ore is not fuel");
    g.click_station_slot(0, &mut cursor, false);
    g.click_inventory_slot(find(&g, item::COAL).unwrap(), &mut cursor, false);
    g.click_station_slot(1, &mut cursor, false);
    assert!(cursor.is_none());
    // Real time through the full update loop; the player stays in reach.
    let settings = Settings::default();
    let mut seconds = 0.0;
    while g.station_slot(2).map_or(0, |s| s.count) < 3 {
        g.update(0.05, None, &settings);
        g.body.pos = Vec4::new(8.5, 4.0, 8.5, 8.5);
        seconds += 0.05;
        if seconds > 9.0 && seconds < 9.1 { assert_eq!(g.world.block(furnace), Some(block::LIT_FURNACE)); }
        assert!(seconds < 40.0, "smelting stalled: {:?}", (0..3).map(|i| g.station_slot(i)).collect::<Vec<_>>());
    }
    assert!((29.5..=30.6).contains(&seconds), "Beta cooks one item per 10 s, took {seconds}");
    assert_eq!(g.station_slot(2), Some(Slot::new(item::IRON_INGOT, 3)));
    assert_eq!(g.station_slot(1), Some(Slot::new(item::COAL, 1)), "one coal smelts eight items");
    g.quick_transfer_station(2);
    assert_eq!(count(&g, item::IRON_INGOT), 3);
    // Cook the pork with the remaining burn time, then extract by cursor.
    g.click_inventory_slot(find(&g, item::RAW_PORK).unwrap(), &mut cursor, false);
    let pork = cursor.unwrap().count;
    g.click_station_slot(0, &mut cursor, false);
    for _ in 0..(pork as usize * 200 + 20) { g.update(0.05, None, &settings); g.body.pos = Vec4::new(8.5, 4.0, 8.5, 8.5); }
    assert_eq!(g.station_slot(2), Some(Slot::new(item::COOKED_PORK, pork)));
    g.click_station_slot(2, &mut cursor, false);
    assert_eq!(cursor, Some(Slot::new(item::COOKED_PORK, pork)));
    g.return_cursor(&mut cursor);
    assert_eq!(count(&g, item::COOKED_PORK), pork);
    assert!(g.active_station.is_none());
    g.health = 10;
    select(&mut g, item::COOKED_PORK);
    let mut input = InputState::default();
    input.press(right());
    g.target = None;
    g.interact(0.05, &input, &Bindings::default());
    assert_eq!(g.health, 18, "cooked pork heals four hearts");
}

#[test]
fn chest_and_double_chest_contents_survive_disk_save_and_reload() {
    let dir = temp_world("chest");
    let mut g = Game::try_load(Some(dir.clone()), 7, GameMode::Survival).unwrap();
    g.creatures.clear();
    let base = g.body.pos.floor().as_ivec4() + IVec4::X * 3;
    // A cleared stone pad around the build site.
    for w in -1..=2 { for z in -1..=1 { for x in -1..=1 {
        let p = base + IVec4::new(x, 0, z, w);
        g.world.set_block(p - IVec4::Y, block::STONE);
        for y in 0..3 { g.world.set_block(p + IVec4::Y * y, AIR); }
    }}}
    g.hotbar[0] = Some(Slot::new(block::PLANKS, 32));
    g.inventory[0] = Some(Slot::new(block::COBBLE, 40));
    g.inventory[1] = Some(Slot { damage: 17, ..Slot::new(item::STONE_PICKAXE, 1) });
    g.inventory[2] = Some(Slot::with_variant(block::WOOL, 5, 14));
    let table = base - IVec4::X * 6;
    g.world.set_block(table, block::CRAFTING_TABLE);
    g.body.pos = table.as_vec4() + Vec4::new(1.5, 0.0, 0.5, 0.5);
    open_station(&mut g, table);
    for _ in 0..2 { assert_eq!(craft(&mut g, &["PPP", "P P", "PPP"], &[('P', P)]), Slot::new(block::CHEST, 1)); }
    g.close_station();
    assert_eq!(count(&g, block::CHEST), 2);
    g.body.pos = base.as_vec4() + Vec4::new(0.5, 0.0, -1.5, 0.5);
    select(&mut g, block::CHEST);
    use_on(&mut g, base - IVec4::Y, IVec4::Y);
    assert_eq!(g.world.block(base), Some(block::CHEST));
    open_station(&mut g, base);
    assert_eq!((g.station_slot_count(), g.station_title()), (27, Some("Chest")));
    for item in [block::COBBLE as ItemId, item::STONE_PICKAXE, block::WOOL as ItemId] {
        g.quick_transfer_to_station(find(&g, item).unwrap());
    }
    g.close_station();
    // The second chest along +W (a genuinely 4D neighbour) forms a large chest.
    // As in Beta 1.5, right-clicking the chest itself opens it, so place on the floor.
    use_on(&mut g, base + IVec4::W - IVec4::Y, IVec4::Y);
    assert_eq!(g.world.block(base + IVec4::W), Some(block::CHEST));
    assert_eq!(count(&g, block::CHEST), 0);
    open_station(&mut g, base + IVec4::W);
    assert_eq!((g.station_slot_count(), g.station_title()), (54, Some("Large chest")));
    let mut cursor = None;
    g.click_inventory_slot(find(&g, block::PLANKS).unwrap(), &mut cursor, false);
    g.click_station_slot(53, &mut cursor, false);
    assert!(cursor.is_none());
    g.close_station();
    let stored: Vec<_> = { open_station(&mut g, base); let v = (0..54).map(|i| g.station_slot(i)).collect(); g.close_station(); v };
    assert_eq!(stored.iter().flatten().count(), 4);
    g.save_checked().unwrap();
    drop(g);
    let mut g = Game::try_load(Some(dir.clone()), 0, GameMode::Creative).unwrap();
    assert_eq!(g.mode, GameMode::Survival, "the saved mode wins");
    assert_eq!(g.world.block(base), Some(block::CHEST));
    assert_eq!(g.world.block(base + IVec4::W), Some(block::CHEST));
    g.body.pos = base.as_vec4() + Vec4::new(0.5, 0.0, -1.5, 0.5);
    open_station(&mut g, base + IVec4::W);
    assert_eq!(g.station_slot_count(), 54);
    let reloaded: Vec<_> = (0..54).map(|i| g.station_slot(i)).collect();
    assert_eq!(reloaded, stored);
    assert!(reloaded.contains(&Some(Slot::new(block::COBBLE, 40))));
    assert!(reloaded.contains(&Some(Slot { damage: 17, ..Slot::new(item::STONE_PICKAXE, 1) })));
    assert!(reloaded.contains(&Some(Slot::with_variant(block::WOOL, 5, 14))));
    assert_eq!(reloaded[53], Some(Slot::new(block::PLANKS, 16)));
    for i in 0..54 { g.quick_transfer_station(i); }
    assert_eq!(count(&g, block::COBBLE), 40);
    assert_eq!(count(&g, block::PLANKS), 16);
    drop(g);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hoe_seed_hydrate_grow_harvest_wheat_and_eat_bread() {
    let mut g = arena(block::GRASS);
    // Beta 1.5 has no tall grass: seeds come from tilling grass with a hoe (1 in 8).
    g.inventory[0] = Some(Slot::new(block::PLANKS, 4));
    g.inventory[1] = Some(Slot::new(item::STICK, 2));
    let table = ground(8, 12, 8) + IVec4::Y;
    g.world.set_block(table, block::CRAFTING_TABLE);
    open_station(&mut g, table);
    assert_eq!(craft(&mut g, &["PP", " S", " S"], &[('P', P), ('S', S)]), Slot::new(item::WOOD_HOE, 1));
    g.close_station();
    select(&mut g, item::WOOD_HOE);
    // Water two blocks along W from the field row: within Beta's 4-block reach,
    // which this world extends symmetrically over X, Z and W.
    let water = ground(6, 8, 8) + IVec4::W * 4;
    g.world.set_block(water, block::WATER);
    let field: Vec<_> = (5..=7).map(|x| ground(x, 8, 8)).collect();
    let dry = ground(11, 4, 5);
    // Excluded from the water only by its W distance of 5.
    let dry_w = ground(10, 8, 7);
    let mut tilled = 0;
    let mut seeds_from = 0;
    let candidates: Vec<_> = (2..14).flat_map(|x| (2..6).map(move |z| ground(x, z, 13))).collect();
    for &p in field.iter().chain([&dry, &dry_w]).chain(candidates.iter()) {
        if count(&g, item::SEEDS) >= 4 && tilled >= 5 { break; }
        let before = g.dropped_items.len();
        use_on(&mut g, p, IVec4::Y);
        assert_eq!(g.world.block(p), Some(block::FARMLAND));
        tilled += 1;
        if g.dropped_items.len() > before { seeds_from += 1; }
        collect(&mut g);
    }
    assert!(seeds_from >= 1 && count(&g, item::SEEDS) >= 4, "tilling {tilled} grass produced {} seeds", count(&g, item::SEEDS));
    assert_eq!(g.hotbar[g.selected].unwrap().damage, tilled as u16, "each till wears the hoe");
    // Hoes do not till blocks with something on top.
    g.world.set_block(ground(2, 2, 2) + IVec4::Y, block::STONE);
    use_on(&mut g, ground(2, 2, 2), IVec4::Y);
    assert_eq!(g.world.block(ground(2, 2, 2)), Some(block::GRASS));
    // Seeds plant only on farmland.
    select(&mut g, item::SEEDS);
    use_on(&mut g, ground(2, 3, 2), IVec4::Y);
    assert_eq!(g.world.block(ground(2, 3, 2) + IVec4::Y), Some(AIR), "seeds need farmland");
    let seeds = count(&g, item::SEEDS);
    for &p in &field { use_on(&mut g, p, IVec4::Y); assert_eq!(g.world.block_state(p + IVec4::Y), Some(BlockState::new(block::CROPS, 0))); }
    assert_eq!(count(&g, item::SEEDS), seeds - 3);
    // Beta's seeds use only the top face of farmland (qj.a); a side face of the
    // block beside a free farmland cell must not plant there.
    let spare = candidates.iter().copied().find(|&p| g.world.block(p) == Some(block::FARMLAND)).expect("spare farmland");
    g.world.set_block(spare + IVec4::X + IVec4::Y, block::STONE);
    use_on(&mut g, spare + IVec4::X + IVec4::Y, IVec4::NEG_X);
    assert_eq!(g.world.block(spare + IVec4::Y), Some(AIR), "seeds planted through a side face");
    // Plants are not replaceable in Beta 1.5: clicking the ground below a
    // flower with a block does not destroy the flower.
    let flower = ground(3, 12, 12) + IVec4::Y;
    g.world.set_block(flower, block::DANDELION);
    g.inventory[26] = Some(Slot::new(block::DIRT, 1));
    select(&mut g, block::DIRT);
    use_on(&mut g, flower - IVec4::Y, IVec4::Y);
    assert_eq!(g.world.block(flower), Some(block::DANDELION));
    assert_eq!(count(&g, block::DIRT), 1);
    // ke.a tills grass under any non-solid material, so the flower stays on farmland.
    select(&mut g, item::WOOD_HOE);
    use_on(&mut g, flower - IVec4::Y, IVec4::Y);
    assert_eq!(g.world.block(flower - IVec4::Y), Some(block::FARMLAND));
    assert_eq!(g.world.block(flower), Some(block::DANDELION));
    // Random ticks at noon with settled light until every crop is ripe.
    settle_light(&mut g);
    g.environment.world_ticks = 6000;
    let mut ticks = 0u64;
    let wanted = column();
    while field.iter().any(|&p| g.world.block_state(p + IVec4::Y) != Some(BlockState::new(block::CROPS, 7))) {
        g.world.update_lighting(&wanted, 128);
        if !g.world.lighting_ready(&wanted) { std::thread::sleep(std::time::Duration::from_millis(1)); continue; }
        g.update_farming(1.0);
        ticks += 20;
        assert!(ticks < 2_000_000, "crops never ripened: {:?}", field.iter().map(|&p| g.world.block_state(p + IVec4::Y)).collect::<Vec<_>>());
        for &p in &field { assert_eq!(g.world.block(p), Some(block::FARMLAND), "watered farmland under crops never reverts"); }
    }
    for &p in &field { assert_eq!(g.world.block_state(p).unwrap().metadata, 7, "{p} is hydrated by water 4 blocks away along W"); }
    assert_eq!(g.world.block(dry), Some(block::DIRT), "dry bare farmland reverts to dirt");
    assert_eq!(g.world.block(dry_w), Some(block::DIRT), "water 5 blocks away along W is out of reach");
    assert!(g.world.block(water).is_some_and(block::is_water));
    // Right-clicking a growing crop with seeds neither replants nor consumes.
    select(&mut g, item::SEEDS);
    let seeds = count(&g, item::SEEDS);
    use_on(&mut g, field[1] + IVec4::Y, IVec4::Y);
    use_on(&mut g, field[1], IVec4::Y);
    assert_eq!(g.world.block_state(field[1] + IVec4::Y), Some(BlockState::new(block::CROPS, 7)));
    assert_eq!(count(&g, item::SEEDS), seeds);
    // Harvest by punching: each ripe crop drops one wheat plus 0-3 seeds.
    g.selected = (0..9).find(|&i| g.hotbar[i].is_none()).unwrap();
    for &p in &field { assert!(mine(&mut g, p + IVec4::Y) <= 0.05, "crops break instantly"); }
    collect(&mut g);
    assert_eq!(count(&g, item::WHEAT), 3);
    assert!(count(&g, item::SEEDS) >= seeds, "harvest only adds seeds");
    for &p in &field { assert_eq!(g.world.block(p), Some(block::FARMLAND)); }
    open_station(&mut g, table);
    // This time take the result onto the cursor and drop it into a slot.
    assert_eq!(lay_out(&mut g, &["WWW"], &[('W', item::WHEAT)]), Slot::new(item::BREAD, 1));
    let mut cursor = None;
    assert!(g.take_crafting_output(&mut cursor));
    assert_eq!(cursor, Some(Slot::new(item::BREAD, 1)));
    assert!(g.crafting_output().is_none() && count(&g, item::WHEAT) == 0);
    let empty = (0..36).find(|&i| g.inventory_slot(i).is_none()).unwrap();
    g.click_inventory_slot(empty, &mut cursor, false);
    assert!(cursor.is_none());
    g.close_station();
    // Beta 1.5 has no hunger: bread restores 5 health directly.
    g.health = 9;
    select(&mut g, item::BREAD);
    g.target = None;
    let mut input = InputState::default();
    input.press(right());
    g.interact(0.05, &input, &Bindings::default());
    assert_eq!(g.health, 14);
    assert_eq!(count(&g, item::BREAD), 0);
}

#[test]
fn bed_crafted_placed_slept_in_sets_spawn_skips_night_and_respawns_there() {
    let mut g = arena(block::GRASS);
    g.inventory[0] = Some(Slot::new(block::PLANKS, 3));
    g.inventory[1] = Some(Slot::new(block::WOOL, 3));
    let table = ground(8, 12, 8) + IVec4::Y;
    g.world.set_block(table, block::CRAFTING_TABLE);
    open_station(&mut g, table);
    assert_eq!(craft(&mut g, &["WWW", "PPP"], &[('W', block::WOOL.into()), ('P', P)]), Slot::new(item::BED_ITEM, 1));
    g.close_station();
    select(&mut g, item::BED_ITEM);
    // kt.a only accepts a top face: the side of a pillar is refused.
    let pillar = ground(13, 8, 8) + IVec4::Y;
    g.world.set_block(pillar, block::STONE);
    use_on(&mut g, pillar, IVec4::X);
    assert_eq!(g.world.block(pillar + IVec4::X), Some(AIR));
    assert_eq!(count(&g, item::BED_ITEM), 1);
    use_on(&mut g, ground(11, 8, 8), IVec4::Y);
    let foot = ground(11, 8, 8) + IVec4::Y;
    let foot_state = g.world.block_state(foot).unwrap();
    assert_eq!(foot_state.id, block::BED);
    let head = foot + mc4d_core::shapes::bed_direction(foot_state.metadata);
    assert_eq!(g.world.block_state(head).map(|s| (s.id, s.metadata & 8)), Some((block::BED, 8)), "the bed occupies two blocks");
    assert_eq!(count(&g, item::BED_ITEM), 0);
    // Daytime: the bed refuses and the spawn is unchanged.
    let world_spawn = g.spawn;
    g.environment.world_ticks = 6000;
    use_on(&mut g, foot, IVec4::Y);
    assert_eq!((g.spawn, g.environment.world_ticks), (world_spawn, 6000));
    // Night, but more than 3 blocks away along W: TOO_FAR_AWAY, nothing happens.
    g.environment.world_ticks = 24000 * 3 + 18000;
    g.body.pos.w += 4.0;
    use_on(&mut g, foot, IVec4::Y);
    assert_eq!((g.spawn, g.environment.world_ticks), (world_spawn, 24000 * 3 + 18000));
    g.body.pos.w -= 4.0;
    // Night: sleeping skips to the next morning and sets the spawn at the bed.
    use_on(&mut g, foot, IVec4::Y);
    assert_eq!(g.environment.world_ticks, 24000 * 4);
    assert!(!g.environment.is_night());
    assert_eq!(g.building.bed_spawn, Some(head));
    let bed_spawn = g.spawn;
    assert!((bed_spawn - head.as_vec4()).length() < 3.5, "spawn {bed_spawn} beside head {head}");
    // Wander off, die and respawn beside the bed rather than at the world spawn.
    g.body.pos = Vec4::new(3.5, 4.0, 3.5, 3.5);
    g.damage(20, "Fall");
    assert!(!g.is_alive());
    g.respawn();
    assert!(g.is_alive());
    assert_eq!(g.body.pos.floor(), bed_spawn.floor());
    assert!((g.body.pos - head.as_vec4()).length() < 3.5);
    assert!(!g.body.is_stuck_states(&|p| g.world.block_state(p)));
    // Breaking the bed returns one bed item and reverts to the world spawn.
    g.selected = (0..9).find(|&i| g.hotbar[i].is_none()).unwrap();
    mine(&mut g, head);
    g.update_building();
    assert_eq!(g.world.block(foot), Some(AIR));
    collect(&mut g);
    assert_eq!(count(&g, item::BED_ITEM), 1);
    g.damage(20, "Fall");
    g.respawn();
    assert!(g.building.bed_spawn.is_none());
    assert!((g.body.pos - head.as_vec4()).length() > 3.5);
}

#[test]
fn iron_armor_reduces_damage_wears_out_and_tools_break() {
    let mut g = arena(block::STONE);
    g.inventory[0] = Some(Slot::new(item::IRON_INGOT, 24));
    let table = ground(8, 12, 8) + IVec4::Y;
    g.world.set_block(table, block::CRAFTING_TABLE);
    open_station(&mut g, table);
    let i = item::IRON_INGOT;
    let pieces = [
        craft(&mut g, &["III", "I I"], &[('I', i)]),
        craft(&mut g, &["I I", "III", "III"], &[('I', i)]),
        craft(&mut g, &["III", "I I", "I I"], &[('I', i)]),
        craft(&mut g, &["I I", "I I"], &[('I', i)]),
    ];
    g.close_station();
    assert_eq!(pieces.map(|s| s.block), [item::IRON_HELMET, item::IRON_CHESTPLATE, item::IRON_LEGGINGS, item::IRON_BOOTS]);
    assert_eq!(count(&g, i), 0);
    // Unarmoured baseline.
    g.damage(4, "Creature");
    assert_eq!(g.health, 16);
    g.health = 20;
    g.invulnerability = 0.0;
    // Equip by clicking each piece into its armour slot; a mismatched slot refuses.
    let mut cursor = None;
    g.click_inventory_slot(find(&g, item::IRON_HELMET).unwrap(), &mut cursor, false);
    g.click_inventory_slot(37, &mut cursor, false);
    assert_eq!(cursor.map(|s| s.block), Some(item::IRON_HELMET), "a helmet cannot go in the chest slot");
    g.click_inventory_slot(36, &mut cursor, false);
    assert!(cursor.is_none());
    for id in [item::IRON_CHESTPLATE, item::IRON_LEGGINGS, item::IRON_BOOTS] { g.quick_transfer(find(&g, id).unwrap()); }
    assert!(g.armor.iter().all(Option::is_some));
    assert_eq!(g.armor_points(), 20);
    // Full armour absorbs 80%: 10 damage -> 2.
    g.damage(10, "Creature");
    assert_eq!(g.health, 18);
    for slot in g.armor.iter().flatten() { assert_eq!(slot.damage, 10, "each piece takes the unreduced damage as wear"); }
    // Falls bypass armour.
    g.invulnerability = 0.0;
    g.damage(3, "Fall");
    assert_eq!(g.health, 15);
    // Armour wears down and finally breaks.
    let helmet_max = item::max_durability(item::IRON_HELMET);
    let mut hits = 0;
    while g.armor[0].is_some() {
        g.health = 20;
        g.invulnerability = 0.0;
        g.damage(4, "Creature");
        hits += 1;
        assert!(hits < 1000);
    }
    assert!(10 + 4 * (hits - 1) <= helmet_max as u32 && 10 + 4 * hits > helmet_max as u32, "helmet broke on the hit that exceeded its {helmet_max} durability");
    assert!(g.armor_points() < 20, "a worn/missing piece lowers protection");
    // A wooden pickaxe survives 59 blocks and breaks on the 60th.
    g.hotbar = [None; 9];
    g.hotbar[0] = Some(Slot::new(item::WOOD_PICKAXE, 1));
    g.selected = 0;
    let max = item::max_durability(item::WOOD_PICKAXE);
    assert_eq!(max, 59);
    let mut broken = 0;
    'mine: for w in 2..14 { for z in 2..14 {
        if (z, w) == (8, 8) { continue; }
        mine(&mut g, ground(4, z, w));
        broken += 1;
        if g.hotbar[0].is_none() { break 'mine; }
        assert_eq!(g.hotbar[0].unwrap().damage, broken);
    }}
    assert_eq!(broken, 60, "the pickaxe breaks on its 60th block");
    assert!(g.toasts.iter().any(|t| t.0 == "Your tool broke"));
    // Swords wear twice as fast on blocks.
    g.hotbar[0] = Some(Slot::new(item::STONE_SWORD, 1));
    g.world.set_block(ground(12, 12, 12) + IVec4::Y, block::DIRT);
    mine(&mut g, ground(12, 12, 12) + IVec4::Y);
    assert_eq!(g.hotbar[0].unwrap().damage, 2);
}

#[test]
fn bed_spawn_worn_gear_and_half_smelted_furnace_survive_disk_reload() {
    let dir = temp_world("bed-furnace");
    let mut g = Game::try_load(Some(dir.clone()), 11, GameMode::Survival).unwrap();
    g.creatures.clear();
    let base = g.body.pos.floor().as_ivec4() + IVec4::X * 3;
    for w in -2..=2 { for z in -2..=2 { for x in -2..=2 {
        let p = base + IVec4::new(x, 0, z, w);
        g.world.set_block(p - IVec4::Y, block::STONE);
        for y in 0..3 { g.world.set_block(p + IVec4::Y * y, AIR); }
    }}}
    g.body = Body::new(base.as_vec4() + Vec4::new(-1.5, 0.0, 0.5, 0.5));
    g.body.on_ground = true;
    g.hotbar = [None; 9];
    g.inventory = [None; 27];
    g.hotbar[0] = Some(Slot::new(item::BED_ITEM, 1));
    g.hotbar[1] = Some(Slot::new(block::FURNACE, 1));
    g.hotbar[2] = Some(Slot { damage: 30, ..Slot::new(item::WOOD_PICKAXE, 1) });
    g.inventory[0] = Some(Slot::new(block::IRON_ORE, 4));
    g.inventory[1] = Some(Slot::new(item::COAL, 1));
    g.inventory[2] = Some(Slot { damage: 40, ..Slot::new(item::IRON_HELMET, 1) });
    select(&mut g, item::BED_ITEM);
    use_on(&mut g, base - IVec4::Y, IVec4::Y);
    let foot = g.world.block_state(base).filter(|s| s.id == block::BED).expect("bed placed");
    let head = base + mc4d_core::shapes::bed_direction(foot.metadata);
    let furnace = base + IVec4::W * 2;
    select(&mut g, block::FURNACE);
    use_on(&mut g, furnace - IVec4::Y, IVec4::Y);
    assert_eq!(g.world.block(furnace), Some(block::FURNACE));
    open_station(&mut g, furnace);
    g.quick_transfer_to_station(find(&g, block::IRON_ORE).unwrap());
    g.quick_transfer_to_station(find(&g, item::COAL).unwrap());
    for _ in 0..300 { g.update_stations(0.05); }
    assert_eq!(g.station_slot(2), Some(Slot::new(item::IRON_INGOT, 1)));
    assert_eq!(g.world.block(furnace), Some(block::LIT_FURNACE));
    g.close_station();
    g.quick_transfer(find(&g, item::IRON_HELMET).unwrap());
    g.environment.world_ticks = 13500;
    use_on(&mut g, base, IVec4::Y);
    assert_eq!(g.building.bed_spawn, Some(head));
    let spawn = g.spawn;
    g.health = 13;
    g.save_checked().unwrap();
    drop(g);

    let mut g = Game::try_load(Some(dir.clone()), 0, GameMode::Creative).unwrap();
    assert_eq!((g.mode, g.health), (GameMode::Survival, 13));
    assert_eq!(g.environment.world_ticks, 24000, "the skipped night stays skipped");
    assert_eq!(g.armor[0], Some(Slot { damage: 40, ..Slot::new(item::IRON_HELMET, 1) }));
    assert_eq!(g.hotbar[2], Some(Slot { damage: 30, ..Slot::new(item::WOOD_PICKAXE, 1) }));
    assert_eq!((g.building.bed_spawn, g.spawn), (Some(head), spawn));
    assert_eq!(g.world.block(furnace), Some(block::LIT_FURNACE));
    // Half-cooked progress and remaining burn time continue after reload: the
    // second ingot needs only the remaining ~5 s.
    for _ in 0..110 { g.update_stations(0.05); }
    g.body.pos = furnace.as_vec4() - Vec4::W;
    open_station(&mut g, furnace);
    assert_eq!(g.station_slot(2), Some(Slot::new(item::IRON_INGOT, 2)));
    assert_eq!(g.station_slot(0), Some(Slot::new(block::IRON_ORE, 2)));
    assert_eq!(g.station_slot(1), None, "the single coal was already burning");
    g.close_station();
    // Death drops everything; respawn is at the reloaded bed.
    g.body.pos = base.as_vec4() + Vec4::new(30.0, 0.0, 0.0, 0.0);
    g.invulnerability = 0.0;
    g.damage(20, "Fall");
    assert!(g.armor.iter().chain(g.hotbar.iter()).all(Option::is_none));
    g.respawn();
    assert!((g.body.pos - head.as_vec4()).length() < 3.5, "respawned at {} not beside bed {head}", g.body.pos);
    drop(g);
    std::fs::remove_dir_all(dir).unwrap();
}
