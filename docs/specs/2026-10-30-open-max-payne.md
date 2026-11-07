# Spec: Open Max Payne (approved)

**Amended 2026-11-07** by the owner: two equal goals, engine and modding tools.

Goal:
  Play Max Payne 1 on current Windows, Linux and macOS with every community fix, and mod it with tools that work on
  those systems and with current modelling software. Never ship Remedy binaries or game assets.

The two goals:
  1. The game. A native engine reads the user's retail install, stays faithful to the original, and at least matches
     every unofficial patch (issues #162 to #168).
  2. Modding. MAX-FX tools and importers/exporters rebuilt for today: archives, particles, models, animation, levels.

In / out:
  In:
    - Native Max Payne 1 engine, through all levels
    - Fixes and modern options from the Widescreen Fix, MOGGA, FixPack and PCGamingWiki
    - Tools: RASMaker, ParticleFX, ActorFX and MaxED replacements as Rust crates
    - Modelling software: a working Blender add-on, import and export
    - Studio: Tauri GUI over the crates, kept working and growing with the tools
    - Wine fallback, CD install and 1.05 patcher, until the engine can finish the game
  Out:
    - Redistributing the game, retail `.ras`, FixPack/SFX blobs, or Remedy EXE/DLL
    - Hosting third-party mod asset trees in this repo
    - Claiming format or engine details without a spec and REA evidence

Behavior:
  Own repo `open-max-payne`, Max Payne 1 only.
  Behaviour comes from retail data and exe evidence; no invented values in code.
  Prefer Rust crates that compile to native binaries on the CI OS matrix.

Order of work:
  Game: Tutorial complete with fighting enemies, then New Game through all levels, then fixes and modern options.
  Modding: KF2/SKD/KFS import in Blender, then export back to the game, then the rest of the tools.
  The two tracks run side by side.

Constraints:
  Own media required; `vendor/` gitignored; no proprietary SDKs in git.
  Linux `.sh` = convenience only; Studio Play/patch must call Rust crates.

Done when (next):
  - [ ] Tutorial plays with enemies that fight back (`docs/specs/2026-11-06-health-and-enemy-ai.md`)
  - [ ] New Game plays through every level
  - [ ] Issues #162 to #168 closed
  - [ ] A model goes from Blender into the game and back
  - [ ] A level can be edited and played
  - [ ] Wine fallback removed once the engine finishes the game

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
  - [x] `openpfx` PSE→RPS export (`-e` parity) after RPS version REA (`openpfx export` + `validate-rps` gate accept; issue #11)
  - [x] Engine REA note naming archive mount + level-load tokens
  - [x] Decompiled `R_File::loadArchives` → `FindFirstFile("*.RAS")` (+ secondary path)
