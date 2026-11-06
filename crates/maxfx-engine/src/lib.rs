//! Native Max Payne 1 engine — level decoder, 3D walkthrough renderer, and KF2/SKD inspector.

pub mod clips;
pub mod decals;
pub mod frontend;
pub mod gpu;
pub mod health;
pub mod hud;
pub mod items;
pub mod kf2;
pub mod menu_ui;
pub mod particles;
pub mod player;
pub mod projectiles;
pub mod render3d;
pub mod scene;

use openras::RasArchive;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
        // Only the first section is the vertex cloud (all 25 retail levels). Later `1c 12`
        // heads are MF arrays with a u24 count whose body may start with 0x16.
        LDB_SECTION_KIND_VEC3 if offset == 0 => {
            if data.len() < offset + 5 {
                return None;
            }
            Some(RasLdbSection {
                kind,
                count,
                flag: Some(data[offset + 4]),
            })
        }
        LDB_SECTION_KIND_MF | LDB_SECTION_KIND_VEC3 | 0x14 => {
            // MemoryFile compressed uint count after 0x1c (`14 <u8>`, `13 <u16>`, `12 <u24>`)
            let mut i = offset + 1;
            let count32 = match data.get(i).copied()? {
                0x14 => {
                    i += 1;
                    *data.get(i)? as u32
                }
                0x13 => {
                    i += 1;
                    u16::from_le_bytes([*data.get(i)?, *data.get(i + 1)?]) as u32
                }
                0x12 => {
                    i += 1;
                    u32::from_le_bytes([*data.get(i)?, *data.get(i + 1)?, *data.get(i + 2)?, 0])
                }
                _ => return None,
            };
            Some(RasLdbSection {
                kind: LDB_SECTION_KIND_MF,
                count: count32.min(u16::MAX as u32) as u16,
                flag: None,
            })
        }
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

/// Sample up to `max` leading geometry vec3s for a native preview frame (#16).
pub fn sample_ras_ldb_vec3_preview(data: &[u8], max: usize) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    if !looks_like_ras_ldb(data) || max == 0 {
        return out;
    }
    let mut i = LDB_RAS_HEADER_LEN;
    while out.len() < max && i + 13 <= data.len() && data[i] == 0x16 {
        i += 1;
        let x = f32::from_le_bytes(data[i..i + 4].try_into().unwrap_or([0; 4]));
        let y = f32::from_le_bytes(data[i + 4..i + 8].try_into().unwrap_or([0; 4]));
        let z = f32::from_le_bytes(data[i + 8..i + 12].try_into().unwrap_or([0; 4]));
        i += 12;
        if x.is_finite() && y.is_finite() && z.is_finite() {
            out.push([x, y, z]);
        }
    }
    out
}

/// Orthographic point-cloud preview window (BYOD smoke for epic #16). Esc / close to exit.
pub fn show_vec3_preview_window(points: &[[f32; 3]], title: &str) -> Result<(), String> {
    use minifb::{Key, Window, WindowOptions};
    const W: usize = 960;
    const H: usize = 640;
    if points.is_empty() {
        return Err("no preview points".into());
    }
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for p in points {
        min_x = min_x.min(p[0]);
        max_x = max_x.max(p[0]);
        min_z = min_z.min(p[2]);
        max_z = max_z.max(p[2]);
    }
    let dx = (max_x - min_x).max(1.0);
    let dz = (max_z - min_z).max(1.0);
    let mut window = Window::new(
        title,
        W,
        H,
        WindowOptions {
            resize: false,
            ..WindowOptions::default()
        },
    )
    .map_err(|e| e.to_string())?;
    window.set_target_fps(30);
    let mut buf = vec![0u32; W * H];
    while window.is_open() && !window.is_key_down(Key::Escape) {
        buf.fill(0x00101820);
        for p in points {
            let u = ((p[0] - min_x) / dx * (W as f32 - 1.0)) as usize;
            let v = ((p[2] - min_z) / dz * (H as f32 - 1.0)) as usize;
            let v = H - 1 - v.min(H - 1);
            let u = u.min(W - 1);
            buf[v * W + u] = 0x00e8f0ff;
        }
        window
            .update_with_buffer(&buf, W, H)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
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

/// Body slice of the first kind-`0x13` section after a vec3 cloud.
pub fn ras_ldb_first_mf_section_body(data: &[u8]) -> Option<&[u8]> {
    let end = ras_ldb_vec3_section_end(data)?;
    let (_, body_off) = offset_plus_mf_count(data, end)?;
    Some(&data[body_off..])
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
    if *i >= data.len() || data[*i] != 0x0d {
        return None;
    }
    *i += 1;
    let len = match data.get(*i).copied() {
        Some(0x14) => {
            *i += 1;
            if *i >= data.len() {
                return None;
            }
            let n = data[*i] as usize;
            *i += 1;
            n
        }
        Some(0x13) => {
            *i += 1;
            if *i + 2 > data.len() {
                return None;
            }
            let n = u16::from_le_bytes(data[*i..*i + 2].try_into().ok()?) as usize;
            *i += 2;
            n
        }
        Some(0x12) => {
            *i += 1;
            if *i + 3 > data.len() {
                return None;
            }
            let n = u32::from_le_bytes([data[*i], data[*i + 1], data[*i + 2], 0]) as usize;
            *i += 3;
            n
        }
        _ => return None,
    };
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
pub const LDB_IMPORT_LIGHTMAP_SEP: [u8; 8] = [0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x18, 0x20];

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
    /// Vertex count (`3` tris, also 4–12 on large meshes).
    pub verts: i32,
    pub normal: [f32; 3],
    pub flag: u8,
    pub attr: i32,
    pub index: i32,
}

/// Parse mesh-poly list after a static mesh: `hdr0`, `hdr1`, then records
/// `point_ref, verts, vec3, u8, 0, attr, f32, f32, 0, index` until the pattern breaks.
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
    let verts = read_mf_i32(data, i)?;
    if !(3..=16).contains(&verts) {
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
        verts,
        normal,
        flag,
        attr,
        index,
    })
}

/// `(id, vec3)` entry in a post-mesh `0x1f` attrib list (not `0x25` materials).
#[derive(Debug, Clone, PartialEq)]
pub struct RasLdbImportMeshAttrib {
    pub id: i32,
    pub value: [f32; 3],
}

/// Parse `0x1f` + count + `(id, vec3) × count` between meshes (BYOD Tutorial).
///
/// Rejects material lists (next tag `0x25`) and empty delimiters (`count == 0`).
pub fn parse_ras_ldb_import_mesh_attribs_at(
    data: &[u8],
    offset: usize,
) -> Option<(u32, Vec<RasLdbImportMeshAttrib>, usize)> {
    if offset >= data.len() || data[offset] != LDB_IMPORT_MATERIAL_LIST_TAG {
        return None;
    }
    let mut i = offset + 1;
    let count = read_mf_i32(data, &mut i)? as u32;
    if count == 0 || count > 100_000 {
        return None;
    }
    if data.get(i) == Some(&LDB_IMPORT_MATERIAL_TAG) {
        return None;
    }
    let mut out = Vec::new();
    for _ in 0..count {
        let id = read_mf_i32(data, &mut i)?;
        let value = read_mf_vec3(data, &mut i)?;
        out.push(RasLdbImportMeshAttrib { id, value });
    }
    Some((count, out, i))
}

/// One ImportLight-like record after the static-mesh chain (BYOD Tutorial).
#[derive(Debug, Clone, PartialEq)]
pub struct RasLdbImportLight {
    pub name: String,
    pub pos: [f32; 3],
    pub subtype: i32,
    pub param_count: u32,
}

/// Parse `count` + light records: dual name, dual `0x1a` xform, subtype int,
/// empty str, raw `0x19` mat3, then ten `0x09` floats (BYOD Tutorial).
pub fn parse_ras_ldb_import_lights_at(
    data: &[u8],
    offset: usize,
    max: usize,
) -> Option<(u32, Vec<RasLdbImportLight>, usize)> {
    let mut i = offset;
    let count = read_mf_i32(data, &mut i)? as u32;
    if count == 0 || count > 100_000 {
        return None;
    }
    // First record must look like Light:: / dual string + 0x1a.
    if data.get(i) != Some(&0x0d) {
        return None;
    }
    let mut out = Vec::new();
    for n in 0..count {
        let name = read_mf_str(data, &mut i)?;
        let _name2 = read_mf_str(data, &mut i)?;
        if data.get(i) != Some(&0x1a) || i + 1 + 36 + 12 > data.len() {
            return None;
        }
        i += 1 + 36;
        let pos = [
            f32::from_le_bytes(data[i..i + 4].try_into().ok()?),
            f32::from_le_bytes(data[i + 4..i + 8].try_into().ok()?),
            f32::from_le_bytes(data[i + 8..i + 12].try_into().ok()?),
        ];
        i += 12;
        if data.get(i) != Some(&0x1a) || i + 1 + 48 > data.len() {
            return None;
        }
        i += 1 + 48; // second xform
        let subtype = read_mf_i32(data, &mut i)?;
        let empty = read_mf_str(data, &mut i)?;
        if !empty.is_empty() {
            return None;
        }
        if data.get(i) != Some(&0x19) || i + 1 + 36 > data.len() {
            return None;
        }
        i += 1 + 36;
        let mut params = 0u32;
        while data.get(i) == Some(&0x09) && i + 5 <= data.len() {
            i += 5;
            params += 1;
        }
        if params == 0 {
            return None;
        }
        if out.len() < max {
            out.push(RasLdbImportLight {
                name,
                pos,
                subtype,
                param_count: params,
            });
        }
        let _ = n;
    }
    Some((count, out, i))
}

/// Head of the post-mesh character/jumppoint block (BYOD Startup).
///
/// Layout: empty `0x1f`, int `0`, count, then named entities (`0d 14` paths,
/// dual `0x1a` transforms, nested DO_* graphs). Returns `(count, offset_at_first_name)`.
pub fn parse_ras_ldb_import_character_head_at(data: &[u8], offset: usize) -> Option<(u32, usize)> {
    let mut i = offset;
    if data.get(i) != Some(&LDB_IMPORT_MATERIAL_LIST_TAG) {
        return None;
    }
    i += 1;
    let empty = read_mf_i32(data, &mut i)?;
    if empty != 0 {
        return None;
    }
    let _zero = read_mf_i32(data, &mut i)?;
    let count = read_mf_i32(data, &mut i)? as u32;
    if count == 0 || count > 10_000 {
        return None;
    }
    if data.get(i) != Some(&0x0d) {
        return None;
    }
    Some((count, i))
}

/// Parse a character `DO_*` hook list at `offset` (BYOD Tutorial.ldb @`0x1f7458f`).
///
/// Layout: `0x1f` + count, then ×N: `0x0d` `DO_*` string, empty nest (`0x1c`+0),
/// empty list (`0x1f`+0), empty nest (`0x1c`+0). Returns `(hooks, offset_after)`.
pub fn parse_ras_ldb_import_do_hooks_at(
    data: &[u8],
    offset: usize,
) -> Option<(Vec<String>, usize)> {
    let mut i = offset;
    if data.get(i) != Some(&LDB_IMPORT_MATERIAL_LIST_TAG) {
        return None;
    }
    i += 1;
    let count = read_mf_i32(data, &mut i)? as usize;
    if count == 0 || count > 64 {
        return None;
    }
    let mut hooks = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_mf_str(data, &mut i)?;
        if !name.starts_with("DO_") {
            return None;
        }
        if data.get(i) != Some(&0x1c) {
            return None;
        }
        i += 1;
        if read_mf_i32(data, &mut i)? != 0 {
            return None;
        }
        if data.get(i) != Some(&LDB_IMPORT_MATERIAL_LIST_TAG) {
            return None;
        }
        i += 1;
        if read_mf_i32(data, &mut i)? != 0 {
            return None;
        }
        if data.get(i) != Some(&0x1c) {
            return None;
        }
        i += 1;
        if read_mf_i32(data, &mut i)? != 0 {
            return None;
        }
        hooks.push(name);
    }
    Some((hooks, i))
}

/// Scan `[offset, offset+window)` for the first parseable `DO_*` hook list.
pub fn find_ras_ldb_import_do_hooks_at(
    data: &[u8],
    offset: usize,
    window: usize,
) -> Option<(usize, Vec<String>, usize)> {
    let end = data.len().min(offset.saturating_add(window));
    let mut i = offset;
    while i < end {
        if data[i] == LDB_IMPORT_MATERIAL_LIST_TAG {
            if let Some((hooks, after)) = parse_ras_ldb_import_do_hooks_at(data, i) {
                return Some((i, hooks, after));
            }
        }
        i += 1;
    }
    None
}

/// Harvest `.DO->` / `->` FSM script action strings from a character-block window.
pub fn ras_ldb_import_do_scripts_at(data: &[u8], offset: usize, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = offset;
    let end = data.len().min(offset.saturating_add(512 * 1024));
    while i + 3 < end && out.len() < max {
        if data[i] == 0x0d && data[i + 1] == 0x14 {
            let len = data[i + 2] as usize;
            let start = i + 3;
            if start + len <= data.len() {
                let bytes = &data[start..start + len];
                if bytes.windows(2).any(|w| w == b"->")
                    && bytes.iter().all(|&b| (0x20..=0x7e).contains(&b))
                {
                    if let Ok(s) = String::from_utf8(bytes.to_vec()) {
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

/// Harvest `::…Jumppoint…` names from a character-block window.
pub fn ras_ldb_import_jumppoint_names_at(data: &[u8], offset: usize, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = offset;
    let end = data.len().min(offset.saturating_add(512 * 1024));
    while i + 3 < end && out.len() < max {
        if data[i] == 0x0d && data[i + 1] == 0x14 {
            let len = data[i + 2] as usize;
            let start = i + 3;
            if start + len <= data.len() {
                let bytes = &data[start..start + len];
                if bytes.windows(9).any(|w| w == b"Jumppoint")
                    && bytes.iter().all(|&b| (0x20..=0x7e).contains(&b))
                {
                    if let Ok(s) = String::from_utf8(bytes.to_vec()) {
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

/// One dual-vec3-cloud object after the point stream (likely ImportStaticMesh).
#[derive(Debug, Clone, PartialEq)]
pub struct RasLdbImportStaticMesh {
    pub a: i32,
    pub b: i32,
    pub point_count: u32,
    pub pos: [f32; 3],
}

fn read_point_count(data: &[u8], i: &mut usize) -> Option<u32> {
    match data.get(*i).copied() {
        Some(0x11) => Some(read_mf_uint_u8(data, i)? as u32),
        Some(0x10) => {
            if *i + 3 > data.len() {
                return None;
            }
            let v = u16::from_le_bytes(data[*i + 1..*i + 3].try_into().ok()?) as u32;
            *i += 3;
            Some(v)
        }
        Some(0x14 | 0x13 | 0x12 | 0x02) => Some(read_mf_i32(data, i)? as u32),
        _ => None,
    }
}

/// Parse a dual point-cloud + `0x1a` mat3 + pos record at `offset`.
///
/// Form A (first mesh): `a`, `b`, u8/`u16` `n`, clouds…
/// Form B (later meshes): optional empty `0x1f` list, then `b`, `n` (often `0x10` u16).
pub fn parse_ras_ldb_import_static_mesh_at(
    data: &[u8],
    offset: usize,
) -> Option<(RasLdbImportStaticMesh, usize)> {
    let mut i = offset;
    // Optional empty material-list delimiter between meshes.
    if data.get(i) == Some(&LDB_IMPORT_MATERIAL_LIST_TAG) {
        i += 1;
        let c = read_mf_i32(data, &mut i)?;
        if c != 0 {
            return None;
        }
    }
    // Form A: a, b, n — Form B: b, n (a defaults 0)
    let first = read_mf_i32(data, &mut i)?;
    let (a, b, n) = if data.get(i) == Some(&0x11) || data.get(i) == Some(&0x10) {
        (0, first, read_point_count(data, &mut i)?)
    } else {
        (
            first,
            read_mf_i32(data, &mut i)?,
            read_point_count(data, &mut i)?,
        )
    };
    if n == 0 || n > 500_000 {
        return None;
    }
    for _ in 0..n {
        read_mf_vec3(data, &mut i)?;
    }
    if data.get(i) != Some(&0x1c) {
        return None;
    }
    i += 1;
    let n2 = read_mf_i32(data, &mut i)? as u32;
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
pub fn parse_ras_ldb_import_point_stream_at(data: &[u8], offset: usize) -> Option<(u32, usize)> {
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
        if matches!(data.get(i), Some(0x14 | 0x13 | 0x12 | 0x02))
            && read_mf_i32(data, &mut i).is_none()
        {
            i = mark;
            break;
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
            // Post-exit stream: `15 <f32> <f32> 15 <f32> <f32> 03|0f|10|11 ... 0e <u8>`
            if j + 19 <= data.len()
                && data[j] == 0x15
                && data[j + 9] == 0x15
                && matches!(data[j + 18], 0x03 | 0x0f | 0x10 | 0x11)
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

fn offset_plus_mf_count(data: &[u8], off: usize) -> Option<(usize, usize)> {
    if data.get(off) != Some(&LDB_SECTION_TAG) {
        return None;
    }
    let mut i = off + 1;
    let count = read_mf_i32(data, &mut i)? as usize;
    Some((count, i))
}

pub fn walk_ras_ldb_known_sections(data: &[u8]) -> Vec<RasLdbSectionSpan> {
    let mut out = Vec::new();
    let mut off = 0;
    while let Some(sec) = parse_ras_ldb_section_at(data, off) {
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
                let Some((n, body_start)) = offset_plus_mf_count(data, off) else {
                    break;
                };
                let body = &data[body_start..];
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
                let end = body_start + consumed;
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

/// BYOD Tutorial.ai preamble after the four-int header: 4×f32, vec3, 3×f32,
/// then ints + first `0x1a` xform. Returns `(end_offset, gravity_like_vec3)`.
pub fn parse_ras_ai_preamble(data: &[u8]) -> Option<(usize, [f32; 3])> {
    let mut i = 0;
    let _ = parse_ras_ai_header(data)?;
    // skip four ints already known; re-parse to advance i
    for _ in 0..4 {
        read_mf_i32(data, &mut i)?;
    }
    let mut f4 = [0f32; 4];
    for slot in &mut f4 {
        if data.get(i) != Some(&0x09) || i + 5 > data.len() {
            return None;
        }
        *slot = f32::from_le_bytes(data[i + 1..i + 5].try_into().ok()?);
        i += 5;
    }
    let vec = read_mf_vec3(data, &mut i)?;
    for _ in 0..3 {
        if data.get(i) != Some(&0x09) || i + 5 > data.len() {
            return None;
        }
        i += 5;
    }
    let _a = read_mf_i32(data, &mut i)?;
    let _b = read_mf_i32(data, &mut i)?;
    if data.get(i) != Some(&0x1a) || i + 1 + 48 > data.len() {
        return None;
    }
    i += 1 + 48;
    let _ = f4;
    Some((i, vec))
}

/// Soft-count known MemoryFile tags from `offset` until an unknown byte.
///
/// Tag sizes match ImportLevel / Tutorial.ai BYOD (incl. raw `0x19` mat3,
/// `0x15` dual-f32, nest `0x1c` / list `0x1f`, long `0d 13` strings).
/// Tutorial.ai walks to EOF.
pub fn soft_walk_mf_tags(data: &[u8], offset: usize, max: usize) -> (usize, usize) {
    let mut i = offset;
    let mut n = 0usize;
    while n < max && i < data.len() {
        let next = match data[i] {
            0x14 | 0x11 | 0x0e => i + 2,
            0x13 | 0x10 => i + 3,
            0x12 => i + 4,
            0x02 => i + 5,
            0x09 => i + 5,
            0x16 => i + 13,
            0x15 => i + 1 + 8,
            0x19 => i + 1 + 36,
            0x1a => i + 1 + 48,
            0x1c | 0x1f => i + 1,
            0x0d if i + 1 < data.len() => match data[i + 1] {
                0x14 if i + 2 < data.len() => i + 3 + data[i + 2] as usize,
                0x13 if i + 4 <= data.len() => {
                    let len = u16::from_le_bytes(data[i + 2..i + 4].try_into().unwrap()) as usize;
                    i + 4 + len
                }
                0x12 if i + 5 <= data.len() => {
                    let len =
                        u32::from_le_bytes([data[i + 2], data[i + 3], data[i + 4], 0]) as usize;
                    i + 5 + len
                }
                _ => break,
            },
            _ => break,
        };
        if next <= i || next > data.len() {
            break;
        }
        i = next;
        n += 1;
    }
    (n, i)
}

/// Harvest `::…` entity path strings (no `->` scripts) from a byte window.
pub fn ras_ldb_import_entity_names_at(data: &[u8], offset: usize, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = offset;
    let end = data.len().min(offset.saturating_add(1024 * 1024));
    while i + 3 < end && out.len() < max {
        if data[i] != 0x0d {
            i += 1;
            continue;
        }
        let mut j = i;
        match read_mf_str(data, &mut j) {
            Some(s)
                if s.starts_with("::")
                    && !s.contains("->")
                    && !s.starts_with("DO_")
                    && s.chars().all(|c| c.is_ascii_graphic() || c == ' ') =>
            {
                out.push(s);
                i = j;
            }
            _ => i += 1,
        }
    }
    out
}

/// Harvest `::…Tele…` path strings from a byte window (post-light entities).
pub fn ras_ldb_import_tele_names_at(data: &[u8], offset: usize, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = offset;
    let end = data.len().min(offset.saturating_add(256 * 1024));
    while i + 3 < end && out.len() < max {
        if data[i] == 0x0d && data[i + 1] == 0x14 {
            let len = data[i + 2] as usize;
            let start = i + 3;
            if start + len <= data.len() {
                let bytes = &data[start..start + len];
                if bytes.starts_with(b"::")
                    && bytes.windows(4).any(|w| w == b"Tele")
                    && bytes.iter().all(|&b| (0x20..=0x7e).contains(&b))
                {
                    if let Ok(s) = String::from_utf8(bytes.to_vec()) {
                        if !s.contains("->") {
                            out.push(s);
                        }
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
#[derive(Debug, Clone, PartialEq)]
pub struct LevelsTxtEntry {
    /// Block name, e.g. `part0_level1`.
    pub id: String,
    /// `Directory` property (e.g. `part1`, `front_end`).
    pub directory: String,
    /// `Level` property basename (e.g. `part0_level1.ldb`).
    pub level_file: String,
    /// `PlayerStartingPlace`, e.g. `::StartRoom::Jumppoint 00`.
    pub player_start: Option<String>,
    /// `PlayerSkinName`, e.g. `young_max_payne` in the house, `max_payne` later.
    pub player_skin: Option<String>,
    /// `WorldSphere`, e.g. `sunset` for the house.
    pub world_sphere: Option<String>,
    /// `AmbientColor` (0..255 RGB, `#define`s resolved). Retail copies it into the `P_Scene`
    /// ambient (`0x6805be`), which becomes `D3DRS_AMBIENT` for lit objects.
    pub ambient: Option<[f32; 3]>,
    /// `FogColor` / `FogStart` / `FogEnd`, present only when `Fogging` is on.
    pub fog: Option<crate::scene::LevelFog>,
    /// `Gravity` (m/s², `#define`s resolved): retail is `( 0, GRAVITY_VALUE , 0 )`, -20.
    pub gravity: Option<[f32; 3]>,
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
    let defines: Vec<(&str, &str)> = text
        .lines()
        .filter_map(|line| {
            let mut words = line.trim().strip_prefix("#define")?.split_whitespace();
            Some((words.next()?, words.next()?))
        })
        .collect();
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
            player_start: prop_value(body, "PlayerStartingPlace"),
            player_skin: prop_value(body, "PlayerSkinName"),
            world_sphere: prop_value(body, "WorldSphere"),
            ambient: prop_value(body, "AmbientColor").and_then(|v| parse_color3(&v, &defines)),
            fog: level_fog(body, &defines),
            gravity: prop_value(body, "Gravity").and_then(|v| parse_color3(&v, &defines)),
        });
    }
    out
}

/// `Fogging = TRUE` block fog. Values may carry a trailing `// comment` after the `;`.
fn level_fog(body: &str, defines: &[(&str, &str)]) -> Option<crate::scene::LevelFog> {
    let value =
        |key| prop_value(body, key).map(|v| v.split(';').next().unwrap_or("").trim().to_string());
    if !value("Fogging")?.eq_ignore_ascii_case("true") {
        return None;
    }
    Some(crate::scene::LevelFog {
        color: parse_color3(&value("FogColor")?, defines)?,
        start: value("FogStart")?.parse().ok()?,
        end: value("FogEnd")?.parse().ok()?,
    })
}

/// `( r, g, b )` with each channel a number or a `#define` name.
fn parse_color3(value: &str, defines: &[(&str, &str)]) -> Option<[f32; 3]> {
    let inner = value.trim().strip_prefix('(')?.strip_suffix(')')?;
    let mut rgb = [0.0f32; 3];
    let mut parts = inner.split(',');
    for c in &mut rgb {
        let word = parts.next()?.trim();
        let word = defines
            .iter()
            .find(|(name, _)| *name == word)
            .map_or(word, |(_, v)| v);
        *c = word.parse().ok()?;
    }
    parts.next().is_none().then_some(rgb)
}

/// `worldspheres.txt` block `[sunset]` → `bg_sunset.kf2`.
pub fn world_sphere_export(catalog: &str, name: &str) -> Option<String> {
    let lower = catalog.to_ascii_lowercase();
    let key = format!("[{}]", name.to_ascii_lowercase());
    let at = lower.find(&key)? + key.len();
    let body = &catalog[at..];
    let end = body.find('}').unwrap_or(body.len());
    prop_value(&body[..end], "ExportData")
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

/// Retail boot after the options dialog: intro from `globalconfig.txt`, then `startup_level`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupPlan {
    pub video: Option<PathBuf>,
    pub level_win_path: String,
    pub level_bytes: Vec<u8>,
}

/// `StartupVideoName` inside the `[startup]` block. Other sections are ignored.
pub fn startup_video_name(text: &str) -> Option<String> {
    let mut in_startup = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            let name = t[1..t.len() - 1].trim();
            in_startup = name.eq_ignore_ascii_case("startup");
            continue;
        }
        if !in_startup {
            continue;
        }
        let Some((k, v)) = t.split_once('=') else {
            continue;
        };
        if !k.trim().eq_ignore_ascii_case("StartupVideoName") {
            continue;
        }
        let v = v
            .trim()
            .trim_end_matches(';')
            .trim()
            .trim_matches('"')
            .trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

/// Loose `data/`, then `x_data.ras`, then other `*.ras`.
pub fn resolve_startup(game_dir: &Path) -> Result<StartupPlan, EngineError> {
    let mut src = GameSource::open(game_dir);
    let config = src.read(r"\data\globalconfig.txt").unwrap_or_default();
    let config_text = String::from_utf8_lossy(&config);
    let video = startup_video_name(&config_text).and_then(|rel| {
        let path = game_dir.join(rel.replace('\\', "/"));
        path.is_file().then_some(path)
    });
    let levels = src
        .read(r"\data\database\levels\levels.txt")
        .ok_or_else(|| {
            EngineError::Msg("levels.txt not found under data/ or a *.ras archive".into())
        })?;
    let entries = parse_levels_txt(&String::from_utf8_lossy(&levels));
    let entry = entries
        .iter()
        .find(|e| e.id.eq_ignore_ascii_case("startup_level"))
        .ok_or_else(|| EngineError::Msg("startup_level missing from levels.txt".into()))?;
    let level_win_path = entry.ldb_archive_path();
    let level_bytes = src
        .read(&level_win_path)
        .ok_or_else(|| EngineError::Msg(format!("startup level not found: {level_win_path}")))?;
    Ok(StartupPlan {
        video,
        level_win_path,
        level_bytes,
    })
}

/// Top-level or nested row from `data\menu\menu.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    pub id: String,
    pub label: String,
    pub level_id: Option<String>,
    pub children: Vec<MenuEntry>,
    /// `S_Quit()` — leave without loading a level.
    pub quits: bool,
}

pub fn menu_label(id: &str) -> String {
    match id {
        "GCID_RESUMEGAME" => "Resume".into(),
        "GCID_STARTTUTORIAL" => "Start Tutorial".into(),
        "GCID_NEWGAME" => "New Game".into(),
        "GCID_NORMAL" => "Normal".into(),
        "GCID_NIGHTMARE" => "Nightmare".into(),
        "GCID_HELL" => "Hell".into(),
        "GCID_TIMEDMODE" => "Timed".into(),
        "GCID_LOAD" => "Load".into(),
        "GCID_SAVE" => "Save".into(),
        "GCID_OPTIONS" => "Options".into(),
        "GCID_CREDITS" => "Credits".into(),
        "GCID_QUIT" => "Quit".into(),
        "GCID_QUIT_YES" => "Yes".into(),
        "GCID_QUIT_NO" => "No".into(),
        "GCID_BACK" => "Back".into(),
        other => other.trim_start_matches("GCID_").to_string(),
    }
}

/// First `[Menu]` in `menu.txt`. `gm_init(id)` becomes `level_id`. `GCID_BACK` is omitted.
pub fn parse_main_menu(text: &str) -> Vec<MenuEntry> {
    let mut stack: Vec<MenuFrame> = Vec::new();
    let mut pending: Option<FrameKind> = None;
    let mut started = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("//") {
            continue;
        }
        if t == "{" {
            if !started {
                if matches!(pending, Some(FrameKind::Menu)) {
                    started = true;
                    stack.push(MenuFrame::new(FrameKind::Menu));
                }
                pending = None;
                continue;
            }
            let kind = pending.take().unwrap_or(FrameKind::Skip);
            stack.push(MenuFrame::new(kind));
            continue;
        }
        if t == "}" {
            let Some(frame) = stack.pop() else {
                continue;
            };
            if stack.is_empty() {
                return frame.children;
            }
            if frame.id.as_deref() != Some("GCID_BACK") && !matches!(frame.kind, FrameKind::Skip) {
                if let Some(id) = frame.id.clone() {
                    stack.last_mut().unwrap().children.push(MenuEntry {
                        label: menu_label(&id),
                        level_id: frame.level_id,
                        children: frame.children,
                        quits: frame.quits,
                        id,
                    });
                }
            }
            continue;
        }
        if t.starts_with('[') {
            let tag = t.trim_start_matches('[').split(']').next().unwrap_or("");
            pending = Some(match tag {
                "Menu" => FrameKind::Menu,
                "Item" => FrameKind::Item,
                _ => FrameKind::Skip,
            });
        }
        if let Some(id) = gcid_assign(t) {
            if let Some(frame) = stack.last_mut() {
                if frame.id.is_none() {
                    frame.id = Some(id);
                }
            }
        }
        if let Some(id) = gm_init_id(t) {
            if let Some(frame) = stack.last_mut() {
                frame.level_id = Some(id);
            }
        }
        if t.to_ascii_lowercase().contains("s_quit(") {
            if let Some(frame) = stack.last_mut() {
                frame.quits = true;
            }
        }
    }
    stack
        .first()
        .map(|f| f.children.clone())
        .unwrap_or_default()
}

#[derive(Clone, Copy)]
enum FrameKind {
    Menu,
    Item,
    Skip,
}

struct MenuFrame {
    kind: FrameKind,
    id: Option<String>,
    level_id: Option<String>,
    quits: bool,
    children: Vec<MenuEntry>,
}

impl MenuFrame {
    fn new(kind: FrameKind) -> Self {
        Self {
            kind,
            id: None,
            level_id: None,
            quits: false,
            children: Vec::new(),
        }
    }
}

fn gcid_assign(line: &str) -> Option<String> {
    let (k, v) = line.split_once('=')?;
    if !k.trim().eq_ignore_ascii_case("name") {
        return None;
    }
    let v = v.trim().trim_end_matches(';').trim().trim_matches('"');
    v.starts_with("GCID_").then(|| v.to_string())
}

fn gm_init_id(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let idx = lower.find("gm_init(")?;
    let rest = line[idx + "gm_init(".len()..].trim_start();
    let end = rest.find(')')?;
    let id = rest[..end].trim().trim_matches('"');
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// End of a level script: the comic page it picks up, then the next `gm_init`.
///
/// `Startup_level.ldb` stores these three calls in order:
/// `mpgnm_pickupnote(P1L0a_001)`, `s_modeswitch(graphicnovel)`, `gm_init(part0_level1)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelHandoff {
    pub novel_page: String,
    pub next_level: String,
}

pub fn level_handoff(data: &[u8]) -> Option<LevelHandoff> {
    // The three calls are one script. A later `gm_init` in the same file
    // (the house does this after an in-level comic) is not this handoff.
    const WINDOW: usize = 192;
    let marker = b"mpgnm_pickupnote(";
    let mut from = 0;
    while let Some(rel) = find_bytes(&data[from..], marker) {
        let at = from + rel;
        let end = (at + WINDOW).min(data.len());
        let window = &data[at..end];
        if let (Some(novel_page), Some(switch_at)) = (
            call_arg(window, marker),
            find_bytes(window, b"s_modeswitch(graphicnovel)"),
        ) {
            if let Some(next_level) = call_arg(&window[switch_at..], b"gm_init(") {
                return Some(LevelHandoff {
                    novel_page,
                    next_level,
                });
            }
        }
        from = at + marker.len();
    }
    None
}

/// Comic pages the level shows on entry, then stays in this level.
///
/// The house start FSM picks up `P1L0a_002`, marks every note read, then picks
/// up `003`–`005` and switches to the graphic novel. Pages before that mark
/// stay unread-cleared. A `gm_init` in the same window is a level handoff.
pub fn level_opening_pages(data: &[u8]) -> Vec<String> {
    let mut from = 0;
    while let Some(rel) = find_bytes_ci(&data[from..], b"startfsm") {
        let at = from + rel;
        let end = (at + 1536).min(data.len());
        let window = &data[at..end];
        if let Some(switch_at) = find_bytes_ci(window, b"s_modeswitch(graphicnovel)") {
            let tail = &window[switch_at..(switch_at + 80).min(window.len())];
            if find_bytes_ci(tail, b"gm_init(").is_none() {
                let body = &window[..switch_at];
                let body = rfind_bytes_ci(body, b"markallnotesread")
                    .map(|mark| &body[mark..])
                    .unwrap_or(body);
                return call_args_ci(body, b"mpgnm_pickupnote(");
            }
        }
        from = at + b"startfsm".len();
    }
    Vec::new()
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn find_bytes_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len())
        .position(|w| w.iter().zip(needle).all(|(a, b)| a.eq_ignore_ascii_case(b)))
}

fn rfind_bytes_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len())
        .rposition(|w| w.iter().zip(needle).all(|(a, b)| a.eq_ignore_ascii_case(b)))
}

fn call_args_ci(data: &[u8], marker: &[u8]) -> Vec<String> {
    let mut from = 0;
    let mut out = Vec::new();
    while let Some(rel) = find_bytes_ci(&data[from..], marker) {
        let at = from + rel;
        let rest = &data[at + marker.len()..];
        let Some(end) = rest.iter().position(|b| *b == b')') else {
            break;
        };
        if let Ok(raw) = std::str::from_utf8(&rest[..end]) {
            let raw = raw.trim().trim_matches('"').trim();
            if !raw.is_empty() {
                out.push(raw.to_string());
            }
        }
        from = at + marker.len();
    }
    out
}

fn call_arg(data: &[u8], marker: &[u8]) -> Option<String> {
    let at = find_bytes(data, marker)?;
    let rest = &data[at + marker.len()..];
    let end = rest.iter().position(|b| *b == b')')?;
    let raw = std::str::from_utf8(&rest[..end]).ok()?;
    let raw = raw.trim().trim_matches('"').trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

/// `P1L0a_001` → `\data\database\graphicnovelpages\part1\textures\P1L0a_001.jpg`.
pub fn novel_page_win_path(page: &str) -> String {
    let part = page
        .as_bytes()
        .get(1)
        .copied()
        .filter(|c| c.is_ascii_digit())
        .map(|c| c - b'0')
        .unwrap_or(1);
    format!("\\data\\database\\graphicnovelpages\\part{part}\\textures\\{page}.jpg")
}

pub fn load_novel_jpeg(game_dir: &Path, page: &str) -> Option<Vec<u8>> {
    GameSource::open(game_dir).read(&novel_page_win_path(page))
}

/// Chrome from `data/graphicnovel/graphicnovel.txt` `[Properties]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicNovelChrome {
    pub default_page: String,
    pub background: String,
    pub background_width: u32,
    pub background_height: u32,
}

/// One row of `graphicnovelpages.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NovelPageDef {
    pub id: String,
    /// Second argument of `A_PlaySound`, for example `P1L0a_001` or `empty`.
    pub sound: String,
    pub new_chapter: bool,
}

/// `bitmaps\gn_background.jpg` under `data/graphicnovel`.
pub fn novel_background_win_path(background: &str) -> String {
    format!(
        "\\data\\graphicnovel\\{}",
        background.replace('/', "\\").trim_start_matches('\\')
    )
}

/// `story\part1\P1L0a_001.wav` under `data/database/sounds`.
pub fn story_wav_win_path(filename: &str) -> String {
    format!(
        "\\data\\database\\sounds\\{}",
        filename.replace('/', "\\").trim_start_matches('\\')
    )
}

pub fn parse_graphic_novel_chrome(text: &str) -> GraphicNovelChrome {
    let mut chrome = GraphicNovelChrome {
        default_page: String::new(),
        background: String::new(),
        background_width: 0,
        background_height: 0,
    };
    for line in text.lines() {
        let Some((key, value)) = assign_pair(line) else {
            continue;
        };
        match key.to_ascii_lowercase().as_str() {
            "defaultpagename" => chrome.default_page = value,
            "background" => chrome.background = value,
            "backgroundwidth" => chrome.background_width = value.parse().unwrap_or(0),
            "backgroundheight" => chrome.background_height = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    chrome
}

pub fn parse_graphic_novel_pages(text: &str) -> Vec<NovelPageDef> {
    let mut pages = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('[') || !line.contains("ExportData") {
            continue;
        }
        let Some(end) = line.find(']') else {
            continue;
        };
        let id = line[1..end].trim();
        if id.is_empty() {
            continue;
        }
        let sound = play_sound_cue(line).unwrap_or_default();
        let new_chapter = line.to_ascii_lowercase().contains("newchapter = yes")
            || line.to_ascii_lowercase().contains("newchapter=yes");
        pages.push(NovelPageDef {
            id: id.to_string(),
            sound,
            new_chapter,
        });
    }
    pages
}

/// One archive or loose file. Same search as the catalogs: loose `data/`, then `x_data.ras`.
pub fn read_game_bytes(game_dir: &Path, win_path: &str) -> Option<Vec<u8>> {
    GameSource::open(game_dir).read(win_path)
}

/// Every weapon in `database/weapons`, by lower-case file name, as the archives list them.
pub fn load_weapon_defs(game_dir: &Path) -> BTreeMap<String, hud::WeaponDef> {
    const DIR: &str = r"\data\database\weapons\";
    let mut source = GameSource::open(game_dir);
    let Some(shooting) = source.read(r"\data\database\shooting.h") else {
        return BTreeMap::new();
    };
    let mut shooting = String::from_utf8_lossy(&shooting).into_owned();
    // Weapons share ammo by number: both shotguns are inventory 5 under different names.
    let ids = source
        .read(r"\data\database\weaponid.h")
        .unwrap_or_default();
    for line in String::from_utf8_lossy(&ids).lines() {
        if line.contains("INVENTORYID_") || line.contains("SLOTID_") {
            shooting.push('\n');
            shooting.push_str(line);
        }
    }
    let mut names = Vec::new();
    for path in ras_candidates(game_dir) {
        let Ok(index) = openras::archive::RasIndex::open(&path) else {
            continue;
        };
        names.extend(index.paths().filter_map(|member| {
            let lower = member.to_ascii_lowercase();
            let name = lower.strip_prefix(DIR)?.strip_suffix(".txt")?;
            (!name.contains('\\')).then(|| name.to_string())
        }));
    }
    let skeleton = source
        .read(r"\data\database\skeletons\default_skeleton.txt")
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let reloads = hud::parse_reload_animations(&skeleton);
    let reload_sounds = hud::parse_reload_sound_frames(&skeleton);
    let shots = hud::parse_shoot_animations(&skeleton);
    let table = clips::ClipTable::parse(&skeleton, "");
    let gun = kf2::parse_body_bones(&skeleton, "RightHandGunBone");
    names
        .into_iter()
        .filter_map(|name| {
            let text = source.read(&format!("{DIR}{name}.txt"))?;
            let text = String::from_utf8_lossy(&text).into_owned();
            let mut def = hud::WeaponDef::parse(&shooting, &text)?;
            let mut projectiles = |index: &str| -> Vec<hud::ProjectileDef> {
                hud::parse_shot_projectiles(&text, index)
                    .into_iter()
                    .filter_map(|(projectile, count)| {
                        let path = format!(r"\data\database\projectiles\{projectile}.txt");
                        let file = source.read(&path)?;
                        let file = String::from_utf8_lossy(&file);
                        let mut shot = hud::ProjectileDef::parse(&shooting, &file, count)?;
                        shot.character_hit = load_splatter(&mut source, &file);
                        Some(shot)
                    })
                    .collect()
            };
            def.projectiles = projectiles("WEAPONANIM_SHOOT");
            def.quiet_projectiles = projectiles("WEAPONANIM_SHOOT_NOEFFECTS");
            def.flying = hud::parse_shot_projectiles(&text, "WEAPONANIM_SHOOT")
                .into_iter()
                .filter_map(|(name, count)| Some((load_flying(&mut source, &name)?, count)))
                .filter(|(flying, _)| flying.damages_character)
                .collect();
            if !def.flying.is_empty() {
                def.projectiles.clear();
            }
            if let Some((file, frame)) = reloads.get(&name) {
                let path = format!(r"\data\database\skeletons\default_skeleton\{file}");
                let clip = source
                    .read(&path)
                    .and_then(|bytes| kf2::parse_kf2_anim(&bytes));
                let fps = clip.as_ref().and_then(|clip| Some(clip.bones.first()?.fps));
                let fps = fps.filter(|fps| *fps > 0.0);
                def.reload_seconds = fps.map_or(0.0, |fps| *frame as f32 / fps);
                def.reload_sound_delay = fps
                    .zip(reload_sounds.get(&name))
                    .map_or(0.0, |(fps, frame)| *frame as f32 / fps);
                def.reload_clip = clip;
            }
            // A bare file lives in the weapon's own folder.
            if let Some(model) = def.model.as_mut().filter(|model| !model.contains('\\')) {
                *model = format!(r"{name}\{model}");
            }
            // Ingram, dual and melee weapons script the shot on the body clip.
            let body = format!("CHARANIM_SHOOT{}", name.to_ascii_uppercase());
            def.fire_sound = def
                .fire_sound
                .or_else(|| hud::weapon_cue(&skeleton, &body, "A_Play3DSound", 1));
            def.muzzle = def
                .muzzle
                .or_else(|| hud::weapon_cue(&skeleton, &body, "PS_StartEffect", 0));
            if let Some(file) = shots.get(&name) {
                let path = format!(r"\data\database\skeletons\default_skeleton\{file}");
                def.fire_clip = source
                    .read(&path)
                    .and_then(|bytes| kf2::parse_kf2_anim(&bytes));
            }
            let own_clips = clips::ClipTable::parse(&text, "");
            def.gun_clip = own_clips
                .get("WEAPONANIM_SHOOT")
                .map(|clip| resolve_win_path(&format!(r"{DIR}{name}\{}", clip.file)))
                .and_then(|path| source.read(&path))
                .and_then(|bytes| kf2::parse_kf2_anim(&bytes))
                .filter(|clip| clip.bones.iter().any(|bone| bone.keys.len() > 1))
                .map(std::sync::Arc::new);
            let mut aim_clip = |way: &str| {
                let index = format!("CHARANIM_SHOOT{way}{}", name.to_ascii_uppercase());
                let file = &table.get(&index)?.file;
                let path = format!(r"\data\database\skeletons\default_skeleton\{file}");
                kf2::parse_kf2_anim(&source.read(&path)?)
            };
            def.aim_clips = aim_clip("LOW")
                .zip(aim_clip("HIGH"))
                .zip(gun.first())
                .and_then(|((low, high), gun)| clips::AimClips::new(low, high, gun))
                .map(std::sync::Arc::new);
            let fps = def
                .fire_clip
                .as_ref()
                .and_then(|clip| Some(clip.bones.first()?.fps));
            let fps = fps.filter(|fps| *fps > 0.0).unwrap_or(1.0);
            def.rounds = hud::parse_rounds(&skeleton, &body);
            for round in &mut def.rounds {
                round.at /= fps;
            }
            let blows = hud::parse_bone_projectiles(&skeleton, &body);
            if let Some((_, projectile)) = blows.first() {
                let path = format!(r"\data\database\projectiles\{projectile}.txt");
                let file = source.read(&path);
                let file = file.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
                def.melee_reach = file.as_deref().and_then(hud::parse_melee_reach);
                // The clip's blows replace the weapon file's own `P_CreateProjectile`.
                def.projectiles = file
                    .as_deref()
                    .and_then(|file| {
                        let blow = hud::ProjectileDef::parse(&shooting, file, 1)?;
                        let damage = hud::parse_melee_damage(file).unwrap_or(blow.damage);
                        Some(hud::ProjectileDef {
                            damage,
                            character_hit: load_splatter(&mut source, file),
                            ..blow
                        })
                    })
                    .into_iter()
                    .collect();
                let fps = def
                    .fire_clip
                    .as_ref()
                    .and_then(|clip| Some(clip.bones.first()?.fps));
                let fps = fps.filter(|fps| *fps > 0.0).unwrap_or(1.0);
                def.melee_blows = blows.iter().map(|(frame, _)| *frame as f32 / fps).collect();
            }
            Some((name, def))
        })
        .collect()
}

/// `database/projectiles/<name>.txt` if it falls, with what its explosion throws.
fn load_flying(source: &mut GameSource, name: &str) -> Option<projectiles::FlyingDef> {
    const DIR: &str = r"\data\database\projectiles\";
    let defines = source
        .read(r"\data\database\projectiles.h")
        .unwrap_or_default();
    let defines = String::from_utf8_lossy(&defines).into_owned();
    let text = source.read(&format!("{DIR}{name}.txt"))?;
    let mut def = projectiles::FlyingDef::parse(&defines, &String::from_utf8_lossy(&text))?;
    let clip = def.explode_clip.as_ref().and_then(|file| {
        let path = resolve_win_path(&format!(r"{DIR}{name}\{file}"));
        kf2::parse_kf2_anim(&source.read(&path)?)
    });
    let fps = clip.and_then(|clip| Some(clip.bones.first()?.fps));
    let fps = fps.filter(|fps| *fps > 0.0).unwrap_or(30.0);
    def.model = def
        .model
        .map(|file| resolve_win_path(&format!(r"{DIR}{name}\{file}")));
    for blast in &mut def.blasts {
        blast.at = blast.frame as f32 / fps;
    }
    def.frags = std::mem::take(&mut def.frag_names)
        .into_iter()
        .filter_map(|(frag, count)| Some((load_flying(source, &frag)?, count)))
        .collect();
    Some(def)
}

/// The health multipliers of `\\data\\difficulty.txt`.
pub fn load_difficulty(game_dir: &Path) -> Option<health::Difficulty> {
    let text = read_game_bytes(game_dir, r"\data\difficulty.txt")?;
    health::Difficulty::parse(&String::from_utf8_lossy(&text))
}

/// Max at full health, from `max_payne.txt` and the difficulty.
pub fn load_player_health(game_dir: &Path) -> Option<health::PlayerHealth> {
    let difficulty = load_difficulty(game_dir)?;
    let skin = read_game_bytes(game_dir, r"\data\database\skins\max_payne.txt")?;
    let ranges = read_game_bytes(game_dir, r"\data\database\ai_ranges.h").unwrap_or_default();
    let max = kf2::skin_max_health(
        &String::from_utf8_lossy(&skin),
        &String::from_utf8_lossy(&ranges),
    )?;
    Some(health::PlayerHealth::new(max as f32, &difficulty))
}

/// `weaponpriority.txt` `[BestWeapons]`, strongest first.
pub fn load_best_weapons(game_dir: &Path) -> Vec<String> {
    read_game_bytes(game_dir, r"\data\weaponpriority.txt")
        .map(|bytes| hud::parse_best_weapons(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default()
}

/// Raw SharedDB text for `breakable` and `dynamic`, from loose `data/` or `x_data.ras`.
pub fn load_sound_catalogs(game_dir: &Path) -> BTreeMap<String, String> {
    ["breakable", "dynamic"]
        .into_iter()
        .filter_map(|category| {
            load_sound_catalog(game_dir, category).map(|text| (category.to_string(), text))
        })
        .collect()
}

/// `\data\database\sounds\{category}.txt` with `soundid.h` in front (its `#include`).
pub fn load_sound_catalog(game_dir: &Path, category: &str) -> Option<String> {
    let mut src = GameSource::open(game_dir);
    let win_path = format!(
        "\\data\\database\\sounds\\{}.txt",
        category.to_ascii_lowercase()
    );
    let text = String::from_utf8(src.read(&win_path)?).ok()?;
    let header = src
        .read(r"\data\database\soundid.h")
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    Some(format!("{header}\n{text}"))
}

/// Filename inside `[cue]` in `sounds/story.txt`. `empty` reads `[Empty]`.
pub fn story_sound_filename(text: &str, cue: &str) -> Option<String> {
    sound_cue_value(text, cue, "filename")
}

/// `maxpayne_{mode}->a_playmusic( cue )` from `data\menu\menu.txt` `[OnInit]`
/// (`menumode` → `max_payne`, `graphicnovelmode` → `graphic_novel`).
pub fn mode_music_cue(menu_txt: &str, mode: &str) -> Option<String> {
    let receiver = format!("maxpayne_{}->a_playmusic(", mode.to_ascii_lowercase());
    menu_txt.lines().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        let at = lower.find(&receiver)? + receiver.len();
        let cue = line[at..]
            .split(')')
            .next()?
            .trim()
            .trim_matches('"')
            .trim();
        (!cue.is_empty()).then(|| cue.to_string())
    })
}

/// `[cue]` `Filename` in `data\database\music\music.txt`, as an archive path.
pub fn music_win_path(music_txt: &str, cue: &str) -> Option<String> {
    let file = sound_cue_value(music_txt, cue, "filename")?;
    let file = file.trim_matches('"').replace('/', "\\");
    Some(format!(
        "\\data\\database\\music\\{}",
        file.trim_start_matches('\\')
    ))
}

/// `camerapaths.txt` `[block]`: archive path of its first `[Animation]` `Filename` (a KF2 camera path).
pub fn camera_path_file(camerapaths_txt: &str, block: &str) -> Option<String> {
    let header = format!("[{}]", block.trim());
    let mut lines = camerapaths_txt.lines().skip_while(|line| {
        let head = line.split("//").next().unwrap_or("").trim();
        !head.eq_ignore_ascii_case(&header)
    });
    lines.next()?;
    // Nested sections are indented; the next block header starts at column 0.
    let file = lines
        .take_while(|line| !line.starts_with('['))
        .filter_map(assign_pair)
        .find(|(key, value)| key.eq_ignore_ascii_case("filename") && !value.is_empty())?
        .1
        .replace('/', "\\");
    Some(format!(
        "\\data\\database\\camerapaths\\{}",
        file.trim_start_matches('\\')
    ))
}

/// `particles.txt` `[effect]`: the `.rps` archive path from `[Default] filename` and the
/// `[Properties] Looping` flag.
pub fn particle_effect_file(particles_txt: &str, effect: &str) -> Option<(String, bool)> {
    let file = sound_cue_value(particles_txt, effect, "filename")?;
    let file = file.trim_matches('"').replace('/', "\\");
    Some((
        format!(
            "\\data\\database\\particles\\{}",
            file.trim_start_matches('\\')
        ),
        sound_cue_looping(particles_txt, effect),
    ))
}

/// The `[CharacterHit]` projectile a projectile file names.
fn load_splatter(source: &mut GameSource, text: &str) -> Option<hud::SplatterDef> {
    let (name, count) = hud::parse_character_hit(text)?;
    let file = source.read(&format!(r"\data\database\projectiles\{name}.txt"))?;
    hud::SplatterDef::parse(&String::from_utf8_lossy(&file), count)
}

/// `OneHandedUpperBodyBone` and `TwoHandedUpperBodyBone` of the default skeleton.
pub fn load_upper_bones(game_dir: &Path) -> [Vec<String>; 2] {
    let skeleton = read_game_bytes(game_dir, r"\data\database\skeletons\default_skeleton.txt")
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    ["OneHandedUpperBodyBone", "TwoHandedUpperBodyBone"]
        .map(|tag| kf2::parse_body_bones(&skeleton, tag))
}

/// The clip table of `skin` over the default skeleton, and the clips it names.
pub fn load_character_clips(game_dir: &Path, skin: &str) -> clips::CharacterClips {
    const SKELETON: &str = r"\data\database\skeletons\default_skeleton";
    let mut source = GameSource::open(game_dir);
    let mut text = |path: &str| {
        let bytes = source.read(path).unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let skeleton = text(&format!("{SKELETON}.txt"));
    let skin = text(&format!(r"\data\database\skins\{skin}.txt"));
    let table = clips::ClipTable::parse(&skeleton, &skin);
    let airborne_speed = hud::parse_sections(&skin)
        .iter()
        .find_map(|section| section.number("airbornespeed"))
        .unwrap_or_default();
    let abort_dive_frame = hud::parse_sections(&skin)
        .iter()
        .find_map(|section| section.number("canabortshootdodgeframe"))
        .unwrap_or_default() as u32;
    let files: std::collections::BTreeSet<&str> = table.files().collect();
    let anims = files
        .into_iter()
        .filter_map(|file| {
            let bytes = source.read(&format!(r"{SKELETON}\{file}"))?;
            let clip = kf2::parse_kf2_anim(&bytes)?;
            Some((file.to_string(), std::sync::Arc::new(clip)))
        })
        .collect();
    clips::CharacterClips {
        table,
        anims,
        airborne_speed,
        abort_dive_frame,
    }
}

/// The sound of a dummy projectile: the cue of the decal it leaves where it lands, as a footstep
/// says `d_createdecal( footstep_right )` and that block plays the step.
pub fn load_dummy_sound(game_dir: &Path, projectile: &str) -> Option<String> {
    let path = format!(r"\data\database\projectiles\{projectile}.txt");
    let text = read_game_bytes(game_dir, &path)?;
    let text = String::from_utf8_lossy(&text).to_ascii_lowercase();
    let decal = hud::weapon_cue_line(&text, "d_createdecal", 0)?;
    let decals = read_game_bytes(game_dir, r"\data\database\decals\decals.txt")?;
    decals::DecalSet::parse(&String::from_utf8_lossy(&decals), &decal)?.sound
}

/// `RightHandGunBone` and `LeftHandGunBone` of the default skeleton.
pub fn load_gun_bones(game_dir: &Path) -> [String; 2] {
    let skeleton = read_game_bytes(game_dir, r"\data\database\skeletons\default_skeleton.txt")
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    ["RightHandGunBone", "LeftHandGunBone"].map(|tag| {
        let bones = kf2::parse_body_bones(&skeleton, tag);
        bones.into_iter().next().unwrap_or_default()
    })
}

/// `CHARANIM_RANDOMDEATH1..4` of the default skeleton, with their sounds.
pub fn load_death_clips(game_dir: &Path) -> Vec<kf2::DeathClip> {
    const DIR: &str = r"\data\database\skeletons\";
    let mut source = GameSource::open(game_dir);
    let Some(skeleton) = source.read(&format!("{DIR}default_skeleton.txt")) else {
        return Vec::new();
    };
    let skeleton = String::from_utf8_lossy(&skeleton).into_owned();
    (1..=4)
        .filter_map(|number| {
            let index = format!("CHARANIM_RANDOMDEATH{number}");
            let header = format!("Index = {index};");
            let line = skeleton.lines().find(|line| line.contains(&header))?;
            let file = line.split_once("Filename =")?.1.split(';').next()?.trim();
            let bytes = source.read(&format!(r"{DIR}default_skeleton\{file}"))?;
            let clip = kf2::parse_kf2_anim(&bytes)?;
            let fps = clip.bones.first()?.fps.max(1.0);
            let sounds = hud::parse_clip_sounds(&skeleton, &index)
                .into_iter()
                .map(|(frame, catalog, cue)| (frame as f32 / fps, catalog, cue))
                .collect();
            let blood = hud::parse_clip_projectiles(&skeleton, &index)
                .into_iter()
                .filter_map(|(frame, name, count)| {
                    let path = format!(r"\data\database\projectiles\{name}.txt");
                    let file = source.read(&path)?;
                    let file = String::from_utf8_lossy(&file);
                    let blood = hud::SplatterDef::parse(&file, count)?;
                    blood.decal.is_some().then(|| (frame as f32 / fps, blood))
                })
                .collect();
            Some(kf2::DeathClip {
                clip,
                sounds,
                blood,
            })
        })
        .collect()
}

/// The model a weapon file names (`WeaponDef::model`) with its bitmaps.
pub fn load_weapon_model(game_dir: &Path, model: &str) -> Option<kf2::KfsMesh> {
    load_packed_model(game_dir, &format!(r"\data\database\weapons\{model}"))
}

/// `CapsuleRadius` of `skins/max_payne.txt`.
pub fn load_body_radius(game_dir: &Path) -> Option<f32> {
    let skin = read_game_bytes(game_dir, r"\data\database\skins\max_payne.txt")?;
    let sections = hud::parse_sections(&String::from_utf8_lossy(&skin));
    sections.iter().find_map(|s| s.number("capsuleradius"))
}

/// The item types `scene` places, with their rules and models.
pub fn load_level_items(
    game_dir: &Path,
    scene: &scene::LdbScene,
    rooms: &BTreeMap<i32, [f32; 3]>,
) -> (items::LevelItems, BTreeMap<String, kf2::KfsMesh>) {
    const DIR: &str = r"\data\database\level_items\";
    let mut level = items::LevelItems::default();
    level.body_radius = load_body_radius(game_dir).unwrap_or_default();
    let mut models = BTreeMap::new();
    let headers = read_game_bytes(game_dir, r"\data\database\level_items.h")
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    for item in &scene.items {
        let kind = item.item_type.to_ascii_lowercase();
        if !level.defs.contains_key(&kind) {
            let text = read_game_bytes(game_dir, &format!("{DIR}{kind}.txt"));
            let text = text.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            let Some(def) = text.and_then(|text| items::LevelItemDef::parse(&headers, &text))
            else {
                continue;
            };
            let path = resolve_win_path(&format!(r"{DIR}{kind}\{}", def.model));
            if let Some(model) = load_packed_model(game_dir, &path) {
                models.insert(kind.clone(), model);
            }
            level.defs.insert(kind.clone(), def);
        }
        let origin = rooms.get(&item.room_id).copied().unwrap_or([0.0; 3]);
        level.items.push(items::LiveItem {
            name: item.name.clone(),
            kind,
            pos: [0, 1, 2].map(|c| item.pos[c] + origin[c]),
            rot: item.rot,
            taken: false,
        });
    }
    (level, models)
}

/// `path` with its `..` steps taken.
fn resolve_win_path(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('\\').filter(|part| !part.is_empty()) {
        if part == ".." {
            parts.pop();
        } else {
            parts.push(part);
        }
    }
    format!(r"\{}", parts.join(r"\"))
}

/// The model of a flying projectile, packed or a bare `.kfs`.
pub fn load_flying_model(game_dir: &Path, win_path: &str) -> Option<kf2::KfsMesh> {
    if !win_path.to_ascii_lowercase().ends_with(".kfs") {
        return load_packed_model(game_dir, win_path);
    }
    let mut source = GameSource::open(game_dir);
    let (folder, _) = win_path.rsplit_once('\\')?;
    let data = source.read(win_path)?;
    kf2::parse_kfs_mesh(&data, |file| {
        let bytes = source.read(&format!(r"{folder}\textures\{file}"))?;
        let (width, height, pixels) = scene::decode_texture_blob(&bytes)?;
        Some(scene::DecodedTexture {
            path: file.to_string(),
            flag: 4,
            width,
            height,
            pixels,
        })
    })
}

/// A packed `.kf2` model. Its materials search `textures;..\\sharedtextures`.
fn load_packed_model(game_dir: &Path, win_path: &str) -> Option<kf2::KfsMesh> {
    let mut source = GameSource::open(game_dir);
    let parts = kf2::parse_world_sphere(&source.read(win_path)?)?;
    let (folder, _) = win_path.rsplit_once('\\')?;
    Some(kf2::mesh_from_parts(win_path, &parts, |file| {
        let bytes = source
            .read(&format!(r"{folder}\textures\{file}"))
            .or_else(|| {
                source.read(&resolve_win_path(&format!(
                    r"{folder}\..\sharedtextures\{file}"
                )))
            })?;
        let (width, height, pixels) = scene::decode_texture_blob(&bytes)?;
        Some(scene::DecodedTexture {
            path: file.to_string(),
            flag: 4,
            width,
            height,
            pixels,
        })
    }))
}

/// `Filename`s of `block`'s `[Material]` named `material`, or of its `[Default]` when it has none.
pub fn material_files(text: &str, block: &str, material: &str) -> Vec<String> {
    let header = format!("[{}]", block.trim());
    let code = |line: &'_ str| line.split("//").next().unwrap_or("").trim().to_string();
    let lines = text
        .lines()
        .skip_while(|line| !code(line).eq_ignore_ascii_case(&header))
        .skip(1);
    let (mut depth, mut opened) = (0_usize, false);
    // `None` inside `[Default]`; the name of the `[Material]` otherwise.
    let mut current: Option<String> = None;
    let (mut defaults, mut matches) = (Vec::new(), Vec::new());
    for line in lines {
        let line = code(line);
        depth += line.matches('{').count();
        depth = depth.saturating_sub(line.matches('}').count());
        opened |= line.contains('{');
        if opened && depth == 0 {
            break;
        }
        match bracket_name(&line).map(str::to_ascii_lowercase).as_deref() {
            Some("default") => current = None,
            Some("material") => current = Some(String::new()),
            _ => {}
        }
        let Some((key, value)) = assign_pair(&line) else {
            continue;
        };
        if key.eq_ignore_ascii_case("name") && current.is_some() {
            current = Some(value);
        } else if key.eq_ignore_ascii_case("filename") {
            match &current {
                None => defaults.push(value),
                Some(name) if name.eq_ignore_ascii_case(material) => matches.push(value),
                Some(_) => {}
            }
        }
    }
    if matches.is_empty() {
        defaults
    } else {
        matches
    }
}

/// Distance model of a `3DSound = YES` cue: full `volume` inside `hotspot` metres, released past
/// `falloff` metres (`sndmfc.dll` `S_SoundOmni` `+0x149` / `+0x14d`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundRange {
    pub hotspot: f32,
    pub falloff: f32,
    pub volume: f32,
}

/// `Hotspot`, `FallOff` and `Volume` of `[cue]`, `#define`s resolved; `None` unless `3DSound = YES`.
pub fn sound_cue_3d(text: &str, cue: &str) -> Option<SoundRange> {
    if !sound_cue_value(text, cue, "3dsound")?.eq_ignore_ascii_case("yes") {
        return None;
    }
    let number = |key: &str| {
        let value = sound_cue_value(text, cue, key)?;
        value.parse::<f32>().ok().or_else(|| {
            text.lines().find_map(|line| {
                let mut words = line.trim().strip_prefix("#define")?.split_whitespace();
                (words.next()? == value).then(|| words.next()?.parse().ok())?
            })
        })
    };
    Some(SoundRange {
        hotspot: number("hotspot")?,
        falloff: number("falloff")?,
        volume: number("volume").unwrap_or(1.0),
    })
}

/// `Looping = YES` inside `[cue]` (siren and engine loops in `sounds/dynamic.txt`).
pub fn sound_cue_looping(text: &str, cue: &str) -> bool {
    sound_cue_value(text, cue, "looping")
        .is_some_and(|v| v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true"))
}

/// First non-empty `key = value` inside `[cue]`, across its `[Properties]`/`[Default]`/`[Sound]` parts.
fn sound_cue_value(text: &str, cue: &str, wanted: &str) -> Option<String> {
    let header = format!("[{}]", cue.trim());
    let mut grab = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if !grab {
            let head = trimmed.split("//").next().unwrap_or("").trim();
            if head.eq_ignore_ascii_case(&header) {
                grab = true;
            }
            continue;
        }
        if let Some(name) = bracket_name(trimmed) {
            if !matches!(
                name.to_ascii_lowercase().as_str(),
                "properties" | "default" | "sound"
            ) {
                break;
            }
        }
        if let Some((key, value)) = assign_pair(line) {
            if key.eq_ignore_ascii_case(wanted) && !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// What the graphic-novel chrome does with one key or click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NovelInput {
    /// Next-page indicator. Past the last collected page, leave the mode.
    Next,
    /// Previous-page indicator. Stays on the first page.
    Previous,
    /// Exit-to-game indicator.
    Quit,
}

/// `None` commits the pending mode exit. `Some` is the page index to show.
pub fn novel_turn(index: usize, count: usize, input: NovelInput) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let index = index.min(count - 1);
    match input {
        NovelInput::Quit => None,
        NovelInput::Previous => Some(index.saturating_sub(1)),
        NovelInput::Next => {
            let next = index + 1;
            if next >= count {
                None
            } else {
                Some(next)
            }
        }
    }
}

/// Escape leaves. Right, Space, Enter, and the left click advance. Left goes back.
pub fn novel_input(escape: bool, advance: bool, back: bool) -> Option<NovelInput> {
    if escape {
        Some(NovelInput::Quit)
    } else if advance {
        Some(NovelInput::Next)
    } else if back {
        Some(NovelInput::Previous)
    } else {
        None
    }
}

fn assign_pair(line: &str) -> Option<(String, String)> {
    let line = line.split("//").next()?.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let value = value.trim().trim_end_matches(';').trim().trim_matches('"');
    if key.trim().is_empty() {
        None
    } else {
        Some((key.trim().to_string(), value.to_string()))
    }
}

fn play_sound_cue(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let at = lower.find("a_playsound(")?;
    let rest = &line[at + "a_playsound(".len()..];
    let end = rest.find(')')?;
    let cue = rest[..end].split(',').nth(1)?.trim().trim_matches('"');
    if cue.is_empty() {
        None
    } else {
        Some(cue.to_string())
    }
}

fn bracket_name(trimmed: &str) -> Option<&str> {
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() || inner.contains('[') {
        None
    } else {
        Some(inner)
    }
}

/// Jpeg bytes for `gn_background.jpg`, plus each collected page and its story wav.
pub fn load_novel_session(game_dir: &Path, pages: &[String]) -> (Vec<u8>, Vec<NovelShownPage>) {
    let mut src = GameSource::open(game_dir);
    let chrome_txt = src
        .read(r"\data\graphicnovel\graphicnovel.txt")
        .unwrap_or_default();
    let chrome = parse_graphic_novel_chrome(&String::from_utf8_lossy(&chrome_txt));
    let background = if chrome.background.is_empty() {
        Vec::new()
    } else {
        src.read(&novel_background_win_path(&chrome.background))
            .unwrap_or_default()
    };
    let pages_txt = src
        .read(r"\data\database\graphicnovelpages\graphicnovelpages.txt")
        .unwrap_or_default();
    let catalog = parse_graphic_novel_pages(&String::from_utf8_lossy(&pages_txt));
    let story_txt = src
        .read(r"\data\database\sounds\story.txt")
        .unwrap_or_default();
    let story = String::from_utf8_lossy(&story_txt);
    let shown = pages
        .iter()
        .map(|id| {
            let def = catalog.iter().find(|p| p.id.eq_ignore_ascii_case(id));
            let cue = def.map(|p| p.sound.clone()).unwrap_or_default();
            let wav = story_sound_filename(&story, &cue)
                .filter(|name| !name.eq_ignore_ascii_case("placeholder.wav"))
                .and_then(|name| src.read(&story_wav_win_path(&name)))
                .unwrap_or_default();
            NovelShownPage {
                id: id.clone(),
                jpeg: src.read(&novel_page_win_path(id)).unwrap_or_default(),
                wav,
            }
        })
        .collect();
    (background, shown)
}

#[derive(Debug, Clone)]
pub struct NovelShownPage {
    pub id: String,
    pub jpeg: Vec<u8>,
    pub wav: Vec<u8>,
}

pub fn level_entry(game_dir: &Path, level_id: &str) -> Result<LevelsTxtEntry, EngineError> {
    let mut src = GameSource::open(game_dir);
    let levels = src
        .read(r"\data\database\levels\levels.txt")
        .ok_or_else(|| {
            EngineError::Msg("levels.txt not found under data/ or a *.ras archive".into())
        })?;
    parse_levels_txt(&String::from_utf8_lossy(&levels))
        .into_iter()
        .find(|e| e.id.eq_ignore_ascii_case(level_id))
        .ok_or_else(|| EngineError::Msg(format!("level {level_id} missing from levels.txt")))
}

/// `levels.txt` entry `level_id` as archive bytes (loose file or `*.ras`).
pub fn load_level_by_id(game_dir: &Path, level_id: &str) -> Result<(String, Vec<u8>), EngineError> {
    let mut src = GameSource::open(game_dir);
    let levels = src
        .read(r"\data\database\levels\levels.txt")
        .ok_or_else(|| {
            EngineError::Msg("levels.txt not found under data/ or a *.ras archive".into())
        })?;
    let entries = parse_levels_txt(&String::from_utf8_lossy(&levels));
    let entry = entries
        .iter()
        .find(|e| e.id.eq_ignore_ascii_case(level_id))
        .ok_or_else(|| EngineError::Msg(format!("level {level_id} missing from levels.txt")))?;
    let win = entry.ldb_archive_path();
    let bytes = src
        .read(&win)
        .ok_or_else(|| EngineError::Msg(format!("level file not found: {win}")))?;
    Ok((win, bytes))
}

pub fn load_main_menu(game_dir: &Path) -> Result<Vec<MenuEntry>, EngineError> {
    let mut src = GameSource::open(game_dir);
    let bytes = src.read(r"\data\menu\menu.txt").ok_or_else(|| {
        EngineError::Msg("menu.txt not found under data/ or a *.ras archive".into())
    })?;
    let menu = parse_main_menu(&String::from_utf8_lossy(&bytes));
    if menu.is_empty() {
        return Err(EngineError::Msg("main menu has no entries".into()));
    }
    Ok(menu)
}

/// Intro path only. Does not read a level.
pub fn startup_video_path(game_dir: &Path) -> Option<PathBuf> {
    let mut src = GameSource::open(game_dir);
    let config = src.read(r"\data\globalconfig.txt")?;
    let name = startup_video_name(&String::from_utf8_lossy(&config))?;
    let path = game_dir.join(name.replace('\\', "/"));
    path.is_file().then_some(path)
}

/// `ffmpeg` BGRA bytes → `0RGB` frame pixels (minifb and the GPU blit both take `0RGB`).
pub fn bgra_to_frame(bytes: &[u8], frame: &mut [u32]) {
    for (px, b) in frame.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *px = u32::from(b[2]) << 16 | u32::from(b[1]) << 8 | u32::from(b[0]);
    }
}

/// Play the intro in `screen`: `ffmpeg -re` decodes 960 × 600 BGRA frames on a pipe, and
/// `ffplay -nodisp` plays the sound. Escape, Space or Enter skips. A missing `ffmpeg` is an
/// error string; the caller still opens the menu.
pub fn play_intro_video(path: &Path, screen: &mut frontend::Screen) -> Result<(), String> {
    use frontend::Frontend;
    use minifb::{Key, KeyRepeat};
    use std::io::Read;
    use std::process::Stdio;
    let (w, h) = (render3d::FRAME_W, render3d::FRAME_H);
    let mut video = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-re", "-i"])
        .arg(path)
        .args(["-an", "-f", "rawvideo", "-pix_fmt", "bgra", "-vf"])
        .arg(format!("scale={w}:{h}"))
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let audio = Command::new("ffplay")
        .args(["-nodisp", "-autoexit", "-loglevel", "error"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok();
    let mut pipe = video.stdout.take().ok_or("ffmpeg: no stdout")?;
    let mut bytes = vec![0u8; w * h * 4];
    let mut frame = vec![0u32; w * h];
    let skip = [Key::Escape, Key::Space, Key::Enter];
    while pipe.read_exact(&mut bytes).is_ok() {
        bgra_to_frame(&bytes, &mut frame);
        screen.pump();
        if !screen.is_open()
            || skip
                .iter()
                .any(|&k| screen.is_key_pressed(k, KeyRepeat::No))
        {
            break;
        }
        screen.show(&frame)?;
    }
    // Kill before closing the pipe, or a skipped video logs ffmpeg's broken-pipe errors.
    let _ = video.kill();
    drop(pipe);
    let _ = video.wait();
    if let Some(mut audio) = audio {
        let _ = audio.kill();
        let _ = audio.wait();
    }
    Ok(())
}

struct GameSource {
    game_dir: PathBuf,
    ras_paths: Vec<PathBuf>,
    opened: Vec<openras::archive::RasIndex>,
    cursor: usize,
}

impl GameSource {
    fn open(game_dir: &Path) -> Self {
        Self {
            game_dir: game_dir.to_path_buf(),
            ras_paths: ras_candidates(game_dir),
            opened: Vec::new(),
            cursor: 0,
        }
    }

    /// Loose `data/` first, then the archives in order. Archives are indexed, not decoded: a
    /// lookup reads only the member it returns.
    fn read(&mut self, win_path: &str) -> Option<Vec<u8>> {
        if let Some(bytes) = loose_bytes(&self.game_dir, win_path) {
            return Some(bytes);
        }
        for index in &self.opened {
            if let Ok(Some(bytes)) = index.read_member(win_path) {
                return Some(bytes);
            }
        }
        while self.cursor < self.ras_paths.len() {
            let path = self.ras_paths[self.cursor].clone();
            self.cursor += 1;
            let Ok(index) = openras::archive::RasIndex::open(&path) else {
                continue;
            };
            let hit = index.read_member(win_path).ok().flatten();
            self.opened.push(index);
            if hit.is_some() {
                return hit;
            }
        }
        None
    }
}

fn loose_bytes(game_dir: &Path, win_path: &str) -> Option<Vec<u8>> {
    let rel = win_to_rel(win_path);
    let loose = game_dir.join(&rel);
    loose.is_file().then(|| fs::read(loose).ok())?
}

fn win_to_rel(win_path: &str) -> PathBuf {
    PathBuf::from(win_path.trim_matches(['\\', '/']).replace('\\', "/"))
}

fn ras_candidates(game_dir: &Path) -> Vec<PathBuf> {
    let mut data = Vec::new();
    let mut rest = Vec::new();
    let Ok(rd) = fs::read_dir(game_dir) else {
        return Vec::new();
    };
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().to_ascii_lowercase();
        if !name.ends_with(".ras") {
            continue;
        }
        if name == "x_data.ras" {
            data.push(ent.path());
        } else {
            rest.push(ent.path());
        }
    }
    rest.sort();
    data.extend(rest);
    data
}

#[cfg(test)]
fn archive_member<'a>(arc: &'a RasArchive, win_path: &str) -> Option<&'a [u8]> {
    let mut want = win_path.replace('/', "\\");
    if !want.starts_with('\\') {
        want.insert(0, '\\');
    }
    for f in &arc.files {
        let dname = arc
            .directories
            .get(f.dir_index as usize)
            .map(|d| d.name.as_str())
            .unwrap_or("\\");
        let full = format!("{dname}{}", f.name);
        if full.eq_ignore_ascii_case(&want) {
            return Some(f.data.as_slice());
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
#[allow(clippy::same_item_push)] // MF fixture builders push tag bytes then payloads
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
        let preview = sample_ras_ldb_vec3_preview(&buf, 8);
        assert_eq!(preview.len(), 2);
        assert_eq!(preview[0], [1.0, 2.0, 3.0]);
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
    fn u24_count_section_after_offset_zero_is_memory_file() {
        // Part1_Level1 @0x6438fb: `1c 12 <u24>` MF array whose first body byte is 0x16.
        let mut data = vec![0u8; 8];
        data.extend_from_slice(&[LDB_SECTION_TAG, 0x12, 0x58, 0x3c, 0x01, 0x16, 0, 0]);
        let sec = parse_ras_ldb_section_at(&data, 8).unwrap();
        assert_eq!(sec.kind, LDB_SECTION_KIND_MF);
        assert_eq!(sec.flag, None);
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
        assert_eq!(polys[0].verts, 3);
        assert_eq!(polys[1].index, 1);
        assert_eq!(body[end], 0x1f);
    }

    #[test]
    fn parses_import_mesh_quad_poly() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x14, 1, 0x14, 0]);
        body.extend_from_slice(&[0x14, 7, 0x14, 4]); // pref, verts=4
        body.push(0x16);
        for v in [0.0f32, -1.0, 0.0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&[0x11, 1, 0x14, 0, 0x14, 1]);
        body.push(0x09);
        body.extend_from_slice(&0.0f32.to_le_bytes());
        body.push(0x09);
        body.extend_from_slice(&0.0f32.to_le_bytes());
        body.extend_from_slice(&[0x14, 0, 0x14, 9]);
        let (hdr0, polys, end) = parse_ras_ldb_import_mesh_polys_at(&body, 0).unwrap();
        assert_eq!(hdr0, 1);
        assert_eq!(polys.len(), 1);
        assert_eq!(polys[0].verts, 4);
        assert_eq!(polys[0].index, 9);
        assert_eq!(end, body.len());
    }

    #[test]
    fn parses_import_mesh_attribs_list() {
        let mut body = Vec::new();
        body.push(LDB_IMPORT_MATERIAL_LIST_TAG);
        body.extend_from_slice(&[0x14, 2]); // count
        for id in [40u8, 41u8] {
            body.extend_from_slice(&[0x14, id]);
            body.push(0x16);
            for v in [1.0f32, 2.0, 3.0] {
                body.extend_from_slice(&v.to_le_bytes());
            }
        }
        body.extend_from_slice(&[0x14, 99]); // next mesh int
        let (count, attrs, end) = parse_ras_ldb_import_mesh_attribs_at(&body, 0).unwrap();
        assert_eq!(count, 2);
        assert_eq!(attrs[1].id, 41);
        assert_eq!(attrs[1].value, [1.0, 2.0, 3.0]);
        assert_eq!(body[end], 0x14);
        // materials must not match
        let mut mats = Vec::new();
        mats.push(LDB_IMPORT_MATERIAL_LIST_TAG);
        mats.extend_from_slice(&[0x14, 1]);
        mats.push(LDB_IMPORT_MATERIAL_TAG);
        assert!(parse_ras_ldb_import_mesh_attribs_at(&mats, 0).is_none());
    }

    #[test]
    fn parses_import_lights() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x14, 1]); // count
        for _ in 0..2 {
            body.extend_from_slice(&[0x0d, 0x14, 8]);
            body.extend_from_slice(b"Light::0");
        }
        for _ in 0..2 {
            body.push(0x1a);
            for v in [
                1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 3.0, 4.0, 5.0,
            ] {
                body.extend_from_slice(&v.to_le_bytes());
            }
        }
        body.extend_from_slice(&[0x14, 3, 0x0d, 0x14, 0]);
        body.push(0x19);
        for v in [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        for _ in 0..10 {
            body.push(0x09);
            body.extend_from_slice(&1.0f32.to_le_bytes());
        }
        body.extend_from_slice(&[0x14, 20]); // next section
        let (count, lights, end) = parse_ras_ldb_import_lights_at(&body, 0, 4).unwrap();
        assert_eq!(count, 1);
        assert_eq!(lights[0].name, "Light::0");
        assert_eq!(lights[0].pos, [3.0, 4.0, 5.0]);
        assert_eq!(lights[0].subtype, 3);
        assert_eq!(lights[0].param_count, 10);
        assert_eq!(body[end], 0x14);
    }

    #[test]
    fn parses_import_character_head_and_jumppoints() {
        let mut body = Vec::new();
        body.push(LDB_IMPORT_MATERIAL_LIST_TAG);
        body.extend_from_slice(&[0x14, 0, 0x14, 0, 0x14, 2]);
        body.extend_from_slice(&[0x0d, 0x14, 21]);
        body.extend_from_slice(b"::start::Jumppoint 00");
        body.extend_from_slice(&[0x0d, 0x14, 0]);
        body.push(0x1a);
        for v in [
            1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0,
        ] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        let jp2 = b"::room::Jumppoint 01";
        body.extend_from_slice(&[0x0d, 0x14, jp2.len() as u8]);
        body.extend_from_slice(jp2);
        let (count, at) = parse_ras_ldb_import_character_head_at(&body, 0).unwrap();
        assert_eq!(count, 2);
        assert_eq!(body[at], 0x0d);
        let names = ras_ldb_import_jumppoint_names_at(&body, at, 8);
        assert_eq!(names, vec!["::start::Jumppoint 00", "::room::Jumppoint 01"]);
    }

    #[test]
    fn parses_import_do_hook_list_tutorial_shape() {
        // BYOD Tutorial.ldb @0x1f7458f: 1f + count + (DO_* + empty nest/list/nest)×N
        let mut body = Vec::new();
        body.push(LDB_IMPORT_MATERIAL_LIST_TAG);
        body.extend_from_slice(&[0x14, 4]);
        for name in [
            "DO_BulletCollides",
            "DO_CharacterCollides",
            "DO_MovedToInvalidPosition",
            "DO_OnDeath",
        ] {
            body.extend_from_slice(&[0x0d, 0x14, name.len() as u8]);
            body.extend_from_slice(name.as_bytes());
            body.extend_from_slice(&[0x1c, 0x14, 0, 0x1f, 0x14, 0, 0x1c, 0x14, 0]);
        }
        body.extend_from_slice(&[0x0d, 0x14, 18]);
        body.extend_from_slice(b"::Alley2::next_ent");
        let sc = b"::Alley2::barrel04.DO->DO_SetHitpoints(15);";
        body.extend_from_slice(&[0x0d, 0x14, sc.len() as u8]);
        body.extend_from_slice(sc);
        let (hooks, end) = parse_ras_ldb_import_do_hooks_at(&body, 0).unwrap();
        assert_eq!(
            hooks,
            vec![
                "DO_BulletCollides",
                "DO_CharacterCollides",
                "DO_MovedToInvalidPosition",
                "DO_OnDeath",
            ]
        );
        assert_eq!(&body[end..end + 3], &[0x0d, 0x14, 18]);
        assert_eq!(
            ras_ldb_import_do_scripts_at(&body, end, 4),
            vec!["::Alley2::barrel04.DO->DO_SetHitpoints(15);"]
        );
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
    fn reads_long_mf_string_0d13() {
        let mut body = Vec::new();
        let s = "::".to_string() + &"A".repeat(80);
        body.push(0x0d);
        body.push(0x13);
        body.extend_from_slice(&(s.len() as u16).to_le_bytes());
        body.extend_from_slice(s.as_bytes());
        let mut i = 0;
        assert_eq!(read_mf_str(&body, &mut i).as_deref(), Some(s.as_str()));
        assert_eq!(i, body.len());
        let (n, end) = soft_walk_mf_tags(&body, 0, 8);
        assert_eq!(n, 1);
        assert_eq!(end, body.len());
    }

    #[test]
    fn parses_ai_preamble_floats_and_vec3() {
        let mut body = Vec::new();
        body.extend_from_slice(&[0x14, 123, 0x14, 10, 0x14, 0, 0x14, 0]);
        for v in [0.3f32, 0.05, 3.0, 5.0] {
            body.push(0x09);
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.push(0x16);
        for v in [0.0f32, -20.0, 0.0] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        for v in [2.0f32, 0.0, 0.31] {
            body.push(0x09);
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&[0x14, 1, 0x14, 5]);
        body.push(0x1a);
        for v in [
            1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
        ] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.push(0xff);
        let (end, grav) = parse_ras_ai_preamble(&body).unwrap();
        assert_eq!(grav, [0.0, -20.0, 0.0]);
        assert_eq!(body[end], 0xff);
        let (n, _) = soft_walk_mf_tags(&body, 0, 32);
        assert!(n >= 8);
        // 0x19 mat3 + 0x15 dual-f32 + nest tags walk cleanly
        let mut ext = body.clone();
        ext.pop(); // drop 0xff
        ext.push(0x19);
        ext.extend_from_slice(&[0u8; 36]);
        ext.push(0x15);
        ext.extend_from_slice(&[0u8; 8]);
        ext.push(0x1c);
        ext.push(0x1f);
        let (n2, end2) = soft_walk_mf_tags(&ext, 0, 64);
        assert!(n2 > n);
        assert_eq!(end2, ext.len());
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
	PlayerStartingPlace = "::StartRoom::Jumppoint 00";
	PlayerSkinName  = "young_max_payne";
	WorldSphere = "sunset";
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
        assert_eq!(entries[0].player_start, None);
        assert_eq!(
            entries[1].player_start.as_deref(),
            Some("::StartRoom::Jumppoint 00")
        );
        assert_eq!(entries[0].player_skin, None);
        assert_eq!(entries[0].world_sphere, None);
        assert_eq!(entries[1].player_skin.as_deref(), Some("young_max_payne"));
        assert_eq!(entries[1].world_sphere.as_deref(), Some("sunset"));
    }

    #[test]
    fn levels_txt_fog_is_read_only_when_fogging_is_on() {
        // Retail: the startup intro fogs from 2000 to 5000, the house sets `Fogging = false`.
        let txt = r#"
[startup_level]
{
	[Properties]
	Directory 		= front_end;
	Level 			= startup_level.ldb;
	Fogging			= TRUE;
	FogColor		= ( 10, 10, 15 );
	FogStart		= 2000.0; // Level adjusts on the fly
	FogEnd			= 5000.0;
}
[part0_level1]
{
	[Properties]
	Directory 		= part1;
	Level 			= part0_level1.ldb;
	Fogging	= false;
	FogColor	= ( 19, 20, 21 );
	FogStart	= -200.0;
	FogEnd		= 500.0;
}
"#;
        let entries = parse_levels_txt(txt);
        assert_eq!(
            entries[0].fog,
            Some(crate::scene::LevelFog {
                color: [10.0, 10.0, 15.0],
                start: 2000.0,
                end: 5000.0,
            })
        );
        assert_eq!(entries[1].fog, None);
    }

    #[test]
    fn levels_txt_reads_the_level_gravity_through_its_define() {
        // Retail levels.txt: `#define GRAVITY_VALUE -20`, `Gravity = ( 0, GRAVITY_VALUE , 0 );`.
        let txt = "#define\t\tGRAVITY_VALUE\t-20\n[startup_level]\n{\n\t[Properties]\n\tDirectory = front_end;\n\tLevel = startup_level.ldb;\n\tGravity \t\t= ( 0, GRAVITY_VALUE , 0 );\n}\n";
        let entries = parse_levels_txt(txt);
        assert_eq!(entries[0].gravity, Some([0.0, -20.0, 0.0]));
    }

    #[test]
    fn bgra_video_bytes_become_0rgb_frame_pixels() {
        let bytes = [0x10, 0x20, 0x30, 0xff, 0x01, 0x02, 0x03, 0x00];
        let mut frame = vec![0u32; 2];
        bgra_to_frame(&bytes, &mut frame);
        assert_eq!(frame, vec![0x0030_2010, 0x0003_0201]);
    }

    #[test]
    fn camera_path_file_reads_the_first_animation_of_the_block() {
        let text = "#define HUD_FADEOUT_TIME 0.25\r\n[shake_quick]\r\n{\r\n\t[Attributes]\r\n\tAbortable \t= FALSE;\r\n\r\n\t[Animationset]\r\n\t{\r\n\t\t[Animation]\r\n\t\tIndex = 0;\r\n\t\tFilename = \"effects\\explosion.kf2\";\r\n\t}\r\n\t[Exit]\r\n\t{\r\n\t}\r\n}\r\n\r\n[shake_normal]\r\n{\r\n\t[Animationset]\r\n\t{\r\n\t\t[Animation]\r\n\t\tFilename = \"effects\\shake_quick.kf2\";\r\n\t}\r\n}\r\n[no_file]\r\n{\r\n}\r\n[Death_01]\t\t// 3 second circle\r\n{\r\n\t[Animationset]\r\n\t{\r\n\t\t[Animation]\r\n\t\tFilename = \"cinematic\\death_01.KF2\";\r\n\t}\r\n}\r\n";
        assert_eq!(
            camera_path_file(text, "Shake_Normal").as_deref(),
            Some("\\data\\database\\camerapaths\\effects\\shake_quick.kf2")
        );
        assert_eq!(
            camera_path_file(text, "shake_quick").as_deref(),
            Some("\\data\\database\\camerapaths\\effects\\explosion.kf2")
        );
        assert_eq!(
            camera_path_file(text, "death_01").as_deref(),
            Some("\\data\\database\\camerapaths\\cinematic\\death_01.KF2")
        );
        assert_eq!(camera_path_file(text, "no_file"), None);
        assert_eq!(camera_path_file(text, "missing"), None);
    }

    #[test]
    fn particle_effect_file_reads_the_default_filename_and_looping() {
        let text = "#define PS_TIMEOUT 0.2\n[light_policecar]\t\t// Policecar lights\n{\n\t[Properties]\n\tLooping = true;\n\tInvisibleTimeout = PS_TIMEOUT ;\n\n\t[Default] \n\tfilename = lights\\light_policecar.rps;\n\tItemsToCache = 1;\n}\n[muzzle_beretta]\n{\n\t[Properties]\n\tLooping = false;\n\t[Default]\n\tfilename = weapons\\beretta.rps;\n}\n";
        assert_eq!(
            particle_effect_file(text, "Light_PoliceCar"),
            Some((
                "\\data\\database\\particles\\lights\\light_policecar.rps".to_string(),
                true
            ))
        );
        assert_eq!(
            particle_effect_file(text, "muzzle_beretta"),
            Some((
                "\\data\\database\\particles\\weapons\\beretta.rps".to_string(),
                false
            ))
        );
        assert_eq!(particle_effect_file(text, "missing"), None);
    }

    #[test]
    fn retail_weapon_models_load_with_their_bitmaps() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let defs = load_weapon_defs(&game);
        if defs.is_empty() {
            return;
        }
        let model = defs["beretta"].model.as_deref().expect("model");
        let mesh = load_weapon_model(&game, model).expect("beretta");
        assert_eq!(mesh.material_textures.len(), 3);
        assert!(mesh.material_textures.iter().all(Option::is_some));
        // `ProjectileOffset = (0, 0.06, 0)`: the barrel runs along +z, above the grip's pivot.
        let barrel = mesh.vertices.iter().map(|v| v[2]).fold(f32::MIN, f32::max);
        assert!((0.1..0.4).contains(&barrel), "front {barrel}");
        // `PumpShotgun` has two material groups and `Slide` one.
        let model = defs["pumpshotgun"].model.as_deref().expect("model");
        let mesh = load_weapon_model(&game, model).expect("pump shotgun");
        assert_eq!(mesh.material_textures.len(), 3);
        assert!(mesh.material_textures.iter().all(Option::is_some));
        assert_eq!(mesh.faces.len(), 203 + 72 + 48);
        assert_eq!(mesh.faces[203].indices[0], 281);
        // Only the slide works when the gun is pumped.
        let clip = defs["pumpshotgun"].gun_clip.as_ref().expect("clip");
        let seconds = crate::render3d::clip_seconds(clip);
        let moved = (1..20)
            .map(|step| kf2::part_verts(&mesh, clip, seconds * step as f32 / 20.0))
            .map(|verts| {
                let (node, run) = mesh.parts.last().expect("slide");
                assert_eq!(node, "Slide");
                let first = run.start;
                assert_eq!(verts[0], mesh.vertices[0]);
                (verts[first][2] - mesh.vertices[first][2]).abs()
            })
            .fold(0.0, f32::max);
        assert!((0.05..0.5).contains(&moved), "slide moved {moved}");
    }

    #[test]
    fn retail_weapons_name_clips_sounds_and_effects_that_exist() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let defs = load_weapon_defs(&game);
        if defs.is_empty() {
            return;
        }
        let sounds = load_sound_catalog(&game, "weapons").expect("weapons.txt");
        let particles = read_game_bytes(&game, r"\data\database\particles\particles.txt")
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .expect("particles.txt");
        let beretta = &defs["beretta"];
        assert_eq!(beretta.shots_per_second, 4.0);
        assert!(!beretta.two_handed);
        let damage = |name: &str| defs[name].projectiles.first().map(|shot| shot.damage);
        assert_eq!(damage("beretta"), Some(5));
        let blood = defs["beretta"].projectiles[0]
            .character_hit
            .as_ref()
            .expect("blood");
        assert_eq!(blood.decal.as_deref(), Some("blood_small"));
        assert_eq!((blood.speed, blood.gravity), (8.0, 1.0));
        assert_eq!(damage("deserteagle"), Some(10));
        assert_eq!(damage("sniper"), Some(70));
        assert_eq!(damage("leadpipe"), Some(9));
        let ingram = &defs["ingram"];
        let quiet: Vec<bool> = ingram.rounds.iter().map(|round| round.quiet).collect();
        assert_eq!(quiet, [false, true, false, true]);
        // All four fit one 1 / 8 s `DefaultShootingFrequency` cycle.
        assert!(ingram.rounds[3].at < 1.0 / ingram.shots_per_second);
        assert!(defs["berettadual"].dual && defs["berettadual"].rounds[1].left);
        assert_eq!(load_gun_bones(&game), ["gun", "hand-l"]);
        let quiet = defs["ingram"].round_projectiles(true)[0].decal.as_deref();
        assert_eq!(quiet, Some("bullethit_noeffects"));
        let shotgun = &defs["pumpshotgun"].projectiles;
        let plan: Vec<(u32, Option<&str>)> = shotgun
            .iter()
            .map(|shot| (shot.count, shot.decal.as_deref()))
            .collect();
        assert_eq!(
            plan,
            [
                (1, Some("bullethit_shotgun")),
                (2, Some("bullethit_shotgun")),
                (2, Some("bullethit_shotgun_group")),
            ]
        );
        assert!(defs["pumpshotgun"].two_handed);
        assert!(beretta.fire_clip.is_some() && beretta.reload_clip.is_some());
        assert!(beretta.aim_clips.is_some() && defs["pumpshotgun"].aim_clips.is_some());
        assert!(beretta.reload_sound_delay > 0.0);
        // These script the shot on the body clip, not in the weapon file.
        assert_eq!(defs["ingram"].fire_sound.as_deref(), Some("shoot_ingram"));
        assert_eq!(defs["ingram"].muzzle.as_deref(), Some("Muzzle_ingram"));
        assert_eq!(
            defs["leadpipe"].fire_sound.as_deref(),
            Some("shoot_leadpipe")
        );
        let pipe = defs["leadpipe"].melee_reach.expect("pipe reach");
        assert!((pipe - 1.8).abs() < 1e-4, "{pipe}");
        assert_eq!(defs["leadpipe"].melee_blows.len(), 2);
        assert_eq!(defs["beretta"].melee_reach, None);
        for (name, def) in &defs {
            if !matches!(name.as_str(), "empty" | "painkiller") {
                let model = def.model.as_deref().unwrap_or_else(|| panic!("{name}"));
                assert!(load_weapon_model(&game, model).is_some(), "{name}: {model}");
            }
        }
        for def in defs.values() {
            if let Some(cue) = &def.fire_sound {
                let file = story_sound_filename(&sounds, cue).unwrap_or_else(|| panic!("{cue}"));
                let path = story_wav_win_path(&file);
                assert!(read_game_bytes(&game, &path).is_some(), "{path}");
            }
            if let Some(effect) = &def.muzzle {
                let (path, _) =
                    particle_effect_file(&particles, effect).unwrap_or_else(|| panic!("{effect}"));
                assert!(read_game_bytes(&game, &path).is_some(), "{path}");
            }
        }
    }

    #[test]
    fn retail_death_clips_end_with_the_body_on_the_floor() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        if read_game_bytes(&game, r"\data\database\skeletons\default_skeleton.txt").is_none() {
            return;
        }
        let deaths = load_death_clips(&game);
        assert_eq!(deaths.len(), 4);
        for death in &deaths {
            let last = kf2::clip_end(&death.clip);
            let locals = kf2::sample_locals(&death.clip, last);
            let world = kf2::compose_world(&death.clip, &locals);
            let pelvis = world["pelvis"].1[1];
            assert!(pelvis < 0.5, "pelvis at {pelvis} m");
            assert_eq!(death.sounds[0].1, "enemy");
        }
    }

    #[test]
    fn win_paths_take_their_parent_steps() {
        assert_eq!(
            resolve_win_path(r"\data\database\level_items\beretta\..\..\weapons\beretta\x.kf2"),
            r"\data\database\weapons\beretta\x.kf2"
        );
    }

    #[test]
    fn retail_tutorial_items_all_have_rules_and_models() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let Some(bytes) = read_game_bytes(&game, r"\data\database\levels\front_end\Tutorial.ldb")
        else {
            return;
        };
        let scene = scene::parse_ldb_scene(&bytes).expect("tutorial");
        let (level, models) = load_level_items(&game, &scene, &BTreeMap::new());
        assert_eq!(level.items.len(), scene.items.len());
        let gun = level.items.iter().find(|item| item.kind == "pumpshotgun");
        // The van's shotgun lies across the floor, turned 115 degrees from the model.
        assert_eq!(gun.expect("van shotgun").rot[0], -0.42261827);
        for item in &level.items {
            assert!(level.defs.contains_key(&item.kind), "{}", item.kind);
            let model = models
                .get(&item.kind)
                .unwrap_or_else(|| panic!("{}", item.kind));
            assert!(
                model.material_textures.iter().any(Option::is_some),
                "{}",
                item.kind
            );
        }
    }

    #[test]
    fn material_files_read_a_flat_particle_block() {
        let text = "[other]\n{\n[Default]\nFilename = other.rps;\n}\n[hit]\t// Hits\n{\n[Properties]\nLooping = false;\n[Default]\nFilename = hit\\default.rps;\n[Material]\nName = \"wood\";\nFilename = hit\\wood.rps;\n}\n[next]\n{\n[Material]\nName = \"metal\";\nFilename = next.rps;\n}\n";
        assert_eq!(material_files(text, "HIT", "Wood"), ["hit\\wood.rps"]);
        assert_eq!(material_files(text, "hit", "metal"), ["hit\\default.rps"]);
    }

    #[test]
    fn material_files_list_every_sound_of_a_nested_block() {
        let text = "[Hit]\n{\n\t[Properties] ID = 0; Probability = 100%;\n\t[Default]\n\t{\n\t\t[Sound]\n\t\tFilename = d1.wav;\n\t\t[Sound]\n\t\tFilename = d2.wav;\n\t}\n\t[Material]\n\t{\n\t\t[General]\n\t\tName = \"carpet\";\n\t\t[Sound]\n\t\tFilename = c1.wav;\n\t\t[Sound]\n\t\tFilename = c2.wav;\n\t}\n}\n[After]\n{\n\t[Default]\n\t{\n\t\t[Sound]\n\t\tFilename = after.wav;\n\t}\n}\n";
        assert_eq!(material_files(text, "hit", "CARPET"), ["c1.wav", "c2.wav"]);
        assert_eq!(material_files(text, "hit", "stone"), ["d1.wav", "d2.wav"]);
        assert!(material_files(text, "missing", "stone").is_empty());
    }

    #[test]
    fn sound_cue_3d_reads_hotspot_falloff_and_volume_with_defines() {
        let text = "#define\tID_DOOR_HOTSPOT\t\t\t\t 2\n#define DYNAMIC_MEDIUMFALLOFF\t20\n[door]\n{\n[Default]\n{\n[Sound]\nFilename = d.wav;\nVolume = 0.6;\n3Dsound = YES;\nHotspot = ID_DOOR_HOTSPOT;\nFallOff = DYNAMIC_MEDIUMFALLOFF;\n}\n}\n[flat]\n{\n[Sound]\nFilename = f.wav;\n3DSound = NO;\nHotspot = 1;\nFallOff = 5;\n}\n";
        assert_eq!(
            sound_cue_3d(text, "door"),
            Some(SoundRange {
                hotspot: 2.0,
                falloff: 20.0,
                volume: 0.6,
            })
        );
        assert_eq!(sound_cue_3d(text, "flat"), None);
        assert_eq!(sound_cue_3d(text, "missing"), None);
    }

    #[test]
    fn sound_cue_looping_reads_the_cue_block_only() {
        let text = "[a_loop]\n{\n[Sound]\nFilename = x.wav;\nLooping = YES;\n}\n[b]\n{\n[Sound]\nFilename = y.wav;\n}\n";
        assert!(sound_cue_looping(text, "A_Loop"));
        assert!(!sound_cue_looping(text, "b"));
        assert!(!sound_cue_looping(text, "missing"));
    }

    #[test]
    fn menu_init_names_the_music_of_each_mode() {
        let menu = "[OnInit]\n{\n\t[Message] String = \"maxpayne_menumode->a_playmusic( max_payne );\";\n\t[Message] String = \"maxpayne_graphicnovelmode->a_playmusic( graphic_novel );\";\n}\n";
        assert_eq!(
            mode_music_cue(menu, "menumode").as_deref(),
            Some("max_payne")
        );
        assert_eq!(
            mode_music_cue(menu, "graphicnovelmode").as_deref(),
            Some("graphic_novel")
        );
        assert_eq!(mode_music_cue(menu, "gamemode"), None);
    }

    #[test]
    fn music_cues_map_to_wavs_in_the_music_database() {
        let music = "#define\tMUSIC_FADETIME\t\t1.0\n[max_payne]\t\t//  1. Max Payne Theme\nFilename \t= \"wavs\\max_theme.wav\";\nFadeInTime\t= MUSIC_FADETIME ;\nLooping\t\t= true;\n\n[just_forget]\nFilename\t= \"wavs\\just_forget_about_it.wav\";\n";
        assert_eq!(
            music_win_path(music, "max_payne").as_deref(),
            Some(r"\data\database\music\wavs\max_theme.wav")
        );
        assert_eq!(music_win_path(music, "nope"), None);
    }

    #[test]
    fn retail_music_cues_say_whether_they_loop() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let Some(music) = read_game_bytes(&game, r"\data\database\music\music.txt") else {
            return;
        };
        let music = String::from_utf8_lossy(&music);
        assert!(sound_cue_looping(&music, "old_building"));
        assert!(!sound_cue_looping(&music, "empty"));
    }

    #[test]
    fn parse_levels_txt_reads_ambient_color_through_defines() {
        let txt = r#"
#define 	AMBIENT_R		32
#define 	AMBIENT_G		33
#define 	AMBIENT_B		34
[startup_level]
{
	[Properties]
	Directory 		= front_end;
	Level 			= startup_level.ldb;
	AmbientColor 	= ( 140, 135, 110 );
}
[part1_level1]
{
	[Properties]
	Directory 		= part1;
	Level 			= part1_level1.ldb;
	AmbientColor 	= ( AMBIENT_R , AMBIENT_G , AMBIENT_B );
}
[part1_level2]
{
	[Properties]
	Directory 		= part1;
	Level 			= part1_level2.ldb;
}
"#;
        let entries = parse_levels_txt(txt);
        assert_eq!(entries[0].ambient, Some([140.0, 135.0, 110.0]));
        assert_eq!(entries[1].ambient, Some([32.0, 33.0, 34.0]));
        assert_eq!(entries[2].ambient, None);
    }

    #[test]
    fn parked_data_is_not_game_data() {
        // Localized games park the FixPack's `data/`, and retail then plays from the archives.
        let game = std::env::temp_dir().join(format!("omp-parked-{}", std::process::id()));
        let parked = game.join("data.disabled-hires");
        fs::create_dir_all(&parked).unwrap();
        fs::write(parked.join("globalconfig.txt"), "hi-res").unwrap();
        let found = loose_bytes(&game, r"\data\globalconfig.txt");
        fs::remove_dir_all(&game).unwrap();
        assert_eq!(found, None);
    }

    #[test]
    fn retail_difficulty_scales_enemies_and_leaves_max_alone() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        if !game.is_dir() {
            return;
        }
        let difficulty = load_difficulty(&game).expect("difficulty.txt");
        assert_eq!(difficulty.enemy_health, 0.8);
        let weapons = load_weapon_defs(&game);
        assert_eq!(weapons["beretta"].slot, Some(1));
        assert_eq!(weapons["pumpshotgun"].slot, Some(2));
        let config = read_game_bytes(&game, r"\data\globalconfig.txt").expect("globalconfig.txt");
        let config = String::from_utf8_lossy(&config);
        assert!(health::Falling::parse(&config).is_some());
        assert!(health::DamageFlash::parse(&config).is_some());
        assert!(weapons["painkiller"].fire_clip.is_some());
        assert_eq!(difficulty.auto_aim, [4.0, 2.0]);
        assert!(weapons["pumpshotgun"].auto_aim);
        assert!(!weapons["sniper"].auto_aim);
        assert_eq!(difficulty.player_health, 1.0);
        assert_eq!(difficulty.regenerate, 0.04);
        assert_eq!(difficulty.gained_immediately, 0.05);
        let max = load_player_health(&game).expect("max_payne.txt");
        assert_eq!(max.max, 60.0);
    }

    #[test]
    fn retail_van_boxes_load_the_pump_shotgun_they_name_as_sawed() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        if !game.is_dir() {
            return;
        }
        let mut player = hud::PlayerInventory {
            defs: load_weapon_defs(&game),
            ..Default::default()
        };
        // `level_items/ammo_shotgun.txt` twice, then `level_items/pumpshotgun.txt`.
        player.pickup_ammo("sawedshotgun", 10);
        player.pickup_ammo("sawedshotgun", 10);
        player.pickup_weapon("pumpshotgun");
        player.pickup_ammo("pumpshotgun", 5);
        let stock = player.stock("pumpshotgun").expect("shells");
        assert_eq!((stock.clip, stock.pocket), (7, 18));
    }

    #[test]
    fn retail_thrown_weapons_fly_and_bullets_stay_rays() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        if !game.is_dir() {
            return;
        }
        let defs = load_weapon_defs(&game);
        for name in ["grenade", "molotov", "m79"] {
            let shot = defs[name].flying.first();
            let shot = shot.unwrap_or_else(|| panic!("{name} throws nothing"));
            assert!(
                !shot.0.blasts.is_empty() || !shot.0.frags.is_empty(),
                "{name}"
            );
            assert!(
                defs[name].projectiles.is_empty(),
                "{name} still fires a ray"
            );
        }
        assert!(defs["beretta"].flying.is_empty());
        let molotov = &defs["molotov"].flying[0].0;
        assert_eq!(molotov.frags[0].1, 5);
        assert!(molotov.frags[0].0.explodes_on_contact);
        let last = molotov.blasts.last().expect("molotov blasts");
        assert!(last.at > 0.0, "blast frames need the clip's rate");
        for name in ["grenade", "molotov", "m79"] {
            let model = defs[name].flying[0].0.model.as_deref().expect("model");
            assert!(load_flying_model(&game, model).is_some(), "{name}: {model}");
        }
    }

    #[test]
    fn retail_death_clips_leave_a_pool_of_blood() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        if !game.is_dir() {
            return;
        }
        let clips = load_death_clips(&game);
        assert!(!clips.is_empty());
        for clip in &clips {
            let (_, blood) = clip.blood.first().expect("every death bleeds");
            assert_eq!(blood.count, 2);
            assert_eq!(blood.decal.as_deref(), Some("blood_large"));
            assert_eq!(blood.speed, 3.0);
        }
    }

    #[test]
    fn retail_items_are_reached_by_max_paynes_capsule() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        if !game.is_dir() {
            return;
        }
        assert_eq!(load_body_radius(&game), Some(0.48));
    }

    #[test]
    fn retail_weapon_files_give_their_clip_sizes() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let defs = load_weapon_defs(&std::path::PathBuf::from(home).join("Games/MaxPayne1"));
        if defs.is_empty() {
            return;
        }
        assert_eq!(defs.len(), 17, "{:?}", defs.keys());
        assert_eq!(defs["beretta"].clip_size, 18);
        assert_eq!(defs["beretta"].pocket_size, 162);
        assert_eq!(defs["mp5"].id, "WEAPONID_MP5");
        assert_eq!(defs["painkiller"].pocket_size, 8);
        assert_eq!(
            defs["beretta"].inventory_id,
            defs["berettadual"].inventory_id
        );
        // `C_Reload` at frame 39 of a 30 fps clip, and at frame 59 of a 60 fps one.
        assert!((defs["beretta"].reload_seconds - 1.3).abs() < 1e-3);
        assert!((defs["ingram"].reload_seconds - 59.0 / 60.0).abs() < 1e-3);
        // `[Revert]`: the sniper reloads as the MP5 does.
        assert_eq!(defs["sniper"].reload_seconds, defs["mp5"].reload_seconds);
        assert_eq!(defs["pumpshotgun"].reload_amount, 1);
        assert_eq!(defs.values().filter(|def| def.shows_dot).count(), 13);
        assert_eq!(defs["beretta"].reload_amount, -1);
        let best = load_best_weapons(
            &std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("Games/MaxPayne1"),
        );
        assert_eq!(
            best.first().map(String::as_str),
            Some("WEAPONID_JACKHAMMER")
        );
        assert_eq!(best.len(), 11);
    }

    #[test]
    fn world_sphere_catalog_names_the_house_sky() {
        let catalog = r#"
[sunset] //used in maxhome
{
	[Geometry]
	ExportData = bg_sunset.kf2;
}
[bronx]
{
	[Geometry]
	ExportData = bg_bronx.kf2;
}
"#;
        assert_eq!(
            world_sphere_export(catalog, "sunset").as_deref(),
            Some("bg_sunset.kf2")
        );
        assert_eq!(
            world_sphere_export(catalog, "SUNSET").as_deref(),
            Some("bg_sunset.kf2")
        );
    }

    #[test]
    fn startup_level_decodes_and_still_names_the_comic() {
        let path = std::path::Path::new(
            "/tmp/omp-xdata-ref/data/database/levels/front_end/Startup_level.ldb",
        );
        if !path.is_file() {
            return;
        }
        let bytes = std::fs::read(path).expect("startup ldb");
        assert!(
            scene::parse_ldb_scene(&bytes).is_some(),
            "startup_level did not decode"
        );
        let hand = level_handoff(&bytes).expect("handoff");
        assert_eq!(hand.novel_page, "P1L0a_001");
        assert_eq!(hand.next_level, "part0_level1");
    }

    #[test]
    fn level_handoff_reads_the_startup_script() {
        let bytes = b"maxpayne_graphicnovelmode->mpgnm_pickupnote(P1L0a_001);x_modeswitch->s_modeswitch(graphicnovel);maxpayne_gamemode->gm_init(part0_level1);";
        let hand = level_handoff(bytes).expect("handoff");
        assert_eq!(hand.novel_page, "P1L0a_001");
        assert_eq!(hand.next_level, "part0_level1");
        assert_eq!(
            novel_page_win_path("P1L0a_001"),
            "\\data\\database\\graphicnovelpages\\part1\\textures\\P1L0a_001.jpg"
        );
    }

    #[test]
    fn graphic_novel_catalog_reads_the_startup_page() {
        let chrome = parse_graphic_novel_chrome(
            "DefaultPageName = \"p1l0a_002\";\nBackground = \"bitmaps\\gn_background.jpg\";\nBackgroundWidth = 640;\nBackgroundHeight = 100;\n",
        );
        assert_eq!(chrome.default_page, "p1l0a_002");
        assert_eq!(chrome.background, "bitmaps\\gn_background.jpg");
        assert_eq!(
            (chrome.background_width, chrome.background_height),
            (640, 100)
        );
        assert_eq!(
            novel_background_win_path(&chrome.background),
            "\\data\\graphicnovel\\bitmaps\\gn_background.jpg"
        );
        let pages = parse_graphic_novel_pages(
            "[P1L0a_001]\t\t{ [geometry] ExportData = \"part1\\P1L0a_001.kf2\"; [general] NewChapter = no;\t[OnInitMessage] String = \"this->A_PlaySound( story, P1L0a_001 );\"; }\n[P1L0a_002]\t\t{ [geometry] ExportData = \"part1\\P1L0a_002.kf2\"; [general] NewChapter = yes;\t[OnInitMessage] String = \"this->A_PlaySound( story, empty );\"; }\n",
        );
        assert_eq!(pages[0].id, "P1L0a_001");
        assert_eq!(pages[0].sound, "P1L0a_001");
        assert!(!pages[0].new_chapter);
        assert!(pages[1].new_chapter);
        assert_eq!(pages[1].sound, "empty");
        let story = "[Empty]\n{\n\t[Sound]\n\t\tFilename = placeholder.wav;\n}\n[P1L0a_001]\n{\n\t[Sound]\n\t\tFilename = story\\part1\\P1L0a_001.wav;\n}\n";
        assert_eq!(
            story_sound_filename(story, "P1L0a_001").as_deref(),
            Some("story\\part1\\P1L0a_001.wav")
        );
        assert_eq!(
            story_wav_win_path("story\\part1\\P1L0a_001.wav"),
            "\\data\\database\\sounds\\story\\part1\\P1L0a_001.wav"
        );
        assert_eq!(novel_turn(0, 1, NovelInput::Next), None);
        assert_eq!(novel_turn(0, 3, NovelInput::Next), Some(1));
        assert_eq!(novel_turn(2, 3, NovelInput::Next), None);
        assert_eq!(novel_turn(0, 3, NovelInput::Previous), Some(0));
        assert_eq!(novel_turn(1, 3, NovelInput::Previous), Some(0));
        assert_eq!(novel_turn(1, 3, NovelInput::Quit), None);
        assert_eq!(novel_input(false, true, false), Some(NovelInput::Next));
        assert_eq!(novel_input(true, true, true), Some(NovelInput::Quit));
        assert_eq!(novel_input(false, false, true), Some(NovelInput::Previous));
    }

    #[test]
    fn retail_graphic_novel_files_match_when_extracted() {
        let root = std::path::Path::new("/tmp/omp-xdata-ref/data");
        let chrome_path = root.join("graphicnovel/graphicnovel.txt");
        let pages_path = root.join("database/graphicnovelpages/graphicnovelpages.txt");
        let story_path = root.join("database/sounds/story.txt");
        if !chrome_path.is_file() || !pages_path.is_file() || !story_path.is_file() {
            return;
        }
        let chrome = parse_graphic_novel_chrome(&std::fs::read_to_string(&chrome_path).unwrap());
        assert_eq!(chrome.default_page, "p1l0a_002");
        assert_eq!(chrome.background_height, 100);
        let pages = parse_graphic_novel_pages(&std::fs::read_to_string(&pages_path).unwrap());
        let intro = pages
            .iter()
            .find(|p| p.id == "P1L0a_001")
            .expect("intro page");
        assert_eq!(intro.sound, "P1L0a_001");
        assert!(!intro.new_chapter);
        let story = std::fs::read_to_string(&story_path).unwrap();
        assert_eq!(
            story_sound_filename(&story, &intro.sound).as_deref(),
            Some("story\\part1\\P1L0a_001.wav")
        );
    }

    #[test]
    fn sound_catalogs_load_breakable_and_dynamic() {
        let game = std::path::Path::new("/home/gzelante/Games/MaxPayne1");
        if !game.join("x_data.ras").is_file() {
            return;
        }
        let catalogs = load_sound_catalogs(game);
        assert_eq!(
            story_sound_filename(
                catalogs.get("breakable").expect("breakable"),
                "window_break_medium"
            )
            .as_deref(),
            Some(r"breakable\glass\break_glass_medium.wav")
        );
        assert_eq!(
            story_sound_filename(
                catalogs.get("dynamic").expect("dynamic"),
                "door_wood_locked"
            )
            .as_deref(),
            Some(r"dynamic\door\door_wood_locked3.wav")
        );
        // `ID_DOOR_*` come from `#include <..\soundid.h>`.
        assert_eq!(
            sound_cue_3d(catalogs.get("dynamic").expect("dynamic"), "briefcase_open"),
            Some(SoundRange {
                hotspot: 2.0,
                falloff: 20.0,
                volume: 1.0,
            })
        );
        let enemy = load_sound_catalog(game, "Enemy").expect("enemy catalog");
        assert_eq!(
            story_sound_filename(&enemy, "mobster_alert").as_deref(),
            Some(r"enemy\mobster\mobster_alert_01a.wav")
        );
        assert!(sound_cue_3d(&enemy, "mobster_alert").is_some_and(|r| r.volume > 0.0));
        assert_eq!(load_sound_catalog(game, "no_such_catalog"), None);
        let win = story_wav_win_path(r"breakable\glass\break_glass_medium.wav");
        assert_eq!(
            win,
            r"\data\database\sounds\breakable\glass\break_glass_medium.wav"
        );
        let wav = read_game_bytes(game, &win).expect("break wav");
        assert!(wav.starts_with(b"RIFF"));
    }

    #[test]
    fn italian_pack_holds_the_startup_story_wav() {
        let path = std::path::Path::new("/home/gzelante/Games/MaxPayne1/x_italian.ras");
        if !path.is_file() {
            return;
        }
        let arc = RasArchive::read(path).expect("x_italian.ras");
        let wav = archive_member(&arc, r"\data\database\sounds\story\part1\P1L0a_001.wav")
            .expect("story wav member");
        assert!(
            wav.starts_with(b"RIFF"),
            "got {:02x?}",
            &wav[..wav.len().min(12)]
        );
        assert_eq!(&wav[8..12], b"WAVE");
        let (_chrome, shown) =
            load_novel_session(path.parent().expect("game dir"), &["P1L0a_001".to_string()]);
        assert!(shown[0].wav.starts_with(b"RIFF"));
        assert_eq!(&shown[0].wav[8..12], b"WAVE");
    }

    #[test]
    fn italian_pack_holds_the_house_opening_wavs() {
        let path = std::path::Path::new("/home/gzelante/Games/MaxPayne1/x_italian.ras");
        if !path.is_file() {
            return;
        }
        let arc = RasArchive::read(path).expect("x_italian.ras");
        for page in ["P1L0a_003", "P1L0a_004", "P1L0a_005"] {
            let member = story_wav_win_path(&format!("story\\part1\\{page}.wav"));
            let wav = archive_member(&arc, &member).expect("house story wav");
            assert!(wav.starts_with(b"RIFF"), "{page}");
            assert_eq!(&wav[8..12], b"WAVE");
        }
    }

    #[test]
    fn level_handoff_ignores_a_comic_that_stays_in_the_level() {
        let mut bytes =
            b"mpgnm_pickupnote(P1L0a_007);s_modeswitch(graphicnovel);fsm_switch(normal);".to_vec();
        bytes.extend(std::iter::repeat_n(b' ', 300));
        bytes.extend(b"gm_init(part1_level1);");
        assert_eq!(level_handoff(&bytes), None);
    }

    #[test]
    fn level_opening_pages_are_the_unread_notes_before_the_novel() {
        let bytes = b"::startroom::startFSM->FSM_Send(start);player->c_pickupweapon(painkiller);MaxPayne_GraphicNovelMode->MPGNM_PickupNote(P1L0a_002);MPGNM_MarkAllNotesRead();MPGNM_PickupNote(P1L0a_003);MPGNM_PickupNote(P1L0a_004);MPGNM_PickupNote(P1L0a_005);x_modeswitch->s_modeswitch(graphicnovel);";
        assert_eq!(
            level_opening_pages(bytes),
            ["P1L0a_003", "P1L0a_004", "P1L0a_005"]
        );
    }

    #[test]
    fn level_opening_pages_ignores_a_handoff_and_a_later_note() {
        let handoff = b"startFSM mpgnm_pickupnote(P1L0a_001);s_modeswitch(graphicnovel);gm_init(part0_level1);";
        assert!(level_opening_pages(handoff).is_empty());
        let note = b"T_Activate mpgnm_pickupnote(P1L0a_007);s_modeswitch(graphicnovel);fsm_switch(normal);";
        assert!(level_opening_pages(note).is_empty());
    }

    #[test]
    fn house_opening_pages_when_the_level_is_present() {
        let path =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !path.is_file() {
            return;
        }
        let data = std::fs::read(path).unwrap();
        assert_eq!(
            level_opening_pages(&data),
            ["P1L0a_003", "P1L0a_004", "P1L0a_005"]
        );
    }

    #[test]
    fn resolve_startup_uses_intro_then_startup_level() {
        let dir = std::env::temp_dir().join(format!("omp-startup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data/database/levels/front_end")).unwrap();
        std::fs::create_dir_all(dir.join("movies")).unwrap();
        std::fs::write(
            dir.join("data/globalconfig.txt"),
            "[startup]\nStartupVideoName = \"movies\\intro.mpg\";\n[character]\nFallingThresholdVelocity = 1.0;\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("data/database/levels/levels.txt"),
            "[startup_level]\n{\n\tDirectory = front_end;\n\tLevel = startup_level.ldb;\n}\n[part0_level1]\n{\n\tDirectory = part1;\n\tLevel = part0_level1.ldb;\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("movies/intro.mpg"), b"mpg").unwrap();
        std::fs::write(
            dir.join("data/database/levels/front_end/startup_level.ldb"),
            b"LEVELBYTES",
        )
        .unwrap();
        let plan = resolve_startup(&dir).expect("plan");
        assert_eq!(
            plan.video.as_deref(),
            Some(dir.join("movies/intro.mpg").as_path())
        );
        assert_eq!(plan.level_bytes, b"LEVELBYTES");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_startup_reads_startup_level_from_x_data_ras() {
        let dir = std::env::temp_dir().join(format!("omp-startup-ras-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("movies")).unwrap();
        std::fs::write(dir.join("movies/intro.mpg"), b"mpg").unwrap();
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
                    name: "\\data\\".into(),
                    timestamp: [0; 16],
                },
                RasDirEntry {
                    name: "\\data\\database\\levels\\".into(),
                    timestamp: [0; 16],
                },
                RasDirEntry {
                    name: "\\data\\database\\levels\\front_end\\".into(),
                    timestamp: [0; 16],
                },
            ],
            files: vec![
                RasFileEntry {
                    name: "globalconfig.txt".into(),
                    dir_index: 1,
                    data: b"[startup]\nStartupVideoName = \"movies\\intro.mpg\";\n".to_vec(),
                    compression_mode: MODE_STORED,
                    compressed_data: None,
                    unknown_0: 0,
                    unknown_1: 0,
                    timestamp: [0; 16],
                },
                RasFileEntry {
                    name: "levels.txt".into(),
                    dir_index: 2,
                    data: b"[startup_level]\n{\nDirectory = front_end;\nLevel = startup_level.ldb;\n}\n"
                        .to_vec(),
                    compression_mode: MODE_STORED,
                    compressed_data: None,
                    unknown_0: 0,
                    unknown_1: 0,
                    timestamp: [0; 16],
                },
                RasFileEntry {
                    name: "startup_level.ldb".into(),
                    dir_index: 3,
                    data: b"FROMRAS".to_vec(),
                    compression_mode: MODE_STORED,
                    compressed_data: None,
                    unknown_0: 0,
                    unknown_1: 0,
                    timestamp: [0; 16],
                },
            ],
        };
        arc.write(&dir.join("x_data.ras"), false).unwrap();
        let plan = resolve_startup(&dir).expect("plan");
        assert_eq!(plan.level_bytes, b"FROMRAS");
        assert_eq!(
            plan.level_win_path,
            "\\data\\database\\levels\\front_end\\startup_level.ldb"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn main_menu_routes_tutorial_and_new_game() {
        let text = r#"
[Menu]
{
	[Item]
	{
		name = GCID_STARTTUTORIAL;
		[Message] String = "maxpayne_gamemode->gm_init( tutorial );";
	}
	[Menu]
	{
		Name = GCID_NEWGAME;
		[Item]
		{
			Name = GCID_NORMAL;
			[Message] String = "maxpayne_gamemode->gm_init( startup_level );";
		}
		[Back]
		Name = GCID_BACK;
	}
	[Item]
	{
		Name = GCID_QUIT;
	}
}
"#;
        let menu = parse_main_menu(text);
        assert_eq!(menu[0].label, "Start Tutorial");
        assert_eq!(menu[0].level_id.as_deref(), Some("tutorial"));
        assert_eq!(
            menu[1].children[0].level_id.as_deref(),
            Some("startup_level")
        );
        assert!(menu[2].level_id.is_none());
        assert!(!menu[2].quits);
    }

    #[test]
    fn first_menu_lists_every_top_entry_and_quit_loads_nothing() {
        let text = r#"
[Menu]
{
	[Item]
	{
		name = GCID_RESUMEGAME;
	}
	[Item]
	{
		name = GCID_STARTTUTORIAL;
		[Message] String = "maxpayne_gamemode->gm_init( tutorial );";
	}
	[Menu]
	{
		Name = GCID_NEWGAME;
		[Item]
		{
			Name = GCID_NORMAL;
			[Message] String = "maxpayne_gamemode->gm_init( startup_level );";
		}
	}
	[Menu]
	{
		Name = GCID_LOAD;
	}
	[Menu]
	{
		Name = GCID_SAVE;
	}
	[Menu]
	{
		Name = GCID_OPTIONS;
	}
	[Menu]
	{
		Name = GCID_CREDITS;
	}
	[Menu]
	{
		Name = GCID_QUIT;
		[Item]
		{
			name = GCID_QUIT_YES;
			[Message] String = "X_ModeSwitch->S_Quit();";
		}
	}
}
"#;
        let menu = parse_main_menu(text);
        let labels: Vec<&str> = menu.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Resume",
                "Start Tutorial",
                "New Game",
                "Load",
                "Save",
                "Options",
                "Credits",
                "Quit",
            ]
        );
        assert_eq!(menu[1].level_id.as_deref(), Some("tutorial"));
        assert_eq!(
            menu[2].children[0].level_id.as_deref(),
            Some("startup_level")
        );
        let yes = &menu[7].children[0];
        assert!(yes.quits);
        assert!(yes.level_id.is_none());
    }

    #[test]
    fn extracted_menu_txt_routes_tutorial_and_new_game() {
        let path = std::path::Path::new("/tmp/omp-xdata-ref/data/menu/menu.txt");
        if !path.is_file() {
            return;
        }
        let text = std::fs::read_to_string(path).unwrap();
        let menu = parse_main_menu(&text);
        let labels: Vec<&str> = menu.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Resume",
                "Start Tutorial",
                "New Game",
                "Load",
                "Save",
                "Options",
                "Credits",
                "Quit",
            ]
        );
        let yes = menu
            .iter()
            .find(|e| e.id == "GCID_QUIT")
            .and_then(|e| e.children.iter().find(|c| c.id == "GCID_QUIT_YES"))
            .unwrap();
        assert!(yes.quits);
        assert!(yes.level_id.is_none());
        let tutorial = menu.iter().find(|e| e.label == "Start Tutorial").unwrap();
        assert_eq!(tutorial.level_id.as_deref(), Some("tutorial"));
        let new_game = menu.iter().find(|e| e.label == "New Game").unwrap();
        assert!(new_game
            .children
            .iter()
            .any(|c| c.level_id.as_deref() == Some("startup_level")));
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
