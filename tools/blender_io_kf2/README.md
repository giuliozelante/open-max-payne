# blender_io_kf2 — early architecture

**Status:** Specification / early architecture (not a finished Blender add-on).

Blender 4.x import/export for Max Payne 1 `.kf2` / `.skd` / `.kfs` will call a future
native `maxfx-core` / CLI parser. Clean-room only — no proprietary 3ds Max SDK linkage.

## Spec

See [`docs/specs/kf2_format.md`](../../docs/specs/kf2_format.md).

## Coordinates (planned)

| Space | Up | Scale |
|-------|----|-------|
| Blender | Z-up | 1.0 m |
| MAX-FX / Max Payne | Y-up (effective) | ~0.01 Max units |

## Stub

`__init__.py` registers a placeholder panel. Do not expect mesh import yet.
