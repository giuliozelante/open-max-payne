# Project status

Where the project stands. Goals and order of work are in
[the product spec](specs/2026-10-30-open-max-payne.md); evidence is in `docs/rea/`.

## How work runs

The owner plays the engine and compares it with retail. `capture engineN` records a run into
`~/Pictures/omp-compare/engineN`, and retail recordings sit beside it. Each finding becomes a small PR into `develop`.

Retail reference: the owner's install runs at 60 fps through DXVK, with the Widescreen Fix's loader parked, so its
gameplay is unmodified. The game is Italian, so it plays from `x_*.ras`; the FixPack's `data.disabled-hires` is not used.

## Engine: known gaps

- Enemies do not shoot or move on `.ai` paths. Phase 2 of the [health and AI spec](specs/2026-11-06-health-and-enemy-ai.md) starts with decoding the state machines.
- `[Target]` hit zones are not applied, and enemy capsule values are still constants.
- Sniper scope and bullet camera are missing.
- The sky shows hard black shapes; level textures look softer than retail.
- The chase camera clips into walls and into Max.
- Max's death restarts the level, but has not been played; its camera is missing.
- Jumps and dives may need Max's running speed carried in; a retail measurement is pending.

## Modding: known gaps

- The Blender add-on is a stub, and the engine crate only reads KF2/SKD/KFS.
- `openpfx` export omits graphs and emitters.
- No `.ldb` writer or level editor.
