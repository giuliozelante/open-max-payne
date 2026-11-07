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
