use clap::{Parser, Subcommand};
use openpfx::{
    pse_header_to_rps_preamble, pse_to_rps_root, read_pse, read_rps, roundtrip_rps, RpsValue,
};
use std::fs;
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
    /// Dump tagged values from a runtime `.rps`
    DumpRps { path: PathBuf },
    /// Write PSE flat-header fields as an RPS tagged preamble (graphs not included)
    ExportPreamble {
        pse: PathBuf,
        #[arg(short = 'o', long)]
        output: PathBuf,
    },
    /// Export PSE root object fields to RPS (graphs / emitters still omitted)
    ExportRoot {
        pse: PathBuf,
        #[arg(short = 'o', long)]
        output: PathBuf,
    },
    /// Parse a retail `.rps` and write it back (encode parity check; BYOD local use)
    Repack {
        input: PathBuf,
        #[arg(short = 'o', long)]
        output: PathBuf,
    },
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
            println!(
                "word0={} word1={} word2={} word3={} word4={} flags={}/{}",
                h.word0, h.word1, h.word2, h.word3, h.word4, h.flag_a, h.flag_b
            );
            println!("float_a={} float_b={}", h.float_a, h.float_b);
            println!("paths ({}):", h.ascii_paths.len());
            for p in &h.ascii_paths {
                println!("  {p}");
            }
        }
        Cmd::DumpRps { path } => {
            let h = read_rps(&path)?;
            println!(
                "{}: {} values, {} bytes walked",
                path.display(),
                h.values.len(),
                h.bytes_consumed
            );
            for (i, v) in h.values.iter().enumerate() {
                match v {
                    RpsValue::U8 { tag, value } => {
                        println!("  [{i}] u8 tag=0x{tag:02x} value={value}")
                    }
                    RpsValue::F32 { value } => println!("  [{i}] f32 {value}"),
                    RpsValue::Str { value } => println!("  [{i}] str {value:?}"),
                    RpsValue::Blob { data } => {
                        let jpeg = data.len() >= 2 && data[0] == 0xff && data[1] == 0xd8;
                        println!(
                            "  [{i}] blob len={}{}",
                            data.len(),
                            if jpeg { " (jpeg)" } else { "" }
                        );
                    }
                    RpsValue::Vec3 { x, y, z } => println!("  [{i}] vec3 ({x}, {y}, {z})"),
                    RpsValue::Unknown { tag, at } => {
                        println!("  [{i}] unknown tag=0x{tag:02x} at={at}")
                    }
                }
            }
        }
        Cmd::ExportPreamble { pse, output } => {
            let h = read_pse(&pse)?;
            let bytes = pse_header_to_rps_preamble(
                h.word0, h.word1, h.word2, h.float_a, h.word3, h.float_b,
            )?;
            fs::write(&output, &bytes)?;
            println!(
                "Wrote {} bytes preamble → {} (particle graphs NOT exported yet)",
                bytes.len(),
                output.display()
            );
        }
        Cmd::ExportRoot { pse, output } => {
            let h = read_pse(&pse)?;
            let bytes = pse_to_rps_root(&h)?;
            fs::write(&output, &bytes)?;
            println!(
                "Wrote {} bytes root → {} (graphs/emitters NOT exported yet)",
                bytes.len(),
                output.display()
            );
        }
        Cmd::Repack { input, output } => {
            let data = fs::read(&input)?;
            let out = roundtrip_rps(&data)?;
            fs::write(&output, &out)?;
            println!(
                "Repacked {} → {} ({} bytes, identical={})",
                input.display(),
                output.display(),
                out.len(),
                out == data
            );
        }
    }
    Ok(())
}
