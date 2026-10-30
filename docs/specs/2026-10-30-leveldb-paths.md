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

## `.ldb` / `.ai` layouts (BYOD RAS extract, issue #14)

**Correction (2026-10-30):** the earlier “16 spaces + `>` then `>`-records” reading is
**not** what retail RAS level blobs use. Front-end extracts
(`Startup_level.ldb`, `Tutorial.ldb`/`.ai`, …) show:

### Retail RAS `.ldb`

Top-level sections share tag byte **`0x1c`** + kind byte + LE `u16` count:

| Kind | Bytes | Body |
|------|-------|------|
| `0x12` | `1c 12 NN NN FF` | `FF` flag + contiguous `0x16` vec3 run. Flag 0 ⇒ `NN` equals run length; flag 1 (Tutorial) ⇒ run is longer than `NN` — always advance by the contiguous run. |
| `0x13` | `1c 13 NN NN` | No flag byte; MemoryFile tags (`14 …`) — Startup section-2. |

`LDB_RAS_MAGIC` = LE u16 for `1c 12`. Helpers: `parse_ras_ldb_section_at`,
`parse_ras_ldb_header`, `count_ras_ldb_leading_vec3`, `ras_ldb_count_section_end`.

Kind `0x13` body (Startup): repeating face records; `count` equals the number
of faces (19149). `parse_ras_ldb_faces` consumes the whole section:

```
<compressed int> × 3   # vertex indices into the 0x12 cloud (0x14 / 0x13 / …)
<compressed int>       # attr (often 804/805)
0x16 vec3 × 2          # normal + point
```

After the face section, Startup continues with another `1c 13` (count=7748)
of **surf** records (`parse_ras_ldb_surfs_ex`):

```
0x16 vec3   # normal
0x16 vec3   # point
<compressed int> × 6
```

Kind `0x13` = MemoryFile payload; try schemas in order: **faces**, **surfs**,
**int list** (`parse_ras_ldb_ints_ex` — Startup §4 has 18944 ints).

### ImportLevel stream (after geometry)

On Startup, after the last `0x1c` section (`ras_ldb_after_geometry`), the file
continues as a flat MemoryFile stream beginning **`14 20`**
([`LDB_IMPORT_VERSION`]) then more ints/strings/JPEG embeds — e.g. absolute
texture path `\\HYPERION\MAX\PROJECTS\TEXTURES\…JPG` + `FFD8` blob. This is the
bridge from RAS geometry packing to the MaxED ImportLevel gate.

Startup ImportLevel stream (~20 MiB): ~2888 printable `0x0d` paths, ~341 JPEG
`FFD8` markers (`ras_ldb_import_paths`). Typed ImportBSP/Character/… decode still
open.

### Retail RAS `.ai`

Starts as a MemoryFile tag stream (`0x14 …`), e.g. Tutorial.ai:
`14 7b 14 0a 14 00 14 00 09 …` (ints 123 / 10 / 0 / 0, then floats).
Helper: `looks_like_memory_file_ai`. **Not** the ImportLevel `0x14 0x20` gate.

Caveat: a naïve `>` split mis-fires on float bytes (`…99 99 3e` = 0.3f).

### Legacy space/`>` probe

`LDB_PREAMBLE` + `split_ldb_records` remain for synthetic tests / any still-unseen
cook path. Do not treat them as the retail RAS layout.

### MaxED MemoryFile `.ldb`

Tagged int `0x14 0x20` (`LDB_IMPORT_VERSION`) — see ExportLevel below.

## Done / next

### Issue #12 (paths) — closed
- [x] Path layout inside `x_level*.ras`
- [x] Token list + Import* RTTI map (best-effort)
- [x] Shared preamble observation + engine helper
- [x] `maxfx-engine` lists `.ldb`/`.ai` pairs
- [x] `levels.txt` → archive path construction + parser/tests

### Issue #14 (chunks)
- [x] Retract false space/`>` primary framing; document RAS `0x121c` + MF `.ai`
- [x] REA: `R_MemoryFile::createChunk` / `openChunk` / `closeChunk` (`rlmfc.dll`)
- [x] REA: `X_LevelDBImportLevel` vtable + version gate (see below)
- [ ] Map post-vec3 `.ldb` body → ImportLevel / ImportBSP
- [ ] Full MemoryFile walk of `.ai` (fix early blob false-positive)

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

**Note:** Retail RAS `.ldb` roots use magic `0x121c` + vec3 run, not `14 20`.
The ImportLevel `0x20` gate likely applies to a nested MemoryFile section (or
MaxED-native exports), not the raw RAS `.ldb` head. Locating that nested stream
inside the post-vec3 body is the next REA step.

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

The 16-space + `>` preamble is **not** a string constant in `MaxPayne.exe`,
`MaxED.exe`, or `rlmfc.dll` (byte search negative). It is produced at write time
by an as-yet-unlocated cook/pretty path — not by `ExportLevel`'s tagged `0x20`
writer alone.

### `R_MemoryFile::AccessMethod` (rlmfc.dll)

| Symbol | Notes |
|--------|-------|
| `getDefaultAccessMethod` / `setDefaultAccessMethod` | DLL global `@ VA 0x1003df4c`, **default `1`** |
| `readTagged` / `writeTagged` | Tagged BasicType channel used by `operator<<`/`>>` |

Default `1` is consistent with the tagged MemoryFile regime (`0x14`/`0x11`/…).
No evidence yet that another AccessMethod emits the space/`>` BYOD outer framing —
that cook still looks post-export (RAS pack or unlocated pretty-writer).

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
