# ParticleFX RPS / PSE — Working Spec (Phase 4)

**Status:** Draft from fixtures + ParticleFX.exe still to be fully RE'd.

## Files

| Ext | Role |
|-----|------|
| `.pse` | ParticleFX **editor** project (paths, graphs, emitter params) |
| `.rps` | **Runtime** particle set cooked for the game |

### PSE example (editor project `.pse`)

Header words (LE):

| Off | Value | Guess |
|-----|-------|-------|
| 0 | `0x60` | Header/payload size or struct size |
| 4 | `5` | Version / emitter count |
| 8 | `100` (`0x64`) | |
| 12 | `float 2.5` (`0x40200000`) | |
| … | | |
| later | ASCII path | `C:\Program Files\...` bitmap / tool paths |

Contains readable tokens: `PARTICLE`, `BITMAP`, etc. (editor-side).

### RPS example (runtime `database/particles/.../*.rps`)

Dense binary (~tens of KiB). Starts with small tagged ints/floats (`0x14` markers similar to MemoryFile tags). No plain ASCII paths; bitmaps referenced by cooked ids.

## Next

1. REA on `ParticleFX.exe` serialize/load path.
2. `tools/openpfx` — dump PSE params; compile PSE→RPS once format locked.
