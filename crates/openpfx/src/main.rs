use clap::{Parser, Subcommand};
use openpfx::read_pse;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "openpfx",
    version,
    about = "ParticleFX PSE/RPS tooling (draft)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Dump PSE editor-project header fields and embedded absolute paths
    DumpPse { path: PathBuf },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.cmd {
        Cmd::DumpPse { path } => {
            let h = read_pse(&path)?;
            println!("word0={} word1={} word2={}", h.word0, h.word1, h.word2);
            println!("float_a={} float_b={}", h.float_a, h.float_b);
            println!("paths ({}):", h.ascii_paths.len());
            for p in &h.ascii_paths {
                println!("  {p}");
            }
        }
    }
    Ok(())
}
