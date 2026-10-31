//! Beta rail center-lines, including slopes and a W-axis extension.
//! Native metadata0..9 follows xb.j; extension shapes occupy16..26.
use crate::{
    block::{self, BlockState},
    glam::{IVec4, Vec4},
};
pub const DIRECTIONS: [IVec4; 6] = [
    IVec4::Z,
    IVec4::NEG_Z,
    IVec4::X,
    IVec4::NEG_X,
    IVec4::W,
    IVec4::NEG_W,
];
pub fn is_rail(id: u8) -> bool {
    matches!(id, block::RAIL | block::POWERED_RAIL | block::DETECTOR_RAIL)
}
pub fn shape(state: BlockState) -> u8 {
    if state.id == block::RAIL {
        state.metadata
    } else if state.metadata & 0x10 != 0 {
        state.metadata & 0x7f
    } else {
        state.metadata & 7
    }
}
pub fn powered(state: BlockState) -> bool {
    state.metadata & if state.metadata & 0x10 != 0 { 128 } else { 8 } != 0
}
pub fn with_power(state: BlockState, on: bool) -> BlockState {
    let bit = if state.metadata & 0x10 != 0 { 128 } else { 8 };
    BlockState::new(
        state.id,
        if on {
            state.metadata | bit
        } else {
            state.metadata & !bit
        },
    )
}
pub fn endpoints(code: u8) -> (Vec4, Vec4) {
    let (a, b) = match code {
        1 => (-IVec4::X, IVec4::X),
        2 => (-IVec4::X, IVec4::X + IVec4::Y * 2),
        3 => (-IVec4::X + IVec4::Y * 2, IVec4::X),
        4 => (-IVec4::Z + IVec4::Y * 2, IVec4::Z),
        5 => (-IVec4::Z, IVec4::Z + IVec4::Y * 2),
        6 => (IVec4::Z, IVec4::X),
        7 => (IVec4::Z, -IVec4::X),
        8 => (-IVec4::Z, -IVec4::X),
        9 => (-IVec4::Z, IVec4::X),
        16 => (-IVec4::W, IVec4::W),
        17 => (-IVec4::W, IVec4::W + IVec4::Y * 2),
        18 => (-IVec4::W + IVec4::Y * 2, IVec4::W),
        19 => (IVec4::X, IVec4::W),
        20 => (IVec4::X, -IVec4::W),
        21 => (-IVec4::X, IVec4::W),
        22 => (-IVec4::X, -IVec4::W),
        23 => (IVec4::Z, IVec4::W),
        24 => (IVec4::Z, -IVec4::W),
        25 => (-IVec4::Z, IVec4::W),
        26 => (-IVec4::Z, -IVec4::W),
        _ => (-IVec4::Z, IVec4::Z),
    };
    (a.as_vec4() * 0.5, b.as_vec4() * 0.5)
}
pub fn project(cell: IVec4, code: u8, pos: Vec4) -> (Vec4, Vec4) {
    let (a, b) = endpoints(code);
    let origin = cell.as_vec4() + Vec4::new(0.5, 0.0625, 0.5, 0.5);
    let direction = b - a;
    let t = ((pos - origin - a).dot(direction) / direction.length_squared()).clamp(0.0, 1.0);
    (origin + a + direction * t, direction.normalize())
}
pub fn neighbor(cell: IVec4, d: IVec4, at: &impl Fn(IVec4) -> Option<BlockState>) -> Option<IVec4> {
    [cell + d, cell + d + IVec4::Y, cell + d - IVec4::Y]
        .into_iter()
        .find(|&p| at(p).is_some_and(|s| is_rail(s.id)))
}
/// Horizontal directions joined by a shape's two ends.
fn links(code: u8) -> [IVec4; 2] {
    let (a, b) = endpoints(code);
    [a, b].map(|e| IVec4::new((e.x * 2.0) as i32, 0, (e.z * 2.0) as i32, (e.w * 2.0) as i32))
}
fn points_to(q: IVec4, toward: IVec4, at: &impl Fn(IVec4) -> Option<BlockState>) -> bool {
    at(q).is_some_and(|s| is_rail(s.id) && links(shape(s)).contains(&toward))
}
/// Count of a rail's ends whose neighbour links back (xb.f connected tracks).
fn mutual_links(cell: IVec4, state: BlockState, at: &impl Fn(IVec4) -> Option<BlockState>) -> usize {
    links(shape(state))
        .into_iter()
        .filter(|&d| neighbor(cell, d, at).is_some_and(|q| points_to(q, -d, at)))
        .count()
}
pub fn connected_shape(cell:IVec4,state:BlockState,at:&impl Fn(IVec4)->Option<BlockState>)->u8{connected_shape_powered(cell,state,false,at)}
pub fn connected_shape_powered(cell: IVec4,state: BlockState,powered:bool,at: &impl Fn(IVec4) -> Option<BlockState>) -> u8 {
    // xb.c: a neighbour joins only when it already leads here or both rails
    // still have a free end, so full tracks laid side by side stay straight.
    let free = mutual_links(cell, state, at) < 2;
    let n = DIRECTIONS.map(|d| {
        neighbor(cell, d, at).filter(|&q| {
            points_to(q, -d, at) || free && at(q).is_some_and(|s| mutual_links(q, s, at) < 2)
        })
    });
    let curves = state.id == block::RAIL;
    let choices: [(usize, usize, u8); 15] = [
        (0, 1, 0),
        (2, 3, 1),
        (4, 5, 16),
        (0, 2, 6),
        (0, 3, 7),
        (1, 3, 8),
        (1, 2, 9),
        (2, 4, 19),
        (2, 5, 20),
        (3, 4, 21),
        (3, 5, 22),
        (0, 4, 23),
        (0, 5, 24),
        (1, 4, 25),
        (1, 5, 26),
    ];
    let current = shape(state);
    let mut choice = choices
        .iter()
        .copied()
        .find(|&(a, b, s)| s == current && n[a].is_some() && n[b].is_some());
    if choice.is_none() {
        choice = choices
            .iter()
            .copied()
            .find(|&(a, b, s)| (curves || s < 2 || s == 16) && n[a].is_some() && n[b].is_some());
    }
    let mut selected = choice.map_or_else(
        || {
            if n[2].is_some() || n[3].is_some() {
                1
            } else if n[4].is_some() || n[5].is_some() {
                16
            } else {
                0
            }
        },
        |(_, _, s)| s,
    );
    if curves && n.iter().filter(|v|v.is_some()).count()>=3 {
        // Native rail logic resolves ambiguous junctions in opposite curve
        // priority orders depending on redstone power; W pairs extend that order.
        let mut priority=vec![8,9,7,6,22,20,21,19,26,24,25,23];if powered{priority.reverse();}
        for code in priority {if let Some(&(a,b,_))=choices.iter().find(|&&(_,_,s)|s==code){if n[a].is_some()&&n[b].is_some(){selected=code;}}}
    }
    if selected == 1 {
        if n[2].is_some_and(|p| p.y > cell.y) {
            selected = 2;
        } else if n[3].is_some_and(|p| p.y > cell.y) {
            selected = 3;
        }
    }
    if selected == 0 {
        if n[1].is_some_and(|p| p.y > cell.y) {
            selected = 4;
        } else if n[0].is_some_and(|p| p.y > cell.y) {
            selected = 5;
        }
    }
    if selected == 16 {
        if n[4].is_some_and(|p| p.y > cell.y) {
            selected = 17;
        } else if n[5].is_some_and(|p| p.y > cell.y) {
            selected = 18;
        }
    }
    selected
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_t_junction_changes_curve_with_power_in_xz_and_xw(){
        let state=BlockState::new(block::RAIL,0);
        let xz=|p|if p==IVec4::X{Some(BlockState::new(block::RAIL,1))}else{[IVec4::ZERO,IVec4::Z,-IVec4::Z].contains(&p).then_some(state)};
        assert_eq!(connected_shape_powered(IVec4::ZERO,state,false,&xz),6);assert_eq!(connected_shape_powered(IVec4::ZERO,state,true,&xz),9);
        let xw=|p|[IVec4::ZERO,IVec4::X,-IVec4::X,IVec4::W].contains(&p).then_some(state);
        assert_eq!(connected_shape_powered(IVec4::ZERO,state,false,&xw),19);assert_eq!(connected_shape_powered(IVec4::ZERO,state,true,&xw),21);
    }

    #[test]
    fn native_curves_and_w_extensions_join_edge_centers() {
        for code in [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
        ] {
            let (a, b) = endpoints(code);
            assert_eq!(a.y.min(b.y), 0.0);
            assert!(a != b);
            let (p, d) = project(IVec4::ZERO, code, Vec4::splat(0.5));
            assert!(p.is_finite() && d.is_normalized());
        }
    }
    #[test]
    fn w_slope_and_power_do_not_alias_native_curve_bits() {
        let p = IVec4::ZERO;
        let at = |q| {
            if q == IVec4::W + IVec4::Y || q == -IVec4::W {
                Some(BlockState::new(block::RAIL, 16))
            } else {
                None
            }
        };
        assert_eq!(
            connected_shape(p, BlockState::new(block::RAIL, 16), &at),
            17
        );
        for code in [0, 1, 2, 3, 4, 5, 16, 17, 18] {
            let s = BlockState::new(block::POWERED_RAIL, code);
            let on = with_power(s, true);
            assert!(powered(on));
            assert_eq!(shape(on), code);
            assert_eq!(with_power(on, false), s);
        }
    }
    #[test]
    fn full_track_ignores_a_side_rail_but_a_free_end_joins_it() {
        let straight = BlockState::new(block::RAIL, 16);
        let lone = BlockState::new(block::RAIL, 0);
        let at = |p: IVec4| {
            if p == IVec4::W || p == -IVec4::W {
                Some(straight)
            } else if p == IVec4::X {
                Some(lone)
            } else {
                None
            }
        };
        assert_eq!(connected_shape(IVec4::ZERO, straight, &at), 16);
        assert_eq!(connected_shape(IVec4::X, lone, &|p| if p == IVec4::ZERO { Some(straight) } else { at(p) }), 0);
        let end = |p: IVec4| if p == IVec4::W { Some(straight) } else if p == IVec4::X { Some(lone) } else { None };
        assert_eq!(connected_shape(IVec4::ZERO, straight, &end), 19);
    }
}
