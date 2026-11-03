//! ParticleFX effects at runtime: particles emitted and moved per `.rps` emitter.
//! Layout and evidence: docs/rea/2026-11-03-particle-emitters.md.

use openpfx::effect::RpsEffect;
use std::sync::Arc;

/// ParticleFX's gravity (tutorial: "Default: 9.81m/s^2"); each particle scales it by its
/// "Gravity Multiplier".
const GRAVITY: f32 = 9.81;

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
}

impl Default for ParticleField {
    fn default() -> Self {
        Self {
            running: Vec::new(),
            seed: 0x9e37_79b9,
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
                let gravity = GRAVITY * emitter.particle.gravity;
                let list = &mut run.particles[k];
                for p in list.iter_mut() {
                    p.vel[1] -= gravity * dt;
                    for c in 0..3 {
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
                    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
                    let speed = between(&emission.velocity.at(u), &mut seed);
                    let local_vel = dir.map(|d| if len > 1e-6 { d / len * speed } else { 0.0 });
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
