# Open Max Payne

Play **Max Payne 1** and use its **MAX-FX** toolchain on modern OSes.
You need your own retail CD / install. This project does **not** redistribute the game.

## Current baseline (honest)

| Area | Status |
|------|--------|
| **Play (Rust)** | `maxfx-play` — Wine win32 `pfx/` on Unix; native EXE on Windows; localized-safe stack |
| **1.05 patcher** | `maxfx-patcher` — HTTPS + SHA-256 allowlist; refuses SFX installers |
| **`openras` (Rust)** | **Alpha** — list / extract / pack RAS/MPM; synthetic CI tests |
| **Studio (Tauri)** | GUI front door (Launch/Stop process tracking) |
| **`.sh` runtime** | Linux convenience only — not required on Windows |
| **`openpfx` / particles** | Draft specification |
| **Blender KF2** | Specification / early architecture (stub add-on) |

## Quick start (Linux)

```bash
./openmaxfx          # native Studio (Tauri)
cargo test -p openras
cargo run -p openras -- list /path/to/mod.mpm
```

Build Studio: `cd studio && bun install && ./build.sh`

Or CLI:

```bash
./openmaxfx install-cd              # insert Max Payne CD first
./openmaxfx setup-game ~/Games/MaxPayne1
~/Games/MaxPayne1/launch-openmaxfx.sh
```

Guide: [START_HERE.md](START_HERE.md) · Play notes: [docs/PLAY_LINUX.md](docs/PLAY_LINUX.md)

## Layout

```
crates/openras/   # Native RAS/MPM CLI + library
studio/           # Tauri Studio
runtime/          # CD install, Wine bootstrap, localized-safe stack
tools/openpfx/    # particles (draft)
tools/blender_io_kf2/   # Blender KF2 (early)
docs/specs/       # clean-room format notes
vendor/           # local-only REA reference (gitignored — never commit)
```

## License / legal

Own a legal copy of Max Payne. No game binaries, retail `.ras`, or proprietary SDKs in this repository.
