//! Small original environmental rules, with horizontal offsets extended to W.
use crate::{
    block::{self, BlockState},
    redstone::NEIGHBORS,
};
use glam::IVec4;
/// Includes solid snow only to register cold weather columns; it is not a
/// melting random-tick target. Flowing lava IDs share stationary material rules.
pub const ACTIVE_BLOCKS: &[u8] = &[
    block::GRASS,
    block::SNOW,
    block::SNOW_LAYER,
    block::ICE,
    block::DANDELION,
    block::ROSE,
    block::BROWN_MUSHROOM,
    block::RED_MUSHROOM,
    block::LAVA,
    block::LAVA_FLOW_START,
    block::LAVA_FLOW_START + 1,
    block::LAVA_FLOW_START + 2,
    block::LAVA_FLOW_START + 3,
    block::LAVA_FLOW_START + 4,
    block::LAVA_FLOW_START + 5,
    block::LAVA_FLOW_START + 6,
    block::LAVA_FLOW_START + 7,
];
/// nb.a(world,xyz,Random): choose0–2 upward random-walk steps. Unlike later
/// versions, a zero-step roll does not attempt three fires on the same level.
pub fn lava_ignition(
    origin: IVec4,
    get: &impl Fn(IVec4) -> Option<BlockState>,
    roll: &mut impl FnMut(u32) -> u32,
) -> Option<IVec4> {
    let mut p = origin;
    for _ in 0..roll(3) % 3 {
        p += IVec4::new(
            roll(3) as i32 - 1,
            1,
            roll(3) as i32 - 1,
            roll(3) as i32 - 1,
        );
        let state = get(p)?;
        if state.id == block::AIR {
            if NEIGHBORS
                .iter()
                .any(|&d| get(p + d).is_some_and(|s| flammable_material(s.id)))
            {
                return Some(p);
            }
        } else if material_blocks_movement(state.id) {
            return None;
        }
    }
    None
}
/// kr.e() tests the material, including wooden mechanisms. This deliberately
/// differs from the six fuel families used by the spreading-fire burn table.
pub fn flammable_material(id: u8) -> bool {
    // Directly reflected kr.e(): note blocks, beds, webs and jukeboxes matter
    // even though the ordinary spreading-fire table does not burn all of them.
    matches!(
        block::definition(id).reference_id,
        5 | 17 | 18 | 25 | 26 | 30 | 35 | 46 | 47 | 53 | 54 | 58 | 63 | 64 | 68 | 84 | 85 | 95
    )
}
pub fn material_blocks_movement(id: u8) -> bool {
    id != block::AIR
        && !matches!(
            block::definition(id).reference_id,
            6 | 8
                | 9
                | 10
                | 11
                | 27
                | 28
                | 37
                | 38
                | 39
                | 40
                | 50
                | 51
                | 55
                | 59
                | 65
                | 66
                | 69
                | 75
                | 76
                | 77
                | 78
                | 83
                | 93
                | 94
        )
}
pub fn snow_can_form(air: BlockState, below: BlockState, block_light: u8) -> bool {
    air.id == block::AIR
        && block_light < 10
        && block::definition(below.id).opaque
        && material_blocks_movement(below.id)
        && below.id != block::ICE
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lava_walk_ignites_beside_wood_in_w_but_zero_steps_and_stone_stop_it() {
        let origin = IVec4::ZERO;
        let fuel = IVec4::Y + IVec4::W;
        let get = |p| {
            Some(BlockState::new(
                if p == fuel { block::PLANKS } else { block::AIR },
                0,
            ))
        };
        let mut rolls = vec![1, 1, 1, 1].into_iter();
        assert_eq!(
            lava_ignition(origin, &get, &mut |_| rolls.next().unwrap()),
            Some(IVec4::Y)
        );
        assert_eq!(lava_ignition(origin, &get, &mut |_| 0), None);
        let stone = |p| {
            Some(BlockState::new(
                if p == IVec4::Y {
                    block::STONE
                } else if p == fuel {
                    block::PLANKS
                } else {
                    block::AIR
                },
                0,
            ))
        };
        assert_eq!(lava_ignition(origin, &stone, &mut |_| 1), None);
    }
    #[test]
    fn snow_does_not_accumulate_over_existing_layers_ice_or_bright_sources() {
        assert!(snow_can_form(block::AIR.into(), block::GRASS.into(), 0));
        assert!(!snow_can_form(block::SNOW.into(), block::GRASS.into(), 0));
        assert!(!snow_can_form(block::AIR.into(), block::ICE.into(), 0));
        assert!(!snow_can_form(block::AIR.into(), block::GRASS.into(), 10));
    }
    #[test]
    fn original_material_flags_include_webs_beds_and_jukeboxes_but_not_stone_wood_slabs() {
        for id in [
            block::BED,
            block::JUKEBOX,
            block::NOTE_BLOCK,
            block::SIGN_POST,
        ] {
            assert!(flammable_material(id));
        }
        assert!(!flammable_material(block::SLAB));
        assert!(material_blocks_movement(block::SIGN_POST));
        assert!(!material_blocks_movement(block::LADDER));
    }
}
