# Particle emitter point lights (`.rps`, ParticleFX.exe)

The ParticleFX tutorial says an emitter can carry one Gouraud point light ("Use Light"). The light lights the
level, the characters and the particles that have lighting on. It is attached to the emitter and moves with it.
Its graphs are "Light Falloff" (radius) and "Light Color" (RGB), both over the emission cycle. There is also a
"Brightness" value, which "has no effect in the current version", and a "Light Position Offset". The editor
adds " (L)" to the name of every emitter that has the light on. These are STRONGLY SUPPORTED claims, since
they come from the tutorial.

## Writer (ParticleFX.exe, VERIFIED static)

Each emitter is written by `0x419e06 → 0x417570` (object `0x30`/4), followed by
`0x419e11 → 0x418fe0`, the companion object `0x40`/2:

| Order | Field | Writer |
|-------|-------|--------|
| 1 | inner object `0x21`/5 | `0x415d00` |
| 2 | graph `+0xcc` | `0x432fd0` |
| 3 | graph `+0xd0` | `0x432fd0` |
| 4 | int `+0xd4`: **Use Light** | `operator<<(int)` |
| 5 | graph `+0xd8`: **Light Color**, 3 channels | `0x432fd0` |
| 6 | graph `+0xdc`: **Light Falloff**, 1 channel | `0x432fd0` |
| 7 | float `+0xe0`: Brightness | import `0x468ac0` |
| 8 | vec3 `+0xe4`: Light Position Offset | import `0x468abc` |

How each field was identified:
- **`+0xd4`:** the emitter-list builder at `0x4032dc` tests it and appends `" (L)"` (`0x47a36c`).
- **`+0xd8`:** the constructor `0x4187a5` builds it as a 3-channel graph named `"Light Color"` (`0x46b404`).
- **`+0xdc`:** the constructor `0x418851` names it `"Light Falloff"` (`0x46b3f4`).

A sampled graph on the wire is `int 0x71, int 3, int channels, int samples`, followed by
`channels × samples` floats. The floats are stored one sample at a time: the
`light_fire_10m.rps` colour graph reads `159, 40, 4, 159, 40, 4`.

## Inner object `0x21`/5 as stored (STRONGLY SUPPORTED, BYOD layout)

The inner object holds, in this order:
- 4 graphs
- 3 vec3
- 4 floats
- 3 ints
- 1 vec3
- 5 floats
- 2 vec3

The second of the four floats is the **cycle length** in seconds:

| File | Cycle length (s) |
|------|------------------|
| `light_red_20m_1s` | 1 |
| `light_red_30m_1s` | 1 |
| `light_stroboflash` | 0.1 |
| `weapons\beretta` | 0.05 |
| `light_policecar` | 5 |
| plain colour lights | 10 |

The falloff values match the file names: `20m` → 19.9, `30m` → 29.6, `10m` → 10.03.

`openpfx::light::rps_lights` walks that layout. Over all 282 retail `.rps` files it found 753 companion objects.
Every one parsed, and 54 of them have the light on. That set covers every `lights\*.rps` and every weapon
muzzle flash.

## Engine

- `PS_StartEffect(effect, …)` queues `(effect, emitter)` on the scene, using the same receiver resolution as
  `A_Play3DSound`: trigger, mesh, FSM, `this`.
- `PS_StopAllEmissions` stops that emitter's effects.
- `particles.txt` `[effect]` supplies the `.rps` path (`[Default] filename`, relative to
  `\data\database\particles\`) and `Looping`.
- **Colour and radius:** each frame the light takes the colour graph (0..255) and the falloff graph (radius),
  sampled linearly at `age / cycle`. A looping effect wraps at the cycle; a one-shot ends after it. Intensity
  is 1, because Brightness has no effect.
- **Position:** the light sits on the emitter plus the offset. An FSM emitter (the intro sirens) is carried by
  its parent DO, the same way a child DO is (`LdbScene::emitter_world_pos`).

The startup intro's `::startroom::police_car*::siren->ps_starteffect(light_policecar, "")` now gives a light that
flashes red and blue over a 5 s loop with a 4.78 m radius.

## Open

- **Characters only:** the lights reach characters only. Level geometry is lightmapped here; retail re-tessellates
  the level geometry under a dynamic light, and that is not modelled.
- **Particles:** no particles are drawn yet, only the lights.
- **Cycle randomisation:** the third inner float (`light_fire_10m` = 2) is probably cycle randomisation
  (HYPOTHESIS) and is ignored.
- **Unknown fields:** the meaning of the three inner ints is UNKNOWN. `light_policecar` has `0,1,1` where the
  others have `0,0,1`.
- **Muzzle flashes:** they need `PS_StartEffect` from the weapon code, which does not exist yet.
