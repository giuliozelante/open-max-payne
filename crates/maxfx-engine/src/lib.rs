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
///
/// Prefer [`ras_ldb_vec3_section_end`] when `flag != 0` (declared count may be
/// smaller than the contiguous `0x16` run — Tutorial).
pub fn ras_ldb_count_section_end(header: &RasLdbHeader) -> usize {
    LDB_RAS_HEADER_LEN + header.count as usize * 13
}

/// End of the leading vec3 cloud using the contiguous `0x16` run (not declared count).
pub fn ras_ldb_vec3_section_end(data: &[u8]) -> Option<usize> {
    if !looks_like_ras_ldb(data) {
        return None;
    }
    Some(LDB_RAS_HEADER_LEN + count_ras_ldb_leading_vec3(data) * 13)
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
    let end = ras_ldb_vec3_section_end(data)?;
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

/// Counts printable paths + JPEG SOI markers in the ImportLevel stream.
pub fn ras_ldb_import_stats(data: &[u8]) -> Option<(usize, usize)> {
    let at = ras_ldb_after_geometry(data)?;
    let slice = &data[at..];
    let paths = ras_ldb_import_paths(data, usize::MAX).len();
    let jpegs = slice.windows(2).filter(|w| w == b"\xff\xd8").count();
    Some((paths, jpegs))
}

/// One texture entry after ImportLevel `0x20`: path + uint flag + raw blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasLdbImportTexture {
    pub path: String,
    pub flag: u8,
    pub blob_len: usize,
    pub jpeg: bool,
}

fn read_mf_uint_u8(data: &[u8], i: &mut usize) -> Option<u8> {
    if *i + 2 > data.len() || data[*i] != 0x11 {
        return None;
    }
    *i += 1;
    let v = data[*i];
    *i += 1;
    Some(v)
}

fn read_mf_str(data: &[u8], i: &mut usize) -> Option<String> {
    if *i + 3 > data.len() || data[*i] != 0x0d || data[*i + 1] != 0x14 {
        return None;
    }
    let len = data[*i + 2] as usize;
    *i += 3;
    if *i + len > data.len() {
        return None;
    }
    let bytes = &data[*i..*i + len];
    *i += len;
    if !bytes.iter().all(|&b| (0x20..=0x7e).contains(&b)) {
        return None;
    }
    String::from_utf8(bytes.to_vec()).ok()
}

/// Parse the texture list that follows ImportLevel version (`14 20` then count int).
///
/// Each entry: `0x0d` path, `0x11` flag, compressed-int length + raw bytes.
/// Returns `(count, textures, byte_offset_after_list)`.
pub fn parse_ras_ldb_import_textures(
    data: &[u8],
    max: usize,
) -> Option<(u32, Vec<RasLdbImportTexture>, usize)> {
    let at = ras_ldb_after_geometry(data)?;
    let mut i = at + 2; // skip 14 20
    let count = read_mf_i32(data, &mut i)? as u32;
    let mut out = Vec::new();
    let mut parsed = 0u32;
    while parsed < count {
        let path = read_mf_str(data, &mut i)?;
        let flag = read_mf_uint_u8(data, &mut i)?;
        let blob_len = read_mf_i32(data, &mut i)? as usize;
        if i + blob_len > data.len() {
            break;
        }
        let jpeg = blob_len >= 2 && data[i] == 0xff && data[i + 1] == 0xd8;
        i += blob_len;
        parsed += 1;
        if out.len() < max {
            out.push(RasLdbImportTexture {
                path,
                flag,
                blob_len,
                jpeg,
            });
        }
    }
    if parsed != count {
        return None;
    }
    Some((count, out, i))
}

/// Material object tag observed after the ImportLevel texture list (`X_LevelDBImportMaterial`).
pub const LDB_IMPORT_MATERIAL_TAG: u8 = 0x25;

/// Container/list marker before material count (BYOD Startup).
pub const LDB_IMPORT_MATERIAL_LIST_TAG: u8 = 0x1f;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasLdbImportMaterial {
    pub name: String,
    pub texture: String,
    pub index: i32,
}

/// Parse material list starting at `offset` (expects `0x1f` then count ints + `0x25` records).
pub fn parse_ras_ldb_import_materials_at(
    data: &[u8],
    offset: usize,
    max: usize,
) -> Option<(u32, Vec<RasLdbImportMaterial>, usize)> {
    if offset >= data.len() || data[offset] != LDB_IMPORT_MATERIAL_LIST_TAG {
        return None;
    }
    let mut i = offset + 1;
    let count = read_mf_i32(data, &mut i)? as u32;
    // First list (after textures) has an extra int before `0x25`; nested lists do not.
    if data.get(i) != Some(&LDB_IMPORT_MATERIAL_TAG) {
        let _unknown = read_mf_i32(data, &mut i)?;
    }
    let mut out = Vec::new();
    let mut parsed = 0u32;
    while parsed < count {
        if i >= data.len() || data[i] != LDB_IMPORT_MATERIAL_TAG {
            break;
        }
        i += 1;
        let name = read_mf_str(data, &mut i)?;
        let texture = read_mf_str(data, &mut i)?;
        // Startup’s last entry lacks the trailing int before a nested `0x1f` list.
        let index = match data.get(i).copied() {
            Some(0x14 | 0x13 | 0x12 | 0x02) => read_mf_i32(data, &mut i)?,
            _ => -1,
        };
        parsed += 1;
        if out.len() < max {
            out.push(RasLdbImportMaterial {
                name,
                texture,
                index,
            });
        }
    }
    if parsed == 0 {
        return None;
    }
    Some((count, out, i))
}

/// MemoryFile bool tag (`0x0e` + u8) seen on ImportMaterialCategory records.
pub const LDB_MF_BOOL_TAG: u8 = 0x0e;

/// One nested texture/member under an [`RasLdbImportMaterialCategory`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasLdbImportCategoryMember {
    pub name: String,
    pub path: String,
    /// Present when a second `0x0d` path precedes the bool pair (often alpha).
    pub path2: Option<String>,
    pub flag_a: u8,
    pub flag_b: u8,
}

/// `X_LevelDBImportMaterialCategory`: name, member_count (incl. primary), primary
/// texture+paths+bools, then `member_count - 1` child members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasLdbImportMaterialCategory {
    pub name: String,
    pub member_count: i32,
    pub texture: String,
    pub path: String,
    pub path2: String,
    pub flag_a: u8,
    pub flag_b: u8,
    pub members: Vec<RasLdbImportCategoryMember>,
}

fn read_mf_bool(data: &[u8], i: &mut usize) -> Option<u8> {
    if *i + 2 > data.len() || data[*i] != LDB_MF_BOOL_TAG {
        return None;
    }
    *i += 1;
    let v = data[*i];
    *i += 1;
    Some(v)
}

fn read_category_member(data: &[u8], i: &mut usize) -> Option<RasLdbImportCategoryMember> {
    let name = read_mf_str(data, i)?;
    // Children never carry the category index int; next must be a path string.
    if data.get(*i) == Some(&0x14) {
        return None;
    }
    let path = read_mf_str(data, i)?;
    let path2 = if data.get(*i) == Some(&0x0d) {
        Some(read_mf_str(data, i)?)
    } else {
        None
    };
    let flag_a = read_mf_bool(data, i)?;
    let flag_b = read_mf_bool(data, i)?;
    Some(RasLdbImportCategoryMember {
        name,
        path,
        path2,
        flag_a,
        flag_b,
    })
}

/// Parse ImportMaterialCategory list at `offset` (count int, then categories).
///
/// Each category: name, member_count, texture, path, path2, bool, bool, then
/// `member_count - 1` child members (`name`, path(s), bool, bool).
pub fn parse_ras_ldb_import_material_categories_at(
    data: &[u8],
    offset: usize,
) -> Option<(u32, Vec<RasLdbImportMaterialCategory>, usize)> {
    if offset >= data.len() {
        return None;
    }
    let mut i = offset;
    let count = read_mf_i32(data, &mut i)? as u32;
    if count == 0 {
        return Some((0, Vec::new(), i));
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name = read_mf_str(data, &mut i)?;
        let member_count = read_mf_i32(data, &mut i)?;
        if member_count < 1 {
            return None;
        }
        let texture = read_mf_str(data, &mut i)?;
        let path = read_mf_str(data, &mut i)?;
        let path2 = read_mf_str(data, &mut i)?;
        let flag_a = read_mf_bool(data, &mut i)?;
        let flag_b = read_mf_bool(data, &mut i)?;
        let child_n = (member_count - 1) as usize;
        let mut members = Vec::with_capacity(child_n);
        for _ in 0..child_n {
            members.push(read_category_member(data, &mut i)?);
        }
        out.push(RasLdbImportMaterialCategory {
            name,
            member_count,
            texture,
            path,
            path2,
            flag_a,
            flag_b,
            members,
        });
    }
    Some((count, out, i))
}

/// Walk consecutive `0x1f` material lists after textures; return offset after the last.
pub fn ras_ldb_after_import_materials(data: &[u8]) -> Option<usize> {
    let (_, _, mut off) = parse_ras_ldb_import_textures(data, 0)?;
    let mut passes = 0;
    while let Some((_, _, end)) = parse_ras_ldb_import_materials_at(data, off, 0) {
        if end <= off {
            break;
        }
        off = end;
        passes += 1;
        if passes >= 8 {
            break;
        }
    }
    if passes == 0 {
        None
    } else {
        Some(off)
    }
}

/// Separator between ImportLightmap blobs after the first (`00 00 00 01 00 01 18 20`).
pub const LDB_IMPORT_LIGHTMAP_SEP: [u8; 8] =
    [0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x18, 0x20];

/// Blob marker immediately before each lightmap payload (`0x18 0x20`).
pub const LDB_IMPORT_LIGHTMAP_MARK: [u8; 2] = [0x18, 0x20];

/// Retail BYOD blob size: `256*256*3 + 18` (RGB plane + 18 trailing/leading bytes).
pub const LDB_IMPORT_LIGHTMAP_RGB_SIZE: u32 = 256 * 256 * 3 + 18;

/// Skip `X_LevelDBImportLightmap` blob list after material categories.
///
/// Layout (BYOD Startup/Tutorial): count int, `0`, u8 `0`, then shared prefix
/// (`0x12`, u32 size, three u32s often `512,0,0`, bytes `01 00 01`, mark
/// `18 20`), then `count` payloads of `size` bytes with
/// [`LDB_IMPORT_LIGHTMAP_SEP`] before each blob after the first.
///
/// Returns `(count, offset_after_list)`.
pub fn parse_ras_ldb_import_lightmaps_at(data: &[u8], offset: usize) -> Option<(u32, usize)> {
    if offset >= data.len() {
        return None;
    }
    let mut i = offset;
    let count = read_mf_i32(data, &mut i)? as u32;
    let z = read_mf_i32(data, &mut i)?;
    if z != 0 {
        return None;
    }
    let u = read_mf_uint_u8(data, &mut i)?;
    if u != 0 {
        return None;
    }
    if i >= data.len() || data[i] != 0x12 {
        return None;
    }
    i += 1;
    if i + 4 > data.len() {
        return None;
    }
    let size = u32::from_le_bytes(data[i..i + 4].try_into().ok()?);
    i += 4;
    if size == 0 || i + 12 + 3 + 2 > data.len() {
        return None;
    }
    let _f1 = u32::from_le_bytes(data[i..i + 4].try_into().ok()?);
    let _f2 = u32::from_le_bytes(data[i + 4..i + 8].try_into().ok()?);
    let _f3 = u32::from_le_bytes(data[i + 8..i + 12].try_into().ok()?);
    i += 12;
    if data[i..i + 3] != [0x01, 0x00, 0x01] {
        return None;
    }
    i += 3;
    if data[i..i + 2] != LDB_IMPORT_LIGHTMAP_MARK {
        return None;
    }
    i += 2;
    // Declared `size` is RGB plane + 18-byte trailer between SEP-framed blobs.
    // The last blob has no trailer — next MF (exits/rooms) begins at RGB end.
    let rgb = if size == LDB_IMPORT_LIGHTMAP_RGB_SIZE {
        (size - 18) as usize
    } else {
        size as usize
    };
    let framed = size as usize;
    for n in 0..count {
        if n > 0 {
            if i + 8 > data.len() || data[i..i + 8] != LDB_IMPORT_LIGHTMAP_SEP {
                return None;
            }
            i += 8;
        }
        let need = if n + 1 == count { rgb } else { framed };
        if i + need > data.len() {
            return None;
        }
        i += need;
    }
    Some((count, i))
}

/// Polygon/surf-like record after a static mesh (normal + attrs).
#[derive(Debug, Clone, PartialEq)]
pub struct RasLdbImportMeshPoly {
    pub point_ref: i32,
    pub normal: [f32; 3],
    pub flag: u8,
    pub attr: i32,
    pub index: i32,
}

/// Parse mesh-poly list after a static mesh: `hdr0`, `hdr1`, then records
/// `point_ref, 3, vec3, u8, 0, attr, f32, f32, 0, index` until the pattern breaks.
pub fn parse_ras_ldb_import_mesh_polys_at(
    data: &[u8],
    offset: usize,
) -> Option<(i32, Vec<RasLdbImportMeshPoly>, usize)> {
    let mut i = offset;
    let hdr0 = read_mf_i32(data, &mut i)?;
    let _hdr1 = read_mf_i32(data, &mut i)?;
    let mut out = Vec::new();
    loop {
        let mark = i;
        let Some(poly) = read_mesh_poly(data, &mut i) else {
            i = mark;
            break;
        };
        out.push(poly);
        if out.len() > 100_000 {
            break;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some((hdr0, out, i))
    }
}

fn read_mesh_poly(data: &[u8], i: &mut usize) -> Option<RasLdbImportMeshPoly> {
    let point_ref = read_mf_i32(data, i)?;
    let three = read_mf_i32(data, i)?;
    if three != 3 {
        return None;
    }
    let normal = read_mf_vec3(data, i)?;
    let flag = read_mf_uint_u8(data, i)?;
    let _z0 = read_mf_i32(data, i)?;
    let attr = read_mf_i32(data, i)?;
    if data.get(*i) != Some(&0x09) || *i + 5 > data.len() {
        return None;
    }
    *i += 5; // f32
    if data.get(*i) != Some(&0x09) || *i + 5 > data.len() {
        return None;
    }
    *i += 5;
    let _z1 = read_mf_i32(data, i)?;
    // Final index int is present on most records; last-in-list may omit it.
    let index = match data.get(*i).copied() {
        Some(0x14 | 0x13 | 0x12 | 0x02) => read_mf_i32(data, i)?,
        _ => -1,
    };
    Some(RasLdbImportMeshPoly {
        point_ref,
        normal,
        flag,
        attr,
        index,
    })
}

/// One dual-vec3-cloud object after the point stream (likely ImportStaticMesh).
#[derive(Debug, Clone, PartialEq)]
pub struct RasLdbImportStaticMesh {
    pub a: i32,
    pub b: i32,
    pub point_count: u8,
    pub pos: [f32; 3],
}

/// Parse a dual point-cloud + `0x1a` mat3 + pos record at `offset`.
pub fn parse_ras_ldb_import_static_mesh_at(
    data: &[u8],
    offset: usize,
) -> Option<(RasLdbImportStaticMesh, usize)> {
    let mut i = offset;
    let a = read_mf_i32(data, &mut i)?;
    let b = read_mf_i32(data, &mut i)?;
    let n = read_mf_uint_u8(data, &mut i)?;
    for _ in 0..n {
        read_mf_vec3(data, &mut i)?;
    }
    if data.get(i) != Some(&0x1c) {
        return None;
    }
    i += 1;
    let n2 = read_mf_i32(data, &mut i)? as u8;
    if n2 != n {
        return None;
    }
    for _ in 0..n2 {
        read_mf_vec3(data, &mut i)?;
    }
    if data.get(i) != Some(&0x1a) || i + 1 + 36 + 12 > data.len() {
        return None;
    }
    i += 1 + 36;
    let x = f32::from_le_bytes(data[i..i + 4].try_into().ok()?);
    let y = f32::from_le_bytes(data[i + 4..i + 8].try_into().ok()?);
    let z = f32::from_le_bytes(data[i + 8..i + 12].try_into().ok()?);
    i += 12;
    Some((
        RasLdbImportStaticMesh {
            a,
            b,
            point_count: n,
            pos: [x, y, z],
        },
        i,
    ))
}

/// Skip-walk the post-exit `0x15` point stream (likely lights/triggers).
///
/// Layout after ImportExit: nest `0x1c` + ints, then `0x12` count (even;
/// body count = count/2), subtype int, first body without id, then
/// `(id, body) × (n-1)`. Each body: `15 f32 f32 15 f32 f32 03 i32 0e u8`.
pub fn parse_ras_ldb_import_point_stream_at(
    data: &[u8],
    offset: usize,
) -> Option<(u32, usize)> {
    if offset >= data.len() {
        return None;
    }
    let mut i = offset;
    let lim = offset.saturating_add(96).min(data.len());
    let mut count = None;
    while i < lim {
        match data[i] {
            0x1c => i += 1,
            0x12 => {
                count = Some(read_mf_i32(data, &mut i)? as u32);
                break;
            }
            0x14 | 0x13 | 0x02 => {
                read_mf_i32(data, &mut i)?;
            }
            _ => return None,
        }
    }
    let count = count?;
    if count < 2 || count % 2 != 0 {
        return None;
    }
    let _subtype = read_mf_i32(data, &mut i)?;
    let nexpect = (count / 2) as usize;
    let mut n = 0usize;
    if data.get(i) == Some(&0x15) {
        i = read_point_body(data, i)?;
        n = 1;
    }
    while n < nexpect {
        read_mf_i32(data, &mut i)?;
        i = read_point_body(data, i)?;
        n += 1;
    }
    // Trailing uncounted bodies (same shape, optional leading id).
    loop {
        let mark = i;
        if matches!(data.get(i), Some(0x14 | 0x13 | 0x12 | 0x02)) {
            if read_mf_i32(data, &mut i).is_none() {
                i = mark;
                break;
            }
        }
        match read_point_body(data, i) {
            Some(next) => {
                i = next;
                n += 1;
            }
            None => {
                i = mark;
                break;
            }
        }
        if n > 500_000 {
            break;
        }
    }
    Some((n as u32, i))
}

fn read_point_body(data: &[u8], mut i: usize) -> Option<usize> {
    if data.get(i) != Some(&0x15) {
        return None;
    }
    i += 1 + 8; // tag + two f32
    if data.get(i) != Some(&0x15) {
        return None;
    }
    i += 1 + 8;
    // Trailer field uses several MemoryFile int/uint encodings.
    match data.get(i).copied() {
        Some(0x03) => {
            // raw i32 (tag 0x03 is also u32-4 in RPS; here it's signed i32)
            if i + 5 > data.len() {
                return None;
            }
            i += 5;
        }
        Some(0x02) | Some(0x14) | Some(0x13) | Some(0x12) => {
            read_mf_i32(data, &mut i)?;
        }
        Some(0x0f) => {
            // u32-3
            if i + 4 > data.len() {
                return None;
            }
            i += 4;
        }
        Some(0x10) => {
            // u32-2
            if i + 3 > data.len() {
                return None;
            }
            i += 3;
        }
        Some(0x11) => {
            read_mf_uint_u8(data, &mut i)?;
        }
        _ => return None,
    }
    if data.get(i) != Some(&LDB_MF_BOOL_TAG) {
        return None;
    }
    i += 2;
    Some(i)
}

/// Harvest `::…::…` exit/room-ish names from the ImportLevel stream after `offset`.
pub fn ras_ldb_import_exit_names_at(data: &[u8], offset: usize, max: usize) -> Vec<String> {
    parse_ras_ldb_import_exits_at(data, offset, max)
        .map(|(exits, _)| exits.into_iter().map(|e| e.name).collect())
        .unwrap_or_default()
}

/// One ImportExit (optional linked room AABB) after lightmaps.
#[derive(Debug, Clone, PartialEq)]
pub struct RasLdbImportExit {
    pub id: i32,
    pub name: String,
    pub corners: u8,
    pub pos: Option<[f32; 3]>,
}

fn is_exit_name_at(data: &[u8], j: usize) -> bool {
    if j + 3 >= data.len() || data[j] != 0x0d || data[j + 1] != 0x14 {
        return false;
    }
    let len = data[j + 2] as usize;
    let start = j + 3;
    if start + len > data.len() {
        return false;
    }
    let bytes = &data[start..start + len];
    bytes.len() >= 4
        && bytes.starts_with(b"::")
        && bytes.iter().all(|&b| (0x20..=0x7e).contains(&b))
}

fn skip_exit_nest_markers(data: &[u8], j: &mut usize) {
    while data.get(*j) == Some(&0x1c) {
        *j += 1;
    }
}

fn peek_exit_at(data: &[u8], i: usize) -> bool {
    let mut j = i;
    skip_exit_nest_markers(data, &mut j);
    // One or more leading ints (nest markers allowed between), then `::` name.
    let mut saw_int = false;
    for _ in 0..8 {
        skip_exit_nest_markers(data, &mut j);
        if is_exit_name_at(data, j) {
            return saw_int;
        }
        if read_mf_i32(data, &mut j).is_none() {
            return false;
        }
        saw_int = true;
    }
    skip_exit_nest_markers(data, &mut j);
    saw_int && is_exit_name_at(data, j)
}

/// Parse ImportExit records after lightmaps: `id` int, `::` name, optional room
/// (`u8` corners + vec3s + axis + `0x1a` mat3 + pos), then skip nested `0x1c`/ints
/// until the next exit or `max` reached.
pub fn parse_ras_ldb_import_exits_at(
    data: &[u8],
    offset: usize,
    max: usize,
) -> Option<(Vec<RasLdbImportExit>, usize)> {
    if offset >= data.len() || max == 0 {
        return None;
    }
    let mut i = offset;
    let mut out = Vec::new();
    while out.len() < max && peek_exit_at(data, i) {
        let record_start = i;
        skip_exit_nest_markers(data, &mut i);
        let mut id = 0i32;
        let mut got_id = false;
        for _ in 0..8 {
            skip_exit_nest_markers(data, &mut i);
            if is_exit_name_at(data, i) {
                break;
            }
            let Some(v) = read_mf_i32(data, &mut i) else {
                break;
            };
            if !got_id {
                id = v;
                got_id = true;
            }
        }
        if !got_id || !is_exit_name_at(data, i) {
            i = record_start;
            break;
        }
        let Some(name) = read_mf_str(data, &mut i) else {
            i = record_start;
            break;
        };
        let mut corners = 0u8;
        let mut pos = None;
        if data.get(i) == Some(&0x11) {
            let mark = i;
            if let Some(n) = read_mf_uint_u8(data, &mut i) {
                if (3..=32).contains(&n) {
                    let mut ok = true;
                    for _ in 0..n {
                        if read_mf_vec3(data, &mut i).is_none() {
                            ok = false;
                            break;
                        }
                    }
                    if ok
                        && read_mf_vec3(data, &mut i).is_some()
                        && data.get(i) == Some(&0x1a)
                        && i + 1 + 36 + 12 <= data.len()
                    {
                        i += 1 + 36; // mat3
                        let x = f32::from_le_bytes(data[i..i + 4].try_into().unwrap());
                        let y = f32::from_le_bytes(data[i + 4..i + 8].try_into().unwrap());
                        let z = f32::from_le_bytes(data[i + 8..i + 12].try_into().unwrap());
                        i += 12;
                        corners = n;
                        pos = Some([x, y, z]);
                    } else {
                        i = mark;
                    }
                } else {
                    i = mark;
                }
            }
        }
        out.push(RasLdbImportExit {
            id,
            name,
            corners,
            pos,
        });
        if out.len() >= max {
            break;
        }
        if peek_exit_at(data, i) {
            continue;
        }
        // Hunt next exit by `::` name; stop on post-exit `0x15` float stream.
        let before = i;
        let mut found = None;
        let lim = i.saturating_add(8192).min(data.len());
        let mut j = i;
        while j < lim {
            // Post-exit stream: `15 <f32> <f32> 15 <f32> <f32> 03 <i32> 0e <u8>`
            if j + 20 <= data.len()
                && data[j] == 0x15
                && data[j + 5] == 0x15
                && data[j + 10] == 0x03
                && data[j + 15] == 0x0e
            {
                break;
            }
            if is_exit_name_at(data, j) {
                let back = before.max(j.saturating_sub(32));
                for s in (back..=j).rev() {
                    if peek_exit_at(data, s) {
                        found = Some(s);
                        break;
                    }
                }
                break;
            }
            j += 1;
        }
        match found {
            Some(s) => i = s,
            None => {
                i = before;
                break;
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some((out, i))
    }
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
                // Advance by contiguous 0x16 run (Tutorial flag=1: run > declared count).
                let run = {
                    let mut i = off + LDB_RAS_HEADER_LEN;
                    let mut n = 0usize;
                    while i + 13 <= data.len() && data[i] == 0x16 {
                        n += 1;
                        i += 13;
                    }
                    n
                };
                let end = off + LDB_RAS_HEADER_LEN + run * 13;
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
    fn parses_import_texture_list() {
        let mut data = Vec::new();
        data.extend_from_slice(&LDB_RAS_MAGIC.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.push(0);
        data.push(LDB_SECTION_TAG);
        data.push(LDB_SECTION_KIND_MF);
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&[0x14, LDB_IMPORT_VERSION as u8]);
        data.extend_from_slice(&[0x14, 1]); // count
        data.extend_from_slice(&[0x0d, 0x14, 5]);
        data.extend_from_slice(b"a.JPG");
        data.extend_from_slice(&[0x11, 4]);
        data.extend_from_slice(&[0x14, 4]); // blob len
        data.extend_from_slice(&[0xff, 0xd8, 0xff, 0xd9]);
        let (n, texs, end) = parse_ras_ldb_import_textures(&data, 8).unwrap();
        assert_eq!(n, 1);
        assert_eq!(texs[0].path, "a.JPG");
        assert_eq!(texs[0].flag, 4);
        assert!(texs[0].jpeg);
        assert_eq!(texs[0].blob_len, 4);
        assert_eq!(end, data.len());
    }

    #[test]
    fn parses_import_materials() {
        let mut body = Vec::new();
        body.push(LDB_IMPORT_MATERIAL_LIST_TAG);
        body.extend_from_slice(&[0x14, 1, 0x14, 0]); // count=1 unknown=0
        body.push(LDB_IMPORT_MATERIAL_TAG);
        body.extend_from_slice(&[0x0d, 0x14, 3]);
        body.extend_from_slice(b"mat");
        body.extend_from_slice(&[0x0d, 0x14, 5]);
        body.extend_from_slice(b"t.JPG");
        body.extend_from_slice(&[0x14, 7]);
        let (n, mats, _) = parse_ras_ldb_import_materials_at(&body, 0, 4).unwrap();
        assert_eq!(n, 1);
        assert_eq!(mats[0].name, "mat");
        assert_eq!(mats[0].texture, "t.JPG");
        assert_eq!(mats[0].index, 7);
    }

    #[test]
    fn harvests_exit_names_after_offset() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x14, 1]);
        body.extend_from_slice(&[0x0d, 0x14, 14]);
        body.extend_from_slice(b"::start::Exit0");
        body.extend_from_slice(&[0x14, 2]);
        body.extend_from_slice(&[0x0d, 0x14, 4]);
        body.extend_from_slice(b"nope"); // not :: — stops list
        let names = ras_ldb_import_exit_names_at(&body, 0, 8);
        assert_eq!(names, vec!["::start::Exit0"]);
        let mut body2 = Vec::new();
        body2.extend_from_slice(&[0x14, 1]);
        body2.extend_from_slice(&[0x0d, 0x14, 14]);
        body2.extend_from_slice(b"::start::Exit0");
        body2.extend_from_slice(&[0x14, 2]);
        body2.extend_from_slice(&[0x0d, 0x14, 13]);
        body2.extend_from_slice(b"::help::Exit1");
        assert_eq!(
            ras_ldb_import_exit_names_at(&body2, 0, 8),
            vec!["::start::Exit0", "::help::Exit1"]
        );
    }

    #[test]
    fn parses_import_mesh_polys_after_static_mesh() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x14, 2, 0x14, 0]); // hdr
        for idx in [0u8, 1u8] {
            body.extend_from_slice(&[0x14, 10 + idx]); // pref
            body.extend_from_slice(&[0x14, 3]);
            body.push(0x16);
            for v in [0.0f32, 0.0, 1.0] {
                body.extend_from_slice(&v.to_le_bytes());
            }
            body.extend_from_slice(&[0x11, 1, 0x14, 0, 0x14, 44]);
            body.push(0x09);
            body.extend_from_slice(&0.0f32.to_le_bytes());
            body.push(0x09);
            body.extend_from_slice(&0.0f32.to_le_bytes());
            body.extend_from_slice(&[0x14, 0, 0x14, idx]);
        }
        body.push(0x1f); // next section
        let (hdr0, polys, end) = parse_ras_ldb_import_mesh_polys_at(&body, 0).unwrap();
        assert_eq!(hdr0, 2);
        assert_eq!(polys.len(), 2);
        assert_eq!(polys[1].index, 1);
        assert_eq!(body[end], 0x1f);
    }

    #[test]
    fn parses_import_static_mesh_clouds() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x14, 4, 0x14, 10, 0x11, 2]); // a,b,n=2
        for _ in 0..2 {
            body.push(0x16);
            for v in [1.0f32, 2.0, 3.0] {
                body.extend_from_slice(&v.to_le_bytes());
            }
        }
        body.extend_from_slice(&[0x1c, 0x14, 2]); // nest count=2
        for _ in 0..2 {
            body.push(0x16);
            for v in [4.0f32, 5.0, 6.0] {
                body.extend_from_slice(&v.to_le_bytes());
            }
        }
        body.push(0x1a);
        for v in [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        for v in [7.0f32, 8.0, 9.0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        let (mesh, end) = parse_ras_ldb_import_static_mesh_at(&body, 0).unwrap();
        assert_eq!(mesh.a, 4);
        assert_eq!(mesh.b, 10);
        assert_eq!(mesh.point_count, 2);
        assert_eq!(mesh.pos, [7.0, 8.0, 9.0]);
        assert_eq!(end, body.len());
    }

    #[test]
    fn skips_post_exit_point_stream() {
        // nest + count=4 (→ 2 bodies) + sub + body + (id, body)
        let mut body = Vec::new();
        body.extend_from_slice(&[0x1c, 0x14, 1, 0x1c]);
        body.extend_from_slice(&[0x12, 4, 0, 0]); // count = 4
        body.extend_from_slice(&[0x14, 3]); // subtype
        // body without id
        body.push(0x15);
        body.extend_from_slice(&1.0f32.to_le_bytes());
        body.extend_from_slice(&2.0f32.to_le_bytes());
        body.push(0x15);
        body.extend_from_slice(&3.0f32.to_le_bytes());
        body.extend_from_slice(&4.0f32.to_le_bytes());
        body.extend_from_slice(&[0x03]);
        body.extend_from_slice(&(-1i32).to_le_bytes());
        body.extend_from_slice(&[0x0e, 0]);
        // id + body
        body.extend_from_slice(&[0x14, 1]);
        body.push(0x15);
        body.extend_from_slice(&5.0f32.to_le_bytes());
        body.extend_from_slice(&6.0f32.to_le_bytes());
        body.push(0x15);
        body.extend_from_slice(&7.0f32.to_le_bytes());
        body.extend_from_slice(&8.0f32.to_le_bytes());
        body.extend_from_slice(&[0x03]);
        body.extend_from_slice(&(-1i32).to_le_bytes());
        body.extend_from_slice(&[0x0e, 0]);
        body.push(0xff);
        let (n, end) = parse_ras_ldb_import_point_stream_at(&body, 0).unwrap();
        assert_eq!(n, 2);
        assert_eq!(end, body.len() - 1);
        assert_eq!(body[end], 0xff);
    }

    #[test]
    fn parses_import_exit_with_room_aabb() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x14, 4]); // flag/id
        let name1 = b"::help::Exit1";
        body.extend_from_slice(&[0x0d, 0x14, name1.len() as u8]);
        body.extend_from_slice(name1);
        body.extend_from_slice(&[0x11, 4]); // 4 corners
        for _ in 0..5 {
            // 4 corners + axis
            body.push(0x16);
            for v in [1.0f32, 2.0, 3.0] {
                body.extend_from_slice(&v.to_le_bytes());
            }
        }
        body.push(0x1a);
        for v in [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        for v in [10.0f32, 20.0, 30.0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        // second exit without room
        body.extend_from_slice(&[0x14, 2]);
        let name2 = b"::start::Exit0";
        body.extend_from_slice(&[0x0d, 0x14, name2.len() as u8]);
        body.extend_from_slice(name2);
        body.extend_from_slice(&[0x1c, 0x14, 1]); // nesting marker
        let (exits, end) = parse_ras_ldb_import_exits_at(&body, 0, 8).unwrap();
        assert_eq!(exits.len(), 2);
        assert_eq!(exits[0].name, "::help::Exit1");
        assert_eq!(exits[0].pos, Some([10.0, 20.0, 30.0]));
        assert_eq!(exits[0].corners, 4);
        assert_eq!(exits[1].name, "::start::Exit0");
        assert!(exits[1].pos.is_none());
        assert!(end > 0);
    }

    #[test]
    fn skips_import_lightmap_blobs() {
        // count=2, shared hdr, two RGB-sized blobs with separator
        let mut body = Vec::new();
        body.extend_from_slice(&[0x14, 2, 0x14, 0, 0x11, 0]);
        body.push(0x12); // mystery
        let size = 32u32; // tiny stand-in (retail uses 196626)
        body.extend_from_slice(&size.to_le_bytes());
        body.extend_from_slice(&512u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&[0x01, 0x00, 0x01]);
        body.extend_from_slice(&[0x18, 0x20]);
        body.extend_from_slice(&vec![0xab; size as usize]);
        body.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x18, 0x20]);
        body.extend_from_slice(&vec![0xcd; size as usize]);
        body.push(0xff); // sentinel after
        let (n, end) = parse_ras_ldb_import_lightmaps_at(&body, 0).unwrap();
        assert_eq!(n, 2);
        assert_eq!(end, body.len() - 1);
        assert_eq!(body[end], 0xff);
    }

    #[test]
    fn last_lightmap_omits_18_byte_trailer() {
        let rgb = (256 * 256 * 3) as usize;
        let size = LDB_IMPORT_LIGHTMAP_RGB_SIZE;
        let mut body = Vec::new();
        body.extend_from_slice(&[0x14, 1, 0x14, 0, 0x11, 0, 0x12]);
        body.extend_from_slice(&size.to_le_bytes());
        body.extend_from_slice(&512u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&[0x01, 0x00, 0x01, 0x18, 0x20]);
        body.extend_from_slice(&vec![0x80; rgb]);
        body.extend_from_slice(&[0x14, 0x04]); // next MF after RGB
        let (n, end) = parse_ras_ldb_import_lightmaps_at(&body, 0).unwrap();
        assert_eq!(n, 1);
        assert_eq!(end, body.len() - 2);
        assert_eq!(&body[end..], &[0x14, 0x04]);
    }

    #[test]
    fn parses_import_material_categories() {
        // count=1 category "wall" member_count=2 → primary + one child
        let mut body = Vec::new();
        body.extend_from_slice(&[0x14, 1]); // category count
        body.extend_from_slice(&[0x0d, 0x14, 4]);
        body.extend_from_slice(b"wall");
        body.extend_from_slice(&[0x14, 2]); // members incl. primary
        body.extend_from_slice(&[0x0d, 0x14, 5]);
        body.extend_from_slice(b"a.JPG");
        body.extend_from_slice(&[0x0d, 0x14, 3]);
        body.extend_from_slice(b"p/a");
        body.extend_from_slice(&[0x0d, 0x14, 0]); // empty path2
        body.extend_from_slice(&[0x0e, 0, 0x0e, 1]); // bools
        // child member
        body.extend_from_slice(&[0x0d, 0x14, 5]);
        body.extend_from_slice(b"b.JPG");
        body.extend_from_slice(&[0x0d, 0x14, 3]);
        body.extend_from_slice(b"p/b");
        body.extend_from_slice(&[0x0e, 1, 0x0e, 0]);
        let (n, cats, end) = parse_ras_ldb_import_material_categories_at(&body, 0).unwrap();
        assert_eq!(n, 1);
        assert_eq!(end, body.len());
        assert_eq!(cats[0].name, "wall");
        assert_eq!(cats[0].member_count, 2);
        assert_eq!(cats[0].texture, "a.JPG");
        assert_eq!(cats[0].path, "p/a");
        assert_eq!(cats[0].path2, "");
        assert_eq!(cats[0].flag_a, 0);
        assert_eq!(cats[0].flag_b, 1);
        assert_eq!(cats[0].members.len(), 1);
        assert_eq!(cats[0].members[0].name, "b.JPG");
        assert_eq!(cats[0].members[0].path, "p/b");
        assert_eq!(cats[0].members[0].path2, None);
        assert_eq!(cats[0].members[0].flag_a, 1);
        assert_eq!(cats[0].members[0].flag_b, 0);
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
