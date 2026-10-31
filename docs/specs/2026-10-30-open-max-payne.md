# Spec: Open Max Payne (approved)

**Amended 2026-10-30** — OpenLara-style north star (engine + tools, BYOD).

Goal:
  Standalone project to play Max Payne 1 and use its MAX-FX toolchain on
  Windows 10+, Linux, and macOS — eventually via **native reimplementations**,
  never by shipping Remedy binaries or game assets.

North star (OpenLara model):
  Rewrite the **game engine** and the **MAX-FX tools** from clean-room REA of
  formats and behaviour. Users supply their own retail CD / install.
  Wine + original `MaxPayne.exe` is the **bridge**, not the destination.

In / out:
  In:
    - Play bridge: CD install, 1.05 patcher, Wine/DXVK localized-safe launch
    - Content: RAS/MPM and related formats (`openras`, later particles/RPS)
    - Author: Blender KF2 path; native MAX-FX tool CLIs (RASMaker parity first)
    - Studio: Tauri GUI over Rust crates
    - **Native Max Payne 1 engine** (long arc): load retail `.ras`, recreate
      gameplay/render/audio enough to play without `MaxPayne.exe`
    - **Native MAX-FX tool suite**: RASMaker and siblings as Rust (native) bins
  Out:
    - Redistributing the game, retail `.ras`, FixPack/SFX blobs, or Remedy EXE/DLL
    - Hosting third-party mod asset trees in this repo
    - Claiming format/engine details without a spec + REA evidence

Behavior:
  Own repo `open-max-payne` — Max Payne 1 only.
  Prefer Rust crates that compile to native binaries on the CI OS matrix.
  Spec + REA before inventing format or engine behaviour (`docs/specs/`).

Near-term order (after Inc 1–3):
  1. RASMaker CLI parity on `openras` (flags + fixtures on OS matrix)
  2. One more MAX-FX tool at a time (REA-backed)
  3. Engine exploration spikes via REA (`MaxPayne.exe` / `rl.dll`) — separate
     increment plans; no “big bang” rewrite claim until a walking skeleton exists

Constraints:
  Own media required; `vendor/` gitignored; no proprietary SDKs in git.
  Linux `.sh` = convenience only; Studio Play/patch must call Rust crates.

Done when (baseline — shipped):
  - [x] Separate repo + README pillars
  - [x] Linux CD install → ~/Games/MaxPayne1 (exe + .ras + launcher)
  - [x] Native Tauri Studio + `./openmaxfx` front door
  - [x] Rust `openras` + synthetic CI tests; Inc 3 patcher/play
  - [x] CI: OS matrix, CodeQL, Dependabot, release workflow

Done when (north star — open):
  - [x] RASMaker flag parity documented + green on matrix
  - [x] At least one additional native MAX-FX tool beyond archiver (`openpfx` dump)
  - [x] Engine walking skeleton loads a BYOD archive (`maxfx-engine` + skeleton spec)
  - [x] Play path can prefer native engine for BYOD smoke; Wine remains fallback
        (`OPENMAXFX_PLAY=native` + Studio checkbox → `maxfx-engine --window` point preview; epic #16)
  - [ ] `openpfx` PSE→RPS export (`-e` parity) after RPS version REA (issue #11 in-level smoke)
  - [x] Engine REA note naming archive mount + level-load tokens
  - [x] Decompiled `R_File::loadArchives` → `FindFirstFile("*.RAS")` (+ secondary path)
