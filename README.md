# Open Max Payne

[![ci](https://github.com/giuliozelante/open-max-payne/actions/workflows/ci.yml/badge.svg)](https://github.com/giuliozelante/open-max-payne/actions/workflows/ci.yml)

Play **Max Payne 1** and use its **MAX-FX** toolchain on modern OSes.
You need your own retail CD / install. This project does **not** redistribute the game.

**North star (OpenLara model):** clean-room native **engine** + **MAX-FX tools** that
read your BYOD data. Wine + original `MaxPayne.exe` is the bridge until the engine exists.
Product spec: [docs/specs/2026-10-30-open-max-payne.md](docs/specs/2026-10-30-open-max-payne.md).

## Current baseline (honest)

| Area | Status |
|------|--------|
| **Play (Rust)** | `maxfx-play` — Wine/`MaxPayne.exe` default; opt-in `OPENMAXFX_PLAY=native` → `maxfx-engine` probe (#16) |
| **1.05 patcher** | `maxfx-patcher` — HTTPS + SHA-256 allowlist; refuses SFX installers |
| **`openras` (Rust)** | **Alpha** — RASMaker `-x/-v/-a/-m/-p/-c` + subcommands; CI tests |
| **`openpfx` (Rust)** | **Draft** — `dump-pse` / `dump-rps` (tags `14`/`11`/`09`/`0d`/`13`); preamble export only |
| **`maxfx-engine`** | **Skeleton** — opens BYOD MP1 RAS via `openras` (not playable) |
| **Studio (Tauri)** | GUI front door (Launch/Stop process tracking) |
| **`.sh` runtime** | Linux convenience only — not required on Windows |
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
crates/openras/       # RAS/MPM + RASMaker-compatible CLI
crates/openpfx/       # ParticleFX PSE dump (draft)
crates/maxfx-engine/  # Native engine skeleton (BYOD archive open)
crates/maxfx-play/    # Wine/native Play bridge
crates/maxfx-patcher/  # 1.05 drop-in apply
studio/               # Tauri Studio
runtime/              # CD install, Wine bootstrap, localized-safe stack
tools/blender_io_kf2/ # Blender KF2 (early)
docs/specs/           # clean-room format notes
vendor/               # local-only REA reference (gitignored — never commit)
```

## License / legal

Own a legal copy of Max Payne. No game binaries, retail `.ras`, or proprietary SDKs in this repository.
