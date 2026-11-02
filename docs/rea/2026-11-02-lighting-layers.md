# Lighting: material layer blending (MaxPayne.exe + e2mfc.dll)

Goal: replace the engine's guessed lightmap math (`lightmap/255 × 1.65`, clamped `0.22..1.4`, in
`render3d::rasterize_tri`) with the retail combination.

## Layer flags (VERIFIED, e2mfc `P_LayerArray::verbose()` `0x10022150`)

The debug dump prints a name for each flag in a layer word:

| Mask | Value | Name |
|------|-------|------|
| `0xe00000` (blend) | `0x000000` | `COPY` |
| | `0x200000` | `ADD` |
| | `0x400000` | `MUL` |
| | `0x600000` | `VERTEX_ALPHA` |
| | `0x800000` | `TEXTURE_ALPHA` |
| | `0xa00000` | `VERTEX_AND_TEXTURE_ALPHA` |
| `0x60000` (diffuse) | `0x20000` / `0x40000` / `0x60000` | `DIFFUSE_COPY` / `DIFFUSE_MUL` / `DIFFUSE_ADD` |
| `0x180000` (specular) | `0x80000` / `0x100000` | `SPECULAR_COPY` / `SPECULAR_ADD` |
| `0x18000 == 0x8000` | low bits `&7`, `>>3 &7` | `TEXTURE%d UV%d` |

`P_Material::addTextureLayer(uint, uint, BLENDING_MODE, FILTER_MODE, TEXTURE_ADDRESS_MODE)` is imported
by the exe at IAT `0x7ca5e0` (28 call sites).

## Level material builder (in progress)

`0x554d30` adds several layers:

- `0x556274`: base layer, blend `COPY` / `ADD` / `VERTEX_ALPHA` depending on `0x574660` / `0x77e0f0`
- `0x5563c2`, `0x556704`: further layers (detail?)
- `0x556a92`: a layer with fixed blend `MUL` (`0x400000`). HYPOTHESIS: the lightmap layer.

**Golden data (2026-11-02, winedbg, save loaded in the house):** breakpoints on the four layer calls
and on the builder's entry. Args are `(texture, uv, blend, filter, address)`:

| Site | Count | Args |
|------|-------|------|
| `0x554d30` entry | 42 | builder ran for 42 materials |
| `0x556274` | 18 | `0, 0, COPY, 0x1000000, 0` |
| `0x5563c2` | 11 / 15 / 2 | `0, 0, COPY` / `TEXTURE_ALPHA` / `ADD`, filter `0x5000000` |
| `0x556704` | 1 / 2 / 1 | `0, 0, COPY` / `1, 1, ADD` / **`1, 1, MUL`**, filter `0x5000000` |
| `0x556a92` | 0 | never hit |

So `0x554d30` is **not** the level-geometry material builder (42 materials vs 137 in the house's
`.ldb`), and its fixed-`MUL` site isn't the house lightmap. Only one material used a second texture on UV
set 1 with `MUL`, which is the classic lightmap layout. The exe imports `addTextureLayer`,
`addShadingLayer`, `enableLighting` and `setLightingColorSource`, but **not** `addLayer`.

## Level lightmap materials (VERIFIED, static, `0x5a56e0`…)

Global level material templates are built in `0x5a5xxx` (stored in `0x8b0718`, `0x8b071c`, `0x8b0720`, …).
They use `addShadingLayer` (IAT `0x7ca480`), `addTextureLayer` (`0x7ca5e0`), `enableLighting(1)`
(`0x7ca498`) and `setAmbientColor` (`0x7ca838`, a colour from `0x5a6140(0, 0, 0, 255.0)`). Textures are
set per stage with `setTexture` (`0x7ca5e4`, stages 0/1/2). Layers, from the pushed arguments:

- `0x8b0718`: `DIFFUSE_COPY` → tex 1 / UV 1 `COPY` → `DIFFUSE_ADD` → tex 0 / UV 0 `MUL`
  ⇒ **texture × (lightmap + vertex diffuse)**
- `0x8b071c`: `DIFFUSE_COPY` → tex 0 `COPY` → tex 2 / UV 2 `TEXTURE_ALPHA` → tex 1 / UV 1 `MUL`
  → `DIFFUSE_ADD` (associative)

The lightmap is texture 1 on UV set 1.

## `MUL` = `D3DTOP_MODULATE` (VERIFIED, `e2driver/e2_d3d8_driver_mfc.dll`)

The stage combiner at `0x10007c76` switches on `layer & 0xe00000`. In the `ADD` branch it stores
**7** (`D3DTOP_ADD`) into the colour-op slot `[esp+0x2a8]`. In the `MUL` branch (`0x10008082`) it stores
**4** (`D3DTOP_MODULATE`). There is no `MODULATE2X`/`4X`, so a lightmap texel of 128 halves the texture.

**Engine consequence:** with black ambient and no dynamic lights, a lightmapped texel is
`texture × lightmap / 255`. The engine's `× 1.65` gain and `0.22..1.4` clamp were guesses that
over-brighten. Vertex diffuse (dynamic lights) is a separate later step.

UNKNOWN (next):
1. Which layer is the lightmap, and its texture/UV set.
2. How `MUL` maps to Direct3D 8 (`D3DTOP_MODULATE` vs `MODULATE2X`), i.e. whether a lightmap texel of
   128 means ×0.5 or ×1.0. That's in e2mfc's device-dependent material code.
3. Golden data option: trigger `P_LayerArray::verbose()` in the running game (debug dump; output via
   `0x1002e760`) to print the real layer list of level materials.

## Static point lights → `P_PointLight` (VERIFIED, static)

`X_LevelDBStaticPointLight` (the `.ldb` point lights the engine parses as `ScenePointLight`) becomes a
`P_PointLight` in `0x5f9450` (constructor call at `0x5f9ee7`):

- `setFalloffRange(R)` with R from the record's `[vt+0x18]`. e2mfc clamps R to ≥ 0.1 (`0x100112c0`).
- colour `(r, g, b, a) × intensity` (`+0xa8`, default 1.0) → light `+0x118` (0..255 scale)
- `setFalloffCurve((0.01, 0.0, 20.0))` (constants `0x3c23d70a`, `0x41a00000`), stored at light `+0x140`

e2mfc `P_PointLight::getAttenuation()` (`0x10011310`) returns `(c0, c1/R, c2/R²)` (zero when R == 0).
The D3D8 driver (`0x1000b990`…) fills `D3DLIGHT8`: Type 1 (point), Diffuse = `+0x118 × 1/255`,
Specular = `+0x128 × 1/255`, light Ambient = 0, Position = `+0x80/+0x84/+0x88`, Range = R,
Attenuation0/1/2 = `getAttenuation()`. So, with Direct3D's fixed-function per-vertex lighting:

**diffuse(v) = Σ lights colour/255 · max(N·L, 0) / (0.01 + 20·(d/R)²), for d < R** (D3D clamps colour to 1).

## Ambient (VERIFIED, driver `0x100165c1`…)

Per lit object the driver sets `D3DRS_LIGHTING` (`0x89`) = 1 and `D3DRS_AMBIENT` (`0x8b`) from the
**material's** ambient colour (`[obj+0x54]`, packed to ARGB). The level templates set black ambient
(`0x5a6140(0,0,0,255)`); other templates use white (`255,255,255,255`, at `0x5a5f67`/`0x5a6051`). The object
material builder (`0x554d30`) sets ambient from `0x574940()` at `0x556eac`, a per-material value not
read yet.

## Open (golden data needed)

1. Object ambient: break at `0x556eac` and dump `[eax]` (G_FColor) per material while loading a save.
2. Do static point lights also add to lightmapped level polygons (the template has `DIFFUSE_ADD`), or are
   they excluded (light collision lists, `P_BaseObject::addLightCollision`)? Compare a wall near a
   lamp: lightmap only vs lightmap + point light.
