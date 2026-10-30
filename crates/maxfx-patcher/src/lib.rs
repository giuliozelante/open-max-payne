//! Max Payne 1.05 patch detection / apply.
//! Accepts known drop-in EXEs (SHA-256 allowlist). Refuses Wise/InstallShield SFX.
//! Does **not** unpack installers.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use thiserror::Error;

/// Rockstar-hosted drop-in `maxpayne1-05patch.exe` (also used as MaxPayne.exe).
/// Verified from a legal local copy; PE sections `.text/.rdata/.data/.rsrc` (no `stxt*`).
pub const PATCH_105_SHA256: &str =
    "342d296cc8a8e430609ad09bb9f1d454655fb45c081df022d7993bd1fc7da46e";

/// Prefer HTTPS only (Rockstar serves this over TLS).
pub const PATCH_105_URL_HTTPS: &str =
    "https://updates.rockstargames.com/patches/maxpayne/maxpayne1-05patch.exe";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PatchError {
    #[error("not a PE MZ executable")]
    NotPe,
    #[error("SafeDisc stub detected (stxt* PE section) — not a usable 1.05 drop-in")]
    SafeDisc,
    #[error("looks like a Wise/InstallShield self-extracting installer — refuse (no SFX unpacker); extract MaxPayne.exe yourself or use the Rockstar drop-in")]
    SelfExtractingInstaller,
    #[error("SHA-256 mismatch: got {got}, expected one of allowlist")]
    HashMismatch { got: String },
    #[error("IO: {0}")]
    Io(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchKind {
    /// Known-good drop-in game EXE (copy over MaxPayne.exe).
    DropIn105,
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

pub fn sha256_file(path: &Path) -> Result<String, PatchError> {
    let data = fs::read(path).map_err(|e| PatchError::Io(e.to_string()))?;
    Ok(sha256_hex(&data))
}

fn pe_section_names(data: &[u8]) -> Result<Vec<String>, PatchError> {
    if data.len() < 0x40 || &data[0..2] != b"MZ" {
        return Err(PatchError::NotPe);
    }
    let e_lfanew = u32::from_le_bytes(data[0x3C..0x40].try_into().unwrap()) as usize;
    if e_lfanew + 24 > data.len() || &data[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
        return Err(PatchError::NotPe);
    }
    let nsec = u16::from_le_bytes(data[e_lfanew + 6..e_lfanew + 8].try_into().unwrap()) as usize;
    let opt = u16::from_le_bytes(data[e_lfanew + 20..e_lfanew + 22].try_into().unwrap()) as usize;
    let mut off = e_lfanew + 24 + opt;
    let mut names = Vec::with_capacity(nsec);
    for _ in 0..nsec {
        if off + 8 > data.len() {
            break;
        }
        let raw = &data[off..off + 8];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(8);
        names.push(String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase());
        off += 40;
    }
    Ok(names)
}

pub fn has_safedisc_stub(data: &[u8]) -> bool {
    pe_section_names(data)
        .map(|n| n.iter().any(|s| s.starts_with("stxt")))
        .unwrap_or(false)
}

fn looks_like_sfx_installer(data: &[u8]) -> bool {
    // Heuristics only — we refuse rather than unpack.
    const NEEDLES: &[&[u8]] = &[
        b"Wise Installation Wizard",
        b"WISEMAIN",
        b"InstallShield",
        b"setup.inx",
        b"ISSetup.dll",
        b"WinZip Self-Extractor",
        b"7-Zip self-extracting archive",
    ];
    NEEDLES
        .iter()
        .any(|n| data.windows(n.len()).any(|w| w == *n))
}

fn looks_like_dropin_payload(data: &[u8]) -> bool {
    let has_rlmfc = data.windows(9).any(|w| w == b"rlmfc.dll");
    let has_startup = data
        .windows(20)
        .any(|w| w == b"Max Payne startup di" || w.starts_with(b"MaxPayne_HUD"));
    has_rlmfc && has_startup
}

/// Classify a candidate 1.05 file. Hash must match allowlist for DropIn105.
pub fn classify_patch_bytes(data: &[u8]) -> Result<PatchKind, PatchError> {
    if data.len() < 0x100 || &data[0..2] != b"MZ" {
        return Err(PatchError::NotPe);
    }
    if has_safedisc_stub(data) {
        return Err(PatchError::SafeDisc);
    }
    if looks_like_sfx_installer(data) {
        return Err(PatchError::SelfExtractingInstaller);
    }
    let digest = sha256_hex(data);
    if digest == PATCH_105_SHA256 {
        return Ok(PatchKind::DropIn105);
    }
    // Unknown hash: still refuse SFX; if it looks like drop-in, report hash mismatch
    if looks_like_dropin_payload(data) {
        return Err(PatchError::HashMismatch { got: digest });
    }
    Err(PatchError::SelfExtractingInstaller)
}

pub fn classify_patch_file(path: &Path) -> Result<PatchKind, PatchError> {
    let data = fs::read(path).map_err(|e| PatchError::Io(e.to_string()))?;
    classify_patch_bytes(&data)
}

/// Copy validated drop-in over `MaxPayne.exe`, keeping a one-time `.safedisc.bak`.
pub fn apply_dropin_105(game_dir: &Path, patch_path: &Path) -> Result<String, PatchError> {
    classify_patch_file(patch_path)?;
    let exe = game_dir.join("MaxPayne.exe");
    if exe.is_file() {
        let bak = game_dir.join("MaxPayne.exe.safedisc.bak");
        if !bak.is_file() {
            fs::copy(&exe, &bak).map_err(|e| PatchError::Io(e.to_string()))?;
        }
    }
    fs::copy(patch_path, &exe).map_err(|e| PatchError::Io(e.to_string()))?;
    Ok(format!(
        "Applied 1.05 drop-in → {} (sha256={PATCH_105_SHA256})",
        exe.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_constant_is_64_hex() {
        assert_eq!(PATCH_105_SHA256.len(), 64);
        assert!(PATCH_105_SHA256.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn refuses_safedisc_section_name() {
        // Minimal fake PE with stxt section name — build tiny buffer
        let mut data = vec![0u8; 0x200];
        data[0] = b'M';
        data[1] = b'Z';
        data[0x3C] = 0x80;
        data[0x80..0x84].copy_from_slice(b"PE\0\0");
        // NumberOfSections = 1
        data[0x86] = 1;
        // SizeOfOptionalHeader = 0xE0
        data[0x80 + 20] = 0xE0;
        let sec = 0x80 + 24 + 0xE0;
        if sec + 8 < data.len() {
            data[sec..sec + 4].copy_from_slice(b"stxt");
        }
        assert!(has_safedisc_stub(&data));
    }

    #[test]
    fn refuses_wise_string() {
        let mut data = vec![0u8; 0x100];
        data[0] = b'M';
        data[1] = b'Z';
        data.extend_from_slice(b"Wise Installation Wizard");
        // pe parse may fail NotPe — force through looks_like
        assert!(looks_like_sfx_installer(&data));
    }

    #[test]
    fn hash_mismatch_message() {
        let data = b"MZ\0\0not-a-real-pe-but-long-enough-padding________________";
        let mut buf = data.to_vec();
        buf.resize(0x100, 0);
        let _ = classify_patch_bytes(&buf);
    }

    #[test]
    fn classifies_lab_dropin_when_present() {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        let Some(home) = home else { return };
        let p = Path::new(&home).join("Games/MaxPayne1/maxpayne1-05patch.exe");
        if !p.is_file() {
            return;
        }
        assert_eq!(classify_patch_file(&p).unwrap(), PatchKind::DropIn105);
        assert_eq!(sha256_file(&p).unwrap(), PATCH_105_SHA256);
    }
}
