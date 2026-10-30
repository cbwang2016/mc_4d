//! Texture packs as Beta 1.5 accepts them: a folder or a `.zip` archive whose
//! root (or single top-level folder) holds terrain.png, gui/, mob/ and so on.
//! Only stored and deflated ZIP entries are supported, as produced by every
//! common archiver; encrypted or ZIP64 archives are rejected.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MAX_ENTRY: usize = 64 << 20;

struct Entry { method: u16, compressed: usize, size: usize, local: usize }
struct Archive { bytes: Vec<u8>, entries: HashMap<String, Entry>, prefix: String }

fn u16_at(b: &[u8], at: usize) -> Option<u16> { b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])) }
fn u32_at(b: &[u8], at: usize) -> Option<u32> { b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])) }

impl Archive {
    fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        // End of central directory: last 22 bytes plus up to 64 KiB comment.
        let floor = bytes.len().saturating_sub(22 + 0xffff);
        let end = (floor..=bytes.len().saturating_sub(22)).rev()
            .find(|&i| u32_at(&bytes, i) == Some(0x0605_4b50)).ok_or("not a ZIP archive")?;
        let count = u16_at(&bytes, end + 10).ok_or("truncated ZIP")? as usize;
        let mut at = u32_at(&bytes, end + 16).ok_or("truncated ZIP")? as usize;
        let mut entries = HashMap::new();
        for _ in 0..count {
            if u32_at(&bytes, at) != Some(0x0201_4b50) { return Err("damaged ZIP directory".into()); }
            let flags = u16_at(&bytes, at + 8).ok_or("truncated ZIP")?;
            let method = u16_at(&bytes, at + 10).ok_or("truncated ZIP")?;
            let compressed = u32_at(&bytes, at + 20).ok_or("truncated ZIP")? as usize;
            let size = u32_at(&bytes, at + 24).ok_or("truncated ZIP")? as usize;
            let name_len = u16_at(&bytes, at + 28).ok_or("truncated ZIP")? as usize;
            let extra = u16_at(&bytes, at + 30).ok_or("truncated ZIP")? as usize;
            let comment = u16_at(&bytes, at + 32).ok_or("truncated ZIP")? as usize;
            let local = u32_at(&bytes, at + 42).ok_or("truncated ZIP")? as usize;
            let name = bytes.get(at + 46..at + 46 + name_len).ok_or("truncated ZIP")?;
            if flags & 1 != 0 { return Err("encrypted ZIP archives are not supported".into()); }
            let name = String::from_utf8_lossy(name).replace('\\', "/");
            if !name.ends_with('/') { entries.insert(name, Entry { method, compressed, size, local }); }
            at += 46 + name_len + extra + comment;
        }
        // Many packs are zipped with one enclosing folder.
        let prefix = if entries.contains_key("terrain.png") { String::new() } else {
            entries.keys().find_map(|k| k.strip_suffix("terrain.png").filter(|p| p.matches('/').count() == 1).map(str::to_owned)).unwrap_or_default()
        };
        Ok(Self { bytes, entries, prefix })
    }

    fn read(&self, name: &str) -> Option<Vec<u8>> {
        let e = self.entries.get(&format!("{}{name}", self.prefix))?;
        if u32_at(&self.bytes, e.local)? != 0x0403_4b50 || e.size > MAX_ENTRY { return None; }
        let start = e.local + 30 + u16_at(&self.bytes, e.local + 26)? as usize + u16_at(&self.bytes, e.local + 28)? as usize;
        let data = self.bytes.get(start..start + e.compressed)?;
        match e.method {
            0 => Some(data.to_vec()),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, e.size.max(1)).ok().filter(|d| d.len() == e.size),
            _ => None,
        }
    }
}

/// The most recently opened archive; packs are read file by file at load time.
fn archive(path: &Path) -> Result<Arc<Archive>, String> {
    static CACHE: Mutex<Option<(PathBuf, std::time::SystemTime, Arc<Archive>)>> = Mutex::new(None);
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).map_err(|e| e.to_string())?;
    let mut cache = CACHE.lock().unwrap();
    if let Some((cached, time, archive)) = cache.as_ref() && cached == path && *time == modified { return Ok(archive.clone()); }
    let archive = Arc::new(Archive::parse(std::fs::read(path).map_err(|e| e.to_string())?)?);
    *cache = Some((path.to_owned(), modified, archive.clone()));
    Ok(archive)
}

pub fn is_zip(path: &Path) -> bool {
    path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))
}

/// Check that `path` is a folder or a readable ZIP texture pack.
pub fn validate(path: &Path) -> Result<(), String> {
    if path.is_dir() { return Ok(()); }
    if is_zip(path) { return archive(path).map(|_| ()); }
    Err("texture pack must be an existing folder or .zip file".into())
}

/// A file from a folder or ZIP pack, or None if the pack does not override it.
pub fn read(pack: &Path, name: &str) -> Option<Vec<u8>> {
    if is_zip(pack) { archive(pack).ok()?.read(name) } else { std::fs::read(pack.join(name)).ok() }
}

/// Folders and ZIP archives directly inside `root`, sorted by name.
pub fn list(root: &Path) -> Vec<PathBuf> {
    let mut packs: Vec<_> = std::fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path())
        .filter(|p| p.is_dir() || is_zip(p)).collect();
    packs.sort();
    packs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal ZIP writer for tests: stored or raw-deflated entries.
    fn zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let (mut out, mut directory) = (Vec::new(), Vec::new());
        for &(name, data, deflate) in files {
            let body = if deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
            let local = out.len() as u32;
            let header = |sig: u32, central: bool| {
                let mut h = sig.to_le_bytes().to_vec();
                if central { h.extend(20u16.to_le_bytes()); }
                h.extend(20u16.to_le_bytes()); h.extend(0u16.to_le_bytes()); h.extend((if deflate { 8u16 } else { 0 }).to_le_bytes());
                h.extend([0; 8]); h.extend((body.len() as u32).to_le_bytes()); h.extend((data.len() as u32).to_le_bytes());
                h.extend((name.len() as u16).to_le_bytes()); h.extend(0u16.to_le_bytes());
                if central { h.extend([0; 6]); h.extend(0u32.to_le_bytes()); h.extend(local.to_le_bytes()); }
                h.extend(name.as_bytes()); h
            };
            out.extend(header(0x0403_4b50, false)); out.extend(&body);
            directory.extend(header(0x0201_4b50, true));
        }
        let start = out.len() as u32;
        out.extend(&directory);
        out.extend(0x0605_4b50u32.to_le_bytes()); out.extend([0; 4]);
        out.extend((files.len() as u16).to_le_bytes()); out.extend((files.len() as u16).to_le_bytes());
        out.extend((directory.len() as u32).to_le_bytes()); out.extend(start.to_le_bytes()); out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn reads_stored_and_deflated_entries_at_root_or_in_one_folder() {
        let terrain = vec![7u8; 5000];
        for prefix in ["", "MyPack/"] {
            let bytes = zip(&[(&format!("{prefix}terrain.png"), &terrain, true), (&format!("{prefix}gui/items.png"), b"items", false)]);
            let archive = Archive::parse(bytes).unwrap();
            assert_eq!(archive.read("terrain.png").unwrap(), terrain);
            assert_eq!(archive.read("gui/items.png").unwrap(), b"items");
            assert!(archive.read("mob/pig.png").is_none());
        }
        assert!(Archive::parse(b"not a zip at all, definitely not".to_vec()).is_err());
    }

    #[test]
    fn real_zip_file_is_listed_validated_and_read() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-texture-packs").join(std::process::id().to_string());
        std::fs::create_dir_all(root.join("Folder Pack")).unwrap();
        std::fs::write(root.join("Zipped.zip"), zip(&[("terrain.png", b"png", true)])).unwrap();
        std::fs::write(root.join("notes.txt"), b"x").unwrap();
        let packs = list(&root);
        assert_eq!(packs.iter().map(|p| p.file_name().unwrap().to_str().unwrap()).collect::<Vec<_>>(), ["Folder Pack", "Zipped.zip"]);
        validate(&packs[1]).unwrap();
        assert_eq!(read(&packs[1], "terrain.png").unwrap(), b"png");
        assert!(validate(&root.join("notes.txt")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
