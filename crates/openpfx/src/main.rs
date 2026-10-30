use clap::{Parser, Subcommand};
use openpfx::{
    pse_header_to_rps_preamble, pse_to_rps_export, read_pse, read_rps, roundtrip_rps,
    scan_pse_emitters, PseExportOptions, RpsValue,
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
        /// Append empty graph header `0x31`/`0` (structural stub)
        #[arg(long)]
        empty_graph: bool,
        /// Append graph + stub emitter-open per scanned PSE emitter name
        #[arg(long)]
        emitter_stubs: bool,
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
            let data = fs::read(&path)?;
            let h = openpfx::parse_pse(&data)?;
            println!(
                "word0={} word1={} word2={} word3={} word4={} flags={}/{}",
                h.word0, h.word1, h.word2, h.word3, h.word4, h.flag_a, h.flag_b
            );
            println!("float_a={} float_b={}", h.float_a, h.float_b);
            let emitters = scan_pse_emitters(&data);
            println!("emitters ({}):", emitters.len());
            for e in &emitters {
                println!("  @{:#x} {}", e.offset, e.name);
            }
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
                    RpsValue::Int { value } => println!("  [{i}] int {value}"),
                    RpsValue::UInt { value } => println!("  [{i}] uint {value}"),
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
                    RpsValue::Chunk { id, version, size } => {
                        println!("  [{i}] chunk id={id:#x} ver={version} size={size}")
                    }
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
        Cmd::ExportRoot {
            pse,
            output,
            empty_graph,
            emitter_stubs,
        } => {
            let data = fs::read(&pse)?;
            let h = openpfx::parse_pse(&data)?;
            let stubs = if emitter_stubs {
                scan_pse_emitters(&data)
                    .into_iter()
                    .map(|e| e.name)
                    .collect()
            } else {
                Vec::new()
            };
            let bytes = pse_to_rps_export(
                &h,
                &PseExportOptions {
                    empty_graph: empty_graph && stubs.is_empty(),
                    emitter_stubs: stubs.clone(),
                },
            )?;
            fs::write(&output, &bytes)?;
            let tag = if !stubs.is_empty() {
                format!("+{}-emitter-stubs", stubs.len())
            } else if empty_graph {
                "+empty-graph".into()
            } else {
                String::new()
            };
            println!(
                "Wrote {} bytes root{} → {} (curves/paths NOT exported yet)",
                bytes.len(),
                tag,
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
