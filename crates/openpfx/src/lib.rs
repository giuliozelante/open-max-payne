//! ParticleFX editor project (`.pse`) + runtime (`.rps`) tooling — draft.
//! Spec: docs/specs/rps_format.md

pub mod memory_file;
pub mod rps;

use std::path::Path;
use thiserror::Error;

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
    const CURVE: &[u8] = &[0x71, 0, 0, 0, 3, 0, 0, 0];
    data.windows(CURVE.len()).filter(|w| *w == CURVE).count()
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
        for _name in &opts.emitter_stubs {
            // Candle-shaped stub; name not yet placed on the RPS wire (REA pending).
            w.write_emitter_open(0.0, 0, 1, 0.0, 0, 10.0, 0, 1, -3.0, 3.0, 2);
            // Four curves required by FUN_00417570 before path embeds.
            let c16 = vec![0.5f32; 16];
            w.write_curve(1, 16, &c16)?;
            w.write_curve(1, 2, &[0.0, 0.0])?;
            w.write_curve(1, 16, &c16)?;
            w.write_curve(1, 16, &c16)?;
            // Texture/path list omitted → companion header only (incomplete vs retail).
            w.write_companion_open();
            w.write_companion_inner_open();
        }
    } else if opts.empty_graph {
        w.write_graph_section_open(0, 1.0, 0, 1, 0.1);
    }
    Ok(w.into_bytes())
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
        with_curve.extend_from_slice(&[0x71, 0, 0, 0, 3, 0, 0, 0]);
        assert_eq!(scan_pse_curve_count(&with_curve), 1);
    }
}
