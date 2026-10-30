//! Remedy `R_MemoryFile` tag writers as used by ParticleFX `-e` export
//! (`RLMFC.DLL::operator<<` overloads observed via REA of `FUN_0041ab00` et al.).
//!
//! Spec: docs/specs/rps_format.md

use crate::rps::{
    TAG_BLOB, TAG_F32, TAG_STR, TAG_U8_A, TAG_U8_B, TAG_VEC3,
};
use crate::PfxError;

/// Object-type markers written as `operator<<(mf, &typeId)` then a version/count int.
/// These appear as `0x14 <marker>` in the RPS byte stream.
pub mod obj {
    /// Particle system root (`FUN_0041ab00`): type `0x60`, version/count `5`.
    pub const PARTICLE_SYSTEM: u8 = 0x60;
    pub const PARTICLE_SYSTEM_VER: u8 = 5;
    /// Graph section (`FUN_00419be0`): type `0x31`.
    pub const GRAPH: u8 = 0x31;
    /// Shared param block (`FUN_00414d20`): type `0x20`, ver `3`.
    pub const PARAM_BLOCK: u8 = 0x20;
    pub const PARAM_BLOCK_VER: u8 = 3;
    /// Emitter / particle node (`FUN_00417570`): type `0x30`, ver `4`.
    pub const EMITTER: u8 = 0x30;
    pub const EMITTER_VER: u8 = 4;
    /// Companion block (`FUN_00418fe0`): type `0x40`, ver `2`.
    pub const COMPANION: u8 = 0x40;
    pub const COMPANION_VER: u8 = 2;
    /// Sampled curve (`FUN_00432fd0`): type `0x71`, ver `3`.
    pub const CURVE: u8 = 0x71;
    pub const CURVE_VER: u8 = 3;
}

/// Growing buffer that emits MemoryFile tags matching retail `.rps`.
#[derive(Debug, Default, Clone)]
pub struct MemoryFileWriter {
    buf: Vec<u8>,
}

impl MemoryFileWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// `operator<<(mf, int*)` — tag `0x14` + u8 payload.
    pub fn write_int_u8(&mut self, value: u8) {
        self.buf.push(TAG_U8_A);
        self.buf.push(value);
    }

    /// `operator<<(mf, uint*)` — tag `0x11` + u8 payload.
    pub fn write_uint_u8(&mut self, value: u8) {
        self.buf.push(TAG_U8_B);
        self.buf.push(value);
    }

    /// `operator<<(mf, float*)` — tag `0x09` + LE f32.
    pub fn write_f32(&mut self, value: f32) {
        self.buf.push(TAG_F32);
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// `operator<<(mf, M_Vector3Template<float>*)` — tag `0x16` + 3 × LE f32.
    pub fn write_vec3(&mut self, x: f32, y: f32, z: f32) {
        self.buf.push(TAG_VEC3);
        self.buf.extend_from_slice(&x.to_le_bytes());
        self.buf.extend_from_slice(&y.to_le_bytes());
        self.buf.extend_from_slice(&z.to_le_bytes());
    }

    /// Relative path string: `0x0d 0x14 <len> <bytes>` (BYOD candle).
    pub fn write_str(&mut self, value: &str) -> Result<(), PfxError> {
        let bytes = value.as_bytes();
        if bytes.len() > 255 {
            return Err(PfxError::Msg("string longer than 255".into()));
        }
        self.buf.push(TAG_STR);
        self.buf.push(TAG_U8_A);
        self.buf.push(bytes.len() as u8);
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    /// Embedded blob: `0x13` + u16_le length + bytes.
    pub fn write_blob(&mut self, data: &[u8]) -> Result<(), PfxError> {
        if data.len() > u16::MAX as usize {
            return Err(PfxError::Msg("blob longer than u16".into()));
        }
        self.buf.push(TAG_BLOB);
        self.buf
            .extend_from_slice(&(data.len() as u16).to_le_bytes());
        self.buf.extend_from_slice(data);
        Ok(())
    }

    /// Write object header pair: type marker then version/count (both via int tags).
    pub fn write_object_header(&mut self, type_id: u8, version: u8) {
        self.write_int_u8(type_id);
        self.write_int_u8(version);
    }

    /// PSE flat-header → RPS tagged preamble (same as `pse_header_to_rps_preamble`).
    pub fn write_pse_preamble(
        &mut self,
        word0: u32,
        word1: u32,
        word2: u32,
        float_a: f32,
        word3: u32,
        float_b: f32,
    ) -> Result<(), PfxError> {
        fn u8_fit(v: u32, name: &str) -> Result<u8, PfxError> {
            u8::try_from(v).map_err(|_| PfxError::Msg(format!("{name}={v} does not fit u8 tag")))
        }
        // FUN_0041ab00 starts with object header 0x60 / 5, then streams fields as
        // uint/float/int — matching the retail candle prefix.
        self.write_int_u8(u8_fit(word0, "word0")?);
        self.write_int_u8(u8_fit(word1, "word1")?);
        self.write_uint_u8(u8_fit(word2, "word2")?);
        self.write_f32(float_a);
        self.write_uint_u8(u8_fit(word3, "word3")?);
        self.write_f32(float_b);
        Ok(())
    }

    /// `FUN_0041ab00` field stream after the `0x60`/`5` header (no graphs).
    /// Layout matches BYOD candle bytes after the object header:
    /// `uint, f32, uint, f32, uint, int, int`.
    #[allow(clippy::too_many_arguments)]
    pub fn write_particle_system_fields(
        &mut self,
        u_a: u8,
        f_a: f32,
        u_b: u8,
        f_b: f32,
        u_c: u8,
        i_a: u8,
        i_b: u8,
    ) {
        self.write_uint_u8(u_a);
        self.write_f32(f_a);
        self.write_uint_u8(u_b);
        self.write_f32(f_b);
        self.write_uint_u8(u_c);
        self.write_int_u8(i_a);
        self.write_int_u8(i_b);
    }

    /// `FUN_00414d20` shared param block.
    pub fn write_param_block(&mut self, f_a: f32, i_a: u8, i_b: u8, f_b: f32) {
        self.write_object_header(obj::PARAM_BLOCK, obj::PARAM_BLOCK_VER);
        self.write_f32(f_a);
        self.write_int_u8(i_a);
        self.write_int_u8(i_b);
        self.write_f32(f_b);
    }

    /// `FUN_00419be0` graph section header (`0x31` + count). Nested nodes omitted.
    pub fn write_graph_header(&mut self, count: u8) {
        self.write_object_header(obj::GRAPH, count);
    }

    /// `FUN_00432fd0` sampled curve: header `0x71`/`3` + channel count + sample count + f32 grid.
    ///
    /// `values` length must be `channels * samples` in sample-major order
    /// (for each sample, emit all channels).
    pub fn write_curve(
        &mut self,
        channels: u8,
        samples: u8,
        values: &[f32],
    ) -> Result<(), PfxError> {
        let need = channels as usize * samples as usize;
        if values.len() != need {
            return Err(PfxError::Msg(format!(
                "curve needs {need} floats, got {}",
                values.len()
            )));
        }
        self.write_object_header(obj::CURVE, obj::CURVE_VER);
        self.write_int_u8(channels);
        self.write_int_u8(samples);
        for v in values {
            self.write_f32(*v);
        }
        Ok(())
    }

    /// `FUN_00418190` texture embed: relative path string + optional JPEG/blob bytes.
    ///
    /// When `data` is `Some`, matches the `param_3 == 0` branch (size +
    /// `R_MemoryFile::write` → on-disk `0x13` blob). `None` writes int `0`
    /// (external reference, no embed).
    pub fn write_embedded_file(
        &mut self,
        relative_path: &str,
        data: Option<&[u8]>,
    ) -> Result<(), PfxError> {
        self.write_str(relative_path)?;
        match data {
            Some(bytes) => self.write_blob(bytes)?,
            None => self.write_int_u8(0),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rps::parse_rps;

    #[test]
    fn object_headers_match_candle_markers() {
        let mut w = MemoryFileWriter::new();
        w.write_object_header(obj::PARTICLE_SYSTEM, obj::PARTICLE_SYSTEM_VER);
        w.write_object_header(obj::GRAPH, 1);
        w.write_object_header(obj::PARAM_BLOCK, obj::PARAM_BLOCK_VER);
        w.write_object_header(obj::EMITTER, obj::EMITTER_VER);
        w.write_object_header(obj::CURVE, obj::CURVE_VER);
        w.write_object_header(obj::COMPANION, obj::COMPANION_VER);
        let h = parse_rps(w.as_bytes()).unwrap();
        let vals: Vec<u8> = h
            .values
            .iter()
            .filter_map(|v| match v {
                crate::rps::RpsValue::U8 { value, .. } => Some(*value),
                _ => None,
            })
            .collect();
        assert_eq!(
            vals,
            vec![0x60, 5, 0x31, 1, 0x20, 3, 0x30, 4, 0x71, 3, 0x40, 2]
        );
    }

    #[test]
    fn preamble_via_writer_matches_helper() {
        let mut w = MemoryFileWriter::new();
        w.write_pse_preamble(96, 5, 12, 2.5, 10, 2.5).unwrap();
        let via_helper = crate::rps::pse_header_to_rps_preamble(96, 5, 12, 2.5, 10, 2.5).unwrap();
        assert_eq!(w.as_bytes(), via_helper.as_slice());
    }

    #[test]
    fn curve_matches_candle_first_curve_header() {
        let samples: Vec<f32> = (0..16).map(|i| i as f32 * 0.1).collect();
        let mut w = MemoryFileWriter::new();
        w.write_curve(1, 16, &samples).unwrap();
        let h = parse_rps(w.as_bytes()).unwrap();
        assert!(matches!(
            &h.values[0],
            crate::rps::RpsValue::U8 {
                value: 0x71,
                ..
            }
        ));
        assert!(matches!(&h.values[1], crate::rps::RpsValue::U8 { value: 3, .. }));
        assert!(matches!(&h.values[2], crate::rps::RpsValue::U8 { value: 1, .. }));
        assert!(matches!(&h.values[3], crate::rps::RpsValue::U8 { value: 16, .. }));
        assert_eq!(h.values.len(), 4 + 16);
    }

    #[test]
    fn embedded_file_writes_str_then_blob() {
        let mut w = MemoryFileWriter::new();
        w.write_embedded_file(r"fire\x.JPG", Some(b"\xff\xd8\xff\xd9"))
            .unwrap();
        let h = parse_rps(w.as_bytes()).unwrap();
        assert!(matches!(&h.values[0], crate::rps::RpsValue::Str { value } if value == r"fire\x.JPG"));
        assert!(matches!(&h.values[1], crate::rps::RpsValue::Blob { data } if data == b"\xff\xd8\xff\xd9"));
    }

    #[test]
    fn candle_prefix_through_graph_header() {
        // fire_candle_flame_large_1.rps values[0..12] (BYOD; not committed).
        let mut w = MemoryFileWriter::new();
        w.write_object_header(obj::PARTICLE_SYSTEM, obj::PARTICLE_SYSTEM_VER);
        w.write_particle_system_fields(12, 2.5, 10, 2.5, 10, 0, 0);
        w.write_graph_header(1);
        w.write_param_block(1.0, 0, 1, 0.1);
        let expected: &[u8] = &[
            0x14, 0x60, 0x14, 0x05, 0x11, 12, 0x09, 0x00, 0x00, 0x20, 0x40, 0x11, 10, 0x09, 0x00,
            0x00, 0x20, 0x40, 0x11, 10, 0x14, 0, 0x14, 0, 0x14, 0x31, 0x14, 1, 0x14, 0x20, 0x14,
            3, 0x09, 0x00, 0x00, 0x80, 0x3f, 0x14, 0, 0x14, 1, 0x09, 0xcd, 0xcc, 0xcc, 0x3d,
        ];
        assert_eq!(w.as_bytes(), expected);
    }
}
