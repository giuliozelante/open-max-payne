//! Interactive 3D third-person / first-person Max Payne 1 engine walkthrough (`maxfx-engine --window`).
//!
//! Features:
//! - Third-person over-the-shoulder camera showing **Max Payne (`Payne_Max_L0.KFS`)** skinned
//!   with `PAYNE_MAX_L0.SKD` + `Stand.kf2` / `Run.kf2` (`V` toggles 3rd / 1st person)
//! - Seamless room portal connectivity (`origin_v = origin_u - ex.relative_offset`)
//! - Sloped floor plane interpolation + strict floor/wall boundary clamping (cannot fall out of world)
//! - Visible 3D enemies (`doe_johnny_l0.kfs`) spawned via level `C_Teleport` FSM scripts + AI pursuit
//! - Shooting (`F` / `Ctrl` / Right-Ctrl), Bullet-Time slow-motion (`B` / Right-Click), and `E` door/barrel interaction

use crate::kf2::{load_character_rigs, load_skin_body, pose_mesh, CharacterRig, KfsFace};
use crate::scene::{
    decode_texture_blob, material_policy, CameraFlyover, DecodedLightmap, DecodedTexture, LdbScene,
    MaterialPolicy, ScenePointLight, ScenePoly, SceneVertex,
};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Child;

const W: usize = 960;
const H: usize = 600;
const NEAR_Z: f32 = 0.15;
const FAR_Z: f32 = 220.0;
const EYE_HEIGHT: f32 = 1.65;
/// One `DetailOffset` step from `materials.txt`, in metres toward the camera.
const DETAIL_OFFSET_M: f32 = 0.02;
/// `data/database/shooting.h` `#define BERETTA_BULLETDAMAGE 5`.
const BERETTA_BULLET_DAMAGE: i32 = 5;

#[derive(Clone, Copy)]
struct ProjVert {
    sx: f32,
    sy: f32,
    inv_z: f32,
    u_over_z: f32,
    v_over_z: f32,
    lu_over_z: f32,
    lv_over_z: f32,
    /// Vertex diffuse (0..1 per channel), perspective-correct like the UVs.
    light_over_z: [f32; 3],
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
    light: [f32; 3],
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
    rig_index: usize,
    pursues: bool,
    invulnerable: bool,
}

pub fn run_3d_walkthrough(
    mut scene: LdbScene,
    level_path: &Path,
    title: &str,
    player_start: Option<&str>,
    player_skin: Option<&str>,
    world_sphere: Option<&str>,
    shot: Option<CameraFlyover>,
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

    let (catalog_player, johnny_rig) = load_character_rigs(level_path);
    let player_rig = player_skin
        .filter(|id| !id.is_empty())
        .and_then(|id| load_skin_body(level_path, id).map(|body| body.rig))
        .or(catalog_player);
    let sky = world_sphere.and_then(|name| load_world_sphere(level_path, name));

    let mut room_origins: BTreeMap<i32, [f32; 3]> = BTreeMap::new();
    for r in &scene.rooms {
        room_origins.insert(r.id, r.world_origin);
    }
    // `.ldb` point light positions are room-local (every light sits within a few metres of every room).
    let world_lights: Vec<ScenePointLight> = scene
        .point_lights
        .iter()
        .map(|l| {
            let ro = room_origins.get(&l.room_id).copied().unwrap_or([0.0; 3]);
            ScenePointLight {
                pos: [0, 1, 2].map(|k| ro[k] + l.pos[k]),
                ..l.clone()
            }
        })
        .collect();

    let mut bodies: Vec<CharacterRig> = Vec::new();
    if let Some(johnny) = johnny_rig {
        bodies.push(johnny);
    }
    let johnny_index = if bodies.is_empty() { None } else { Some(0) };
    let mut skin_cache: BTreeMap<String, (usize, bool)> = BTreeMap::new();

    // Build active enemies from `scene.characters` (already C_Teleport-resolved)
    let mut enemies: Vec<EnemyState> = Vec::new();
    for ch in &scene.characters {
        let key = ch.skin.to_ascii_lowercase();
        let (rig_index, invulnerable) = if let Some(hit) = skin_cache.get(&key).copied() {
            hit
        } else if let Some(body) = load_skin_body(level_path, &ch.skin) {
            let idx = bodies.len();
            let inv = body.invulnerable;
            bodies.push(body.rig);
            skin_cache.insert(key, (idx, inv));
            (idx, inv)
        } else if let Some(idx) = johnny_index {
            skin_cache.insert(key, (idx, false));
            (idx, false)
        } else {
            continue;
        };
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
            health: if ch.starts_dead() { 0 } else { 2 },
            hit_flash: 0.0,
            rig_index,
            pursues: ch.pursues(),
            invulnerable,
        });
    }
    // Tutorial-only stand-in. The house has no room named Alley, so this stays out of New Game.
    let alley_id = scene
        .rooms
        .iter()
        .find(|r| r.name.to_ascii_lowercase().contains("alley"))
        .map(|r| r.id);
    if let Some(alley_id) = alley_id {
        if let Some(&alley_ro) = room_origins.get(&alley_id) {
            if !enemies.iter().any(|e| e.room_id == alley_id) {
                let gx = alley_ro[0] - 8.0;
                let gz = alley_ro[2] + 1.0;
                let gy =
                    find_floor_y(&scene, gx, alley_ro[1] - 5.0, gz).unwrap_or(alley_ro[1] - 10.0);
                if let Some(rig_index) = johnny_index {
                    enemies.push(EnemyState {
                        name: "::Alley2::Guard_01".into(),
                        room_id: alley_id,
                        world_pos: [gx, gy, gz],
                        yaw: 0.0,
                        walk_phase: 0.0,
                        health: 2,
                        hit_flash: 0.0,
                        rig_index,
                        pursues: true,
                        invulnerable: false,
                    });
                }
            }
        }
    }

    let mut jp_idx = scene.spawn_jumppoint_index(player_start);

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
    let mut anim_clock = 0.0f32;
    let mut shot_clock = 0.0f32;
    println!(
        "  3D Engine: Spawned ({} actors, {} skins, player={}, sky={}) | WASD move, Mouse/Arrows look, F/Ctrl shoot, B/RightClick Bullet-Time, E interact, V 3rd/1st person, Tab next spawn, G fly/walk",
        enemies.len(),
        skin_cache.len(),
        player_skin.unwrap_or("max_payne"),
        world_sphere.unwrap_or("none")
    );

    if scene.sound_catalogs.is_empty() {
        scene.sound_catalogs = crate::load_sound_catalogs(&catalog_root(level_path));
    }
    if let Some(text) =
        crate::read_game_bytes(&catalog_root(level_path), r"\data\database\materials.txt")
            .and_then(|bytes| String::from_utf8(bytes).ok())
    {
        scene.apply_material_surfaces(&crate::scene::parse_material_surfaces(&text));
    }
    scene.apply_startup_hitpoints();
    scene.apply_startup_hides();
    scene.apply_startup_enables();
    let mut break_voice: Option<Child> = None;

    let mut fb = vec![0u32; W * H];
    let mut zb = vec![f32::INFINITY; W * H];
    let base_dt = 1.0 / 60.0f32;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        if let Some(shot) = shot.as_ref() {
            if shot_clock >= shot.duration || window.is_key_pressed(Key::Space, KeyRepeat::No) {
                break;
            }
        }
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
            if scene.activate_nearest_trigger(&room_origins, player_pos, 36.0) {
                play_queued_sounds(&mut scene, &catalog_root(level_path), &mut break_voice);
            }
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
        let player_moving = mv_len > 0.01;
        if player_moving {
            walk_phase += player_dt * speed * 1.9;
        } else {
            walk_phase = 0.0;
        }
        anim_clock += player_dt;

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
            if can_stand_at(&scene, &room_origins, nx, player_pos[1], nz) {
                player_pos[0] = nx;
                player_pos[2] = nz;
            } else if can_stand_at(&scene, &room_origins, nx, player_pos[1], player_pos[2]) {
                player_pos[0] = nx;
            } else if can_stand_at(&scene, &room_origins, player_pos[0], player_pos[1], nz) {
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
                if en.health <= 0 || en.invulnerable {
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
            let mesh_hit = aim_hit_dynamic(&scene, &room_origins, player_pos, aim_dir);
            match (mesh_hit, best_hit) {
                (Some((midx, mt)), enemy) if enemy.is_none_or(|(_, et)| mt < et) => {
                    if scene.damage_mesh(midx, BERETTA_BULLET_DAMAGE) {
                        println!("  3D Engine: {} broke", scene.dynamic_meshes[midx].name);
                        play_queued_sounds(&mut scene, &catalog_root(level_path), &mut break_voice);
                    }
                }
                (_, Some((idx, _))) => {
                    let en = &mut enemies[idx];
                    en.health -= 1;
                    en.hit_flash = 0.25;
                    println!(
                        "  3D Engine: Hit {}! (health={})",
                        en.name,
                        en.health.max(0)
                    );
                }
                _ => {
                    let probe = [
                        player_pos[0] + aim_dir[0] * 4.0,
                        player_pos[1] + aim_dir[1] * 4.0,
                        player_pos[2] + aim_dir[2] * 4.0,
                    ];
                    toggle_nearest_dynamic_mesh(&mut scene, &room_origins, probe, 16.0);
                }
            }
        }

        // Update Enemy AI (turn toward Max Payne and advance along floor when within 24m)
        let player_feet = [player_pos[0], player_pos[1] - EYE_HEIGHT, player_pos[2]];
        for en in &mut enemies {
            en.hit_flash = (en.hit_flash - base_dt).max(0.0);
            if en.health <= 0 || !en.pursues {
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
                    if can_stand_at(&scene, &room_origins, nx, en.world_pos[1] + EYE_HEIGHT, nz) {
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

        if let Some(shot) = shot.as_ref() {
            let (pos, fwd) = shot.pose(shot_clock);
            player_pos = pos;
            let horiz = (fwd[0] * fwd[0] + fwd[2] * fwd[2]).sqrt().max(0.001);
            pitch = fwd[1].atan2(horiz);
            yaw = fwd[0].atan2(fwd[2]);
            third_person = false;
            shot_clock += base_dt;
        }
        scene.step_dynamic_anims(base_dt);
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();

        // Compute camera position (3rd-person over-the-shoulder vs 1st-person)
        let cam_right = [cy, 0.0, -sy];
        let cam_up = [-sy * sp, cp, -cy * sp];
        let cam_fwd = [sy * cp, sp, cy * cp];
        let cam_pos = if third_person && !fly_mode {
            let mut dist = CHASE_BACK;
            while dist > 0.45 {
                let cand = chase_eye(player_pos, cam_fwd, dist);
                if !hits_wall(&scene, &room_origins, cand[0], cand[1], cand[2]) {
                    break;
                }
                dist -= 0.25;
            }
            chase_eye(player_pos, cam_fwd, dist)
        } else {
            player_pos
        };

        let fov_scale = (H as f32) * 0.85;
        // World sphere (house = sunset) or the flat ceiling when the level names none.
        if let Some(ref tex) = sky {
            paint_sky_strip(tex, cam_right, cam_up, cam_fwd, fov_scale, &mut fb);
        } else {
            for y in 0..H {
                let row_col = if y < H / 2 { 0x00161c28 } else { 0x001e1e22 };
                fb[y * W..(y + 1) * W].fill(row_col);
            }
        }
        zb.fill(f32::INFINITY);
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
        draw_dynamic_meshes(
            &scene,
            &room_origins,
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

        // 3. Render each character with the skin the level named.
        for en in &enemies {
            if let Some(erig) = bodies.get(en.rig_index) {
                render_kfs_character(
                    erig,
                    en.world_pos,
                    en.yaw,
                    if en.walk_phase > 0.0 {
                        en.walk_phase
                    } else {
                        anim_clock
                    },
                    en.walk_phase > 0.0,
                    en.health <= 0,
                    en.hit_flash > 0.0,
                    &world_lights,
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
            if let Some(ref prig) = player_rig {
                render_kfs_character(
                    prig,
                    player_feet,
                    player_mesh_yaw(yaw),
                    if player_moving {
                        walk_phase
                    } else {
                        anim_clock
                    },
                    player_moving,
                    false,
                    false,
                    &world_lights,
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
        if dm.opening_anim().is_none() {
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
        // Play the opening track (retail `DO_Animate`); it holds the end pose.
        if dm.active_track.is_none() {
            if let Some(name) = dm.opening_anim().map(|anim| anim.name.clone()) {
                dm.play_track(&name);
            }
        }
        if let Some(anim) = dm.opening_anim() {
            println!(
                "  3D Engine: DO_Animate {} ({}) -> {:?}",
                dm.name,
                anim.name,
                anim.scripts.first().map(|s| s.as_str()).unwrap_or("ok")
            );
        }
    }
}

fn can_stand_at(
    scene: &LdbScene,
    rooms: &BTreeMap<i32, [f32; 3]>,
    wx: f32,
    wy: f32,
    wz: f32,
) -> bool {
    !hits_wall(scene, rooms, wx, wy, wz) && find_floor_y(scene, wx, wy, wz).is_some()
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
            if !blocks_character(scene, poly.material_id) {
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

fn hits_wall(scene: &LdbScene, rooms: &BTreeMap<i32, [f32; 3]>, wx: f32, wy: f32, wz: f32) -> bool {
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
            if !blocks_character(scene, poly.material_id) {
                continue;
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
    for (idx, dm) in scene.dynamic_meshes.iter().enumerate() {
        if dm.hidden || dm.name.contains("_Tip_") || dm.name.ends_with("Start_FSM.DO") {
            continue;
        }
        let (xform, origin) = dynamic_world(scene, idx, rooms);
        for poly in &dm.polys {
            if poly.vertices.len() < 3 || !blocks_character(scene, poly.material_id) {
                continue;
            }
            let n = xform_dir(xform, poly.normal);
            if n[1].abs() > 0.45 {
                continue;
            }
            let mut min_x = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            let mut min_y = f32::INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            let mut min_z = f32::INFINITY;
            let mut max_z = f32::NEG_INFINITY;
            let mut world = Vec::with_capacity(poly.vertices.len());
            for v in &poly.vertices {
                let p = xform_point(xform, origin, v.pos);
                world.push(p);
                min_x = min_x.min(p[0]);
                max_x = max_x.max(p[0]);
                min_y = min_y.min(p[1]);
                max_y = max_y.max(p[1]);
                min_z = min_z.min(p[2]);
                max_z = max_z.max(p[2]);
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
            let plane_dist = (wx - world[0][0]) * n[0] + (wz - world[0][2]) * n[2];
            if plane_dist.abs() < radius {
                return true;
            }
        }
    }
    false
}

fn poly_policy(scene: &LdbScene, material_id: i32) -> MaterialPolicy {
    scene
        .materials
        .get(&material_id)
        .map(|m| material_policy(&m.category))
        .unwrap_or(MaterialPolicy {
            draw: true,
            character_collides: true,
            writes_z: true,
            detail_offset: 0,
            skybox: false,
        })
}

fn blocks_character(scene: &LdbScene, material_id: i32) -> bool {
    poly_policy(scene, material_id).character_collides
}

const CHASE_BACK: f32 = 2.2;
const CHASE_UP: f32 = 0.4;

fn chase_eye(player: [f32; 3], cam_fwd: [f32; 3], dist: f32) -> [f32; 3] {
    [
        player[0] - cam_fwd[0] * dist,
        player[1] - cam_fwd[1] * dist + CHASE_UP,
        player[2] - cam_fwd[2] * dist,
    ]
}

#[allow(clippy::too_many_arguments)]
fn draw_dynamic_meshes(
    scene: &LdbScene,
    room_origins: &BTreeMap<i32, [f32; 3]>,
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
    for (idx, dm) in scene.dynamic_meshes.iter().enumerate() {
        if dm.hidden {
            continue;
        }
        if dm.name.contains("_Tip_") || dm.name.ends_with("Start_FSM.DO") {
            continue;
        }
        let (mat, origin) = dynamic_world(scene, idx, room_origins);
        render_poly_batch(
            &dm.polys,
            mat,
            origin,
            scene,
            cam_pos,
            cam_right,
            cam_up,
            cam_fwd,
            fov_scale,
            use_lightmaps,
            wireframe,
            false,
            fb,
            zb,
        );
    }
}

/// Closest dynamic mesh that has hit points and whose posed bounds meet the ray.
fn aim_hit_dynamic(
    scene: &LdbScene,
    rooms: &BTreeMap<i32, [f32; 3]>,
    origin: [f32; 3],
    dir: [f32; 3],
) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32)> = None;
    for (idx, dm) in scene.dynamic_meshes.iter().enumerate() {
        if dm.hidden || dm.hitpoints.is_none() || dm.polys.is_empty() {
            continue;
        }
        let (mat, pos) = dynamic_world(scene, idx, rooms);
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for poly in &dm.polys {
            for v in &poly.vertices {
                let p = xform_point(mat, pos, v.pos);
                for axis in 0..3 {
                    min[axis] = min[axis].min(p[axis]);
                    max[axis] = max[axis].max(p[axis]);
                }
            }
        }
        for axis in 0..3 {
            min[axis] -= 0.2;
            max[axis] += 0.2;
        }
        if let Some(t) = ray_slab(origin, dir, min, max) {
            if (0.2..=40.0).contains(&t) && best.is_none_or(|(_, b)| t < b) {
                best = Some((idx, t));
            }
        }
    }
    best
}

fn ray_slab(origin: [f32; 3], dir: [f32; 3], min: [f32; 3], max: [f32; 3]) -> Option<f32> {
    let mut tmin = 0.0f32;
    let mut tmax = f32::INFINITY;
    for i in 0..3 {
        if dir[i].abs() < 1e-8 {
            if origin[i] < min[i] || origin[i] > max[i] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir[i];
        let (mut t1, mut t2) = ((min[i] - origin[i]) * inv, (max[i] - origin[i]) * inv);
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return None;
        }
    }
    if tmax < 0.0 {
        return None;
    }
    Some(if tmin >= 0.0 { tmin } else { tmax })
}

fn play_queued_sounds(scene: &mut LdbScene, game_dir: &Path, voice: &mut Option<Child>) {
    let queued = std::mem::take(&mut scene.sound_queue);
    for sound in queued {
        let Some(filename) = sound.filename.as_deref() else {
            continue;
        };
        let Some(bytes) = crate::read_game_bytes(game_dir, &crate::story_wav_win_path(filename))
        else {
            continue;
        };
        crate::menu_ui::stop_voice(voice);
        *voice = crate::menu_ui::start_voice(&bytes);
    }
}

fn catalog_root(path: &Path) -> PathBuf {
    let start = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let mut dir = start;
    loop {
        if dir.join("x_data.ras").is_file() || dir.join("data").is_dir() {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => return start.to_path_buf(),
        }
    }
}

fn dynamic_world(
    scene: &LdbScene,
    idx: usize,
    rooms: &BTreeMap<i32, [f32; 3]>,
) -> ([f32; 9], [f32; 3]) {
    let ro = rooms
        .get(&scene.dynamic_meshes[idx].room_id)
        .copied()
        .unwrap_or([0.0; 3]);
    let (mat, pos) = scene.dynamic_world_pose(idx);
    (mat, [ro[0] + pos[0], ro[1] + pos[1], ro[2] + pos[2]])
}

fn xform_dir(mat: [f32; 9], v: [f32; 3]) -> [f32; 3] {
    [
        mat[0] * v[0] + mat[3] * v[1] + mat[6] * v[2],
        mat[1] * v[0] + mat[4] * v[1] + mat[7] * v[2],
        mat[2] * v[0] + mat[5] * v[1] + mat[8] * v[2],
    ]
}

fn xform_point(mat: [f32; 9], origin: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    let d = xform_dir(mat, v);
    [d[0] + origin[0], d[1] + origin[1], d[2] + origin[2]]
}

/// Mesh yaw matches the look yaw, so the camera behind that look sits over the shoulder.
fn player_mesh_yaw(look_yaw: f32) -> f32 {
    look_yaw
}

/// Lightmap layer `MUL` = `D3DTOP_MODULATE` (`e2_d3d8_driver_mfc` `0x10008082`): per channel
/// `texture × lightmap / 255`, no overbright. Alpha bits are dropped.
fn lightmap_modulate(texture: u32, lightmap: u32) -> u32 {
    let channel =
        |shift: u32| (((texture >> shift) & 0xff) * ((lightmap >> shift) & 0xff) / 255) << shift;
    channel(16) | channel(8) | channel(0)
}

/// Retail static point light: `setFalloffCurve(0.01, 0, 20)` and `getAttenuation()` = `(c0, c1/R, c2/R²)`
/// (e2mfc `0x10011310`), so D3D gives `1 / (0.01 + 20·(d/R)²)`. Zero past the range, as D3D does.
fn point_light_attenuation(d: f32, radius: f32) -> f32 {
    let r = radius.max(0.1); // e2mfc clamps the range to ≥ 0.1 (`0x100112c0`)
    if d > r {
        return 0.0;
    }
    1.0 / (0.01 + 20.0 * (d / r) * (d / r))
}

fn light_rgb(l: &ScenePointLight) -> [f32; 3] {
    [0, 1, 2].map(|c| l.color[c] * l.intensity)
}

/// D3D fixed-function diffuse for an object material (colour source 0, diffuse 255, black scene
/// ambient, zero emissive): `Σ colour/255 · max(N·L, 0) · attenuation`, clamped to 1.
fn vertex_diffuse(pos: [f32; 3], normal: [f32; 3], lights: &[&ScenePointLight]) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for l in lights {
        let to = [l.pos[0] - pos[0], l.pos[1] - pos[1], l.pos[2] - pos[2]];
        let d = (to[0] * to[0] + to[1] * to[1] + to[2] * to[2]).sqrt();
        let atten = point_light_attenuation(d, l.radius);
        if atten == 0.0 || d < 1e-6 {
            continue;
        }
        let n_dot_l = (normal[0] * to[0] + normal[1] * to[1] + normal[2] * to[2]) / d;
        if n_dot_l <= 0.0 {
            continue;
        }
        let rgb = light_rgb(l);
        for c in 0..3 {
            out[c] += rgb[c] / 255.0 * n_dot_l * atten;
        }
    }
    out.map(|c| c.min(1.0))
}

/// Driver `0x10023ec0`: key = |colour| / 255 × attenuation at the object origin. Sorted descending
/// (partition `0x100253c0`) and capped at `setMaximumLightsPerObject(8)` (exe `0x40deb5`).
/// The collision test (light range reaches the object's bounding sphere) is STRONGLY SUPPORTED, not read.
fn select_object_lights(
    lights: &[ScenePointLight],
    origin: [f32; 3],
    radius: f32,
) -> Vec<&ScenePointLight> {
    let mut keyed: Vec<(f32, &ScenePointLight)> = lights
        .iter()
        .filter_map(|l| {
            let d = ((l.pos[0] - origin[0]).powi(2)
                + (l.pos[1] - origin[1]).powi(2)
                + (l.pos[2] - origin[2]).powi(2))
            .sqrt();
            if d > l.radius + radius {
                return None;
            }
            let r = l.radius.max(0.1);
            let rgb = light_rgb(l);
            let mag = (rgb[0] * rgb[0] + rgb[1] * rgb[1] + rgb[2] * rgb[2]).sqrt();
            Some((mag / (0.01 + 20.0 * (d / r) * (d / r)) / 255.0, l))
        })
        .collect();
    keyed.sort_by(|a, b| b.0.total_cmp(&a.0));
    keyed.into_iter().take(8).map(|(_, l)| l).collect()
}

/// Area-weighted smooth vertex normals. KFS stores no normals (retail builds them at load, method
/// UNKNOWN) and its winding is not documented, so the mesh-wide sign is chosen to face outward.
fn smooth_normals(pts: &[[f32; 3]], faces: &[KfsFace]) -> Vec<[f32; 3]> {
    let mut n = vec![[0.0f32; 3]; pts.len()];
    for f in faces {
        let [a, b, c] = f.indices.map(|i| pts[i]);
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let fnorm = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        for &i in &f.indices {
            for k in 0..3 {
                n[i][k] += fnorm[k];
            }
        }
    }
    let count = pts.len().max(1) as f32;
    let centroid = [0, 1, 2].map(|k| pts.iter().map(|p| p[k]).sum::<f32>() / count);
    let outward: f32 = pts
        .iter()
        .zip(&n)
        .map(|(p, v)| (0..3).map(|k| (p[k] - centroid[k]) * v[k]).sum::<f32>())
        .sum();
    let sign = if outward < 0.0 { -1.0 } else { 1.0 };
    n.into_iter()
        .map(|v| {
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            if len < 1e-12 {
                [0.0; 3]
            } else {
                v.map(|c| c * sign / len)
            }
        })
        .collect()
}

/// Pull a decal toward the camera by its `DetailOffset` distance.
fn decal_depth(z: f32, depth_bias: f32) -> f32 {
    z - depth_bias
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
    rig: &CharacterRig,
    world_pos: [f32; 3],
    yaw: f32,
    anim_time: f32,
    moving: bool,
    dead: bool,
    hit_flash: bool,
    lights: &[ScenePointLight],
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
    let mesh = &rig.mesh;
    let anim = if moving {
        rig.walk.as_ref().or(rig.idle.as_ref())
    } else {
        rig.idle.as_ref().or(rig.walk.as_ref())
    };
    let posed = if dead {
        mesh.vertices.clone()
    } else {
        pose_mesh(mesh, rig.skin.as_ref(), rig.bind.as_ref(), anim, anim_time)
    };

    // Pre-transform character vertices into camera space
    let mut cam_pts = Vec::with_capacity(posed.len());
    let mut world_pts = Vec::with_capacity(posed.len());
    for &[lx, ly, lz] in &posed {
        let (px, py, pz) = if dead {
            // Lie flat on the floor when dead
            (lx, 0.18 + lz.abs() * 0.5, ly - 0.85)
        } else {
            (lx, ly, lz)
        };
        let wx = world_pos[0] + px * cy + pz * sy;
        let wy = world_pos[1] + py;
        let wz = world_pos[2] - px * sy + pz * cy;
        world_pts.push([wx, wy, wz]);
        let dx = wx - cam_pos[0];
        let dy = wy - cam_pos[1];
        let dz = wz - cam_pos[2];
        let cx = dx * cam_right[0] + dy * cam_right[1] + dz * cam_right[2];
        let cy_c = dx * cam_up[0] + dy * cam_up[1] + dz * cam_up[2];
        let cz = dx * cam_fwd[0] + dy * cam_fwd[1] + dz * cam_fwd[2];
        cam_pts.push([cx, cy_c, cz]);
    }

    let vertex_light: Vec<[f32; 3]> = if hit_flash {
        vec![[1.35; 3]; world_pts.len()]
    } else {
        let normals = smooth_normals(&world_pts, &mesh.faces);
        let radius = world_pts
            .iter()
            .map(|p| {
                ((p[0] - world_pos[0]).powi(2)
                    + (p[1] - world_pos[1]).powi(2)
                    + (p[2] - world_pos[2]).powi(2))
                .sqrt()
            })
            .fold(0.0f32, f32::max);
        let picked = select_object_lights(lights, world_pos, radius);
        world_pts
            .iter()
            .zip(&normals)
            .map(|(p, n)| vertex_diffuse(*p, *n, &picked))
            .collect()
    };
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
                light: vertex_light[i0],
            },
            CamVert {
                cx: c1[0],
                cy: c1[1],
                cz: c1[2],
                u: face.uvs[1][0],
                v: face.uvs[1][1],
                lu: 0.5,
                lv: 0.5,
                light: vertex_light[i1],
            },
            CamVert {
                cx: c2[0],
                cy: c2[1],
                cz: c2[2],
                u: face.uvs[2][0],
                v: face.uvs[2][1],
                lu: 0.5,
                lv: 0.5,
                light: vertex_light[i2],
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
                light_over_z: cv.light.map(|c| c * inv_z),
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
                rasterize_tri(
                    fb,
                    zb,
                    proj[0],
                    proj[k],
                    proj[k + 1],
                    tex,
                    None,
                    None,
                    true,
                    0.0,
                );
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
    for opaque in [true, false] {
        for poly in polys {
            let policy = poly_policy(scene, poly.material_id);
            if !policy.draw || policy.writes_z != opaque {
                continue;
            }
            draw_poly(
                poly,
                policy,
                mat,
                origin,
                scene,
                cam_pos,
                cam_right,
                cam_up,
                cam_fwd,
                fov_scale,
                use_lightmaps,
                wireframe,
                cull_backfaces,
                fb,
                zb,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_poly(
    poly: &ScenePoly,
    policy: MaterialPolicy,
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
                return;
            }
        }
    }

    let material = scene.materials.get(&poly.material_id);
    let tex = material
        .and_then(|m| m.texture_index)
        .and_then(|ti| scene.textures.get(ti));
    let alpha_reference = material.and_then(|m| m.alpha_reference);
    let lmap = if use_lightmaps && poly.lightmap_id >= 0 {
        scene.lightmaps.get(poly.lightmap_id as usize)
    } else {
        None
    };

    let shade = ((wn[0] * 0.35 + wn[1] * 0.75 + wn[2] * 0.55).abs() * 0.55 + 0.45).clamp(0.35, 1.0);
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
            light: [shade; 3],
        });
    }
    if all_behind || all_far {
        return;
    }

    let clipped = clip_near_plane(&cam_verts);
    if clipped.len() < 3 {
        return;
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
            light_over_z: cv.light.map(|c| c * inv_z),
        });
    }

    for k in 1..proj.len() - 1 {
        let p0 = proj[0];
        let p1 = proj[k];
        let p2 = proj[k + 1];
        if wireframe {
            draw_line(fb, p0.sx, p0.sy, p1.sx, p1.sy, 0x0066ccff);
            draw_line(fb, p1.sx, p1.sy, p2.sx, p2.sy, 0x0066ccff);
            draw_line(fb, p2.sx, p2.sy, p0.sx, p0.sy, 0x0066ccff);
        } else {
            rasterize_tri(
                fb,
                zb,
                p0,
                p1,
                p2,
                tex,
                alpha_reference,
                lmap,
                policy.writes_z,
                policy.detail_offset as f32 * DETAIL_OFFSET_M,
            );
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
                light: [0, 1, 2].map(|c| a.light[c] + (b.light[c] - a.light[c]) * t),
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
    alpha_reference: Option<u8>,
    lmap: Option<&DecodedLightmap>,
    write_z: bool,
    depth_bias: f32,
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
            let z = decal_depth(1.0 / inv_z, depth_bias);
            let idx = row + x;
            if z >= zb[idx] {
                continue;
            }

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
            // Alpha test before the depth write, so cut-out texels hide nothing.
            // Retail passes when alpha >= AlphaReference (D3DCMP_GREATEREQUAL).
            if alpha_reference.is_some_and(|reference| ((base_rgb >> 24) as u8) < reference) {
                continue;
            }
            if write_z {
                zb[idx] = z;
            }

            let mut r = ((base_rgb >> 16) & 0xff) as f32;
            let mut g = ((base_rgb >> 8) & 0xff) as f32;
            let mut b = (base_rgb & 0xff) as f32;

            if let Some(lm) = lmap {
                let lu = (w0 * p0.lu_over_z + w1 * p1.lu_over_z + w2 * p2.lu_over_z) * z;
                let lv = (w0 * p0.lv_over_z + w1 * p1.lv_over_z + w2 * p2.lv_over_z) * z;
                let lx = (lu.clamp(0.0, 1.0) * (lm.width - 1) as f32) as u32;
                let ly = (lv.clamp(0.0, 1.0) * (lm.height - 1) as f32) as u32;
                if let Some(&lcol) = lm.pixels.get((ly * lm.width + lx) as usize) {
                    let lit = lightmap_modulate(base_rgb, lcol);
                    r = ((lit >> 16) & 0xff) as f32;
                    g = ((lit >> 8) & 0xff) as f32;
                    b = (lit & 0xff) as f32;
                }
            } else {
                let light = [0, 1, 2].map(|c| {
                    (w0 * p0.light_over_z[c] + w1 * p1.light_over_z[c] + w2 * p2.light_over_z[c])
                        * z
                });
                r *= light[0];
                g *= light[1];
                b *= light[2];
            }

            let ri = (r.min(255.0) as u32) << 16;
            let gi = (g.min(255.0) as u32) << 8;
            let bi = b.min(255.0) as u32;
            fb[idx] = ri | gi | bi;
        }
    }
}

/// `bg_sunset_all.tga` is a 360°×90° strip: bottom edge on the horizon, top edge at the zenith.
pub(crate) fn sky_strip_uv(dir: [f32; 3]) -> [f32; 2] {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2])
        .sqrt()
        .max(1e-8);
    let x = dir[0] / len;
    let y = dir[1] / len;
    let z = dir[2] / len;
    let u = (x.atan2(z) / std::f32::consts::TAU + 0.5).rem_euclid(1.0);
    let lat = y.clamp(-1.0, 1.0).asin().max(0.0);
    let v = (1.0 - lat / (std::f32::consts::FRAC_PI_2)).clamp(0.0, 0.999);
    [u, v]
}

fn paint_sky_strip(
    tex: &DecodedTexture,
    cam_right: [f32; 3],
    cam_up: [f32; 3],
    cam_fwd: [f32; 3],
    fov_scale: f32,
    fb: &mut [u32],
) {
    let half_w = W as f32 * 0.5;
    let half_h = H as f32 * 0.5;
    let tw = tex.width.max(1);
    let th = tex.height.max(1);
    for y in 0..H {
        for x in 0..W {
            let cx = (x as f32 + 0.5 - half_w) / fov_scale;
            let cy = (half_h - (y as f32 + 0.5)) / fov_scale;
            let dir = [
                cam_right[0] * cx + cam_up[0] * cy + cam_fwd[0],
                cam_right[1] * cx + cam_up[1] * cy + cam_fwd[1],
                cam_right[2] * cx + cam_up[2] * cy + cam_fwd[2],
            ];
            let [u, v] = sky_strip_uv(dir);
            let tx = ((u * tw as f32) as u32).min(tw - 1);
            let ty = ((v * th as f32) as u32).min(th - 1);
            fb[y * W + x] = tex.pixels[(ty * tw + tx) as usize];
        }
    }
}

fn world_sphere_roots(level_path: &Path) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![std::path::PathBuf::from(
        "/tmp/omp-xdata-ref/data/database/worldspheres",
    )];
    for anc in level_path.ancestors() {
        dirs.push(anc.join("data/database/worldspheres"));
        dirs.push(anc.join("xdata/data/database/worldspheres"));
    }
    dirs
}

fn read_ci(dirs: &[std::path::PathBuf], name: &str) -> Option<Vec<u8>> {
    let want = name.to_ascii_lowercase();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for ent in rd.flatten() {
            if ent
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&want)
            {
                if let Ok(bytes) = std::fs::read(ent.path()) {
                    return Some(bytes);
                }
            }
        }
    }
    None
}

fn load_world_sphere(level_path: &Path, name: &str) -> Option<DecodedTexture> {
    let roots = world_sphere_roots(level_path);
    let catalog = read_ci(&roots, "worldspheres.txt")
        .or_else(|| crate::kf2::load_bytes_from_ras(level_path, &["worldspheres.txt"]))?;
    let catalog = String::from_utf8_lossy(&catalog);
    let export = crate::world_sphere_export(&catalog, name)?;
    let kf2 = read_ci(&roots, &export)
        .or_else(|| crate::kf2::load_bytes_from_ras(level_path, &[&export]))?;
    let image = crate::first_image_name(&kf2)?;
    let mut tex_dirs: Vec<std::path::PathBuf> = roots.iter().map(|r| r.join("textures")).collect();
    tex_dirs.extend(roots.iter().cloned());
    let bytes = read_ci(&tex_dirs, &image)
        .or_else(|| crate::kf2::load_bytes_from_ras(level_path, &[&image]))?;
    let (width, height, pixels) = decode_texture_blob(&bytes)?;
    Some(DecodedTexture {
        path: image,
        flag: 4,
        width,
        height,
        pixels,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SceneDynamicMesh, SceneMaterial, ScenePoly, SceneVertex};

    fn light(pos: [f32; 3], rgb: f32, radius: f32, intensity: f32) -> ScenePointLight {
        ScenePointLight {
            id: 0,
            room_id: 0,
            pos,
            color: [rgb, rgb, rgb, 255.0],
            radius,
            intensity,
        }
    }

    #[test]
    fn point_light_attenuation_uses_retail_falloff_curve() {
        // 1 / (0.01 + 20·(d/R)²), zero past the range.
        assert_eq!(point_light_attenuation(0.0, 10.0), 100.0);
        assert!((point_light_attenuation(5.0, 10.0) - 1.0 / 5.01).abs() < 1e-6);
        assert_eq!(point_light_attenuation(12.0, 10.0), 0.0);
    }

    #[test]
    fn vertex_diffuse_is_lambert_times_attenuation_clamped() {
        let l = [light([0.0, 5.0, 0.0], 255.0, 10.0, 1.0)];
        let lit = vertex_diffuse(
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            &l.iter().collect::<Vec<_>>(),
        );
        assert!((lit[0] - 1.0 / 5.01).abs() < 1e-5);
        let back = vertex_diffuse(
            [0.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            &l.iter().collect::<Vec<_>>(),
        );
        assert_eq!(back, [0.0, 0.0, 0.0]);
        let near = [light([0.0, 0.1, 0.0], 255.0, 10.0, 1.0)];
        let sat = vertex_diffuse(
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            &near.iter().collect::<Vec<_>>(),
        );
        assert_eq!(sat, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn smooth_normals_point_outward_for_either_winding() {
        // Tetrahedron with its apex up; the apex normal must point +Y whatever the face winding.
        let pts = [
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, -1.0],
            [1.0, 0.0, -1.0],
            [0.0, 0.0, 1.0],
        ];
        let face = |indices| KfsFace {
            indices,
            uvs: [[0.0; 2]; 3],
            mat_index: 0,
        };
        let ccw = [
            face([0, 1, 2]),
            face([0, 2, 3]),
            face([0, 3, 1]),
            face([1, 3, 2]),
        ];
        let cw = [
            face([0, 2, 1]),
            face([0, 3, 2]),
            face([0, 1, 3]),
            face([1, 2, 3]),
        ];
        for faces in [&ccw[..], &cw[..]] {
            let n = smooth_normals(&pts, faces);
            assert!(n[0][1] > 0.99, "apex normal {:?}", n[0]);
        }
    }

    #[test]
    fn object_lights_sort_brightest_first_and_cap_at_eight() {
        let mut lights: Vec<ScenePointLight> = (0..10)
            .map(|i| light([i as f32 * 0.5, 0.0, 0.0], 255.0, 20.0, 1.0))
            .collect();
        lights.push(light([0.2, 0.0, 0.0], 1.0, 20.0, 1.0)); // close but dim
        lights.push(light([30.0, 0.0, 0.0], 255.0, 5.0, 1.0)); // out of reach
        let picked = select_object_lights(&lights, [0.0, 0.0, 0.0], 1.0);
        assert_eq!(picked.len(), 8);
        let xs: Vec<f32> = picked.iter().map(|l| l.pos[0]).collect();
        assert_eq!(xs, vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5]);
    }

    #[test]
    fn lightmap_modulates_the_texture_like_d3dtop_modulate() {
        // e2_d3d8_driver_mfc 0x10008082: layer MUL -> colour op 4 (D3DTOP_MODULATE).
        assert_eq!(lightmap_modulate(0x0080_8080, 0x0080_8080), 0x0040_4040);
        assert_eq!(lightmap_modulate(0x0020_4060, 0x00ff_ffff), 0x0020_4060);
        assert_eq!(lightmap_modulate(0x00ff_ffff, 0x0000_0000), 0x0000_0000);
    }

    #[test]
    fn chase_eye_sits_behind_and_above_the_player() {
        let eye = chase_eye([0.0, 1.6, 0.0], [0.0, 0.0, 1.0], CHASE_BACK);
        assert!(eye[0].abs() < 1e-4);
        assert!((eye[1] - 2.0).abs() < 1e-4);
        assert!((eye[2] + CHASE_BACK).abs() < 1e-4);
    }

    #[test]
    fn sunset_strip_keeps_the_sun_on_the_horizon() {
        let ahead = sky_strip_uv([0.0, 0.0, 1.0]);
        assert!((ahead[0] - 0.5).abs() < 0.02);
        assert!(
            ahead[1] > 0.95,
            "horizon sits on the bottom edge, got {}",
            ahead[1]
        );
        let up = sky_strip_uv([0.0, 1.0, 0.0]);
        assert!(up[1] < 0.05, "zenith sits on the top edge, got {}", up[1]);
        let right = sky_strip_uv([1.0, 0.0, 0.0]);
        assert!((right[0] - 0.75).abs() < 0.02);
    }

    #[test]
    fn house_sunset_strip_loads_when_the_catalog_is_present() {
        let catalog = Path::new("/tmp/omp-xdata-ref/data/database/worldspheres/worldspheres.txt");
        if !catalog.is_file() {
            return;
        }
        let tex = load_world_sphere(Path::new("/tmp/omp-xdata-ref"), "sunset").expect("sunset");
        assert_eq!(tex.width, 1024);
        assert_eq!(tex.height, 256);
    }

    fn empty_scene() -> LdbScene {
        LdbScene {
            textures: Vec::new(),
            materials: BTreeMap::new(),
            lightmaps: Vec::new(),
            exits: Vec::new(),
            static_meshes: Vec::new(),
            jumppoints: Vec::new(),
            fsms: Vec::new(),
            characters: Vec::new(),
            triggers: Vec::new(),
            dynamic_meshes: Vec::new(),
            items: Vec::new(),
            point_lights: Vec::new(),
            rooms: Vec::new(),
            bytes_consumed: 0,
            sound_catalogs: BTreeMap::new(),
            sound_queue: Vec::new(),
        }
    }

    fn vert(pos: [f32; 3]) -> SceneVertex {
        SceneVertex {
            pos,
            uv: [0.0; 2],
            luv: [0.0; 2],
        }
    }

    #[test]
    fn closed_wood_door_blocks_and_dummy_does_not() {
        let mut scene = empty_scene();
        scene.materials.insert(
            1,
            SceneMaterial {
                id: 1,
                category: "wood".into(),
                texture_path: String::new(),
                texture_index: None,
                alpha_reference: None,
            },
        );
        scene.materials.insert(
            2,
            SceneMaterial {
                id: 2,
                category: "dummy".into(),
                texture_path: String::new(),
                texture_index: None,
                alpha_reference: None,
            },
        );
        let quad = ScenePoly {
            vertices: vec![
                vert([0.0, 0.0, -1.0]),
                vert([0.0, 2.0, -1.0]),
                vert([0.0, 2.0, 1.0]),
                vert([0.0, 0.0, 1.0]),
            ],
            normal: [1.0, 0.0, 0.0],
            material_id: 1,
            lightmap_id: -1,
        };
        scene.dynamic_meshes.push(SceneDynamicMesh {
            name: "StartRoom::door".into(),
            room_id: 1,
            world_mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            world_pos: [0.0; 3],
            local_start_mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            local_start_pos: [0.0; 3],
            toggled: false,
            hidden: false,
            hitpoints: None,
            polys: vec![quad],
            anims: Vec::new(),
            active_track: None,
            track_time: 0.0,
            track_duration: 0.0,
        });
        let rooms = BTreeMap::new();
        assert!(hits_wall(&scene, &rooms, 0.1, 1.65, 0.0));
        scene.dynamic_meshes[0].polys[0].material_id = 2;
        assert!(!hits_wall(&scene, &rooms, 0.1, 1.65, 0.0));
    }

    #[test]
    fn graffiti_depth_is_closer_than_the_wall() {
        let wall = decal_depth(2.0, 0.0);
        let leaflet = decal_depth(2.0, DETAIL_OFFSET_M);
        assert!(leaflet < wall);
        assert!((wall - leaflet - 0.02).abs() < 1e-5);
    }

    #[test]
    fn player_faces_the_look_direction() {
        assert!((player_mesh_yaw(0.4) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn hidden_dynamic_mesh_is_not_drawn() {
        let mut scene = empty_scene();
        scene.dynamic_meshes.push(glass_quad(false));
        assert!(
            painted_pixels(&scene) > 0,
            "visible glass must cover pixels"
        );
        scene.dynamic_meshes[0].hidden = true;
        assert_eq!(
            painted_pixels(&scene),
            0,
            "::Bathroom::glass01.DO must be skipped when hidden"
        );
    }

    #[test]
    fn texels_below_the_alpha_reference_are_not_drawn() {
        // e2_d3d8_driver_mfc 0x100353f8: D3DRS_ALPHAFUNC = 7 (D3DCMP_GREATEREQUAL).
        let mut scene = empty_scene();
        scene.textures.push(DecodedTexture {
            path: "mask".into(),
            flag: 4,
            width: 1,
            height: 1,
            pixels: vec![0x0e80_8080],
        });
        scene.materials.insert(
            1,
            SceneMaterial {
                id: 1,
                category: "leaves".into(),
                texture_path: String::new(),
                texture_index: Some(0),
                alpha_reference: Some(15),
            },
        );
        scene.dynamic_meshes.push(glass_quad(false));
        assert_eq!(painted_pixels(&scene), 0, "alpha 14 fails reference 15");
        scene.textures[0].pixels[0] = 0x0f80_8080;
        assert!(painted_pixels(&scene) > 0, "alpha 15 passes reference 15");
    }

    #[test]
    fn interacting_with_a_drawer_plays_its_open_track() {
        let mut scene = empty_scene();
        let mut drawer = glass_quad(false);
        drawer.name = "::startroom::New mesh 62::drawer.DO".into();
        drawer.anims.push(crate::scene::SceneDynamicAnim {
            name: "open".into(),
            duration: 0.7,
            start_mat: drawer.local_start_mat,
            start_pos: [0.0; 3],
            end_mat: drawer.local_start_mat,
            end_pos: [0.3, 0.0, 0.0],
            scripts: Vec::new(),
            curves: [Vec::new(), Vec::new()],
        });
        scene.dynamic_meshes.push(drawer);
        toggle_nearest_dynamic_mesh(&mut scene, &BTreeMap::new(), [0.5, 0.0, 0.0], 36.0);
        let drawer = &scene.dynamic_meshes[0];
        assert_eq!(drawer.active_track.as_deref(), Some("open"));
        assert_eq!(
            drawer.posed_local().1,
            [0.0; 3],
            "starts closed, then animates"
        );
    }

    fn glass_quad(hidden: bool) -> SceneDynamicMesh {
        SceneDynamicMesh {
            name: "::Bathroom::glass01.DO".into(),
            room_id: 0,
            world_mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            world_pos: [0.0; 3],
            local_start_mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            local_start_pos: [0.0; 3],
            toggled: false,
            hidden,
            hitpoints: None,
            polys: vec![ScenePoly {
                vertices: vec![
                    vert([0.0, 0.0, -1.0]),
                    vert([0.0, 2.0, -1.0]),
                    vert([0.0, 2.0, 1.0]),
                    vert([0.0, 0.0, 1.0]),
                ],
                normal: [1.0, 0.0, 0.0],
                material_id: 1,
                lightmap_id: -1,
            }],
            anims: Vec::new(),
            active_track: None,
            track_time: 0.0,
            track_duration: 0.0,
        }
    }

    fn painted_pixels(scene: &LdbScene) -> usize {
        let mut fb = vec![0u32; W * H];
        let mut zb = vec![f32::INFINITY; W * H];
        draw_dynamic_meshes(
            scene,
            &BTreeMap::new(),
            [-4.0, 1.0, 0.0],
            [0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            (H as f32) * 0.85,
            false,
            false,
            &mut fb,
            &mut zb,
        );
        fb.iter().filter(|px| **px != 0).count()
    }
}
