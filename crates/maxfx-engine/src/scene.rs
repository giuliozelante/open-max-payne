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
    /// `Some(AlphaReference)` when the texture carries an alpha mask (`P_Map(colour, alpha)`,
    /// `0x5dc3d8`); its alpha is in the top byte. `None` is opaque.
    pub alpha_reference: Option<u8>,
    /// Masked texels blend by their alpha after the alpha test (`materials.txt`
    /// `BlendedAlphaTest`, surface record `+0x20`); `Leaves` stays a hard cut-out.
    pub blended: bool,
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
    /// `.ldb` list 1 → DMA `+0x8c` → runtime anim `+0x9`: sent when `DO_Animate` starts (`0x6f40aa`).
    pub start_scripts: Vec<String>,
    /// `.ldb` list 2 → DMA `+0x9c` → runtime anim `+0x19`: sent when a reversed track reaches 0.
    pub reverse_end_scripts: Vec<String>,
    /// `.ldb` list 3 → DMA `+0xac` → runtime anim `+0x29`: sent when the track reaches its end.
    pub end_scripts: Vec<String>,
    /// Two sample channels. Each sample is a tagged `f32` (`0x09`). One channel
    /// runs along the start→end distance; the other is a 0..1 clock.
    pub curves: [Vec<f32>; 2],
}

/// Retail DO playback fields beside the playhead (`X_LevelRuntimeDynamicObject`).
#[derive(Debug, Clone, Default)]
pub struct DoAnimState {
    /// `+0x483 != 0`: a track is advancing.
    pub playing: bool,
    /// `+0x4a7`, toggled by `DO_InvertAnimation`.
    pub reverse: bool,
    /// `+0x4a3`, set by `DO_PauseAnimation(t)`: > 0 counts down, < 0 holds forever.
    pub hold: f32,
    /// `+0x487` / `+0x49f`: track and playhead saved by `DO_StopAnimation`.
    pub saved: Option<(String, f32)>,
    /// Start-list messages waiting for the next step (retail sends them lagged).
    pub queued: Vec<String>,
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
    /// Set by `DO_Hide`. Parsed meshes start visible.
    pub hidden: bool,
    /// Set by `DO_EnableCollisions` (DO `+0x4ae`). When false the player walks through the mesh.
    pub collisions: bool,
    /// Set by `DO_SetHitpoints`. `None` means the mesh is not damaged by shots.
    pub hitpoints: Option<i32>,
    pub polys: Vec<ScenePoly>,
    pub anims: Vec<SceneDynamicAnim>,
    /// Track started by `DO_Animate`. Held at the end pose after it finishes.
    pub active_track: Option<String>,
    pub track_time: f32,
    pub track_duration: f32,
    pub anim_state: DoAnimState,
}

impl SceneDynamicMesh {
    /// The track whose name is `open`, or the first `open*` track.
    pub fn opening_anim(&self) -> Option<&SceneDynamicAnim> {
        self.anims
            .iter()
            .find(|anim| anim.name.eq_ignore_ascii_case("open"))
            .or_else(|| {
                self.anims
                    .iter()
                    .find(|anim| anim.name.to_ascii_lowercase().starts_with("open"))
            })
    }

    /// Start `track_name`. Unknown names do nothing.
    pub fn play_track(&mut self, track_name: &str) -> bool {
        let Some(track) = self
            .anims
            .iter()
            .find(|anim| anim.name.eq_ignore_ascii_case(track_name))
        else {
            return false;
        };
        // Apply 0x6f3d20 zeroes playhead and hold; reverse (+0x4a7) is left as it was.
        self.track_duration = track.duration;
        self.track_time = 0.0;
        self.active_track = Some(track.name.clone());
        let start = track.start_scripts.clone();
        let state = &mut self.anim_state;
        state.playing = true;
        state.hold = 0.0;
        state.queued.extend(start);
        true
    }

    /// One `0x6f34c0` tick. Returns the end list (`+0x29`, or `+0x19` when reversed)
    /// on the frame the track runs out.
    pub fn step_animation(&mut self, dt: f32) -> Option<Vec<String>> {
        let state = &mut self.anim_state;
        if !state.playing {
            return None;
        }
        let dir = if state.reverse { -1.0 } else { 1.0 };
        if state.hold == 0.0 {
            self.track_time += dir * dt;
        } else if state.hold > 0.0 {
            state.hold -= dt;
            if state.hold < 0.0 {
                self.track_time -= dir * state.hold;
                state.hold = 0.0;
            }
        }
        let reached_end = self.track_duration <= self.track_time;
        if !reached_end && self.track_time >= 0.0 {
            return None;
        }
        self.track_time = if reached_end {
            self.track_duration
        } else {
            0.0
        };
        state.playing = false;
        state.hold = 0.0;
        state.reverse = false;
        let name = self.active_track.as_ref()?;
        let anim = self
            .anims
            .iter()
            .find(|anim| anim.name.eq_ignore_ascii_case(name))?;
        Some(if reached_end {
            anim.end_scripts.clone()
        } else {
            anim.reverse_end_scripts.clone()
        })
    }

    /// `DO_InvertAnimation` (`0x6f40e0`): flip direction while a track plays.
    fn invert_animation(&mut self) {
        if self.anim_state.playing {
            self.anim_state.reverse = !self.anim_state.reverse;
        }
    }

    /// `DO_PauseAnimation(t)` (`0x6f4100`): set the hold time while a track plays.
    fn pause_animation(&mut self, hold: f32) {
        if self.anim_state.playing {
            self.anim_state.hold = hold;
        }
    }

    /// `DO_StopAnimation` (`0x6f4120`): save track and playhead, stop advancing.
    /// The pose stays where it was.
    fn stop_animation(&mut self) {
        if self.anim_state.playing {
            if let Some(name) = self.active_track.clone() {
                self.anim_state.saved = Some((name, self.track_time));
            }
            self.anim_state.playing = false;
        }
    }

    /// `DO_ResumeAnimation` (`0x6f4150`): restore what Stop saved, with no hold.
    fn resume_animation(&mut self) {
        let Some((name, time)) = self.anim_state.saved.take() else {
            return;
        };
        if let Some(duration) = self
            .anims
            .iter()
            .find(|anim| anim.name == name)
            .map(|anim| anim.duration)
        {
            self.track_duration = duration;
        }
        self.active_track = Some(name);
        self.track_time = time;
        self.anim_state.playing = true;
        self.anim_state.hold = 0.0;
    }

    /// World pose ignoring the parent's motion: the track-local pose `L`
    /// carried into the rest frame, `L · L0⁻¹ · W0` (`0x6e1a00`). At rest this is `W0`.
    pub fn posed_local(&self) -> ([f32; 9], [f32; 3]) {
        let rest = (self.local_start_mat, self.local_start_pos);
        let world = (self.world_mat, self.world_pos);
        compose_pose(compose_pose(self.track_local(), invert_pose(rest)), world)
    }

    /// `L`: the active `DO_Animate` track, the toggled opening pose, or the rest pose.
    fn track_local(&self) -> ([f32; 9], [f32; 3]) {
        if let Some(name) = &self.active_track {
            if let Some(anim) = self
                .anims
                .iter()
                .find(|anim| anim.name.eq_ignore_ascii_case(name))
            {
                return pose_on_track(self.track_time, anim);
            }
        }
        match self.opening_anim() {
            Some(anim) if self.toggled => (anim.end_mat, anim.end_pos),
            _ => (self.local_start_mat, self.local_start_pos),
        }
    }
}

fn pose_on_track(track_time: f32, anim: &SceneDynamicAnim) -> ([f32; 9], [f32; 3]) {
    let time = track_time.min(anim.duration.max(0.0));
    let (w, t) = track_weights(
        &anim.curves,
        anim.start_pos,
        anim.end_pos,
        anim.duration,
        time,
    );
    let pos = [
        anim.start_pos[0] + (anim.end_pos[0] - anim.start_pos[0]) * w,
        anim.start_pos[1] + (anim.end_pos[1] - anim.start_pos[1]) * w,
        anim.start_pos[2] + (anim.end_pos[2] - anim.start_pos[2]) * w,
    ];
    let mat = if time <= 0.0 {
        anim.start_mat
    } else if time >= anim.duration {
        anim.end_mat
    } else {
        crate::kf2::quat_to_mat(crate::kf2::quat_slerp(
            crate::kf2::mat_to_quat(anim.start_mat),
            crate::kf2::mat_to_quat(anim.end_mat),
            t,
        ))
    };
    (mat, pos)
}

/// Apply `first`, then `then` (retail row-vector `first · then`).
fn compose_pose(first: ([f32; 9], [f32; 3]), then: ([f32; 9], [f32; 3])) -> ([f32; 9], [f32; 3]) {
    let mut mat = [0.0f32; 9];
    for c in 0..3 {
        let col = xform_point(
            then.0,
            [0.0; 3],
            [first.0[c * 3], first.0[c * 3 + 1], first.0[c * 3 + 2]],
        );
        mat[c * 3..c * 3 + 3].copy_from_slice(&col);
    }
    (mat, xform_point(then.0, then.1, first.1))
}

fn invert_pose((m, p): ([f32; 9], [f32; 3])) -> ([f32; 9], [f32; 3]) {
    let det = m[0] * (m[4] * m[8] - m[7] * m[5]) - m[3] * (m[1] * m[8] - m[7] * m[2])
        + m[6] * (m[1] * m[5] - m[4] * m[2]);
    if det.abs() < 1.0e-12 {
        return (
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            [-p[0], -p[1], -p[2]],
        );
    }
    let inv = [
        (m[4] * m[8] - m[7] * m[5]) / det,
        (m[7] * m[2] - m[1] * m[8]) / det,
        (m[1] * m[5] - m[4] * m[2]) / det,
        (m[6] * m[5] - m[3] * m[8]) / det,
        (m[0] * m[8] - m[6] * m[2]) / det,
        (m[3] * m[2] - m[0] * m[5]) / det,
        (m[3] * m[7] - m[6] * m[4]) / det,
        (m[6] * m[1] - m[0] * m[7]) / det,
        (m[0] * m[4] - m[3] * m[1]) / det,
    ];
    let t = xform_point(inv, [0.0; 3], p);
    (inv, [-t[0], -t[1], -t[2]])
}

/// Retail sampler `0x5ff100`: translation lerp weight is the distance curve
/// (`+0xbc`) times `1/|end - start|` (`+0x88`); slerp `t` is the second curve
/// (`+0xd8`) raw. Neither is clamped. An empty curve falls back to linear
/// time (HYPOTHESIS: retail leaves the output unwritten).
fn track_weights(
    curves: &[Vec<f32>; 2],
    start: [f32; 3],
    end: [f32; 3],
    duration: f32,
    time: f32,
) -> (f32, f32) {
    if time <= 0.0 {
        return (0.0, 0.0);
    }
    if time >= duration {
        return (1.0, 1.0);
    }
    let u = time / duration;
    let dist = dist3(end, start);
    let w = match sample_curve(&curves[0], u) {
        Some(d) if dist > 0.0 => d / dist,
        Some(_) => 0.0,
        None => u,
    };
    (w, sample_curve(&curves[1], u).unwrap_or(u))
}

/// Uniform piecewise-linear curve sample at `u` in `0..1` (`0x783450`).
fn sample_curve(curve: &[f32], u: f32) -> Option<f32> {
    match curve.len() {
        0 => None,
        1 => Some(curve[0]),
        n => {
            let f = u * (n - 1) as f32;
            let k = (f.floor() as usize).min(n - 2);
            let frac = f - k as f32;
            Some(curve[k] * (1.0 - frac) + curve[k + 1] * frac)
        }
    }
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
    /// Orientation of the first transform (columns, like `SceneDynamicMesh::world_mat`).
    pub mat: [f32; 9],
    pub jp_type: i32,
}

impl SceneJumppoint {
    /// Player yaw (forward = `(sin, 0, cos)`) along the jumppoint's +z axis, the same forward
    /// convention as the cutscene cameras.
    pub fn yaw(&self) -> f32 {
        self.mat[6].atan2(self.mat[8])
    }
}

#[derive(Debug, Clone)]
pub struct SceneFsm {
    pub name: String,
    pub room_id: i32,
    pub pos: [f32; 3],
    pub states: Vec<String>,
    pub default_state: String,
    pub startup_actions: Vec<String>,
    /// Every named script of the record (states, commands, events), in file order.
    pub hooks: Vec<FsmScript>,
    /// State scripts (`.ldb` map 2, runtime `+0x441`): `FSM_Switch` runs them.
    pub state_scripts: Vec<FsmScript>,
    /// Commands (`.ldb` map 3, runtime `+0x431`): `FSM_Send` runs them.
    pub commands: Vec<FsmScript>,
    /// Current state (runtime `+0x365`); starts at `default_state`.
    pub state: String,
}

/// `(name, calls with no state, per-state calls in file order)`.
pub type FsmScript = (String, Vec<String>, Vec<(String, Vec<String>)>);

#[derive(Debug, Clone)]
pub struct SceneCharacter {
    pub name: String,
    pub skin: String,
    pub room_id: i32,
    pub pos: [f32; 3],
    pub startup_scripts: Vec<String>,
    /// MaxED `OnDeath` tab (`.ldb` list 2 of 4): sent when the character dies.
    pub on_death: Vec<String>,
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
    pub enabled: bool,
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

#[derive(Debug, Clone, Default)]
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
    /// Category name → SharedDB text (`dynamic.txt` and the same block shape).
    pub sound_catalogs: BTreeMap<String, String>,
    /// Sounds a verb asked to play. Dispatch only enqueues them.
    pub sound_queue: Vec<QueuedSound>,
    /// Emitters whose sounds `A_StopAll3DSounds` stopped since the last frame.
    pub sound_stops: Vec<String>,
    /// Last `maxpayne_gamemode->A_PlayMusic(cue)` not yet played; `""` stops the music.
    pub music_request: Option<String>,
    /// `camerapaths.txt` blocks `player->CAM_AnimateInPlace` asked for, oldest first.
    pub camera_shakes: Vec<String>,
    /// `(block, character)` of each `CAM_AnimateParented`, oldest first.
    pub death_cams: Vec<(String, String)>,
    /// Particle effects `PS_StartEffect` started. Dispatch only enqueues them.
    pub effect_queue: Vec<QueuedEffect>,
    /// Emitters whose effects `PS_StopAllEmissions` stopped since the last frame.
    pub effect_stops: Vec<String>,
    /// Dynamic mesh the view is attached to by `CAM_AttachToDO`, if any.
    pub attached_camera: Option<usize>,
    /// Cleared by `GM_SetPlayerControls(false)` during cutscenes.
    pub player_controls: bool,
    /// Full-screen colour from `MPHM_FadeToColor`, if any.
    pub screen_fade: Option<ScreenFade>,
    /// `levels.txt` `AmbientColor` (0..255 RGB): ambient light on characters. Black until set.
    pub ambient: [f32; 3],
    /// Last `S_ModeSwitch` request (`graphicnovel` ends the startup intro).
    pub mode_switch: Option<String>,
    /// Distance fog now: `levels.txt` at load, then `GM_ChangeFog`. `None` = no fog.
    pub fog: Option<LevelFog>,
    /// `GM_ChangeFog` with a fade time: `(from, to, seconds, elapsed)`.
    pub fog_fade: Option<(LevelFog, LevelFog, f32, f32)>,
}

/// Linear distance fog: `levels.txt` `FogColor` / `FogStart` / `FogEnd` (when `Fogging` is on)
/// or `GM_ChangeFog(0xRRGGBB, front, back, seconds)`. Colour channels are 0..255.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelFog {
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
}

/// `MPHM_FadeToColor(from, to, seconds)`: colours are `0xRRGGBBAA`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenFade {
    pub from: [u8; 4],
    pub to: [u8; 4],
    pub duration: f32,
    pub elapsed: f32,
}

impl ScreenFade {
    /// `[r, g, b, a]` now, held at `to` once the fade is over.
    pub fn color(&self) -> [u8; 4] {
        let t = if self.duration > 0.0 {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        } else {
            1.0
        };
        std::array::from_fn(|i| {
            let (a, b) = (f32::from(self.from[i]), f32::from(self.to[i]));
            (a + (b - a) * t).round() as u8
        })
    }
}

/// One `A_Play3DSound` accepted by a mesh or trigger that this scene actually stores.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedSound {
    pub category: String,
    pub cue: String,
    pub filename: Option<String>,
    /// The mesh or trigger that plays it, so `A_StopAll3DSounds` can stop it.
    pub emitter: String,
    /// `Looping = YES`: plays until its emitter is stopped or the level ends.
    pub looping: bool,
    /// Distance model of a `3DSound = YES` cue; `None` plays at full volume.
    pub range: Option<crate::SoundRange>,
}

/// One `PS_StartEffect(effect, ...)` on a mesh, trigger or FSM this scene stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedEffect {
    /// `particles.txt` block name.
    pub effect: String,
    /// Full name of the object that emits it.
    pub emitter: String,
}

/// One `Receiver->Verb(args);` call stored on an anim, FSM, or trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message<'a> {
    pub receiver: &'a str,
    pub verb: &'a str,
    pub args: Vec<&'a str>,
}

impl<'a> Message<'a> {
    pub fn parse(raw: &'a str) -> Option<Self> {
        let trimmed = raw.trim().trim_end_matches(';').trim();
        if trimmed.is_empty() {
            return None;
        }
        let (receiver, call_part) = if let Some((receiver, call)) = trimmed.split_once("->") {
            (receiver.trim(), call.trim())
        } else if let Some((receiver, call)) = trimmed.split_once(' ') {
            (receiver.trim(), call.trim())
        } else {
            ("this", trimmed)
        };
        if receiver.is_empty() || call_part.is_empty() {
            return None;
        }
        let (verb, args) = if let Some((verb, args)) = call_part.split_once('(') {
            let body = args.trim_end_matches(')').trim();
            let parsed = if body.is_empty() {
                Vec::new()
            } else {
                body.split(',')
                    .map(|part| part.trim().trim_matches('"'))
                    .filter(|part| !part.is_empty())
                    .collect()
            };
            (verb.trim(), parsed)
        } else {
            (call_part.trim(), Vec::new())
        };
        if verb.is_empty() {
            None
        } else {
            Some(Self {
                receiver,
                verb,
                args,
            })
        }
    }
}

/// Script boolean argument. Retail scripts mix `TRUE` and `true` (the startup intro sends both
/// `frame.DO->DO_Hide(true)` and `cam_fade_1.DO->DO_Hide(TRUE)` from one track end).
fn script_bool(arg: Option<&str>) -> Option<bool> {
    let arg = arg?.trim();
    if arg == "1" || arg.eq_ignore_ascii_case("true") {
        Some(true)
    } else if arg == "0" || arg.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

impl LdbScene {
    /// Masked materials take their category's `AlphaReference` from `materials.txt`.
    pub fn apply_material_surfaces(&mut self, surfaces: &BTreeMap<String, MaterialSurface>) {
        for material in self.materials.values_mut() {
            if material.alpha_reference.is_none() {
                continue;
            }
            if let Some((_, surface)) = surfaces
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&material.category))
            {
                material.alpha_reference = Some(surface.alpha_reference.clamp(0, 255) as u8);
                material.blended = surface.blended_alpha_test;
            }
        }
    }

    /// World pose of dynamic mesh `index`, following its parent's motion:
    /// `posed_local · W0(parent)⁻¹ · world(parent)` (`0x6e1a00`, parent `[+0x189]`).
    /// Room origin is not added.
    pub fn dynamic_world_pose(&self, index: usize) -> ([f32; 9], [f32; 3]) {
        let mesh = &self.dynamic_meshes[index];
        let own = mesh.posed_local();
        let Some(pi) = parent_name(&mesh.name)
            .and_then(|p| self.dynamic_meshes.iter().position(|m| m.name == p))
        else {
            return own;
        };
        let parent = &self.dynamic_meshes[pi];
        let rest = invert_pose((parent.world_mat, parent.world_pos));
        compose_pose(compose_pose(own, rest), self.dynamic_world_pose(pi))
    }

    /// Match the receiver as stored, then the same path with `.DO` appended.
    pub fn resolve_mesh_mut<'a>(
        &'a mut self,
        context_name: &str,
        receiver: &str,
    ) -> Option<&'a mut SceneDynamicMesh> {
        let index = self.resolve_mesh_index(context_name, receiver)?;
        Some(&mut self.dynamic_meshes[index])
    }

    fn resolve_mesh_index(&self, context_name: &str, receiver: &str) -> Option<usize> {
        let find = |name: &str| {
            self.dynamic_meshes
                .iter()
                .position(|mesh| mesh.name.eq_ignore_ascii_case(name))
        };
        if let Some(index) = find(receiver) {
            return Some(index);
        }
        let target = mesh_target_name(context_name, receiver)?;
        if target.eq_ignore_ascii_case(receiver) {
            return None;
        }
        find(&target)
    }

    /// Run one stored message. Unknown verbs do nothing.
    pub fn dispatch_message(&mut self, context_name: &str, raw_msg: &str) -> bool {
        let Some(msg) = Message::parse(raw_msg) else {
            return false;
        };
        match msg.verb.to_ascii_lowercase().as_str() {
            "do_animate" => {
                let Some(track_name) = msg.args.first().copied() else {
                    return false;
                };
                self.resolve_mesh_mut(context_name, msg.receiver)
                    .is_some_and(|mesh| mesh.play_track(track_name))
            }
            "do_invertanimation" | "do_stopanimation" | "do_resumeanimation" => {
                let verb = msg.verb.to_ascii_lowercase();
                self.resolve_mesh_mut(context_name, msg.receiver)
                    .is_some_and(|mesh| {
                        match verb.as_str() {
                            "do_invertanimation" => mesh.invert_animation(),
                            "do_stopanimation" => mesh.stop_animation(),
                            _ => mesh.resume_animation(),
                        }
                        true
                    })
            }
            "do_pauseanimation" => {
                let Some(hold) = msg.args.first().and_then(|arg| arg.parse::<f32>().ok()) else {
                    return false;
                };
                self.resolve_mesh_mut(context_name, msg.receiver)
                    .is_some_and(|mesh| {
                        mesh.pause_animation(hold);
                        true
                    })
            }
            "t_enable" => {
                let Some(enabled) = script_bool(msg.args.first().copied()) else {
                    return false;
                };
                self.resolve_trigger_mut(context_name, msg.receiver)
                    .is_some_and(|trigger| {
                        trigger.enabled = enabled;
                        true
                    })
            }
            "fsm_switch" => {
                let Some(state) = msg.args.first().copied() else {
                    return false;
                };
                self.fsm_switch(msg.receiver, state)
            }
            "fsm_send" => {
                let Some(command) = msg.args.first().copied() else {
                    return false;
                };
                self.fsm_send(msg.receiver, command)
            }
            "fsm_randomize" => self.fsm_randomize(msg.receiver),
            "a_play3dsound" => self.queue_3d_sound(context_name, msg.receiver, &msg.args),
            // `maxpayne_gamemode->A_PlaySound(maxhouse, P1L0a_cam)`: not positional.
            "a_playsound" => {
                let before = self.sound_queue.len();
                let receiver = msg.receiver.to_string();
                let queued = self.queue_sound(receiver, &msg.args);
                if let Some(sound) = self.sound_queue.get_mut(before) {
                    sound.range = None;
                }
                queued
            }
            // Every retail call goes to `maxpayne_gamemode`; the game mode owns the music.
            "a_playmusic" => {
                let cue = msg
                    .args
                    .first()
                    .map_or("", |a| a.trim().trim_matches('"').trim());
                self.music_request = Some(cue.to_string());
                true
            }
            // All 64 retail calls are enemy death cams (`Death_01..03`, `special_killed`).
            "cam_animateparented" => {
                let Some(block) = msg.args.first().map(|a| a.trim()).filter(|a| !a.is_empty())
                else {
                    return false;
                };
                let target = if msg.receiver.eq_ignore_ascii_case("this") {
                    context_name
                } else {
                    msg.receiver
                };
                let Some(character) = self
                    .characters
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(target))
                else {
                    return false;
                };
                let entry = (block.to_string(), character.name.clone());
                self.death_cams.push(entry);
                true
            }
            // Retail sends it to `player` only (313 of 313 calls in the shipped levels).
            "cam_animateinplace" => {
                let Some(block) = msg.args.first().map(|a| a.trim()).filter(|a| !a.is_empty())
                else {
                    return false;
                };
                self.camera_shakes.push(block.to_string());
                true
            }
            "ps_starteffect" => {
                let Some(effect) = msg.args.first().map(|a| a.trim()).filter(|a| !a.is_empty())
                else {
                    return false;
                };
                let Some(emitter) = self.sound_emitter(context_name, msg.receiver) else {
                    return false;
                };
                self.effect_queue.push(QueuedEffect {
                    effect: effect.to_string(),
                    emitter,
                });
                true
            }
            "ps_stopallemissions" => match self.sound_emitter(context_name, msg.receiver) {
                Some(emitter) => {
                    self.effect_stops.push(emitter);
                    true
                }
                None => false,
            },
            "a_stopall3dsounds" => match self.sound_emitter(context_name, msg.receiver) {
                Some(emitter) => {
                    self.sound_stops.push(emitter);
                    true
                }
                None => false,
            },
            "mphm_fadetocolor" => {
                let colour = |i: usize| {
                    let arg = msg.args.get(i)?.trim();
                    let hex = arg.strip_prefix("0x").or_else(|| arg.strip_prefix("0X"))?;
                    u32::from_str_radix(hex, 16).ok().map(u32::to_be_bytes)
                };
                let (Some(from), Some(to), Some(duration)) = (
                    colour(0),
                    colour(1),
                    msg.args.get(2).and_then(|a| a.trim().parse::<f32>().ok()),
                ) else {
                    return false;
                };
                self.screen_fade = Some(ScreenFade {
                    from,
                    to,
                    duration,
                    elapsed: 0.0,
                });
                true
            }
            "s_modeswitch" => {
                let Some(mode) = msg.args.first().map(|m| m.trim()).filter(|m| !m.is_empty())
                else {
                    return false;
                };
                self.mode_switch = Some(mode.to_ascii_lowercase());
                true
            }
            "gm_changefog" => {
                let arg = |i: usize| msg.args.get(i).map(|a| a.trim());
                let colour = arg(0)
                    .and_then(|a| a.strip_prefix("0x").or_else(|| a.strip_prefix("0X")))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok());
                let number = |i| arg(i).and_then(|a| a.parse::<f32>().ok());
                let (Some(rgb), Some(start), Some(end), Some(seconds)) =
                    (colour, number(1), number(2), number(3))
                else {
                    return false;
                };
                let [_, r, g, b] = rgb.to_be_bytes();
                let to = LevelFog {
                    color: [f32::from(r), f32::from(g), f32::from(b)],
                    start,
                    end,
                };
                match self.fog {
                    Some(from) if seconds > 0.0 => self.fog_fade = Some((from, to, seconds, 0.0)),
                    _ => {
                        self.fog = Some(to);
                        self.fog_fade = None;
                    }
                }
                true
            }
            "gm_setplayercontrols" => {
                let Some(controls) = script_bool(msg.args.first().copied()) else {
                    return false;
                };
                self.player_controls = controls;
                true
            }
            "cam_attachtodo" => {
                // Arg 0 (camera `+0x14f`, 0x672d12) is the camera's weight in percent: 100
                // attaches, 0 hands the view back to the player.
                let Some(weight) = msg.args.first().and_then(|a| a.trim().parse::<f32>().ok())
                else {
                    return false;
                };
                let Some(index) = self.resolve_mesh_index(context_name, msg.receiver) else {
                    return false;
                };
                self.attached_camera = (weight > 0.0).then_some(index);
                true
            }
            "do_hide" => {
                let Some(hidden) = script_bool(msg.args.first().copied()) else {
                    return false;
                };
                self.resolve_mesh_mut(context_name, msg.receiver)
                    .is_some_and(|mesh| {
                        mesh.hidden = hidden;
                        true
                    })
            }
            "do_enablecollisions" => {
                let Some(collisions) = script_bool(msg.args.first().copied()) else {
                    return false;
                };
                self.resolve_mesh_mut(context_name, msg.receiver)
                    .is_some_and(|mesh| {
                        mesh.collisions = collisions;
                        true
                    })
            }
            "do_sethitpoints" => {
                let Some(points) = msg.args.first().and_then(|arg| arg.parse::<i32>().ok()) else {
                    return false;
                };
                self.resolve_mesh_mut(context_name, msg.receiver)
                    .is_some_and(|mesh| {
                        mesh.hitpoints = Some(points);
                        true
                    })
            }
            _ => false,
        }
    }

    /// Match the receiver as stored, then the same path with `.TRIGGER` appended.
    pub fn resolve_trigger_mut<'a>(
        &'a mut self,
        context_name: &str,
        receiver: &str,
    ) -> Option<&'a mut SceneTrigger> {
        if let Some(index) = self
            .triggers
            .iter()
            .position(|trigger| trigger.name.eq_ignore_ascii_case(receiver))
        {
            return Some(&mut self.triggers[index]);
        }
        let target = trigger_target_name(context_name, receiver)?;
        if target.eq_ignore_ascii_case(receiver) {
            return None;
        }
        self.triggers
            .iter_mut()
            .find(|trigger| trigger.name.eq_ignore_ascii_case(&target))
    }

    fn queue_3d_sound(&mut self, context_name: &str, receiver: &str, args: &[&str]) -> bool {
        let category = args.first().copied().unwrap_or("");
        let cue = args.get(1).copied().unwrap_or("");
        if category.is_empty() || cue.is_empty() {
            return false;
        }
        let Some(emitter) = self.sound_emitter(context_name, receiver) else {
            return false;
        };
        self.queue_sound(emitter, args)
    }

    /// Queue `(category, cue, …)` from `emitter`, resolved now if its catalog is loaded.
    fn queue_sound(&mut self, emitter: String, args: &[&str]) -> bool {
        let category = args.first().copied().unwrap_or("");
        let cue = args.get(1).copied().unwrap_or("");
        if category.is_empty() || cue.is_empty() {
            return false;
        }
        let mut sound = QueuedSound {
            category: category.to_string(),
            cue: cue.to_string(),
            filename: None,
            emitter,
            looping: false,
            range: None,
        };
        let catalog_loaded = self
            .sound_catalogs
            .keys()
            .any(|key| key.eq_ignore_ascii_case(category));
        if !self.resolve_sound(&mut sound) && catalog_loaded {
            return false;
        }
        // An unloaded catalog is read when the sound plays (`render3d::resolve_queued_sounds`).
        self.sound_queue.push(sound);
        true
    }

    /// Fill `sound`'s file, looping and distance model from its loaded catalog.
    /// `false` when the catalog is not loaded or has no such cue.
    pub fn resolve_sound(&self, sound: &mut QueuedSound) -> bool {
        let Some(text) = self
            .sound_catalogs
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(&sound.category))
            .map(|(_, text)| text)
        else {
            return false;
        };
        sound.filename = crate::story_sound_filename(text, &sound.cue);
        sound.looping = crate::sound_cue_looping(text, &sound.cue);
        sound.range = crate::sound_cue_3d(text, &sound.cue);
        sound.filename.is_some()
    }

    /// Full name of the trigger, mesh or FSM `receiver` names, if this scene stores it.
    fn sound_emitter(&mut self, context_name: &str, receiver: &str) -> Option<String> {
        if let Some(trigger) = self.resolve_trigger_mut(context_name, receiver) {
            return Some(trigger.name.clone());
        }
        if let Some(mesh) = self.resolve_mesh_mut(context_name, receiver) {
            return Some(mesh.name.clone());
        }
        // Floating FSMs emit sounds too (MaxED "dynamic entities"); `this` is the running FSM.
        let fsm_name = if receiver.eq_ignore_ascii_case("this") {
            context_name
        } else {
            receiver
        };
        if let Some(index) = self.fsm_index(fsm_name) {
            return Some(self.fsms[index].name.clone());
        }
        // Speech: `player->A_Play3DSound(maxhouse, P1L0a_01, head)` and characters
        // (`::teleport::Max->…`) talk too.
        if receiver.eq_ignore_ascii_case("player") {
            return Some("player".to_string());
        }
        self.characters
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(fsm_name))
            .map(|c| c.name.clone())
    }

    /// Send `name`'s `OnDeath` messages. `false` when no character has that name.
    pub fn kill_character(&mut self, name: &str) -> bool {
        let Some(character) = self
            .characters
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
        else {
            return false;
        };
        let (name, messages) = (character.name.clone(), character.on_death.clone());
        for message in &messages {
            self.dispatch_message(&name, message);
        }
        true
    }

    /// Current world position of an emitter named by `sound_emitter`, room origin not added.
    pub fn emitter_world_pos(&self, emitter: &str) -> Option<[f32; 3]> {
        let find_mesh = |name: &str| {
            self.dynamic_meshes
                .iter()
                .position(|mesh| mesh.name.eq_ignore_ascii_case(name))
        };
        if let Some(index) = find_mesh(emitter) {
            return Some(self.dynamic_world_pose(index).1);
        }
        if let Some(trigger) = self
            .triggers
            .iter()
            .find(|trigger| trigger.name.eq_ignore_ascii_case(emitter))
        {
            return Some(trigger.pos);
        }
        if let Some(character) = self
            .characters
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(emitter))
        {
            return Some(character.pos);
        }
        let fsm = self
            .fsms
            .iter()
            .find(|fsm| fsm.name.eq_ignore_ascii_case(emitter))?;
        // FSM positions are world positions at load; carry them with the parent mesh like a child DO.
        let Some(parent) = parent_name(&fsm.name).and_then(|name| find_mesh(&name)) else {
            return Some(fsm.pos);
        };
        let mesh = &self.dynamic_meshes[parent];
        let own = ([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], fsm.pos);
        let rest = invert_pose((mesh.world_mat, mesh.world_pos));
        Some(compose_pose(compose_pose(own, rest), self.dynamic_world_pose(parent)).1)
    }

    /// Advance the screen fade; a finished fade to a transparent colour goes away.
    pub fn step_screen_fade(&mut self, dt: f32) {
        let Some(fade) = self.screen_fade.as_mut() else {
            return;
        };
        fade.elapsed += dt;
        if fade.elapsed >= fade.duration && fade.to[3] == 0 {
            self.screen_fade = None;
        }
    }

    /// Advance a `GM_ChangeFog` fade: every fog value moves linearly from the old fog to the new.
    pub fn step_fog(&mut self, dt: f32) {
        let Some((from, to, seconds, elapsed)) = self.fog_fade.as_mut() else {
            return;
        };
        *elapsed += dt;
        let k = (*elapsed / *seconds).min(1.0);
        let mix = |a: f32, b: f32| a + (b - a) * k;
        self.fog = Some(LevelFog {
            color: std::array::from_fn(|c| mix(from.color[c], to.color[c])),
            start: mix(from.start, to.start),
            end: mix(from.end, to.end),
        });
        if k >= 1.0 {
            self.fog_fade = None;
        }
    }

    /// Run startup `DO_SetHitpoints` calls. Other startup verbs stay untouched.
    pub fn apply_startup_hitpoints(&mut self) {
        let scripts: Vec<(String, String)> = self
            .fsms
            .iter()
            .flat_map(|fsm| {
                fsm.startup_actions.iter().filter_map(|act| {
                    let verb = Message::parse(act)?.verb.to_ascii_lowercase();
                    (verb == "do_sethitpoints").then(|| (fsm.name.clone(), act.clone()))
                })
            })
            .collect();
        for (name, act) in scripts {
            self.dispatch_message(&name, &act);
        }
    }

    /// Startup `DO_Hide` only. Other startup verbs stay untouched.
    pub fn apply_startup_hides(&mut self) {
        let scripts: Vec<(String, String)> = self
            .fsms
            .iter()
            .flat_map(|fsm| {
                fsm.startup_actions.iter().filter_map(|act| {
                    let verb = Message::parse(act)?.verb.to_ascii_lowercase();
                    (verb == "do_hide").then(|| (fsm.name.clone(), act.clone()))
                })
            })
            .collect();
        for (name, act) in scripts {
            self.dispatch_message(&name, &act);
        }
    }

    /// Startup `T_Enable` only. Other startup verbs stay untouched.
    pub fn apply_startup_enables(&mut self) {
        let scripts: Vec<(String, String)> = self
            .fsms
            .iter()
            .flat_map(|fsm| {
                fsm.startup_actions.iter().filter_map(|act| {
                    let verb = Message::parse(act)?.verb.to_ascii_lowercase();
                    (verb == "t_enable").then(|| (fsm.name.clone(), act.clone()))
                })
            })
            .collect();
        for (name, act) in scripts {
            self.dispatch_message(&name, &act);
        }
    }

    /// Startup `DO_Animate` only (the startup intro's `Start_FSM`, idle machinery). Other startup
    /// verbs stay untouched.
    pub fn apply_startup_animates(&mut self) {
        let scripts: Vec<(String, String)> = self
            .fsms
            .iter()
            .flat_map(|fsm| {
                fsm.startup_actions.iter().filter_map(|act| {
                    let verb = Message::parse(act)?.verb.to_ascii_lowercase();
                    (verb == "do_animate").then(|| (fsm.name.clone(), act.clone()))
                })
            })
            .collect();
        for (name, act) in scripts {
            self.dispatch_message(&name, &act);
        }
    }

    /// Startup `FSM_Send` only (door locks, ceiling fans). Other startup verbs stay untouched.
    pub fn apply_startup_sends(&mut self) {
        let scripts: Vec<(String, String)> = self
            .fsms
            .iter()
            .flat_map(|fsm| {
                fsm.startup_actions.iter().filter_map(|act| {
                    let verb = Message::parse(act)?.verb.to_ascii_lowercase();
                    (verb == "fsm_send").then(|| (fsm.name.clone(), act.clone()))
                })
            })
            .collect();
        for (name, act) in scripts {
            self.dispatch_message(&name, &act);
        }
    }

    /// Subtract `amount` from a mesh that has hit points. At zero, run its `DO_OnDeath` hook.
    pub fn damage_mesh(&mut self, index: usize, amount: i32) -> bool {
        let Some(mesh) = self.dynamic_meshes.get_mut(index) else {
            return false;
        };
        let Some(points) = mesh.hitpoints else {
            return false;
        };
        if points <= 0 {
            return false;
        }
        let next = points.saturating_sub(amount.max(0));
        mesh.hitpoints = Some(next);
        if next > 0 {
            return false;
        }
        let fsm_name = strip_mesh_suffix(&mesh.name).to_string();
        self.dispatch_hook(&fsm_name, "DO_OnDeath")
    }

    fn dispatch_hook(&mut self, fsm_name: &str, hook_name: &str) -> bool {
        let Some(index) = self.fsm_index(fsm_name) else {
            return false;
        };
        let fsm = &self.fsms[index];
        let Some(script) = find_script(&fsm.hooks, hook_name) else {
            return false;
        };
        let state = fsm.state.clone();
        self.run_fsm_script(index, script, &state)
    }

    /// `FSM_Switch` (0x6e30a0): store the new state, then run its script with the old state.
    fn fsm_switch(&mut self, fsm_name: &str, state: &str) -> bool {
        let Some(index) = self.fsm_index(fsm_name) else {
            return false;
        };
        let old = std::mem::replace(&mut self.fsms[index].state, state.to_string());
        if let Some(script) = find_script(&self.fsms[index].state_scripts, state) {
            self.run_fsm_script(index, script, &old);
        }
        true
    }

    /// `FSM_Randomize` (0x6e3310): switch to state `rand() % count` of the state list.
    fn fsm_randomize(&mut self, fsm_name: &str) -> bool {
        let Some(index) = self.fsm_index(fsm_name) else {
            return false;
        };
        let states = &self.fsms[index].states;
        if states.is_empty() {
            return false;
        }
        let state = states[crt_rand() as usize % states.len()].clone();
        self.fsm_switch(fsm_name, &state)
    }

    /// `FSM_Send` (0x6e2f10): run the command's script with the current state.
    fn fsm_send(&mut self, fsm_name: &str, command: &str) -> bool {
        let Some(index) = self.fsm_index(fsm_name) else {
            return false;
        };
        let fsm = &self.fsms[index];
        let Some(script) = find_script(&fsm.commands, command) else {
            return false;
        };
        let state = fsm.state.clone();
        self.run_fsm_script(index, script, &state)
    }

    fn fsm_index(&self, fsm_name: &str) -> Option<usize> {
        self.fsms
            .iter()
            .position(|fsm| fsm.name.eq_ignore_ascii_case(fsm_name))
    }

    /// The calls with no state, then the calls of the `section` state. Nested FSM scripts stop
    /// at depth 32, the limit retail warns at (`0x89b294`).
    fn run_fsm_script(&mut self, index: usize, script: FsmScript, section: &str) -> bool {
        thread_local! {
            static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        }
        if DEPTH.get() >= 32 {
            return false;
        }
        DEPTH.set(DEPTH.get() + 1);
        let name = self.fsms[index].name.clone();
        let (_, plain, per_state) = script;
        let calls = per_state
            .into_iter()
            .filter(|(state, _)| state.eq_ignore_ascii_case(section))
            .flat_map(|(_, calls)| calls);
        let mut ran = false;
        for act in plain.into_iter().chain(calls) {
            ran |= self.dispatch_message(&name, &act);
        }
        DEPTH.set(DEPTH.get() - 1);
        ran
    }

    /// `E` near an enabled trigger runs that FSM's unconditional `T_Activate` calls.
    pub fn activate_nearest_trigger(
        &mut self,
        room_origins: &BTreeMap<i32, [f32; 3]>,
        pos: [f32; 3],
        max_d2: f32,
    ) -> bool {
        let mut best: Option<(usize, f32)> = None;
        for (idx, trigger) in self.triggers.iter().enumerate() {
            if !trigger.enabled {
                continue;
            }
            let ro = room_origins
                .get(&trigger.room_id)
                .copied()
                .unwrap_or([0.0; 3]);
            let wx = ro[0] + trigger.pos[0];
            let wy = ro[1] + trigger.pos[1];
            let wz = ro[2] + trigger.pos[2];
            let d2 = (wx - pos[0]).powi(2) + (wy - pos[1]).powi(2) + (wz - pos[2]).powi(2);
            if d2 < max_d2 && best.is_none_or(|(_, best_d2)| d2 < best_d2) {
                best = Some((idx, d2));
            }
        }
        let Some((idx, _)) = best else {
            return false;
        };
        let fsm_name = strip_trigger_suffix(&self.triggers[idx].name).to_string();
        self.dispatch_hook(&fsm_name, "T_Activate")
    }

    pub fn step_dynamic_anims(&mut self, dt: f32) {
        let queued: Vec<(String, Vec<String>)> = self
            .dynamic_meshes
            .iter_mut()
            .filter(|mesh| !mesh.anim_state.queued.is_empty())
            .map(|mesh| {
                (
                    mesh.name.clone(),
                    std::mem::take(&mut mesh.anim_state.queued),
                )
            })
            .collect();
        for (name, scripts) in queued {
            for script in &scripts {
                self.dispatch_message(&name, script);
            }
        }
        let mut pending = Vec::new();
        for mesh in &mut self.dynamic_meshes {
            if let Some(scripts) = mesh.step_animation(dt) {
                pending.push((mesh.name.clone(), scripts));
            }
        }
        for (name, scripts) in pending {
            for script in &scripts {
                self.dispatch_message(&name, script);
            }
        }
    }
}

fn mesh_target_name(context_name: &str, receiver: &str) -> Option<String> {
    let target = match receiver {
        "this" => context_name.to_string(),
        "parent" => format!("{}.DO", parent_path(context_name)?),
        path if path.starts_with("::") => {
            if path.to_ascii_lowercase().ends_with(".do") {
                path.to_string()
            } else {
                format!("{path}.DO")
            }
        }
        rel => format!("{}::{rel}.DO", parent_path(context_name)?),
    };
    Some(target)
}

fn trigger_target_name(context_name: &str, receiver: &str) -> Option<String> {
    let target = match receiver {
        "this" => context_name.to_string(),
        "parent" => format!("{}.TRIGGER", parent_path(context_name)?),
        path if path.starts_with("::") => {
            if path.to_ascii_lowercase().ends_with(".trigger") {
                path.to_string()
            } else {
                format!("{path}.TRIGGER")
            }
        }
        rel if rel.to_ascii_lowercase().ends_with(".trigger") => {
            format!("{}::{rel}", parent_path(context_name)?)
        }
        rel => format!("{}::{rel}.TRIGGER", parent_path(context_name)?),
    };
    Some(target)
}

fn strip_trigger_suffix(name: &str) -> &str {
    if name.to_ascii_lowercase().ends_with(".trigger") {
        &name[..name.len() - ".trigger".len()]
    } else {
        name
    }
}

/// MSVCRT `rand()`: `seed = seed * 214013 + 2531011`, result bits 16..30. Unseeded start is 1.
fn crt_rand() -> u32 {
    thread_local! {
        static SEED: std::cell::Cell<u32> = const { std::cell::Cell::new(1) };
    }
    let seed = SEED.get().wrapping_mul(214_013).wrapping_add(2_531_011);
    SEED.set(seed);
    (seed >> 16) & 0x7fff
}

fn find_script(scripts: &[FsmScript], name: &str) -> Option<FsmScript> {
    scripts
        .iter()
        .find(|(script, _, _)| script.eq_ignore_ascii_case(name))
        .cloned()
}

fn strip_mesh_suffix(name: &str) -> &str {
    if name.to_ascii_lowercase().ends_with(".do") {
        &name[..name.len() - 3]
    } else {
        name
    }
}

fn parent_path(context_name: &str) -> Option<&str> {
    let stripped = context_name
        .strip_suffix(".DO")
        .or_else(|| context_name.strip_suffix(".TRIGGER"))
        .unwrap_or(context_name);
    stripped.rsplit_once("::").map(|(parent, _)| parent)
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

fn skip_hook_map(
    data: &[u8],
    i: &mut usize,
) -> Option<Vec<(String, Vec<String>, Vec<(String, Vec<String>)>)>> {
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
        let mut state_actions = Vec::with_capacity(c2);
        for _ in 0..c2 {
            let state_name = read_str(data, i)?;
            let s_acts = read_str_list(data, i, 0x1c)?;
            state_actions.push((state_name, s_acts));
        }
        let mut after = read_str_list(data, i, 0x1c)?;
        acts.append(&mut after);
        out.push((hname, acts, state_actions));
    }
    Some(out)
}

/// `colour` with `mask` intensity in the top byte (nearest sample when sizes differ).
fn with_alpha_mask(colour: &DecodedTexture, mask: &DecodedTexture) -> DecodedTexture {
    let mut pixels = colour.pixels.clone();
    for y in 0..colour.height {
        for x in 0..colour.width {
            let mx = x * mask.width / colour.width.max(1);
            let my = y * mask.height / colour.height.max(1);
            // A texel missing from a short decode reads as transparent instead of panicking.
            let m = mask
                .pixels
                .get((my * mask.width + mx) as usize)
                .copied()
                .unwrap_or(0);
            let alpha = (((m >> 16) & 0xff) + ((m >> 8) & 0xff) + (m & 0xff)) / 3;
            if let Some(p) = pixels.get_mut((y * colour.width + x) as usize) {
                *p = (alpha << 24) | (*p & 0x00ff_ffff);
            }
        }
    }
    DecodedTexture {
        path: format!("{}|{}", colour.path, mask.path),
        flag: colour.flag,
        width: colour.width,
        height: colour.height,
        pixels,
    }
}

/// Embedded texture for `path` by full path, file name, then stem.
fn lookup_texture(tex_lookup: &BTreeMap<String, usize>, path: &str) -> Option<usize> {
    let mut clean = path.to_ascii_lowercase();
    if let Some(stripped) = clean.strip_suffix(" new") {
        clean = stripped.trim().to_string();
    }
    let base = clean.rsplit(['\\', '/']).next().unwrap_or(&clean);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    tex_lookup
        .get(&clean)
        .or_else(|| tex_lookup.get(base))
        .or_else(|| tex_lookup.get(stem))
        .copied()
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
            let texture_index = lookup_texture(&tex_lookup, &texture_path);
            materials.insert(
                id,
                SceneMaterial {
                    id,
                    category,
                    texture_path,
                    texture_index,
                    alpha_reference: None,
                    blended: false,
                },
            );
        }
        mat_passes += 1;
    }

    // 3. Material categories
    let (_, categories, after_cats) = super::parse_ras_ldb_import_material_categories_at(data, i)?;
    // 0x5dc800: a material record's texture file (+0x1c) wins over its name; a third
    // string is its alpha image, combined into the texture (0x5dc3d8, `P_Map(colour, alpha)`).
    let mut masked: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for category in &categories {
        let records = std::iter::once((&category.texture, &category.path, category.path2.as_str()))
            .chain(
                category
                    .members
                    .iter()
                    .map(|m| (&m.name, &m.path, m.path2.as_deref().unwrap_or(""))),
            );
        for (name, file, alpha) in records {
            let colour = (!file.is_empty())
                .then(|| lookup_texture(&tex_lookup, file))
                .flatten();
            let mask = (!alpha.is_empty())
                .then(|| lookup_texture(&tex_lookup, alpha))
                .flatten();
            for material in materials.values_mut() {
                if !material.texture_path.eq_ignore_ascii_case(name) {
                    continue;
                }
                if let Some(index) = colour {
                    material.texture_index = Some(index);
                }
                let (Some(colour), Some(mask)) = (material.texture_index, mask) else {
                    continue;
                };
                let index = *masked.entry((colour, mask)).or_insert_with(|| {
                    textures.push(with_alpha_mask(&textures[colour], &textures[mask]));
                    textures.len() - 1
                });
                material.texture_index = Some(index);
                // `ID_ALPHAREFERENCE` until `materials.txt` overrides it per category.
                material.alpha_reference = Some(15);
            }
        }
    }
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
        // Mesh attribs (`0x1f` + count + (id, vec3)*count), may be empty
        if data.get(i) == Some(&0x1f) {
            let mut p = i + 1;
            if let Some(acnt) = read_i32(data, &mut p) {
                // An empty list belongs to this mesh too. After the last mesh the light
                // count follows it (Part1_Level6 @0x65a854, Startup_level @0x135ec82).
                if acnt == 0 {
                    i = p;
                } else if acnt > 0 && data.get(p) != Some(&0x25) {
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
            let (mat, pos) = read_xform(data, &mut i)?;
            let _x2 = read_xform_pos(data, &mut i)?;
            let room_id = read_i32(data, &mut i)?;
            let _unk = read_str(data, &mut i)?;
            let jp_type = read_i32(data, &mut i)?;
            jumppoints.push(SceneJumppoint {
                name,
                room_id,
                pos,
                mat,
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
            // `X_LevelDBFSM` (0x5fad40): the startup script (list, per-state map, list), then
            // maps of named scripts: states +0xe4, commands +0xf4, events +0x104.
            let mut hooks = skip_hook_map(data, &mut i)?;
            let _extra = read_str_list(data, &mut i, 0x1c)?;
            let state_scripts = skip_hook_map(data, &mut i)?;
            let commands = skip_hook_map(data, &mut i)?;
            hooks.extend(state_scripts.iter().cloned());
            hooks.extend(commands.iter().cloned());
            hooks.append(&mut skip_hook_map(data, &mut i)?);
            fsms.push(SceneFsm {
                name,
                room_id,
                pos,
                states,
                state: default_state.clone(),
                default_state,
                startup_actions,
                hooks,
                state_scripts,
                commands,
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
            // MaxED tabs, by content over every shipped level: StartUp, OnDeath (all 99 death
            // counters and death cams), OnActivate (149 alert shouts), then one UNKNOWN list.
            // `startup_scripts` keeps all four: `pursues` reads the OnActivate combat switch.
            let mut startup_scripts = read_str_list(data, &mut i, 0x1c)?;
            let on_death = read_str_list(data, &mut i, 0x1c)?;
            startup_scripts.extend(on_death.iter().cloned());
            for _ in 0..2 {
                startup_scripts.append(&mut read_str_list(data, &mut i, 0x1c)?);
            }
            characters.push(SceneCharacter {
                name,
                skin,
                room_id,
                pos,
                startup_scripts,
                on_death,
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
                enabled: true,
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
                    // DMA reader 0x5ff420: three 0x1c lists → +0x8c, +0x9c, +0xac.
                    let start_scripts = read_str_list(data, &mut i, 0x1c)?;
                    let reverse_end_scripts = read_str_list(data, &mut i, 0x1c)?;
                    let end_scripts = read_str_list(data, &mut i, 0x1c)?;
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
                        start_scripts,
                        reverse_end_scripts,
                        end_scripts,
                    });
                }
                // Six bools to `X_LevelDBDynamicMesh` +0xa0..+0xa5 (0x5ef340); +0xa0 seeds
                // the DO's collisions flag +0x4ae (0x6f2c7c).
                let collisions = *data.get(i + 1)? != 0;
                i += 12; // `0e u8` each
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
                    hidden: false,
                    collisions,
                    hitpoints: None,
                    polys,
                    anims,
                    active_track: None,
                    track_time: 0.0,
                    track_duration: 0.0,
                    anim_state: Default::default(),
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
        let all_acts =
            fsm.startup_actions
                .iter()
                .chain(fsm.states.iter())
                .chain(fsm.hooks.iter().flat_map(|(_, h, states)| {
                    h.iter()
                        .chain(states.iter().flat_map(|(_, acts)| acts.iter()))
                }));
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
        sound_catalogs: BTreeMap::new(),
        sound_queue: Vec::new(),
        sound_stops: Vec::new(),
        music_request: None,
        camera_shakes: Vec::new(),
        death_cams: Vec::new(),
        effect_queue: Vec::new(),
        effect_stops: Vec::new(),
        attached_camera: None,
        player_controls: true,
        screen_fade: None,
        ambient: [0.0; 3],
        mode_switch: None,
        fog: None,
        fog_fade: None,
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

fn xform_point(mat: [f32; 9], origin: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    [
        origin[0] + mat[0] * v[0] + mat[3] * v[1] + mat[6] * v[2],
        origin[1] + mat[1] * v[0] + mat[4] * v[1] + mat[7] * v[2],
        origin[2] + mat[2] * v[0] + mat[5] * v[1] + mat[8] * v[2],
    ]
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
            sound_catalogs: BTreeMap::new(),
            sound_queue: Vec::new(),
            sound_stops: Vec::new(),
            music_request: None,
            camera_shakes: Vec::new(),
            death_cams: Vec::new(),
            effect_queue: Vec::new(),
            effect_stops: Vec::new(),
            attached_camera: None,
            player_controls: true,
            screen_fade: None,
            ambient: [0.0; 3],
            mode_switch: None,
            fog: None,
            fog_fade: None,
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
                    mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                    jp_type: 0,
                },
                SceneJumppoint {
                    name: "Jumppoint 00".into(),
                    room_id: 0,
                    pos: [1.0, 0.0, 2.0],
                    mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                    jp_type: 0,
                },
            ],
        );
        let idx = scene.spawn_jumppoint_index(Some("::StartRoom::Jumppoint 00"));
        assert_eq!(idx, 1);
        assert_eq!(scene.jumppoints[idx].pos, [1.0, 0.0, 2.0]);
    }

    #[test]
    fn jumppoint_yaw_faces_its_z_axis() {
        // The house start (`::startroom::Jumppoint 00`, z = +Z) faces into the room, away from
        // the door; `Part1_Level2b`'s start has z = -Z. Player yaw: forward = (sin, 0, cos).
        let mut jp = SceneJumppoint {
            name: "::startroom::Jumppoint 00".into(),
            room_id: 0,
            pos: [0.0; 3],
            mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            jp_type: 0,
        };
        assert!(jp.yaw().abs() < 1e-6);
        jp.mat = [-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0];
        assert!((jp.yaw().abs() - std::f32::consts::PI).abs() < 1e-6);
        // z = (0.30, 0, -0.95), the Part1_Level1b start.
        jp.mat = [-0.95, 0.0, -0.30, 0.0, 1.0, 0.0, 0.30, 0.0, -0.95];
        let (s, c) = jp.yaw().sin_cos();
        // Inputs are rounded to two decimals, so compare at that precision.
        assert!((s - 0.30).abs() < 1e-2 && (c + 0.95).abs() < 1e-2);
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

    #[test]
    fn masked_materials_take_blending_from_their_category() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        let material = |id: i32, category: &str| SceneMaterial {
            id,
            category: category.into(),
            texture_path: String::new(),
            texture_index: Some(0),
            alpha_reference: Some(0),
            blended: false,
        };
        scene.materials.insert(1, material(1, "leaves"));
        scene.materials.insert(2, material(2, "glass"));
        let surfaces = parse_material_surfaces(
            "[Leaves]\nAlphaReference = 128;\nBlendedAlphaTest = FALSE;\n[Glass]\nAlphaReference = 15;\n",
        );
        scene.apply_material_surfaces(&surfaces);
        assert!(!scene.materials[&1].blended, "Leaves is a hard cut-out");
        assert!(
            scene.materials[&2].blended,
            "BlendedAlphaTest defaults to TRUE"
        );
    }

    fn actor(scripts: &[&str]) -> SceneCharacter {
        SceneCharacter {
            name: "actor".into(),
            skin: "lab_junkie".into(),
            room_id: 1,
            pos: [0.0; 3],
            startup_scripts: scripts.iter().map(|s| (*s).to_string()).collect(),
            on_death: Vec::new(),
        }
    }

    #[test]
    fn a_dying_character_sends_on_death_and_its_counter_starts_the_death_cam() {
        let mut scene = door_fsm_scene();
        let mut e1 = actor(&[]);
        e1.name = "::storage::e1".into();
        e1.on_death = vec!["::storage::death_counter->fsm_Send(add1);".into()];
        scene.characters.push(e1);
        scene.fsms[0].name = "::storage::death_counter".into();
        scene.fsms[0].state = "3".into();
        scene.fsms[0].commands = vec![(
            "add1".into(),
            Vec::new(),
            vec![(
                "3".into(),
                vec!["::storage::e1->CAM_AnimateParented(death_03);".into()],
            )],
        )];
        assert!(scene.kill_character("::STORAGE::e1"));
        assert_eq!(
            scene.death_cams,
            [("death_03".to_string(), "::storage::e1".to_string())]
        );
        assert!(!scene.kill_character("::storage::nobody"));
    }

    #[test]
    fn retail_storage_enemy_on_death_feeds_the_death_counter() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let Some(level) =
            crate::read_game_bytes(&game, r"\data\database\levels\part2\Part2_Level3.ldb")
        else {
            return;
        };
        let scene = parse_ldb_scene(&level).expect("Part2_Level3");
        let e1 = scene
            .characters
            .iter()
            .find(|c| c.name == "::storage::e1")
            .expect("e1");
        assert_eq!(e1.on_death, ["::storage::death_counter->fsm_Send(add1);"]);
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
        // The intro runs on the level's own scripts: Start_FSM's startup DO_Animate, then
        // FSM_Send(start) to each camera rig, CAM_AttachToDO and DO_Animate, and finally
        // S_ModeSwitch(graphicnovel).
        let mut scene = scene;
        scene.apply_startup_sends();
        scene.apply_startup_animates();
        let mut cameras: Vec<String> = Vec::new();
        for _ in 0..60 * 90 {
            scene.step_dynamic_anims(1.0 / 60.0);
            if let Some(i) = scene.attached_camera {
                let name = &scene.dynamic_meshes[i].name;
                if cameras.last() != Some(name) {
                    cameras.push(name.clone());
                }
            }
            if scene.mode_switch.is_some() {
                break;
            }
        }
        assert_eq!(scene.mode_switch.as_deref(), Some("graphicnovel"));
        assert_eq!(cameras.len(), 5, "{cameras:?}");
        assert!(cameras[4].ends_with("altitude::camera.DO"));
    }

    fn door_mesh(anims: Vec<SceneDynamicAnim>) -> SceneDynamicMesh {
        SceneDynamicMesh {
            name: "::startroom::door.DO".into(),
            room_id: 1,
            world_mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            world_pos: [1.0, 0.0, 2.0],
            local_start_mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            local_start_pos: [0.0, 0.0, 0.0],
            toggled: false,
            hidden: false,
            collisions: true,
            hitpoints: None,
            polys: Vec::new(),
            anims,
            active_track: None,
            track_time: 0.0,
            track_duration: 0.0,
            anim_state: DoAnimState::default(),
        }
    }

    fn track(name: &str, end: [f32; 3]) -> SceneDynamicAnim {
        SceneDynamicAnim {
            name: name.into(),
            duration: 1.3,
            start_mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            start_pos: [0.0, 0.0, 0.0],
            end_mat: [0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0],
            end_pos: end,
            start_scripts: Vec::new(),
            reverse_end_scripts: Vec::new(),
            end_scripts: Vec::new(),
            curves: [Vec::new(), Vec::new()],
        }
    }

    /// Mesh whose rest pose is the track's start, half way through `anim`.
    fn mid_track(anim: SceneDynamicAnim) -> SceneDynamicMesh {
        let mut mesh = door_mesh(vec![anim]);
        mesh.world_pos = mesh.local_start_pos;
        mesh.play_track("open");
        mesh.track_time = mesh.track_duration / 2.0;
        mesh
    }

    #[test]
    fn mid_track_rotation_slerps_by_the_second_curve() {
        // 0x5ff100: curve +0xd8 is the raw slerp t between start and end.
        let mut anim = track("open", [0.0; 3]);
        anim.end_mat = [0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        anim.curves[1] = vec![0.0, 1.0];
        let (mat, _) = mid_track(anim).posed_local();
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!((mat[0] - h).abs() < 1.0e-4 && (mat[1] - h).abs() < 1.0e-4);
    }

    #[test]
    fn mid_track_translation_is_distance_curve_over_length() {
        // 0x5ff100: weight = sample(+0xbc) * (+0x88 = 1/|end - start|), no clamp.
        let mut anim = track("open", [10.0, 0.0, 0.0]);
        anim.curves[0] = vec![0.0, 12.0, 10.0];
        let (_, pos) = mid_track(anim).posed_local();
        assert!((pos[0] - 12.0).abs() < 1.0e-4);
    }

    #[test]
    fn opening_track_is_open_not_the_trailing_timer() {
        let mut door = door_mesh(vec![
            track("open", [0.2, 0.0, 0.0]),
            track("timer", [9.0, 9.0, 9.0]),
        ]);
        assert_eq!(door.opening_anim().unwrap().name, "open");
        assert_eq!(door.posed_local().1, [1.0, 0.0, 2.0]);
        door.toggled = true;
        assert_eq!(door.posed_local().1, [1.2, 0.0, 2.0]);
    }

    #[test]
    fn house_materials_use_their_record_texture_path() {
        // 0x5dc800: a material record is (name +0xc, texture file +0x1c, bool, bool).
        let path =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !path.is_file() {
            return;
        }
        let scene = parse_ldb_scene(&std::fs::read(path).unwrap()).expect("house");
        let tex = |id: i32| {
            let index = scene.materials[&id].texture_index.expect("textured");
            scene.textures[index].path.clone()
        };
        assert!(tex(16).ends_with("MaxHouseWall01_512x512.jpg"));
        assert!(tex(2).ends_with("Concrete01_128x128.jpg"));
        let untextured: Vec<_> = scene
            .materials
            .values()
            .filter(|m| m.texture_index.is_none())
            .map(|m| m.texture_path.as_str())
            .collect();
        assert!(untextured.is_empty(), "{untextured:?}");
    }

    #[test]
    fn short_mask_pixels_do_not_panic() {
        let tex = |pixels: Vec<u32>| DecodedTexture {
            path: String::new(),
            flag: 4,
            width: 2,
            height: 2,
            pixels,
        };
        let out = with_alpha_mask(&tex(vec![0x00ff_ffff; 3]), &tex(vec![0x00ff_ffff]));
        assert_eq!(out.pixels.len(), 3);
        assert_eq!(out.pixels[0] >> 24, 0xff);
        assert_eq!(out.pixels[1] >> 24, 0, "missing mask texel is transparent");
    }

    #[test]
    fn masked_materials_take_their_category_alpha_reference() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        for (id, category, alpha) in [(1, "leaves", Some(15)), (2, "wood", None)] {
            scene.materials.insert(
                id,
                SceneMaterial {
                    id,
                    category: category.into(),
                    texture_path: String::new(),
                    texture_index: None,
                    alpha_reference: alpha,
                    blended: false,
                },
            );
        }
        let surfaces = parse_material_surfaces(
            "[Leaves]\nAlphaReference = 127;\nBlendedAlphaTest = FALSE;\n\
             [Wood]\nAlphaReference = 40;\nBlendedAlphaTest = TRUE;\n",
        );
        scene.apply_material_surfaces(&surfaces);
        assert_eq!(scene.materials[&1].alpha_reference, Some(127));
        assert_eq!(
            scene.materials[&2].alpha_reference, None,
            "opaque stays opaque"
        );
    }

    #[test]
    fn house_plant_material_carries_its_alpha_mask() {
        // 0x5dc3d8: P_Map(colour, alpha) when the record has an alpha image.
        let path =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !path.is_file() {
            return;
        }
        let scene = parse_ldb_scene(&std::fs::read(path).unwrap()).expect("house");
        let plant = &scene.materials[&52];
        assert_eq!(plant.alpha_reference, Some(15));
        let tex = &scene.textures[plant.texture_index.unwrap()];
        assert!(tex.pixels.iter().any(|p| p >> 24 == 0));
        assert!(tex.pixels.iter().any(|p| p >> 24 > 0x80));
        assert_eq!(scene.materials[&16].alpha_reference, None);
    }

    #[test]
    fn child_mesh_turns_with_its_animating_parent() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        let mut parent = door_mesh(vec![track("open", [0.0; 3])]);
        parent.name = "::room::hinge.DO".into();
        parent.world_pos = [0.0; 3];
        let mut child = door_mesh(Vec::new());
        child.name = "::room::hinge::panel.DO".into();
        child.local_start_pos = [1.0, 0.0, 0.0];
        child.world_pos = [1.0, 0.0, 0.0];
        scene.dynamic_meshes.push(parent);
        scene.dynamic_meshes.push(child);
        scene.dynamic_meshes[0].play_track("open");
        scene.dynamic_meshes[0].track_time = 1.3;
        let (mat, pos) = scene.dynamic_world_pose(1);
        // Parent end pose maps x to -z, so the panel swings from +x to -z.
        assert_eq!(pos, [0.0, 0.0, -1.0]);
        assert_eq!(&mat[0..3], &[0.0, 0.0, -1.0]);
    }

    #[test]
    fn house_child_rest_world_is_local_then_parent_world() {
        // 0x6e1a00: X = L · X(parent); with R191 = B = I the rest world is L0 · W0(parent).
        let path =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !path.is_file() {
            return;
        }
        let scene = parse_ldb_scene(&std::fs::read(path).unwrap()).expect("house");
        let mut children = 0;
        for mesh in &scene.dynamic_meshes {
            let Some(parent) = parent_name(&mesh.name)
                .and_then(|p| scene.dynamic_meshes.iter().find(|m| m.name == p))
            else {
                continue;
            };
            children += 1;
            let pos = xform_point(parent.world_mat, parent.world_pos, mesh.local_start_pos);
            for k in 0..3 {
                assert!(
                    (pos[k] - mesh.world_pos[k]).abs() < 1.0e-3,
                    "{}: {pos:?} vs {:?}",
                    mesh.name,
                    mesh.world_pos
                );
            }
        }
        assert!(children > 0);
    }

    #[test]
    fn house_startroom_door_uses_the_open_track() {
        let path =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !path.is_file() {
            return;
        }
        let scene = parse_ldb_scene(&std::fs::read(path).unwrap()).expect("house");
        let door = scene
            .dynamic_meshes
            .iter()
            .find(|mesh| mesh.name == "::startroom::New mesh 53::door.DO")
            .expect("front door");
        let anim = door.opening_anim().expect("open track");
        assert_eq!(anim.name, "open");
        assert!((anim.duration - 1.3).abs() < 0.01);
    }

    #[test]
    fn message_parse_reads_a_stored_call() {
        let msg = Message::parse("::Bathroom::lid.DO->DO_Animate(open);").unwrap();
        assert_eq!(msg.receiver, "::Bathroom::lid.DO");
        assert_eq!(msg.verb, "DO_Animate");
        assert_eq!(msg.args, ["open"]);
        let bare = Message::parse("DO_Animate(1)").unwrap();
        assert_eq!(bare.receiver, "this");
        assert_eq!(bare.args, ["1"]);
    }

    #[test]
    fn do_animate_starts_the_named_track() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        let mut door = door_mesh(vec![
            track("open", [0.4, 0.0, 0.0]),
            track("open2", [0.0; 3]),
        ]);
        door.name = "::startroom::door_dummy::Door.DO".into();
        door.anims[0].end_scripts =
            vec!["::startroom::door_dummy::Door.DO->DO_Animate(open2);".into()];
        scene.dynamic_meshes.push(door);
        let started = scene.dispatch_message(
            "::startroom::door_dummy::Door::t1.TRIGGER",
            "parent->do_animate(open);",
        );
        assert!(started);
        assert_eq!(
            scene.dynamic_meshes[0].active_track.as_deref(),
            Some("open")
        );
        scene.step_dynamic_anims(1.3);
        assert_eq!(
            scene.dynamic_meshes[0].active_track.as_deref(),
            Some("open2")
        );
    }

    /// One door playing `open` (1.3 s) in a scene, for the `0x6f34c0` tick tests.
    fn playing_door() -> LdbScene {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        scene
            .dynamic_meshes
            .push(door_mesh(vec![track("open", [0.4, 0.0, 0.0])]));
        assert!(scene.dispatch_message("", "::startroom::door.DO->DO_Animate(open);"));
        scene
    }

    #[test]
    fn do_pause_animation_holds_then_carries_the_overshoot() {
        // 0x6f4100 sets hold +0x4a3; 0x6f34c0 counts it down and adds the overshoot.
        let mut scene = playing_door();
        assert!(scene.dispatch_message("", "::startroom::door.DO->DO_PauseAnimation(1);"));
        scene.step_dynamic_anims(0.6);
        assert_eq!(scene.dynamic_meshes[0].track_time, 0.0);
        scene.step_dynamic_anims(0.6);
        assert!((scene.dynamic_meshes[0].track_time - 0.2).abs() < 1.0e-5);
    }

    #[test]
    fn do_invert_animation_runs_back_and_sends_the_reverse_end_list() {
        // 0x6f40e0 toggles reverse; below 0 the tick sends anim +0x19 (.ldb list 2).
        let mut scene = playing_door();
        scene.dynamic_meshes[0].anims[0].reverse_end_scripts = vec!["this->DO_Hide(true);".into()];
        scene.dynamic_meshes[0].anims[0].end_scripts = vec!["this->DO_SetHitpoints(7);".into()];
        scene.step_dynamic_anims(0.5);
        assert!(scene.dispatch_message("", "::startroom::door.DO->DO_InvertAnimation();"));
        scene.step_dynamic_anims(0.7);
        let door = &scene.dynamic_meshes[0];
        assert_eq!(door.track_time, 0.0);
        assert!(door.hidden);
        assert_eq!(door.hitpoints, None);
    }

    #[test]
    fn do_stop_freezes_and_resume_continues_from_the_saved_playhead() {
        // 0x6f4120 saves curve + playhead and clears the curve; 0x6f4150 restores them.
        let mut scene = playing_door();
        scene.step_dynamic_anims(0.4);
        assert!(scene.dispatch_message("", "::startroom::door.DO->DO_StopAnimation();"));
        scene.step_dynamic_anims(0.5);
        assert!((scene.dynamic_meshes[0].track_time - 0.4).abs() < 1.0e-5);
        assert!(scene.dispatch_message("", "::startroom::door.DO->DO_ResumeAnimation();"));
        scene.step_dynamic_anims(0.3);
        assert!((scene.dynamic_meshes[0].track_time - 0.7).abs() < 1.0e-5);
    }

    #[test]
    fn do_animate_sends_the_start_list_on_the_next_step() {
        // Apply 0x6f3d20 sends anim +0x9 (.ldb list 1) through the lagged queue.
        let mut scene = bare_scene(Vec::new(), Vec::new());
        let mut open = track("open", [0.4, 0.0, 0.0]);
        open.start_scripts = vec!["this->DO_Hide(true);".into()];
        scene.dynamic_meshes.push(door_mesh(vec![open]));
        assert!(scene.dispatch_message("", "::startroom::door.DO->DO_Animate(open);"));
        assert!(!scene.dynamic_meshes[0].hidden);
        scene.step_dynamic_anims(0.1);
        assert!(scene.dynamic_meshes[0].hidden);
    }

    #[test]
    fn retail_do_animate_strings_start_the_named_mesh() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if house.is_file() {
            let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
            let call = "::Bathroom::lid.DO->DO_Animate(open);";
            assert!(scene.dispatch_message("", call));
            let lid = scene
                .dynamic_meshes
                .iter()
                .find(|mesh| mesh.name == "::Bathroom::lid.DO")
                .expect("lid");
            assert_eq!(lid.active_track.as_deref(), Some("open"));
        }
        let startup = std::path::Path::new(
            "/tmp/omp-xdata-ref/data/database/levels/front_end/Startup_level.ldb",
        );
        if startup.is_file() {
            let mut scene = parse_ldb_scene(&std::fs::read(startup).unwrap()).expect("startup");
            let call = "::startroom::cam_rotator::cam_rotator::altitude::camera.DO->DO_Animate(1);";
            assert!(scene.dispatch_message("", call));
            let cam = scene
                .dynamic_meshes
                .iter()
                .find(|mesh| mesh.name.ends_with("altitude::camera.DO"))
                .expect("camera");
            assert_eq!(cam.active_track.as_deref(), Some("1"));
        }
    }

    #[test]
    fn test_dispatch_t_enable_disables_trigger() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        scene.triggers.push(SceneTrigger {
            name: "::Bathroom::t_toilet.TRIGGER".into(),
            room_id: 0,
            pos: [0.0; 3],
            radius: 0.2,
            trigger_type: 0,
            enabled: true,
        });
        let dispatched = scene.dispatch_message(
            "::Bathroom::lid.DO",
            "::Bathroom::t_toilet.TRIGGER->T_Enable(false);",
        );
        assert!(
            dispatched,
            "T_Enable must be recognized by dispatch_message"
        );
        let trigger = scene
            .triggers
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case("::Bathroom::t_toilet.TRIGGER"))
            .expect("trigger must exist");
        assert!(!trigger.enabled, "T_Enable(false) must disable the trigger");
    }

    #[test]
    fn retail_t_enable_disables_the_bathroom_trigger() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !house.is_file() {
            return;
        }
        let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        let name = "::Bathroom::t_toilet.TRIGGER";
        assert!(
            scene
                .triggers
                .iter()
                .any(|t| t.name.eq_ignore_ascii_case(name) && t.enabled),
            "parsed trigger starts enabled"
        );
        assert!(scene.dispatch_message(
            "::Bathroom::lid.DO",
            "::Bathroom::t_toilet.TRIGGER->T_Enable(false);",
        ));
        let trigger = scene
            .triggers
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .expect("trigger");
        assert!(!trigger.enabled);
    }

    #[test]
    fn unparsed_emitter_fails_closed() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        scene.triggers.push(SceneTrigger {
            name: "::Bathroom::t_toilet.TRIGGER".into(),
            room_id: 0,
            pos: [0.0; 3],
            radius: 0.2,
            trigger_type: 0,
            enabled: true,
        });
        let res = scene.dispatch_message(
            "::Bathroom::lid.DO",
            "::Bathroom::water_faucet->PS_StopAllEmissions(\"\");",
        );
        assert!(!res, "unparsed emitter receiver must fail closed");
        assert!(scene.triggers[0].enabled);
        assert!(scene.sound_queue.is_empty());
    }

    #[test]
    fn looping_cues_queue_with_their_emitter_and_stop_all_stops_that_emitter() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        let mut siren = crate::render3d::test_glass_quad(false);
        siren.name = "::startroom::car::siren".into();
        scene.dynamic_meshes.push(siren);
        scene.sound_catalogs.insert(
            "dynamic".into(),
            "[Police_Wail_Siren_Close_Loop]\n{\n[Sound]\nFilename = dynamic\\cars\\Police_Wail_Siren_Close.wav;\n3DSound = YES;\nHotspot = 2;\nFallOff = 50;\nLooping = YES;\n}\n[door_wood_locked]\n{\n[Sound]\nFilename = dynamic\\door\\door_wood_locked3.wav;\n}\n"
                .into(),
        );
        let ctx = "::startroom::car";
        assert!(scene.dispatch_message(
            ctx,
            "::startroom::car::siren->a_play3dsound(dynamic, Police_Wail_Siren_Close_Loop, \"\");"
        ));
        assert!(scene.dispatch_message(
            ctx,
            "::startroom::car::siren->A_Play3DSound(dynamic, door_wood_locked, \"\");"
        ));
        assert_eq!(scene.sound_queue[0].emitter, "::startroom::car::siren");
        assert!(scene.sound_queue[0].looping);
        assert_eq!(
            scene.sound_queue[0].range,
            Some(crate::SoundRange {
                hotspot: 2.0,
                falloff: 50.0,
                volume: 1.0,
            })
        );
        assert_eq!(scene.sound_queue[1].range, None);
        assert!(!scene.sound_queue[1].looping);
        assert!(scene.dispatch_message(ctx, "::startroom::car::siren->a_stopall3dsounds(\"\");"));
        assert_eq!(
            scene.sound_stops,
            vec!["::startroom::car::siren".to_string()]
        );
    }

    #[test]
    fn a_play_music_requests_the_cue_and_an_empty_cue_stops() {
        let mut scene = door_fsm_scene();
        assert!(scene.dispatch_message("::x", "maxpayne_gamemode->a_playmusic(old_building);"));
        assert_eq!(scene.music_request.as_deref(), Some("old_building"));
        assert!(scene.dispatch_message("::x", "maxpayne_gamemode->A_PlayMusic(\"\");"));
        assert_eq!(scene.music_request.as_deref(), Some(""));
    }

    #[test]
    fn cam_animate_in_place_queues_the_camera_path_block() {
        let mut scene = door_fsm_scene();
        assert!(scene.dispatch_message(
            "::startroom::door",
            "player->CAM_AnimateInPlace(shake_quick);"
        ));
        assert!(!scene.dispatch_message("::startroom::door", "player->cam_animateinplace();"));
        assert_eq!(scene.camera_shakes, ["shake_quick".to_string()]);
    }

    #[test]
    fn the_player_and_characters_speak_and_the_game_mode_plays_flat_sounds() {
        let mut scene = door_fsm_scene();
        let mut max = actor(&[]);
        max.name = "::teleport::Max".into();
        max.pos = [1.0, 0.0, 2.0];
        scene.characters.push(max);
        scene.sound_catalogs.insert(
            "maxhouse".into(),
            "[P1L0a_01]\n{\n[Sound]\nFilename = ingame\\part1\\P1L0a_01.wav;\n3DSound = YES;\nHotspot = 2;\nFallOff = 20;\n}\n[P1L0a_cam]\n{\n[Sound]\nFilename = ingame\\part1\\cam.wav;\n3DSound = NO;\n}\n"
                .into(),
        );
        assert!(scene.dispatch_message("::x", "player->A_Play3DSound(maxhouse, P1L0a_01, head);"));
        assert!(scene.dispatch_message(
            "::x",
            "::teleport::Max->a_play3dsound(maxhouse, P1L0a_01, head);"
        ));
        assert!(scene.dispatch_message(
            "::x",
            "maxpayne_gamemode->A_PlaySound(maxhouse, P1L0a_cam);"
        ));
        let emitters: Vec<_> = scene
            .sound_queue
            .iter()
            .map(|s| s.emitter.as_str())
            .collect();
        assert_eq!(emitters, ["player", "::teleport::Max", "maxpayne_gamemode"]);
        assert_eq!(scene.sound_queue[2].range, None);
        assert_eq!(
            scene.emitter_world_pos("::teleport::Max"),
            Some([1.0, 0.0, 2.0])
        );
        assert_eq!(scene.emitter_world_pos("player"), None);
    }

    #[test]
    fn a_floating_fsm_emits_its_own_sounds() {
        // The startup intro's sirens are Floating FSMs (`::startroom::police_car1::siren`).
        let mut scene = door_fsm_scene();
        scene.fsms[0].name = "::startroom::police_car1::siren".into();
        scene.sound_catalogs.insert(
            "dynamic".into(),
            "[siren_loop]\n{\n[Sound]\nFilename = dynamic\\cars\\siren.wav;\nLooping = YES;\n}\n"
                .into(),
        );
        let fsm = "::startroom::police_car1::siren";
        assert!(scene.dispatch_message(fsm, "this->A_Play3DSound(dynamic, siren_loop, \"\");"));
        assert!(scene.dispatch_message(
            "::startroom",
            "::startroom::police_car1::siren->a_stopall3dsounds(\"\");"
        ));
        assert_eq!(scene.sound_queue[0].emitter, fsm);
        assert_eq!(scene.sound_stops, vec![fsm.to_string()]);
    }

    #[test]
    fn ps_start_effect_queues_on_the_emitter_and_stop_all_emissions_stops_it() {
        let mut scene = door_fsm_scene();
        scene.fsms[0].name = "::startroom::police_car1::siren".into();
        let fsm = "::startroom::police_car1::siren";
        assert!(scene.dispatch_message(
            "::startroom",
            "::startroom::police_car1::siren->ps_starteffect(light_policecar, \"\");"
        ));
        assert!(scene.dispatch_message(fsm, "this->PS_StopAllEmissions(\"\");"));
        assert_eq!(
            scene.effect_queue,
            [QueuedEffect {
                effect: "light_policecar".into(),
                emitter: fsm.into(),
            }]
        );
        assert_eq!(scene.effect_stops, [fsm.to_string()]);
    }

    #[test]
    fn a_floating_fsm_emitter_follows_its_parent_mesh() {
        let mut scene = door_fsm_scene();
        let mut open = track("open", [0.0, 0.0, 3.0]);
        open.end_mat = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        scene.dynamic_meshes[0] = door_mesh(vec![open]);
        scene.fsms[0].name = "::startroom::door::siren".into();
        scene.fsms[0].pos = [1.0, 1.0, 2.0];
        assert_eq!(
            scene.emitter_world_pos("::startroom::door::siren"),
            Some([1.0, 1.0, 2.0])
        );
        scene.dynamic_meshes[0].toggled = true;
        assert_eq!(
            scene.emitter_world_pos("::startroom::door::siren"),
            Some([1.0, 1.0, 5.0])
        );
        assert_eq!(
            scene.emitter_world_pos("::startroom::door.DO"),
            Some([1.0, 0.0, 5.0])
        );
    }

    #[test]
    fn test_dispatch_a_play3d_sound_on_action_button_trigger() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        let receiver = "::babys_room::New mesh 24::Action button trigger 01";
        scene.triggers.push(SceneTrigger {
            name: receiver.into(),
            room_id: 0,
            pos: [0.0; 3],
            radius: 0.2,
            trigger_type: 0,
            enabled: true,
        });
        scene.sound_catalogs.insert(
            "dynamic".into(),
            "[door_wood_locked] // rattle\n{\n[Sound]\nFilename = dynamic\\door\\door_wood_locked3.wav;\n}\n"
                .into(),
        );
        let dispatched = scene.dispatch_message(
            "::babys_room::New mesh 24",
            "::babys_room::New mesh 24::Action button trigger 01->A_Play3DSound(dynamic, door_wood_locked, \"\");",
        );
        assert!(
            dispatched,
            "A_Play3DSound on an action button trigger must dispatch"
        );
        assert_eq!(scene.sound_queue.len(), 1);
        assert_eq!(scene.sound_queue[0].category, "dynamic");
        assert_eq!(scene.sound_queue[0].cue, "door_wood_locked");
        assert_eq!(
            scene.sound_queue[0].filename.as_deref(),
            Some(r"dynamic\door\door_wood_locked3.wav")
        );
    }

    #[test]
    fn retail_a_play3dsound_queues_the_locked_door_cue() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        let catalog = std::path::Path::new("/tmp/omp-xdata-ref/data/database/sounds/dynamic.txt");
        if !house.is_file() || !catalog.is_file() {
            return;
        }
        let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        let receiver = "::babys_room::New mesh 24::Action button trigger 01";
        let near: Vec<_> = scene
            .triggers
            .iter()
            .filter(|trigger| {
                trigger
                    .name
                    .to_ascii_lowercase()
                    .contains("action button trigger 01")
            })
            .map(|trigger| trigger.name.clone())
            .collect::<Vec<_>>();
        assert!(
            scene.triggers.iter().any(|trigger| {
                trigger.name.eq_ignore_ascii_case(receiver)
                    || trigger
                        .name
                        .eq_ignore_ascii_case(&format!("{receiver}.TRIGGER"))
            }),
            "parsed names: {near:?}"
        );
        scene
            .sound_catalogs
            .insert("dynamic".into(), std::fs::read_to_string(catalog).unwrap());
        let call = format!("{receiver}->A_Play3DSound(dynamic, door_wood_locked, \"\");");
        assert!(scene.dispatch_message("::babys_room::New mesh 24", &call));
        assert_eq!(scene.sound_queue.len(), 1);
        assert_eq!(scene.sound_queue[0].cue, "door_wood_locked");
        assert_eq!(
            scene.sound_queue[0].filename.as_deref(),
            Some(r"dynamic\door\door_wood_locked3.wav")
        );
    }

    #[test]
    fn e_near_the_action_button_queues_the_locked_door() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        let catalog = std::path::Path::new("/tmp/omp-xdata-ref/data/database/sounds/dynamic.txt");
        if !house.is_file() || !catalog.is_file() {
            return;
        }
        let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        let fsm_name = "::babys_room::New mesh 24::Action button trigger 01";
        let call = format!("{fsm_name}->A_Play3DSound(dynamic, door_wood_locked, \"\");");
        let fsm = scene
            .fsms
            .iter()
            .find(|fsm| fsm.name.eq_ignore_ascii_case(fsm_name))
            .expect("action fsm");
        let activate = fsm
            .hooks
            .iter()
            .find(|(name, _, _)| name.eq_ignore_ascii_case("T_Activate"))
            .expect("activate");
        assert!(
            activate.1.iter().any(|act| act == &call),
            "unconditional={:?} states={:?}",
            activate.1,
            activate.2
        );
        scene
            .sound_catalogs
            .insert("dynamic".into(), std::fs::read_to_string(catalog).unwrap());
        let trigger = scene
            .triggers
            .iter()
            .find(|trigger| {
                trigger.name.eq_ignore_ascii_case(fsm_name)
                    || trigger
                        .name
                        .eq_ignore_ascii_case(&format!("{fsm_name}.TRIGGER"))
            })
            .expect("trigger");
        let origins: BTreeMap<i32, [f32; 3]> = scene
            .rooms
            .iter()
            .map(|room| (room.id, room.world_origin))
            .collect();
        let ro = origins.get(&trigger.room_id).copied().unwrap_or([0.0; 3]);
        let at = [
            ro[0] + trigger.pos[0],
            ro[1] + trigger.pos[1],
            ro[2] + trigger.pos[2],
        ];
        assert!(scene.activate_nearest_trigger(&origins, at, 36.0));
        assert_eq!(scene.sound_queue.len(), 1);
        assert_eq!(scene.sound_queue[0].cue, "door_wood_locked");
        assert_eq!(
            scene.sound_queue[0].filename.as_deref(),
            Some(r"dynamic\door\door_wood_locked3.wav")
        );
        scene.sound_queue.clear();
        let far = [at[0] + 100.0, at[1], at[2]];
        assert!(!scene.activate_nearest_trigger(&origins, far, 36.0));
        assert!(scene.sound_queue.is_empty());
    }

    const HIDE: &str = "::startroom::door.DO->DO_Hide(true);";
    const SHOW: &str = "::startroom::door.DO->DO_Hide(false);";

    /// `::startroom::door` FSM in state `closed`, next to its door mesh.
    fn door_fsm_scene() -> LdbScene {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        scene.dynamic_meshes.push(door_mesh(Vec::new()));
        scene.fsms.push(SceneFsm {
            name: "::startroom::door".into(),
            room_id: 1,
            pos: [0.0; 3],
            states: vec!["closed".into(), "open".into()],
            default_state: "closed".into(),
            startup_actions: Vec::new(),
            hooks: vec![(
                "T_Activate".into(),
                Vec::new(),
                vec![("open".into(), vec![SHOW.into()])],
            )],
            state_scripts: vec![(
                "open".into(),
                Vec::new(),
                vec![("closed".into(), vec![HIDE.into()])],
            )],
            commands: vec![(
                "toggle".into(),
                Vec::new(),
                vec![
                    ("closed".into(), vec![HIDE.into()]),
                    ("open".into(), vec![SHOW.into()]),
                ],
            )],
            state: "closed".into(),
        });
        scene
    }

    #[test]
    fn fsm_switch_runs_the_new_state_script_with_the_previous_state_section() {
        // 0x6e30a0 saves the old state (0x6e1670), stores the new one at +0x365, then runs the
        // state's script (map +0x441, 0x6e3910) with the old state.
        let mut scene = door_fsm_scene();
        assert!(scene.dispatch_message("", "::startroom::door->FSM_Switch(open);"));
        assert_eq!(scene.fsms[0].state, "open");
        assert!(scene.dynamic_meshes[0].hidden);
    }

    #[test]
    fn fsm_send_runs_the_command_section_for_the_current_state() {
        // 0x6e2f10 looks the command up in map +0x431 (0x6e37c0) with the current state.
        let mut scene = door_fsm_scene();
        assert!(scene.dispatch_message("", "::startroom::door->FSM_Send(toggle);"));
        assert!(scene.dynamic_meshes[0].hidden);
        scene.fsms[0].state = "open".into();
        assert!(scene.dispatch_message("", "::startroom::door->FSM_Send(toggle);"));
        assert!(!scene.dynamic_meshes[0].hidden);
    }

    #[test]
    fn event_hooks_run_the_current_state_section() {
        let mut scene = door_fsm_scene();
        scene.dynamic_meshes[0].hidden = true;
        assert!(!scene.dispatch_hook("::startroom::door", "T_Activate"));
        assert!(scene.dynamic_meshes[0].hidden);
        scene.fsms[0].state = "open".into();
        assert!(scene.dispatch_hook("::startroom::door", "T_Activate"));
        assert!(!scene.dynamic_meshes[0].hidden);
    }

    #[test]
    fn fsm_randomize_switches_to_a_state_picked_by_crt_rand() {
        // 0x6e3310: `rand() % states` (MSVCRT `rand`, IAT 0x7ca368), then the Switch path.
        // The first `rand()` of an unseeded CRT is 41, so state 1 of 2.
        let mut scene = door_fsm_scene();
        assert!(scene.dispatch_message("", "::startroom::door->FSM_Randomize();"));
        assert_eq!(scene.fsms[0].state, "open");
        assert!(scene.dynamic_meshes[0].hidden, "the open state script ran");
    }

    #[test]
    fn gm_change_fog_sets_the_fog_and_fades_to_it() {
        // Startup intro: `maxpayne_gamemode->gm_changefog(0x141414, -200, 400, 0);`.
        let mut scene = bare_scene(Vec::new(), Vec::new());
        assert!(scene.dispatch_message(
            "",
            "maxpayne_gamemode->gm_changefog(0x141414, -200, 400, 0);"
        ));
        let near = LevelFog {
            color: [20.0, 20.0, 20.0],
            start: -200.0,
            end: 400.0,
        };
        assert_eq!(scene.fog, Some(near));
        // Part2_Level3: `gm_changefog(0x441616, 0, 80, 0)`; a 2 s fade is half way after 1 s.
        assert!(scene.dispatch_message("", "MaxPayne_GameMode->GM_ChangeFog(0x441616, 0, 80, 2);"));
        scene.step_fog(1.0);
        assert_eq!(
            scene.fog,
            Some(LevelFog {
                color: [44.0, 21.0, 21.0],
                start: -100.0,
                end: 240.0
            })
        );
        scene.step_fog(1.5);
        assert_eq!(
            scene.fog,
            Some(LevelFog {
                color: [68.0, 22.0, 22.0],
                start: 0.0,
                end: 80.0
            })
        );
    }

    #[test]
    fn s_mode_switch_records_the_requested_mode() {
        // The startup level ends its intro with `x_modeswitch->s_modeswitch(graphicnovel);`.
        let mut scene = bare_scene(Vec::new(), Vec::new());
        assert_eq!(scene.mode_switch, None);
        assert!(scene.dispatch_message("", "x_modeswitch->s_modeswitch(graphicnovel);"));
        assert_eq!(scene.mode_switch.as_deref(), Some("graphicnovel"));
    }

    #[test]
    fn startup_do_animate_starts_the_track() {
        // `::start::Start_FSM` starts the intro with `Start_FSM.DO->DO_Animate(1);` at startup.
        let mut scene = bare_scene(Vec::new(), Vec::new());
        scene
            .dynamic_meshes
            .push(door_mesh(vec![track("open", [0.4, 0.0, 0.0])]));
        let mut fsm = door_fsm_scene().fsms.remove(0);
        fsm.startup_actions = vec!["::startroom::door.DO->DO_Animate(open);".into()];
        scene.fsms.push(fsm);
        scene.apply_startup_animates();
        assert_eq!(
            scene.dynamic_meshes[0].active_track.as_deref(),
            Some("open")
        );
    }

    #[test]
    fn startup_fsm_send_runs_the_command() {
        let mut scene = door_fsm_scene();
        scene.fsms[0]
            .startup_actions
            .push("::startroom::door->FSM_Send(toggle);".into());
        scene.apply_startup_sends();
        assert!(scene.dynamic_meshes[0].hidden);
    }

    #[test]
    fn fsm_switch_to_itself_from_its_own_script_stops() {
        // Retail warns at a depth above 32 (`0x89b294`, "recursively switching").
        let mut scene = door_fsm_scene();
        scene.fsms[0].state_scripts[0]
            .1
            .push("::startroom::door->FSM_Switch(open);".into());
        assert!(scene.dispatch_message("", "::startroom::door->FSM_Switch(open);"));
        assert_eq!(scene.fsms[0].state, "open");
    }

    #[test]
    fn do_enable_collisions_sets_the_mesh_flag() {
        // 0x6f43f0 stores the argument in +0x4ae, which characters and the player camera
        // test before reacting to a DO contact.
        let mut scene = bare_scene(Vec::new(), Vec::new());
        scene.dynamic_meshes.push(door_mesh(Vec::new()));
        assert!(scene.dispatch_message("", "::startroom::door.DO->DO_EnableCollisions(false);"));
        assert!(!scene.dynamic_meshes[0].collisions);
        assert!(scene.dispatch_message("", "::startroom::door.DO->DO_EnableCollisions(true);"));
        assert!(scene.dynamic_meshes[0].collisions);
    }

    #[test]
    fn cam_attach_to_do_sets_and_clears_the_attached_camera() {
        // `::finito::camera2` starts with `CAM_AttachToDO(100, false)` and its last track ends
        // with `CAM_AttachToDO(0, false)` plus `GM_SetPlayerControls(true)`.
        let mut scene = bare_scene(Vec::new(), Vec::new());
        scene.dynamic_meshes.push(door_mesh(Vec::new()));
        assert!(scene.dispatch_message("", "::startroom::door.DO->CAM_AttachToDO(100, false);"));
        assert_eq!(scene.attached_camera, Some(0));
        assert!(scene.dispatch_message("", "::startroom::door.DO->CAM_AttachToDO(0, false);"));
        assert_eq!(scene.attached_camera, None);
    }

    #[test]
    fn gm_set_player_controls_toggles_the_flag() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        assert!(scene.player_controls);
        assert!(scene.dispatch_message("", "maxpayne_gamemode->GM_SetPlayerControls(false);"));
        assert!(!scene.player_controls);
        assert!(scene.dispatch_message("", "MaxPayne_GameMode->gm_setplayercontrols(true);"));
        assert!(scene.player_controls);
    }

    #[test]
    fn mphm_fade_to_color_ramps_rrggbbaa_and_holds_the_target() {
        // `::finito::camera2` track 2 ends with `MPHM_FadeToColor(0x0, 0x000000ff, 0.4)`.
        let mut scene = bare_scene(Vec::new(), Vec::new());
        assert!(scene.dispatch_message(
            "",
            "maxpayne_hudmode->MPHM_FadeToColor(0x0, 0x000000ff, 0.4);"
        ));
        assert_eq!(scene.screen_fade.map(|f| f.color()), Some([0, 0, 0, 0]));
        scene.step_screen_fade(0.2);
        assert_eq!(scene.screen_fade.map(|f| f.color()), Some([0, 0, 0, 128]));
        scene.step_screen_fade(1.0);
        assert_eq!(scene.screen_fade.map(|f| f.color()), Some([0, 0, 0, 255]));
    }

    #[test]
    fn mphm_fade_to_transparent_clears_the_fade_when_done() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        assert!(scene.dispatch_message(
            "",
            "maxpayne_hudmode->mphm_fadetocolor(0x000000ff, 0x0, 0.5);"
        ));
        scene.step_screen_fade(0.6);
        assert_eq!(scene.screen_fade, None);
    }

    #[test]
    fn test_dispatch_do_hide_hides_dynamic_mesh() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        let mut mesh = door_mesh(Vec::new());
        mesh.name = "::Bathroom::glass01.DO".into();
        mesh.hidden = false;
        scene.dynamic_meshes.push(mesh);
        let dispatched = scene.dispatch_message(
            "::Bathroom::glass01.DO",
            "::Bathroom::glass01.DO->do_hide(true);",
        );
        assert!(dispatched, "do_hide must be recognized by dispatch_message");
        let mesh = scene
            .dynamic_meshes
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case("::Bathroom::glass01.DO"))
            .expect("glass mesh must exist");
        assert!(mesh.hidden, "do_hide(true) must set mesh.hidden == true");
    }

    #[test]
    fn script_booleans_ignore_case() {
        // The startup intro's frame track ends with `cam_fade_1.DO->DO_Hide(TRUE);` (upper case,
        // next to `frame.DO->DO_Hide(true);`): the black wipe plate must hide too.
        let mut scene = bare_scene(Vec::new(), Vec::new());
        let mut mesh = door_mesh(Vec::new());
        mesh.name = "::start::cam_fade_1.DO".into();
        mesh.hidden = false;
        scene.dynamic_meshes.push(mesh);
        assert!(scene.dispatch_message(
            "::start::frame.DO",
            "::start::cam_fade_1.DO->DO_Hide(TRUE);"
        ));
        assert!(scene.dynamic_meshes[0].hidden);
        assert!(scene.dispatch_message(
            "::start::frame.DO",
            "::start::cam_fade_1.DO->DO_HIDE(FALSE);"
        ));
        assert!(!scene.dynamic_meshes[0].hidden);
    }

    #[test]
    fn glass_death_hook_hides_the_mesh() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !house.is_file() {
            return;
        }
        let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        let fsm = scene
            .fsms
            .iter()
            .find(|fsm| fsm.name.eq_ignore_ascii_case("::Bathroom::glass01"))
            .expect("glass fsm");
        let death = fsm
            .hooks
            .iter()
            .find(|(name, _, _)| name.eq_ignore_ascii_case("DO_OnDeath"))
            .expect("death hook");
        assert!(
            death
                .1
                .iter()
                .any(|act| act.eq_ignore_ascii_case("::Bathroom::glass01.DO->do_hide(true);")),
            "hooks={:?}",
            fsm.hooks
                .iter()
                .map(|(name, acts, states)| (name.clone(), acts.clone(), states.clone()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            fsm.startup_actions,
            ["::Bathroom::glass01.DO->DO_SetHitpoints(20);"]
        );
        let bullet = fsm
            .hooks
            .iter()
            .find(|(name, _, _)| name.eq_ignore_ascii_case("DO_BulletCollides"))
            .map(|(_, acts, _)| acts.as_slice())
            .unwrap_or(&[]);
        assert!(bullet.is_empty());
        let projectile = "::Bathroom::glass01->P_CreateProjectile(debris_glass, 1);";
        assert!(
            death.1.iter().any(|act| act == projectile),
            "death={:?}",
            death.1
        );
        assert!(!scene.dispatch_message("::Bathroom::glass01", projectile));
        assert!(scene.sound_queue.is_empty());
    }

    #[test]
    fn toilet_lid_open_is_the_activate_hook() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !house.is_file() {
            return;
        }
        let scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        let fsm = scene
            .fsms
            .iter()
            .find(|fsm| fsm.name.eq_ignore_ascii_case("::Bathroom::t_toilet"))
            .expect("toilet fsm");
        assert_eq!(fsm.states, ["closed", "open"]);
        assert_eq!(fsm.default_state, "open");
        let activate = fsm
            .hooks
            .iter()
            .find(|(name, _, _)| name == "T_Activate")
            .expect("activate");
        assert!(activate.1.is_empty(), "unconditional={:?}", activate.1);
        assert_eq!(
            activate.2,
            [
                (
                    "closed".to_string(),
                    vec![
                        "::Bathroom::lid.DO->DO_Animate(open);".to_string(),
                        "::Bathroom::t_toilet.TRIGGER->T_Enable(false);".to_string(),
                    ],
                ),
                (
                    "open".to_string(),
                    vec![
                        "::Bathroom::lid.DO->DO_Animate(close);".to_string(),
                        "::Bathroom::t_toilet.TRIGGER->T_Enable(false);".to_string(),
                    ],
                ),
            ]
        );
        let lid = scene
            .dynamic_meshes
            .iter()
            .find(|mesh| mesh.name.eq_ignore_ascii_case("::Bathroom::lid.DO"))
            .expect("lid");
        let names: Vec<_> = lid.anims.iter().map(|anim| anim.name.as_str()).collect();
        assert_eq!(names, ["close", "open"], "anims={names:?}");
    }

    #[test]
    fn bathroom_unconditional_hooks_outside_glass_use_known_verbs() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !house.is_file() {
            return;
        }
        let scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        let known = [
            "do_animate",
            "t_enable",
            "a_play3dsound",
            "do_hide",
            "do_sethitpoints",
            "ps_starteffect",
            "ps_stopallemissions",
            "p_createprojectile",
        ];
        let mut other = Vec::new();
        for fsm in scene.fsms.iter().filter(|fsm| {
            fsm.name.to_ascii_lowercase().contains("bathroom")
                && !fsm.name.eq_ignore_ascii_case("::Bathroom::glass01")
        }) {
            for (hook, acts, _) in &fsm.hooks {
                for act in acts {
                    let Some(msg) = Message::parse(act) else {
                        continue;
                    };
                    let verb = msg.verb.to_ascii_lowercase();
                    if !known.contains(&verb.as_str()) {
                        other.push(format!("{} / {} / {act}", fsm.name, hook));
                    }
                }
            }
        }
        assert_eq!(
            other,
            [
                "::Bathroom::camera2 / start / ::Bathroom::camera2.DO->CAM_AttachToDO(100, false);",
                "::Bathroom::lid / DO_MovedToInvalidPosition / ::Bathroom::lid.DO->DO_InvertAnimation();",
                "::Bathroom::water_faucet / T_Activate / ::Bathroom::water_faucet->A_StopAll3DSounds(\"\");",
            ]
        );
        let faucet = scene
            .fsms
            .iter()
            .find(|fsm| fsm.name.eq_ignore_ascii_case("::Bathroom::water_faucet"))
            .expect("faucet");
        let activate = faucet
            .hooks
            .iter()
            .find(|(name, _, _)| name == "T_Activate")
            .expect("faucet activate");
        assert!(activate.2.is_empty(), "states={:?}", activate.2);
        assert_eq!(
            activate.1,
            [
                "::Bathroom::water_faucet->PS_StopAllEmissions(\"\");",
                "::Bathroom::water_faucet->A_StopAll3DSounds(\"\");",
                "::Bathroom::water_faucet->ps_starteffect(faucet_open_clear, \"\");",
                "::Bathroom::water_faucet->A_Play3DSound(dynamic, faucet_open, \"\");",
            ]
        );
    }

    #[test]
    fn four_beretta_shots_break_the_bathroom_glass() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !house.is_file() {
            return;
        }
        let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        scene.apply_startup_hitpoints();
        let index = scene
            .dynamic_meshes
            .iter()
            .position(|mesh| mesh.name.eq_ignore_ascii_case("::Bathroom::glass01.DO"))
            .expect("glass");
        assert_eq!(scene.dynamic_meshes[index].hitpoints, Some(20));
        assert!(!scene.dynamic_meshes[index].hidden);
        for _ in 0..3 {
            assert!(!scene.damage_mesh(index, 5));
            assert!(!scene.dynamic_meshes[index].hidden);
        }
        assert!(scene.damage_mesh(index, 5));
        assert!(scene.dynamic_meshes[index].hidden);
        assert_eq!(scene.dynamic_meshes[index].hitpoints, Some(0));
    }

    #[test]
    fn four_beretta_shots_break_the_second_bathroom_pane() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !house.is_file() {
            return;
        }
        let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        scene.apply_startup_hitpoints();
        let index = scene
            .dynamic_meshes
            .iter()
            .position(|mesh| mesh.name.eq_ignore_ascii_case("::Bathroom::glass02.DO"))
            .expect("glass02");
        assert_eq!(scene.dynamic_meshes[index].hitpoints, Some(20));
        for _ in 0..3 {
            assert!(!scene.damage_mesh(index, 5));
        }
        assert!(scene.damage_mesh(index, 5));
        assert!(scene.dynamic_meshes[index].hidden);
        assert_eq!(scene.dynamic_meshes[index].hitpoints, Some(0));
    }

    #[test]
    fn house_death_hooks_that_do_not_start_with_hide() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !house.is_file() {
            return;
        }
        let scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        let mut other = Vec::new();
        for fsm in &scene.fsms {
            let Some(first) = fsm.hooks.iter().find_map(|(name, acts, _)| {
                if name.eq_ignore_ascii_case("DO_OnDeath") {
                    acts.first().cloned()
                } else {
                    None
                }
            }) else {
                continue;
            };
            let Some(msg) = Message::parse(&first) else {
                continue;
            };
            if !msg.verb.eq_ignore_ascii_case("do_hide") {
                other.push(format!("{} / {first}", fsm.name));
            }
        }
        assert!(other.is_empty(), "{other:?}");
    }

    #[test]
    fn startup_hides_the_broken_entrance_tv() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !house.is_file() {
            return;
        }
        let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        let hidden_at_start = [
            "::entrance05::New mesh 57::tv5::tv01.DO",
            "::entrance05::New mesh 57::tv5::tv02.DO",
            "::entrance05::New mesh 57::tv5::tv03.DO",
            "::entrance05::New mesh 57::tv5::tv04.DO",
            "::entrance05::New mesh 57::tv5::tv_broken.DO",
        ];
        for name in hidden_at_start {
            let mesh = scene
                .dynamic_meshes
                .iter()
                .find(|mesh| mesh.name.eq_ignore_ascii_case(name))
                .expect(name);
            assert!(!mesh.hidden, "{name}");
        }
        scene.apply_startup_hides();
        for name in hidden_at_start {
            let mesh = scene
                .dynamic_meshes
                .iter()
                .find(|mesh| mesh.name.eq_ignore_ascii_case(name))
                .expect(name);
            assert!(mesh.hidden, "{name}");
        }
        let glass = scene
            .dynamic_meshes
            .iter()
            .find(|mesh| mesh.name.eq_ignore_ascii_case("::Bathroom::glass01.DO"))
            .expect("glass");
        assert!(!glass.hidden);
    }

    #[test]
    fn startup_disables_the_bathroom_muffle_trigger() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !house.is_file() {
            return;
        }
        let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        let muffle = scene
            .triggers
            .iter()
            .find(|trigger| {
                trigger
                    .name
                    .eq_ignore_ascii_case("::Bathroom::muffle_sound_trigger.TRIGGER")
            })
            .expect("muffle");
        assert!(muffle.enabled);
        let toilet = scene
            .triggers
            .iter()
            .find(|trigger| {
                trigger
                    .name
                    .eq_ignore_ascii_case("::Bathroom::t_toilet.TRIGGER")
            })
            .expect("toilet");
        assert!(toilet.enabled);
        scene.apply_startup_enables();
        let muffle = scene
            .triggers
            .iter()
            .find(|trigger| {
                trigger
                    .name
                    .eq_ignore_ascii_case("::Bathroom::muffle_sound_trigger.TRIGGER")
            })
            .expect("muffle");
        assert!(!muffle.enabled);
        let toilet = scene
            .triggers
            .iter()
            .find(|trigger| {
                trigger
                    .name
                    .eq_ignore_ascii_case("::Bathroom::t_toilet.TRIGGER")
            })
            .expect("toilet");
        assert!(toilet.enabled);
    }

    #[test]
    fn retail_do_hide_hides_bathroom_glass() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        if !house.is_file() {
            return;
        }
        let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        let name = "::Bathroom::glass01.DO";
        let mesh = scene
            .dynamic_meshes
            .iter()
            .find(|mesh| mesh.name.eq_ignore_ascii_case(name))
            .expect("glass mesh");
        assert!(!mesh.hidden);
        assert!(scene.dispatch_message(name, "::Bathroom::glass01.DO->do_hide(true);"));
        let mesh = scene
            .dynamic_meshes
            .iter()
            .find(|mesh| mesh.name.eq_ignore_ascii_case(name))
            .expect("glass mesh");
        assert!(mesh.hidden);
    }

    #[test]
    fn test_dispatch_a_play3d_sound_on_dynamic_mesh_without_do_suffix() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        scene.sound_catalogs.insert(
            "breakable".into(),
            "[window_break_medium] // Glass breaking\n{\n[Sound]\nFilename = breakable\\glass\\break_glass_medium.wav;\n}\n"
                .into(),
        );
        let mut mesh = door_mesh(Vec::new());
        mesh.name = "::Bathroom::glass01.DO".into();
        mesh.hidden = true;
        scene.dynamic_meshes.push(mesh);
        let dispatched = scene.dispatch_message(
            "::Bathroom::glass01.DO",
            "::Bathroom::glass01->A_Play3DSound(breakable, window_break_medium, \"\");",
        );
        assert!(
            dispatched,
            "a_play3dsound must resolve ::Bathroom::glass01 to ::Bathroom::glass01.DO"
        );
        assert_eq!(scene.sound_queue.len(), 1);
        assert_eq!(scene.sound_queue[0].cue, "window_break_medium");
        assert_eq!(
            scene.sound_queue[0].filename.as_deref(),
            Some(r"breakable\glass\break_glass_medium.wav")
        );
    }

    #[test]
    fn retail_glass_break_sound_resolves_the_mesh_without_do_suffix() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        let catalog = std::path::Path::new("/tmp/omp-xdata-ref/data/database/sounds/breakable.txt");
        if !house.is_file() || !catalog.is_file() {
            return;
        }
        let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        assert!(scene
            .dynamic_meshes
            .iter()
            .any(|mesh| mesh.name.eq_ignore_ascii_case("::Bathroom::glass01.DO")));
        scene.sound_catalogs.insert(
            "breakable".into(),
            std::fs::read_to_string(catalog).unwrap(),
        );
        assert!(scene.dispatch_message(
            "::Bathroom::glass01.DO",
            "::Bathroom::glass01->A_Play3DSound(breakable, window_break_medium, \"\");",
        ));
        assert_eq!(scene.sound_queue.len(), 1);
        assert_eq!(scene.sound_queue[0].cue, "window_break_medium");
        assert_eq!(
            scene.sound_queue[0].filename.as_deref(),
            Some(r"breakable\glass\break_glass_medium.wav")
        );
    }

    #[test]
    fn test_dispatch_drawer_opener_trigger_sound() {
        let mut scene = bare_scene(Vec::new(), Vec::new());
        scene.sound_catalogs.insert(
            "dynamic".into(),
            "[Wooden_Desk_Drawer_Open]\n{\n[Sound]\nFilename = dynamic\\door\\Wooden_Desk_Drawer_Open.wav;\n}\n"
                .into(),
        );
        scene.triggers.push(SceneTrigger {
            name: "::entrance03::New mesh 62::drawer::drawer-opener.TRIGGER".into(),
            room_id: 0,
            pos: [0.0; 3],
            radius: 0.2,
            trigger_type: 0,
            enabled: true,
        });
        let dispatched = scene.dispatch_message(
            "::entrance03::New mesh 62::drawer.DO",
            "::entrance03::New mesh 62::drawer::drawer-opener->A_Play3DSound(dynamic, Wooden_Desk_Drawer_Open, \"\");",
        );
        assert!(
            dispatched,
            "a_play3dsound must resolve drawer-opener to its .TRIGGER entity record"
        );
        assert_eq!(scene.sound_queue.len(), 1);
        assert_eq!(scene.sound_queue[0].cue, "Wooden_Desk_Drawer_Open");
        assert_eq!(
            scene.sound_queue[0].filename.as_deref(),
            Some(r"dynamic\door\Wooden_Desk_Drawer_Open.wav")
        );
    }

    #[test]
    fn retail_drawer_sound_resolves_drawer_opener_trigger() {
        let house =
            std::path::Path::new("/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb");
        let catalog = std::path::Path::new("/tmp/omp-xdata-ref/data/database/sounds/dynamic.txt");
        if !house.is_file() || !catalog.is_file() {
            return;
        }
        let mut scene = parse_ldb_scene(&std::fs::read(house).unwrap()).expect("house");
        let near: Vec<_> = scene
            .triggers
            .iter()
            .filter(|trigger| trigger.name.to_ascii_lowercase().contains("drawer-opener"))
            .map(|trigger| trigger.name.clone())
            .collect::<Vec<_>>();
        scene
            .sound_catalogs
            .insert("dynamic".into(), std::fs::read_to_string(catalog).unwrap());
        let dispatched = scene.dispatch_message(
            "::entrance03::New mesh 62::drawer.DO",
            "::entrance03::New mesh 62::drawer::drawer-opener->A_Play3DSound(dynamic, Wooden_Desk_Drawer_Open, \"\");",
        );
        assert!(dispatched, "parsed trigger names: {near:?}");
        let queued = scene.sound_queue.last().expect("queued sound");
        assert_eq!(queued.cue, "Wooden_Desk_Drawer_Open");
        assert_eq!(
            queued.filename.as_deref(),
            Some(r"dynamic\door\Wooden_Desk_Drawer_Open.wav")
        );
    }
}
