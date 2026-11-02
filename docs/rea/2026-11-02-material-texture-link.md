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

## Alpha masks (2026-11-02)

**VERIFIED (static):** the runtime material reader `0x5dc800` looks both strings up in the texture
container (`0x5ebc70` → map find `0x5e7ae0`). It passes the colour image and the second (alpha) image to
`0x5dc360` → builder `0x5dc3d8`. With an alpha image, the builder fetches both `G_Image`s (`0x600980`) and
constructs `P_Map::P_Map(const G_Image* colour, const G_Image* alpha)` (import `0x7ca48c`; e2mfc
`0x1001f100`), and sets `P_Map+0x84 = 1`. Without one it passes `(colour, 0)` and sets `+0x84 = 0`.
So the mask becomes the texture's alpha channel.

**VERIFIED (data):** in the `.ldb` import records, `path2` is the alpha image (`*_alpha.jpg`/`*_alpha.pcx`,
e.g. plant: `Graffiti\Plant01_128x128_alpha.pcx`). The mask images are embedded as texture blobs (PCX flag
`0x03`, JPEG flag `0x04`). The PCX masks checked (plant, needle, blood) have greyscale palettes, so
alpha = mask intensity.

**STRONGLY SUPPORTED:** records with an alpha mask store bool `+0x31 = 1` (`0e 01`), and the wallpaper
record stores `0e 00`.

**VERIFIED (game data, `data/database/materials.txt`):** each category sets `AlphaReference`
(default `ID_ALPHAREFERENCE` = 15; `[Leaves]` = 127) and `BlendedAlphaTest` (default TRUE; `[Leaves]` FALSE).

UNKNOWN: the exact comparison (`>` vs `>=` the reference), and how `BlendedAlphaTest` blends. e2mfc
`P_Material::enableAlphaTest` / `setAlphaReference` are the places to read next.
