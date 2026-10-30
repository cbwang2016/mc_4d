//! Original eh dungeon rooms/loot, extended across W. Spawners are deliberately
//! excluded. Plans depend only on base terrain, never on generation order.
use crate::{
    block,
    item::{self, Stack},
    shapes::HORIZONTAL_DIRECTIONS,
};
use glam::IVec4;
use std::collections::HashMap;
#[derive(Clone, Debug)]
pub struct Candidate {
    pub center: IVec4,
    pub radius: IVec4,
    state: u64,
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub blocks: Vec<(IVec4, u8)>,
    pub chests: Vec<(IVec4, Vec<Stack>)>,
}
#[derive(Clone)]
struct Random(u64);
impl Random {
    fn seeded(seed: u64) -> Self {
        Self((seed ^ 0x5deece66d) & ((1 << 48) - 1))
    }
    fn bits(&mut self, n: u32) -> u32 {
        self.0 = (self.0.wrapping_mul(0x5deece66d).wrapping_add(11)) & ((1 << 48) - 1);
        (self.0 >> (48 - n)) as u32
    }
    fn next(&mut self, n: u32) -> u32 {
        loop {
            let b = self.bits(31);
            if n.is_power_of_two() {
                return ((n as u64 * b as u64) >> 31) as u32;
            }
            let v = b % n;
            if b.wrapping_sub(v).wrapping_add(n - 1) < 0x80000000 {
                return v;
            }
        }
    }
}
pub fn candidate(seed: u32, column: IVec4, attempt: u8) -> Candidate {
    let mixed = (seed as u64)
        ^ (column.x as i64 as u64).wrapping_mul(341873128712)
        ^ (column.z as i64 as u64).wrapping_mul(132897987541)
        ^ (column.w as i64 as u64).wrapping_mul(198491317)
        ^ (attempt as u64).wrapping_mul(0x9e3779b97f4a7c15);
    let mut rng = Random::seeded(mixed);
    let center = IVec4::new(
        column.x * 16 + rng.next(16) as i32,
        rng.next(128) as i32,
        column.z * 16 + rng.next(16) as i32,
        column.w * 16 + rng.next(16) as i32,
    );
    let radius = IVec4::new(
        2 + rng.next(2) as i32,
        0,
        2 + rng.next(2) as i32,
        2 + rng.next(2) as i32,
    );
    Candidate {
        center,
        radius,
        state: rng.0,
    }
}
impl Candidate {
    pub fn bounds(&self) -> (IVec4, IVec4) {
        (
            self.center - self.radius - IVec4::ONE,
            self.center + self.radius + IVec4::new(1, 4, 1, 1),
        )
    }
    pub fn intersects_chunk(&self, c: IVec4) -> bool {
        let (lo, hi) = self.bounds();
        let start = c * 16;
        lo.cmple(start + IVec4::splat(15)).all() && hi.cmpge(start).all()
    }
}
fn solid(id: u8) -> bool {
    block::is_solid(id)
}
/// Exact11-way eh.a(Random) item table, including failed rare rolls and cocoa.
pub fn loot(roll: &mut impl FnMut(u32) -> u32) -> Option<Stack> {
    Some(match roll(11) % 11 {
        0 => Stack::new(item::SADDLE, 1),
        1 => Stack::new(item::IRON_INGOT, 1 + roll(4) % 4),
        2 => Stack::new(item::BREAD, 1),
        3 => Stack::new(item::WHEAT, 1 + roll(4) % 4),
        4 => Stack::new(item::GUNPOWDER, 1 + roll(4) % 4),
        5 => Stack::new(item::STRING, 1 + roll(4) % 4),
        6 => Stack::new(item::BUCKET, 1),
        7 => {
            if roll(100) % 100 != 0 {
                return None;
            }
            Stack::new(item::GOLDEN_APPLE, 1)
        }
        8 => {
            if roll(2) % 2 != 0 {
                return None;
            }
            Stack::new(item::REDSTONE, 1 + roll(4) % 4)
        }
        9 => {
            if roll(10) % 10 != 0 {
                return None;
            }
            Stack::new(item::RECORD_13 + (roll(2) % 2) as u16, 1)
        }
        _ => Stack::with_variant(item::DYE, 1, 3),
    })
}
pub fn plan(c: &Candidate, get: &mut impl FnMut(IVec4) -> u8) -> Option<Plan> {
    let (lo, hi) = c.bounds();
    if lo.y < 0 || hi.y >= 128 {
        return None;
    }
    let mut openings = 0;
    for w in lo.w..=hi.w {
        for z in lo.z..=hi.z {
            for x in lo.x..=hi.x {
                if !solid(get(IVec4::new(x, lo.y, z, w))) || !solid(get(IVec4::new(x, hi.y, z, w)))
                {
                    return None;
                }
                if x == lo.x || x == hi.x || z == lo.z || z == hi.z || w == lo.w || w == hi.w {
                    let p = IVec4::new(x, c.center.y, z, w);
                    if get(p) == block::AIR && get(p + IVec4::Y) == block::AIR {
                        openings += 1;
                    }
                }
            }
        }
    }
    // Preserve the original1..5 doorway-width allowance across the extra W span.
    if openings == 0 || openings > 5 * (2 * c.radius.w + 1) {
        return None;
    }
    let mut rng = Random(c.state);
    let mut edits = HashMap::new();
    for w in lo.w..=hi.w {
        for x in lo.x..=hi.x {
            for y in (lo.y..hi.y).rev() {
                for z in lo.z..=hi.z {
                    let p = IVec4::new(x, y, z, w);
                    let wall = x == lo.x
                        || x == hi.x
                        || z == lo.z
                        || z == hi.z
                        || w == lo.w
                        || w == hi.w
                        || y == lo.y;
                    if !wall || !solid(get(p - IVec4::Y)) {
                        edits.insert(p, block::AIR);
                    } else if solid(get(p)) {
                        edits.insert(
                            p,
                            if y == lo.y && rng.next(4) != 0 {
                                block::MOSS_STONE
                            } else {
                                block::COBBLE
                            },
                        );
                    }
                }
            }
        }
    }
    let mut chests = Vec::new();
    for _ in 0..2 {
        for _ in 0..3 {
            let p = c.center
                + IVec4::new(
                    rng.next((2 * c.radius.x + 1) as u32) as i32 - c.radius.x,
                    0,
                    rng.next((2 * c.radius.z + 1) as u32) as i32 - c.radius.z,
                    rng.next((2 * c.radius.w + 1) as u32) as i32 - c.radius.w,
                );
            let mut at = |p| edits.get(&p).copied().unwrap_or_else(|| get(p));
            if at(p) != block::AIR
                || HORIZONTAL_DIRECTIONS
                    .into_iter()
                    .filter(|&d| solid(at(p + d)))
                    .count()
                    != 1
            {
                continue;
            }
            edits.insert(p, block::CHEST);
            let mut slots = vec![Stack::EMPTY; 27];
            for _ in 0..8 {
                if let Some(stack) = loot(&mut |n| rng.next(n)) {
                    let index = rng.next(27) as usize;
                    slots[index] = stack;
                }
            }
            chests.push((p, slots));
            break;
        }
    }
    let mut blocks: Vec<_> = edits.into_iter().collect();
    blocks.sort_by_key(|(p, _)| p.to_array());
    Some(Plan { blocks, chests })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_loot_has_rare_failures_and_original_cocoa_metadata() {
        let mut values = vec![7, 1].into_iter();
        assert!(loot(&mut |_| values.next().unwrap()).is_none());
        let mut values = vec![9, 0, 1].into_iter();
        assert_eq!(
            loot(&mut |_| values.next().unwrap()),
            Some(Stack::new(item::RECORD_CAT, 1))
        );
        assert_eq!(
            loot(&mut |_| 10),
            Some(Stack::with_variant(item::DYE, 1, 3))
        );
    }
    #[test]
    fn room_requires_supported_enclosure_and_an_opening_and_excludes_spawners() {
        let c = Candidate {
            center: IVec4::new(0, 20, 0, 0),
            radius: IVec4::new(2, 0, 2, 2),
            state: 123,
        };
        let (lo, _) = c.bounds();
        let door = IVec4::new(lo.x, 20, 0, 0);
        assert!(plan(&c, &mut |_| block::STONE).is_none());
        let get = |p| {
            if p == door || p == door + IVec4::Y {
                block::AIR
            } else {
                block::STONE
            }
        };
        let a = plan(&c, &mut |p| get(p)).unwrap();
        let b = plan(&c, &mut |p| get(p)).unwrap();
        assert_eq!(a.blocks, b.blocks);
        assert_eq!(a.chests, b.chests);
        assert!(a.blocks.iter().any(|(_, id)| *id == block::MOSS_STONE));
        assert_eq!(
            a.blocks.iter().find(|(p, _)| *p == c.center).unwrap().1,
            block::AIR
        );
        assert!(a.chests.len() <= 2);
        assert!(a.chests.iter().all(|(_, s)| s.len() == 27));
        assert!(plan(&c, &mut |p| if p.y == lo.y { block::AIR } else { get(p) }).is_none());
    }
}
