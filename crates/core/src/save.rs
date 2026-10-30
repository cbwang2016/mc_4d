//! On-disk storage for edited chunks. Chunks without saved data are generated from
//! the seed; loaded snapshots remain clean until another edit requires saving.
//!
//! Layout: `<dir>/chunks/<x>_<y>_<z>_<w>.bin`, each a run-length encoded block array.

use crate::chunk::{CHUNK_VOLUME, Chunk, ChunkData};
use crate::world::ChunkSource;
use glam::IVec4;
use std::io;
use std::path::{Path, PathBuf};

const LEGACY_MAGIC: &[u8; 4] = b"C4D1";
const MAGIC: &[u8; 4] = b"C4D2";

pub struct SaveDir {
    root: PathBuf,
}

impl SaveDir {
    pub fn new(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("chunks"))?;
        Ok(Self { root })
    }

    /// Refuse damaged chunk files before gameplay can regenerate and overwrite them.
    pub fn validate_existing(root: &Path) -> io::Result<()> {
        let directory=root.join("chunks");
        let entries=match std::fs::read_dir(&directory) {
            Ok(entries)=>entries,
            Err(error) if error.kind()==io::ErrorKind::NotFound=>return Ok(()),
            Err(error)=>return Err(error),
        };
        for entry in entries {
            let entry=entry?;
            if entry.path().extension().and_then(|e|e.to_str())!=Some("bin") {continue;}
            let metadata=entry.metadata()?;
            if !entry.file_type()?.is_file() || metadata.len()>(CHUNK_VOLUME*10+12) as u64 {
                return Err(io::Error::new(io::ErrorKind::InvalidData,format!("Invalid saved chunk: {}",entry.path().display())));
            }
            if decode(&std::fs::read(entry.path())?).is_none() {
                return Err(io::Error::new(io::ErrorKind::InvalidData,format!("Damaged saved chunk: {}. Original file preserved.",entry.path().display())));
            }
        }
        Ok(())
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
        // Its current contents already exist on disk; only subsequent edits
        // need another write. The chunk source reloads it after eviction.
        decode(&bytes)
    }
}

fn encode_plane(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let value = bytes[i];
        let mut run = 1;
        while i + run < bytes.len() && bytes[i + run] == value { run += 1; }
        out.extend_from_slice(&(run as u32).to_le_bytes());
        out.push(value);
        i += run;
    }
    out
}

/// C4D2: magic, ID payload length, metadata payload length, then two bounded
/// run-length planes. A zero metadata length means every state uses metadata zero.
pub fn encode(chunk: &Chunk) -> Vec<u8> {
    let ids = match &chunk.data {
        ChunkData::Uniform(id) => {
            let mut data = (CHUNK_VOLUME as u32).to_le_bytes().to_vec();
            data.push(*id);
            data
        }
        ChunkData::Dense(ids) => encode_plane(ids),
    };
    let metadata = chunk.metadata_plane().map(encode_plane).unwrap_or_default();
    let mut out = Vec::with_capacity(12 + ids.len() + metadata.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(ids.len() as u32).to_le_bytes());
    out.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    out.extend(ids);
    out.extend(metadata);
    out
}

fn decode_plane(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() % 5 != 0 || bytes.len() > CHUNK_VOLUME * 5 { return None; }
    let mut plane = Vec::with_capacity(CHUNK_VOLUME);
    for record in bytes.chunks_exact(5) {
        let run = u32::from_le_bytes(record[..4].try_into().ok()?) as usize;
        if run == 0 || run > CHUNK_VOLUME - plane.len() { return None; }
        plane.extend(std::iter::repeat_n(record[4], run));
    }
    (plane.len() == CHUNK_VOLUME).then_some(plane)
}

pub fn decode(bytes: &[u8]) -> Option<Chunk> {
    match bytes.get(..4)? {
        magic if magic == LEGACY_MAGIC => Chunk::from_planes(decode_plane(&bytes[4..])?, None),
        magic if magic == MAGIC => {
            let ids_len = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize;
            let metadata_len = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?) as usize;
            if ids_len > CHUNK_VOLUME * 5 || metadata_len > CHUNK_VOLUME * 5 { return None; }
            let ids_end = 12usize.checked_add(ids_len)?;
            let end = ids_end.checked_add(metadata_len)?;
            if end != bytes.len() { return None; }
            let ids = decode_plane(bytes.get(12..ids_end)?)?;
            let metadata = if metadata_len == 0 { None } else { Some(decode_plane(bytes.get(ids_end..end)?)?) };
            Chunk::from_planes(ids, metadata)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_bytes_load_as_zero_metadata_and_new_saves_roundtrip_both_planes() {
        let mut legacy = b"C4D1".to_vec();
        legacy.extend_from_slice(&(CHUNK_VOLUME as u32).to_le_bytes());
        legacy.push(crate::block::WOOL);
        let mut chunk = decode(&legacy).unwrap();
        assert!(chunk.metadata_plane().is_none());
        let p = IVec4::new(15, 2, 4, 9);
        chunk.set_state(p, crate::block::BlockState::new(crate::block::WOOL, 15));
        let encoded = encode(&chunk);
        assert_eq!(&encoded[..4], b"C4D2");
        let restored = decode(&encoded).unwrap();
        assert_eq!(restored.uniform_id(), Some(crate::block::WOOL));
        assert_eq!(restored.get_state(p), chunk.get_state(p));
        assert_eq!(restored.get_state(IVec4::ZERO).metadata, 0);
        assert!(!restored.modified);
        assert_ne!(restored.identity, chunk.identity);
    }

    #[test]
    fn corrupt_state_plane_lengths_and_runs_are_rejected() {
        let encoded = encode(&Chunk::uniform(1));
        for end in 0..encoded.len() { assert!(decode(&encoded[..end]).is_none()); }
        let mut oversized = encoded.clone();
        oversized[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&oversized).is_none());
        let mut zero_run = encoded.clone();
        zero_run[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode(&zero_run).is_none());
        let mut trailing = encoded;
        trailing.push(0);
        assert!(decode(&trailing).is_none());
    }

    #[test]
    fn fluid_states_roundtrip() {
        use crate::block::{FluidKind, LAVA, WATER};
        let mut chunk = Chunk::uniform(0);
        for (x, id) in [WATER, LAVA, FluidKind::Water.flowing(3), FluidKind::Water.flowing(8),
            FluidKind::Lava.flowing(6)].into_iter().enumerate() {
            chunk.set(IVec4::new(x as i32, 1, 0, 0), id);
        }
        let restored = decode(&encode(&chunk)).unwrap();
        assert_eq!(restored.fluid_cells, 5);
        for x in 0..5 { assert_eq!(restored.get(IVec4::new(x, 1, 0, 0)), chunk.get(IVec4::new(x, 1, 0, 0))); }
    }

    #[test]
    fn roundtrip() {
        let mut c = Chunk::uniform(1);
        c.set(IVec4::new(3, 4, 5, 6), 9);
        let d = decode(&encode(&c)).unwrap();
        assert_eq!(d.get(IVec4::new(3, 4, 5, 6)), 9);
        assert_eq!(d.get(IVec4::ZERO), 1);
        assert_eq!(decode(&encode(&Chunk::uniform(0))).unwrap().uniform_id(), Some(0));
    }

    #[test]
    fn persisted_chunks_are_clean_until_edited_again() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("mc4d-save-clean-{}-{stamp}", std::process::id()));
        let save = SaveDir::new(&root).unwrap();
        let c = IVec4::new(1, 2, 3, 4);
        let mut chunk = Chunk::uniform(1);
        chunk.set(IVec4::ZERO, 9);
        save.save_chunk(c, &chunk).unwrap();
        let mut restored = save.load(c).unwrap();
        assert_eq!(restored.get(IVec4::ZERO), 9);
        assert!(!restored.modified);
        restored.set(IVec4::ZERO, 3);
        assert!(restored.modified);
        std::fs::remove_dir_all(root).unwrap();
    }
}
