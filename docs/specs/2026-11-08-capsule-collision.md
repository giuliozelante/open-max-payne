# Character collision as a capsule

**Status.** Proposed 2026-11-08. Needs the owner's approval before any code.

**Problem.** Max walks into walls and through solid props. In engine26 the owner went into the wall leaving the
Tutorial's secret room and sank under the level, and clipped into the big trash containers at the end of the first
alley. #201 stops a body caught in a wall from going deeper, but the cause is the collision model itself:

- Walls are tested as a bounding box plus the distance to the polygon's infinite plane. A point beside a narrow
  polygon, or past its edge, is judged by a plane it is not near.
- Floors and walls are separate passes with fixed thresholds (`normal.y` 0.45), so a sloped prop face is either
  ignored or treated as floor.
- The body is a vertical slab from 0.35 m above the feet to 0.10 m under the eye, with a hardcoded radius of 0.38 m.
  Retail's data says 0.48.
- A step is accepted or refused whole. Nothing slides a body along a wall, so it either stops dead or a rule like
  the railing escape lets it through.

## What retail gives

| Value | Source | Retail value |
|---|---|---|
| Max's capsule | `skins/max_payne.txt` | `CapsuleRadius 0.48`, `CapsuleBottom 0.0`, `CapsuleTop 1.95` |
| Capsule limits | `globalconfig.txt` | radius ≤ 0.50, bottom ≥ 0.0, top ≤ 2.00 |
| Enemy capsule | `globalconfig.txt` | `AICapsuleRadius 0.31` |
| Which polygons block | `materials.txt` | `CharacterCollides` per material (already read) |
| Dynamic objects | `globalconfig.txt` | `MinSpeedCharacterCollidesDynamicObject 0.01` |

**VERIFIED (strings and xrefs):** the exe validates start points with `MaxPayne_StartPointTestCapsule`. Before the
error "Startpoint above floor translation collided" (`0x4528e3`) it calls `0x647610` on a local test object
(`[ebp-0x24c]`). That routine subtracts the object's `+0x54` from the start point and calls `0x647660`. The code then
compares a field of the result (`[ebp-0xdc]`) with the owner returned by `0x748300` (a `+0x8` getter). It prints the
error, with a name read through the `+0x148` getter `0x5e3b20`, when they differ. Other test classes are
`CharacterTestSphere`, `DodgeTestSphere` and `TestSphere`.

**UNKNOWN:** what `0x647660` sweeps, and the per-frame character mover. The next step is to read `0x647660` and find
its callers in the character update. An earlier draft called `0x5e3b20` the sweep; it is a getter.

## Design

1. **REA first.** Read `0x5e3b20` and the character update that calls it. Record in `docs/rea/` how the capsule is
   swept, how it slides, how it rides steps and slopes, and how a dodge's test sphere is used. The steps below are the
   expected shape and change with what the exe shows.
2. **Capsule from data.** Use `scene.player_capsule` (already loaded from the skin) for radius, bottom and top. Delete
   the 0.38 m constant and the slab heights.
3. **Real distance.** Test the capsule's segment against each blocking triangle by closest-point distance, for static
   and dynamic meshes alike, with a per-room or grid prefilter so the cost stays flat.
4. **Sweep and slide.** Move in substeps no longer than the radius. On contact, remove the motion into the contact
   normal and continue along the surface, as retail's "translation collided" checks suggest.
5. **Floor from the same test.** The capsule's bottom rests on the highest blocking surface under it within a step
   height. That height comes from the exe or the data, not from the current `STEP_UP` guess.
6. **Remove the escape hatches** that exist only because of the old model: the railing rule in `can_step`, #201's
   no-deeper rule, and the special name filters in `dynamic_wall_hit` (`_Tip_`, `Start_FSM.DO`). Keep each one whose
   reason the exe confirms.

## Verification

- Table tests with synthetic geometry: a thin wall, a narrow pillar, a box prop, a slope, a step, and a corner.
  Fast moves (dives) must not tunnel through any of them.
- Retail-gated headless walks in the Tutorial: the secret room's window wall and the first alley's trash containers
  must stop Max.
- A capture of the Tutorial played through, with no clipping.

## Risks

- The mover is spread across virtual calls; the decode may take longer than the rewrite.
- A full test against every triangle each frame is too slow on large levels without the prefilter.
- Scripted moves (`C_Teleport`, jump points) and enemies share the code, so changes there need their own checks.

## Decision for the owner

Approve the REA-then-rewrite above, or ask for a smaller first step. The smallest useful step is 2 plus 3: the data
capsule and real triangle distance, without sliding.
