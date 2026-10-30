//! Beta1.5 power rules lifted into the six horizontal directions. Weak power
//! reaches devices; strong power also energizes one opaque support block.
//! Reference: ri/wh/bo/ct/nm/vg bytecode in redstone-bytecode.txt.
use crate::{
    block::{self, BlockState},
    shapes::{self, HORIZONTAL_DIRECTIONS},
};
use glam::IVec4;
pub const NEIGHBORS: [IVec4; 8] = [
    IVec4::X,
    IVec4::NEG_X,
    IVec4::Y,
    IVec4::NEG_Y,
    IVec4::Z,
    IVec4::NEG_Z,
    IVec4::W,
    IVec4::NEG_W,
];
pub const COMPONENTS: &[u8] = &[
    block::REDSTONE_WIRE,
    block::LEVER,
    block::BUTTON,
    block::STONE_PRESSURE_PLATE,
    block::WOOD_PRESSURE_PLATE,
    block::REDSTONE_TORCH,
    block::REDSTONE_TORCH_OFF,
    block::REPEATER,
    block::REPEATER_OFF,
    block::WOOD_DOOR,
    block::IRON_DOOR,
    block::DISPENSER,
    block::NOTE_BLOCK,
    block::TNT,
    block::DETECTOR_RAIL,
];
pub fn repeater_direction(m: u8) -> IVec4 {
    if m & 16 != 0 {
        if m & 1 == 0 { IVec4::W } else { IVec4::NEG_W }
    } else {
        [IVec4::NEG_Z, IVec4::X, IVec4::Z, IVec4::NEG_X][(m & 3) as usize]
    }
}
pub fn repeater_metadata(d: IVec4) -> u8 {
    if d == IVec4::W {
        16
    } else if d == IVec4::NEG_W {
        17
    } else {
        [IVec4::NEG_Z, IVec4::X, IVec4::Z, IVec4::NEG_X]
            .iter()
            .position(|&v| v == d)
            .unwrap_or(0) as u8
    }
}
pub fn repeater_delay(m: u8) -> u64 {
    2 * (1 + u64::from((m >> 2) & 3))
}
pub fn emitter(id: u8) -> bool {
    matches!(
        id,
        block::REDSTONE_WIRE
            | block::LEVER
            | block::BUTTON
            | block::STONE_PRESSURE_PLATE
            | block::WOOD_PRESSURE_PLATE
            | block::REDSTONE_TORCH
            | block::REDSTONE_TORCH_OFF
            | block::REPEATER
            | block::REPEATER_OFF
            | block::DETECTOR_RAIL
    )
}
fn opaque(p: IVec4, get: &impl Fn(IVec4) -> Option<BlockState>) -> bool {
    get(p).is_some_and(|s| block::definition(s.id).opaque)
}
/// Wire links across a level surface or one up/down step, without crossing a
/// full ceiling. Unknown cells never invent a power source.
pub fn wire_neighbors(p: IVec4, get: &impl Fn(IVec4) -> Option<BlockState>) -> Vec<IVec4> {
    let mut out = Vec::with_capacity(6);
    for d in HORIZONTAL_DIRECTIONS {
        let q = p + d;
        if get(q).is_some_and(|s| s.id == block::REDSTONE_WIRE) {
            out.push(q)
        }
        if opaque(q, get) {
            if !opaque(p + IVec4::Y, get)
                && get(q + IVec4::Y).is_some_and(|s| s.id == block::REDSTONE_WIRE)
            {
                out.push(q + IVec4::Y)
            }
        } else if get(q - IVec4::Y).is_some_and(|s| s.id == block::REDSTONE_WIRE) {
            out.push(q - IVec4::Y)
        }
    }
    out
}
fn wire_output(p: IVec4, toward: IVec4, get: &impl Fn(IVec4) -> Option<BlockState>) -> bool {
    if toward == IVec4::NEG_Y {
        return true;
    }
    if toward.y != 0 {
        return false;
    }
    let connections = HORIZONTAL_DIRECTIONS.map(|d| {
        let q = p + d;
        get(q).is_some_and(|s| emitter(s.id))
            || !opaque(q, get) && get(q - IVec4::Y).is_some_and(|s| emitter(s.id))
            || !opaque(p + IVec4::Y, get)
                && opaque(q, get)
                && get(q + IVec4::Y).is_some_and(|s| emitter(s.id))
    });
    if !connections.iter().any(|&v| v) {
        return true;
    }
    // Original side numbers are opposite the outgoing vector: an end of dust
    // powers the block ahead of it, opposite the connected wire behind it.
    let Some(back) = HORIZONTAL_DIRECTIONS.iter().position(|&d| d == -toward) else {
        return false;
    };
    connections[back]
        && HORIZONTAL_DIRECTIONS
            .iter()
            .zip(connections)
            .all(|(&d, c)| !c || d == toward || d == -toward)
}
pub fn direct_output(
    p: IVec4,
    toward: IVec4,
    strong: bool,
    allow_wire: bool,
    get: &impl Fn(IVec4) -> Option<BlockState>,
) -> u8 {
    let Some(s) = get(p) else { return 0 };
    let powered = match s.id {
        block::LEVER | block::BUTTON => {
            s.metadata & 8 != 0 && (!strong || toward == shapes::torch_support(s.metadata))
        }
        block::STONE_PRESSURE_PLATE | block::WOOD_PRESSURE_PLATE => {
            s.metadata != 0 && (!strong || toward == IVec4::NEG_Y)
        }
        block::REDSTONE_TORCH => {
            toward != shapes::torch_support(s.metadata) && (!strong || toward == IVec4::Y)
        }
        block::DETECTOR_RAIL => crate::rails::powered(s) && (!strong || toward == IVec4::NEG_Y),
        block::REPEATER => toward == repeater_direction(s.metadata),
        block::REDSTONE_WIRE => {
            return if allow_wire && wire_output(p, toward, get) {
                s.metadata.min(15)
            } else {
                0
            };
        }
        _ => false,
    };
    if powered { 15 } else { 0 }
}
pub fn signal_from(
    p: IVec4,
    toward: IVec4,
    allow_wire: bool,
    get: &impl Fn(IVec4) -> Option<BlockState>,
) -> u8 {
    if opaque(p, get) {
        NEIGHBORS
            .into_iter()
            .map(|d| direct_output(p + d, -d, true, allow_wire, get))
            .max()
            .unwrap_or(0)
    } else {
        direct_output(p, toward, false, allow_wire, get)
    }
}
pub fn input_power(p: IVec4, allow_wire: bool, get: &impl Fn(IVec4) -> Option<BlockState>) -> u8 {
    NEIGHBORS
        .into_iter()
        .map(|d| signal_from(p + d, -d, allow_wire, get))
        .max()
        .unwrap_or(0)
}
pub fn wire_power(p: IVec4, get: &impl Fn(IVec4) -> Option<BlockState>) -> u8 {
    if input_power(p, false, get) > 0 {
        return 15;
    }
    wire_neighbors(p, get)
        .into_iter()
        .filter_map(|q| get(q).map(|s| s.metadata.min(15).saturating_sub(1)))
        .max()
        .unwrap_or(0)
}
pub fn torch_input(
    p: IVec4,
    state: BlockState,
    get: &impl Fn(IVec4) -> Option<BlockState>,
) -> bool {
    let d = shapes::torch_support(state.metadata);
    signal_from(p + d, -d, true, get) > 0
}
pub fn repeater_input(
    p: IVec4,
    state: BlockState,
    get: &impl Fn(IVec4) -> Option<BlockState>,
) -> bool {
    let d = -repeater_direction(state.metadata);
    signal_from(p + d, -d, true, get) > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[test]
    fn switches_strongly_power_only_their_support_and_torches_invert_it() {
        let p = IVec4::ZERO;
        let mut m = HashMap::new();
        m.insert(p, BlockState::new(block::LEVER, 5 | 8));
        m.insert(p - IVec4::Y, block::STONE.into());
        let get = |p| Some(*m.get(&p).unwrap_or(&block::AIR.into()));
        assert_eq!(signal_from(p - IVec4::Y, IVec4::W, true, &get), 15);
        assert_eq!(direct_output(p, IVec4::W, true, true, &get), 0);
        assert!(torch_input(
            p - IVec4::Y + IVec4::X,
            BlockState::new(block::REDSTONE_TORCH, 1),
            &get
        ));
    }
    #[test]
    fn wire_strength_attenuates_across_w_and_does_not_self_sustain() {
        let p = IVec4::ZERO;
        let mut m = HashMap::new();
        m.insert(p, BlockState::new(block::LEVER, 5 | 8));
        for n in 1..=16 {
            m.insert(p + IVec4::W * n, block::REDSTONE_WIRE.into());
        }
        for _ in 0..20 {
            for n in 1..=16 {
                let q = p + IVec4::W * n;
                let v = wire_power(q, &|p| Some(*m.get(&p).unwrap_or(&block::AIR.into())));
                m.insert(q, BlockState::new(block::REDSTONE_WIRE, v));
            }
        }
        for n in 1..=16 {
            assert_eq!(m[&(p + IVec4::W * n)].metadata, 16 - n as u8);
        }
        m.remove(&p);
        for _ in 0..20 {
            for n in 1..=16 {
                let q = p + IVec4::W * n;
                let v = wire_power(q, &|p| Some(*m.get(&p).unwrap_or(&block::AIR.into())));
                m.insert(q, BlockState::new(block::REDSTONE_WIRE, v));
            }
        }
        assert!(m.values().all(|s| s.metadata == 0));
    }
    #[test]
    fn repeaters_emit_only_forward_and_keep_all_four_delays_in_w() {
        for d in HORIZONTAL_DIRECTIONS {
            for delay in 0..4 {
                let metadata = repeater_metadata(d) | (delay << 2);
                assert_eq!(repeater_direction(metadata), d);
                assert_eq!(repeater_delay(metadata), 2 * (1 + delay as u64));
                let get = |p| {
                    Some(if p == IVec4::ZERO {
                        BlockState::new(block::REPEATER, metadata)
                    } else {
                        block::AIR.into()
                    })
                };
                assert_eq!(direct_output(IVec4::ZERO, d, false, true, &get), 15);
                assert_eq!(direct_output(IVec4::ZERO, -d, false, true, &get), 0);
            }
        }
    }
}
