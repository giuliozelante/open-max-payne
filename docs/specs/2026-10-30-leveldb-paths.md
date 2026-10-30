# Spec: Level DB paths (REA / BYOD probe)

Goal:
  Document how Max Payne 1 stores levels relative to `R_File::loadArchives` +
  `X_LEVELDB_*_LOAD` tokens — without inventing full `.ldb` geometry layout.

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

**Note:** `MaxPayne.exe` has **no** literal `.ldb` string. Paths are built at runtime
(or come from scripts). Nearby string table: `MP_GM_LEVEL_INIT1` then the
`X_LEVELDB_*_LOAD` tokens in order.

## `X_LEVELDB_*_LOAD` tokens → Import RTTI (MaxPayne.exe)

Tokens sit in `.rdata` after `MP_GM_LEVEL_INIT1`. Mapping from `X_LevelDBImport*` RTTI:

| Token | Import RTTI (best fit) |
|-------|-------------------------|
| `X_LEVELDB_RC_LOAD` | `X_LevelDBImportRoom` / `…RoomContainer` |
| `X_LEVELDB_DMC_LOAD` | `X_LevelDBImportDynamicMesh` / `…Container` |
| `X_LEVELDB_FSMC_LOAD` | `X_LevelDBImportFSM` / `…FSMContainer` |
| `X_LEVELDB_SLC_LOAD` | `X_LevelDBImportStaticLight` / `…Container` |
| `X_LEVELDB_SMC_LOAD` | `X_LevelDBImportStaticMesh` / `…Container` |
| `X_LEVELDB_LC_LOAD` | `X_LevelDBImportLightmap` / `…Container` (or static light — confirm) |
| `X_LEVELDB_MC_LOAD` | `X_LevelDBImportMaterial` / mesh polygon — confirm |
| `X_LEVELDB_TI_LOAD` | `X_LevelDBImportTextureImage` / trigger / level item — confirm |

Also present: `X_LevelDBImportLevel` (top-level), `ImportBSP`, `ImportCharacter`,
`ImportTrigger`, `ImportStartpoint`, `ImportExit`, `ImportLevelItem`, …

## `.ldb` / `.ai` preamble (BYOD)

Both `.ldb` and `.ai` from `x_level1.ras` share:

```
16 × 0x20   then   0x3E ('>')
```

Constant in code: `maxfx_engine::LDB_PREAMBLE` / `looks_like_ldb_or_ai`.

After the preamble, files are **high-entropy** (`.ldb` entropy ~7.8) with embedded
floats — not a space-filled format overall. Full chunk layout still needs
decompile of `X_LevelDBImportLevel` (or the `*_LOAD` dispatch).

## Done / next (issue #12)

- [x] Path layout inside `x_level*.ras`
- [x] Token list + Import* RTTI map (best-effort)
- [x] Shared preamble observation + engine helper
- [x] `maxfx-engine` lists `.ldb`/`.ai` pairs
- [ ] Decompile `X_LevelDBImportLevel` / `*_LOAD` dispatch → open path construction
- [ ] Chunk / section format inside `.ldb` after preamble
