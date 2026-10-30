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

## Implementation plan

1. `tools/blender_io_kf2` — Blender 4.x addon (stub registered); import strings + bones first.
2. Synthetic unit tests once chunk header layout is locked (no retail `.kf2` in git).
3. Cross-check writer against `KFExport.dle` REA (local vendor only).
