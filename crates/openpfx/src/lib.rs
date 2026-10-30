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

/// PSE flat fields → RPS root object through pre-graph ints (`FUN_0041ab00` before graphs).
pub fn pse_to_rps_root(h: &PseHeader) -> Result<Vec<u8>, PfxError> {
    pse_to_rps_root_into(h, false)
}

/// Like [`pse_to_rps_root`], optionally appending an empty graph header (`0x31`/`0`).
pub fn pse_to_rps_root_into(h: &PseHeader, empty_graph: bool) -> Result<Vec<u8>, PfxError> {
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
    if empty_graph {
        w.write_graph_header(0);
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
        assert_eq!(&with_graph[expected.len()..], &[0x14, 0x31, 0x14, 0]);
    }
}
