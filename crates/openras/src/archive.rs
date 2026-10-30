//! RAS/MPM archive read/write (clean-room from docs/specs/ras_format.md).

use crate::crypto::{decrypt_with_seed, encrypt_with_seed};
use crate::lzss::{compress as lzss_compress, decompress as lzss_decompress};
use crate::pathutil::{rel_to_win_dir, validate_basename, win_dir_to_rel, PathError};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use thiserror::Error;

pub const MAGIC: &[u8; 4] = b"RAS\0";
pub const HEADER_SIZE: usize = 0x2C;
pub const VERSION_1_2: f32 = 1.2;
pub const COMPAT_MP1: u32 = 3;
pub const MODE_LZSS: u32 = 1;
pub const MODE_STORED: u32 = 3;

#[derive(Debug, Error)]
pub enum RasError {
    #[error("{0}")]
    Msg(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Path(#[from] PathError),
}

#[derive(Debug, Clone)]
pub struct RasDirEntry {
    pub name: String,
    pub timestamp: [u8; 16],
}

#[derive(Debug, Clone)]
pub struct RasFileEntry {
    pub name: String,
    pub dir_index: u32,
    pub data: Vec<u8>,
    pub compression_mode: u32,
    pub compressed_data: Option<Vec<u8>>,
    pub unknown_0: u32,
    pub unknown_1: u32,
    pub timestamp: [u8; 16],
}

impl RasFileEntry {
    pub fn uncompressed_size(&self) -> usize {
        self.data.len()
    }
}

#[derive(Debug, Clone)]
pub struct RasArchive {
    pub seed: i32,
    pub version: f32,
    pub compatibility: u32,
    pub directories: Vec<RasDirEntry>,
    pub files: Vec<RasFileEntry>,
}

impl RasArchive {
    pub fn read(path: &Path) -> Result<Self, RasError> {
        let data = fs::read(path)?;
        Self::from_bytes(&data)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, RasError> {
        if data.len() < HEADER_SIZE || &data[..4] != MAGIC.as_slice() {
            return Err(RasError::Msg("Not a RAS archive".into()));
        }
        let seed = i32::from_le_bytes(data[4..8].try_into().unwrap());
        let meta = decrypt_with_seed(&data[8..HEADER_SIZE], seed);
        let mut hdr = data[..HEADER_SIZE].to_vec();
        hdr[8..].copy_from_slice(&meta);

        let file_count = u32::from_le_bytes(hdr[8..12].try_into().unwrap());
        let dir_count = u32::from_le_bytes(hdr[12..16].try_into().unwrap());
        let file_table_size = u32::from_le_bytes(hdr[16..20].try_into().unwrap()) as usize;
        let dir_table_size = u32::from_le_bytes(hdr[20..24].try_into().unwrap()) as usize;
        let version = f32::from_bits(u32::from_le_bytes(hdr[0x18..0x1C].try_into().unwrap()));
        let header_crc = u32::from_le_bytes(hdr[0x1C..0x20].try_into().unwrap());
        let file_crc = u32::from_le_bytes(hdr[0x20..0x24].try_into().unwrap());
        let dir_crc = u32::from_le_bytes(hdr[0x24..0x28].try_into().unwrap());
        let compatibility = u32::from_le_bytes(hdr[0x28..0x2C].try_into().unwrap());

        let mut check = hdr.clone();
        check[0x1C..0x20].copy_from_slice(&0u32.to_le_bytes());
        if crc32fast::hash(&check) != header_crc {
            return Err(RasError::Msg(
                "Header CRC mismatch (archive corrupted or wrong decrypt)".into(),
            ));
        }

        let mut off = HEADER_SIZE;
        if off + file_table_size + dir_table_size > data.len() {
            return Err(RasError::Msg("Truncated archive tables".into()));
        }
        let ft = decrypt_with_seed(&data[off..off + file_table_size], seed);
        off += file_table_size;
        let dt = decrypt_with_seed(&data[off..off + dir_table_size], seed);
        off += dir_table_size;

        if crc32fast::hash(&ft) != file_crc {
            return Err(RasError::Msg("File table CRC mismatch".into()));
        }
        if crc32fast::hash(&dt) != dir_crc {
            return Err(RasError::Msg("Directory table CRC mismatch".into()));
        }

        let directories = parse_dirs(&dt, dir_count as usize)?;
        let files_meta = parse_files(&ft, file_count as usize)?;

        let mut files = Vec::with_capacity(files_meta.len());
        for meta_f in files_meta {
            if off + meta_f.compressed_size > data.len() {
                return Err(RasError::Msg(format!(
                    "Truncated payload for {}",
                    meta_f.name
                )));
            }
            let blob = data[off..off + meta_f.compressed_size].to_vec();
            off += meta_f.compressed_size;
            let mut payload = match meta_f.mode {
                MODE_STORED => blob.clone(),
                MODE_LZSS => lzss_decompress(&blob, Some(meta_f.uncompressed_size)),
                other => {
                    return Err(RasError::Msg(format!(
                        "Unsupported compression mode {other} for {}",
                        meta_f.name
                    )));
                }
            };
            payload.truncate(meta_f.uncompressed_size);
            files.push(RasFileEntry {
                name: meta_f.name,
                dir_index: meta_f.dir_index,
                data: payload,
                compression_mode: meta_f.mode,
                compressed_data: if meta_f.mode != MODE_STORED {
                    Some(blob)
                } else {
                    None
                },
                unknown_0: meta_f.unknown_0,
                unknown_1: meta_f.unknown_1,
                timestamp: meta_f.timestamp,
            });
        }

        Ok(Self {
            seed,
            version,
            compatibility,
            directories,
            files,
        })
    }

    pub fn extract(&self, dest: &Path) -> Result<(), RasError> {
        fs::create_dir_all(dest)?;
        for d in &self.directories {
            let rel = win_dir_to_rel(&d.name)?;
            if !rel.as_os_str().is_empty() {
                fs::create_dir_all(dest.join(&rel))?;
            }
        }
        for f in &self.files {
            validate_basename(&f.name)?;
            let dname = self
                .directories
                .get(f.dir_index as usize)
                .map(|d| d.name.as_str())
                .unwrap_or("\\");
            let rel_dir = win_dir_to_rel(dname)?;
            let out_dir = if rel_dir.as_os_str().is_empty() {
                dest.to_path_buf()
            } else {
                dest.join(&rel_dir)
            };
            fs::create_dir_all(&out_dir)?;
            let out_path = out_dir.join(&f.name);
            // Ensure final path stays under dest
            let canon_dest = fs::canonicalize(dest).unwrap_or_else(|_| dest.to_path_buf());
            if let Ok(parent) = fs::canonicalize(&out_dir) {
                if !parent.starts_with(&canon_dest) {
                    return Err(RasError::Msg(format!(
                        "refusing to write outside dest: {}",
                        f.name
                    )));
                }
            }
            fs::write(out_path, &f.data)?;
        }
        Ok(())
    }

    pub fn to_bytes(&self, compress: bool) -> Result<Vec<u8>, RasError> {
        let seed = self.seed;
        let mut file_table = Vec::new();
        let mut dir_table = Vec::new();
        let mut payloads = Vec::new();

        for d in &self.directories {
            dir_table.extend_from_slice(d.name.as_bytes());
            dir_table.push(0);
            dir_table.extend_from_slice(&d.timestamp);
        }

        for f in &self.files {
            validate_basename(&f.name)?;
            let (mode, blob) = if compress && f.compression_mode == MODE_STORED {
                let comp = lzss_compress(&f.data);
                if comp.len() < f.data.len() {
                    (MODE_LZSS, comp)
                } else {
                    (MODE_STORED, f.data.clone())
                }
            } else if f.compression_mode == MODE_LZSS {
                if let Some(ref c) = f.compressed_data {
                    if !compress {
                        (MODE_LZSS, c.clone())
                    } else {
                        (MODE_LZSS, lzss_compress(&f.data))
                    }
                } else {
                    (MODE_LZSS, lzss_compress(&f.data))
                }
            } else {
                (MODE_STORED, f.data.clone())
            };

            file_table.extend_from_slice(f.name.as_bytes());
            file_table.push(0);
            file_table.extend_from_slice(&(f.data.len() as u32).to_le_bytes());
            file_table.extend_from_slice(&(blob.len() as u32).to_le_bytes());
            file_table.extend_from_slice(&f.unknown_0.to_le_bytes());
            file_table.extend_from_slice(&f.dir_index.to_le_bytes());
            file_table.extend_from_slice(&f.unknown_1.to_le_bytes());
            file_table.extend_from_slice(&mode.to_le_bytes());
            file_table.extend_from_slice(&f.timestamp);
            payloads.extend_from_slice(&blob);
        }

        let file_crc = crc32fast::hash(&file_table);
        let dir_crc = crc32fast::hash(&dir_table);

        let mut hdr = vec![0u8; HEADER_SIZE];
        hdr[0..4].copy_from_slice(MAGIC);
        hdr[4..8].copy_from_slice(&seed.to_le_bytes());
        hdr[8..12].copy_from_slice(&(self.files.len() as u32).to_le_bytes());
        hdr[12..16].copy_from_slice(&(self.directories.len() as u32).to_le_bytes());
        hdr[16..20].copy_from_slice(&(file_table.len() as u32).to_le_bytes());
        hdr[20..24].copy_from_slice(&(dir_table.len() as u32).to_le_bytes());
        hdr[0x18..0x1C].copy_from_slice(&self.version.to_bits().to_le_bytes());
        hdr[0x1C..0x20].copy_from_slice(&0u32.to_le_bytes());
        hdr[0x20..0x24].copy_from_slice(&file_crc.to_le_bytes());
        hdr[0x24..0x28].copy_from_slice(&dir_crc.to_le_bytes());
        hdr[0x28..0x2C].copy_from_slice(&self.compatibility.to_le_bytes());
        let header_crc = crc32fast::hash(&hdr);
        hdr[0x1C..0x20].copy_from_slice(&header_crc.to_le_bytes());

        let enc_meta = encrypt_with_seed(&hdr[8..], seed);
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&seed.to_le_bytes());
        out.extend_from_slice(&enc_meta);
        out.extend_from_slice(&encrypt_with_seed(&file_table, seed));
        out.extend_from_slice(&encrypt_with_seed(&dir_table, seed));
        out.extend_from_slice(&payloads);
        Ok(out)
    }

    pub fn write(&self, path: &Path, compress: bool) -> Result<(), RasError> {
        fs::write(path, self.to_bytes(compress)?)?;
        Ok(())
    }

    pub fn from_directory(root: &Path, seed: i32) -> Result<Self, RasError> {
        let mut dirs: BTreeMap<String, RasDirEntry> = BTreeMap::new();
        dirs.insert(
            "\\".into(),
            RasDirEntry {
                name: "\\".into(),
                timestamp: [0; 16],
            },
        );
        let mut pending: Vec<(String, String, Vec<u8>)> = Vec::new();

        fn walk(
            root: &Path,
            dir: &Path,
            dirs: &mut BTreeMap<String, RasDirEntry>,
            pending: &mut Vec<(String, String, Vec<u8>)>,
        ) -> Result<(), RasError> {
            let rel = dir.strip_prefix(root).unwrap_or(Path::new(""));
            let win = rel_to_win_dir(rel);
            dirs.entry(win.clone()).or_insert_with(|| RasDirEntry {
                name: win.clone(),
                timestamp: [0; 16],
            });
            let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| e.file_name());
            for ent in entries {
                let ft = ent.file_type()?;
                if ft.is_dir() {
                    walk(root, &ent.path(), dirs, pending)?;
                } else if ft.is_file() {
                    let name = ent.file_name().to_string_lossy().into_owned();
                    validate_basename(&name)?;
                    pending.push((win.clone(), name, fs::read(ent.path())?));
                }
            }
            Ok(())
        }

        walk(root, root, &mut dirs, &mut pending)?;

        let dir_list: Vec<_> = dirs.into_values().collect();
        let index: BTreeMap<_, _> = dir_list
            .iter()
            .enumerate()
            .map(|(i, d)| (d.name.clone(), i as u32))
            .collect();
        let files = pending
            .into_iter()
            .map(|(win, name, data)| RasFileEntry {
                name,
                dir_index: index[&win],
                data,
                compression_mode: MODE_STORED,
                compressed_data: None,
                unknown_0: 0,
                unknown_1: 0,
                timestamp: [0; 16],
            })
            .collect();

        Ok(Self {
            seed,
            version: VERSION_1_2,
            compatibility: COMPAT_MP1,
            directories: dir_list,
            files,
        })
    }

    pub fn list_text(&self, archive_label: &str) -> String {
        let mut s = format!(
            "{archive_label}: {} files, {} dirs, ver={} compat={} seed=0x{:08x}\n",
            self.files.len(),
            self.directories.len(),
            self.version,
            self.compatibility,
            self.seed as u32
        );
        for f in &self.files {
            let d = self
                .directories
                .get(f.dir_index as usize)
                .map(|d| d.name.as_str())
                .unwrap_or("?");
            s.push_str(&format!(
                "  {d}{}  {} bytes  mode={}\n",
                f.name,
                f.uncompressed_size(),
                f.compression_mode
            ));
        }
        s
    }
}

struct FileMeta {
    name: String,
    uncompressed_size: usize,
    compressed_size: usize,
    unknown_0: u32,
    dir_index: u32,
    unknown_1: u32,
    mode: u32,
    timestamp: [u8; 16],
}

fn parse_c_string(buf: &[u8], i: usize) -> Result<(String, usize), RasError> {
    let z = buf[i..]
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| RasError::Msg("unterminated C string in table".into()))?;
    let name = String::from_utf8_lossy(&buf[i..i + z]).into_owned();
    Ok((name, i + z + 1))
}

fn parse_dirs(dt: &[u8], count: usize) -> Result<Vec<RasDirEntry>, RasError> {
    let mut out = Vec::with_capacity(count);
    let mut i = 0;
    for _ in 0..count {
        let (name, ni) = parse_c_string(dt, i)?;
        i = ni;
        if i + 16 > dt.len() {
            return Err(RasError::Msg("truncated dir timestamp".into()));
        }
        let mut ts = [0u8; 16];
        ts.copy_from_slice(&dt[i..i + 16]);
        i += 16;
        // Validate path components (allow leading \ in stored form)
        let _ = win_dir_to_rel(&name)?;
        out.push(RasDirEntry {
            name,
            timestamp: ts,
        });
    }
    Ok(out)
}

fn parse_files(ft: &[u8], count: usize) -> Result<Vec<FileMeta>, RasError> {
    let mut out = Vec::with_capacity(count);
    let mut i = 0;
    for _ in 0..count {
        let (name, ni) = parse_c_string(ft, i)?;
        i = ni;
        validate_basename(&name)?;
        if i + 24 + 16 > ft.len() {
            return Err(RasError::Msg("truncated file entry".into()));
        }
        let unc = u32::from_le_bytes(ft[i..i + 4].try_into().unwrap()) as usize;
        let comp = u32::from_le_bytes(ft[i + 4..i + 8].try_into().unwrap()) as usize;
        let u0 = u32::from_le_bytes(ft[i + 8..i + 12].try_into().unwrap());
        let di = u32::from_le_bytes(ft[i + 12..i + 16].try_into().unwrap());
        let u1 = u32::from_le_bytes(ft[i + 16..i + 20].try_into().unwrap());
        let mode = u32::from_le_bytes(ft[i + 20..i + 24].try_into().unwrap());
        i += 24;
        let mut ts = [0u8; 16];
        ts.copy_from_slice(&ft[i..i + 16]);
        i += 16;
        out.push(FileMeta {
            name,
            uncompressed_size: unc,
            compressed_size: comp,
            unknown_0: u0,
            dir_index: di,
            unknown_1: u1,
            mode,
            timestamp: ts,
        });
    }
    Ok(out)
}
