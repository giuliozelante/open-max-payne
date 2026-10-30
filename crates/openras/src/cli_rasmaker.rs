//! RASMaker 1.20-compatible flag parsing (REA strings from RASMaker.exe).
//!
//! ```text
//! Usage: openras [options] [source] [destination]
//!   -x  extract files
//!   -v  view archive contents
//!   -a  add files
//!   -m  move files (deletes from source)
//!   -c  compress all files (requires -a or -m)
//!   -p  exclude source path (requires -a or -m)
//! ```

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RasMakerCmd {
    View {
        archive: PathBuf,
    },
    Extract {
        archive: PathBuf,
    },
    Add {
        source: PathBuf,
        dest: PathBuf,
        compress: bool,
        exclude_source_path: bool,
        delete_source: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Usage(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Usage(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// True when argv looks like RASMaker-style flags rather than clap subcommands.
pub fn looks_like_rasmaker(args: &[String]) -> bool {
    args.iter().any(|a| {
        matches!(
            a.as_str(),
            "-x" | "-v" | "-a" | "-m" | "-c" | "-p" | "-h" | "-?" | "/?" | "--rasmaker-help"
        )
    })
}

pub fn parse_rasmaker(args: &[String]) -> Result<RasMakerCmd, ParseError> {
    let mut extract = false;
    let mut view = false;
    let mut add = false;
    let mut move_files = false;
    let mut compress = false;
    let mut exclude_source_path = false;
    let mut positionals: Vec<PathBuf> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-x" => extract = true,
            "-v" => view = true,
            "-a" => add = true,
            "-m" => {
                add = true;
                move_files = true;
            }
            "-c" => compress = true,
            "-p" => exclude_source_path = true,
            "-h" | "-?" | "/?" | "--rasmaker-help" => {
                return Err(ParseError::Usage(usage_text("openras")));
            }
            s if s.starts_with('-') => {
                return Err(ParseError::Usage(format!(
                    "Unknown option '{s}'.\n{}",
                    usage_text("openras")
                )));
            }
            s => positionals.push(PathBuf::from(s)),
        }
        i += 1;
    }

    let mode_count = [extract, view, add].iter().filter(|&&b| b).count();
    if mode_count != 1 {
        return Err(ParseError::Usage(format!(
            "Specify exactly one of -x, -v, -a, or -m.\n{}",
            usage_text("openras")
        )));
    }

    if compress && !add {
        return Err(ParseError::Usage(
            "-c requires -a or -m.\n".to_string() + &usage_text("openras"),
        ));
    }
    if exclude_source_path && !add {
        return Err(ParseError::Usage(
            "-p requires -a or -m.\n".to_string() + &usage_text("openras"),
        ));
    }

    if view {
        let archive = positionals
            .first()
            .cloned()
            .ok_or_else(|| ParseError::Usage("Missing archive for -v.\n".into()))?;
        return Ok(RasMakerCmd::View { archive });
    }
    if extract {
        let archive = positionals
            .first()
            .cloned()
            .ok_or_else(|| ParseError::Usage("Missing archive for -x.\n".into()))?;
        return Ok(RasMakerCmd::Extract { archive });
    }

    // add / move
    if positionals.len() < 2 {
        return Err(ParseError::Usage(
            "Missing source and destination for -a/-m.\n".into(),
        ));
    }
    let source = positionals[0].clone();
    let dest = positionals[1].clone();
    if !exclude_source_path && source.is_absolute() {
        return Err(ParseError::Usage(
            "Error: Source path must be relative when \"-p\" option is not used.\n".into(),
        ));
    }
    Ok(RasMakerCmd::Add {
        source,
        dest,
        compress,
        exclude_source_path,
        delete_source: move_files,
    })
}

pub fn usage_text(program: &str) -> String {
    format!(
        "Usage: {program} [options] [source] [destination]\n\
         \n\
           -x\textract files\n\
           -v\tview archive contents\n\
           -a\tadd files\n\
           -m\tmove files (deletes from source)\n\
           -c\tcompress all files (also requires -a or -m)\n\
           -p\texclude source path (also requires -a or -m)\n\
         \n\
         Examples:\n\
           {program} -v data.ras\n\
           {program} -x data.ras\n\
           {program} -a data data.ras\n\
           {program} -a -p temp data.ras\n\
           {program} -a -c -p temp data.ras\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn parse_view() {
        let c = parse_rasmaker(&args(&["-v", "mod.mpm"])).unwrap();
        assert_eq!(
            c,
            RasMakerCmd::View {
                archive: PathBuf::from("mod.mpm")
            }
        );
    }

    #[test]
    fn parse_extract() {
        let c = parse_rasmaker(&args(&["-x", "data.ras"])).unwrap();
        assert_eq!(
            c,
            RasMakerCmd::Extract {
                archive: PathBuf::from("data.ras")
            }
        );
    }

    #[test]
    fn parse_add_keep_path() {
        let c = parse_rasmaker(&args(&["-a", "data", "data.ras"])).unwrap();
        assert_eq!(
            c,
            RasMakerCmd::Add {
                source: PathBuf::from("data"),
                dest: PathBuf::from("data.ras"),
                compress: false,
                exclude_source_path: false,
                delete_source: false,
            }
        );
    }

    #[test]
    fn parse_add_exclude_compress_move() {
        let c = parse_rasmaker(&args(&["-m", "-p", "-c", "temp", "out.ras"])).unwrap();
        assert_eq!(
            c,
            RasMakerCmd::Add {
                source: PathBuf::from("temp"),
                dest: PathBuf::from("out.ras"),
                compress: true,
                exclude_source_path: true,
                delete_source: true,
            }
        );
    }

    #[test]
    fn reject_absolute_without_p() {
        let err = parse_rasmaker(&args(&["-a", "/tmp/x", "out.ras"])).unwrap_err();
        assert!(err.to_string().contains("must be relative"));
    }

    #[test]
    fn looks_like_detects_flags() {
        assert!(looks_like_rasmaker(&args(&["-v", "a.ras"])));
        assert!(!looks_like_rasmaker(&args(&["list", "a.ras"])));
    }
}
