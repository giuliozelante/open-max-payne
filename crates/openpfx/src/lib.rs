//! ParticleFX editor project (`.pse`) + runtime (`.rps`) tooling — draft.
//! Spec: docs/specs/rps_format.md

pub mod curve_sample;
pub mod memory_file;
pub mod rps;

use std::path::Path;
use thiserror::Error;

pub use curve_sample::{sample_linear, Keyframe as CurveKeyframe};

/// Interpret [`PseCurveFloats::floats`] as interleaved `(t, v)` for one channel.
pub fn pse_curve_to_keyframes(parsed: &PseCurveFloats) -> Result<Vec<CurveKeyframe>, PfxError> {
    if !parsed.floats.len().is_multiple_of(2) {
        return Err(PfxError::Msg(format!(
            "curve {}: odd float count {}",
            parsed.name,
            parsed.floats.len()
        )));
    }
    let mut out = Vec::with_capacity(parsed.floats.len() / 2);
    for pair in parsed.floats.as_chunks::<2>().0 {
        out.push(CurveKeyframe {
            t: pair[0],
            values: vec![pair[1]],
        });
    }
    Ok(out)
}
pub use memory_file::{obj as rps_obj, MemoryFileWriter};
pub use rps::{
    encode_rps, parse_rps, pse_header_to_rps_preamble, read_rps, roundtrip_rps, RpsHeader, RpsValue,
};

#[derive(Debug, Error)]
pub enum PfxError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Msg(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PseHeader {
    pub word0: u32,
    pub word1: u32,
    pub word2: u32,
    /// LE float at +0x0C (observed 2.5 on Bullet_Impact_default.pse).
    pub float_a: f32,
    /// LE u32 at +0x10 (observed 10 on that fixture — not a float).
    pub word3: u32,
    /// LE float at +0x14 (observed 2.5).
    pub float_b: f32,
    /// LE u32 at +0x18 — streamed as uint in RPS after `float_b` (`FUN_0041ab00`).
    pub word4: u32,
    /// LE u32 at +0x1C / +0x20 — streamed as int tags before the graph section.
    pub flag_a: u32,
    pub flag_b: u32,
    pub ascii_paths: Vec<String>,
}

/// Best-effort PSE header + absolute-path harvest.
pub fn read_pse(path: &Path) -> Result<PseHeader, PfxError> {
    let data = std::fs::read(path)?;
    parse_pse(&data)
}

pub fn parse_pse(data: &[u8]) -> Result<PseHeader, PfxError> {
    if data.len() < 36 {
        return Err(PfxError::Msg(format!(
            "PSE too short ({} bytes)",
            data.len()
        )));
    }
    let word0 = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let word1 = u32::from_le_bytes(data[4..8].try_into().unwrap());
    let word2 = u32::from_le_bytes(data[8..12].try_into().unwrap());
    let float_a = f32::from_le_bytes(data[12..16].try_into().unwrap());
    let word3 = u32::from_le_bytes(data[16..20].try_into().unwrap());
    let float_b = f32::from_le_bytes(data[20..24].try_into().unwrap());
    let word4 = u32::from_le_bytes(data[24..28].try_into().unwrap());
    let flag_a = u32::from_le_bytes(data[28..32].try_into().unwrap());
    let flag_b = u32::from_le_bytes(data[32..36].try_into().unwrap());

    let mut ascii_paths = Vec::new();
    let mut i = 0;
    while i + 3 < data.len() {
        if &data[i..i + 3].to_ascii_uppercase() == b"C:\\" {
            let mut j = i;
            while j < data.len() && data[j] != 0 && data[j] >= 32 {
                j += 1;
            }
            ascii_paths.push(String::from_utf8_lossy(&data[i..j]).into_owned());
            i = j;
        } else {
            i += 1;
        }
    }

    Ok(PseHeader {
        word0,
        word1,
        word2,
        float_a,
        word3,
        float_b,
        word4,
        flag_a,
        flag_b,
        ascii_paths,
    })
}

/// Emitter stub found by scanning raw LE object headers in a `.pse`.
///
/// Editor format stores type/version as untagged `u32` pairs (not MemoryFile tags).
/// Pattern (BYOD `Bullet_Impact_default.pse`): `0x30`/`4` + `0x20`/`3` + two `u32` +
/// `u8` length-prefixed ASCII name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PseEmitterRef {
    pub offset: usize,
    pub name: String,
}

/// Count raw `0x71`/`3` curve object headers in a `.pse` (editor keyframe blocks).
pub fn scan_pse_curve_count(data: &[u8]) -> usize {
    scan_pse_curves(data).len()
}

/// Curve header found in a `.pse`: `0x71`/`3` + `u8` + `'t'` + len-prefixed name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PseCurveRef {
    pub offset: usize,
    pub name: String,
    /// Byte after the name; keyframe body starts here (layout still WIP).
    pub body_offset: usize,
}

/// Keyframe floats extracted between the PSE curve name and a `PPP` (`50 50 50`) marker.
#[derive(Debug, Clone, PartialEq)]
pub struct PseCurveFloats {
    pub name: String,
    /// Seven LE `u32` words immediately after the name (semantics WIP).
    pub header_words: [u32; 7],
    /// LE `f32` values until the `50 50 50` sentinel (paired time/value in simple cases).
    pub floats: Vec<f32>,
}

/// Best-effort: name + header words + float run until `PPP` for one [`PseCurveRef`].
pub fn parse_pse_curve_floats(data: &[u8], curve: &PseCurveRef) -> Option<PseCurveFloats> {
    let mut i = curve.body_offset;
    if i + 28 > data.len() {
        return None;
    }
    let mut header_words = [0u32; 7];
    for w in &mut header_words {
        *w = u32::from_le_bytes(data[i..i + 4].try_into().ok()?);
        i += 4;
    }
    let mut floats = Vec::new();
    // Bound the run: PPP sentinel, next object header, or a hard cap.
    const MAX_FLOATS: usize = 64;
    while i + 4 <= data.len() && floats.len() < MAX_FLOATS {
        if data[i] == 0x50 && data[i + 1] == 0x50 && data[i + 2] == 0x50 {
            break;
        }
        if data[i..].starts_with(&[0x71, 0, 0, 0]) || data[i..].starts_with(&[0x30, 0, 0, 0]) {
            break;
        }
        floats.push(f32::from_le_bytes(data[i..i + 4].try_into().ok()?));
        i += 4;
    }
    if floats.is_empty() {
        return None;
    }
    // Drop a trailing odd float if we stopped on a boundary mid-pair.
    if floats.len() % 2 == 1 {
        floats.pop();
    }
    if floats.is_empty() {
        return None;
    }
    Some(PseCurveFloats {
        name: curve.name.clone(),
        header_words,
        floats,
    })
}

/// Scan `.pse` for curve object headers and their editor names.
pub fn scan_pse_curves(data: &[u8]) -> Vec<PseCurveRef> {
    const CURVE: &[u8] = &[0x71, 0, 0, 0, 3, 0, 0, 0];
    let mut out = Vec::new();
    let mut i = 0;
    while i + CURVE.len() + 3 < data.len() {
        if &data[i..i + CURVE.len()] != CURVE {
            i += 1;
            continue;
        }
        let mut j = i + CURVE.len();
        // Observed: u8 (often 1) + 't' + u8 len + ascii name
        if j + 2 >= data.len() || data[j + 1] != b't' {
            i += 1;
            continue;
        }
        j += 2; // skip channel-ish byte + 't'
        let len = data[j] as usize;
        j += 1;
        if j + len > data.len() {
            i += 1;
            continue;
        }
        let name = String::from_utf8_lossy(&data[j..j + len]).into_owned();
        j += len;
        if name.chars().all(|c| c.is_ascii_graphic() || c == ' ') && !name.is_empty() {
            out.push(PseCurveRef {
                offset: i,
                name,
                body_offset: j,
            });
        }
        i += CURVE.len();
    }
    out
}

/// Scan `.pse` bytes for emitter object headers and their names.
pub fn scan_pse_emitters(data: &[u8]) -> Vec<PseEmitterRef> {
    const EMIT: &[u8] = &[0x30, 0, 0, 0, 4, 0, 0, 0, 0x20, 0, 0, 0, 3, 0, 0, 0];
    let mut out = Vec::new();
    let mut i = 0;
    while i + EMIT.len() + 8 < data.len() {
        if &data[i..i + EMIT.len()] != EMIT {
            i += 1;
            continue;
        }
        let name_at = i + EMIT.len() + 8; // skip two u32 after param header
        if name_at >= data.len() {
            break;
        }
        let len = data[name_at] as usize;
        let start = name_at + 1;
        if start + len > data.len() {
            i += 1;
            continue;
        }
        let name = String::from_utf8_lossy(&data[start..start + len]).into_owned();
        if name.chars().all(|c| c.is_ascii_graphic() || c == ' ') && !name.is_empty() {
            out.push(PseEmitterRef { offset: i, name });
        }
        i += EMIT.len();
    }
    out
}

/// PSE flat fields → RPS root object through pre-graph ints (`FUN_0041ab00` before graphs).
pub fn pse_to_rps_root(h: &PseHeader) -> Result<Vec<u8>, PfxError> {
    pse_to_rps_root_into(h, false)
}

/// Options for [`pse_to_rps_root_into`].
#[derive(Debug, Clone, Default)]
pub struct PseExportOptions {
    /// Append graph section with zero emitters (`0x31`/`1` + param + count `0`).
    pub empty_graph: bool,
    /// Append graph section + stub emitter-open (+ 4 minimal curves + companion open)
    /// per name. **Not** game-complete — texture embeds / companion body incomplete.
    pub emitter_stubs: Vec<String>,
}

/// Like [`pse_to_rps_root`], with optional graph / emitter stubs.
pub fn pse_to_rps_root_into(h: &PseHeader, empty_graph: bool) -> Result<Vec<u8>, PfxError> {
    pse_to_rps_export(
        h,
        &PseExportOptions {
            empty_graph,
            ..Default::default()
        },
    )
}

/// PSE → RPS root (+ optional graph/emitter stubs).
pub fn pse_to_rps_export(h: &PseHeader, opts: &PseExportOptions) -> Result<Vec<u8>, PfxError> {
    pse_to_rps_export_with_curves(h, opts, &[])
}

/// Like [`pse_to_rps_export`], but `per_emitter_curves[i]` supplies up to four sampled
/// grids (sample-major `f32` slices) for emitter stub `i`. Missing slots get placeholders.
pub fn pse_to_rps_export_with_curves(
    h: &PseHeader,
    opts: &PseExportOptions,
    per_emitter_curves: &[Vec<Vec<f32>>],
) -> Result<Vec<u8>, PfxError> {
    use memory_file::MemoryFileWriter;
    fn u8_fit(v: u32, name: &str) -> Result<u8, PfxError> {
        u8::try_from(v).map_err(|_| PfxError::Msg(format!("{name}={v} does not fit u8 tag")))
    }
    let mut w = MemoryFileWriter::new();
    w.write_object_header(memory_file::obj::PARTICLE_SYSTEM, u8_fit(h.word1, "word1")?);
    // word0 must be 0x60 for the load gate; write_object_header already emitted type/ver
    // from PARTICLE_SYSTEM / word1. Re-check word0:
    if h.word0 != u32::from(memory_file::obj::PARTICLE_SYSTEM) {
        return Err(PfxError::Msg(format!(
            "PSE word0={:#x} want {:#x}",
            h.word0,
            memory_file::obj::PARTICLE_SYSTEM
        )));
    }
    w.write_particle_system_fields(
        u8_fit(h.word2, "word2")?,
        h.float_a,
        u8_fit(h.word3, "word3")?,
        h.float_b,
        u8_fit(h.word4, "word4")?,
        u8_fit(h.flag_a, "flag_a")?,
        u8_fit(h.flag_b, "flag_b")?,
    );
    if !opts.emitter_stubs.is_empty() {
        let n = u8::try_from(opts.emitter_stubs.len()).map_err(|_| {
            PfxError::Msg(format!("too many emitters ({})", opts.emitter_stubs.len()))
        })?;
        w.write_graph_section_open(n, 1.0, 0, 1, 0.1);
        for (ei, _name) in opts.emitter_stubs.iter().enumerate() {
            // Candle-shaped stub; name not yet placed on the RPS wire (REA pending).
            w.write_emitter_open(0.0, 0, 1, 0.0, 0, 10.0, 0, 1, -3.0, 3.0, 2);
            let sampled = per_emitter_curves
                .get(ei)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            // Four curves required by FUN_00417570 before path embeds.
            for slot in 0..4 {
                if let Some(grid) = sampled.get(slot) {
                    let samples = u8::try_from(grid.len()).map_err(|_| {
                        PfxError::Msg(format!("curve samples {} > 255", grid.len()))
                    })?;
                    w.write_curve(1, samples, grid)?;
                } else if slot == 1 {
                    w.write_curve(1, 2, &[0.0, 0.0])?;
                } else {
                    let c16 = vec![0.5f32; 16];
                    w.write_curve(1, 16, &c16)?;
                }
            }
            // Texture/path list omitted; zero companion body (FUN_00418fe0 order).
            w.write_companion_stub()?;
        }
    } else if opts.empty_graph {
        w.write_graph_section_open(0, 1.0, 0, 1, 0.1);
    }
    Ok(w.into_bytes())
}

/// Build up to four sampled 1-channel grids for each emitter from PSE curve objects
/// that fall between that emitter and the next.
pub fn sample_emitter_curves_from_pse(
    data: &[u8],
    sample_count: u8,
) -> Result<Vec<Vec<Vec<f32>>>, PfxError> {
    let emitters = scan_pse_emitters(data);
    let curves = scan_pse_curves(data);
    let mut out = Vec::with_capacity(emitters.len());
    for (i, em) in emitters.iter().enumerate() {
        let end = emitters.get(i + 1).map(|e| e.offset).unwrap_or(data.len());
        let mut grids = Vec::new();
        for c in curves
            .iter()
            .filter(|c| c.offset > em.offset && c.offset < end)
            .take(4)
        {
            let Some(parsed) = parse_pse_curve_floats(data, c) else {
                continue;
            };
            let Ok(kfs) = pse_curve_to_keyframes(&parsed) else {
                continue;
            };
            if kfs.is_empty() {
                continue;
            }
            let t_max = kfs.last().map(|k| k.t).unwrap_or(1.0).max(1.0);
            let Ok(grid) = sample_linear(&kfs, 1, 0.0, t_max, sample_count as usize) else {
                continue;
            };
            grids.push(grid);
        }
        out.push(grids);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synthetic_header_and_path() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&96u32.to_le_bytes());
        buf.extend_from_slice(&5u32.to_le_bytes());
        buf.extend_from_slice(&35u32.to_le_bytes());
        buf.extend_from_slice(&2.5f32.to_le_bytes());
        buf.extend_from_slice(&10u32.to_le_bytes());
        buf.extend_from_slice(&2.5f32.to_le_bytes());
        buf.extend_from_slice(&10u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(br"C:\Program Files\MAX-FX Tools\ParticleFX\Examples\x.tga");
        buf.push(0);

        let h = parse_pse(&buf).unwrap();
        assert_eq!(h.word0, 96);
        assert_eq!(h.word1, 5);
        assert_eq!(h.word2, 35);
        assert!((h.float_a - 2.5).abs() < 1e-6);
        assert_eq!(h.word3, 10);
        assert!((h.float_b - 2.5).abs() < 1e-6);
        assert_eq!(h.word4, 10);
        assert_eq!(h.flag_a, 0);
        assert_eq!(h.flag_b, 0);
        assert_eq!(h.ascii_paths.len(), 1);
        assert!(h.ascii_paths[0].contains("ParticleFX"));
    }

    #[test]
    fn pse_to_rps_root_matches_candle_root_prefix() {
        let h = PseHeader {
            word0: 96,
            word1: 5,
            word2: 12,
            float_a: 2.5,
            word3: 10,
            float_b: 2.5,
            word4: 10,
            flag_a: 0,
            flag_b: 0,
            ascii_paths: vec![],
        };
        let bytes = pse_to_rps_root(&h).unwrap();
        let expected: &[u8] = &[
            0x14, 0x60, 0x14, 0x05, 0x11, 12, 0x09, 0x00, 0x00, 0x20, 0x40, 0x11, 10, 0x09, 0x00,
            0x00, 0x20, 0x40, 0x11, 10, 0x14, 0, 0x14, 0,
        ];
        assert_eq!(bytes, expected);
        let with_graph = pse_to_rps_root_into(&h, true).unwrap();
        assert!(with_graph.starts_with(expected));
        // 0x31/1 + param(1,0,1,0.1) + emitter count 0
        let graph_empty: &[u8] = &[
            0x14, 0x31, 0x14, 1, 0x14, 0x20, 0x14, 3, 0x09, 0x00, 0x00, 0x80, 0x3f, 0x14, 0, 0x14,
            1, 0x09, 0xcd, 0xcc, 0xcc, 0x3d, 0x14, 0,
        ];
        assert_eq!(&with_graph[expected.len()..], graph_empty);
        let with_stubs = pse_to_rps_export(
            &h,
            &PseExportOptions {
                emitter_stubs: vec!["Spark".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(with_stubs.starts_with(expected));
        assert_eq!(
            &with_stubs[expected.len()..expected.len() + 4],
            &[0x14, 0x31, 0x14, 1]
        );
        assert_eq!(with_stubs[expected.len() + graph_empty.len() - 1], 1);
        assert!(with_stubs.len() > expected.len() + graph_empty.len());
    }

    #[test]
    fn scan_pse_emitters_finds_name_after_param_header() {
        let mut buf = vec![0u8; 36]; // flat header padding
        buf.extend_from_slice(&[0x30, 0, 0, 0, 4, 0, 0, 0, 0x20, 0, 0, 0, 3, 0, 0, 0]);
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.push(5);
        buf.extend_from_slice(b"Spark");
        buf.extend_from_slice(&[0x30, 0, 0, 0, 4, 0, 0, 0, 0x20, 0, 0, 0, 3, 0, 0, 0]);
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.push(4);
        buf.extend_from_slice(b"Dust");
        let found = scan_pse_emitters(&buf);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Spark");
        assert_eq!(found[1].name, "Dust");
        let mut with_curve = buf.clone();
        with_curve.extend_from_slice(&[0x71, 0, 0, 0, 3, 0, 0, 0, 1, b't', 6]);
        with_curve.extend_from_slice(b"Size m");
        with_curve.extend_from_slice(&[0u8; 8]);
        let curves = scan_pse_curves(&with_curve);
        assert_eq!(curves.len(), 1);
        assert_eq!(curves[0].name, "Size m");
        assert_eq!(scan_pse_curve_count(&with_curve), 1);
        // header 7×u32 + float pairs + PPP
        let mut raw = vec![0u8; 36];
        raw.extend_from_slice(&[0x71, 0, 0, 0, 3, 0, 0, 0, 1, b't', 4]);
        raw.extend_from_slice(b"Test");
        for w in [1u32, 0, 0, 2, 1, 1, 2] {
            raw.extend_from_slice(&w.to_le_bytes());
        }
        raw.extend_from_slice(&0.0f32.to_le_bytes());
        raw.extend_from_slice(&1.0f32.to_le_bytes());
        raw.extend_from_slice(&1.0f32.to_le_bytes());
        raw.extend_from_slice(&4.0f32.to_le_bytes());
        raw.extend_from_slice(&[0x50, 0x50, 0x50]);
        let c = &scan_pse_curves(&raw)[0];
        let parsed = parse_pse_curve_floats(&raw, c).unwrap();
        assert_eq!(parsed.header_words[3], 2);
        assert_eq!(parsed.floats.len(), 4);
        assert!((parsed.floats[1] - 1.0).abs() < 1e-5);
        assert!((parsed.floats[3] - 4.0).abs() < 1e-5);
        let kfs = pse_curve_to_keyframes(&parsed).unwrap();
        assert_eq!(kfs.len(), 2);
        let grid = sample_linear(&kfs, 1, 0.0, 1.0, 2).unwrap();
        assert!((grid[0] - 1.0).abs() < 1e-5);
        assert!((grid[1] - 4.0).abs() < 1e-5);
    }
}
