# Max-FX KF2 / SKD / KFS — Working Spec (Phase 3)

**Status:** Draft from fixture hex + `KFExport.dle` imports (`R_MemoryFile::createChunk` / `closeChunk`). Full struct alignment still being confirmed in REA against `rl.dll`.

**Sources**

- Retail / mod loose `database/**/*.kf2` (static props, weapons, anims)
- Example skin pair: `database/skins/.../*_l0.{skd,kfs}`
- `vendor/.../KFExport.dle` (Max 4 plugin, PE32)

## High-level

KF2/SKD/KFS are **tagged binary streams** written through Remedy `R_MemoryFile` chunk APIs (same family as many Max-FX assets). Not RIFF fourccs; records begin with a type byte group dominated by `0x0C` (chunk) and `0x0D` (tagged value / string).

| Extension | Typical content |
|-----------|-----------------|
| `.kf2` | Mesh + materials + optional skeletal animation channels |
| `.skd` | Skeleton / bind pose / bone hierarchy |
| `.kfs` | Skinned character mesh + texture refs (often pairs with `.skd`) |

## Recurring tags (observed)

| Prefix | Meaning (inferred) | Notes |
|--------|--------------------|-------|
| `0C 0F 00 01` | Geometry / material container | Often first in prop KF2; followed by texture path string |
| `0C 10 00 01` | Material / texture slot | Contains names like `HealthBottle`, JPG refs |
| `0C 12 00 01` | Animation / node block | Anim KF2; `a=0x500` (1280) often present |
| `0C 13 00 01` | Bone / node channel | Follows `0C 12`; name string e.g. `Pelvis`, `Thrower` |
| `0C 14 00 01` | Skin / skeleton data | Opens `.skd` |
| `0D 14 <len>` | Length-prefixed Latin-1 string | `len` is `uint8`; then `len` bytes |

Example (medipack):

```
0C 0F 00 01  00 00 00 00  00 F0 00 00
0D 14 1A  "textures;..\sharedtextures"
0C 10 00 01  ...
0D 14 0C  "HealthBottle"
```

Example (dodgeleft anim): bone names `Pelvis`, `Toe-R`, `Foot-R`, `UpLeg-L`, …

## Coordinates (from Max Payne modding lore + KFExport Max SDK usage)

- Authored in **3ds Max** (Z-up); runtime Max Payne is effectively **Y-up** with ~**0.01** unit scale in many pipelines.
- KFExport pulls `INode` / `Mesh` / `Matrix3` / `Quat` via Max SDK (`mesh.dll`, `Matrix3` mangled imports in `KFExport.dle`).

## Open questions (next REA passes)

1. Exact on-disk layout of `createChunk(id, size)` header words (endian, whether size includes header).
2. Vertex / index / UV packing and Physique 4-weight encoding in `0C 0F` / `0C 10` payloads.
3. Keyframe track encoding inside `0C 13` (times + quat/pos).
4. Differences between static prop KF2 vs skeletal anim KF2 vs KFS.

## Clean-room struct sketch (working)

```text
KF2 stream := { Chunk | TaggedString }*
Chunk      := 0x0C <u16 kind> <u16 flags?> <payload...>
TaggedString := 0x0D 0x14 <u8 len> <latin1[len]>
```

Exact `createChunk` size words still TBD (REA). Vertex buffers likely follow material
string chunks; skeletal tracks live under `0C 12` / `0C 13`.

## Clip frame rate (VERIFIED, retail `default_skeleton\anim`, 2026-11-03)

Each skeletal bone channel (`0x00010012` v5) opens with a `0x00010013` name chunk:
`0D 14 <len> <name>`, then a tagged integer, then `0E <flag>`. The integer is the clip's
frames per second; keyframe numbers and the channel duration count frames at that rate.

| Clip | Duration (frames) | Rate | Length |
|---|---|---|---|
| `GetHit_1000ms` | 121 | 120 | 1.0 s |
| `GetHit_1500ms` | 181 | 120 | 1.5 s |
| `2_GetHit_1000ms` | 76 | 75 | 1.0 s |
| `gethit_crouched_0750ms` | 61 | 80 | 0.76 s |
| `Stand` | 220 | 15 | 14.7 s |
| `Run` | 40 | 50 | 0.8 s |
| `Walk` | 40 | 32 | 1.25 s |

The `_XXXXms` file names match the computed length, which fixes the field's meaning. Rates
in the retail set include 12, 15, 30, 50, 120 and 240 (`0x13` u16 tag for 240). The `0E` flag
after the rate is UNKNOWN (1 on the looping Stand/Run/Walk clips and on DodgeLeft, 0 on
the GetHit clips). The engine previously assumed 30 fps for every clip, so Stand played
twice as fast.

## Implementation plan

1. `tools/blender_io_kf2` — Blender 4.x addon (stub registered); import strings + bones first.
2. Synthetic unit tests once chunk header layout is locked (no retail `.kf2` in git).
3. Cross-check writer against `KFExport.dle` REA (local vendor only).

## World spheres (`database/worldspheres/bg_*.kf2`)

Static meshes in a packed variant of the chunks, checked on all 11 retail files. A count is `0x10` + `u16` or `0x11` + `u8`.

| Chunk | Payload |
|---|---|
| `0x0001000f` | Search path, material count, one chunk per material: name, then bitmaps (colour, then opacity) |
| `0x00010005` | One mesh, holding the five chunks below |
| `0x00010000` | Name, parent, `0x1a` + 12 floats: three matrix columns, then position |
| `0x00010006` | Count, positions (`f32` x3), then as many normals, packed with no tags |
| `0x00010007` | Count of `u16` indices, a triangle list |
| `0x0001000c` | Material count, material name |
| `0x0001000e` | Channel, count, one `(u, v, w)` per vertex; `v` runs from −1 at the zenith to 0, wrapping |

Every sphere has `Sphere01` (`SkyboxDefault`). The skyline spheres add `Cylinder01` (`SkyboxBuildings`) with a colour and an opacity bitmap. The root matrix turns and lowers each sphere, which is how `intro` differs from `docks`.

## Weapon models

A weapon's `*_Lod0.kf2` uses the packed mesh layout of world spheres. Its meshes are a hierarchy: `Beretta` is the root and
turns Max's z up into y up, and `Breach` and `Barrel` sit under it with local transforms (`0x00010000` stores name, parent
and root). VERIFIED by plotting the Beretta on the `Gun` bone of `Stand.kf2` and `idle_aim_beretta.kf2`: the grip sits in
the hand and the barrel points along the arm. `beretta_shoot.kf2` animates `Breach`, which the engine does not play yet.
