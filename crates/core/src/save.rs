//! On-disk storage for chunks the player has modified. Unmodified chunks are regenerated
//! from the seed, so only edits are saved.
//!
//! Layout: `<dir>/chunks/<x>_<y>_<z>_<w>.bin`, each a run-length encoded block array.

use crate::chunk::{CHUNK_VOLUME, Chunk, ChunkData};
use crate::world::ChunkSource;
use glam::IVec4;
use std::io;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"C4D1";

pub struct SaveDir {
    root: PathBuf,
}

impl SaveDir {
    pub fn new(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("chunks"))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, c: IVec4) -> PathBuf {
        self.root.join("chunks").join(format!("{}_{}_{}_{}.bin", c.x, c.y, c.z, c.w))
    }

    pub fn save_chunk(&self, c: IVec4, chunk: &Chunk) -> io::Result<()> {
        let tmp = self.path(c).with_extension("tmp");
        std::fs::write(&tmp, encode(chunk))?;
        std::fs::rename(tmp, self.path(c))
    }
}

impl ChunkSource for SaveDir {
    fn load(&self, c: IVec4) -> Option<Chunk> {
        let bytes = std::fs::read(self.path(c)).ok()?;
        let mut chunk = decode(&bytes)?;
        // Keep it marked modified so it is saved again if evicted.
        chunk.modified = true;
        Some(chunk)
    }
}

pub fn encode(chunk: &Chunk) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    match &chunk.data {
        ChunkData::Uniform(b) => {
            out.extend_from_slice(&(CHUNK_VOLUME as u32).to_le_bytes());
            out.push(*b);
        }
        ChunkData::Dense(d) => {
            let mut i = 0;
            while i < d.len() {
                let b = d[i];
                let mut run = 1;
                while i + run < d.len() && d[i + run] == b {
                    run += 1;
                }
                out.extend_from_slice(&(run as u32).to_le_bytes());
                out.push(b);
                i += run;
            }
        }
    }
    out
}

pub fn decode(bytes: &[u8]) -> Option<Chunk> {
    if bytes.get(..4)? != MAGIC {
        return None;
    }
    let mut blocks = Vec::with_capacity(CHUNK_VOLUME);
    for rec in bytes[4..].chunks(5) {
        if rec.len() != 5 {
            return None;
        }
        let run = u32::from_le_bytes(rec[..4].try_into().ok()?) as usize;
        if blocks.len() + run > CHUNK_VOLUME {
            return None;
        }
        blocks.extend(std::iter::repeat_n(rec[4], run));
    }
    (blocks.len() == CHUNK_VOLUME).then(|| Chunk::from_dense(blocks))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut c = Chunk::uniform(1);
        c.set(IVec4::new(3, 4, 5, 6), 9);
        let d = decode(&encode(&c)).unwrap();
        assert_eq!(d.get(IVec4::new(3, 4, 5, 6)), 9);
        assert_eq!(d.get(IVec4::ZERO), 1);
        assert_eq!(decode(&encode(&Chunk::uniform(0))).unwrap().uniform_id(), Some(0));
    }
}
