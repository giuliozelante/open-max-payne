# Particle emitters: graph layout and the intro's effects (2026-11-03)

Goal: draw the intro's snow and the police-car lights. This note fixes what each sampled graph in an `.rps`
emitter means. The light block is already in `2026-11-03-particle-lights.md`.

## Effects the intro starts (VERIFIED, `Startup_level.ldb` FSM strings)

| Emitter (DO) | Effect |
|---|---|
| `::start::c1_y::c1_rot::C1_PAN::f1` | `snowstorm_60m_loop`, `snowstorm_20m_loop` |
| `::startroom::cam_mover::camera1::snow`, `…::altitude::camera::final_snow`, `…::choopper_move::f1`, `…::police_car_dynamic::f_snow`, `::startroom::d1::cam_new_01::f1` | `snowstorm_60m_loop` |
| `::startroom::d1::cam_new_01::f1` | `blizzard_10m_loop` |
| `…::police_car{1,2,3}::siren`, `…::police_car_dynamic{,2}::siren` | `light_policecar` |
| `…::police_chopper::smoke` (both rooms) | `chopper_smoke` |

`particles.txt` `[snowstorm_60m_loop]` ("Snowstorm 60 meter cube"): `Looping = true`,
`InitialUpdateLength = 4.0`, `Sorting = false`. The file is `particles\snow\snowstorm_60m.rps` (133 038 bytes).

## Graph names and members (VERIFIED, static, `ParticleFX.exe`)

Each graph is created by the factory at `0x431a20` with its name string and stored in a member. The writers
emit the members in this order.

**Emission (`0x21`/5 inner object, writer `0x415d00`), constructor `0x414f..`:**

| Member | Name string | Address |
|---|---|---|
| `+0x44` | `Emission Rate` | `0x46b330` |
| `+0x48` | `Velocity m/s` | `0x46b320` |
| `+0x4c` | `Lifetime t` | `0x46b314` |
| `+0x50` | `Red=X, Green=Y, Blue=Z : Emission Direction` | `0x46b2e8` |

After the graphs the writer emits vec3 `+0x54`, `+0x60`, `+0x6c`, then the scalars `+0x78` … `+0xc0`. The
earlier light note counts these as "3 vec3, 4 floats, 3 ints, 1 vec3, 5 floats, 2 vec3". The second float is
the cycle length.

**Particle (emitter object `0x30`/4, writer `0x417570`), constructor `0x4168..`:**

| Member | Name string | Address |
|---|---|---|
| `+0x70` | `Size m` | `0x46b388` |
| `+0x74` | `Rotation °` | `0x46b37c` |
| `+0x78` | `Vertex Alpha` | `0x46b36c` |
| `+0x7c` | `Rgb color c` | `0x46b360` |

The writer first emits the shared parameter block (`0x20`/3, `0x414d20`) and the members `+0x54`..`+0x60`,
then these four graphs, then the texture (`0x46112e` path and embedded image), then `+0x88`, `+0x8c`,
`+0x90`, `+0xb8`, `+0xbc`.

**Companion (`0x40`/2, constructor `0x4185..`):** `Size *` (`0x46b41c`) and `Rotation *` (`0x46b410`), the
two graphs at `+0xcc` / `+0xd0`, then Use Light, `Light Color` and `Light Falloff`. HYPOTHESIS: these are
multipliers over the emission cycle, as the `*` suffix suggests.

## `snowstorm_60m.rps` read against that layout (STRONGLY SUPPORTED, `openpfx dump-rps`)

```
[0..8]   root 0x60/5: uint 500, f32 2.5, uint 10, f32 2.5, uint 10, int 0, int 0
[9..10]  0x31/1 graph section
[11..17] 0x20/3 parameter block: f32 1, int 0, int 1, f32 5, int 2
[18..19] 0x30/4 emitter
[20..32] 0x20/3 parameter block: f32 0.8, int 2, int 0, f32 0, int 0, f32 20, int 1, int 1,
         f32 -180, f32 180, uint 7
[33..52] graph 0x71/3, 1 channel x 16 samples, ~1.0 (Size m)
[53..72] graph 0x71/3, 1 x 16, -200..139 (Rotation °: random spins)
[73..]   graph 0x71/3, 1 x 2 ... (Vertex Alpha), then Rgb
```

Graphs are sampled over the particle's life (16 samples here). The root's `uint 500` is probably the
particle maximum, since the editor shows "Particle Maximum" and "Particle Count".

## Still UNKNOWN (next)

- The labels of the scalar fields in the two `0x20`/3 parameter blocks, the emission vec3s and floats
  (emission volume or box size, gravity, wind, blending mode, sorting). Next step: read the
  `CParticleEmitterFormView` / `CParticleFormView` dialog control bindings (DDX) in `ParticleFX.exe`.
- How the game's reader (`PS_ParticleSystem` in the runtime) interprets "60m cube": the box size field.
- Additive or alpha blending per particle texture (`PSCombination%02dAlpha.tga` hints at alpha variants).
