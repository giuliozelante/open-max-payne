# Command-line switches

Retail 1.05 `MaxPayne.exe`, `objdump -d -M intel`. Issue #166.

**VERIFIED.** `0x40cb90` compares one switch (without its dash) against a list and sets a byte flag:

| Switch | String | Flag |
|---|---|---|
| `window` | `0x867c14` | `0x8a6480` |
| `nodialog` | `0x867c08` | clears `0x867afc` |
| `screenshot` | `0x867bfc` | `0x8a6481` |
| `profile` | `0x867bf4` | `0x8a6482` |
| `developer` | `0x867be8` | `0x8a6483` |
| `enableloadingstats` | `0x867bd4` | `0x8a6484` |
| `debugmem` | `0x867bc8` | `0x8a6485` |
| `disable3dpreloads` | `0x867bb4` | `0x8a6486` |
| `savecombinationmaps` | `0x867ba0` | `0x8a6487` |
| `skipstartup` | `0x867b94` | `0x8a6488` |
| `developerkeys` | `0x867b84` | `0x8a6489` |
| `showreference` | `0x867b74` | not read yet |

**VERIFIED.** With `screenshot` set (getter `0x40cd60`), key code `0x79` (F10) at `0x4218ea` saves a screenshot. `0x40e810`
creates `screenshots` when missing (`0x867e60`, `0x867e54`) and hands `\screenshots\shot%04i.tga` (`0x867e38`) to
`P_Driver::saveScreenShot`. **HYPOTHESIS:** the driver picks the first free number; the engine does.

**STRONGLY SUPPORTED.** `skipstartup` (getter `0x40cdc0`) is tested at `0x416786` and `0x42a5dc` during startup. The engine
reads it as PCGamingWiki describes it: skip the startup video.

The engine takes the switches with one dash, as retail's shortcuts pass them. `window`, `nodialog`, `disable3dpreloads` and
`novidmemcheck` change nothing: the engine always opens a window and has no launcher dialog, Direct3D preloads or video
memory check. `novidmemcheck` is not in this list; it belongs to a later patch or the FixPack.
