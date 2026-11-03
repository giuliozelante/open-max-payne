//! ParticleFX effects at runtime: particles emitted and moved per `.rps` emitter.
//! Layout and evidence: docs/rea/2026-11-03-particle-emitters.md.

use openpfx::effect::RpsEffect;
use std::sync::Arc;

/// Gravity until the level sets its own: ParticleFX's preview default ("Default: 9.81m/s^2").
/// In the game the level's `levels.txt` `Gravity` applies (retail: -20 m/s²); each particle scales
/// it by its "Gravity Multiplier".
const DEFAULT_GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];

/// Which bitmap a sprite shows: effect key, emitter index, sequence and frame.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpriteTexture {
    pub effect: String,
    pub emitter: usize,
    pub sequence: usize,
    pub frame: usize,
}

/// One particle to draw: a camera-facing square of side `size` metres at `pos`.
#[derive(Debug, Clone, PartialEq)]
pub struct Sprite {
    pub pos: [f32; 3],
    pub size: f32,
    /// Colour graph and vertex alpha, 0..1, multiplied with the bitmap.
    pub rgba: [f32; 4],
    pub additive: bool,
    pub texture: SpriteTexture,
}

#[derive(Debug, Clone)]
struct Particle {
    pos: [f32; 3],
    vel: [f32; 3],
    age: f32,
    life: f32,
    size_multiplier: f32,
    sequence: usize,
}

#[derive(Debug, Clone)]
struct Running {
    emitter: String,
    key: String,
    effect: Arc<RpsEffect>,
    looping: bool,
    emitting: bool,
    time: f32,
    /// Fractional particles owed per emitter.
    owed: Vec<f32>,
    /// Live particles per emitter.
    particles: Vec<Vec<Particle>>,
}

/// Every running effect and its particles.
#[derive(Debug, Clone)]
pub struct ParticleField {
    running: Vec<Running>,
    seed: u32,
    /// The level's gravity, m/s².
    pub gravity: [f32; 3],
}

impl Default for ParticleField {
    fn default() -> Self {
        Self {
            running: Vec::new(),
            seed: 0x9e37_79b9,
            gravity: DEFAULT_GRAVITY,
        }
    }
}

impl ParticleField {
    /// `PS_StartEffect` on `emitter`: `key` names the effect's bitmaps for the renderer.
    pub fn start(&mut self, emitter: String, key: String, effect: Arc<RpsEffect>, looping: bool) {
        let n = effect.emitters.len();
        self.running.push(Running {
            emitter,
            key,
            effect,
            looping,
            emitting: true,
            time: 0.0,
            owed: vec![0.0; n],
            particles: vec![Vec::new(); n],
        });
    }

    /// `PS_StopAllEmissions`: no new particles; the live ones finish their lives.
    pub fn stop(&mut self, emitter: &str) {
        for run in &mut self.running {
            if run.emitter.eq_ignore_ascii_case(emitter) {
                run.emitting = false;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }

    /// Advance `dt` seconds. `frame_of(emitter)` is the emitter's world position and rotation
    /// (columns); an emitter that is not found emits nothing.
    pub fn step(&mut self, dt: f32, frame_of: impl Fn(&str) -> Option<([f32; 3], [f32; 9])>) {
        let mut seed = self.seed;
        for run in &mut self.running {
            run.time += dt;
            let frame = frame_of(&run.emitter);
            let mut live: usize = run.particles.iter().map(Vec::len).sum();
            for (k, emitter) in run.effect.emitters.iter().enumerate() {
                let gravity = self.gravity.map(|g| g * emitter.particle.gravity);
                let list = &mut run.particles[k];
                for p in list.iter_mut() {
                    for c in 0..3 {
                        p.vel[c] += gravity[c] * dt;
                        p.pos[c] += p.vel[c] * dt;
                    }
                    p.age += dt;
                }
                let before = list.len();
                list.retain(|p| p.age < p.life - 1e-4);
                live -= before - list.len();

                let emission = &emitter.emission;
                let cycle = emission.cycle_length.max(1e-3);
                let in_cycle = run.looping || run.time <= cycle;
                let Some((origin, rot)) = frame.filter(|_| run.emitting && in_cycle) else {
                    continue;
                };
                let u = (run.time % cycle) / cycle;
                let rate = between(&emission.rate.at(u), &mut seed);
                run.owed[k] += rate * dt;
                while run.owed[k] >= 1.0 - 1e-4 {
                    run.owed[k] -= 1.0;
                    if live >= run.effect.max_particles as usize {
                        continue;
                    }
                    let local: [f32; 3] = std::array::from_fn(|c| {
                        let t = random(&mut seed);
                        emission.position[c]
                            + emission.box_min[c]
                            + (emission.box_max[c] - emission.box_min[c]) * t
                    });
                    let direction = emission.direction.at(u);
                    let dir: [f32; 3] = std::array::from_fn(|c| {
                        direction.get(c).copied().unwrap_or(0.0)
                            + emission.spread[c] * (random(&mut seed) * 2.0 - 1.0)
                    });
                    // HYPOTHESIS: the direction is not normalised; its length scales the
                    // velocity (snowstorm_60m: direction (0, -3, 30) x 0.25..0.5 m/s).
                    let speed = between(&emission.velocity.at(u), &mut seed);
                    let local_vel = dir.map(|d| d * speed);
                    let sequences = emitter.particle.sequences.len().max(1);
                    list.push(Particle {
                        pos: transform(rot, origin, local),
                        vel: transform(rot, [0.0; 3], local_vel),
                        age: 0.0,
                        life: between(&emission.lifetime.at(u), &mut seed).max(1e-3),
                        size_multiplier: between(&emission.size_multiplier.at(u), &mut seed),
                        sequence: (random(&mut seed) * sequences as f32) as usize % sequences,
                    });
                    live += 1;
                }
            }
        }
        self.seed = seed;
        self.running.retain(|run| {
            let alive = run.particles.iter().any(|p| !p.is_empty());
            let cycle = run
                .effect
                .emitters
                .iter()
                .map(|e| e.emission.cycle_length)
                .fold(0.0, f32::max);
            alive || (run.emitting && (run.looping || run.time <= cycle))
        });
    }

    /// Every live particle as a sprite.
    pub fn sprites(&self) -> Vec<Sprite> {
        let mut out = Vec::new();
        for run in &self.running {
            for (k, emitter) in run.effect.emitters.iter().enumerate() {
                let particle = &emitter.particle;
                for p in &run.particles[k] {
                    let v = p.age / p.life;
                    let rgb = particle.rgb.at(v);
                    let channel =
                        |c: usize| (rgb.get(c).copied().unwrap_or(255.0) / 255.0).clamp(0.0, 1.0);
                    let alpha = (particle.alpha.at(v).first().copied().unwrap_or(255.0) / 255.0)
                        .clamp(0.0, 1.0);
                    let frames = particle
                        .sequences
                        .get(p.sequence)
                        .map_or(1, |s| s.len().max(1));
                    let frame = if particle.animation_fps > 0.0 {
                        (p.age * particle.animation_fps) as usize % frames
                    } else {
                        ((v * frames as f32) as usize).min(frames - 1)
                    };
                    out.push(Sprite {
                        pos: p.pos,
                        size: particle.size.at(v).first().copied().unwrap_or(0.0)
                            * p.size_multiplier,
                        rgba: [channel(0), channel(1), channel(2), alpha],
                        additive: particle.additive,
                        texture: SpriteTexture {
                            effect: run.key.clone(),
                            emitter: k,
                            sequence: p.sequence,
                            frame,
                        },
                    });
                }
            }
        }
        out
    }
}

/// The bitmap of one frame as an RGBA texture (alpha in the top byte). The colour file is a JPEG or
/// TGA; the alpha file, when the particle is alpha-blended, is a grey PCX or TGA mask. Additive
/// particles have no alpha file and are opaque.
pub fn frame_texture(frame: &openpfx::effect::RpsFrame) -> Option<crate::scene::DecodedTexture> {
    let (width, height, color) = crate::scene::decode_texture_blob(&frame.color)?;
    let alpha = match &frame.alpha {
        None => None,
        Some((_, bytes)) => decode_pcx_grey(bytes)
            .or_else(|| {
                crate::scene::decode_texture_blob(bytes)
                    .map(|(w, h, px)| (w, h, px.iter().map(|p| (p >> 16) as u8).collect()))
            })
            .filter(|(w, h, _)| (*w, *h) == (width, height))
            .map(|(_, _, grey)| grey),
    };
    let pixels = color
        .iter()
        .enumerate()
        .map(|(i, &rgb)| {
            let a = alpha
                .as_ref()
                .map_or(0xff, |grey| grey.get(i).copied().unwrap_or(0xff));
            (u32::from(a) << 24) | (rgb & 0x00ff_ffff)
        })
        .collect();
    Some(crate::scene::DecodedTexture {
        path: frame.color_path.clone(),
        flag: 0,
        width,
        height,
        pixels,
    })
}

/// 8-bit, one-plane RLE PCX as a grey image (the red channel of its 256-colour palette).
fn decode_pcx_grey(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if data.len() < 128 || data[0] != 0x0a || data[2] != 1 || data[3] != 8 || data[65] != 1 {
        return None;
    }
    let word = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]);
    let width = u32::from(word(8).checked_sub(word(4))?) + 1;
    let height = u32::from(word(10).checked_sub(word(6))?) + 1;
    let stride = usize::from(word(66));
    let mut rows = Vec::with_capacity(stride * height as usize);
    let mut i = 128;
    while rows.len() < stride * height as usize {
        let byte = *data.get(i)?;
        i += 1;
        if byte >= 0xc0 {
            let value = *data.get(i)?;
            i += 1;
            rows.extend(std::iter::repeat_n(value, usize::from(byte & 0x3f)));
        } else {
            rows.push(byte);
        }
    }
    let palette =
        (data.len() >= 769 && data[data.len() - 769] == 0x0c).then(|| &data[data.len() - 768..]);
    let grey = (0..height as usize)
        .flat_map(|y| (0..width as usize).map(move |x| (y, x)))
        .map(|(y, x)| {
            let index = rows[y * stride + x];
            palette.map_or(index, |p| p[usize::from(index) * 3])
        })
        .collect();
    Some((width, height, grey))
}

/// A random value between a two-channel graph's min and max (a one-channel graph is constant).
fn between(minmax: &[f32], seed: &mut u32) -> f32 {
    match minmax {
        [lo, hi, ..] => lo + (hi - lo) * random(seed),
        [v] => *v,
        [] => 0.0,
    }
}

/// xorshift32 in 0..1.
fn random(seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed >> 8) as f32 / (1u32 << 24) as f32
}

/// `rot` (columns) times `v`, plus `origin`.
fn transform(rot: [f32; 9], origin: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| origin[r] + rot[r] * v[0] + rot[3 + r] * v[1] + rot[6 + r] * v[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use openpfx::effect::{Graph, RpsEffect, RpsEmission, RpsEmitter, RpsFrame, RpsParticle};

    fn flat(channels: usize, values: &[f32]) -> Graph {
        Graph {
            channels,
            samples: [values, values].concat(),
        }
    }

    /// One emitter: `rate` per second, `life` seconds, straight down at 2 m/s, from a point.
    fn effect(rate: f32, life: f32, gravity: f32) -> RpsEffect {
        RpsEffect {
            max_particles: 1000,
            emitters: vec![RpsEmitter {
                particle: RpsParticle {
                    gravity,
                    animation_fps: 0.0,
                    size: Graph {
                        channels: 1,
                        samples: vec![0.0, 2.0],
                    },
                    rotation: flat(1, &[0.0]),
                    alpha: flat(1, &[255.0]),
                    rgb: flat(3, &[255.0, 128.0, 0.0]),
                    additive: false,
                    sequences: vec![vec![RpsFrame {
                        color_path: "a.tga".into(),
                        color: Vec::new(),
                        alpha: None,
                    }]],
                },
                emission: RpsEmission {
                    rate: flat(2, &[rate, rate]),
                    velocity: flat(2, &[2.0, 2.0]),
                    lifetime: flat(2, &[life, life]),
                    direction: flat(3, &[0.0, -1.0, 0.0]),
                    position: [0.0, 0.0, 0.0],
                    spread: [0.0; 3],
                    cycle_length: 1.0,
                    box_max: [0.0; 3],
                    box_min: [0.0; 3],
                    size_multiplier: flat(2, &[1.0, 1.0]),
                    rotation_multiplier: flat(2, &[1.0, 1.0]),
                },
            }],
        }
    }

    const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

    fn at_origin(_: &str) -> Option<([f32; 3], [f32; 9])> {
        Some(([0.0, 10.0, 0.0], IDENTITY))
    }

    /// 8-bit, one-plane, RLE PCX (version 5) with a grey palette, as the snow alpha masks.
    fn pcx(width: u16, height: u16, rows: &[&[u8]]) -> Vec<u8> {
        let mut out = vec![0u8; 128];
        out[..4].copy_from_slice(&[0x0a, 5, 1, 8]);
        out[8..10].copy_from_slice(&(width - 1).to_le_bytes());
        out[10..12].copy_from_slice(&(height - 1).to_le_bytes());
        out[65] = 1;
        out[66..68].copy_from_slice(&width.to_le_bytes());
        for row in rows {
            for &v in *row {
                // A run of one, the RLE form for values >= 0xc0; plain bytes otherwise.
                if v >= 0xc0 {
                    out.extend([0xc1, v]);
                } else {
                    out.push(v);
                }
            }
        }
        out.push(0x0c);
        for i in 0..=255u8 {
            out.extend([i, i, i]);
        }
        out
    }

    fn tga(width: u16, height: u16, bgr: &[[u8; 3]]) -> Vec<u8> {
        let mut out = vec![0u8; 18];
        out[2] = 2;
        out[12..14].copy_from_slice(&width.to_le_bytes());
        out[14..16].copy_from_slice(&height.to_le_bytes());
        out[16] = 24;
        out[17] = 0x20; // top-left origin
        for px in bgr {
            out.extend(px);
        }
        out
    }

    #[test]
    fn a_frame_takes_its_colour_from_one_file_and_its_alpha_from_the_other() {
        // Snow: snow0000.TGA (colour) + snow0000.PCX (alpha mask).
        let frame = RpsFrame {
            color_path: "snow0000.TGA".into(),
            color: tga(2, 1, &[[0, 0, 255], [255, 0, 0]]),
            alpha: Some(("snow0000.PCX".into(), pcx(2, 1, &[&[0x10, 0xf0]]))),
        };
        let texture = frame_texture(&frame).expect("texture");
        assert_eq!((texture.width, texture.height), (2, 1));
        assert_eq!(texture.pixels, [0x10ff_0000, 0xf000_00ff]);
        // Additive particles have no alpha file: fully opaque.
        let additive = RpsFrame {
            alpha: None,
            ..frame
        };
        assert_eq!(
            frame_texture(&additive).unwrap().pixels,
            [0xffff_0000, 0xff00_00ff]
        );
    }

    #[test]
    fn an_emitter_spawns_at_its_rate_and_particles_die_with_their_lifetime() {
        let mut field = ParticleField::default();
        field.start(
            "::room::snow".into(),
            "snow".into(),
            std::sync::Arc::new(effect(100.0, 0.5, 0.0)),
            true,
        );
        for _ in 0..10 {
            field.step(0.01, at_origin);
        }
        // 100 per second for 0.1 s.
        assert_eq!(field.sprites().len(), 10);
        for _ in 0..100 {
            field.step(0.01, at_origin);
        }
        // Steady state: rate x lifetime.
        assert_eq!(field.sprites().len(), 50);
    }

    #[test]
    fn particles_move_with_their_velocity_and_gravity() {
        let mut field = ParticleField::default();
        field.start(
            "e".into(),
            "k".into(),
            std::sync::Arc::new(effect(10.0, 5.0, 1.0)),
            false,
        );
        field.step(0.1, at_origin);
        field.stop("e");
        for _ in 0..10 {
            field.step(0.1, at_origin);
        }
        let sprites = field.sprites();
        assert_eq!(sprites.len(), 1);
        // Spawned at y = 10 going down at 2 m/s, then 1.0 s of motion with 9.81 m/s^2.
        let y = sprites[0].pos[1];
        let expected = 10.0 - 2.0 * 1.0 - 0.5 * 9.81 * 1.0;
        assert!(
            (y - expected).abs() < 0.6,
            "y = {y}, expected about {expected}"
        );
    }

    #[test]
    fn particles_fall_with_the_level_gravity() {
        // levels.txt `Gravity = ( 0, GRAVITY_VALUE , 0 )` with `GRAVITY_VALUE -20`; the tutorial:
        // the editor's gravity "does not affect the gravity setting used in the game".
        let mut def = effect(10.0, 5.0, 0.5);
        def.emitters[0].emission.velocity = flat(2, &[0.0, 0.0]);
        let mut field = ParticleField::default();
        field.gravity = [0.0, -20.0, 0.0];
        field.start("e".into(), "k".into(), std::sync::Arc::new(def), false);
        field.step(0.1, at_origin);
        field.stop("e");
        field.step(1.0, at_origin);
        // One step of 1 s at half of -20 m/s^2: velocity -10, moved -10 (semi-implicit).
        let y = field.sprites()[0].pos[1];
        assert!((y - 0.0).abs() < 1e-3, "y = {y}");
    }

    #[test]
    fn the_direction_graph_scales_the_velocity() {
        // snowstorm_60m: Emission Direction (0, -3, 30) with Velocity 0.25..0.5 is a sideways storm,
        // not flakes drifting at half a metre per second.
        let mut def = effect(10.0, 5.0, 0.0);
        def.emitters[0].emission.direction = flat(3, &[0.0, -3.0, 30.0]);
        def.emitters[0].emission.velocity = flat(2, &[0.5, 0.5]);
        let mut field = ParticleField::default();
        field.start("e".into(), "k".into(), std::sync::Arc::new(def), false);
        field.step(0.1, at_origin);
        field.stop("e");
        field.step(1.0, at_origin);
        let p = field.sprites()[0].pos;
        assert!((p[2] - 15.0).abs() < 1e-3, "{p:?}");
        assert!((p[1] - (10.0 - 1.5)).abs() < 1e-3, "{p:?}");
    }

    #[test]
    fn a_sprite_takes_size_colour_and_alpha_from_the_graphs_over_its_life() {
        let mut field = ParticleField::default();
        field.start(
            "e".into(),
            "k".into(),
            std::sync::Arc::new(effect(10.0, 2.0, 0.0)),
            false,
        );
        field.step(0.1, at_origin);
        field.stop("e");
        for _ in 0..10 {
            field.step(0.1, at_origin);
        }
        let sprite = field.sprites()[0].clone();
        // Half way through a 2 s life: the size graph 0 -> 2 is at 1.
        assert!((sprite.size - 1.0).abs() < 0.06, "size {}", sprite.size);
        assert_eq!(sprite.rgba, [1.0, 128.0 / 255.0, 0.0, 1.0]);
        assert!(!sprite.additive);
        assert_eq!(
            sprite.texture,
            SpriteTexture {
                effect: "k".into(),
                emitter: 0,
                sequence: 0,
                frame: 0
            }
        );
    }

    #[test]
    fn a_stopped_effect_ends_once_its_particles_are_gone() {
        let mut field = ParticleField::default();
        field.start(
            "e".into(),
            "k".into(),
            std::sync::Arc::new(effect(100.0, 0.2, 0.0)),
            true,
        );
        field.step(0.1, at_origin);
        field.stop("E");
        field.step(0.5, at_origin);
        assert!(field.sprites().is_empty());
        assert!(field.is_empty());
    }
}
