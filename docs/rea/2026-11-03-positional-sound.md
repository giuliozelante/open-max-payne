# Positional 3D sound (`sndmfc.dll` `S_SoundOmni`)

The sound catalogs (`data\database\sounds\*.txt`) give every `3DSound = YES` cue a `Hotspot` and a `FallOff`
in metres, plus a `Volume`. The values are numbers or `#define`s. Some come from the catalog itself
(`DYNAMIC_MEDIUMHOTSPOT 1`, `DYNAMIC_MEDIUMFALLOFF 20`); others come from
`#include <..\soundid.h>` (`ID_DOOR_HOTSPOT 2`, `ID_DOOR_FALLOFF 20`). The police sirens in the
startup intro use 2 / 50.

## Retail (`sndmfc.dll`, ImageBase `0x10000000`)

- **Setters, VERIFIED:** `S_SoundOmni::setHotspot` (`0x1000c350`) stores `+0x149` and
  `setFalloff` (`0x1000c3a0`) stores `+0x14d`. Each setter keeps falloff ≥ hotspot and sets the dirty
  byte `+0x13d`.
- **Range cut, VERIFIED:** the voice update at `0x10002f7b` computes the listener distance with a
  fast inverse square root (magic `0xbe7fffff`, then one Newton step with 1.5 / 0.5). At
  `0x10002fec` it compares that distance with the falloff `+0x14d`. Past the falloff it calls
  `0x1000bb60`, which stops the hardware buffer (`+0x1b9`). So a voice past its falloff is released.
- **Gain inside the falloff, UNKNOWN:** how the gain is computed there has not been read.

## Engine

The engine's gain model is a HYPOTHESIS: the DirectSound 3D inverse-distance law, with the hotspot as the
minimum distance:

| Listener distance | Gain |
|-------------------|------|
| ≤ hotspot | `Volume` |
| up to the falloff | `Volume · hotspot / distance` |
| past the falloff | 0 |

The listener is the previous frame's eye. An emitter's position comes from `LdbScene::emitter_world_pos`
(mesh, trigger or FSM, carried by its parent DO) plus its room origin.

`ffplay` sets its volume only at start (`-volume`, linear percent), so a voice cannot follow the listener
smoothly:

- **One-shot:** plays at the gain it has when it starts. If it starts past its falloff, it is dropped.
- **Loop out of range:** a loop such as a siren keeps its voice. Its player is killed when the listener
  leaves the falloff and started again on the way back.
- **Loop in range:** the player is restarted when the gain halves or doubles, which is a 6 dB step.
  Smooth attenuation needs an in-process mixer. That means a new dependency, which needs approval first.

## Catalogs

`A_Play3DSound(category, cue, …)` names its catalog file: `data\database\sounds\{category}.txt`. Besides
`dynamic` (3724 calls) and `breakable` (1599), the shipped levels use 25 more catalogs: `enemy` 306,
`bunker` 206, `ambient` 148, `asgard` 118, and down to `aesir` 1. In total that is about 1360 calls. The
exe holds a `*.txt` wildcard (`0x481ec0`), which suggests retail lists the directory. That is a
HYPOTHESIS; the reference was not followed.

The engine preloads `breakable` and `dynamic`. It loads any other catalog the first time a queued cue needs
it (`load_sound_catalog`, with `soundid.h` in front), and a catalog that is missing is read only once.

## Open

- Gain law and rolloff factor inside the falloff (read the `rl.dll` / DirectSound 3D setup).
- Doppler (`setVelocity`), `FrequencyScaling` and `Pitch`.
