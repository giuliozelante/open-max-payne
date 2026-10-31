//! Native ActorFX / Keyframe 2.0 (`.kf2`, `.kfs`, and `.skd`) chunk, mesh, and bone/material decoder.
//!
//! `R_MemoryFile::createChunk` writes `0x0c` + `id: u32` + `version: u32` + `size: u32`
//! (`size` includes the 13-byte header). `.kfs` / `.kf2` files contain sequential top-level
//! chunks (`0x0001000f` materials, `0x00010005` TriMesh geometry, `0x00010011` keyframe
//! animations) and `.skd` files open with chunk `0x00010014` (skin/bone weights).

use crate::scene::{decode_texture_blob, DecodedTexture};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kf2Chunk {
    pub offset: usize,
    pub id: u32,
    pub version: u32,
    pub size: u32,
    pub strings: Vec<String>,
    pub vec3_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kf2Summary {
    pub file_size: usize,
    pub chunks: Vec<Kf2Chunk>,
    pub all_strings: Vec<String>,
    pub total_vec3: usize,
}

#[derive(Debug, Clone)]
pub struct KfsFace {
    pub indices: [usize; 3],
    pub uvs: [[f32; 2]; 3],
    pub mat_index: usize,
}

#[derive(Debug, Clone)]
pub struct KfsMesh {
    pub name: String,
    /// Vertices in Max-FX Y-up character space (`y = 0` at feet, `y = ~1.85` at head).
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<KfsFace>,
    pub material_textures: Vec<Option<DecodedTexture>>,
}

fn read_i32(data: &[u8], i: &mut usize) -> Option<i32> {
    match *data.get(*i)? {
        0x14 => {
            let v = *data.get(*i + 1)? as i32;
            *i += 2;
            Some(v)
        }
        0x13 => {
            let v = u16::from_le_bytes([*data.get(*i + 1)?, *data.get(*i + 2)?]) as i32;
            *i += 3;
            Some(v)
        }
        0x12 => {
            let v = u32::from_le_bytes([
                *data.get(*i + 1)?,
                *data.get(*i + 2)?,
                *data.get(*i + 3)?,
                0,
            ]) as i32;
            *i += 4;
            Some(v)
        }
        0x02 => {
            let v = i32::from_le_bytes(data.get(*i + 1..*i + 5)?.try_into().ok()?);
            *i += 5;
            Some(v)
        }
        0x11 => {
            let v = *data.get(*i + 1)? as i32;
            *i += 2;
            Some(v)
        }
        _ => None,
    }
}

fn read_str(data: &[u8], i: &mut usize) -> Option<String> {
    if *data.get(*i)? != 0x0d {
        return None;
    }
    *i += 1;
    let len = read_i32(data, i)? as usize;
    let bytes = data.get(*i..*i + len)?;
    *i += len;
    Some(String::from_utf8_lossy(bytes).into_owned())
}

/// Parse a `.kfs` / `.kf2` character mesh (`0x0001000f` materials + `0x00010005` TriMesh)
/// and resolve its textures via `resolve_tex(filename_lower)`.
pub fn parse_kfs_mesh<F>(data: &[u8], mut resolve_tex: F) -> Option<KfsMesh>
where
    F: FnMut(&str) -> Option<DecodedTexture>,
{
    if data.len() < 26 || data[0] != 0x0c {
        return None;
    }
    let mut mat_map: BTreeMap<String, String> = BTreeMap::new();
    let mut off = 0usize;
    let mut mesh_name = String::new();
    let mut root_mat = [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let mut root_pos = [0.0f32; 3];
    let mut raw_verts: Vec<[f32; 3]> = Vec::new();
    let mut raw_faces: Vec<[usize; 3]> = Vec::new();
    let mut mat_names: Vec<String> = Vec::new();
    let mut face_mats: Vec<usize> = Vec::new();
    let mut uv_faces: Vec<[usize; 3]> = Vec::new();
    let mut uv_verts: Vec<[f32; 2]> = Vec::new();

    while off + 13 <= data.len() && data[off] == 0x0c {
        let cid = u32::from_le_bytes(data[off + 1..off + 5].try_into().ok()?);
        let sz = u32::from_le_bytes(data[off + 9..off + 13].try_into().ok()?) as usize;
        if sz < 13 || off + sz > data.len() {
            break;
        }
        if cid == 0x0001_000f {
            let mut i = off + 13;
            let _search_path = read_str(data, &mut i)?;
            let mcnt = read_i32(data, &mut i)? as usize;
            for _ in 0..mcnt {
                if i + 13 > off + sz || data[i] != 0x0c {
                    break;
                }
                let ssz = u32::from_le_bytes(data[i + 9..i + 13].try_into().ok()?) as usize;
                let mut p = i + 13;
                let mut strs = Vec::new();
                while p < i + ssz {
                    if data[p] == 0x0d && p + 2 < i + ssz && data[p + 1] == 0x14 {
                        if let Some(s) = read_str(data, &mut p) {
                            strs.push(s);
                            continue;
                        }
                    }
                    p += 1;
                }
                if let Some(mname) = strs.first() {
                    if let Some(tex) = strs.iter().skip(1).find(|s| {
                        let l = s.to_ascii_lowercase();
                        l.ends_with(".jpg") || l.ends_with(".tga") || l.ends_with(".pcx")
                    }) {
                        mat_map.insert(mname.to_ascii_lowercase(), tex.to_ascii_lowercase());
                    }
                }
                i += ssz;
            }
        } else if cid == 0x0001_0005 {
            let mut sub = off + 13;
            while sub + 13 <= off + sz && data[sub] == 0x0c {
                let scid = u32::from_le_bytes(data[sub + 1..sub + 5].try_into().ok()?);
                let ssz = u32::from_le_bytes(data[sub + 9..sub + 13].try_into().ok()?) as usize;
                if ssz < 13 || sub + ssz > off + sz {
                    break;
                }
                let mut p = sub + 13;
                match scid {
                    0x0001_0000 => {
                        mesh_name = read_str(data, &mut p)?;
                        let _parent = read_str(data, &mut p)?;
                        if data.get(p) == Some(&0x1a) && p + 49 <= sub + ssz {
                            for (k, m) in root_mat.iter_mut().enumerate() {
                                let o = p + 1 + k * 4;
                                *m = f32::from_le_bytes(data[o..o + 4].try_into().ok()?);
                            }
                            let o = p + 37;
                            root_pos = [
                                f32::from_le_bytes(data[o..o + 4].try_into().ok()?),
                                f32::from_le_bytes(data[o + 4..o + 8].try_into().ok()?),
                                f32::from_le_bytes(data[o + 8..o + 12].try_into().ok()?),
                            ];
                        }
                    }
                    0x0001_0006 => {
                        let vcnt = read_i32(data, &mut p)? as usize;
                        raw_verts.reserve(vcnt);
                        for _ in 0..vcnt {
                            if data.get(p) != Some(&0x16) || p + 13 > sub + ssz {
                                break;
                            }
                            let x = f32::from_le_bytes(data[p + 1..p + 5].try_into().ok()?);
                            let y = f32::from_le_bytes(data[p + 5..p + 9].try_into().ok()?);
                            let z = f32::from_le_bytes(data[p + 9..p + 13].try_into().ok()?);
                            p += 13;
                            raw_verts.push([x, y, z]);
                        }
                    }
                    0x0001_0007 => {
                        let fcnt = read_i32(data, &mut p)? as usize;
                        raw_faces.reserve(fcnt);
                        for _ in 0..fcnt {
                            if p + 13 > sub + ssz || data[p] != 0x0c {
                                break;
                            }
                            let fsz =
                                u32::from_le_bytes(data[p + 9..p + 13].try_into().ok()?) as usize;
                            let mut q = p + 13;
                            let _three = read_i32(data, &mut q)?;
                            let v0 = read_i32(data, &mut q)? as usize;
                            let v1 = read_i32(data, &mut q)? as usize;
                            let v2 = read_i32(data, &mut q)? as usize;
                            raw_faces.push([v0, v1, v2]);
                            p += fsz;
                        }
                    }
                    0x0001_000c => {
                        let mcnt = read_i32(data, &mut p)? as usize;
                        for _ in 0..mcnt {
                            mat_names.push(read_str(data, &mut p)?);
                        }
                        let fcnt = read_i32(data, &mut p)? as usize;
                        face_mats.reserve(fcnt);
                        for _ in 0..fcnt {
                            face_mats.push(read_i32(data, &mut p)? as usize);
                        }
                    }
                    0x0001_000e => {
                        let _ch = read_i32(data, &mut p)?;
                        let ufcnt = read_i32(data, &mut p)? as usize;
                        uv_faces.reserve(ufcnt);
                        for _ in 0..ufcnt {
                            if p + 13 > sub + ssz || data[p] != 0x0c {
                                break;
                            }
                            let fsz =
                                u32::from_le_bytes(data[p + 9..p + 13].try_into().ok()?) as usize;
                            let mut q = p + 13;
                            let _three = read_i32(data, &mut q)?;
                            let u0 = read_i32(data, &mut q)? as usize;
                            let u1 = read_i32(data, &mut q)? as usize;
                            let u2 = read_i32(data, &mut q)? as usize;
                            uv_faces.push([u0, u1, u2]);
                            p += fsz;
                        }
                        let _z0 = read_i32(data, &mut p)?;
                        let _z1 = read_i32(data, &mut p)?;
                        let uvcnt = read_i32(data, &mut p)? as usize;
                        uv_verts.reserve(uvcnt);
                        for _ in 0..uvcnt {
                            if data.get(p) != Some(&0x16) || p + 13 > sub + ssz {
                                break;
                            }
                            let u = f32::from_le_bytes(data[p + 1..p + 5].try_into().ok()?);
                            let v = f32::from_le_bytes(data[p + 5..p + 9].try_into().ok()?);
                            p += 13;
                            uv_verts.push([u, v]);
                        }
                    }
                    _ => {}
                }
                sub += ssz;
            }
        }
        off += sz;
    }

    if raw_verts.is_empty() || raw_faces.is_empty() {
        return None;
    }

    // Transform raw 3ds Max vertices into Max-FX Y-up character space and relax T-pose arms
    let mut vertices = Vec::with_capacity(raw_verts.len());
    for &[vx, vy, vz] in &raw_verts {
        let mut wx = root_mat[0] * vx + root_mat[3] * vy + root_mat[6] * vz + root_pos[0];
        let mut wy = root_mat[1] * vx + root_mat[4] * vy + root_mat[7] * vz + root_pos[1];
        let mut wz = root_mat[2] * vx + root_mat[5] * vy + root_mat[8] * vz + root_pos[2];

        // Relax horizontal T-pose arms (`|wx| > 0.17`, `wy in 1.15..1.60`) downward by 68 deg around shoulder
        if wx.abs() > 0.17 && (1.15..=1.60).contains(&wy) {
            let sign = wx.signum();
            let sx = sign * 0.17;
            let sy = 1.43;
            let dx = (wx - sx).abs();
            let dy = wy - sy;
            // Rotate arm downward around shoulder (dx -> -dy, dy -> dx)
            let angle = 1.18f32; // ~68 degrees down
            let (sa, ca) = angle.sin_cos();
            let rx = dx * ca + dy * sa;
            let ry = -dx * sa + dy * ca;
            wx = sx + sign * rx;
            wy = sy + ry;
            // Right hand (`wx > 0`) aims slightly forward (`-Z` in character space)
            if sign > 0.0 && dx > 0.18 {
                wz -= (dx - 0.18) * 0.65;
                wy += (dx - 0.18) * 0.35;
            }
        }
        vertices.push([wx, wy, wz]);
    }

    let mut material_textures = Vec::with_capacity(mat_names.len().max(1));
    if mat_names.is_empty() {
        material_textures.push(None);
    } else {
        for mname in &mat_names {
            let tex = mat_map
                .get(&mname.to_ascii_lowercase())
                .and_then(|tfile| resolve_tex(tfile));
            material_textures.push(tex);
        }
    }

    let mut faces = Vec::with_capacity(raw_faces.len());
    for (idx, &v_idx) in raw_faces.iter().enumerate() {
        if v_idx.iter().any(|&v| v >= vertices.len()) {
            continue;
        }
        let uvs = if let Some(&[u0, u1, u2]) = uv_faces.get(idx) {
            [
                uv_verts.get(u0).copied().unwrap_or([0.0, 0.0]),
                uv_verts.get(u1).copied().unwrap_or([0.0, 0.0]),
                uv_verts.get(u2).copied().unwrap_or([0.0, 0.0]),
            ]
        } else {
            [[0.0, 0.0]; 3]
        };
        let mat_index = face_mats
            .get(idx)
            .copied()
            .unwrap_or(0)
            .min(material_textures.len().saturating_sub(1));
        faces.push(KfsFace {
            indices: v_idx,
            uvs,
            mat_index,
        });
    }

    Some(KfsMesh {
        name: mesh_name,
        vertices,
        faces,
        material_textures,
    })
}

/// Load `max_payne` (`Payne_Max_L0.KFS`) and enemy (`doe_johnny_l0.kfs`) character meshes
/// from loose extracted files or `x_data.ras` near `level_or_game_path`.
pub fn load_character_meshes(level_or_game_path: &Path) -> (Option<KfsMesh>, Option<KfsMesh>) {
    // 1. Check extracted skins tree if present (`/tmp/omp-accept-11/xdata/data/database/skins` or `game_dir/data/database/skins`)
    let mut skin_dirs: Vec<PathBuf> = vec![PathBuf::from(
        "/tmp/omp-accept-11/xdata/data/database/skins",
    )];
    for anc in level_or_game_path.ancestors() {
        skin_dirs.push(anc.join("data/database/skins"));
        skin_dirs.push(anc.join("xdata/data/database/skins"));
    }

    for sdir in &skin_dirs {
        let max_kfs = sdir.join("max_payne/Payne_Max_L0.KFS");
        let johnny_kfs = sdir.join("C1_All_Johnny/doe_johnny_l0.kfs");
        if max_kfs.is_file() && johnny_kfs.is_file() {
            let max_mesh = std::fs::read(&max_kfs).ok().and_then(|bytes| {
                parse_kfs_mesh(&bytes, |tfile| {
                    load_tex_from_dirs(
                        tfile,
                        &[sdir.join("max_payne/textures"), sdir.join("sharedtextures")],
                    )
                })
            });
            let johnny_mesh = std::fs::read(&johnny_kfs).ok().and_then(|bytes| {
                parse_kfs_mesh(&bytes, |tfile| {
                    load_tex_from_dirs(tfile, &[sdir.join("sharedtextures")])
                })
            });
            if max_mesh.is_some() || johnny_mesh.is_some() {
                return (max_mesh, johnny_mesh);
            }
        }
    }

    // 2. Fallback: load directly from `x_data.ras` in ancestor or `~/Games/MaxPayne1/x_data.ras`
    let mut ras_candidates: Vec<PathBuf> = Vec::new();
    for anc in level_or_game_path.ancestors() {
        ras_candidates.push(anc.join("x_data.ras"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        ras_candidates.push(PathBuf::from(home).join("Games/MaxPayne1/x_data.ras"));
    }
    for ras_path in ras_candidates {
        if !ras_path.is_file() {
            continue;
        }
        if let Ok(arc) = openras::RasArchive::read(&ras_path) {
            let mut by_base: BTreeMap<String, usize> = BTreeMap::new();
            for (idx, f) in arc.files.iter().enumerate() {
                by_base.insert(f.name.to_ascii_lowercase(), idx);
            }
            let resolve = |tfile: &str| -> Option<DecodedTexture> {
                let idx = *by_base.get(&tfile.to_ascii_lowercase())?;
                let bytes = &arc.files[idx].data;
                let (width, height, pixels) = decode_texture_blob(bytes)?;
                Some(DecodedTexture {
                    path: tfile.to_string(),
                    flag: 4,
                    width,
                    height,
                    pixels,
                })
            };
            let max_mesh = by_base
                .get("payne_max_l0.kfs")
                .and_then(|&idx| parse_kfs_mesh(&arc.files[idx].data, resolve));
            let johnny_mesh = by_base
                .get("doe_johnny_l0.kfs")
                .and_then(|&idx| parse_kfs_mesh(&arc.files[idx].data, resolve));
            return (max_mesh, johnny_mesh);
        }
    }

    (None, None)
}

fn load_tex_from_dirs(tfile: &str, dirs: &[PathBuf]) -> Option<DecodedTexture> {
    let target = tfile.to_ascii_lowercase();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for ent in rd.flatten() {
            if ent.file_name().to_string_lossy().to_ascii_lowercase() == target {
                let bytes = std::fs::read(ent.path()).ok()?;
                let (width, height, pixels) = decode_texture_blob(&bytes)?;
                return Some(DecodedTexture {
                    path: tfile.to_string(),
                    flag: 4,
                    width,
                    height,
                    pixels,
                });
            }
        }
    }
    None
}

/// Walk top-level `0x0c` chunks (and nested strings/vec3 counts) in a `.kf2` or `.skd` buffer.
pub fn parse_kf2_or_skd(data: &[u8]) -> Option<Kf2Summary> {
    if data.len() < 13 || data[0] != 0x0c {
        return None;
    }
    let mut chunks = Vec::new();
    let mut all_strings = Vec::new();
    let mut total_vec3 = 0usize;
    let mut off = 0usize;

    while off + 13 <= data.len() && data[off] == 0x0c {
        let id = u32::from_le_bytes(data[off + 1..off + 5].try_into().ok()?);
        let version = u32::from_le_bytes(data[off + 5..off + 9].try_into().ok()?);
        let size = u32::from_le_bytes(data[off + 9..off + 13].try_into().ok()?);
        if (size as usize) < 13 || off + size as usize > data.len() {
            break;
        }
        let payload = &data[off + 13..off + size as usize];
        let mut strings = Vec::new();
        let mut vec3_count = 0usize;
        let mut i = 0usize;
        while i < payload.len() {
            if payload[i] == 0x0d && i + 2 < payload.len() && payload[i + 1] == 0x14 {
                let len = payload[i + 2] as usize;
                let start = i + 3;
                if (1..=96).contains(&len) && start + len <= payload.len() {
                    let bytes = &payload[start..start + len];
                    if bytes.iter().all(|&b| (0x20..=0x7e).contains(&b)) {
                        if let Ok(s) = String::from_utf8(bytes.to_vec()) {
                            strings.push(s.clone());
                            if all_strings.len() < 128 && !all_strings.contains(&s) {
                                all_strings.push(s);
                            }
                        }
                        i = start + len;
                        continue;
                    }
                }
            } else if payload[i] == 0x16 && i + 13 <= payload.len() {
                let x = f32::from_le_bytes(payload[i + 1..i + 5].try_into().unwrap_or([0; 4]));
                let y = f32::from_le_bytes(payload[i + 5..i + 9].try_into().unwrap_or([0; 4]));
                let z = f32::from_le_bytes(payload[i + 9..i + 13].try_into().unwrap_or([0; 4]));
                if x.is_finite()
                    && y.is_finite()
                    && z.is_finite()
                    && x.abs() < 1e5
                    && y.abs() < 1e5
                    && z.abs() < 1e5
                {
                    vec3_count += 1;
                    total_vec3 += 1;
                    i += 13;
                    continue;
                }
            }
            i += 1;
        }
        chunks.push(Kf2Chunk {
            offset: off,
            id,
            version,
            size,
            strings,
            vec3_count,
        });
        off += size as usize;
    }

    if chunks.is_empty() {
        None
    } else {
        Some(Kf2Summary {
            file_size: data.len(),
            chunks,
            all_strings,
            total_vec3,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synthetic_kf2_chunk() {
        let mut buf = Vec::new();
        buf.push(0x0c);
        buf.extend_from_slice(&0x0001_000fu32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        let payload = [0x0d, 0x14, 6, b'P', b'e', b'l', b'v', b'i', b's'];
        let size = (13 + payload.len()) as u32;
        buf.extend_from_slice(&size.to_le_bytes());
        buf.extend_from_slice(&payload);
        let sum = parse_kf2_or_skd(&buf).unwrap();
        assert_eq!(sum.chunks.len(), 1);
        assert_eq!(sum.all_strings, vec!["Pelvis"]);
    }
}
