# Material → texture link (MaxPayne.exe + `.ldb`)

Artifact: `/home/gzelante/Games/MaxPayne1/MaxPayne.exe` (PE i386, ImageBase `0x400000`).
Data: `Part0_Level1.ldb` (house), extracted under `/tmp/omp-xlevel1`.

## Classes (RTTI COL, offset 0)

| Class | vtable | slot 0 (read) | slot `+0x4` (write) |
|-------|--------|---------------|---------------------|
| `X_LevelDBImportMaterial` | `0x7d2160` | `0x5dc800` | `0x5dc7b0` |
| `X_LevelDBMaterial` | `0x7d26ac` | `0x5dc800` | `0x5dc7b0` |
| `X_LevelDBImportTextureImage` | `0x7d2570` | `0x5db3e0` → `0x600860` | `0x5db3f0` → `0x600820` |

## Material record (VERIFIED)

The writer `0x5dc7b0` emits, via `operator<<(R_MemoryFile&, const std::string&)` (import `0x7cab80`) and
`operator<<(…, const bool&)` (`0x7caa7c`):

1. `0x72f650()` → `+0xc`: the material **name** (shared name accessor)
2. `0x5dc710()` → `+0x1c`: the **texture file** the material uses
3. bool `+0x31`, bool `+0x32` (meaning UNKNOWN)

The reader `0x5dc800` fills the same fields (`+0xc`, `+0x1c`, `+0x31`, `+0x32`).

On disk these are the members of the import "material categories" section (engine:
`parse_ras_ldb_import_material_categories_at`). Each member is `(name, path, path2, bool, bool)`, and a
category's first member is stored inline (`texture`, `path`, `path2`). Example at file `0x772779`:
`"MAXHOUSEWALL01_512X256.JPG"` → `"X:\Projects\Textures\MaxHouse2\MaxHouseWall01_512x512.jpg"` → `""` → `0e 00` → `0e 00`.

## Resolution rule (VERIFIED on the house level)

- A material in the `0x1f` material lists carries only `(category, name, id)`.
- Its texture is the record whose name equals the material name: use the record's `path` when it is
  non-empty, otherwise the name itself is the file.
- Name-only matching (the engine before this fix) left 23/137 materials untextured. The record `path`
  resolves all 23 to embedded textures. Examples:
  - `MAXHOUSEWALL01_512X256.JPG` → `MaxHouseWall01_512x512.jpg` (the hallway wallpaper, 407 polys)
  - `X:\…\CARPET01A.JPG` → `Concrete01_128x128.jpg` (the name and the file differ completely)
  - `LAMP20-CENTER_B.JPG` → `Lamp20-center_b2.jpg`

## `path2` = alpha mask (STRONGLY SUPPORTED)

Non-empty `path2` values are all `*_alpha.jpg` / `*_alpha.pcx` (e.g. `Curtains01_128x256_alpha.jpg`,
`V&Needle02_128x128_alpha.pcx`). HYPOTHESIS: this is the alpha (cutout) channel for the texture. Not used
yet; it's the next materials step (plants and curtains render as opaque cards today).
