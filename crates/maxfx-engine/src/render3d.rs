//! Interactive 3D third-person / first-person Max Payne 1 engine walkthrough (`maxfx-engine --window`).
//!
//! Features:
//! - Third-person over-the-shoulder camera showing **Max Payne (`Payne_Max_L0.KFS`)** with walking
//!   leg-swing animation + gun-aiming posture (`V` toggles 3rd-person / 1st-person)
//! - Seamless room portal connectivity (`origin_v = origin_u - ex.relative_offset`)
//! - Sloped floor plane interpolation + strict floor/wall boundary clamping (cannot fall out of world)
//! - Visible 3D enemies (`doe_johnny_l0.kfs`) spawned via level `C_Teleport` FSM scripts + AI pursuit
//! - Shooting (`F` / `Ctrl` / Right-Ctrl), Bullet-Time slow-motion (`B` / Right-Click), and `E` door/barrel interaction

use crate::kf2::{load_character_meshes, KfsMesh};
use crate::scene::{DecodedLightmap, DecodedTexture, LdbScene, ScenePoly, SceneVertex};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use std::collections::BTreeMap;
use std::path::Path;

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

#[derive(Clone)]
struct EnemyState {
    name: String,
    room_id: i32,
    world_pos: [f32; 3],
    yaw: f32,
    walk_phase: f32,
    health: i32,
    hit_flash: f32,
}

pub fn run_3d_walkthrough(
    mut scene: LdbScene,
    level_path: &Path,
    title: &str,
) -> Result<(), String> {
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

    let (player_mesh, enemy_mesh) = load_character_meshes(level_path);

    let mut room_origins: BTreeMap<i32, [f32; 3]> = BTreeMap::new();
    for r in &scene.rooms {
        room_origins.insert(r.id, r.world_origin);
    }

    // Build active enemies from `scene.characters` (already C_Teleport-resolved)
    let mut enemies: Vec<EnemyState> = Vec::new();
    for ch in &scene.characters {
        let ro = room_origins.get(&ch.room_id).copied().unwrap_or([0.0; 3]);
        let mut wy = ro[1] + ch.pos[1];
        if let Some(fy) = find_floor_y(
            &scene,
            ro[0] + ch.pos[0],
            wy + EYE_HEIGHT,
            ro[2] + ch.pos[2],
        ) {
            wy = fy;
        }
        enemies.push(EnemyState {
            name: ch.name.clone(),
            room_id: ch.room_id,
            world_pos: [ro[0] + ch.pos[0], wy, ro[2] + ch.pos[2]],
            yaw: 0.0,
            walk_phase: 0.0,
            health: 2,
            hit_flash: 0.0,
        });
    }
    // Also place an active mobster guard in ::Alley2 (room 2) if present so the player encounters
    // an enemy right outside the StartRoom door.
    if let Some(&alley2_ro) = room_origins.get(&2) {
        if !enemies.iter().any(|e| e.room_id == 2) {
            let gx = alley2_ro[0] - 8.0;
            let gz = alley2_ro[2] + 1.0;
            let gy =
                find_floor_y(&scene, gx, alley2_ro[1] - 5.0, gz).unwrap_or(alley2_ro[1] - 10.0);
            enemies.push(EnemyState {
                name: "::Alley2::Guard_01".into(),
                room_id: 2,
                world_pos: [gx, gy, gz],
                yaw: 0.0,
                walk_phase: 0.0,
                health: 2,
                hit_flash: 0.0,
            });
        }
    }

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
                let wx = ro[0] + jp.pos[0];
                let wz = ro[2] + jp.pos[2];
                let wy = find_floor_y(scene, wx, ro[1] + jp.pos[1] + EYE_HEIGHT, wz)
                    .unwrap_or(ro[1] + jp.pos[1]);
                [wx, wy + EYE_HEIGHT, wz]
            } else {
                [0.0, EYE_HEIGHT, 0.0]
            }
        };

    let mut player_pos = spawn_pos(&scene, &room_origins, jp_idx);
    let mut yaw: f32 = std::f32::consts::PI; // face -Z toward van & door in StartRoom
    let mut pitch: f32 = -0.08;
    let mut vel_y: f32 = 0.0;
    let mut walk_phase: f32 = 0.0;
    let mut third_person = true;
    let mut bullet_time = false;
    let mut shot_timer: f32 = 0.0;
    let mut fly_mode = false;
    let mut use_lightmaps = true;
    let mut wireframe = false;
    let mut last_mouse: Option<(f32, f32)> = None;
    println!(
        "  3D Engine: Spawned ({} enemies loaded, player mesh={}) | WASD move, Mouse/Arrows look, F/Ctrl shoot, B/RightClick Bullet-Time, E interact, V 3rd/1st person, Tab next spawn, G fly/walk",
        enemies.len(),
        player_mesh.is_some()
    );

    let mut fb = vec![0u32; W * H];
    let mut zb = vec![f32::INFINITY; W * H];
    let base_dt = 1.0 / 60.0f32;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        if window.is_key_pressed(Key::V, KeyRepeat::No) {
            third_person = !third_person;
        }
        if window.is_key_pressed(Key::B, KeyRepeat::No) {
            bullet_time = !bullet_time;
        }
        let bullet_time_active = bullet_time || window.get_mouse_down(MouseButton::Right);
        let world_dt = if bullet_time_active {
            base_dt * 0.30
        } else {
            base_dt
        };
        let player_dt = if bullet_time_active {
            base_dt * 0.65
        } else {
            base_dt
        };
        shot_timer = (shot_timer - base_dt).max(0.0);

        if window.is_key_pressed(Key::Tab, KeyRepeat::No) && !scene.jumppoints.is_empty() {
            jp_idx = (jp_idx + 1) % scene.jumppoints.len();
            player_pos = spawn_pos(&scene, &room_origins, jp_idx);
            vel_y = 0.0;
            if let Some(jp) = scene.jumppoints.get(jp_idx) {
                println!(
                    "  3D Engine: Teleported to {} (room {})",
                    jp.name, jp.room_id
                );
            }
        }
        if window.is_key_pressed(Key::G, KeyRepeat::No) {
            fly_mode = !fly_mode;
            vel_y = 0.0;
        }
        if window.is_key_pressed(Key::L, KeyRepeat::No) {
            use_lightmaps = !use_lightmaps;
        }
        if window.is_key_pressed(Key::F1, KeyRepeat::No) {
            wireframe = !wireframe;
        }

        // Interact (`E`): trigger nearest dynamic mesh / door within 6m
        if window.is_key_pressed(Key::E, KeyRepeat::No) {
            toggle_nearest_dynamic_mesh(&mut scene, &room_origins, player_pos, 36.0);
        }

        // Look controls (Arrows + left-mouse drag)
        let turn_speed = 2.2 * base_dt;
        if window.is_key_down(Key::Left) {
            yaw -= turn_speed;
        }
        if window.is_key_down(Key::Right) {
            yaw += turn_speed;
        }
        if window.is_key_down(Key::Up) {
            pitch = (pitch + turn_speed).min(1.25);
        }
        if window.is_key_down(Key::Down) {
            pitch = (pitch - turn_speed).max(-1.25);
        }
        if window.get_mouse_down(MouseButton::Left) {
            if let Some((mx, my)) = window.get_mouse_pos(MouseMode::Pass) {
                if let Some((lx, ly)) = last_mouse {
                    yaw += (mx - lx) * 0.006;
                    pitch = (pitch - (my - ly) * 0.006).clamp(-1.25, 1.25);
                }
                last_mouse = Some((mx, my));
            }
        } else {
            last_mouse = None;
        }

        // Movement controls (WASD)
        let speed = if window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift) {
            11.0
        } else {
            5.8
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
        let mv_len = (mv[0] * mv[0] + mv[2] * mv[2]).sqrt();
        if mv_len > 0.01 {
            walk_phase += player_dt * speed * 1.9;
        } else {
            walk_phase = 0.0;
        }

        if fly_mode {
            if window.is_key_down(Key::Space) {
                mv[1] += 1.0;
            }
            if window.is_key_down(Key::C) {
                mv[1] -= 1.0;
            }
            player_pos[0] += mv[0] * speed * player_dt;
            player_pos[1] += mv[1] * speed * player_dt;
            player_pos[2] += mv[2] * speed * player_dt;
        } else {
            let dx = mv[0] * speed * player_dt;
            let dz = mv[2] * speed * player_dt;
            let nx = player_pos[0] + dx;
            let nz = player_pos[2] + dz;
            // Strict boundary & wall check: never allow stepping off walkable floor into void!
            if can_stand_at(&scene, nx, player_pos[1], nz) {
                player_pos[0] = nx;
                player_pos[2] = nz;
            } else if can_stand_at(&scene, nx, player_pos[1], player_pos[2]) {
                player_pos[0] = nx;
            } else if can_stand_at(&scene, player_pos[0], player_pos[1], nz) {
                player_pos[2] = nz;
            }

            if window.is_key_pressed(Key::Space, KeyRepeat::No) && vel_y.abs() < 0.05 {
                vel_y = 5.2;
            }
            vel_y -= 15.0 * player_dt;
            player_pos[1] += vel_y * player_dt;
            if let Some(floor_y) = find_floor_y(&scene, player_pos[0], player_pos[1], player_pos[2])
            {
                if player_pos[1] <= floor_y + EYE_HEIGHT {
                    player_pos[1] = floor_y + EYE_HEIGHT;
                    vel_y = 0.0;
                }
            } else {
                // Clamp vertical fall if jumping over a gap
                vel_y = 0.0;
            }
        }

        // Shoot (`F` or `LeftCtrl` / `RightCtrl`)
        let fire_pressed = window.is_key_pressed(Key::F, KeyRepeat::No)
            || window.is_key_pressed(Key::LeftCtrl, KeyRepeat::No)
            || window.is_key_pressed(Key::RightCtrl, KeyRepeat::No);
        if fire_pressed && shot_timer <= 0.0 {
            shot_timer = 0.18;
            let aim_dir = [sy * cp, sp, cy * cp];
            // Raycast against living enemies
            let mut best_hit: Option<(usize, f32)> = None;
            for (idx, en) in enemies.iter().enumerate() {
                if en.health <= 0 {
                    continue;
                }
                let center = [en.world_pos[0], en.world_pos[1] + 1.0, en.world_pos[2]];
                let to_en = [
                    center[0] - player_pos[0],
                    center[1] - player_pos[1],
                    center[2] - player_pos[2],
                ];
                let t = to_en[0] * aim_dir[0] + to_en[1] * aim_dir[1] + to_en[2] * aim_dir[2];
                if (0.5..=65.0).contains(&t) {
                    let cx = player_pos[0] + aim_dir[0] * t - center[0];
                    let cy_d = (player_pos[1] + aim_dir[1] * t - center[1]) * 0.55;
                    let cz = player_pos[2] + aim_dir[2] * t - center[2];
                    let dist_sq = cx * cx + cy_d * cy_d + cz * cz;
                    if dist_sq < 0.65 && best_hit.is_none_or(|(_, b)| t < b) {
                        best_hit = Some((idx, t));
                    }
                }
            }
            if let Some((idx, _)) = best_hit {
                let en = &mut enemies[idx];
                en.health -= 1;
                en.hit_flash = 0.25;
                println!(
                    "  3D Engine: Hit {}! (health={})",
                    en.name,
                    en.health.max(0)
                );
            } else {
                // Also allow shooting nearby breakable barrels / doors along aim ray
                let probe = [
                    player_pos[0] + aim_dir[0] * 4.0,
                    player_pos[1] + aim_dir[1] * 4.0,
                    player_pos[2] + aim_dir[2] * 4.0,
                ];
                toggle_nearest_dynamic_mesh(&mut scene, &room_origins, probe, 16.0);
            }
        }

        // Update Enemy AI (turn toward Max Payne and advance along floor when within 24m)
        let player_feet = [player_pos[0], player_pos[1] - EYE_HEIGHT, player_pos[2]];
        for en in &mut enemies {
            en.hit_flash = (en.hit_flash - base_dt).max(0.0);
            if en.health <= 0 {
                continue;
            }
            let dx = player_feet[0] - en.world_pos[0];
            let dy = player_feet[1] - en.world_pos[1];
            let dz = player_feet[2] - en.world_pos[2];
            let dist = (dx * dx + dz * dz).sqrt();
            if dist < 24.0 && dy.abs() < 4.5 {
                en.yaw = dx.atan2(dz);
                if dist > 3.2 {
                    let step = 2.6 * world_dt;
                    let nx = en.world_pos[0] + (dx / dist) * step;
                    let nz = en.world_pos[2] + (dz / dist) * step;
                    if can_stand_at(&scene, nx, en.world_pos[1] + EYE_HEIGHT, nz) {
                        en.world_pos[0] = nx;
                        en.world_pos[2] = nz;
                        if let Some(fy) = find_floor_y(&scene, nx, en.world_pos[1] + EYE_HEIGHT, nz)
                        {
                            en.world_pos[1] = fy;
                        }
                        en.walk_phase += world_dt * 5.0;
                    }
                } else {
                    en.walk_phase = 0.0;
                }
            }
        }

        // Compute camera position (3rd-person over-the-shoulder vs 1st-person)
        let cam_right = [cy, 0.0, -sy];
        let cam_up = [-sy * sp, cp, -cy * sp];
        let cam_fwd = [sy * cp, sp, cy * cp];
        let cam_pos = if third_person && !fly_mode {
            let mut dist = 2.05f32;
            while dist > 0.45 {
                let cand_x = player_pos[0] - cam_fwd[0] * dist + cam_right[0] * 0.28;
                let cand_y = player_pos[1] - cam_fwd[1] * dist + 0.18;
                let cand_z = player_pos[2] - cam_fwd[2] * dist + cam_right[2] * 0.28;
                if !hits_wall(&scene, cand_x, cand_y, cand_z) {
                    break;
                }
                dist -= 0.25;
            }
            [
                player_pos[0] - cam_fwd[0] * dist + cam_right[0] * 0.28,
                player_pos[1] - cam_fwd[1] * dist + 0.18,
                player_pos[2] - cam_fwd[2] * dist + cam_right[2] * 0.28,
            ]
        } else {
            player_pos
        };

        // Clear sky/ceiling + depth buffer
        for y in 0..H {
            let row_col = if y < H / 2 { 0x00161c28 } else { 0x001e1e22 };
            fb[y * W..(y + 1) * W].fill(row_col);
        }
        zb.fill(f32::INFINITY);

        let fov_scale = (H as f32) * 0.85;
        const IDENT_MAT: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

        // 1. Render static room meshes (with inward-normal backface culling)
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
                true,
                &mut fb,
                &mut zb,
            );
        }

        // 2. Render dynamic meshes (doors, breakables, furniture, vans, cameras)
        for dm in &scene.dynamic_meshes {
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
                false,
                &mut fb,
                &mut zb,
            );
        }

        // 3. Render Enemies (`doe_johnny_l0.kfs`)
        if let Some(ref emesh) = enemy_mesh {
            for en in &enemies {
                render_kfs_character(
                    emesh,
                    en.world_pos,
                    en.yaw,
                    en.walk_phase,
                    en.health <= 0,
                    en.hit_flash > 0.0,
                    cam_pos,
                    cam_right,
                    cam_up,
                    cam_fwd,
                    fov_scale,
                    wireframe,
                    &mut fb,
                    &mut zb,
                );
            }
        }

        // 4. Render Player (`Payne_Max_L0.KFS`) in 3rd-person view
        if third_person && !fly_mode {
            if let Some(ref pmesh) = player_mesh {
                render_kfs_character(
                    pmesh,
                    player_feet,
                    yaw + std::f32::consts::PI,
                    walk_phase,
                    false,
                    false,
                    cam_pos,
                    cam_right,
                    cam_up,
                    cam_fwd,
                    fov_scale,
                    wireframe,
                    &mut fb,
                    &mut zb,
                );
            }
        }

        // 5. Render muzzle flash / bullet tracer when firing
        if shot_timer > 0.10 {
            draw_line(
                &mut fb,
                (W as f32) * 0.56,
                (H as f32) * 0.62,
                (W as f32) * 0.50,
                (H as f32) * 0.50,
                0x00ffee77,
            );
        }

        // Draw crosshair (gold in Bullet-Time, white/amber normally)
        let cross_col = if bullet_time_active {
            0x00ffaa22
        } else {
            0x00ffdd66
        };
        let cx = W / 2;
        let cy_scr = H / 2;
        for d in -4i32..=4 {
            fb[cy_scr * W + (cx as i32 + d) as usize] = cross_col;
            fb[(cy_scr as i32 + d) as usize * W + cx] = cross_col;
        }

        window
            .update_with_buffer(&fb, W, H)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn toggle_nearest_dynamic_mesh(
    scene: &mut LdbScene,
    room_origins: &BTreeMap<i32, [f32; 3]>,
    pos: [f32; 3],
    max_d2: f32,
) {
    let mut best_dm: Option<(usize, f32)> = None;
    for (idx, dm) in scene.dynamic_meshes.iter().enumerate() {
        if dm.anims.is_empty() {
            continue;
        }
        let ro = room_origins.get(&dm.room_id).copied().unwrap_or([0.0; 3]);
        let wx = ro[0] + dm.world_pos[0];
        let wy = ro[1] + dm.world_pos[1];
        let wz = ro[2] + dm.world_pos[2];
        let d2 = (wx - pos[0]).powi(2) + (wy - pos[1]).powi(2) + (wz - pos[2]).powi(2);
        if d2 < max_d2 && best_dm.is_none_or(|(_, b)| d2 < b) {
            best_dm = Some((idx, d2));
        }
    }
    if let Some((idx, _)) = best_dm {
        let dm = &mut scene.dynamic_meshes[idx];
        dm.toggled = !dm.toggled;
        if let Some(anim) = dm.anims.last() {
            println!(
                "  3D Engine: DO_Animate {} ({}) -> {:?}",
                dm.name,
                anim.name,
                anim.scripts.first().map(|s| s.as_str()).unwrap_or("ok")
            );
        }
    }
}

fn can_stand_at(scene: &LdbScene, wx: f32, wy: f32, wz: f32) -> bool {
    !hits_wall(scene, wx, wy, wz) && find_floor_y(scene, wx, wy, wz).is_some()
}

fn find_floor_y(scene: &LdbScene, wx: f32, wy: f32, wz: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    let feet_y = wy - EYE_HEIGHT;
    for sm in &scene.static_meshes {
        let ox = sm.world_origin[0];
        let oy = sm.world_origin[1];
        let oz = sm.world_origin[2];
        for poly in &sm.polys {
            if poly.normal[1] < 0.45 || poly.vertices.len() < 3 {
                continue;
            }
            let v0 = [
                poly.vertices[0].pos[0] + ox,
                poly.vertices[0].pos[1] + oy,
                poly.vertices[0].pos[2] + oz,
            ];
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
                    // Plane interpolation for sloped floors / stairs / ramps
                    let y = v0[1]
                        - (poly.normal[0] * (wx - v0[0]) + poly.normal[2] * (wz - v0[2]))
                            / poly.normal[1];
                    if y <= feet_y + 1.15 && y >= feet_y - 6.0 && best.is_none_or(|b| y > b) {
                        best = Some(y);
                    }
                }
            }
        }
    }
    best
}

fn hits_wall(scene: &LdbScene, wx: f32, wy: f32, wz: f32) -> bool {
    let body_min_y = wy - EYE_HEIGHT + 0.35;
    let body_max_y = wy - 0.10;
    let radius = 0.38f32;
    for sm in &scene.static_meshes {
        let ox = sm.world_origin[0];
        let oy = sm.world_origin[1];
        let oz = sm.world_origin[2];
        for poly in &sm.polys {
            if poly.normal[1].abs() > 0.45 || poly.vertices.len() < 3 {
                continue;
            }
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
            if body_max_y < min_y || body_min_y > max_y {
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
            let plane_dist = (wx - v0[0]) * poly.normal[0] + (wz - v0[2]) * poly.normal[2];
            if plane_dist.abs() < radius {
                return true;
            }
        }
    }
    false
}

fn point_in_xz_tri(px: f32, pz: f32, a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> bool {
    let eps = -0.02f32;
    let d1 = (px - b[0]) * (a[2] - b[2]) - (a[0] - b[0]) * (pz - b[2]);
    let d2 = (px - c[0]) * (b[2] - c[2]) - (b[0] - c[0]) * (pz - c[2]);
    let d3 = (px - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (pz - a[2]);
    let has_neg = (d1 < -eps) || (d2 < -eps) || (d3 < -eps);
    let has_pos = (d1 > eps) || (d2 > eps) || (d3 > eps);
    !(has_neg && has_pos)
}

#[allow(clippy::too_many_arguments)]
fn render_kfs_character(
    mesh: &KfsMesh,
    world_pos: [f32; 3],
    yaw: f32,
    walk_phase: f32,
    dead: bool,
    hit_flash: bool,
    cam_pos: [f32; 3],
    cam_right: [f32; 3],
    cam_up: [f32; 3],
    cam_fwd: [f32; 3],
    fov_scale: f32,
    wireframe: bool,
    fb: &mut [u32],
    zb: &mut [f32],
) {
    let half_w = W as f32 * 0.5;
    let half_h = H as f32 * 0.5;
    let (sy, cy) = yaw.sin_cos();
    let leg_swing = walk_phase.sin() * 0.26;

    // Pre-transform character vertices into camera space
    let mut cam_pts = Vec::with_capacity(mesh.vertices.len());
    for &[lx, ly, lz] in &mesh.vertices {
        let (px, py, pz) = if dead {
            // Lie flat on the floor when dead
            (lx, 0.18 + lz.abs() * 0.5, ly - 0.85)
        } else if ly < 0.90 {
            // Procedural leg swing during walk
            let swing = if lx < 0.0 { leg_swing } else { -leg_swing };
            (lx, ly, lz + swing * (0.90 - ly))
        } else {
            (lx, ly, lz)
        };
        let wx = world_pos[0] + px * cy + pz * sy;
        let wy = world_pos[1] + py;
        let wz = world_pos[2] - px * sy + pz * cy;
        let dx = wx - cam_pos[0];
        let dy = wy - cam_pos[1];
        let dz = wz - cam_pos[2];
        let cx = dx * cam_right[0] + dy * cam_right[1] + dz * cam_right[2];
        let cy_c = dx * cam_up[0] + dy * cam_up[1] + dz * cam_up[2];
        let cz = dx * cam_fwd[0] + dy * cam_fwd[1] + dz * cam_fwd[2];
        cam_pts.push([cx, cy_c, cz]);
    }

    let shade = if hit_flash { 1.35 } else { 0.92 };
    for face in &mesh.faces {
        let [i0, i1, i2] = face.indices;
        let c0 = cam_pts[i0];
        let c1 = cam_pts[i1];
        let c2 = cam_pts[i2];
        if c0[2] < NEAR_Z && c1[2] < NEAR_Z && c2[2] < NEAR_Z {
            continue;
        }
        if c0[2] > FAR_Z && c1[2] > FAR_Z && c2[2] > FAR_Z {
            continue;
        }
        let cam_verts = [
            CamVert {
                cx: c0[0],
                cy: c0[1],
                cz: c0[2],
                u: face.uvs[0][0],
                v: face.uvs[0][1],
                lu: 0.5,
                lv: 0.5,
            },
            CamVert {
                cx: c1[0],
                cy: c1[1],
                cz: c1[2],
                u: face.uvs[1][0],
                v: face.uvs[1][1],
                lu: 0.5,
                lv: 0.5,
            },
            CamVert {
                cx: c2[0],
                cy: c2[1],
                cz: c2[2],
                u: face.uvs[2][0],
                v: face.uvs[2][1],
                lu: 0.5,
                lv: 0.5,
            },
        ];
        let clipped = clip_near_plane(&cam_verts);
        if clipped.len() < 3 {
            continue;
        }
        let mut proj = Vec::with_capacity(clipped.len());
        for cv in &clipped {
            let inv_z = 1.0 / cv.cz;
            proj.push(ProjVert {
                sx: half_w + cv.cx * fov_scale * inv_z,
                sy: half_h - cv.cy * fov_scale * inv_z,
                inv_z,
                u_over_z: cv.u * inv_z,
                v_over_z: cv.v * inv_z,
                lu_over_z: 0.0,
                lv_over_z: 0.0,
            });
        }
        let tex = mesh
            .material_textures
            .get(face.mat_index)
            .and_then(|t| t.as_ref());
        for k in 1..proj.len() - 1 {
            if wireframe {
                draw_line(
                    fb, proj[0].sx, proj[0].sy, proj[k].sx, proj[k].sy, 0x00ff8844,
                );
            } else {
                rasterize_tri(fb, zb, proj[0], proj[k], proj[k + 1], tex, None, shade);
            }
        }
    }
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
    cull_backfaces: bool,
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
        let wn = [
            mat[0] * poly.normal[0] + mat[3] * poly.normal[1] + mat[6] * poly.normal[2],
            mat[1] * poly.normal[0] + mat[4] * poly.normal[1] + mat[7] * poly.normal[2],
            mat[2] * poly.normal[0] + mat[5] * poly.normal[1] + mat[8] * poly.normal[2],
        ];
        if cull_backfaces && !wireframe {
            if let Some(SceneVertex { pos: p0, .. }) = poly.vertices.first() {
                let wx0 = mat[0] * p0[0] + mat[3] * p0[1] + mat[6] * p0[2] + origin[0];
                let wy0 = mat[1] * p0[0] + mat[4] * p0[1] + mat[7] * p0[2] + origin[1];
                let wz0 = mat[2] * p0[0] + mat[5] * p0[1] + mat[8] * p0[2] + origin[2];
                let to_poly = [wx0 - cam_pos[0], wy0 - cam_pos[1], wz0 - cam_pos[2]];
                if (to_poly[0] * wn[0] + to_poly[1] * wn[1] + to_poly[2] * wn[2]) >= 0.05 {
                    continue;
                }
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

        let clipped = clip_near_plane(&cam_verts);
        if clipped.len() < 3 {
            continue;
        }

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

        let shade =
            ((wn[0] * 0.35 + wn[1] * 0.75 + wn[2] * 0.55).abs() * 0.55 + 0.45).clamp(0.35, 1.0);

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
            let w0 = ((p1.sx - px) * (p2.sy - py) - (p2.sx - px) * (p1.sy - py)) * inv_area;
            let w1 = ((p2.sx - px) * (p0.sy - py) - (p0.sx - px) * (p2.sy - py)) * inv_area;
            let w2 = 1.0 - w0 - w1;
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
