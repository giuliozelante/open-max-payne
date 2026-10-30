//! ParticleFX editor project (`.pse`) dump — draft, REA still in progress.
//! Spec: docs/specs/rps_format.md

use std::path::Path;
use thiserror::Error;

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
    pub ascii_paths: Vec<String>,
}

/// Best-effort PSE header + absolute-path harvest.
pub fn read_pse(path: &Path) -> Result<PseHeader, PfxError> {
    let data = std::fs::read(path)?;
    parse_pse(&data)
}

pub fn parse_pse(data: &[u8]) -> Result<PseHeader, PfxError> {
    if data.len() < 24 {
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
        ascii_paths,
    })
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
        buf.extend_from_slice(br"C:\Program Files\MAX-FX Tools\ParticleFX\Examples\x.tga");
        buf.push(0);

        let h = parse_pse(&buf).unwrap();
        assert_eq!(h.word0, 96);
        assert_eq!(h.word1, 5);
        assert_eq!(h.word2, 35);
        assert!((h.float_a - 2.5).abs() < 1e-6);
        assert_eq!(h.word3, 10);
        assert!((h.float_b - 2.5).abs() < 1e-6);
        assert_eq!(h.ascii_paths.len(), 1);
        assert!(h.ascii_paths[0].contains("ParticleFX"));
    }
}
