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
- **VERIFIED:** the two bases share most slots. They differ in slot 0 (destructor), 8, 12, 17, 19, 20, 21 and 22
  (0-based). Slot 19 is `0x63eee0` for the capsule and `0x658dd0` for the sphere; they call `0x63b410` and `0x63b1a0`.
  Slot 21 calls `0x5c0ba0` and `0x5c05f0`; slot 22 calls `0x5bf8e0` (818 instructions) and `0x5bf2f0` (357).
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
- **VERIFIED:** `0x64fda0`, called from `0x645340` at `0x6454f6`, indexes a container with `imul 0x51eb851f` / `sar 5`
  (divide by 100). The element is 100 bytes. What it holds is not read.
- **UNKNOWN:** the virtual callee that tests the capsule against the level, and where a step-up or a slope changes
  the target. `0x696f60` has no float immediate of its own between `0x697210` and `0x697619`.
- **VERIFIED (2026-11-21):** slot 25 (`+0x64`) is `0x64b4f0` in the capsule base (`0x7d4d64`), the sphere base
  (`0x7d5008`) and `MaxPayne_StartPointTestCapsule` (`0x7cd010`). `X_Character` (`0x7d5950`) overrides it with
  `0x6954f0` and shares slots 23, 24 and 26 with the capsule base, so it derives from that base. It also overrides
  slot 27 (`0x696e90`, base `0x755fa0`).
- **VERIFIED:** `MaxPayne.exe` has 56 `call [reg+0x64]` sites, spread over about 45 functions. Most receivers belong to
  other classes, so the list alone does not name the caller; the sites near the shape code are `0x6464d4`,
  `0x6382c1` and `0x6982a9` (function starts, approximate).
- **VERIFIED:** `0x6954f0` does no float work (no `fld`/`fmul`/`fcom` in its 242 instructions). It walks a list at
  `[edi+0x8]`, skips entries whose byte `+0x125` is clear, and compares each entry's class (`call [eax+0x48]`)
  with two RTTI types through `type_info::name`: `X_Character` (`0x8684c8`) and `X_LevelRuntimeDynamicObject`
  (`0x8683c0`). A character goes to `0x695490`, a dynamic object to `0x695390`; both after `0x65a1c0`.
  `0x695390` reads a vector at `+0x7f4`/`+0x7f8`/`+0x7fc` of its object.
- **VERIFIED:** the base `0x64b4f0` (78 instructions, no float work) fills a static list at `0x8b8100` through
  `0x5cb890`, `0x5cac20` and `0x5c84c0`.
- **VERIFIED:** `0x646430` calls slot 25 on itself (`0x646557`) with the static list `0x8b3b88`, then `0x64b660`.
  It runs only when byte `+0x124` is set; byte `+0x88` picks the other branch, which calls slot 24 (`+0x60`)
  instead (`0x6464b7`). Its one caller is `0x646884`.
- **STRONGLY SUPPORTED (ruled out):** slot 25 is not the mover. It gathers the objects a shape touches and, for a
  character, reacts to other characters and dynamic objects. The step that tests the capsule against level
  triangles is elsewhere; `0x646884`'s function and slot 24 are the next places to read.
- **VERIFIED:** slot 24 in the capsule base (`0x63e430`, `ret 0xc`) takes the vector from `0x63e070`, turns it by
  the shape's 3x3 from `0x74e8e0` (`0x63e53d`-`0x63e586`), and passes it with the shape's `+0x12c` and the result of
  `0x648a90` to `0x5bd240` on the object at `[arg1+0x188]`. `0x5bd240` (56 instructions) calls `0x5bba80`,
  `0x5c43e0`, `0x4459d0`, `0x449460`, `0x5b4b00`, `0x5bd0a0` and `0x5c4450`. **HYPOTHESIS:** the `0x5b`-`0x5c`
  range, which also holds slot 22's `0x5bf8e0`, is the collision library, and `0x5bd240` hands it a shape to sweep.
- **VERIFIED:** `0x5bd240` sits in the level-database code (`0x5b`-`0x5d` holds the `X_LEVELDB_*_LOAD` and
  "Level Database Importer" strings). It adds `0.02` (`0x7d20c8`) to a squared length (`0x449460`) and an argument,
  and `0x5bd0a0` adds `0.02` again (`0x7d20cc`). **HYPOTHESIS:** a padded bound for a level query, not the sweep.
- **VERIFIED:** `0x63f0a0` (`ret 0x18`) tests one runtime shape against another. It runs only when the other
  shape's `+0x125` and this shape's `+0x124` are set (or the reverse when argument 5 is non-zero), the same bytes
  slot 25 checks. The character reaches it at `0x695022`.
- **VERIFIED:** `0x649a10` (about 920 instructions, one `fsqrt`, constants `0`, `1` and `4`) is shared by
  `0x63f0a0`, `0x64ab30`, `0x64abf0`, `0x64ad70`, the AI (`0x4f0b70`) and `0x6d8b00`. `0x64abf0` and `0x64ad70`
  are called from `0x694ab0`, which runs from game-mode setup (`0x451e70`, string "Character \"%s\" capsule too
  small for camera sphere", and `0x452410` from the `MaxPayne_GameMode` constructor `0x44f190`).
  **STRONGLY SUPPORTED:** `0x649a10` is the shared shape-versus-shape distance core; whether level triangles go
  through it is not read.
- **VERIFIED:** `0x646430`'s caller starts at `0x6467f7` (234 instructions, no float work). It reads the owner
  through `0x748300` and position through `0x642d40`/`0x642d50`, and also calls `0x5c4450`.

## The per-frame character physics update `0x68c960`

- **VERIFIED:** the debug overlay `0x422bc0` prints the player's `vel`, `Grav.vel` and `Anim.vel` (strings `0x869be4`
  and neighbours) through `0x421ec0`, `0x421ed0` and `0x421ee0`. They forward to `X_Character` getters: velocity is
  the vector at `+0x41e` (`0x694540`), gravity velocity comes from the member at `+0x404` (`0x694550`, then
  `0x742840`), animation velocity is at `+0x42a` (`0x694560`).
- **VERIFIED:** apart from that getter and one store in `0x682ab0`, the velocity fields are written only by
  `0x68c960` (about 1 750 instructions, `ret 0x8`, 11 uses of `+0x404`). Its one caller is `0x687f20`, called from
  `0x44c050` in the game mode. **STRONGLY SUPPORTED:** `0x68c960` is the character's per-frame physics step.
- **VERIFIED:** `0x68c960` moves the character through two routines, not through `0x696f60`:
  `0x6480a0(up, [0x891b7c], &target, &hit)` at `0x68d20c`, then, when that finds ground and the character is not
  airborne (`[edi+0x228] <= 0`), `0x647df0(up, 0.05, &target, [0x891b80], [0x891b7c], 0x8b4c10)` at `0x68d277`.
  `up` is the vector at `+0x50` of the object at `[this+0x60]` (`0x642d60`, `0x74fe10`).
- **VERIFIED:** `0x891b80` and `0x891b7c` are set once, by `0x695880` and `0x695870`, from arguments 9 and 10 of the
  `MaxPayne_GameMode` constructor (`0x44f436`, `0x44f43f`).

### Ground test `0x648040`

- **VERIFIED:** `0x648040` returns true when `dot(up, n) > cos(angle * 0.01745329)` (`0x7cfb04`, `fcos`) and
  `dot(move, n) <= FLT_MIN` (`0x7cefb8`): the contact normal `n` faces up within `angle`, and the move does not lift
  off it. `0x6480a0` asks the contact list from `0x646710` (records of `0x28` bytes: normal at `+0x4`, object at
  `+0x24`) for the first contact that passes, and returns that object as the ground. With no such contact it combines
  the normals (`0x6485e0`) and gives up when `|dot(n, -up)| < 0.03` (`0x7d4ed0`, a double); the rest of that branch
  (`0x648226`-`0x6485c3`, through `0x64af10` and `0x64c910`) is read below.
- **VERIFIED:** that branch runs only when the touched normals are all near-horizontal (walls). It saves the shape's
  transform (`0x74e8e0`, 3x4) and bytes `+0xb0`/`+0x88`, moves the shape `0.05` m (`0x3d4ccccd`) along its own up
  axis (row `+0xc` of the 3x3) with `0x64af10` (flag `0`, list `0x8b3ae0`), then tests a line `0.1` m straight down
  from there (`0.05 * 2.0`, `0x40000000`) with `0x64c910`. A hit keeps the best-facing contact as the ground; no hit
  clears it. `0x642ed0` then restores the saved transform. **STRONGLY SUPPORTED:** this is a probe for a body wedged
  between walls with its feet over ground, not the wall slide. The slide is still **UNKNOWN**; it is not in
  `0x6480a0`.

### Drag-down `0x647df0`

- **VERIFIED:** `0x647df0` probes straight down from the target. The probe length is `|move| * tan(a4 * 0.01745329)`
  (`fptan` at `0x647edc`), at least `0.05` m (`0x7d5924`, the clamp at `0x647ef1`). The probe is turned by the
  shape's 3x3 (`0x74e8e0`) and tested with `0x647b90`. When it hits and `dot(n, up) > cos(a5)` (`0x647fea`), the
  shape is moved onto the hit with `0x64f100` (after `0x6437e0`), and the routine returns true.
- **STRONGLY SUPPORTED:** `a4` (`0x891b80`) is `MaximumDragDownAngle` and `a5` (`0x891b7c`) is
  `MaximumSteepnessAngle`. Retail `globalconfig.txt` sets them to `60` and `46` degrees; the parser stores them at
  `+0x10` and `+0xc` of its block (`0x50b22b`, `0x50b215`). The use matches the names: a tangent turns the drag-down
  angle into a distance, a cosine turns the steepness angle into a floor test. The path from the parsed block to the
  constructor arguments (`0x4171a1`, `0x44bc17`) is not traced.
- **Consequence for the engine:** a surface is ground when its normal is within 46 degrees of up (`n.y > 0.695`),
  and a walking character is pulled down onto ground up to `max(0.05, |move| * tan 60)` below it. The engine's
  `0.45` normal threshold and its step-height snap are not these rules.

## Next

- **STRONGLY SUPPORTED (ruled out):** `0x645ca0`, the only other routine `0x64af10` alone calls, sorts what was hit
  by RTTI name (`type_info::name`, `0x7ca34c`) and formats messages (`sprintf`); not the slide. `0x645340` calls
  itself at `0x645a26` over the children from `+0x24` (`0x411d90`, `0x640dc0`): it carries a moved transform down a
  hierarchy; not the slide either.

1. Find the slide: read `0x64af10`'s use of the contact list after `0x645340`/`0x645ca0`, where a blocked move
   should lose its component along the contact normal. `0x649a10` (the shape distance core) comes after.
2. Resolve the virtual calls inside `0x63b410`, `0x5bf8e0` and `0x646f30` (for example `[ebx+0x4c]` in `0x646f30`).
   Identify each receiver's class from how the object is built, then read its vtable, as done above.
3. Alternatively, trace it at run time: run retail under Wine with a debugger (`winedbg`) and break on `0x647660`
   while Max walks into a wall. That gives the real call chain in one session.
