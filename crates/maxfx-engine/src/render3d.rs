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
use crate::projectiles::{Explosion, Flight, Flyer, FlyingDef};
use crate::scene::{
    decode_texture_blob, material_policy, DecodedLightmap, DecodedTexture, LdbScene, LevelFog,
    MaterialPolicy, ScenePointLight, ScenePoly, SceneVertex,
};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Child;

const W: usize = 960;
const H: usize = 600;
/// `--renderer sw` or `OPENMAXFX_RENDERER=sw` selects the software rasterizer; anything else
/// asks for the hardware renderer and falls back to software when no GPU is available.
static SOFTWARE_RENDERER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn use_software_renderer(software: bool) {
    SOFTWARE_RENDERER.store(software, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn hardware_renderer() -> bool {
    let env_sw = std::env::var("OPENMAXFX_RENDERER").is_ok_and(|v| v.eq_ignore_ascii_case("sw"));
    !(env_sw || SOFTWARE_RENDERER.load(std::sync::atomic::Ordering::Relaxed))
}

/// Frame size shared with the hardware renderer.
pub(crate) const FRAME_W: usize = W;
pub(crate) const FRAME_H: usize = H;
pub(crate) const NEAR_Z: f32 = 0.15;
/// Back plane. Retail (`0x67a4de`) pushes it each frame past the far side of every bounding
/// sphere in view, whatever the fog, so it never clips what is visible; here it is a depth range.
pub(crate) const FAR_Z: f32 = 100_000.0;
const EYE_HEIGHT: f32 = 1.65;
/// One `DetailOffset` step from `materials.txt`, in metres toward the camera.
pub(crate) const DETAIL_OFFSET_M: f32 = 0.02;

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
    health: f32,
    /// The skin's `MaximumHealth` times `EnemyHealthMultiplier` (0x691bd0).
    max_health: f32,
    hit_flash: f32,
    rig_index: usize,
    pursues: bool,
    invulnerable: bool,
    /// Waits out of the world in a sealed room until a script teleports it.
    held: bool,
    /// `AutoAimTarget`.
    auto_aim_target: bool,
    /// Index of the death clip, seconds into it, and how many of its sounds have played.
    death: Option<(usize, f32, usize)>,
}

/// How a level run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelEnd {
    Closed,
    /// Max died, and the level restarts. Retail reloads the last save, which the engine lacks.
    Died,
}

#[allow(clippy::too_many_arguments)]
pub fn run_3d_walkthrough(
    screen: &mut crate::frontend::Screen,
    mut scene: LdbScene,
    level_path: &Path,
    title: &str,
    player_start: Option<&str>,
    player_skin: Option<&str>,
    world_sphere: Option<&str>,
    cinematic: bool,
) -> Result<LevelEnd, String> {
    screen.set_title(title);
    let (catalog_player, johnny_rig) = load_character_rigs(level_path);
    let mut player_rig = player_skin
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
    let mut skin_cache: BTreeMap<String, (usize, bool, bool)> = BTreeMap::new();

    let difficulty = crate::load_difficulty(&catalog_root(level_path));
    let enemy_health = difficulty.map_or(1.0, |difficulty| difficulty.enemy_health);
    let auto_aim_scale = difficulty.map_or([1.0; 2], |difficulty| difficulty.auto_aim);
    // Build active enemies from `scene.characters`, which wait until a script teleports them.
    let mut enemies: Vec<EnemyState> = Vec::new();
    for ch in &scene.characters {
        let key = ch.skin.to_ascii_lowercase();
        let (rig_index, invulnerable, auto_aim_target) =
            if let Some(hit) = skin_cache.get(&key).copied() {
                hit
            } else if let Some(body) = load_skin_body(level_path, &ch.skin) {
                let idx = bodies.len();
                let found = (idx, body.invulnerable, body.auto_aim_target);
                bodies.push(body.rig);
                skin_cache.insert(key, found);
                found
            } else if let Some(idx) = johnny_index {
                skin_cache.insert(key, (idx, false, false));
                (idx, false, false)
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
        let max_health = bodies[rig_index].max_health.max(1) as f32 * enemy_health;
        enemies.push(EnemyState {
            name: ch.name.clone(),
            room_id: ch.room_id,
            world_pos: [ro[0] + ch.pos[0], wy, ro[2] + ch.pos[2]],
            yaw: 0.0,
            walk_phase: 0.0,
            health: if ch.starts_dead() { 0.0 } else { max_health },
            max_health,
            hit_flash: 0.0,
            rig_index,
            pursues: ch.pursues(),
            invulnerable,
            held: false,
            auto_aim_target,
            death: None,
        });
    }
    let mut jp_idx = scene.spawn_jumppoint_index(player_start);
    let start_room = scene.jumppoints.get(jp_idx).map_or(-1, |jp| jp.room_id);

    let mut player_pos = spawn_pos(&scene, &room_origins, jp_idx);
    let mut yaw: f32 = scene.jumppoints.get(jp_idx).map_or(0.0, |jp| jp.yaw());
    let mut pitch: f32 = 0.0;
    let mut vel_y: f32 = 0.0;
    let mut walk_phase: f32 = 0.0;
    let mut third_person = true;
    let mut bullet_time = false;
    let mut right_was_down = false;
    let mut shot_timer: f32 = 0.0;
    // Seconds into the player's shot or reload clip.
    let mut level_end = LevelEnd::Closed;
    let mut action_time: f32 = 0.0;
    let mut strip_weapon = scene.inventory.current.clone();
    let mut strip_age = f32::MAX;
    // `(seconds into the clip, message)` of the painkiller still to come.
    let mut pill_messages: Vec<(f32, String)> = Vec::new();
    // The `CHARANIM_` index of `CharacterRig::action`.
    let mut action_index = String::new();
    let mut reload_seen = 0;
    // Reach of the melee weapon swung last, and the clip times of its blows still to land.
    let mut melee: Option<f32> = None;
    let mut shot_seed: u32 = 0x2545_f491;
    let mut droplets: Vec<crate::decals::Droplet> = Vec::new();
    let mut flyers: Vec<(Flyer, FlyingDef)> = Vec::new();
    let mut explosions: Vec<Explosion> = Vec::new();
    // The rounds of the running shoot clip still to come, and the weapon firing them.
    let mut pending: Vec<crate::hud::ShotRound> = Vec::new();
    let mut firing = String::new();
    // Seconds until the reload's sound, and its cue.
    let mut reload_cue: Option<(f32, String)> = None;
    let mut fly_mode = false;
    let mut use_lightmaps = true;
    let mut wireframe = false;
    let mut last_mouse: Option<(f32, f32)> = None;
    let mut anim_clock = 0.0f32;
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
    scene.apply_startup_items();
    scene.apply_startup_sends();
    scene.apply_startup_animates();
    scene.apply_startup_effects();
    if cinematic {
        // The scripts own the camera; the player body stays where the level puts it (Max with the
        // sniper rifle on the rooftop in the startup intro).
        scene.player_controls = false;
    }
    // Built after the material surfaces, so batches follow the draw policies.
    // The session's window: its device and swapchain are rebuilt for this scene, not reopened.
    let mut gpu = screen.take_gpu(&scene);
    if let Some((renderer, _)) = gpu.as_mut() {
        renderer.set_sky(sky.as_ref());
    }
    if gpu.is_none() {
        screen.fall_back_to_software(title)?;
    }
    println!(
        "  3D Engine: renderer = {}",
        if gpu.is_some() {
            "hardware (wgpu)"
        } else {
            "software"
        }
    );
    let mut sounds = SoundChannels::default();
    // Level scripts' `A_PlayMusic`; dropped (stopped) when the level ends.
    let mut music = crate::menu_ui::Music::default();
    let camerapaths_txt = crate::read_game_bytes(
        &catalog_root(level_path),
        r"\data\database\camerapaths\camerapaths.txt",
    )
    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    .unwrap_or_default();
    let mut camera_shake: Option<CameraShake> = None;
    // `CAM_AnimateParented`: the path and the character it follows.
    let mut death_cam: Option<(CameraShake, String)> = None;
    // Last frame's eye: the sound listener.
    let mut listener = player_pos;
    let game_root = catalog_root(level_path);
    let particles_txt =
        crate::read_game_bytes(&game_root, r"\data\database\particles\particles.txt")
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
    let hit_effects = HitEffects {
        game_dir: &game_root,
        particles_txt: &particles_txt,
        sounds_txt: crate::load_sound_catalog(&game_root, "projectiles").unwrap_or_default(),
    };
    let mut particle_lights = ParticleLights::default();
    let mut particle_sprites = ParticleSprites::default();
    scene.inventory.defs = crate::load_weapon_defs(&game_root);
    let mut player_health = crate::load_player_health(&game_root);
    let mut last_health = player_health.as_ref().map_or(0.0, |max| max.health);
    let config = crate::read_game_bytes(&game_root, r"\data\globalconfig.txt").unwrap_or_default();
    let config = String::from_utf8_lossy(&config);
    let falling = crate::health::Falling::parse(&config);
    let damage_flash = crate::health::DamageFlash::parse(&config);
    let death_clips = crate::load_death_clips(&game_root);
    let player_clips = crate::load_character_clips(&game_root, player_skin.unwrap_or("max_payne"));
    let mut motion = crate::player::PlayerMotion::default();
    let mut grounded = true;
    let mut dummy_sounds: BTreeMap<String, Option<String>> = BTreeMap::new();
    if let Some(rig) = player_rig.as_mut() {
        rig.upper_bones = crate::load_upper_bones(&game_root);
        rig.gun_bones = crate::load_gun_bones(&game_root);
    }
    let (mut level_items, item_models) = crate::load_level_items(&game_root, &scene, &room_origins);
    let flyer_models: BTreeMap<String, crate::kf2::KfsMesh> = scene
        .inventory
        .defs
        .values()
        .flat_map(|def| &def.flying)
        .filter_map(|(flying, _)| {
            let path = flying.model.as_ref()?;
            Some((path.clone(), crate::load_flying_model(&game_root, path)?))
        })
        .collect();
    let weapon_models: BTreeMap<String, crate::kf2::KfsMesh> = scene
        .inventory
        .defs
        .iter()
        .filter_map(|(name, def)| {
            let model = crate::load_weapon_model(&game_root, def.model.as_deref()?)?;
            Some((name.clone(), model))
        })
        .collect();
    scene.inventory.best_weapons = crate::load_best_weapons(&game_root);
    scene.apply_startup_player_scripts();
    let hud_assets = crate::hud::HudAssets::load(&game_root);
    // Latin-1, like the HUD font's character codes.
    let hud_strings = crate::read_game_bytes(&game_root, r"\data\database\stringtable.txt")
        .map(|bytes| {
            bytes
                .iter()
                .map(|byte| char::from(*byte))
                .collect::<String>()
        })
        .map(|text| crate::hud::parse_string_table(&text))
        .unwrap_or_default();
    let mut print_log = crate::hud::HudPrintLog::default();
    let mut tip_log = crate::hud::HudPrintLog::default();
    let text = |path: &str| {
        crate::read_game_bytes(&game_root, path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    };
    let mut chase_rig = text(r"\data\globalconfig.txt")
        .zip(text(r"\data\database\skeletons\default_skeleton.txt"))
        .and_then(|(config, skeleton)| ChaseRig::parse(&config, &skeleton))
        .unwrap_or(RETAIL_RIG);
    let stand_point = chase_rig.point;
    // The mesh the player last ran into: `DO_CharacterCollides` fires once per contact.
    let mut last_contact = None;
    let mut capture = std::env::var_os("OPENMAXFX_CAPTURE_DIR").map(|dir| {
        let dir = PathBuf::from(dir);
        if let Err(error) = std::fs::create_dir_all(&dir) {
            eprintln!("  3D Engine: capture folder: {error}");
        }
        FrameCapture::new(dir, title)
    });
    let mut video = capture.as_ref().and_then(|capture| {
        let path = capture.dir.join(format!("{}.mp4", capture.level));
        VideoCapture::start(&path)
            .inspect_err(|error| eprintln!("  3D Engine: video capture: {error}"))
            .ok()
    });
    let mut decals = crate::decals::DecalField::load(
        &game_root,
        scene
            .inventory
            .defs
            .values()
            .flat_map(|def| def.projectiles.iter().chain(&def.quiet_projectiles))
            .flat_map(|projectile| {
                let splatter = projectile.character_hit.as_ref();
                let blood = splatter.and_then(|splatter| splatter.decal.as_deref());
                projectile.decal.as_deref().into_iter().chain(blood)
            }),
    );
    let mut slow_motion = crate::read_game_bytes(&game_root, r"\data\globalconfig.txt")
        .and_then(|bytes| {
            let characters = crate::read_game_bytes(&game_root, r"\data\database\characters.h")
                .map(|text| String::from_utf8_lossy(&text).into_owned())
                .unwrap_or_default();
            crate::hud::SlowMotion::parse(&String::from_utf8_lossy(&bytes), &characters)
        })
        .unwrap_or(crate::hud::SlowMotion {
            level: 1.0,
            active: false,
            seconds: 15.0,
            threshold: 0.001,
            dive_threshold: 0.01,
            dive_penalty: 0.1,
            heartbeat_message: String::new(),
            heartbeat_interval: 1.25,
            heartbeat_in: 0.0,
            regeneration_seconds: 1.0e8,
            kill_bonus: 0.12,
            start_messages: Vec::new(),
            end_messages: Vec::new(),
            refused_messages: Vec::new(),
        });

    let mut fb = vec![0u32; W * H];
    let mut zb = vec![f32::INFINITY; W * H];
    let mut background = vec![0u32; W * H];
    let mut last_frame = std::time::Instant::now();

    screen.set_mouse_captured(true);
    loop {
        let now = std::time::Instant::now();
        let base_dt = frame_dt(now - last_frame);
        last_frame = now;
        screen.pump();
        let window: &dyn crate::frontend::Frontend = &*screen;
        if !window.is_open() || window.is_key_down(Key::Escape) {
            break;
        }
        // A cinematic level (the startup intro) runs on its own scripts and ends when they ask
        // for the graphic novel (`S_ModeSwitch`); Space skips it.
        if cinematic
            && (scene.mode_switch.is_some() || window.is_key_pressed(Key::Space, KeyRepeat::No))
        {
            break;
        }
        if motion.death_over() {
            level_end = LevelEnd::Died;
            break;
        }
        if window.is_key_pressed(Key::V, KeyRepeat::No) {
            third_person = !third_person;
        }
        let right_down = window.get_mouse_down(MouseButton::Right);
        let right_pressed = right_down && !right_was_down;
        right_was_down = right_down;
        let key = |key| i32::from(window.is_key_down(key) && scene.player_controls);
        let (forward_keys, right_keys) = (key(Key::W) - key(Key::S), key(Key::D) - key(Key::A));
        let held = forward_keys != 0 || right_keys != 0;
        // A direction turns the button into a dodge; a dive takes a weapon and gauge that allow it.
        let dodge = (right_pressed && held && !fly_mode && motion.can_dodge(grounded)).then(|| {
            let weapon = scene.inventory.current.as_ref();
            let allowed = weapon
                .and_then(|name| scene.inventory.defs.get(name))
                .is_some_and(|def| def.dive_allowed);
            if allowed && slow_motion.pay_for_dive() {
                crate::player::DodgeKind::Dive
            } else {
                crate::player::DodgeKind::Roll
            }
        });
        let pressed = (window.is_key_pressed(Key::B, KeyRepeat::No) || right_pressed)
            && dodge.is_none()
            && !motion.dodging();
        let was_slow = slow_motion.active;
        let asked = pressed && !bullet_time;
        bullet_time = slow_motion.switch(bullet_time, pressed, base_dt);
        for message in slow_motion.transition(was_slow, asked) {
            scene.dispatch_message("", message);
        }
        if let Some(message) = slow_motion.heartbeat(base_dt).map(str::to_string) {
            scene.dispatch_message("", &message);
        }
        // `[StartMessages]` set one speed for everything, Max included.
        let world_dt =
            base_dt * scene.game_speed.value * scene.dodge_speed.value * scene.global_speed.value;
        let player_dt = world_dt;
        let weapon = scene.inventory.current.as_ref();
        let weapon = weapon.and_then(|name| scene.inventory.defs.get(name));
        let rate = weapon.map_or(1.0, |def| def.slow_motion_rate);
        shot_timer = (shot_timer - cycle_seconds(base_dt, world_dt, rate)).max(0.0);

        if window.is_key_pressed(Key::F2, KeyRepeat::No) && !scene.jumppoints.is_empty() {
            jp_idx = (jp_idx + 1) % scene.jumppoints.len();
            player_pos = spawn_pos(&scene, &room_origins, jp_idx);
            yaw = scene.jumppoints[jp_idx].yaw();
            vel_y = 0.0;
            if let Some(jp) = scene.jumppoints.get(jp_idx) {
                println!(
                    "  3D Engine: Teleported to {} (room {})",
                    jp.name, jp.room_id
                );
            }
        }
        if window.is_key_pressed(Key::F3, KeyRepeat::No) {
            fly_mode = !fly_mode;
            vel_y = 0.0;
        }
        if scene.player_controls && window.is_key_pressed(Key::G, KeyRepeat::No) {
            scene.inventory.take_best_loaded();
        }
        let pill_key = window.is_key_pressed(Key::Tab, KeyRepeat::No)
            || window.is_key_pressed(Key::Backspace, KeyRepeat::No);
        let hurt = player_health
            .as_ref()
            .is_some_and(|max| max.health < max.max);
        // HYPOTHESIS: retail keeps the pills at full health.
        if pill_key
            && hurt
            && scene.player_controls
            && pill_messages.is_empty()
            && scene.inventory.take_painkiller()
        {
            let index = "CHARANIM_SHOOTPAINKILLER";
            let clip = scene.inventory.defs.get("painkiller");
            let clip = clip.and_then(|def| def.fire_clip.clone());
            let rate = clip
                .as_ref()
                .and_then(|clip| clip.bones.first())
                .map(|b| b.fps);
            pill_messages = player_clips
                .table
                .get(index)
                .map_or(Vec::new(), |def| def.messages.clone())
                .into_iter()
                .map(|(frame, text)| (frame as f32 / rate.unwrap_or(1.0).max(1.0), text))
                .collect();
            action_time = 0.0;
            action_index = "CHARANIM_SHOOT".to_string();
            if let Some(rig) = player_rig.as_mut() {
                rig.action = clip;
                rig.action_time = 0.0;
                rig.aim = None;
            }
        }
        if window.is_key_pressed(Key::L, KeyRepeat::No) {
            use_lightmaps = !use_lightmaps;
        }
        if window.is_key_pressed(Key::F1, KeyRepeat::No) {
            wireframe = !wireframe;
        }

        // Interact (`E`): the action button trigger in reach
        if scene.player_controls && window.is_key_pressed(Key::R, KeyRepeat::No) {
            scene.inventory.start_reload();
        }
        if scene.player_controls {
            // The Tutorial's tip names keys 1-6, and `SLOTID_*` runs 0 to 5.
            let keys = [
                Key::Key1,
                Key::Key2,
                Key::Key3,
                Key::Key4,
                Key::Key5,
                Key::Key6,
            ];
            for (slot, key) in (0..).zip(keys) {
                if window.is_key_pressed(key, KeyRepeat::No) {
                    scene.inventory.select_slot(slot);
                }
            }
            let wheel = window.scroll();
            if wheel != 0.0 {
                scene.inventory.step_weapon(wheel.signum() as i32);
            }
        }
        if scene.player_controls && window.is_key_pressed(Key::E, KeyRepeat::No) {
            let feet = [player_pos[0], player_pos[1] - EYE_HEIGHT, player_pos[2]];
            if let Some((name, gap)) = scene.nearest_action_trigger(&room_origins, feet) {
                println!("  3D Engine: use: {name} is {gap:.2} m out of reach");
            }
            if scene.activate_nearest_trigger(&room_origins, feet) {
                play_queued_sounds(
                    &mut scene,
                    &catalog_root(level_path),
                    &mut sounds,
                    &room_origins,
                    listener,
                );
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
        let hard_mouse = window.mouse_motion();
        if let Some(motion) = look_motion(hard_mouse, scene.player_controls) {
            (yaw, pitch) = mouse_look(yaw, pitch, motion);
        } else if hard_mouse.is_none() && window.get_mouse_down(MouseButton::Left) {
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
        let fly_speed = if window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift)
        {
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
        if !scene.player_controls {
            mv = [0.0; 3];
        }
        let controls = crate::player::Controls {
            forward: forward_keys,
            right: right_keys,
            jump: window.is_key_pressed(Key::Space, KeyRepeat::No) && scene.player_controls,
            crouch: window.is_key_down(Key::C) && scene.player_controls,
            dodge,
            two_handed: player_rig.as_ref().is_some_and(|rig| rig.two_handed),
            grounded,
            yaw,
            airborne_speed: player_clips.airborne_speed,
        };
        let stepped = if fly_mode {
            crate::player::MotionStep::default()
        } else {
            motion.step(&player_clips, &controls, player_dt)
        };
        if let Some(rig) = player_rig.as_mut() {
            rig.motion = stepped.clip.clone();
        }
        let mv_len = (mv[0] * mv[0] + mv[2] * mv[2]).sqrt();
        let player_moving = stepped.clip.is_some() || (fly_mode && mv_len > 0.01);
        walk_phase = match (fly_mode, player_moving) {
            (false, _) => motion.time(),
            (true, true) => move_clip_time(walk_phase, player_dt),
            (true, false) => 0.0,
        };
        anim_clock += player_dt;

        if fly_mode {
            if window.is_key_down(Key::Space) {
                mv[1] += 1.0;
            }
            if window.is_key_down(Key::C) {
                mv[1] -= 1.0;
            }
            player_pos[0] += mv[0] * fly_speed * player_dt;
            player_pos[1] += mv[1] * fly_speed * player_dt;
            player_pos[2] += mv[2] * fly_speed * player_dt;
        } else {
            let dx = stepped.velocity[0] * player_dt;
            let dz = stepped.velocity[1] * player_dt;
            let nx = player_pos[0] + dx;
            let nz = player_pos[2] + dz;
            let contact = dynamic_wall_hit(&scene, &room_origins, nx, player_pos[1], nz);
            if contact != last_contact {
                if let Some(index) = contact {
                    scene.character_collides(index);
                }
                last_contact = contact;
            }
            // Strict boundary & wall check: never allow stepping off walkable floor into void!
            if can_stand_at(&scene, &room_origins, nx, player_pos[1], nz) {
                player_pos[0] = nx;
                player_pos[2] = nz;
            } else if can_stand_at(&scene, &room_origins, nx, player_pos[1], player_pos[2]) {
                player_pos[0] = nx;
            } else if can_stand_at(&scene, &room_origins, player_pos[0], player_pos[1], nz) {
                player_pos[2] = nz;
            }

            let feet = [player_pos[0], player_pos[1] - EYE_HEIGHT, player_pos[2]];
            for text in &stepped.messages {
                use crate::player::ClipEffect;
                match crate::player::clip_effect(text) {
                    Some(ClipEffect::Jump(speed)) => vel_y = speed,
                    Some(ClipEffect::Sound { catalog, cue }) => {
                        queue_sound(&mut scene, &catalog, &cue, feet);
                    }
                    Some(ClipEffect::Projectile(name)) => {
                        let cue = dummy_sounds
                            .entry(name)
                            .or_insert_with_key(|name| crate::load_dummy_sound(&game_root, name));
                        if let Some(cue) = cue.clone() {
                            queue_sound(&mut scene, "characters", &cue, feet);
                        }
                    }
                    Some(ClipEffect::Reload) => scene.inventory.reload_now(),
                    Some(ClipEffect::BulletTimeOff) => bullet_time = false,
                    Some(ClipEffect::Script) => {
                        scene.dispatch_message("", text);
                    }
                    None => {}
                }
            }
            vel_y += particle_sprites.field.gravity[1] * player_dt;
            player_pos[1] += vel_y * player_dt;
            if let Some(floor_y) = find_floor_y(&scene, player_pos[0], player_pos[1], player_pos[2])
            {
                if player_pos[1] <= floor_y + EYE_HEIGHT {
                    player_pos[1] = floor_y + EYE_HEIGHT;
                    // 0x68dd1d. HYPOTHESIS: only the fall counts, not the speed along the ground.
                    if let (Some(max), Some(falling)) = (player_health.as_mut(), &falling) {
                        max.damage(falling.damage(-vel_y, max.max));
                    }
                    vel_y = 0.0;
                }
                grounded = vel_y <= 0.0 && player_pos[1] - EYE_HEIGHT - floor_y < STEP_UP;
            } else {
                // Clamp vertical fall if jumping over a gap
                vel_y = 0.0;
                grounded = true;
            }
        }

        if !cinematic {
            let feet = [player_pos[0], player_pos[1] - EYE_HEIGHT, player_pos[2]];
            scene.step_touch_triggers(&room_origins, feet);
        }
        scene.inventory.step(world_dt);
        // The last round's cycle runs out first, so its clip is not cut short.
        if scene.player_controls && shot_timer <= 0.0 && pending.is_empty() && !motion.diving() {
            scene.inventory.auto_reload();
        }

        // Shoot (left mouse, `F` or `LeftCtrl` / `RightCtrl`)
        // No weapon file marks a gun automatic, so held fire repeats for all of them.
        let fire_held = window.get_mouse_down(MouseButton::Left)
            || window.is_key_down(Key::F)
            || window.is_key_down(Key::LeftCtrl)
            || window.is_key_down(Key::RightCtrl);
        if fire_held && scene.player_controls && shot_timer <= 0.0 {
            let def = scene.inventory.current.as_ref();
            let def = def.and_then(|name| scene.inventory.defs.get(name)).cloned();
            if let Some(def) = def.filter(|_| scene.inventory.can_fire()) {
                // A new cycle restarts the clip, so rounds it did not reach are dropped.
                shot_timer = 1.0 / def.shots_per_second.max(0.1);
                let feet = [player_pos[0], player_pos[1] - EYE_HEIGHT, player_pos[2]];
                if let Some(cue) = &def.fire_sound {
                    queue_weapon_sound(&mut scene, cue, feet);
                }
                melee = def.melee_reach;
                firing = scene.inventory.current.clone().unwrap_or_default();
                pending = if melee.is_some() {
                    let blow = |at: &f32| crate::hud::ShotRound {
                        at: *at,
                        ..Default::default()
                    };
                    def.melee_blows.iter().map(blow).collect()
                } else if def.rounds.is_empty() {
                    vec![crate::hud::ShotRound::default()]
                } else {
                    def.rounds.clone()
                };
                if let Some(rig) = player_rig.as_mut() {
                    // A dive's clips aim the arms themselves.
                    rig.action = def.fire_clip.clone().filter(|_| !motion.diving());
                    action_time = 0.0;
                    rig.action_time = 0.0;
                    action_index = "CHARANIM_SHOOT".to_string();
                    rig.aim = def.aim_clips.clone();
                }
                if let Some(muzzle) = &def.muzzle {
                    scene.effect_queue.push(crate::scene::QueuedEffect {
                        effect: muzzle.clone(),
                        emitter: crate::scene::GUN_EMITTERS[0].to_string(),
                    });
                }
            } else {
                // Starts the reload, or takes the best loaded weapon.
                scene.inventory.fire();
            }
        }
        let due = pending
            .first()
            .copied()
            .filter(|round| action_time >= round.at);
        let mut fired = None;
        if let Some(round) = due {
            pending.remove(0);
            if scene.inventory.fire() {
                fired = Some(round);
            } else {
                pending.clear();
            }
        }
        if let Some(round) = fired {
            let def = scene.inventory.defs.get(&firing);
            if let Some(rig) = player_rig.as_mut() {
                rig.gun_clip = def
                    .and_then(|def| def.gun_clip.clone())
                    .map(|clip| (clip, 0.0));
            }
            let shot = def.map_or(Vec::new(), |def| {
                def.round_projectiles(round.quiet).to_vec()
            });
            let thrown = def.map_or(Vec::new(), |def| def.flying.clone());
            let muzzle = def
                .and_then(|def| def.muzzle.clone())
                .filter(|_| round.left);
            if let Some(muzzle) = muzzle {
                scene.effect_queue.push(crate::scene::QueuedEffect {
                    effect: muzzle,
                    emitter: crate::scene::GUN_EMITTERS[1].to_string(),
                });
            }
            let reach = melee;
            let aim_dir = [sy * cp, sp, cy * cp];
            for (flying, count) in &thrown {
                launch(
                    &mut flyers,
                    flying,
                    *count,
                    player_pos,
                    aim_dir,
                    &mut shot_seed,
                );
            }
            // The dot marks the view's centre, so a bullet leaves from the camera. A blow leaves
            // from Max.
            let (aim_from, aim) = if third_person && !fly_mode && reach.is_none() {
                let eye = chase_view(&scene, &room_origins, &chase_rig, player_pos, aim_dir);
                (eye, chase_look(&chase_rig, player_pos, aim_dir, eye).2)
            } else {
                (player_pos, aim_dir)
            };
            let within = |t: f32| reach.is_none_or(|reach| t <= reach);
            // The options' `AutoAim Toggle` (0x89aecd) has no engine setting yet: it stays on.
            let aim = def
                .filter(|def| def.auto_aim && reach.is_none())
                .and_then(|_| {
                    let wall = aim_hit_level(&scene, aim_from, aim).map_or(SHOT_RANGE, |hit| hit.0);
                    auto_aim_at(&enemies, aim_from, aim, auto_aim_scale, wall)
                })
                .unwrap_or(aim);
            let rays = shot
                .iter()
                .flat_map(|projectile| std::iter::repeat_n(projectile, projectile.count as usize));
            for projectile in rays {
                let aim_dir = spread_aim(aim, projectile.accuracy, &mut shot_seed);
                let block = projectile.decal.as_deref().unwrap_or_default();
                // Raycast against living enemies
                let mut best_hit: Option<(usize, f32)> = None;
                for (idx, en) in enemies.iter().enumerate() {
                    if en.health <= 0.0 || en.invulnerable || en.held {
                        continue;
                    }
                    if let Some(t) = ray_hits_character(aim_from, aim_dir, en.world_pos) {
                        if t <= 100.0 && within(t) && best_hit.is_none_or(|(_, b)| t < b) {
                            best_hit = Some((idx, t));
                        }
                    }
                }
                let level_hit =
                    aim_hit_level(&scene, aim_from, aim_dir).filter(|hit| within(hit.0));
                let mesh_hit = aim_hit_dynamic(&scene, &room_origins, aim_from, aim_dir)
                    .filter(|hit| within(hit.1))
                    .filter(|hit| level_hit.is_none_or(|level| hit.1 < level.0));
                let category = |scene: &LdbScene, id: i32| {
                    let material = scene.materials.get(&id);
                    material.map_or(String::new(), |m| m.category.clone())
                };
                let at = |t: f32| [0, 1, 2].map(|c| aim_from[c] + aim_dir[c] * t);
                match (mesh_hit, best_hit) {
                    (Some((midx, mt, normal, material)), enemy)
                        if enemy.is_none_or(|(_, et)| mt < et) =>
                    {
                        let kind = category(&scene, material);
                        decals.add(block, &kind, at(mt), normal, aim_dir, Some(midx));
                        if let Some(set) = decals.sets.get(block) {
                            let frame = crate::decals::surface_frame(normal, aim_dir);
                            let hit = at(mt);
                            hit_effects.start(
                                &mut scene,
                                &mut particle_sprites,
                                set,
                                &kind,
                                hit,
                                frame,
                            );
                        }
                        let scripted = scene.bullet_hits(midx);
                        println!(
                            "  3D Engine: shot hit {} at {mt:.1} m (scripted: {scripted})",
                            scene.dynamic_meshes[midx].name
                        );
                        let broke = scene.damage_mesh(midx, projectile.damage);
                        if broke {
                            println!("  3D Engine: {} broke", scene.dynamic_meshes[midx].name);
                        }
                        if scripted || broke {
                            play_queued_sounds(
                                &mut scene,
                                &catalog_root(level_path),
                                &mut sounds,
                                &room_origins,
                                listener,
                            );
                        }
                    }
                    (_, Some((idx, t))) => {
                        // `decals.txt` leaves "Character" out of the holes, so only the block's
                        // sound and effect run; `[CharacterHit]` throws the blood.
                        let hit = at(t);
                        if let Some(set) = decals.sets.get(block) {
                            let back = aim_dir.map(|c| -c);
                            let frame = crate::decals::surface_frame(back, aim_dir);
                            let sprites = &mut particle_sprites;
                            hit_effects.start(&mut scene, sprites, set, "character", hit, frame);
                        }
                        if let Some(splatter) = &projectile.character_hit {
                            let gravity = particle_sprites.field.gravity;
                            for _ in 0..splatter.count {
                                let dir = spread_aim(aim_dir, splatter.accuracy, &mut shot_seed);
                                droplets.push(crate::decals::Droplet {
                                    pos: hit,
                                    vel: dir.map(|c| c * splatter.speed),
                                    gravity: gravity.map(|c| c * splatter.gravity),
                                    decal: splatter.decal.clone().unwrap_or_default(),
                                    age: 0.0,
                                });
                            }
                        }
                        let en = &mut enemies[idx];
                        if hurt_enemy(en, projectile.damage, &mut scene, death_clips.len()) {
                            slow_motion.reward_kill();
                        }
                    }
                    _ => {
                        if let Some((t, normal, material)) = level_hit {
                            let kind = category(&scene, material);
                            decals.add(block, &kind, at(t), normal, aim_dir, None);
                            if let Some(set) = decals.sets.get(block) {
                                let frame = crate::decals::surface_frame(normal, aim_dir);
                                let hit = at(t);
                                hit_effects.start(
                                    &mut scene,
                                    &mut particle_sprites,
                                    set,
                                    &kind,
                                    hit,
                                    frame,
                                );
                            }
                        }
                    }
                }
            }
        }

        droplets.retain_mut(|drop| {
            let from = drop.pos;
            let (length, dir) = drop.step(world_dt);
            let delta = dir.map(|c| c * length);
            let Some((t, normal, material)) = segment_surface(&scene, from, delta) else {
                return drop.age < DROPLET_SECONDS;
            };
            let kind = scene.materials.get(&material);
            let kind = kind.map_or("", |material| material.category.as_str());
            let hit = [0, 1, 2].map(|c| from[c] + delta[c] * t);
            decals.add(&drop.decal, kind, hit, normal, dir, None);
            false
        });

        let gravity = particle_sprites.field.gravity;
        let mut landed = Vec::new();
        flyers.retain_mut(|(flyer, def)| {
            let flight = flyer.step(def, gravity, world_dt, |from, delta| {
                flight_contact(&scene, &enemies, from, delta)
            });
            match flight {
                Flight::Exploded(at) => {
                    landed.push((at, flyer.normal, def.clone()));
                    false
                }
                Flight::Bounced => {
                    if let Some(cue) = &def.bounce_sound {
                        queue_sound(&mut scene, "projectiles", cue, flyer.pos);
                    }
                    true
                }
                Flight::Flying => true,
            }
        });
        for (at, normal, def) in landed {
            let up = if normal == [0.0; 3] {
                [0.0, 1.0, 0.0]
            } else {
                normal
            };
            let kind = segment_surface(
                &scene,
                [0, 1, 2].map(|c| at[c] + up[c] * 0.05),
                up.map(|c| c * -0.1),
            )
            .and_then(|hit| scene.materials.get(&hit.2))
            .map_or(String::new(), |material| material.category.clone());
            let frame = crate::decals::surface_frame(up, up.map(|c| -c));
            let block = def.decal.as_deref().unwrap_or_default();
            if normal != [0.0; 3] {
                decals.add(block, &kind, at, normal, up.map(|c| -c), None);
            }
            if let Some(set) = decals.sets.get(block) {
                hit_effects.start(&mut scene, &mut particle_sprites, set, &kind, at, frame);
            }
            hit_effects.explode(&mut scene, &mut particle_sprites, &def, at, frame);
            if let Some(shake) = &def.shake {
                scene.dispatch_message("::x", &format!("player->cam_animateinplace({shake});"));
            }
            for (frag, count) in &def.frags {
                let from = [0, 1, 2].map(|c| at[c] + up[c] * 0.05);
                launch(&mut flyers, frag, *count, from, up, &mut shot_seed);
            }
            explosions.push(Explosion {
                pos: at,
                blasts: def.blasts.clone(),
                age: 0.0,
            });
        }
        let mut blasts = Vec::new();
        explosions.retain_mut(|explosion| {
            let pos = explosion.pos;
            blasts.extend(
                explosion
                    .step(world_dt)
                    .into_iter()
                    .map(|blast| (pos, blast)),
            );
            !explosion.blasts.is_empty()
        });
        for (pos, blast) in blasts {
            for en in enemies
                .iter_mut()
                .filter(|en| en.health > 0.0 && !en.invulnerable && !en.held)
            {
                let far =
                    crate::projectiles::distance_to_character(pos, en.world_pos, ENEMY_HEIGHT);
                let damage = blast.damage_at(far).round() as i32;
                if damage > 0 && hurt_enemy(en, damage, &mut scene, death_clips.len()) {
                    slow_motion.reward_kill();
                }
            }
            // The docs: "The radius must hit the DO's pivot to cause damage to it."
            for idx in 0..scene.dynamic_meshes.len() {
                let (_, pivot) = dynamic_world(&scene, idx, &room_origins);
                let gap = sub3(pivot, pos);
                let damage = blast.damage_at(dot3(gap, gap).sqrt()).round() as i32;
                if damage > 0 && scene.damage_mesh(idx, damage) {
                    println!("  3D Engine: {} broke", scene.dynamic_meshes[idx].name);
                }
            }
        }
        play_queued_sounds(
            &mut scene,
            &catalog_root(level_path),
            &mut sounds,
            &room_origins,
            listener,
        );

        let feet = [player_pos[0], player_pos[1] - EYE_HEIGHT, player_pos[2]];
        level_items.step(&mut scene, feet, world_dt);

        if let Some(rig) = player_rig.as_mut() {
            let weapon = scene.inventory.current.as_ref();
            let def = weapon.and_then(|name| scene.inventory.defs.get(name));
            rig.two_handed = def.is_some_and(|def| def.two_handed);
            rig.two_handed_bones = def.is_some_and(|def| def.two_handed_bones);
        }

        if scene.inventory.reload_clips != reload_seen {
            let weapon = scene.inventory.current.as_deref().unwrap_or_default();
            let index = format!("CHARANIM_RELOAD{}", weapon.to_ascii_uppercase());
            let clip = player_clips.table.get(&index);
            if clip.is_some_and(crate::player::stops_bullet_time) {
                bullet_time = false;
            }
            let def = scene.inventory.current.as_ref();
            if let Some(def) = def.and_then(|name| scene.inventory.defs.get(name)) {
                if let Some(rig) = player_rig.as_mut() {
                    rig.action = def.reload_clip.clone().filter(|_| !motion.diving());
                    action_time = 0.0;
                    action_index = "CHARANIM_RELOAD".to_string();
                    rig.aim = None;
                }
                reload_cue = def
                    .reload_sound
                    .clone()
                    .map(|cue| (def.reload_sound_delay, cue));
            }
        }
        reload_seen = scene.inventory.reload_clips;
        if let Some((delay, cue)) = &mut reload_cue {
            *delay -= world_dt;
            if *delay <= 0.0 {
                let feet = [player_pos[0], player_pos[1] - EYE_HEIGHT, player_pos[2]];
                let cue = cue.clone();
                queue_weapon_sound(&mut scene, &cue, feet);
                reload_cue = None;
            }
        }
        action_time += world_dt;
        let (due, later) = std::mem::take(&mut pill_messages)
            .into_iter()
            .partition::<Vec<_>, _>(|(at, _)| *at <= action_time);
        pill_messages = later;
        for (_, text) in due {
            use crate::player::ClipEffect;
            match crate::player::clip_effect(&text) {
                Some(ClipEffect::Sound { catalog, cue }) => {
                    let feet = [player_pos[0], player_pos[1] - EYE_HEIGHT, player_pos[2]];
                    queue_sound(&mut scene, &catalog, &cue, feet);
                }
                _ => {
                    scene.dispatch_message("", &text.replacen("this->", "player->", 1));
                }
            }
        }
        if let Some(rig) = player_rig.as_mut() {
            rig.action_time = action_time;
            rig.aim_pitch = pitch;
            if let Some((clip, time)) = rig.gun_clip.as_mut() {
                *time += world_dt;
                if *time >= clip_seconds(clip) {
                    rig.gun_clip = None;
                }
            }
            if rig
                .action
                .as_ref()
                .is_some_and(|clip| action_time >= clip_seconds(clip))
            {
                rig.action = None;
            }
            let stand = if rig.two_handed {
                "CHARANIM_2_STAND"
            } else {
                "CHARANIM_STAND"
            };
            let action = if rig.action.is_some() {
                action_index.as_str()
            } else {
                ""
            };
            rig.fade
                .advance(motion.index().unwrap_or(stand), action, player_dt);
            let time = player_clip_time(rig, player_moving, walk_phase, anim_clock, action_time);
            if let Some((_, pose)) = character_locals(rig, player_moving, time) {
                rig.fade.remember(pose);
            }
        }

        if let Some(level) = scene.slow_motion_level.take() {
            slow_motion.level = level.clamp(0.0, 1.0);
        }
        for (name, order) in std::mem::take(&mut scene.character_orders) {
            if name.eq_ignore_ascii_case("player") {
                if let Some(max) = player_health.as_mut() {
                    max.obey(&order);
                }
            } else if let Some(en) = enemies
                .iter_mut()
                .find(|en| en.name.eq_ignore_ascii_case(&name))
            {
                obey(en, &order, &scene, &room_origins);
            }
        }
        for en in &mut enemies {
            en.held = en.room_id != start_room && scene.room_sealed(en.room_id);
        }

        if let Some(max) = player_health.as_mut() {
            max.step(world_dt);
            if let Some(flash) = damage_flash.and_then(|flash| flash.of(last_health - max.health)) {
                scene.screen_flash = Some(flash);
            }
            last_health = max.health;
            if max.health <= 0.0 {
                motion.die(&player_clips, yaw);
                scene.player_controls = false;
                fly_mode = false;
                pending.clear();
            }
        }

        // Update Enemy AI (turn toward Max Payne and advance along floor when within 24m)
        let player_feet = [player_pos[0], player_pos[1] - EYE_HEIGHT, player_pos[2]];
        for en in &mut enemies {
            en.hit_flash = (en.hit_flash - base_dt).max(0.0);
            if let Some((clip, time, played)) = &mut en.death {
                let before = *time;
                *time += world_dt;
                let blood = death_clips.get(*clip).map_or(&[][..], |d| &d.blood);
                let due = blood.iter().filter(|(at, _)| (before..*time).contains(at));
                for (_, splatter) in due {
                    // HYPOTHESIS: the `torso` bone is taken as half the character's height.
                    let torso = [0.0, ENEMY_HEIGHT / 2.0, 0.0];
                    let pos = [0, 1, 2].map(|c| en.world_pos[c] + torso[c]);
                    let gravity = particle_sprites.field.gravity;
                    for _ in 0..splatter.count {
                        let dir = spread_aim([0.0, 1.0, 0.0], splatter.accuracy, &mut shot_seed);
                        droplets.push(crate::decals::Droplet {
                            pos,
                            vel: dir.map(|c| c * splatter.speed),
                            gravity: gravity.map(|c| c * splatter.gravity),
                            decal: splatter.decal.clone().unwrap_or_default(),
                            age: 0.0,
                        });
                    }
                }
                let sounds = death_clips.get(*clip).map_or(&[][..], |d| &d.sounds);
                while let Some((_, catalog, cue)) =
                    sounds.get(*played).filter(|sound| sound.0 <= *time)
                {
                    *played += 1;
                    queue_sound(&mut scene, catalog, cue, en.world_pos);
                }
            }
            if en.health <= 0.0 || !en.pursues || en.held {
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
                        en.walk_phase = move_clip_time(en.walk_phase, world_dt);
                    }
                } else {
                    en.walk_phase = 0.0;
                }
            }
        }

        scene.step_dynamic_anims(base_dt);
        scene.step_screen_fade(base_dt);
        scene.step_fog(base_dt);
        scene.step_presentation(base_dt);
        // Scripted sounds (sirens, the chopper) start and stop with the scripts.
        play_queued_sounds(
            &mut scene,
            &catalog_root(level_path),
            &mut sounds,
            &room_origins,
            listener,
        );
        if let Some(rig) = player_rig.as_ref() {
            let feet = [player_pos[0], player_pos[1] - EYE_HEIGHT, player_pos[2]];
            let time = player_clip_time(rig, player_moving, walk_phase, anim_clock, action_time);
            let pose = character_locals(rig, player_moving, time);
            scene.held_guns = [0, 1].map(|hand| {
                let (clip, locals) = pose.as_ref()?;
                let bone = &rig.gun_bones[hand];
                let (pos, rot) = bone_frame(clip, locals, bone, feet, player_mesh_yaw(yaw))?;
                Some((rot, pos))
            });
        }
        start_queued_effects(
            &mut scene,
            &game_root,
            &particles_txt,
            &mut particle_lights,
            &mut particle_sprites,
        );
        particle_lights.step(base_dt);
        particle_sprites.step(&scene, &room_origins, base_dt);
        let frame_lights: Vec<ScenePointLight> = world_lights
            .iter()
            .cloned()
            .chain(particle_lights.point_lights(&scene, &room_origins))
            .collect();
        let mut sprites = particle_sprites.sprites();
        sprites.extend(decals.sprites(
            |mesh| scene.dynamic_meshes.get(mesh).is_none_or(|dm| dm.hidden),
            |at| point_brightness(at, &frame_lights, scene.ambient),
        ));
        start_camera_shakes(
            &mut scene,
            &catalog_root(level_path),
            &camerapaths_txt,
            &mut camera_shake,
        );
        if let Some(shake) = camera_shake.as_mut() {
            shake.time += world_dt;
        }
        for (block, target) in std::mem::take(&mut scene.death_cams) {
            if let Some(anim) =
                load_camera_path(&catalog_root(level_path), &camerapaths_txt, &block)
            {
                death_cam = Some((CameraShake { anim, time: 0.0 }, target));
            }
        }
        if let Some((path, _)) = death_cam.as_mut() {
            path.time += world_dt;
        }
        if let Some(cue) = scene.music_request.take() {
            music.play_cue(&catalog_root(level_path), &cue);
        }
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let camera_point = if motion.diving() {
            chase_rig.dive_point
        } else if motion.crouching() {
            chase_rig.crouch_point
        } else {
            stand_point
        };
        chase_rig.point = approach(
            chase_rig.point,
            camera_point,
            chase_rig.point_speed * base_dt,
        );

        // Compute camera position (3rd-person over-the-shoulder vs 1st-person)
        let mut cam_right = [cy, 0.0, -sy];
        let mut cam_up = [-sy * sp, cp, -cy * sp];
        let mut cam_fwd = [sy * cp, sp, cy * cp];
        let mut cam_pos = player_pos;
        if third_person && !fly_mode {
            cam_pos = chase_view(&scene, &room_origins, &chase_rig, player_pos, cam_fwd);
            (cam_right, cam_up, cam_fwd) = chase_look(&chase_rig, player_pos, cam_fwd, cam_pos);
        }
        if let Some((pos, right, up, fwd)) = attached_view(&scene, &room_origins) {
            (cam_pos, cam_right, cam_up, cam_fwd) = (pos, right, up, fwd);
        } else if let Some(view) = death_cam.as_ref().and_then(|(path, target)| {
            let pose = path.pose()?;
            let en = enemies
                .iter()
                .find(|en| en.name.eq_ignore_ascii_case(target))?;
            Some(parented_view(en.world_pos, en.yaw, pose))
        }) {
            (cam_pos, cam_right, cam_up, cam_fwd) = view;
        } else if let Some(delta) = camera_shake.as_ref().and_then(CameraShake::delta) {
            (cam_pos, cam_right, cam_up, cam_fwd) =
                shaken_view((cam_pos, cam_right, cam_up, cam_fwd), delta);
        }
        listener = cam_pos;

        let fov_scale = fov_scale(scene.fov_multiplier.value);
        // World sphere (house = sunset) or the flat ceiling when the level names none.
        // The hardware renderer draws the sky itself unless it shows the CPU frame (wireframe).
        let cpu_frame = gpu.is_none() || wireframe;
        if !cpu_frame {
        } else if let Some(ref tex) = sky {
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
        let mut characters = Vec::new();
        let mut scene_view = None;
        if gpu.is_some() && !wireframe {
            for en in enemies.iter().filter(|en| !en.held) {
                if let Some(erig) = bodies.get(en.rig_index) {
                    let moving = en.walk_phase > 0.0;
                    let time = if moving { en.walk_phase } else { anim_clock };
                    let dying = en
                        .death
                        .and_then(|(clip, time, _)| Some((death_clips.get(clip)?, time)));
                    let verts = match dying {
                        Some((death, time)) => {
                            dying_world_verts(erig, death, en.world_pos, en.yaw, time)
                        }
                        None => character_world_verts(
                            erig,
                            en.world_pos,
                            en.yaw,
                            time,
                            moving,
                            en.health <= 0.0,
                        ),
                    };
                    let light = character_vertex_light(
                        &erig.mesh,
                        &verts,
                        en.world_pos,
                        en.hit_flash > 0.0,
                        &frame_lights,
                        scene.ambient,
                    );
                    characters.push(crate::gpu::GpuCharacter {
                        mesh: &erig.mesh,
                        verts,
                        two_sided: en.health <= 0.0 && dying.is_none(),
                        light,
                    });
                }
            }
            let items = placed_items(&level_items, &item_models);
            for (mesh, verts, at) in items
                .into_iter()
                .chain(placed_flyers(&flyers, &flyer_models))
            {
                let light =
                    character_vertex_light(mesh, &verts, at, false, &frame_lights, scene.ambient);
                characters.push(crate::gpu::GpuCharacter {
                    mesh,
                    verts,
                    two_sided: false,
                    light,
                });
            }
            if let (true, false, Some(prig)) = (third_person, fly_mode, player_rig.as_ref()) {
                let time =
                    player_clip_time(prig, player_moving, walk_phase, anim_clock, action_time);
                let verts = character_world_verts(
                    prig,
                    player_feet,
                    player_mesh_yaw(yaw),
                    time,
                    player_moving,
                    false,
                );
                let light = character_vertex_light(
                    &prig.mesh,
                    &verts,
                    player_feet,
                    false,
                    &frame_lights,
                    scene.ambient,
                );
                characters.push(crate::gpu::GpuCharacter {
                    mesh: &prig.mesh,
                    verts,
                    two_sided: false,
                    light,
                });
                let mesh_yaw = player_mesh_yaw(yaw);
                for (weapon, verts) in held_guns(
                    &scene,
                    &weapon_models,
                    prig,
                    player_feet,
                    mesh_yaw,
                    time,
                    player_moving,
                ) {
                    let light = character_vertex_light(
                        weapon,
                        &verts,
                        player_feet,
                        false,
                        &frame_lights,
                        scene.ambient,
                    );
                    characters.push(crate::gpu::GpuCharacter {
                        mesh: weapon,
                        verts,
                        // Held weapons draw both sides, as `character_face_normal` says.
                        two_sided: true,
                        light,
                    });
                }
            }
            scene_view = Some((cam_pos, cam_right, cam_up, cam_fwd));
        } else {
            let mut tris = Vec::new();
            for sm in static_meshes_in_view(&scene, scene.attached_camera) {
                queue_poly_batch(
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
                    &mut tris,
                );
            }
            flush_tris(&tris, &mut fb, &mut zb, raster_threads());
        }

        // 2. Render dynamic meshes (doors, breakables, furniture, vans, cameras); the hardware
        // renderer already drew them in step 1.
        if gpu.is_none() || wireframe {
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
        }

        // 3. Render each character with the skin the level named (the hardware renderer drew
        // them in step 1).
        let cpu_characters = gpu.is_none() || wireframe;
        for en in enemies.iter().filter(|en| cpu_characters && !en.held) {
            if let Some(erig) = bodies.get(en.rig_index) {
                let moving = en.walk_phase > 0.0;
                let time = if moving { en.walk_phase } else { anim_clock };
                let dying = en
                    .death
                    .and_then(|(clip, time, _)| Some((death_clips.get(clip)?, time)));
                let dead = en.health <= 0.0 && dying.is_none();
                let verts = match dying {
                    Some((death, time)) => {
                        dying_world_verts(erig, death, en.world_pos, en.yaw, time)
                    }
                    None => character_world_verts(erig, en.world_pos, en.yaw, time, moving, dead),
                };
                render_kfs_character(
                    &erig.mesh,
                    &verts,
                    en.world_pos,
                    dead,
                    en.hit_flash > 0.0,
                    &frame_lights,
                    scene.ambient,
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

        for (mesh, verts, at) in placed_items(&level_items, &item_models)
            .into_iter()
            .chain(placed_flyers(&flyers, &flyer_models))
            .filter(|_| cpu_characters)
        {
            render_kfs_character(
                mesh,
                &verts,
                at,
                false,
                false,
                &frame_lights,
                scene.ambient,
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

        // 4. Render Player (`Payne_Max_L0.KFS`) in 3rd-person view
        if third_person && !fly_mode && cpu_characters {
            if let Some(ref prig) = player_rig {
                let time =
                    player_clip_time(prig, player_moving, walk_phase, anim_clock, action_time);
                let mesh_yaw = player_mesh_yaw(yaw);
                let verts =
                    character_world_verts(prig, player_feet, mesh_yaw, time, player_moving, false);
                let guns = held_guns(
                    &scene,
                    &weapon_models,
                    prig,
                    player_feet,
                    mesh_yaw,
                    time,
                    player_moving,
                );
                let body = std::iter::once((&prig.mesh, verts, false));
                let guns = guns.into_iter().map(|(mesh, verts)| (mesh, verts, true));
                for (mesh, verts, two_sided) in body.chain(guns) {
                    render_kfs_character(
                        mesh,
                        &verts,
                        player_feet,
                        two_sided,
                        false,
                        &frame_lights,
                        scene.ambient,
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
        }

        if gpu.is_none() && !wireframe {
            apply_fog(&mut fb, &zb, scene.fog);
            draw_sprites(
                &mut fb,
                &mut zb,
                (cam_pos, cam_right, cam_up, cam_fwd),
                fov_scale,
                scene.fog,
                &sprites,
                |key| {
                    particle_sprites
                        .texture(key)
                        .or_else(|| decals.texture(key))
                },
            );
        }

        // Hardware: everything drawn so far (sky, wireframe geometry) is the background; the HUD
        // below goes to a transparent overlay above the GPU scene.
        if gpu.is_some() {
            background.copy_from_slice(&fb);
            fb.fill(crate::gpu::HUD_CLEAR);
        }
        draw_letterbox(&mut fb, scene.widescreen.value);
        draw_screen_flash(&scene, &mut fb);

        if let Some(assets) = hud_assets.as_ref() {
            let layout = &assets.layout;
            for (tip, id) in scene.hud_prints.drain(..) {
                let text = hud_strings.get(&id).unwrap_or(&id);
                let text = crate::hud::expand_keys(text);
                let (log, style) = if tip {
                    (&mut tip_log, &layout.tip)
                } else {
                    (&mut print_log, &layout.print)
                };
                for line in crate::hud::wrap_text(&assets.font, &text, 620.0) {
                    log.push(style, line);
                }
            }
            tip_log.step(&layout.tip, base_dt);
            print_log.step(&layout.print, base_dt);
        }
        if let Some(assets) = hud_assets.as_ref().filter(|_| crosshair_visible(&scene)) {
            let mut target = crate::hud::HudTarget {
                pixels: &mut fb,
                width: W,
                height: H,
            };
            if scene.inventory.current != strip_weapon {
                strip_weapon = scene.inventory.current.clone();
                strip_age = 0.0;
            }
            strip_age += base_dt;
            let strip_visible = assets.layout.weapon_strip.visibility(strip_age);
            let state = crate::hud::HudState {
                health: player_health
                    .as_ref()
                    .map_or(1.0, crate::health::PlayerHealth::fraction),
                slow_motion: &slow_motion,
                inventory: &scene.inventory,
                weapon_strip: strip_visible,
            };
            target.draw_hud(assets, &state);
            target.draw_crosshair(&scene.inventory);
            target.draw_prints(&assets.font, &assets.layout.tip, &tip_log);
            target.draw_prints(&assets.font, &assets.layout.print, &print_log);
        }

        if let (Some((renderer, _)), Some(view)) = (gpu.as_mut(), scene_view) {
            renderer.set_sprites(&sprites, view, |key| {
                particle_sprites
                    .texture(key)
                    .or_else(|| decals.texture(key))
            });
        }
        let still = capture.as_mut().and_then(|c| c.due(base_dt));
        let moving = video.as_mut().is_some_and(|v| v.pace.due(base_dt));
        if still.is_some() || moving {
            let mut frame = vec![0u32; W * H];
            match (gpu.as_mut(), scene_view) {
                (Some((renderer, _)), Some(view)) => {
                    let mut depth = vec![f32::INFINITY; W * H];
                    let lit = use_lightmaps;
                    renderer.render(&scene, view, lit, &characters, &mut frame, &mut depth);
                    composite_hud(&mut frame, &fb);
                }
                _ => frame.copy_from_slice(&fb),
            }
            if let Some(video) = video.as_ref().filter(|_| moving) {
                video.push(&frame);
            }
            if let Some(path) = still {
                if let Err(error) = write_ppm(&path, W, H, &frame) {
                    eprintln!("  3D Engine: capture failed: {error}");
                }
            }
        }
        match gpu.as_mut() {
            Some((renderer, surface)) => renderer.present(
                surface,
                screen.size(),
                &scene,
                scene_view,
                use_lightmaps,
                &characters,
                wireframe.then_some(background.as_slice()),
                &fb,
            ),
            None => {
                if let Some(fade) = scene.screen_fade {
                    blend_screen(&mut fb, fade.color());
                }
                screen.show(&fb)?
            }
        }
    }
    screen.set_mouse_captured(false);
    screen.put_gpu(gpu);
    Ok(level_end)
}

/// Saves a frame every few seconds into `OPENMAXFX_CAPTURE_DIR`, for comparing with retail. It is
/// the engine's own image, so nothing else on the desktop can end up in it.
struct FrameCapture {
    dir: PathBuf,
    level: String,
    every: f32,
    wait: f32,
    count: u32,
}

impl FrameCapture {
    fn new(dir: PathBuf, level: &str) -> Self {
        let level = level.chars().filter(char::is_ascii_alphanumeric).collect();
        Self {
            dir,
            level,
            every: 2.0,
            wait: 0.0,
            count: 0,
        }
    }

    /// The next file to write, when one is due.
    fn due(&mut self, seconds: f32) -> Option<PathBuf> {
        self.wait -= seconds;
        if self.wait > 0.0 {
            return None;
        }
        self.wait = self.every;
        self.count += 1;
        let name = format!("{}_{:04}.ppm", self.level, self.count);
        Some(self.dir.join(name))
    }
}

/// Seconds between video frames: 30 a second plays back smoothly.
const VIDEO_INTERVAL: f32 = 1.0 / 30.0;

/// Says when the next video frame is due.
#[derive(Debug, Default)]
struct VideoPace {
    wait: f32,
}

impl VideoPace {
    fn due(&mut self, seconds: f32) -> bool {
        self.wait -= seconds;
        if self.wait > 0.0 {
            return false;
        }
        self.wait = VIDEO_INTERVAL;
        true
    }
}

/// Frames piped to `ffmpeg`. A thread does the writing and a full queue drops frames, so a slow
/// encoder cannot change the timing being recorded.
struct VideoCapture {
    pace: VideoPace,
    frames: Option<std::sync::mpsc::SyncSender<Vec<u8>>>,
    writer: Option<std::thread::JoinHandle<()>>,
    encoder: Child,
}

impl VideoCapture {
    fn start(path: &Path) -> std::io::Result<Self> {
        let mut encoder = std::process::Command::new("ffmpeg")
            .args(video_arguments(path, W, H))
            .stdin(std::process::Stdio::piped())
            .spawn()?;
        let mut stdin = encoder.stdin.take().ok_or(std::io::ErrorKind::BrokenPipe)?;
        let (frames, queue) = std::sync::mpsc::sync_channel::<Vec<u8>>(8);
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            while let Ok(frame) = queue.recv() {
                if stdin.write_all(&frame).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            pace: VideoPace::default(),
            frames: Some(frames),
            writer: Some(writer),
            encoder,
        })
    }

    fn push(&self, frame: &[u32]) {
        if let Some(frames) = &self.frames {
            let _ = frames.try_send(video_bytes(frame));
        }
    }
}

impl Drop for VideoCapture {
    /// Closing the pipe tells `ffmpeg` to finish the file.
    fn drop(&mut self) {
        self.frames = None;
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        let _ = self.encoder.wait();
    }
}

/// `ffmpeg` arguments reading raw frames from stdin. Frames are stamped by wall clock, so playback
/// keeps real speed whatever rate the engine reaches, and a fragmented file survives a kill.
fn video_arguments(path: &Path, width: usize, height: usize) -> Vec<String> {
    let size = format!("{width}x{height}");
    let args = [
        "-y",
        "-loglevel",
        "error",
        "-use_wallclock_as_timestamps",
        "1",
        "-f",
        "rawvideo",
        "-pixel_format",
        "bgr0",
        "-video_size",
        &size,
        "-i",
        "pipe:0",
        "-fps_mode",
        "vfr",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-crf",
        "18",
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+frag_keyframe+empty_moov",
    ];
    let mut args: Vec<String> = args.map(String::from).to_vec();
    args.push(path.display().to_string());
    args
}

/// `frame` as the `bgr0` bytes `ffmpeg` reads.
fn video_bytes(frame: &[u32]) -> Vec<u8> {
    frame.iter().flat_map(|pixel| pixel.to_le_bytes()).collect()
}

/// `hud` over `frame`: the top byte of a HUD pixel is its transparency.
fn composite_hud(frame: &mut [u32], hud: &[u32]) {
    for (pixel, over) in frame.iter_mut().zip(hud) {
        *pixel = crate::hud::blend(*pixel, *over & 0x00ff_ffff, 255 - (*over >> 24));
    }
}

/// Rows of each widescreen bar: an eighth of the frame height when fully in (retail 1920x1080
/// cutscene frames are black on rows 0..135 and 945..1080), scaled while it slides in.
fn letterbox_rows(fraction: f32) -> usize {
    ((H / 8) as f32 * fraction.clamp(0.0, 1.0)).round() as usize
}

/// Black widescreen bars over the top and bottom of the frame (`GM_EnableWidescreen`).
fn draw_letterbox(fb: &mut [u32], fraction: f32) {
    let rows = letterbox_rows(fraction);
    fb[..rows * W].fill(0);
    fb[(H - rows) * W..].fill(0);
}

/// `globalconfig.txt` `[camera]` `DefaultFOV`, in degrees across retail's 4:3 frame. A wider window
/// keeps retail's height and sees more at the sides.
const DEFAULT_FOV: f32 = 70.0;

/// Projection scale for a `gm_changeFOV` multiplier, which scales the view angle (the game's
/// error text: "multiplier makes fov out of range"). A non-positive one keeps the default.
pub(crate) fn fov_scale(multiplier: f32) -> f32 {
    let multiplier = if multiplier > 0.0 { multiplier } else { 1.0 };
    let half = (DEFAULT_FOV * 0.5).to_radians() * multiplier;
    H as f32 * 4.0 / 3.0 * 0.5 / half.min(1.5).tan()
}

/// Radians per mouse count of free look.
const MOUSE_LOOK: f32 = 0.003;

/// The mouse's move this frame, when the player has the controls. `GM_SetPlayerControls` takes them
/// away for cinematics, and a mouse moved then must not turn Max.
fn look_motion(motion: Option<(f32, f32)>, controls: bool) -> Option<(f32, f32)> {
    motion.filter(|_| controls)
}

/// `(yaw, pitch)` after a mouse move: right turns right, up looks up.
fn mouse_look(yaw: f32, pitch: f32, (dx, dy): (f32, f32)) -> (f32, f32) {
    (
        yaw + dx * MOUSE_LOOK,
        (pitch - dy * MOUSE_LOOK).clamp(-1.25, 1.25),
    )
}

/// Longest step one frame may take (a loading stall or a window drag).
const MAX_FRAME_DT: f32 = 0.1;

/// Game time for one frame: the wall time since the previous frame, capped at `MAX_FRAME_DT`.
fn frame_dt(elapsed: std::time::Duration) -> f32 {
    elapsed.as_secs_f32().min(MAX_FRAME_DT)
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

/// Highest ledge the feet ride onto. Retail moves a capsule with a round bottom and has no step
/// value; HYPOTHESIS: its radius, `max_payne.txt` `CapsuleRadius`.
const STEP_UP: f32 = 0.48;

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
                    if y <= feet_y + STEP_UP && y >= feet_y - 6.0 && best.is_none_or(|b| y > b) {
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
    dynamic_wall_hit(scene, rooms, wx, wy, wz).is_some()
}

/// The dynamic mesh a body at this spot runs into.
fn dynamic_wall_hit(
    scene: &LdbScene,
    rooms: &BTreeMap<i32, [f32; 3]>,
    wx: f32,
    wy: f32,
    wz: f32,
) -> Option<usize> {
    let body_min_y = wy - EYE_HEIGHT + 0.35;
    let body_max_y = wy - 0.10;
    let radius = 0.38f32;
    for (idx, dm) in scene.dynamic_meshes.iter().enumerate() {
        if dm.hidden
            || !dm.collisions
            || dm.name.contains("_Tip_")
            || dm.name.ends_with("Start_FSM.DO")
        {
            continue;
        }
        let (xform, origin) = dynamic_world(scene, idx, rooms);
        for poly in &dm.polys {
            if poly.vertices.len() < 3 || !blocks_character(scene, poly.material_id) {
                continue;
            }
            let n = xform_dir(xform, poly.normal);
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
            if n[1].abs() > 0.45 {
                // A floor or lid the feet cannot ride onto, like a van's load bed.
                let step = wy - EYE_HEIGHT + STEP_UP;
                if max_y > step && min_y < body_max_y && xz_gap(&world, wx, wz) < radius {
                    return Some(idx);
                }
                continue;
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
                return Some(idx);
            }
        }
    }
    None
}

/// How far `(x, z)` is from a convex polygon seen from above: 0 over it.
fn xz_gap(poly: &[[f32; 3]], x: f32, z: f32) -> f32 {
    let over = (1..poly.len() - 1).any(|k| point_in_xz_tri(x, z, poly[0], poly[k], poly[k + 1]));
    if over {
        return 0.0;
    }
    let edge = |(a, b): (&[f32; 3], &[f32; 3])| {
        let (ex, ez) = (b[0] - a[0], b[2] - a[2]);
        let along = ((x - a[0]) * ex + (z - a[2]) * ez) / (ex * ex + ez * ez).max(1e-9);
        let along = along.clamp(0.0, 1.0);
        (a[0] + ex * along - x).hypot(a[2] + ez * along - z)
    };
    let next = poly.iter().cycle().skip(1);
    poly.iter()
        .zip(next)
        .map(edge)
        .fold(f32::INFINITY, f32::min)
}

pub(crate) fn poly_policy(scene: &LdbScene, material_id: i32) -> MaterialPolicy {
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

/// Clock of a character's move clip after `dt` seconds of moving. Clips carry their own frame
/// rate (`Kf2Anim` bones), so the clock is plain game time.
fn move_clip_time(time: f32, dt: f32) -> f32 {
    time + dt
}

/// The player camera's offsets, each `(right, up, forward)` in the player's yaw frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ChaseRig {
    /// `DefaultCameraPointOffset` (skeleton), from the feet: where the camera is cast from.
    point: [f32; 3],
    /// `CrouchingCameraPointOffset`.
    crouch_point: [f32; 3],
    /// `ShootDodgingCameraPointOffset`.
    dive_point: [f32; 3],
    /// `InitialBackwardsSpeed`: metres per second the camera moves between those points.
    point_speed: f32,
    /// `DefaultAimingPointOffset`, from the feet: where aiming starts.
    aim_point: [f32; 3],
    /// `LookAtMinDistance`: how far along the aim the camera looks.
    look_at: f32,
    /// `globalconfig.txt` `[camera]` `UpFollowDistance`: the camera above the player.
    up: [f32; 3],
    /// `MiddleFollowDistance`: the camera behind the player.
    middle: [f32; 3],
    /// `DownFollowDistance`: the camera below the player.
    down: [f32; 3],
}

impl ChaseRig {
    fn parse(globalconfig: &str, skeleton: &str) -> Option<Self> {
        Some(Self {
            point: config_vector(skeleton, "DefaultCameraPointOffset")?,
            crouch_point: config_vector(skeleton, "CrouchingCameraPointOffset")?,
            dive_point: config_vector(skeleton, "ShootDodgingCameraPointOffset")?,
            point_speed: config_number(globalconfig, "InitialBackwardsSpeed")?,
            aim_point: config_vector(skeleton, "DefaultAimingPointOffset")?,
            look_at: config_number(globalconfig, "LookAtMinDistance")?,
            up: config_vector(globalconfig, "UpFollowDistance")?,
            middle: config_vector(globalconfig, "MiddleFollowDistance")?,
            down: config_vector(globalconfig, "DownFollowDistance")?,
        })
    }
}

/// `key = n;` in a retail text file.
fn config_number(text: &str, key: &str) -> Option<f32> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with(key))?;
    line.split_once('=')?
        .1
        .split(';')
        .next()?
        .trim()
        .parse()
        .ok()
}

/// `from` moved up to `step` metres toward `to`.
fn approach(from: [f32; 3], to: [f32; 3], step: f32) -> [f32; 3] {
    let gap: [f32; 3] = std::array::from_fn(|c| to[c] - from[c]);
    let length = dot3(gap, gap).sqrt();
    if length <= step {
        return to;
    }
    std::array::from_fn(|c| from[c] + gap[c] * step / length)
}

/// `key = ( x, y, z );` in a retail text file.
fn config_vector(text: &str, key: &str) -> Option<[f32; 3]> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with(key))?;
    let body = line.split_once('(')?.1.split_once(')')?.0;
    let mut parts = body.split(',').map(|part| part.trim().parse::<f32>().ok());
    Some([parts.next()??, parts.next()??, parts.next()??])
}

/// The retail values, for data that does not load.
const RETAIL_RIG: ChaseRig = ChaseRig {
    point: [0.12, 1.75, -0.12],
    crouch_point: [0.12, 1.15, -0.12],
    dive_point: [0.12, 0.90, -0.12],
    point_speed: 5.0,
    aim_point: [0.12, 1.50, 0.0],
    look_at: 5.0,
    up: [0.06, 0.40, -0.40],
    middle: [0.06, 0.5, -4.00],
    down: [0.06, 0.80, -1.00],
};

/// `offset` from `base` in the yaw frame of `cam_fwd`.
fn yaw_offset(base: [f32; 3], cam_fwd: [f32; 3], offset: [f32; 3]) -> [f32; 3] {
    let ahead = normalize([cam_fwd[0], 0.0, cam_fwd[2]]);
    let right = [ahead[2], 0.0, -ahead[0]];
    [
        base[0] + right[0] * offset[0] + ahead[0] * offset[2],
        base[1] + offset[1],
        base[2] + right[2] * offset[0] + ahead[2] * offset[2],
    ]
}

/// The camera point the follow ray starts from.
fn chase_start(rig: &ChaseRig, player: [f32; 3], cam_fwd: [f32; 3]) -> [f32; 3] {
    let feet = [player[0], player[1] - EYE_HEIGHT, player[2]];
    yaw_offset(feet, cam_fwd, rig.point)
}

/// The follow position: an orbit around the camera point. The offset turns with the view's
/// pitch, so the player stays in frame; `Up` and `Down` take over toward looking straight down
/// and straight up. HYPOTHESIS for the pitched frame and the linear blend: the distances'
/// comments name the positions and the update applies a matrix, but the blend is not read.
fn chase_eye(rig: &ChaseRig, player: [f32; 3], cam_fwd: [f32; 3]) -> [f32; 3] {
    let steep = cam_fwd[1].clamp(-1.0, 1.0).abs().asin() / std::f32::consts::FRAC_PI_2;
    let end = if cam_fwd[1] < 0.0 { rig.up } else { rig.down };
    let follow = [0, 1, 2].map(|c| rig.middle[c] + (end[c] - rig.middle[c]) * steep);
    let ahead = normalize([cam_fwd[0], 0.0, cam_fwd[2]]);
    let right = [ahead[2], 0.0, -ahead[0]];
    let up = cross3(cam_fwd, right);
    let start = chase_start(rig, player, cam_fwd);
    [0, 1, 2].map(|c| start[c] + right[c] * follow[0] + up[c] * follow[1] + cam_fwd[c] * follow[2])
}

/// Right, up and forward of a chase camera at `eye`, for a character aiming `aim`. It does not
/// look along the aim but at the point `LookAtMinDistance` along it from the aiming point, so the
/// view tips toward the character and the crosshair sits just above the head.
fn chase_look(
    rig: &ChaseRig,
    player: [f32; 3],
    aim: [f32; 3],
    eye: [f32; 3],
) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let feet = [player[0], player[1] - EYE_HEIGHT, player[2]];
    let start = yaw_offset(feet, aim, rig.aim_point);
    let target = [0, 1, 2].map(|c| start[c] + aim[c] * rig.look_at);
    let fwd = normalize(sub3(target, eye));
    let ahead = normalize([aim[0], 0.0, aim[2]]);
    let right = normalize(cross3([0.0, 1.0, 0.0], fwd));
    let right = if dot3(right, right) > 0.5 {
        right
    } else {
        [ahead[2], 0.0, -ahead[0]]
    };
    (right, cross3(fwd, right), fwd)
}

/// How far the chase camera stays off what it hits: `globalconfig.txt` `[camera]`
/// `SphereRadius` (0.1) plus the near plane, so the wall itself is not clipped.
const CAMERA_CLEARANCE: f32 = 0.1 + NEAR_Z;

/// Third-person eye: cast from the player toward the follow position and stop before the first
/// level polygon in the way (retail casts the camera "from its startpoint" toward the follow
/// distance). `CameraCollision` polygons count: they exist for this.
fn chase_camera(scene: &LdbScene, rig: &ChaseRig, player: [f32; 3], cam_fwd: [f32; 3]) -> [f32; 3] {
    let start = chase_start(rig, player, cam_fwd);
    let want = chase_eye(rig, player, cam_fwd);
    let d = sub3(want, start);
    let len = dot3(d, d).sqrt();
    let Some(t) = segment_hit(scene, start, d) else {
        return keep_clear(scene, want);
    };
    let k = (t * len - CAMERA_CLEARANCE).max(0.0) / len;
    keep_clear(scene, [0, 1, 2].map(|c| start[c] + d[c] * k))
}

/// `chase_camera`, held short of the moving objects too.
fn chase_view(
    scene: &LdbScene,
    rooms: &BTreeMap<i32, [f32; 3]>,
    rig: &ChaseRig,
    player: [f32; 3],
    cam_fwd: [f32; 3],
) -> [f32; 3] {
    let start = chase_start(rig, player, cam_fwd);
    let eye = chase_camera(scene, rig, player, cam_fwd);
    let d = sub3(eye, start);
    let len = dot3(d, d).sqrt();
    let Some(t) = dynamic_segment_hit(scene, rooms, start, d) else {
        return eye;
    };
    let k = (t * len - CAMERA_CLEARANCE).max(0.0) / len;
    [0, 1, 2].map(|c| start[c] + d[c] * k)
}

/// First visible moving object along the segment, as its `t` in 0..1.
fn dynamic_segment_hit(
    scene: &LdbScene,
    rooms: &BTreeMap<i32, [f32; 3]>,
    from: [f32; 3],
    d: [f32; 3],
) -> Option<f32> {
    let mut best: Option<f32> = None;
    for (idx, dm) in scene.dynamic_meshes.iter().enumerate() {
        if dm.hidden || !dm.collisions {
            continue;
        }
        let (xform, origin) = dynamic_world(scene, idx, rooms);
        for poly in dm
            .polys
            .iter()
            .filter(|p| p.vertices.len() >= 3 && blocks_camera(scene, p.material_id))
        {
            let n = xform_dir(xform, poly.normal);
            let denom = dot3(n, d);
            if denom.abs() < 1e-6 {
                continue;
            }
            let world: Vec<[f32; 3]> = poly
                .vertices
                .iter()
                .map(|v| xform_point(xform, origin, v.pos))
                .collect();
            let t = dot3(n, sub3(world[0], from)) / denom;
            if !(0.0..=1.0).contains(&t) || best.is_some_and(|b| t >= b) {
                continue;
            }
            if inside_convex(&world, n, [0, 1, 2].map(|c| from[c] + d[c] * t)) {
                best = Some(t);
            }
        }
    }
    best
}

/// Push the eye out to `CAMERA_CLEARANCE` from every camera-blocking polygon it sits over: the
/// retail camera is a sphere (`SphereRadius`), so walls alongside the follow ray hold it off too.
fn keep_clear(scene: &LdbScene, mut eye: [f32; 3]) -> [f32; 3] {
    for sm in &scene.static_meshes {
        let o = sm.world_origin;
        for poly in sm
            .polys
            .iter()
            .filter(|p| p.vertices.len() >= 3 && blocks_camera(scene, p.material_id))
        {
            let n = poly.normal;
            let world = poly_world(&poly.vertices, o);
            let dist = dot3(n, sub3(eye, world[0]));
            if dist.abs() >= CAMERA_CLEARANCE {
                continue;
            }
            let foot = [0, 1, 2].map(|k| eye[k] - n[k] * dist);
            if !inside_convex(&world, n, foot) {
                continue;
            }
            let push = CAMERA_CLEARANCE.copysign(dist) - dist;
            eye = [0, 1, 2].map(|k| eye[k] + n[k] * push);
        }
    }
    eye
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn poly_world(vertices: &[SceneVertex], o: [f32; 3]) -> Vec<[f32; 3]> {
    vertices
        .iter()
        .map(|v| [v.pos[0] + o[0], v.pos[1] + o[1], v.pos[2] + o[2]])
        .collect()
}

/// `p` (on the polygon's plane) is on the same side of every edge of the convex polygon.
fn inside_convex(world: &[[f32; 3]], n: [f32; 3], p: [f32; 3]) -> bool {
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let (mut pos, mut neg) = (false, false);
    for i in 0..world.len() {
        let a = world[i];
        let b = world[(i + 1) % world.len()];
        let side = dot3(cross(sub3(b, a), sub3(p, a)), n);
        pos |= side > 1e-6;
        neg |= side < -1e-6;
    }
    !(pos && neg)
}

/// Drawn solid surfaces and `CameraCollision`; not the sky, decals or script helpers.
fn blocks_camera(scene: &LdbScene, material_id: i32) -> bool {
    let camera_only = scene
        .materials
        .get(&material_id)
        .is_some_and(|m| m.category.eq_ignore_ascii_case("cameracollision"));
    let policy = poly_policy(scene, material_id);
    camera_only || (policy.draw && policy.writes_z && !policy.skybox)
}

/// Smallest `t` in `0..=1` where `from + t * d` crosses a static polygon.
fn segment_hit(scene: &LdbScene, from: [f32; 3], d: [f32; 3]) -> Option<f32> {
    segment_surface(scene, from, d).map(|hit| hit.0)
}

/// First camera-blocking polygon along the segment: its `t` in 0..1, normal and material.
fn segment_surface(scene: &LdbScene, from: [f32; 3], d: [f32; 3]) -> Option<(f32, [f32; 3], i32)> {
    let mut best: Option<(f32, [f32; 3], i32)> = None;
    for sm in &scene.static_meshes {
        let o = sm.world_origin;
        for poly in sm
            .polys
            .iter()
            .filter(|p| p.vertices.len() >= 3 && blocks_camera(scene, p.material_id))
        {
            let n = poly.normal;
            let denom = dot3(n, d);
            if denom.abs() < 1e-6 {
                continue;
            }
            let world = poly_world(&poly.vertices, o);
            let t = dot3(n, sub3(world[0], from)) / denom;
            if !(0.0..=1.0).contains(&t) || best.is_some_and(|b| t >= b.0) {
                continue;
            }
            let hit = [from[0] + d[0] * t, from[1] + d[1] * t, from[2] + d[2] * t];
            if inside_convex(&world, n, hit) {
                best = Some((t, n, poly.material_id));
            }
        }
    }
    best
}

/// Visible and not a script helper (`_Tip_`, `Start_FSM.DO`).
pub(crate) fn dynamic_mesh_drawn(dm: &crate::scene::SceneDynamicMesh) -> bool {
    !dm.hidden && !dm.name.contains("_Tip_") && !dm.name.ends_with("Start_FSM.DO")
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
    let mut tris = Vec::new();
    for (idx, dm) in scene.dynamic_meshes.iter().enumerate() {
        if !dynamic_mesh_drawn(dm) {
            continue;
        }
        let (mat, origin) = dynamic_world(scene, idx, room_origins);
        queue_poly_batch(
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
            &mut tris,
        );
    }
    flush_tris(&tris, fb, zb, raster_threads());
}

/// `globalconfig.txt` `AICapsuleRadius` and `max_payne.txt` `CapsuleTop`.
const ENEMY_RADIUS: f32 = 0.31;
const ENEMY_HEIGHT: f32 = 1.95;

/// Distance along the ray to a standing character whose feet are at `feet`.
fn ray_hits_character(origin: [f32; 3], dir: [f32; 3], feet: [f32; 3]) -> Option<f32> {
    ray_hits_capsule(origin, dir, feet, ENEMY_RADIUS, ENEMY_HEIGHT)
}

/// Where a shot from `origin` along `aim` turns to, when it passes through a target's capsule
/// grown by `scale` (radius, height) nearer than `reach`.
fn auto_aim_at(
    enemies: &[EnemyState],
    origin: [f32; 3],
    aim: [f32; 3],
    scale: [f32; 2],
    reach: f32,
) -> Option<[f32; 3]> {
    let [radius, height] = [ENEMY_RADIUS * scale[0], ENEMY_HEIGHT * scale[1]];
    let target = enemies
        .iter()
        .filter(|en| en.health > 0.0 && en.auto_aim_target && !en.held)
        .filter_map(|en| {
            Some((
                ray_hits_capsule(origin, aim, en.world_pos, radius, height)?,
                en,
            ))
        })
        .filter(|(t, _)| *t <= reach)
        .min_by(|a, b| a.0.total_cmp(&b.0))?
        .1;
    // HYPOTHESIS: `AutoAimTargetBoneName` (`JBack`) sits half way up, as the torso does for blood.
    let [x, y, z] = target.world_pos;
    let to = sub3([x, y + ENEMY_HEIGHT / 2.0, z], origin);
    Some(to.map(|c| c / dot3(to, to).sqrt()))
}

fn ray_hits_capsule(
    origin: [f32; 3],
    dir: [f32; 3],
    feet: [f32; 3],
    radius: f32,
    height: f32,
) -> Option<f32> {
    let to = sub3(feet, origin);
    let flat = dir[0] * dir[0] + dir[2] * dir[2];
    if flat < 1e-8 {
        return None;
    }
    let t = (to[0] * dir[0] + to[2] * dir[2]) / flat;
    let (dx, dz) = (
        origin[0] + dir[0] * t - feet[0],
        origin[2] + dir[2] * t - feet[2],
    );
    let inside = dx * dx + dz * dz <= radius * radius;
    let across = origin[1] + dir[1] * t - feet[1];
    (t > 0.0 && inside && (0.0..=height).contains(&across)).then_some(t)
}

/// `shooting.h` `CROSSHAIR_CASTLENGTH`.
const SHOT_RANGE: f32 = 100.0;
/// How long a blood droplet that lands nowhere stays in flight.
const DROPLET_SECONDS: f32 = 3.0;

/// Where a shot stops on the level: distance, polygon normal and material.
fn aim_hit_level(
    scene: &LdbScene,
    origin: [f32; 3],
    dir: [f32; 3],
) -> Option<(f32, [f32; 3], i32)> {
    let hit = segment_surface(scene, origin, dir.map(|c| c * SHOT_RANGE))?;
    Some((hit.0 * SHOT_RANGE, hit.1, hit.2))
}

/// Closest dynamic mesh that bullets affect and whose polygons the ray crosses.
fn aim_hit_dynamic(
    scene: &LdbScene,
    rooms: &BTreeMap<i32, [f32; 3]>,
    origin: [f32; 3],
    dir: [f32; 3],
) -> Option<(usize, f32, [f32; 3], i32)> {
    let mut best: Option<(usize, f32, [f32; 3], i32)> = None;
    for (idx, dm) in scene.dynamic_meshes.iter().enumerate() {
        let shootable = dm.hitpoints.is_some() || scene.reacts_to_bullets(idx);
        if dm.hidden || !shootable || dm.polys.is_empty() {
            continue;
        }
        let (mat, pos) = dynamic_world(scene, idx, rooms);
        for poly in dm.polys.iter().filter(|p| p.vertices.len() >= 3) {
            let world: Vec<[f32; 3]> = poly
                .vertices
                .iter()
                .map(|v| xform_point(mat, pos, v.pos))
                .collect();
            let n = xform_dir(mat, poly.normal);
            let along = dot3(n, dir);
            if along.abs() < 1e-6 {
                continue;
            }
            let t = dot3(n, sub3(world[0], origin)) / along;
            if !(0.2..=SHOT_RANGE).contains(&t) || best.is_some_and(|hit| t >= hit.1) {
                continue;
            }
            if inside_convex(&world, n, [0, 1, 2].map(|c| origin[c] + dir[c] * t)) {
                best = Some((idx, t, n, poly.material_id));
            }
        }
    }
    best
}

/// Sound voices, each tagged with its emitter. Dropping it stops them all (end of level).
#[derive(Default)]
struct SoundChannels {
    voices: Vec<Voice>,
}

/// One cue's player. A positional loop keeps its voice while out of range, with no player.
struct Voice {
    emitter: String,
    wav: PathBuf,
    looping: bool,
    range: Option<crate::SoundRange>,
    at: Option<[f32; 3]>,
    /// The player and the gain it was started at.
    child: Option<(Child, f32)>,
}

impl Voice {
    fn kill(&mut self) {
        if let Some((mut child, _)) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn start(&mut self, gain: f32) {
        self.kill();
        let volume = (gain * 100.0).round().clamp(1.0, 100.0) as u32;
        self.child = std::process::Command::new("ffplay")
            .args(sound_player_args(&self.wav, self.looping, volume))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()
            .map(|child| (child, gain));
    }
}

impl Drop for SoundChannels {
    fn drop(&mut self) {
        for voice in &mut self.voices {
            voice.kill();
        }
    }
}

/// Linear gain of a positional cue `distance` metres from the listener: full inside the hotspot,
/// `hotspot / distance` beyond it, nothing past the falloff.
fn sound_gain(distance: f32, range: &crate::SoundRange) -> f32 {
    if distance > range.falloff {
        return 0.0;
    }
    range.volume * (range.hotspot / distance).min(1.0)
}

/// What a positional voice needs this frame, given the gain it was started at (if running).
#[derive(Debug, PartialEq)]
enum VoiceAction {
    Keep,
    /// Out of range: release the player (retail `0x1000bb60` stops the buffer).
    Stop,
    /// (Re)start at the new gain: `ffplay` cannot change volume while it plays.
    Start,
}

fn voice_action(running: Option<f32>, gain: f32) -> VoiceAction {
    match running {
        _ if gain <= 0.0 => {
            if running.is_some() {
                VoiceAction::Stop
            } else {
                VoiceAction::Keep
            }
        }
        None => VoiceAction::Start,
        Some(started) if gain > started * 2.0 || gain < started * 0.5 => VoiceAction::Start,
        Some(_) => VoiceAction::Keep,
    }
}

/// `ffplay` without a window at `volume` percent; `-loop 0` repeats a looping cue until it is killed.
pub(crate) fn sound_player_args(wav: &Path, looping: bool, volume: u32) -> Vec<String> {
    let mut args: Vec<String> = ["-nodisp", "-autoexit", "-loglevel", "quiet"]
        .map(String::from)
        .to_vec();
    args.extend(["-volume".to_string(), volume.to_string()]);
    if looping {
        args.extend(["-loop".to_string(), "0".to_string()]);
    }
    args.push(wav.display().to_string());
    args
}

/// Load the catalog of each queued cue whose category is not loaded yet, then resolve the cue.
fn resolve_queued_sounds(scene: &mut LdbScene, game_dir: &Path) {
    for index in 0..scene.sound_queue.len() {
        if scene.sound_queue[index].filename.is_some() {
            continue;
        }
        let category = scene.sound_queue[index].category.to_ascii_lowercase();
        if !scene
            .sound_catalogs
            .keys()
            .any(|key| key.eq_ignore_ascii_case(&category))
        {
            // An empty text marks a missing catalog, so it is read once.
            let text = crate::load_sound_catalog(game_dir, &category).unwrap_or_default();
            scene.sound_catalogs.insert(category, text);
        }
        let mut sound = scene.sound_queue[index].clone();
        scene.resolve_sound(&mut sound);
        scene.sound_queue[index] = sound;
    }
}

/// Gain of `voice` heard at `listener`: full for a non-positional cue or an emitter with no position.
fn voice_gain(
    voice: &Voice,
    scene: &LdbScene,
    rooms: &BTreeMap<i32, [f32; 3]>,
    listener: [f32; 3],
) -> f32 {
    let Some(range) = &voice.range else {
        return 1.0;
    };
    let pos = voice.at.or_else(|| {
        let pos = scene.emitter_world_pos(&voice.emitter)?;
        let room = emitter_room(scene, &voice.emitter);
        let origin = rooms.get(&room).copied().unwrap_or([0.0; 3]);
        Some([0, 1, 2].map(|k| origin[k] + pos[k]))
    });
    let Some(pos) = pos else {
        return range.volume;
    };
    let d = (0..3)
        .map(|k| (pos[k] - listener[k]).powi(2))
        .sum::<f32>()
        .sqrt();
    sound_gain(d, range)
}

/// Stop what `A_StopAll3DSounds` asked for, start the queued cues, and keep positional loops at
/// the listener's distance: a loop is released past its falloff and restarted when the gain
/// halves or doubles (`ffplay` cannot change volume while it plays).
fn play_queued_sounds(
    scene: &mut LdbScene,
    game_dir: &Path,
    sounds: &mut SoundChannels,
    rooms: &BTreeMap<i32, [f32; 3]>,
    listener: [f32; 3],
) {
    for emitter in std::mem::take(&mut scene.sound_stops) {
        sounds.voices.retain_mut(|voice| {
            if voice.emitter.eq_ignore_ascii_case(&emitter) {
                voice.kill();
                return false;
            }
            true
        });
    }
    // Reap one-shots that finished.
    sounds.voices.retain_mut(|voice| {
        voice.looping
            || voice
                .child
                .as_mut()
                .is_some_and(|(child, _)| matches!(child.try_wait(), Ok(None)))
    });
    for voice in sounds.voices.iter_mut().filter(|voice| voice.looping) {
        let gain = voice_gain(voice, scene, rooms, listener);
        match voice_action(voice.child.as_ref().map(|(_, g)| *g), gain) {
            VoiceAction::Keep => {}
            VoiceAction::Stop => voice.kill(),
            VoiceAction::Start => voice.start(gain),
        }
    }
    resolve_queued_sounds(scene, game_dir);
    for sound in std::mem::take(&mut scene.sound_queue) {
        let Some(filename) = sound.filename.as_deref() else {
            continue;
        };
        let Some(bytes) = crate::read_game_bytes(game_dir, &crate::story_wav_win_path(filename))
        else {
            continue;
        };
        let name = filename.replace(['\\', '/'], "_");
        let wav = std::env::temp_dir().join(format!("omp-{}-{name}", std::process::id()));
        if !wav.is_file() && std::fs::write(&wav, &bytes).is_err() {
            continue;
        }
        let mut voice = Voice {
            emitter: sound.emitter,
            wav,
            looping: sound.looping,
            range: sound.range,
            at: sound.at,
            child: None,
        };
        let gain = voice_gain(&voice, scene, rooms, listener);
        if gain > 0.0 {
            voice.start(gain);
        }
        // A one-shot out of range is never heard; a loop waits for the listener.
        if voice.looping || voice.child.is_some() {
            sounds.voices.push(voice);
        }
    }
}

/// One `CAM_AnimateInPlace` path playing on the player camera.
struct CameraShake {
    anim: crate::kf2::Kf2Anim,
    time: f32,
}

impl CameraShake {
    /// The path's `(rotation, position)` now, or `None` once the path is over.
    fn pose(&self) -> Option<([f32; 9], [f32; 3])> {
        let bone = self.anim.bones.first()?;
        if self.time * bone.fps >= bone.duration as f32 {
            return None;
        }
        crate::kf2::sample_locals(&self.anim, self.time).remove(&bone.name.to_ascii_lowercase())
    }

    /// Camera-local `(rotation, offset)` now, or `None` once the path is over.
    fn delta(&self) -> Option<([f32; 9], [f32; 3])> {
        let first = self.anim.bones.first()?.keys.first()?;
        let (mat, pos) = self.pose()?;
        // Offset from the first key, so the path starts on the player camera. Every retail shake
        // starts with an identity rotation, so the key rotation is used as is.
        Some((mat, [0, 1, 2].map(|k| pos[k] - first.pos[k])))
    }
}

/// The view of a camera-path pose `(rotation, offset)` in the frame of a character standing at
/// `feet` and facing `yaw` (its +z), with the path's x right, y up and z forward.
fn parented_view(feet: [f32; 3], yaw: f32, pose: ([f32; 9], [f32; 3])) -> View {
    let (sy, cy) = yaw.sin_cos();
    let world = |v: [f32; 3]| [v[0] * cy + v[2] * sy, v[1], v[2] * cy - v[0] * sy];
    let (mat, offset) = pose;
    let at = world(offset);
    let fwd = world(xform_dir(mat, [0.0, 0.0, 1.0]));
    let up = world(xform_dir(mat, [0.0, 1.0, 0.0]));
    let right = [
        up[1] * fwd[2] - up[2] * fwd[1],
        up[2] * fwd[0] - up[0] * fwd[2],
        up[0] * fwd[1] - up[1] * fwd[0],
    ];
    ([0, 1, 2].map(|k| feet[k] + at[k]), right, up, fwd)
}

/// Start the path of the last `CAM_AnimateInPlace` block; it replaces a running one.
fn start_camera_shakes(
    scene: &mut LdbScene,
    game_dir: &Path,
    camerapaths_txt: &str,
    shake: &mut Option<CameraShake>,
) {
    for block in std::mem::take(&mut scene.camera_shakes) {
        if let Some(anim) = load_camera_path(game_dir, camerapaths_txt, &block) {
            *shake = Some(CameraShake { anim, time: 0.0 });
        }
    }
}

/// The `Camera01` KF2 path of a `camerapaths.txt` block.
fn load_camera_path(
    game_dir: &Path,
    camerapaths_txt: &str,
    block: &str,
) -> Option<crate::kf2::Kf2Anim> {
    crate::camera_path_file(camerapaths_txt, block)
        .and_then(|path| crate::read_game_bytes(game_dir, &path))
        .and_then(|bytes| crate::kf2::parse_kf2_anim(&bytes))
}

/// The view moved by a camera-local `(rotation, offset)`: x along right, y up, z forward.
fn shaken_view(view: View, delta: ([f32; 9], [f32; 3])) -> View {
    let (pos, right, up, fwd) = view;
    let world = |v: [f32; 3]| [0, 1, 2].map(|k| right[k] * v[0] + up[k] * v[1] + fwd[k] * v[2]);
    let (mat, offset) = delta;
    let moved = world(offset);
    let new_fwd = world(xform_dir(mat, [0.0, 0.0, 1.0]));
    let new_up = world(xform_dir(mat, [0.0, 1.0, 0.0]));
    let new_right = [
        new_up[1] * new_fwd[2] - new_up[2] * new_fwd[1],
        new_up[2] * new_fwd[0] - new_up[0] * new_fwd[2],
        new_up[0] * new_fwd[1] - new_up[1] * new_fwd[0],
    ];
    (
        [0, 1, 2].map(|k| pos[k] + moved[k]),
        new_right,
        new_up,
        new_fwd,
    )
}

/// Stop what `PS_StopAllEmissions` asked for, then start the lights of queued `PS_StartEffect`s.
fn start_queued_effects(
    scene: &mut LdbScene,
    game_dir: &Path,
    particles_txt: &str,
    lights: &mut ParticleLights,
    sprites: &mut ParticleSprites,
) {
    for emitter in std::mem::take(&mut scene.effect_stops) {
        lights.stop(&emitter);
        sprites.field.stop(&emitter);
    }
    for effect in std::mem::take(&mut scene.effect_queue) {
        let Some((win_path, looping)) = crate::particle_effect_file(particles_txt, &effect.effect)
        else {
            continue;
        };
        let rps = lights
            .files
            .entry(win_path.to_ascii_lowercase())
            .or_insert_with(|| {
                crate::read_game_bytes(game_dir, &win_path)
                    .map(|bytes| openpfx::light::rps_lights(&bytes))
                    .unwrap_or_default()
            })
            .clone();
        sprites.start(effect.emitter.clone(), game_dir, &win_path, looping);
        lights.start(effect.emitter, rps, looping);
    }
}

/// What a bullet hit starts: the `[Material]` variant of its sound and effect, or the `[Default]`.
struct HitEffects<'a> {
    game_dir: &'a Path,
    particles_txt: &'a str,
    /// `sounds/projectiles.txt`.
    sounds_txt: String,
}

impl HitEffects<'_> {
    /// The effect and sound a projectile's own explosion clip names.
    fn explode(
        &self,
        scene: &mut LdbScene,
        sprites: &mut ParticleSprites,
        def: &FlyingDef,
        point: [f32; 3],
        frame: [f32; 9],
    ) {
        let set = crate::decals::DecalSet {
            effect: def.effect.clone(),
            sound: def.sound.clone(),
            ..Default::default()
        };
        self.start(scene, sprites, &set, "", point, frame);
    }

    fn start(
        &self,
        scene: &mut LdbScene,
        sprites: &mut ParticleSprites,
        set: &crate::decals::DecalSet,
        category: &str,
        point: [f32; 3],
        frame: [f32; 9],
    ) {
        let files = set.effect.as_deref().map_or(Vec::new(), |effect| {
            crate::material_files(self.particles_txt, effect, category)
        });
        let effect = files.first().and_then(|file| {
            let win_path = format!("\\data\\database\\particles\\{file}");
            sprites.effect(self.game_dir, &win_path)
        });
        if let Some((key, effect)) = effect {
            sprites.field.start_at((point, frame), key, effect);
        }
        let Some(cue) = set.sound.as_deref() else {
            return;
        };
        let files = crate::material_files(&self.sounds_txt, cue, category);
        // HYPOTHESIS: retail picks among a cue's `[Sound]`s at random.
        let pick = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.subsec_nanos() as usize);
        if let Some(file) = files.get(pick % files.len().max(1)) {
            scene.sound_queue.push(crate::scene::QueuedSound {
                category: "projectiles".to_string(),
                cue: cue.to_string(),
                filename: Some(file.clone()),
                emitter: String::new(),
                looping: false,
                range: crate::sound_cue_3d(&self.sounds_txt, cue),
                at: Some(point),
            });
        }
    }
}

/// Particles of running effects (`particles::ParticleField`) and their decoded bitmaps.
#[derive(Default)]
struct ParticleSprites {
    field: crate::particles::ParticleField,
    /// Parsed `.rps` per archive path (lower case); `None` when it does not parse.
    effects: BTreeMap<String, Option<std::sync::Arc<openpfx::effect::RpsEffect>>>,
    textures: BTreeMap<crate::particles::SpriteTexture, Option<DecodedTexture>>,
}

impl ParticleSprites {
    fn effect(
        &mut self,
        game_dir: &Path,
        win_path: &str,
    ) -> Option<(String, std::sync::Arc<openpfx::effect::RpsEffect>)> {
        let key = win_path.to_ascii_lowercase();
        let effect = self
            .effects
            .entry(key.clone())
            .or_insert_with(|| {
                crate::read_game_bytes(game_dir, win_path)
                    .and_then(|bytes| openpfx::effect::rps_effect(&bytes))
                    .map(std::sync::Arc::new)
            })
            .clone()?;
        Some((key, effect))
    }

    fn start(&mut self, emitter: String, game_dir: &Path, win_path: &str, looping: bool) {
        if let Some((key, effect)) = self.effect(game_dir, win_path) {
            self.field.start(emitter, key, effect, looping);
        }
    }

    fn step(&mut self, scene: &LdbScene, rooms: &BTreeMap<i32, [f32; 3]>, dt: f32) {
        if let Some(gravity) = scene.gravity {
            self.field.gravity = gravity;
        }
        self.field
            .step(dt, |emitter| emitter_frame(scene, rooms, emitter));
    }

    /// Live sprites, with their bitmaps decoded on first use.
    fn sprites(&mut self) -> Vec<crate::particles::Sprite> {
        let sprites = self.field.sprites();
        for sprite in &sprites {
            let key = &sprite.texture;
            if self.textures.contains_key(key) {
                continue;
            }
            let texture = self
                .effects
                .get(&key.effect)
                .and_then(Option::as_ref)
                .and_then(|effect| effect.emitters.get(key.emitter))
                .and_then(|e| e.particle.sequences.get(key.sequence))
                .and_then(|frames| frames.get(key.frame))
                .and_then(crate::particles::frame_texture);
            self.textures.insert(key.clone(), texture);
        }
        sprites
    }

    fn texture(&self, key: &crate::particles::SpriteTexture) -> Option<&DecodedTexture> {
        self.textures.get(key)?.as_ref()
    }
}

/// World position and rotation (columns) of a particle emitter: a dynamic mesh's frame, an FSM
/// carried by its parent mesh, or a trigger's or character's position.
fn emitter_frame(
    scene: &LdbScene,
    rooms: &BTreeMap<i32, [f32; 3]>,
    emitter: &str,
) -> Option<([f32; 3], [f32; 9])> {
    if let Some(idx) = scene
        .dynamic_meshes
        .iter()
        .position(|mesh| mesh.name.eq_ignore_ascii_case(emitter))
    {
        let (mat, pos) = dynamic_world(scene, idx, rooms);
        return Some((pos, mat));
    }
    let (rot, pos) = scene.emitter_world_pose(emitter)?;
    let origin = rooms
        .get(&emitter_room(scene, emitter))
        .copied()
        .unwrap_or([0.0; 3]);
    Some(([0, 1, 2].map(|k| pos[k] + origin[k]), rot))
}

/// Point lights of running particle effects (ParticleFX emitter "Use Light").
#[derive(Default)]
struct ParticleLights {
    /// Lights per `.rps` archive path (lower case), read once.
    files: BTreeMap<String, Vec<openpfx::light::RpsLight>>,
    /// `(emitter, lights, looping, seconds since start)`.
    active: Vec<(String, Vec<openpfx::light::RpsLight>, bool, f32)>,
}

impl ParticleLights {
    fn start(&mut self, emitter: String, lights: Vec<openpfx::light::RpsLight>, looping: bool) {
        if !lights.is_empty() {
            self.active.push((emitter, lights, looping, 0.0));
        }
    }

    fn stop(&mut self, emitter: &str) {
        self.active
            .retain(|(owner, ..)| !owner.eq_ignore_ascii_case(emitter));
    }

    /// Age every effect; a one-shot effect ends with its longest cycle.
    fn step(&mut self, dt: f32) {
        for (_, _, _, age) in &mut self.active {
            *age += dt;
        }
        self.active.retain(|(_, lights, looping, age)| {
            *looping || lights.iter().any(|light| *age < light.cycle_length)
        });
    }

    fn point_lights(
        &self,
        scene: &LdbScene,
        rooms: &BTreeMap<i32, [f32; 3]>,
    ) -> Vec<ScenePointLight> {
        let mut out = Vec::new();
        for (emitter, lights, looping, age) in &self.active {
            let Some(pos) = scene.emitter_world_pos(emitter) else {
                continue;
            };
            let room = emitter_room(scene, emitter);
            let origin = rooms.get(&room).copied().unwrap_or([0.0; 3]);
            for light in lights {
                let t = if *looping && light.cycle_length > 0.0 {
                    age % light.cycle_length
                } else {
                    *age
                };
                let (rgb, radius) = light.sample(t);
                out.push(ScenePointLight {
                    id: -1,
                    room_id: room,
                    pos: [0, 1, 2].map(|k| origin[k] + pos[k] + light.offset[k]),
                    color: [rgb[0], rgb[1], rgb[2], 255.0],
                    radius,
                    // ParticleFX "Brightness" has no effect; the colour graph sets the strength.
                    intensity: 1.0,
                });
            }
        }
        out
    }
}

/// Room of the mesh, trigger or FSM named `emitter` (0 when none matches).
fn emitter_room(scene: &LdbScene, emitter: &str) -> i32 {
    // Held guns are already in the world.
    if crate::scene::GUN_EMITTERS.contains(&emitter) {
        return -1;
    }
    scene
        .dynamic_meshes
        .iter()
        .map(|mesh| (&mesh.name, mesh.room_id))
        .chain(scene.triggers.iter().map(|t| (&t.name, t.room_id)))
        .chain(scene.fsms.iter().map(|fsm| (&fsm.name, fsm.room_id)))
        .chain(scene.characters.iter().map(|c| (&c.name, c.room_id)))
        .find(|(name, _)| name.eq_ignore_ascii_case(emitter))
        .map_or(0, |(_, room)| room)
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

pub(crate) fn dynamic_world(
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

/// Eye position on the floor under jumppoint `idx`.
fn spawn_pos(scene: &LdbScene, room_origins: &BTreeMap<i32, [f32; 3]>, idx: usize) -> [f32; 3] {
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
}

/// One headless frame (`W` x `H`, `0x00RRGGBB`): level geometry and dynamic meshes, seen from the
/// dynamic mesh named `camera` (as `CAM_AttachToDO` would) or from the walkthrough's spawn view.
pub fn render_frame(
    scene: &LdbScene,
    camera: Option<&str>,
) -> Result<(usize, usize, Vec<u32>), String> {
    let rooms: BTreeMap<i32, [f32; 3]> =
        scene.rooms.iter().map(|r| (r.id, r.world_origin)).collect();
    let camera_idx = match camera {
        Some(name) => Some(
            scene
                .dynamic_meshes
                .iter()
                .position(|m| m.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("no dynamic mesh named {name}"))?,
        ),
        None => None,
    };
    let (pos, right, up, fwd) = match camera_idx {
        Some(idx) => camera_view(scene, idx, &rooms),
        None => {
            let pos = spawn_pos(scene, &rooms, scene.spawn_jumppoint_index(None));
            let (sy, cy) = std::f32::consts::PI.sin_cos();
            let (sp, cp) = (-0.08f32).sin_cos();
            (
                pos,
                [cy, 0.0, -sy],
                [-sy * sp, cp, -cy * sp],
                [sy * cp, sp, cy * cp],
            )
        }
    };
    let mut fb = vec![0x0016_1c28u32; W * H];
    let mut zb = vec![f32::INFINITY; W * H];
    let fov_scale = fov_scale(scene.fov_multiplier.value);
    const IDENT_MAT: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let mut tris = Vec::new();
    for sm in static_meshes_in_view(scene, camera_idx) {
        queue_poly_batch(
            &sm.polys,
            IDENT_MAT,
            sm.world_origin,
            scene,
            pos,
            right,
            up,
            fwd,
            fov_scale,
            true,
            false,
            true,
            &mut fb,
            &mut tris,
        );
    }
    flush_tris(&tris, &mut fb, &mut zb, raster_threads());
    draw_dynamic_meshes(
        scene, &rooms, pos, right, up, fwd, fov_scale, true, false, &mut fb, &mut zb,
    );
    apply_fog(&mut fb, &zb, scene.fog);
    Ok((W, H, fb))
}

/// Binary PPM (`P6`) of a `0x00RRGGBB` frame.
pub fn write_ppm(path: &Path, w: usize, h: usize, fb: &[u32]) -> std::io::Result<()> {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for px in fb {
        out.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, *px as u8]);
    }
    std::fs::write(path, out)
}

/// Static room meshes a view shows. A level camera (`camera`, a dynamic mesh) sees the exit
/// group of its room (`LdbScene::room_group`): `startup_level`'s opening water scene overlaps
/// its street in space. The free player camera does not track its room, so it sees every room.
pub(crate) fn static_meshes_in_view(
    scene: &LdbScene,
    camera: Option<usize>,
) -> impl Iterator<Item = &crate::scene::SceneStaticMesh> {
    let group = camera
        .and_then(|i| scene.dynamic_meshes.get(i))
        .map(|dm| scene.room_group(dm.room_id));
    scene
        .static_meshes
        .iter()
        .filter(move |sm| match (&group, sm.room_id) {
            (Some(group), Some(room)) => group.contains(&room),
            _ => true,
        })
}

/// The HUD crosshair is hidden while a cutscene camera owns the view.
fn crosshair_visible(scene: &LdbScene) -> bool {
    scene.attached_camera.is_none()
}

/// Eye position, right, up, forward.
type View = ([f32; 3], [f32; 3], [f32; 3], [f32; 3]);

/// Eye, right, up and forward of the `CAM_AttachToDO` mesh: forward is its +z axis (as the
/// startup flyover), up its y axis, right = up × forward like the player camera.
fn attached_view(scene: &LdbScene, rooms: &BTreeMap<i32, [f32; 3]>) -> Option<View> {
    let idx = scene.attached_camera?;
    scene.dynamic_meshes.get(idx)?;
    Some(camera_view(scene, idx, rooms))
}

fn camera_view(scene: &LdbScene, idx: usize, rooms: &BTreeMap<i32, [f32; 3]>) -> View {
    let (mat, pos) = dynamic_world(scene, idx, rooms);
    let fwd = normalize(xform_dir(mat, [0.0, 0.0, 1.0]));
    let up = normalize(xform_dir(mat, [0.0, 1.0, 0.0]));
    let right = [
        up[1] * fwd[2] - up[2] * fwd[1],
        up[2] * fwd[0] - up[0] * fwd[2],
        up[0] * fwd[1] - up[1] * fwd[0],
    ];
    (pos, right, up, fwd)
}

/// Linear fog amount at view depth `z`: 0 before `start`, 1 from `end` on.
pub(crate) fn fog_amount(fog: &LevelFog, z: f32) -> f32 {
    if fog.end <= fog.start {
        return if z >= fog.end { 1.0 } else { 0.0 };
    }
    ((z - fog.start) / (fog.end - fog.start)).clamp(0.0, 1.0)
}

/// Software fog after rasterisation: each pixel with a depth mixes toward the fog colour by its
/// view depth. Sky pixels (no depth) stay clear.
pub(crate) fn apply_fog(fb: &mut [u32], zb: &[f32], fog: Option<LevelFog>) {
    let Some(fog) = fog else {
        return;
    };
    for (px, &z) in fb.iter_mut().zip(zb).filter(|(_, z)| z.is_finite()) {
        let k = fog_amount(&fog, z);
        let mix = |shift: u32, c: f32| {
            let v = ((*px >> shift) & 0xff) as f32;
            ((v + (c - v) * k).round() as u32) << shift
        };
        *px = mix(16, fog.color[0]) | mix(8, fog.color[1]) | mix(0, fog.color[2]);
    }
}

/// Mix `rgba` over every `0x00RRGGBB` pixel by its alpha.
/// The running `MPHM_FlashToColor` over the frame, or over the hardware renderer's overlay.
fn draw_screen_flash(scene: &LdbScene, fb: &mut [u32]) {
    let Some(flash) = scene.screen_flash else {
        return;
    };
    let opacity = flash.opacity().unwrap_or(0.0);
    let alpha = ((flash.color & 0xff) as f32 * opacity) as u32;
    for px in fb {
        *px = crate::hud::blend(*px, flash.color >> 8, alpha);
    }
}

pub(crate) fn blend_screen(fb: &mut [u32], rgba: [u8; 4]) {
    let a = u32::from(rgba[3]);
    if a == 0 {
        return;
    }
    let mix = |c: u32, k: u8| (c * (255 - a) + u32::from(k) * a + 127) / 255;
    for px in fb {
        let r = mix((*px >> 16) & 0xff, rgba[0]);
        let g = mix((*px >> 8) & 0xff, rgba[1]);
        let b = mix(*px & 0xff, rgba[2]);
        *px = (r << 16) | (g << 8) | b;
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1.0e-6);
    [v[0] / len, v[1] / len, v[2] / len]
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
/// Bilinear lightmap texel at `(u, v)`, clamped to the edges (wgpu `ClampToEdge`, linear filter).
/// HYPOTHESIS: retail filters lightmaps linearly; the `addTextureLayer` filter values (`0x1000000`,
/// `0x5000000`) are not decoded, but the retail house shades smoothly.
fn sample_lightmap(lm: &DecodedLightmap, u: f32, v: f32) -> u32 {
    let (w, h) = (lm.width.max(1), lm.height.max(1));
    let x = (u.clamp(0.0, 1.0) * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let y = (v.clamp(0.0, 1.0) * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x as u32, y as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let texel = |tx: u32, ty: u32| lm.pixels.get((ty * w + tx) as usize).copied().unwrap_or(0);
    let (c00, c10, c01, c11) = (texel(x0, y0), texel(x1, y0), texel(x0, y1), texel(x1, y1));
    let channel = |shift: u32| {
        let c = |p: u32| ((p >> shift) & 0xff) as f32;
        let top = c(c00) + (c(c10) - c(c00)) * fx;
        let bottom = c(c01) + (c(c11) - c(c01)) * fx;
        ((top + (bottom - top) * fy).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// Particle sprites as camera-facing squares: depth-tested against the frame without writing
/// depth, the bitmap times the sprite colour, blended by alpha or added (`Sprite::additive`).
pub(crate) fn draw_sprites<'a>(
    fb: &mut [u32],
    zb: &mut [f32],
    (cam, right, up, fwd): View,
    fov_scale: f32,
    fog: Option<LevelFog>,
    sprites: &[crate::particles::Sprite],
    texture: impl Fn(&crate::particles::SpriteTexture) -> Option<&'a DecodedTexture>,
) {
    for sprite in sprites {
        let d = sub3(sprite.pos, cam);
        let cz = dot3(d, fwd);
        if cz < NEAR_Z {
            continue;
        }
        let Some(tex) = texture(&sprite.texture) else {
            continue;
        };
        if let Some(axes) = sprite.facing {
            draw_world_quad(
                fb,
                zb,
                (cam, right, up, fwd),
                fov_scale,
                sprite,
                axes,
                tex,
                fog,
            );
            continue;
        }
        let sx = W as f32 * 0.5 + dot3(d, right) * fov_scale / cz;
        let sy = H as f32 * 0.5 - dot3(d, up) * fov_scale / cz;
        let half = sprite.size * 0.5 * fov_scale / cz;
        if half < 0.25 {
            continue;
        }
        let fogged = fog.filter(|_| !sprite.additive);
        let haze = fogged.map_or(0.0, |fog| fog_amount(&fog, cz));
        let fog_colour = fogged.map_or([0.0; 3], |fog| fog.color);
        let x0 = (sx - half).floor().max(0.0) as usize;
        let x1 = ((sx + half).ceil().max(0.0) as usize).min(W);
        let y0 = (sy - half).floor().max(0.0) as usize;
        let y1 = ((sy + half).ceil().max(0.0) as usize).min(H);
        for y in y0..y1 {
            for x in x0..x1 {
                let idx = y * W + x;
                if cz >= zb[idx] {
                    continue;
                }
                let u = (x as f32 + 0.5 - (sx - half)) / (2.0 * half);
                let v = (y as f32 + 0.5 - (sy - half)) / (2.0 * half);
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                let texel = sample_texture(tex, u, v);
                let a = ((texel >> 24) as f32 / 255.0) * sprite.rgba[3];
                let dst = fb[idx];
                let channel = |shift: u32, k: usize| {
                    let src = ((texel >> shift) & 0xff) as f32 * sprite.rgba[k];
                    let src = src + (fog_colour[k] - src) * haze;
                    let back = ((dst >> shift) & 0xff) as f32;
                    let out = if sprite.additive {
                        back + src * a
                    } else {
                        src * a + back * (1.0 - a)
                    };
                    (out.round().clamp(0.0, 255.0) as u32) << shift
                };
                fb[idx] = channel(16, 0) | channel(8, 1) | channel(0, 2);
            }
        }
    }
}

/// A sprite fixed in the world, such as a decal: two triangles with affine texture coordinates,
/// which is close enough at decal size.
#[allow(clippy::too_many_arguments)]
fn draw_world_quad(
    fb: &mut [u32],
    zb: &[f32],
    (cam, right, up, fwd): View,
    fov_scale: f32,
    sprite: &crate::particles::Sprite,
    [axis_x, axis_y]: [[f32; 3]; 2],
    tex: &crate::scene::DecodedTexture,
    fog: Option<LevelFog>,
) {
    let half = sprite.size * 0.5;
    let corner = |a: f32, b: f32| {
        let p = [0, 1, 2].map(|k| sprite.pos[k] + (axis_x[k] * a + axis_y[k] * b) * half);
        let d = sub3(p, cam);
        let cz = dot3(d, fwd);
        (cz >= NEAR_Z).then(|| {
            [
                W as f32 * 0.5 + dot3(d, right) * fov_scale / cz,
                H as f32 * 0.5 - dot3(d, up) * fov_scale / cz,
                cz,
            ]
        })
    };
    let (Some(a), Some(b), Some(c), Some(d)) = (
        corner(-1.0, 1.0),
        corner(1.0, 1.0),
        corner(1.0, -1.0),
        corner(-1.0, -1.0),
    ) else {
        return;
    };
    let haze = fog.map_or(0.0, |fog| fog_amount(&fog, a[2]));
    let fog_colour = fog.map_or([0.0; 3], |fog| fog.color);
    for (p, uv) in [
        ([a, b, c], [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]),
        ([a, c, d], [[0.0, 0.0], [1.0, 1.0], [0.0, 1.0]]),
    ] {
        let area =
            (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if area.abs() < 1e-3 {
            continue;
        }
        let min = |k: usize| p.iter().map(|v| v[k]).fold(f32::MAX, f32::min);
        let max = |k: usize| p.iter().map(|v| v[k]).fold(f32::MIN, f32::max);
        let (x0, x1) = (
            min(0).floor().max(0.0) as usize,
            (max(0).ceil().max(0.0) as usize).min(W),
        );
        let (y0, y1) = (
            min(1).floor().max(0.0) as usize,
            (max(1).ceil().max(0.0) as usize).min(H),
        );
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((p[1][0] - px) * (p[2][1] - py) - (p[2][0] - px) * (p[1][1] - py)) / area;
                let w1 = ((p[2][0] - px) * (p[0][1] - py) - (p[0][0] - px) * (p[2][1] - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let idx = y * W + x;
                if w0 * p[0][2] + w1 * p[1][2] + w2 * p[2][2] >= zb[idx] {
                    continue;
                }
                let u = w0 * uv[0][0] + w1 * uv[1][0] + w2 * uv[2][0];
                let v = w0 * uv[0][1] + w1 * uv[1][1] + w2 * uv[2][1];
                let texel = sample_texture(tex, u, v);
                let alpha = ((texel >> 24) as f32 / 255.0) * sprite.rgba[3];
                let dst = fb[idx];
                let channel = |shift: u32, k: usize| {
                    let src = ((texel >> shift) & 0xff) as f32 * sprite.rgba[k];
                    let src = src + (fog_colour[k] - src) * haze;
                    let back = ((dst >> shift) & 0xff) as f32;
                    ((src * alpha + back * (1.0 - alpha))
                        .round()
                        .clamp(0.0, 255.0) as u32)
                        << shift
                };
                fb[idx] = channel(16, 0) | channel(8, 1) | channel(0, 2);
            }
        }
    }
}

/// Bilinear diffuse texel at `(u, v)` with repeat addressing, all four channels (alpha is
/// filtered before the alpha test, as in D3D). Retail offers bilinear, trilinear and anisotropic
/// filtering (`FilteringMode`); the software renderer has no mipmaps.
pub(crate) fn sample_texture(t: &crate::scene::DecodedTexture, u: f32, v: f32) -> u32 {
    let (w, h) = (t.width.max(1), t.height.max(1));
    let x = (u * w as f32 - 0.5).rem_euclid(w as f32);
    let y = (v * h as f32 - 0.5).rem_euclid(h as f32);
    let (x0, y0) = ((x as u32).min(w - 1), (y as u32).min(h - 1));
    let (x1, y1) = ((x0 + 1) % w, (y0 + 1) % h);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let texel = |tx: u32, ty: u32| {
        t.pixels
            .get((ty * w + tx) as usize)
            .copied()
            .unwrap_or(0x0099_9999)
    };
    let (c00, c10, c01, c11) = (texel(x0, y0), texel(x1, y0), texel(x0, y1), texel(x1, y1));
    let channel = |shift: u32| {
        let c = |p: u32| ((p >> shift) & 0xff) as f32;
        let top = c(c00) + (c(c10) - c(c00)) * fx;
        let bottom = c(c01) + (c(c11) - c(c01)) * fx;
        ((top + (bottom - top) * fy).round() as u32) << shift
    };
    channel(24) | channel(16) | channel(8) | channel(0)
}

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

/// How lit a point with no facing is, 0..1.
fn point_brightness(at: [f32; 3], lights: &[ScenePointLight], ambient: [f32; 3]) -> [f32; 3] {
    let mut sum = ambient.map(|c| c / 255.0);
    for light in lights {
        let gap = sub3(light.pos, at);
        let fade = point_light_attenuation(dot3(gap, gap).sqrt(), light.radius);
        let rgb = light_rgb(light);
        for c in 0..3 {
            sum[c] += rgb[c] / 255.0 * fade;
        }
    }
    sum.map(|c| c.min(1.0))
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
    // Signed distance from each edge, so the slack is in metres whatever the triangle's size.
    const SLACK: f32 = 0.005;
    let side = |p: [f32; 3], q: [f32; 3]| {
        let (ex, ez) = (q[0] - p[0], q[2] - p[2]);
        let length = (ex * ex + ez * ez).sqrt().max(1e-6);
        ((px - p[0]) * ez - ex * (pz - p[2])) / length
    };
    let sides = [side(b, a), side(c, b), side(a, c)];
    !(sides.iter().any(|d| *d < -SLACK) && sides.iter().any(|d| *d > SLACK))
}

/// Character vertices posed (or laid flat when dead) and placed in the world.
/// A model, its world vertices and its position.
type PlacedMesh<'a> = (&'a crate::kf2::KfsMesh, Vec<[f32; 3]>, [f32; 3]);

/// Each item still lying in the level.
fn placed_items<'a>(
    level: &crate::items::LevelItems,
    models: &'a BTreeMap<String, crate::kf2::KfsMesh>,
) -> Vec<PlacedMesh<'a>> {
    level
        .items
        .iter()
        .filter(|item| !item.taken)
        .filter_map(|item| {
            let mesh = models.get(&item.kind)?;
            let place = |v: &[f32; 3]| item.world_point(*v);
            Some((mesh, mesh.vertices.iter().map(place).collect(), item.pos))
        })
        .collect()
}

/// The projectiles in the air, with their world vertices.
fn placed_flyers<'a>(
    flyers: &[(Flyer, FlyingDef)],
    models: &'a BTreeMap<String, crate::kf2::KfsMesh>,
) -> Vec<PlacedMesh<'a>> {
    flyers
        .iter()
        .filter_map(|(flyer, def)| {
            let mesh = models.get(def.model.as_ref()?)?;
            let place = |v: &[f32; 3]| [0, 1, 2].map(|c| v[c] + flyer.pos[c]);
            Some((mesh, mesh.vertices.iter().map(place).collect(), flyer.pos))
        })
        .collect()
}

/// Send `count` of `def` off from `from`, along `aim`.
fn launch(
    flyers: &mut Vec<(Flyer, FlyingDef)>,
    def: &FlyingDef,
    count: u32,
    from: [f32; 3],
    aim: [f32; 3],
    seed: &mut u32,
) {
    for _ in 0..count {
        let dir = crate::projectiles::launch_direction(def.direction, aim);
        let dir = spread_aim(dir, def.accuracy, seed);
        let flyer = Flyer {
            pos: from,
            vel: dir.map(|c| c * def.speed),
            age: 0.0,
            resting: false,
            normal: [0.0; 3],
        };
        flyers.push((flyer, def.clone()));
    }
}

/// What a move of a flying projectile runs into first: the level or a living character.
fn flight_contact(
    scene: &LdbScene,
    enemies: &[EnemyState],
    from: [f32; 3],
    delta: [f32; 3],
) -> Option<(f32, [f32; 3])> {
    let level = segment_surface(scene, from, delta).map(|hit| (hit.0, hit.1));
    let length = dot3(delta, delta).sqrt();
    if length < 1e-6 {
        return level;
    }
    let dir = delta.map(|c| c / length);
    let enemy = enemies
        .iter()
        .filter(|en| en.health > 0.0 && !en.held)
        .filter_map(|en| ray_hits_character(from, dir, en.world_pos))
        .map(|t| t / length)
        .filter(|t| *t <= 1.0)
        .min_by(f32::total_cmp);
    match (level, enemy) {
        (Some(hit), Some(t)) if hit.0 <= t => Some(hit),
        (_, Some(t)) => Some((t, dir.map(|c| -c))),
        (hit, None) => hit,
    }
}

/// Take `damage` off `en`. True if that killed it.
fn hurt_enemy(en: &mut EnemyState, damage: i32, scene: &mut LdbScene, clips: usize) -> bool {
    let alive = en.health > 0.0;
    en.health -= damage as f32;
    en.hit_flash = 0.25;
    println!(
        "  3D Engine: Hit {}! (health={})",
        en.name,
        en.health.max(0.0)
    );
    if !alive || en.health > 0.0 {
        return false;
    }
    scene.kill_character(&en.name);
    // `DeathCause = PROJECTILEDEATH_RANDOM`.
    let pick = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.subsec_nanos() as usize);
    en.death = Some((pick % clips.max(1), 0.0, 0));
    true
}

/// Each gun in the player's hands, with its world vertices.
fn held_guns<'a>(
    scene: &LdbScene,
    models: &'a BTreeMap<String, crate::kf2::KfsMesh>,
    rig: &CharacterRig,
    world_pos: [f32; 3],
    yaw: f32,
    anim_time: f32,
    moving: bool,
) -> Vec<(&'a crate::kf2::KfsMesh, Vec<[f32; 3]>)> {
    let Some(name) = scene.inventory.current.as_ref() else {
        return Vec::new();
    };
    let Some(weapon) = models.get(name) else {
        return Vec::new();
    };
    let dual = scene.inventory.defs.get(name).is_some_and(|def| def.dual);
    let hands = if dual { 2 } else { 1 };
    rig.gun_bones[..hands]
        .iter()
        .filter_map(|bone| {
            let verts = held_weapon_verts(rig, weapon, bone, world_pos, yaw, anim_time, moving)?;
            Some((weapon, verts))
        })
        .collect()
}

fn character_clip(rig: &CharacterRig, moving: bool) -> Option<&crate::kf2::Kf2Anim> {
    let (idle, walk) = if rig.two_handed {
        (rig.two_handed_idle.as_ref(), rig.two_handed_walk.as_ref())
    } else {
        (None, None)
    };
    let idle = idle.or(rig.idle.as_ref());
    let walk = rig.motion.as_deref().or(walk).or(rig.walk.as_ref());
    if moving {
        walk.or(idle)
    } else {
        rig.action.as_ref().or(idle).or(walk)
    }
}

pub(crate) fn clip_seconds(clip: &crate::kf2::Kf2Anim) -> f32 {
    clip.bones
        .first()
        .filter(|bone| bone.fps > 0.0)
        .map_or(0.0, |bone| bone.duration as f32 / bone.fps)
}

/// A script's order to `en`.
fn obey(
    en: &mut EnemyState,
    order: &crate::scene::CharacterOrder,
    scene: &LdbScene,
    rooms: &BTreeMap<i32, [f32; 3]>,
) {
    match order {
        crate::scene::CharacterOrder::SetHealth(health) => en.health = health * en.max_health,
        // Enemies take these once they fight: nothing sends them to one in the Tutorial.
        crate::scene::CharacterOrder::AddHealth(_)
        | crate::scene::CharacterOrder::CauseDamage(_)
        | crate::scene::CharacterOrder::SetImmortal(_)
        | crate::scene::CharacterOrder::SetDodgeImmortal(_)
        | crate::scene::CharacterOrder::SetInvulnerable(_) => {}
        crate::scene::CharacterOrder::Teleport(place) => {
            let found = scene
                .jumppoints
                .iter()
                .position(|jp| scene.jumppoint_place(jp).eq_ignore_ascii_case(place));
            if let Some(index) = found {
                let [x, y, z] = spawn_pos(scene, rooms, index);
                en.world_pos = [x, y - EYE_HEIGHT, z];
                en.yaw = scene.jumppoints[index].yaw();
                en.room_id = scene.jumppoints[index].room_id;
            }
        }
    }
}

/// `dir` thrown off by `accuracy`, as retail does at `0x7068cd`: a random unit vector scaled by
/// `rand * Accuracy / 2500` added to the aim, then normalised again.
fn spread_aim(dir: [f32; 3], accuracy: f32, seed: &mut u32) -> [f32; 3] {
    let offset = loop {
        let v: [f32; 3] = std::array::from_fn(|_| crate::particles::random(seed) * 2.0 - 1.0);
        let length = dot3(v, v).sqrt();
        if length > 1e-6 && length <= 1.0 {
            break v.map(|c| c / length);
        }
    };
    let scale = crate::particles::random(seed) * accuracy / 2500.0;
    let spread: [f32; 3] = std::array::from_fn(|c| dir[c] + offset[c] * scale);
    let length = dot3(spread, spread).sqrt();
    spread.map(|c| c / length)
}

/// A shot's `locals` at `time`, turned toward where the character aims.
fn aimed_locals(
    rig: &CharacterRig,
    locals: crate::kf2::BoneLocals,
    time: f32,
) -> crate::kf2::BoneLocals {
    let Some(aim) = rig.aim.as_ref() else {
        return locals;
    };
    let (clip, how_far) = aim.toward(rig.aim_pitch);
    let mut aimed = crate::kf2::sample_locals(clip, time);
    crate::clips::mix_locals(&locals, &mut aimed, how_far);
    aimed
}

/// The hierarchy and bone locals of a character now: its clip, with the upper bones of a running
/// character's action laid over the run.
fn character_locals(
    rig: &CharacterRig,
    moving: bool,
    time: f32,
) -> Option<(&crate::kf2::Kf2Anim, crate::kf2::BoneLocals)> {
    let clip = character_clip(rig, moving)?;
    let mut locals = crate::kf2::sample_locals(clip, time);
    if rig.action.is_some() && !moving {
        locals = aimed_locals(rig, locals, time);
    }
    if let Some(action) = rig.action.as_ref().filter(|_| moving) {
        let upper = &rig.upper_bones[usize::from(rig.two_handed_bones)];
        let action = crate::kf2::sample_locals(action, rig.action_time);
        let mut action = aimed_locals(rig, action, rig.action_time);
        for bone in upper {
            if let Some(local) = action.remove(bone) {
                locals.insert(bone.clone(), local);
            }
        }
    }
    rig.fade.mix(&mut locals);
    Some((clip, locals))
}

/// How much of a shooting cycle a frame uses. The weapon files say of the multiplier: near 0
/// "shooting will slow down when time slows", near 1 it "will happen with constant speed".
fn cycle_seconds(real: f32, world: f32, multiplier: f32) -> f32 {
    world + (real - world) * multiplier
}

/// Where the player's current clip is.
fn player_clip_time(
    rig: &CharacterRig,
    moving: bool,
    walk_phase: f32,
    anim_clock: f32,
    action_time: f32,
) -> f32 {
    match (moving, &rig.action) {
        (true, _) => walk_phase,
        (false, Some(_)) => action_time,
        (false, None) => anim_clock,
    }
}

/// World position and rotation (columns) of `bone`, for a character at `world_pos` turned `yaw`.
fn bone_frame(
    anim: &crate::kf2::Kf2Anim,
    locals: &crate::kf2::BoneLocals,
    bone: &str,
    world_pos: [f32; 3],
    yaw: f32,
) -> Option<crate::particles::Frame> {
    let (rot, pos) = *crate::kf2::compose_world(anim, locals).get(&bone.to_ascii_lowercase())?;
    let (sy, cy) = yaw.sin_cos();
    let turn = |[x, y, z]: [f32; 3]| [x * cy + z * sy, y, z * cy - x * sy];
    let [x, y, z] = turn(pos);
    let columns = [0, 3, 6].map(|c| turn([rot[c], rot[c + 1], rot[c + 2]]));
    Some((
        [world_pos[0] + x, world_pos[1] + y, world_pos[2] + z],
        std::array::from_fn(|i| columns[i / 3][i % 3]),
    ))
}

/// Queue a `sounds/weapons.txt` cue heard from `at`.
fn queue_weapon_sound(scene: &mut LdbScene, cue: &str, at: [f32; 3]) {
    queue_sound(scene, "weapons", cue, at);
}

/// Queue a cue of `sounds/{catalog}.txt` heard from `at`.
fn queue_sound(scene: &mut LdbScene, catalog: &str, cue: &str, at: [f32; 3]) {
    scene.sound_queue.push(crate::scene::QueuedSound {
        category: catalog.to_string(),
        cue: cue.to_string(),
        filename: None,
        emitter: String::new(),
        looping: false,
        range: None,
        at: Some(at),
    });
}

/// `weapon` on the `Gun` bone of `rig`, in the world (`LeftHandGunBone` and `RightHandGunBone`
/// name the bones of dual weapons).
pub(crate) fn held_weapon_verts(
    rig: &CharacterRig,
    weapon: &crate::kf2::KfsMesh,
    bone: &str,
    world_pos: [f32; 3],
    yaw: f32,
    anim_time: f32,
    moving: bool,
) -> Option<Vec<[f32; 3]>> {
    let (anim, locals) = character_locals(rig, moving, anim_time)?;
    let posed = rig
        .gun_clip
        .as_ref()
        .map(|(clip, time)| crate::kf2::part_verts(weapon, clip, *time));
    let rest = posed.as_ref().unwrap_or(&weapon.vertices);
    let held = crate::kf2::bone_verts(rest, anim, &locals, bone)?;
    let (sy, cy) = yaw.sin_cos();
    let place = |[x, y, z]: [f32; 3]| {
        [
            world_pos[0] + x * cy + z * sy,
            world_pos[1] + y,
            world_pos[2] - x * sy + z * cy,
        ]
    };
    Some(held.into_iter().map(place).collect())
}

/// A body in its death clip, held on the last key once it ends.
fn dying_world_verts(
    rig: &CharacterRig,
    death: &crate::kf2::DeathClip,
    world_pos: [f32; 3],
    yaw: f32,
    time: f32,
) -> Vec<[f32; 3]> {
    let (sy, cy) = yaw.sin_cos();
    let time = time.min(crate::kf2::clip_end(&death.clip));
    pose_mesh(
        &rig.mesh,
        rig.skin.as_ref(),
        rig.bind.as_ref(),
        Some(&death.clip),
        time,
    )
    .into_iter()
    .map(|[x, y, z]| {
        [
            world_pos[0] + x * cy + z * sy,
            world_pos[1] + y,
            world_pos[2] - x * sy + z * cy,
        ]
    })
    .collect()
}

pub(crate) fn character_world_verts(
    rig: &CharacterRig,
    world_pos: [f32; 3],
    yaw: f32,
    anim_time: f32,
    moving: bool,
    dead: bool,
) -> Vec<[f32; 3]> {
    let (sy, cy) = yaw.sin_cos();
    let mesh = &rig.mesh;
    let pose = character_locals(rig, moving, anim_time).filter(|_| !dead);
    let posed = match pose {
        Some((anim, locals)) => {
            crate::kf2::pose_locals(mesh, rig.skin.as_ref(), rig.bind.as_ref(), anim, &locals)
        }
        None => mesh.vertices.clone(),
    };
    posed
        .iter()
        .map(|&[lx, ly, lz]| {
            let (px, py, pz) = if dead {
                // Lie flat on the floor when dead
                (lx, 0.18 + lz.abs() * 0.5, ly - 0.85)
            } else {
                (lx, ly, lz)
            };
            [
                world_pos[0] + px * cy + pz * sy,
                world_pos[1] + py,
                world_pos[2] - px * sy + pz * cy,
            ]
        })
        .collect()
}

/// Unit front normal of a posed character triangle. KFS fronts are wound so that
/// `(b - a) x (c - a)` points out of the mesh (188 of 193 head triangles of `Payne_Max_L0`).
/// Zero for what is drawn two-sided: dead bodies, whose lie-flat placement mirrors the mesh, and
/// held weapons.
pub(crate) fn character_face_normal(
    world: &[[f32; 3]],
    [a, b, c]: [usize; 3],
    two_sided: bool,
) -> [f32; 3] {
    let (Some(&a), Some(&b), Some(&c)) = (world.get(a), world.get(b), world.get(c)) else {
        return [0.0; 3];
    };
    if two_sided {
        return [0.0; 3];
    }
    let (e1, e2) = (sub3(b, a), sub3(c, a));
    let n = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let len = dot3(n, n).sqrt();
    if len < 1e-12 {
        return [0.0; 3];
    }
    n.map(|x| x / len)
}

/// Per-vertex diffuse of a posed character: the retail point-light selection and D3D diffuse
/// (#37), or a flat 1.35 while a hit flash lasts.
pub(crate) fn character_vertex_light(
    mesh: &crate::kf2::KfsMesh,
    world: &[[f32; 3]],
    world_pos: [f32; 3],
    hit_flash: bool,
    lights: &[ScenePointLight],
    ambient: [f32; 3],
) -> Vec<[f32; 3]> {
    if hit_flash {
        return vec![[1.35; 3]; world.len()];
    }
    let normals = smooth_normals(world, &mesh.faces);
    let radius = world
        .iter()
        .map(|p| {
            ((p[0] - world_pos[0]).powi(2)
                + (p[1] - world_pos[1]).powi(2)
                + (p[2] - world_pos[2]).powi(2))
            .sqrt()
        })
        .fold(0.0f32, f32::max);
    let picked = select_object_lights(lights, world_pos, radius);
    world
        .iter()
        .zip(&normals)
        .map(|(p, n)| {
            // D3D fixed function: white material ambient × D3DRS_AMBIENT + Σ light diffuse, clamped.
            let d = vertex_diffuse(*p, *n, &picked);
            std::array::from_fn(|c| (d[c] + ambient[c] / 255.0).min(1.0))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn render_kfs_character(
    mesh: &crate::kf2::KfsMesh,
    world: &[[f32; 3]],
    world_pos: [f32; 3],
    two_sided: bool,
    hit_flash: bool,
    lights: &[ScenePointLight],
    ambient: [f32; 3],
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

    // Pre-transform character vertices into camera space
    let mut cam_pts = Vec::with_capacity(world.len());
    for &[wx, wy, wz] in world {
        let dx = wx - cam_pos[0];
        let dy = wy - cam_pos[1];
        let dz = wz - cam_pos[2];
        let cx = dx * cam_right[0] + dy * cam_right[1] + dz * cam_right[2];
        let cy_c = dx * cam_up[0] + dy * cam_up[1] + dz * cam_up[2];
        let cz = dx * cam_fwd[0] + dy * cam_fwd[1] + dz * cam_fwd[2];
        cam_pts.push([cx, cy_c, cz]);
    }

    let vertex_light = character_vertex_light(mesh, world, world_pos, hit_flash, lights, ambient);
    let mut tris = Vec::new();
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
        let n = character_face_normal(world, face.indices, two_sided);
        if !wireframe && dot3(sub3(world[i0], cam_pos), n) >= 0.05 {
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
                tris.push(QueuedTri {
                    p: [proj[0], proj[k], proj[k + 1]],
                    tex,
                    alpha_reference: None,
                    lmap: None,
                    write_z: true,
                    depth_bias: 0.0,
                    blended: false,
                });
            }
        }
    }
    flush_tris(&tris, fb, zb, raster_threads());
}

/// Queue and draw one batch at once (the reference path the tests compare against).
#[cfg(test)]
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
    let mut tris = Vec::new();
    queue_poly_batch(
        polys,
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
        &mut tris,
    );
    flush_tris(&tris, fb, zb, raster_threads());
}

/// Project, clip and queue `polys` (opaque first, then non-z-writing), as `render_poly_batch`
/// draws them. Wireframe lines go straight to `fb`.
#[allow(clippy::too_many_arguments)]
fn queue_poly_batch<'a>(
    polys: &[ScenePoly],
    mat: [f32; 9],
    origin: [f32; 3],
    scene: &'a LdbScene,
    cam_pos: [f32; 3],
    cam_right: [f32; 3],
    cam_up: [f32; 3],
    cam_fwd: [f32; 3],
    fov_scale: f32,
    use_lightmaps: bool,
    wireframe: bool,
    cull_backfaces: bool,
    fb: &mut [u32],
    tris: &mut Vec<QueuedTri<'a>>,
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
                tris,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_poly<'a>(
    poly: &ScenePoly,
    policy: MaterialPolicy,
    mat: [f32; 9],
    origin: [f32; 3],
    scene: &'a LdbScene,
    cam_pos: [f32; 3],
    cam_right: [f32; 3],
    cam_up: [f32; 3],
    cam_fwd: [f32; 3],
    fov_scale: f32,
    use_lightmaps: bool,
    wireframe: bool,
    cull_backfaces: bool,
    fb: &mut [u32],
    tris: &mut Vec<QueuedTri<'a>>,
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

    let shade = poly_shade(wn);
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
            tris.push(QueuedTri {
                p: [p0, p1, p2],
                tex,
                alpha_reference,
                lmap,
                write_z: policy.writes_z,
                depth_bias: policy.detail_offset as f32 * DETAIL_OFFSET_M,
                blended: alpha_reference.is_some() && material.is_some_and(|m| m.blended),
            });
        }
    }
}

/// Flat shade for polygons without a lightmap, from the world normal.
pub(crate) fn poly_shade(wn: [f32; 3]) -> f32 {
    ((wn[0] * 0.35 + wn[1] * 0.75 + wn[2] * 0.55).abs() * 0.55 + 0.45).clamp(0.35, 1.0)
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

/// One projected triangle waiting for [`flush_tris`].
#[derive(Clone, Copy)]
struct QueuedTri<'a> {
    p: [ProjVert; 3],
    tex: Option<&'a DecodedTexture>,
    alpha_reference: Option<u8>,
    lmap: Option<&'a DecodedLightmap>,
    write_z: bool,
    depth_bias: f32,
    /// Mix with the frame by texel alpha after the alpha test (`BlendedAlphaTest`).
    blended: bool,
}

/// Rasterise `tris` in order. With more than one thread the frame is split into row bands and
/// each band runs the whole list in order, so every pixel sees the same sequence as a serial
/// pass and the result is identical.
fn flush_tris(tris: &[QueuedTri], fb: &mut [u32], zb: &mut [f32], threads: usize) {
    if threads <= 1 || tris.len() < 16 {
        for t in tris {
            raster_band(t, fb, zb, 0, H);
        }
        return;
    }
    let rows = H.div_ceil(threads);
    std::thread::scope(|scope| {
        for (band, (fb_band, zb_band)) in fb
            .chunks_mut(rows * W)
            .zip(zb.chunks_mut(rows * W))
            .enumerate()
        {
            let y0 = band * rows;
            let y1 = (y0 + rows).min(H);
            scope.spawn(move || {
                for t in tris {
                    raster_band(t, fb_band, zb_band, y0, y1);
                }
            });
        }
    });
}

/// Worker threads for the software rasterizer.
fn raster_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().min(16))
}

/// Rasterise rows `y0..y1` of `t` into band buffers whose row 0 is frame row `y0`.
/// Barycentric slack, so rounding cannot reject a pixel on a shared edge from both triangles.
const EDGE_SLACK: f32 = -1.0e-5;

fn raster_band(t: &QueuedTri, fb: &mut [u32], zb: &mut [f32], y0: usize, y1: usize) {
    let [p0, p1, p2] = t.p;
    let (tex, alpha_reference, lmap, write_z, depth_bias) =
        (t.tex, t.alpha_reference, t.lmap, t.write_z, t.depth_bias);
    let area = (p1.sx - p0.sx) * (p2.sy - p0.sy) - (p2.sx - p0.sx) * (p1.sy - p0.sy);
    if area.abs() < 1e-3 {
        return;
    }
    let inv_area = 1.0 / area;

    let min_x = p0.sx.min(p1.sx).min(p2.sx).floor().max(0.0) as usize;
    let max_x = p0.sx.max(p1.sx).max(p2.sx).ceil().min((W - 1) as f32) as usize;
    let min_y = (p0.sy.min(p1.sy).min(p2.sy).floor().max(0.0) as usize).max(y0);
    let max_y =
        (p0.sy.max(p1.sy).max(p2.sy).ceil().min((H - 1) as f32) as usize).min(y1.saturating_sub(1));
    if min_x > max_x || min_y > max_y || y1 <= y0 {
        return;
    }

    for y in min_y..=max_y {
        let py = y as f32 + 0.5;
        let row = (y - y0) * W;
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let w0 = ((p1.sx - px) * (p2.sy - py) - (p2.sx - px) * (p1.sy - py)) * inv_area;
            let w1 = ((p2.sx - px) * (p0.sy - py) - (p0.sx - px) * (p2.sy - py)) * inv_area;
            let w2 = 1.0 - w0 - w1;
            if w0 < EDGE_SLACK || w1 < EDGE_SLACK || w2 < EDGE_SLACK {
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
                sample_texture(t, u, v)
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
                let lit = lightmap_modulate(base_rgb, sample_lightmap(lm, lu, lv));
                r = ((lit >> 16) & 0xff) as f32;
                g = ((lit >> 8) & 0xff) as f32;
                b = (lit & 0xff) as f32;
            } else {
                let light = [0, 1, 2].map(|c| {
                    (w0 * p0.light_over_z[c] + w1 * p1.light_over_z[c] + w2 * p2.light_over_z[c])
                        * z
                });
                r *= light[0];
                g *= light[1];
                b *= light[2];
            }

            if t.blended {
                let a = ((base_rgb >> 24) & 0xff) as f32 / 255.0;
                let dst = fb[idx];
                r = r * a + ((dst >> 16) & 0xff) as f32 * (1.0 - a);
                g = g * a + ((dst >> 8) & 0xff) as f32 * (1.0 - a);
                b = b * a + (dst & 0xff) as f32 * (1.0 - a);
            }
            let ri = (r.min(255.0) as u32) << 16;
            let gi = (g.min(255.0) as u32) << 8;
            let bi = b.min(255.0) as u32;
            fb[idx] = ri | gi | bi;
        }
    }
}

/// One mesh of a world sphere and its bitmaps, for [`bake_sky`].
pub(crate) struct SkyLayer<'a> {
    mesh: &'a crate::kf2::SkyMesh,
    colour: &'a DecodedTexture,
    opacity: Option<&'a DecodedTexture>,
}

/// Size of the baked sky: 360° by 180°, as wide as the widest world sphere bitmap.
const SKY_SIZE: (u32, u32) = (2048, 1024);

/// Paints the world sphere's meshes, in order, onto one latitude-longitude image (`sky_uv`), so
/// both renderers draw any sphere the same way and need no mesh pass of their own.
pub(crate) fn bake_sky(layers: &[SkyLayer], width: u32, height: u32) -> DecodedTexture {
    let mut sky = DecodedTexture {
        path: String::new(),
        flag: 4,
        width,
        height,
        pixels: vec![0; (width * height) as usize],
    };
    for layer in layers {
        for tri in layer.mesh.indices.as_chunks::<3>().0 {
            paint_sky_triangle(layer, *tri, &mut sky);
        }
    }
    sky
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// The texel of `tex` at `uv`, wrapping both ways.
fn sky_texel(tex: &DecodedTexture, uv: [f32; 2]) -> u32 {
    let x = (uv[0].rem_euclid(1.0) * tex.width as f32) as u32;
    let y = (uv[1].rem_euclid(1.0) * tex.height as f32) as u32;
    tex.pixels[(y.min(tex.height - 1) * tex.width + x.min(tex.width - 1)) as usize]
}

fn paint_sky_triangle(layer: &SkyLayer, tri: [usize; 3], sky: &mut DecodedTexture) {
    use std::f32::consts::{PI, TAU};
    let mesh = layer.mesh;
    let p = tri.map(|i| mesh.vertices[i]);
    let uv = tri.map(|i| mesh.uvs[i]);
    let normal = cross3(sub3(p[1], p[0]), sub3(p[2], p[0]));
    let (area, plane) = (dot3(normal, normal), dot3(p[0], normal));
    if area == 0.0 || plane == 0.0 {
        return;
    }
    let edges = [cross3(p[0], p[1]), cross3(p[1], p[2]), cross3(p[2], p[0])];
    let side = dot3(edges[0], p[2]).signum();
    let covers = |d: [f32; 3]| edges.iter().all(|e| dot3(*e, d) * side >= 0.0);

    // Texels to test: the span of the triangle's outline, padded by a texel or two.
    let (w, h) = (sky.width as i32, sky.height as i32);
    let yaw0 = p[0][0].atan2(p[0][2]);
    let (mut top, mut bottom) = (f32::MAX, f32::MIN);
    let (mut west, mut east) = (f32::MAX, f32::MIN);
    let mut on_pole = false;
    for k in 0..3 {
        for step in 0..16 {
            let t = step as f32 / 16.0;
            let (a, b) = (p[k], p[(k + 1) % 3]);
            let d = normalize([0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t));
            let row = (0.5 - d[1].clamp(-1.0, 1.0).asin() / PI) * h as f32;
            (top, bottom) = (top.min(row - 2.0), bottom.max(row + 2.0));
            let turn = (d[0].atan2(d[2]) - yaw0 + PI).rem_euclid(TAU) - PI;
            (west, east) = (west.min(turn), east.max(turn));
            on_pole |= d[1].abs() > 0.999;
        }
    }
    let mut all_yaws = east - west >= PI || on_pole;
    if covers([0.0, 1.0, 0.0]) {
        (top, all_yaws) = (0.0, true);
    }
    if covers([0.0, -1.0, 0.0]) {
        (bottom, all_yaws) = (h as f32, true);
    }
    let (first, count) = if all_yaws {
        (0, w)
    } else {
        let scale = w as f32 / TAU;
        let first = ((yaw0 + west + PI) * scale) as i32 - 2;
        (first, ((east - west) * scale) as i32 + 5)
    };

    for y in (top.max(0.0) as i32)..(bottom.min(h as f32) as i32) {
        let lat = (0.5 - (y as f32 + 0.5) / h as f32) * PI;
        let (sin_lat, cos_lat) = lat.sin_cos();
        for column in first..first + count.min(w) {
            let x = column.rem_euclid(w);
            let yaw = ((x as f32 + 0.5) / w as f32 - 0.5) * TAU;
            let d = [yaw.sin() * cos_lat, sin_lat, yaw.cos() * cos_lat];
            let along = dot3(d, normal);
            if along == 0.0 || !covers(d) {
                continue;
            }
            let reach = plane / along;
            if reach <= 0.0 {
                continue;
            }
            let hit = d.map(|c| c * reach);
            let (a, b) = (sub3(p[0], hit), sub3(p[1], hit));
            let c = sub3(p[2], hit);
            let w0 = dot3(cross3(b, c), normal) / area;
            let w1 = dot3(cross3(c, a), normal) / area;
            let w2 = 1.0 - w0 - w1;
            let at = [
                uv[0][0] * w0 + uv[1][0] * w1 + uv[2][0] * w2,
                uv[0][1] * w0 + uv[1][1] * w1 + uv[2][1] * w2,
            ];
            let colour = sky_texel(layer.colour, at);
            let index = (y * w + x) as usize;
            sky.pixels[index] = match layer.opacity {
                None => colour,
                Some(map) => {
                    let alpha = sky_texel(map, at) & 0xff;
                    let mix = |shift: u32| {
                        let front = (colour >> shift) & 0xff;
                        let back = (sky.pixels[index] >> shift) & 0xff;
                        ((front * alpha + back * (255 - alpha)) / 255) << shift
                    };
                    mix(16) | mix(8) | mix(0)
                }
            };
        }
    }
}

/// Baked sky coordinates of a view direction: yaw across, zenith at the top, nadir at the bottom.
pub(crate) fn sky_uv(dir: [f32; 3]) -> [f32; 2] {
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2])
        .sqrt()
        .max(1e-8);
    let x = dir[0] / len;
    let y = dir[1] / len;
    let z = dir[2] / len;
    let u = (x.atan2(z) / std::f32::consts::TAU + 0.5).rem_euclid(1.0);
    let v = (0.5 - y.clamp(-1.0, 1.0).asin() / std::f32::consts::PI).clamp(0.0, 0.999);
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
            let [u, v] = sky_uv(dir);
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
    let meshes = crate::kf2::parse_world_sphere(&kf2)?;
    let mut tex_dirs: Vec<std::path::PathBuf> = roots.iter().map(|r| r.join("textures")).collect();
    tex_dirs.extend(roots.iter().cloned());
    let bitmap = |image: &String| {
        let bytes = read_ci(&tex_dirs, image)
            .or_else(|| crate::kf2::load_bytes_from_ras(level_path, &[image]))?;
        let (width, height, pixels) = decode_texture_blob(&bytes)?;
        Some(DecodedTexture {
            path: image.clone(),
            flag: 4,
            width,
            height,
            pixels,
        })
    };
    let bitmaps: Vec<Vec<Option<DecodedTexture>>> = meshes
        .iter()
        .map(|mesh| mesh.textures.iter().map(&bitmap).collect())
        .collect();
    let layers: Vec<SkyLayer> = meshes
        .iter()
        .zip(&bitmaps)
        .filter_map(|(mesh, images)| {
            Some(SkyLayer {
                mesh,
                colour: images.first()?.as_ref()?,
                opacity: images.get(1).and_then(Option::as_ref),
            })
        })
        .collect();
    Some(bake_sky(&layers, SKY_SIZE.0, SKY_SIZE.1))
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
pub(crate) fn test_empty_scene() -> LdbScene {
    tests::empty_scene()
}

/// View looking along `fwd` with world up, right = up × forward (the player camera's basis).
#[cfg(test)]
pub(crate) fn test_view(pos: [f32; 3], fwd: [f32; 3]) -> crate::gpu::GpuView {
    let up = [0.0, 1.0, 0.0];
    let right = [
        up[1] * fwd[2] - up[2] * fwd[1],
        up[2] * fwd[0] - up[0] * fwd[2],
        up[0] * fwd[1] - up[1] * fwd[0],
    ];
    (pos, right, up, fwd)
}

#[cfg(test)]
pub(crate) fn test_glass_quad(hidden: bool) -> crate::scene::SceneDynamicMesh {
    tests::glass_quad(hidden)
}

/// Software render of the dynamic meshes only (the hardware renderer's reference).
#[cfg(test)]
pub(crate) fn test_software_dynamic(
    scene: &LdbScene,
    (pos, right, up, fwd): crate::gpu::GpuView,
) -> (Vec<u32>, Vec<f32>) {
    let mut fb = vec![0u32; W * H];
    let mut zb = vec![f32::INFINITY; W * H];
    let rooms: BTreeMap<i32, [f32; 3]> =
        scene.rooms.iter().map(|r| (r.id, r.world_origin)).collect();
    draw_dynamic_meshes(
        scene,
        &rooms,
        pos,
        right,
        up,
        fwd,
        fov_scale(1.0),
        true,
        false,
        &mut fb,
        &mut zb,
    );
    (fb, zb)
}

/// Software render of one character facing +z at `pos` (the hardware renderer's reference).
#[cfg(test)]
pub(crate) fn test_software_character(
    rig: &CharacterRig,
    pos: [f32; 3],
    hit_flash: bool,
    (cam, right, up, fwd): crate::gpu::GpuView,
) -> (Vec<u32>, Vec<f32>) {
    let mut fb = vec![0u32; W * H];
    let mut zb = vec![f32::INFINITY; W * H];
    let verts = character_world_verts(rig, pos, 0.0, 0.0, false, false);
    render_kfs_character(
        &rig.mesh,
        &verts,
        pos,
        false,
        hit_flash,
        &[],
        [0.0; 3],
        cam,
        right,
        up,
        fwd,
        fov_scale(1.0),
        false,
        &mut fb,
        &mut zb,
    );
    (fb, zb)
}

/// The software sky for `view` (the hardware renderer's reference).
#[cfg(test)]
pub(crate) fn test_software_sky(
    tex: &DecodedTexture,
    (_, right, up, fwd): crate::gpu::GpuView,
) -> Vec<u32> {
    let mut fb = vec![0u32; W * H];
    paint_sky_strip(tex, right, up, fwd, fov_scale(1.0), &mut fb);
    fb
}

/// Software render of the static meshes only (the hardware renderer's reference).
#[cfg(test)]
pub(crate) fn test_software_static(
    scene: &LdbScene,
    (pos, right, up, fwd): crate::gpu::GpuView,
) -> (Vec<u32>, Vec<f32>) {
    let mut fb = vec![0u32; W * H];
    let mut zb = vec![f32::INFINITY; W * H];
    const IDENT_MAT: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    for sm in static_meshes_in_view(scene, scene.attached_camera) {
        render_poly_batch(
            &sm.polys,
            IDENT_MAT,
            sm.world_origin,
            scene,
            pos,
            right,
            up,
            fwd,
            fov_scale(1.0),
            true,
            false,
            true,
            &mut fb,
            &mut zb,
        );
    }
    (fb, zb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SceneDynamicMesh, SceneMaterial, ScenePoly, SceneVertex};

    fn shake(keys: &[(u32, [f32; 3])], duration: u32) -> CameraShake {
        let ident = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        CameraShake {
            anim: crate::kf2::Kf2Anim {
                bones: vec![crate::kf2::AnimBone {
                    name: "Camera01".into(),
                    parent: None,
                    duration,
                    fps: 30.0,
                    keys: keys
                        .iter()
                        .map(|&(frame, pos)| crate::kf2::AnimKeyframe {
                            frame,
                            mat: ident,
                            pos,
                        })
                        .collect(),
                }],
            },
            time: 0.0,
        }
    }

    #[test]
    fn a_camera_shake_offsets_from_its_first_key_and_ends_with_the_path() {
        let mut path = shake(&[(0, [0.5, 0.0, 0.0]), (30, [0.5, 0.3, 0.0])], 31);
        assert_eq!(path.delta().map(|d| d.1), Some([0.0, 0.0, 0.0]));
        path.time = 0.5;
        let (_, offset) = path.delta().expect("still playing");
        assert!((offset[1] - 0.15).abs() < 1e-6, "offset={offset:?}");
        path.time = 31.0 / 30.0;
        assert_eq!(path.delta(), None);
    }

    #[test]
    fn retail_shake_quick_plays_the_explosion_path_and_ends() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let Some(text) =
            crate::read_game_bytes(&game, r"\data\database\camerapaths\camerapaths.txt")
        else {
            return;
        };
        let text = String::from_utf8_lossy(&text).into_owned();
        let mut scene = LdbScene::default();
        assert!(scene.dispatch_message("::x", "player->cam_animateinplace(shake_quick);"));
        let mut shake = None;
        start_camera_shakes(&mut scene, &game, &text, &mut shake);
        let mut shake = shake.expect("shake_quick loads effects\\explosion.kf2");
        let mut largest = 0.0f32;
        let mut frames = 0;
        while let Some((_, offset)) = shake.delta() {
            largest = offset.iter().fold(largest, |m, c| m.max(c.abs()));
            shake.time += 1.0 / 60.0;
            frames += 1;
            assert!(frames < 6000, "the path must end");
        }
        assert!(largest > 0.0 && largest < 0.5, "largest offset {largest} m");
    }

    #[test]
    fn a_death_cam_sits_in_the_frame_of_its_character() {
        let back = [-1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, -1.0];
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5);
        let (eye, _, up, fwd) = parented_view([10.0, 0.0, 0.0], 0.0, (back, [0.0, 0.5, 2.0]));
        assert!(close(eye, [10.0, 0.5, 2.0]), "eye={eye:?}");
        assert!(close(fwd, [0.0, 0.0, -1.0]), "fwd={fwd:?}");
        assert!(close(up, [0.0, 1.0, 0.0]), "up={up:?}");
        let half_turn = std::f32::consts::FRAC_PI_2;
        let (eye, right, _, fwd) =
            parented_view([10.0, 0.0, 0.0], half_turn, (back, [0.0, 0.5, 2.0]));
        assert!(close(eye, [12.0, 0.5, 0.0]), "eye={eye:?}");
        assert!(close(fwd, [-1.0, 0.0, 0.0]), "fwd={fwd:?}");
        assert!(close(right, [0.0, 0.0, 1.0]), "right={right:?}");
    }

    #[test]
    fn shaken_view_moves_and_turns_the_eye_in_camera_space() {
        let view = (
            [1.0, 2.0, 3.0],
            [0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        );
        let ident = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        assert_eq!(shaken_view(view, (ident, [0.0; 3])), view);
        let (pos, right, up, fwd) = shaken_view(view, (ident, [1.0, 0.5, 2.0]));
        assert_eq!(pos, [3.0, 2.5, 2.0]);
        assert_eq!((right, up, fwd), (view.1, view.2, view.3));
    }

    fn flash(cycle_length: f32) -> openpfx::light::RpsLight {
        openpfx::light::RpsLight {
            cycle_length,
            color: vec![[255.0, 0.0, 0.0], [0.0, 0.0, 255.0]],
            falloff: vec![4.0, 8.0],
            offset: [0.0, 1.0, 0.0],
        }
    }

    fn siren_scene() -> LdbScene {
        let mut scene = LdbScene::default();
        let mut car = glass_quad(false);
        car.name = "::startroom::car.DO".into();
        car.room_id = 3;
        car.world_pos = [2.0, 0.0, 0.0];
        car.local_start_pos = [0.0; 3];
        scene.dynamic_meshes.push(car);
        scene
    }

    #[test]
    fn a_particle_light_sits_on_its_emitter_with_the_sampled_colour_and_radius() {
        let scene = siren_scene();
        let rooms = BTreeMap::from([(3, [10.0, 0.0, 0.0])]);
        let mut lights = ParticleLights::default();
        lights.start("::startroom::car.DO".into(), vec![flash(2.0)], true);
        lights.step(1.0);
        let out = lights.point_lights(&scene, &rooms);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pos, [12.0, 1.0, 0.0]);
        assert_eq!(out[0].color, [127.5, 0.0, 127.5, 255.0]);
        assert_eq!(out[0].radius, 6.0);
        assert_eq!(out[0].intensity, 1.0);
    }

    fn clip(bone: &str, pos: [f32; 3]) -> crate::kf2::Kf2Anim {
        crate::kf2::Kf2Anim {
            bones: vec![crate::kf2::AnimBone {
                name: bone.to_string(),
                parent: None,
                duration: 30,
                fps: 15.0,
                keys: vec![crate::kf2::AnimKeyframe {
                    frame: 0,
                    mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                    pos,
                }],
            }],
        }
    }

    #[test]
    fn an_action_clip_replaces_the_idle_but_not_the_run() {
        let mesh = crate::kf2::KfsMesh {
            name: String::new(),
            vertices: Vec::new(),
            faces: Vec::new(),
            material_textures: Vec::new(),
            parts: Vec::new(),
        };
        let rig = CharacterRig {
            mesh,
            skin: None,
            idle: Some(clip("idle", [0.0; 3])),
            walk: Some(clip("run", [0.0; 3])),
            bind: None,
            action: Some(clip("shoot", [0.0; 3])),
            motion: None,
            fade: Default::default(),
            gun_clip: None,
            aim: None,
            aim_pitch: 0.0,
            two_handed_idle: None,
            two_handed_walk: None,
            two_handed: false,
            max_health: 0,
            action_time: 0.0,
            upper_bones: [Vec::new(), Vec::new()],
            two_handed_bones: false,
            gun_bones: [String::new(), String::new()],
        };
        let name = |moving| character_clip(&rig, moving).map(|c| c.bones[0].name.as_str());
        assert_eq!(name(false), Some("shoot"));
        assert_eq!(name(true), Some("run"));
    }

    #[test]
    fn the_camera_point_moves_toward_its_target_and_stops_on_it() {
        let from = [0.0, 1.75, 0.0];
        let to = [0.0, 0.75, 0.0];
        assert_eq!(approach(from, to, 0.25), [0.0, 1.5, 0.0]);
        assert_eq!(approach(from, to, 2.0), to);
    }

    #[test]
    fn a_chosen_motion_clip_replaces_the_run() {
        let rig = CharacterRig {
            mesh: crate::kf2::KfsMesh {
                name: String::new(),
                vertices: Vec::new(),
                faces: Vec::new(),
                material_textures: Vec::new(),
                parts: Vec::new(),
            },
            skin: None,
            idle: Some(clip("idle", [0.0; 3])),
            walk: Some(clip("run", [0.0; 3])),
            bind: None,
            action: None,
            motion: Some(std::sync::Arc::new(clip("strafe", [0.0; 3]))),
            fade: Default::default(),
            gun_clip: None,
            aim: None,
            aim_pitch: 0.0,
            two_handed_idle: None,
            two_handed_walk: None,
            two_handed: false,
            max_health: 0,
            action_time: 0.0,
            upper_bones: [Vec::new(), Vec::new()],
            two_handed_bones: false,
            gun_bones: [String::new(), String::new()],
        };
        let name = |moving| character_clip(&rig, moving).map(|c| c.bones[0].name.as_str());
        assert_eq!(name(true), Some("strafe"));
        assert_eq!(name(false), Some("idle"));
    }

    #[test]
    fn a_flash_darkens_the_frame_by_its_alpha() {
        let scene = LdbScene {
            screen_flash: Some(crate::scene::ScreenFlash {
                color: 0x0000_0080,
                fade_in: 0.0,
                hold: 1.0,
                fade_out: 0.0,
                age: 0.5,
            }),
            ..Default::default()
        };
        let mut fb = [0x00ff_ffffu32];
        draw_screen_flash(&scene, &mut fb);
        assert_eq!(fb[0], 0x007f_7f7f);
    }

    #[test]
    fn firing_slows_with_time_by_the_weapons_multiplier() {
        // A frame of 0.1 s in bullet time running at a tenth of the speed.
        assert_eq!(cycle_seconds(0.1, 0.01, 0.0), 0.01);
        assert_eq!(cycle_seconds(0.1, 0.01, 1.0), 0.1);
        assert!((cycle_seconds(0.1, 0.01, 0.25) - 0.0325).abs() < 1e-6);
    }

    #[test]
    fn a_shot_strays_within_its_accuracy() {
        let mut seed = 0x1234_5678;
        let aim = [0.0, 0.0, 1.0];
        assert_eq!(spread_aim(aim, 0.0, &mut seed), aim);
        let shots: Vec<[f32; 3]> = (0..200)
            .map(|_| spread_aim(aim, 150.0, &mut seed))
            .collect();
        // Pump shotgun: at most 150 / 2500 = 0.06 off a unit aim.
        let widest = shots.iter().map(|s| s[0].hypot(s[1])).fold(0.0, f32::max);
        assert!((0.02..=0.0601).contains(&widest), "{widest}");
    }

    #[test]
    fn a_running_character_takes_only_the_upper_bones_from_its_action() {
        let two_bones = |pelvis: [f32; 3], torso: [f32; 3]| {
            let mut anim = clip("Pelvis", pelvis);
            anim.bones.extend(clip("Torso", torso).bones);
            anim
        };
        let mesh = crate::kf2::KfsMesh {
            name: String::new(),
            vertices: Vec::new(),
            faces: Vec::new(),
            material_textures: Vec::new(),
            parts: Vec::new(),
        };
        let mut rig = CharacterRig {
            mesh,
            skin: None,
            idle: None,
            walk: Some(two_bones([1.0; 3], [2.0; 3])),
            bind: None,
            action: Some(two_bones([8.0; 3], [9.0; 3])),
            motion: None,
            fade: Default::default(),
            gun_clip: None,
            aim: None,
            aim_pitch: 0.0,
            two_handed_idle: None,
            two_handed_walk: None,
            two_handed: false,
            max_health: 0,
            action_time: 0.0,
            upper_bones: [vec!["torso".to_string()], Vec::new()],
            two_handed_bones: false,
            gun_bones: [String::new(), String::new()],
        };
        let (_, locals) = character_locals(&rig, true, 0.0).expect("pose");
        assert_eq!(locals["pelvis"].1, [1.0; 3]);
        assert_eq!(locals["torso"].1, [9.0; 3]);
        // Two-handed weapons have their own set, empty here.
        rig.two_handed_bones = true;
        let (_, locals) = character_locals(&rig, true, 0.0).expect("pose");
        assert_eq!(locals["torso"].1, [2.0; 3]);
    }

    #[test]
    fn a_gun_emitter_follows_the_gun_in_world_space() {
        let mut scene = LdbScene::default();
        let rot = [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0];
        scene.held_guns[1] = Some((rot, [1.0, 2.0, 3.0]));
        // Room 0 must not shift a position already in the world.
        let rooms = BTreeMap::from([(0, [100.0, 0.0, 0.0])]);
        let emitter = crate::scene::GUN_EMITTERS[1];
        assert_eq!(
            emitter_frame(&scene, &rooms, emitter),
            Some(([1.0, 2.0, 3.0], rot))
        );
        assert_eq!(
            emitter_frame(&scene, &rooms, crate::scene::GUN_EMITTERS[0]),
            None
        );
    }

    #[test]
    fn a_two_handed_weapon_uses_the_two_handed_clips() {
        let mesh = crate::kf2::KfsMesh {
            name: String::new(),
            vertices: Vec::new(),
            faces: Vec::new(),
            material_textures: Vec::new(),
            parts: Vec::new(),
        };
        let mut rig = CharacterRig {
            mesh,
            skin: None,
            idle: Some(clip("idle", [0.0; 3])),
            walk: Some(clip("run", [0.0; 3])),
            bind: None,
            action: None,
            motion: None,
            fade: Default::default(),
            gun_clip: None,
            aim: None,
            aim_pitch: 0.0,
            two_handed_idle: Some(clip("2_idle", [0.0; 3])),
            two_handed_walk: Some(clip("2_run", [0.0; 3])),
            two_handed: true,
            max_health: 0,
            action_time: 0.0,
            upper_bones: [Vec::new(), Vec::new()],
            two_handed_bones: false,
            gun_bones: [String::new(), String::new()],
        };
        let name = |rig: &CharacterRig, moving: bool| {
            let clip = character_clip(rig, moving).expect("clip");
            clip.bones[0].name.clone()
        };
        assert_eq!(name(&rig, false), "2_idle");
        assert_eq!(name(&rig, true), "2_run");
        rig.action = Some(clip("shoot", [0.0; 3]));
        assert_eq!(name(&rig, false), "shoot");
        rig.two_handed = false;
        rig.action = None;
        assert_eq!(name(&rig, false), "idle");
    }

    #[test]
    fn a_teleported_character_stands_on_the_start_point_and_faces_its_way() {
        let mut scene = empty_scene();
        scene.jumppoints.push(crate::scene::SceneJumppoint {
            name: "::Street::Tele_02".to_string(),
            room_id: 7,
            pos: [1.0, 0.0, 2.0],
            mat: [0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0],
            jp_type: 1,
        });
        let rooms = BTreeMap::from([(7, [10.0, 0.0, 20.0])]);
        let mut en = EnemyState {
            name: "::Teleport::E01".to_string(),
            room_id: 1,
            world_pos: [0.0; 3],
            yaw: 0.0,
            walk_phase: 0.0,
            health: 100.0,
            max_health: 80.0,
            hit_flash: 0.0,
            rig_index: 0,
            pursues: false,
            invulnerable: false,
            held: false,
            auto_aim_target: true,
            death: None,
        };
        let place = "::street::tele_02".to_string();
        obey(
            &mut en,
            &crate::scene::CharacterOrder::Teleport(place),
            &scene,
            &rooms,
        );
        assert_eq!(en.world_pos[0], 11.0);
        assert_eq!(en.world_pos[2], 22.0);
        assert_eq!(en.room_id, 7);
        assert!(
            (en.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
            "{}",
            en.yaw
        );
        let health = crate::scene::CharacterOrder::SetHealth(0.5);
        obey(&mut en, &health, &scene, &rooms);
        assert_eq!(en.health, 40.0);
    }

    fn target_at(z: f32) -> EnemyState {
        EnemyState {
            name: String::new(),
            room_id: 0,
            world_pos: [0.0, 0.0, z],
            yaw: 0.0,
            walk_phase: 0.0,
            health: 10.0,
            max_health: 10.0,
            hit_flash: 0.0,
            rig_index: 0,
            pursues: false,
            invulnerable: false,
            held: false,
            auto_aim_target: true,
            death: None,
        }
    }

    #[test]
    fn a_near_miss_inside_the_grown_capsule_turns_to_the_torso() {
        let enemies = [target_at(10.0)];
        let origin = [1.0, 1.0, 0.0];
        let aim = [0.0, 0.0, 1.0];
        assert_eq!(ray_hits_character(origin, aim, enemies[0].world_pos), None);
        let turned = auto_aim_at(&enemies, origin, aim, [4.0, 2.0], 100.0).expect("target");
        let torso = [0.0, ENEMY_HEIGHT / 2.0, 10.0];
        let to = sub3(torso, origin);
        let length = dot3(to, to).sqrt();
        for c in 0..3 {
            assert!((turned[c] - to[c] / length).abs() < 1e-5, "{turned:?}");
        }
    }

    #[test]
    fn auto_aim_needs_the_capsule_reach_and_a_living_target() {
        let origin = [2.0, 1.0, 0.0];
        let aim = [0.0, 0.0, 1.0];
        let enemies = [target_at(10.0)];
        assert_eq!(auto_aim_at(&enemies, origin, aim, [4.0, 2.0], 100.0), None);
        let origin = [1.0, 1.0, 0.0];
        assert_eq!(auto_aim_at(&enemies, origin, aim, [4.0, 2.0], 5.0), None);
        let mut dead = target_at(10.0);
        dead.health = 0.0;
        assert_eq!(auto_aim_at(&[dead], origin, aim, [4.0, 2.0], 100.0), None);
        let mut plain = target_at(10.0);
        plain.auto_aim_target = false;
        assert_eq!(auto_aim_at(&[plain], origin, aim, [4.0, 2.0], 100.0), None);
    }

    #[test]
    fn the_mouse_looks_only_while_the_player_has_the_controls() {
        assert_eq!(look_motion(Some((3.0, 4.0)), true), Some((3.0, 4.0)));
        assert_eq!(look_motion(Some((3.0, 4.0)), false), None);
        assert_eq!(look_motion(None, true), None);
    }

    #[test]
    fn a_shot_turns_toward_where_the_character_aims() {
        let mesh = crate::kf2::KfsMesh {
            name: String::new(),
            vertices: Vec::new(),
            faces: Vec::new(),
            material_textures: Vec::new(),
            parts: Vec::new(),
        };
        let aimed = |degrees: f32, y: f32| {
            let (sin, cos) = degrees.to_radians().sin_cos();
            let mut clip = clip("gun", [0.0, y, 0.0]);
            clip.bones[0].keys[0].mat = [1.0, 0.0, 0.0, 0.0, cos, -sin, 0.0, sin, cos];
            clip
        };
        let aim = crate::clips::AimClips::new(aimed(-90.0, 0.0), aimed(90.0, 2.0), "gun");
        let rig = CharacterRig {
            mesh,
            skin: None,
            idle: None,
            walk: None,
            bind: None,
            action: Some(clip("gun", [0.0, 1.0, 0.0])),
            motion: None,
            fade: Default::default(),
            gun_clip: None,
            aim: aim.map(std::sync::Arc::new),
            aim_pitch: -45_f32.to_radians(),
            two_handed_idle: None,
            two_handed_walk: None,
            two_handed: false,
            max_health: 0,
            action_time: 0.0,
            upper_bones: [Vec::new(), Vec::new()],
            two_handed_bones: false,
            gun_bones: [String::new(), String::new()],
        };
        let (_, locals) = character_locals(&rig, false, 0.0).expect("pose");
        let (mat, pos) = locals["gun"];
        assert!((pos[1] - 0.5).abs() < 1e-5, "{pos:?}");
        assert!(
            (mat[7] - -45_f32.to_radians().sin()).abs() < 1e-5,
            "{mat:?}"
        );
    }

    #[test]
    fn a_character_starts_a_new_clip_from_its_last_pose() {
        let mesh = crate::kf2::KfsMesh {
            name: String::new(),
            vertices: Vec::new(),
            faces: Vec::new(),
            material_textures: Vec::new(),
            parts: Vec::new(),
        };
        let mut rig = CharacterRig {
            mesh,
            skin: None,
            idle: Some(clip("arm", [0.0; 3])),
            walk: None,
            bind: None,
            action: None,
            motion: None,
            fade: Default::default(),
            gun_clip: None,
            aim: None,
            aim_pitch: 0.0,
            two_handed_idle: None,
            two_handed_walk: None,
            two_handed: false,
            max_health: 0,
            action_time: 0.0,
            upper_bones: [Vec::new(), Vec::new()],
            two_handed_bones: false,
            gun_bones: [String::new(), String::new()],
        };
        let last = crate::kf2::sample_locals(&clip("arm", [2.0, 0.0, 0.0]), 0.0);
        rig.fade.advance("CHARANIM_RUN", "", 0.0);
        rig.fade.remember(last);
        rig.fade.advance("CHARANIM_STAND", "", 0.0);
        let (_, locals) = character_locals(&rig, false, 0.0).expect("pose");
        assert_eq!(locals["arm"].1, [2.0, 0.0, 0.0]);
    }

    #[test]
    fn a_clip_lasts_its_frames_at_its_rate() {
        assert_eq!(clip_seconds(&clip("gun", [0.0; 3])), 2.0);
    }

    #[test]
    fn the_gun_frame_turns_with_the_character() {
        let anim = clip("Gun", [1.0, 2.0, 0.0]);
        let locals = crate::kf2::sample_locals(&anim, 0.0);
        let (pos, rot) = bone_frame(
            &anim,
            &locals,
            "gun",
            [10.0, 0.0, 0.0],
            std::f32::consts::FRAC_PI_2,
        )
        .expect("frame");
        // Yaw 90 degrees sends x to -z, as `held_weapon_verts` places vertices.
        assert!(
            (pos[0] - 10.0).abs() < 1e-5 && (pos[1] - 2.0).abs() < 1e-5,
            "{pos:?}"
        );
        assert!((pos[2] + 1.0).abs() < 1e-5, "{pos:?}");
        assert!((rot[2] + 1.0).abs() < 1e-5, "{rot:?}");
    }

    #[test]
    fn a_sound_with_a_position_is_heard_from_there() {
        // sounds/projectiles.txt with soundid.h: Hotspot 5, FallOff 15, Volume 0.4.
        let voice = Voice {
            emitter: String::new(),
            wav: PathBuf::new(),
            looping: false,
            range: Some(crate::SoundRange {
                hotspot: 5.0,
                falloff: 15.0,
                volume: 0.4,
            }),
            at: Some([100.0, 0.0, 0.0]),
            child: None,
        };
        let (scene, rooms) = (LdbScene::default(), BTreeMap::new());
        assert_eq!(voice_gain(&voice, &scene, &rooms, [101.0, 0.0, 0.0]), 0.4);
        assert_eq!(voice_gain(&voice, &scene, &rooms, [0.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn retail_bullet_hit_starts_the_materials_dust_and_sound() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let Some(particles) =
            crate::read_game_bytes(&game, r"\data\database\particles\particles.txt")
        else {
            return;
        };
        let particles_txt = String::from_utf8_lossy(&particles).into_owned();
        let hits = HitEffects {
            game_dir: &game,
            particles_txt: &particles_txt,
            sounds_txt: crate::load_sound_catalog(&game, "projectiles").expect("catalog"),
        };
        let mut scene = LdbScene::default();
        let mut sprites = ParticleSprites::default();
        let frame = crate::decals::surface_frame([0.0, 1.0, 0.0], [0.0, -1.0, 0.0]);
        let decals = crate::decals::DecalField::load(&game, ["bullethit_generic"].into_iter());
        let set = &decals.sets["bullethit_generic"];
        for material in ["wood", "no such material"] {
            hits.start(
                &mut scene,
                &mut sprites,
                set,
                material,
                [1.0, 2.0, 3.0],
                frame,
            );
        }
        sprites.step(&scene, &BTreeMap::new(), 0.05);
        let live = sprites.sprites();
        assert!(!live.is_empty());
        // Dust leaves the hit, not 7 m off it.
        let near = |s: &crate::particles::Sprite| (s.pos[1] - 2.0).abs() < 1.0;
        assert!(live.iter().all(near), "{:?}", live[0].pos);
        assert_eq!(scene.sound_queue.len(), 2);
        let wood = scene.sound_queue[0].filename.as_deref().expect("file");
        assert!(wood.to_ascii_lowercase().contains("wood"), "{wood}");
        let path = crate::story_wav_win_path(wood);
        assert!(crate::read_game_bytes(&game, &path).is_some(), "{path}");
        let range = scene.sound_queue[1].range.expect("3D cue");
        assert_eq!((range.hotspot, range.falloff), (5.0, 15.0));
    }

    #[test]
    fn retail_snowstorm_fills_its_box_with_snowflakes() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let Some(particles) =
            crate::read_game_bytes(&game, r"\data\database\particles\particles.txt")
        else {
            return;
        };
        let particles = String::from_utf8_lossy(&particles).into_owned();
        // Startup intro: `::startroom::cam_mover::camera1::snow->PS_StartEffect(snowstorm_60m_loop)`.
        let mut scene = siren_scene();
        assert!(scene.dispatch_message(
            "::startroom",
            "::startroom::car.DO->PS_StartEffect(snowstorm_60m_loop, \"\");"
        ));
        let mut lights = ParticleLights::default();
        let mut sprites = ParticleSprites::default();
        start_queued_effects(&mut scene, &game, &particles, &mut lights, &mut sprites);
        let rooms = BTreeMap::new();
        for _ in 0..20 {
            sprites.step(&scene, &rooms, 0.05);
        }
        let live = sprites.sprites();
        // 553..1284 flakes per second living 0.2..1.06 s, capped at 500, plus the faint haze.
        assert!(live.len() > 200, "{} sprites", live.len());
        let flake = live
            .iter()
            .find(|s| s.texture.emitter == 0)
            .expect("a snowflake");
        assert!(!flake.additive);
        let bitmap = sprites.texture(&flake.texture).expect("decoded snowflake");
        assert_eq!((bitmap.width, bitmap.height), (16, 16));
    }

    #[test]
    fn retail_police_car_light_flashes_red_and_blue() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        let Some(particles) =
            crate::read_game_bytes(&game, r"\data\database\particles\particles.txt")
        else {
            return;
        };
        let particles = String::from_utf8_lossy(&particles).into_owned();
        let mut scene = siren_scene();
        assert!(scene.dispatch_message(
            "::startroom",
            "::startroom::car.DO->ps_starteffect(light_policecar, \"\");"
        ));
        let mut lights = ParticleLights::default();
        let mut sprites = ParticleSprites::default();
        start_queued_effects(&mut scene, &game, &particles, &mut lights, &mut sprites);
        let rooms = BTreeMap::new();
        let (mut red, mut blue) = (false, false);
        for _ in 0..100 {
            lights.step(0.05);
            let out = lights.point_lights(&scene, &rooms);
            assert_eq!(out.len(), 1, "looping light must stay on");
            assert!(
                (out[0].radius - 4.78).abs() < 0.01,
                "radius={}",
                out[0].radius
            );
            let [r, _, b, _] = out[0].color;
            red |= r > 200.0 && b < 50.0;
            blue |= b > 200.0 && r < 50.0;
        }
        assert!(red && blue, "red={red} blue={blue}");
    }

    #[test]
    fn particle_lights_loop_end_and_stop_with_their_effect() {
        let scene = siren_scene();
        let rooms = BTreeMap::new();
        let mut lights = ParticleLights::default();
        lights.start("::startroom::car.DO".into(), vec![flash(2.0)], true);
        lights.step(3.0);
        // Looping: 3 s into a 2 s cycle samples at 1 s.
        assert_eq!(lights.point_lights(&scene, &rooms)[0].radius, 6.0);
        lights.stop("::STARTROOM::car.DO");
        assert!(lights.point_lights(&scene, &rooms).is_empty());

        lights.start("::startroom::car.DO".into(), vec![flash(2.0)], false);
        lights.step(1.5);
        assert_eq!(lights.point_lights(&scene, &rooms).len(), 1);
        lights.step(1.0);
        assert!(lights.point_lights(&scene, &rooms).is_empty());
    }

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
    fn sound_gain_is_full_inside_the_hotspot_inverse_beyond_and_silent_past_the_falloff() {
        let range = crate::SoundRange {
            hotspot: 2.0,
            falloff: 50.0,
            volume: 0.5,
        };
        assert_eq!(sound_gain(1.0, &range), 0.5);
        assert_eq!(sound_gain(8.0, &range), 0.125);
        assert_eq!(sound_gain(50.0, &range), 0.02);
        assert_eq!(sound_gain(50.5, &range), 0.0);
    }

    #[test]
    fn a_voice_restarts_only_when_its_gain_halves_or_doubles_and_stops_out_of_range() {
        assert_eq!(voice_action(None, 0.0), VoiceAction::Keep);
        assert_eq!(voice_action(None, 0.3), VoiceAction::Start);
        assert_eq!(voice_action(Some(0.3), 0.0), VoiceAction::Stop);
        assert_eq!(voice_action(Some(0.3), 0.5), VoiceAction::Keep);
        assert_eq!(voice_action(Some(0.3), 0.16), VoiceAction::Keep);
        assert_eq!(voice_action(Some(0.3), 0.61), VoiceAction::Start);
        assert_eq!(voice_action(Some(0.3), 0.14), VoiceAction::Start);
    }

    #[test]
    fn retail_level_catalogs_load_when_a_cue_needs_them() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        if crate::read_game_bytes(&game, r"\data\database\sounds\enemy.txt").is_none() {
            return;
        }
        let mut scene = LdbScene::default();
        let mut thug = glass_quad(false);
        thug.name = "::bar::thug.DO".into();
        scene.dynamic_meshes.push(thug);
        assert!(scene.dispatch_message(
            "::bar",
            "::bar::thug.DO->A_Play3DSound(enemy, mobster_alert, \"\");"
        ));
        assert_eq!(scene.sound_queue[0].filename, None);
        resolve_queued_sounds(&mut scene, &game);
        assert_eq!(
            scene.sound_queue[0].filename.as_deref(),
            Some(r"enemy\mobster\mobster_alert_01a.wav")
        );
        assert!(scene.sound_queue[0].range.is_some());
        assert!(scene.sound_catalogs.contains_key("enemy"));
    }

    #[test]
    fn retail_house_speech_resolves_to_a_wav() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let game = std::path::PathBuf::from(home).join("Games/MaxPayne1");
        if crate::read_game_bytes(&game, r"\data\database\sounds\maxhouse.txt").is_none() {
            return;
        }
        let mut scene = LdbScene::default();
        assert!(scene.dispatch_message("::x", "player->A_Play3DSound(maxhouse, P1L0a_01, head);"));
        resolve_queued_sounds(&mut scene, &game);
        let file = scene.sound_queue[0]
            .filename
            .clone()
            .expect("P1L0a_01 file");
        let wav = crate::read_game_bytes(&game, &crate::story_wav_win_path(&file)).expect("wav");
        assert!(wav.starts_with(b"RIFF"));
    }

    #[test]
    fn sound_player_args_set_the_start_volume() {
        let args = sound_player_args(std::path::Path::new("/x/s.wav"), false, 37);
        let at = args.iter().position(|a| a == "-volume").expect("-volume");
        assert_eq!(args[at + 1], "37");
    }

    #[test]
    fn looping_sounds_play_until_stopped() {
        let wav = std::path::Path::new("/x/s.wav");
        let once = sound_player_args(wav, false, 100);
        assert!(!once.iter().any(|a| a == "-loop"));
        assert!(once.iter().any(|a| a == "-autoexit"));
        let looped = sound_player_args(wav, true, 100);
        let at = looped.iter().position(|a| a == "-loop").expect("-loop");
        assert_eq!(looped[at + 1], "0");
        assert_eq!(looped.last().map(String::as_str), Some("/x/s.wav"));
    }

    #[test]
    fn character_light_adds_the_level_ambient_before_the_clamp() {
        // One vertex, no faces: no normal, so point lights add nothing and only the ambient shows.
        let rig = crate::kf2::KfsMesh {
            name: "dot".into(),
            vertices: vec![[0.0; 3]],
            faces: Vec::new(),
            material_textures: Vec::new(),
            parts: Vec::new(),
        };
        let verts = vec![[0.0, 0.0, 0.0]];
        let dark = character_vertex_light(&rig, &verts, [0.0; 3], false, &[], [0.0; 3]);
        assert_eq!(dark[0], [0.0, 0.0, 0.0]);
        let lit = character_vertex_light(&rig, &verts, [0.0; 3], false, &[], [140.0, 135.0, 110.0]);
        assert_eq!(lit[0], [140.0 / 255.0, 135.0 / 255.0, 110.0 / 255.0]);
        let near = [light([0.0, 0.1, 0.0], 255.0, 10.0, 1.0)];
        let sat = character_vertex_light(&rig, &verts, [0.0; 3], false, &near, [300.0, 0.0, 0.0]);
        assert_eq!(sat[0], [1.0, 0.0, 0.0]);
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
    fn lightmaps_are_sampled_bilinearly() {
        // Retail lightmaps shade smoothly; nearest sampling showed every texel as a hard step.
        let lm = DecodedLightmap {
            width: 2,
            height: 1,
            pixels: vec![0x0000_0000, 0x00ff_ffff],
        };
        // Texel centres at u = 0.25 and 0.75 (D3D/wgpu convention), clamped at the edges.
        assert_eq!(sample_lightmap(&lm, 0.25, 0.5), 0x0000_0000);
        assert_eq!(sample_lightmap(&lm, 0.75, 0.5), 0x00ff_ffff);
        assert_eq!(sample_lightmap(&lm, 0.5, 0.5), 0x0080_8080);
        assert_eq!(sample_lightmap(&lm, 1.0, 0.5), 0x00ff_ffff);
    }

    #[test]
    fn textures_are_sampled_bilinearly_and_wrap() {
        // Retail draws filtered textures (FilteringBilinear/Trilinear); nearest looked pixelated.
        let tex = crate::scene::DecodedTexture {
            path: String::new(),
            flag: 0,
            width: 2,
            height: 1,
            pixels: vec![0x0000_0000, 0xffff_ffff],
        };
        assert_eq!(sample_texture(&tex, 0.25, 0.5), 0x0000_0000);
        assert_eq!(sample_texture(&tex, 0.75, 0.5), 0xffff_ffff);
        assert_eq!(sample_texture(&tex, 0.5, 0.5), 0x8080_8080);
        // Repeat addressing: u = 0 sits between the last and the first texel.
        assert_eq!(sample_texture(&tex, 0.0, 0.5), 0x8080_8080);
        assert_eq!(sample_texture(&tex, 1.25, 0.5), 0x0000_0000);
    }

    #[test]
    fn lightmap_modulates_the_texture_like_d3dtop_modulate() {
        // e2_d3d8_driver_mfc 0x10008082: layer MUL -> colour op 4 (D3DTOP_MODULATE).
        assert_eq!(lightmap_modulate(0x0080_8080, 0x0080_8080), 0x0040_4040);
        assert_eq!(lightmap_modulate(0x0020_4060, 0x00ff_ffff), 0x0020_4060);
        assert_eq!(lightmap_modulate(0x00ff_ffff, 0x0000_0000), 0x0000_0000);
    }

    #[test]
    fn chase_camera_stops_in_front_of_a_wall_behind_the_player() {
        // Backing Max against a wall pushed the camera through it: the whole house showed.
        let mut scene = empty_scene();
        let v = |x: f32, y: f32| SceneVertex {
            pos: [x, y, -1.0],
            uv: [0.0; 2],
            luv: [0.0; 2],
        };
        scene.static_meshes.push(crate::scene::SceneStaticMesh {
            mesh_id: 0,
            room_id: None,
            world_origin: [0.0; 3],
            polys: vec![ScenePoly {
                vertices: vec![v(-5.0, -5.0), v(5.0, -5.0), v(5.0, 5.0), v(-5.0, 5.0)],
                normal: [0.0, 0.0, 1.0],
                material_id: 0,
                lightmap_id: -1,
            }],
        });
        let player = [0.0, 1.6, 0.0];
        let fwd = [0.0, 0.0, 1.0];
        let eye = chase_camera(&scene, &RETAIL_RIG, player, fwd);
        assert!(eye[2] > -1.0 && eye[2] < -0.5, "{eye:?}");
        // Nothing in the way: the full follow distance.
        assert_eq!(
            chase_camera(&empty_scene(), &RETAIL_RIG, player, fwd),
            chase_eye(&RETAIL_RIG, player, fwd)
        );
        // A skybox wall does not stop the camera.
        scene.materials.insert(
            0,
            crate::scene::SceneMaterial {
                id: 0,
                category: "Skybox".into(),
                texture_path: String::new(),
                texture_index: None,
                alpha_reference: None,
                blended: false,
            },
        );
        assert_eq!(
            chase_camera(&scene, &RETAIL_RIG, player, fwd),
            chase_eye(&RETAIL_RIG, player, fwd)
        );
    }

    #[test]
    fn chase_camera_keeps_its_clearance_from_a_wall_beside_it() {
        // A wall alongside the follow ray is never hit by it; the eye ended up 0.1 m off the
        // wall and the near plane showed the level behind it.
        let mut scene = empty_scene();
        let v = |y: f32, z: f32| SceneVertex {
            pos: [0.3, y, z],
            uv: [0.0; 2],
            luv: [0.0; 2],
        };
        scene.static_meshes.push(crate::scene::SceneStaticMesh {
            mesh_id: 0,
            room_id: None,
            world_origin: [0.0; 3],
            polys: vec![ScenePoly {
                vertices: vec![v(-5.0, -5.0), v(5.0, -5.0), v(5.0, 5.0), v(-5.0, 5.0)],
                normal: [-1.0, 0.0, 0.0],
                material_id: 0,
                lightmap_id: -1,
            }],
        });
        let player = [0.0, EYE_HEIGHT, 0.0];
        let eye = chase_camera(&scene, &RETAIL_RIG, player, [0.0, 0.0, 1.0]);
        assert!((eye[0] - (0.3 - CAMERA_CLEARANCE)).abs() < 1e-4, "{eye:?}");
        assert!((eye[2] + 4.12).abs() < 1e-4, "{eye:?}");
    }

    #[test]
    fn the_move_clip_runs_on_real_time() {
        // The run clip played ~11x fast: its clock advanced by dt * speed * 1.9. Clips carry
        // their own frame rate (Run.kf2 50 fps), so one second of running is one clip second.
        assert_eq!(move_clip_time(0.25, 1.0), 1.25);
    }

    #[test]
    fn widescreen_bars_cover_an_eighth_of_the_height_each() {
        // Retail 1920x1080 cutscene frames: rows 0..135 and 945..1080 are black (1/8 each).
        assert_eq!(letterbox_rows(1.0), H / 8);
        assert_eq!(letterbox_rows(0.0), 0);
        assert_eq!(letterbox_rows(0.5), (H as f32 / 16.0).round() as usize);
        let mut fb = vec![0x00ff_ffffu32; W * H];
        draw_letterbox(&mut fb, 1.0);
        let bar = H / 8;
        assert_eq!(fb[(bar - 1) * W], 0);
        assert_eq!(fb[bar * W], 0x00ff_ffff);
        assert_eq!(fb[(H - bar) * W + W - 1], 0);
        assert_eq!(fb[(H - bar - 1) * W + W - 1], 0x00ff_ffff);
    }

    #[test]
    fn the_fov_multiplier_widens_the_horizontal_angle() {
        // HYPOTHESIS: gm_changefov scales the view angle (house cutscenes use 1.1).
        // `globalconfig.txt` `DefaultFOV = 70`, across retail's 4:3 frame at this window's height.
        let width = H as f32 * 4.0 / 3.0;
        let half_default = (width * 0.5 / fov_scale(1.0)).atan().to_degrees();
        assert!((half_default - 35.0).abs() < 1e-3, "{half_default}");
        let half = |scale: f32| (width * 0.5 / scale).atan();
        assert!((half(fov_scale(1.1)) - 1.1 * half(fov_scale(1.0))).abs() < 1e-5);
    }

    #[test]
    fn a_blended_sprite_takes_the_fog_at_its_own_depth_and_an_additive_one_does_not() {
        use crate::particles::{Sprite, SpriteTexture};
        let white = crate::scene::DecodedTexture {
            path: String::new(),
            flag: 0,
            width: 1,
            height: 1,
            pixels: vec![0xffff_ffff],
        };
        let fog = crate::scene::LevelFog {
            color: [0.0, 0.0, 0.0],
            start: 0.0,
            end: 10.0,
        };
        let centre_after = |additive: bool| {
            let sprite = Sprite {
                pos: [0.0, 0.0, 5.0],
                size: 2.0,
                rgba: [1.0; 4],
                additive,
                texture: SpriteTexture {
                    effect: "k".into(),
                    emitter: 0,
                    sequence: 0,
                    frame: 0,
                },
                facing: None,
            };
            let (mut fb, mut zb) = (vec![0u32; W * H], vec![f32::INFINITY; W * H]);
            let view = test_view([0.0; 3], [0.0, 0.0, 1.0]);
            draw_sprites(
                &mut fb,
                &mut zb,
                view,
                fov_scale(1.0),
                Some(fog),
                &[sprite],
                |_| Some(&white),
            );
            fb[(H / 2) * W + W / 2]
        };
        assert_eq!(centre_after(false), 0x0080_8080);
        assert_eq!(centre_after(true), 0x00ff_ffff);
    }

    #[test]
    fn sprites_blend_over_the_frame_and_hide_behind_closer_surfaces() {
        use crate::particles::{Sprite, SpriteTexture};
        let white = crate::scene::DecodedTexture {
            path: String::new(),
            flag: 0,
            width: 1,
            height: 1,
            pixels: vec![0xffff_ffff],
        };
        let key = SpriteTexture {
            effect: "k".into(),
            emitter: 0,
            sequence: 0,
            frame: 0,
        };
        let sprite = |rgba: [f32; 4], additive: bool| Sprite {
            pos: [0.0, 0.0, 5.0],
            size: 2.0,
            rgba,
            additive,
            texture: key.clone(),
            facing: None,
        };
        let view = test_view([0.0; 3], [0.0, 0.0, 1.0]);
        let centre = (H / 2) * W + W / 2;
        let draw = |fb: &mut Vec<u32>, zb: &mut Vec<f32>, sprite: Sprite| {
            draw_sprites(fb, zb, view, fov_scale(1.0), None, &[sprite], |_| {
                Some(&white)
            });
        };
        // Alpha blend: half of white over black.
        let (mut fb, mut zb) = (vec![0u32; W * H], vec![f32::INFINITY; W * H]);
        draw(&mut fb, &mut zb, sprite([1.0, 1.0, 1.0, 0.5], false));
        assert_eq!(fb[centre], 0x0080_8080);
        assert_eq!(zb[centre], f32::INFINITY, "sprites do not write depth");
        // Additive: half of red added to grey.
        let mut fb = vec![0x0040_4040u32; W * H];
        draw(&mut fb, &mut zb, sprite([1.0, 0.0, 0.0, 0.5], true));
        assert_eq!(fb[centre], 0x00c0_4040);
        // A surface 3 m away hides a sprite 5 m away.
        let (mut fb, mut zb) = (vec![0u32; W * H], vec![3.0f32; W * H]);
        draw(&mut fb, &mut zb, sprite([1.0, 1.0, 1.0, 1.0], false));
        assert_eq!(fb[centre], 0);
    }

    /// `globalconfig.txt` and the skeleton: camera point plus `MiddleFollowDistance`.
    #[test]
    fn looking_level_the_camera_is_four_metres_behind_the_camera_point() {
        let eye = chase_eye(&RETAIL_RIG, [0.0, EYE_HEIGHT, 0.0], [0.0, 0.0, 1.0]);
        for (got, want) in eye.iter().zip([0.18, 2.25, -4.12]) {
            assert!((got - want).abs() < 1e-5, "{eye:?}");
        }
    }

    #[test]
    fn the_camera_turns_with_the_players_yaw() {
        // Facing +x, the player's right is -z.
        let eye = chase_eye(&RETAIL_RIG, [0.0, EYE_HEIGHT, 0.0], [1.0, 0.0, 0.0]);
        for (got, want) in eye.iter().zip([-4.12, 2.25, -0.18]) {
            assert!((got - want).abs() < 1e-5, "{eye:?}");
        }
    }

    #[test]
    fn looking_straight_down_the_camera_orbits_above_the_player() {
        let eye = chase_eye(&RETAIL_RIG, [0.0, EYE_HEIGHT, 0.0], [0.0, -1.0, 0.0001]);
        for (got, want) in eye.iter().zip([0.18, 2.15, 0.28]) {
            assert!((got - want).abs() < 1e-3, "{eye:?}");
        }
    }

    #[test]
    fn looking_straight_up_the_camera_orbits_below_the_player() {
        let eye = chase_eye(&RETAIL_RIG, [0.0, EYE_HEIGHT, 0.0], [0.0, 1.0, 0.0001]);
        for (got, want) in eye.iter().zip([0.18, 0.75, -0.92]) {
            assert!((got - want).abs() < 1e-3, "{eye:?}");
        }
    }

    #[test]
    fn max_stays_near_the_view_axis_at_every_pitch() {
        // Flat offsets swung him out of the frame as the view tilted.
        for degrees in [-80.0f32, -60.0, -45.0, -20.0, 0.0, 20.0, 45.0, 60.0, 80.0] {
            let (s, c) = degrees.to_radians().sin_cos();
            let fwd = [0.0, s, c];
            let player = [0.0, EYE_HEIGHT, 0.0];
            let eye = chase_eye(&RETAIL_RIG, player, fwd);
            let to_max = normalize(sub3(chase_start(&RETAIL_RIG, player, fwd), eye));
            let off_axis = dot3(to_max, fwd).clamp(-1.0, 1.0).acos().to_degrees();
            assert!(off_axis < 46.0, "{degrees}: {off_axis}");
        }
    }

    #[test]
    fn the_chase_camera_looks_at_a_point_ahead_of_the_aiming_point() {
        let player = [0.0, EYE_HEIGHT, 0.0];
        let aim = [0.0, 0.0, 1.0];
        let eye = chase_eye(&RETAIL_RIG, player, aim);
        let (right, up, fwd) = chase_look(&RETAIL_RIG, player, aim, eye);
        let target = [0.12, 1.5, 5.0];
        let want = normalize(sub3(target, eye));
        for c in 0..3 {
            assert!((fwd[c] - want[c]).abs() < 1e-5, "{fwd:?} {want:?}");
        }
        assert!(dot3(right, fwd).abs() < 1e-5 && dot3(up, fwd).abs() < 1e-5);
        assert!(right[0] > 0.9 && up[1] > 0.9, "{right:?} {up:?}");
    }

    #[test]
    fn the_chase_ray_starts_at_the_camera_point() {
        let start = chase_start(&RETAIL_RIG, [0.0, EYE_HEIGHT, 0.0], [0.0, 0.0, 1.0]);
        for (got, want) in start.iter().zip([0.12, 1.75, -0.12]) {
            assert!((got - want).abs() < 1e-5, "{start:?}");
        }
    }

    #[test]
    fn the_rig_reads_its_offsets_from_the_retail_text() {
        let config = "[camera]\n{\n\t[general]\n\tUpFollowDistance\t= ( 0.06, 0.40, -0.40 );\n\tMiddleFollowDistance\t= ( 0.06, 0.5, -4.00 ); //was 0,0.75,-3\n\tDownFollowDistance\t= ( 0.06, 0.80, -1.00 );\n\tInitialBackwardsSpeed\t= 5.0; \t// m/s\t- was 5\n\tLookAtMinDistance\t= 5.0;\t// minimum distance from aiming startpoint\n}\n";
        let skeleton = "[Sk]\nDefaultCameraPointOffset\t\t= ( 0.12, 1.75, -0.12 );\nCrouchingCameraPointOffset\t\t= ( 0.12, 1.15, -0.12 );\nShootDodgingCameraPointOffset\t= ( 0.12, 0.90, -0.12 );\nDefaultAimingPointOffset\t\t= ( 0.12, 1.50, 0.00 );\n";
        assert_eq!(ChaseRig::parse(config, skeleton), Some(RETAIL_RIG));
    }

    #[test]
    fn a_point_on_or_beside_a_triangle_edge_counts_as_inside() {
        // The floor strip at the foot of the house stairs is two slim triangles: edge points were
        // rejected from both, so Max could not step across it.
        let (a, b, c) = ([8.12, 0.0, -4.06], [9.88, 0.0, -4.06], [9.88, 0.0, -3.94]);
        assert!(point_in_xz_tri(9.0, -4.0, a, b, c), "on the diagonal");
        assert!(point_in_xz_tri(9.0, -4.06, a, b, c), "on the long edge");
        assert!(point_in_xz_tri(9.5, -4.03, a, b, c), "inside");
    }

    #[test]
    fn a_point_far_from_a_tiny_triangle_is_outside() {
        // Unscaled cross products of a small triangle stay inside any fixed tolerance, so its
        // slope reached across the house as an invisible ramp in a doorway.
        let (a, b, c) = ([0.0, 0.0, 0.0], [0.05, 0.0, 0.0], [0.05, 0.0, 0.05]);
        assert!(!point_in_xz_tri(0.5, 0.5, a, b, c));
        assert!(!point_in_xz_tri(-0.2, 0.02, a, b, c));
        assert!(point_in_xz_tri(0.04, 0.01, a, b, c));
    }

    #[test]
    fn a_point_clearly_outside_a_triangle_is_outside() {
        let (a, b, c) = ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0]);
        assert!(!point_in_xz_tri(0.0, 1.0, a, b, c));
        assert!(!point_in_xz_tri(2.0, 0.5, a, b, c));
        assert!(!point_in_xz_tri(0.5, -0.5, a, b, c));
    }

    fn floors_at(heights: &[f32]) -> LdbScene {
        let mut scene = empty_scene();
        let v = |x: f32, y: f32, z: f32| SceneVertex {
            pos: [x, y, z],
            uv: [0.0; 2],
            luv: [0.0; 2],
        };
        let polys = heights
            .iter()
            .map(|y| ScenePoly {
                vertices: vec![
                    v(-5.0, *y, -5.0),
                    v(5.0, *y, -5.0),
                    v(5.0, *y, 5.0),
                    v(-5.0, *y, 5.0),
                ],
                normal: [0.0, 1.0, 0.0],
                material_id: 0,
                lightmap_id: -1,
            })
            .collect();
        scene.static_meshes.push(crate::scene::SceneStaticMesh {
            mesh_id: 0,
            room_id: None,
            world_origin: [0.0; 3],
            polys,
        });
        scene
    }

    #[test]
    fn a_stair_riser_is_stepped_onto() {
        let scene = floors_at(&[0.0, 0.3]);
        assert_eq!(find_floor_y(&scene, 0.0, EYE_HEIGHT, 0.0), Some(0.3));
    }

    #[test]
    fn a_surface_above_the_capsule_radius_is_not_stepped_onto() {
        // A railing 1 m up lifted Max onto it at the foot of the house stairs.
        let scene = floors_at(&[0.0, 1.0]);
        assert_eq!(find_floor_y(&scene, 0.0, EYE_HEIGHT, 0.0), Some(0.0));
    }

    #[test]
    fn the_mouse_turns_the_view_with_no_button_held() {
        let (yaw, pitch) = mouse_look(1.0, 0.0, (100.0, -100.0));
        assert!((yaw - 1.3).abs() < 1e-5);
        assert!((pitch - 0.3).abs() < 1e-5);
    }

    #[test]
    fn looking_up_and_down_stops_short_of_vertical() {
        assert_eq!(mouse_look(0.0, 1.2, (0.0, -1000.0)).1, 1.25);
        assert_eq!(mouse_look(0.0, -1.2, (0.0, 1000.0)).1, -1.25);
    }

    #[test]
    fn a_shot_past_a_meshs_edge_misses_it() {
        // The quad spans z -1..1: padded bounds counted a shot 0.1 m beyond its edge.
        let mut scene = empty_scene();
        let mut quad = glass_quad(false);
        quad.hitpoints = Some(10);
        scene.dynamic_meshes.push(quad);
        let rooms = BTreeMap::new();
        let dir = [1.0, 0.0, 0.0];
        assert_eq!(
            aim_hit_dynamic(&scene, &rooms, [-4.0, 1.0, 0.9], dir).map(|hit| hit.0),
            Some(0)
        );
        assert_eq!(aim_hit_dynamic(&scene, &rooms, [-4.0, 1.0, 1.1], dir), None);
    }

    #[test]
    fn a_shot_lands_on_the_surface_it_crosses() {
        let mut scene = empty_scene();
        let mut quad = glass_quad(false);
        quad.hitpoints = Some(10);
        scene.dynamic_meshes.push(quad);
        let hit = aim_hit_dynamic(&scene, &BTreeMap::new(), [-4.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        assert!((hit.expect("hit").1 - 4.0).abs() < 1e-4);
    }

    #[test]
    fn a_shot_hits_a_character_inside_the_capsule_radius_only() {
        let dir = [0.0, 0.0, 1.0];
        let feet = [0.0, 0.0, 10.0];
        let hit = ray_hits_character([0.25, 1.0, 0.0], dir, feet);
        assert!((hit.expect("hit") - 10.0).abs() < 0.1);
        assert_eq!(ray_hits_character([0.4, 1.0, 0.0], dir, feet), None);
    }

    #[test]
    fn a_shot_over_a_characters_head_or_behind_the_shooter_misses() {
        let dir = [0.0, 0.0, 1.0];
        assert_eq!(
            ray_hits_character([0.0, 2.5, 0.0], dir, [0.0, 0.0, 10.0]),
            None
        );
        assert_eq!(
            ray_hits_character([0.0, 1.0, 0.0], dir, [0.0, 0.0, -10.0]),
            None
        );
    }

    fn wall_scene() -> LdbScene {
        let mut scene = empty_scene();
        let v = |x: f32, y: f32| SceneVertex {
            pos: [x, y, 5.0],
            uv: [0.0; 2],
            luv: [0.0; 2],
        };
        scene.static_meshes.push(crate::scene::SceneStaticMesh {
            mesh_id: 0,
            room_id: None,
            world_origin: [0.0; 3],
            polys: vec![ScenePoly {
                vertices: vec![v(-5.0, -5.0), v(5.0, -5.0), v(5.0, 5.0), v(-5.0, 5.0)],
                normal: [0.0, 0.0, -1.0],
                material_id: 7,
                lightmap_id: -1,
            }],
        });
        scene
    }

    #[test]
    fn a_shot_reports_the_wall_it_stops_at() {
        let hit = aim_hit_level(&wall_scene(), [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        let (t, normal, material) = hit.expect("wall");
        assert!((t - 5.0).abs() < 1e-3);
        assert_eq!(normal, [0.0, 0.0, -1.0]);
        assert_eq!(material, 7);
    }

    #[test]
    fn a_shot_into_the_open_stops_at_nothing() {
        assert_eq!(
            aim_hit_level(&wall_scene(), [0.0; 3], [0.0, 0.0, -1.0]),
            None
        );
    }

    fn quad_sprite(facing: [[f32; 3]; 2]) -> crate::particles::Sprite {
        crate::particles::Sprite {
            pos: [0.0, 0.0, 5.0],
            size: 2.0,
            rgba: [1.0; 4],
            additive: false,
            texture: crate::particles::SpriteTexture {
                effect: "k".into(),
                emitter: 0,
                sequence: 0,
                frame: 0,
            },
            facing: Some(facing),
        }
    }

    fn drawn_quad(facing: [[f32; 3]; 2]) -> Vec<u32> {
        let white = crate::scene::DecodedTexture {
            path: String::new(),
            flag: 0,
            width: 1,
            height: 1,
            pixels: vec![0xffff_ffff],
        };
        let (mut fb, mut zb) = (vec![0u32; W * H], vec![f32::INFINITY; W * H]);
        draw_sprites(
            &mut fb,
            &mut zb,
            test_view([0.0; 3], [0.0, 0.0, 1.0]),
            fov_scale(1.0),
            None,
            &[quad_sprite(facing)],
            |_| Some(&white),
        );
        fb
    }

    #[test]
    fn a_decal_facing_the_view_covers_its_square() {
        let fb = drawn_quad([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        assert_eq!(fb[(H / 2) * W + W / 2 + 40], 0x00ff_ffff);
    }

    #[test]
    fn a_decal_seen_edge_on_covers_almost_nothing() {
        // A camera-facing square would cover these pixels.
        let fb = drawn_quad([[0.0, 0.0, 1.0], [0.0, 1.0, 0.0]]);
        assert_eq!(fb[(H / 2) * W + W / 2 + 40], 0);
    }

    #[test]
    fn a_frame_is_due_at_once_then_every_interval_with_new_names() {
        let mut capture = FrameCapture::new(PathBuf::from("out"), "Open Max Payne - tutorial");
        let first = capture.due(0.1).expect("first frame");
        assert_eq!(first, PathBuf::from("out/OpenMaxPaynetutorial_0001.ppm"));
        assert_eq!(capture.due(1.0), None);
        let second = capture.due(1.2).expect("second frame");
        assert_eq!(second, PathBuf::from("out/OpenMaxPaynetutorial_0002.ppm"));
    }

    #[test]
    fn a_video_frame_is_due_at_once_then_thirty_times_a_second() {
        let mut pace = VideoPace::default();
        assert!(pace.due(0.01));
        assert!(!pace.due(0.01));
        assert!(pace.due(0.03));
    }

    #[test]
    fn a_slow_engine_does_not_owe_the_video_its_missed_frames() {
        let mut pace = VideoPace::default();
        assert!(pace.due(0.5));
        assert!(!pace.due(0.01));
    }

    #[test]
    fn video_frames_are_stamped_by_the_clock_and_sized_to_the_frame() {
        let args = video_arguments(Path::new("out/level.mp4"), 960, 600).join(" ");
        assert!(args.contains("-use_wallclock_as_timestamps 1"), "{args}");
        assert!(args.contains("-video_size 960x600"), "{args}");
        assert!(args.contains("-pixel_format bgr0"), "{args}");
        assert!(args.ends_with("out/level.mp4"), "{args}");
    }

    #[test]
    fn a_pixel_reaches_the_video_blue_first() {
        assert_eq!(video_bytes(&[0x0011_2233]), [0x33, 0x22, 0x11, 0]);
    }

    #[test]
    fn the_hud_layer_composites_over_the_saved_frame() {
        let clear = crate::gpu::HUD_CLEAR;
        let half = crate::hud::blend(clear, 0x00ff_ffff, 128);
        let mut frame = vec![0x0040_4040, 0x0000_0000, 0x0040_4040];
        composite_hud(&mut frame, &[clear, half, 0x0011_2233]);
        assert_eq!(frame, vec![0x0040_4040, 0x0080_8080, 0x0011_2233]);
    }

    #[test]
    fn the_baked_sky_covers_the_whole_sphere() {
        assert_eq!(sky_uv([0.0, 0.0, 1.0]), [0.5, 0.5]);
        assert_eq!(sky_uv([1.0, 0.0, 0.0]), [0.75, 0.5]);
        assert_eq!(sky_uv([0.0, 1.0, 0.0])[1], 0.0);
        assert_eq!(sky_uv([0.0, -1.0, 0.0])[1], 0.999);
    }

    fn flat_texture(pixels: Vec<u32>) -> DecodedTexture {
        DecodedTexture {
            path: "t.tga".into(),
            flag: 0,
            width: pixels.len() as u32,
            height: 1,
            pixels,
        }
    }

    /// A triangle around `+z` with every corner on `uv`.
    fn sky_triangle(uv: [f32; 2]) -> crate::kf2::SkyMesh {
        crate::kf2::SkyMesh {
            vertices: vec![[-50.0, -50.0, 50.0], [50.0, -50.0, 50.0], [0.0, 60.0, 50.0]],
            uvs: vec![uv; 3],
            indices: vec![0, 1, 2],
            textures: Vec::new(),
            node: String::new(),
        }
    }

    fn baked_at(sky: &DecodedTexture, dir: [f32; 3]) -> u32 {
        let [u, v] = sky_uv(dir);
        let (x, y) = (
            (u * sky.width as f32) as u32,
            (v * sky.height as f32) as u32,
        );
        sky.pixels[(y * sky.width + x) as usize]
    }

    #[test]
    fn a_sky_triangle_paints_only_the_directions_it_covers() {
        let mesh = sky_triangle([0.0, 0.0]);
        let colour = flat_texture(vec![0x00ff_0000]);
        let layer = SkyLayer {
            mesh: &mesh,
            colour: &colour,
            opacity: None,
        };
        let sky = bake_sky(&[layer], 256, 128);
        assert_eq!(baked_at(&sky, [0.0, 0.0, 1.0]), 0x00ff_0000);
        assert_eq!(baked_at(&sky, [0.0, 0.0, -1.0]), 0);
        assert_eq!(baked_at(&sky, [1.0, 0.0, 0.0]), 0);
    }

    #[test]
    fn sky_texture_coordinates_wrap_around_negative_v() {
        // World spheres store v from -1 at the zenith up to 0.
        let mesh = sky_triangle([0.75, -0.25]);
        let colour = DecodedTexture {
            path: "t.tga".into(),
            flag: 0,
            width: 2,
            height: 2,
            pixels: vec![1, 2, 3, 4],
        };
        let layer = SkyLayer {
            mesh: &mesh,
            colour: &colour,
            opacity: None,
        };
        let sky = bake_sky(&[layer], 256, 128);
        assert_eq!(baked_at(&sky, [0.0, 0.0, 1.0]), 4);
    }

    #[test]
    fn a_sky_layer_with_an_opacity_map_blends_over_the_one_before() {
        let (behind, front) = (sky_triangle([0.0, 0.0]), sky_triangle([0.0, 0.0]));
        let red = flat_texture(vec![0x00ff_0000]);
        let blue = flat_texture(vec![0x0000_00ff]);
        let half = flat_texture(vec![0x0080_8080]);
        let sky = bake_sky(
            &[
                SkyLayer {
                    mesh: &behind,
                    colour: &red,
                    opacity: None,
                },
                SkyLayer {
                    mesh: &front,
                    colour: &blue,
                    opacity: Some(&half),
                },
            ],
            256,
            128,
        );
        assert_eq!(baked_at(&sky, [0.0, 0.0, 1.0]), 0x007f_0080);
    }

    #[test]
    fn house_sunset_sky_bakes_when_the_catalog_is_present() {
        let catalog = Path::new("/tmp/omp-xdata-ref/data/database/worldspheres/worldspheres.txt");
        if !catalog.is_file() {
            return;
        }
        let tex = load_world_sphere(Path::new("/tmp/omp-xdata-ref"), "sunset").expect("sunset");
        assert_eq!((tex.width, tex.height), SKY_SIZE);
        assert!(tex.pixels.iter().any(|px| *px != 0));
    }

    pub(super) fn empty_scene() -> LdbScene {
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
            sound_stops: Vec::new(),
            music_request: None,
            camera_shakes: Vec::new(),
            death_cams: Vec::new(),
            effect_queue: Vec::new(),
            held_guns: [None, None],
            effect_stops: Vec::new(),
            attached_camera: None,
            player_controls: true,
            screen_fade: None,
            ambient: [0.0; 3],
            mode_switch: None,
            fog: None,
            fog_fade: None,
            widescreen: crate::scene::Ramp::at(0.0),
            fov_multiplier: crate::scene::Ramp::at(1.0),
            game_speed: crate::scene::Ramp::at(1.0),
            dodge_speed: crate::scene::Ramp::at(1.0),
            global_speed: crate::scene::Ramp::at(1.0),
            slow_motion_level: None,
            character_orders: Vec::new(),
            closed_exits: Default::default(),
            screen_flash: None,
            inventory: Default::default(),
            hud_prints: Vec::new(),
            gravity: None,
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
                blended: false,
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
                blended: false,
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
            parent: String::new(),
            room_id: 1,
            world_mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            world_pos: [0.0; 3],
            local_start_mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            local_start_pos: [0.0; 3],
            toggled: false,
            hidden: false,
            collisions: true,
            hitpoints: None,
            polys: vec![quad],
            anims: Vec::new(),
            active_track: None,
            track_time: 0.0,
            track_duration: 0.0,
            anim_state: Default::default(),
        });
        let rooms = BTreeMap::new();
        assert!(hits_wall(&scene, &rooms, 0.1, 1.65, 0.0));
        scene.dynamic_meshes[0].polys[0].material_id = 2;
        assert!(!hits_wall(&scene, &rooms, 0.1, 1.65, 0.0));
    }

    /// A dynamic mesh holding one flat square at `height`, x 0..2 and z -1..1.
    fn flat_mesh(height: f32) -> SceneDynamicMesh {
        let mut mesh = glass_quad(false);
        let corner = |x, z| {
            let mut vertex = mesh.polys[0].vertices[0];
            vertex.pos = [x, height, z];
            vertex
        };
        let vertices = vec![
            corner(0.0, -1.0),
            corner(0.0, 1.0),
            corner(2.0, 1.0),
            corner(2.0, -1.0),
        ];
        mesh.polys[0].vertices = vertices;
        mesh.polys[0].normal = [0.0, 1.0, 0.0];
        mesh
    }

    #[test]
    fn a_dynamic_mesh_stops_the_chase_camera() {
        // A dive beside the van put the camera inside it.
        let mut scene = empty_scene();
        let mut wall = flat_mesh(0.0);
        let corner = |x, y| {
            let mut vertex = wall.polys[0].vertices[0];
            vertex.pos = [x, y, -1.5];
            vertex
        };
        wall.polys[0].vertices = vec![
            corner(-5.0, -5.0),
            corner(5.0, -5.0),
            corner(5.0, 5.0),
            corner(-5.0, 5.0),
        ];
        wall.polys[0].normal = [0.0, 0.0, 1.0];
        wall.polys[0].material_id = 0;
        scene.dynamic_meshes.push(wall);
        let rooms = BTreeMap::new();
        let eye = chase_view(
            &scene,
            &rooms,
            &RETAIL_RIG,
            [0.0, 1.6, 0.0],
            [0.0, 0.0, 1.0],
        );
        assert!(eye[2] > -1.5, "{eye:?}");
        scene.dynamic_meshes[0].hidden = true;
        let eye = chase_view(
            &scene,
            &rooms,
            &RETAIL_RIG,
            [0.0, 1.6, 0.0],
            [0.0, 0.0, 1.0],
        );
        assert!(eye[2] < -1.5, "{eye:?}");
    }

    #[test]
    fn a_decal_takes_the_ambient_and_the_lights_it_sits_in() {
        let lamp = light([0.0; 3], 255.0, 2.0, 1.0);
        let lit = point_brightness([0.0; 3], std::slice::from_ref(&lamp), [51.0; 3]);
        assert_eq!(lit, [1.0; 3]);
        let dark = point_brightness([9.0, 0.0, 0.0], std::slice::from_ref(&lamp), [51.0; 3]);
        assert_eq!(dark, [0.2; 3]);
    }

    #[test]
    fn a_dynamic_floor_too_high_to_step_on_stops_the_body() {
        let mut scene = empty_scene();
        scene.dynamic_meshes.push(flat_mesh(0.75));
        let rooms = BTreeMap::new();
        assert!(hits_wall(&scene, &rooms, -0.2, EYE_HEIGHT, 0.0));
        assert!(hits_wall(&scene, &rooms, 1.0, EYE_HEIGHT, 0.0));
        assert!(!hits_wall(&scene, &rooms, -1.0, EYE_HEIGHT, 0.0));
    }

    #[test]
    fn a_dynamic_floor_blocks_only_between_step_and_head_height() {
        let rooms = BTreeMap::new();
        for height in [STEP_UP - 0.1, EYE_HEIGHT + 0.5] {
            let mut scene = empty_scene();
            scene.dynamic_meshes.push(flat_mesh(height));
            assert!(!hits_wall(&scene, &rooms, 1.0, EYE_HEIGHT, 0.0), "{height}");
        }
    }

    #[test]
    fn do_with_collisions_disabled_does_not_block() {
        let mut scene = empty_scene();
        scene.dynamic_meshes.push(glass_quad(false));
        let rooms = BTreeMap::new();
        assert!(hits_wall(&scene, &rooms, 0.1, 1.65, 0.0));
        scene.dynamic_meshes[0].collisions = false;
        assert!(!hits_wall(&scene, &rooms, 0.1, 1.65, 0.0));
    }

    #[test]
    fn attached_camera_looks_along_the_do_z_axis() {
        // The four house cutscene cameras (`::entrance05::camera1`, `::startroom::camera2`,
        // `::entrance06::camera2`, `::Bathroom::camera2`) frame their rooms along +z; along -z
        // they face a wall (offline frames, docs/rea/2026-11-02-camera-attach-to-do.md).
        let mut scene = empty_scene();
        let mut cam = glass_quad(false);
        // Yaw 90 degrees: local z points along world +x.
        cam.world_mat = [0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0];
        cam.local_start_mat = cam.world_mat;
        cam.world_pos = [2.0, 1.5, -3.0];
        cam.local_start_pos = cam.world_pos;
        scene.dynamic_meshes.push(cam);
        let rooms = BTreeMap::new();
        assert!(attached_view(&scene, &rooms).is_none());
        scene.attached_camera = Some(0);
        let (pos, right, up, fwd) = attached_view(&scene, &rooms).expect("attached");
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(close(pos, [2.0, 1.5, -3.0]), "{pos:?}");
        assert!(close(fwd, [1.0, 0.0, 0.0]), "{fwd:?}");
        assert!(close(up, [0.0, 1.0, 0.0]), "{up:?}");
        assert!(close(right, [0.0, 0.0, -1.0]), "{right:?}");
    }

    #[test]
    fn frame_step_follows_the_real_clock_with_a_cap() {
        // Scripts, fades and animations follow wall time, so a slow frame does not slow the
        // cutscene against its real-time speech; a loading stall is capped instead of skipping.
        assert_eq!(frame_dt(std::time::Duration::from_millis(33)), 0.033);
        assert_eq!(frame_dt(std::time::Duration::from_secs(2)), MAX_FRAME_DT);
    }

    #[test]
    fn software_fog_mixes_by_view_depth_and_spares_the_sky() {
        let fog = crate::scene::LevelFog {
            color: [0.0, 0.0, 0.0],
            start: 0.0,
            end: 100.0,
        };
        let mut fb = vec![0x00ff_ffffu32; 3];
        let zb = [50.0, 200.0, f32::INFINITY];
        apply_fog(&mut fb, &zb, Some(fog));
        assert_eq!(fb, vec![0x0080_8080, 0x0000_0000, 0x00ff_ffff]);
    }

    #[test]
    fn screen_fade_blends_by_alpha() {
        let mut fb = vec![0x00ff_8000u32, 0x0000_00ffu32];
        blend_screen(&mut fb, [0, 0, 0, 128]);
        assert_eq!(fb, vec![0x007f_4000, 0x0000_007f]);
        blend_screen(&mut fb, [255, 255, 255, 255]);
        assert_eq!(fb, vec![0x00ff_ffff, 0x00ff_ffff]);
    }

    #[test]
    fn render_frame_from_a_camera_do_paints_what_it_faces() {
        let mut scene = empty_scene();
        scene.dynamic_meshes.push(glass_quad(false));
        let mut cam = glass_quad(false);
        cam.name = "::Bathroom::camera2.DO".into();
        cam.polys.clear();
        // Local +z (the view axis) along world +x, toward the glass at x = 0.
        cam.world_mat = [0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0];
        cam.local_start_mat = cam.world_mat;
        cam.world_pos = [-4.0, 1.0, 0.0];
        cam.local_start_pos = cam.world_pos;
        scene.dynamic_meshes.push(cam);
        let (w, h, fb) = render_frame(&scene, Some("::bathroom::camera2.DO")).expect("frame");
        assert_eq!((w, h), (W, H));
        assert!(fb.iter().any(|px| *px != fb[0]), "the glass must show");
        assert!(render_frame(&scene, Some("::nowhere.DO")).is_err());
    }

    #[test]
    fn a_surface_past_the_fog_end_draws_in_the_fog_colour() {
        // The intro's statue stands 714 m out under fog `-700, 700`, darker than the retail sky.
        let mut scene = empty_scene();
        scene.fog = Some(crate::scene::LevelFog {
            color: [20.0, 20.0, 20.0],
            start: -5.0,
            end: 5.0,
        });
        scene.dynamic_meshes.push(glass_quad(false));
        let mut cam = glass_quad(false);
        cam.name = "::Bathroom::camera2.DO".into();
        cam.polys.clear();
        cam.world_mat = [0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0];
        cam.local_start_mat = cam.world_mat;
        cam.world_pos = [-20.0, 1.0, 0.0];
        cam.local_start_pos = cam.world_pos;
        scene.dynamic_meshes.push(cam);
        let (_, _, fb) = render_frame(&scene, Some("::bathroom::camera2.DO")).expect("frame");
        assert!(fb.contains(&0x0014_1414));
    }

    #[test]
    fn ppm_is_binary_rgb_with_a_header() {
        let path = std::env::temp_dir().join(format!("omp-frame-{}.ppm", std::process::id()));
        write_ppm(&path, 2, 1, &[0x00ff_8000, 0x0000_00ff]).expect("write");
        let bytes = std::fs::read(&path).expect("read");
        let _ = std::fs::remove_file(&path);
        assert_eq!(bytes, b"P6\n2 1\n255\n\xff\x80\x00\x00\x00\xff".to_vec());
    }

    #[test]
    fn parallel_band_raster_matches_serial_raster_exactly() {
        // Overlapping quads at several depths, some with an alpha-masked texture and some
        // that do not write depth: any change in per-pixel order would change the result.
        let tex = DecodedTexture {
            path: "t.tga".into(),
            flag: 0,
            width: 8,
            height: 8,
            pixels: (0..64u32)
                .map(|i| {
                    if i % 3 == 0 {
                        0x0012_3456
                    } else {
                        0xff00_0000 | (i * 0x0003_0507)
                    }
                })
                .collect(),
        };
        let mut tris = Vec::new();
        for k in 0..40 {
            let x = 40.0 + (k * 37 % 700) as f32;
            let y = 20.0 + (k * 53 % 450) as f32;
            let z = 1.0 + (k % 7) as f32;
            let v = |sx: f32, sy: f32, u: f32, vv: f32| ProjVert {
                sx,
                sy,
                inv_z: 1.0 / z,
                u_over_z: u / z,
                v_over_z: vv / z,
                lu_over_z: 0.0,
                lv_over_z: 0.0,
                light_over_z: [0.7 / z; 3],
            };
            for (a, b, c) in [
                (
                    v(x, y, 0.0, 0.0),
                    v(x + 260.0, y, 1.0, 0.0),
                    v(x + 260.0, y + 180.0, 1.0, 1.0),
                ),
                (
                    v(x, y, 0.0, 0.0),
                    v(x + 260.0, y + 180.0, 1.0, 1.0),
                    v(x, y + 180.0, 0.0, 1.0),
                ),
            ] {
                tris.push(QueuedTri {
                    p: [a, b, c],
                    tex: (k % 2 == 0).then_some(&tex),
                    alpha_reference: (k % 4 == 0).then_some(128),
                    lmap: None,
                    write_z: k % 3 != 0,
                    depth_bias: 0.0,
                    // Blending reads the frame, so it also has to keep the per-pixel order.
                    blended: k % 8 == 0,
                });
            }
        }
        let run = |threads: usize| {
            let mut fb = vec![0u32; W * H];
            let mut zb = vec![f32::INFINITY; W * H];
            flush_tris(&tris, &mut fb, &mut zb, threads);
            (fb, zb)
        };
        let (fb1, zb1) = run(1);
        let (fb8, zb8) = run(8);
        assert!(fb1.iter().any(|&p| p != 0), "the quads must draw");
        assert!(fb1 == fb8, "colour differs between serial and parallel");
        assert!(
            zb1.iter()
                .zip(&zb8)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "depth differs between serial and parallel"
        );
    }

    #[test]
    fn crosshair_is_hidden_while_a_cutscene_camera_is_attached() {
        let mut scene = empty_scene();
        scene.dynamic_meshes.push(glass_quad(false));
        assert!(crosshair_visible(&scene));
        scene.attached_camera = Some(0);
        assert!(!crosshair_visible(&scene));
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
                blended: false,
            },
        );
        scene.dynamic_meshes.push(glass_quad(false));
        assert_eq!(painted_pixels(&scene), 0, "alpha 14 fails reference 15");
        scene.textures[0].pixels[0] = 0x0f80_8080;
        assert!(painted_pixels(&scene) > 0, "alpha 15 passes reference 15");
    }

    /// A white texel of `alpha` on the glass quad, its material blended or cut out.
    fn masked_quad_scene(alpha: u32, blended: bool) -> LdbScene {
        let mut scene = empty_scene();
        scene.textures.push(DecodedTexture {
            path: "plate".into(),
            flag: 4,
            width: 1,
            height: 1,
            pixels: vec![(alpha << 24) | 0x00ff_ffff],
        });
        scene.materials.insert(
            1,
            SceneMaterial {
                id: 1,
                category: "externalwall".into(),
                texture_path: String::new(),
                texture_index: Some(0),
                alpha_reference: Some(15),
                blended,
            },
        );
        scene.dynamic_meshes.push(glass_quad(false));
        scene
    }

    fn centre_pixel(scene: &LdbScene) -> u32 {
        let mut fb = vec![0u32; W * H];
        let mut zb = vec![f32::INFINITY; W * H];
        draw_dynamic_meshes(
            scene,
            &BTreeMap::new(),
            [-4.0, 1.0, 0.0],
            [0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            fov_scale(1.0),
            false,
            false,
            &mut fb,
            &mut zb,
        );
        fb[(H / 2) * W + W / 2]
    }

    #[test]
    fn blended_texels_mix_with_the_frame_by_their_alpha() {
        // BlendedAlphaTest: past the alpha test, colour = src * a + dst * (1 - a). Over a black
        // frame, alpha 128 gives half the cut-out colour (within rounding).
        let opaque = centre_pixel(&masked_quad_scene(128, false));
        let blended = centre_pixel(&masked_quad_scene(128, true));
        assert!(opaque != 0, "the quad must draw");
        for shift in [0, 8, 16] {
            let (o, b) = ((opaque >> shift) & 0xff, (blended >> shift) & 0xff);
            assert!(b.abs_diff(o * 128 / 255) <= 1, "{shift}: {o} -> {b}");
        }
    }

    pub(super) fn glass_quad(hidden: bool) -> SceneDynamicMesh {
        SceneDynamicMesh {
            name: "::Bathroom::glass01.DO".into(),
            parent: String::new(),
            room_id: 0,
            world_mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            world_pos: [0.0; 3],
            local_start_mat: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            local_start_pos: [0.0; 3],
            toggled: false,
            hidden,
            collisions: true,
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
            anim_state: Default::default(),
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
            fov_scale(1.0),
            false,
            false,
            &mut fb,
            &mut zb,
        );
        fb.iter().filter(|px| **px != 0).count()
    }
}
