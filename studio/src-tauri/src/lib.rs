use maxfx_patcher::{
    apply_dropin_105, classify_patch_file, sha256_file, PatchKind, PATCH_105_SHA256,
    PATCH_105_URL_HTTPS,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::State;

struct GameProc(Mutex<Option<Child>>);

#[derive(Clone, Serialize)]
struct Paths {
    root: String,
    out: String,
    sample: String,
    game_default: String,
    install_cd: String,
    bootstrap: String,
    fixpack_guide_url: String,
    patch_105_url: String,
}

#[derive(Clone, Serialize)]
struct GameStatus {
    game_dir: String,
    has_exe: bool,
    /// Non-English language pack stem (`italian`, `french`, …) if `x_<lang>.ras` present.
    localized_lang: Option<String>,
    safedisc: bool,
    has_patch_105_installer: bool,
    has_d3d8: bool,
    has_d3d9: bool,
    has_launcher: bool,
    has_fixpack_marker: bool,
    has_hires_data: bool,
    has_asi_loader: bool,
    has_mod_data: bool,
    graphics_stack_ok: bool,
    summary: String,
    next_step: String,
}

const FIXPACK_GUIDE_URL: &str = "https://steamcommunity.com/sharedfiles/filedetails/?id=1633394421";

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn looks_like_root(p: &Path) -> bool {
    p.join("openmaxfx").is_file()
        && p.join("runtime").join("install-from-cd.sh").is_file()
        && (p.join("crates").join("openras").is_dir() || p.join("tools").join("openras").is_dir())
}

fn discover_root() -> Result<PathBuf, String> {
    if let Ok(env) = std::env::var("OPEN_MAX_PAYNE_ROOT") {
        let p = PathBuf::from(env);
        if looks_like_root(&p) {
            return Ok(p);
        }
    }
    // src-tauri → studio → repo
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for cand in [
        manifest.join("../.."),
        manifest.join("../../.."),
        std::env::current_dir().unwrap_or_default(),
    ] {
        if let Ok(c) = cand.canonicalize() {
            if looks_like_root(&c) {
                return Ok(c);
            }
        }
    }
    Err("Cannot find Open Max Payne repo root. Set OPEN_MAX_PAYNE_ROOT.".into())
}

fn paths_now() -> Result<Paths, String> {
    let root = discover_root()?;
    let sample = root.join("vendor/fixtures/Basic Room Example Mod.mpm");
    Ok(Paths {
        root: root.display().to_string(),
        out: root.join("easy-output").display().to_string(),
        sample: if sample.is_file() {
            sample.display().to_string()
        } else {
            String::new()
        },
        game_default: home_dir().join("Games/MaxPayne1").display().to_string(),
        install_cd: root
            .join("runtime/install-from-cd.sh")
            .display()
            .to_string(),
        bootstrap: root
            .join("runtime/bootstrap-linux.sh")
            .display()
            .to_string(),
        fixpack_guide_url: FIXPACK_GUIDE_URL.to_string(),
        patch_105_url: PATCH_105_URL_HTTPS.to_string(),
    })
}

fn pe_has_safedisc_stub(exe: &Path) -> bool {
    std::fs::read(exe)
        .map(|d| maxfx_patcher::has_safedisc_stub(&d))
        .unwrap_or(false)
}

fn d3d9_looks_like_thirteenag(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    meta.len() < 200_000
}

fn game_status_inner(dest: &Path) -> GameStatus {
    let exe = dest.join("MaxPayne.exe");
    let has_exe = exe.is_file();
    let localized_lang = maxfx_play::detect_localized_lang(dest);
    let localized = localized_lang.is_some();
    let safedisc = has_exe && pe_has_safedisc_stub(&exe);
    let has_patch_105_installer = dest.join("maxpayne1-05patch.exe").is_file();
    let has_d3d8 = dest.join("d3d8.dll").is_file();
    let has_d3d9 = dest.join("d3d9.dll").is_file();
    let has_launcher = dest.join("launch-openmaxfx.sh").is_file();
    let has_hires_data = dest.join("data").is_dir()
        && !dest.join("data").is_symlink()
        && !dest.join("data/globalconfig.txt").is_file();
    let has_mod_data = dest.join("data/globalconfig.txt").is_file()
        || (dest.join("data").is_symlink() && dest.join("data/globalconfig.txt").is_file());
    let asi = dest.join("dinput.dll");
    let has_asi_loader = asi.is_file()
        && std::fs::metadata(&asi)
            .map(|m| m.len() > 500_000)
            .unwrap_or(false);
    let has_fixpack_marker = dest.join("d3d8.dll").is_file()
        && (dest.join("scripts/MaxPayne.WidescreenFix.asi").is_file()
            || dest.join("dsound.dll").is_file()
            || dest.join("dgVoodoo.conf").is_file()
            || dest.join("d3d9.ini").is_file())
        && !safedisc;

    let thirteenag = has_d3d9 && d3d9_looks_like_thirteenag(&dest.join("d3d9.dll"));
    let graphics_stack_ok = has_exe
        && !safedisc
        && has_d3d8
        && has_d3d9
        && !thirteenag
        && !(localized && has_hires_data)
        && !(localized && has_asi_loader);

    let (summary, next_step) = if !has_exe {
        (
            "No MaxPayne.exe — install from CD first.".into(),
            "Install Max Payne (CD), then Apply localized-safe stack.".into(),
        )
    } else if safedisc {
        (
            "SafeDisc stub still on MaxPayne.exe (stxt*).".into(),
            "Download & apply 1.05 EXE, then Apply localized-safe stack.".into(),
        )
    } else if localized && (has_hires_data || has_asi_loader || thirteenag) {
        let lang = localized_lang.as_deref().unwrap_or("?");
        (
            format!(
                "Localized pack (x_{lang}.ras): FixPack hi-res / ASI / ThirteenAG d3d9 often crash under Wine."
            ),
            "Click Apply localized-safe stack, then Launch.".into(),
        )
    } else if !has_d3d8 || !has_d3d9 {
        (
            "Missing d3d8to9 / DXVK helpers.".into(),
            "Apply localized-safe stack (or Graphics helpers only).".into(),
        )
    } else if graphics_stack_ok {
        (
            if has_mod_data {
                "Graphics stack OK; loose mod data/ present.".into()
            } else if let Some(ref lang) = localized_lang {
                format!("Graphics stack OK (localized: {lang}).")
            } else {
                "Graphics stack OK.".into()
            },
            "Launch game.".into(),
        )
    } else {
        (
            "Playable-ish; run localized-safe stack to normalize wrappers.".into(),
            "Apply localized-safe stack → Launch.".into(),
        )
    };

    GameStatus {
        game_dir: dest.display().to_string(),
        has_exe,
        localized_lang,
        safedisc,
        has_patch_105_installer,
        has_d3d8,
        has_d3d9,
        has_launcher,
        has_fixpack_marker,
        has_hires_data,
        has_asi_loader,
        has_mod_data,
        graphics_stack_ok,
        summary,
        next_step,
    }
}

/// Wine `explorer` treats args starting with `/` as switches. Never pass a Unix
/// absolute path there — use a basename (cwd = game) or `Z:\...`.
fn wine_program_arg(game: &Path, program: &Path) -> Result<String, String> {
    let game_abs = game.canonicalize().unwrap_or_else(|_| game.to_path_buf());
    let prog_abs = program
        .canonicalize()
        .map_err(|e| format!("Program not found ({}): {e}", program.display()))?;
    if prog_abs.starts_with(&game_abs) {
        if let Some(name) = prog_abs.file_name() {
            return Ok(name.to_string_lossy().into_owned());
        }
    }
    Ok(format!(
        "Z:{}",
        prog_abs.to_string_lossy().replace('/', "\\")
    ))
}

fn spawn_wine_desktop(
    game: &Path,
    desktop: &str,
    size: &str,
    program: &Path,
) -> Result<String, String> {
    if !command_exists("wine") {
        return Err("wine not found — install wine-staging".into());
    }
    let prog_arg = wine_program_arg(game, program)?;
    // Guard: never launch MaxPayne.exe from patch/fixpack helpers
    let lower = prog_arg.to_ascii_lowercase();
    if lower == "maxpayne.exe" || lower.ends_with("\\maxpayne.exe") {
        return Err(
            "Refusing to start MaxPayne.exe here — this action is for the patch/FixPack installer only."
                .into(),
        );
    }

    let log = game.join("openmaxfx-launch.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|e| e.to_string())?;
    let log_err = log_file.try_clone().map_err(|e| e.to_string())?;
    let desk = format!("/desktop={desktop},{size}");
    // Basename or Z:\… — never a Unix path starting with `/` (Wine treats those as switches).
    Command::new("wine")
        .args(["explorer", &desk, &prog_arg])
        .current_dir(game)
        .env("WINEPREFIX", game.join(".wine"))
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Started Wine desktop {desktop} → {prog_arg} (file {}). Log: {}",
        program.display(),
        log.display()
    ))
}

fn command_exists(name: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {name} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_capture(mut cmd: Command) -> Result<String, String> {
    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn failed: {e}"))?;
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    if !out.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    if !out.status.success() {
        return Err(format!(
            "exit {}: {}",
            out.status.code().unwrap_or(-1),
            text.trim()
        ));
    }
    Ok(text)
}

#[tauri::command]
fn get_paths() -> Result<Paths, String> {
    paths_now()
}

#[tauri::command]
fn list_archive(path: String) -> Result<String, String> {
    let arc = openras::RasArchive::read(Path::new(&path)).map_err(|e| e.to_string())?;
    Ok(arc.list_text(&path))
}

#[tauri::command]
fn extract_archive(path: String) -> Result<String, String> {
    let root = discover_root()?;
    let out = root.join("easy-output");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let stem = Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("extract");
    let dest = out.join(stem);
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let arc = openras::RasArchive::read(Path::new(&path)).map_err(|e| e.to_string())?;
    arc.extract(&dest).map_err(|e| e.to_string())?;
    Ok(format!(
        "Extracted {} files -> {}\n→ {}",
        arc.files.len(),
        dest.display(),
        dest.display()
    ))
}

#[tauri::command]
fn pack_archive(folder: String, out: String) -> Result<String, String> {
    let arc = openras::RasArchive::from_directory(Path::new(&folder), 0x12345678, true)
        .map_err(|e| e.to_string())?;
    arc.write(Path::new(&out), false)
        .map_err(|e| e.to_string())?;
    Ok(format!("Wrote {out} ({} files)", arc.files.len()))
}

#[tauri::command]
fn find_cd() -> Result<String, String> {
    let mut cmd = Command::new("bash");
    cmd.args([
        "-lc",
        r#"
set -e
while IFS= read -r m; do
  [ -z "$m" ] && continue
  if [ -d "$m/Disk1" ] || [ -f "$m/Disk1/data1.cab" ]; then
    echo "$m"
    exit 0
  fi
done < <(findmnt -n -t iso9660,udf -o TARGET 2>/dev/null || true)
exit 1
"#,
    ]);
    run_capture(cmd).map(|s| s.trim().to_string())
}

#[tauri::command]
fn install_from_cd(cd: String, dest: String) -> Result<String, String> {
    let root = discover_root()?;
    let script = root.join("runtime/install-from-cd.sh");
    let mut cmd = Command::new("bash");
    cmd.arg(&script);
    if !cd.trim().is_empty() {
        cmd.arg(cd.trim());
        cmd.arg(&dest);
    } else {
        cmd.env("OPENMAXFX_INSTALL_DEST", &dest);
    }
    run_capture(cmd)
}

#[tauri::command]
fn setup_game(dest: String) -> Result<String, String> {
    let root = discover_root()?;
    let script = root.join("runtime/bootstrap-linux.sh");
    let mut cmd = Command::new("bash");
    cmd.arg(&script).arg(&dest);
    run_capture(cmd)
}

#[tauri::command]
fn game_status(dest: String) -> Result<GameStatus, String> {
    let dir = PathBuf::from(&dest);
    if !dir.is_dir() {
        return Err(format!("Not a folder: {dest}"));
    }
    Ok(game_status_inner(&dir))
}

#[tauri::command]
fn ensure_patch_105(dest: String) -> Result<String, String> {
    let dir = PathBuf::from(&dest);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let patch = dir.join("maxpayne1-05patch.exe");
    if patch.is_file() {
        match classify_patch_file(&patch) {
            Ok(PatchKind::DropIn105) => {
                return Ok(format!(
                    "Already present + SHA-256 OK: {} ({PATCH_105_SHA256})",
                    patch.display()
                ));
            }
            Err(e) => {
                return Err(format!(
                    "Existing {} rejected: {e}. Delete it and retry, or place a known drop-in (sha256={PATCH_105_SHA256}).",
                    patch.display()
                ));
            }
        }
    }
    if !command_exists("curl") {
        return Err(format!(
            "curl missing — place maxpayne1-05patch.exe manually (sha256={PATCH_105_SHA256})"
        ));
    }
    // HTTPS only
    if !PATCH_105_URL_HTTPS.starts_with("https://") {
        return Err("Refusing non-HTTPS patch URL".into());
    }
    let status = Command::new("curl")
        .args(["-fL", "--retry", "3", "-o"])
        .arg(&patch)
        .arg(PATCH_105_URL_HTTPS)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        let _ = std::fs::remove_file(&patch);
        return Err(format!("HTTPS download failed (exit {:?})", status.code()));
    }
    let got = sha256_file(&patch).map_err(|e| e.to_string())?;
    if got != PATCH_105_SHA256 {
        let _ = std::fs::remove_file(&patch);
        return Err(format!(
            "Downloaded file SHA-256 mismatch (got {got}, expected {PATCH_105_SHA256})"
        ));
    }
    classify_patch_file(&patch).map_err(|e| e.to_string())?;
    Ok(format!("Downloaded + verified {}", patch.display()))
}

#[tauri::command]
fn run_patch_105(dest: String) -> Result<String, String> {
    let dir = PathBuf::from(&dest);
    ensure_patch_105(dest)?;
    let patch = dir.join("maxpayne1-05patch.exe");
    apply_dropin_105(&dir, &patch).map_err(|e| e.to_string())
}

#[tauri::command]
fn run_fixpack_installer(dest: String, installer: String) -> Result<String, String> {
    let dir = PathBuf::from(&dest);
    let inst = PathBuf::from(&installer);
    if !inst.is_file() {
        return Err(format!("Installer not found: {installer}"));
    }
    // Never launch a Max Payne drop-in / game EXE as “FixPack”
    if classify_patch_file(&inst).is_ok() || pe_has_safedisc_stub(&inst) {
        return Err(
            "Refusing: that file looks like MaxPayne.exe / 1.05 drop-in, not FixPack".into(),
        );
    }
    let name = inst
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name == "maxpayne.exe" || name.contains("1-05") || name.contains("105") {
        return Err("Refusing game/patch-shaped filename for FixPack runner".into());
    }
    let local = dir.join(inst.file_name().unwrap_or_default());
    if local != inst {
        let _ = std::fs::copy(&inst, &local);
    }
    let prog = if local.is_file() { local } else { inst };
    spawn_wine_desktop(&dir, "FixPack", "1280x720", &prog)
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    for opener in ["xdg-open", "open"] {
        if Command::new(opener)
            .arg(&url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }
    Err(format!("Could not open URL: {url}"))
}

#[tauri::command]
fn apply_localized_safe_stack(dest: String) -> Result<String, String> {
    let root = discover_root()?;
    let overlays = root.join("runtime/overlays");
    maxfx_play::apply_localized_safe_stack(Path::new(&dest), &overlays).map_err(|e| e.to_string())
}

fn resolve_maxfx_engine(root: &Path) -> Option<PathBuf> {
    for rel in [
        "target/release/maxfx-engine",
        "target/debug/maxfx-engine",
        "crates/maxfx-engine/target/release/maxfx-engine",
        "crates/maxfx-engine/target/debug/maxfx-engine",
    ] {
        let p = root.join(rel);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

#[tauri::command]
fn launch_game(
    dest: String,
    prefer_native: Option<bool>,
    state: State<'_, GameProc>,
) -> Result<String, String> {
    let dir = PathBuf::from(&dest);
    {
        let mut slot = state.0.lock().map_err(|e| e.to_string())?;
        if let Some(child) = slot.as_mut() {
            match child.try_wait() {
                Ok(None) => {
                    return Err("Game already running — use Stop first".into());
                }
                Ok(Some(_)) => {
                    *slot = None;
                }
                Err(_) => {
                    *slot = None;
                }
            }
        }
    }

    let prefer_native = prefer_native.unwrap_or(false);
    if prefer_native {
        let root = discover_root()?;
        let engine = resolve_maxfx_engine(&root).ok_or_else(|| {
            "maxfx-engine binary not found — build with: cargo build -p maxfx-engine".to_string()
        })?;
        std::env::set_var("OPENMAXFX_PLAY", "native");
        std::env::set_var("OPENMAXFX_ENGINE", &engine);
        std::env::set_var("OPENMAXFX_ENGINE_WINDOW", "1");
        let probe = dir.join("data.disabled-hires/database/levels/front_end/Tutorial.ldb");
        if probe.is_file() {
            std::env::set_var("OPENMAXFX_ENGINE_PROBE", &probe);
        }
    } else {
        std::env::remove_var("OPENMAXFX_PLAY");
        std::env::remove_var("OPENMAXFX_ENGINE_WINDOW");
        if maxfx_play::detect_localized_lang(&dir).is_some() {
            let st = game_status_inner(&dir);
            if !st.graphics_stack_ok {
                apply_localized_safe_stack(dest.clone())?;
            }
        }
        maxfx_play::graphics_dlls_ok(&dir).map_err(|e| e.to_string())?;
    }

    let child = maxfx_play::spawn_game(&dir).map_err(|e| e.to_string())?;
    let log = dir.join("openmaxfx-launch.log");
    let mut slot = state.0.lock().map_err(|e| e.to_string())?;
    *slot = Some(child);
    Ok(if prefer_native {
        format!(
            "Launched native maxfx-engine preview (Wine still available if unchecked). Log {}",
            log.display()
        )
    } else {
        format!(
            "Launched MaxPayne (Wine win32 prefix pfx/ or native EXE). Watch {}",
            log.display()
        )
    })
}

#[tauri::command]
fn stop_game(state: State<'_, GameProc>) -> Result<String, String> {
    let mut slot = state.0.lock().map_err(|e| e.to_string())?;
    match slot.take() {
        Some(mut child) => {
            let _ = child.kill();
            let _ = child.wait();
            Ok("Stopped game process".into())
        }
        None => Ok("No tracked game process".into()),
    }
}

#[tauri::command]
fn game_running(state: State<'_, GameProc>) -> Result<bool, String> {
    let mut slot = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(child) = slot.as_mut() {
        match child.try_wait() {
            Ok(None) => Ok(true),
            Ok(Some(_)) => {
                *slot = None;
                Ok(false)
            }
            Err(_) => {
                *slot = None;
                Ok(false)
            }
        }
    } else {
        Ok(false)
    }
}

#[tauri::command]
fn run_tests() -> Result<String, String> {
    let root = discover_root()?;
    let mut cmd = Command::new("cargo");
    cmd.args([
        "test",
        "-p",
        "openras",
        "-p",
        "maxfx-patcher",
        "-p",
        "maxfx-play",
        "--",
        "--quiet",
    ]);
    cmd.current_dir(&root);
    run_capture(cmd)
}

#[tauri::command]
fn open_in_file_manager(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    let target = if p.is_file() {
        p.parent().unwrap_or(p.as_path()).to_path_buf()
    } else {
        p
    };
    std::fs::create_dir_all(&target).ok();
    for opener in ["xdg-open", "open"] {
        if Command::new(opener)
            .arg(&target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("No file manager opener found".into())
}

#[tauri::command]
fn inspect_maxed_level(path: String) -> Result<String, String> {
    let p = PathBuf::from(&path);
    let data = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let scene = maxfx_engine::scene::parse_ldb_scene(&data).ok_or_else(|| {
        "Could not parse LDB scene (want RAS 0x121c + ImportLevel 0x20)".to_string()
    })?;
    let mut out = format!(
        "MaxED Level: {} ({} bytes, consumed {:#x}/{:#x})\nRooms: {} | Static meshes: {} | Dynamic meshes: {} | Portals: {}\nTextures: {} | Materials: {} | Lightmaps: {} | Jumppoints: {} | FSMs: {} | Characters: {} | Triggers: {} | Items: {} | Point lights: {}\n\nRooms:\n",
        p.display(),
        data.len(),
        scene.bytes_consumed,
        data.len(),
        scene.rooms.len(),
        scene.static_meshes.len(),
        scene.dynamic_meshes.len(),
        scene.exits.len(),
        scene.textures.len(),
        scene.materials.len(),
        scene.lightmaps.len(),
        scene.jumppoints.len(),
        scene.fsms.len(),
        scene.characters.len(),
        scene.triggers.len(),
        scene.items.len(),
        scene.point_lights.len(),
    );
    for r in &scene.rooms {
        out.push_str(&format!(
            "  [{:2}] {:18} origin=({:6.1},{:5.1},{:6.1}) meshes={:?} exits={} DOs={}\n",
            r.id,
            r.name,
            r.world_origin[0],
            r.world_origin[1],
            r.world_origin[2],
            r.static_meshes,
            r.exits.len(),
            r.dynamic_meshes.len()
        ));
    }
    out.push_str("\nJumppoints:\n");
    for j in scene.jumppoints.iter().take(12) {
        out.push_str(&format!(
            "  {} (room {}) pos=({:.1},{:.1},{:.1})\n",
            j.name, j.room_id, j.pos[0], j.pos[1], j.pos[2]
        ));
    }
    if !scene.characters.is_empty() {
        out.push_str("\nCharacters:\n");
        for c in &scene.characters {
            out.push_str(&format!(
                "  {} skin={} (room {}) pos=({:.1},{:.1},{:.1})\n",
                c.name, c.skin, c.room_id, c.pos[0], c.pos[1], c.pos[2]
            ));
        }
    }
    Ok(out)
}

#[tauri::command]
fn launch_3d_level(path: String, state: State<'_, GameProc>) -> Result<String, String> {
    let root = discover_root()?;
    let engine = resolve_maxfx_engine(&root).ok_or_else(|| {
        "maxfx-engine binary not found — build with: cargo build -p maxfx-engine".to_string()
    })?;
    let p = PathBuf::from(&path);
    if !p.is_file() {
        return Err(format!("Level file not found: {}", p.display()));
    }
    let child = Command::new(&engine)
        .args(["--window", &path])
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Ok(mut slot) = state.0.lock() {
        *slot = Some(child);
    }
    Ok(format!(
        "Launched 3D walkthrough on {} (WASD + Mouse/Arrows, E interact, Tab next spawn, G fly/walk)",
        p.display()
    ))
}

#[tauri::command]
fn inspect_particlefx(path: String) -> Result<String, String> {
    let p = PathBuf::from(&path);
    let data = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
    if ext.eq_ignore_ascii_case("pse") {
        let h = openpfx::parse_pse(&data).map_err(|e| e.to_string())?;
        let emitters = openpfx::scan_pse_emitters(&data);
        let curves = openpfx::scan_pse_curves(&data);
        let stems = openpfx::scan_pse_bitmap_stems(&data);
        let mut out = format!(
            "ParticleFX PSE: {} ({} bytes)\nHeader: word0={} word1={} word2={} word3={} word4={} float_a={} float_b={}\nEmitters ({}): {:?}\nCurves ({}): {:?}\nBitmap stems ({}): {:?}\n",
            p.display(),
            data.len(),
            h.word0,
            h.word1,
            h.word2,
            h.word3,
            h.word4,
            h.float_a,
            h.float_b,
            emitters.len(),
            emitters.iter().map(|e| &e.name).collect::<Vec<_>>(),
            curves.len(),
            curves.iter().take(12).map(|c| &c.name).collect::<Vec<_>>(),
            stems.len(),
            stems.iter().take(8).collect::<Vec<_>>(),
        );
        for pth in h.ascii_paths.iter().take(8) {
            out.push_str(&format!("  path: {pth}\n"));
        }
        Ok(out)
    } else {
        let rps = openpfx::parse_rps(&data).map_err(|e| e.to_string())?;
        let gate = openpfx::validate_rps_game_gates(&data);
        let blobs = rps
            .values
            .iter()
            .filter(|v| matches!(v, openpfx::RpsValue::Blob { .. }))
            .count();
        Ok(format!(
            "ParticleFX RPS: {} ({} bytes, consumed {}/{})\nValues: {} | Embedded frame blobs: {} | PS_ParticleSystem gate: {}\n",
            p.display(),
            data.len(),
            rps.bytes_consumed,
            data.len(),
            rps.values.len(),
            blobs,
            match gate {
                Ok(()) => "OK (0x60 / ver 5)".to_string(),
                Err(e) => format!("FAIL ({e})"),
            }
        ))
    }
}

#[tauri::command]
fn export_particlefx_rps(
    pse_path: String,
    bitmap_root: String,
    out_path: String,
) -> Result<String, String> {
    let pse = PathBuf::from(&pse_path);
    let data = std::fs::read(&pse).map_err(|e| format!("{}: {e}", pse.display()))?;
    let h = openpfx::parse_pse(&data).map_err(|e| e.to_string())?;
    let stubs: Vec<String> = openpfx::scan_pse_emitters(&data)
        .into_iter()
        .map(|e| e.name)
        .collect();
    let sampled = openpfx::sample_emitter_curves_from_pse(&data, 32).map_err(|e| e.to_string())?;
    let texture_sequences = if !bitmap_root.trim().is_empty() {
        openpfx::build_texture_sequences_from_pse(&data, Path::new(&bitmap_root), stubs.len())
    } else {
        Vec::new()
    };
    let bytes = openpfx::pse_to_rps_export_with_curves(
        &h,
        &openpfx::PseExportOptions {
            empty_graph: false,
            emitter_stubs: stubs.clone(),
            texture_stubs: Vec::new(),
            texture_embeds: Vec::new(),
            texture_sequences,
        },
        &sampled,
    )
    .map_err(|e| e.to_string())?;
    openpfx::validate_rps_game_gates(&bytes).map_err(|e| e.to_string())?;
    std::fs::write(&out_path, &bytes).map_err(|e| e.to_string())?;
    Ok(format!(
        "Exported {} ({} bytes, {} emitters, PS_ParticleSystem gate OK)",
        out_path,
        bytes.len(),
        stubs.len()
    ))
}

#[tauri::command]
fn inspect_actorfx(path: String) -> Result<String, String> {
    let p = PathBuf::from(&path);
    let data = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let sum = maxfx_engine::kf2::parse_kf2_or_skd(&data)
        .ok_or_else(|| "Not a valid 0x0c chunked KF2/SKD file".to_string())?;
    let mut out = format!(
        "ActorFX KF2/SKD: {} ({} bytes)\nTop-level chunks: {} | Total vec3 vertices/keys: {}\nBones / Materials / Nodes ({}): {:?}\n\nChunks:\n",
        p.display(),
        sum.file_size,
        sum.chunks.len(),
        sum.total_vec3,
        sum.all_strings.len(),
        &sum.all_strings[..sum.all_strings.len().min(24)]
    );
    for c in &sum.chunks {
        out.push_str(&format!(
            "  @{:#08x} id={:#010x} ver={} size={} vec3={} strings={:?}\n",
            c.offset,
            c.id,
            c.version,
            c.size,
            c.vec3_count,
            &c.strings[..c.strings.len().min(8)]
        ));
    }
    Ok(out)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {}))
        .manage(GameProc(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            get_paths,
            list_archive,
            extract_archive,
            pack_archive,
            find_cd,
            install_from_cd,
            setup_game,
            game_status,
            ensure_patch_105,
            run_patch_105,
            run_fixpack_installer,
            apply_localized_safe_stack,
            open_url,
            launch_game,
            stop_game,
            game_running,
            run_tests,
            open_in_file_manager,
            inspect_maxed_level,
            launch_3d_level,
            inspect_particlefx,
            export_particlefx_rps,
            inspect_actorfx,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Open Max Payne Studio");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_root_and_paths_succeed() {
        let root = discover_root().expect("discover_root must locate repo root");
        assert!(looks_like_root(&root));
        let paths = paths_now().expect("paths_now must succeed");
        assert!(!paths.root.is_empty());
        assert!(!paths.game_default.is_empty());
    }
}
