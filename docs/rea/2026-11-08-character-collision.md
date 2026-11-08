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

## Next

1. Resolve the virtual calls inside `0x63b410`, `0x5bf8e0` and `0x646f30` (for example `[ebx+0x4c]` in `0x646f30`).
   Identify each receiver's class from how the object is built, then read its vtable, as done above.
2. Alternatively, trace it at run time: run retail under Wine with a debugger (`winedbg`) and break on `0x647660`
   while Max walks into a wall. That gives the real call chain in one session.
3. Find the character update's call into `0x647660` (one of the five callers) to see the per-frame step and how a
   step-up or a slope is handled.
