# ParticleFX RPS / PSE — Working Spec (Phase 4)

**Status:** Draft. Rust `crates/openpfx`: `dump-pse`, `dump-rps`, `export-preamble`,
`MemoryFileWriter` (RLMFC tag parity). Full graph encode still WIP.

**REA artifacts:**
- `ParticleFX.exe` sha256 `3ce05189…69c7`
- BYOD `x_data.ras` → `\data\database\particles\**\*.rps` (mode 3 stored; not in git)

## Files

| Ext | Role |
|-----|------|
| `.pse` | ParticleFX **editor** project (paths, graphs, emitter params) |
| `.rps` | **Runtime** particle set cooked for the game |

## CLI (from ParticleFX.exe strings + help builder `FUN_00405a10`)

```
psysedit [-h] [-y] [-r] [-e filename(s)] [-d directory]
  -h   Display this help
  -y   Allow overwrite of existing files
  -r   Override readonly flag of existing files
  -e   Export files (.pse -> .rps)   # filename may contain wildcards; not recursive
  -d   Export destination directory (must exist)
```

## Export write path (REA, issue #11)

`ParticleFX.exe` `-e` lands in `FUN_0040d1d0`:

1. File dialog filter `Runtime Particle Sets (*.rps)|*.rps||` (+ overwrite/`-y`/`-r` gates)
2. `R_MemoryFile::R_MemoryFile(buf, 0x2000)`
3. `FUN_0041ab00(mf, particleSystem*)` — serialize root + graphs
4. `R_MemoryFile::save(mf, path)`

### Object writers (callee chain)

| Addr | Role | Header pair (`operator<<(mf,int*)` ×2) |
|------|------|----------------------------------------|
| `FUN_0041ab00` | Particle system root | type `0x60`, ver `5` then uint/float fields |
| `FUN_00419be0` | Graph section | type `0x31`, ver `1`, then param block, then **emitter-count** int |
| `FUN_00414d20` | Shared param block | type `0x20`, ver `3` |
| `FUN_00417570` | Emitter / node | type `0x30`, ver `4` (+ paths, curves) |
| `FUN_00418fe0` | Companion block | type `0x40`, ver `2` |
| `FUN_00415d00` | Companion inner | type `0x21`, ver `5` (+ curves/vec3) |
| `FUN_00432fd0` | Sampled curve | type `0x71`, ver `3` then f32 samples |
| `FUN_00418190` | Embed texture | `operator<<(CString path)` + size/`MemoryFile::write` → str + blob |

`FUN_00418190(mf, path, flag)`: loads file via `R_File::load`, writes relative path,
then either embeds bytes (`flag==0`) or writes int `0` (no embed).

### `RLMFC.DLL` `R_MemoryFile::writeTagged` (REA)

`writeTag(BasicType)` writes the type enum as **one byte** (when tags enabled),
then `write` copies `size` payload bytes.

**Signed `int` compression** (`operator<<(mf, int const&)` @ `rlmfc` `0x10017a10`):

| Tag | Payload bytes | When |
|-----|---------------|------|
| `0x14` | 1 | value fits in 1 byte |
| `0x13` | 2 | else fits in 2 |
| `0x12` | 3 | else fits in 3 |
| `0x02` | 4 | full int |

**Unsigned `uint` compression** (`0x10017a90`):

| Tag | Payload bytes |
|-----|---------------|
| `0x11` | 1 |
| `0x10` | 2 |
| `0x0f` | 3 |
| `0x03` | 4 |

Other fixed tags: `float` → `0x09` + 4; `vec2` → `0x15` + 8; `vec3` → `0x16` + 12;
`vec4` → `0x17` + 16; `signed char` → `0x07` + 1.

**Embedded file** (`FUN_00418190`): compressed int length then raw
`MemoryFile::write` (untagged). Walker promotes length+raw → `RpsValue::Blob`
when the next byte is not a known MemoryFile tag (JPEG `FFD8…` common).

Retail candle stays in the 1-byte int/uint regime (`0x14`/`0x11`).

Mirrored in Rust: `openpfx::MemoryFileWriter` + `parse_rps` multi-byte int/uint
tags (`0x13`/`0x12`/`0x02`, `0x10`/`0x0f`/`0x03`).

**Chunk** (`createChunk`): tag `0x0c` + 12-byte header (`id`, `version`, `size`);
`size` includes tag+header (`payload = size - 0x0d`). Used by LevelDB importers;
RPS particle exports observed so far stay in the flat tagged regime.

### PSE vs RPS object headers

| Format | Object type/version |
|--------|---------------------|
| `.pse` (editor) | Untagged LE `u32` pairs (`0x30`/`4` emitter, `0x71`/`3` curve, …) + `u8`-len names |
| `.rps` (runtime) | MemoryFile int tags (`0x14 0x30`, `0x14 0x04`, …) |

`scan_pse_emitters` harvests emitter names from the PSE raw pattern for export WIP.

**Curves:** PSE stores editor keyframes (name + control points after `0x71`/`3`).
RPS stores a **sampled grid** written by `FUN_00432fd0` (channels × samples f32s)
via evaluator `FUN_00432240`. Export stubs currently emit placeholder samples;
full `-e` parity needs that sampler (or a fixture capture from retail `-e`).

### Curve object offsets (`FUN_00432fd0` / `FUN_00432240`, ParticleFX.exe)

| Off | Role |
|-----|------|
| `+0x08` | channel count (`int`) |
| `+0x0c` | sort-channels flag |
| `+0x30` / `+0x34` | sample time range min / max (`float`) |
| `+0x4c` | sample count along time |
| `+0x54`… | keyframe container (STL-ish; iterators `FUN_00410410` / `FUN_00414000`) |
| `+0x68` | keyframe count |

Per keyframe (through list cursor): time at `*node`, channel values at
`node+4+channel*4`. Sampler walks keyframes until `time >= t`, then interpolates
(`FUN_00431dd0` / `FUN_004320b0` / `FUN_00433b60`).

### Version / structure gate (load)

`FUN_00422050` (`operator>>` pair):

```
read int → must be 0x60   else Invalid File Structure
read int → must be 5      else Invalid File Version (format uses %8x)
```

Strings: `PS_ParticleSystem: Invalid File Version in %s\nCurrent: %8x  Expected: %8x`
and `T_Graph: Invalid File Version…`.

REA: the `T_Graph` version string is referenced from `FUN_00424410`, which gates
**type `0x71`, version `3`** — i.e. the **curve** object (`FUN_00432fd0` writer),
not the `0x31` particle-graph section. Editor “graphs” are curve UIs.

**Expected root magic = type `0x60`, version `5`.** Same values PSE flat header word0/word1.

## PSE flat header

| Off | Type | Notes |
|-----|------|-------|
| 0 | u32 | often `96` (`0x60`) — load-gate type |
| 4 | u32 | often `5` — load-gate version |
| 8 | u32 | varies (e.g. `35` / `50`) → RPS uint |
| 12 | f32 | e.g. `2.5` |
| 16 | u32 | e.g. `10` — **not** a float → RPS uint |
| 20 | f32 | e.g. `2.5` |
| 24 | u32 | e.g. `10` → RPS uint (`FUN_0041ab00` field) |
| 28 | u32 | flag → RPS int |
| 32 | u32 | flag → RPS int |
| later | mixed | gravity / paths / graphs (WIP) |

`openpfx export-root` emits the `0x60`/`5` object + these fields (no graphs).

## RPS tagged stream (BYOD)

Retail `.rps` files are a **tag-byte stream** (Remedy `R_MemoryFile`-style), not the PSE flat u32 layout.

Observed tags (consistent across multiple `x_data.ras` samples):

| Tag | Payload | Role |
|-----|---------|------|
| `0x14` | 1 × u8 | int channel (`operator<<(int*)`) |
| `0x11` | 1 × u8 | uint channel |
| `0x09` | 4 × LE f32 | float |
| `0x0d` | `14 <len>` + bytes | relative path string |
| `0x13` | u16_le length + bytes | embedded blob (often JPEG `FFD8`) |
| `0x16` | 3 × LE f32 | vec3 |

Common prefix (candle):

```
14 60 | 14 05 | 11 NN | 09 00 00 20 40 | 11 0a | 09 00 00 20 40 | …
```

BYOD `fire_candle_flame_large_1.rps` (2347 bytes): walker consumes **entire** file
(312 values); `encode_rps` round-trips byte-identical (local probe, not in git).

`openpfx dump-rps` walks until unknown tag; `export-preamble` writes the six-value
header from a PSE (graphs omitted). Full `-e` parity = stream the object writers above.

## Next

1. Encode emitter/curve/string/blob bodies to match candle (finish `FUN_00417570`)
2. `openpfx export` end-to-end vs BYOD fixture
3. Retire Python `tools/openpfx` if any remains
