# ParticleFX RPS / PSE — Working Spec (Phase 4)

**Status:** Draft. Rust `crates/openpfx`: `dump-pse`, `dump-rps`, `export-preamble` (header only).

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

Export implementation likely continues in `EditorDoc` / `E2MFC` / `RLMFC` (not fully decompiled yet).

RPS load errors (from ParticleFX.exe strings):

```
PS_ParticleSystem: Invalid File Version in %s
Current: %8x  Expected: %8x
PS_ParticleSystem: Invalid File Structure in %s
```

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
| `0x14` | 1 × u8 | uint8 channel A |
| `0x11` | 1 × u8 | uint8 channel B |
| `0x09` | 4 × LE f32 | float |
| `0x0d` | `14 <len>` + bytes | relative path string (e.g. `fire\candleflame2\pflame.JPG`) |
| `0x13` | u16_le length + bytes | embedded blob (often JPEG starting `FFD8`) |
| `0x16` | 3 × LE f32 | vec3 (very common: `(0,0,0)`, `(0,1,0)`, …) |

Common prefix (candle / light examples):

```
14 60 | 14 05 | 11 NN | 09 00 00 20 40 | 11 0a | 09 00 00 20 40 | …
```

i.e. u8(96), u8(5), u8(N), f32(2.5), u8(10), f32(2.5) — same *ideas* as the PSE header, packed as tags.
`openpfx dump-rps` walks until an unknown tag; `export-preamble` writes that six-value header from a PSE (graphs omitted).

## Next

1. REA: `-e` / `EditorDoc` export → full graph encode + version `%8x`
2. `openpfx export` parity with `psysedit -e` once version magic known
3. Retire Python `tools/openpfx`
