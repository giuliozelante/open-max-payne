//! Runtime Particle Set (`.rps`) — tagged stream dump from BYOD samples.
//! Spec: docs/specs/rps_format.md

use crate::PfxError;
use std::path::Path;

/// Observed MemoryFile-style type tags in retail `.rps` (stored mode in `x_data.ras`).
pub const TAG_U8_A: u8 = 0x14;
pub const TAG_U8_B: u8 = 0x11;
pub const TAG_F32: u8 = 0x09;
/// String: `0x0d` + `0x14` + len_u8 + bytes (BYOD candle RPS).
pub const TAG_STR: u8 = 0x0d;
/// Blob: `0x13` + u16_le length + bytes (often embedded JPEG `FFD8…`).
pub const TAG_BLOB: u8 = 0x13;
/// Vec3: `0x16` + 3 × LE f32 (BYOD frequency analysis across `x_data.ras` RPS files).
pub const TAG_VEC3: u8 = 0x16;

#[derive(Debug, Clone, PartialEq)]
pub enum RpsValue {
    U8 { tag: u8, value: u8 },
    F32 { value: f32 },
    Str { value: String },
    Blob { len: usize, jpeg_soi: bool },
    Vec3 { x: f32, y: f32, z: f32 },
    Unknown { tag: u8, at: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RpsHeader {
    /// First values: typically u8(96), u8(5), u8(N), f32, … matching PSE word0/word1.
    pub values: Vec<RpsValue>,
    pub bytes_consumed: usize,
}

pub fn read_rps(path: &Path) -> Result<RpsHeader, PfxError> {
    parse_rps(&std::fs::read(path)?)
}

/// Walk tagged values until an unknown tag or EOF (bounded).
pub fn parse_rps(data: &[u8]) -> Result<RpsHeader, PfxError> {
    if data.len() < 4 {
        return Err(PfxError::Msg(format!(
            "RPS too short ({} bytes)",
            data.len()
        )));
    }
    let mut values = Vec::new();
    let mut i = 0;
    while i < data.len() && values.len() < 100_000 {
        let tag = data[i];
        i += 1;
        match tag {
            TAG_U8_A | TAG_U8_B => {
                if i >= data.len() {
                    break;
                }
                values.push(RpsValue::U8 {
                    tag,
                    value: data[i],
                });
                i += 1;
            }
            TAG_F32 => {
                if i + 4 > data.len() {
                    break;
                }
                let value = f32::from_le_bytes(data[i..i + 4].try_into().unwrap());
                values.push(RpsValue::F32 { value });
                i += 4;
            }
            TAG_STR => {
                // 0x0d 14 <len> <bytes>
                if i + 2 > data.len() || data[i] != TAG_U8_A {
                    values.push(RpsValue::Unknown { tag, at: i - 1 });
                    break;
                }
                let len = data[i + 1] as usize;
                i += 2;
                if i + len > data.len() {
                    values.push(RpsValue::Unknown { tag, at: i - 3 });
                    break;
                }
                let value = String::from_utf8_lossy(&data[i..i + len]).into_owned();
                values.push(RpsValue::Str { value });
                i += len;
            }
            TAG_BLOB => {
                if i + 2 > data.len() {
                    values.push(RpsValue::Unknown { tag, at: i - 1 });
                    break;
                }
                let len = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
                i += 2;
                if i + len > data.len() {
                    values.push(RpsValue::Unknown { tag, at: i - 3 });
                    break;
                }
                let jpeg_soi = len >= 2 && data[i] == 0xff && data[i + 1] == 0xd8;
                values.push(RpsValue::Blob { len, jpeg_soi });
                i += len;
            }
            TAG_VEC3 => {
                if i + 12 > data.len() {
                    values.push(RpsValue::Unknown { tag, at: i - 1 });
                    break;
                }
                let x = f32::from_le_bytes(data[i..i + 4].try_into().unwrap());
                let y = f32::from_le_bytes(data[i + 4..i + 8].try_into().unwrap());
                let z = f32::from_le_bytes(data[i + 8..i + 12].try_into().unwrap());
                values.push(RpsValue::Vec3 { x, y, z });
                i += 12;
            }
            _ => {
                values.push(RpsValue::Unknown { tag, at: i - 1 });
                break;
            }
        }
    }
    Ok(RpsHeader {
        values,
        bytes_consumed: i,
    })
}

/// Emit the common PSE→RPS tagged preamble for the flat header fields we understand.
/// Full particle graphs are **not** included — this is REA-backed header only.
pub fn pse_header_to_rps_preamble(
    word0: u32,
    word1: u32,
    word2: u32,
    float_a: f32,
    word3: u32,
    float_b: f32,
) -> Result<Vec<u8>, PfxError> {
    fn u8_fit(v: u32, name: &str) -> Result<u8, PfxError> {
        u8::try_from(v).map_err(|_| PfxError::Msg(format!("{name}={v} does not fit u8 tag")))
    }
    let mut out = vec![
        TAG_U8_A,
        u8_fit(word0, "word0")?,
        TAG_U8_A,
        u8_fit(word1, "word1")?,
        TAG_U8_B,
        u8_fit(word2, "word2")?,
        TAG_F32,
    ];
    out.extend_from_slice(&float_a.to_le_bytes());
    out.push(TAG_U8_B);
    out.push(u8_fit(word3, "word3")?);
    out.push(TAG_F32);
    out.extend_from_slice(&float_b.to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_synthetic_rps_matching_byod_prefix() {
        // Mirrors fire_candle_flame_large_1.rps start (no retail bytes committed).
        let buf = vec![
            0x14, 96, 0x14, 5, 0x11, 12, 0x09, 0x00, 0x00, 0x20, 0x40, 0x11, 10, 0x09, 0x00, 0x00,
            0x20, 0x40, 0x11, 10, 0x14, 0, 0x14, 0, 0x14, 49, 0xff,
        ];
        // last 0xff stops walker
        let h = parse_rps(&buf).unwrap();
        assert!(matches!(
            h.values[0],
            RpsValue::U8 {
                tag: 0x14,
                value: 96
            }
        ));
        assert!(matches!(
            h.values[1],
            RpsValue::U8 {
                tag: 0x14,
                value: 5
            }
        ));
        assert!(matches!(
            h.values[2],
            RpsValue::U8 {
                tag: 0x11,
                value: 12
            }
        ));
        match &h.values[3] {
            RpsValue::F32 { value } => assert!((value - 2.5).abs() < 1e-6),
            _ => panic!("expected f32"),
        }
        assert!(matches!(
            h.values.last().unwrap(),
            RpsValue::Unknown { tag: 0xff, .. }
        ));
    }

    #[test]
    fn walks_string_tag() {
        let mut buf = vec![0x14, 1, TAG_STR, TAG_U8_A, 5];
        buf.extend_from_slice(b"hello");
        buf.push(0xff);
        let h = parse_rps(&buf).unwrap();
        assert!(matches!(&h.values[1], RpsValue::Str { value } if value == "hello"));
    }

    #[test]
    fn walks_vec3_tag() {
        let mut buf = vec![TAG_VEC3];
        for f in [0.0f32, 1.0, 2.0] {
            buf.extend_from_slice(&f.to_le_bytes());
        }
        buf.push(0xff);
        let h = parse_rps(&buf).unwrap();
        assert!(matches!(
            &h.values[0],
            RpsValue::Vec3 { x, y, z } if *x == 0.0 && *y == 1.0 && *z == 2.0
        ));
    }

    #[test]
    fn walks_jpeg_blob_tag() {
        let jpeg = b"\xff\xd8\xff\xd9";
        let mut buf = vec![TAG_BLOB];
        buf.extend_from_slice(&(jpeg.len() as u16).to_le_bytes());
        buf.extend_from_slice(jpeg);
        buf.push(0xff);
        let h = parse_rps(&buf).unwrap();
        assert!(matches!(
            &h.values[0],
            RpsValue::Blob {
                len: 4,
                jpeg_soi: true
            }
        ));
    }

    #[test]
    fn pse_preamble_roundtrips_through_walker() {
        let bytes = pse_header_to_rps_preamble(96, 5, 35, 2.5, 10, 2.5).unwrap();
        let h = parse_rps(&bytes).unwrap();
        assert_eq!(h.values.len(), 6);
        assert!(matches!(h.values[0], RpsValue::U8 { value: 96, .. }));
        assert!(matches!(
            h.values[2],
            RpsValue::U8 {
                tag: 0x11,
                value: 35
            }
        ));
    }
}
