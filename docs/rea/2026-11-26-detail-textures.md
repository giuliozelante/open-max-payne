# Material detail textures

Retail 1.05 `MaxPayne.exe`, `objdump -d -M intel`. Addresses are VAs (ImageBase `0x400000`).
Issue #187. Data: `x_data.ras` → `data/database/materials.txt`.

## Keys and the surface record

**VERIFIED.** The `materials.txt` key table is filled at `0x52d9f2`–`0x52db02`: each entry pairs a
key string with the scratch field the parser writes.

| Key | String | Scratch field |
|---|---|---|
| `AvailableInDetailLevels` | `0x8808d8` | `0x8ad5a0` (int) |
| `DetailTilesPerMeter` | `0x880f0c` | `0x8ad59c` (float) |
| `DetailColorTextureName` | `0x880ef4` | `0x8ad49c` (256-byte string) |
| `DetailAlphaTextureName` | `0x880edc` | `0x8ad39c` (256-byte string) |
| `BlendedAlphaTest` | `0x880ec8` | `0x8ad398` (bool) |

`0x52dba0` copies the scratch fields into the category's surface record (0x55 bytes, one per
category, array at owner `+0x8`, built by `0x52ed71`–`0x52ee47`):

| Offset | Field | Accessor |
|---|---|---|
| `+0x2b` | `AvailableInDetailLevels` | `0x52e3a0` |
| `+0x2f` | has detail (bool) | `0x433940` |
| `+0x30` | `DetailTilesPerMeter` | `0x74d340` (`fld [ecx+0x30]`) |
| `+0x34` | `DetailColorTextureName` (`std::string`) | `0x7269d0` |
| `+0x44` | `DetailAlphaTextureName` (`std::string`) | `0x52e390` |

**VERIFIED.** `+0x2f` is set at `0x52defe`–`0x52df1d` from the two name lengths (`+0x3c`,
`+0x4c`): both empty clears it, both non-empty sets it, one of the two empty jumps to the error
path at `0x52e0eb`. A category draws a detail layer only when it names both images.

## When the layer is drawn

**VERIFIED.** `0x40c8ff`–`0x40c90a` reads the `Detail Texture` registry value (`0x404220`, key
string `0x866068`, under `Advanced Settings`) and stores `value == 2` in `0x8b0757` through
`0x5ac880`. `0x5ac890` returns that byte. Only the top slider position turns detail textures on.

**VERIFIED.** `AvailableInDetailLevels` is not that gate. `0x52e3a0` returns
`surface+0x2b <= 0x880eb0`, and `0x880eb0` is written once, at `0x4169ff`, with the
`Material Variations` registry value (`0x404130` → `0x65a400` → `0x65a3f0`). Its two readers
(`0x52c3a2`, `0x534bff`) sit in the material lookup, not in the renderer.
**HYPOTHESIS:** it picks whether a category's variation is used at the current
`Material Variations` level; it does not affect detail drawing.

## Material classes

**VERIFIED.** `0x5a6220` chooses one of 14 prebuilt materials per polygon from lightmap,
alpha-test and detail flags; detail is chosen only when `0x5ac890()` and the surface's `+0x2f`
are both true. The builder around `0x5a5705` creates the materials and records per class whether it has a lightmap
(`0x8b068c[i]`), alpha (`0x8b069c[i]`) and detail (`0x8b06ac[i]`) layer.

Class 1 (`0x8b071c`, lightmap + detail), `addTextureLayer(texture, uv, blend, filter, address)`
at `0x5a57fe`–`0x5a5838`:

| Layer | Texture | UV set | Blend | Filter |
|---|---|---|---|---|
| 0 | 0 (base) | 0 | `COPY` | default (`0x5a61d0`) |
| 1 | 2 (detail) | 2 | `TEXTURE_ALPHA` (`0x800000`) | default |
| 2 | 1 (lightmap) | 1 | `MUL` (`0x400000`) | `0x5000000` |

Blend names come from `docs/rea/2026-11-02-lighting-layers.md`. **STRONGLY SUPPORTED:**
`TEXTURE_ALPHA` lerps toward the detail colour by the detail texel's alpha, so
`base' = mix(base, detail.rgb, detail.a)`, then the lightmap multiplies the result. The
alpha image therefore sets how strongly the detail shows; it is not a cut-out mask.

## The detail image

**VERIFIED.** `0x5a64de`–`0x5a6669` runs when the class has a detail layer. It keys a cache
(`0x8b06c8`) by the pair of names, builds the `P_Map` from the colour image (`+0x34`) and the
alpha image (`+0x44`) with `0x5b3fd0`, validates it (`P_Map::validate`, `0x7ca5d4`), and calls
`setTexture` on stage 2 when the class has a lightmap, stage 1 otherwise. Paths in
`materials.txt` are relative (`..\..\sharedtextures\…`); the colour and alpha pair follows the
same colour + alpha `P_Map` rule as level textures.

## Detail UVs

**VERIFIED (structure).** `0x5a7200`–`0x5a7ed7` writes the detail UVs per polygon:

1. `0x5a7285`: `k = surface+0x30` (`DetailTilesPerMeter`).
2. `e1 = P1 − P0`. Among the other vertices it keeps the edge `eb = Pb − P0` with the smallest
   `|cos(e1, eb)|` (`0x5b3cc0`), and `n = normalize(eb × e1)`.
3. `W = [e1; eb; n]` (rows), inverted (`0x5b3d20` determinant, `0x43de20` inverse, identity when
   singular). `U = [uv1 − uv0, 0; uvb − uv0, 0; 0, 0, 1]` from UV set 0 (`0x5f19d0`, vertex
   record `+0x4`). `M = W⁻¹ · U` maps a world delta to a base-UV delta.
4. `M⁻¹` row lengths are the world lengths of one base-UV unit along u and v. Scales:
   `su = |row0(M⁻¹)| · k`, `sv = |row1(M⁻¹)| · k` (`0x5a7b8f`–`0x5a7bae`).
5. Per vertex: `(u, v) = (Pi − P0) · M + P0 · M`, then `detail = (u · su, v · sv)`
   (`0x5a7d5a`–`0x5a7d6d`).
6. The polygon's detail UVs are shifted by a whole number so their minimum sits near 0
   (`0x5aca20`, `0x5a7e80`–`0x5a7ead`); the wrap hides the shift.

**STRONGLY SUPPORTED:** for a planar-mapped polygon this is `detail_uv = base_uv ·
(world length per base-UV unit) · DetailTilesPerMeter` per axis, so the detail grain has a fixed
world density and follows the base texture's axes. **HYPOTHESIS:** level units are metres
(as `DetailOffset` already assumes in `render3d.rs`). **UNKNOWN:** the vertex-record slot the
result is written to (`0x5f19f0` returns `+0xc`), and filter defaults per class.

## Ruled out

- `0x5569f0` / `0x556a92` (fixed `MUL` layer) and `0x5568xx` (texture projection, 0.5 bias
  matrix): projected-texture paths, not detail.
- `0x8b0754` / `0x8b0758`–`0x8b0760` (+59, wrap at 192): a colour-cycling debug path in the same
  routine, off in normal play.

## Golden data still needed

Detail UVs for one wall and one floor polygon in `Part0_Level1`, read with winedbg at
`0x5a7d6d` with `Detail Texture = 2`, to confirm step 5 and the unit scale before the engine
uses it.
