# ParticleFX RPS / PSE — Working Spec (Phase 4)

**Status:** Draft. PSE dump in Rust `crates/openpfx`. Full serialize/export still REA-backed WIP.

**REA artifact:** `vendor/max-fx-tools/ParticleFX_files/ParticleFX.exe` (sha256 `3ce05189…69c7`).

## Files

| Ext | Role |
|-----|------|
| `.pse` | ParticleFX **editor** project (paths, graphs, emitter params) |
| `.rps` | **Runtime** particle set cooked for the game |

## CLI (from ParticleFX.exe strings)

```
-e    Export files (.pse -> .rps)
```

MFC doc filter: `ParticleFX Files (*.pse)`. Runtime filter: `Runtime Particle Sets (*.rps)`.
Export UI: `Export data to run particle system in other programs` (Ctrl+E).

RPS load errors (version gate):

```
PS_ParticleSystem: Invalid File Version in %s
Current: %8x  Expected: %8x
PS_ParticleSystem: Invalid File Structure in %s
```

## PSE example (editor project `.pse`)

Observed on `Bullet_Impact_default.pse` (local vendor fixture; not in git):

| Off | Value | Notes |
|-----|-------|-------|
| 0 | `0x60` (96) | struct / block size guess |
| 4 | `5` | version / count guess |
| 8 | `0x23` (35) | varies by project |
| 12 | float `2.5` | |
| 16 | `0x0A` (10) | u32 — **not** a float |
| 20 | float `2.5` | |
| later | ASCII | `C:\Program Files\MAX-FX Tools\...` paths, bitmap leaf names |

`openpfx dump-pse` prints these fields (synthetic unit test; no retail bytes in CI).

### RPS example (runtime `database/particles/.../*.rps`)

Dense binary. No plain ASCII paths; bitmaps by cooked ids. Version checked as `%8x` against expected.

## Next

1. REA: locate `-e` export path → `PS_ParticleSystem` write.
2. `openpfx export` once RPS version magic is known from REA.
3. Retire Python `tools/openpfx` once Rust CLI covers dump + export.
