//! ActorFX / Keyframe 2.0 (`.kf2`, `.kfs`, `.skd`) format: chunks, meshes, skins and clips.
//!
//! `R_MemoryFile::createChunk` writes `0x0c` + `id: u32` + `version: u32` + `size: u32`
//! (`size` includes the 13-byte header). `.kfs` / `.kf2` files contain sequential top-level
//! chunks (`0x0001000f` materials, `0x00010005` TriMesh geometry, `0x00010011` keyframe
//! animations) and `.skd` files open with chunk `0x00010014` (skin/bone weights).

use std::collections::BTreeMap;

/// A decoded bitmap a material names.
#[derive(Debug, Clone)]
pub struct DecodedTexture {
    pub path: String,
    pub flag: u8,
    pub width: u32,
    pub height: u32,
    /// Packed `0x00RRGGBB` pixels (`width * height`).
    pub pixels: Vec<u32>,
}

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
    /// The node each run of `vertices` hangs from, in a packed model.
    pub parts: Vec<(String, std::ops::Range<usize>)>,
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
    /// Frames per second, stored after the bone name (`Stand.kf2` 15, `Run.kf2` 50).
    pub fps: f32,
    pub keys: Vec<AnimKeyframe>,
}

/// Skeletal clip: one `0x00010012` ver-5 chunk per bone (`idle_aim.kf2`, `Walk.kf2`).
#[derive(Debug, Clone, PartialEq)]
pub struct Kf2Anim {
    pub bones: Vec<AnimBone>,
}

pub fn read_i32(data: &[u8], i: &mut usize) -> Option<i32> {
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

pub fn read_str(data: &[u8], i: &mut usize) -> Option<String> {
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
        parts: Vec::new(),
    })
}

pub fn apply_weight_list(weights: &mut [Vec<BoneWeight>], recs: &[(usize, usize, f32)]) {
    for &(v, b, w) in recs {
        if v < weights.len() && w.is_finite() && w > 0.0 {
            weights[v].push(BoneWeight { bone: b, weight: w });
        }
    }
}

pub fn read_weight_recs(data: &[u8], i: &mut usize, n: usize) -> Option<Vec<(usize, usize, f32)>> {
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

pub fn read_0x1a(data: &[u8], i: &mut usize) -> Option<([f32; 9], [f32; 3])> {
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

/// Frame rate for a channel whose name chunk carries none (synthetic data only; retail clips all
/// store one).
pub const DEFAULT_KF2_FPS: f32 = 30.0;

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
            let mut fps = DEFAULT_KF2_FPS;
            if payload.first() == Some(&0x0c) && payload.len() >= 13 {
                let ssz = u32::from_le_bytes(payload[9..13].try_into().ok()?) as usize;
                if ssz >= 13 && ssz <= payload.len() {
                    let mut p = 13usize;
                    name = read_str(&payload[..ssz], &mut p).unwrap_or_default();
                    if let Some(rate) = read_i32(&payload[..ssz], &mut p).filter(|&r| r > 0) {
                        fps = rate as f32;
                    }
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
                    fps,
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

pub fn identity_mat() -> [f32; 9] {
    [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
}

pub fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Spherical blend of two column-major rotation matrices.
pub fn mat_to_quat(m: [f32; 9]) -> [f32; 4] {
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

pub fn quat_slerp(a: [f32; 4], mut b: [f32; 4], t: f32) -> [f32; 4] {
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

pub fn quat_norm(q: [f32; 4]) -> [f32; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1.0e-8 {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
    }
}

pub fn quat_to_mat(q: [f32; 4]) -> [f32; 9] {
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

pub fn mul_rt(rp: [f32; 9], tp: [f32; 3], rl: [f32; 9], tl: [f32; 3]) -> ([f32; 9], [f32; 3]) {
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

pub fn xform(mat: [f32; 9], pos: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    [
        mat[0] * v[0] + mat[3] * v[1] + mat[6] * v[2] + pos[0],
        mat[1] * v[0] + mat[4] * v[1] + mat[7] * v[2] + pos[1],
        mat[2] * v[0] + mat[5] * v[1] + mat[8] * v[2] + pos[2],
    ]
}

pub fn transpose3(m: [f32; 9]) -> [f32; 9] {
    [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]]
}

pub fn invert_rt(r: [f32; 9], t: [f32; 3]) -> ([f32; 9], [f32; 3]) {
    let rt = transpose3(r);
    let tinv = [
        -(rt[0] * t[0] + rt[3] * t[1] + rt[6] * t[2]),
        -(rt[1] * t[0] + rt[4] * t[1] + rt[7] * t[2]),
        -(rt[2] * t[0] + rt[5] * t[1] + rt[8] * t[2]),
    ];
    (rt, tinv)
}

/// Sample parent-relative locals at `time_s` (each bone's own frame rate, looping on its duration).
pub fn sample_locals(anim: &Kf2Anim, time_s: f32) -> BTreeMap<String, ([f32; 9], [f32; 3])> {
    let mut out = BTreeMap::new();
    for bone in &anim.bones {
        if bone.keys.is_empty() {
            continue;
        }
        let dur = bone.duration.max(1) as f32;
        let frame = (time_s.max(0.0) * bone.fps) % dur;
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
            // Retail `0x55e300`: component-wise lerp of the 3x4 key matrices, no slerp.
            let mat = std::array::from_fn(|k| a.mat[k] + (b.mat[k] - a.mat[k]) * t);
            (mat, lerp3(a.pos, b.pos, t))
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
    fn halfway_between_keys_is_the_component_wise_matrix_average() {
        // Retail 0x55e300: key[k] + (key[k+1] - key[k]) * frac on all 3x4 components, no slerp
        // (docs/rea/2026-11-01-kf2-vertex-animation.md, "Bone keyframe sampler found").
        let yaw90 = [0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0];
        let anim = Kf2Anim {
            bones: vec![AnimBone {
                name: "Pelvis".into(),
                parent: None,
                duration: 10,
                fps: 30.0,
                keys: vec![
                    AnimKeyframe {
                        frame: 0,
                        mat: identity_mat(),
                        pos: [0.0; 3],
                    },
                    AnimKeyframe {
                        frame: 2,
                        mat: yaw90,
                        pos: [2.0, 0.0, 0.0],
                    },
                ],
            }],
        };
        let (mat, pos) = sample_locals(&anim, 1.0 / 30.0)["pelvis"];
        let want = [0.5, 0.0, -0.5, 0.0, 1.0, 0.0, 0.5, 0.0, 0.5];
        for (got, want) in mat.iter().zip(want) {
            assert!((got - want).abs() < 1e-5, "{mat:?}");
        }
        assert!((pos[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn clips_play_at_the_frame_rate_stored_in_their_name_chunk() {
        // Retail Stand.kf2: `0d 14 06 "Pelvis" 14 0f 0e 01`, i.e. 15 frames per second.
        let mut name_pl = Vec::new();
        push_str(&mut name_pl, "Pelvis");
        push_u8(&mut name_pl, 15);
        name_pl.extend_from_slice(&[0x0e, 0x01]);
        let mut ch = Vec::new();
        push_chunk(&mut ch, 0x0001_0013, 0, &name_pl);
        push_str(&mut ch, "");
        ch.extend_from_slice(&[0x0e, 0x01]);
        push_u8(&mut ch, 31); // duration
        push_u8(&mut ch, 2); // nkeys
        for (frame, x) in [(0u8, 0.0f32), (30, 30.0)] {
            push_u8(&mut ch, frame);
            ch.push(0x1a);
            for v in identity_mat() {
                ch.extend_from_slice(&v.to_le_bytes());
            }
            for v in [x, 0.0, 0.0] {
                ch.extend_from_slice(&v.to_le_bytes());
            }
        }
        let mut buf = Vec::new();
        push_chunk(&mut buf, 0x0001_0012, 5, &ch);
        let anim = parse_kf2_anim(&buf).expect("anim");
        let (_, pos) = sample_locals(&anim, 1.0)["pelvis"];
        assert!((pos[0] - 15.0).abs() < 1e-4, "{pos:?}");
    }
}
