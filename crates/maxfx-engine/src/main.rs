use maxfx_engine::{
    classify_ldb_lead, count_ras_ldb_leading_vec3, discover_ras_files, list_level_packages,
    looks_like_ldb_or_ai, looks_like_memory_file_ai, looks_like_memory_file_ldb,
    looks_like_ras_ldb, open_archive, parse_levels_txt, parse_ras_ldb_header,
    parse_ras_ldb_section_at, ras_ldb_count_section_end, split_ldb_records, LDB_IMPORT_VERSION,
    LDB_RAS_MAGIC, LDB_SECTION_KIND_MF, LDB_SECTION_TAG,
};
use openras::RasArchive;
use std::env;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: maxfx-engine <archive.ras|archive.mpm|game-dir|level.ldb|.ai>");
        return ExitCode::FAILURE;
    };
    let p = Path::new(&path);
    if p.is_file() {
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
        if ext.eq_ignore_ascii_case("ldb") || ext.eq_ignore_ascii_case("ai") {
            return match std::fs::read(p) {
                Ok(data) => {
                    if looks_like_ras_ldb(&data) {
                        let h = parse_ras_ldb_header(&data).unwrap();
                        let nvec = count_ras_ldb_leading_vec3(&data);
                        let end = ras_ldb_count_section_end(&h);
                        println!(
                            "{}: {} bytes, RAS .ldb magic {:#x} count={} flag={} leading_vec3={} (count_match={})",
                            p.display(),
                            data.len(),
                            h.magic,
                            h.count,
                            h.flag,
                            nvec,
                            h.flag == 0 && h.count as usize == nvec
                        );
                        if h.flag == 0 {
                            if let Some(s2) = parse_ras_ldb_section_at(&data, end) {
                                println!(
                                    "  section2 @{end}: tag={:#x} kind={:#x} count={} {}",
                                    LDB_SECTION_TAG,
                                    s2.kind,
                                    s2.count,
                                    if s2.kind == LDB_SECTION_KIND_MF {
                                        "MF-body"
                                    } else {
                                        "other"
                                    }
                                );
                            }
                        }
                        return ExitCode::SUCCESS;
                    }
                    if looks_like_memory_file_ldb(&data) {
                        println!(
                            "{}: {} bytes, MemoryFile-native ImportLevel (version tag {:#x})",
                            p.display(),
                            data.len(),
                            LDB_IMPORT_VERSION
                        );
                        return ExitCode::SUCCESS;
                    }
                    if looks_like_memory_file_ai(&data) {
                        println!(
                            "{}: {} bytes, MemoryFile-tagged .ai (head 0x14 {:#x})",
                            p.display(),
                            data.len(),
                            data[1]
                        );
                        return ExitCode::SUCCESS;
                    }
                    if !looks_like_ldb_or_ai(&data) {
                        eprintln!(
                            "error: unrecognized level blob (want RAS {:#x}, MF 14 {:#x}, or legacy space/>)",
                            LDB_RAS_MAGIC, LDB_IMPORT_VERSION
                        );
                        return ExitCode::FAILURE;
                    }
                    let recs = split_ldb_records(&data).unwrap();
                    println!(
                        "{}: {} bytes, {} >-records after legacy preamble",
                        p.display(),
                        data.len(),
                        recs.len()
                    );
                    let mut leads = std::collections::BTreeMap::<u8, usize>::new();
                    for r in &recs {
                        if let Some(b) = r.lead_byte() {
                            *leads.entry(b).or_default() += 1;
                        }
                    }
                    println!("lead bytes (top):");
                    let mut pairs: Vec<_> = leads.into_iter().collect();
                    pairs.sort_by_key(|a| std::cmp::Reverse(a.1));
                    for (b, n) in pairs.into_iter().take(12) {
                        println!("  0x{b:02x} ({}): {n}", classify_ldb_lead(b));
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            };
        }
    }
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
