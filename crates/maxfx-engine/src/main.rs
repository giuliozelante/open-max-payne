use maxfx_engine::open_archive;
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: maxfx-engine <archive.ras|archive.mpm>");
        return ExitCode::FAILURE;
    };
    match open_archive(std::path::Path::new(&path)) {
        Ok(a) => {
            println!(
                "{}: {} files, {} dirs, ver={} compat={}",
                a.label, a.file_count, a.dir_count, a.version, a.compatibility
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
