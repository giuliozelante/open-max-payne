//! End-to-end integration tests for `maxfx-engine` (`parse_ldb_scene`, `kf2`, CLI, and optional BYOD smoke).

use maxfx_engine::{
    kf2::parse_kf2_or_skd,
    scene::{decode_texture_blob, parse_ldb_scene},
    LDB_IMPORT_LIGHTMAP_MARK, LDB_IMPORT_LIGHTMAP_RGB_SIZE, LDB_IMPORT_MATERIAL_LIST_TAG,
    LDB_IMPORT_MATERIAL_TAG, LDB_IMPORT_VERSION, LDB_SECTION_KIND_VEC3, LDB_SECTION_TAG,
};
use std::path::Path;

fn push_mf_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&[0x0d, 0x14, s.len() as u8]);
    buf.extend_from_slice(s.as_bytes());
}

fn push_xform(buf: &mut Vec<u8>, pos: [f32; 3]) {
    buf.push(0x1a);
    for v in [
        1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, pos[0], pos[1], pos[2],
    ] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
}

/// Build a complete synthetic `.ldb` binary that exercises all 14 `ImportLevel` sections
/// through `parse_ldb_scene` to exact EOF.
fn build_synthetic_full_ldb() -> Vec<u8> {
    let mut b = Vec::new();

    // Geometry preamble: section 1 (`0x1c 0x12`, count=3, flag=0) + 3 vec3s
    b.extend_from_slice(&[LDB_SECTION_TAG, LDB_SECTION_KIND_VEC3, 3, 0, 0]);
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]] {
        b.push(0x16);
        for c in v {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }
    // Section 2 (`0x1c 0x14 1` -> 1 face record)
    b.extend_from_slice(&[LDB_SECTION_TAG, 0x14, 1]);
    b.extend_from_slice(&[0x14, 0, 0x14, 3, 0x14, 0, 0x14, 1]); // i0, i1, i2, attr
    for v in [[0.0f32, 1.0, 0.0], [0.0, 0.0, 0.0]] {
        b.push(0x16);
        for c in v {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }

    // ImportLevel header: `14 20`
    b.extend_from_slice(&[0x14, LDB_IMPORT_VERSION as u8]);

    // 1. Textures (1 uncompressed 24-bit 1x1 TGA)
    b.extend_from_slice(&[0x14, 1]);
    push_mf_str(&mut b, "floor.tga");
    b.extend_from_slice(&[0x11, 0]); // flag = 0
    let mut tga = vec![0u8; 21];
    tga[2] = 2;
    tga[12] = 1;
    tga[14] = 1;
    tga[16] = 24;
    tga[18..21].copy_from_slice(&[0x40, 0x80, 0xc0]);
    b.extend_from_slice(&[0x14, tga.len() as u8]);
    b.extend_from_slice(&tga);

    // 2. Materials (1 list with 1 material id=0 -> floor.tga)
    b.push(LDB_IMPORT_MATERIAL_LIST_TAG);
    b.extend_from_slice(&[0x14, 1, 0x14, 0]);
    b.push(LDB_IMPORT_MATERIAL_TAG);
    push_mf_str(&mut b, "stone");
    push_mf_str(&mut b, "floor.tga");
    b.extend_from_slice(&[0x14, 0]);

    // 3. Material categories (count=1, name, member_count=1, texture, path, path2, bool, bool)
    b.extend_from_slice(&[0x14, 1]);
    push_mf_str(&mut b, "stone");
    b.extend_from_slice(&[0x14, 1]); // member_count = 1
    push_mf_str(&mut b, "floor.tga");
    push_mf_str(&mut b, "floor.tga");
    push_mf_str(&mut b, "");
    b.extend_from_slice(&[0x0e, 0, 0x0e, 0]);

    // 4. Lightmaps (1 lightmap of 256*256*3 + 18 declared size, last blob omits 18B trailer)
    b.extend_from_slice(&[0x14, 1, 0x14, 0, 0x11, 0, 0x12]);
    b.extend_from_slice(&LDB_IMPORT_LIGHTMAP_RGB_SIZE.to_le_bytes());
    b.extend_from_slice(&512u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&[0x01, 0x00, 0x01]);
    b.extend_from_slice(&LDB_IMPORT_LIGHTMAP_MARK);
    b.extend(std::iter::repeat_n(0xdfu8, 256 * 256 * 3));

    // 5. Exits (pair: exit with room corners + link exit without corners, then post-exit 0x15 stream)
    b.extend_from_slice(&[0x14, 1]);
    push_mf_str(&mut b, "::StartRoom::Exit0");
    b.extend_from_slice(&[0x11, 3]);
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        b.push(0x16);
        for c in v {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }
    b.push(0x16);
    for c in [0.0f32, 0.0, 1.0] {
        b.extend_from_slice(&c.to_le_bytes());
    }
    push_xform(&mut b, [0.0, 0.0, 0.0]);
    b.extend_from_slice(&[0x14, 1, 0x14, 0]);
    push_mf_str(&mut b, "::StartRoom::Exit0");
    b.extend_from_slice(&[0x1c, 0x14, 1, 0x1c, 0x14, 0]);

    // 6. Static point stream (`0x1c` + `0x12` count=3 + subtype=4 + 3 point bodies)
    b.push(0x1c);
    b.extend_from_slice(&[0x12, 3, 0, 0, 0x14, 4]);
    for k in 0..3u8 {
        if k > 0 {
            b.extend_from_slice(&[0x14, k]);
        }
        for uv in [[0.0f32, 0.0], [0.5, 0.5]] {
            b.push(0x15);
            b.extend_from_slice(&uv[0].to_le_bytes());
            b.extend_from_slice(&uv[1].to_le_bytes());
        }
        b.extend_from_slice(&[0x03, 0xff, 0xff, 0xff, 0xff, 0x0e, 0]);
    }

    // 7. Static mesh (Form A: a=1, b=501, n=3) + 1 triangle poly
    b.extend_from_slice(&[0x14, 1, 0x13, 0xf5, 0x01, 0x11, 3]);
    for v in [[0.0f32, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 0.0, 4.0]] {
        b.push(0x16);
        for c in v {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }
    b.extend_from_slice(&[0x1c, 0x14, 3]);
    for _ in 0..3 {
        b.push(0x16);
        for c in [0.0f32, 1.0, 0.0] {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }
    push_xform(&mut b, [0.0, 0.0, 0.0]);
    // 1 mesh poly (pref=0, verts=3, normal=(0,1,0), flag=1, mat=0, lmap=0, 2 floats, z1=0)
    b.extend_from_slice(&[0x14, 1, 0x14, 0, 0x14, 0, 0x14, 3]);
    b.push(0x16);
    for c in [0.0f32, 1.0, 0.0] {
        b.extend_from_slice(&c.to_le_bytes());
    }
    b.extend_from_slice(&[0x11, 1, 0x14, 0, 0x14, 0]);
    b.push(0x09);
    b.extend_from_slice(&1.0f32.to_le_bytes());
    b.push(0x09);
    b.extend_from_slice(&1.0f32.to_le_bytes());
    b.extend_from_slice(&[0x14, 0]);

    // 8. Directional lights = 0 (`14 00`)
    b.extend_from_slice(&[0x14, 0]);

    // 9. Jumppoints = 1
    b.extend_from_slice(&[0x14, 1]);
    push_mf_str(&mut b, "::StartRoom::Jumppoint 00");
    push_mf_str(&mut b, "");
    push_xform(&mut b, [1.0, 0.0, 1.0]);
    push_xform(&mut b, [1.0, 0.0, 1.0]);
    b.extend_from_slice(&[0x14, 0]);
    push_mf_str(&mut b, "");
    b.extend_from_slice(&[0x14, 2]);

    // 10. FSMs = 0, 11. Characters = 0, 12. Triggers = 0
    b.extend_from_slice(&[0x14, 0, 0x14, 0, 0x14, 0]);

    // 13. Dynamic point stream (1 body) + Dynamic meshes = 0
    b.extend_from_slice(&[0x1c, 0x13, 1, 0, 0x14, 1]);
    for uv in [[0.0f32, 0.0], [0.5, 0.5]] {
        b.push(0x15);
        b.extend_from_slice(&uv[0].to_le_bytes());
        b.extend_from_slice(&uv[1].to_le_bytes());
    }
    b.extend_from_slice(&[0x03, 0xff, 0xff, 0xff, 0xff, 0x0e, 0]);
    b.extend_from_slice(&[0x14, 0]); // 0 dynamic meshes

    // 14. Level items (ver=14, count=0), Point lights = 0, Rooms = 1
    b.extend_from_slice(&[0x14, 14, 0x14, 0, 0x14, 0, 0x14, 1]);
    b.extend_from_slice(&[0x14, 0]); // room_id = 0
    b.extend_from_slice(&[0x1c, 0x14, 1, 0x13, 0xf5, 0x01]); // static_meshes = [501]
    b.extend_from_slice(&[0x1c, 0x14, 0]); // list 1
    b.extend_from_slice(&[0x1c, 0x14, 1]); // list 2 (exits)
    push_mf_str(&mut b, "::StartRoom::Exit0");
    for _ in 3..9 {
        b.extend_from_slice(&[0x1c, 0x14, 0]);
    }
    push_mf_str(&mut b, "::StartRoom");
    b.push(0x09);
    b.extend_from_slice(&1.0f32.to_le_bytes());
    b.extend_from_slice(&[0x14, 0, 0x14, 1, 0x14, 0, 0x14, 1]);

    b
}

#[test]
fn e2e_synthetic_ldb_scene_decodes_to_exact_eof() {
    let bytes = build_synthetic_full_ldb();
    let scene = parse_ldb_scene(&bytes).expect("synthetic full LDB should parse");
    assert_eq!(scene.bytes_consumed, bytes.len());
    assert_eq!(scene.textures.len(), 1);
    assert_eq!(scene.textures[0].pixels, vec![0x00c08040]);
    assert_eq!(scene.lightmaps.len(), 1);
    assert_eq!(scene.static_meshes.len(), 1);
    assert_eq!(scene.static_meshes[0].mesh_id, 501);
    assert_eq!(scene.static_meshes[0].polys.len(), 1);
    assert_eq!(scene.jumppoints.len(), 1);
    assert_eq!(scene.rooms.len(), 1);
    assert_eq!(scene.rooms[0].name, "::StartRoom");
}

#[test]
fn e2e_pcx_and_kf2_decoders_roundtrip() {
    // 8-bit 2x1 PCX with VGA palette marker 0x0c at EOF-769
    let mut pcx = vec![0u8; 128 + 2 + 1 + 768];
    pcx[0] = 0x0a;
    pcx[2] = 1;
    pcx[3] = 8;
    pcx[8] = 1; // xmax = 1 -> width = 2
    pcx[10] = 0; // ymax = 0 -> height = 1
    pcx[66] = 2; // bytes per line = 2
    pcx[128] = 1; // pixel 0 -> palette index 1
    pcx[129] = 2; // pixel 1 -> palette index 2
    let pal_pos = pcx.len() - 769;
    pcx[pal_pos] = 0x0c;
    pcx[pal_pos + 1 + 3..pal_pos + 1 + 6].copy_from_slice(&[0x10, 0x20, 0x30]);
    pcx[pal_pos + 1 + 6..pal_pos + 1 + 9].copy_from_slice(&[0xaa, 0xbb, 0xcc]);
    let (w, h, px) = decode_texture_blob(&pcx).expect("PCX should decode");
    assert_eq!((w, h), (2, 1));
    assert_eq!(px, vec![0x00102030, 0x00aabbcc]);

    // Multi-chunk KF2
    let mut kf2 = Vec::new();
    for (id, name) in [(0x0001_000fu32, "SkinMat"), (0x0001_0014u32, "Pelvis")] {
        kf2.push(0x0c);
        kf2.extend_from_slice(&id.to_le_bytes());
        kf2.extend_from_slice(&1u32.to_le_bytes());
        let sz = (13 + 3 + name.len() + 13) as u32;
        kf2.extend_from_slice(&sz.to_le_bytes());
        push_mf_str(&mut kf2, name);
        kf2.push(0x16);
        for c in [1.0f32, 2.0, 3.0] {
            kf2.extend_from_slice(&c.to_le_bytes());
        }
    }
    let sum = parse_kf2_or_skd(&kf2).expect("KF2 should parse");
    assert_eq!(sum.chunks.len(), 2);
    assert_eq!(sum.total_vec3, 2);
    assert_eq!(sum.all_strings, vec!["SkinMat", "Pelvis"]);
}

#[test]
fn e2e_byod_retail_levels_decode_to_eof_when_present() {
    let candidates = [
        "/home/gzelante/Games/MaxPayne1/data.disabled-hires/database/levels/front_end/Tutorial.ldb",
        "/tmp/omp-xlevel1/data/database/levels/part1/Part0_Level1.ldb",
    ];
    for p in candidates {
        let path = Path::new(p);
        if !path.is_file() {
            continue;
        }
        let data = std::fs::read(path).unwrap();
        let scene = parse_ldb_scene(&data).unwrap_or_else(|| panic!("failed to parse {p}"));
        assert_eq!(scene.bytes_consumed, data.len(), "EOF mismatch on {p}");
        assert!(!scene.rooms.is_empty());
        assert!(!scene.static_meshes.is_empty());
        assert!(!scene.jumppoints.is_empty());
    }
}
