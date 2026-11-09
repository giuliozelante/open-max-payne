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
- **VERIFIED:** the immediates `0x8b0900` and `0x8b08f0` occur only inside `0x5bf8e0`, nine times each. No instruction loads
  the floats. The same pair is passed to `0x5b9f00` (`0x5bfe56`, `0x5bff43`, `0x5c01f2`), to `0x5ba5b0`
  (`0x5c02d1`, `0x5c03f2`, `0x5c04f6`), and to `0x5ba000` (`0x5c02fb`, `0x5c041c`). A return of 3 or 4 from
  `0x5b9f00` jumps to `0x5c051d`. Any non-zero from `0x5ba5b0` or `0x5ba000` does the same. `0x5c051d` sets `eax`
  to 1 and returns. The point and the direction written through those pointers are not read.
- **VERIFIED:** `0x5b9f00` (`ret 0xc`) at `0x5bfe56` and `0x5bff43` takes `ecx = 0x8b0900`, `edx = 0x8b08f0`,
  and stack arguments `0x8b0878`, `0x8b08d0`, `0x8b08c0`. The call at `0x5c01f2` passes `[esp+0x5c]` and `edi` in
  place of the last two. `0x5b9fe0` is `this+0x30` and `0x5b9ef0` is `this+0x24`. It copies `arg0+0x30` to
  `0x8b0d38+0x10` and `arg0+0x24` to `0x8b0d38+4` and to `edx`. `0x5ca490` returns the dot of `this+4` with
  `point - (this+0x10)`. With `s0` that value for the first point and `s1` for the second, `0x5b9d30` returns 0
  when both are greater than 0 and 1 when both are less than 0. Both zero copies the first point and returns 2.
  `s0 == 0` with `s1 != 0`, and `s0 > 0` with `s1 == 0`, copy a point and return 3. `s0 < 0` with `s1 == 0` copies
  the second point and returns 4. Otherwise `t = s0 / (s0 - s1)` is clamped to `[0, 1]`, `0x507f50` writes the
  first point plus `t` times the difference, and the return is 4 when `s0 < 0` and 3 otherwise. `0x5b9f00` turns
  an inner 0 into 1 and an inner 1 into 2. An inner 3 or 4 is returned unchanged when `0x5f70e0` returns 0, and
  becomes 0 when `0x5f70e0` returns non-zero. `0x5f70e0` returns 1 at `0x5f7154` when the absolute dot of
  `(argument - this+0x30)` with `this+0x24` is greater than `0.001` (`0x7d3050`). It returns 0 at `0x5f7337`.
- **VERIFIED:** from the call in `0x646020`, slot 23's `[ebp+0x10]` and `[ebp+0x14]` are the same position pointer.
  `0x5f2c20` is `this+0x18` and `0x5f2c10` is `this+0x8`, so `0x5bf8e0` receives `child+0x88`, `child+0x18`, the
  dword at `child+0xc`, that position pointer twice, the float at `this+0x12c`, and the rotated `this+0x130`.
  `0x5bf1d0` copies `child+0x18` to `0x8b0878+0x24`. `0x5bf1f0` copies the vec3 at `[child+0xc]` to
  `0x8b0878+0x30`. `0x8b0940` is the rotated `this+0x130`, negated at `0x5bfd33` when its dot with `0x8b0878+0x24`
  is negative. The float's stack slot is overwritten at `0x5bfb26` with `1.05` times the float plus `0.025`
  (`0x7d20c0`, `0x7d20c4`). `0x8b0910` is `child+0x18` multiplied by that sum. At `0x5bfd73` both point globals
  are that position minus `0x8b0940`, and then minus `0x8b0910`. `0x5ca330` is called with `0x8b0910` and each
  point, and its return is passed with `0x8b0878` to `0x5bafa0`. A non-zero `al` jumps to the return of 1
  (`0x5bfe01`, `0x5bfe2a`). At `0x5bfe76` both point globals are overwritten
  with the position plus `0x8b0940`, and that is what `0x5b9f00` tests. The two points are equal on this call, so
  the two dots are equal.
- **VERIFIED:** `0x5bafa0` (`ret 0xc` at `0x5bb0d0`) does not read its incoming `ecx`. `arg0` is the object,
  `arg1` and `arg2` are vec3 pointers. It returns 0 at `0x5bb0d5` when the absolute dot of `arg0+0x24` with `arg1`
  is below `0.001` (`0x7d2084`, same bits as `0x7d3050`). Otherwise
  `t = dot(arg0+0x24, arg2 - arg0+0x30) / dot(arg0+0x24, arg1)`, using `0x5b4900`. `t > 1` and `t < 0` also return 0.
  The kept `t` writes `arg2 - t*arg1` to `0x8b0c28` and returns 1 only when `0x5f70e0` on that point returns 0.
  On the calls before the point stores, `arg1` is the rotated `this+0x130` and `arg2` is the position. On the later
  calls, `arg2` is `0x8b08d0` or `0x8b08c0` and `arg1` is the vec3 `0x5ca330` wrote.
- **VERIFIED:** when the absolute dot is not above `0.001`, `0x5f70e0` walks `count = [this+0x18]` vec3s. A count of 0 or
  less returns 0. Element `i` is at `[[[this+0x10]+4] + ([this+0x14] + i) * 12]`. The first pair uses element
  `count - 1`, then each element pairs with the one before it. The edge is current minus previous, crossed with
  `this+0x24`, then divided by its length (`fsqrt`, no zero test). If the dot of `argument - current` with that
  unit is greater than `0.001`, it returns 1. This compare is not an absolute value. If none are, it returns 0.
  `[this+0x18]` is the dword `0x5ca5c0` increments, and `[this+0x14]` is the vec3 count saved at the first append.
- **VERIFIED:** `0x5b5ad0`, called at `0x5b5900`, empties `this+0x88` and appends copies of vec3s from `this+0x8`.
  Each record is 24 bytes: the dot of that vec3 with one vector, the dot with a second vector, and the index.
  The first vector is `(1, 0, 0)` crossed with `this+0x18`. When that equals `(0, 0, 0)`, it becomes `(0, 1, 0)`
  crossed with `this+0x18`. When that is also zero, it is `(0, 0, 1)` and is not crossed. The second vector is the
  first crossed with `this+0x18`. `0x5b6190` returns 1 only when all three components compare equal. `0x5b5990`
  returns 1 when the first record's first double is greater and -1 when it is less. On a tie it compares the second
  doubles the other way. The append then reads the records from the last toward the first.
- **VERIFIED:** slot 0 of the vtable at `0x7d1fa4` is `0x5e9080`. It reads an unsigned count through the import at
  `0x7cab1c`, then calls the import at `0x7caafc` that many times and appends each 12-byte result to `this+0x8`.
  The same import then writes `this+0x18`, and the import at `0x7caaf0` writes `this+0x24`. Slot 2 of that vtable
  is `0x5b58f0`, the only caller of the copy into `this+0x88`.
- **STRONGLY SUPPORTED:** the complete-object locator at `0x7ed4b0` names `.?AVX_LevelDBImportExit@@`. The import
  at `0x7caafc` is `R_MemoryFile`'s extractor for `M_Vector3Template`, `0x7cab1c` extracts an unsigned int, and
  `0x7caaf0` extracts `M_Matrix4x3Template`.
- **VERIFIED:** `0x642d40` returns `[ecx+0x60]`. The call at `0x64636c` pushes that pointer as `arg0` of `0x646020`.
  The child passed to slot 23 is `[[arg0+0x10] + i*4]`. `0x646020` does not load that element's first dword.
  It does read `[element+0x80]` through `0x40e420`. The constructor that writes vtable `0x7d1fa4` stores 0 at
  `+0x80` and does not write that field again.
- **VERIFIED:** `0x642ed0` stores its first stack argument at `this+0x60` and sets the bytes at `+0x88` and `+0xb0`
  to 1. It also copies nine dwords from its third argument onto `this+0x30`. At `0x642597` that first argument is
  the return of `0x656680`, which walks `[[container+8] + i*4]` and returns the element whose string compares
  equal under `_stricmp` (`0x7ca358`), or 0. At `0x648592` the stored pointer is one just read from another
  object's `+0x60`. The stores of the dwords inside the array at `+0x10` are not in these functions.
- **Ruled out:** the pointer array `0x63aa10` reads is not written by the stores read this pass.
  `0x438ef5` copies 12 dwords between two stack locals. `0x43940e`, `0x43943a`, and `0x439460` write
  `[pointer+0x10]`, where the pointer is the return of `0x43aea0` or `0x43abe0`, and they do not change a
  dword at `+0x14`. `0x40da80` writes 0 from `+0x8` through `+0xe8`. `0x6471a1` and `0x64b178` call
  `0x64ffc0` with `ecx = this+0x10`, `arg0 = [this+0x14]`, and `arg1 = [this+0x18]`. Both exits of that
  call, `0x6500f8` and `0x650104`, write `[ecx+8] = arg0`, so the call sets `[this+0x18] = [this+0x14]`.
  The loop it takes when those two differ steps by `0x64` and calls `0x41c3a0`, which is `ret`. That
  container is the 100-byte element vector, not the pointer array. No instruction adds 4 to `[reg+0x14]`
  for `eax`, `ecx`, `edx`, `ebx`, `esi`, or `edi`.
- **VERIFIED:** the characters `0x656680` compares are at `[[[element+8]+0x148]+4]`. `0x63aa40` returns that
  address: `0x748300` reads `[element+8]` and `0x5e3b20` is `lea eax, [ecx+0x148]`. A null pointer is
  replaced by the dword at `0x7caa2c`, as the argument string is. `0x63a320` returns `[element+0x1c]`.
  `0x6566f0` walks that as a list: the next node is `[node]`, and `0x64b370` on `[node+8]` returns
  `[node+8]+0x110`, which is passed to the import at `0x7cac44`. That list is not the pointer array at
  `+0x10`. The functions from `0x656770` to `0x6572b0` operate on tree nodes with fields at `+8` and
  `+0x14`. They do not advance the pointer array's end.
- **STRONGLY SUPPORTED:** the constructor at `0x5e3490` writes vtable `0x7d2690` and then `0x7d2808`.
  The complete-object locator behind `0x7d2808` names `.?AVX_LevelDBRoom@@`. It zeros `[this+0x10]`,
  `[this+0x14]`, and `[this+0x1c]`, and it calls `std::rs::_Tidy` (`0x7caa9c`) on `this+0x148` with
  argument 0. It stores nothing at `+8`. Its only direct caller is `0x5cf91d`, inside `0x5cf900`,
  which then writes vtable `0x7d2264` (`.?AVX_LevelDBImportRoom@@`) and the return of `0x5cc300` at
  `+0x188`. `0x5cfe20` builds those rooms in an array whose elements are `0x18c` bytes. The name
  lookup reads `[[[element+8]+0x148]+4]`, so this constructor does not show that the element is the room.
- **VERIFIED:** `0x6d99e0` passes its first stack argument to `0x642ed0`, which stores it at the crosshair's
  `+0x60`. `0x691180` forwards its own first argument there, and also to `0x642a90` on the capsule at
  `character+0x1e`. `0x687fce` passes `[character+0x6d8]`. The non-zero store of that field is `0x69aef8`:
  it calls `0x642d40` on `[ebp-0x1c]`, then `0x63a280` on that result. `0x63a280` calls `0x76f96a`, reads
  `[return+0x184]`, and returns `0x656600` of the container at `[this+4]` for that index. `0x683915` and
  `0x688047` write 0 to `+0x6d8`. None of these stores writes `[element+8]` or advances `[element+0x14]`.
- **Ruled out:** no instruction loads `[reg+0xc]`, adds 4, and stores that register back. `0x5dad80`,
  the usual call on `[reg+8]`, swaps `[ecx]` with `[edx]`. At `0x65c051` a lookup is followed by an
  allocation of `0x29b` bytes and `0x72f7e0`. `0x65f680` then inserts into the tree at `edi+0xdb`; it
  compares keys at `node+0xc` with the sentinel `0x8b3f58`. `0x65c110` writes the new object to
  `[node+0x10]`. That is a tree node's value, not the pointer array.
- **VERIFIED:** `0x72fdb0` (`ret 0x10`) does not store a dword at `+8`, `+0x10`, or `+0x14`. It writes
  the byte 0 at `+0x10` and `+0x14`. It passes `[this+0x18]` and `[this+0x1c]` to
  `P_Buffer::setUsedCount` (`0x7ca80c`) and calls `P_TriangleMeshBase::lock` (`0x7ca58c`) on `this`.
  At `0x72fe85` it calls `0x6423e0` with `ecx = this+0x140`. The two stack values are the
  `R_MemoryFile` from `Chunk::getMemoryFile` (`0x7caba0`) and this function's first argument.
  `0x6423e0` passes its second stack argument to `0x656680`.
- **VERIFIED:** `[esp+0xc8]` in `0x65bf80` is that function's second argument. `0x65bf80` is slot 5 of
  `X_DecalImplementation` (vtable `0x7d5178`, COL `0x7f4948`, offset 0, `ret 0x8`). It is the container
  passed to `0x656680` at `0x65c051`, and the same pointer goes to `0x667d00` (`ecx = this+0x22`) and
  `0x58f860` (`ecx = this+4`). None of the three writes `[container+8]` or `[container+0x14]`.
  `0x667d00` increments the dword at `[this+0xc]` of the subobject and stores a node at `[this+4]`.
- **VERIFIED:** `0x668937` does not call that slot. Its `this` is the decal base at `+0x22`, and the call
  uses the vtable of `[this+0x16]`, an `X_ParticleSystemImplementation` (`0x7d57e8`, slot 5 is `0x673d90`).
  The decal slot caller is `0x454676`, in `MaxPayne_GameMode` slot 25 (`0x454230`, vtable `0x7ccec4`).
  `ecx` is the decal at `[GameMode+0x9b]`. Argument 0 is `Chunk::getMemoryFile`. Argument 1 is the
  address `GameMode+0x13d` (`0x496bc0` on `GameMode+0x104`). That call does not write the container.
- **VERIFIED:** `0x655ea0`, called from `0x493bf6` on `GameMode+0x104`, zeroes `[this+8]`, `[this+0xc]`, and
  `[this+0x10]`. `0x656620` then sets `[this]` to 0 and `[this+0xc]` equal to `[this+8]`. `0x655f00` frees
  `[this+8]` and zeroes the same three fields. `0x6564c0` pops one pointer when the byte at `+0x24` is
  nonzero: it calls slot 0 of `[[end]-4]` with argument 1 and moves the end back by 4. `0x45008e` calls
  `0x493f80` with `ecx = GameMode+0x104`. That calls `0x655f90` on `this+0x39`, stores its argument at
  `[container]`, sets the byte at `+0x24` to 1, and walks objects whose name is read by `0x5e3b20`.
  At `0x656258` it calls `0x447dd0` with `ecx = container+4`. The first argument is `[container+0xc]`,
  the second is 1, and the third is the address of the `0xe0`-byte object `0x639bd0` returned.
  When no free slot remains, `0x447e62` writes that pointer into the new buffer and `0x447ed3`
  (or `0x447ef4`) stores the new end at `[container+0xc]` and the new begin at `[container+8]`.
  When a slot remains, `0x447f57` writes the pointer at the current end and `0x447f73` adds 4 to
  `[container+0xc]`. `0x639bd0` zeroes the new object's `[this+0x10]` and `[this+0x14]`.
- **VERIFIED:** `0x639bd0` does not store `[this+8]`. It writes vtable `0x7d4d10`,
  `X_LevelRuntimeRoom`, then calls `0x639ee0`. `0x639f43` stores the second argument at `[this+4]`
  and `0x639f46` stores the first argument at `[this+8]`. At the call from `0x6561ea` the first
  argument is the return of `0x76f96a` and the second is the container.
- **VERIFIED:** `0x747810` is `ret 0x8`. The call at `0x639f6b` discards two arguments and does not insert.
  `0x561030` returns its destination because `[room+0x14]` and `[room+0x10]` are both still 0, so
  `0x639f73` writes 0 back to `[room+0x14]`. That is not a child-array append.
- **VERIFIED:** `0x63ce30` (`ret 0xc`) inserts into the pointer vector at `room+0xc`. Begin is
  `[room+0x10]`, end is `[room+0x14]`, and capacity is `[room+0x18]`. `0x639bf9` stores 0 there too, so
  the first call has no free slot. Argument 0 is the current end, argument 1 is 1, and argument 2 is
  the address of the `0x76f96a` return. `0x63d060` asks `0x76f5b6` for `count*4` bytes. `0x63cec2`
  writes that return into the new buffer, `0x63cf36` stores the buffer at `[room+0x10]`, and
  `0x63cf33` stores `buffer+4` at `[room+0x14]`. When a slot remains, `0x63d03b` writes the pointer
  and `0x63d044` adds 4 to the end. `0x5e3a30` returns the first dword of `[[room+8]+0x2c]` when it
  differs from `+0x30`, else 0. `0x5e3a50` advances the cursor at `[room+8]+0x160` by 4. Each
  non-zero dword is passed to `0x76f96a` with `.?AVX_LevelDBExit@@` (`0x875ed8`) and
  `.?AVX_LevelDBImportExit@@` (`0x875ef8`).
- **VERIFIED:** `0x76f96a` is `jmp [0x7ca344]`, MSVCRT `__RTDynamicCast`. The call at `0x639f89`
  passes the source dword, a vf delta of 0, `X_LevelDBExit`, `X_LevelDBImportExit`, and
  `isReference` 0, so a failed cast stores 0. The call at `0x6561c3` casts the return of slot 9
  (`call [eax+0x24]`) from `.?AVX_LevelDBRoom@@` (`0x876148`) to `.?AVX_LevelDBImportRoom@@`
  (`0x876168`). That return is the first argument of `0x639bd0` and is already stored at `[room+8]`.
- **VERIFIED:** `0x5f2be0` is `lea eax, [ecx+0x28]; ret`. `0x5e39e0` returns the first dword of the vector at
  `[[room+8]+0xc]` when it differs from `+0x10`, else 0. The 9 dwords copied to `[room+0x80]`, and then
  to `[room+0x5c]`, are that object's `+0x28`. Nothing tests the pointer before the copy. `0x63a063`
  treats `[room+0x5c]` as a 3x3, divides the cofactors by the determinant (`fdivr` of `1.0` at
  `0x7cbdd0`), and `0x63a14b` writes that result back over `[room+0x5c]`. `[room+0x80]` keeps the
  original 9 dwords. That is not a mesh.
- **VERIFIED:** `0x63d9f0` (`ret 4`) does not write. `0x735570` is `mov eax, [ecx+0x14]; ret`, called on
  the `X_LevelDBImportLevel` at `[ebp-0x28]`. The search walks that object's vector, begin at `+8` and
  end at `+0xc`, stride 8, and compares `[record+4]` with the element. A match returns its index. No
  match returns the count, and an empty range returns 0. The caller stores that index and `0x63a1d0`
  calls `0x5644a0` with `ecx = room+0x28`, the dword at `[room+0x30]`, and the address of the index.
- **VERIFIED:** `0x5644a0` (`ret 8`) appends that index to the vector at `room+0x28`. Begin is
  `[room+0x2c]`, end is `[room+0x30]`, and capacity is `[room+0x34]`. `0x639bff` stores 0 at all three,
  so the first call has no free slot. Argument 0 is the current end and argument 1 is the address of
  the index. `0x43f390` asks `0x76f5b6` for `count*4` bytes. `0x661290` writes the index into that
  buffer, `0x56457e` stores the buffer at `[room+0x2c]`, and `0x564587` stores `buffer+4` at
  `[room+0x30]`. When a slot remains, `0x56a6d0` writes the index at the current end and `0x56461b`
  adds 4. A miss still appends the count `0x63d9f0` returned.
- **VERIFIED:** `0x63a480` does not append. `0x748300` returns `[room+8]`. For each element of that
  vector, `0x5f2ba0` returns `([element+0x10]-[element+0xc])/12`, or 0 when `+0xc` is null, and
  `0x5f2bc0` returns `[element+0xc]+index*12`. `[room+0x38]` starts at `FLT_MAX` (`0x7f7fffff`) and
  keeps the lesser component. `[room+0x44]` starts at `-FLT_MAX` (`0xff7fffff`) and keeps the greater
  component. The scale runs unless a `(max-min)` component compares greater than `5000.0`
  (`0x7d4d38`); that path calls `0x63aa40` and leaves the corners. Otherwise the corners are moved to
  `1.2` (`0x7d4d08`) times their offset from the center. Neither path reads `[room+0x10]` or
  `[room+0x2c]`. The 12-byte records are not shown to be triangles.
- **VERIFIED:** `0x5ca660` does not read those points. Its `this` is `[importRoom+0x188]+4`. It adds
  the index to `[this+4]` and calls `0x5b89c0` with `[this]`. `0x5da220` returns `[base+4]`, and
  `0x5b89c0` adds `(index+[container+8])*68` to that. The caller passes `[record+0x40]` to `0x542130`,
  and a non-zero `al` stores 1 at `[room+0xda]`.
- **VERIFIED:** `0x542130` is `mov al, [ecx+0x29]; ret`. `[room+0xda]` becomes 1 when that byte is
  non-zero and stays 0 when every tested byte is zero. `0x63a250` returns `[ecx+0xda]`. Its only
  direct caller, `0x67a9d7`, walks the pointer vector at `this+0x5ec` and passes `[element+0x6c]`.
  `0x67a9b0` returns 1 if any of those bytes is non-zero, otherwise 0. Its only direct caller is
  `0x44ecf7`, with `ecx = [esi+0x1074]`.
- **VERIFIED:** a non-zero result at `0x44ecf7` is not a collision test. `0x44ed00` calls `0x6da420` on
  `esi+0x12d2`. That returns 0 when `[this]` is 0 and 1 otherwise. Only the 1 path saves
  `P_Material::getRuntimeFogEnable` (`0x7ca5b8`), calls `setRuntimeFogEnable` (`0x7ca558`) with `cl`
  cleared, and calls `0x60de40` with the string `Rendering` (`0x888dbc`). `0x7482e0` returns `[ecx+4]`
  and `0x748300` returns `[ecx+8]`; the stack values are arguments of `0x60de40` (`ret 0x14`). The saved
  fog value is restored at `0x44ed6b`. A zero flag skips this block and joins at `0x44ed75`.
- **VERIFIED:** `0x5e3490` stores 0 at `[importRoom+0xc]`, `+0x10`, and `+0x14`. The vector object is at
  `room+8`. `0x5e57f0` appends one dword. Vtable slot 0 (`0x5e3b30`) and `0x5e4550` (reached from slot 2)
  both call it. The value is 0 when the byte at `[[[room+4]+0x14]+0x14]` is 0 and `0x754650` returns 0,
  or when `0x5cb8d0` returns the end pointer `[container+0xc]` (`0x5da210` reads `[ecx+8]`). Otherwise the
  appended dword is `[node+4]`. The 12-byte points would be at that object's `+0xc`. They are not a mesh.
- **VERIFIED:** `0x5cb8d0` returns one 8-byte record from a contiguous array. `0x5cbc90` searches it with stride 8.
  Begin is `[map+4]` (`0x5d7d40` reads `[ecx+4]`). End is `[map+8]` (`0x5da210` reads `[ecx+8]`). `0x5b56e0`
  compares the first dword of each pointer (`cmp`, then `setl`). The record is kept only when those dwords are
  equal. The room copies `[record+4]`. The key is the dword at `[[importRoom+0xac]+index*4]`.
- **VERIFIED:** slot 20 (`0x5cfa90`) builds the rooms through `0x5cfe20` (size `0x18c`, constructor `0x5cf900`).
  `0x421a60` stores `[parent+0x2c]` at each new room's `+4`. The map then sits at `[[[room+4]+0x14]+4]`.
  Neither call writes `[record+4]`.
- **VERIFIED:** `0x5f5ee0` appends one 8-byte record. Begin is `[this+4]`, end `[this+8]`, capacity `[this+0xc]`.
  Slot 0 of vtable `0x7d2f38` (`0x5f4380`) and slot 2 (`0x5f4620`) call it. The second dword is the
  `__RTDynamicCast` return whose type is `.?AVX_LevelDBMeshPolygon@@` (`0x886bd8`). The first dword is what
  IAT `0x7ca980` writes. That vtable is `X_LevelDBObjectContainerBase` of `X_LevelDBMeshPolygon` with
  `std::less<int>`. A sibling container for `X_LevelDBStaticMesh` is vtable `0x7d3018`. The room searches
  the same record shape. This does not show that `[[importRoom+4]+0x14]` is vtable `0x7d2f38`. The polygon
  constructor writes separate dwords at `+0xc`, `+0x10`, and `+0x14`. Those are not the point vector.
- **VERIFIED:** `[importRoom+4]` is the `X_LevelDBImportLevel`. Slot 10 of vtable `0x7d1ff8`
  (`0x5b74a0`) passes that level to `0x5b8000`, which passes it to `0x5cfa00`. `0x5e5400` pushes
  that argument at `0x5e558a` and `0x5ecd80` stores it at `[container+0x2c]`. `0x5cfa90` copies
  `[container+0x2c]` to each new room's `+4`. `[[importRoom+4]+0x14]` is `[ImportLevel+0x14]`.
  `0x5eba70` stores 0 there, then `0x5ebae0` fills it. The import constructor sets vtable `0x7d1ff8`
  and calls `0x5ebae0` again. That call stores slot 11's return at `[level+0x14]` (`0x5ebb12`) and
  slot 9's return at `[level+0xc]` (`0x5ebb26`).
- **VERIFIED:** slot 11 is `0x5b74b0`. It allocates `0x40` bytes and calls `0x5ac8a0`, which writes
  vtable `0x7d1f6c`. That type is `.?AVX_LevelDBImportStaticMeshContainer@@`. Slot 9 is `0x5b7490`.
  It allocates `0x2c` bytes and calls `0x5b5da0`, which writes vtable `0x7d1fc0`. That type is
  `.?AVX_LevelDBImportExitContainer@@`. `0x5e9520` puts the 20-byte vector at that object's `+4`
  (begin `[this+8]`, end `[this+0xc]`). The room's appended dword is `[record+0x10]` from that
  vector, or 0.
- **VERIFIED:** slot 0 of that container (`0x5b6090`) calls `0x5b60a0`. `0x5b64b0` passes the
  element at `[this+0x1f] + index*[this+0x23]` to `__RTDynamicCast` with both types `0x875ed8`
  (`.?AVX_LevelDBExit@@`), vf delta 0, and `isReference` 0. `0x5da266` stores that return at the
  new record's `+0x10`. `0x5b6590` inserts the record into the vector at `this+4`. `0x5da6d3`
  copies `[source+0x10]` onto the inserted element. The room's appended pointer is that exit.
- **VERIFIED:** slot 0 of the static-mesh container is `0x5f37e0`. It calls `0x5f4a10`, which inserts
  into the map at `this+4`. `0x5f4af1` casts `[this+0x1f] + index*[this+0x23]` to
  `.?AVX_LevelDBStaticMesh@@` (`0x886e18`, both sides, vf delta 0, `isReference` 0). `0x5f4b2f` stores
  that return as the record's second dword. `0x5f6090` inserts the 8-byte record, and `0x5d03fe` copies
  `[source+4]` to `[dest+4]`. The room's element is that static mesh. The 25-byte array at `this+0x30`
  is a different vector.
- **VERIFIED:** `X_LevelDBStaticMesh` is vtable `0x7d2f70` (COL `0x7ef1b0`). Constructor `0x5f28d0` zeroes
  `[this+0xc]`, `[this+0x10]`, and `[this+0x14]`. Slot 0 `0x5f2f70` reads an unsigned count through IAT
  `0x7cab1c` (`operator>>` of `R_MemoryFile` and `unsigned int`) and grows the vector at `this+8` by
  12-byte records (`0x2aaaaaab`, `sar 1`). IAT `0x7caafc` writes each one: `operator>>` of
  `M_Vector3Template<M>`, with `edx` at `[this+0xc] + index*12`. The same reader fills the vector at
  `this+0x18`, sized by an int from IAT `0x7ca980`. IAT `0x7caaf0` then reads an
  `M_Matrix4x3Template<M>` into `[this+0x28]`. These records are vector3 values, not triangles.
- **VERIFIED:** `0x5f5260` allocates `0x44` bytes and calls `0x5f1e40`. That constructor writes `0x7d2690`,
  then `0x7d2f38`, then `0x7d2f00`, and leaves vtable `0x7d2ec8` at `0x5f1f6f`. COL `0x7ef108` names
  `.?AVX_LevelDBMeshPolygonContainer@@`. `0x5f2b52` stores that pointer at `[staticMesh+0x58]`. Slot 0 of
  the mesh calls slot 0 of that object, `0x5f22c0`, with the memory file. The base vtable `0x7d2f38` is
  not the finished object.
- **VERIFIED:** `0x5f22c0` calls `0x5f4380` with the memory file, then casts `[record+4]` to
  `.?AVX_LevelDBMeshPolygon@@` (`0x886bd8`, both sides). It then reads an int through IAT `0x7ca980`
  and an `M_Vector3Template<M>` through IAT `0x7caafc`. `0x5f57a0` inserts that pair into the map at
  `this+0x34`. `0x5f54c0` copies the int and the three floats onto the node at `+0xc`. One int and one
  vector are not a triangle.
- **VERIFIED:** slot 0 `0x5f1cb0` reads two ints into `[this+0xc]` and `[this+0x10]` through IAT
  `0x7ca980`, and an `M_Vector3Template<M>` into `[this+0x14]` through IAT `0x7caafc`. An unsigned
  count from IAT `0x7cab1c` is stored at `[this+0x20]`. Two more ints go to `[this+0x2c]` and
  `[this+0x30]`, two floats from IAT `0x7cab24` go to `[this+0x34]` and `[this+0x38]`, and one int
  goes to `[this+0x3c]`. `0x5f1c94` stores the return of `0x5de320` at `[this+0x24]`. `0x5f1ca4`
  stores 0, or `[record+4]` from an 8-byte range on `[[this+4]+8]`, at `[this+0x28]`. Two ints and
  one vector are not a triangle.
- **VERIFIED:** slot 6 of `X_LevelDBImportStaticMesh` is `0x5a70a0` (vtable `0x7d1ee8`). `0x40e390`
  returns `[this+0x58]`, and the function calls that object's slot 10. The return is cast from
  `X_LevelDBMeshPolygon` (`0x886bd8`) to `X_LevelDBImportMeshPolygon` (`0x886c00`). `0x65a2e0` returns
  `[polygon+0x10]`. `0x5f19b0` returns the dword at `[[polygon+8]+4] + ([polygon+0xc] + arg)*25`.
  `0x5f2bc0` uses that dword as the index into this mesh's 12-byte vector. Corners 0 and 1 are read
  before the count is tested. Corners 2 and up are read while the index is less than `[polygon+0x10]`.
  The int at `+0xc` is the base added to the corner number, not the vertex index. The count is not
  fixed at 3.
- **VERIFIED:** `0x5f1930` stores 0 at `[this+8]` (`0x5f1951`). The pointer `0x5f19b0` reads is written
  later. Slot 11 of the polygon container (`0x7d2ec8`) is `0x5f2250`, and `0x5f228c` copies
  `[polygonContainer+0x30]` into `[element+8]`. `0x5f22b0` is `mov [ecx+0x30], arg; ret 4`. Slot 11 of
  `X_LevelDBImportStaticMeshContainer` (`0x5ac930`, vtable `0x7d1f6c`) and its slot 0 `0x5f37e0` both
  pass the address of `this+0x30` through `0x40e390`, which returns `[mesh+0x58]`. `[importPolygon+8]`
  is that address, so its `+4` is `[container+0x34]`, the begin of the 25-byte records. `0x5cc219` and
  `0x5ef869` pass `this+0x2c` for elements of `0xaa` and `0xa6` bytes. The stores at `0x5a522d`,
  `0x5a52d0`, and `0x5a52e8` copy `+4`, `+8`, and `+0xc` together; that object is a string.
- **VERIFIED:** `0x5b3c10` takes the address `0x5f19d0` returns, record `+4`. It loads that dword and the
  dword at record `+8`. `0x5ada30` stores them as the first two components of a 12-byte vector, and the
  third component is the constant 0. The three calls pass corners 0 and 1 and the corner index saved at
  `[ebp-0x44]`. Each result is a stack temporary. `0x5adb50` subtracts two of those vectors with `fld`
  and `fsub`, so both dwords are floats.
- **VERIFIED:** both callers of `0x5f19f0` (`0x5a7dfb`, `0x5a7f94`) load the dword at the returned address
  and the next dword. Those are record `+0xc` and record `+0x10`. The first stores them at
  `0x8b0150+0x28` and `+0x2c`; the second stores them at `0x8b00f8+0x28` and `+0x2c`. The polygon
  pointer is stored at `+0x30`. `0x5f1ae0` returns the byte at record `+0x18`, stored at `+0x3c`.
  `0x5acfc0` passes that block to `0x5ae3c0`, and `0x5b09d2` divides the container by 64, so the block
  is one 64-byte element. No instruction in `.text` loads `0x8b0178` except its store.
- **VERIFIED:** `0x5acfa0` returns the container begin plus the index times 64. `0x5a953d` loads
  `[element+0x28]` and `0x5a9548` loads `[element+0x2c]`. That container is the second stack argument of
  `0x5a8b90`, loaded at `0x5a8e15`. The calls at `0x5a803e`, `0x5a8097`, and `0x5a80f5` pass `0x8b02e8`.
  `0x5abf40` loads both dwords with `fld` and returns 1 when each component is within the immediate
  `0x3b03126f` (0.002) of the other pair.
- **VERIFIED:** `0x5f1a10` reads the dword at record `+0x14`. It returns -1 when that dword is -1, and
  otherwise returns the dword masked with `0x7fffff`. The callers `0x5a7470`, `0x5a7567`, and `0x5a7665`
  skip the lookup when the return is negative. Otherwise `0x5c3eb0` returns the begin of the container
  at `0x8b02a8` plus that index times 8, and the caller reads the record's first dword. With the byte
  at `+0x18`, the 25 bytes of the record are accounted for.
- **VERIFIED:** `0x5a7178` stores `esi` at `0x8b0290` and the return of `0x65a2e0` at `0x8b0294`.
  `0x65a2e0` returns `[polygon+0x10]`, the corner count. `esi` starts at 0 (`0x5a70d7`). After the store,
  `0x5a7185` adds that count minus 2 to `esi`. `0x5ad380` inserts the two dwords into the container at
  `0x8b02a8`, and `0x5b4670` copies them onto the element. The other path stores -1 at `0x8b0290`
  (`0x5a718b`) and does not advance `esi`.
- **VERIFIED:** `0x64db90` returns 0 at `0x64dd4f` when the product of two dots is ordered and greater than
  or equal to the negative `FLT_MIN` at `0x7d4ec8` (`0x80800000`). The normal is the cross product of
  `(second stack argument - first)` and `(third stack argument - first)`. The dots are that normal with
  `ecx - first` and with `(ecx - first) + edx`. The only direct caller is `0x698836`, inside `0x698130`.
  `0x709aa8` is the projectile caller, in `0x7097d0`. The other caller is `0x6d921d`, in `0x6d8be0`.
  The dword at `0x7d7480` is `0x6d8be0`, slot 31 of the crosshair vtable `0x7d7404`. No direct call
  targets `0x6d8be0`. No direct-call chain from the character update `0x696f60` reaches `0x698130`,
  and none of the functions that chain reaches contains `call [reg+0x7c]`.
- **VERIFIED:** `0x8b49b8` is `[0x8b4920+0x98]` and `0x8b4954` is `[0x8b4920+0x34]`. No instruction
  stores either absolute address. The character update passes `0x8b4920` as the second stack argument
  of `0x64af10`. `0x654f20`, called on `0x654aa0`'s object at `0x654d13`, writes 0 to `+0x34` and `-1`
  (`0xbf800000`) to `+0x98`. `0x654efa` then writes the dword at `0x7d4f6c` (`FLT_MAX`, `0x7f7fffff`)
  to `+0x98` and does not write `+0x34` again. `0x64b013` copies `+0x34` into the global and `0x64b0de`
  copies `+0x98`. `0x64c910` stores neither field. It calls `0x647660` with 0 in `[ebp+0x10]`, and that
  function jumps to `0x6478e9` when the byte is 0, past its own call to `0x654aa0`. The compare at
  `0x69722b` is `fcomp` against `FLT_MAX`; `test ah, 0x40; jne` returns 1 when the two values are equal.
  The pointer test at `0x697241` is the other arm.
- **VERIFIED:** the image stores `0x696f60` only at `0x7d59a4`. Of the 13 `call [reg+0x54]` sites, only
  `0x44d754` and `0x764e8d` test `al`. `0x44d751` calls the slot on `ebx+0x258` and jumps to `0x44d7be`
  when `al` is 0. `0x764e8a` calls the slot on the pointer `0x7269e0` returns, `[ecx+0xc]`, and jumps to
  `0x764ebf` when `al` is 0. None of the 13 sites adds `0x1e` in the preceding `0x80` bytes. `0x40ec08`
  multiplies `ST0` after the call. `0x743e90` forwards the callee's `eax`; `0x69c9a5` adds `0x185` before
  calling it, and `0x73aff5` returns that `eax` without testing it.
- **VERIFIED:** `0x5e1fe0` inserts one 20-byte record. `0x5e2310` copies `[source+0x10]` to
  `[dest+0x10]` at `0x5e2457`. `0x5e0a40`, slot 0 of vtable `0x7d27ac`, stores the `__RTDynamicCast`
  return of `.?AVX_LevelDBMaterialCategory@@` (`0x8876d0`) at that offset and passes the record to
  `0x5e1fe0`. The vtable's type is `X_LevelDBObjectContainerBase` of `std::rs`,
  `X_LevelDBMaterialCategory`, and `r_stril`. This does not show that `[ImportLevel+0xc]` is that
  container.
- **VERIFIED:** `0x64db30` saves the byte, clears it, and calls `0x64be70` with the vector at `+0xb1` only when that vector's sum of
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

1. The two sites that test the byte are not formed with `+0x1e`. Do not keep scanning `call [reg+0x54]`. `0x696f60` is still only stored at `0x7d59a4`. Do not reopen `0x44d751`, `0x764e8a`, `0x743e90`, `0x64af10`, `0x654aa0`, `0x654f20`, `0x647660`, `0x64c910`, `0x64db90`, `0x698130`, `0x6d8be0`, `0x438ef5`, `0x43940e`, `0x40da80`, `0x64ffc0`, `0x656770`, `0x5e3490`, `0x63a280`, `0x65f680`, `0x72fdb0`, `0x65bf80`, `0x668937`, `0x655ea0`, `0x6564c0`, `0x447dd0`, `0x639bd0`, `0x747810`, `0x63ce30`, `0x76f96a`, `0x5f2be0`, `0x63d9f0`, `0x5644a0`, `0x63a480`, `0x5ca660`, `0x542130`, `0x44ed00`, `0x5e57f0`, `0x5cb8d0`, `0x5f5ee0`, `0x5e7ae0`, `0x5e0a40`, `0x5ebae0`, `0x5b60a0`, `0x5f4a10`, `0x5f2f70`, `0x5f1e40`, `0x5f22c0`, `0x5f1cb0`, `0x5a70a0`, `0x5f19b0`, `0x5f1930`, `0x5f2250`, `0x5f22b0`, `0x5ac930`, `0x5f37e0`, `0x5f19d0`, `0x5b3c10`, `0x5ada30`, `0x5adb50`, `0x5f19f0`, `0x5f1ae0`, `0x5acfc0`, `0x5ae3c0`, `0x5b09c0`, `0x5acfa0`, `0x5abf40`, `0x5f1a10`, `0x5c3eb0`, `0x5ad380`, `0x5aea80`, `0x5b0e80`, `0x5b4670`, or `0x65a2e0`. Do not restart the slot-25 search.
