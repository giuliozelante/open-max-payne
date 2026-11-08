//! Particles and emissions of a runtime particle set (`.rps`), enough to simulate them.
//! Layout: docs/rea/2026-11-03-particle-emitters.md.

use crate::rps::{parse_rps, RpsValue};

/// A sampled ParticleFX graph (`0x71`/3): `channels` values per sample, samples evenly spaced
/// over the particle's life (particle graphs) or the emission cycle (emission graphs).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Graph {
    pub channels: usize,
    /// One sample's channels after another.
    pub samples: Vec<f32>,
}

impl Graph {
    /// Channel values at `u` in 0..1 (clamped), linear between samples.
    pub fn at(&self, u: f32) -> Vec<f32> {
        let count = self.samples.len() / self.channels.max(1);
        let Some(last) = count.checked_sub(1) else {
            return vec![0.0; self.channels];
        };
        let pos = u.clamp(0.0, 1.0) * last as f32;
        let i = (pos.floor() as usize).min(last);
        let j = (i + 1).min(last);
        let w = pos - i as f32;
        (0..self.channels)
            .map(|c| {
                let a = self.samples[i * self.channels + c];
                let b = self.samples[j * self.channels + c];
                a + (b - a) * w
            })
            .collect()
    }
}

/// One bitmap of a particle's sequence: the colour file and, for alpha-blended particles, the
/// alpha file, as embedded (empty when the file was not embedded).
#[derive(Debug, Clone, PartialEq)]
pub struct RpsFrame {
    pub color_path: String,
    pub color: Vec<u8>,
    pub alpha: Option<(String, Vec<u8>)>,
}

/// A particle (`0x30`/4): how one element looks and moves over its life.
#[derive(Debug, Clone, PartialEq)]
pub struct RpsParticle {
    /// "Gravity Multiplier": 1 falls at full gravity, 0 floats.
    pub gravity: f32,
    /// "Animation FPS" of the bitmap sequence.
    pub animation_fps: f32,
    /// "Size m" over life.
    pub size: Graph,
    /// "Rotation °" over life.
    pub rotation: Graph,
    /// "Vertex Alpha" over life, 0..255.
    pub alpha: Graph,
    /// "Rgb color c" over life, 0..255 per channel, multiplied with the bitmap.
    pub rgb: Graph,
    /// HYPOTHESIS: additive blending (no alpha files; `fire\\reelflames\\flames-additive`).
    /// Alpha-blended particles carry a colour and an alpha file per frame (snow: TGA + PCX).
    pub additive: bool,
    /// Bitmap sequences (a particle picks one); each is a list of frames.
    pub sequences: Vec<Vec<RpsFrame>>,
}

/// An emission (`0x40`/2 with inner `0x21`/5): where, how often and how fast particles appear.
#[derive(Debug, Clone, PartialEq)]
pub struct RpsEmission {
    /// "Emission Rate": particles per second, min and max, over the cycle.
    pub rate: Graph,
    /// "Velocity m/s": min and max, over the cycle.
    pub velocity: Graph,
    /// "Lifetime t": seconds, min and max, over the cycle.
    pub lifetime: Graph,
    /// "Emission Direction": X, Y, Z over the cycle.
    pub direction: Graph,
    /// Third vec3 of the emission: "Relative Position", the offset from the emitter.
    pub position: [f32; 3],
    /// Second vec3 (HYPOTHESIS: the "X/Y/Z +/-" direction randomisation).
    pub spread: [f32; 3],
    /// Emission cycle length in seconds (second float; see particle-lights.md).
    pub cycle_length: f32,
    /// Spawn box corners (HYPOTHESIS: the "Square" emission volume; `snowstorm_60m` is ±30/±10/±30).
    pub box_max: [f32; 3],
    pub box_min: [f32; 3],
    /// "Sphere" emitter shape, hotspot and falloff radius; `None` is the box.
    pub sphere: Option<[f32; 2]>,
    /// "Size Multiplier": min and max, over the cycle.
    pub size_multiplier: Graph,
    /// "Rotation Multiplier": min and max, over the cycle.
    pub rotation_multiplier: Graph,
}

/// A particle with the emission that emits it.
#[derive(Debug, Clone, PartialEq)]
pub struct RpsEmitter {
    pub particle: RpsParticle,
    pub emission: RpsEmission,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RpsEffect {
    /// "Max # of particles" (root `uint`).
    pub max_particles: u32,
    pub emitters: Vec<RpsEmitter>,
}

/// Particles and emissions in file order. HYPOTHESIS: each emission emits the particle written
/// just before it (every retail file checked alternates particle, emission).
pub fn rps_effect(data: &[u8]) -> Option<RpsEffect> {
    let values = parse_rps(data).ok()?.values;
    let max_particles = values
        .get(2)
        .and_then(int_value)
        .and_then(|v| u32::try_from(v).ok())?;
    let mut emitters = Vec::new();
    let mut particle: Option<RpsParticle> = None;
    let mut i = 0;
    while i + 4 <= values.len() {
        let header: Vec<Option<i32>> = values[i..i + 4].iter().map(int_value).collect();
        let mut cursor = Cursor {
            values: &values,
            at: i + 4,
        };
        if header == [Some(0x30), Some(4), Some(0x20), Some(3)] {
            if let Some(p) = cursor.particle() {
                particle = Some(p);
                i = cursor.at;
                continue;
            }
        } else if header == [Some(0x40), Some(2), Some(0x21), Some(5)] {
            if let Some(emission) = cursor.emission() {
                if let Some(particle) = particle.take() {
                    emitters.push(RpsEmitter { particle, emission });
                }
                i = cursor.at;
                continue;
            }
        }
        i += 1;
    }
    share_stored_files(&mut emitters);
    Some(RpsEffect {
        max_particles,
        emitters,
    })
}

/// A file is embedded once; later frames that name it carry no bytes. Fill them from the first
/// frame that holds the file, matching names without case.
fn share_stored_files(emitters: &mut [RpsEmitter]) {
    let mut stored: std::collections::BTreeMap<String, Vec<u8>> = Default::default();
    let frames = emitters
        .iter_mut()
        .flat_map(|e| e.particle.sequences.iter_mut().flatten());
    for frame in frames {
        let mut fill = |path: &str, bytes: &mut Vec<u8>| {
            let key = path.to_ascii_lowercase();
            if bytes.is_empty() {
                if let Some(found) = stored.get(&key) {
                    bytes.clone_from(found);
                }
            } else {
                stored.entry(key).or_insert_with(|| bytes.clone());
            }
        };
        fill(&frame.color_path, &mut frame.color);
        if let Some((path, bytes)) = frame.alpha.as_mut() {
            fill(path, bytes);
        }
    }
}

fn int_value(value: &RpsValue) -> Option<i32> {
    match value {
        RpsValue::U8 { value, .. } => Some(i32::from(*value)),
        RpsValue::Int { value } => Some(*value),
        RpsValue::UInt { value } => i32::try_from(*value).ok(),
        _ => None,
    }
}

struct Cursor<'a> {
    values: &'a [RpsValue],
    at: usize,
}

impl Cursor<'_> {
    fn next(&mut self) -> Option<&RpsValue> {
        let value = self.values.get(self.at)?;
        self.at += 1;
        Some(value)
    }

    fn int(&mut self) -> Option<i32> {
        int_value(self.next()?)
    }

    fn float(&mut self) -> Option<f32> {
        match self.next()? {
            RpsValue::F32 { value } => Some(*value),
            _ => None,
        }
    }

    fn vec3(&mut self) -> Option<[f32; 3]> {
        match self.next()? {
            RpsValue::Vec3 { x, y, z } => Some([*x, *y, *z]),
            _ => None,
        }
    }

    fn graph(&mut self) -> Option<Graph> {
        if self.int()? != 0x71 || self.int()? != 3 {
            return None;
        }
        let channels = usize::try_from(self.int()?).ok()?;
        let count = usize::try_from(self.int()?).ok()?;
        let samples = (0..channels * count)
            .map(|_| self.float())
            .collect::<Option<Vec<_>>>()?;
        Some(Graph { channels, samples })
    }

    fn path(&mut self) -> Option<String> {
        match self.next()? {
            RpsValue::Str { value } => Some(value.clone()),
            _ => None,
        }
    }

    /// An embedded file, or an int (0) when the file was not embedded (`FUN_00418190`).
    fn embedded(&mut self) -> Option<Vec<u8>> {
        match self.next()? {
            RpsValue::Blob { data } => Some(data.clone()),
            other => int_value(other).map(|_| Vec::new()),
        }
    }

    /// After `0x30`, 4, `0x20`, 3 (`FUN_00417570`).
    fn particle(&mut self) -> Option<RpsParticle> {
        let gravity = self.float()?;
        self.int()?; // Air resistance, element velocity (HYPOTHESIS).
        self.int()?; // Air resistance, element size (HYPOTHESIS).
        self.float()?; // Air resistance constant (HYPOTHESIS).
        self.int()?; // Animation type (HYPOTHESIS).
        let animation_fps = self.float()?;
        self.int()?;
        self.int()?;
        self.float()?; // Random rotation from.
        self.float()?; // Random rotation to.
        self.int()?; // Random mirroring flags (HYPOTHESIS).
        let size = self.graph()?;
        let rotation = self.graph()?;
        let alpha = self.graph()?;
        let rgb = self.graph()?;
        self.int()?;
        let additive = self.int()? != 0;
        let sequence_count = usize::try_from(self.int()?).ok()?;
        let mut sequences = Vec::with_capacity(sequence_count.min(64));
        for _ in 0..sequence_count {
            let count = usize::try_from(self.int()?).ok()?;
            let mut frames = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                let color_path = self.path()?;
                let color = self.embedded()?;
                let alpha = if additive {
                    None
                } else {
                    Some((self.path()?, self.embedded()?))
                };
                frames.push(RpsFrame {
                    color_path,
                    color,
                    alpha,
                });
            }
            sequences.push(frames);
        }
        Some(RpsParticle {
            gravity,
            animation_fps,
            size,
            rotation,
            alpha,
            rgb,
            additive,
            sequences,
        })
    }

    /// After `0x40`, 2, `0x21`, 5 (`FUN_00418fe0` → `FUN_00415d00`).
    fn emission(&mut self) -> Option<RpsEmission> {
        let rate = self.graph()?;
        let velocity = self.graph()?;
        let lifetime = self.graph()?;
        let direction = self.graph()?;
        self.vec3()?;
        let spread = self.vec3()?;
        let position = self.vec3()?;
        self.float()?;
        let cycle_length = self.float()?;
        self.float()?;
        self.float()?;
        self.int()?;
        self.int()?;
        let shape = self.int()?;
        self.vec3()?;
        for _ in 0..3 {
            self.float()?;
        }
        let hotspot = self.float()?;
        let falloff = self.float()?;
        let box_max = self.vec3()?;
        let box_min = self.vec3()?;
        let size_multiplier = self.graph()?;
        let rotation_multiplier = self.graph()?;
        self.int()?; // Use Light.
        self.graph()?; // Light Color.
        self.graph()?; // Light Falloff.
        self.float()?; // Brightness.
        self.vec3()?; // Light Position Offset.
        Some(RpsEmission {
            rate,
            velocity,
            lifetime,
            direction,
            position,
            spread,
            cycle_length,
            box_max,
            box_min,
            sphere: (shape == 1).then_some([hotspot, falloff]),
            size_multiplier,
            rotation_multiplier,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rps::{encode_rps, RpsValue};

    fn int(value: i32) -> RpsValue {
        RpsValue::Int { value }
    }
    fn f(value: f32) -> RpsValue {
        RpsValue::F32 { value }
    }
    fn v3(x: f32, y: f32, z: f32) -> RpsValue {
        RpsValue::Vec3 { x, y, z }
    }
    fn curve(channels: i32, samples: &[f32]) -> Vec<RpsValue> {
        let mut out = vec![
            int(0x71),
            int(3),
            int(channels),
            int(samples.len() as i32 / channels),
        ];
        out.extend(samples.iter().map(|&s| f(s)));
        out
    }

    /// `0x30`/4 particle as in `snowstorm_60m.rps`: parameter block, four graphs, an int, the
    /// additive flag, then one bitmap sequence (colour and alpha file each; none = additive).
    fn particle(gravity: f32, frames: &[(&str, &[u8], &str, &[u8])]) -> Vec<RpsValue> {
        let mut out = vec![int(0x30), int(4), int(0x20), int(3)];
        out.extend([
            f(gravity),
            int(2),
            int(0),
            f(0.0),
            int(0),
            f(20.0),
            int(1),
            int(1),
        ]);
        out.extend([f(-180.0), f(180.0), RpsValue::UInt { value: 7 }]);
        out.extend(curve(1, &[0.0, 1.0, 0.0]));
        out.extend(curve(1, &[0.0, 90.0]));
        out.extend(curve(1, &[170.0, 170.0]));
        out.extend(curve(3, &[255.0, 128.0, 0.0, 255.0, 128.0, 0.0]));
        let additive = frames.is_empty();
        out.extend([
            int(1),
            int(i32::from(additive)),
            int(1),
            int(frames.len() as i32),
        ]);
        for (color_path, color, alpha_path, alpha) in frames {
            out.push(RpsValue::Str {
                value: color_path.to_string(),
            });
            out.push(RpsValue::Blob {
                data: color.to_vec(),
            });
            out.push(RpsValue::Str {
                value: alpha_path.to_string(),
            });
            out.push(RpsValue::Blob {
                data: alpha.to_vec(),
            });
        }
        out
    }

    /// `0x40`/2 emission with its inner `0x21`/5, light off.
    fn emission(cycle: f32, shape: i32, box_max: [f32; 3], box_min: [f32; 3]) -> Vec<RpsValue> {
        let mut out = vec![int(0x40), int(2), int(0x21), int(5)];
        out.extend(curve(2, &[500.0, 1000.0, 500.0, 1000.0]));
        out.extend(curve(2, &[0.25, 0.5, 0.25, 0.5]));
        out.extend(curve(2, &[0.2, 1.0, 0.2, 1.0]));
        out.extend(curve(3, &[0.0, -1.0, 0.0, 0.0, -1.0, 0.0]));
        out.extend([v3(0.0, -3.0, 12.0), v3(2.0, 2.0, 2.0), v3(0.0, 5.0, -30.0)]);
        out.extend([f(0.8), f(cycle), f(0.0), f(1.0)]);
        out.extend([int(0), int(0), int(shape), v3(0.0, 1.0, 0.0)]);
        out.extend([f(1.0), f(1.0), f(0.0), f(3.0), f(6.0)]);
        out.extend([
            v3(box_max[0], box_max[1], box_max[2]),
            v3(box_min[0], box_min[1], box_min[2]),
        ]);
        out.extend(curve(2, &[0.3, 0.4, 0.3, 0.4]));
        out.extend(curve(2, &[1.0, 1.0, 1.0, 1.0]));
        out.push(int(0));
        out.extend(curve(3, &[0.0; 6]));
        out.extend(curve(1, &[0.0, 0.0]));
        out.extend([f(1.0), v3(0.0, 0.0, 0.0)]);
        out
    }

    fn file(blocks: &[Vec<RpsValue>]) -> Vec<u8> {
        let mut values = vec![int(0x60), int(5), RpsValue::UInt { value: 500 }];
        for block in blocks {
            values.extend(block.iter().cloned());
        }
        encode_rps(&values).unwrap()
    }

    #[test]
    fn reads_each_particle_with_the_emission_that_follows_it() {
        let data = file(&[
            particle(
                0.8,
                &[("snow\\snow0000.TGA", b"tga0", "snow\\snow0000.PCX", b"pcx0")],
            ),
            emission(5.0, 0, [30.0, 10.0, 30.0], [-30.0, -10.0, -30.0]),
            particle(0.0, &[]),
            emission(2.0, 1, [1.0, 1.0, 1.0], [-1.0, -1.0, -1.0]),
        ]);
        let effect = rps_effect(&data).expect("effect");
        assert_eq!(effect.max_particles, 500);
        assert_eq!(effect.emitters.len(), 2);
        let snow = &effect.emitters[0];
        assert_eq!(snow.particle.gravity, 0.8);
        assert!(!snow.particle.additive);
        assert_eq!(snow.particle.sequences.len(), 1);
        let frame = &snow.particle.sequences[0][0];
        assert_eq!(frame.color, b"tga0");
        let alpha = ("snow\\snow0000.PCX".to_string(), b"pcx0".to_vec());
        assert_eq!(frame.alpha, Some(alpha));
        assert_eq!(snow.particle.size.at(0.5), [1.0]);
        assert_eq!(snow.particle.rgb.at(1.0), [255.0, 128.0, 0.0]);
        assert_eq!(snow.emission.cycle_length, 5.0);
        assert_eq!(snow.emission.rate.at(0.0), [500.0, 1000.0]);
        assert_eq!(snow.emission.lifetime.at(0.0), [0.2, 1.0]);
        assert_eq!(snow.emission.direction.at(0.0), [0.0, -1.0, 0.0]);
        assert_eq!(snow.emission.position, [0.0, 5.0, -30.0]);
        assert_eq!(snow.emission.sphere, None);
        assert_eq!(snow.emission.spread, [2.0, 2.0, 2.0]);
        assert_eq!(snow.emission.box_max, [30.0, 10.0, 30.0]);
        assert_eq!(snow.emission.box_min, [-30.0, -10.0, -30.0]);
        assert_eq!(snow.emission.size_multiplier.at(0.0), [0.3, 0.4]);
        assert_eq!(effect.emitters[1].emission.cycle_length, 2.0);
        assert_eq!(effect.emitters[1].emission.sphere, Some([3.0, 6.0]));
        assert!(effect.emitters[1].particle.additive);
        assert_eq!(effect.emitters[1].particle.sequences, [Vec::new()]);
    }

    #[test]
    fn a_frame_stored_once_is_shared_by_later_particles_that_name_it() {
        // `glass_shatter_safety_window_1x2.rps`: the second emitter names the first's shard files
        // and carries no bytes for them.
        let shard = ("debris\\s_glass01_0000.JPG", "debris\\s_glass01_0000.PCX");
        let data = file(&[
            particle(1.0, &[(shard.0, b"jpg0", shard.1, b"pcx0")]),
            emission(0.01, 0, [0.5, 1.0, 0.0], [-0.5, -1.0, 0.0]),
            particle(1.0, &[(&shard.0.to_ascii_lowercase(), b"", shard.1, b"")]),
            emission(0.01, 0, [0.5, 1.0, 0.0], [-0.5, -1.0, 0.0]),
        ]);
        let effect = rps_effect(&data).expect("effect");
        let frame = &effect.emitters[1].particle.sequences[0][0];
        assert_eq!(frame.color, b"jpg0");
        assert_eq!(
            frame.alpha.as_ref().map(|(_, b)| b.as_slice()),
            Some(&b"pcx0"[..])
        );
    }

    #[test]
    fn graphs_interpolate_between_samples() {
        let g = Graph {
            channels: 2,
            samples: vec![0.0, 10.0, 4.0, 30.0],
        };
        assert_eq!(g.at(0.5), [2.0, 20.0]);
        assert_eq!(g.at(2.0), [4.0, 30.0]);
        assert_eq!(g.at(-1.0), [0.0, 10.0]);
    }
}
