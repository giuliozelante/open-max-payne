# Remedy Archive System (RAS / MPM) — Format Spec

**Status:** Verified against `rl.dll` (`R_File::decryptWithSeed`, `R_File::loadArchiveHeader`, `R_File::decompress`) via REA/Ghidra 12.1.2 and fixture `Basic Room Example Mod.mpm` (plus a retail `x_level1.ras` during REA — not kept in-repo).

**Endianness:** Little-endian.  
**Extensions:** `.ras` (game data) and `.mpm` (mod packages) share the same container; only naming differs.

## On-disk layout

```
[ RASHeader 44 bytes ]
[ File table   — encrypted, size = file_table_size ]
[ Dir table    — encrypted, size = dir_table_size  ]
[ File payloads in table order — no padding ]
```

### RASHeader (44 / `0x2C` bytes)

| Off | Type | Field |
|-----|------|--------|
| 0x00 | `char[4]` | Magic `RAS\0` (`52 41 53 00`) |
| 0x04 | `int32` | Encryption seed (0 treated as 1) |
| 0x08 | `uint32` | File count *(encrypted)* |
| 0x0C | `uint32` | Directory count *(encrypted)* |
| 0x10 | `uint32` | File table size in bytes *(encrypted)* |
| 0x14 | `uint32` | Directory table size in bytes *(encrypted)* |
| 0x18 | `float32` | Archiver version; Max Payne 1 tools require `1.2` (`0x3F99999A`) *(encrypted)* |
| 0x1C | `uint32` | Header CRC-32 *(encrypted; see below)* |
| 0x20 | `uint32` | File-table CRC-32 *(encrypted)* |
| 0x24 | `uint32` | Dir-table CRC-32 *(encrypted)* |
| 0x28 | `uint32` | Format compatibility: `3` = Max Payne 1, `4` = Max Payne 2 *(encrypted)* |

Bytes `0x08..0x2B` (36 bytes) are encrypted with `decryptWithSeed` using the seed at `0x04`.

#### Header CRC

After decrypting the 36-byte block:

1. Save `header_crc` at `0x1C`.
2. Write `0` at `0x1C`.
3. Compute IEEE CRC-32 (zlib / ISO 3309 poly) over the full 44-byte header (including magic + seed).
4. Must equal saved `header_crc`.

File/dir table CRCs are CRC-32 of the **decrypted** table blobs.

## Encryption — `R_File::decryptWithSeed` (`rl.dll` `0x10012b60`)

```c
void decryptWithSeed(uint8_t *buf, int count, int32_t seed) {
    if (seed == 0) seed = 1;
    for (int i = 0; i < count; i++) {
        uint8_t r = (uint8_t)(i % 5) & 7;
        buf[i] = (buf[i] << r) | (buf[i] >> (8 - r));   // rotate left
        seed = seed * 0xAB + (seed / 0xB1) * -0x763D;   // MSVC toward-zero div
        buf[i] = (uint8_t)((((int8_t)i + 3) * 6) ^ buf[i]) + (int8_t)seed;
    }
}
```

Constants: `0xAB=171`, `0xB1=177`, `0x763D=30269`.

**Encrypt** (inverse, same seed schedule):

```c
void encryptWithSeed(uint8_t *buf, int count, int32_t seed) {
    if (seed == 0) seed = 1;
    for (int i = 0; i < count; i++) {
        seed = seed * 0xAB + (seed / 0xB1) * -0x763D;
        uint8_t x = (uint8_t)((buf[i] - (int8_t)seed) ^ (((int8_t)i + 3) * 6));
        uint8_t r = (uint8_t)(i % 5) & 7;
        buf[i] = (x >> r) | (x << (8 - r));             // rotate right
    }
}
```

File table and directory table each restart from the **original** archive seed (not a continued keystream).

## Directory table entry

```
char name[];          // NUL-terminated, full path with backslashes, e.g. "\data\menu\"
                      // Root is "\"
uint8_t timestamp[16]; // FILETIME-like / DOS-ish stamp used by tools (opaque to extractor)
```

## File table entry

```
char name[];              // NUL-terminated basename only
uint32_t uncompressed_size;
uint32_t compressed_size;
uint32_t unknown_0;       // observed 0
uint32_t dir_index;       // 0-based index into directory table
uint32_t unknown_1;       // observed 0
uint32_t compression_mode; // 1 = LZSS, 3 = stored (uncompressed)
uint8_t timestamp[16];
```

Payloads follow immediately after both tables, in file-table order. Offset of file *i* = sum of `compressed_size` of files `0..i-1`.

### Compression modes

| Mode | Meaning |
|------|---------|
| 3 | Stored — `compressed_size == uncompressed_size`, raw bytes |
| 1 | Okumura-style LZSS: 4 KiB ring, fill `0x20`, write pos `0xFEE`, flag bytes, match len = `(low_nibble)+3` |

`R_File::decompress` (`0x10012fa0`) implements the LZSS decode used for mode 1.

## RASMaker CLI (1.20 / Jul 4 2001)

REA strings from `vendor/max-fx-tools/Rasmaker/RASMaker.exe` (sha256 `0020b019…884c`):

```
Usage: RASMaker [options] [source] [destination]
  -x  extract files          → cwd (creates data\… from archive paths)
  -v  view archive contents
  -a  add files
  -m  move files (deletes from source after add)
  -c  compress all files (requires -a or -m)
  -p  exclude source path (requires -a or -m)
```

Examples from the binary:

| Invocation | Meaning |
|------------|---------|
| `-a data data.ras` | Pack `data\`, **keep** `data\` in entry names (source must be relative) |
| `-a -p e:\temp data.ras` | Pack under temp, **strip** that path from names |
| `-x data.ras` | Extract into current directory |
| `-v data.ras` | List |

Absolute source without `-p` → error: `Source path must be relative when "-p" option is not used.`

**openras** accepts the same flag form (`openras -v …`) in addition to subcommands `list` / `extract` / `pack`.

## Evidence

| Artifact | Role |
|----------|------|
| `vendor/max-fx-tools/Rasmaker/rl.dll` | Crypto, CRC, LZSS, header load |
| `vendor/max-fx-tools/Rasmaker/RASMaker.exe` | Thin CLI front-end |
| `vendor/fixtures/Basic Room Example Mod.mpm` | 3 files, all mode 3; header/table CRC match |
| Retail `x_level1.ras` (game install; not in repo) | Large MP1 level archive (compat 3, ver 1.2) |
