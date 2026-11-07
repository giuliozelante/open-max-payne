# Player health and enemy AI

**Status.** Approved 2026-11-06, with the three decisions below as recommended. Phase 1 first.

**Problem.** The Tutorial's gunfight cannot play. Max has no health: the HUD draws a fixed `1.0`
(`render3d.rs:1562`). Enemies never shoot. Those with `pursues` walk straight at Max using hand-written
numbers (24 m range, 3.2 m stop, 2.6 m/s, `render3d.rs:1114`), which is the kind of invented value the
engine should not contain. Scripts that drive the fight are ignored: `C_SetStateMachine`, `C_GoTo`,
`C_GoToAndShoot`, `C_SetIdle`, `C_SetWoundedAnimations`.

**Goal.** The Tutorial's combat stages play from retail data: enemies notice Max, aim, shoot and move on the
level's AI network; Max can be hurt, heal and die.

## What the retail data gives

Health is almost entirely text data, so phase 1 needs little reverse engineering.

| Value | Source | Retail value |
|---|---|---|
| Max's health | `skins/max_payne.txt` | `MaximumHealth = 60`, `WoundedHealth = 12` |
| Enemy health classes | `ai_ranges.h`, then each skin | 15 to 240 |
| Difficulty scaling | `difficulty.txt`, `[normal]` / `[default]` | enemy 0.8, player 1.0 |
| Painkiller | `difficulty.txt` | 20% of maximum, 5% at once, 4%/s regeneration |
| Hit zones | skin `[Target]`, `characters.h` | head 5, arms 0.75, legs 0.5 |
| Falling damage | `globalconfig.txt` | threshold 15 m/s, exponent 2.1 |
| Damage flash | `globalconfig.txt` | `DamageFlash*` keys |
| Kill bonus | skin, `characters.h` | `SlowMotionBonusForKiller = 0.12` |
| Enemy senses | skin `[AI]`, `ai_ranges.h` | mobster: sight 50 m, shot heard 15 m, near 2 m, interest 30 s |
| Enemy aim | skin `[AI]`, `ai_ranges.h` | mobster: `AimingSpeed` 10, `ShootingCone` 120, turn 520 |
| Enemy fire rate | skin `ShootingFrequencyMultiplier` | `SHOOTRATE_CLASS_*`, 0.10 to 0.70 |

The AI network is in `levels/front_end/Tutorial.ai` (142,248 bytes). Its format is **UNKNOWN**; only
`AINodeCastHeight` and `AINodeCollides` are named in the exe.

## State machines are not data

**VERIFIED:** the machines are compiled into `MaxPayne.exe`. No retail file defines them. The names are in
strings at `0x87c1fc`-`0x87d064`, each referenced from a builder function (`StandAndShoot` at `0x4ea8d0`,
`MobsterCombat` near `0x4e3603`). Together the builders span about 117 KB, `0x4cf700`-`0x4ec150`.

**STRONGLY SUPPORTED:** the builders are tables written as code. `StandAndShoot` is a flat run of calls with
small integer arguments: `0x4ed6a0` (579 call sites), `0x4ec6f0` (194), `0x4ec910` (91), then `0x4ee130` and
`0x4ee690`. Reading those helpers should give states, actions, conditions and transitions.

So the rule "read the data, invent nothing" has to bend in one place. The proposal is to decode the builders
with a script, record the result in `docs/rea/`, and carry only the machines we need as a Rust table. Each
table cites its builder address, as `clips::blend_seconds` does for the retail blend table. The alternative,
reading the tables out of the owner's exe at runtime, ties the engine to one exact binary.

The Tutorial uses few of them: `RunWalkAndShoot` (6 calls), `StandAndShoot` (3), `StandAndShootStatic` (3),
`MobsterCombat` (1), plus `C_GoTo` (3) and `C_GoToAndShoot` (3).

## Phases

Each phase is its own PR series and is usable alone.

**1. Health.** No AI needed.
- Read max and wounded health from skins, with the `[normal]` / `[default]` difficulty multipliers.
- Add a player health model, and drive the HUD pain bar and damage flash from it.
- Handle `C_AddHealth`, `C_CauseDamage`, `C_SetImmortal`, `C_SetDodgeImmortal`, painkiller use and falling
  damage.
- Apply `[Target]` multipliers. Read the retail hit code first: the current hit test is one capsule and has
  no bones, and guessing zones from hit height is not acceptable.
- Play Max's death clip, then restart the level (see decisions).

**Re-plan for phases 2 and 3, 2026-11-07,** after the first decode (`docs/rea/2026-11-07-ai-state-machines.md`). The risk
below did not show: machines are plain rule tables. Enemies also run on the player's input interface, so the engine's
character should take the same inputs from either the keyboard or a machine. Proposed order:

1. Read enemy perception. The machine evaluator is done.
2. Give enemies a shared character step: move, turn, shoot from inputs.
3. Carry `StandAndShoot` and `StandAndShootStatic`, then fire with the projectile path Max uses.
4. Decode `.ai`, then `RunWalkAndShoot` and `MobsterCombat`.

**2. Standing enemies.** No navigation needed.
- Decode the builders and write the REA note.
- Add a machine interpreter with the idle family, `StandAndShootStatic` and `StandAndShoot`.
- Add perception from `[AI]`, with line of sight through the existing hit tests.
- Enemies aim at `AimingSpeed`, then fire through the weapon and projectile path Max already uses, with
  shot clips and their frame messages.
- Delete the hand-written pursuit.

**3. Moving enemies.**
- Decode `.ai`, then add pathing, `C_GoTo` and `C_GoToAndShoot`.
- Add `RunWalkAndShoot` and `MobsterCombat`, and enemy clip blending.

**Left out:** mercenary and melee machines, enemy dodges, adaptive difficulty (`levels.txt` `MinimumHealth`,
`HardcoreHealth`), Hard-boiled and later difficulties, saved games.

## Verification

Each decoded value gets a retail-gated test against the real files. Machines get table tests from the REA
note, and scripted scenarios run headless: an enemy with Max in sight fires within its aim limits. Behaviour
is checked last against a player capture. The two retail videos have not been checked for the Tutorial
gunfight; if they lack it, a new retail recording is needed before phase 2 closes.

## Risks

- `.ai` may take long to decode. Phase 3 is the only phase blocked by it.
- Builders may call code that the table reading misses, such as conditions hidden in virtual calls. The first
  decode of `StandAndShoot` settles this before the interpreter is written.
- Painkiller timing is a **HYPOTHESIS**: 5% at once, then 4%/s up to 20% in total. Keys say so; the exe has not
  been read.
- Enemy accuracy has no obvious key yet. `BULLETSPREAD` applies to Max's projectiles, and
  `projectiles/enemy_beretta.txt` is a dropped weapon with `Damage = 0`. Must be found in phase 2, not tuned by
  eye.

## Decisions

1. **Machines as a cited Rust table.** Recommended over reading the exe at runtime. This is the one exception
   to "no manual values".
2. **Death restarts the level.** Retail reloads the last save, but the engine has no saves, so restart is the
   closest cheap behaviour. Real saves stay out of scope.
3. **Start with phase 1 only.** Phases 2 and 3 can then be re-planned from what the first builder decode shows.
