//! 16⁴ chunks of block ids.

use crate::block::BlockId;
use glam::IVec4;

pub const CHUNK_BITS: i32 = 4;
pub const CHUNK: i32 = 1 << CHUNK_BITS;
pub const CHUNK_VOLUME: usize = (CHUNK * CHUNK * CHUNK * CHUNK) as usize;

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
    Dense(Box<[BlockId]>),
}

#[derive(Clone)]
pub struct Chunk {
    pub data: ChunkData,
    /// Changed by the player since generation (needs saving).
    pub modified: bool,
    /// Bumped on every edit, so the renderer knows to re-upload.
    pub version: u32,
}

impl Chunk {
    pub fn uniform(id: BlockId) -> Self {
        Self { data: ChunkData::Uniform(id), modified: false, version: 0 }
    }

    /// Builds a chunk from dense data, collapsing it to `Uniform` when possible.
    pub fn from_dense(blocks: Vec<BlockId>) -> Self {
        debug_assert_eq!(blocks.len(), CHUNK_VOLUME);
        let first = blocks[0];
        let data = if blocks.iter().all(|&b| b == first) {
            ChunkData::Uniform(first)
        } else {
            ChunkData::Dense(blocks.into_boxed_slice())
        };
        Self { data, modified: false, version: 0 }
    }

    #[inline]
    pub fn get(&self, local: IVec4) -> BlockId {
        match &self.data {
            ChunkData::Uniform(b) => *b,
            ChunkData::Dense(d) => d[local_index(local)],
        }
    }

    pub fn set(&mut self, local: IVec4, id: BlockId) {
        if let ChunkData::Uniform(b) = self.data {
            if b == id {
                return;
            }
            self.data = ChunkData::Dense(vec![b; CHUNK_VOLUME].into_boxed_slice());
        }
        if let ChunkData::Dense(d) = &mut self.data {
            d[local_index(local)] = id;
        }
        self.modified = true;
        self.version = self.version.wrapping_add(1);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coords_roundtrip() {
        let b = IVec4::new(-1, 17, -33, 5);
        assert_eq!(chunk_of(b), IVec4::new(-1, 1, -3, 0));
        assert_eq!(local_of(b), IVec4::new(15, 1, 15, 5));
        assert_eq!(chunk_of(b) * CHUNK + local_of(b), b);
    }

    #[test]
    fn uniform_becomes_dense_on_edit() {
        let mut c = Chunk::uniform(0);
        c.set(IVec4::new(1, 2, 3, 4), 7);
        assert_eq!(c.get(IVec4::new(1, 2, 3, 4)), 7);
        assert_eq!(c.get(IVec4::ZERO), 0);
        assert!(c.modified);
    }
}
