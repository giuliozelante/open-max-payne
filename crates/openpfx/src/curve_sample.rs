//! Sample editor keyframes onto a regular time grid (`FUN_00432fd0` outer loop).
//!
//! Full retail uses `FUN_00432240` + STL keyframe lists; this is linear
//! interpolation between `(t, channels[])` knots — enough to emit RPS curves
//! once PSE keyframes are parsed.

use crate::PfxError;

/// One keyframe: time + per-channel values.
#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe {
    pub t: f32,
    pub values: Vec<f32>,
}

/// Sample `keyframes` onto `sample_count` points from `t_min`..=`t_max` (inclusive).
///
/// Output layout matches `FUN_00432fd0`: sample-major, then channels
/// (`sample0.ch0, sample0.ch1, …, sample1.ch0, …`).
pub fn sample_linear(
    keyframes: &[Keyframe],
    channels: usize,
    t_min: f32,
    t_max: f32,
    sample_count: usize,
) -> Result<Vec<f32>, PfxError> {
    if channels == 0 || sample_count == 0 {
        return Err(PfxError::Msg(
            "channels and sample_count must be > 0".into(),
        ));
    }
    if keyframes.is_empty() {
        return Err(PfxError::Msg("need at least one keyframe".into()));
    }
    for kf in keyframes {
        if kf.values.len() != channels {
            return Err(PfxError::Msg(format!(
                "keyframe channels {} != {channels}",
                kf.values.len()
            )));
        }
    }
    let mut sorted = keyframes.to_vec();
    sorted.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));

    let mut out = Vec::with_capacity(channels * sample_count);
    for i in 0..sample_count {
        let t = if sample_count == 1 {
            t_min
        } else {
            t_min + (t_max - t_min) * (i as f32) / ((sample_count - 1) as f32)
        };
        let vals = eval_at(&sorted, t);
        out.extend_from_slice(&vals);
    }
    Ok(out)
}

fn eval_at(sorted: &[Keyframe], t: f32) -> Vec<f32> {
    if t <= sorted[0].t {
        return sorted[0].values.clone();
    }
    if t >= sorted[sorted.len() - 1].t {
        return sorted[sorted.len() - 1].values.clone();
    }
    for w in sorted.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t >= a.t && t <= b.t {
            let span = b.t - a.t;
            let u = if span.abs() < f32::EPSILON {
                0.0
            } else {
                (t - a.t) / span
            };
            return a
                .values
                .iter()
                .zip(b.values.iter())
                .map(|(x, y)| x + (y - x) * u)
                .collect();
        }
    }
    sorted[sorted.len() - 1].values.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_two_keyframes_one_channel() {
        let kfs = vec![
            Keyframe {
                t: 0.0,
                values: vec![0.0],
            },
            Keyframe {
                t: 1.0,
                values: vec![2.0],
            },
        ];
        let grid = sample_linear(&kfs, 1, 0.0, 1.0, 3).unwrap();
        assert_eq!(grid.len(), 3);
        assert!((grid[0] - 0.0).abs() < 1e-5);
        assert!((grid[1] - 1.0).abs() < 1e-5);
        assert!((grid[2] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn sample_major_two_channels() {
        let kfs = vec![
            Keyframe {
                t: 0.0,
                values: vec![0.0, 10.0],
            },
            Keyframe {
                t: 1.0,
                values: vec![2.0, 20.0],
            },
        ];
        let grid = sample_linear(&kfs, 2, 0.0, 1.0, 2).unwrap();
        assert_eq!(grid, vec![0.0, 10.0, 2.0, 20.0]);
    }
}
