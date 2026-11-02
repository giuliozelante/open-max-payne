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

### `R_File::loadArchives` (`rl.dll` `0x1000e8f0`) — decompiled

Behaviour (REA pseudocode + data bytes):

1. `setArchiveHandling(this, 1)`
2. Copy `\` into `m_path` (`DAT_10049604` = `"\\"`)
3. `FindFirstFileA("*.RAS", …)` (`DAT_100495fc` = `"*.RAS"`) under that path
4. For each hit: append `R_RASArchive` to `RASArchiveList`, call `loadArchiveHeader(this, entry, cFileName)`
5. If `sm_strSecondaryArchiveSearchPath` is non-empty: append `"*.RAS"` and `FindFirstFileA` there too (mod / override path)

**There is no hardcoded `x_level1.ras` list in `loadArchives`.** Mount order = Win32 find enumeration order for `*.RAS` in the game directory (plus optional secondary path). Localized `x_<lang>.ras` is picked up the same way.

Next REA increments:
  1. Trace one `X_LEVELDB_*_LOAD` into in-archive paths
  2. Minimal draw: one room wireframe from extracted assets

Done when:
  - [x] Crate + `open_archive` + synthetic test
  - [x] REA-backed note naming archive mount + level-load tokens
  - [x] CLI `maxfx-engine <archive.ras>` prints summary
  - [x] Decompiled `loadArchives`: mount = `FindFirstFile("*.RAS")` (+ optional secondary path)
  - [x] Minimal draw: `--window` orthographic vec3 preview from BYOD `.ldb` (epic #16 smoke)
