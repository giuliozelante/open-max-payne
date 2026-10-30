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

Startup ImportLevel stream (~20 MiB) begins:

```
14 20                         # LDB_IMPORT_VERSION
<compressed int> N            # texture count (Startup: 150)
repeat N:
  0x0d path
  0x11 flag                   # often 4; 0 seen with non-JPEG blob
  <compressed int> len + raw  # often JPEG FFD8…FFD9
```

Helpers: `parse_ras_ldb_import_textures`, then
`parse_ras_ldb_import_materials_at` (`0x1f` list + `0x25` name/texture/index —
`X_LevelDBImportMaterial`). Startup nests a second `0x1f` list (no padding int). Immediately after:

**`X_LevelDBImportMaterialCategory`** (`parse_ras_ldb_import_material_categories_at`):
count int, then for each category: name, `member_count` (includes primary),
texture name, path, path2, two `0x0e` bools, then `member_count - 1` child
members (name, one or two paths, two bools). BYOD: Startup 9 cats @ `0x5b8d8d`
→ `0x5bb95f`; Tutorial 25 cats @ `0x7a7026` → `0x7aac8f`.

**`X_LevelDBImportLightmap`** (skip-walker `parse_ras_ldb_import_lightmaps_at`):
after categories, `count`, `0`, u8 `0`, prefix `0x12` + u32 `size`
(`256*256*3+18` on BYOD) + three u32s (`512,0,0`) + `01 00 01` + mark
`18 20`, then `count` payloads with sep `00 00 00 01 00 01 18 20` between
non-last blobs (full `size` incl. 18-byte trailer). **Last blob is RGB only**
(`size-18`) — exits/rooms resume immediately. Startup 45 → `0xe2bdf3`;
Tutorial 9 → `0x95ad7b`. **ImportExit** (`parse_ras_ldb_import_exits_at`): after lightmaps, records are
`id` int + `::room::ExitN` name, optionally a room AABB (`11 <n>` corners +
axis `0x16` + `0x1a` mat3 + pos), then nested `0x1c`/ints until the next exit.
Linked exits often alternate with/without AABB. Immediately after:

**Point stream** (`parse_ras_ldb_import_point_stream_at`, likely lights/triggers):
nest `0x1c` + ints, `0x12` even count, subtype int, then `count/2` bodies
(`15 f32 f32 15 f32 f32 03 i32 0e u8`; first lacks id). Startup 65107 bodies
→ `0xfdd873` (+ trailing bodies → Startup `0x1197022` / Tutorial `0xb8ea3b`).
**Static mesh-like** (`parse_ras_ldb_import_static_mesh_at`): form A `a`,`b`,`n`;
form B optional empty `0x1f` + `b`,`n` (`0x10` u16 for large `n`). Dual vec3
clouds + `0x1a` mat3 + pos. Startup #1 @ `0x1197022` (50 pts); #2 form B
36797 pts. **Mesh polys** (`parse_ras_ldb_import_mesh_polys_at`): hdr0 (=count)
+ hdr1 + `point_ref, verts(3..=16), normal, u8, 0, attr, f32, f32, 0, index`
(Startup #2 → 21081 polys). Optional **mesh attribs**
(`parse_ras_ldb_import_mesh_attribs_at`): `0x1f` + count + `(id, vec3)×count`
(not `0x25` materials) then next mesh. After meshes: Startup **character head**
(`parse_ras_ldb_import_character_head_at`) empty `0x1f` + `0` + count, then
`::…Jumppoint…` / room entity graph with dual `0x1a` xforms + `DO_*` hooks
(full walk still open). Tutorial: **lights** (`parse_ras_ldb_import_lights_at`)
count + dual name + dual `0x1a` + subtype + empty str + `0x19` mat3 + 10×`0x09`
(21 lights → teleports).

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
