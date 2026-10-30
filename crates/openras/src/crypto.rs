//! RAS metadata stream cipher — clean-room from docs/specs/ras_format.md
//! (R_File::decryptWithSeed / encrypt schedule).

#[inline]
fn i8(x: i32) -> i32 {
    let x = x as u8 as i32;
    if x >= 128 {
        x - 256
    } else {
        x
    }
}

#[inline]
fn i32_trunc(x: i64) -> i32 {
    x as i32
}

/// MSVC toward-zero signed 32-bit division.
#[inline]
fn c_div(a: i32, b: i32) -> i32 {
    if a == 0 {
        return 0;
    }
    if (a < 0) == (b < 0) {
        a / b
    } else {
        -((-a) / b)
    }
}

#[inline]
fn advance_seed(seed: i32) -> i32 {
    i32_trunc(i64::from(seed) * 0xAB + i64::from(c_div(seed, 0xB1)) * -0x763D)
}

pub fn decrypt_with_seed(data: &[u8], mut seed: i32) -> Vec<u8> {
    let mut buf = data.to_vec();
    if seed == 0 {
        seed = 1;
    }
    for i in 0..buf.len() {
        let r = (i % 5) & 7;
        let mut b = buf[i];
        if r != 0 {
            b = b.rotate_left(r as u32);
        }
        buf[i] = b;
        seed = advance_seed(seed);
        let mixed = ((i8(i as i32) + 3) * 6) as u8 ^ buf[i];
        buf[i] = mixed.wrapping_add(i8(seed) as u8);
    }
    buf
}

pub fn encrypt_with_seed(data: &[u8], mut seed: i32) -> Vec<u8> {
    let mut buf = data.to_vec();
    if seed == 0 {
        seed = 1;
    }
    for i in 0..buf.len() {
        seed = advance_seed(seed);
        let mut x = buf[i].wrapping_sub(i8(seed) as u8);
        x ^= ((i8(i as i32) + 3) * 6) as u8;
        let r = (i % 5) & 7;
        if r != 0 {
            x = x.rotate_right(r as u32);
        }
        buf[i] = x;
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let seed = 0x2C145828_i32;
        let mut plain: Vec<u8> = (0u8..=255).collect();
        plain.extend_from_slice(b"basic_room.ldb\x00");
        plain.extend(std::iter::repeat_n(0u8, 40));
        let enc = encrypt_with_seed(&plain, seed);
        assert_ne!(enc, plain);
        assert_eq!(decrypt_with_seed(&enc, seed), plain);
    }

    #[test]
    fn seed_zero_becomes_one() {
        let plain = b"abcdefghijklmnop";
        assert_eq!(
            decrypt_with_seed(&encrypt_with_seed(plain, 0), 0),
            plain.to_vec()
        );
    }
}
