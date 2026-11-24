# Adaptive difficulty

Retail 1.05 `MaxPayne.exe`, `objdump -d -M intel`. Addresses are VAs (ImageBase `0x400000`). Issue #165.

## The difficulty object

**VERIFIED.** The object `0x4213c0` returns holds one table per mode. `+0xf9` is the mode, 1 to 4, named by `0x6761e0`
and matched by `0x676130`: `normal` (`0x86a814`), `nightmare` (`0x86a81c`), `hell` (`0x86a828`), `timedmode` (`0x86a830`).
`0x6760a0` returns that mode's table at `+0x5`, `+0x41`, `+0x7d` or `+0xb9`; `0x676570` fills each from `difficulty.txt`.

A mode table has rows of 32 bytes at `+0x20`, a row count at `+0x18` and a starting row at `+0x1c`. `+0xf5` is the
current row, and `0x6760e0` returns its address: `table[+0x20] + row * 32`.

| Call | Effect |
|---|---|
| `0x676100(mode)` | sets the mode and resets the row to the table's `+0x1c` |
| `0x676230` | row + 1, at most count - 1 |
| `0x676260` | row - 1, at least 0 |
| `0x676280` / `0x676290` | set / get the row |

Row getters read floats at `+0x0` (`0x676370`), `+0x4`, `+0x8`, `+0xc`, `+0x10`, `+0x14`, `+0x18` and `+0x1c`. `0x691c0c` multiplies
`+0x0` into the player's maximum health, so it is `PlayerHealthMultiplier`. The other fields are not mapped yet.

**STRONGLY SUPPORTED.** The keys `0x676570` describes are `frequentdeathmessages`, `endofgamemessages`, `minimum`,
`default`, `maximum`, `SaveGameLimit`, `IntermediateLevels`, `HealthGainedFromPainkiller`, `DeathFrequency` and
`FrequentDeaths`, so the rows are steps between `[minimum]`, `[default]` and `[maximum]`.

**UNKNOWN.** How `IntermediateLevels` places the rows, and which row is the easiest. The next read is the loop after
`0x6767a1`, which allocates `count * 16 + 4` bytes.

## What a level measures

**VERIFIED.** The player object keeps three counters, cleared at `0x686f38`-`0x686f44`:

- `+0x75c` deaths, + 1 in the death handler at `0x69651a`.
- `+0x764` seconds played, and `+0x760` a health sum. Once per game second (`0x687843`-`0x6878cc`, `+0x768` carries the
  fraction), the sum gains the current health (`0x6d71b0`) plus painkillers (`0x694630`) times the mode's `+0xc`
  (`0x676400`) times the maximum health (`0x691bd0`).
- `+0x778` adds the frame time while `0x742910` reports 0, and is zeroed otherwise. Seconds stop counting once it
  reaches 60.0 (`0x7cd178`). **HYPOTHESIS:** an idle player does not count.

## The level-end step

**VERIFIED.** `0x453b40` runs from level init (`0x44fe2b`, after the `Level init` string) and judges the level just played.
The health value is `h = (sum / seconds) / maximum health` (`0x453b65`-`0x453b9b`). The level's `[Difficulty]` block
(parsed at `0x525883`-`0x525aec`) holds:

| Key | Offset | Getter |
|---|---|---|
| `MaximumHealth` | `+0xc8` | `0x526640` |
| `MinimumHealth` | `+0xcc` | `0x526650` |
| `MinimumDeaths` | `+0xd0` | `0x40e550` |
| `MaximumDeaths` | `+0xd4` | `0x40e560` |
| `MinimumTime` | `+0xd8` | `0x5f8f00` |
| `MaximumTime` | `+0xdc` | `0x526660` |
| `HardcoreHealth` | `+0xe0` | `0x40e590` |
| `HardcoreDeaths` | `+0xe4` | `0x526670` |
| `HardcoreTime` | `+0xe8` | `0x40e5b0` |

Two scores start at 0 (`0x453d1d`-`0x453df4`):

| Test | Score |
|---|---|
| `h > MaximumHealth` | up + 1 |
| otherwise `h < MinimumHealth` | down + 1 |
| `deaths < MinimumDeaths` | up + 1 |
| otherwise `deaths > MaximumDeaths` | down + 1 |
| `seconds < MinimumTime` | up + 1 |
| otherwise `seconds > MaximumTime` | down + 1 |

A hardcore run is `h > HardcoreHealth`, or `deaths < HardcoreDeaths`, or `seconds < HardcoreTime` (`0x453e1c`-`0x453e7e`).
Up 2 or more, or a hardcore run, calls `0x676230` and stores +1 in the level's history record. Otherwise down 2 or more calls
`0x676260` and stores -1 (`0x453e86`-`0x453ebb`). The record (36 bytes, list at `+0x12e4`) also keeps the row, deaths, h and
seconds; `0x456d10` prints the list under `Difficulty   Deaths   Health    Time    Level Name`.

## Why retail escalates

**HYPOTHESIS.** The counters live in the player object, and a load replaces it with the saved one. Deaths are counted when
Max dies, but the reload that follows restores the count from before the death. A level finished after deaths then
scores as one finished without them, which matches the reported escalation. The test is a debug run with the
`0x456d10` dump after a level with several deaths.

## Ruled out

- `0x6760e0` is not the selection; it only returns the current row.
- `Difficulty Deaths Health Time` is a debug header (`0x86db4c`, pushed at `0x456da1`), not a parser key.
