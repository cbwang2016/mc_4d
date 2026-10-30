//! GPU geometry is generated from the same block-local boxes as gameplay.
//! Geometry is deduplicated; each state header points into a compact box pool.
use bytemuck::{Pod, Zeroable};
use mc4d_core::{block::{self, BlockId, BlockShape, BlockState}, shapes::{self, Aabb4}};
use std::collections::HashMap;

pub const STATES_PER_BLOCK: usize = 32;
pub const CUTOUT: u32 = 1;
pub const LEAVES: u32 = 2;
pub const FENCE: u32 = 4;
pub const FULL_CUBE: u32 = 8;
pub const DOOR: u32 = 16;
pub const SNOW: u32 = 32;
pub const UNSHADED: u32 = 64;
pub const SIGN: u32 = 128;
pub const STANDING_SIGN: u32 = 256;
pub const OPAQUE: u32 = 512;
pub const RAIL: u32 = 1024;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuBox { min: [f32; 4], max: [f32; 4], requirements: [u32; 4] }
impl GpuBox {
    fn new(aabb: Aabb4, connection: u32) -> Self {
        Self { min: aabb.min.to_array(), max: aabb.max.to_array(), requirements: [connection, 0, 0, 0] }
    }
}

pub fn flags(id: BlockId) -> u32 {
    static FLAGS: std::sync::OnceLock<[u32; block::BLOCK_COUNT]> = std::sync::OnceLock::new();
    FLAGS.get_or_init(|| std::array::from_fn(|i| compute_flags(i as u8)))[id as usize]
}
fn compute_flags(id: BlockId) -> u32 {
    if !block::registered(id) || id == block::AIR { return 0; }
    let mut flags = match block::definition(id).shape {
        BlockShape::Plant | BlockShape::Torch | BlockShape::Rail | BlockShape::Wire | BlockShape::Web
            | BlockShape::Ladder | BlockShape::Door | BlockShape::Fire => CUTOUT,
        BlockShape::Fence => FENCE,
        _ => 0,
    };
    if matches!(block::definition(id).shape, BlockShape::Plant | BlockShape::Torch | BlockShape::Fire
        | BlockShape::Web | BlockShape::Rail | BlockShape::Wire | BlockShape::Ladder) { flags |= UNSHADED; }
    if block::definition(id).shape == BlockShape::Sign { flags |= SIGN; }
    if block::definition(id).shape == BlockShape::Rail {flags|=RAIL;}
    if id == block::SIGN_POST { flags |= STANDING_SIGN; }
    if block::definition(id).opaque { flags |= OPAQUE; }
    if id == block::LEAVES { flags |= LEAVES; }
    if block::definition(id).shape == BlockShape::Door { flags |= DOOR; }
    if id == block::SNOW || id == block::SNOW_LAYER { flags |= SNOW; }
    if id == block::GLASS { flags |= CUTOUT; }
    if (0..STATES_PER_BLOCK).all(|metadata| {
        let boxes = shapes::visual_boxes(BlockState::new(id, metadata as u8), 0);
        boxes.as_slice() == [Aabb4::UNIT]
    }) { flags |= FULL_CUBE; }
    flags
}

pub fn encode() -> Vec<u8> {
    let mut headers = vec![[0u32; 4]; block::BLOCK_COUNT * STATES_PER_BLOCK];
    let mut boxes: Vec<GpuBox> = Vec::new();
    let mut shared: HashMap<Vec<u32>, (u32, u32)> = HashMap::new();
    for &id in block::ALL_BLOCKS {
        let flags = flags(id);
        for metadata in 0..STATES_PER_BLOCK {
            let state = BlockState::new(id, metadata as u8);
            let base = shapes::visual_boxes(state, 0);
            let mut parts: Vec<_> = base.as_slice().iter().copied().map(|b| GpuBox::new(b, 0)).collect();
            if flags & FENCE != 0 {
                for bit in 0..6 {
                    for &aabb in shapes::visual_boxes(state, 1 << bit).as_slice() {
                        if !base.as_slice().contains(&aabb) { parts.push(GpuBox::new(aabb, 1 << bit)); }
                    }
                }
            }
            assert!(parts.len() <= 16, "shared shape capacity");
            let key: Vec<u32> = bytemuck::cast_slice::<GpuBox, u32>(&parts).to_vec();
            let (first, count) = *shared.entry(key).or_insert_with(|| {
                let pair = (boxes.len() as u32, parts.len() as u32); boxes.extend(parts); pair
            });
            headers[id as usize * STATES_PER_BLOCK + metadata] = [first, count, flags, 0];
        }
    }
    let mut out = bytemuck::cast_slice(&headers).to_vec();
    out.extend_from_slice(bytemuck::cast_slice(&boxes));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_boxes_match_gameplay_shapes_for_all_states_and_fence_connections() {
        let bytes = encode();
        let header_bytes = block::BLOCK_COUNT * STATES_PER_BLOCK * 16;
        let headers: &[[u32; 4]] = bytemuck::cast_slice(&bytes[..header_bytes]);
        let boxes: &[GpuBox] = bytemuck::cast_slice(&bytes[header_bytes..]);
        assert!(bytes.len() < 1024 * 1024, "deduplicated lookup should remain small");
        for &id in block::ALL_BLOCKS { for metadata in 0..32u8 { for connections in [0, 1, 2, 4, 8, 16, 32, 63] {
            let header = headers[id as usize * 32 + metadata as usize];
            let actual: Vec<_> = boxes[header[0] as usize..(header[0] + header[1]) as usize].iter()
                .filter(|b| b.requirements[0] == 0 || b.requirements[0] & connections as u32 != 0)
                .map(|b| Aabb4::new(b.min.into(), b.max.into())).collect();
            assert_eq!(actual.as_slice(), shapes::visual_boxes(BlockState::new(id, metadata), connections).as_slice());
        }}}
    }
}
