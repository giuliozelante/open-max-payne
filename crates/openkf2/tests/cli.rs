use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn chunk(id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0x0c];
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(13 + payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

fn text(s: &str) -> Vec<u8> {
    let mut out = vec![0x0d, 0x14, s.len() as u8];
    out.extend_from_slice(s.as_bytes());
    out
}

fn triangle_kfs() -> Vec<u8> {
    let mut node = text("Tri");
    node.extend(text(""));
    let mut vertices = vec![0x14, 3];
    for v in [[1f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
        vertices.push(0x16);
        vertices.extend(v.iter().flat_map(|x| x.to_le_bytes()));
    }
    let mut faces = vec![0x14, 1];
    faces.extend(chunk(0x0001_0008, &[0x14, 3, 0x14, 0, 0x14, 1, 0x14, 2]));
    let mut mesh = chunk(0x0001_0000, &node);
    mesh.extend(chunk(0x0001_0006, &vertices));
    mesh.extend(chunk(0x0001_0007, &faces));
    chunk(0x0001_0005, &mesh)
}

fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("openkf2-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn openkf2(args: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_openkf2"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn export_writes_a_glb_of_the_mesh() {
    let dir = scratch("mesh");
    let (mesh, out) = (dir.join("tri.kfs"), dir.join("tri.glb"));
    std::fs::write(&mesh, triangle_kfs()).unwrap();
    let run = openkf2(&[Path::new("export"), &mesh, Path::new("-o"), &out]);
    assert!(run.status.success(), "{run:?}");
    let glb = std::fs::read(&out).unwrap();
    assert_eq!(&glb[..4], b"glTF");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_file_that_is_no_mesh_fails_and_names_itself() {
    let dir = scratch("bad");
    let (mesh, out) = (dir.join("bad.kfs"), dir.join("bad.glb"));
    std::fs::write(&mesh, b"not a mesh").unwrap();
    let run = openkf2(&[Path::new("export"), &mesh, Path::new("-o"), &out]);
    assert!(!run.status.success());
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("bad.kfs"),
        "{run:?}"
    );
    assert!(!out.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_skin_needs_its_pose() {
    let dir = scratch("pose");
    let (mesh, out) = (dir.join("tri.kfs"), dir.join("tri.glb"));
    std::fs::write(&mesh, triangle_kfs()).unwrap();
    let run = openkf2(&[
        Path::new("export"),
        &mesh,
        Path::new("--skin"),
        &mesh,
        Path::new("-o"),
        &out,
    ]);
    assert!(!run.status.success());
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("--pose"),
        "{run:?}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
