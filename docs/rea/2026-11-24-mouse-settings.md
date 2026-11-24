# Mouse sensitivity, inversion and smoothing

Retail 1.05 `MaxPayne.exe`, `objdump -d -M intel`. Addresses are VAs (ImageBase `0x400000`).

## Registry values

**VERIFIED.** The `Input Settings` loader (`0x4cb58d`-`0x4cb669`) reads four mouse values after the bindings:

| Value | Default | Stored at | Rule |
|---|---|---|---|
| `Invert Mouse` | 0 | `0x8a968c` (byte) | non-zero means on |
| `Invert Mouse Wheel` | 0 | `0x8a968d` (byte) | non-zero means on |
| `Mouse Sensitivity X` | `0x1388` | `0x8a9690` (float) | `dword * 0.0001` (`0x7cbdc4`); below 0 or above 1 becomes 0.5 |
| `Mouse Sensitivity Y` | `0x1388` | `0x8a9694` (float) | same |

`0x4cb30b`-`0x4cb323` write the same defaults before reading (0, 0, 0.5, 0.5). The values go to the mouse object from `0x516d80`
through four setters: `0x519960` (+0x51 invert), `0x576430` (+0x52 wheel invert), `0x519850` (+0x53 X) and
`0x742870` (+0x57 Y).

## Per-frame use

**VERIFIED.** `0x51916a`-`0x5191ec` turns raw counts into the accumulated mouse position:

1. Each axis's counts are multiplied by `0x519480(sensitivity)` = `sensitivity * 1.4 + 0.1` (`0x7d045c`, `0x7cb8b4`).
   The default 0.5 gives 0.8; the range is 0.1 to 1.5.
2. Both are multiplied by the float at +0x87 (1.0 from `0x518c2c`).
3. `0x519370` smooths them. A static carry per axis (`0x8ac6b0`, `0x8ac6b4`) takes the input; the output is half the carry (`0x7cbdd4` = 0.5)
   when that axis moved this frame, all of it when it did not, and the output leaves the carry.
4. The result is added to the position at +0x5b / +0x5f.

`0x5195c0` takes the frame's position delta, negates Y when +0x51 is set (`0x5195f6`), and scales it by 0.0025 (`0x3b23d70a`)
and by 1 / the float at +0x4d. `0x5198f5` negates the wheel delta when +0x52 is set.

**UNKNOWN.** What turns that delta into radians of yaw and pitch. The engine keeps its own 0.003 rad per count at the
default gain and scales it by `gain / 0.8`, so only the ratio between settings is retail's.

**HYPOTHESIS.** The smoothing runs once per retail frame, so its lag shrinks at higher frame rates.
The engine runs it once per frame too; `OPENMAXFX_MOUSE_SMOOTHING=0` turns it off (issue #164).
