# Sound channels and voice priority

Retail 1.05 `MaxPayne.exe` (ImageBase `0x400000`) and `sndmfc.dll` (ImageBase `0x10000000`),
`objdump -d -M intel`. Issue #167.

## Channel count

**VERIFIED.** `0x40c7f6` reads the launcher's `Sound Channels` value (`0x404190`, key string
`0x865f14`, under `Advanced Settings`) and turns it into a channel count: 0 → 8, 1 → 10,
2 → 12. Any other value leaves 0. The count goes to
`S_Listener::setProperties(channels, 44100, speakers = 3, device, hwnd)` (IAT `0x7cad6c`,
call at `0x40c8ef`). The owner's prefix stores 2, so retail mixes at most 12 sounds.

## Priority

**VERIFIED.** Each cue's `Priority` (sound catalogs, `#define`s in `soundid.h`, such as
`SOUND_PRIORITY_DEFAULT` 0, `ID_BULLETHIT_PRIORITY` 5, `ID_DOOR_PRIORITY` 20) is read into the
cue at `+0x27` (`0x534710`) and handed to `S_Sound::setPriority` (`0x588837`, `0x58c400`).

**VERIFIED.** `sndmfc` `0x1000a8f0` computes a sound's internal priority (`+0x193`):

- Start with the priority (`+0x18e`).
- Formula 1 or 3: subtract the playing time × the listener's time factor (`+0x181`).
- Formula 2 or 3: add a volume term × the volume factor (`+0x185`). For a 3D sound the term is
  `−distance` (squared distance, then a Newton-refined reciprocal square root at
  `0x1000a9ba`–`0x1000aa2b`). Otherwise it is `+0x124 − 1.0`.
- Clamp to ±`1.134e38`.

The listener constructor (`0x10001ee3`–`0x10001ef5`) sets both factors to 1.0 and the
formula to 2. The exe never imports `setPriorityFormula` or the factor setters, so retail
ranks a 3D sound by `priority − distance`. **HYPOTHESIS:** `+0x124` is the 2D volume, so a
full-volume 2D sound ranks at its priority.

## Taking a channel

**VERIFIED.** When a sound starts and no channel is free, `0x1000ab98`–`0x1000ac1b`
recomputes the internal priority of every playing sound whose override flag (`+0x192`) is
clear, sorts them ascending with `qsort` (comparator `0x1000ad10`), stops the first
(`0x1000bb60`) and gives its channel to the new sound. **UNKNOWN:** whether the new sound
takes part in that sort. The code found does not compare the two.

## Engine

`render3d::SoundChannels` keeps at most `limit` voices playing. The limit comes from
`user.reg` (`sound_channels_from_registry`), or 12 when there is no prefix. That fallback is
a HYPOTHESIS. A new cue with every channel busy stops the lowest-ranked voice
(`voice_rank` = priority − distance). A loop coming back into range takes a channel only
from a voice it outranks. Without that check, two loops would swap one channel every frame.
That check is an engine choice, not retail evidence.
