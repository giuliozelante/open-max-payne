# CLAUDE.md — Open Max Payne

Standalone project: Max Payne 1 play + MAX-FX tools → **OpenLara-style north star**
(native engine + native tools, BYOD). **Baseline today:** Wine Play bridge + Rust
`openras` / patcher / play. Engine = long arc; RASMaker parity next.

## Do

- Keep pillars separate: Play / Content / Author / Studio / Engine (long arc)
- Spec + REA before claiming format or engine details (`docs/specs/`)
- Never commit `vendor/`, game installs, `*.exe`/`*.dll`/`*.ras`/`*.mpm`, or proprietary SDKs
- Prefer Studio (`./openmaxfx`) as the “for dummies” path
- Prefer Rust crates (`openras`, `openpfx`, `maxfx-patcher`, `maxfx-play`, `maxfx-engine`); no Python front door
- Studio Play/patch must call Rust crates — do not add Windows-required `.sh` paths
- Near-term: PSE→RPS export + engine level-load REA; Wine Play remains bridge

## Don’t

- Ship Max Payne binaries or third-party mod asset trees
- Treat this repo as a host for other games’/mods’ content
- Claim a playable native engine without a walking skeleton + REA-backed specs

## Key paths

- `crates/openras` — RAS/MPM + RASMaker flags (`cargo test -p openras`)
- `crates/openpfx` — ParticleFX PSE dump (`cargo test -p openpfx`)
- `crates/maxfx-engine` — engine skeleton (`cargo test -p maxfx-engine`)
- `runtime/install-from-cd.sh` — CD → `~/Games/MaxPayne1` + bootstrap
- `runtime/bootstrap-linux.sh` — DXVK / d3d8to9 / launcher (+ localized auto-park)
- `runtime/localized-safe-stack.sh` — localized CD (`x_french/german/italian/…`): park hi-res/ASI, d3d8to9+DXVK
- `runtime/detect-localized-lang.sh` — `x_<lang>.ras` detection (not english/data/music/level*)
- `docs/PLAY_LINUX.md` — SafeDisc / FixPack / localized checklist
- `studio/` — Tauri Studio. Front door: `./openmaxfx`
- Build Studio: `cd studio && ./build.sh` (Cursor pins RUSTC=1.83 — do not `source rust-env.sh` in fish)
- Workspace root: `Cargo.toml` (members: `crates/openras`, `studio/src-tauri`)
