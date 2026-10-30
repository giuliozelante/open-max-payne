# Spec: Level DB paths (REA / BYOD probe)

Goal:
  Document how Max Payne 1 stores levels relative to `R_File::loadArchives` +
  `X_LEVELDB_*_LOAD` tokens — without inventing `.ldb` binary layout yet.

## Archive mount (done)

`rl.dll` `R_File::loadArchives` mounts every `*.RAS` in the game dir (and optional
secondary path). See `2026-10-30-engine-skeleton.md`.

## On-disk level packages (BYOD probe, not in git)

`openras -v ~/Games/MaxPayne1/x_level1.ras` (compat 3, ver 1.2):

| Pattern | Role |
|---------|------|
| `\data\database\levels\part1\*.ldb` | Level database blob (LZSS mode 1; multi‑MB) |
| `\data\database\levels\part1\*.ai` | Sibling AI / script companion |

Paired names e.g. `Part1_Level1.ldb` + `Part1_Level1.ai` (case varies).

`x_level2.ras` / `x_level3.ras` follow the same `data\database\levels\...` layout
for later parts.

## `X_LEVELDB_*_LOAD` tokens (MaxPayne.exe strings)

| Token | Likely container (from nearby RTTI) |
|-------|-------------------------------------|
| `X_LEVELDB_RC_LOAD` | Room (`X_LevelDBRoom`) |
| `X_LEVELDB_SMC_LOAD` | Static mesh (`X_LevelDBStaticMesh`) |
| `X_LEVELDB_SLC_LOAD` | Static light |
| `X_LEVELDB_LC_LOAD` | Light |
| `X_LEVELDB_MC_LOAD` | Mesh / mesh polygon |
| `X_LEVELDB_FSMC_LOAD` | FSM-related mesh container |
| `X_LEVELDB_DMC_LOAD` | Dynamic mesh |
| `X_LEVELDB_TI_LOAD` | TBD (no clear RTTI neighbour yet) |

These are **loader tags**, not filenames. Filenames are the `.ldb` paths above.

## `.ldb` / `.ai` first bytes (BYOD extract — not committed)

After `openras` LZSS extract of `Part1_Level6.ldb` (~6.9 MiB) and sibling `.ai`:

- No obvious ASCII magic (`RAS`, `LDB`, etc.) in the first 64 bytes.
- Dense runs of `0x20` with sparse non-space markers (`>`, `4`, NULs, etc.).
- Treat layout as **unknown** until the `.ldb` loader is decompiled — do not invent tags.

## Next (issue #12)

1. REA: find which code path opens `*.ldb` after mount (follow `X_LEVELDB_*_LOAD` / `X_LevelDBImport*`)
2. Lock `.ldb` magic/version from that loader
3. [x] `maxfx-engine` lists `.ldb`/`.ai` pairs from a user-supplied `x_level*.ras`
