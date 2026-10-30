//! Beta 1.5 item and interactive-block behaviour driven through the real
//! `Game` input path (right/left clicks via `interact`, simulation updates).
use super::*;
use super::vehicles::VehicleKind;
use crate::input::{Binding, MouseBtn};
use mc4d_core::block::BlockState;
use mc4d_core::chunk::{Chunk, WORLD_CHUNKS_Y};
use mc4d_core::raycast::Hit;
use item::ItemId;

const FLOOR: i32 = 3;

/// Open-sky column (chunk x=z=w=0) with a stone floor topped by `top`.
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
fn ground(x: i32, z: i32, w: i32) -> IVec4 { IVec4::new(x, FLOOR, z, w) }
fn aim(g: &mut Game, block: IVec4, normal: IVec4) {
    g.target = Some(Target::Block(Hit { block, normal, dist: 2.0 }));
}
fn click(g: &mut Game, button: MouseBtn) {
    let mut input = InputState::default();
    input.press(Binding::Mouse(button));
    g.interact(0.05, &input, &Bindings::default());
    // Release: the next click is a new click, as in play.
    g.interact(0.05, &InputState::default(), &Bindings::default());
}
fn use_on(g: &mut Game, block: IVec4, normal: IVec4) { aim(g, block, normal); click(g, MouseBtn::Right); }
fn left_on(g: &mut Game, block: IVec4) { aim(g, block, IVec4::Y); click(g, MouseBtn::Left); }
fn use_air(g: &mut Game) { g.target = None; click(g, MouseBtn::Right); }
fn hold(g: &mut Game, stack: Slot) { g.selected = 0; g.hotbar[0] = Some(stack); }
fn count(g: &Game, id: impl Into<ItemId>) -> u32 {
    let id = id.into();
    g.hotbar.iter().chain(g.inventory.iter()).flatten().filter(|s| s.block == id).map(|s| s.count).sum()
}
fn find(g: &Game, id: impl Into<ItemId>) -> Option<usize> {
    let id = id.into();
    (0..36).find(|&i| g.inventory_slot(i).is_some_and(|s| s.block == id))
}
/// Swap `id` into the selected hotbar slot with cursor clicks.
fn select(g: &mut Game, id: impl Into<ItemId>) {
    let id = id.into();
    let index = find(g, id).unwrap_or_else(|| panic!("no item {id}"));
    if index < 9 { g.selected = index; return; }
    let mut cursor = None;
    for slot in [index, g.selected, index] { g.click_inventory_slot(slot, &mut cursor, false); }
    assert!(cursor.is_none() && g.hotbar[g.selected].is_some_and(|s| s.block == id));
}
/// Fill a 3^4 stone block centred five blocks ahead of the eye.
fn wall_ahead(g: &mut Game) -> IVec4 {
    let c = (g.eye() + g.basis().forward * 5.0).floor().as_ivec4();
    for w in -1..=1 { for z in -1..=1 { for y in -1..=1 { for x in -1..=1 {
        g.world.set_block(c + IVec4::new(x, y, z, w), block::STONE);
    }}}}
    c
}
fn thrown(g: &Game, id: ItemId) -> usize {
    g.device_draws().iter().filter(|d| d.kind == 13 && d.item == id).count()
}
/// Lay out `rows` one item per cell with cursor clicks, then shift-click the result.
fn craft(g: &mut Game, rows: &[&str], keys: &[(char, ItemId)]) -> Slot {
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
    let output = g.crafting_output().unwrap_or_else(|| panic!("no recipe for {rows:?}"));
    assert!(g.craft_to_inventory(), "crafting {rows:?}");
    assert!(g.crafting_grid.iter().all(Option::is_none));
    output
}
fn open_table(g: &mut Game) {
    let table = ground(8, 12, 8) + IVec4::Y;
    g.world.set_block(table, block::CRAFTING_TABLE);
    use_on(g, table, IVec4::Y);
    assert_eq!(g.active_station, Some(table));
}

#[test]
fn beta_registry_has_cookies_but_no_maps_shears_fire_charges_or_cobweb_item_use() {
    // Item names in the client's gb registry include cookie and no map/shears/fireball.
    assert!(item::valid(item::COOKIE));
    assert_eq!(item::stack_limit(item::COOKIE), 8);
    for absent in [358u16, 359, 385] { assert!(!item::valid(absent), "{absent} is not a Beta 1.5 item"); }
}

#[test]
fn instant_bow_shot_consumes_one_arrow_sticks_in_a_wall_and_is_picked_back_up() {
    let mut g = arena(block::STONE);
    wall_ahead(&mut g);
    hold(&mut g, Slot::new(item::BOW, 1));
    use_air(&mut g);
    assert!(g.arrows.is_empty(), "no ammunition: pz.a fires nothing");
    g.inventory[5] = Some(Slot::new(item::ARROW, 3));
    use_air(&mut g);
    assert_eq!(g.arrows.len(), 1, "Beta 1.5 bows fire on the click itself, without charging");
    assert_eq!(count(&g, item::ARROW), 2);
    assert_eq!(g.hotbar[0], Some(Slot::new(item::BOW, 1)), "the bow neither wears nor is consumed");
    for _ in 0..40 { g.update_arrows(0.05); if g.arrows[0].stuck { break; } }
    assert!(g.arrows[0].stuck, "arrow sticks in the wall");
    let at = g.arrows[0].pos;
    g.body.pos = at - Vec4::Y * 0.8;
    g.update_arrows(0.4);
    assert!(g.arrows.is_empty());
    assert_eq!(count(&g, item::ARROW), 3, "the player's stuck arrow returns to the inventory");
}

#[test]
fn buckets_take_sources_and_pour_on_the_clicked_face_including_w() {
    let mut g = arena(block::STONE);
    let lava = ground(10, 8, 8) + IVec4::Y;
    g.world.set_block(lava, block::LAVA);
    hold(&mut g, Slot::new(item::BUCKET, 1));
    use_on(&mut g, lava, IVec4::Y);
    assert_eq!(g.world.block(lava), Some(AIR));
    assert_eq!(g.hotbar[0], Some(Slot::new(item::LAVA_BUCKET, 1)));
    // A solid neighbour on the clicked face refuses the pour.
    use_on(&mut g, ground(12, 8, 8), IVec4::W);
    assert_eq!(g.hotbar[0], Some(Slot::new(item::LAVA_BUCKET, 1)));
    let pillar = ground(12, 8, 8) + IVec4::Y;
    g.world.set_block(pillar, block::STONE);
    use_on(&mut g, pillar, IVec4::W);
    assert_eq!(g.world.block(pillar + IVec4::W), Some(block::LAVA), "poured into the +W neighbour");
    assert_eq!(g.hotbar[0], Some(Slot::new(item::BUCKET, 1)));
    // Flowing water is not a source; the empty bucket ignores it.
    let flow = ground(4, 4, 4) + IVec4::Y;
    g.world.set_block(flow, block::FluidKind::Water.flowing(3));
    use_on(&mut g, flow, IVec4::Y);
    assert_eq!(g.hotbar[0], Some(Slot::new(item::BUCKET, 1)));
    let water = ground(4, 12, 4) + IVec4::Y;
    g.world.set_block(water, block::WATER);
    use_on(&mut g, water, IVec4::Y);
    assert_eq!(g.hotbar[0], Some(Slot::new(item::WATER_BUCKET, 1)));
    // aw.a pours into any non-solid material, e.g. over a standing torch.
    let support = ground(5, 5, 5) + IVec4::Y;
    g.world.set_block(support, block::STONE);
    g.world.set_block_state(support + IVec4::X, BlockState::new(block::TORCH, 5));
    use_on(&mut g, support, IVec4::X);
    assert_eq!(g.world.block(support + IVec4::X), Some(block::WATER));
    assert_eq!(g.hotbar[0], Some(Slot::new(item::BUCKET, 1)));
    // Milk (milking itself is a mob interaction) empties on a block click.
    hold(&mut g, Slot::new(item::MILK, 1));
    use_on(&mut g, ground(6, 6, 6), IVec4::Y);
    assert_eq!(g.hotbar[0], Some(Slot::new(item::BUCKET, 1)));
}

#[test]
fn flint_and_steel_lights_the_face_wears_on_every_use_and_its_fire_primes_tnt() {
    let mut g = arena(block::STONE);
    let tnt = ground(10, 8, 8) + IVec4::Y;
    g.world.set_block(tnt, block::TNT);
    hold(&mut g, Slot::new(item::FLINT_AND_STEEL, 1));
    use_on(&mut g, tnt, IVec4::X);
    assert_eq!(g.world.block(tnt), Some(block::TNT), "wu.a never primes TNT directly");
    assert_eq!(g.world.block(tnt + IVec4::X), Some(block::FIRE));
    assert_eq!(g.hotbar[0].unwrap().damage, 1);
    use_on(&mut g, tnt, IVec4::NEG_Y);
    assert_eq!(g.hotbar[0].unwrap().damage, 2, "a blocked face still wears the tool");
    let mut primed = false;
    for _ in 0..4000 {
        g.update_fire(0.05);
        if g.world.block(tnt) != Some(block::TNT) { primed = true; break; }
    }
    assert!(primed, "the adjacent fire eventually burns the TNT");
    assert_eq!(g.device_draws().iter().filter(|d| d.kind == 17).count(), 1, "burnt TNT becomes primed TNT");
    assert!(g.dropped_items.iter().all(|d| d.stack.block != ItemId::from(block::TNT)));
    g.hotbar[0].as_mut().unwrap().damage = item::max_durability(item::FLINT_AND_STEEL);
    use_on(&mut g, ground(4, 4, 4), IVec4::Y);
    assert!(g.hotbar[0].is_none(), "worn out after 64 uses");
}

#[test]
fn breaking_tnt_by_hand_primes_it_instead_of_dropping_it() {
    let mut g = arena(block::STONE);
    let tnt = ground(10, 8, 8) + IVec4::Y;
    g.world.set_block(tnt, block::TNT);
    left_on(&mut g, tnt);
    assert_eq!(g.world.block(tnt), Some(AIR));
    assert_eq!(g.device_draws().iter().filter(|d| d.kind == 17).count(), 1);
    assert!(g.dropped_items.is_empty());
}

#[test]
fn bone_meal_ripens_crops_grows_each_sapling_species_and_is_always_consumed() {
    let mut g = arena(block::GRASS);
    g.body.pos = Vec4::new(8.5, FLOOR as f32 + 1.0, 2.5, 2.5);
    hold(&mut g, Slot::with_variant(item::DYE, 10, 15));
    let soil = ground(3, 3, 3);
    g.world.set_block(soil, block::FARMLAND);
    let crop = soil + IVec4::Y;
    g.world.set_block_state(crop, BlockState::new(block::CROPS, 0));
    use_on(&mut g, crop, IVec4::Y);
    assert_eq!(g.world.block_state(crop), Some(BlockState::new(block::CROPS, 7)));
    assert_eq!(g.hotbar[0].unwrap().count, 9);
    use_on(&mut g, crop, IVec4::Y);
    assert_eq!(g.hotbar[0].unwrap().count, 8, "js.a consumes the dye even on ripe crops");
    use_on(&mut g, ground(5, 5, 5), IVec4::Y);
    assert_eq!(g.hotbar[0].unwrap().count, 8, "grass is not fertilised in Beta 1.5");
    for (species, x) in [(0u8, 3), (1, 8), (2, 13)] {
        let sapling = ground(x, 12, 12) + IVec4::Y;
        g.world.set_block_state(sapling, BlockState::new(block::SAPLING, species));
        use_on(&mut g, sapling, IVec4::Y);
        assert_eq!(g.world.block_state(sapling), Some(BlockState::new(block::LOG, species)));
        let mut top = sapling;
        while g.world.block(top + IVec4::Y) == Some(block::LOG) { top += IVec4::Y; }
        assert_eq!(g.world.block_state(top + IVec4::Y), Some(BlockState::new(block::LEAVES, species)), "species {species}");
    }
    assert_eq!(g.hotbar[0].unwrap().count, 5);
}

#[test]
fn leaves_decay_after_the_trunk_goes_and_drop_only_matching_saplings() {
    let mut g = arena(block::GRASS);
    g.body.pos = Vec4::new(12.5, FLOOR as f32 + 1.0, 3.5, 3.5);
    // Canopies stay inside the loaded chunk so decay never waits on unknown cells.
    let trunks = [(0u8, ground(3, 12, 12)), (1, ground(12, 12, 12)), (2, ground(3, 3, 3))].map(|(species, below)| {
        let p = below + IVec4::Y;
        g.world.set_block_state(p, BlockState::new(block::SAPLING, species));
        assert!(g.grow_sapling(p, species));
        (species, p)
    });
    g.update_farming(0.0);
    for (_, p) in trunks {
        let mut y = p;
        while g.world.block(y) == Some(block::LOG) { g.world.set_block(y, AIR); y += IVec4::Y; }
    }
    let leaves = |g: &Game| (0..16).flat_map(|x| (0..16).flat_map(move |z| (0..16).flat_map(move |w| (4..16).map(move |y| IVec4::new(x, y, z, w)))))
        .filter(|&p| g.world.block(p) == Some(block::LEAVES)).count();
    let before = leaves(&g);
    assert!(before > 100);
    for round in 0..4000 {
        g.update_farming(1.0);
        if round % 50 == 0 && leaves(&g) == 0 { break; }
    }
    assert_eq!(leaves(&g), 0, "all {before} disconnected leaves decay");
    assert!(g.dropped_items.iter().all(|d| d.stack.block == ItemId::from(block::SAPLING)), "Beta 1.5 leaves drop no apples");
    for (species, p) in trunks {
        let centre = p.as_vec4() + Vec4::splat(0.5);
        let near: Vec<_> = g.dropped_items.iter().filter(|d| {
            let d = d.body.pos - centre;
            d.x.abs() < 3.0 && d.z.abs() < 3.0 && d.w.abs() < 3.0
        }).collect();
        assert!(!near.is_empty(), "species {species} dropped saplings");
        assert!(near.iter().all(|d| d.stack.variant == species));
    }
}

#[test]
fn dyes_from_flowers_and_bones_craft_coloured_wool() {
    let mut g = arena(block::STONE);
    g.hotbar[0] = Some(Slot::new(block::WOOL, 2));
    g.inventory[0] = Some(Slot::new(block::ROSE, 1));
    g.inventory[1] = Some(Slot::new(item::BONE, 1));
    assert_eq!(craft(&mut g, &["R"], &[('R', block::ROSE.into())]), Slot::with_variant(item::DYE, 2, 1));
    assert_eq!(craft(&mut g, &["DW"], &[('D', item::DYE), ('W', block::WOOL.into())]), Slot::with_variant(block::WOOL, 1, 14));
    assert_eq!(craft(&mut g, &["B"], &[('B', item::BONE)]), Slot::with_variant(item::DYE, 3, 15));
    assert!(g.hotbar.iter().chain(g.inventory.iter()).flatten().any(|s| *s == Slot::with_variant(block::WOOL, 1, 14)));
}

#[test]
fn snowballs_and_eggs_are_thrown_one_at_a_time_and_vanish_on_impact() {
    let mut g = arena(block::STONE);
    wall_ahead(&mut g);
    for id in [item::SNOWBALL, item::EGG] {
        hold(&mut g, Slot::new(id, 2));
        use_air(&mut g);
        assert_eq!(g.hotbar[0], Some(Slot::new(id, 1)));
        assert_eq!(thrown(&g, id), 1);
        for _ in 0..40 { g.update_devices(0.05); }
        assert_eq!(thrown(&g, id), 0, "{id} breaks on the wall");
        assert!(g.dropped_items.is_empty(), "thrown {id} leaves no item");
    }
}

#[test]
fn powered_dispenser_throws_snowballs_and_eggs_as_projectiles() {
    let mut g = arena(block::STONE);
    let d = ground(8, 4, 8) + IVec4::Y;
    g.world.set_block_state(d, BlockState::new(block::DISPENSER, 3));
    // Facing +Z into a backstop three blocks away.
    for w in -1..=1 { for y in 0..=2 { for x in -1..=1 { g.world.set_block(d + IVec4::new(x, y, 3, w), block::STONE); }}}
    let mut station = Station::new(d, StationKind::Dispenser);
    station.slots[0] = Slot::new(item::SNOWBALL, 1);
    station.slots[1] = Slot::new(item::EGG, 1);
    g.stations.push(station);
    let lever = d + IVec4::W;
    for (pulse, _) in [0, 1].iter().enumerate() {
        g.world.set_block_state(lever, BlockState::new(block::LEVER, 5 | 8));
        g.update_redstone(0.0);
        g.update_devices(0.0);
        for _ in 0..4 { g.update_redstone(0.05); g.update_devices(0.05); }
        assert_eq!(thrown(&g, item::SNOWBALL) + thrown(&g, item::EGG), 1, "pulse {pulse}");
        for _ in 0..60 { g.update_redstone(0.05); g.update_devices(0.05); }
        g.world.set_block_state(lever, BlockState::new(block::LEVER, 5));
        for _ in 0..4 { g.update_redstone(0.05); g.update_devices(0.05); }
    }
    assert!(g.stations[0].slots.iter().all(|s| s.count == 0));
    assert!(g.dropped_items.iter().all(|d| !matches!(d.stack.block, item::SNOWBALL | item::EGG)), "fired, not dropped");
}

#[test]
fn reeds_paper_books_sugar_cookies_bowls_and_stew_craft_and_cookies_heal_one() {
    let mut g = arena(block::STONE);
    g.inventory[0] = Some(Slot::new(item::SUGAR_CANE, 4));
    g.inventory[1] = Some(Slot::new(item::WHEAT, 2));
    g.inventory[2] = Some(Slot::with_variant(item::DYE, 1, 3));
    g.inventory[3] = Some(Slot::new(block::PLANKS, 3));
    g.inventory[4] = Some(Slot::new(block::BROWN_MUSHROOM, 1));
    g.inventory[5] = Some(Slot::new(block::RED_MUSHROOM, 1));
    open_table(&mut g);
    let (cane, paper, wheat, cocoa, plank, bowl): (ItemId, ItemId, ItemId, ItemId, ItemId, ItemId) =
        (item::SUGAR_CANE, item::PAPER, item::WHEAT, item::DYE, block::PLANKS.into(), item::BOWL);
    assert_eq!(craft(&mut g, &["RRR"], &[('R', cane)]), Slot::new(item::PAPER, 3));
    assert_eq!(craft(&mut g, &["P", "P", "P"], &[('P', paper)]), Slot::new(item::BOOK, 1));
    assert_eq!(craft(&mut g, &["R"], &[('R', cane)]), Slot::new(item::SUGAR, 1));
    assert_eq!(craft(&mut g, &["WCW"], &[('W', wheat), ('C', cocoa)]), Slot::new(item::COOKIE, 8));
    assert_eq!(craft(&mut g, &["P P", " P "], &[('P', plank)]), Slot::new(item::BOWL, 4));
    assert_eq!(craft(&mut g, &["B", "R", "O"], &[('B', block::BROWN_MUSHROOM.into()), ('R', block::RED_MUSHROOM.into()), ('O', bowl)]),
        Slot::new(item::MUSHROOM_STEW, 1));
    g.close_station();
    g.health = 10;
    select(&mut g, item::COOKIE);
    g.target = None;
    click(&mut g, MouseBtn::Right);
    assert_eq!(g.health, 11);
    assert_eq!(count(&g, item::COOKIE), 7);
}

#[test]
fn cake_needs_ground_and_either_click_eats_a_slice_only_when_hurt() {
    let mut g = arena(block::STONE);
    hold(&mut g, Slot::new(item::CAKE_ITEM, 2));
    let wall = ground(12, 8, 8) + IVec4::Y * 2;
    g.world.set_block(wall, block::STONE);
    use_on(&mut g, wall, IVec4::X);
    assert_eq!(g.world.block(wall + IVec4::X), Some(AIR), "no cake on air");
    let cake = ground(10, 8, 8) + IVec4::Y;
    use_on(&mut g, ground(10, 8, 8), IVec4::Y);
    assert_eq!(g.world.block_state(cake), Some(BlockState::new(block::CAKE, 0)));
    assert_eq!(g.hotbar[0].unwrap().count, 1);
    g.hotbar[0] = None;
    use_on(&mut g, cake, IVec4::Y);
    assert_eq!(g.world.block_state(cake).unwrap().metadata, 0, "th.c does nothing at full health");
    g.health = 5;
    use_on(&mut g, cake, IVec4::Y);
    assert_eq!((g.health, g.world.block_state(cake).unwrap().metadata), (8, 1));
    left_on(&mut g, cake);
    assert_eq!((g.health, g.world.block_state(cake).unwrap().metadata), (11, 2), "th.b: a left click also eats");
    g.health = 1;
    for _ in 0..3 { use_on(&mut g, cake, IVec4::Y); }
    assert_eq!(g.world.block_state(cake), Some(BlockState::new(block::CAKE, 5)));
    use_on(&mut g, cake, IVec4::Y);
    assert_eq!(g.world.block(cake), Some(AIR), "the sixth slice removes the cake");
    assert_eq!(g.health, 13);
    assert!(g.dropped_items.is_empty());
}

#[test]
fn pumpkins_need_a_normal_cube_below_and_face_the_look_direction() {
    let mut g = arena(block::STONE);
    let wall = ground(12, 8, 8) + IVec4::Y * 2;
    g.world.set_block(wall, block::STONE);
    hold(&mut g, Slot::new(block::PUMPKIN, 8));
    use_on(&mut g, wall, IVec4::X);
    assert_eq!(g.world.block(wall + IVec4::X), Some(AIR), "es.a requires a normal cube below");
    for (turn, x) in [(0, 4), (1, 6), (2, 10), (3, 12)] {
        if turn > 0 { g.orient.yaw(std::f32::consts::FRAC_PI_2); }
        let below = ground(x, 4, 4);
        use_on(&mut g, below, IVec4::Y);
        let state = g.world.block_state(below + IVec4::Y).unwrap();
        assert_eq!(state.id, block::PUMPKIN);
        let front = match state.metadata { 0 => Vec4::Z, 1 => Vec4::NEG_X, 2 => Vec4::NEG_Z, 3 => Vec4::X, m => panic!("metadata {m}") };
        assert!(front.dot(g.basis().forward_h) > 0.5, "turn {turn}: metadata {}", state.metadata);
    }
    g.inventory[0] = Some(Slot::new(block::TORCH, 1));
    g.hotbar[0] = Some(Slot::new(block::PUMPKIN, 1));
    let lantern = craft(&mut g, &["P", "T"], &[('P', block::PUMPKIN.into()), ('T', block::TORCH.into())]);
    assert_eq!(lantern, Slot::new(block::JACK_O_LANTERN, 1));
    select(&mut g, block::JACK_O_LANTERN);
    use_on(&mut g, ground(8, 12, 12), IVec4::Y);
    let placed = g.world.block_state(ground(8, 12, 12) + IVec4::Y).unwrap();
    assert_eq!(placed.id, block::JACK_O_LANTERN);
    assert_eq!(block::light_emission(placed), 15);
}

#[test]
fn fences_need_solid_material_below_and_never_stack_directly() {
    let mut g = arena(block::STONE);
    hold(&mut g, Slot::new(block::FENCE, 8));
    use_on(&mut g, ground(10, 8, 8), IVec4::Y);
    let first = ground(10, 8, 8) + IVec4::Y;
    assert_eq!(g.world.block(first), Some(block::FENCE));
    use_on(&mut g, first, IVec4::Y);
    assert_eq!(g.world.block(first + IVec4::Y), Some(AIR), "jc.a rejects a fence directly on a fence");
    let glass = ground(5, 5, 5) + IVec4::Y;
    g.world.set_block(glass, block::GLASS);
    use_on(&mut g, glass, IVec4::Y);
    assert_eq!(g.world.block(glass + IVec4::Y), Some(block::FENCE), "glass is a solid material");
    let side = ground(12, 12, 12) + IVec4::Y * 2;
    g.world.set_block(side, block::STONE);
    use_on(&mut g, side, IVec4::W);
    assert_eq!(g.world.block(side + IVec4::W), Some(AIR));
    assert_eq!(g.hotbar[0].unwrap().count, 6);
}

#[test]
fn stairs_ascend_away_from_the_placer() {
    let mut g = arena(block::STONE);
    hold(&mut g, Slot::new(block::WOOD_STAIRS, 4));
    for (turn, x) in [(0, 4), (1, 6), (2, 10), (3, 12)] {
        if turn > 0 { g.orient.yaw(std::f32::consts::FRAC_PI_2); }
        use_on(&mut g, ground(x, 4, 4), IVec4::Y);
        let state = g.world.block_state(ground(x, 4, 4) + IVec4::Y).unwrap();
        let high = match state.metadata { 0 => Vec4::X, 1 => Vec4::NEG_X, 2 => Vec4::Z, 3 => Vec4::NEG_Z, m => panic!("metadata {m}") };
        assert!(high.dot(g.basis().forward_h) > 0.5, "turn {turn}");
    }
}

#[test]
fn iron_doors_swallow_right_clicks_while_wooden_doors_toggle() {
    let mut g = arena(block::STONE);
    hold(&mut g, Slot::new(item::IRON_DOOR_ITEM, 1));
    use_on(&mut g, ground(10, 8, 8), IVec4::Y);
    let iron = ground(10, 8, 8) + IVec4::Y;
    assert_eq!(g.world.block(iron), Some(block::IRON_DOOR));
    assert_eq!(g.world.block(iron + IVec4::Y), Some(block::IRON_DOOR));
    hold(&mut g, Slot::new(block::DIRT, 4));
    for normal in [IVec4::NEG_X, IVec4::X, IVec4::Z, IVec4::W] { use_on(&mut g, iron, normal); }
    assert_eq!(g.hotbar[0].unwrap().count, 4, "ki.a consumes the click: nothing is built against an iron door");
    assert_eq!(g.world.block_state(iron).unwrap().metadata & 4, 0, "only redstone opens it");
    hold(&mut g, Slot::new(item::WOOD_DOOR_ITEM, 1));
    use_on(&mut g, ground(6, 8, 8), IVec4::Y);
    let wood = ground(6, 8, 8) + IVec4::Y;
    g.hotbar[0] = None;
    use_on(&mut g, wood + IVec4::Y, IVec4::X);
    assert_eq!(g.world.block_state(wood).unwrap().metadata & 4, 4);
    use_on(&mut g, wood, IVec4::X);
    assert_eq!(g.world.block_state(wood).unwrap().metadata & 4, 0);
}

#[test]
fn note_block_left_click_plays_its_pitch_without_retuning() {
    let mut g = arena(block::STONE);
    let note = ground(10, 8, 8) + IVec4::Y;
    g.world.set_block_state(note, BlockState::new(block::NOTE_BLOCK, 12));
    left_on(&mut g, note);
    let sound = g.sounds.iter().find(|s| s.name.starts_with("note.")).expect("note sound");
    assert_eq!(sound.name, "note.bd", "stone below selects the bass drum");
    assert!((sound.pitch - 1.0).abs() < 1e-6);
    assert_eq!(g.world.block_state(note).unwrap().metadata, 12);
    g.sounds.clear();
    use_on(&mut g, note, IVec4::Y);
    assert_eq!(g.world.block_state(note).unwrap().metadata, 13, "right click tunes up a semitone");
    assert!(g.sounds.iter().any(|s| s.name == "note.bd"));
}

#[test]
fn glowstone_drops_one_dust_and_obsidian_needs_a_diamond_pickaxe() {
    let mut g = arena(block::STONE);
    let glow = ground(10, 8, 8) + IVec4::Y;
    g.world.set_block(glow, block::GLOW);
    g.mine_block(Hit { block: glow, normal: IVec4::Y, dist: 2.0 }, 100.0);
    assert_eq!(g.dropped_items.iter().map(|d| (d.stack.block, d.stack.count)).collect::<Vec<_>>(), vec![(item::GLOWSTONE_DUST, 1)]);
    g.dropped_items.clear();
    for (pick, drops) in [(item::IRON_PICKAXE, false), (item::DIAMOND_PICKAXE, true)] {
        hold(&mut g, Slot::new(pick, 1));
        let obsidian = ground(6, 8, 8) + IVec4::Y;
        g.world.set_block(obsidian, block::OBSIDIAN);
        g.mine_block(Hit { block: obsidian, normal: IVec4::Y, dist: 2.0 }, 1000.0);
        assert_eq!(g.world.block(obsidian), Some(AIR));
        assert_eq!(g.dropped_items.iter().any(|d| d.stack.block == ItemId::from(block::OBSIDIAN)), drops, "{pick}");
        g.dropped_items.clear();
    }
}

#[test]
fn minecart_items_go_on_rails_and_carts_ride_open_cargo_or_burn_coal() {
    let mut g = arena(block::STONE);
    for (kind, id) in [(VehicleKind::Minecart, item::MINECART), (VehicleKind::ChestCart, item::CHEST_MINECART), (VehicleKind::FurnaceCart, item::FURNACE_MINECART)] {
        g.vehicles.saved.entities.clear();
        g.vehicles.saved.riding = None;
        g.vehicles.active_cargo = None;
        let rail = ground(10, 8, 8) + IVec4::Y;
        g.world.set_block(rail, AIR);
        hold(&mut g, Slot::new(id, 1));
        use_on(&mut g, ground(12, 8, 8), IVec4::Y);
        assert!(g.vehicles.saved.entities.is_empty(), "carts need a rail");
        assert_eq!(g.hotbar[0], Some(Slot::new(id, 1)));
        g.hotbar[1] = Some(Slot::new(block::RAIL, 1));
        g.selected = 1;
        use_on(&mut g, ground(10, 8, 8), IVec4::Y);
        assert_eq!(g.world.block(rail), Some(block::RAIL));
        g.selected = 0;
        use_on(&mut g, rail, IVec4::Y);
        assert_eq!(g.vehicles.saved.entities.len(), 1);
        assert_eq!(g.vehicles.saved.entities[0].kind, kind);
        assert!(g.hotbar[0].is_none());
        // Hold the cart in front of the eye so the click ray meets it.
        g.vehicles.saved.entities[0].position = g.eye() + g.basis().forward * 2.0 - Vec4::Y * 0.35;
        hold(&mut g, Slot::new(item::COAL, 2));
        use_air(&mut g);
        match kind {
            VehicleKind::Minecart => assert_eq!(g.vehicles.saved.riding, Some(0)),
            VehicleKind::ChestCart => assert_eq!(g.vehicles.active_cargo, Some(0)),
            _ => {
                assert_eq!(g.vehicles.saved.entities[0].fuel, 1200, "xb.a: each coal adds 1200 ticks");
                assert_eq!(g.hotbar[0], Some(Slot::new(item::COAL, 1)));
            }
        }
    }
}

#[test]
fn sugar_cane_plants_beside_water_including_w_and_stacks_on_itself() {
    let mut g = arena(block::GRASS);
    hold(&mut g, Slot::new(item::SUGAR_CANE, 4));
    use_on(&mut g, ground(4, 4, 4), IVec4::Y);
    assert_eq!(g.world.block(ground(4, 4, 4) + IVec4::Y), Some(AIR), "fd.a needs water beside the soil");
    g.world.set_block(ground(12, 12, 13), block::WATER);
    use_on(&mut g, ground(12, 12, 12), IVec4::Y);
    let cane = ground(12, 12, 12) + IVec4::Y;
    assert_eq!(g.world.block(cane), Some(block::REEDS), "water one step along W counts");
    use_on(&mut g, cane, IVec4::Y);
    assert_eq!(g.world.block(cane + IVec4::Y), Some(block::REEDS));
    assert_eq!(g.hotbar[0].unwrap().count, 2);
}
