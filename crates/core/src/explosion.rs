//! Original Beta blast attenuation and damage, extended symmetrically into W.
//! The4D direction lattice uses eight samples per axis so planning remains bounded.
use crate::{
    block::{self, BlockState},
    raycast::raycast_states,
    shapes::Aabb4,
};
use glam::{IVec4, Vec4};
pub const GRID: usize = 8;
pub const DIRECTION_CELLS: usize = GRID * GRID * GRID * GRID + 8;
pub fn direction(index: usize) -> Option<Vec4> {
    if index >= GRID.pow(4) {
        let axis = index - GRID.pow(4);
        if axis >= 8 {
            return None;
        }
        let mut d = Vec4::ZERO;
        d[axis / 2] = if axis % 2 == 0 { 1.0 } else { -1.0 };
        return Some(d);
    }
    let coordinates = [
        index % GRID,
        (index / GRID) % GRID,
        (index / GRID / GRID) % GRID,
        (index / GRID / GRID / GRID) % GRID,
    ];
    if !coordinates.iter().any(|&v| v == 0 || v == GRID - 1) {
        return None;
    }
    Some(
        Vec4::from_array(coordinates.map(|v| v as f32 / (GRID - 1) as f32 * 2.0 - 1.0)).normalize(),
    )
}
pub fn strength(radius: u8, seed: u64, index: usize) -> f32 {
    let mut x = seed
        .wrapping_add(index as u64)
        .wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^= x >> 31;
    radius as f32 * (0.7 + ((x >> 40) as f32 / 16777216.0) * 0.6)
}
pub fn trace(
    center: Vec4,
    direction: Vec4,
    mut strength: f32,
    get: &impl Fn(IVec4) -> Option<BlockState>,
    mut affected: impl FnMut(IVec4),
) {
    let mut position = center;
    let mut previous = None;
    for _ in 0..128 {
        if strength <= 0.0 {
            break;
        }
        let cell = position.floor().as_ivec4();
        let Some(state) = get(cell) else { break };
        if state.id != block::AIR {
            strength -= (block::definition(state.id).resistance / 5.0 + 0.3) * 0.3;
        }
        if strength > 0.0 && state.id != block::AIR && previous != Some(cell) {
            affected(cell);
            previous = Some(cell);
        }
        position += direction * 0.3;
        strength -= 0.225;
    }
}
/// A bounded4D exposure sample: center and all sixteen AABB corners. Block
/// selection geometry means open doors and partial slabs shield correctly.
pub fn exposure(center: Vec4, bounds: Aabb4, get: &impl Fn(IVec4) -> Option<BlockState>) -> f32 {
    let inset = (bounds.max - bounds.min) * 0.05;
    let lo = bounds.min + inset;
    let hi = bounds.max - inset;
    let mut visible = 0;
    for sample in 0..17 {
        let point = if sample == 16 {
            (lo + hi) * 0.5
        } else {
            Vec4::new(
                if sample & 1 == 0 { lo.x } else { hi.x },
                if sample & 2 == 0 { lo.y } else { hi.y },
                if sample & 4 == 0 { lo.z } else { hi.z },
                if sample & 8 == 0 { lo.w } else { hi.w },
            )
        };
        let delta = center - point;
        let distance = delta.length();
        if distance < 1e-5
            || raycast_states(
                point,
                delta / distance,
                distance,
                &|p| Some(get(p).unwrap_or_else(|| block::BEDROCK.into())),
                false,
            )
            .is_none()
        {
            visible += 1;
        }
    }
    visible as f32 / 17.0
}
pub fn damage(radius: u8, distance: f32, exposure: f32) -> u8 {
    let impact = (1.0 - distance / (radius as f32 * 2.0)).max(0.0) * exposure;
    if impact <= 0.0 {
        0
    } else {
        ((impact * impact + impact) * 0.5 * 16.0 * radius as f32 + 1.0)
            .floor()
            .min(255.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_material_resistance_stops_blasts_before_blocks_behind_shield() {
        for shield in [block::STONE, block::OBSIDIAN, block::WATER] {
            let mut hit = Vec::new();
            let get = |p: IVec4| {
                Some(
                    if p.x == 1 {
                        shield
                    } else if p.x == 2 {
                        block::PLANKS
                    } else {
                        block::AIR
                    }
                    .into(),
                )
            };
            trace(Vec4::splat(0.5), Vec4::X, 4.0, &get, |p| hit.push(p));
            assert!(!hit.iter().any(|p| p.x >= 2), "shield{id}", id = shield);
            if shield == block::OBSIDIAN || shield == block::WATER {
                assert!(hit.is_empty());
            }
        }
    }
    #[test]
    fn complete_wall_prevents_entity_damage_but_open_doorway_restores_exposure() {
        let body = Aabb4::new(Vec4::new(2.2, 0.0, 0.2, 0.2), Vec4::new(2.8, 1.8, 0.8, 0.8));
        let center = Vec4::splat(0.5);
        let wall = |p: IVec4| Some(if p.x == 1 { block::STONE } else { block::AIR }.into());
        assert_eq!(exposure(center, body, &wall), 0.0);
        assert_eq!(damage(3, 2.0, 0.0), 0);
        let air = |_: IVec4| Some(block::AIR.into());
        assert_eq!(exposure(center, body, &air), 1.0);
        assert_eq!(damage(3, 0.0, 1.0), 49);
        assert_eq!(damage(4, 0.0, 1.0), 65);
    }
}
