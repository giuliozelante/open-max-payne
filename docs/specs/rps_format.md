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
| `FUN_00419be0` | Graph section | type `0x31`, count |
| `FUN_00414d20` | Shared param block | type `0x20`, ver `3` |
| `FUN_00417570` | Emitter / node | type `0x30`, ver `4` (+ paths, curves) |
| `FUN_00418fe0` | Companion block | type `0x40`, ver `2` |
| `FUN_00432fd0` | Sampled curve | type `0x71`, ver `3` then f32 samples |
| `FUN_00418190` | Embed texture | `operator<<(CString path)` + size/`MemoryFile::write` → str + blob |

`FUN_00418190(mf, path, flag)`: loads file via `R_File::load`, writes relative path,
then either embeds bytes (`flag==0`) or writes int `0` (no embed).

`RLMFC.DLL::operator<<` overload → on-disk tags:

| C++ arg | Tag | Payload |
|---------|-----|---------|
| `int*` | `0x14` | u8 |
| `uint*` | `0x11` | u8 |
| `float*` | `0x09` | LE f32 |
| `M_Vector3Template<float>*` | `0x16` | 3 × LE f32 |
| string (BYOD) | `0x0d` | `14 <len>` + bytes |
| blob (BYOD JPEG) | `0x13` | u16_le len + bytes |

Mirrored in Rust: `openpfx::MemoryFileWriter` + `openpfx::rps_obj::*`.

### Version / structure gate (load)

`FUN_00422050` (`operator>>` pair):

```
read int → must be 0x60   else Invalid File Structure
read int → must be 5      else Invalid File Version (format uses %8x)
```

Strings: `PS_ParticleSystem: Invalid File Version in %s\nCurrent: %8x  Expected: %8x`
and `T_Graph: Invalid File Version…` (graph object gate).

**Expected root magic = type `0x60`, version `5`.** Same values PSE flat header word0/word1.

## PSE flat header

| Off | Type | Notes |
|-----|------|-------|
| 0 | u32 | often `96` (`0x60`) |
| 4 | u32 | often `5` |
| 8 | u32 | varies (e.g. `35`) |
| 12 | f32 | e.g. `2.5` |
| 16 | u32 | e.g. `10` — **not** a float |
| 20 | f32 | e.g. `2.5` |
| later | ASCII | absolute tool/bitmap paths |

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
