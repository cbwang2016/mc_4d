//! 16⁴ chunks of block ids.

use crate::block::{self, BlockId, BlockState};
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

static NEXT_CHUNK_IDENTITY: AtomicU64 = AtomicU64::new(1);
use glam::IVec4;

pub const CHUNK_BITS: i32 = 4;
pub const CHUNK: i32 = 1 << CHUNK_BITS;
pub const CHUNK_VOLUME: usize = (CHUNK * CHUNK * CHUNK * CHUNK) as usize;
pub const MACROCELL_COUNT: usize = 256;
pub const OCCUPANCY_WORDS: usize = 16;

fn macro_index(index: usize) -> usize {
    ((index >> 2) & 3) | ((index >> 4) & 12) | ((index >> 6) & 48) | ((index >> 8) & 192)
}

fn opaque(id: BlockId) -> bool { id != block::AIR && id != block::GLASS && !block::is_water(id) }

#[derive(Clone)]
struct MacroCounts {
    visible: [u16; MACROCELL_COUNT],
    opaque: [u16; MACROCELL_COUNT],
}

impl MacroCounts {
    fn uniform(id: BlockId) -> Self {
        Self { visible: [if id != block::AIR { 256 } else { 0 }; MACROCELL_COUNT],
            opaque: [if opaque(id) { 256 } else { 0 }; MACROCELL_COUNT] }
    }
}

/// World height in blocks: Y ∈ [0, WORLD_HEIGHT).
pub const WORLD_HEIGHT: i32 = 256;
pub const WORLD_CHUNKS_Y: i32 = WORLD_HEIGHT / CHUNK;

/// Index of a block inside a chunk; `x` varies fastest. The GPU uses the same layout.
#[inline]
pub fn local_index(l: IVec4) -> usize {
    (l.x | (l.y << 4) | (l.z << 8) | (l.w << 12)) as usize
}

#[inline]
pub fn chunk_of(block: IVec4) -> IVec4 {
    block >> CHUNK_BITS
}

#[inline]
pub fn local_of(block: IVec4) -> IVec4 {
    block & (CHUNK - 1)
}

#[derive(Clone)]
pub enum ChunkData {
    /// Every block has the same id.
    Uniform(BlockId),
    Dense(Arc<[BlockId]>),
}

#[derive(Clone)]
pub struct Chunk {
    /// Runtime hints exist only for newly generated dungeon chests. Saved
    /// container contents live in the game level, not in derived terrain data.
    pub generated_chests: Vec<(u16, Vec<crate::item::Stack>)>,
    pub data: ChunkData,
    /// Changed since generation or the last successful save (needs saving).
    pub modified: bool,
    /// Bumped on every edit, so the renderer knows to re-upload.
    pub version: u32,
    /// Number of source or flowing fluid cells, for bounded activation on load.
    pub fluid_cells: u32,
    /// Identity changes when a chunk is reloaded, even if its edit version is zero.
    pub identity: u64,
    pub data_version: u32,
    pub metadata_version: u32,
    /// Only opacity/emission changes invalidate derived lighting.
    pub light_state_version: u32,
    pub emissive_cells: u32,
    present_ids: [u64; 4],
    metadata: Option<Arc<[u8]>>,
    metadata_nonzero: u32,
    metadata_flags:u8,
    macro_counts: Option<Arc<MacroCounts>>,
}

impl Chunk {
    pub fn uniform(id: BlockId) -> Self {
        let mut present_ids = [0; 4]; present_ids[(id / 64) as usize] |= 1u64 << (id % 64);
        Self { generated_chests: Vec::new(), data: ChunkData::Uniform(id), modified: false, version: 0,
            fluid_cells: if crate::block::is_fluid(id) { CHUNK_VOLUME as u32 } else { 0 },
            identity: NEXT_CHUNK_IDENTITY.fetch_add(1, Ordering::Relaxed), data_version: 0, metadata_version: 0,
            light_state_version: 0, emissive_cells: if block::light_emission(id.into()) > 0 { CHUNK_VOLUME as u32 } else { 0 },
            present_ids,
            metadata: None, metadata_nonzero: 0, metadata_flags:0, macro_counts: None }
    }

    /// Builds a chunk from dense data, collapsing it to `Uniform` when possible.
    pub fn from_dense(blocks: Vec<BlockId>) -> Self {
        debug_assert_eq!(blocks.len(), CHUNK_VOLUME);
        let first = blocks[0];
        if blocks.iter().all(|&b| b == first) { return Self::uniform(first); }
        let mut counts = MacroCounts::uniform(block::AIR);
        let mut fluid_cells = 0;
        let mut emissive_cells = 0;
        let mut present_ids = [0; 4];
        for (index, &id) in blocks.iter().enumerate() {
            fluid_cells += block::is_fluid(id) as u32;
            emissive_cells += (block::light_emission(id.into()) > 0) as u32;
            present_ids[(id / 64) as usize] |= 1u64 << (id % 64);
            let m = macro_index(index);
            counts.visible[m] += (id != block::AIR) as u16;
            counts.opaque[m] += opaque(id) as u16;
        }
        Self { generated_chests: Vec::new(), data: ChunkData::Dense(blocks.into()), modified: false, version: 0, fluid_cells,
            identity: NEXT_CHUNK_IDENTITY.fetch_add(1, Ordering::Relaxed), data_version: 0, metadata_version: 0,
            light_state_version: 0, emissive_cells,
            present_ids,
            metadata: None, metadata_nonzero: 0, metadata_flags:0, macro_counts: Some(Arc::new(counts)) }
    }

    #[inline]
    pub fn get(&self, local: IVec4) -> BlockId {
        match &self.data {
            ChunkData::Uniform(b) => *b,
            ChunkData::Dense(d) => d[local_index(local)],
        }
    }

    /// Build a clean chunk from validated ID and optional state planes.
    pub fn from_planes(blocks: Vec<BlockId>, metadata: Option<Vec<u8>>) -> Option<Self> {
        if blocks.len() != CHUNK_VOLUME || metadata.as_ref().is_some_and(|m| m.len() != CHUNK_VOLUME) { return None; }
        let mut chunk = Self::from_dense(blocks);
        if let Some(metadata) = metadata {
            (chunk.metadata_nonzero,chunk.metadata_flags)=metadata.iter().fold((0u32,0u8),|(count,bits),&m|(count+u32::from(m!=0),bits|m));
            if chunk.metadata_nonzero != 0 { chunk.metadata = Some(metadata.into()); }
        }
        // Verified Beta light emission is per blockID (lit furnaces/torches have
        // distinct IDs), so from_dense already counted it. Species metadata does
        // not require another full64KiB voxel scan.
        Some(chunk)
    }

    #[inline]
    pub fn get_state(&self, local: IVec4) -> BlockState {
        BlockState::new(self.get(local), self.metadata.as_ref().map_or(0, |m| m[local_index(local)]))
    }

    /// Conservative metadata bit summary; resetting the whole plane clears it.
    pub fn may_have_metadata_bits(&self,bits:u8)->bool{self.metadata_flags&bits!=0}

    pub fn metadata_plane(&self) -> Option<&[u8]> { self.metadata.as_deref() }

    /// Conservative ID summary for bounded plant/tile activation. Removal may
    /// leave a bit set; false positives are safe and loading rebuilds exact bits.
    pub fn may_contain(&self, id: BlockId) -> bool { self.present_ids[(id / 64) as usize] & (1u64 << (id % 64)) != 0 }
    pub fn may_contain_any(&self, ids: &[BlockId]) -> bool { ids.iter().any(|&id| self.may_contain(id)) }

    /// Byte-only edits place the default state, clearing any previous metadata.
    pub fn set(&mut self, local: IVec4, id: BlockId) { self.set_state(local, id.into()); }

    /// Returns whether the state changed. Clone snapshots share immutable planes;
    /// only an edited plane is copied, so background readers retain exact snapshots.
    pub fn set_state(&mut self, local: IVec4, state: BlockState) -> bool {
        let old = self.get_state(local);
        if old == state { return false; }
        let old_emission = block::light_emission(old);
        let new_emission = block::light_emission(state);
        if old_emission != new_emission || block::light_opacity(old) != block::light_opacity(state) {
            self.light_state_version = self.light_state_version.wrapping_add(1);
        }
        self.emissive_cells = self.emissive_cells - (old_emission > 0) as u32 + (new_emission > 0) as u32;
        let index = local_index(local);
        if old.id != state.id {
            self.present_ids[(state.id / 64) as usize] |= 1u64 << (state.id % 64);
            self.fluid_cells = self.fluid_cells - block::is_fluid(old.id) as u32 + block::is_fluid(state.id) as u32;
            if let ChunkData::Uniform(id) = self.data {
                self.data = ChunkData::Dense(vec![id; CHUNK_VOLUME].into());
                self.macro_counts = Some(Arc::new(MacroCounts::uniform(id)));
            }
            let m = macro_index(index);
            let counts = Arc::make_mut(self.macro_counts.as_mut().unwrap());
            counts.visible[m] = counts.visible[m] - (old.id != block::AIR) as u16 + (state.id != block::AIR) as u16;
            counts.opaque[m] = counts.opaque[m] - opaque(old.id) as u16 + opaque(state.id) as u16;
            if let ChunkData::Dense(data) = &mut self.data { Arc::make_mut(data)[index] = state.id; }
            self.data_version = self.data_version.wrapping_add(1);
        }
        if old.metadata != state.metadata {
            self.metadata_nonzero = self.metadata_nonzero - (old.metadata != 0) as u32 + (state.metadata != 0) as u32;
            self.metadata_flags|=state.metadata;
            if self.metadata_nonzero == 0 {
                self.metadata = None;self.metadata_flags=0;
            } else {
                let plane = self.metadata.get_or_insert_with(|| vec![0; CHUNK_VOLUME].into());
                Arc::make_mut(plane)[index] = state.metadata;
            }
            self.metadata_version = self.metadata_version.wrapping_add(1);
        }
        self.modified = true;
        self.version = self.version.wrapping_add(1);
        true
    }

    pub fn uniform_id(&self) -> Option<BlockId> {
        match self.data {
            ChunkData::Uniform(b) => Some(b),
            ChunkData::Dense(_) => None,
        }
    }

    pub fn dense(&self) -> Option<&[BlockId]> {
        match &self.data {
            ChunkData::Uniform(_) => None,
            ChunkData::Dense(d) => Some(d),
        }
    }

    /// Two 256-bit masks for 4^4 macrocells: any non-air voxel, then any opaque
    /// voxel. Separate masks preserve water/glass tint in primary rays while
    /// allowing ghost rays to skip those transparent regions completely.
    pub fn occupancy_masks(&self) -> [u32; OCCUPANCY_WORDS] {
        let mut masks = [0; OCCUPANCY_WORDS];
        if let Some(counts) = &self.macro_counts {
            for i in 0..MACROCELL_COUNT {
                if counts.visible[i] > 0 { masks[i / 32] |= 1 << (i % 32); }
                if counts.opaque[i] > 0 { masks[8 + i / 32] |= 1 << (i % 32); }
            }
        } else if let Some(id) = self.uniform_id() {
            if id != block::AIR { masks[..8].fill(u32::MAX); }
            if opaque(id) { masks[8..].fill(u32::MAX); }
        }
        masks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_summary_skips_species_only_forest_and_rebuilds_after_save() {
        let mut chunk=Chunk::from_planes(vec![block::LEAVES;CHUNK_VOLUME],Some(vec![1;CHUNK_VOLUME])).unwrap();
        assert!(!chunk.may_have_metadata_bits(8));assert_eq!(chunk.emissive_cells,0);
        let snapshot=chunk.clone();chunk.set_state(IVec4::ZERO,BlockState::new(block::LEAVES,9));
        assert!(chunk.may_have_metadata_bits(8));assert!(!snapshot.may_have_metadata_bits(8));
        chunk.set_state(IVec4::ZERO,BlockState::new(block::LEAVES,1));
        let restored=crate::save::decode(&crate::save::encode(&chunk)).unwrap();
        assert!(!restored.may_have_metadata_bits(8));
    }

    #[test]
    fn coords_roundtrip() {
        let b = IVec4::new(-1, 17, -33, 5);
        assert_eq!(chunk_of(b), IVec4::new(-1, 1, -3, 0));
        assert_eq!(local_of(b), IVec4::new(15, 1, 15, 5));
        assert_eq!(chunk_of(b) * CHUNK + local_of(b), b);
    }

    #[test]
    fn metadata_is_optional_and_snapshot_edits_copy_only_the_changed_plane() {
        let p = IVec4::new(3, 4, 5, 6);
        let mut chunk = Chunk::uniform(block::WOOL);
        assert!(chunk.metadata_plane().is_none());
        let original = chunk.clone();
        chunk.set_state(p, BlockState::new(block::WOOL, 14));
        assert_eq!(chunk.uniform_id(), Some(block::WOOL));
        assert_eq!(chunk.data_version, 0);
        assert_eq!(chunk.metadata_version, 1);
        assert_eq!(original.get_state(p), BlockState::new(block::WOOL, 0));
        let colored = chunk.clone();
        chunk.set(p, block::WOOL);
        assert!(chunk.metadata_plane().is_none());
        assert_eq!(colored.get_state(p), BlockState::new(block::WOOL, 14));
        chunk.set_state(p, BlockState::new(block::LOG, 2));
        let shared = chunk.clone();
        if let (ChunkData::Dense(a), ChunkData::Dense(b)) = (&chunk.data, &shared.data) { assert!(Arc::ptr_eq(a, b)); } else { panic!("dense planes"); }
        chunk.set(p, block::AIR);
        assert_eq!(shared.get_state(p), BlockState::new(block::LOG, 2));
        assert_eq!(chunk.get_state(p), BlockState::new(block::AIR, 0));
        assert_ne!(Chunk::uniform(block::AIR).identity, Chunk::uniform(block::AIR).identity);
    }

    #[test]
    fn uniform_becomes_dense_on_edit() {
        let mut c = Chunk::uniform(0);
        c.set(IVec4::new(1, 2, 3, 4), 7);
        assert_eq!(c.get(IVec4::new(1, 2, 3, 4)), 7);
        assert_eq!(c.get(IVec4::ZERO), 0);
        assert!(c.modified);
    }

    #[test]
    fn masks_keep_transparency_and_clear_only_after_last_occupied_voxel() {
        let mut c = Chunk::uniform(block::AIR);
        let a = IVec4::new(5, 9, 1, 13);
        let b = a + IVec4::X;
        c.set(a, block::STONE);
        c.set(b, block::GLASS);
        let m = 1 + 4 * (2 + 4 * (0 + 4 * 3));
        let mask = 1 << (m % 32);
        assert_ne!(c.occupancy_masks()[m / 32] & mask, 0);
        assert_ne!(c.occupancy_masks()[8 + m / 32] & mask, 0);
        c.set(a, block::FluidKind::Water.flowing(3));
        assert_eq!(c.occupancy_masks()[8 + m / 32] & mask, 0);
        c.set(a, block::AIR);
        assert_ne!(c.occupancy_masks()[m / 32] & mask, 0);
        c.set(b, block::AIR);
        assert_eq!(c.occupancy_masks(), [0; OCCUPANCY_WORDS]);
    }

    #[test]
    fn masks_match_full_voxel_scan_after_dense_load_and_many_edits() {
        let mut c = Chunk::uniform(block::AIR);
        let ids = [block::AIR, block::STONE, block::GLASS, block::WATER,
            block::FluidKind::Water.flowing(4), block::FluidKind::Lava.flowing(5)];
        let mut state = 41u32;
        for i in 0..4096 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let p = IVec4::new((state & 15) as i32, ((state >> 8) & 15) as i32,
                ((state >> 16) & 15) as i32, ((state >> 24) & 15) as i32);
            c.set(p, ids[i % ids.len()]);
        }
        let mut expected = [0; OCCUPANCY_WORDS];
        for w in 0..16 { for z in 0..16 { for y in 0..16 { for x in 0..16 {
            let id = c.get(IVec4::new(x, y, z, w));
            let m = (x / 4 + 4 * (y / 4 + 4 * (z / 4 + 4 * (w / 4)))) as usize;
            if id != block::AIR { expected[m / 32] |= 1 << (m % 32); }
            if id != block::AIR && id != block::GLASS && !block::is_water(id) { expected[8 + m / 32] |= 1 << (m % 32); }
        }}}}
        assert_eq!(c.occupancy_masks(), expected);
        assert_eq!(Chunk::from_dense(c.dense().unwrap().to_vec()).occupancy_masks(), expected);
        let mut solid = Chunk::uniform(block::STONE);
        solid.set(IVec4::ZERO, block::AIR);
        assert_eq!(solid.occupancy_masks(), [u32::MAX; OCCUPANCY_WORDS]);
    }
}
