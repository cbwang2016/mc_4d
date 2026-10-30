//! Original Beta chest topology: at most two horizontal neighbors, opaque blocks
//! above either half prevent opening. W extends the same rules as X/Z.
use crate::{
    block::{self, BlockState},
    shapes::HORIZONTAL_DIRECTIONS,
};
use glam::IVec4;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChestPair {
    pub first: IVec4,
    pub second: Option<IVec4>,
}
impl ChestPair {
    pub fn positions(self) -> impl Iterator<Item = IVec4> {
        std::iter::once(self.first).chain(self.second)
    }
}
fn neighbors(p: IVec4, get: &impl Fn(IVec4) -> Option<BlockState>) -> Option<Vec<IVec4>> {
    let mut found = Vec::with_capacity(2);
    for d in HORIZONTAL_DIRECTIONS {
        if get(p + d)?.id == block::CHEST {
            found.push(p + d);
        }
    }
    Some(found)
}
pub fn can_place_chest(p: IVec4, get: &impl Fn(IVec4) -> Option<BlockState>) -> bool {
    let Some(adjacent) = neighbors(p, get) else {
        return false;
    };
    if adjacent.len() > 1 {
        return false;
    }
    adjacent
        .into_iter()
        .all(|q| neighbors(q, get).is_some_and(|v| v.is_empty()))
}
pub fn chest_pair(p: IVec4, get: &impl Fn(IVec4) -> Option<BlockState>) -> Option<ChestPair> {
    if get(p)?.id != block::CHEST {
        return None;
    }
    let adjacent = neighbors(p, get)?;
    if adjacent.is_empty() {
        return Some(ChestPair {
            first: p,
            second: None,
        });
    }
    if adjacent.len() != 1 {
        return None;
    }
    let other = adjacent[0];
    let others = neighbors(other, get)?;
    if others.as_slice() != [p] {
        return None;
    }
    let (first, second) = if [p.x, p.z, p.w] < [other.x, other.z, other.w] {
        (p, other)
    } else {
        (other, p)
    };
    Some(ChestPair {
        first,
        second: Some(second),
    })
}
pub fn can_open_chest(pair: ChestPair, get: &impl Fn(IVec4) -> Option<BlockState>) -> bool {
    pair.positions()
        .all(|p| get(p + IVec4::Y).is_some_and(|s| !block::definition(s.id).opaque))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[test]
    fn pair_order_is_stable_from_either_half_on_every_horizontal_axis() {
        for d in HORIZONTAL_DIRECTIONS {
            let p = IVec4::ZERO;
            let m = HashMap::from([(p, block::CHEST.into()), (p + d, block::CHEST.into())]);
            let get = |q| Some(*m.get(&q).unwrap_or(&block::AIR.into()));
            let pair = chest_pair(p, &get).unwrap();
            assert_eq!(chest_pair(p + d, &get), Some(pair));
            assert!(
                [pair.first.x, pair.first.z, pair.first.w]
                    < [
                        pair.second.unwrap().x,
                        pair.second.unwrap().z,
                        pair.second.unwrap().w
                    ]
            );
        }
    }
    #[test]
    fn triples_corners_and_opaque_partner_roofs_are_rejected() {
        let mut m = HashMap::from([
            (IVec4::ZERO, block::CHEST.into()),
            (IVec4::X, block::CHEST.into()),
        ]);
        let get = |q| Some(*m.get(&q).unwrap_or(&block::AIR.into()));
        assert!(!can_place_chest(IVec4::X * 2, &get));
        assert!(!can_place_chest(IVec4::W, &get));
        m.insert(IVec4::X + IVec4::Y, block::STONE.into());
        let get = |q| Some(*m.get(&q).unwrap_or(&block::AIR.into()));
        assert!(!can_open_chest(
            chest_pair(IVec4::ZERO, &get).unwrap(),
            &get
        ));
        m.insert(IVec4::X + IVec4::Y, block::GLASS.into());
        let get = |q| Some(*m.get(&q).unwrap_or(&block::AIR.into()));
        assert!(can_open_chest(chest_pair(IVec4::ZERO, &get).unwrap(), &get));
    }
}
