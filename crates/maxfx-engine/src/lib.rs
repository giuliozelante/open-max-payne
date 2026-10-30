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

/// Legacy probe: 16 × `0x20` then ASCII `>` (`0x3e`).
///
/// **Retracted for retail RAS extracts** (front_end / part levels): those `.ldb`
/// files start with [`LDB_RAS_MAGIC`] and `.ai` with MemoryFile `0x14` tags.
/// `0x3e` inside `.ai` is often a float byte, not a record delimiter. Kept for
/// synthetic tests and any still-unseen cook path.
pub const LDB_PREAMBLE: &[u8] = b"                >"; // 16 spaces + '>'

/// Retail RAS `.ldb` section tag byte (first byte of each top-level section).
pub const LDB_SECTION_TAG: u8 = 0x1c;

/// Vertex-cloud section kind (byte after tag): `1c 12` → LE u16 `0x121c`.
pub const LDB_SECTION_KIND_VEC3: u8 = 0x12;

/// Follow-on section kind (byte after tag): `1c 13` → often MemoryFile-tagged body.
pub const LDB_SECTION_KIND_MF: u8 = 0x13;

/// Convenience: LE u16 for a vec3 section header word0 (`1c 12`).
pub const LDB_RAS_MAGIC: u16 = u16::from_le_bytes([LDB_SECTION_TAG, LDB_SECTION_KIND_VEC3]);

/// Vec3 section head: `tag` + `kind` + `count_u16` + `flag_u8` (= 5 bytes).
pub const LDB_RAS_HEADER_LEN: usize = 5;

/// `X_LevelDBImportLevel` MemoryFile version gate (`operator>>` int must be this).
/// REA: MaxPayne.exe `0x5ec350` vs string `X_LevelDB Wrong version, expected %d…`.
pub const LDB_IMPORT_VERSION: u32 = 0x20;

/// True for any currently recognized level blob layout.
pub fn looks_like_ldb_or_ai(data: &[u8]) -> bool {
    looks_like_ras_ldb(data)
        || looks_like_memory_file_ai(data)
        || looks_like_memory_file_ldb(data)
        || (data.len() >= LDB_PREAMBLE.len() && data.starts_with(LDB_PREAMBLE))
}

/// Retail RAS `.ldb`: section tag [`LDB_SECTION_TAG`] + kind [`LDB_SECTION_KIND_VEC3`].
pub fn looks_like_ras_ldb(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == LDB_SECTION_TAG && data[1] == LDB_SECTION_KIND_VEC3
}

/// One top-level RAS `.ldb` section head (`1c <kind> <count_u16> …`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasLdbSection {
    pub kind: u8,
    pub count: u16,
    /// Present for kind `0x12` (vec3 cloud); first body byte for kind `0x13` is often `0x14`.
    pub flag: Option<u8>,
}

/// Parsed first (vec3) section of a retail RAS `.ldb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasLdbHeader {
    pub magic: u16,
    pub count: u16,
    pub flag: u8,
}

pub fn parse_ras_ldb_header(data: &[u8]) -> Option<RasLdbHeader> {
    let sec = parse_ras_ldb_section_at(data, 0)?;
    if sec.kind != LDB_SECTION_KIND_VEC3 {
        return None;
    }
    Some(RasLdbHeader {
        magic: LDB_RAS_MAGIC,
        count: sec.count,
        flag: sec.flag?,
    })
}

/// Parse a `1c <kind> <u16>` section head at `offset`.
///
/// Kind `0x12` consumes an extra flag byte. Kind `0x13` does not (body starts
/// immediately — BYOD Startup section-2 begins `14 00 14 03 …`).
pub fn parse_ras_ldb_section_at(data: &[u8], offset: usize) -> Option<RasLdbSection> {
    if data.len() < offset + 4 || data[offset] != LDB_SECTION_TAG {
        return None;
    }
    let kind = data[offset + 1];
    let count = u16::from_le_bytes([data[offset + 2], data[offset + 3]]);
    match kind {
        LDB_SECTION_KIND_VEC3 => {
            if data.len() < offset + 5 {
                return None;
            }
            Some(RasLdbSection {
                kind,
                count,
                flag: Some(data[offset + 4]),
            })
        }
        LDB_SECTION_KIND_MF => Some(RasLdbSection {
            kind,
            count,
            flag: None,
        }),
        _ => Some(RasLdbSection {
            kind,
            count,
            flag: None,
        }),
    }
}

/// Byte offset just past the first vec3 section header + `header.count` × 13.
pub fn ras_ldb_count_section_end(header: &RasLdbHeader) -> usize {
    LDB_RAS_HEADER_LEN + header.count as usize * 13
}

/// Count contiguous `0x16` + 12-byte float triples after the RAS header.
pub fn count_ras_ldb_leading_vec3(data: &[u8]) -> usize {
    if data.len() < LDB_RAS_HEADER_LEN {
        return 0;
    }
    let mut i = LDB_RAS_HEADER_LEN;
    let mut n = 0;
    while i + 13 <= data.len() && data[i] == 0x16 {
        n += 1;
        i += 13;
    }
    n
}

/// One face-like record from a kind-`0x13` MemoryFile body (BYOD Startup).
///
/// Vertex indices are compressed ints (`0x14`/`0x13`/…) — not fixed u8 — so
/// they can address the full `0x12` vertex cloud past 255.
#[derive(Debug, Clone, PartialEq)]
pub struct RasLdbFace {
    pub indices: [i32; 3],
    pub attr: i32,
    pub normal: [f32; 3],
    pub point: [f32; 3],
}

fn read_mf_i32(data: &[u8], i: &mut usize) -> Option<i32> {
    if *i >= data.len() {
        return None;
    }
    match data[*i] {
        0x14 => {
            *i += 1;
            if *i >= data.len() {
                return None;
            }
            let v = data[*i] as i32;
            *i += 1;
            Some(v)
        }
        0x13 => {
            *i += 1;
            if *i + 2 > data.len() {
                return None;
            }
            let mut b = [0u8; 4];
            b[..2].copy_from_slice(&data[*i..*i + 2]);
            *i += 2;
            Some(i32::from_le_bytes(b))
        }
        0x12 => {
            *i += 1;
            if *i + 3 > data.len() {
                return None;
            }
            let mut b = [0u8; 4];
            b[..3].copy_from_slice(&data[*i..*i + 3]);
            *i += 3;
            Some(i32::from_le_bytes(b))
        }
        0x02 => {
            *i += 1;
            if *i + 4 > data.len() {
                return None;
            }
            let v = i32::from_le_bytes(data[*i..*i + 4].try_into().ok()?);
            *i += 4;
            Some(v)
        }
        _ => None,
    }
}

fn read_mf_vec3(data: &[u8], i: &mut usize) -> Option<[f32; 3]> {
    if *i + 13 > data.len() || data[*i] != 0x16 {
        return None;
    }
    *i += 1;
    let x = f32::from_le_bytes(data[*i..*i + 4].try_into().ok()?);
    let y = f32::from_le_bytes(data[*i + 4..*i + 8].try_into().ok()?);
    let z = f32::from_le_bytes(data[*i + 8..*i + 12].try_into().ok()?);
    *i += 12;
    Some([x, y, z])
}

/// Parse up to `max` face-like records from a kind-`0x13` section body (no header).
pub fn parse_ras_ldb_faces(body: &[u8], max: usize) -> Vec<RasLdbFace> {
    parse_ras_ldb_faces_ex(body, max).0
}

/// Kind-`0x13` record: normal + point + six compressed ints (BYOD Startup §3).
#[derive(Debug, Clone, PartialEq)]
pub struct RasLdbSurf {
    pub normal: [f32; 3],
    pub point: [f32; 3],
    pub attrs: [i32; 6],
}

/// Parse `max` compressed ints from a kind-`0x13` body (BYOD Startup §4 index list).
pub fn parse_ras_ldb_ints_ex(body: &[u8], max: usize) -> (Vec<i32>, usize) {
    let mut out = Vec::with_capacity(max);
    let mut i = 0;
    while out.len() < max && i < body.len() {
        match read_mf_i32(body, &mut i) {
            Some(v) => out.push(v),
            None => break,
        }
    }
    (out, i)
}

/// Parse up to `max` surf records; returns `(records, bytes_consumed)`.
pub fn parse_ras_ldb_surfs_ex(body: &[u8], max: usize) -> (Vec<RasLdbSurf>, usize) {
    let mut out = Vec::new();
    let mut i = 0;
    while out.len() < max && i < body.len() {
        let start = i;
        let Some(normal) = read_mf_vec3(body, &mut i) else {
            break;
        };
        let Some(point) = read_mf_vec3(body, &mut i) else {
            break;
        };
        let mut attrs = [0i32; 6];
        let mut ok = true;
        for a in &mut attrs {
            match read_mf_i32(body, &mut i) {
                Some(v) => *a = v,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok || i == start {
            break;
        }
        out.push(RasLdbSurf {
            normal,
            point,
            attrs,
        });
    }
    (out, i)
}

/// Like [`parse_ras_ldb_faces`], also returning bytes consumed from `body`.
pub fn parse_ras_ldb_faces_ex(body: &[u8], max: usize) -> (Vec<RasLdbFace>, usize) {
    let mut out = Vec::new();
    let mut i = 0;
    while out.len() < max && i < body.len() {
        let start = i;
        let Some(i0) = read_mf_i32(body, &mut i) else {
            break;
        };
        let Some(i1) = read_mf_i32(body, &mut i) else {
            break;
        };
        let Some(i2) = read_mf_i32(body, &mut i) else {
            break;
        };
        let Some(attr) = read_mf_i32(body, &mut i) else {
            break;
        };
        let Some(normal) = read_mf_vec3(body, &mut i) else {
            break;
        };
        let Some(point) = read_mf_vec3(body, &mut i) else {
            break;
        };
        if i == start {
            break;
        }
        out.push(RasLdbFace {
            indices: [i0, i1, i2],
            attr,
            normal,
            point,
        });
    }
    (out, i)
}

/// Body slice of the first kind-`0x13` section after a flag-0 vec3 cloud.
pub fn ras_ldb_first_mf_section_body(data: &[u8]) -> Option<&[u8]> {
    let h = parse_ras_ldb_header(data)?;
    if h.flag != 0 {
        return None;
    }
    let end = ras_ldb_count_section_end(&h);
    let sec = parse_ras_ldb_section_at(data, end)?;
    if sec.kind != LDB_SECTION_KIND_MF {
        return None;
    }
    Some(&data[end + 4..])
}

/// Top-level `0x1c` sections with known sizes (vec3 cloud + face `0x13` only).
///
/// Stops before a kind-`0x13` section whose body is not face-shaped (Startup's
/// second `0x13`), leaving its offset as the walk end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasLdbSectionSpan {
    pub offset: usize,
    pub kind: u8,
    pub count: u16,
    pub end: usize,
}

/// Byte offset where geometry `0x1c` sections end and the ImportLevel MF stream
/// begins (expects `0x14 0x20` = [`LDB_IMPORT_VERSION`]).
pub fn ras_ldb_after_geometry(data: &[u8]) -> Option<usize> {
    let spans = walk_ras_ldb_known_sections(data);
    let end = spans.last().filter(|s| s.end > s.offset).map(|s| s.end)?;
    if data.len() >= end + 2 && data[end] == 0x14 && data[end + 1] == LDB_IMPORT_VERSION as u8 {
        Some(end)
    } else {
        None
    }
}

/// Harvest MemoryFile `0x0d` path strings from the ImportLevel stream after geometry.
pub fn ras_ldb_import_paths(data: &[u8], max: usize) -> Vec<String> {
    let Some(at) = ras_ldb_after_geometry(data) else {
        return Vec::new();
    };
    let slice = &data[at..];
    let mut out = Vec::new();
    let mut i = 0;
    while i < slice.len() && out.len() < max {
        if slice[i] == 0x0d && i + 2 < slice.len() && slice[i + 1] == 0x14 {
            let len = slice[i + 2] as usize;
            let start = i + 3;
            if start + len <= slice.len() {
                let bytes = &slice[start..start + len];
                if bytes.iter().all(|&b| (0x20..=0x7e).contains(&b)) {
                    let s = String::from_utf8(bytes.to_vec()).unwrap_or_default();
                    if s.contains('\\') || s.contains('/') || s.contains('.') {
                        out.push(s);
                    }
                }
                i = start + len;
                continue;
            }
        }
        i += 1;
    }
    out
}

pub fn walk_ras_ldb_known_sections(data: &[u8]) -> Vec<RasLdbSectionSpan> {
    let mut out = Vec::new();
    let mut off = 0;
    loop {
        let Some(sec) = parse_ras_ldb_section_at(data, off) else {
            break;
        };
        match sec.kind {
            LDB_SECTION_KIND_VEC3 => {
                let end = off + LDB_RAS_HEADER_LEN + sec.count as usize * 13;
                if end > data.len() {
                    break;
                }
                out.push(RasLdbSectionSpan {
                    offset: off,
                    kind: sec.kind,
                    count: sec.count,
                    end,
                });
                off = end;
            }
            LDB_SECTION_KIND_MF => {
                let body = &data[off + 4..];
                let n = sec.count as usize;
                let (faces, face_bytes) = parse_ras_ldb_faces_ex(body, n);
                let (surfs, surf_bytes) = parse_ras_ldb_surfs_ex(body, n);
                let (ints, int_bytes) = parse_ras_ldb_ints_ex(body, n);
                let consumed = if faces.len() == n {
                    face_bytes
                } else if surfs.len() == n {
                    surf_bytes
                } else if ints.len() == n {
                    int_bytes
                } else {
                    out.push(RasLdbSectionSpan {
                        offset: off,
                        kind: sec.kind,
                        count: sec.count,
                        end: off,
                    });
                    break;
                };
                let end = off + 4 + consumed;
                out.push(RasLdbSectionSpan {
                    offset: off,
                    kind: sec.kind,
                    count: sec.count,
                    end,
                });
                off = end;
            }
            _ => break,
        }
        if out.len() > 64 {
            break;
        }
    }
    out
}

/// MaxED MemoryFile-native `.ldb` prefix: tagged int `LDB_IMPORT_VERSION` (`0x14 0x20`).
pub fn looks_like_memory_file_ldb(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0x14 && data[1] == LDB_IMPORT_VERSION as u8
}

/// Retail RAS `.ai`: MemoryFile int tag stream (first byte `0x14`). BYOD Tutorial.ai
/// opens `14 7b 14 0a …` (ints 123 / 10) — not the ImportLevel `0x20` gate.
pub fn looks_like_memory_file_ai(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == 0x14 && data[1] != LDB_IMPORT_VERSION as u8
}

/// First four compressed ints from a retail RAS `.ai` head (BYOD Tutorial:
/// 123, 10, 0, 0). Full schema still WIP — walker dies early on later tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasAiHeader {
    pub a: i32,
    pub b: i32,
    pub c: i32,
    pub d: i32,
}

pub fn parse_ras_ai_header(data: &[u8]) -> Option<RasAiHeader> {
    if !looks_like_memory_file_ai(data) {
        return None;
    }
    let mut i = 0;
    let a = read_mf_i32(data, &mut i)?;
    let b = read_mf_i32(data, &mut i)?;
    let c = read_mf_i32(data, &mut i)?;
    let d = read_mf_i32(data, &mut i)?;
    Some(RasAiHeader { a, b, c, d })
}

/// Best-effort label for a space-stripped record lead byte (BYOD `.ai` histogram).
pub fn classify_ldb_lead(lead: u8) -> &'static str {
    match lead {
        0x14 | 0x11 | 0x09 | 0x0d | 0x16 | 0x13 | 0x10 => "mf-tag",
        0x30 => "type-0x30",
        0x20 => "type-0x20",
        0x71 => "type-0x71",
        0xc7 => "ai-c7",
        0xcb => "ai-cb",
        0x80 => "ai-80",
        0x89 => "ai-89",
        _ => "other",
    }
}

/// BYOD `.ai` probe: records whose space-stripped payload starts `c7 <u8>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiC7Record {
    pub sub: u8,
    /// Bytes after `c7 <sub>` (often MemoryFile-tagged).
    pub tail: Vec<u8>,
}

/// BYOD `.ai` probe: dense payloads starting `89 6a 93 7a` (common lead family).
pub const AI_89_MAGIC: &[u8] = &[0x89, 0x6a, 0x93, 0x7a];

impl LdbRecord {
    /// Parse as `c7` opcode record if the dense payload matches.
    pub fn as_ai_c7(&self) -> Option<AiC7Record> {
        let s = self.space_stripped();
        if s.len() < 2 || s[0] != 0xc7 {
            return None;
        }
        Some(AiC7Record {
            sub: s[1],
            tail: s[2..].to_vec(),
        })
    }

    /// True when space-stripped payload begins with [`AI_89_MAGIC`].
    pub fn is_ai_89_magic(&self) -> bool {
        self.space_stripped().starts_with(AI_89_MAGIC)
    }
}

/// One `>`-terminated record from a `.ldb` / `.ai` body (BYOD observation).
///
/// Files start with [`LDB_PREAMBLE`], then a stream of payloads separated by
/// ASCII `>` (`0x3e`). Payloads are **space-padded** (`0x20`); after stripping
/// spaces, retail `.ai` records often begin with opcode-like bytes (e.g. `0xc7`)
/// and embed MemoryFile-style tags (`0x14` / `0x09` / `0x16`).
///
/// Root framing is **not** `R_MemoryFile` chunk tag `0x0c` (rlmfc REA; see
/// `docs/specs/2026-10-30-leveldb-paths.md`). Opcode map still needs
/// `X_LevelDBImportLevel` REA (issue #14).
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
    // Legacy cook only — do not use on RAS `0x121c` / MF `.ai` blobs.
    if data.len() < LDB_PREAMBLE.len() || !data.starts_with(LDB_PREAMBLE) {
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
        assert_eq!(LDB_IMPORT_VERSION, 0x20);
        assert!(looks_like_memory_file_ldb(&[0x14, 0x20, 0x00]));
        assert!(!looks_like_memory_file_ldb(LDB_PREAMBLE));
        assert_eq!(classify_ldb_lead(0x14), "mf-tag");
        assert_eq!(classify_ldb_lead(0xc7), "ai-c7");
        let mut magic = Vec::from(LDB_PREAMBLE);
        magic.extend_from_slice(b"\x89\x6a\x93\x7a\x00>");
        let recs = split_ldb_records(&magic).unwrap();
        assert!(recs[0].is_ai_89_magic());
    }

    #[test]
    fn ras_ldb_magic_and_leading_vec3_run() {
        // Synthetic: 1c 12 | ab cd | 00 | 16 + 3×f32 | 16 + 3×f32 | 09 …
        let mut buf = Vec::new();
        buf.extend_from_slice(&LDB_RAS_MAGIC.to_le_bytes());
        buf.extend_from_slice(&0xcdabu16.to_le_bytes());
        buf.push(0);
        for _ in 0..2 {
            buf.push(0x16);
            buf.extend_from_slice(&1.0f32.to_le_bytes());
            buf.extend_from_slice(&2.0f32.to_le_bytes());
            buf.extend_from_slice(&3.0f32.to_le_bytes());
        }
        buf.push(0x09);
        assert!(looks_like_ras_ldb(&buf));
        assert!(looks_like_ldb_or_ai(&buf));
        let h = parse_ras_ldb_header(&buf).unwrap();
        assert_eq!(h.magic, LDB_RAS_MAGIC);
        assert_eq!(h.count, 0xcdab);
        assert_eq!(h.flag, 0);
        assert_eq!(count_ras_ldb_leading_vec3(&buf), 2);
        // flag=0: declared count matches contiguous run when we build it that way
        let mut tight = Vec::new();
        tight.extend_from_slice(&LDB_RAS_MAGIC.to_le_bytes());
        tight.extend_from_slice(&2u16.to_le_bytes());
        tight.push(0);
        for _ in 0..2 {
            tight.push(0x16);
            tight.extend_from_slice(&0f32.to_le_bytes());
            tight.extend_from_slice(&0f32.to_le_bytes());
            tight.extend_from_slice(&0f32.to_le_bytes());
        }
        tight.push(0x1c); // next section often reuses magic high byte
        let ht = parse_ras_ldb_header(&tight).unwrap();
        assert_eq!(ht.count as usize, count_ras_ldb_leading_vec3(&tight));
        assert_eq!(ras_ldb_count_section_end(&ht), tight.len() - 1);
        assert!(looks_like_memory_file_ai(&[0x14, 0x7b, 0x14, 0x0a]));
        assert!(!looks_like_memory_file_ai(&[0x14, 0x20]));
        // kind 0x13 section: no flag byte; body starts with MF int tag
        let mut s2 = Vec::new();
        s2.push(LDB_SECTION_TAG);
        s2.push(LDB_SECTION_KIND_MF);
        s2.extend_from_slice(&3u16.to_le_bytes());
        s2.extend_from_slice(&[0x14, 0x00, 0x14, 0x03]);
        let sec = parse_ras_ldb_section_at(&s2, 0).unwrap();
        assert_eq!(sec.kind, LDB_SECTION_KIND_MF);
        assert_eq!(sec.count, 3);
        assert_eq!(sec.flag, None);
        assert_eq!(s2[4], 0x14);
    }

    #[test]
    fn parses_face_like_mf_record() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x14, 0, 0x14, 3, 0x14, 0]);
        body.push(0x13);
        body.extend_from_slice(&804u16.to_le_bytes());
        body.push(0x16);
        for v in [0.0f32, 0.0, 1.0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.push(0x16);
        for v in [-1.5f32, -1.5, -2.0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        let faces = parse_ras_ldb_faces(&body, 2);
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].indices, [0, 3, 0]);
        assert_eq!(faces[0].attr, 804);
        // index ≥ 256 uses 0x13 compressed int
        let mut body2 = Vec::new();
        body2.push(0x13);
        body2.extend_from_slice(&300u16.to_le_bytes());
        body2.extend_from_slice(&[0x14, 1, 0x14, 2]);
        body2.push(0x13);
        body2.extend_from_slice(&10u16.to_le_bytes());
        body2.push(0x16);
        for v in [0.0f32, 1.0, 0.0] {
            body2.extend_from_slice(&v.to_le_bytes());
        }
        body2.push(0x16);
        for v in [1.0f32, 2.0, 3.0] {
            body2.extend_from_slice(&v.to_le_bytes());
        }
        let f2 = parse_ras_ldb_faces(&body2, 1);
        assert_eq!(f2[0].indices, [300, 1, 2]);
        assert_eq!(f2[0].attr, 10);
        assert!((faces[0].normal[2] - 1.0).abs() < 1e-5);
        assert!((faces[0].point[0] + 1.5).abs() < 1e-5);
    }

    #[test]
    fn walks_vec3_then_face_sections() {
        let mut data = Vec::new();
        data.extend_from_slice(&LDB_RAS_MAGIC.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0);
        data.push(0x16);
        for v in [0.0f32, 0.0, 0.0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let face_start = data.len();
        data.push(LDB_SECTION_TAG);
        data.push(LDB_SECTION_KIND_MF);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&[0x14, 0, 0x14, 1, 0x14, 2]);
        data.push(0x13);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.push(0x16);
        for v in [0.0f32, 1.0, 0.0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        data.push(0x16);
        for v in [1.0f32, 2.0, 3.0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        // trailing unknown 0x13 head
        data.push(LDB_SECTION_TAG);
        data.push(LDB_SECTION_KIND_MF);
        data.extend_from_slice(&9u16.to_le_bytes());
        data.push(0x16); // not a face
        let spans = walk_ras_ldb_known_sections(&data);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].kind, LDB_SECTION_KIND_VEC3);
        assert_eq!(spans[1].kind, LDB_SECTION_KIND_MF);
        assert_eq!(spans[1].count, 1);
        assert_eq!(spans[1].offset, face_start);
        assert_eq!(spans[2].end, spans[2].offset); // stop marker
    }

    #[test]
    fn finds_import_level_after_geometry() {
        let mut data = Vec::new();
        data.extend_from_slice(&LDB_RAS_MAGIC.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.push(0);
        // empty face section count 0
        data.push(LDB_SECTION_TAG);
        data.push(LDB_SECTION_KIND_MF);
        data.extend_from_slice(&0u16.to_le_bytes());
        let at = data.len();
        data.extend_from_slice(&[0x14, LDB_IMPORT_VERSION as u8, 0x14, 1]);
        assert_eq!(ras_ldb_after_geometry(&data), Some(at));
    }

    #[test]
    fn parses_surf_records() {
        let mut body = Vec::new();
        body.push(0x16);
        for v in [1.0f32, 0.0, 0.0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.push(0x16);
        for v in [-1.5f32, 2.5, 6.0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        for v in [0u8, 6, 1, 6, 0, 6] {
            body.extend_from_slice(&[0x14, v]);
        }
        let (surfs, n) = parse_ras_ldb_surfs_ex(&body, 1);
        assert_eq!(surfs.len(), 1);
        assert_eq!(n, body.len());
        assert_eq!(surfs[0].attrs, [0, 6, 1, 6, 0, 6]);
    }

    #[test]
    fn parses_ai_header_four_ints() {
        let buf = [0x14u8, 123, 0x14, 10, 0x14, 0, 0x14, 0, 0x09, 0, 0, 0, 0];
        let h = parse_ras_ai_header(&buf).unwrap();
        assert_eq!(
            h,
            RasAiHeader {
                a: 123,
                b: 10,
                c: 0,
                d: 0
            }
        );
        assert!(parse_ras_ai_header(&[0x14, 0x20]).is_none());
    }

    #[test]
    fn split_ldb_records_on_gt_after_preamble() {
        let mut buf = Vec::from(LDB_PREAMBLE);
        buf.extend_from_slice(b"  \xc7  \x00  \x14\n  >");
        buf.extend_from_slice(b"abc>");
        buf.extend_from_slice(b"tail");
        let recs = split_ldb_records(&buf).unwrap();
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].lead_byte(), Some(0xc7));
        assert_eq!(recs[0].space_stripped(), vec![0xc7, 0x00, 0x14, 0x0a]);
        let c7 = recs[0].as_ai_c7().unwrap();
        assert_eq!(c7.sub, 0);
        assert_eq!(c7.tail, vec![0x14, 0x0a]);
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
