//! Okumura-style LZSS as used by rl.dll R_File::decompress (compression mode 1).
//!
//! `rl.dll` `0x10012ce0` / `0x10012fa0` frames mode-1 payloads with a 12-byte
//! header: `b"RA->"` + `uncompressed_size: u32` + `compressed_body_size: u32`.

const N: usize = 4096;
const F: usize = 18;
const THRESHOLD: usize = 2;
pub const LZSS_MAGIC: &[u8; 4] = b"RA->";
pub const LZSS_HEADER_LEN: usize = 12;

pub fn decompress(src: &[u8], uncompressed_size: Option<usize>) -> Vec<u8> {
    let (body, hdr_uncomp) = if src.len() >= LZSS_HEADER_LEN && &src[..4] == LZSS_MAGIC {
        let u = u32::from_le_bytes(src[4..8].try_into().unwrap()) as usize;
        let c = u32::from_le_bytes(src[8..12].try_into().unwrap()) as usize;
        let end = LZSS_HEADER_LEN.saturating_add(c).min(src.len());
        (&src[LZSS_HEADER_LEN..end], Some(u))
    } else {
        (src, None)
    };
    let target_len = uncompressed_size.or(hdr_uncomp);
    let mut text_buf = vec![0x20u8; N];
    let mut r = 0xFEE;
    let mut out = Vec::with_capacity(target_len.unwrap_or(body.len() * 2));
    let mut pb = 0usize;
    let mut flags: u32 = 0;

    loop {
        if target_len.is_some_and(|sz| out.len() >= sz) {
            break;
        }
        flags >>= 1;
        if flags & 0x100 == 0 {
            if pb >= body.len() {
                break;
            }
            flags = u32::from(body[pb]) | 0xFF00;
            pb += 1;
        }
        if flags & 1 != 0 {
            if pb >= body.len() {
                break;
            }
            let b = body[pb];
            pb += 1;
            out.push(b);
            text_buf[r] = b;
            r = (r + 1) & 0xFFF;
        } else {
            if pb + 1 >= body.len() {
                break;
            }
            let b0 = body[pb];
            let b1 = body[pb + 1];
            pb += 2;
            let offset = usize::from(b0) | ((usize::from(b1) & 0xF0) << 4);
            let limit = (usize::from(b1) & 0x0F) + 2;
            for j in 0..=limit {
                let b = text_buf[(offset + j) & 0xFFF];
                out.push(b);
                text_buf[r] = b;
                r = (r + 1) & 0xFFF;
            }
        }
    }

    if let Some(sz) = target_len {
        out.truncate(sz);
    }
    out
}

/// Greedy LZSS encoder producing streams accepted by [`decompress`] and retail `rl.dll`.
pub fn compress(src: &[u8]) -> Vec<u8> {
    let mut text_buf = vec![0x20u8; N];
    let mut r = 0xFEE;
    let mut out = Vec::with_capacity(LZSS_HEADER_LEN + src.len());
    out.extend_from_slice(LZSS_MAGIC);
    out.extend_from_slice(&(src.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    let mut pos = 0usize;
    let n = src.len();

    while pos < n {
        let flag_pos = out.len();
        out.push(0);
        let mut flags: u8 = 0;
        let mut block = Vec::new();
        for bit in 0..8 {
            if pos >= n {
                break;
            }
            let max_len = F.min(n - pos);
            let mut best_len = 0usize;
            let mut best_off = 0usize;
            if max_len > THRESHOLD {
                for off in 0..N {
                    let mut length = 0usize;
                    while length < max_len && text_buf[(off + length) & 0xFFF] == src[pos + length]
                    {
                        length += 1;
                    }
                    if length > best_len {
                        best_len = length;
                        best_off = off;
                        if best_len == max_len {
                            break;
                        }
                    }
                }
            }
            if best_len > THRESHOLD {
                let match_len = best_len.min(F);
                let nibble = (match_len - 3) as u8;
                block.push((best_off & 0xFF) as u8);
                block.push((((best_off >> 4) as u8) & 0xF0) | (nibble & 0x0F));
                for j in 0..match_len {
                    let b = src[pos + j];
                    text_buf[r] = b;
                    r = (r + 1) & 0xFFF;
                }
                pos += match_len;
            } else {
                flags |= 1 << bit;
                let b = src[pos];
                block.push(b);
                text_buf[r] = b;
                r = (r + 1) & 0xFFF;
                pos += 1;
            }
        }
        out[flag_pos] = flags;
        out.extend_from_slice(&block);
    }
    let body_len = (out.len() - LZSS_HEADER_LEN) as u32;
    out[8..12].copy_from_slice(&body_len.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lzss_roundtrip_ascii() {
        let src = b"Hello Hello Hello Max Payne Hello";
        let comp = compress(src);
        let out = decompress(&comp, Some(src.len()));
        assert_eq!(out, src);
    }

    #[test]
    fn lzss_roundtrip_binary() {
        let src: Vec<u8> = (0..500).map(|i| (i % 251) as u8).collect();
        let comp = compress(&src);
        assert_eq!(&comp[..4], b"RA->");
        let uncomp_hdr = u32::from_le_bytes(comp[4..8].try_into().unwrap()) as usize;
        let body_hdr = u32::from_le_bytes(comp[8..12].try_into().unwrap()) as usize;
        assert_eq!(uncomp_hdr, src.len());
        assert_eq!(body_hdr + 12, comp.len());
        let out = decompress(&comp, Some(src.len()));
        assert_eq!(out, src);
    }
}
