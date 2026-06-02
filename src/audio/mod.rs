//! The song library: discovering WAV files in a directory.

pub mod playback;
pub mod stream;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Song {
    pub name: String,
    pub path: PathBuf,
}

/// Load every WAV file in `dir`, sorted case-insensitively by name. Hidden files
pub fn load_dir(dir: &Path) -> Result<Vec<Song>> {
    let entries =
        fs::read_dir(dir).with_context(|| format!("reading song directory {}", dir.display()))?;

    let mut songs = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name.starts_with('.') {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext != "wav" && ext != "wave" {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("untitled")
            .to_string();
        songs.push(Song { name, path });
    }
    songs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(songs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_dir_keeps_only_wav_files() {
        let dir = std::env::temp_dir().join(format!("mb_lib_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.wav"), b"x").unwrap();
        std::fs::write(dir.join("b.txt"), b"x").unwrap();
        std::fs::write(dir.join("c.WAV"), b"x").unwrap();

        let songs = load_dir(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        let names: Vec<_> = songs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["a", "c"]); // b.txt skipped, sorted case-insensitively
    }
}
