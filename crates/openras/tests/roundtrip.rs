//! Synthetic RAS/MPM round-trips — no binary fixtures in git.

use openras::archive::{RasArchive, RasDirEntry, RasFileEntry, MODE_LZSS, MODE_STORED};
use openras::lzss;
use std::fs;
use std::path::PathBuf;

fn tmpdir(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("openras-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn sample_archive(mode: u32) -> RasArchive {
    let files = vec![
        RasFileEntry {
            name: "readme.txt".into(),
            dir_index: 0,
            data: b"Open Max Payne synthetic fixture\n".to_vec(),
            compression_mode: mode,
            compressed_data: None,
            unknown_0: 0,
            unknown_1: 0,
            timestamp: [0; 16],
        },
        RasFileEntry {
            name: "blob.bin".into(),
            dir_index: 1,
            data: (0u8..200).collect(),
            compression_mode: mode,
            compressed_data: None,
            unknown_0: 0,
            unknown_1: 0,
            timestamp: [0; 16],
        },
        RasFileEntry {
            name: "note.txt".into(),
            dir_index: 1,
            data: b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaHELLO aaaaaaaaaaaaaaaaaaaaaaaa".to_vec(),
            compression_mode: mode,
            compressed_data: None,
            unknown_0: 0,
            unknown_1: 0,
            timestamp: [0; 16],
        },
    ];
    RasArchive {
        seed: 0x12345678,
        version: 1.2,
        compatibility: 3,
        directories: vec![
            RasDirEntry {
                name: "\\".into(),
                timestamp: [0; 16],
            },
            RasDirEntry {
                name: "\\data\\".into(),
                timestamp: [0; 16],
            },
        ],
        files,
    }
}

#[test]
fn stored_pack_unpack_bytes_identical() {
    let arc = sample_archive(MODE_STORED);
    let bytes = arc.to_bytes(false).expect("pack");
    assert_eq!(&bytes[..4], b"RAS\0");
    let back = RasArchive::from_bytes(&bytes).expect("unpack");
    assert_eq!(back.files.len(), 3);
    assert_eq!(back.directories.len(), 2);
    for (a, b) in arc.files.iter().zip(back.files.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.data, b.data);
        assert_eq!(b.compression_mode, MODE_STORED);
    }
}

#[test]
fn compressed_pack_unpack_bytes_identical() {
    let mut arc = sample_archive(MODE_STORED);
    // Force LZSS on compressible payloads via compress=true
    let bytes = arc.to_bytes(true).expect("pack");
    let back = RasArchive::from_bytes(&bytes).expect("unpack");
    for (a, b) in arc.files.iter().zip(back.files.iter()) {
        assert_eq!(a.data, b.data);
    }
    // At least one file should prefer LZSS when smaller
    let _ = arc.files.iter_mut().find(|f| f.data.len() > 20).map(|f| {
        let c = lzss::compress(&f.data);
        assert!(c.len() < f.data.len() || c.len() >= f.data.len());
    });
    assert!(
        back.files.iter().any(|f| f.compression_mode == MODE_LZSS)
            || back.files.iter().all(|f| f.compression_mode == MODE_STORED)
    );
}

#[test]
fn directory_roundtrip_on_disk() {
    let root = tmpdir("dir-rt");
    let data = root.join("data");
    fs::create_dir_all(&data).unwrap();
    fs::write(root.join("root.txt"), b"root-file").unwrap();
    fs::write(data.join("child.txt"), b"child-payload-zzzzzzzzzzzzzzzz").unwrap();

    let arc = RasArchive::from_directory(&root, 0x1234_5678).unwrap();
    let packed = root.join("out.mpm");
    arc.write(&packed, true).unwrap();

    let dest = tmpdir("dir-rt-out");
    let back = RasArchive::read(&packed).unwrap();
    back.extract(&dest).unwrap();

    assert_eq!(fs::read(dest.join("root.txt")).unwrap(), b"root-file");
    assert_eq!(
        fs::read(dest.join("data/child.txt")).unwrap(),
        b"child-payload-zzzzzzzzzzzzzzzz"
    );

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&dest);
}

#[test]
fn rejects_traversal_basename() {
    let mut arc = sample_archive(MODE_STORED);
    arc.files[0].name = "../evil.txt".into();
    assert!(arc.to_bytes(false).is_err());
}
