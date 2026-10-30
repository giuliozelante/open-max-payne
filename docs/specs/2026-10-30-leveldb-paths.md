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

## `.ldb` / `.ai` preamble + records (BYOD, issue #14)

Both `.ldb` and `.ai` from `x_level1.ras` share:

```
16 × 0x20   then   0x3E ('>')
```

Constant in code: `maxfx_engine::LDB_PREAMBLE` / `looks_like_ldb_or_ai`.

### Record framing

After that first `>`, the body is a stream of **payloads separated by `>`**.
Payloads are densely padded with `0x20`. Helpers:

- `split_ldb_records` → `Vec<LdbRecord>`
- `LdbRecord::space_stripped` / `lead_byte` for opcode probing
- CLI: `maxfx-engine path/to/file.ldb` (or `.ai`) prints record count + lead-byte histogram

### Observations (part0_level1)

| File | Size | `>` records (approx) | Notes |
|------|------|----------------------|-------|
| `.ai` | ~45 KiB | ~435 | Space-stripped leads include `0x89`, `0x14`, `0x55`, …; some records start `c7 …` then MemoryFile-like `14`/`09`/`16` |
| `.ldb` | ~15 MiB | ~238k | Same `>` framing; even more space-heavy early on |

**Not** a flat MemoryFile stream (skipping spaces still hits non-tag opcodes immediately).

### `.ai` lead probes (part0_level1)

| Lead | Approx count | Notes |
|------|--------------|-------|
| `0x89` | ~110 | often `89 6a 93 7a` magic then mixed/`14`/`16` tags (`is_ai_89_magic`) |
| `0x14` | ~72 | MemoryFile int tag as first dense byte |
| `0xc7` | ~6 | `c7 <sub> <tail…>`; helper `LdbRecord::as_ai_c7` |
| `0x80` | singleton rows | often 1-byte records |

Opcode / section semantics still need importer decompile beyond the version gate.

## Done / next

### Issue #12 (paths) — closed
- [x] Path layout inside `x_level*.ras`
- [x] Token list + Import* RTTI map (best-effort)
- [x] Shared preamble observation + engine helper
- [x] `maxfx-engine` lists `.ldb`/`.ai` pairs
- [x] `levels.txt` → archive path construction + parser/tests

### Issue #14 (chunks)
- [x] `>`-record split + space-strip probe + CLI histogram
- [x] REA: `R_MemoryFile::createChunk` / `openChunk` / `closeChunk` (`rlmfc.dll`)
- [x] REA: `X_LevelDBImportLevel` vtable + version gate (see below)
- [ ] Full opcode / section map inside records
- [ ] Typed section decode inside records

## `X_LevelDBImportLevel` (MaxPayne.exe REA)

| Item | Value |
|------|-------|
| RTTI | `.?AVX_LevelDBImportLevel@@` @ VA `0x887388` |
| COL | `0x7ed5d8` |
| vtable | `0x7d1ff8` |
| Slot 0 (version gate) | `0x5ec350` |

Slot 0 (disasm + IAT):

1. `operator>>(R_MemoryFile&, int&)` (`rlmfc`)
2. Compare to **`0x20`** (32)
3. On mismatch: `R_Exception` + `"X_LevelDB Wrong version, expected %d, found %d!"`
4. On match: virtual-calls child importers at `this+0x4`, `+0x8`, `+0xc`, `+0x14` with the same `R_MemoryFile*`

Constant: `maxfx_engine::LDB_IMPORT_VERSION` (`0x20`).

**Note:** BYOD `.ldb`/`.ai` roots still begin with space-padded `>` records (dense head
`3e …`, not `14 20`). The `0x20` gate almost certainly applies to a MemoryFile
view **after** record/chunk extraction, not to the raw file preamble. Bridging
`>`-records → MemoryFile for that gate is the next REA step.

## `X_LevelDBExportLevel` (MaxED.exe REA)

| Item | Value |
|------|-------|
| RTTI | `.?AVX_LevelDBExportLevel@@` |
| vtable | `0x67d3e4` |
| Write slot (`+4`) | `0x5ebe10` |

Write path:

1. `operator<<(R_MemoryFile&, int const&)` with value **`0x20`**
   (`rlmfc` `??6…ABH@Z` @ IAT `0x673e70`)
2. Virtual-calls child exporters at `this+0x4/8/c/14` **slot +4** (export)

So a MemoryFile-native `.ldb` from MaxED should begin `0x14 0x20` (1-byte int tag).
Retail RAS BYOD blobs do **not** — they use the space/`>` outer framing instead
(possible post-export pack, or a different cook path). Both observations stay in
the spec until a round-trip MaxED export fixture is captured locally.

## MemoryFile ChunkHeader (rlmfc REA, issue #14)

`createChunk(id, version)` writes:

| Field | Wire |
|-------|------|
| tag | `0x0c` (`writeTagged` BasicType) |
| id | `u32` LE |
| version | `u32` LE |
| size | `u32` LE — **total** chunk bytes including tag+header; patched by `closeChunk` |

`openChunk` does `readTag(0xc)` + `read(12)`, then payload length = `size - 0x0d`.

**BYOD `.ldb` / `.ai` root is not this framing.** Scans for valid `0x0c`+header on
`part0_level1.ai` / `.ldb` found **zero** rooted chunks. Top-level remains
space-padded `>` records; tag `0x0c` may appear nested later inside importer
payloads. Rust walker: `openpfx::RpsValue::Chunk` / `TAG_CHUNK`.

### Space padding note

Between `>` delimiters, `0x20` bytes act as padding around significant tokens
(non-space runs stay contiguous, e.g. `14 0a`). Space-stripping is the right
dense view for opcode probes; it is not a MemoryFile stream by itself
(lead bytes like `0xc7` / `0xcb` / `0xf7` before any `0x14` tags).
