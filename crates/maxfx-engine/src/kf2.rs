//! Native ActorFX / Keyframe 2.0 (`.kf2` and `.skd`) chunk and bone/material walker.
//!
//! `R_MemoryFile::createChunk` writes `0x0c` + `id: u32` + `version: u32` + `size: u32`
//! (`size` includes the 13-byte header). `.kf2` files contain sequential top-level
//! chunks (`0x0001000f` materials, `0x00010000` geometry/nodes, `0x00010011` keyframe
//! animations) and `.skd` files open with chunk `0x00010014` (skin/bone weights).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kf2Chunk {
    pub offset: usize,
    pub id: u32,
    pub version: u32,
    pub size: u32,
    pub strings: Vec<String>,
    pub vec3_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kf2Summary {
    pub file_size: usize,
    pub chunks: Vec<Kf2Chunk>,
    pub all_strings: Vec<String>,
    pub total_vec3: usize,
}

/// Walk top-level `0x0c` chunks (and nested strings/vec3 counts) in a `.kf2` or `.skd` buffer.
pub fn parse_kf2_or_skd(data: &[u8]) -> Option<Kf2Summary> {
    if data.len() < 13 || data[0] != 0x0c {
        return None;
    }
    let mut chunks = Vec::new();
    let mut all_strings = Vec::new();
    let mut total_vec3 = 0usize;
    let mut off = 0usize;

    while off + 13 <= data.len() && data[off] == 0x0c {
        let id = u32::from_le_bytes(data[off + 1..off + 5].try_into().ok()?);
        let version = u32::from_le_bytes(data[off + 5..off + 9].try_into().ok()?);
        let size = u32::from_le_bytes(data[off + 9..off + 13].try_into().ok()?);
        if (size as usize) < 13 || off + size as usize > data.len() {
            break;
        }
        let payload = &data[off + 13..off + size as usize];
        let mut strings = Vec::new();
        let mut vec3_count = 0usize;
        let mut i = 0usize;
        while i < payload.len() {
            if payload[i] == 0x0d && i + 2 < payload.len() && payload[i + 1] == 0x14 {
                let len = payload[i + 2] as usize;
                let start = i + 3;
                if (1..=96).contains(&len) && start + len <= payload.len() {
                    let bytes = &payload[start..start + len];
                    if bytes.iter().all(|&b| (0x20..=0x7e).contains(&b)) {
                        if let Ok(s) = String::from_utf8(bytes.to_vec()) {
                            strings.push(s.clone());
                            if all_strings.len() < 128 && !all_strings.contains(&s) {
                                all_strings.push(s);
                            }
                        }
                        i = start + len;
                        continue;
                    }
                }
            } else if payload[i] == 0x16 && i + 13 <= payload.len() {
                let x = f32::from_le_bytes(payload[i + 1..i + 5].try_into().unwrap_or([0; 4]));
                let y = f32::from_le_bytes(payload[i + 5..i + 9].try_into().unwrap_or([0; 4]));
                let z = f32::from_le_bytes(payload[i + 9..i + 13].try_into().unwrap_or([0; 4]));
                if x.is_finite()
                    && y.is_finite()
                    && z.is_finite()
                    && x.abs() < 1e5
                    && y.abs() < 1e5
                    && z.abs() < 1e5
                {
                    vec3_count += 1;
                    total_vec3 += 1;
                    i += 13;
                    continue;
                }
            }
            i += 1;
        }
        chunks.push(Kf2Chunk {
            offset: off,
            id,
            version,
            size,
            strings,
            vec3_count,
        });
        off += size as usize;
    }

    if chunks.is_empty() {
        None
    } else {
        Some(Kf2Summary {
            file_size: data.len(),
            chunks,
            all_strings,
            total_vec3,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synthetic_kf2_chunk() {
        let mut buf = Vec::new();
        buf.push(0x0c);
        buf.extend_from_slice(&0x0001_000fu32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        let payload = [0x0d, 0x14, 6, b'P', b'e', b'l', b'v', b'i', b's'];
        let size = (13 + payload.len()) as u32;
        buf.extend_from_slice(&size.to_le_bytes());
        buf.extend_from_slice(&payload);
        let sum = parse_kf2_or_skd(&buf).unwrap();
        assert_eq!(sum.chunks.len(), 1);
        assert_eq!(sum.all_strings, vec!["Pelvis"]);
    }
}
