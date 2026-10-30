//! Beta1.5 xg fire rules, including its much faster age15 burn stage.
//! Fuel values and10-tick schedule are taken directly from the original class.
use crate::{
    block::{self, BlockState},
    redstone::NEIGHBORS,
    shapes::HORIZONTAL_DIRECTIONS,
};
use glam::IVec4;
pub trait FireWorld {
    fn get(&self, p: IVec4) -> Option<BlockState>;
    fn set(&mut self, p: IVec4, state: BlockState);
    fn wet(&self, p: IVec4) -> bool;
    fn prime_tnt(&mut self, p: IVec4);
}
/// (encouragement, flammability); many later flammable blocks intentionally are
/// absent: Beta1.5 registered only these six fuel families.
pub fn fuel(id: u8) -> (u32, u32) {
    match id {
        block::PLANKS => (5, 20),
        block::LOG => (5, 5),
        block::LEAVES => (30, 60),
        block::BOOKSHELF => (30, 20),
        block::TNT => (15, 100),
        block::WOOL => (30, 60),
        _ => (0, 0),
    }
}
pub fn supported(p: IVec4, get: &impl Fn(IVec4) -> Option<BlockState>) -> Option<bool> {
    let mut unknown = false;
    if let Some(s) = get(p - IVec4::Y) {
        if block::definition(s.id).opaque {
            return Some(true);
        }
    } else {
        unknown = true
    }
    for d in NEIGHBORS {
        match get(p + d) {
            Some(s) if fuel(s.id).0 > 0 => return Some(true),
            None => unknown = true,
            _ => {}
        }
    }
    if unknown { None } else { Some(false) }
}
fn neighboring_fuel(p: IVec4, world: &impl FireWorld) -> u32 {
    NEIGHBORS
        .into_iter()
        .filter_map(|d| world.get(p + d).map(|s| fuel(s.id).0))
        .max()
        .unwrap_or(0)
}
fn burn(p: IVec4, chance: u32, world: &mut impl FireWorld, roll: &mut impl FnMut(u32) -> u32) {
    let Some(state) = world.get(p) else { return };
    if roll(chance) % chance >= fuel(state.id).1 {
        return;
    }
    let ignite = roll(2) % 2 == 0 && !world.wet(p);
    if state.id == block::TNT {
        world.prime_tnt(p);
    }
    world.set(
        p,
        if ignite {
            block::FIRE.into()
        } else {
            block::AIR.into()
        },
    );
}
/// Returns whether a10-tick scheduled callback remains necessary; age15 fires
/// still receive the world's ordinary random ticks.
pub fn tick(p: IVec4, world: &mut impl FireWorld, roll: &mut impl FnMut(u32) -> u32) -> bool {
    let Some(state) = world.get(p).filter(|s| s.id == block::FIRE) else {
        return false;
    };
    let age = state.metadata.min(15);
    let eternal = world
        .get(p - IVec4::Y)
        .is_some_and(|s| s.id == block::NETHERRACK);
    if !eternal && (world.wet(p) || HORIZONTAL_DIRECTIONS.into_iter().any(|d| world.wet(p + d))) {
        world.set(p, block::AIR.into());
        return false;
    }
    if age < 15 {
        world.set(p, BlockState::new(block::FIRE, age + 1));
    }
    if !eternal && neighboring_fuel(p, world) == 0 {
        if !world
            .get(p - IVec4::Y)
            .is_some_and(|s| block::definition(s.id).opaque)
            || age > 3
        {
            world.set(p, block::AIR.into());
        }
        return age < 15;
    }
    if !eternal
        && world.get(p - IVec4::Y).is_some_and(|s| fuel(s.id).0 == 0)
        && age == 15
        && roll(4) % 4 == 0
    {
        world.set(p, block::AIR.into());
        return false;
    }
    if age % 2 == 0 && age > 2 {
        for d in NEIGHBORS {
            burn(p + d, if d.y != 0 { 250 } else { 300 }, world, roll);
        }
        for w in -1..=1 {
            for z in -1..=1 {
                for x in -1..=1 {
                    for y in -1..=4 {
                        let q = p + IVec4::new(x, y, z, w);
                        if q == p || world.get(q).is_none_or(|s| s.id != block::AIR) {
                            continue;
                        }
                        let encouragement = neighboring_fuel(q, world);
                        let chance = if y > 1 { y as u32 * 100 } else { 100 };
                        if encouragement > 0
                            && roll(chance) % chance <= encouragement
                            && !world.wet(q)
                            && HORIZONTAL_DIRECTIONS.into_iter().all(|d| !world.wet(q + d))
                        {
                            world.set(q, block::FIRE.into());
                        }
                    }
                }
            }
        }
    }
    if age == 15 {
        for d in NEIGHBORS {
            burn(p + d, 1, world, roll);
        }
    }
    age < 15
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[derive(Default)]
    struct TestWorld {
        cells: HashMap<IVec4, BlockState>,
        rain: bool,
        primed: Vec<IVec4>,
    }
    impl FireWorld for TestWorld {
        fn get(&self, p: IVec4) -> Option<BlockState> {
            Some(*self.cells.get(&p).unwrap_or(&block::AIR.into()))
        }
        fn set(&mut self, p: IVec4, s: BlockState) {
            self.cells.insert(p, s);
        }
        fn wet(&self, _: IVec4) -> bool {
            self.rain
        }
        fn prime_tnt(&mut self, p: IVec4) {
            self.primed.push(p);
        }
    }
    #[test]
    fn fire_ages_on_stone_extinguishes_and_rain_respects_netherrack() {
        let mut w = TestWorld::default();
        w.set(IVec4::ZERO, block::FIRE.into());
        w.set(IVec4::NEG_Y, block::STONE.into());
        for _ in 0..5 {
            tick(IVec4::ZERO, &mut w, &mut |_| 0);
        }
        assert_eq!(w.get(IVec4::ZERO).unwrap().id, block::AIR);
        w.set(IVec4::ZERO, block::FIRE.into());
        w.rain = true;
        tick(IVec4::ZERO, &mut w, &mut |_| 0);
        assert_eq!(w.get(IVec4::ZERO).unwrap().id, block::AIR);
        w.set(IVec4::ZERO, block::FIRE.into());
        w.set(IVec4::NEG_Y, block::NETHERRACK.into());
        tick(IVec4::ZERO, &mut w, &mut |_| 0);
        assert_eq!(w.get(IVec4::ZERO).unwrap().id, block::FIRE);
    }
    #[test]
    fn age15_burns_fuel_in_w_and_primes_tnt_without_drops() {
        let mut w = TestWorld::default();
        w.set(IVec4::ZERO, BlockState::new(block::FIRE, 15));
        w.set(IVec4::NEG_Y, block::NETHERRACK.into());
        w.set(IVec4::W, block::TNT.into());
        w.set(IVec4::NEG_W, block::LOG.into());
        tick(IVec4::ZERO, &mut w, &mut |n| n - 1);
        assert_eq!(w.primed, vec![IVec4::W]);
        assert_eq!(w.get(IVec4::W).unwrap().id, block::AIR);
        assert_eq!(w.get(IVec4::NEG_W).unwrap().id, block::AIR);
        assert_eq!(fuel(block::CHEST), (0, 0));
        assert_eq!(fuel(block::WOOD_STAIRS), (0, 0));
    }
}
