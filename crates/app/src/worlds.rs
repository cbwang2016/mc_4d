//! Save-list metadata and bounded world management, independent of terrain loading.
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct Entry {
    pub folder: String,
    pub name: String,
    pub modified: u64,
    pub size_bytes: u64,
    pub valid: bool,
}
#[derive(Default, Serialize, Deserialize)]
struct Label {
    name: String,
}
pub fn list(root: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        let Some(folder) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !crate::settings::valid_world_name(&folder) {
            continue;
        }
        let level = entry.path().join("level.toml");
        if !level.is_file() {
            continue;
        }
        let name = std::fs::read_to_string(entry.path().join("world.toml"))
            .ok()
            .and_then(|s| toml::from_str::<Label>(&s).ok())
            .filter(|l| !l.name.trim().is_empty())
            .map_or_else(|| folder.clone(), |l| l.name);
        let valid = std::fs::read_to_string(&level).ok().is_some_and(|text|crate::game::Game::validate_level(&text).is_ok());
        let modified = level
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |t| t.as_secs());
        entries.push(Entry {
            folder,
            name,
            modified,
            size_bytes: directory_size(&entry.path()),
            valid,
        });
    }
    entries.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.folder.cmp(&b.folder)));
    entries
}
// Ignore links: listing one save must never traverse another directory.
fn directory_size(root: &Path) -> u64 {
    let mut total=0u64;
    let mut pending=vec![root.to_owned()];
    while let Some(path)=pending.pop() {
        for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
            let Ok(kind)=entry.file_type() else {continue};
            if kind.is_dir() {pending.push(entry.path());}
            else if kind.is_file() {total=total.saturating_add(entry.metadata().map_or(0,|m|m.len()));}
        }
    }
    total
}
/// Gregorian UTC date without platform-specific local-time APIs.
pub fn details(entry: &Entry) -> String {
    let days=(entry.modified/86400) as i64+719468;
    let era=days/146097;
    let doe=days-era*146097;
    let yoe=(doe-doe/1460+doe/36524-doe/146096)/365;
    let mut year=yoe+era*400;
    let doy=doe-(365*yoe+yoe/4-yoe/100);
    let mp=(5*doy+2)/153;
    let day=doy-(153*mp+2)/5+1;
    let month=mp+if mp<10 {3} else {-9};
    year+=i64::from(month<=2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02} UTC | {:.2} MiB",entry.modified/3600%24,entry.modified/60%60,entry.size_bytes as f64/1048576.0)
}
fn existing(root: &Path, folder: &str) -> io::Result<PathBuf> {
    if !crate::settings::valid_world_name(folder) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid world folder",
        ));
    }
    let path = root.join(folder);
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "world must be a real save directory",
        ));
    }
    if !path.join("level.toml").is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "world metadata missing",
        ));
    }
    Ok(path)
}
pub fn rename(root: &Path, folder: &str, name: &str) -> io::Result<()> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid world name",
        ));
    }
    let path = existing(root, folder)?;
    let text = toml::to_string(&Label { name: name.into() }).map_err(io::Error::other)?;
    std::fs::write(path.join("world.toml.tmp"), text)?;
    std::fs::rename(path.join("world.toml.tmp"), path.join("world.toml"))
}
pub fn delete(root: &Path, folder: &str) -> io::Result<()> {
    std::fs::remove_dir_all(existing(root, folder)?)
}
pub fn seed(text: &str) -> i64 {
    let text = text.trim();
    if text.is_empty() {
        return std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as i64;
    }
    if let Ok(number) = text.parse::<i64>() {
        return number;
    }
    text.encode_utf16()
        .fold(0u32, |hash, c| hash.wrapping_mul(31).wrapping_add(c as u32)) as i32 as i64
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_keeps_chunks_and_delete_is_scoped_to_selected_world() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-world-management")
            .join(format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(root.join("first")).unwrap();
        std::fs::create_dir_all(root.join("second")).unwrap();
        for name in ["first", "second"] {
            std::fs::write(
                root.join(name).join("level.toml"),
                "seed=7\npos=[0,64,0,0]\n",
            )
            .unwrap();
        }
        std::fs::write(root.join("first/chunk.bin"), [1, 2, 3]).unwrap();
        rename(&root, "first", "四维世界").unwrap();
        assert_eq!(
            list(&root)
                .iter()
                .find(|e| e.folder == "first")
                .unwrap()
                .name,
            "四维世界"
        );
        assert_eq!(
            std::fs::read(root.join("first/chunk.bin")).unwrap(),
            [1, 2, 3]
        );
        assert!(delete(&root, "../second").is_err());
        delete(&root, "first").unwrap();
        assert!(root.join("second/level.toml").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn seed_text_has_stable_java_hash_and_accepts_negative_numbers() {
        assert_eq!(seed("abc"), 96354);
        assert_eq!(seed("-1"), -1);
        assert_eq!(seed(" 1337 "), 1337);
        assert_eq!(seed("-9223372036854775808"), i64::MIN);
        assert_eq!(seed("9223372036854775807"), i64::MAX);
        // Out-of-range numbers fall back to String.hashCode, as in Beta.
        assert_eq!(seed("9223372036854775808"), -1_773_151_197);
        assert_eq!(seed("Glacier"), 1_772_835_215);
    }
}
