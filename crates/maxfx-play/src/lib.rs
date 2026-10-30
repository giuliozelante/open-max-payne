//! Play helpers: localized language detect, graphics fail-closed checks, launch.
//! Linux/macOS use Wine with an isolated win32 prefix. Windows runs the EXE natively.
//! Shell scripts are optional convenience only — Studio must call this crate.

use maxfx_patcher::{has_safedisc_stub, PatchError};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlayError {
    #[error("{0}")]
    Msg(String),
    #[error(transparent)]
    Patch(#[from] PatchError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Non-English `x_<lang>.ras` stem, or None.
pub fn detect_localized_lang(game: &Path) -> Option<String> {
    let rd = fs::read_dir(game).ok()?;
    for ent in rd.flatten() {
        let name = ent.file_name();
        let lower = name.to_string_lossy().to_ascii_lowercase();
        if !lower.starts_with("x_") || !lower.ends_with(".ras") {
            continue;
        }
        let stem = &lower[2..lower.len() - 4];
        match stem {
            "english" | "data" | "music" | "level" => continue,
            s if s.starts_with("level") && s[5..].chars().all(|c| c.is_ascii_digit()) => {
                continue;
            }
            s => return Some(s.to_string()),
        }
    }
    None
}

pub fn graphics_dlls_ok(game: &Path) -> Result<(), PlayError> {
    let d3d8 = game.join("d3d8.dll");
    let d3d9 = game.join("d3d9.dll");
    if !d3d8.is_file() {
        return Err(PlayError::Msg(
            "missing d3d8.dll (d3d8to9) — run Apply localized-safe stack / setup-game".into(),
        ));
    }
    if !d3d9.is_file() {
        return Err(PlayError::Msg(
            "missing d3d9.dll (DXVK x32) — run Apply localized-safe stack / setup-game".into(),
        ));
    }
    let sz = fs::metadata(&d3d9)?.len();
    if sz < 200_000 {
        return Err(PlayError::Msg(format!(
            "d3d9.dll is too small ({sz} bytes) — looks like ThirteenAG wrapper, not DXVK x32"
        )));
    }
    Ok(())
}

fn park_hires_and_asi(game: &Path, lang: &str) -> Result<Vec<String>, PlayError> {
    let mut notes = Vec::new();
    let old = game.join("data.disabled-italian");
    let hires = game.join("data.disabled-hires");
    if old.is_dir() && !hires.exists() {
        fs::rename(&old, &hires)?;
        notes.push("migrated data.disabled-italian → data.disabled-hires".into());
    }
    let data = game.join("data");
    if data.is_dir()
        && !data
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        && !data.join("globalconfig.txt").is_file()
    {
        if hires.exists() {
            let _ = fs::remove_dir_all(&hires);
        }
        fs::rename(&data, &hires)?;
        notes.push(format!(
            "parked hi-res data/ ({lang}) → data.disabled-hires"
        ));
    }
    let dinput = game.join("dinput.dll");
    if dinput.is_file() {
        let sz = fs::metadata(&dinput)?.len();
        if sz > 500_000 {
            let bak = game.join("dinput.dll.asi-loader.bak");
            fs::rename(&dinput, &bak)?;
            notes.push("parked ASI dinput.dll".into());
        }
    }
    Ok(notes)
}

fn copy_if_exists(src: &Path, dst: &Path) -> Result<bool, PlayError> {
    if src.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
        return Ok(true);
    }
    Ok(false)
}

/// Apply localized-safe graphics stack using overlays under `overlay_root` (repo `runtime/overlays`).
/// Fails if DXVK/d3d8to9 cannot be installed into the game dir.
pub fn apply_localized_safe_stack(game: &Path, overlay_root: &Path) -> Result<String, PlayError> {
    let mut notes = Vec::new();
    if let Some(lang) = detect_localized_lang(game) {
        notes.push(format!("localized pack: x_{lang}.ras"));
        notes.extend(park_hires_and_asi(game, &lang)?);
    } else {
        notes.push("no non-English language pack — graphics wrappers only".into());
    }

    let d3d8_src = overlay_root.join("d3d8to9/d3d8.dll");
    let d3d9_src = overlay_root.join("dxvk/x32/d3d9.dll");
    if !copy_if_exists(&d3d8_src, &game.join("d3d8.dll"))? {
        return Err(PlayError::Msg(format!(
            "missing overlay {} — run bootstrap once on Linux or place d3d8to9 manually",
            d3d8_src.display()
        )));
    }
    notes.push("installed d3d8to9 → d3d8.dll".into());

    let d3d9_dst = game.join("d3d9.dll");
    if d3d9_dst.is_file() {
        let sz = fs::metadata(&d3d9_dst)?.len();
        if sz < 200_000 {
            let bak = game.join("d3d9.dll.thirteenag.bak");
            if !bak.is_file() {
                fs::copy(&d3d9_dst, &bak)?;
            }
        }
    }
    if !copy_if_exists(&d3d9_src, &d3d9_dst)? {
        return Err(PlayError::Msg(format!(
            "missing overlay {} — run bootstrap once on Linux or place DXVK x32 manually",
            d3d9_src.display()
        )));
    }
    notes.push("installed DXVK x32 → d3d9.dll".into());
    graphics_dlls_ok(game)?;
    Ok(notes.join("\n"))
}

fn wine_prefix(game: &Path) -> PathBuf {
    game.join("pfx")
}

/// Spawn the game. Caller owns the Child (for process manager).
pub fn spawn_game(game: &Path) -> Result<Child, PlayError> {
    graphics_dlls_ok(game)?;
    let exe = game.join("MaxPayne.exe");
    if !exe.is_file() {
        return Err(PlayError::Msg("MaxPayne.exe missing".into()));
    }
    let bytes = fs::read(&exe)?;
    if has_safedisc_stub(&bytes) {
        return Err(PlayError::Msg(
            "MaxPayne.exe still has SafeDisc stub — apply 1.05 drop-in first".into(),
        ));
    }

    let log_path = game.join("openmaxfx-launch.log");
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let log_err = log.try_clone()?;

    if cfg!(windows) {
        return Command::new(&exe)
            .current_dir(game)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .spawn()
            .map_err(PlayError::from);
    }

    // Unix: Wine win32 prefix
    let prefix = wine_prefix(game);
    fs::create_dir_all(&prefix)?;
    let wine = which("wine").ok_or_else(|| {
        PlayError::Msg("wine not found — install wine-staging (32-bit capable)".into())
    })?;

    let mut cmd = Command::new(&wine);
    cmd.current_dir(game)
        .env("WINEARCH", "win32")
        .env("WINEPREFIX", &prefix)
        .env(
            "WINEDLLOVERRIDES",
            std::env::var("WINEDLLOVERRIDES")
                .unwrap_or_else(|_| "d3d8=n,b;d3d9=n,b;dsound=n,b".into()),
        )
        .env(
            "DXVK_FRAME_RATE",
            std::env::var("DXVK_FRAME_RATE").unwrap_or_else(|_| "60".into()),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));

    // Virtual desktop via explorer (basename only — never Unix absolute as wine arg)
    if std::env::var("OPENMAXFX_VIRTUAL_DESKTOP").unwrap_or_else(|_| "1".into()) != "0" {
        let desk = std::env::var("OPENMAXFX_DESKTOP").unwrap_or_else(|_| "1280x720".into());
        cmd.args([
            "explorer",
            &format!("/desktop=OpenMaxPayne,{desk}"),
            "./MaxPayne.exe",
        ]);
    } else {
        cmd.arg("./MaxPayne.exe");
    }

    cmd.spawn().map_err(PlayError::from)
}

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        for p in std::env::split_paths(&paths) {
            let cand = p.join(bin);
            if cand.is_file() {
                return Some(cand);
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_italian_not_data() {
        let dir = std::env::temp_dir().join(format!("maxfx-play-lang-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("x_data.ras"), b"x").unwrap();
        fs::write(dir.join("x_level1.ras"), b"x").unwrap();
        fs::write(dir.join("x_italian.ras"), b"x").unwrap();
        assert_eq!(detect_localized_lang(&dir).as_deref(), Some("italian"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn english_only_is_none() {
        let dir = std::env::temp_dir().join(format!("maxfx-play-en-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("x_english.ras"), b"x").unwrap();
        fs::write(dir.join("x_data.ras"), b"x").unwrap();
        assert!(detect_localized_lang(&dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn graphics_fail_closed_missing() {
        let dir = std::env::temp_dir().join(format!("maxfx-play-gfx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(graphics_dlls_ok(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
