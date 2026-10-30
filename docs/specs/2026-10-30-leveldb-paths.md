# Spec: Level DB paths (REA / BYOD probe)

Goal:
  Document how Max Payne 1 stores levels relative to `R_File::loadArchives` +
  `X_LEVELDB_*_LOAD` tokens — without inventing full `.ldb` geometry layout.

## Archive mount (done)

`rl.dll` `R_File::loadArchives` mounts every `*.RAS` in the game dir (and optional
secondary path). See `2026-10-30-engine-skeleton.md`.

## Path construction (issue #12) — SharedDB `levels.txt`

**There is no literal `.ldb` string in `MaxPayne.exe`.** Level files are named by
the SharedDB catalog:

| Evidence | Detail |
|----------|--------|
| Exe string | `Levels.txt` |
| Exe string | `Level "%s" not found from the database` |
| BYOD | `x_data.ras` → `\data\database\levels\levels.txt` (mode 3) |

Each top-level block (e.g. `[part0_level1]`) sets:

```
Directory = part1;
Level     = part0_level1.ldb;
```

**In-archive path:**

```
\data\database\levels\{Directory}\{Level}
```

Example: `\data\database\levels\part1\part0_level1.ldb`

Sibling AI package (same basename, `.ai`) lives beside the `.ldb` inside the
matching `x_level*.ras` (not listed in `levels.txt`; paired by name).

Rust: `maxfx_engine::parse_levels_txt` + `LevelsTxtEntry::ldb_archive_path`.

### Call chain (strings / catalog — ImportLevel decompile still open)

```
boot → R_File::loadArchives (*.RAS)
     → SharedDB reads \data\database\levels\levels.txt
     → resolve Directory + Level → open .ldb from mounted RAS
     → X_LEVELDB_*_LOAD tokens drive Import* RTTI while parsing the blob
```

`X_LEVELDB_*_LOAD` are **importer phase tokens**, not path builders. Paths come
from `levels.txt` before those tokens run.

## On-disk level packages (BYOD probe, not in git)

`openras -v ~/Games/MaxPayne1/x_level1.ras` (compat 3, ver 1.2):

| Pattern | Role |
|---------|------|
| `\data\database\levels\part1\*.ldb` | Level database blob (LZSS mode 1; multi‑MB) |
| `\data\database\levels\part1\*.ai` | Sibling AI / script companion |

Paired names e.g. `Part1_Level1.ldb` + `Part1_Level1.ai` (case varies).

## `X_LEVELDB_*_LOAD` tokens → Import RTTI (MaxPayne.exe)

Tokens sit in `.rdata` after `MP_GM_LEVEL_INIT1`. Mapping from `X_LevelDBImport*` RTTI:

| Token | Import RTTI (best fit) |
|-------|-------------------------|
| `X_LEVELDB_RC_LOAD` | `X_LevelDBImportRoom` / `…RoomContainer` |
| `X_LEVELDB_DMC_LOAD` | `X_LevelDBImportDynamicMesh` / `…Container` |
| `X_LEVELDB_FSMC_LOAD` | `X_LevelDBImportFSM` / `…FSMContainer` |
| `X_LEVELDB_SLC_LOAD` | `X_LevelDBImportStaticLight` / `…Container` |
| `X_LEVELDB_SMC_LOAD` | `X_LevelDBImportStaticMesh` / `…Container` |
| `X_LEVELDB_LC_LOAD` | `X_LevelDBImportLightmap` / `…Container` |
| `X_LEVELDB_MC_LOAD` | `X_LevelDBImportMaterial` / mesh polygon |
| `X_LEVELDB_TI_LOAD` | `X_LevelDBImportTextureImage` / level item |

Also present: `X_LevelDBImportLevel` (top-level), `ImportBSP`, `ImportCharacter`,
`ImportTrigger`, `ImportStartpoint`, `ImportExit`, `ImportLevelItem`, …

## `.ldb` / `.ai` preamble (BYOD)

Both `.ldb` and `.ai` from `x_level1.ras` share:

```
16 × 0x20   then   0x3E ('>')
```

Constant in code: `maxfx_engine::LDB_PREAMBLE` / `looks_like_ldb_or_ai`.

After the preamble, files stay **space-heavy** early on (`.ai` shows embedded
`0x14` tag-like bytes and many `>` record markers). Full chunk layout still needs
decompile of `X_LevelDBImportLevel`.

## Done / next (issue #12)

- [x] Path layout inside `x_level*.ras`
- [x] Token list + Import* RTTI map (best-effort)
- [x] Shared preamble observation + engine helper
- [x] `maxfx-engine` lists `.ldb`/`.ai` pairs
- [x] `levels.txt` → archive path construction + parser/tests
- [ ] Decompile `X_LevelDBImportLevel` / `*_LOAD` dispatch (chunk format)
- [ ] Chunk / section format inside `.ldb` after preamble
