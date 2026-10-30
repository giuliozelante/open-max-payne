use maxfx_engine::{discover_ras_files, list_level_packages, open_archive};
use openras::RasArchive;
use std::env;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: maxfx-engine <archive.ras|archive.mpm|game-dir>");
        return ExitCode::FAILURE;
    };
    let p = Path::new(&path);
    if p.is_dir() {
        return match discover_ras_files(p) {
            Ok(files) => {
                println!("{}: {} *.ras", p.display(), files.len());
                for f in files {
                    match open_archive(&f) {
                        Ok(a) => println!(
                            "  {}: {} files, ver={} compat={}",
                            f.file_name().unwrap_or_default().to_string_lossy(),
                            a.file_count,
                            a.version,
                            a.compatibility
                        ),
                        Err(e) => println!("  {}: error: {e}", f.display()),
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    match RasArchive::read(p) {
        Ok(arc) => {
            match open_archive(p) {
                Ok(a) => println!(
                    "{}: {} files, {} dirs, ver={} compat={}",
                    a.label, a.file_count, a.dir_count, a.version, a.compatibility
                ),
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            }
            let pkgs = list_level_packages(&arc);
            if !pkgs.is_empty() {
                println!("level packages ({}):", pkgs.len());
                for pkg in pkgs {
                    match &pkg.ai {
                        Some(ai) => println!("  {} + {}", pkg.ldb, ai),
                        None => println!("  {} (no .ai)", pkg.ldb),
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
