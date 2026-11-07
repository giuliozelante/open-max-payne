//! KF2 models as binary glTF 2.0.

use crate::{
    compose_world, invert_rt, mat_to_quat, sample_locals, AnimBone, BoneWeight, DecodedTexture,
    Kf2Anim, KfsMesh, SkdSkin,
};
use std::collections::BTreeMap;

/// Triangles of one material, with a vertex per distinct position and UV.
#[derive(Debug, Clone, PartialEq)]
pub struct Primitive {
    pub material: usize,
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Empty on a model with no skin.
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Joint {
    pub name: String,
    pub parent: Option<usize>,
    pub translation: [f32; 3],
    /// `[x, y, z, w]`.
    pub rotation: [f32; 4],
    /// Column-major, as glTF stores it.
    pub inverse_bind: [f32; 16],
}

/// One joint's keys in a clip, in glTF axes.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub joint: usize,
    /// Seconds, strictly increasing.
    pub times: Vec<f32>,
    pub translations: Vec<[f32; 3]>,
    pub rotations: Vec<[f32; 4]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub name: String,
    pub channels: Vec<Channel>,
}

/// A model in glTF axes.
#[derive(Debug, Clone)]
pub struct Model {
    pub name: String,
    pub primitives: Vec<Primitive>,
    pub joints: Vec<Joint>,
    pub textures: Vec<Option<DecodedTexture>>,
    pub animations: Vec<Animation>,
}

/// Max-FX is left-handed with a character facing +z, as `max_payne`'s `toe-l` and `hand-l` show.
/// Negating x keeps it facing +z, which is the front of a glTF asset.
fn mirror(p: [f32; 3]) -> [f32; 3] {
    [0.0 - p[0], p[1], p[2]]
}

fn mirror_rotation(m: [f32; 9]) -> [f32; 9] {
    let mut out = m;
    for i in [1, 2, 3, 6] {
        out[i] = 0.0 - m[i];
    }
    out
}

fn joints(anim: &Kf2Anim) -> Vec<Joint> {
    let locals = sample_locals(anim, 0.0);
    let world = compose_world(anim, &locals);
    let index = |name: &str| {
        anim.bones
            .iter()
            .position(|b| b.name.eq_ignore_ascii_case(name))
    };
    anim.bones
        .iter()
        .map(|bone| {
            let key = bone.name.to_ascii_lowercase();
            let (mat, pos) = locals[&key];
            let (world_mat, world_pos) = world.get(&key).copied().unwrap_or((mat, pos));
            let (r, t) = invert_rt(mirror_rotation(world_mat), mirror(world_pos));
            Joint {
                name: bone.name.clone(),
                parent: bone.parent.as_deref().and_then(index),
                translation: mirror(pos),
                rotation: mat_to_quat(mirror_rotation(mat)),
                inverse_bind: [
                    r[0], r[1], r[2], 0.0, r[3], r[4], r[5], 0.0, r[6], r[7], r[8], 0.0, t[0],
                    t[1], t[2], 1.0,
                ],
            }
        })
        .collect()
}

/// A vertex's joints and weights. A vertex with none rides the first joint, because glTF needs
/// weights that sum to one.
fn influences(weights: &[BoneWeight], joint_of: &[Option<u16>]) -> ([u16; 4], [f32; 4]) {
    let (mut joints, mut out) = ([0u16; 4], [0f32; 4]);
    let known = weights
        .iter()
        .filter_map(|w| Some((joint_of.get(w.bone).copied()??, w.weight)));
    for (k, (joint, weight)) in known.take(4).enumerate() {
        joints[k] = joint;
        out[k] = weight;
    }
    let sum: f32 = out.iter().sum();
    if sum > 0.0 {
        (joints, out.map(|w| w / sum))
    } else {
        ([0; 4], [1.0, 0.0, 0.0, 0.0])
    }
}

/// Convert `mesh`, skinned to the bind pose in `rig` when it has one.
pub fn model(mesh: &KfsMesh, rig: Option<(&SkdSkin, &Kf2Anim)>) -> Model {
    let joints = rig.map_or_else(Vec::new, |(_, anim)| joints(anim));
    let joint_of: Vec<Option<u16>> = rig.map_or_else(Vec::new, |(skin, _)| {
        let find = |name: &String| {
            let found = joints
                .iter()
                .position(|j| j.name.eq_ignore_ascii_case(name));
            found.map(|at| at as u16)
        };
        skin.bones.iter().map(find).collect()
    });
    let mut primitives: BTreeMap<usize, Primitive> = BTreeMap::new();
    let mut seen: BTreeMap<(usize, usize, [u32; 2]), u32> = BTreeMap::new();
    for face in &mesh.faces {
        let prim = primitives.entry(face.mat_index).or_insert(Primitive {
            material: face.mat_index,
            positions: Vec::new(),
            uvs: Vec::new(),
            joints: Vec::new(),
            weights: Vec::new(),
            indices: Vec::new(),
        });
        for corner in [0, 2, 1] {
            let (vertex, uv) = (face.indices[corner], face.uvs[corner]);
            let key = (face.mat_index, vertex, uv.map(f32::to_bits));
            let index = *seen.entry(key).or_insert_with(|| {
                prim.positions.push(mirror(mesh.vertices[vertex]));
                prim.uvs.push(uv);
                if let Some((skin, _)) = rig {
                    let weights = skin.weights.get(vertex).map_or(&[][..], Vec::as_slice);
                    let (joints, weights) = influences(weights, &joint_of);
                    prim.joints.push(joints);
                    prim.weights.push(weights);
                }
                prim.positions.len() as u32 - 1
            });
            prim.indices.push(index);
        }
    }
    Model {
        name: mesh.name.clone(),
        primitives: primitives.into_values().collect(),
        joints,
        textures: mesh.material_textures.clone(),
        animations: Vec::new(),
    }
}

/// `clip` as an animation of `model`'s joints.
pub fn animation(model: &Model, name: &str, clip: &Kf2Anim) -> Animation {
    let joint_of = |bone: &AnimBone| {
        let name = &bone.name;
        model
            .joints
            .iter()
            .position(|j| j.name.eq_ignore_ascii_case(name))
    };
    let channels = clip.bones.iter().filter_map(|bone| {
        let mut channel = Channel {
            joint: joint_of(bone)?,
            times: Vec::with_capacity(bone.keys.len()),
            translations: Vec::with_capacity(bone.keys.len()),
            rotations: Vec::with_capacity(bone.keys.len()),
        };
        let mut last_frame = None;
        for key in &bone.keys {
            if last_frame.is_some_and(|frame| key.frame <= frame) {
                continue;
            }
            last_frame = Some(key.frame);
            let mut rotation = mat_to_quat(mirror_rotation(key.mat));
            if let Some(previous) = channel.rotations.last() {
                let dot: f32 = (0..4).map(|k| previous[k] * rotation[k]).sum();
                if dot < 0.0 {
                    rotation = rotation.map(|v| 0.0 - v);
                }
            }
            channel.times.push(key.frame as f32 / bone.fps);
            channel.translations.push(mirror(key.pos));
            channel.rotations.push(rotation);
        }
        Some(channel)
    });
    Animation {
        name: name.to_string(),
        channels: channels.collect(),
    }
}

fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn numbers(values: &[f32]) -> String {
    let text: Vec<String> = values.iter().map(f32::to_string).collect();
    text.join(",")
}

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;

/// The binary chunk and the JSON that describes its contents.
#[derive(Default)]
struct Buffers {
    bin: Vec<u8>,
    views: Vec<String>,
    accessors: Vec<String>,
}

impl Buffers {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        let target = target.map_or(String::new(), |t| format!(",\"target\":{t}"));
        self.views.push(format!(
            r#"{{"buffer":0,"byteOffset":{},"byteLength":{}{target}}}"#,
            self.bin.len(),
            bytes.len()
        ));
        self.bin.extend_from_slice(bytes);
        self.bin.resize(self.bin.len().next_multiple_of(4), 0);
        self.views.len() - 1
    }

    fn accessor(
        &mut self,
        bytes: &[u8],
        target: Option<u32>,
        component: u32,
        count: usize,
        kind: &str,
        bounds: &str,
    ) -> usize {
        let view = self.view(bytes, target);
        self.accessors.push(format!(
            r#"{{"bufferView":{view},"componentType":{component},"count":{count},"type":"{kind}"{bounds}}}"#
        ));
        self.accessors.len() - 1
    }

    fn floats<const N: usize>(
        &mut self,
        values: &[[f32; N]],
        target: Option<u32>,
        kind: &str,
        bounds: &str,
    ) -> usize {
        let bytes: Vec<u8> = values
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        self.accessor(&bytes, target, FLOAT, values.len(), kind, bounds)
    }
}

fn position_bounds(positions: &[[f32; 3]]) -> String {
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in positions {
        for k in 0..3 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    format!(r#","min":[{}],"max":[{}]"#, numbers(&min), numbers(&max))
}

fn primitive_json(prim: &Primitive, buffers: &mut Buffers) -> String {
    let bounds = position_bounds(&prim.positions);
    let position = buffers.floats(&prim.positions, Some(ARRAY_BUFFER), "VEC3", &bounds);
    let uv = buffers.floats(&prim.uvs, Some(ARRAY_BUFFER), "VEC2", "");
    let mut attributes = format!(r#""POSITION":{position},"TEXCOORD_0":{uv}"#);
    if !prim.joints.is_empty() {
        let bytes: Vec<u8> = prim
            .joints
            .iter()
            .flatten()
            .flat_map(|j| j.to_le_bytes())
            .collect();
        let count = prim.joints.len();
        let target = Some(ARRAY_BUFFER);
        let joints = buffers.accessor(&bytes, target, UNSIGNED_SHORT, count, "VEC4", "");
        let weights = buffers.floats(&prim.weights, Some(ARRAY_BUFFER), "VEC4", "");
        attributes.push_str(&format!(r#","JOINTS_0":{joints},"WEIGHTS_0":{weights}"#));
    }
    let bytes: Vec<u8> = prim.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
    let target = Some(ELEMENT_ARRAY_BUFFER);
    let count = prim.indices.len();
    let indices = buffers.accessor(&bytes, target, UNSIGNED_INT, count, "SCALAR", "");
    format!(
        r#"{{"attributes":{{{attributes}}},"indices":{indices},"material":{}}}"#,
        prim.material
    )
}

fn animation_json(animation: &Animation, buffers: &mut Buffers) -> String {
    let (mut samplers, mut channels) = (Vec::new(), Vec::new());
    for channel in &animation.channels {
        let (first, last) = (channel.times[0], channel.times[channel.times.len() - 1]);
        let bounds = format!(r#","min":[{first}],"max":[{last}]"#);
        let times: Vec<[f32; 1]> = channel.times.iter().map(|&t| [t]).collect();
        let input = buffers.floats(&times, None, "SCALAR", &bounds);
        let translations = buffers.floats(&channel.translations, None, "VEC3", "");
        let rotations = buffers.floats(&channel.rotations, None, "VEC4", "");
        for (output, path) in [(translations, "translation"), (rotations, "rotation")] {
            channels.push(format!(
                r#"{{"sampler":{},"target":{{"node":{},"path":"{path}"}}}}"#,
                samplers.len(),
                channel.joint
            ));
            samplers.push(format!(
                r#"{{"input":{input},"output":{output},"interpolation":"LINEAR"}}"#
            ));
        }
    }
    format!(
        r#"{{"name":"{}","samplers":[{}],"channels":[{}]}}"#,
        escaped(&animation.name),
        samplers.join(","),
        channels.join(",")
    )
}

fn joint_json(index: usize, joint: &Joint, joints: &[Joint]) -> String {
    let children: Vec<String> = (0..joints.len())
        .filter(|&k| joints[k].parent == Some(index))
        .map(|k| k.to_string())
        .collect();
    let children = if children.is_empty() {
        String::new()
    } else {
        format!(r#","children":[{}]"#, children.join(","))
    };
    format!(
        r#"{{"name":"{}","translation":[{}],"rotation":[{}]{children}}}"#,
        escaped(&joint.name),
        numbers(&joint.translation),
        numbers(&joint.rotation)
    )
}

fn material_json(index: usize, texture: &Option<DecodedTexture>, images: usize) -> String {
    let (name, color) = match texture {
        Some(texture) => (
            escaped(&texture.path),
            format!(r#""baseColorTexture":{{"index":{images}}},"#),
        ),
        None => (format!("material_{index}"), String::new()),
    };
    format!(
        r#"{{"name":"{name}","pbrMetallicRoughness":{{{color}"metallicFactor":0,"roughnessFactor":1}}}}"#
    )
}

/// `model` as a `.glb` file.
pub fn to_glb(model: &Model) -> Vec<u8> {
    let mut buffers = Buffers::default();
    let primitives: Vec<String> = model
        .primitives
        .iter()
        .map(|prim| primitive_json(prim, &mut buffers))
        .collect();
    let (mut images, mut textures, mut materials) = (Vec::new(), Vec::new(), Vec::new());
    for (index, texture) in model.textures.iter().enumerate() {
        materials.push(material_json(index, texture, images.len()));
        if let Some(texture) = texture {
            let view = buffers.view(&png(texture), None);
            let source = images.len();
            images.push(format!(r#"{{"bufferView":{view},"mimeType":"image/png"}}"#));
            textures.push(format!(r#"{{"source":{source}}}"#));
        }
    }
    let mut nodes: Vec<String> = model
        .joints
        .iter()
        .enumerate()
        .map(|(index, joint)| joint_json(index, joint, &model.joints))
        .collect();
    let mesh_node = nodes.len();
    let skin = if model.joints.is_empty() {
        ""
    } else {
        r#","skin":0"#
    };
    nodes.push(format!(
        r#"{{"name":"{}","mesh":0{skin}}}"#,
        escaped(&model.name)
    ));
    let mut roots: Vec<String> = (0..model.joints.len())
        .filter(|&k| model.joints[k].parent.is_none())
        .map(|k| k.to_string())
        .collect();
    roots.push(mesh_node.to_string());

    let mut extra = String::new();
    if !model.joints.is_empty() {
        let inverse: Vec<[f32; 16]> = model.joints.iter().map(|j| j.inverse_bind).collect();
        let inverse = buffers.floats(&inverse, None, "MAT4", "");
        let list: Vec<String> = (0..model.joints.len()).map(|k| k.to_string()).collect();
        extra.push_str(&format!(
            r#","skins":[{{"joints":[{}],"inverseBindMatrices":{inverse}}}]"#,
            list.join(",")
        ));
    }
    if !model.animations.is_empty() {
        let animations: Vec<String> = model
            .animations
            .iter()
            .map(|animation| animation_json(animation, &mut buffers))
            .collect();
        extra.push_str(&format!(r#","animations":[{}]"#, animations.join(",")));
    }
    if !images.is_empty() {
        extra.push_str(&format!(
            r#","images":[{}],"textures":[{}]"#,
            images.join(","),
            textures.join(",")
        ));
    }
    let json = format!(
        concat!(
            r#"{{"asset":{{"version":"2.0","generator":"openkf2"}},"scene":0,"#,
            r#""scenes":[{{"nodes":[{roots}]}}],"nodes":[{nodes}],"#,
            r#""meshes":[{{"name":"{name}","primitives":[{primitives}]}}],"#,
            r#""materials":[{materials}],"accessors":[{accessors}],"#,
            r#""bufferViews":[{views}],"buffers":[{{"byteLength":{length}}}]{extra}}}"#
        ),
        roots = roots.join(","),
        nodes = nodes.join(","),
        name = escaped(&model.name),
        primitives = primitives.join(","),
        materials = materials.join(","),
        accessors = buffers.accessors.join(","),
        views = buffers.views.join(","),
        length = buffers.bin.len(),
        extra = extra,
    );
    let mut json = json.into_bytes();
    json.resize(json.len().next_multiple_of(4), b' ');
    let total = 12 + 8 + json.len() + 8 + buffers.bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(buffers.bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&buffers.bin);
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// An RGB PNG in stored deflate blocks: glTF takes PNG or JPEG, and retail also has TGA and PCX.
fn png(texture: &DecodedTexture) -> Vec<u8> {
    let width = texture.width as usize;
    let mut raw = Vec::with_capacity((width * 3 + 1) * texture.height as usize);
    for row in texture.pixels.chunks(width.max(1)) {
        raw.push(0);
        raw.extend(
            row.iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]),
        );
    }
    let mut zlib = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    while let Some(block) = blocks.next() {
        zlib.push(blocks.peek().is_none() as u8);
        zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&texture.width.to_be_bytes());
    header.extend_from_slice(&texture.height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    png_chunk(&mut out, b"IHDR", &header);
    png_chunk(&mut out, b"IDAT", &zlib);
    png_chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnimKeyframe, KfsFace};

    const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

    fn triangle() -> KfsMesh {
        KfsMesh {
            name: "Tri \"A\"".to_string(),
            vertices: vec![[1.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 3.0]],
            faces: vec![KfsFace {
                indices: [0, 1, 2],
                uvs: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
                mat_index: 0,
            }],
            material_textures: vec![None],
            parts: Vec::new(),
        }
    }

    fn bone(name: &str, parent: Option<&str>, mat: [f32; 9], pos: [f32; 3]) -> AnimBone {
        AnimBone {
            name: name.to_string(),
            parent: parent.map(str::to_string),
            duration: 1,
            fps: 30.0,
            keys: vec![AnimKeyframe { frame: 0, mat, pos }],
        }
    }

    fn rig() -> (SkdSkin, Kf2Anim) {
        let one = |bone, weight| BoneWeight { bone, weight };
        let skin = SkdSkin {
            name: "Tri".to_string(),
            bones: vec!["ARM".to_string(), "Root".to_string()],
            weights: vec![vec![one(1, 1.0)], vec![one(0, 0.25), one(1, 0.75)], vec![]],
        };
        let anim = Kf2Anim {
            bones: vec![
                bone("Root", None, IDENTITY, [1.0, 2.0, 3.0]),
                bone("Arm", Some("root"), IDENTITY, [4.0, 0.0, 0.0]),
            ],
        };
        (skin, anim)
    }

    #[test]
    fn left_handed_points_mirror_on_x_and_triangles_turn_over() {
        let model = model(&triangle(), None);
        let prim = &model.primitives[0];
        let at = |k: usize| prim.positions[prim.indices[k] as usize];
        assert_eq!(at(0), [-1.0, 0.0, 0.0]);
        assert_eq!(at(1), [0.0, 0.0, 3.0]);
        assert_eq!(at(2), [0.0, 2.0, 0.0]);
        let uv = |k: usize| prim.uvs[prim.indices[k] as usize];
        assert_eq!(uv(1), [0.0, 1.0]);
    }

    #[test]
    fn a_vertex_with_two_uvs_becomes_two_vertices() {
        let mut mesh = triangle();
        mesh.faces.push(KfsFace {
            indices: [0, 2, 1],
            uvs: [[0.5, 0.5], [0.0, 1.0], [1.0, 0.0]],
            mat_index: 0,
        });
        let model = model(&mesh, None);
        assert_eq!(model.primitives[0].positions.len(), 4);
        assert_eq!(model.primitives[0].indices.len(), 6);
    }

    #[test]
    fn each_material_gets_its_own_primitive() {
        let mut mesh = triangle();
        mesh.material_textures.push(None);
        mesh.faces.push(KfsFace {
            indices: [0, 2, 1],
            uvs: [[0.0; 2]; 3],
            mat_index: 1,
        });
        let model = model(&mesh, None);
        let materials: Vec<_> = model.primitives.iter().map(|p| p.material).collect();
        assert_eq!(materials, [0, 1]);
    }

    #[test]
    fn joints_keep_the_hierarchy_in_mirrored_axes() {
        let (skin, anim) = rig();
        let model = model(&triangle(), Some((&skin, &anim)));
        assert_eq!(model.joints[0].parent, None);
        assert_eq!(model.joints[1].parent, Some(0));
        assert_eq!(model.joints[0].translation, [-1.0, 2.0, 3.0]);
        assert_eq!(model.joints[1].translation, [-4.0, 0.0, 0.0]);
        assert_eq!(model.joints[1].rotation, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_rotation_mirrors_with_its_axis() {
        let quarter_turn_about_y = [0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0];
        let (skin, mut anim) = rig();
        anim.bones[0].keys[0].mat = quarter_turn_about_y;
        let model = model(&triangle(), Some((&skin, &anim)));
        let [x, y, z, w] = model.joints[0].rotation;
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!(x.abs() < 1e-6 && z.abs() < 1e-6, "{x} {z}");
        assert!(
            (y + half).abs() < 1e-6 && (w - half).abs() < 1e-6,
            "{y} {w}"
        );
    }

    #[test]
    fn inverse_bind_undoes_the_joints_world_position() {
        let (skin, anim) = rig();
        let model = model(&triangle(), Some((&skin, &anim)));
        let m = model.joints[1].inverse_bind;
        assert_eq!(&m[12..], [5.0, -2.0, -3.0, 1.0]);
        assert_eq!(m[0], 1.0);
    }

    #[test]
    fn skin_weights_follow_bone_names_not_skin_order() {
        let (skin, anim) = rig();
        let model = model(&triangle(), Some((&skin, &anim)));
        let prim = &model.primitives[0];
        let second = prim.indices[2] as usize;
        assert_eq!(prim.joints[second], [1, 0, 0, 0]);
        assert_eq!(prim.weights[second], [0.25, 0.75, 0.0, 0.0]);
    }

    #[test]
    fn a_vertex_with_no_weights_rides_the_first_joint() {
        let (skin, anim) = rig();
        let model = model(&triangle(), Some((&skin, &anim)));
        let prim = &model.primitives[0];
        let third = prim.indices[1] as usize;
        assert_eq!(prim.joints[third], [0, 0, 0, 0]);
        assert_eq!(prim.weights[third], [1.0, 0.0, 0.0, 0.0]);
    }

    fn key(frame: u32, mat: [f32; 9], pos: [f32; 3]) -> AnimKeyframe {
        AnimKeyframe { frame, mat, pos }
    }

    fn walk() -> Kf2Anim {
        let mut arm = bone("ARM", Some("Root"), IDENTITY, [1.0, 0.0, 0.0]);
        arm.fps = 10.0;
        arm.keys.push(key(5, IDENTITY, [2.0, 0.0, 3.0]));
        Kf2Anim {
            bones: vec![arm, bone("Tail", None, IDENTITY, [0.0; 3])],
        }
    }

    #[test]
    fn clip_keys_become_seconds_in_mirrored_axes() {
        let (skin, anim) = rig();
        let model = model(&triangle(), Some((&skin, &anim)));
        let clip = animation(&model, "walk", &walk());
        assert_eq!(clip.name, "walk");
        let channel = &clip.channels[0];
        assert_eq!(channel.joint, 1);
        assert_eq!(channel.times, [0.0, 0.5]);
        assert_eq!(channel.translations[1], [-2.0, 0.0, 3.0]);
        assert_eq!(channel.rotations[1], [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_clip_bone_the_model_lacks_is_left_out() {
        let (skin, anim) = rig();
        let model = model(&triangle(), Some((&skin, &anim)));
        assert_eq!(animation(&model, "walk", &walk()).channels.len(), 1);
    }

    #[test]
    fn two_keys_on_one_frame_keep_the_first() {
        let (skin, anim) = rig();
        let model = model(&triangle(), Some((&skin, &anim)));
        let mut clip = walk();
        clip.bones[0].keys.push(key(5, IDENTITY, [9.0; 3]));
        let channel = &animation(&model, "walk", &clip).channels[0];
        assert_eq!(channel.times, [0.0, 0.5]);
        assert_eq!(channel.translations[1], [-2.0, 0.0, 3.0]);
    }

    #[test]
    fn neighbouring_rotations_stay_on_one_side_of_the_sphere() {
        let (skin, anim) = rig();
        let model = model(&triangle(), Some((&skin, &anim)));
        let half_turn_about_x = [1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, -1.0];
        let mut clip = walk();
        clip.bones[0].keys[0].mat = half_turn_about_x;
        clip.bones[0].keys[1].mat = [1.0, 0.0, 0.0, 0.0, -1.0, 0.01, 0.0, -0.01, -1.0];
        let rotations = &animation(&model, "walk", &clip).channels[0].rotations;
        let dot: f32 = (0..4).map(|k| rotations[0][k] * rotations[1][k]).sum();
        assert!(dot > 0.0, "{rotations:?}");
    }

    #[test]
    fn a_glb_animation_targets_translation_and_rotation_of_its_joint() {
        let (skin, anim) = rig();
        let mut model = model(&triangle(), Some((&skin, &anim)));
        model.animations = vec![animation(&model, "walk", &walk())];
        let glb = to_glb(&model);
        let json = std::str::from_utf8(&glb[20..20 + u32_at(&glb, 12) as usize]).unwrap();
        assert!(json.contains(r#""animations":[{"name":"walk""#), "{json}");
        assert!(
            json.contains(r#""target":{"node":1,"path":"translation"}"#),
            "{json}"
        );
        assert!(
            json.contains(r#""target":{"node":1,"path":"rotation"}"#),
            "{json}"
        );
        assert!(json.contains(r#""interpolation":"LINEAR""#), "{json}");
        assert!(json.contains(r#""min":[0],"max":[0.5]"#), "{json}");
    }

    fn u32_at(data: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn a_glb_has_a_header_then_json_then_binary_chunks() {
        let glb = to_glb(&model(&triangle(), None));
        assert_eq!(&glb[..4], b"glTF");
        assert_eq!(u32_at(&glb, 4), 2);
        assert_eq!(u32_at(&glb, 8) as usize, glb.len());
        let json_len = u32_at(&glb, 12) as usize;
        assert_eq!(json_len % 4, 0);
        assert_eq!(&glb[16..20], b"JSON");
        let bin = 20 + json_len;
        assert_eq!(&glb[bin + 4..bin + 8], b"BIN\0");
        assert_eq!(bin + 8 + u32_at(&glb, bin) as usize, glb.len());
    }

    #[test]
    fn json_escapes_names_and_lists_position_bounds() {
        let glb = to_glb(&model(&triangle(), None));
        let json = std::str::from_utf8(&glb[20..20 + u32_at(&glb, 12) as usize]).unwrap();
        assert!(json.contains(r#""name":"Tri \"A\"""#), "{json}");
        assert!(json.contains(r#""min":[-1,0,0],"max":[0,2,3]"#), "{json}");
        assert!(!json.contains(r#""skins""#), "{json}");
    }

    #[test]
    fn a_skinned_glb_names_its_joints_and_attributes() {
        let (skin, anim) = rig();
        let glb = to_glb(&model(&triangle(), Some((&skin, &anim))));
        let json = std::str::from_utf8(&glb[20..20 + u32_at(&glb, 12) as usize]).unwrap();
        assert!(json.contains(r#""skins":[{"joints":[0,1]"#), "{json}");
        assert!(json.contains(r#""children":[1]"#), "{json}");
        assert!(
            json.contains("JOINTS_0") && json.contains("WEIGHTS_0"),
            "{json}"
        );
    }

    #[test]
    fn textures_are_embedded_as_png() {
        let mut mesh = triangle();
        mesh.material_textures[0] = Some(DecodedTexture {
            path: "skin.jpg".to_string(),
            flag: 0,
            width: 1,
            height: 1,
            pixels: vec![0x0011_2233],
        });
        let glb = to_glb(&model(&mesh, None));
        let json = std::str::from_utf8(&glb[20..20 + u32_at(&glb, 12) as usize]).unwrap();
        assert!(json.contains(r#""mimeType":"image/png""#), "{json}");
        assert!(json.contains(r#""baseColorTexture":{"index":0}"#), "{json}");
        assert!(glb.windows(8).any(|w| w == b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn png_checksums_match_their_published_values() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn a_png_stores_rows_behind_a_filter_byte() {
        let png = png(&DecodedTexture {
            path: String::new(),
            flag: 0,
            width: 2,
            height: 1,
            pixels: vec![0x0011_2233, 0x0044_5566],
        });
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 2);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 1);
        let row = [0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
        assert!(png.windows(row.len()).any(|w| w == row));
    }
}
