# Spec: maxfx-engine walking skeleton (approved with north star)

Goal:
  First native-engine crate that opens a BYOD Max Payne 1 RAS/MPM via `openras`.
  Not a playable game.

In / out:
  In: `crates/maxfx-engine`; load archive; reject non-MP1 compatibility; unit test on synthetic RAS
  Out: renderer, entities, bullet-time, audio, claiming “playable”

Next REA increments (do not invent without evidence):
  1. `MaxPayne.exe` / game `rl.dll` — archive mount at boot, level load entrypoints
  2. Level / KF2 / mesh formats already sketched under `docs/specs/`
  3. Minimal draw: one room wireframe or textured quad from extracted assets

Done when:
  - [x] Crate + `open_archive` + synthetic test
  - [ ] REA-backed note naming the real level-load call chain
  - [ ] Optional CLI that prints archive summary for a user-supplied `.ras`
