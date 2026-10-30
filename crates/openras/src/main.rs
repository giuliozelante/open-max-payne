use clap::{Parser, Subcommand};
use openras::cli_rasmaker::{looks_like_rasmaker, parse_rasmaker, usage_text, RasMakerCmd};
use openras::RasArchive;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "openras",
    version,
    about = "Remedy Archive System (RAS/MPM) for Max Payne 1 — also accepts RASMaker -x/-v/-a flags"
)]
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
        /// RASMaker `-p`: strip source directory from entry paths (default for this subcommand)
        #[arg(short = 'p', long = "exclude-source-path", default_value_t = true)]
        exclude_source_path: bool,
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
    let raw: Vec<String> = env::args().skip(1).collect();
    let result = if looks_like_rasmaker(&raw) {
        run_rasmaker(&raw)
    } else {
        let cli = Cli::parse();
        run_subcommand(cli)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_rasmaker(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    match parse_rasmaker(args) {
        Err(e) => {
            eprintln!("{e}");
            if !e.to_string().contains("Usage:") {
                eprint!("{}", usage_text("openras"));
            }
            Err(e.into())
        }
        Ok(cmd) => match cmd {
            RasMakerCmd::View { archive } => {
                let arc = RasArchive::read(&archive)?;
                print!("{}", arc.list_text(&archive.display().to_string()));
                Ok(())
            }
            RasMakerCmd::Extract { archive } => {
                let arc = RasArchive::read(&archive)?;
                let dest = PathBuf::from(".");
                arc.extract(&dest)?;
                println!("Extracted {} files -> {}", arc.files.len(), dest.display());
                Ok(())
            }
            RasMakerCmd::Add {
                source,
                dest,
                compress,
                exclude_source_path,
                delete_source,
            } => {
                let arc = RasArchive::from_directory(&source, 0x1234_5678, exclude_source_path)?;
                arc.write(&dest, compress)?;
                println!("Wrote {} ({} files)", dest.display(), arc.files.len());
                if delete_source {
                    remove_packed_sources(&source)?;
                    println!("Removed source {}", source.display());
                }
                Ok(())
            }
        },
    }
}

fn remove_packed_sources(source: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if source.is_file() {
        fs::remove_file(source)?;
    } else if source.is_dir() {
        fs::remove_dir_all(source)?;
    }
    Ok(())
}

fn run_subcommand(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
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
            exclude_source_path,
        } => {
            let arc = RasArchive::from_directory(&directory, seed, exclude_source_path)?;
            arc.write(&output, compress)?;
            println!("Wrote {} ({} files)", output.display(), arc.files.len());
        }
    }
    Ok(())
}
