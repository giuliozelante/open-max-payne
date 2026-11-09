# AI state machines

First decode of the machines compiled into `MaxPayne.exe`, for phase 2 of
[the health and AI spec](../specs/2026-11-06-health-and-enemy-ai.md).

## Shape

**VERIFIED:** each builder (`0x4cf700`-`0x4ec150`) is a run of constructor calls with small integer arguments. A
symbolic pass over the disassembly recovers them with no gaps in the machines read so far.

| Call | Meaning |
|---|---|
| `0x4ed6a0(n)` | condition `n` |
| `0x4ee130(38, c)` | not `c` |
| `0x4ee690(a, 39, b)` | `a` and `b` |
| `0x4eec50(a, 40, b)` | `a` or `b` |
| `0x4ed940(n, x)` | condition `n` with a float argument |
| `0x4ec6f0(a)`, `0x4ec910(a, b)`, `0x4ecad0(a, b, c)` | a list of action codes |
| `0x4f8a70(when, do, until)` | a rule |
| `0x4f8ae0(when, do, until, ...)` | a rule that runs a sub-machine |
| `0x4f8e30`, `0x4f91e0`, `0x4f93d0`, `0x4f95d0`, `0x4f97e0`, `0x4f9a00` | a state from 1, 3, 4, 5 or more rules |

**VERIFIED (`0x4fa280`, `MaxPayne_AIStateMachine`, vtable `0x7cfcac`):** a state holds its rules in order, the first
having priority. Each update tests the running rule's `until`. If it holds, every rule may take over; if not, only
rules well above it may. The first rule whose `when` holds becomes current, with the old rule left and the new one
entered and its timer reset. Otherwise the running rule goes on. Rule timers advance by the frame time.

**STRONGLY SUPPORTED:** operators 38, 39 and 40 come from how they read in `StandAndShoot`, not from their code.

## Actions

**VERIFIED:** `0x4ecec0` runs a list through the switch at `0x4ecef5`, calling `MaxPayne_ArtificialPlayerInput`
(vtable `0x7cfcd4`). Enemies are driven through the same input interface as a player.

| Code | Effect | Code | Effect |
|---|---|---|---|
| 0 | nothing | 8 | backward and left |
| 1 | forward 1.0 | 9 | left |
| 2 | forward 0.5 | 10 | input `0x5ea` |
| 3 | backward | 11, 12 | turn, -1 and +1 |
| 4 | forward and right | 13, 14 | shoot, arguments (1, 0) and (1, 1) |
| 5 | backward and right | 16 | input `0x5e9` |
| 6 | right | 20-27 | inputs `0x5ef` to `0x5f6` |
| 7 | forward and left | 17, 28, 29, 30 | further flags, unnamed |

Shoot (`0x4ff5b0`) only sets the trigger after its own tests: the target in a cone (`0x4f2fc0`), seen or recently seen,
and not blocked (`0x4f5cb0`).

## Conditions

**VERIFIED:** `0x4ffc40` handles 1 to 23 and `0x4ffef0` handles 24 to 34.

| Code | Meaning |
|---|---|
| 1, 2 | true, false |
| 3 | enemy seen (`+0x3ad`, by the debug dump at `0x4f74f9`) |
| 4 | enemy perceived (`+0x3af`) |
| 7, 8 | target to one side, to the other (**HYPOTHESIS** from their pairing with turns 11 and 12) |
| 14 | in the enemy's room (`0x4f2200`) |
| 24, 25 | distance to the enemy below, at least `x` (`0x4f3320`) |
| 26 | random below `x` |
| 29, 30 | in a cone of `x`, and its opposite (`0x4f2fc0`) |
| 33, 34 | a counter at `+0x5fd` against `x` |

The others are unnamed.

## What the Tutorial uses

`StandAndShoot`, in priority order: with no perceived enemy, wait until one is perceived; perceived but not seen, action 28
until seen; target to one side, turn and shoot; to the other, turn and shoot; otherwise shoot. `RunWalkAndShoot` adds distance bands: around 1.2 m it
walks and shoots, closer it backs away, and past about 6 m it runs. `MobsterCombat` picks sub-machines by distance
with limits of 1, 4, 9.6 and 100 m, through `0x4f8ae0`.

## Open

- Conditions 5, 6, 9 to 13, 15 to 23, 27, 28, 31, 32.
- Enemy perception: the update of `+0x3ad` and `+0x3af` from the skin's `[AI]` keys (`VisualPerceivingRadius`,
  `GeneralPerceivingRadius`, `ShootingCone`, `EnemyInterestTime`).

## Perception (first read, 2026-11-08)

The conditions read three flags of the AI object: `0x4f6260` returns `+0x3ad`, `0x4f6280` returns `+0x3ae`,
`0x4f6290` returns `+0x3af`. Condition 3 ("seen") is `+0x3ad` and condition 4 ("perceived") is `+0x3af`.

- **VERIFIED:** `0x4f7440` clears `+0x3ac`, `+0x3ad`, `+0x3ae`, `+0x3af` and the float `+0x3b0`. It has three callers
  (`0x4fecb5`, `0x4ff10a`, `0x500534`). `0x4f1604`-`0x4f1693` zero the same fields and `+0x3b4` when the object is
  built.
- **VERIFIED:** inside the routine that starts near `0x4f6700`, two points are transformed by 3x4 matrices
  (`0x4f6b27`-`0x4f6bf6`), then `0x64c910` tests the segment between them. When the test reports nothing in the way,
  `+0x3ad` and `+0x3ac` become 1; otherwise `+0x3ac` becomes 0 and `+0x3ad` keeps its value. The two points are kept
  at `+0x3c0` and `+0x3cc`, and `+0x3b8` is set to 0.2.
- **STRONGLY SUPPORTED:** `0x64c910` is a line-of-sight ray test against the level, and the points are the AI's eye and
  its target. `+0x3b8 = 0.2` is the delay before the next test. Neither the eye bone nor the range and cone gates
  before the test (`0x4f6700`-`0x4f6b16`) have been read.
- **VERIFIED:** `0x4f6df0(stimulus, dt)` runs the "perceived" memory:
  - `+0x3b4 -= dt` on every call.
  - With a stimulus: if not yet perceived and `+0x3b4` has run out, it calls `0x6d73b0` on the object `0x694430(1)`
    returns from the AI's character (`+0x3dc`). Then it sets `+0x3af = 1` and `+0x3b0 =` the skin field at `+0xdd`
    (`0x531290`).
  - Without one, a perceived AI counts `+0x3b0` down by `dt`. At zero `+0x3af` clears and `0x4f7470(0)` zeroes
    `+0x3b4`. `0x4f7470(1)` instead loads `+0x3b4` from the same skin field.
- **VERIFIED:** when the `AI` section (`0x87d4f8`) has a count above zero (`0x5300a4`), `0x52fc80` copies the
  key globals into the skin record. `AimingSpeed` lands at `+0xb5`, `TurnLeftRightSpeed` at `+0xb9`,
  `AimUpDownSpeed` at `+0xbd`, `VisualPerceivingRadius` at `+0xc1`, `GeneralPerceivingRadius` at `+0xc5`,
  `ShootingCone` at `+0xc9`, `ActivationReactionTime` at `+0xcd`, `PerceivingGroupOneRadius` at `+0xd1`,
  `PerceivingGroupTwoRadius` at `+0xd5`, `PerceivingGroupThreeRadius` at `+0xd9`, `EnemyInterestTime` at `+0xdd`
  and `ActivateOtherCharactersRadius` at `+0xe1`. The dword at `+0xb1` is copied from `0x8ad728`, which has no
  key string in `.text`. The "perceived" memory lasts `EnemyInterestTime` seconds after the last stimulus.
- **HYPOTHESIS:** `0x6d73b0` is the AI's alert reaction, and `+0x3b4` is a cool-down before it can alert again.

- **VERIFIED:** `0x4f6df0` has three callers. `0x4f6c9a` passes the OR of `+0x3ad` and a local byte, and the float
  at the caller's argument `+0x4` (`0x5e53d0` is `fld [ecx+4]; ret`). `0x4feb3c` passes stimulus `1` and the value
  in `ebp`. `0x4fec9a` passes stimulus `1` and `0`, with `ecx` set to `this+0x157`.

- **VERIFIED:** `0x4f62a0` is the sight update. `0x4f18a0` writes a vector from the two characters at `+0x3d8` and `+0x3dc` when `0x642d40` returns the same value for both. The sum of squares of that vector is turned into a length by the `1.5` / `0.5` table iteration (`0x3fc00000`, `0x3f000000`) and stored at `[ebp-0x1c]`.
- **VERIFIED:** `0x6841c5` copies `VisualPerceivingRadius` (`skin+0xc1`, getter `0x531220`) to `+0x9e` through `0x6d72a0`, and `GeneralPerceivingRadius` (`skin+0xc5`, `0x531230`) to `+0xa2` through `0x6d72c0`. The sight test reads those with `0x6d7290` and `0x6d72b0`.
- **VERIFIED:** when `0x4f17b0` returns nonzero, a radius greater than the length sets the local byte at `[ebp-0x74]`. The radius is `PerceivingGroupOneRadius` (`+0xd1`) when the target's byte `+0x2` is set (`0x6d73e0`), else group two (`+0xd5`, byte `+0x3`), else group three (`+0xd9`, byte `+0x4` via `0x421800`), else `+0xa2` when `+0x3ae` is clear. `0x4f17b0` returns 1 when `0x642d40` matches for the two characters.
- **VERIFIED:** the line-of-sight block runs only after `+0x3b8` has counted down to 0, `0x4f17b0` was nonzero, the dot of the two vectors is greater than 0, and `+0x9e` is greater than the length. `ShootingCone` (`+0xc9`) is not loaded in `0x4f62a0`. The local byte at `0x4f6c80` is `[ebp-0x74]`, OR-ed with `+0x3ad`, and passed to `0x4f6df0` as the stimulus.
- **VERIFIED:** `0x684249` copies `ShootingCone` to `+0xaa` through `0x6d7320`. `0x4ff5e8` loads it, multiplies by `0.5` (`0x7cbdd4`) and by `π/180` (`0x7cfb04`), and passes that to `0x4f2fc0`. When the angle is not 0, `0x4f2fc0` takes `sin` and `cos` of its negation (`0x4f3037`).

Not read yet: the body of `0x4f18a0` when the two `0x642d40` results differ, and the compare inside `0x4f2fc0` after the sine and cosine.
