//! Full Max Payne 1 `.ldb` level scene decoder (`X_LevelDBImportLevel` version `0x20`).
//!
//! Decodes textures (JPEG, uncompressed TGA, ZSoft PCX), `256×256` RGB lightmaps,
//! materials, portal exits, static room meshes, dynamic meshes (doors/triggers/movers),
//! jumppoints, FSMs, characters, triggers, items, point lights, and the room table
//! with cycle-consistent BFS world origins.

#![allow(clippy::type_complexity)]

use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone)]
pub struct DecodedTexture {
    pub path: String,
    pub flag: u8,
    pub width: u32,
    pub height: u32,
    /// Packed `0x00RRGGBB` pixels (`width * height`).
    pub pixels: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct DecodedLightmap {
    pub width: u32,
    pub height: u32,
    /// Packed `0x00RRGGBB` pixels (`256 * 256`).
    pub pixels: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct SceneMaterial {
    pub id: i32,
    pub category: String,
    pub texture_path: String,
    pub texture_index: Option<usize>,
}

/// Draw and character-collision rules from `data/database/materials.txt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialPolicy {
    pub draw: bool,
    pub character_collides: bool,
    pub writes_z: bool,
    /// `DetailOffset` in `materials.txt`. Larger values sit closer to the camera.
    pub detail_offset: i32,
    pub skybox: bool,
}

/// Category behavior. Unknown names stay solid and visible.
pub fn material_policy(category: &str) -> MaterialPolicy {
    let mut policy = MaterialPolicy {
        draw: true,
        character_collides: true,
        writes_z: true,
        detail_offset: 0,
        skybox: false,
    };
    match category.to_ascii_lowercase().as_str() {
        "ai_node_collision_nodraw" => {
            policy.draw = false;
            policy.character_collides = false;
        }
        "cameracollision" => policy.draw = false,
        "character" => {
            policy.draw = false;
            policy.character_collides = false;
        }
        "charactercollision_nodraw" => policy.draw = false,
        "dummy" => {
            policy.draw = false;
            policy.character_collides = false;
        }
        "graffiti" => {
            policy.character_collides = false;
            policy.writes_z = false;
            policy.detail_offset = 1;
        }
        "leaves" | "water" | "nocollision" => policy.character_collides = false,
        "skybox" => {
            policy.draw = false;
            policy.skybox = true;
        }
        _ => {}
    }
    policy
}

/// Alpha fields from one `[Category]` in `materials.txt`.
///
/// `ID_ALPHAREFERENCE` is 15. `Leaves` is the category that overrides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialSurface {
    pub alpha_reference: i32,
    pub blended_alpha_test: bool,
}

pub fn parse_material_surfaces(text: &str) -> BTreeMap<String, MaterialSurface> {
    let mut defines = BTreeMap::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let Some(rest) = line.strip_prefix("#define") else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        if let (Some(name), Some(val)) = (parts.next(), parts.next()) {
            if let Ok(n) = val.parse::<i32>() {
                defines.insert(name.to_string(), n);
            }
        }
    }
    let mut out = BTreeMap::new();
    let mut name: Option<String> = None;
    let mut alpha = 0i32;
    let mut blended = true;
    let mut saw = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if let Some(inner) = line.strip_prefix('[') {
            if let Some(end) = inner.find(']') {
                if let Some(prev) = name.take() {
                    if saw {
                        out.insert(
                            prev,
                            MaterialSurface {
                                alpha_reference: alpha,
                                blended_alpha_test: blended,
                            },
                        );
                    }
                }
                name = Some(inner[..end].trim().to_string());
                alpha = 0;
                blended = true;
                saw = false;
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_end_matches(';').trim();
        if key.eq_ignore_ascii_case("AlphaReference") {
            alpha = value
                .parse::<i32>()
                .ok()
                .or_else(|| defines.get(value).copied())
                .unwrap_or(0);
            saw = true;
        } else if key.eq_ignore_ascii_case("BlendedAlphaTest") {
            blended = value.eq_ignore_ascii_case("TRUE");
            saw = true;
        }
    }
    if let Some(prev) = name {
        if saw {
            out.insert(
                prev,
                MaterialSurface {
                    alpha_reference: alpha,
                    blended_alpha_test: blended,
                },
            );
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct SceneVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub luv: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct ScenePoly {
    pub vertices: Vec<SceneVertex>,
    pub normal: [f32; 3],
    pub material_id: i32,
    pub lightmap_id: i32,
}

#[derive(Debug, Clone)]
pub struct SceneStaticMesh {
    pub mesh_id: i32,
    pub room_id: Option<i32>,
    pub world_origin: [f32; 3],
    pub polys: Vec<ScenePoly>,
}

#[derive(Debug, Clone)]
pub struct SceneDynamicAnim {
    pub name: String,
    pub duration: f32,
    pub start_mat: [f32; 9],
    pub start_pos: [f32; 3],
    pub end_mat: [f32; 9],
    pub end_pos: [f32; 3],
    pub scripts: Vec<String>,
    /// Two sample channels. Each sample is a tagged `f32` (`0x09`). One channel
    /// runs along the start→end distance; the other is a 0..1 clock.
    pub curves: [Vec<f32>; 2],
}

#[derive(Debug, Clone)]
pub struct SceneDynamicMesh {
    pub name: String,
    pub room_id: i32,
    pub world_mat: [f32; 9],
    pub world_pos: [f32; 3],
    pub local_start_mat: [f32; 9],
    pub local_start_pos: [f32; 3],
    pub toggled: bool,
    pub polys: Vec<ScenePoly>,
    pub anims: Vec<SceneDynamicAnim>,
}

#[derive(Debug, Clone)]
pub struct ScenePortalExit {
    pub name: String,
    pub corners: Vec<[f32; 3]>,
    pub normal: [f32; 3],
    pub relative_offset: [f32; 3],
    pub target_room_id: i32,
    pub target_exit_name: String,
}

#[derive(Debug, Clone)]
pub struct SceneJumppoint {
    pub name: String,
    pub room_id: i32,
    pub pos: [f32; 3],
    pub jp_type: i32,
}

#[derive(Debug, Clone)]
pub struct SceneFsm {
    pub name: String,
    pub room_id: i32,
    pub pos: [f32; 3],
    pub states: Vec<String>,
    pub default_state: String,
    pub startup_actions: Vec<String>,
    pub hooks: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Clone)]
pub struct SceneCharacter {
    pub name: String,
    pub skin: String,
    pub room_id: i32,
    pub pos: [f32; 3],
    pub startup_scripts: Vec<String>,
}

impl SceneCharacter {
    /// Startup `C_Kill` (Michelle is already dead when the house prologue begins).
    pub fn starts_dead(&self) -> bool {
        self.startup_scripts
            .iter()
            .any(|s| s.to_ascii_lowercase().contains("c_kill"))
    }

    /// Chase only when a startup script enters combat. Placed actors stay on their mark.
    pub fn pursues(&self) -> bool {
        self.startup_scripts.iter().any(|s| {
            let s = s.to_ascii_lowercase();
            s.contains("mobstercombat") || s.contains("c_goto(")
        })
    }
}

#[derive(Debug, Clone)]
pub struct SceneTrigger {
    pub name: String,
    pub room_id: i32,
    pub pos: [f32; 3],
    pub radius: f32,
    pub trigger_type: i32,
}

#[derive(Debug, Clone)]
pub struct SceneItem {
    pub name: String,
    pub item_type: String,
    pub room_id: i32,
    pub pos: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct ScenePointLight {
    pub id: i32,
    pub room_id: i32,
    pub pos: [f32; 3],
    pub color: [f32; 4],
    pub radius: f32,
    pub intensity: f32,
}

#[derive(Debug, Clone)]
pub struct SceneRoom {
    pub id: i32,
    pub name: String,
    pub static_meshes: Vec<i32>,
    pub exits: Vec<String>,
    pub dynamic_meshes: Vec<String>,
    pub world_origin: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct LdbScene {
    pub textures: Vec<DecodedTexture>,
    pub materials: BTreeMap<i32, SceneMaterial>,
    pub lightmaps: Vec<DecodedLightmap>,
    pub exits: Vec<ScenePortalExit>,
    pub static_meshes: Vec<SceneStaticMesh>,
    pub jumppoints: Vec<SceneJumppoint>,
    pub fsms: Vec<SceneFsm>,
    pub characters: Vec<SceneCharacter>,
    pub triggers: Vec<SceneTrigger>,
    pub dynamic_meshes: Vec<SceneDynamicMesh>,
    pub items: Vec<SceneItem>,
    pub point_lights: Vec<ScenePointLight>,
    pub rooms: Vec<SceneRoom>,
    pub bytes_consumed: usize,
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
        _ => None,
    }
}

fn read_u8_tag(data: &[u8], i: &mut usize) -> Option<u8> {
    if *data.get(*i)? != 0x11 {
        return None;
    }
    let v = *data.get(*i + 1)?;
    *i += 2;
    Some(v)
}

fn read_point_count(data: &[u8], i: &mut usize) -> Option<usize> {
    match *data.get(*i)? {
        0x11 => Some(read_u8_tag(data, i)? as usize),
        0x10 => {
            let v = u16::from_le_bytes([*data.get(*i + 1)?, *data.get(*i + 2)?]) as usize;
            *i += 3;
            Some(v)
        }
        0x14 | 0x13 | 0x12 | 0x02 => Some(read_i32(data, i)? as usize),
        _ => None,
    }
}

fn read_f32_tag(data: &[u8], i: &mut usize) -> Option<f32> {
    if *data.get(*i)? != 0x09 {
        return None;
    }
    let v = f32::from_le_bytes(data.get(*i + 1..*i + 5)?.try_into().ok()?);
    *i += 5;
    Some(v)
}

/// `n` animation samples. On disk each one is 5 bytes; a `0x09` sample is a float.
fn read_f32_samples(data: &[u8], i: &mut usize, n: usize) -> Option<Vec<f32>> {
    let mut out = Vec::with_capacity(n.min(8192));
    for _ in 0..n {
        if data.get(*i) == Some(&0x09) {
            out.push(read_f32_tag(data, i)?);
        } else if *i + 5 <= data.len() {
            *i += 5;
        } else {
            return None;
        }
    }
    Some(out)
}

fn read_vec2_tag(data: &[u8], i: &mut usize) -> Option<[f32; 2]> {
    if *data.get(*i)? != 0x15 {
        return None;
    }
    let u = f32::from_le_bytes(data.get(*i + 1..*i + 5)?.try_into().ok()?);
    let v = f32::from_le_bytes(data.get(*i + 5..*i + 9)?.try_into().ok()?);
    *i += 9;
    Some([u, v])
}

fn read_vec3_tag(data: &[u8], i: &mut usize) -> Option<[f32; 3]> {
    if *data.get(*i)? != 0x16 {
        return None;
    }
    let x = f32::from_le_bytes(data.get(*i + 1..*i + 5)?.try_into().ok()?);
    let y = f32::from_le_bytes(data.get(*i + 5..*i + 9)?.try_into().ok()?);
    let z = f32::from_le_bytes(data.get(*i + 9..*i + 13)?.try_into().ok()?);
    *i += 13;
    Some([x, y, z])
}

fn read_xform(data: &[u8], i: &mut usize) -> Option<([f32; 9], [f32; 3])> {
    if *data.get(*i)? != 0x1a || *i + 49 > data.len() {
        return None;
    }
    let mut mat = [0.0f32; 9];
    for (k, m) in mat.iter_mut().enumerate() {
        let off = *i + 1 + k * 4;
        *m = f32::from_le_bytes(data[off..off + 4].try_into().ok()?);
    }
    let p = *i + 37;
    let x = f32::from_le_bytes(data[p..p + 4].try_into().ok()?);
    let y = f32::from_le_bytes(data[p + 4..p + 8].try_into().ok()?);
    let z = f32::from_le_bytes(data[p + 8..p + 12].try_into().ok()?);
    *i += 49;
    Some((mat, [x, y, z]))
}

fn read_xform_pos(data: &[u8], i: &mut usize) -> Option<[f32; 3]> {
    read_xform(data, i).map(|(_, p)| p)
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

fn read_str_list(data: &[u8], i: &mut usize, tag: u8) -> Option<Vec<String>> {
    if *data.get(*i)? != tag {
        return None;
    }
    *i += 1;
    let count = read_i32(data, i)? as usize;
    let mut out = Vec::with_capacity(count.min(256));
    for _ in 0..count {
        out.push(read_str(data, i)?);
    }
    Some(out)
}

fn skip_color_and_bool(data: &[u8], i: &mut usize) -> Option<()> {
    match *data.get(*i)? {
        0x03 => *i += 5,
        0x0f => *i += 4,
        0x10 => *i += 3,
        0x11 => *i += 2,
        0x14 | 0x13 | 0x12 | 0x02 => {
            read_i32(data, i)?;
        }
        _ => return None,
    }
    if *data.get(*i)? != 0x0e {
        return None;
    }
    *i += 2;
    Some(())
}

/// Decode JPEG, uncompressed 24/32-bit TGA, or 8-bit ZSoft PCX into `0x00RRGGBB` pixels.
pub fn decode_texture_blob(blob: &[u8]) -> Option<(u32, u32, Vec<u32>)> {
    if blob.len() >= 2 && blob[0] == 0xff && blob[1] == 0xd8 {
        let mut dec = jpeg_decoder::Decoder::new(blob);
        let rgb = dec.decode().ok()?;
        let info = dec.info()?;
        let w = info.width as u32;
        let h = info.height as u32;
        let mut pixels = Vec::with_capacity((w * h) as usize);
        match info.pixel_format {
            jpeg_decoder::PixelFormat::RGB24 => {
                let mut k = 0;
                while k + 3 <= rgb.len() {
                    pixels.push(
                        ((rgb[k] as u32) << 16) | ((rgb[k + 1] as u32) << 8) | (rgb[k + 2] as u32),
                    );
                    k += 3;
                }
            }
            jpeg_decoder::PixelFormat::L8 => {
                for &g in &rgb {
                    let v = g as u32;
                    pixels.push((v << 16) | (v << 8) | v);
                }
            }
            _ => return None,
        }
        return Some((w, h, pixels));
    }
    // Uncompressed true-color TGA (byte 2 == 2)
    if blob.len() >= 18 && blob[1] == 0 && blob[2] == 2 {
        let id_len = blob[0] as usize;
        let w = u16::from_le_bytes([blob[12], blob[13]]) as u32;
        let h = u16::from_le_bytes([blob[14], blob[15]]) as u32;
        let bpp = blob[16];
        let flip_y = (blob[17] & 0x20) == 0;
        let step = (bpp / 8) as usize;
        let start = 18 + id_len;
        if (step == 3 || step == 4)
            && w > 0
            && h > 0
            && start + (w * h) as usize * step <= blob.len()
        {
            let mut pixels = vec![0u32; (w * h) as usize];
            let raw = &blob[start..];
            for y in 0..h as usize {
                let dst_y = if flip_y { h as usize - 1 - y } else { y };
                for x in 0..w as usize {
                    let off = (y * w as usize + x) * step;
                    let b = raw[off] as u32;
                    let g = raw[off + 1] as u32;
                    let r = raw[off + 2] as u32;
                    pixels[dst_y * w as usize + x] = (r << 16) | (g << 8) | b;
                }
            }
            return Some((w, h, pixels));
        }
    }
    // ZSoft PCX (manufacturer 0x0a, 8-bit 1-plane with 256-color VGA palette at EOF-769)
    if blob.len() > 896 && blob[0] == 0x0a && blob[2] == 1 && blob[3] == 8 {
        let xmin = u16::from_le_bytes([blob[4], blob[5]]) as u32;
        let ymin = u16::from_le_bytes([blob[6], blob[7]]) as u32;
        let xmax = u16::from_le_bytes([blob[8], blob[9]]) as u32;
        let ymax = u16::from_le_bytes([blob[10], blob[11]]) as u32;
        if xmax >= xmin && ymax >= ymin {
            let w = xmax - xmin + 1;
            let h = ymax - ymin + 1;
            let bpl = u16::from_le_bytes([blob[66], blob[67]]) as usize;
            let pal_pos = blob.len() - 769;
            if blob[pal_pos] == 0x0c {
                let pal = &blob[pal_pos + 1..];
                let mut indices = Vec::with_capacity(bpl * h as usize);
                let mut p = 128;
                while indices.len() < bpl * h as usize && p < pal_pos {
                    let b = blob[p];
                    p += 1;
                    if (b & 0xc0) == 0xc0 {
                        let cnt = (b & 0x3f) as usize;
                        let val = *blob.get(p)?;
                        p += 1;
                        for _ in 0..cnt {
                            indices.push(val);
                        }
                    } else {
                        indices.push(b);
                    }
                }
                if indices.len() >= bpl * h as usize {
                    let mut pixels = Vec::with_capacity((w * h) as usize);
                    for y in 0..h as usize {
                        for x in 0..w as usize {
                            let idx = indices[y * bpl + x] as usize * 3;
                            let r = pal[idx] as u32;
                            let g = pal[idx + 1] as u32;
                            let b = pal[idx + 2] as u32;
                            pixels.push((r << 16) | (g << 8) | b);
                        }
                    }
                    return Some((w, h, pixels));
                }
            }
        }
    }
    None
}

fn read_point_stream_vec(data: &[u8], i: &mut usize) -> Option<Vec<(usize, [f32; 2], [f32; 2])>> {
    while data.get(*i) == Some(&0x1c)
        || (matches!(data.get(*i), Some(0x14 | 0x13 | 0x02)) && data.get(*i + 2) != Some(&0x15))
    {
        if data[*i] == 0x1c {
            *i += 1;
            if matches!(data.get(*i), Some(0x12 | 0x13)) {
                let mark = *i;
                let cnt = read_i32(data, i)? as usize;
                let _sub = read_i32(data, i)?;
                if data.get(*i) == Some(&0x15) {
                    return read_point_bodies(data, i, cnt);
                }
                *i = mark;
            }
        } else {
            read_i32(data, i)?;
        }
    }
    let cnt = read_i32(data, i)? as usize;
    let _sub = read_i32(data, i)?;
    read_point_bodies(data, i, cnt)
}

fn read_point_bodies(
    data: &[u8],
    i: &mut usize,
    cnt: usize,
) -> Option<Vec<(usize, [f32; 2], [f32; 2])>> {
    let mut out = Vec::with_capacity(cnt.min(200_000));
    for k in 0..cnt {
        let vid = if k == 0 {
            0
        } else {
            read_i32(data, i)? as usize
        };
        let uv = read_vec2_tag(data, i)?;
        let luv = read_vec2_tag(data, i)?;
        skip_color_and_bool(data, i)?;
        out.push((vid, uv, luv));
    }
    // Optional trailing uncounted bodies
    loop {
        let mark = *i;
        let Some(vid) = read_i32(data, i) else {
            *i = mark;
            break;
        };
        if data.get(*i) != Some(&0x15) {
            *i = mark;
            break;
        }
        let Some(uv) = read_vec2_tag(data, i) else {
            *i = mark;
            break;
        };
        let Some(luv) = read_vec2_tag(data, i) else {
            *i = mark;
            break;
        };
        if skip_color_and_bool(data, i).is_none() {
            *i = mark;
            break;
        }
        out.push((vid as usize, uv, luv));
    }
    Some(out)
}

fn read_mesh_polys_resolved(
    data: &[u8],
    i: &mut usize,
    verts: &[[f32; 3]],
    points: &[(usize, [f32; 2], [f32; 2])],
) -> Option<Vec<ScenePoly>> {
    let poly_count = read_i32(data, i)? as usize;
    let _hdr1 = read_i32(data, i)?;
    let mut polys = Vec::with_capacity(poly_count.min(32_768));
    for p in 0..poly_count {
        let pref = read_i32(data, i)? as usize;
        let nverts = read_i32(data, i)? as usize;
        let normal = read_vec3_tag(data, i)?;
        let _flag = read_u8_tag(data, i)?;
        let mat_id = read_i32(data, i)?;
        let lmap_id = read_i32(data, i)?;
        let _u_scale = read_f32_tag(data, i)?;
        let _v_scale = read_f32_tag(data, i)?;
        let _z1 = read_i32(data, i)?;
        if p + 1 < poly_count {
            let _idx = read_i32(data, i)?;
        }
        let mut pverts = Vec::with_capacity(nverts);
        for c in 0..nverts {
            if let Some(&(vid, uv, luv)) = points.get(pref + c) {
                if let Some(&pos) = verts.get(vid) {
                    pverts.push(SceneVertex { pos, uv, luv });
                }
            }
        }
        if pverts.len() >= 3 {
            polys.push(ScenePoly {
                vertices: pverts,
                normal,
                material_id: mat_id,
                lightmap_id: lmap_id,
            });
        }
    }
    Some(polys)
}

fn skip_hook_map(data: &[u8], i: &mut usize) -> Option<Vec<(String, Vec<String>)>> {
    if *data.get(*i)? != 0x1f {
        return None;
    }
    *i += 1;
    let cnt = read_i32(data, i)? as usize;
    let mut out = Vec::with_capacity(cnt);
    for _ in 0..cnt {
        let hname = read_str(data, i)?;
        let mut acts = read_str_list(data, i, 0x1c)?;
        if *data.get(*i)? != 0x1f {
            return None;
        }
        *i += 1;
        let c2 = read_i32(data, i)? as usize;
        for _ in 0..c2 {
            let _state_name = read_str(data, i)?;
            let mut s_acts = read_str_list(data, i, 0x1c)?;
            acts.append(&mut s_acts);
        }
        let mut after = read_str_list(data, i, 0x1c)?;
        acts.append(&mut after);
        out.push((hname, acts));
    }
    Some(out)
}

/// Parse an entire `.ldb` byte buffer into a walkable [`LdbScene`].
pub fn parse_ldb_scene(data: &[u8]) -> Option<LdbScene> {
    let import_off = super::ras_ldb_after_geometry(data)?;
    let mut i = import_off + 2; // skip `14 20`

    // 1. Textures
    let tex_count = read_i32(data, &mut i)? as usize;
    let mut textures = Vec::with_capacity(tex_count);
    let mut tex_lookup: BTreeMap<String, usize> = BTreeMap::new();
    for idx in 0..tex_count {
        let path = read_str(data, &mut i)?;
        let flag = read_u8_tag(data, &mut i)?;
        let blob_len = read_i32(data, &mut i)? as usize;
        let blob = data.get(i..i + blob_len)?;
        i += blob_len;
        let (width, height, pixels) =
            decode_texture_blob(blob).unwrap_or_else(|| (1, 1, vec![0x00888888]));
        let lower = path.to_ascii_lowercase();
        let base = lower
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&lower)
            .to_string();
        let stem = base
            .rsplit_once('.')
            .map(|(s, _)| s.to_string())
            .unwrap_or_else(|| base.clone());
        tex_lookup.entry(lower).or_insert(idx);
        tex_lookup.entry(base).or_insert(idx);
        tex_lookup.entry(stem).or_insert(idx);
        textures.push(DecodedTexture {
            path,
            flag,
            width,
            height,
            pixels,
        });
    }

    // 2. Materials (walk consecutive 0x1f material lists; second list holds zero-based IDs)
    let mut materials = BTreeMap::new();
    let mut mat_passes = 0;
    while data.get(i) == Some(&0x1f) && mat_passes < 4 {
        let mark = i;
        i += 1;
        let Some(mcnt) = read_i32(data, &mut i) else {
            i = mark;
            break;
        };
        if data.get(i) != Some(&0x25) {
            // first material list has an extra header int before 0x25
            let mut probe = i;
            if read_i32(data, &mut probe).is_some() && data.get(probe) == Some(&0x25) {
                i = probe;
            } else {
                i = mark;
                break;
            }
        }
        for _ in 0..mcnt {
            if data.get(i) != Some(&0x25) {
                break;
            }
            i += 1;
            let category = read_str(data, &mut i)?;
            let texture_path = read_str(data, &mut i)?;
            let id = if matches!(data.get(i), Some(0x14 | 0x13 | 0x12 | 0x02)) {
                read_i32(data, &mut i)?
            } else {
                -1
            };
            let mut clean = texture_path.to_ascii_lowercase();
            if let Some(stripped) = clean.strip_suffix(" new") {
                clean = stripped.trim().to_string();
            }
            let base = clean.rsplit(['\\', '/']).next().unwrap_or(&clean);
            let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
            let texture_index = tex_lookup
                .get(&clean)
                .or_else(|| tex_lookup.get(base))
                .or_else(|| tex_lookup.get(stem))
                .copied();
            materials.insert(
                id,
                SceneMaterial {
                    id,
                    category,
                    texture_path,
                    texture_index,
                },
            );
        }
        mat_passes += 1;
    }

    // 3. Material categories
    let (_, _, after_cats) = super::parse_ras_ldb_import_material_categories_at(data, i)?;
    i = after_cats;

    // 4. Lightmaps (`256×256` RGB planes)
    let lm_count = read_i32(data, &mut i)? as usize;
    let _z = read_i32(data, &mut i)?;
    let _u = read_u8_tag(data, &mut i)?;
    if *data.get(i)? != 0x12 {
        return None;
    }
    i += 1;
    let lm_size = u32::from_le_bytes(data.get(i..i + 4)?.try_into().ok()?) as usize;
    i += 4 + 12 + 3 + 2; // 3 u32s + 01 00 01 + 18 20
    let rgb_len = if lm_size as u32 == super::LDB_IMPORT_LIGHTMAP_RGB_SIZE {
        lm_size - 18
    } else {
        lm_size
    };
    let mut lightmaps = Vec::with_capacity(lm_count);
    for n in 0..lm_count {
        if n > 0 {
            i += 8; // LDB_IMPORT_LIGHTMAP_SEP
        }
        let need = if n + 1 == lm_count { rgb_len } else { lm_size };
        let plane = data.get(i..i + rgb_len.min(need))?;
        let mut pixels = Vec::with_capacity(256 * 256);
        let mut k = 0;
        while k + 3 <= plane.len() && pixels.len() < 256 * 256 {
            // BGR/RGB in lightmap plane
            pixels.push(
                ((plane[k + 2] as u32) << 16) | ((plane[k + 1] as u32) << 8) | (plane[k] as u32),
            );
            k += 3;
        }
        lightmaps.push(DecodedLightmap {
            width: 256,
            height: 256,
            pixels,
        });
        i += need;
    }

    // 5. Portal Exits
    let mut exits = Vec::new();
    let (_, exits_end) = super::parse_ras_ldb_import_exits_at(data, i, 512)?;
    let mut ei = i;
    while ei < exits_end {
        if data.get(ei) == Some(&0x0d)
            && data.get(ei + 1) == Some(&0x14)
            && data.get(ei + 3..ei + 5) == Some(b"::")
        {
            let mark = ei;
            if let Some(name) = read_str(data, &mut ei) {
                if data.get(ei) == Some(&0x11) {
                    if let Some(ncorn) = read_u8_tag(data, &mut ei) {
                        let mut corners = Vec::with_capacity(ncorn as usize);
                        let mut ok = true;
                        for _ in 0..ncorn {
                            if let Some(c) = read_vec3_tag(data, &mut ei) {
                                corners.push(c);
                            } else {
                                ok = false;
                                break;
                            }
                        }
                        if ok {
                            if let (
                                Some(normal),
                                Some(rel),
                                Some(_id_b),
                                Some(target_room_id),
                                Some(target_exit_name),
                            ) = (
                                read_vec3_tag(data, &mut ei),
                                read_xform_pos(data, &mut ei),
                                read_i32(data, &mut ei),
                                read_i32(data, &mut ei),
                                read_str(data, &mut ei),
                            ) {
                                exits.push(ScenePortalExit {
                                    name,
                                    corners,
                                    normal,
                                    relative_offset: rel,
                                    target_room_id,
                                    target_exit_name,
                                });
                                continue;
                            }
                        }
                    }
                }
            }
            ei = mark + 1;
        } else {
            ei += 1;
        }
    }
    i = exits_end;

    // 6. Static mesh point stream (`vid, uv, luv`)
    let static_points = read_point_stream_vec(data, &mut i)?;

    // 7. Static meshes + polygons + optional mesh attribs
    let mut static_meshes = Vec::new();
    loop {
        let mark = i;
        if data.get(i) == Some(&0x1f) {
            let mut p = i + 1;
            if read_i32(data, &mut p) == Some(0) {
                i = p;
            } else {
                break;
            }
        }
        let Some(first) = read_i32(data, &mut i) else {
            i = mark;
            break;
        };
        let (mesh_id, npts) = if matches!(data.get(i), Some(0x11 | 0x10)) {
            let Some(n) = read_point_count(data, &mut i) else {
                i = mark;
                break;
            };
            (first, n)
        } else if let Some(b) = read_i32(data, &mut i) {
            if !matches!(data.get(i), Some(0x11 | 0x10)) {
                i = mark;
                break;
            }
            let Some(n) = read_point_count(data, &mut i) else {
                i = mark;
                break;
            };
            (b, n)
        } else {
            i = mark;
            break;
        };
        if npts == 0 || npts > 500_000 || data.get(i) != Some(&0x16) {
            i = mark;
            break;
        }
        let mut verts = Vec::with_capacity(npts);
        for _ in 0..npts {
            verts.push(read_vec3_tag(data, &mut i)?);
        }
        if *data.get(i)? != 0x1c {
            i = mark;
            break;
        }
        i += 1;
        let n2 = read_i32(data, &mut i)? as usize;
        i += n2 * 13; // vertex normals
        let _editor_pos = read_xform_pos(data, &mut i)?;
        let polys = read_mesh_polys_resolved(data, &mut i, &verts, &static_points)?;
        // Optional non-empty mesh attribs (`0x1f` + count + (id, vec3)*count)
        if data.get(i) == Some(&0x1f) {
            let mut p = i + 1;
            if let Some(acnt) = read_i32(data, &mut p) {
                if acnt > 0 && data.get(p) != Some(&0x25) {
                    i = p;
                    for _ in 0..acnt {
                        read_i32(data, &mut i)?;
                        read_vec3_tag(data, &mut i)?;
                    }
                }
            }
        }
        static_meshes.push(SceneStaticMesh {
            mesh_id,
            room_id: None,
            world_origin: [0.0, 0.0, 0.0],
            polys,
        });
    }

    // 8. Directional/spot lights (may be 0 when preceded by trailing 0 from last poly)
    if data.get(i) == Some(&0x14) && data.get(i + 1) == Some(&0x00) {
        let mut p = i + 2;
        if let Some(next_cnt) = read_i32(data, &mut p) {
            if next_cnt > 0 && data.get(p) == Some(&0x0d) {
                // `14 00` was either 0 lights or trailing poly index; check if `p` is Light:: or Jumppoint
                let mut sp = p;
                if let Some(s) = read_str(data, &mut sp) {
                    if !s.starts_with("Light::") {
                        i += 2; // 0 lights! `next_cnt` at `i` is jumppoint count
                    }
                }
            }
        }
    }
    if let Some((_, _, lend)) = super::parse_ras_ldb_import_lights_at(data, i, 0) {
        // verify first light was actually a light record (has `0x19` mat3)
        i = lend;
    }

    // 9. Jumppoints.
    // House levels start at the count (`14 <n>`). Startup wraps that count:
    // empty `0x1f` list, int 0, then the count, then the named records.
    let mut jumppoints = Vec::new();
    let jcnt = if data.get(i) == Some(&0x1f) {
        match super::parse_ras_ldb_import_character_head_at(data, i) {
            Some((count, at)) => {
                i = at;
                Some(count as i32)
            }
            None => read_i32(data, &mut i),
        }
    } else {
        read_i32(data, &mut i)
    };
    if let Some(jcnt) = jcnt {
        for _ in 0..jcnt {
            let name = read_str(data, &mut i)?;
            let _parent = read_str(data, &mut i)?;
            let pos = read_xform_pos(data, &mut i)?;
            let _x2 = read_xform_pos(data, &mut i)?;
            let room_id = read_i32(data, &mut i)?;
            let _unk = read_str(data, &mut i)?;
            let jp_type = read_i32(data, &mut i)?;
            jumppoints.push(SceneJumppoint {
                name,
                room_id,
                pos,
                jp_type,
            });
        }
    }

    // 10. FSMs
    let mut fsms = Vec::new();
    if let Some(fcnt) = read_i32(data, &mut i) {
        for _ in 0..fcnt {
            let name = read_str(data, &mut i)?;
            let _parent = read_str(data, &mut i)?;
            let pos = read_xform_pos(data, &mut i)?;
            let _x2 = read_xform_pos(data, &mut i)?;
            let room_id = read_i32(data, &mut i)?;
            let _unk = read_str(data, &mut i)?;
            let states = read_str_list(data, &mut i, 0x1c)?;
            let default_state = read_str(data, &mut i)?;
            let startup_actions = read_str_list(data, &mut i, 0x1c)?;
            let mut hooks = skip_hook_map(data, &mut i)?;
            let _extra = read_str_list(data, &mut i, 0x1c)?;
            hooks.append(&mut skip_hook_map(data, &mut i)?);
            hooks.append(&mut skip_hook_map(data, &mut i)?);
            hooks.append(&mut skip_hook_map(data, &mut i)?);
            fsms.push(SceneFsm {
                name,
                room_id,
                pos,
                states,
                default_state,
                startup_actions,
                hooks,
            });
        }
    }

    // 11. Characters
    let mut characters = Vec::new();
    if let Some(ccnt) = read_i32(data, &mut i) {
        for _ in 0..ccnt {
            let name = read_str(data, &mut i)?;
            let _parent = read_str(data, &mut i)?;
            let pos = read_xform_pos(data, &mut i)?;
            let _x2 = read_xform_pos(data, &mut i)?;
            let room_id = read_i32(data, &mut i)?;
            let _unk = read_str(data, &mut i)?;
            let skin = read_str(data, &mut i)?;
            let mut startup_scripts = read_str_list(data, &mut i, 0x1c)?;
            for _ in 0..3 {
                startup_scripts.append(&mut read_str_list(data, &mut i, 0x1c)?);
            }
            characters.push(SceneCharacter {
                name,
                skin,
                room_id,
                pos,
                startup_scripts,
            });
        }
    }

    // 12. Triggers
    let mut triggers = Vec::new();
    if let Some(tcnt) = read_i32(data, &mut i) {
        for _ in 0..tcnt {
            let name = read_str(data, &mut i)?;
            let _parent = read_str(data, &mut i)?;
            let pos = read_xform_pos(data, &mut i)?;
            let _x2 = read_xform_pos(data, &mut i)?;
            let room_id = read_i32(data, &mut i)?;
            let _target = read_str(data, &mut i)?;
            let radius = read_f32_tag(data, &mut i)?;
            let trigger_type = read_i32(data, &mut i)?;
            triggers.push(SceneTrigger {
                name,
                room_id,
                pos,
                radius,
                trigger_type,
            });
        }
    }

    // 13. Dynamic mesh point stream + Dynamic meshes (`X_LevelDBImportDynamicMesh`)
    let mut dynamic_meshes = Vec::new();
    if let Some(dyn_points) = read_point_stream_vec(data, &mut i) {
        if let Some(dm_cnt) = read_i32(data, &mut i) {
            for d in 0..dm_cnt {
                let name = read_str(data, &mut i)?;
                let npts = read_point_count(data, &mut i)?;
                let mut verts = Vec::with_capacity(npts);
                for _ in 0..npts {
                    verts.push(read_vec3_tag(data, &mut i)?);
                }
                if *data.get(i)? != 0x1c {
                    return None;
                }
                i += 1;
                let n2 = read_i32(data, &mut i)? as usize;
                i += n2 * 13;
                let (local_start_mat, local_start_pos) = read_xform(data, &mut i)?;
                let polys = read_mesh_polys_resolved(data, &mut i, &verts, &dyn_points)?;
                if *data.get(i)? != 0x1f {
                    return None;
                }
                i += 1;
                let acnt = read_i32(data, &mut i)? as usize;
                for _ in 0..acnt {
                    read_i32(data, &mut i)?;
                    read_vec3_tag(data, &mut i)?;
                }
                let _parent = read_str(data, &mut i)?;
                let (world_mat, world_pos) = read_xform(data, &mut i)?;
                let _x2 = read_xform_pos(data, &mut i)?;
                let room_id = read_i32(data, &mut i)?;
                let _unk_s = read_str(data, &mut i)?;
                let anim_cnt = read_i32(data, &mut i)? as usize;
                let mut anims = Vec::with_capacity(anim_cnt);
                for _ in 0..anim_cnt {
                    let aname = read_str(data, &mut i)?;
                    let duration = read_f32_tag(data, &mut i)?;
                    let (start_mat, start_pos) = read_xform(data, &mut i)?;
                    let (end_mat, end_pos) = read_xform(data, &mut i)?;
                    let mut scripts = read_str_list(data, &mut i, 0x1c)?;
                    scripts.append(&mut read_str_list(data, &mut i, 0x1c)?);
                    scripts.append(&mut read_str_list(data, &mut i, 0x1c)?);
                    let mut curves = [Vec::new(), Vec::new()];
                    for curve in &mut curves {
                        let _cty = read_i32(data, &mut i)?;
                        let _cver = read_i32(data, &mut i)?;
                        let c_ch = read_i32(data, &mut i)? as usize;
                        let c_smp = read_i32(data, &mut i)? as usize;
                        *curve = read_f32_samples(data, &mut i, c_ch.saturating_mul(c_smp))?;
                    }
                    anims.push(SceneDynamicAnim {
                        name: aname,
                        duration,
                        start_mat,
                        start_pos,
                        curves,
                        end_mat,
                        end_pos,
                        scripts,
                    });
                }
                i += 12; // 6 bools (`0e u8`)
                let _f_start = read_i32(data, &mut i)?;
                let _f_cnt = read_i32(data, &mut i)?;
                let _s_start = read_i32(data, &mut i)?;
                if d + 1 < dm_cnt {
                    let _next_idx = read_i32(data, &mut i)?;
                }
                dynamic_meshes.push(SceneDynamicMesh {
                    name,
                    room_id,
                    world_mat,
                    world_pos,
                    local_start_mat,
                    local_start_pos,
                    toggled: false,
                    polys,
                    anims,
                });
            }
        }
    }

    // 14. Level items (`X_LevelDBImportLevelItem`) + Point lights + Rooms (`X_LevelDBImportRoom`)
    let mut items = Vec::new();
    let mut point_lights = Vec::new();
    let mut rooms = Vec::new();
    if let Some(_item_ver) = read_i32(data, &mut i) {
        if let Some(icnt) = read_i32(data, &mut i) {
            for _ in 0..icnt {
                let name = read_str(data, &mut i)?;
                let _parent = read_str(data, &mut i)?;
                let pos = read_xform_pos(data, &mut i)?;
                let _x2 = read_xform_pos(data, &mut i)?;
                let room_id = read_i32(data, &mut i)?;
                let _unk = read_str(data, &mut i)?;
                let item_type = read_str(data, &mut i)?;
                items.push(SceneItem {
                    name,
                    item_type,
                    room_id,
                    pos,
                });
            }
        }
        if let Some(pl_cnt) = read_i32(data, &mut i) {
            for _ in 0..pl_cnt {
                let id = read_i32(data, &mut i)?;
                let _name = read_str(data, &mut i)?;
                let pos = read_xform_pos(data, &mut i)?;
                let _x2 = read_xform_pos(data, &mut i)?;
                let room_id = read_i32(data, &mut i)?;
                let _unk = read_str(data, &mut i)?;
                let r = read_f32_tag(data, &mut i)?;
                let g = read_f32_tag(data, &mut i)?;
                let b = read_f32_tag(data, &mut i)?;
                let a = read_f32_tag(data, &mut i)?;
                let radius = read_f32_tag(data, &mut i)?;
                let intensity = read_f32_tag(data, &mut i)?;
                point_lights.push(ScenePointLight {
                    id,
                    room_id,
                    pos,
                    color: [r, g, b, a],
                    radius,
                    intensity,
                });
            }
        }
        if let Some(rcnt) = read_i32(data, &mut i) {
            for _ in 0..rcnt {
                let rid = read_i32(data, &mut i)?;
                let mut sm_ids = Vec::new();
                let mut exit_names = Vec::new();
                let mut dm_names = Vec::new();
                let mut list_idx = 0usize;
                while data.get(i) == Some(&0x1c) {
                    i += 1;
                    let cnt = read_i32(data, &mut i)? as usize;
                    for _ in 0..cnt {
                        if data.get(i) == Some(&0x0d) {
                            let s = read_str(data, &mut i)?;
                            if list_idx == 2 {
                                exit_names.push(s);
                            } else if list_idx == 7 {
                                dm_names.push(s);
                            }
                        } else {
                            let v = read_i32(data, &mut i)?;
                            if list_idx == 0 {
                                sm_ids.push(v);
                            }
                        }
                    }
                    list_idx += 1;
                }
                let rname = read_str(data, &mut i)?;
                let _f = read_f32_tag(data, &mut i)?;
                for _ in 0..4 {
                    read_i32(data, &mut i)?;
                }
                rooms.push(SceneRoom {
                    id: rid,
                    name: rname,
                    static_meshes: sm_ids,
                    exits: exit_names,
                    dynamic_meshes: dm_names,
                    world_origin: [0.0, 0.0, 0.0],
                });
            }
        }
    }

    // Compute room world origins via portal graph BFS from start room (or room with first jumppoint)
    if !rooms.is_empty() {
        let start_room = jumppoints
            .iter()
            .find(|j| j.name.contains("StartRoom"))
            .or_else(|| {
                jumppoints
                    .iter()
                    .find(|j| j.name.contains("Start") || j.name.contains("Jumppoint"))
            })
            .map(|j| j.room_id)
            .unwrap_or(rooms[0].id);
        let mut origins: BTreeMap<i32, [f32; 3]> = BTreeMap::new();
        let mut queue = VecDeque::new();
        origins.insert(start_room, [0.0, 0.0, 0.0]);
        queue.push_back(start_room);
        while let Some(u) = queue.pop_front() {
            let u_orig = origins[&u];
            if let Some(room) = rooms.iter().find(|r| r.id == u) {
                for ex_name in &room.exits {
                    if let Some(ex) = exits.iter().find(|e| &e.name == ex_name) {
                        let v = ex.target_room_id;
                        if let std::collections::btree_map::Entry::Vacant(e) = origins.entry(v) {
                            // `ex.relative_offset` transforms room `u` local coords into room `v`:
                            // `P_v = P_u + rel` => `origin_v = origin_u - rel`
                            let v_orig = [
                                u_orig[0] - ex.relative_offset[0],
                                u_orig[1] - ex.relative_offset[1],
                                u_orig[2] - ex.relative_offset[2],
                            ];
                            e.insert(v_orig);
                            queue.push_back(v);
                        }
                    }
                }
            }
        }
        for r in &mut rooms {
            if let Some(&orig) = origins.get(&r.id) {
                r.world_origin = orig;
            }
            for &sm_id in &r.static_meshes {
                if let Some(sm) = static_meshes.iter_mut().find(|m| m.mesh_id == sm_id) {
                    sm.room_id = Some(r.id);
                    sm.world_origin = r.world_origin;
                }
            }
        }
    }

    // Apply FSM `C_Teleport("::Room::Jumppoint")` scripts to characters starting in `::Teleport`
    for fsm in &fsms {
        let all_acts = fsm
            .startup_actions
            .iter()
            .chain(fsm.states.iter())
            .chain(fsm.hooks.iter().flat_map(|(_, h)| h.iter()));
        for act in all_acts {
            let lower = act.to_ascii_lowercase();
            if let Some(arrow_pos) = lower.find("->c_teleport(") {
                let actor = &act[..arrow_pos].trim();
                let rest = &act[arrow_pos + 13..];
                let target = rest.trim_matches(|c: char| {
                    c == '(' || c == ')' || c == ';' || c == '"' || c.is_whitespace()
                });
                if let Some(jp) = jumppoints
                    .iter()
                    .find(|j| j.name.eq_ignore_ascii_case(target))
                {
                    if let Some(ch) = characters
                        .iter_mut()
                        .find(|c| c.name.eq_ignore_ascii_case(actor))
                    {
                        ch.room_id = jp.room_id;
                        ch.pos = jp.pos;
                    }
                }
            }
        }
    }

    Some(LdbScene {
        textures,
        materials,
        lightmaps,
        exits,
        static_meshes,
        jumppoints,
        fsms,
        characters,
        triggers,
        dynamic_meshes,
        items,
        point_lights,
        rooms,
        bytes_consumed: i,
    })
}

impl LdbScene {
    /// `::StartRoom::Jumppoint 00` when the node name is only `Jumppoint 00`.
    pub fn jumppoint_place(&self, jp: &SceneJumppoint) -> String {
        if jp.name.starts_with("::") {
            return jp.name.clone();
        }
        match self.rooms.iter().find(|r| r.id == jp.room_id) {
            Some(room) if !room.name.is_empty() => format!("{}::{}", room.name, jp.name),
            _ => jp.name.clone(),
        }
    }

    /// Index of `PlayerStartingPlace`, else the first StartRoom jumppoint.
    pub fn spawn_jumppoint_index(&self, place: Option<&str>) -> usize {
        if let Some(place) = place {
            if let Some(i) = self
                .jumppoints
                .iter()
                .position(|j| self.jumppoint_place(j).eq_ignore_ascii_case(place))
            {
                return i;
            }
        }
        self.jumppoints
            .iter()
            .position(|j| {
                let place = self.jumppoint_place(j);
                place.to_ascii_lowercase().contains("startroom")
                    && place.to_ascii_lowercase().contains("jumppoint")
            })
            .or_else(|| {
                self.jumppoints
                    .iter()
                    .position(|j| j.name.to_ascii_lowercase().contains("jumppoint"))
            })
            .unwrap_or(0)
    }
}

/// One animated dynamic object in the intro camera rig.
#[derive(Debug, Clone)]
struct FlyBone {
    /// Rest orientation. Column 0, 1, 2 are the local axes.
    mat: [f32; 9],
    start_pos: [f32; 3],
    end_pos: [f32; 3],
    duration: f32,
    /// Distance-along-segment samples. Empty means a linear blend.
    curve: Vec<f32>,
}

/// Startup flyover: ancestor bones run from t = 0, and `altitude::camera`
/// plays tracks 1..5 in order. Track 5's script is the graphic-novel handoff.
#[derive(Debug, Clone)]
pub struct CameraFlyover {
    ancestors: Vec<FlyBone>,
    segments: Vec<FlyBone>,
    room_origin: [f32; 3],
    forward: [f32; 3],
    pub duration: f32,
}

impl CameraFlyover {
    /// World position and look direction at `time` seconds.
    pub fn pose(&self, time: f32) -> ([f32; 3], [f32; 3]) {
        let mut origin = [0.0f32; 3];
        let mut mat = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        for (i, bone) in self.ancestors.iter().enumerate() {
            let local = bone.pos_at(time);
            if i == 0 {
                origin = local;
            } else {
                origin = xform_point(mat, origin, local);
            }
            mat = bone.mat;
        }
        let (seg, local_t) = segment_at(&self.segments, time);
        let local = seg.pos_at(local_t);
        let mut pos = xform_point(mat, origin, local);
        pos[0] += self.room_origin[0];
        pos[1] += self.room_origin[1];
        pos[2] += self.room_origin[2];
        (pos, self.forward)
    }
}

impl FlyBone {
    fn pos_at(&self, time: f32) -> [f32; 3] {
        let w = if self.duration <= 0.0 {
            1.0
        } else {
            curve_weight(&self.curve, self.duration, time.min(self.duration))
        };
        [
            self.start_pos[0] + (self.end_pos[0] - self.start_pos[0]) * w,
            self.start_pos[1] + (self.end_pos[1] - self.start_pos[1]) * w,
            self.start_pos[2] + (self.end_pos[2] - self.start_pos[2]) * w,
        ]
    }
}

fn xform_point(mat: [f32; 9], origin: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    [
        origin[0] + mat[0] * v[0] + mat[3] * v[1] + mat[6] * v[2],
        origin[1] + mat[1] * v[0] + mat[4] * v[1] + mat[7] * v[2],
        origin[2] + mat[2] * v[0] + mat[5] * v[1] + mat[8] * v[2],
    ]
}

fn curve_weight(curve: &[f32], duration: f32, time: f32) -> f32 {
    let linear = if duration <= 0.0 {
        1.0
    } else {
        (time / duration).clamp(0.0, 1.0)
    };
    let Some(&last) = curve.last() else {
        return linear;
    };
    if curve.len() < 2 || last.abs() < 1.0e-4 {
        return linear;
    }
    let f = linear * (curve.len() - 1) as f32;
    let i = (f.floor() as usize).min(curve.len() - 1);
    let j = (i + 1).min(curve.len() - 1);
    let frac = f - i as f32;
    let sample = curve[i] + (curve[j] - curve[i]) * frac;
    (sample / last).clamp(0.0, 1.0)
}

fn segment_at(segments: &[FlyBone], time: f32) -> (&FlyBone, f32) {
    let mut remain = time.max(0.0);
    for seg in segments {
        if remain <= seg.duration || seg.duration <= 0.0 {
            return (seg, remain.min(seg.duration.max(0.0)));
        }
        remain -= seg.duration;
    }
    let last = segments.last().expect("flyover has a segment");
    (last, last.duration)
}

fn bone_from_anim(mesh: &SceneDynamicMesh, anim: &SceneDynamicAnim) -> FlyBone {
    let delta = dist3(anim.end_pos, anim.start_pos);
    let mut curve = Vec::new();
    let mut best_err = f32::MAX;
    for samples in &anim.curves {
        let Some(&last) = samples.last() else {
            continue;
        };
        if samples.len() < 2 {
            continue;
        }
        let err = (last.abs() - delta).abs();
        if err < best_err {
            best_err = err;
            curve.clone_from(samples);
        }
    }
    if delta <= 0.05 || best_err > (delta * 0.05).max(0.5) {
        curve.clear();
    }
    FlyBone {
        mat: mesh.world_mat,
        start_pos: anim.start_pos,
        end_pos: anim.end_pos,
        duration: anim.duration,
        curve,
    }
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

fn parent_name(name: &str) -> Option<String> {
    let bare = name.strip_suffix(".DO").unwrap_or(name);
    let (parent, _) = bare.rsplit_once("::")?;
    Some(format!("{parent}.DO"))
}

/// The `startup_level` camera rig, if this scene has the rotator that ends in
/// `s_modeswitch(graphicnovel)`.
pub fn camera_flyover(scene: &LdbScene) -> Option<CameraFlyover> {
    let cam = scene.dynamic_meshes.iter().find(|m| {
        m.name.ends_with("altitude::camera.DO")
            && m.anims.iter().any(|a| {
                a.scripts.iter().any(|s| {
                    s.to_ascii_lowercase()
                        .contains("s_modeswitch(graphicnovel)")
                })
            })
    })?;
    let mut chain = vec![cam];
    while let Some(parent) = parent_name(&chain.last()?.name) {
        let Some(mesh) = scene.dynamic_meshes.iter().find(|m| m.name == parent) else {
            break;
        };
        chain.push(mesh);
    }
    chain.reverse();
    let camera = chain.pop()?;
    let ancestors = chain
        .iter()
        .filter_map(|mesh| {
            mesh.anims
                .iter()
                .find(|a| a.name == "1")
                .or_else(|| mesh.anims.first())
                .map(|anim| bone_from_anim(mesh, anim))
        })
        .collect::<Vec<_>>();
    if ancestors.is_empty() {
        return None;
    }
    let mut segments = Vec::new();
    for index in 1..=5 {
        let anim = camera.anims.iter().find(|a| a.name == index.to_string())?;
        segments.push(bone_from_anim(camera, anim));
    }
    let duration = segments.iter().map(|s| s.duration).sum();
    let room_origin = scene
        .rooms
        .iter()
        .find(|r| r.id == camera.room_id)
        .map(|r| r.world_origin)
        .unwrap_or([0.0; 3]);
    let z = [
        camera.world_mat[6],
        camera.world_mat[7],
        camera.world_mat[8],
    ];
    let len = (z[0] * z[0] + z[1] * z[1] + z[2] * z[2]).sqrt().max(1.0e-6);
    Some(CameraFlyover {
        ancestors,
        segments,
        room_origin,
        forward: [z[0] / len, z[1] / len, z[2] / len],
        duration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_uncompressed_24bit_tga() {
        let mut tga = vec![0u8; 18 + 3];
        tga[2] = 2; // uncompressed true-color
        tga[12] = 1; // width = 1
        tga[14] = 1; // height = 1
        tga[16] = 24; // 24 bpp
        tga[18] = 0x11; // B
        tga[19] = 0x22; // G
        tga[20] = 0x33; // R
        let (w, h, px) = decode_texture_blob(&tga).unwrap();
        assert_eq!((w, h), (1, 1));
        assert_eq!(px, vec![0x00332211]);
    }

    fn bare_scene(rooms: Vec<SceneRoom>, jumppoints: Vec<SceneJumppoint>) -> LdbScene {
        LdbScene {
            textures: Vec::new(),
            materials: BTreeMap::new(),
            lightmaps: Vec::new(),
            exits: Vec::new(),
            static_meshes: Vec::new(),
            jumppoints,
            fsms: Vec::new(),
            characters: Vec::new(),
            triggers: Vec::new(),
            dynamic_meshes: Vec::new(),
            items: Vec::new(),
            point_lights: Vec::new(),
            rooms,
            bytes_consumed: 0,
        }
    }

    #[test]
    fn spawn_uses_player_starting_place() {
        let scene = bare_scene(
            vec![
                SceneRoom {
                    id: 0,
                    name: "::startroom".into(),
                    static_meshes: Vec::new(),
                    exits: Vec::new(),
                    dynamic_meshes: Vec::new(),
                    world_origin: [0.0; 3],
                },
                SceneRoom {
                    id: 4,
                    name: "::Bathroom".into(),
                    static_meshes: Vec::new(),
                    exits: Vec::new(),
                    dynamic_meshes: Vec::new(),
                    world_origin: [0.0; 3],
                },
            ],
            vec![
                SceneJumppoint {
                    name: "Jumppoint 00".into(),
                    room_id: 4,
                    pos: [9.0, 0.0, 0.0],
                    jp_type: 0,
                },
                SceneJumppoint {
                    name: "Jumppoint 00".into(),
                    room_id: 0,
                    pos: [1.0, 0.0, 2.0],
                    jp_type: 0,
                },
            ],
        );
        let idx = scene.spawn_jumppoint_index(Some("::StartRoom::Jumppoint 00"));
        assert_eq!(idx, 1);
        assert_eq!(scene.jumppoints[idx].pos, [1.0, 0.0, 2.0]);
    }

    #[test]
    fn material_policy_matches_materials_txt() {
        let dummy = material_policy("dummy");
        assert!(!dummy.draw);
        assert!(!dummy.character_collides);

        let sky = material_policy("Skybox");
        assert!(!sky.draw);
        assert!(sky.character_collides);
        assert!(sky.skybox);

        let graffiti = material_policy("Graffiti");
        assert!(graffiti.draw);
        assert!(!graffiti.character_collides);
        assert!(!graffiti.writes_z);
        assert_eq!(graffiti.detail_offset, 1);

        let wood = material_policy("wood");
        assert!(wood.draw);
        assert!(wood.character_collides);
        assert!(wood.writes_z);
        assert_eq!(wood.detail_offset, 0);

        let pass = material_policy("NoCollision");
        assert!(pass.draw);
        assert!(!pass.character_collides);

        let ai = material_policy("ai_node_collision_nodraw");
        assert!(!ai.draw);
        assert!(!ai.character_collides);

        let shell = material_policy("CharacterCollision_NoDraw");
        assert!(!shell.draw);
        assert!(shell.character_collides);
    }

    #[test]
    fn material_surfaces_use_the_file_alpha_reference() {
        let text = "#define ID_ALPHAREFERENCE 15\n[Default]\nAlphaReference = ID_ALPHAREFERENCE;\nBlendedAlphaTest = TRUE;\n[Leaves]\nAlphaReference = 127;\nBlendedAlphaTest = FALSE;\n";
        let surfaces = parse_material_surfaces(text);
        assert_eq!(surfaces["Default"].alpha_reference, 15);
        assert!(surfaces["Default"].blended_alpha_test);
        assert_eq!(surfaces["Leaves"].alpha_reference, 127);
        assert!(!surfaces["Leaves"].blended_alpha_test);
        let path = std::path::Path::new("/tmp/omp-xdata-ref/data/database/materials.txt");
        if path.is_file() {
            let retail = parse_material_surfaces(&std::fs::read_to_string(path).unwrap());
            assert_eq!(retail["Leaves"].alpha_reference, 127);
            assert!(!retail["Leaves"].blended_alpha_test);
            assert_eq!(retail["Glass"].alpha_reference, 15);
            assert!(retail["Glass"].blended_alpha_test);
        }
    }

    fn actor(scripts: &[&str]) -> SceneCharacter {
        SceneCharacter {
            name: "actor".into(),
            skin: "lab_junkie".into(),
            room_id: 1,
            pos: [0.0; 3],
            startup_scripts: scripts.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn placed_house_actors_stay_put_until_combat() {
        let michelle = actor(&["::teleport::michelle->c_kill(1);"]);
        assert!(michelle.starts_dead());
        assert!(!michelle.pursues());

        let junkie = actor(&["::babys_room::e1->C_RemoveAllWeapons(beretta);"]);
        assert!(!junkie.starts_dead());
        assert!(!junkie.pursues());

        let combat = actor(&["::babys_room::e1->C_SetStateMachine(mobstercombat);"]);
        assert!(combat.pursues());
        assert!(!combat.starts_dead());
    }

    #[test]
    fn startup_level_reads_the_rotator_camera_tracks() {
        let path = std::path::Path::new(
            "/tmp/omp-xdata-ref/data/database/levels/front_end/Startup_level.ldb",
        );
        if !path.is_file() {
            return;
        }
        let bytes = std::fs::read(path).unwrap();
        let scene = parse_ldb_scene(&bytes).expect("startup scene");
        assert_eq!(scene.bytes_consumed, bytes.len());
        let jp = scene
            .jumppoints
            .iter()
            .find(|j| j.name == "::start::Jumppoint 00")
            .expect("jumppoint");
        assert_eq!(jp.pos, [-139.0, -67.5, -1149.0]);
        let cam = scene
            .dynamic_meshes
            .iter()
            .find(|m| m.name.ends_with("altitude::camera.DO"))
            .expect("camera");
        let track1 = cam.anims.iter().find(|a| a.name == "1").expect("track 1");
        assert_eq!(track1.duration, 7.0);
        assert!((track1.start_pos[0] + 0.07).abs() < 0.02);
        assert!((track1.start_pos[1] - 0.2).abs() < 0.02);
        assert!((track1.start_pos[2] - 1.56).abs() < 0.02);
        let fly = camera_flyover(&scene).expect("flyover");
        assert_eq!(fly.segments.len(), 5);
        assert!((fly.duration - 28.5).abs() < 0.05);
        let (pos, forward) = fly.pose(0.0);
        assert!((pos[0] + 40.57).abs() < 0.05);
        assert!((pos[1] + 58.55).abs() < 0.05);
        assert!((pos[2] + 34.44).abs() < 0.05);
        let flen =
            (forward[0] * forward[0] + forward[1] * forward[1] + forward[2] * forward[2]).sqrt();
        assert!((flen - 1.0).abs() < 0.01);
    }
}
