use clap::{Parser, Subcommand};
use openras::RasArchive;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "openras", version, about = "Remedy Archive System (RAS/MPM) for Max Payne 1")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List archive contents
    List { archive: PathBuf },
    /// Extract archive to a directory
    Extract {
        archive: PathBuf,
        #[arg(short = 'd', long = "dest", alias = "out", short_alias = 'o')]
        dest: PathBuf,
    },
    /// Pack a directory into .ras/.mpm
    Pack {
        directory: PathBuf,
        output: PathBuf,
        #[arg(long, default_value = "0x12345678", value_parser = parse_seed)]
        seed: i32,
        #[arg(short = 'c', long)]
        compress: bool,
    },
}

fn parse_seed(s: &str) -> Result<i32, String> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i32::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else {
        t.parse::<i32>().map_err(|e| e.to_string())
    }
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
        Cmd::List { archive } => {
            let arc = RasArchive::read(&archive)?;
            print!("{}", arc.list_text(&archive.display().to_string()));
        }
        Cmd::Extract { archive, dest } => {
            let arc = RasArchive::read(&archive)?;
            arc.extract(&dest)?;
            println!("Extracted {} files -> {}", arc.files.len(), dest.display());
        }
        Cmd::Pack {
            directory,
            output,
            seed,
            compress,
        } => {
            let arc = RasArchive::from_directory(&directory, seed)?;
            arc.write(&output, compress)?;
            println!("Wrote {} ({} files)", output.display(), arc.files.len());
        }
    }
    Ok(())
}
