# Open Max Payne

[![ci](https://github.com/giuliozelante/open-max-payne/actions/workflows/ci.yml/badge.svg)](https://github.com/giuliozelante/open-max-payne/actions/workflows/ci.yml)

Play **Max Payne 1** on today's Windows, Linux and macOS with every community fix built in, and mod it with
tools that work on those systems and with today's 3D software.
You need your own retail CD or install. This project does **not** redistribute the game.

## Goals

Two goals carry equal weight:

1. **The game.** A native engine that plays Max Payne 1 from your retail files, faithful to the original, and at
   least matching every unofficial patch (issues #162 to #168).
2. **Modding.** The MAX-FX tools and importers/exporters rebuilt for current systems and modelling software:
   archives, particles, models, animation and levels.

Behaviour comes from the game's data and from evidence in the original exe, never from invented values. Wine with
the original `MaxPayne.exe` stays as a fallback until the engine can finish the game.
Product spec: [docs/specs/2026-10-30-open-max-payne.md](docs/specs/2026-10-30-open-max-payne.md).
Current state: [docs/STATUS.md](docs/STATUS.md).

## What works today

| Area | Status |
|------|--------|
| **Engine: Tutorial** | Plays start to finish from retail data: movement, rolls and dives, weapons, HUD, sound, snow and particles, scripted enemies, health, painkillers, falling damage |
| **Engine: enemies** | They appear, take hits and die, but do not shoot back or find paths yet |
| **Engine: New Game** | Starts the intro and early levels; the full game has not been played through |
| **Engine: fixes** | Tracked in issues #162 to #168; widescreen, settings and saves are still open |
| **Wine fallback** | `maxfx-play` runs the original `MaxPayne.exe`; `OPENMAXFX_PLAY=native` picks the engine |
| **1.05 patcher** | `maxfx-patcher`: HTTPS and a SHA-256 allowlist; refuses SFX installers |
| **`openras`** | Complete RASMaker replacement for RAS/MPM archives |
| **`openpfx`** | Reads and repacks `.rps`; PSE to RPS export is partial |
| **Studio (Tauri)** | GUI over the crates: RASMaker, Play, MaxED 3D, ParticleFX, ActorFX tabs |
| **Blender add-on** | Stub: it lists names from KF2/SKD/KFS files and imports no geometry yet |
| **Level editing** | Levels can be read and viewed; no editor or `.ldb` writer yet |
| **`.sh` runtime** | Linux convenience only; not required on Windows |

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
crates/openpfx/       # ParticleFX PSE and RPS tools
crates/maxfx-engine/  # Native engine: levels, rendering, gameplay, audio
crates/maxfx-play/    # Picks the native engine or the Wine fallback
crates/maxfx-patcher/  # 1.05 drop-in apply
studio/               # Tauri Studio
runtime/              # CD install, Wine bootstrap, localized-safe stack
tools/blender_io_kf2/ # Blender add-on (stub)
docs/specs/           # format notes and plans
docs/rea/             # reverse-engineering notes with addresses
vendor/               # local-only REA reference (gitignored — never commit)
```

## License / legal

Own a legal copy of Max Payne. No game binaries, retail `.ras`, or proprietary SDKs in this repository.
