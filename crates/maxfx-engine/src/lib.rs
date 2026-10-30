//! Native Max Payne 1 engine — walking skeleton only.
//!
//! Loads a BYOD RAS/MPM via `openras`. Rendering / gameplay come later
//! (see `docs/specs/2026-10-30-engine-skeleton.md`).

use openras::RasArchive;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Ras(#[from] openras::RasError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Msg(String),
}

/// Minimal handle: an opened retail/mod archive the engine will eventually drive.
#[derive(Debug)]
pub struct EngineArchive {
    pub label: String,
    pub file_count: usize,
    pub dir_count: usize,
    pub compatibility: u32,
    pub version: f32,
}

pub fn open_archive(path: &Path) -> Result<EngineArchive, EngineError> {
    let arc = RasArchive::read(path)?;
    if arc.compatibility != openras::archive::COMPAT_MP1 {
        return Err(EngineError::Msg(format!(
            "unsupported RAS compatibility {} (want {})",
            arc.compatibility,
            openras::archive::COMPAT_MP1
        )));
    }
    Ok(EngineArchive {
        label: path.display().to_string(),
        file_count: arc.files.len(),
        dir_count: arc.directories.len(),
        compatibility: arc.compatibility,
        version: arc.version,
    })
}

/// Discover `*.ras` / `*.RAS` under `dir` — mirrors `R_File::loadArchives` FindFirstFile glob
/// (order is filesystem-dependent, same as the retail loader).
pub fn discover_ras_files(dir: &Path) -> Result<Vec<std::path::PathBuf>, EngineError> {
    let mut out = Vec::new();
    for ent in std::fs::read_dir(dir)? {
        let ent = ent?;
        let name = ent.file_name();
        let s = name.to_string_lossy();
        if s.len() >= 4 && s[s.len() - 4..].eq_ignore_ascii_case(".ras") {
            out.push(ent.path());
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openras::archive::{RasArchive, RasDirEntry, RasFileEntry, MODE_STORED};

    fn sample_arc() -> RasArchive {
        RasArchive {
            seed: 1,
            version: 1.2,
            compatibility: 3,
            directories: vec![RasDirEntry {
                name: "\\".into(),
                timestamp: [0; 16],
            }],
            files: vec![RasFileEntry {
                name: "dummy.txt".into(),
                dir_index: 0,
                data: b"x".to_vec(),
                compression_mode: MODE_STORED,
                compressed_data: None,
                unknown_0: 0,
                unknown_1: 0,
                timestamp: [0; 16],
            }],
        }
    }

    #[test]
    fn opens_synthetic_mp1_archive() {
        let dir = std::env::temp_dir().join(format!("maxfx-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.ras");
        sample_arc().write(&path, false).unwrap();
        let opened = open_archive(&path).unwrap();
        assert_eq!(opened.file_count, 1);
        assert_eq!(opened.compatibility, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_finds_ras_case_insensitive() {
        let dir = std::env::temp_dir().join(format!("maxfx-engine-disc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        sample_arc().write(&dir.join("a.RAS"), false).unwrap();
        sample_arc().write(&dir.join("b.ras"), false).unwrap();
        std::fs::write(dir.join("nope.txt"), b"x").unwrap();
        let found = discover_ras_files(&dir).unwrap();
        assert_eq!(found.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
