use maxfx_engine::{discover_ras_files, list_level_packages, open_archive, parse_levels_txt};
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
            if let Some(txt) = extract_levels_txt(&arc) {
                let entries = parse_levels_txt(&txt);
                println!("levels.txt entries ({}):", entries.len());
                for e in entries.iter().take(12) {
                    println!("  {} → {}", e.id, e.ldb_archive_path());
                }
                if entries.len() > 12 {
                    println!("  … {} more", entries.len() - 12);
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

fn extract_levels_txt(arc: &RasArchive) -> Option<String> {
    const WANT: &str = "\\data\\database\\levels\\levels.txt";
    for f in &arc.files {
        let dname = arc
            .directories
            .get(f.dir_index as usize)
            .map(|d| d.name.as_str())
            .unwrap_or("\\");
        let full = format!("{dname}{}", f.name);
        if full.eq_ignore_ascii_case(WANT) {
            return Some(String::from_utf8_lossy(&f.data).into_owned());
        }
    }
    None
}
