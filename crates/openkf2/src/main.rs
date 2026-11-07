use clap::{Parser, Subcommand};
use openkf2::{decode_texture_blob, gltf, parse_kf2_anim, parse_kfs_mesh, parse_skd};
use openkf2::{DecodedTexture, Kf2Anim};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "openkf2", version, about = "ActorFX KF2/SKD/KFS model tooling")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Export a model to binary glTF
    Export(Export),
}

#[derive(clap::Args)]
struct Export {
    /// The `.kfs` or `.kf2` mesh
    mesh: PathBuf,
    /// The `.skd` skin
    #[arg(long, requires = "pose")]
    skin: Option<PathBuf>,
    /// The `.kf2` that holds the bind pose, needed with `--skin`
    #[arg(long)]
    pose: Option<PathBuf>,
    /// A `.kf2` clip, named after its file; repeat for more
    #[arg(long, requires = "skin")]
    clip: Vec<PathBuf>,
    /// A folder to look for textures in, after the mesh's own; repeat for more
    #[arg(long)]
    textures: Vec<PathBuf>,
    #[arg(short = 'o', long)]
    output: PathBuf,
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_clip(path: &Path) -> Result<Kf2Anim, String> {
    parse_kf2_anim(&read(path)?).ok_or_else(|| format!("{}: not a KF2 clip", path.display()))
}

/// The texture named `file` in the first of `dirs` that has it, ignoring case.
fn find_texture(file: &str, dirs: &[PathBuf]) -> Option<DecodedTexture> {
    let found = dirs
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flat_map(|entries| entries.flatten())
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(file)
        });
    let Some((width, height, pixels)) = found
        .and_then(|entry| std::fs::read(entry.path()).ok())
        .and_then(|bytes| decode_texture_blob(&bytes))
    else {
        eprintln!("warning: texture {file} not found or not readable");
        return None;
    };
    Some(DecodedTexture {
        path: file.to_string(),
        flag: 0,
        width,
        height,
        pixels,
    })
}

fn export(args: &Export) -> Result<(), String> {
    let mut dirs = vec![args.mesh.parent().unwrap_or(Path::new(".")).to_path_buf()];
    dirs.extend_from_slice(&args.textures);
    let mesh = parse_kfs_mesh(&read(&args.mesh)?, |file| find_texture(file, &dirs))
        .ok_or_else(|| format!("{}: not a KF2 mesh", args.mesh.display()))?;
    let skin = match &args.skin {
        Some(path) => Some(
            parse_skd(&read(path)?)
                .ok_or_else(|| format!("{}: not an SKD skin", path.display()))?,
        ),
        None => None,
    };
    let pose = args.pose.as_deref().map(read_clip).transpose()?;
    let mut model = gltf::model(&mesh, skin.as_ref().zip(pose.as_ref()));
    for path in &args.clip {
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        let animation = gltf::animation(&model, &name, &read_clip(path)?);
        model.animations.push(animation);
    }
    std::fs::write(&args.output, gltf::to_glb(&model))
        .map_err(|e| format!("{}: {e}", args.output.display()))
}

fn main() -> ExitCode {
    let Cmd::Export(args) = Cli::parse().cmd;
    match export(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}
