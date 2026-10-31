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

/// One SKD influence (`P_SkinMesh` allows at most two — `TooManyBonesPerVertex`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneWeight {
    pub bone: usize,
    pub weight: f32,
}

/// Parsed `.skd` (`0x00010014`): bone names + ≤2 weights per vertex.
#[derive(Debug, Clone, PartialEq)]
pub struct SkdSkin {
    pub name: String,
    pub bones: Vec<String>,
    /// `weights[v]` is 1–2 influences. Empty vec = unskinned (keep bind pose).
    pub weights: Vec<Vec<BoneWeight>>,
}

/// One parent-relative `0x1a` keyframe inside a `0x00010012` bone channel.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimKeyframe {
    pub frame: u32,
    pub mat: [f32; 9],
    pub pos: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimBone {
    pub name: String,
    pub parent: Option<String>,
    pub duration: u32,
    pub keys: Vec<AnimKeyframe>,
}

/// Skeletal clip: one `0x00010012` ver-5 chunk per bone (`idle_aim.kf2`, `Walk.kf2`).
#[derive(Debug, Clone, PartialEq)]
pub struct Kf2Anim {
    pub bones: Vec<AnimBone>,
}

/// KFS + optional SKD weights + idle/walk clips for the walkthrough.
#[derive(Debug, Clone)]
pub struct CharacterRig {
    pub mesh: KfsMesh,
    pub skin: Option<SkdSkin>,
    pub idle: Option<Kf2Anim>,
    pub walk: Option<Kf2Anim>,
    /// `Pose.kf2` / `Widepose.kf2` — bind-pose locals for `inv(bind) * pose`.
    pub bind: Option<Kf2Anim>,
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

    // Transform raw 3ds Max vertices into Max-FX Y-up character space.
    // Arm pose comes from SKD weights + KF2 clips (`idle_aim` / `Walk`), not a mesh hack.
    let mut vertices = Vec::with_capacity(raw_verts.len());
    for &[vx, vy, vz] in &raw_verts {
        let wx = root_mat[0] * vx + root_mat[3] * vy + root_mat[6] * vz + root_pos[0];
        let wy = root_mat[1] * vx + root_mat[4] * vy + root_mat[7] * vz + root_pos[1];
        let wz = root_mat[2] * vx + root_mat[5] * vy + root_mat[8] * vz + root_pos[2];
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

fn apply_weight_list(weights: &mut [Vec<BoneWeight>], recs: &[(usize, usize, f32)]) {
    for &(v, b, w) in recs {
        if v < weights.len() && w.is_finite() && w > 0.0 {
            weights[v].push(BoneWeight { bone: b, weight: w });
        }
    }
}

fn read_weight_recs(data: &[u8], i: &mut usize, n: usize) -> Option<Vec<(usize, usize, f32)>> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let v = read_i32(data, i)? as usize;
        let b = read_i32(data, i)? as usize;
        if *data.get(*i)? != 0x09 || *i + 5 > data.len() {
            return None;
        }
        let w = f32::from_le_bytes(data[*i + 1..*i + 5].try_into().ok()?);
        *i += 5;
        out.push((v, b, w));
    }
    Some(out)
}

/// Parse an `.skd` skin (`0x00010014`): rigid 1-bone list, optional dual-influence
/// blend lists, bone names. Vertex count is taken from the first weight index range.
pub fn parse_skd(data: &[u8]) -> Option<SkdSkin> {
    if data.len() < 13 || data[0] != 0x0c {
        return None;
    }
    let cid = u32::from_le_bytes(data[1..5].try_into().ok()?);
    if cid != 0x0001_0014 {
        return None;
    }
    let sz = u32::from_le_bytes(data[9..13].try_into().ok()?) as usize;
    if sz < 13 || sz > data.len() {
        return None;
    }
    let payload = &data[13..sz];
    let mut i = 0usize;
    let n_rigid = read_i32(payload, &mut i)? as usize;
    if n_rigid > 200_000 {
        return None;
    }
    let rigid = read_weight_recs(payload, &mut i, n_rigid)?;
    let n_blend = read_i32(payload, &mut i)? as usize;
    if n_blend > 200_000 {
        return None;
    }
    let blend0 = read_weight_recs(payload, &mut i, n_blend)?;
    let mut blend1 = Vec::new();
    if n_blend > 0 {
        // Second influence list: either `n_blend` again or a bare run of n_blend recs.
        let mark = i;
        if let Some(n2) = read_i32(payload, &mut i) {
            if n2 as usize == n_blend {
                if let Some(recs) = read_weight_recs(payload, &mut i, n_blend) {
                    blend1 = recs;
                } else {
                    i = mark;
                }
            } else {
                i = mark;
                if let Some(recs) = read_weight_recs(payload, &mut i, n_blend) {
                    blend1 = recs;
                } else {
                    i = mark;
                }
            }
        }
    }
    // Skip to mesh name.
    while i < payload.len() && payload[i] != 0x0d {
        i += 1;
    }
    let name = read_str(payload, &mut i).unwrap_or_default();
    let n_bones = read_i32(payload, &mut i)? as usize;
    if n_bones == 0 || n_bones > 256 {
        return None;
    }
    let mut bones = Vec::with_capacity(n_bones);
    for _ in 0..n_bones {
        bones.push(read_str(payload, &mut i)?);
    }
    let n_hint = rigid
        .iter()
        .chain(blend0.iter())
        .chain(blend1.iter())
        .map(|r| r.0)
        .max()
        .unwrap_or(0)
        + 1;
    let mut weights = vec![Vec::new(); n_hint];
    apply_weight_list(&mut weights, &rigid);
    apply_weight_list(&mut weights, &blend0);
    apply_weight_list(&mut weights, &blend1);
    Some(SkdSkin {
        name,
        bones,
        weights,
    })
}

fn read_0x1a(data: &[u8], i: &mut usize) -> Option<([f32; 9], [f32; 3])> {
    if data.get(*i) != Some(&0x1a) || *i + 49 > data.len() {
        return None;
    }
    *i += 1;
    let mut mat = [0f32; 9];
    for slot in &mut mat {
        *slot = f32::from_le_bytes(data.get(*i..*i + 4)?.try_into().ok()?);
        *i += 4;
    }
    let pos = [
        f32::from_le_bytes(data.get(*i..*i + 4)?.try_into().ok()?),
        f32::from_le_bytes(data.get(*i + 4..*i + 8)?.try_into().ok()?),
        f32::from_le_bytes(data.get(*i + 8..*i + 12)?.try_into().ok()?),
    ];
    *i += 12;
    Some((mat, pos))
}

/// Parse a skeletal KF2 (`0x00010012` ver 5 channels).
pub fn parse_kf2_anim(data: &[u8]) -> Option<Kf2Anim> {
    if data.len() < 13 || data[0] != 0x0c {
        return None;
    }
    let mut bones = Vec::new();
    let mut off = 0usize;
    while off + 13 <= data.len() && data[off] == 0x0c {
        let cid = u32::from_le_bytes(data[off + 1..off + 5].try_into().ok()?);
        let sz = u32::from_le_bytes(data[off + 9..off + 13].try_into().ok()?) as usize;
        if sz < 13 || off + sz > data.len() {
            break;
        }
        if cid == 0x0001_0012 {
            let payload = &data[off + 13..off + sz];
            let mut i = 0usize;
            let mut name = String::new();
            if payload.first() == Some(&0x0c) && payload.len() >= 13 {
                let ssz = u32::from_le_bytes(payload[9..13].try_into().ok()?) as usize;
                if ssz >= 13 && ssz <= payload.len() {
                    let mut p = 13usize;
                    name = read_str(&payload[..ssz], &mut p).unwrap_or_default();
                    i = ssz;
                }
            }
            let parent = match read_str(payload, &mut i) {
                Some(s) if !s.is_empty() => Some(s),
                _ => None,
            };
            if payload.get(i) == Some(&0x0e) {
                i += 2;
            }
            let duration = read_i32(payload, &mut i).unwrap_or(0) as u32;
            let nkeys = read_i32(payload, &mut i).unwrap_or(0) as usize;
            let mut keys = Vec::new();
            for _ in 0..nkeys {
                let frame = read_i32(payload, &mut i).unwrap_or(0) as u32;
                let Some((mat, pos)) = read_0x1a(payload, &mut i) else {
                    break;
                };
                keys.push(AnimKeyframe { frame, mat, pos });
            }
            if !name.is_empty() && !keys.is_empty() {
                bones.push(AnimBone {
                    name,
                    parent,
                    duration,
                    keys,
                });
            }
        }
        off += sz;
    }
    if bones.is_empty() {
        None
    } else {
        Some(Kf2Anim { bones })
    }
}

fn identity_mat() -> [f32; 9] {
    [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Spherical blend of two column-major rotation matrices.
fn slerp_mat(a: [f32; 9], b: [f32; 9], t: f32) -> [f32; 9] {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    quat_to_mat(quat_slerp(mat_to_quat(a), mat_to_quat(b), t))
}

fn mat_to_quat(m: [f32; 9]) -> [f32; 4] {
    let (m00, m10, m20) = (m[0], m[1], m[2]);
    let (m01, m11, m21) = (m[3], m[4], m[5]);
    let (m02, m12, m22) = (m[6], m[7], m[8]);
    let trace = m00 + m11 + m22;
    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s]
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s]
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s]
    }
}

fn quat_slerp(a: [f32; 4], mut b: [f32; 4], t: f32) -> [f32; 4] {
    let mut dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if dot < 0.0 {
        b = [-b[0], -b[1], -b[2], -b[3]];
        dot = -dot;
    }
    if dot > 0.9995 {
        let r = [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
            a[3] + (b[3] - a[3]) * t,
        ];
        return quat_norm(r);
    }
    let theta = dot.clamp(-1.0, 1.0).acos();
    let sin_theta = theta.sin();
    let w1 = ((1.0 - t) * theta).sin() / sin_theta;
    let w2 = (t * theta).sin() / sin_theta;
    [
        a[0] * w1 + b[0] * w2,
        a[1] * w1 + b[1] * w2,
        a[2] * w1 + b[2] * w2,
        a[3] * w1 + b[3] * w2,
    ]
}

fn quat_norm(q: [f32; 4]) -> [f32; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1.0e-8 {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
    }
}

fn quat_to_mat(q: [f32; 4]) -> [f32; 9] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (x * y, x * z, y * z);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    [
        1.0 - 2.0 * (yy + zz),
        2.0 * (xy + wz),
        2.0 * (xz - wy),
        2.0 * (xy - wz),
        1.0 - 2.0 * (xx + zz),
        2.0 * (yz + wx),
        2.0 * (xz + wy),
        2.0 * (yz - wx),
        1.0 - 2.0 * (xx + yy),
    ]
}

fn mul_rt(rp: [f32; 9], tp: [f32; 3], rl: [f32; 9], tl: [f32; 3]) -> ([f32; 9], [f32; 3]) {
    let mut r = [0f32; 9];
    for j in 0..3 {
        for i in 0..3 {
            r[i + 3 * j] =
                rp[i] * rl[3 * j] + rp[i + 3] * rl[1 + 3 * j] + rp[i + 6] * rl[2 + 3 * j];
        }
    }
    let t = [
        rp[0] * tl[0] + rp[3] * tl[1] + rp[6] * tl[2] + tp[0],
        rp[1] * tl[0] + rp[4] * tl[1] + rp[7] * tl[2] + tp[1],
        rp[2] * tl[0] + rp[5] * tl[1] + rp[8] * tl[2] + tp[2],
    ];
    (r, t)
}

fn xform(mat: [f32; 9], pos: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    [
        mat[0] * v[0] + mat[3] * v[1] + mat[6] * v[2] + pos[0],
        mat[1] * v[0] + mat[4] * v[1] + mat[7] * v[2] + pos[1],
        mat[2] * v[0] + mat[5] * v[1] + mat[8] * v[2] + pos[2],
    ]
}

fn transpose3(m: [f32; 9]) -> [f32; 9] {
    [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]]
}

fn invert_rt(r: [f32; 9], t: [f32; 3]) -> ([f32; 9], [f32; 3]) {
    let rt = transpose3(r);
    let tinv = [
        -(rt[0] * t[0] + rt[3] * t[1] + rt[6] * t[2]),
        -(rt[1] * t[0] + rt[4] * t[1] + rt[7] * t[2]),
        -(rt[2] * t[0] + rt[5] * t[1] + rt[8] * t[2]),
    ];
    (rt, tinv)
}

/// Sample parent-relative locals at `time_s` (30 fps, loops on each bone's duration).
pub fn sample_locals(anim: &Kf2Anim, time_s: f32) -> BTreeMap<String, ([f32; 9], [f32; 3])> {
    let mut out = BTreeMap::new();
    for bone in &anim.bones {
        if bone.keys.is_empty() {
            continue;
        }
        let dur = bone.duration.max(1) as f32;
        let frame = (time_s.max(0.0) * 30.0) % dur;
        let mut i = 0usize;
        while i + 1 < bone.keys.len() && bone.keys[i + 1].frame as f32 <= frame {
            i += 1;
        }
        let a = &bone.keys[i];
        let (mat, pos) = if i + 1 >= bone.keys.len() || (a.frame as f32) > frame {
            (a.mat, a.pos)
        } else {
            let b = &bone.keys[i + 1];
            let span = (b.frame - a.frame) as f32;
            let t = if span <= 0.0 {
                0.0
            } else {
                ((frame - a.frame as f32) / span).clamp(0.0, 1.0)
            };
            (slerp_mat(a.mat, b.mat, t), lerp3(a.pos, b.pos, t))
        };
        out.insert(bone.name.to_ascii_lowercase(), (mat, pos));
    }
    out
}

/// Compose model-space bone matrices from parent-relative locals.
pub fn compose_world(
    anim: &Kf2Anim,
    locals: &BTreeMap<String, ([f32; 9], [f32; 3])>,
) -> BTreeMap<String, ([f32; 9], [f32; 3])> {
    let mut world = BTreeMap::new();
    let mut guard = 0usize;
    while world.len() < anim.bones.len() && guard < anim.bones.len() + 2 {
        guard += 1;
        for bone in &anim.bones {
            let key = bone.name.to_ascii_lowercase();
            if world.contains_key(&key) {
                continue;
            }
            let (lm, lp) = locals
                .get(&key)
                .copied()
                .unwrap_or_else(|| (identity_mat(), [0.0; 3]));
            match &bone.parent {
                None => {
                    world.insert(key, (lm, lp));
                }
                Some(p) => {
                    let pk = p.to_ascii_lowercase();
                    if let Some(&(pm, pp)) = world.get(&pk) {
                        world.insert(key, mul_rt(pm, pp, lm, lp));
                    }
                }
            }
        }
    }
    world
}

/// Linear-blend skin: `Σ w_i * pose_i * inv(bind_i) * v` (`P_SkinMesh`).
pub fn skin_vertices(
    verts: &[[f32; 3]],
    skin: &SkdSkin,
    bind_world: &BTreeMap<String, ([f32; 9], [f32; 3])>,
    pose_world: &BTreeMap<String, ([f32; 9], [f32; 3])>,
) -> Vec<[f32; 3]> {
    let n_bones = skin.bones.len();
    let mut skin_rt = vec![(identity_mat(), [0.0f32; 3]); n_bones];
    for (i, name) in skin.bones.iter().enumerate() {
        let key = name.to_ascii_lowercase();
        let bind = bind_world
            .get(&key)
            .copied()
            .unwrap_or_else(|| (identity_mat(), [0.0; 3]));
        let pose = pose_world.get(&key).copied().unwrap_or(bind);
        let (bi, bti) = invert_rt(bind.0, bind.1);
        skin_rt[i] = mul_rt(pose.0, pose.1, bi, bti);
    }
    let mut out = Vec::with_capacity(verts.len());
    for (vi, &v) in verts.iter().enumerate() {
        let infl = skin.weights.get(vi).map(|w| w.as_slice()).unwrap_or(&[]);
        if infl.is_empty() {
            out.push(v);
            continue;
        }
        let mut acc = [0.0f32; 3];
        let mut wsum = 0.0f32;
        for inf in infl {
            if inf.bone >= n_bones {
                continue;
            }
            let (rm, tp) = skin_rt[inf.bone];
            let p = xform(rm, tp, v);
            acc[0] += p[0] * inf.weight;
            acc[1] += p[1] * inf.weight;
            acc[2] += p[2] * inf.weight;
            wsum += inf.weight;
        }
        if wsum > 1e-6 {
            out.push([acc[0] / wsum, acc[1] / wsum, acc[2] / wsum]);
        } else {
            out.push(v);
        }
    }
    out
}

/// Pose `mesh.vertices` with `anim` at `time_s`. `bind` is `Pose.kf2` when present.
pub fn pose_mesh(
    mesh: &KfsMesh,
    skin: Option<&SkdSkin>,
    bind: Option<&Kf2Anim>,
    anim: Option<&Kf2Anim>,
    time_s: f32,
) -> Vec<[f32; 3]> {
    let (Some(skin), Some(anim)) = (skin, anim) else {
        return mesh.vertices.clone();
    };
    let pose_world = compose_world(anim, &sample_locals(anim, time_s));
    let bind_world = if let Some(bind) = bind {
        compose_world(bind, &sample_locals(bind, 0.0))
    } else {
        BTreeMap::new()
    };
    skin_vertices(&mesh.vertices, skin, &bind_world, &pose_world)
}

fn find_file_ci(dirs: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    let targets: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for ent in rd.flatten() {
            let n = ent.file_name().to_string_lossy().to_ascii_lowercase();
            if targets.iter().any(|t| t == &n) {
                return Some(ent.path());
            }
        }
    }
    None
}

pub(crate) fn load_bytes_from_ras(
    level_or_game_path: &Path,
    basenames: &[&str],
) -> Option<Vec<u8>> {
    let mut ras_candidates: Vec<PathBuf> = Vec::new();
    for anc in level_or_game_path.ancestors() {
        ras_candidates.push(anc.join("x_data.ras"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        ras_candidates.push(PathBuf::from(home).join("Games/MaxPayne1/x_data.ras"));
    }
    let wants: Vec<String> = basenames.iter().map(|n| n.to_ascii_lowercase()).collect();
    for ras_path in ras_candidates {
        if !ras_path.is_file() {
            continue;
        }
        if let Ok(arc) = openras::RasArchive::read(&ras_path) {
            for f in &arc.files {
                if wants.iter().any(|w| f.name.to_ascii_lowercase() == *w) {
                    return Some(f.data.clone());
                }
            }
        }
    }
    None
}

fn load_anim_named(level_or_game_path: &Path, names: &[&str]) -> Option<Kf2Anim> {
    let mut dirs = vec![PathBuf::from(
        "/tmp/omp-xdata-ref/data/database/skeletons/default_skeleton/anim",
    )];
    for anc in level_or_game_path.ancestors() {
        dirs.push(anc.join("data/database/skeletons/default_skeleton/anim"));
        dirs.push(anc.join("xdata/data/database/skeletons/default_skeleton/anim"));
    }
    if let Some(p) = find_file_ci(&dirs, names) {
        if let Ok(bytes) = std::fs::read(p) {
            if let Some(anim) = parse_kf2_anim(&bytes) {
                return Some(anim);
            }
        }
    }
    load_bytes_from_ras(level_or_game_path, names).and_then(|b| parse_kf2_anim(&b))
}

fn load_skd_near(level_or_game_path: &Path, names: &[&str]) -> Option<SkdSkin> {
    let mut dirs = vec![PathBuf::from(
        "/tmp/omp-xdata-ref/data/database/skins/max_payne",
    )];
    dirs.push(PathBuf::from(
        "/tmp/omp-xdata-ref/data/database/skins/C1_All_Johnny",
    ));
    for anc in level_or_game_path.ancestors() {
        dirs.push(anc.join("data/database/skins/max_payne"));
        dirs.push(anc.join("data/database/skins/C1_All_Johnny"));
        dirs.push(anc.join("xdata/data/database/skins/max_payne"));
        dirs.push(anc.join("xdata/data/database/skins/C1_All_Johnny"));
    }
    if let Some(p) = find_file_ci(&dirs, names) {
        if let Ok(bytes) = std::fs::read(p) {
            if let Some(sk) = parse_skd(&bytes) {
                return Some(sk);
            }
        }
    }
    load_bytes_from_ras(level_or_game_path, names).and_then(|b| parse_skd(&b))
}

/// `max_payne.txt` rest clip (`CHARANIM_STAND`).
pub fn player_rest_anim_names() -> [&'static str; 1] {
    ["stand.kf2"]
}

/// `max_payne.txt` reverts the walk slot onto `CHARANIM_RUN`.
pub fn player_move_anim_names() -> [&'static str; 1] {
    ["run.kf2"]
}

fn attach_rig(mesh: KfsMesh, skd_names: &[&str], path: &Path) -> CharacterRig {
    rig_from_mesh(mesh, load_skd_near(path, skd_names), path)
}

fn rig_from_mesh(mesh: KfsMesh, skin: Option<SkdSkin>, path: &Path) -> CharacterRig {
    CharacterRig {
        mesh,
        skin,
        idle: load_anim_named(path, &player_rest_anim_names()),
        walk: load_anim_named(path, &player_move_anim_names()),
        bind: load_anim_named(path, &["pose.kf2", "widepose.kf2"]),
    }
}

/// First `[Geometry]` pair in a skin `.txt` (`ExportData` / `SkinData`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinLodFiles {
    pub kfs: String,
    pub skd: String,
}

pub fn skin_lod_files(text: &str) -> Option<SkinLodFiles> {
    for line in text.lines() {
        if let (Some(kfs), Some(skd)) = (lod_field(line, "ExportData"), lod_field(line, "SkinData"))
        {
            return Some(SkinLodFiles { kfs, skd });
        }
    }
    None
}

/// `C_SetInvulnerable` in the skin's OnInit (the baby in the house).
pub fn skin_blocks_projectiles(text: &str) -> bool {
    text.to_ascii_lowercase().contains("c_setinvulnerable")
}

fn lod_field(line: &str, key: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let key_l = key.to_ascii_lowercase();
    let idx = lower.find(&key_l)?;
    let rest = line[idx + key.len()..].trim_start();
    let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
    let end = rest.find(';').unwrap_or(rest.len());
    let v = rest[..end].trim().trim_matches('"').replace('\\', "/");
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// A skin `.txt` plus the mesh it names. `invulnerable` comes from that file's OnInit.
pub struct SkinBody {
    pub rig: CharacterRig,
    pub invulnerable: bool,
}

fn skin_roots(level_or_game_path: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/tmp/omp-xdata-ref/data/database/skins")];
    for anc in level_or_game_path.ancestors() {
        dirs.push(anc.join("data/database/skins"));
        dirs.push(anc.join("xdata/data/database/skins"));
    }
    dirs
}

fn read_skin_bytes(txt_dir: Option<&Path>, rel: &str, game: &Path) -> Option<Vec<u8>> {
    if let Some(dir) = txt_dir {
        let joined = dir.join(rel);
        if let Ok(bytes) = std::fs::read(&joined) {
            return Some(bytes);
        }
    }
    let base = Path::new(rel).file_name()?.to_str()?;
    if let Some(dir) = txt_dir {
        if let Ok(bytes) = std::fs::read(dir.join(base)) {
            return Some(bytes);
        }
    }
    load_bytes_from_ras(game, &[base])
}

fn texture_dirs_for(txt_dir: Option<&Path>, kfs_rel: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = txt_dir {
        if let Some(parent) = dir.join(kfs_rel).parent() {
            dirs.push(parent.join("textures"));
            if let Some(skins) = parent.parent() {
                dirs.push(skins.join("sharedtextures"));
            }
        }
        dirs.push(dir.join("textures"));
        if let Some(skins) = dir.parent() {
            dirs.push(skins.join("sharedtextures"));
        }
    }
    dirs
}

/// Load `database/skins/<skin_id>.txt` and the KFS/SKD its first geometry line names.
pub fn load_skin_body(level_or_game_path: &Path, skin_id: &str) -> Option<SkinBody> {
    if skin_id.is_empty() {
        return None;
    }
    let txt_name = format!("{skin_id}.txt");
    let loose = find_file_ci(&skin_roots(level_or_game_path), &[&txt_name]);
    let (txt_dir, text) = if let Some(path) = loose {
        let text = std::fs::read_to_string(&path).ok()?;
        (path.parent().map(Path::to_path_buf), text)
    } else {
        let bytes = load_bytes_from_ras(level_or_game_path, &[&txt_name])?;
        (None, String::from_utf8_lossy(&bytes).into_owned())
    };
    let lod = skin_lod_files(&text)?;
    let kfs = read_skin_bytes(txt_dir.as_deref(), &lod.kfs, level_or_game_path)?;
    let skd = read_skin_bytes(txt_dir.as_deref(), &lod.skd, level_or_game_path)
        .and_then(|bytes| parse_skd(&bytes));
    let tex_dirs = texture_dirs_for(txt_dir.as_deref(), &lod.kfs);
    let mesh = parse_kfs_mesh(&kfs, |tfile| {
        load_tex_from_dirs(tfile, &tex_dirs).or_else(|| {
            let bytes = load_bytes_from_ras(level_or_game_path, &[tfile])?;
            let (width, height, pixels) = decode_texture_blob(&bytes)?;
            Some(DecodedTexture {
                path: tfile.to_string(),
                flag: 4,
                width,
                height,
                pixels,
            })
        })
    })?;
    Some(SkinBody {
        rig: rig_from_mesh(mesh, skd, level_or_game_path),
        invulnerable: skin_blocks_projectiles(&text),
    })
}

/// Load Max + Johnny as skinned rigs (KF2 idle/walk when BYOD `x_data` is present).
pub fn load_character_rigs(
    level_or_game_path: &Path,
) -> (Option<CharacterRig>, Option<CharacterRig>) {
    let (max_m, johnny_m) = load_character_meshes(level_or_game_path);
    (
        max_m.map(|m| attach_rig(m, &["payne_max_l0.skd"], level_or_game_path)),
        johnny_m.map(|m| attach_rig(m, &["doe_johnny_l0.skd"], level_or_game_path)),
    )
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
    fn skin_lod_reads_the_geometry_line() {
        let young = "[LOD]\n{\n\t[Geometry] ExportData = young_max_payne_l0.kfs; SkinData = young_max_payne_l0.skd;\n";
        let got = skin_lod_files(young).unwrap();
        assert_eq!(got.kfs, "young_max_payne_l0.kfs");
        assert_eq!(got.skd, "young_max_payne_l0.skd");

        let staged = "[Geometry] ExportData = ..\\young_max_payne\\young_max_payne_l0.kfs; SkinData = ..\\young_max_payne\\young_max_payne_l0.skd;";
        let got = skin_lod_files(staged).unwrap();
        assert_eq!(got.kfs, "../young_max_payne/young_max_payne_l0.kfs");
        assert_eq!(got.skd, "../young_max_payne/young_max_payne_l0.skd");

        assert!(skin_blocks_projectiles(
            r#"this->C_SetInvulnerable ( TRUE );"#
        ));
        assert!(!skin_blocks_projectiles("this->c_sethealth( 1 );"));
    }

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

    fn push_str(buf: &mut Vec<u8>, s: &str) {
        buf.push(0x0d);
        buf.push(0x14);
        buf.push(s.len() as u8);
        buf.extend_from_slice(s.as_bytes());
    }

    fn push_u8(buf: &mut Vec<u8>, v: u8) {
        buf.push(0x14);
        buf.push(v);
    }

    fn push_f32(buf: &mut Vec<u8>, v: f32) {
        buf.push(0x09);
        buf.extend_from_slice(&v.to_le_bytes());
    }

    fn push_chunk(buf: &mut Vec<u8>, id: u32, ver: u32, payload: &[u8]) {
        buf.push(0x0c);
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&ver.to_le_bytes());
        let sz = 13 + payload.len();
        buf.extend_from_slice(&(sz as u32).to_le_bytes());
        buf.extend_from_slice(payload);
    }

    #[test]
    fn parses_synthetic_kf2_anim_and_skins_one_bone() {
        // 0x00010013 name chunk
        let mut name_pl = Vec::new();
        push_str(&mut name_pl, "Pelvis");
        let mut name_ch = Vec::new();
        push_chunk(&mut name_ch, 0x0001_0013, 0, &name_pl);

        let mut ch = name_ch;
        push_str(&mut ch, ""); // root
        ch.extend_from_slice(&[0x0e, 0x01]);
        push_u8(&mut ch, 30); // duration
        push_u8(&mut ch, 1); // nkeys
        push_u8(&mut ch, 0); // frame 0
        ch.push(0x1a);
        // 90° around Y, column-major: [0,0,-1, 0,1,0, 1,0,0]
        for v in [0.0f32, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0] {
            ch.extend_from_slice(&v.to_le_bytes());
        }
        for v in [0.0f32, 0.0, 0.0] {
            ch.extend_from_slice(&v.to_le_bytes());
        }

        let mut anim_buf = Vec::new();
        push_chunk(&mut anim_buf, 0x0001_0012, 5, &ch);
        let anim = parse_kf2_anim(&anim_buf).expect("anim");
        assert_eq!(anim.bones.len(), 1);
        assert_eq!(anim.bones[0].name, "Pelvis");
        assert_eq!(anim.bones[0].keys.len(), 1);

        // SKD: 1 rigid vert → bone 0, name Test / Pelvis
        let mut skd_pl = Vec::new();
        push_u8(&mut skd_pl, 1); // n_rigid
        push_u8(&mut skd_pl, 0); // vert
        push_u8(&mut skd_pl, 0); // bone
        push_f32(&mut skd_pl, 1.0);
        push_u8(&mut skd_pl, 0); // n_blend
        push_u8(&mut skd_pl, 1);
        push_str(&mut skd_pl, "Test");
        push_u8(&mut skd_pl, 1);
        push_str(&mut skd_pl, "Pelvis");
        let mut skd = Vec::new();
        push_chunk(&mut skd, 0x0001_0014, 0, &skd_pl);
        let skin = parse_skd(&skd).expect("skd");
        assert_eq!(skin.bones, vec!["Pelvis".to_string()]);
        assert_eq!(skin.weights[0][0].bone, 0);

        let verts = vec![[1.0, 0.0, 0.0]];
        let locals = sample_locals(&anim, 0.0);
        let pose_world = compose_world(&anim, &locals);
        let bind_world = BTreeMap::new(); // identity bind
        let posed = skin_vertices(&verts, &skin, &bind_world, &pose_world);
        // Ry(90°) * (1,0,0) = (0,0,-1)
        assert!((posed[0][0]).abs() < 1e-4);
        assert!((posed[0][1]).abs() < 1e-4);
        assert!((posed[0][2] + 1.0).abs() < 1e-4);
    }

    #[test]
    fn halfway_between_identity_and_yaw_90_is_yaw_45() {
        let yaw90 = [0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0];
        let mid = slerp_mat(identity_mat(), yaw90, 0.5);
        let c = (std::f32::consts::FRAC_PI_4).cos();
        assert!((mid[0] - c).abs() < 1e-4);
        assert!(mid[1].abs() < 1e-4);
        assert!((mid[2] + c).abs() < 1e-4);
        assert!((mid[6] - c).abs() < 1e-4);
        assert!((mid[8] - c).abs() < 1e-4);
        let len = (mid[0] * mid[0] + mid[1] * mid[1] + mid[2] * mid[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-4);
    }

    #[test]
    fn player_uses_stand_and_run_clips() {
        assert_eq!(player_rest_anim_names(), ["stand.kf2"]);
        assert_eq!(player_move_anim_names(), ["run.kf2"]);
    }

    #[test]
    fn byod_max_skd_and_idle_aim_when_extracted() {
        let skd_p = Path::new("/tmp/omp-xdata-ref/data/database/skins/max_payne/PAYNE_MAX_L0.SKD");
        let anim_p = Path::new(
            "/tmp/omp-xdata-ref/data/database/skeletons/default_skeleton/anim/idle_aim.kf2",
        );
        if !skd_p.is_file() || !anim_p.is_file() {
            return;
        }
        let skin = parse_skd(&std::fs::read(skd_p).unwrap()).expect("retail SKD");
        assert_eq!(skin.bones.len(), 28);
        assert!(skin.bones.iter().any(|b| b == "Pelvis"));
        assert!(skin.bones.iter().any(|b| b == "Gun"));
        assert_eq!(skin.weights.len(), 874);
        let weighted = skin.weights.iter().filter(|w| !w.is_empty()).count();
        assert_eq!(weighted, 874);
        let two = skin.weights.iter().filter(|w| w.len() == 2).count();
        assert_eq!(two, 226);
        let anim = parse_kf2_anim(&std::fs::read(anim_p).unwrap()).expect("idle_aim");
        assert!(anim.bones.len() >= 20);
        assert_eq!(anim.bones[0].name, "Pelvis");
        assert!(anim.bones[0].parent.is_none());
        assert_eq!(anim.bones[1].parent.as_deref(), Some("Pelvis"));
        let pose_p =
            Path::new("/tmp/omp-xdata-ref/data/database/skeletons/default_skeleton/anim/Pose.kf2");
        let kfs_p = Path::new("/tmp/omp-xdata-ref/data/database/skins/max_payne/Payne_Max_L0.KFS");
        if pose_p.is_file() && kfs_p.is_file() {
            let bind = parse_kf2_anim(&std::fs::read(pose_p).unwrap()).expect("Pose.kf2");
            let mesh = parse_kfs_mesh(&std::fs::read(kfs_p).unwrap(), |_| None).expect("kfs");
            let posed = pose_mesh(&mesh, Some(&skin), Some(&bind), Some(&anim), 0.0);
            let min_y = posed.iter().map(|v| v[1]).fold(f32::MAX, f32::min);
            let max_y = posed.iter().map(|v| v[1]).fold(f32::MIN, f32::max);
            let span = max_y - min_y;
            assert!(
                (1.2..=2.8).contains(&span),
                "idle_aim posed height {span} (y {min_y}..{max_y})"
            );
        }
    }
}
