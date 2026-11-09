# Character collision in MaxPayne.exe

Step 1 of `docs/specs/2026-11-08-capsule-collision.md`. PE i386, ImageBase `0x400000`. Work in progress: this note
records what is read so far and where the reading goes next.

## Data

- **VERIFIED:** `skins/max_payne.txt` gives `CapsuleRadius 0.48`, `CapsuleBottom 0.0`, `CapsuleTop 1.95`.
  `globalconfig.txt` caps them at radius 0.50, bottom 0.0, top 2.00 (`CapsuleRadiusLimit`, `CapsuleBottomLimit`,
  `CapsuleTopLimit`, read at `0x50b26d` / `0x50b257`), and gives `AICapsuleRadius 0.31` and
  `MinSpeedCharacterCollidesDynamicObject 0.01` (`0x50b307`).
- **VERIFIED:** the exe checks a skin's capsule against those caps ("Capsule for skin \"%s\" too large", string
  `0x8929cc`) and against the camera sphere ("capsule too small for camera sphere", `0x4522b3`).

## The move routine `0x647660`

- **VERIFIED:** `0x647660(target, owner, flag)` is called from five places: `0x4429f0`, `0x647629` (the wrapper
  `0x647610`, which first subtracts the object's position at `+0x54`), `0x647d63`, `0x64c935` (inside the
  line-of-sight test `0x64c910`) and `0x705b56`.
- **VERIFIED:** it subtracts the object's position (`+0x54`, `+0x58`, `+0x5c`) from the target. When every axis of the
  difference lies within ±`FLT_MIN` (constants `0x7d4ec8`, `0x7cefb8`), it does nothing.
- **VERIFIED:** it is 252 instructions and calls 17 routines. By size and float work, the geometry is in `0x654fa0`
  (361 instructions, 142 float operations), `0x646f30` (552, 79), `0x645340` (558, 52), `0x654aa0` (379, 54) and
  `0x64f100` (236, 54). `0x6479b0` (156, no float) and `0x748300` (a `+0x8` getter) handle bookkeeping.
- **STRONGLY SUPPORTED:** the start-point check (`0x4528e3`, "Startpoint above floor translation collided") moves a
  `MaxPayne_StartPointTestCapsule` with `0x647610` and reports a collision when the object it ended against is not its
  own owner (`[ebp-0xdc]` against `0x748300`).
- **Correction:** `0x5e3b20` is a getter (`lea eax,[ecx+0x148]`), the name printed in the error, not a sweep.

## Next

- **STRONGLY SUPPORTED (ruled out):** `0x654fa0` is not the collision core. It copies two 3x4 blocks of the object
  (`+0x38` through `0x4f84f0`, and `+0x68`) and multiplies them, so it rebuilds the object's world transform after
  the move.

## Shape classes

- **VERIFIED (RTTI):** the moving shapes are `X_LevelRuntimeCapsuleObjectBase` (vtable `0x7d4d64`) and
  `X_LevelRuntimeSphereObjectBase` (vtable `0x7d5008`). `MaxPayne_StartPointTestCapsule` (`0x7cd010`) derives from the
  capsule, and `CharacterTestSphere` (`0x7d5d10`), `DodgeTestSphere` and `TestSphere` from the sphere.
- **VERIFIED:** the two bases share most slots. In the first 31 slots they differ at 0 (destructor), 8, 12, 18,
  20, 22, 23 and 24. Slot 19 (`+0x4c`) is `0x64ab20` on both, and that function is `ret 0x10`. Slot 20 (`+0x50`) is
  `0x63eee0` for the capsule and `0x658dd0` for the sphere; they call `0x63b410` and `0x63b1a0`. Slot 22 (`+0x58`)
  calls `0x5c0ba0` and `0x5c05f0`. Slot 23 (`+0x5c`) calls `0x5bf8e0` and `0x5bf2f0`. Slot 21 (`+0x54`) is shared
  (`0x64c270`). An earlier count had these one slot low.
- **Ruled out:** `0x646f30` and `0x645340` do little float work themselves; their helpers are getters (`0x642d40`,
  `0x642d50` = position at `+0x54`, `0x74e8e0` = `+0x30`, `0x411d90` = `+0x24`). A static walk four calls deep from
  `0x63b410`, `0x5bf8e0` and `0x5c0ba0` finds only one small float routine, `0x449400` (19 float operations, one square
  root), a length or normalisation.
- **Ruled out:** `e2mfc.dll`'s `P_CollisionBuffer` (`collide`, `execute`, bounding spheres) is the renderer's
  object-versus-light and visibility pass, not character movement.
- **STRONGLY SUPPORTED:** the shape-versus-level math is reached through virtual calls (`call [reg+n]`), which the
  static walk does not follow.

## Who moves a character

- **VERIFIED:** `X_Character` (RTTI `.?AVX_Character@@`, type descriptor `0x8684c8`, vtable `0x7d5950`) slot `+0x54`
  is `0x696f60`. That method calls the shared query `0x64af10` at `0x6971fd` with the flag argument `0`, then the
  line-of-sight test `0x64c910` at `0x697210`. Later, at `0x697619`, it calls `0x647b90`. When that returns non-zero
  it subtracts two vectors and calls the wrapper `0x647610` at `0x697689`.
- **VERIFIED:** `0x647b90` is the same early-out as the top of `0x647660`: each axis of `target - position(+0x54)`
  is compared with `±FLT_MIN` (`0x7d4ec8`, `0x7cefb8`). The wrapper `0x647610` pushes flag `0` into `0x647660`.
- **VERIFIED:** none of the five direct callers of `0x647660` is this character method. The other callers of the
  wrapper are the start-point check `0x4524d0`, the camera path `0x672150` (`X_CameraImplementation`, vtable slot
  `0x7d5718`, string "Absolute camera path collides to level" at `0x672410`), `0x6d80c0` (class not read), and the
  projectile path `0x7079b0` (reached from `0x705980`; projectile strings at `0x89ee90`).
- **VERIFIED:** `0x64af10` is not character-only. It is also called from `0x4f5ff0`, `0x4f6135`, `0x4f69bd`,
  `0x500eae`, `0x50442d`, `0x648471`, `0x64aef4`, `0x64f3b0` and `0x71a0b3`.
- **VERIFIED:** `0x64af10` calls `0x645340` at `0x64b2ba`, then `0x645ca0` at `0x64b2d4` (`0x645ca0` has no other
  caller). `0x646f30` runs only when `ebp+0x10` is non-zero (`0x64b2d9`). The character passes `0`, so that call
  skips `0x646f30`. Inside `0x647660` the character's flag `0` likewise skips `0x645340` at `0x647885`. `0x646f30`
  still reaches `0x645340` at `0x6472fc` when some other caller enters it.
- **VERIFIED:** `0x64fda0`, called from `0x645340` at `0x6454f6`, is an insert. `imul 0x51eb851f` / `sar 5` divides
  by 100, and the copy loop steps `0x64` through `0x6521f0`. The bytes inside one element are not read.

## Virtual calls

- **VERIFIED:** `X_Character` (`0x7d5950`) keeps the capsule's slot 20 (`0x63eee0`), slot 22 (`0x63e280`) and slot 23
  (`0x63e350`). It overrides slot 19 with `0x695070`. Slot 21 stays the update `0x696f60`. `MaxPayne_StartPointTestCapsule`
  (`0x7cd010`) and `CharacterTestSphere` (`0x7d5d10`) override slot 19 with `0x4f0db0` and keep those other slots.
- **VERIFIED:** `0x646f30` at `0x6470f0` does `mov ebx,[this]` / `call [ebx+0x4c]` with four arguments and `ecx` set back
  to `this`. On the two shape bases that slot returns immediately. On a character it enters `0x695070`. The character's
  flag 0 still does not enter `0x646f30`, so this call is not on that path.
- **VERIFIED:** `0x695070` and `0x4f0db0` both call `0x649300`, then `type_info::name` (import `0x7ca34c`), then
  `call [edx+0x48]`. `0x695070` returns at `0x69538c` (`ret 0x10`) and has no x87 instruction of its own, so it does
  not itself change the three floats `0x646f30` compares after the call. It also calls `[reg+0x18]`, `[reg+0x1c]`,
  `[reg+0x20]` and `[reg+0x24]`. Those callees are not read.
- **VERIFIED:** `0x63b410` (`0x63b410`..`0x63b730`) has no vtable call. Its one indirect call is the `G_Color`
  constructor (`0x7ca90c`, grphmfc.dll).
- **VERIFIED:** `0x5bf8e0` returns at `0x5c0534` (`ret 0x1c`). It calls `0x449400` at `0x5c0007` and does not call
  `0x5c2520`.
- **VERIFIED:** `0x5c2520` calls `P_VirtualObject::getTriangleMesh(int)` (e2mfc, `0x7ca49c`) and
  `P_TriangleMeshBase::lock` (`0x7ca58c`). It calls `0x5c2740` three times, with corner 0, 1 and 2. That helper loads
  `ecx` from `[this+0x1c]` and calls slot `+0x30` of that object. `0x5c2760` loads `ecx` from `[this+0x18]`, calls
  slot `+0x70`, and copies three dwords out. `P_TriangleMeshBase::unlock` (`0x7ca590`) follows. The only caller of
  `0x5c2520` is `0x5c1f3f`, inside `0x5c1e20`.
- **Ruled out, narrowed:** `P_CollisionBuffer` stays the renderer's visibility pass. `getTriangleMesh` is a different
  e2mfc entry.
- **UNKNOWN:** the class at a triangle mesh's `+0x18` and `+0x1c`, and whether slot 23 reaches `0x5c1e20`.

## What runs for flag 0

- **VERIFIED:** in `0x647660`, `[ebp+0x10]` is the flag. Zero jumps from `0x647719` to `0x6478e9`, so the call to
  `0x645340` at `0x647885` does not run. `0x64af10` has already called `0x645340` at `0x64b2ba` before that.
- **VERIFIED:** from `0x6478e9` the move subtracts the target (`edi+0xc`) from the position (`0x642d50`). `0x64bd90`
  returns immediately when that delta's length squared is at most `FLT_MIN` (`0x7cefb8`). Otherwise it multiplies the
  delta by the 3×3 from `0x642d40` / `0x63a900` and calls `0x64be70` with the local delta and `this+0xc0`.
- **VERIFIED:** `0x64be70` walks the list at `this+0xc0` (sentinel `0x8b3d84`). For a node whose `+0xc` object passes
  `0x642a80` and fails `0x64dae0`, it multiplies the delta by the 3×3 from `0x642d70` and calls that object's slot
  `+0x6c` with the character and the product. Slot `+0x6c` is `ret 0x8` (`0x755fa0`) on the capsule, the sphere, the
  start-point capsule and `CharacterTestSphere`. `X_Character` overrides it with `0x696e90`. `X_Projectile` (`0x7d8110`)
  overrides it with `0x704ef0`.
- **VERIFIED:** `0x6437e0` then decides `0x64f100`. The path that finds a component equal to `FLT_MAX` (`0x7d4f6c`)
  returns 0, and `0x64f100` is skipped. The other return is the byte at `[ebp-0xe]`.
- **VERIFIED:** `0x64f100` leaves immediately unless `0x64c2b0` returns non-zero and `0x646710` returns a container
  whose `(end-begin)` divides by 40 (`imul 0x66666667` / `sar 4`). The same function is also called from `0x648014`.
  The local accumulator at `[ebp-0x20]` starts at `(0, 0, 0)`. `0x64f307` is the loop tail, not another formula: it
  adds `0x28` to the element pointer and jumps to `0x64f183` until the pointer meets the end. An element whose
  scalar at `+0x1c` is `≤ -2` (`0x7d4f5c`) or `≥ 0.02` (`0x7d4e04`) jumps straight there and is not counted. An
  element inside that window multiplies the vector at `+0x4` by `0.015 - scalar` (`0x7d4e14`). With `v` that
  product and `a` the accumulator, `t = dot(v, a) / dot(v, v)`. `t ≤ 0` (`0x7cb8b0`) does `a += v`. `0 < t < 1`
  (`0x7cbdd0`) does `a += v * (1 - t)`. `t ≥ 1` adds nothing. All three arms count the element. `0x64f100` has no
  square root. When the count is greater than zero, it adds the accumulator to the position from `0x642d50` and
  calls `0x64af10` at `0x64f3b0` with that sum, the global at `0x8b38e8`, and `0`. `0x654fa0` follows.
- **VERIFIED:** `P_VirtualObject::getTriangleMesh` is not on this path. Its call sites are `0x4a8d02`, `0x5c25d3`,
  `0x5c2c0c`, `0x67c822`, `0x69871b`, `0x6dbc49`, `0x731513`, `0x7323a6` and `0x7323c4`. `0x698130`, which contains
  `0x69871b` and then calls `0x64db90`, is reached from `X_Projectile` slot 5 (`0x7097d0` at vtable `0x7d8124`) and
  from `0x6d921d`. Six direct calls up from `0x6d921d` land in `0x452cd0` / `0x44fda0`, not in `0x696f60`.
- **VERIFIED:** `0x646710` returns `this+0x90`. Begin and end of the 40-byte container are `this+0x94` and
  `this+0x98`. When `[this+0x88]` is non-zero it rebuilds that container, then clears the byte. When the byte is
  already zero and `[this+0x8c]` equals the global at `0x8b3d70`, it returns the container without rebuilding.
- **VERIFIED:** on the rebuild, `0x646c80` replaces the vector at `+0x4` and the vector at `+0x10` with each vector
  divided by its own length (`fsqrt`, then divide `1`). `0x646dd0` then erases an element when
  `dot(position - vector_at_+0x10, vector_at_+0x4) ≤ 0`. `0x646ad0`, called before those two, dots two `+0x4`
  vectors, clamps the dot to `[-1, 1]` (`0x7ce988`, `0x7cbdd0`), and passes it to `_CIacos` (`0x76faf8` jumps
  through MSVCRT import `0x7ca378`). It erases the later element when `fabs` of that result is greater than the
  dword at `0x8b3d4c`. That address is past the initialized image, so the file does not hold the threshold.
- **STRONGLY SUPPORTED:** the record's first dword is the vtable at `0x7cce84`. Its RTTI type descriptor is
  `.?AV?$M_Plane3Template@M@@` (complete-object locator `0x7de358`, type descriptor `0x86c3d0`). The first method,
  `0x448bb0`, copies three floats to `this+0x4` and divides them by their length. The erase path writes that same
  vtable back to the element.
- **VERIFIED:** `0x640d10` is the store into `+0x1c`. It copies its second argument there and its third argument to
  `+0x24`, and copies two vectors from its first argument. Its only caller is `0x640cc2`, inside `0x63fc40`. The
  value it stores is the float left in `[ebp+0x8]` by `fst` at `0x640af7`. `0x63fc40` does not call `0x640d10` when
  that float is greater than `0.02` (`0x7d4e04`, compare at `0x640afa`, jump at `0x640b05`).
- **VERIFIED:** `0x63fc40`'s only caller is `0x6954d8`, in `0x695490`. That function's only caller is `0x695755`, in
  `0x6954f0`. `0x6954f0` has no direct caller. The dword at `0x7d59b4` is `0x6954f0`, which is slot 25 of
  `X_Character` (`0x7d5950`). `0x652640` and `0x5c84f0`, reached from the container rebuild, copy records of 40 bytes.
  They do not compute `+0x1c`.
- **Ruled out:** `0x696f60` through `0x697800` has no `call [reg+0x64]`. The `call [eax+0x64]` at `0x684cdb`
  reads `[return+0x1c]` as a string for `R_STR`, so that slot is not the character's plane builder.
- **UNKNOWN:** which virtual call enters `X_Character` slot 25 while Max moves. The flag-0 move's direct calls do
  not. The bit-level approximation that feeds `0x640af7` is not named. The capsule-versus-triangle distance, if it
  is `0x64db90`, is not called from the flag-0 move.

## Next

1. Find the call through `X_Character` slot 25 (`[reg+0x64]` → `0x6954f0`) and name the float at `0x640af7`.
2. Read `0x64db90` only together with a caller that the character update reaches. The projectile caller is not that.
