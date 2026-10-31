//! Interactive 3D textured + lightmapped first-person renderer for Max Payne 1 `.ldb` scenes.
//!
//! Features:
//! - Perspective camera with WASD movement, mouse/arrow look, Space jump, Shift sprint, `G` fly/walk mode
//! - Capsule floor snapping + wall collision against static and dynamic room geometry
//! - Perspective-correct (or subdivided affine) texture + `256×256` RGB lightmap modulation
//! - Interactive `E` use key: triggers nearby doors / dynamic meshes (`DO_Animate`) and prints FSM scripts
//! - `Tab` cycles between level Jumppoints / Rooms; `L` toggles lightmaps; `F` toggles wireframe

use crate::scene::{DecodedLightmap, DecodedTexture, LdbScene, ScenePoly};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use std::collections::BTreeMap;

const W: usize = 960;
const H: usize = 600;
const NEAR_Z: f32 = 0.15;
const FAR_Z: f32 = 220.0;
const EYE_HEIGHT: f32 = 1.65;

#[derive(Clone, Copy)]
struct ProjVert {
    sx: f32,
    sy: f32,
    inv_z: f32,
    u_over_z: f32,
    v_over_z: f32,
    lu_over_z: f32,
    lv_over_z: f32,
}

#[derive(Clone, Copy)]
struct CamVert {
    cx: f32,
    cy: f32,
    cz: f32,
    u: f32,
    v: f32,
    lu: f32,
    lv: f32,
}

pub fn run_3d_walkthrough(mut scene: LdbScene, title: &str) -> Result<(), String> {
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
    window.set_target_fps(60);

    // Room origin lookup
    let mut room_origins: BTreeMap<i32, [f32; 3]> = BTreeMap::new();
    for r in &scene.rooms {
        room_origins.insert(r.id, r.world_origin);
    }

    // Spawn at StartRoom::Jumppoint 00 (or first jumppoint)
    let mut jp_idx = scene
        .jumppoints
        .iter()
        .position(|j| j.name.contains("StartRoom") && j.name.contains("Jumppoint"))
        .or_else(|| {
            scene
                .jumppoints
                .iter()
                .position(|j| j.name.contains("Jumppoint"))
        })
        .unwrap_or(0);

    let spawn_pos =
        |scene: &LdbScene, room_origins: &BTreeMap<i32, [f32; 3]>, idx: usize| -> [f32; 3] {
            if let Some(jp) = scene.jumppoints.get(idx) {
                let ro = room_origins.get(&jp.room_id).copied().unwrap_or([0.0; 3]);
                [
                    ro[0] + jp.pos[0],
                    ro[1] + jp.pos[1] + EYE_HEIGHT,
                    ro[2] + jp.pos[2],
                ]
            } else if let Some(sm) = scene.static_meshes.first() {
                if let Some(p) = sm.polys.first().and_then(|p| p.vertices.first()) {
                    [
                        sm.world_origin[0] + p.pos[0],
                        sm.world_origin[1] + p.pos[1] + EYE_HEIGHT,
                        sm.world_origin[2] + p.pos[2],
                    ]
                } else {
                    [0.0, EYE_HEIGHT, 0.0]
                }
            } else {
                [0.0, EYE_HEIGHT, 0.0]
            }
        };

    let mut cam_pos = spawn_pos(&scene, &room_origins, jp_idx);
    let mut yaw: f32 = std::f32::consts::PI; // face -Z into room by default
    let mut pitch: f32 = 0.0;
    let mut vel_y: f32 = 0.0;
    let mut fly_mode = false;
    let mut use_lightmaps = true;
    let mut wireframe = false;
    let mut last_mouse: Option<(f32, f32)> = None;
    let mut hud_msg = format!(
        "Spawned at {} | WASD move, Mouse/Arrows look, E interact, Tab next spawn, G fly/walk, L lightmap, F wire",
        scene
            .jumppoints
            .get(jp_idx)
            .map(|j| j.name.as_str())
            .unwrap_or("origin")
    );
    println!("  3D Engine: {hud_msg}");

    let mut fb = vec![0u32; W * H];
    let mut zb = vec![f32::INFINITY; W * H];
    let dt = 1.0 / 60.0f32;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        // Toggle keys
        if window.is_key_pressed(Key::Tab, KeyRepeat::No) && !scene.jumppoints.is_empty() {
            jp_idx = (jp_idx + 1) % scene.jumppoints.len();
            cam_pos = spawn_pos(&scene, &room_origins, jp_idx);
            vel_y = 0.0;
            if let Some(jp) = scene.jumppoints.get(jp_idx) {
                hud_msg = format!("Teleported to {} (room {})", jp.name, jp.room_id);
                println!("  3D Engine: {hud_msg}");
            }
        }
        if window.is_key_pressed(Key::G, KeyRepeat::No) {
            fly_mode = !fly_mode;
            vel_y = 0.0;
            hud_msg = format!(
                "Mode: {}",
                if fly_mode {
                    "Free-Fly"
                } else {
                    "Walk + Gravity"
                }
            );
            println!("  3D Engine: {hud_msg}");
        }
        if window.is_key_pressed(Key::L, KeyRepeat::No) {
            use_lightmaps = !use_lightmaps;
        }
        if window.is_key_pressed(Key::F, KeyRepeat::No) {
            wireframe = !wireframe;
        }

        // Interact (`E`): trigger nearest dynamic mesh / door or FSM within 6m
        if window.is_key_pressed(Key::E, KeyRepeat::No) {
            let mut best_dm: Option<(usize, f32)> = None;
            for (idx, dm) in scene.dynamic_meshes.iter().enumerate() {
                if dm.anims.is_empty() {
                    continue;
                }
                let ro = room_origins.get(&dm.room_id).copied().unwrap_or([0.0; 3]);
                let wx = ro[0] + dm.world_pos[0];
                let wy = ro[1] + dm.world_pos[1];
                let wz = ro[2] + dm.world_pos[2];
                let d2 = (wx - cam_pos[0]).powi(2)
                    + (wy - cam_pos[1]).powi(2)
                    + (wz - cam_pos[2]).powi(2);
                if d2 < 36.0 && best_dm.is_none_or(|(_, b)| d2 < b) {
                    best_dm = Some((idx, d2));
                }
            }
            if let Some((idx, _)) = best_dm {
                let dm = &mut scene.dynamic_meshes[idx];
                dm.toggled = !dm.toggled;
                if let Some(anim) = dm.anims.last() {
                    hud_msg = format!(
                        "DO_Animate {} ({}) -> {:?}",
                        dm.name,
                        anim.name,
                        anim.scripts.first().map(|s| s.as_str()).unwrap_or("ok")
                    );
                    println!("  3D Engine: {hud_msg}");
                }
            }
        }

        // Look controls (Arrows + left-mouse drag)
        let turn_speed = 2.2 * dt;
        if window.is_key_down(Key::Left) {
            yaw -= turn_speed;
        }
        if window.is_key_down(Key::Right) {
            yaw += turn_speed;
        }
        if window.is_key_down(Key::Up) {
            pitch = (pitch + turn_speed).min(1.45);
        }
        if window.is_key_down(Key::Down) {
            pitch = (pitch - turn_speed).max(-1.45);
        }
        if window.get_mouse_down(MouseButton::Left) {
            if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Pass) {
                if let Some((lx, ly)) = last_mouse {
                    yaw += (mx - lx) * 0.006;
                    pitch = (pitch - (my - ly) * 0.006).clamp(-1.45, 1.45);
                }
                last_mouse = Some((mx, my));
            }
        } else {
            last_mouse = None;
        }

        // Movement controls (WASD)
        let speed = if window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift) {
            14.0
        } else {
            6.5
        };
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let fwd = if fly_mode {
            [sy * cp, sp, cy * cp]
        } else {
            [sy, 0.0, cy]
        };
        let right = [cy, 0.0, -sy];
        let mut mv = [0.0f32; 3];
        if window.is_key_down(Key::W) {
            mv[0] += fwd[0];
            mv[1] += fwd[1];
            mv[2] += fwd[2];
        }
        if window.is_key_down(Key::S) {
            mv[0] -= fwd[0];
            mv[1] -= fwd[1];
            mv[2] -= fwd[2];
        }
        if window.is_key_down(Key::D) {
            mv[0] += right[0];
            mv[2] += right[2];
        }
        if window.is_key_down(Key::A) {
            mv[0] -= right[0];
            mv[2] -= right[2];
        }
        if fly_mode {
            if window.is_key_down(Key::Space) {
                mv[1] += 1.0;
            }
            if window.is_key_down(Key::C) {
                mv[1] -= 1.0;
            }
            cam_pos[0] += mv[0] * speed * dt;
            cam_pos[1] += mv[1] * speed * dt;
            cam_pos[2] += mv[2] * speed * dt;
        } else {
            let next_x = cam_pos[0] + mv[0] * speed * dt;
            let next_z = cam_pos[2] + mv[2] * speed * dt;
            if !hits_wall(&scene, &room_origins, next_x, cam_pos[1], next_z) {
                cam_pos[0] = next_x;
                cam_pos[2] = next_z;
            }
            if window.is_key_pressed(Key::Space, KeyRepeat::No) && vel_y.abs() < 0.05 {
                vel_y = 5.5;
            }
            vel_y -= 15.0 * dt;
            cam_pos[1] += vel_y * dt;
            if let Some(floor_y) =
                find_floor_y(&scene, &room_origins, cam_pos[0], cam_pos[1], cam_pos[2])
            {
                if cam_pos[1] <= floor_y + EYE_HEIGHT {
                    cam_pos[1] = floor_y + EYE_HEIGHT;
                    vel_y = 0.0;
                }
            }
        }

        // Clear sky/ceiling + depth buffer
        for y in 0..H {
            let row_col = if y < H / 2 { 0x00161c28 } else { 0x001e1e22 };
            fb[y * W..(y + 1) * W].fill(row_col);
        }
        zb.fill(f32::INFINITY);

        // Camera basis
        let cam_right = [cy, 0.0, -sy];
        let cam_up = [-sy * sp, cp, -cy * sp];
        let cam_fwd = [sy * cp, sp, cy * cp];
        let fov_scale = (H as f32) * 0.85;

        // Render static room meshes
        const IDENT_MAT: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        for sm in &scene.static_meshes {
            render_poly_batch(
                &sm.polys,
                IDENT_MAT,
                sm.world_origin,
                &scene,
                cam_pos,
                cam_right,
                cam_up,
                cam_fwd,
                fov_scale,
                use_lightmaps,
                wireframe,
                &mut fb,
                &mut zb,
            );
        }

        // Render dynamic meshes (doors, breakables, furniture, vans, cameras)
        for dm in &scene.dynamic_meshes {
            // Skip invisible FSM helper cubes (`01_Tip_*`, `Start_FSM`)
            if dm.name.contains("_Tip_") || dm.name.ends_with("Start_FSM.DO") {
                continue;
            }
            let ro = room_origins.get(&dm.room_id).copied().unwrap_or([0.0; 3]);
            let (mat, pos) = if dm.toggled {
                if let Some(anim) = dm.anims.last() {
                    if dm.world_pos == dm.local_start_pos {
                        (anim.end_mat, anim.end_pos)
                    } else {
                        (
                            dm.world_mat,
                            [
                                dm.world_pos[0] + (anim.end_pos[0] - anim.start_pos[0]),
                                dm.world_pos[1] + (anim.end_pos[1] - anim.start_pos[1]),
                                dm.world_pos[2] + (anim.end_pos[2] - anim.start_pos[2]),
                            ],
                        )
                    }
                } else {
                    (dm.world_mat, dm.world_pos)
                }
            } else {
                (dm.world_mat, dm.world_pos)
            };
            let origin = [ro[0] + pos[0], ro[1] + pos[1], ro[2] + pos[2]];
            render_poly_batch(
                &dm.polys,
                mat,
                origin,
                &scene,
                cam_pos,
                cam_right,
                cam_up,
                cam_fwd,
                fov_scale,
                use_lightmaps,
                wireframe,
                &mut fb,
                &mut zb,
            );
        }

        // Draw crosshair
        let cx = W / 2;
        let cy_scr = H / 2;
        for d in -4i32..=4 {
            fb[cy_scr * W + (cx as i32 + d) as usize] = 0x00ffdd66;
            fb[(cy_scr as i32 + d) as usize * W + cx] = 0x00ffdd66;
        }

        window
            .update_with_buffer(&fb, W, H)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn find_floor_y(
    scene: &LdbScene,
    _room_origins: &BTreeMap<i32, [f32; 3]>,
    wx: f32,
    wy: f32,
    wz: f32,
) -> Option<f32> {
    let mut best: Option<f32> = None;
    let feet_y = wy - EYE_HEIGHT;
    for sm in &scene.static_meshes {
        let ox = sm.world_origin[0];
        let oy = sm.world_origin[1];
        let oz = sm.world_origin[2];
        for poly in &sm.polys {
            if poly.normal[1] < 0.55 || poly.vertices.len() < 3 {
                continue;
            }
            let v0 = [
                poly.vertices[0].pos[0] + ox,
                poly.vertices[0].pos[1] + oy,
                poly.vertices[0].pos[2] + oz,
            ];
            // Quick Y range check (within step-up / drop window around feet)
            if v0[1] > feet_y + 1.2 || v0[1] < feet_y - 6.0 {
                continue;
            }
            for k in 1..poly.vertices.len() - 1 {
                let v1 = [
                    poly.vertices[k].pos[0] + ox,
                    poly.vertices[k].pos[1] + oy,
                    poly.vertices[k].pos[2] + oz,
                ];
                let v2 = [
                    poly.vertices[k + 1].pos[0] + ox,
                    poly.vertices[k + 1].pos[1] + oy,
                    poly.vertices[k + 1].pos[2] + oz,
                ];
                if point_in_xz_tri(wx, wz, v0, v1, v2) {
                    let y = v0[1];
                    if best.is_none_or(|b| y > b) {
                        best = Some(y);
                    }
                }
            }
        }
    }
    best
}

fn hits_wall(
    scene: &LdbScene,
    _room_origins: &BTreeMap<i32, [f32; 3]>,
    wx: f32,
    wy: f32,
    wz: f32,
) -> bool {
    let body_y = wy - EYE_HEIGHT * 0.5;
    let radius = 0.45f32;
    for sm in &scene.static_meshes {
        let ox = sm.world_origin[0];
        let oy = sm.world_origin[1];
        let oz = sm.world_origin[2];
        for poly in &sm.polys {
            if poly.normal[1].abs() > 0.45 || poly.vertices.len() < 3 {
                continue;
            }
            // Skip nodraw collision helper materials only if they are AI-only
            if let Some(mat) = scene.materials.get(&poly.material_id) {
                if mat.category.contains("ai_node") {
                    continue;
                }
            }
            let mut min_x = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            let mut min_y = f32::INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            let mut min_z = f32::INFINITY;
            let mut max_z = f32::NEG_INFINITY;
            for v in &poly.vertices {
                let px = v.pos[0] + ox;
                let py = v.pos[1] + oy;
                let pz = v.pos[2] + oz;
                min_x = min_x.min(px);
                max_x = max_x.max(px);
                min_y = min_y.min(py);
                max_y = max_y.max(py);
                min_z = min_z.min(pz);
                max_z = max_z.max(pz);
            }
            if body_y < min_y || body_y > max_y {
                continue;
            }
            if wx < min_x - radius
                || wx > max_x + radius
                || wz < min_z - radius
                || wz > max_z + radius
            {
                continue;
            }
            let v0 = [
                poly.vertices[0].pos[0] + ox,
                poly.vertices[0].pos[1] + oy,
                poly.vertices[0].pos[2] + oz,
            ];
            let dist = (wx - v0[0]) * poly.normal[0] + (wz - v0[2]) * poly.normal[2];
            if dist.abs() < radius {
                return true;
            }
        }
    }
    false
}

fn point_in_xz_tri(px: f32, pz: f32, a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> bool {
    let d1 = (px - b[0]) * (a[2] - b[2]) - (a[0] - b[0]) * (pz - b[2]);
    let d2 = (px - c[0]) * (b[2] - c[2]) - (b[0] - c[0]) * (pz - c[2]);
    let d3 = (px - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (pz - a[2]);
    let has_neg = (d1 < 0.0) || (d2 < 0.0) || (d3 < 0.0);
    let has_pos = (d1 > 0.0) || (d2 > 0.0) || (d3 > 0.0);
    !(has_neg && has_pos)
}

#[allow(clippy::too_many_arguments)]
fn render_poly_batch(
    polys: &[ScenePoly],
    mat: [f32; 9],
    origin: [f32; 3],
    scene: &LdbScene,
    cam_pos: [f32; 3],
    cam_right: [f32; 3],
    cam_up: [f32; 3],
    cam_fwd: [f32; 3],
    fov_scale: f32,
    use_lightmaps: bool,
    wireframe: bool,
    fb: &mut [u32],
    zb: &mut [f32],
) {
    let half_w = W as f32 * 0.5;
    let half_h = H as f32 * 0.5;

    for poly in polys {
        if let Some(m) = scene.materials.get(&poly.material_id) {
            if m.category.contains("nodraw") {
                continue;
            }
        }
        // World-space polygon normal + backface culling (inward normal dot(v0 - cam, n) <= 0)
        let wn = [
            mat[0] * poly.normal[0] + mat[3] * poly.normal[1] + mat[6] * poly.normal[2],
            mat[1] * poly.normal[0] + mat[4] * poly.normal[1] + mat[7] * poly.normal[2],
            mat[2] * poly.normal[0] + mat[5] * poly.normal[1] + mat[8] * poly.normal[2],
        ];
        if let Some(v0) = poly.vertices.first() {
            let p0 = v0.pos;
            let wx0 = mat[0] * p0[0] + mat[3] * p0[1] + mat[6] * p0[2] + origin[0];
            let wy0 = mat[1] * p0[0] + mat[4] * p0[1] + mat[7] * p0[2] + origin[1];
            let wz0 = mat[2] * p0[0] + mat[5] * p0[1] + mat[8] * p0[2] + origin[2];
            let to_poly = [wx0 - cam_pos[0], wy0 - cam_pos[1], wz0 - cam_pos[2]];
            if !wireframe && (to_poly[0] * wn[0] + to_poly[1] * wn[1] + to_poly[2] * wn[2]) >= 0.05
            {
                continue;
            }
        }

        let tex = scene
            .materials
            .get(&poly.material_id)
            .and_then(|m| m.texture_index)
            .and_then(|ti| scene.textures.get(ti));
        let lmap = if use_lightmaps && poly.lightmap_id >= 0 {
            scene.lightmaps.get(poly.lightmap_id as usize)
        } else {
            None
        };

        // Transform vertices into camera space
        let mut cam_verts = Vec::with_capacity(poly.vertices.len());
        let mut all_behind = true;
        let mut all_far = true;
        for v in &poly.vertices {
            let p = v.pos;
            let wx = mat[0] * p[0] + mat[3] * p[1] + mat[6] * p[2] + origin[0];
            let wy = mat[1] * p[0] + mat[4] * p[1] + mat[7] * p[2] + origin[1];
            let wz = mat[2] * p[0] + mat[5] * p[1] + mat[8] * p[2] + origin[2];
            let dx = wx - cam_pos[0];
            let dy = wy - cam_pos[1];
            let dz = wz - cam_pos[2];
            let cx = dx * cam_right[0] + dy * cam_right[1] + dz * cam_right[2];
            let cy = dx * cam_up[0] + dy * cam_up[1] + dz * cam_up[2];
            let cz = dx * cam_fwd[0] + dy * cam_fwd[1] + dz * cam_fwd[2];
            if cz >= NEAR_Z {
                all_behind = false;
            }
            if cz <= FAR_Z {
                all_far = false;
            }
            cam_verts.push(CamVert {
                cx,
                cy,
                cz,
                u: v.uv[0],
                v: v.uv[1],
                lu: v.luv[0],
                lv: v.luv[1],
            });
        }
        if all_behind || all_far {
            continue;
        }

        // Clip polygon against near plane `cz >= NEAR_Z`
        let clipped = clip_near_plane(&cam_verts);
        if clipped.len() < 3 {
            continue;
        }

        // Project to screen
        let mut proj = Vec::with_capacity(clipped.len());
        for cv in &clipped {
            let inv_z = 1.0 / cv.cz;
            let sx = half_w + cv.cx * fov_scale * inv_z;
            let sy = half_h - cv.cy * fov_scale * inv_z;
            proj.push(ProjVert {
                sx,
                sy,
                inv_z,
                u_over_z: cv.u * inv_z,
                v_over_z: cv.v * inv_z,
                lu_over_z: cv.lu * inv_z,
                lv_over_z: cv.lv * inv_z,
            });
        }

        // Directional shade fallback based on polygon normal
        let shade = ((poly.normal[0] * 0.35 + poly.normal[1] * 0.75 + poly.normal[2] * 0.55).abs()
            * 0.55
            + 0.45)
            .clamp(0.35, 1.0);

        for k in 1..proj.len() - 1 {
            let p0 = proj[0];
            let p1 = proj[k];
            let p2 = proj[k + 1];
            if wireframe {
                draw_line(fb, p0.sx, p0.sy, p1.sx, p1.sy, 0x0066ccff);
                draw_line(fb, p1.sx, p1.sy, p2.sx, p2.sy, 0x0066ccff);
                draw_line(fb, p2.sx, p2.sy, p0.sx, p0.sy, 0x0066ccff);
            } else {
                rasterize_tri(fb, zb, p0, p1, p2, tex, lmap, shade);
            }
        }
    }
}

fn clip_near_plane(verts: &[CamVert]) -> Vec<CamVert> {
    let mut out = Vec::with_capacity(verts.len() + 2);
    for i in 0..verts.len() {
        let a = verts[i];
        let b = verts[(i + 1) % verts.len()];
        let a_in = a.cz >= NEAR_Z;
        let b_in = b.cz >= NEAR_Z;
        if a_in {
            out.push(a);
        }
        if a_in != b_in {
            let t = (NEAR_Z - a.cz) / (b.cz - a.cz);
            out.push(CamVert {
                cx: a.cx + (b.cx - a.cx) * t,
                cy: a.cy + (b.cy - a.cy) * t,
                cz: NEAR_Z,
                u: a.u + (b.u - a.u) * t,
                v: a.v + (b.v - a.v) * t,
                lu: a.lu + (b.lu - a.lu) * t,
                lv: a.lv + (b.lv - a.lv) * t,
            });
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn rasterize_tri(
    fb: &mut [u32],
    zb: &mut [f32],
    p0: ProjVert,
    p1: ProjVert,
    p2: ProjVert,
    tex: Option<&DecodedTexture>,
    lmap: Option<&DecodedLightmap>,
    shade: f32,
) {
    let area = (p1.sx - p0.sx) * (p2.sy - p0.sy) - (p2.sx - p0.sx) * (p1.sy - p0.sy);
    if area.abs() < 1e-3 {
        return;
    }
    let inv_area = 1.0 / area;

    let min_x = p0.sx.min(p1.sx).min(p2.sx).floor().max(0.0) as usize;
    let max_x = p0.sx.max(p1.sx).max(p2.sx).ceil().min((W - 1) as f32) as usize;
    let min_y = p0.sy.min(p1.sy).min(p2.sy).floor().max(0.0) as usize;
    let max_y = p0.sy.max(p1.sy).max(p2.sy).ceil().min((H - 1) as f32) as usize;
    if min_x > max_x || min_y > max_y {
        return;
    }

    for y in min_y..=max_y {
        let py = y as f32 + 0.5;
        let row = y * W;
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let mut w0 = ((p1.sx - px) * (p2.sy - py) - (p2.sx - px) * (p1.sy - py)) * inv_area;
            let mut w1 = ((p2.sx - px) * (p0.sy - py) - (p0.sx - px) * (p2.sy - py)) * inv_area;
            let mut w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let inv_z = w0 * p0.inv_z + w1 * p1.inv_z + w2 * p2.inv_z;
            if inv_z <= 0.0 {
                continue;
            }
            let z = 1.0 / inv_z;
            let idx = row + x;
            if z >= zb[idx] {
                continue;
            }
            zb[idx] = z;

            let _ = (&mut w0, &mut w1, &mut w2);
            let u = (w0 * p0.u_over_z + w1 * p1.u_over_z + w2 * p2.u_over_z) * z;
            let v = (w0 * p0.v_over_z + w1 * p1.v_over_z + w2 * p2.v_over_z) * z;

            let base_rgb = if let Some(t) = tex {
                let tx = (u.rem_euclid(1.0) * t.width as f32) as u32 % t.width.max(1);
                let ty = (v.rem_euclid(1.0) * t.height as f32) as u32 % t.height.max(1);
                t.pixels
                    .get((ty * t.width + tx) as usize)
                    .copied()
                    .unwrap_or(0x00999999)
            } else {
                0x008899aa
            };

            let mut r = ((base_rgb >> 16) & 0xff) as f32;
            let mut g = ((base_rgb >> 8) & 0xff) as f32;
            let mut b = (base_rgb & 0xff) as f32;

            if let Some(lm) = lmap {
                let lu = (w0 * p0.lu_over_z + w1 * p1.lu_over_z + w2 * p2.lu_over_z) * z;
                let lv = (w0 * p0.lv_over_z + w1 * p1.lv_over_z + w2 * p2.lv_over_z) * z;
                let lx = (lu.clamp(0.0, 1.0) * (lm.width - 1) as f32) as u32;
                let ly = (lv.clamp(0.0, 1.0) * (lm.height - 1) as f32) as u32;
                if let Some(&lcol) = lm.pixels.get((ly * lm.width + lx) as usize) {
                    let lr = (((lcol >> 16) & 0xff) as f32 / 255.0 * 1.65).clamp(0.22, 1.4);
                    let lg = (((lcol >> 8) & 0xff) as f32 / 255.0 * 1.65).clamp(0.22, 1.4);
                    let lb = ((lcol & 0xff) as f32 / 255.0 * 1.65).clamp(0.22, 1.4);
                    r *= lr;
                    g *= lg;
                    b *= lb;
                }
            } else {
                r *= shade;
                g *= shade;
                b *= shade;
            }

            let ri = (r.min(255.0) as u32) << 16;
            let gi = (g.min(255.0) as u32) << 8;
            let bi = b.min(255.0) as u32;
            fb[idx] = ri | gi | bi;
        }
    }
}

fn draw_line(fb: &mut [u32], x0: f32, y0: f32, x1: f32, y1: f32, col: u32) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil() as i32;
    if steps <= 0 || steps > 4000 {
        return;
    }
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let x = (x0 + (x1 - x0) * t) as i32;
        let y = (y0 + (y1 - y0) * t) as i32;
        if x >= 0 && x < W as i32 && y >= 0 && y < H as i32 {
            fb[y as usize * W + x as usize] = col;
        }
    }
}
