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
- **UNKNOWN:** the class at a triangle mesh's `+0x18` and `+0x1c`. Slot 23's body `0x63e350` does not call `0x5c1e20`.

## What runs for flag 0

- **VERIFIED:** in `0x647660`, `[ebp+0x10]` is the flag. Zero jumps from `0x647719` to `0x6478e9`, so the call to
  `0x645340` at `0x647885` does not run. `0x64af10` has already called `0x645340` at `0x64b2ba` before that.
- **VERIFIED:** from `0x6478e9` the move subtracts the target (`edi+0xc`) from the position (`0x642d50`). `0x64bd90`
  returns immediately when that delta's length squared is at most `FLT_MIN` (`0x7cefb8`). Otherwise it multiplies the
  delta by the 3×3 from `0x642d40` / `0x63a900` and calls `0x64be70` with the local delta and `this+0xc0`.
- **VERIFIED:** `0x64be70` walks the list at `this+0xc0` (sentinel `0x8b3d84`). `0x642a80` returns 1 when the dword
  at the node object's `+0x60` is not zero; zero skips the node (`0x64bea2`). `0x64dae0` returns the byte at the
  walker's `+0xf0`, not a field of the node. When that byte is not zero, the walker copies the delta's three floats
  onto `this+0xb1` and does not call slot 27 (`0x64beb1`). When the byte is zero, it multiplies the delta by the 3×3
  from `0x642d70` and calls the node object's slot `+0x6c` with the walker and the product. Slot `+0x6c` is `ret 0x8`
  (`0x755fa0`) on the capsule, the sphere, the start-point capsule and `CharacterTestSphere`. `X_Character` overrides
  it with `0x696e90`. `X_Projectile` (`0x7d8110`) overrides it with `0x704ef0`.
- **VERIFIED:** `0x696e90` (`ret 0x8`) returns immediately when `0x642a80` says the dword at `this+0x60` is zero. It
  does not load `[ebp+8]`, the walker. It adds the three floats at `[ebp+0xc]` (the product just pushed at
  `0x64bf56`) to the position from `0x642d50`. `0x64aee0` then calls `0x64af10` with that sum, global `0x8b49c8` and
  flag 0, and calls `0x6461c0`. `0x687f10` stores 1 at `[this-0x1e+0x21c]` and jumps to `0x64f010`, which stores 1
  at `this+0xb0` and `this+0x88`. The byte at `+0x88` is the one `0x646710` tests before rebuilding the 40-byte
  container. The one-time init of global `0x8b4a6c` (bit 0) registers `0x696f50`, which jumps to `0x41c3a0`.
- **VERIFIED:** `0x6461c0` returns without writing when `[this+0x84]` equals the dword from `0x642d40`, the three
  floats at `this+0x78` equal the position from `0x642d50`, `[this+0x6c]` is not zero, and
  `[this+0x70] - [this+0x6c]` is outside `0..63` (`test ecx, 0xffffffc0` at `0x646224`). Otherwise it calls
  `0x646020` with the dword from `0x642d40`, the position, and global `0x8b82a0`, then stores that dword at
  `+0x84` and the position at `+0x78`.
- **VERIFIED:** `0x646020` (`ret 0xc`) calls `0x64c950`, which inserts one element into the container at `this+0x68`
  (begin `this+0x6c`, end `this+0x70`, stride 64 via `sar 6`). The inserted pointer is the return of `0x659c70`,
  called with the dword from `0x642d40` and global `0x8b82a0`. It then stores 1 at `[arg0+0xdb]` (`0x63c3c0`).
  The child count is `([arg0+0x14] - [arg0+0x10]) / 4`, or 0 when `[arg0+0x10]` is 0 (`0x63aa20`); a count of 0
  returns. Child `i` is `[[arg0+0x10] + i*4]` (`0x63aa10`). The dword at `[[child+0x80]+0x184]` indexes the pointer
  array at `[[arg0+4]+8]` (`0x63a2c0`, `0x656600`). A looked-up object whose byte at `+0xdb` is not zero is skipped
  (`0x63c3d0`). Otherwise slot 23 of `this` is called with `[arg0+8]`, the child, and the position pointer twice.
  On both capsule vtables that slot is `0x63e350` (`ret 0x10`). Only a return of exactly 1 continues. `0x4f84f0`
  then writes a 3×4 into a local with `child+0x24` (`0x411d90`) as its stack argument. The position is multiplied
  by the 3×4 at `child+0x24`, including the translation at `+0x24` of that block. `0x646020` calls itself with the
  looked-up object, that product, and the local 3×4. `this` stays the original object.
- **VERIFIED:** `0x63e350` multiplies `this+0x130` (`0x63e070`) by the 3×3 at `this+0x30` (`0x74e8e0`) and does not
  add a translation. It passes that product, `this+0x12c`, both position pointers, and `[0x5f2c10(child)+4]` to
  `0x5f2c20` on the child, then passes that return to `0x5b5d80`, then calls `0x5bf8e0` on `[[arg0]+0x188]`. The
  value compared with 1 is that return. `0x5bf8e0` returns 0 at `0x5bfc04` when the compare at `0x5bfbf7` does not
  take `test ah, 0x41`, and returns 1 at `0x5c051d` only when `0x5ba5b0` returns non-zero. The loop that ends at
  `0x5c0519` returns 0. `0x63e350` does not call `0x5c1e20`. The direct callers of `0x5c1e20` are `0x5c1d3c`,
  `0x5c1fd9`, `0x5c2350`, `0x5c2391`, `0x5c23d2` and `0x5c2413`.
- **VERIFIED:** `0x5ba5b0` (`ret 0x14`) returns 0, 1, 2, or 3. `0x5bf8e0` treats every non-zero the same
  (`test eax, eax` at `0x5c04fb`). `0x449460` is the dot of the vec3 at `ecx` with the vec3 at `edx`. `0x5adb50`
  subtracts its argument from `this`. `0x5b3c90` multiplies `this` by its float argument. `0x5b4b00` is a square
  root and `0x5b4930` divides 1 by its argument. `0x7cb8b0` is 0. At the call from `0x5c04f6`, `ecx` is `0x8b0900`,
  `edx` is `0x8b08f0`, arg0 is `0x8b0930`, arg1 is `0x8b0920`, and arg2, arg3, arg4 are `ebp`, `edi`, `ebx`. Just
  before that call the three floats returned by `0x5b5eb0` are copied to both globals; `0x8b0930` then adds
  `0x8b0940` and `0x8b0920` subtracts it. The return at `0x5ba900` is 1 when `|arg1-arg0|²` is not 0,
  `|arg4-arg3|²` is 0, the dot `D` of `(arg3-arg0)` with `unit(arg1-arg0)` is greater than 0 and less than
  `|arg1-arg0|`, and `|arg3 - (arg0 + D·unit)|²` is less than `arg2²` (`fcompp` at `0x5ba8e7`, `test ah, 0x41` goes
  to the 0 return). A non-zero `|arg4-arg3|²` jumps to `0x5ba903`.
- **VERIFIED:** `0x507f50` (`ret 0x8`) writes `edx + scalar·(arg1 - edx)` into `ecx`. `0x507f10` flips the sign bit of
  each of the three floats at `ecx`. On the `0x5ba903` path, `D4` is the dot of `(arg4-arg0)` with `unit(arg1-arg0)`.
  Both `D` and `D4` at or below 0, or both at or above `|arg1-arg0|`, return 0. When `D < 0`, `t = |D| / (|D| + |D4|)`
  is clamped to `[0, 1]` and `0x507f50` stores `arg3 + t·(arg4-arg3)` into the `ecx` argument. If that point's
  distance squared to `arg0` is less than `arg2²`, `edx` is set to `-unit(arg1-arg0)` and the return is 3
  (`0x5baab4`). When `D > |arg1-arg0|`, the same `t` uses `|D - length|` and `|D4 - length|`, the distance test is
  against `arg1`, `edx` is set to `+unit(arg1-arg0)`, and the return is 3 (`0x5baba0`). When `0 ≤ D ≤ |arg1-arg0|`,
  the return at `0x5bac2e` is 1 if `|(arg3-arg0) - D·unit|² < arg2²`, and that return does not write `edx`. A failed
  distance test continues at `0x5bac31`. The shared 0 is at `0x5badc9`.
- **VERIFIED:** `0x5bac31` dots `unit(arg4-arg3)` with `unit(arg1-arg0)`. `||dot| - 1| < 0.0001` (`0x7d2094`,
  and `0x7d20f0` is 1.0) returns 0. Otherwise `0x5ba370` (`ret 0x14`) builds `unit(arg4-arg3) × unit(arg1-arg0)`
  through `0x4f8860`. `|cross|² <= FLT_MIN` returns 0. So does `|dot(arg3-arg0, unit(cross))| >= arg2`. It writes
  two floats to its two pointer arguments and returns 1. Both of those floats below 0 returns 0 from `0x5ba5b0`.
  The caller keeps the first when `0 ≤ first ≤ second`, and otherwise keeps the second. A kept value greater than
  `|arg4-arg3|` returns 0. The kept value scales `unit(arg4-arg3)` and is added to `arg3`, and that sum is written
  to the `ecx` argument. The dot of `(that point - arg0)` with `unit(arg1-arg0)` must be greater than 0 and less
  than `|arg1-arg0|`. `edx` then receives that point minus `arg0`, minus the dot times the unit, and `0x5b3c40`
  divides the result by its length (`0x5b48d0` squares a component, `0x5b4900` returns 0 for a 0 argument and
  `1/x` otherwise). The return is 2 (`0x5badbd`).
- **VERIFIED:** `0x64dac0` writes its argument byte to `+0xf0` and then `0x64daf0` zeroes the three floats at `+0xb1`.
  `0x64db30` saves the byte, clears it, and calls `0x64be70` with the vector at `+0xb1` only when that vector's sum of
  squares is greater than `FLT_MIN` (`0x7cefb8`, compare at `0x64db5b`). It then zeroes the vector and restores the
  byte. `0x6434c2` writes 0 to the byte. `0x642285` copies the byte from another object.
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
- **VERIFIED:** that float is a length minus two fields. The length is the sum of squares of
  `[ebp-0x94] - [ebp-0x4c]`, passed through the same `1.5` / `0.5` table iteration the sight test uses on a distance
  (`0x640a60`). `[ebp-0x94]` is argument0 `+0x24` plus `this+0x130` rotated by the 3×3 at argument0 (`0x6406b5`,
  from `[ebp-0x40] + [ebp-0x34]`). `[ebp-0x4c]` is the translation of the 3×4 that `0x4f84f0` writes from
  `other+0x30`, minus `other+0x130` rotated by the 3×3 at `other+0x30` (`0x640063`). `0x63e070` is the address of
  `+0x130`. The two subtracted floats are `[this+0x12c]` and `[other+0x12c]`, loaded by `0x658b50` at `0x63fdc8`
  and `0x63fdd2` and still in `[ebp+0xc]` and `[ebp-0x98]` on this path. `0x658b80` is what writes `+0x12c`. The
  crosshair constructor passes `0.05` (`0x3d4ccccd`) into that writer. An earlier length in the same function is
  rejected when it is greater than the sum of those two fields plus `0.02` (`0x63fca5`, compare at `0x63fdb8`).
- **VERIFIED:** `0x63fc40`'s only caller is `0x6954d8`, in `0x695490`. That function's only caller is `0x695755`, in
  `0x6954f0`. `0x6954f0` has no direct caller. The dword at `0x7d59b4` is `0x6954f0`, slot 25 of the vtable at
  `0x7d5950`. `0x652640` and `0x5c84f0`, reached from the container rebuild, copy records of 40 bytes.
  They do not compute `+0x1c`.
- **Ruled out:** `0x696f60` through `0x697800` has no `call [reg+0x64]`. The `call [eax+0x64]` at `0x684cdb`
  reads `[return+0x1c]` as a string for `R_STR`, so that slot is not the character's plane builder.
- **VERIFIED:** `0x696f60` is slot 21 of the vtable at `0x7d5950` (`0x7d59a4`). Its `ecx` is the subobject whose vtable the
  constructor writes at `+0x1e` (`0x682ad8`, store at `0x682f8a`). The call to `0x64af10` at `0x6971fd` uses
  `ecx+0x522`, which is outer `+0x540`. `0x64c910`, called on that same pointer at `0x697210`, calls `0x647660` on
  it. The constructor builds that subobject with `0x6d7940` (`0x682da5`). `0x658900` first writes the sphere vtable
  `0x7d5008`; `0x6d7aa4` then writes `0x7d7404`.
- **STRONGLY SUPPORTED:** the RTTI on `0x7d7404` names `.?AVX_Crosshair@@`, with bases
  `X_LevelRuntimeSphereObjectBase` and `X_LevelRuntimeObjectBase` (complete-object locator `0x7fc490`, type
  descriptor `0x89aed0`).
- **VERIFIED:** slot 25 of `0x7d7404` is `0x64b4f0` (`0x7d7468`). The rebuild calls it at `0x646557`. That function
  does not call `0x6954f0` or `0x63fc40`. It divides a 40-byte container by 40 and, when the count is not zero,
  shrinks it through `0x5cac20` and `0x5c84c0`. At this call the container is the global `0x8b3b88`. The caller
  ignores the count and calls `0x64b660`.
- **VERIFIED:** `0x4f84f0` writes a 3×4. It copies `other+0x30` (`0x4f8601`) and multiplies each of its three rows by the 3×3 of `0x63fc40`'s third argument (`0x4f865f`, three passes). The translation it writes is that argument's `+0x24` (`0x4f870c`). `0x43dfc0` returns the identity at `0x8b82e0` (`1,0,0 / 0,1,0 / 0,0,1`); `0x4f84f0` copies it onto the destination and then replaces it with this product (`0x4f87f8`).
- **VERIFIED:** `0x6954f0` returns with `ret 0xc` (`0x6957fa`). Its `eax` on the empty path is the begin pointer at `[arg+4]` (`0x6957e3`). Inside it, `0x695752` subtracts `0x1e` from its `this` and calls `0x695490`, which adds `0x1e` back before `0x63fc40`. The only dword in the image equal to `0x6954f0` is the vtable slot at `0x7d59b4`.
- **STRONGLY SUPPORTED (RTTI):** the complete object is `X_Character`. Its primary vtable is `0x7d59d0` at offset 0 (complete-object locator `0x7f5f00`, type descriptor `0x8684c8`). The vtable stored at `+0x1e` is `0x7d5950`; that locator's offset field is `0x1e`, and the base at member displacement `0x1e` is `X_LevelRuntimeCapsuleObjectBase` (the level-object base sits at the same displacement). Two more vtables are stored on the same object: `0x7d59fc` at `+0x15b` (`X_ParticleSystemReceiver`) and `0x7d5930` at `+0x17c` (`X_CameraTarget`).
- **VERIFIED:** slot 25 of `0x7d4d64`, `0x7d5008`, `0x7cd010`, `0x7d5d10` and `0x7d7404` is `0x64b4f0`. Slot 25 of `0x7d5950` is the only one of these that is `0x6954f0`. Slot 25 of the primary vtable `0x7d59d0` is `0x53e210`. The image stores the immediate `0x7d5950` only at `0x682f8a` and `0x68320e`. `0x682ab0` returns the object in `eax` (`0x682faa`, `ret` at `0x682fb8`). `0x682fd3` is not inside that function: `0x682fd0` calls `0x6831e0`, frees `esi` when bit 0 of its stack argument is set, and returns with `ret 4`. The only caller of `0x682ab0` is `0x452d72`, inside `0x452cd0`, which passes a `0x810`-byte block as `ecx`.
- **VERIFIED:** `0x452cd0` stores that returned pointer at `[edi+0x8]` (`0x452e3b`) and returns `edi` (`0x4530b3`, `ret 0x48`). Caller `0x4520f2` stores the return at `[esi+0x1080]`. `0x451e60` returns `[[this+0x1080]+0x8]`, the character. Caller `0x450a04` passes `[return+0x8]` to `0x4ff850`, which adds `0x1e` and calls `0x63e070` and `0x658b50`. It does not call slot 25.
- **Ruled out:** none of the 66 callers of `0x451e60` reaches a `call [reg+0x64]` in the following 200 instructions. The callers that do add `0x1e` are `0x44dfa3`, `0x44e064`, `0x44e0e1`, `0x44e112`, `0x450eba`, `0x451045` and `0x453fdf`. The first six call `0x74e8e0` or `0x642d40`. `0x453fea` calls slot 17 (`[eax+0x44]`). `0x455b84` adds `0x1e` to the character from `[wrapper+0x8]` and calls `0x642d50`. Sixty of the callers pass the returned pointer into a direct call within 25 instructions. None of those callees contains a `call [reg+0x64]`, and neither do their direct callees. Eight pass it to `0x694430`, which returns `this+0x22e`. `0x697360` reads `[this+0x75c]`, `0x697370` reads `[this+0x764]`, `0x697380` reads `[this+0x76c]`.
- **Ruled out:** none of the 56 `call [reg+0x64]` sites in `.text` is within 80 bytes of an `lea` or `add` of `+0x1e`. `0x49b0a4` and `0x49b723` push no argument. `0x684c08`, like `0x684cdb`, reads `[return+0x1c]` as a string. The call at `0x6982b6` is inside `X_Character` method `0x697cd0`, on the mesh object. `0x5789d0` is `lea eax, [ecx+0x1e]; ret`. Its nine callers, all in `0x578029`–`0x578be7`, read the container at the returned pointer `+0x4` and do not call slot 25.
- **Ruled out:** the only two sites that call both `[reg+0x54]` and `[reg+0x64]` are `0x410209` and `0x5ebbb6`. The first uses `[ebx+0x9]` for `+0x64` and a different pointer, `[ebx+0xd]`, for `+0x54`. The second writes vtable `0x7d2b7c` onto `this` (`0x5ebb90`); that is `X_LevelDBLevel`, and its slot 25 is `0x5ef560`.
- **Ruled out:** none of the other 48 `call [reg+0x64]` sites stores a vtable onto the object in `ecx`. Back to the previous `ret`, the only stores of a code-range immediate are exception objects on the stack: `0x7cade4` (`.?AVR_MemoryException_Allocation@@`) at `0x57b386`, `0x57b43b` and `0x57b5a6`, and `0x7d12dc` (`.?AVKF_Exception_InvalidGeometry@KF2@@`) at `0x57cfc3` and `0x57d0aa`. The store of `0x7cbda4` at `0x40ded9` is in the previous function, which returns at `0x40dee2`. That table is eight pointers long; the dword at `+0x20` is outside the image, so it has no slot 25.
- **VERIFIED:** where the object is a return value, the getter does not store a vtable either. `0x57ba90` returns `[ecx+0x115]`. `0x57baa0` returns the dword at `[[ecx+0x111]]`. `0x60dbe0` returns `[ecx+0x48]`. `0x72f530` returns `[ecx+0x4c]`. `0x421390` returns `[ecx+0x93]`. `0x503100` returns `[ecx+0x1b5]`. `0x57fd40` loads `[ecx+0x110]` and returns the result of `0x76f96a`. `0x57fd70` loads `[[ecx+0x110]+0x118]`, decrements it, and returns the result of `0x57fd10`.
- **Ruled out:** none of those 48 calls adds or takes the address of `+0x1e` between the call and the previous `ret`. The same windows, read as bytes, contain no `lea` or `add` of `0x1e`. The longest window is 732 instructions.
- **VERIFIED:** the field loads used as `ecx` are `[reg+0x115]` (`0x57b6b5`, `0x57b6c7`, `0x57cb8c`), `[esi+0x2b]` (`0x4a8cab`), `[esi+0x8]` (`0x63830b`), `[ecx]` (`0x743ef9`) and `[edi+0x1b6]` (`0x7683f6`). `[ebp-0x14]`, `[ebp+0x8]` and `[esp+0x8]` are a local and two arguments.
- **Ruled out:** no dword store into `+0x115`, `+0x2b` or `+0x1b6` is in a function that adds or takes the address of `+0x1e`. `0x5614ef` and `0x57b8a8` write 0 to `+0x115`, next to 0 at `+0x110` and `+0x111`. `0x57c814` writes 0. `0x561829` copies another object's `+0x115`. `0x561bf7` writes its second argument, and its caller `0x561394` forwards its own arguments. `0x767c18` writes 0 to `+0x1b6`. `0x768111` writes its argument; one caller passes 0 and the other passes the return of `0x669680`.
- **UNKNOWN:** which call passes the pointer at `X_Character+0x1e` as `ecx` to slot 25 while Max moves. The fields loaded at the remaining calls are not stored from that address. Every other shape's slot 25 is `0x64b4f0`. `0x64db90` is not called from the flag-0 move.

## Next

1. Find the reads of `0x8b0900` and `0x8b08f0` after `0x5bf8e0` returns. A success leaves a point in `0x8b0900`. Return 2 also leaves a unit vector in `0x8b08f0`, and return 3 leaves the segment direction there.
2. Read `0x64db90` only together with a caller that the character update reaches. The projectile caller is not that.
