# CLAUDE.md — Open Max Payne

Standalone project: Max Payne 1 play + MAX-FX tools. **Baseline:** Linux Play + Rust `openras`. Win/mac Play, Blender, particles = planned/draft.

## Do

- Keep pillars separate: Play / Content / Author / Studio
- Spec + REA before claiming format details (`docs/specs/`)
- Never commit `vendor/`, game installs, `*.exe`/`*.dll`/`*.ras`/`*.mpm`, or proprietary SDKs
- Prefer Studio (`./openmaxfx`) as the “for dummies” path
- Prefer `crates/openras` (Rust); no Python front door

## Don’t

- Ship Max Payne binaries or third-party mod asset trees
- Treat this repo as a host for other games’/mods’ content
- Invent Wise/InstallShield unpackers or experimental ASI loaders without a signed-off Inc 3 plan

## Key paths

- `crates/openras` — RAS/MPM (`cargo test -p openras`)
- `runtime/install-from-cd.sh` — CD → `~/Games/MaxPayne1` + bootstrap
- `runtime/bootstrap-linux.sh` — DXVK / d3d8to9 / launcher (+ localized auto-park)
- `runtime/localized-safe-stack.sh` — localized CD (`x_french/german/italian/…`): park hi-res/ASI, d3d8to9+DXVK
- `runtime/detect-localized-lang.sh` — `x_<lang>.ras` detection (not english/data/music/level*)
- `docs/PLAY_LINUX.md` — SafeDisc / FixPack / localized checklist
- `studio/` — Tauri Studio. Front door: `./openmaxfx`
- Build Studio: `cd studio && ./build.sh` (Cursor pins RUSTC=1.83 — do not `source rust-env.sh` in fish)
- Workspace root: `Cargo.toml` (members: `crates/openras`, `studio/src-tauri`)
