//! Emitter point lights in a runtime particle set (`.rps`).
//! Layout: docs/rea/2026-11-03-particle-lights.md.

use crate::rps::{parse_rps, RpsValue};

/// One emitter's Gouraud point light (ParticleFX "Use Light").
#[derive(Debug, Clone, PartialEq)]
pub struct RpsLight {
    /// Emission cycle length in seconds; the graphs span one cycle.
    pub cycle_length: f32,
    /// "Light Color" graph samples, RGB 0..255, evenly spaced over the cycle.
    pub color: Vec<[f32; 3]>,
    /// "Light Falloff" graph samples (light radius, metres), evenly spaced over the cycle.
    pub falloff: Vec<f32>,
    /// "Light Position Offset" from the emitter.
    pub offset: [f32; 3],
}

impl RpsLight {
    /// Colour and radius at `t` seconds into the cycle (clamped to the cycle).
    pub fn sample(&self, t: f32) -> ([f32; 3], f32) {
        let u = if self.cycle_length > 0.0 {
            (t / self.cycle_length).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let color = lerp_samples(&self.color, u);
        let falloff = lerp_samples(&self.falloff.iter().map(|&r| [r]).collect::<Vec<_>>(), u);
        (color, falloff[0])
    }
}

fn lerp_samples<const N: usize>(samples: &[[f32; N]], u: f32) -> [f32; N] {
    let Some(last) = samples.len().checked_sub(1) else {
        return [0.0; N];
    };
    let at = u * last as f32;
    let i = (at.floor() as usize).min(last);
    let j = (i + 1).min(last);
    let w = at - i as f32;
    std::array::from_fn(|c| samples[i][c] + (samples[j][c] - samples[i][c]) * w)
}

/// Lights of every emitter with "Use Light" on, in file order.
pub fn rps_lights(data: &[u8]) -> Vec<RpsLight> {
    let Ok(header) = parse_rps(data) else {
        return Vec::new();
    };
    let values = header.values;
    let mut lights = Vec::new();
    let mut i = 0;
    while i + 4 <= values.len() {
        let starts_companion = [0x40, 2, 0x21, 5]
            .iter()
            .enumerate()
            .all(|(k, &want)| int_value(&values[i + k]) == Some(want));
        if starts_companion {
            let mut cursor = Cursor {
                values: &values,
                at: i + 4,
            };
            if let Some(light) = cursor.companion_light() {
                lights.extend(light);
                i = cursor.at;
                continue;
            }
        }
        i += 1;
    }
    lights
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

    /// Sampled graph (`FUN_00432fd0`): `0x71`, 3, channels, samples, then the f32 samples,
    /// one sample's channels after another.
    fn curve(&mut self) -> Option<(usize, Vec<f32>)> {
        if self.int()? != 0x71 || self.int()? != 3 {
            return None;
        }
        let channels = usize::try_from(self.int()?).ok()?;
        let samples = usize::try_from(self.int()?).ok()?;
        let floats = (0..channels * samples)
            .map(|_| self.float())
            .collect::<Option<Vec<_>>>()?;
        Some((channels, floats))
    }

    /// The rest of a `0x40`/2 block after its headers. `Some(None)` when the light is off.
    fn companion_light(&mut self) -> Option<Option<RpsLight>> {
        // Inner `0x21`/5 (`FUN_00415d00`).
        for _ in 0..4 {
            self.curve()?;
        }
        for _ in 0..3 {
            self.vec3()?;
        }
        self.float()?;
        let cycle_length = self.float()?;
        self.float()?;
        self.float()?;
        for _ in 0..3 {
            self.int()?;
        }
        self.vec3()?;
        for _ in 0..5 {
            self.float()?;
        }
        self.vec3()?;
        self.vec3()?;
        // `+0xcc`, `+0xd0` graphs, then `+0xd4` Use Light.
        self.curve()?;
        self.curve()?;
        let use_light = self.int()?;
        let (color_channels, color) = self.curve()?;
        let (falloff_channels, falloff) = self.curve()?;
        self.float()?; // Brightness: "no effect in the current version of ParticleFX".
        let offset = self.vec3()?;
        if color_channels != 3 || falloff_channels != 1 {
            return None;
        }
        Some((use_light != 0).then(|| RpsLight {
            cycle_length,
            color: color.as_chunks::<3>().0.to_vec(),
            falloff,
            offset,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rps::encode_rps;

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

    /// Companion block `0x40`/2 as `FUN_00418fe0` writes it, with inner `0x21`/5.
    fn companion(cycle: f32, use_light: i32, color: &[f32], falloff: &[f32]) -> Vec<RpsValue> {
        let mut out = vec![int(0x40), int(2), int(0x21), int(5)];
        for _ in 0..3 {
            out.extend(curve(2, &[0.0; 4]));
        }
        out.extend(curve(3, &[0.0; 6]));
        out.extend([v3(0.0, 0.0, 0.0), v3(0.0, 0.0, 0.0), v3(0.0, 0.0, 0.0)]);
        out.extend([f(0.8), f(cycle), f(0.0), f(1.0)]);
        out.extend([int(0), int(1), int(1), v3(0.0, 1.0, 0.0)]);
        out.extend([f(1.0), f(1.0), f(0.0), f(0.0), f(0.0)]);
        out.extend([v3(1.0, 1.0, 1.0), v3(0.0, 0.0, 0.0)]);
        out.extend(curve(2, &[0.0; 4]));
        out.extend(curve(2, &[0.0; 4]));
        out.push(int(use_light));
        out.extend(curve(3, color));
        out.extend(curve(1, falloff));
        out.extend([f(4.0), v3(0.0, 0.5, 0.0)]);
        out
    }

    fn file(blocks: &[Vec<RpsValue>]) -> Vec<u8> {
        let mut values = vec![int(0x60), int(5)];
        for block in blocks {
            values.extend(block.iter().cloned());
        }
        encode_rps(&values).unwrap()
    }

    #[test]
    fn reads_the_light_of_an_emitter_with_use_light_on() {
        let data = file(&[companion(
            0.5,
            1,
            &[255.0, 0.0, 0.0, 0.0, 0.0, 255.0],
            &[5.0, 5.0],
        )]);
        assert_eq!(
            rps_lights(&data),
            [RpsLight {
                cycle_length: 0.5,
                color: vec![[255.0, 0.0, 0.0], [0.0, 0.0, 255.0]],
                falloff: vec![5.0, 5.0],
                offset: [0.0, 0.5, 0.0],
            }]
        );
    }

    #[test]
    fn skips_emitters_without_a_light() {
        let data = file(&[
            companion(1.0, 0, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[2.0, 2.0]),
            companion(2.0, 1, &[9.0, 9.0, 9.0, 9.0, 9.0, 9.0], &[3.0, 3.0]),
        ]);
        let lights = rps_lights(&data);
        assert_eq!(lights.len(), 1);
        assert_eq!(lights[0].cycle_length, 2.0);
    }

    #[test]
    fn samples_interpolate_linearly_over_the_cycle() {
        let light = RpsLight {
            cycle_length: 2.0,
            color: vec![[0.0, 0.0, 0.0], [200.0, 100.0, 0.0]],
            falloff: vec![4.0, 8.0],
            offset: [0.0; 3],
        };
        assert_eq!(light.sample(1.0), ([100.0, 50.0, 0.0], 6.0));
        assert_eq!(light.sample(5.0), ([200.0, 100.0, 0.0], 8.0));
    }
}
