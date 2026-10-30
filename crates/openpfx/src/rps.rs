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
/// Vec3: `0x16` + 3 × LE f32 (BYOD frequency analysis across `x_data.ras` RPS files).
pub const TAG_VEC3: u8 = 0x16;
/// Chunk header: `0x0c` + id_u32 + version_u32 + size_u32 (`R_MemoryFile::createChunk`).
/// `size` is total bytes including the tag + 12-byte header (`payload = size - 0x0d`).
pub const TAG_CHUNK: u8 = 0x0c;

/// Signed int size-compression tags (RLMFC `operator<<(mf, int)`).
pub const TAG_I32_1: u8 = 0x14; // == TAG_U8_A
pub const TAG_I32_2: u8 = 0x13;
pub const TAG_I32_3: u8 = 0x12;
pub const TAG_I32_4: u8 = 0x02;

/// Unsigned int size-compression tags (RLMFC `operator<<(mf, uint)`).
pub const TAG_U32_1: u8 = 0x11; // == TAG_U8_B
pub const TAG_U32_2: u8 = 0x10;
pub const TAG_U32_3: u8 = 0x0f;
pub const TAG_U32_4: u8 = 0x03;

/// Tags that begin a MemoryFile typed value (not raw embedded payload).
const KNOWN_TAGS: &[u8] = &[
    TAG_I32_4, TAG_U32_4, 0x07, TAG_F32, TAG_CHUNK, TAG_STR, TAG_U32_3, TAG_U32_2, TAG_U8_B,
    TAG_I32_3, TAG_I32_2, TAG_U8_A, 0x15, TAG_VEC3, 0x17,
];

#[derive(Debug, Clone, PartialEq)]
pub enum RpsValue {
    U8 {
        tag: u8,
        value: u8,
    },
    /// Multi-byte compressed signed int (`0x13`/`0x12`/`0x02`).
    Int {
        value: i32,
    },
    /// Multi-byte compressed unsigned int (`0x10`/`0x0f`/`0x03`).
    UInt {
        value: u32,
    },
    F32 {
        value: f32,
    },
    Str {
        value: String,
    },
    /// Length-prefixed raw embed (`FUN_00418190`): compressed int + untaged bytes.
    Blob {
        data: Vec<u8>,
    },
    Vec3 {
        x: f32,
        y: f32,
        z: f32,
    },
    /// `R_MemoryFile` chunk header (payload follows; walker does not auto-skip).
    Chunk {
        id: u32,
        version: u32,
        size: u32,
    },
    Unknown {
        tag: u8,
        at: usize,
    },
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

fn push_compressed_int(out: &mut Vec<u8>, value: i32) {
    let bits = value as u32;
    let bytes = value.to_le_bytes();
    if bits & 0xff80_0000 != 0 {
        out.push(TAG_I32_4);
        out.extend_from_slice(&bytes);
    } else if bits & 0x007f_8000 != 0 {
        out.push(TAG_I32_3);
        out.extend_from_slice(&bytes[..3]);
    } else if bits & 0x0000_7f80 != 0 {
        out.push(TAG_I32_2);
        out.extend_from_slice(&bytes[..2]);
    } else {
        out.push(TAG_U8_A);
        out.push(bytes[0]);
    }
}

fn try_take_raw_blob(data: &[u8], i: &mut usize, len: usize, values: &mut Vec<RpsValue>) -> bool {
    if len == 0 || *i + len > data.len() {
        return false;
    }
    if KNOWN_TAGS.contains(&data[*i]) {
        return false;
    }
    values.push(RpsValue::Blob {
        data: data[*i..*i + len].to_vec(),
    });
    *i += len;
    true
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
                let value = data[i];
                i += 1;
                if !try_take_raw_blob(data, &mut i, value as usize, &mut values) {
                    values.push(RpsValue::U8 { tag, value });
                }
            }
            TAG_I32_2 => {
                if i + 2 > data.len() {
                    values.push(RpsValue::Unknown { tag, at: i - 1 });
                    break;
                }
                let mut b = [0u8; 4];
                b[..2].copy_from_slice(&data[i..i + 2]);
                i += 2;
                let value = i32::from_le_bytes(b);
                if value < 0 || !try_take_raw_blob(data, &mut i, value as usize, &mut values) {
                    values.push(RpsValue::Int { value });
                }
            }
            TAG_I32_3 => {
                if i + 3 > data.len() {
                    values.push(RpsValue::Unknown { tag, at: i - 1 });
                    break;
                }
                let mut b = [0u8; 4];
                b[..3].copy_from_slice(&data[i..i + 3]);
                i += 3;
                let value = i32::from_le_bytes(b);
                if value < 0 || !try_take_raw_blob(data, &mut i, value as usize, &mut values) {
                    values.push(RpsValue::Int { value });
                }
            }
            TAG_I32_4 => {
                if i + 4 > data.len() {
                    values.push(RpsValue::Unknown { tag, at: i - 1 });
                    break;
                }
                let value = i32::from_le_bytes(data[i..i + 4].try_into().unwrap());
                i += 4;
                if value < 0 || !try_take_raw_blob(data, &mut i, value as usize, &mut values) {
                    values.push(RpsValue::Int { value });
                }
            }
            TAG_U32_2 => {
                if i + 2 > data.len() {
                    values.push(RpsValue::Unknown { tag, at: i - 1 });
                    break;
                }
                let mut b = [0u8; 4];
                b[..2].copy_from_slice(&data[i..i + 2]);
                i += 2;
                let value = u32::from_le_bytes(b);
                values.push(RpsValue::UInt { value });
            }
            TAG_U32_3 => {
                if i + 3 > data.len() {
                    values.push(RpsValue::Unknown { tag, at: i - 1 });
                    break;
                }
                let mut b = [0u8; 4];
                b[..3].copy_from_slice(&data[i..i + 3]);
                i += 3;
                let value = u32::from_le_bytes(b);
                values.push(RpsValue::UInt { value });
            }
            TAG_U32_4 => {
                if i + 4 > data.len() {
                    values.push(RpsValue::Unknown { tag, at: i - 1 });
                    break;
                }
                let value = u32::from_le_bytes(data[i..i + 4].try_into().unwrap());
                i += 4;
                values.push(RpsValue::UInt { value });
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
            TAG_CHUNK => {
                if i + 12 > data.len() {
                    values.push(RpsValue::Unknown { tag, at: i - 1 });
                    break;
                }
                let id = u32::from_le_bytes(data[i..i + 4].try_into().unwrap());
                let version = u32::from_le_bytes(data[i + 4..i + 8].try_into().unwrap());
                let size = u32::from_le_bytes(data[i + 8..i + 12].try_into().unwrap());
                values.push(RpsValue::Chunk { id, version, size });
                i += 12;
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

/// Parse then re-encode; errors if the walker hit an unknown tag before EOF.
pub fn roundtrip_rps(data: &[u8]) -> Result<Vec<u8>, PfxError> {
    let h = parse_rps(data)?;
    if let Some(RpsValue::Unknown { tag, at }) = h.values.last() {
        return Err(PfxError::Msg(format!(
            "unknown tag 0x{tag:02x} at {at}; cannot roundtrip"
        )));
    }
    if h.bytes_consumed != data.len() {
        return Err(PfxError::Msg(format!(
            "walker stopped at {} of {} bytes",
            h.bytes_consumed,
            data.len()
        )));
    }
    encode_rps(&h.values)
}

/// Encode known tagged values back to bytes (stops at `Unknown`).
pub fn encode_rps(values: &[RpsValue]) -> Result<Vec<u8>, PfxError> {
    let mut out = Vec::new();
    for v in values {
        match v {
            RpsValue::U8 { tag, value } => {
                out.push(*tag);
                out.push(*value);
            }
            RpsValue::Int { value } => push_compressed_int(&mut out, *value),
            RpsValue::UInt { value } => {
                let bytes = value.to_le_bytes();
                if *value & 0xff00_0000 != 0 {
                    out.push(TAG_U32_4);
                    out.extend_from_slice(&bytes);
                } else if *value & 0x00ff_0000 != 0 {
                    out.push(TAG_U32_3);
                    out.extend_from_slice(&bytes[..3]);
                } else if *value & 0x0000_ff00 != 0 {
                    out.push(TAG_U32_2);
                    out.extend_from_slice(&bytes[..2]);
                } else {
                    out.push(TAG_U8_B);
                    out.push(bytes[0]);
                }
            }
            RpsValue::F32 { value } => {
                out.push(TAG_F32);
                out.extend_from_slice(&value.to_le_bytes());
            }
            RpsValue::Str { value } => {
                let bytes = value.as_bytes();
                if bytes.len() > 255 {
                    return Err(PfxError::Msg("string longer than 255".into()));
                }
                out.push(TAG_STR);
                out.push(TAG_U8_A);
                out.push(bytes.len() as u8);
                out.extend_from_slice(bytes);
            }
            RpsValue::Blob { data } => {
                if data.len() > i32::MAX as usize {
                    return Err(PfxError::Msg("blob too large".into()));
                }
                push_compressed_int(&mut out, data.len() as i32);
                out.extend_from_slice(data);
            }
            RpsValue::Vec3 { x, y, z } => {
                out.push(TAG_VEC3);
                out.extend_from_slice(&x.to_le_bytes());
                out.extend_from_slice(&y.to_le_bytes());
                out.extend_from_slice(&z.to_le_bytes());
            }
            RpsValue::Chunk { id, version, size } => {
                out.push(TAG_CHUNK);
                out.extend_from_slice(&id.to_le_bytes());
                out.extend_from_slice(&version.to_le_bytes());
                out.extend_from_slice(&size.to_le_bytes());
            }
            RpsValue::Unknown { tag, at } => {
                return Err(PfxError::Msg(format!(
                    "cannot encode unknown tag 0x{tag:02x} at {at}"
                )));
            }
        }
    }
    Ok(out)
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
    fn walks_chunk_header_tag() {
        // size = 0x0d (tag+header only) + 4 payload bytes → total 0x11
        let mut buf = vec![TAG_CHUNK];
        buf.extend_from_slice(&0x1234u32.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&0x11u32.to_le_bytes());
        buf.extend_from_slice(&[0xaa, 0xbb, 0xcc, 0xdd]);
        let h = parse_rps(&buf).unwrap();
        assert!(matches!(
            h.values[0],
            RpsValue::Chunk {
                id: 0x1234,
                version: 1,
                size: 0x11
            }
        ));
        // walker stops at raw payload (not a tag)
        assert!(matches!(h.values[1], RpsValue::Unknown { tag: 0xaa, .. }));
    }

    #[test]
    fn walks_jpeg_blob_as_length_plus_raw() {
        let jpeg = b"\xff\xd8\xff\xd9";
        let mut buf = Vec::new();
        push_compressed_int(&mut buf, jpeg.len() as i32);
        buf.extend_from_slice(jpeg);
        buf.push(0xff);
        let h = parse_rps(&buf).unwrap();
        match &h.values[0] {
            RpsValue::Blob { data } => {
                assert_eq!(data, jpeg);
                assert_eq!(&data[..2], b"\xff\xd8");
            }
            _ => panic!("expected blob"),
        }
    }

    #[test]
    fn walks_multi_byte_int_and_uint() {
        let mut buf = Vec::new();
        push_compressed_int(&mut buf, 0x100);
        // uint 959 → 0x10 + le16
        buf.push(TAG_U32_2);
        buf.extend_from_slice(&959u16.to_le_bytes());
        let h = parse_rps(&buf).unwrap();
        assert!(matches!(h.values[0], RpsValue::Int { value: 0x100 }));
        assert!(matches!(h.values[1], RpsValue::UInt { value: 959 }));
        assert_eq!(h.bytes_consumed, buf.len());
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

    #[test]
    fn roundtrip_rps_rejects_trailing_unknown() {
        let buf = vec![0x14, 1, 0xff];
        assert!(roundtrip_rps(&buf).is_err());
    }

    #[test]
    fn roundtrip_rps_full_synthetic() {
        let values = vec![
            RpsValue::U8 {
                tag: TAG_U8_A,
                value: 96,
            },
            RpsValue::U8 {
                tag: TAG_U8_A,
                value: 5,
            },
            RpsValue::Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        let bytes = encode_rps(&values).unwrap();
        assert_eq!(roundtrip_rps(&bytes).unwrap(), bytes);
    }

    #[test]
    fn encode_roundtrips_mixed_tags() {
        let values = vec![
            RpsValue::U8 {
                tag: TAG_U8_A,
                value: 96,
            },
            RpsValue::F32 { value: 2.5 },
            RpsValue::Str {
                value: "a\\b.JPG".into(),
            },
            RpsValue::Blob {
                data: b"\xff\xd8\xff\xd9".to_vec(),
            },
            RpsValue::Vec3 {
                x: 0.0,
                y: 1.0,
                z: 0.0,
            },
        ];
        let bytes = encode_rps(&values).unwrap();
        let back = parse_rps(&bytes).unwrap();
        assert_eq!(back.values, values);
        assert_eq!(back.bytes_consumed, bytes.len());
        assert_eq!(roundtrip_rps(&bytes).unwrap(), bytes);
    }
}
