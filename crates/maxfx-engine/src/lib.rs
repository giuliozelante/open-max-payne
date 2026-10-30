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

/// Level package entries inside an opened `x_level*.ras` (paths use archive win form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelPackage {
    pub ldb: String,
    pub ai: Option<String>,
}

/// Observed BYOD `.ldb` / `.ai` file preamble: 16 × `0x20` then ASCII `>` (`0x3e`).
pub const LDB_PREAMBLE: &[u8] = b"                >"; // 16 spaces + '>'

pub fn looks_like_ldb_or_ai(data: &[u8]) -> bool {
    data.len() >= LDB_PREAMBLE.len() && data.starts_with(LDB_PREAMBLE)
}

/// One `>`-terminated record from a `.ldb` / `.ai` body (BYOD observation).
///
/// Files start with [`LDB_PREAMBLE`], then a stream of payloads separated by
/// ASCII `>` (`0x3e`). Payloads are **space-padded** (`0x20`); after stripping
/// spaces, retail `.ai` records often begin with opcode-like bytes (e.g. `0xc7`)
/// and embed MemoryFile-style tags (`0x14` / `0x09` / `0x16`). Full opcode map
/// still needs `X_LevelDBImportLevel` REA (issue #14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LdbRecord {
    /// Bytes between the previous `>` and this record's terminating `>` (exclusive of `>`).
    pub raw: Vec<u8>,
}

impl LdbRecord {
    /// Payload with `0x20` padding removed (lossy view for opcode probing).
    pub fn space_stripped(&self) -> Vec<u8> {
        self.raw.iter().copied().filter(|&b| b != 0x20).collect()
    }

    /// First non-space byte, if any.
    pub fn lead_byte(&self) -> Option<u8> {
        self.raw.iter().copied().find(|&b| b != 0x20)
    }
}

/// Split `.ldb` / `.ai` bytes into `>`-delimited records after the shared preamble.
///
/// Returns `None` if the preamble does not match. The preamble's own trailing
/// `>` starts the first record boundary (empty leading chunk is dropped).
pub fn split_ldb_records(data: &[u8]) -> Option<Vec<LdbRecord>> {
    if !looks_like_ldb_or_ai(data) {
        return None;
    }
    let body = &data[LDB_PREAMBLE.len()..]; // after first '>'
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &b) in body.iter().enumerate() {
        if b == b'>' {
            out.push(LdbRecord {
                raw: body[start..i].to_vec(),
            });
            start = i + 1;
        }
    }
    if start < body.len() {
        out.push(LdbRecord {
            raw: body[start..].to_vec(),
        });
    }
    Some(out)
}

/// One level entry from `\data\database\levels\levels.txt` (SharedDB catalog).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelsTxtEntry {
    /// Block name, e.g. `part0_level1`.
    pub id: String,
    /// `Directory` property (e.g. `part1`, `front_end`).
    pub directory: String,
    /// `Level` property basename (e.g. `part0_level1.ldb`).
    pub level_file: String,
}

impl LevelsTxtEntry {
    /// In-archive win path: `\data\database\levels\{Directory}\{Level}`.
    pub fn ldb_archive_path(&self) -> String {
        format!(
            "\\data\\database\\levels\\{}\\{}",
            self.directory, self.level_file
        )
    }

    /// Sibling `.ai` path (same basename); may be absent in the RAS.
    pub fn ai_archive_path(&self) -> String {
        let base = self
            .level_file
            .trim_end_matches(".ldb")
            .trim_end_matches(".LDB");
        format!("\\data\\database\\levels\\{}\\{base}.ai", self.directory)
    }
}

/// Parse `levels.txt` SharedDB catalog into archive-relative `.ldb` paths.
///
/// Path construction (BYOD + exe strings `Levels.txt`,
/// `Level "%s" not found from the database`):
/// `\data\database\levels\` + `Directory` + `\` + `Level`.
pub fn parse_levels_txt(text: &str) -> Vec<LevelsTxtEntry> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Find `[id]`
        let Some(rel) = text[i..].find('[') else {
            break;
        };
        let start = i + rel;
        let Some(end_rel) = text[start + 1..].find(']') else {
            break;
        };
        let id = text[start + 1..start + 1 + end_rel].trim();
        let after = start + 1 + end_rel + 1;
        // Skip nested property section headers like [Properties] — only top-level
        // blocks that own Directory+Level count. Look ahead for `{`.
        let rest = text[after..].trim_start();
        if !rest.starts_with('{') {
            i = after;
            continue;
        }
        let body_start = after + (text[after..].len() - rest.len()) + 1;
        let Some(body_end_rel) = text[body_start..].find('}') else {
            break;
        };
        let body = &text[body_start..body_start + body_end_rel];
        i = body_start + body_end_rel + 1;

        // Nested `[Properties]` etc. appear inside body; still fine for field scan.
        let directory = prop_value(body, "Directory");
        let level_file = prop_value(body, "Level");
        let (Some(directory), Some(level_file)) = (directory, level_file) else {
            continue;
        };
        if id.eq_ignore_ascii_case("Properties")
            || id.eq_ignore_ascii_case("Difficulty")
            || id.eq_ignore_ascii_case("Timedmode")
        {
            continue;
        }
        out.push(LevelsTxtEntry {
            id: id.to_string(),
            directory,
            level_file,
        });
    }
    out
}

fn prop_value(body: &str, key: &str) -> Option<String> {
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if !k.trim().eq_ignore_ascii_case(key) {
            continue;
        }
        let v = v.trim().trim_end_matches(';').trim();
        let v = v.trim_matches('"').trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

/// Pair `.ldb` / `.ai` basenames under `\\data\\database\\levels\\` (BYOD probe helper).
pub fn list_level_packages(arc: &RasArchive) -> Vec<LevelPackage> {
    let mut ldbs: Vec<String> = Vec::new();
    let mut ais: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for f in &arc.files {
        let dname = arc
            .directories
            .get(f.dir_index as usize)
            .map(|d| d.name.as_str())
            .unwrap_or("\\");
        let full = format!("{dname}{}", f.name);
        let lower = full.to_ascii_lowercase();
        if !lower.contains("\\data\\database\\levels\\") {
            continue;
        }
        if lower.ends_with(".ldb") {
            ldbs.push(full);
        } else if lower.ends_with(".ai") {
            let key = lower.trim_end_matches(".ai").to_string();
            ais.insert(key, full);
        }
    }
    ldbs.sort();
    ldbs.into_iter()
        .map(|ldb| {
            let key = ldb
                .to_ascii_lowercase()
                .trim_end_matches(".ldb")
                .to_string();
            LevelPackage {
                ai: ais.get(&key).cloned(),
                ldb,
            }
        })
        .collect()
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
    fn ldb_preamble_matches_byod_observation() {
        let mut buf = Vec::from(LDB_PREAMBLE);
        buf.extend_from_slice(&[0x00, 0x01, 0x02]);
        assert!(looks_like_ldb_or_ai(&buf));
        assert!(!looks_like_ldb_or_ai(b"RAS\0"));
    }

    #[test]
    fn split_ldb_records_on_gt_after_preamble() {
        let mut buf = Vec::from(LDB_PREAMBLE);
        buf.extend_from_slice(b"  \xc7  \x14\n  >");
        buf.extend_from_slice(b"abc>");
        buf.extend_from_slice(b"tail");
        let recs = split_ldb_records(&buf).unwrap();
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].lead_byte(), Some(0xc7));
        assert_eq!(recs[0].space_stripped(), vec![0xc7, 0x14, 0x0a]);
        assert_eq!(recs[1].raw, b"abc");
        assert_eq!(recs[2].raw, b"tail");
        assert!(split_ldb_records(b"nope").is_none());
    }

    #[test]
    fn list_level_packages_pairs_ldb_ai() {
        let arc = RasArchive {
            seed: 1,
            version: 1.2,
            compatibility: 3,
            directories: vec![
                RasDirEntry {
                    name: "\\".into(),
                    timestamp: [0; 16],
                },
                RasDirEntry {
                    name: "\\data\\database\\levels\\part1\\".into(),
                    timestamp: [0; 16],
                },
            ],
            files: vec![
                RasFileEntry {
                    name: "Part1_Level1.ldb".into(),
                    dir_index: 1,
                    data: b"LDB".to_vec(),
                    compression_mode: MODE_STORED,
                    compressed_data: None,
                    unknown_0: 0,
                    unknown_1: 0,
                    timestamp: [0; 16],
                },
                RasFileEntry {
                    name: "Part1_Level1.ai".into(),
                    dir_index: 1,
                    data: b"AI".to_vec(),
                    compression_mode: MODE_STORED,
                    compressed_data: None,
                    unknown_0: 0,
                    unknown_1: 0,
                    timestamp: [0; 16],
                },
            ],
        };
        let pkgs = list_level_packages(&arc);
        assert_eq!(pkgs.len(), 1);
        assert!(pkgs[0].ldb.ends_with("Part1_Level1.ldb"));
        assert!(pkgs[0].ai.as_ref().unwrap().ends_with("Part1_Level1.ai"));
    }

    #[test]
    fn parse_levels_txt_builds_archive_paths() {
        let txt = r#"
[startup_level]
{
	[Properties]
	Directory 		= front_end;
	Level 			= startup_level.ldb;
}
[part0_level1]
{
	[Properties]
	Directory 		= part1;
	Level 			= part0_level1.ldb;
}
"#;
        let entries = parse_levels_txt(txt);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "startup_level");
        assert_eq!(
            entries[0].ldb_archive_path(),
            "\\data\\database\\levels\\front_end\\startup_level.ldb"
        );
        assert_eq!(
            entries[1].ai_archive_path(),
            "\\data\\database\\levels\\part1\\part0_level1.ai"
        );
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
