# Spec: maxfx-engine walking skeleton (approved with north star)

Goal:
  First native-engine crate that opens a BYOD Max Payne 1 RAS/MPM via `openras`.
  Not a playable game.

In / out:
  In: `crates/maxfx-engine`; load archive; reject non-MP1 compatibility; unit test on synthetic RAS
  Out: renderer, entities, bullet-time, audio, claiming “playable”

## REA notes (strings evidence — 2026-10-30)

**Game install (BYOD, not in git):** `MaxPayne.exe` sha256 `342d296c…da46e` (1.05 drop-in) + sibling `rl.dll`.

`R_File` archive API (mangled exports / RTTI from `MaxPayne.exe` + `rl.dll` strings):

| Symbol | Role (from name) |
|--------|------------------|
| `R_File::loadArchives` | Mount RAS set at boot |
| `R_File::unloadArchives` | Tear down |
| `R_File::loadArchiveHeader` | Header parse (already mirrored in `openras`) |
| `R_File::decryptWithSeed` / `decompress` | Already in `openras` |
| `R_File::enablePriorityArchive` | Mod / override archive |
| `R_File::setSecondaryArchiveSearchPath` | Search path |
| `R_File::RASArchiveList` | Linked list of mounted archives |
| `R_File::RAS_ARCHIVE_VERSION` | Version constant |

Level DB load tokens (exe strings): `X_LEVELDB_RC_LOAD`, `X_LEVELDB_LC_LOAD`, `X_LEVELDB_MC_LOAD`, `X_LEVELDB_SMC_LOAD`, `X_LEVELDB_FSMC_LOAD`, `X_LEVELDB_DMC_LOAD`, `X_LEVELDB_SLC_LOAD`, `X_LEVELDB_TI_LOAD`.

Next REA increments (Ghidra timed out on full exe this session — resume with `rl.dll` first):
  1. Decompile `R_File::loadArchives` — which `.ras` names are mounted and in what order
  2. Trace one `X_LEVELDB_*_LOAD` into file paths inside the archive
  3. Minimal draw: one room wireframe from extracted assets

Done when:
  - [x] Crate + `open_archive` + synthetic test
  - [x] REA-backed note naming archive mount + level-load tokens
  - [x] CLI `maxfx-engine <archive.ras>` prints summary
  - [ ] Decompiled `loadArchives` call chain + ordered RAS list
