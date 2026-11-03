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

Per lit object the driver sets `D3DRS_LIGHTING` (`0x89`) = 1 and `D3DRS_AMBIENT` (`0x8b`) from
`[obj+0x54]`, packed to ARGB. **Correction (2026-11-02, see below):** `obj` is the `P_Scene`, not the material. The level templates set black ambient
(`0x5a6140(0,0,0,255)`); other templates use white (`255,255,255,255`, at `0x5a5f67`/`0x5a6051`). The object
material builder (`0x554d30`) sets ambient from `0x574940()` at `0x556eac`, a per-material value not
read yet.

## Object ambient and colour sources (2026-11-02)

**VERIFIED (dynamic, winedbg at `0x556eac`, save loaded):** all 60 object materials captured set ambient
`(255, 255, 255, 255)`, which is white.

**VERIFIED (static, driver `0x10017752`…`0x10017849`):** the driver toggles `D3DRS_AMBIENTMATERIALSOURCE`
(`0x93`), `D3DRS_DIFFUSEMATERIALSOURCE` (`0x91`), `D3DRS_SPECULARMATERIALSOURCE` (`0x92`) between 0
(`D3DMCS_MATERIAL`) and 1 (`D3DMCS_COLOR1`, the vertex colour), and `D3DRS_EMISSIVEMATERIALSOURCE`
(`0x94`) between 0/1/2, per material. One `SetMaterial` call site.

~~HYPOTHESIS: object materials use `COLOR1` for ambient~~ — **ruled out**, see next section.

## Colour source, `D3DMATERIAL8` and global ambient (2026-11-02, static)

**Colour-source word (VERIFIED, e2mfc `setLightingColorSource` `0x10025130`, driver `0x1001771a`…`0x10017849`).**
`P_Material+0x2c` holds four 2-bit fields. The driver maps each to one render state; field value 0 =
`D3DMCS_MATERIAL`, 1 = `D3DMCS_COLOR1`, 2 = `D3DMCS_COLOR2`:

| Bits | Render state |
|------|--------------|
| `0x03` | `D3DRS_AMBIENTMATERIALSOURCE` (`0x93`) |
| `0x0c` | `D3DRS_DIFFUSEMATERIALSOURCE` (`0x91`) |
| `0x30` | `D3DRS_SPECULARMATERIALSOURCE` (`0x92`) |
| `0xc0` | `D3DRS_EMISSIVEMATERIALSOURCE` (`0x94`) |

`D3DRS_COLORVERTEX` (`0x8d`) = 1 only when the word is non-zero. The `P_Material` constructor
(`0x100230d0`, store at `0x1002322f`) sets the word to 0.

**Who sets it (VERIFIED).** Only the exe calls it (IAT `0x7ca598`, 8 sites); e2mfc has no internal caller and
no other module imports it:

| Site(s) | Function | Value | Meaning |
|---------|----------|-------|---------|
| `0x65e01f`, `0x65e0b7`, `0x65e218`, `0x65e360` | `0x65dafb` (`X_Decal`, string "X_Decal: Multiple alpha files…") | 4 | diffuse = vertex colour |
| `0x4a8ab3` | `0x4a8aa0` (no direct caller; vtable) | 4 | diffuse = vertex colour |
| `0x59fab3` | `0x59fa61` | 5 | ambient + diffuse = vertex colour |
| `0x5a01bd`, `0x5a02e5` | `0x5a007d` (global templates, e.g. `0x8b00b8`) | 5 | ambient + diffuse = vertex colour |

The object material builder `0x554d30`…`0x557800` never calls it, so object materials keep **0**:
vertex colours do not enter lighting.

**`D3DMATERIAL8` (VERIFIED, driver `0x1001785f`…`0x100179c5`, the only `SetMaterial` call `[vt+0xa8]`).**
Diffuse / Ambient / Specular / Emissive = `P_Material` `getDiffuseColor` (`+0x44`) / `getAmbientColor` (`+0x34`) /
`getSpecularColor` (`+0x54`) / `getEmissiveColor` (`+0x64`) × 1/255 (`0x10063784`); Power = `getSpecularExponent`.
Constructor defaults: diffuse and specular 255, emissive 0.

**Global ambient (VERIFIED, static).** The object behind `[ebp-0x20]` at `0x10016608` is not the material
(the material arrives separately via `getDeviceMaterial` → `P_D3DMaterialInfo`). `P_Scene::getAmbientColor`
(`0x10003280`) returns `this+0x54`, so the same offset as the `D3DRS_AMBIENT` read. STRONGLY SUPPORTED:
that object is the `P_Scene`. The `P_Scene` constructor (`0x10018820`, `0x1001887b`…) sets ambient to
`(0, 0, 0, 255)`. Neither `P_Scene::setAmbientColor` (`0x100031e0`, `0x10003220`) has a caller or address
reference in e2mfc, and no module imports them. So `D3DRS_AMBIENT` stays **black**. **Superseded 2026-11-03:** the exe writes the field
directly; see "Level ambient" below.

**Engine consequence.** D3D fixed-function per vertex, for an object material (colour source 0):

    colour = emissive + ambient_mat × 0 + Σ lights diffuse_mat · light/255 · max(N·L, 0) · atten(d)

The white object ambient captured at `0x556eac` contributes nothing. Objects are lit by emissive plus
point lights only. The engine's directional `shade` (`render3d.rs`, non-lightmapped polygons) is a guess.

**Object diffuse/emissive (VERIFIED, static, `0x5560a5`… and `0x5561c5`…).** The builder reads a mode
from the runtime material at `+0x46` (`0x574970`): mode 1 → `setEmissiveColor(c)`, mode 2 →
`setDiffuseColor(c)`, where `c` is a colour argument. Other values set neither (emissive stays 0, diffuse 255).
Blend at `+0x56` (`0x5749d0`): 0 `COPY`, 1 `ADD`, 2 `MUL`. The colour `c` is the runtime material's `+0x26`
(`0x574950`); ambient is `+0x16` (`0x574940`).

## Object material record on disk (VERIFIED, static reader + KFS bytes)

Object materials come from **KF2/KFS** files, not the `.ldb`. In `x_data.ras` the parent chunk `0x1000f` occurs
in 305 `.kf2` and 157 `.kfs` files (e.g. `skins/young_max_payne/young_max_payne_L0.kfs` at offset 0) and in no
`.ldb`. Chunk header: tag `0x0c` + `id`, `version`, `size` (see `docs/specs/rps_format.md`).

`0x577570` reads chunk `0x1000f`: name string, `int count`, then `count` material sub-chunks (`0x83`-byte
runtime records). `0x574a30` reads each sub-chunk `0x10010`, in this order (tags: `0x0d` string, `0x0e` bool,
`0x14` int, `0x09` float):

| # | Field | Runtime / setter |
|---|-------|------------------|
| 1 | name | `0x6dfb30` |
| 2–6 | 5 bools | `0x5748e0`, `0x77b190`, `0x574900`, `0x574910`, `0x77c050` |
| 7 | int **mode** | `0x574980` → `+0x46` |
| 8 | int | `0x5749a0` → `+0x4a` |
| 9 | int | `0x576410` |
| 10 | 4 floats **ambient** | `+0x16` |
| 11 | 4 floats **colour `c`** | `+0x26` |
| 12 | 4 floats | `+0x36` (`0x574960`) |
| 13–14 | 2 floats | `0x574930`, `0x5749c0` → `+0x62` |
| 15–16 | 2 ints | `0x5749e0`, … |
| 17 | float | … (reader continues past `0x574cbe`, not mapped) |

Example, material `Face` in `young_max_payne_L0.kfs`: bools all 0; mode **2**, 0, 0; ambient
`(255, 255, 255, 255)`, which matches the golden white at `0x556eac`; colour `(172, 143, 127, 255)`; third colour
`(229, 229, 229, 255)`; then `1.0`, `8.0`, `0`, `0`, `50.0`; then bool 1 and texture sub-chunk `0x10011`
(`FaceTexture`, four `face_maxpayne_*.jpg` frames). With mode 2, `c` becomes the D3D **diffuse** colour.

## Lights the exe creates (VERIFIED, imports)

Only `P_PointLight` (`0x5f9ee7`) and `P_SpotLight` (`0x5f9c87`) are constructed. No directional light, and no
`P_Scene::setAmbientColor`. `P_VirtualLight::setMaximumLightsPerObject(8)` at `0x40deb5`. More
`setFalloffRange` calls at `0x77cfb1`, `0x77d08e`, `0x77d5c8` (dynamic lights, not read yet).

## Lit stage programs (VERIFIED, golden data 2026-11-02)

**Capture:** winedbg launch (`-skipstartup`), driver loaded at `0x1c00000` under the debugger (it moved to
`0x1b00000` without one, so rebase per run with `info share`). Gate: first `SetMaterial` (`+0x179c5`). The
menu never reaches it, so the menu draws nothing lit. Then each request to the cached `SetTextureStageState`
wrapper `+0x2f3b0` (args `stage, type, value` at `[esp+4]`) was logged with the current `D3DMATERIAL8` diffuse.
The driver caches stage states at `+0x84b64 + (stage·32 + type)·12` (value, valid flag). Save loaded in
the house, Max on screen; 4000 requests, 72 lit materials. (`D3DTA`: 0 diffuse, 1 current, 2 texture; `D3DTOP`:
1 disable, 2 selectarg1, 4 modulate, 7 add.)

| Count | Lit | Stage 0 | Stage 1 | Result |
|------:|-----|---------|---------|--------|
| 69 | yes | `ADD(TEXTURE uv1, DIFFUSE)` | `MODULATE(TEXTURE uv0, CURRENT)` | texture × (lightmap + vertex diffuse) |
| 2 | yes | `MODULATE(TEXTURE uv0, DIFFUSE)` | disabled | texture × vertex diffuse |
| 2 | no | `SELECTARG1(TEXTURE uv0)` | `MODULATE(TEXTURE uv1, CURRENT)` | texture × lightmap |

- The 69 match the `0x8b0718` level template, so the template is confirmed.
- A lit single-texture `COPY` layer becomes `MODULATE(TEXTURE, DIFFUSE)`. So the stage-0 `SELECTARG` HYPOTHESIS is
  **ruled out**: lighting does reach object pixels. Mapping the 2 single-stage draws to object/character materials
  is STRONGLY SUPPORTED (single texture, lit, matches the `0x554d30` layer list), not tied to a specific mesh.
- All 72 lit materials had diffuse `(255, 255, 255)`. `setDiffuseColor` from the KFS colour `c` only runs in the
  `0x556000`/`0x556120` helpers. When those run is not mapped, so the default diffuse 255 is what reached D3D here.

## Light selection per object (VERIFIED, static, driver + e2mfc)

The driver walks the object's light-collision list (`P_BaseObject::getFirstLightCollision` /
`getNextLightCollision`, loop at `0x100239ea`). For each light it calls light `vt+0x60` (e2mfc `P_PointLight`
`0x10011550`), which dispatches to the object's device object `[obj+0x114]` `vt+0x10`. For a point light that is driver
`0x10023ec0` (spot: `vt+0x14` → `0x10024270`, not read). It appends an 8-byte entry `(key, light index [light+0x110])`:

    d   = |light position (+0x80) − object position (+0x80)| × object scale [obj+0xbc]
    key = |light colour (+0x118..+0x120)| × 1 / (a0 + a1·d + a2·d²) / 255     (a = getAttenuation())

The entries are sorted with `std::sort` (16-element insertion threshold `0x80` bytes). The partition `0x100253c0` advances
while `key > pivot`, so the order is **descending** (brightest first). Up to `getMaximumLightsPerObject()` (8, set by the exe
at `0x40deb5`) are used. Which lights enter the collision list (light range vs object bounds, collision group/mask) is
STRONGLY SUPPORTED as a bounding-sphere test, not read.

**Light positions (STRONGLY SUPPORTED, data):** the `.ldb` static point-light position is **room-local**. In the house,
all 24 lights lie within 2–13 m of characters in every room when read as world positions. With
`room.world_origin + pos` they fall in their own rooms, and the engine's lit Max matches retail at the hallway lamp.

**Vertex normals:** the KFS mesh chunk `0x10005` has no normal sub-chunk (`0x10000` header, `0x10006` vertices, `0x10007`
faces, `0x1000b` one byte per face (smoothing group? UNKNOWN), `0x1000c` materials, `0x1000e` UV faces). So normals are
built at load time. The method is UNKNOWN.

**Engine consequence (STRONGLY SUPPORTED):** a lit object pixel is `texture × clamp(Σ point/spot lights ·
max(N·L,0) · atten)`, per vertex then Gouraud, with black ambient and zero emissive. Light selection: see above.
Lightmapped level geometry gets no lights; see next section.

## Level ambient: `levels.txt` `AmbientColor` (2026-11-03)

**VERIFIED (static):** the `levels.txt` key table entry at `0x52555d` binds `"AmbientColor"` (`0x880000`)
to storage `0x8acdd8`. `0x525e50`…`0x525e7c` copies the three floats into the level info at `+0x58`.

**VERIFIED (static):** `0x680500` creates the `P_Scene` (`0x6805b8` stores it at `+0x5b4`) and copies its
colour argument straight into `P_Scene+0x54` (`0x6805be`…`0x6805d8`). That is the field `getAmbientColor`
returns and the driver packs into `D3DRS_AMBIENT`. The setter is bypassed, which is why no
`setAmbientColor` call shows up. Two more direct writers (`0x67ac30`, `0x67b29a`, gated by byte `0x8b485d`)
are not identified yet.

**VERIFIED (dynamic, winedbg, plain launch into the menu's startup level, break `0x6805be`):** the copied
colour is `(140.0, 135.0, 110.0, 255.0)` (`430c0000 43070000 42dc0000 437f0000`), hit twice. That is exactly
`[startup_level] AmbientColor = ( 140, 135, 110 )`. The constructor chain `0x503ec0` → `0x503a90` →
`0x677580` → `0x680500` only forwards arguments.

**Engine consequence (STRONGLY SUPPORTED):** object materials have white ambient and colour source 0
(above), so a character vertex is `clamp(AmbientColor/255 + Σ point-light diffuse)`. Level templates set
black material ambient, so lightmapped geometry does not change. The official MaxED tutorial
("Dynamic lighting") agrees: AmbientColor "is an RBG value defining the level of ambient character
lighting", and point lights and spotlights do not affect static geometry; particle lights do.
`render3d::character_vertex_light` takes `LdbScene::ambient`, filled from `levels.txt`.

## Lights per draw: level geometry is unlit by point lights (2026-11-02)

**Driver code (VERIFIED, static).** For a material with lighting enabled (`[vt+0x40]`), the pre-draw path
(`0x100165c1`…`0x100166ba`) sets `D3DRS_LIGHTING` = 1 and `D3DRS_AMBIENT`, then computes
`n = min(light list size, getMaximumLightsPerObject(), device cap [+0x110]+0x20c)`. The light list is the
8-byte `(key, index)` vector at `+0x24`/`+0x28` of the object's device object. It then calls
`LightEnable(index, TRUE)` (device vt `+0xb8`) for the first `n`. The post-draw method `0x10016a60` recomputes
`n` the same way and disables those lights (`0x10016ad1`…`0x10016af9`). So the enabled set per draw is exactly
the object's own light list. Lights do not leak from one draw to the next. The other `+0xb8` calls in the driver are on
non-device vtables.

**Golden data (VERIFIED, winedbg, house save loaded with `-skipstartup <save>`).** Breakpoint on
`driver+0x16ad1` (driver at `0x1c00000`): logged `n` (`edi`), the device object (`ebx`), and the cached stage
states (stage 0 / 1 `COLOROP`, stage 0 `TEXCOORDINDEX`) for 6000 lit draws:

| Draws | Stage 0 | Stage 1 | Lights `n` | Device objects |
|------:|---------|---------|-----------:|---------------:|
| 5687 | `ADD` (7), uv1 | `MODULATE` (4) | **0** | level geometry (two objects take 2557 and 1386 of these draws) |
| 250 | `MODULATE` (4), uv0 | disabled | 6 | one (`0x7163000`) |
| 63 | `SELECTARG1` (2), uv0 | disabled | 0 | — |

The lightmap template's `DIFFUSE` term is therefore **zero** in retail: level polygons are
`texture × lightmap`. The only draws with lights are the single-stage object material, which is a character (Max). Its
6 lights fit under the cap of 8.

**Ruled out:** static point lights adding to lightmapped level polygons (in the house, every such draw had `n = 0`).
Not checked: other levels. Dynamic lights do enter level objects' lists (see the second capture below).

**Engine consequence:** none. `render3d::rasterize_tri` already draws lightmapped polygons as
`lightmap_modulate(texture, lightmap)` with no vertex diffuse.

**Capture recipe notes:** the minimum install reads `D:\DISK1\Levels` from `cd.ini` (`0x40ae60`) and only checks
that the path exists. A directory `cd/DISK1/Levels -> <install>/Levels` with `dosdevices/d:` pointed at `cd`
passes the check without the CD (restore the link afterwards). Launch directly with
`wine winedbg --file C:\…\script.txt Z:\…\MaxPayne.exe -nodialog -skipstartup C:\…\save.mps`. Paths with spaces
or under `/tmp/claude-…` (leading `-` path component) failed with "Couldn't start process" / "Path not found",
and so did the `explorer /desktop` + `cmd` wrapper this time.

## Dynamic lights do reach level geometry (2026-11-02, second capture)

**Setup.** Same house save. `e2_d3d8_driver_mfc.dll` loaded at `0x1d60000` this time, not `0x1c00000`, so a
fixed-address script fails (`Invalid address 0x1c16ad1`). The capture drove winedbg interactively through a FIFO:
break on save load (`0x556eac`), read the driver base from `info share`, then break on the three particle
emitter `setFalloffRange` calls (`0x77cfb1`, `0x77d08e`, `0x77d5c8`, all in `PS_RTParticleEmitter` code). On the
first hit it logged 2000 draws at `driver+0x16ad1` (lights `n`, device object, stage cache at
`driver+0x84b70`/`+0x84cf0`/`+0x84be8`), in three rounds. The player fired the pistol before rounds 1 and 2.

| Round | Trigger | Lightmapped level draws (stage 0 `ADD` uv1, stage 1 `MODULATE`) | Character draws |
|------:|---------|------------------------------------------------------------------|-----------------|
| 0 | `0x77cfb1` (ambient effect, before the gun was drawn) | 1897 × `n = 0` | 82 × `n = 6` |
| 1 | `0x77d08e` (shot) | **923 × `n = 1`**, 830 × `n = 0` | `n` = 3…8 |
| 2 | `0x77d08e` (shot, later) | 1700 × `n = 0` | `n` = 3…7 |

**VERIFIED (dynamic):** right after a shot, one light enters the light lists of level geometry. Two level
objects took most of those draws (533 and 286). It is gone by the next round, which matches a muzzle-flash
lifetime. Round 0's emitter light did not reach level geometry.

So the level template (`0x8b0718`: `DIFFUSE_COPY` → lightmap `COPY` → `DIFFUSE_ADD` → texture `MUL`) evaluates to
**texture × (lightmap + Σ dynamic point-light diffuse)**. The static point lights never contribute (first
capture). Dynamic particle lights do, using the D3D point-light formula above.

**Engine consequence:** `render3d` has no dynamic lights. Its muzzle flash is a sprite only, and lightmapped
polygons get no diffuse term. To match retail, a shot should add a short-lived point light whose diffuse term is
added to the lightmap before the texture multiply. The emitter's light colour, range and lifetime come from the
particle effect data (`PS_RTParticleEmitter`, `0x77d08e` path) and are not read yet.

## Open (golden data needed)

1. ~~Object ambient~~ (done above: white).
2. ~~Do static point lights also add to lightmapped level polygons?~~ No (see above, house only).
3. ~~Dynamic lights: do they enter level-object light lists?~~ Yes, after a shot (second capture above).
4. Muzzle-flash light parameters (colour, range, lifetime) from the particle effect data behind `0x77d08e`.

## Lightmap filtering (HYPOTHESIS, 2026-11-03)

Both renderers sample lightmaps bilinearly, clamped to the edge (`render3d::sample_lightmap`, and the
wgpu `lmap_sampler` with linear filtering). Nearest sampling showed each lightmap texel as a hard step.
In the retail house, the owner sees the same shading change smoothly. The `FILTER_MODE` values passed
to `addTextureLayer` (`0x1000000`, `0x5000000`) are not decoded, so this rests on that visual
comparison. Diffuse textures keep nearest sampling until the filter values are read.
