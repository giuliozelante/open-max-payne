# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# Open Max Payne

Standalone project: Max Payne 1 play + MAX-FX tools → **OpenLara-style north star**
(native engine + native tools, BYOD — user supplies retail install at `~/Games/MaxPayne1`).
**Baseline today:** Wine Play bridge + Rust `openras` / patcher / play. Engine = long arc.
Product spec: `docs/specs/2026-10-30-open-max-payne.md`. Latest handoff/state: `docs/HANDOFF-CLAUDE-CODE.md`.

## Do

- Keep pillars separate: Play / Content / Author / Studio / Engine (long arc)
- Spec + REA before claiming format or engine details (`docs/specs/`, `docs/rea/`)
- Label REA claims **VERIFIED / STRONGLY SUPPORTED / HYPOTHESIS / UNKNOWN**, with addresses
  (PE i386, ImageBase `0x400000`). Record ruled-out paths too.
- Prefer Studio (`./openmaxfx`) as the “for dummies” path
- Prefer Rust crates; no Python front door
- Studio Play/patch must call Rust crates — do not add Windows-required `.sh` paths
  (`runtime/*.sh` are Linux convenience only)

## Don’t

- Commit `vendor/` (local REA reference), game installs, `*.exe`/`*.dll`/`*.ras`/`*.mpm`,
  FixPack blobs, or proprietary SDKs
- Ship Max Payne binaries or third-party mod asset trees, or host other games’/mods’ content
- Claim a playable native engine without a walking skeleton + REA-backed specs
- Conflate DO_Animate (dynamic-mesh transform curves) with KF2 vertex animation — separate
  retail paths (see `docs/rea/2026-11-01-*.md`)

## Commands

Cargo workspace root `Cargo.toml`; members: `openras`, `openpfx`, `maxfx-patcher`,
`maxfx-play`, `maxfx-engine` (under `crates/`) and `studio/src-tauri` (package
`open-max-payne-studio`). Use `--offline` when deps already fetched.

```bash
# What CI runs (.github/workflows/ci.yml, RUSTFLAGS=-Dwarnings)
cargo fmt --all -- --check
cargo clippy -p openras -p openpfx -p maxfx-patcher -p maxfx-play -p maxfx-engine --all-targets -- -D warnings
cargo test   -p openras -p openpfx -p maxfx-patcher -p maxfx-play -p maxfx-engine
cargo check  -p open-max-payne-studio          # needs webkit2gtk-4.1 / gtk3 dev libs
shellcheck -x runtime/*.sh openmaxfx

# Single test
cargo test -p openras <test_name_substring>

# Run tools
cargo run -p openras -- list /path/to/mod.mpm   # RASMaker-compatible flags -x/-v/-a/-m/-p/-c
cargo run -p openpfx -- dump-pse|dump-rps|export|validate-rps ...
cargo run -p maxfx-engine -- --window ...       # 3D walkthrough over BYOD .ldb
cargo run -p maxfx-engine -- --frame out.ppm [--camera NAME.DO] level.ldb  # headless frame for visual checks

# Studio (Tauri 2 + Vite/TS, bun)
cd studio && bun install && ./build.sh          # build.sh sources rust-env.sh (bash); never source it from fish
./openmaxfx                                     # launches built Studio, else `bun run tauri dev`
```

## Architecture

- **Content crates** (`openras`, `openpfx`): format libs + thin CLIs. `openras` = RAS/MPM archives
  (crypto + `RA->` 12-byte LZSS header, RASMaker CLI in `cli_rasmaker.rs`). `openpfx` = ParticleFX
  PSE dump / PSE→RPS export. Format specs: `docs/specs/{ras,rps,kf2}_format.md`.
- **`maxfx-engine`**: lib + bin. `scene.rs` decodes level `.ldb` (textures, lightmaps, rooms,
  portals, meshes, FSMs, triggers); `kf2.rs` KF2/SKD/KFS animation + skinning; `render3d.rs`
  windowed walkthrough; `menu_ui.rs` startup/menu/comic flow. Most logic + tests in `lib.rs`.
- **Play**: `maxfx-play` picks backend from `OPENMAXFX_PLAY` (default Wine + retail
  `MaxPayne.exe`; `native` → `maxfx-engine` located via `OPENMAXFX_ENGINE`/`PATH`, with
  `OPENMAXFX_ENGINE_WINDOW`, `OPENMAXFX_ENGINE_PROBE`). `maxfx-patcher` applies 1.05
  (HTTPS + SHA-256 allowlist, refuses SFX installers).
- **Studio**: `studio/src-tauri/src/lib.rs` = Tauri commands (`generate_handler!`) that call the
  crates and set the Play env vars; `studio/src/main.ts` = single-page UI tabs (RASMaker, Play,
  MaxED 3D, ParticleFX, ActorFX).
- **Linux runtime** (`runtime/`): `install-from-cd.sh` (CD → `~/Games/MaxPayne1` + bootstrap),
  `bootstrap-linux.sh` (DXVK / d3d8to9 / launcher), `localized-safe-stack.sh` +
  `detect-localized-lang.sh` (localized CDs via `x_<lang>.ras`). Checklist: `docs/PLAY_LINUX.md`.
- `tools/blender_io_kf2/`: Blender KF2 add-on stub (early).

## Git

Branches: `main` (default), `develop`, `feature/*`, `bugfix/*`. CI on push to main + all PRs.
