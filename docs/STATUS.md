# Project status

Where the project stands on 2026-11-08, and how to pick the work up. Goals and order of work are in
[the product spec](specs/2026-10-30-open-max-payne.md); evidence is in `docs/rea/`.

## How work runs

### The loop

The owner plays the engine and compares it with retail. On `capture engineN` the previous `engine*` folder under
`~/Pictures/omp-compare/` is deleted, `develop` is built, and the engine runs with
`OPENMAXFX_CAPTURE_DIR=$HOME/Pictures/omp-compare/engineN`. The capture (an `.mp4` and `.ppm` frames, no sound
track) is reviewed with contact sheets against the retail recordings in `original*` (never deleted). Each finding
becomes its own PR.

Retail reference: the owner's install runs at 60 fps through DXVK, with the Widescreen Fix's loader parked, so its
gameplay is unmodified. The game is Italian, so it plays from `x_*.ras`; the FixPack's `data.disabled-hires` is not
used. The HD recording `original-video-hd/Screencast_20261106_214821.webm` (211 s, maximum settings) covers the
Tutorial.

### Rules every change follows

- **Assets define the game.** Behaviour comes from retail data (`.ldb`, `.txt`, `.h`, `.rps`, `.kf2`) or from the
  exe. No invented values. Where retail compiles data into the exe (AI machines, default key bindings, the scope
  shake curve), it becomes a Rust table that cites the address it was read from.
- **Evidence first.** Read the exe (`objdump` listing, string table, imports), record findings in `docs/rea/` with
  **VERIFIED / STRONGLY SUPPORTED / HYPOTHESIS / UNKNOWN** and addresses, then implement. Ruled-out paths are recorded
  too. Retail screenshots are a final check, not a source.
- **Software factory** (the owner's `software-factory.md`): plan, isolate, build, check, review. Major work gets a
  short spec in `docs/specs/` and waits for approval. One issue per branch from `develop`, PR into `develop`, and no
  merge without an explicit "merge N" from the owner.
- **TDD.** A failing test first (run, seen failing for the right reason), then the smallest fix. A changed or removed
  test is named in the PR. Retail-gated tests read `~/Games/MaxPayne1` and skip without it.
- **Gates before every commit:** `cargo fmt --all -- --check`, `cargo clippy -p openras -p openpfx -p openkf2
  -p maxfx-patcher -p maxfx-play -p maxfx-engine --all-targets -- -D warnings`, `cargo test` on the same crates. PRs
  state the commands and results, and what was not checked (most often: not played yet).
- **Commits:** Conventional Commits, one task per commit, pushed after a green commit, no AI trailers.
- **Workflows:** actionlint before pushing `.github/workflows/` changes. The repo has no act wrapper.
- One cargo process at a time; use `--offline` once dependencies are fetched.

## Done in the last sessions (PRs #183-#203)

- **Docs:** README, product spec and this file reflect the two goals (#183).
- **Engine visuals:** fog-coloured sky where the world sphere has no mesh (#185), 16x anisotropic filtering (#186),
  no invented white flash on hit enemies (#195).
- **Sniper scope:** use key zooms with `FOVMultiplier` (#188); the view sways as `ShakeLength` / `ShakeStrength*`
  say, through retail's Catmull-Rom curve `0x68e820` (#198).
- **Tips:** printed during level cameras (#194) and at level start (#200). Their `#TOKEN#`s name the keys the player
  bound, as retail does (#196).
- **Controls:** every game action reads retail's 32-slot binding table, from the Wine prefix's `user.reg` or
  retail's defaults at `0x4cb090` (#197). Notes: [key bindings](rea/2026-11-07-key-bindings.md).
- **Sound:** mixed in-process with rodio. Retail WAVs are MS ADPCM; all 1,331 decode. `ffplay` remains only for the
  intro video (#199).
- **Collision stopgap:** a body caught in a wall can no longer go deeper (#201).
- **Particles:** sub-frame bursts emit at any frame rate (#202). Frames that reuse a stored bitmap now get it, which
  fixed 125 of the 282 retail effects (#203).
- **Modding:** `openkf2` crate with KF2/SKD/KFS readers (#189), glTF export of mesh, skin, textures and clips
  (#190, #191), and `openkf2 export` (#192), linted and tested in CI (#193).
- **REA:** AI state machines decoded as rule tables, with the phase 2 plan (#184).

## Open PRs

- **#204** — [capsule collision spec](specs/2026-11-08-capsule-collision.md). Approved by the owner; the PR is still
  open to land the text.
- **#205** — AI perception findings: the seen and perceived flags, the line-of-sight test, and `EnemyInterestTime`
  memory. Docs only.
- Branch **`docs/collision-rea`** (no PR yet) — [character collision notes](rea/2026-11-08-character-collision.md),
  step 1 of #204 in progress.

## What to do next, in order

1. **Collision (approved, #204).** Finish the exe reading: `0x646f30` and `0x645340` are the next routines, then the
   character update's call into `0x647660`. Then rewrite: the capsule from skin data, triangle distance, sweep and
   slide, and floors from the same test. Done when the Tutorial's secret-room wall and the first alley's trash
   containers stop Max in a capture.
2. **Enemies, phase 2** ([spec](specs/2026-11-06-health-and-enemy-ai.md)). Read the range and cone gates before the
   sight ray (`0x4f6700`-`0x4f6b16`) and what feeds the stimulus. Then the interpreter for `StandAndShoot`, aiming at
   `AimingSpeed`, and firing through the player's projectile path. Delete the hand-written pursuit.
3. **Releases (on hold until the owner says go).** A GitHub Actions release workflow that builds binaries for Linux,
   Windows and macOS on `v*` tags, plus end-user docs for launching them. Needs a spec first.
4. **Modding:** KF2/SKD/KFS writers and glTF import (steps 4-5 of the
   [converter spec](specs/2026-11-07-gltf-converter.md)). Import needs a JSON parser (`serde_json`), not yet
   approved. Then the Blender add-on.
5. **Owner checks pending:** open `max_cli.glb` in Blender, and `capture engine27` to verify #200-#203.

## Engine: known gaps

- Enemies do not perceive, aim, shoot or follow `.ai` paths (item 2 above).
- Collision: walls are a box plus plane distance with a 0.38 m radius; retail uses a 0.48 m capsule (item 1).
- `[Target]` hit zones are not applied, and enemy capsule values are still constants.
- Bullet camera and its shake are missing.
- The chase camera clips into walls and into Max.
- Max's death restarts the level, but has not been played; its camera is missing.
- Measured from the txt data, dive, roll and jumps cover 5.8, 3.8 and 3.6 m. If retail goes further, the likely
  cause is the running speed being carried in; a retail recording of a standing and a running dive would settle it.
- Keyboard aim turns at 2.2 rad/s, which has no retail source. Bindings cannot be saved yet, and on Windows they are
  not read from the registry.
- Key names for codes `input.txt` lacks use US Windows names; retail under Wine shows Wine's names.
- Material detail textures (#187), and the community-fix parity issues #162-#168.

## Modding: known gaps

- Blender add-on is a stub; glTF export works but has no vertex normals and is untested in Blender.
- No KF2/SKD/KFS writers or glTF import yet.
- `openpfx` export omits graphs and emitters.
- No `.ldb` writer or level editor.
