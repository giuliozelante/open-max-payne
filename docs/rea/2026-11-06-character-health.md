# Character health in `MaxPayne.exe`

Addresses are PE i386 VAs, ImageBase `0x400000`.

**VERIFIED: `C_SetHealth` takes a fraction of maximum health.** The argument check at `0x6999a0` compares it
with `0.0` (`0x7cb8b0`) and `1.0` (`0x7cbdd0`) and raises `X_CharacterException_InvalidHealthValue`, whose text
is "must be within range 0..1". The handler at `0x699ad0` multiplies the argument by the result of `0x691bd0`
and passes that to the absolute setter at `0x695fb0`. So `this->c_sethealth( 1 );` in `max_payne.txt` `[OnInit]`
and `::Teleport::E01->C_SetHealth(1);` in the Tutorial both mean full health, not one point.

**VERIFIED: maximum health is the skin value times a difficulty multiplier.** `0x691bd0` loads the integer from
`0x5310a0` and multiplies it by one of two getters on the difficulty object at `[esi+0x490]`: `0x676370` (`+0x0`)
or `0x676390` (`+0x8`). The table built at `0x677150` names those slots `EnemyHealthMultiplier` and
`PlayerHealthMultiplier`.

**STRONGLY SUPPORTED:** the virtual call at `[edx+0x8c]` in `0x691bd0` selects the enemy slot, so it answers
whether the character is AI-driven. Its target has not been read.

**STRONGLY SUPPORTED:** `0x695fb0` scales the wounded limit the same way, as `0x5310b0 / 0x5310a0 * 0x691bd0`.
That reads as `WoundedHealth / MaximumHealth` of the scaled maximum; `0x5310b0` has not been matched to the key.

**VERIFIED: `C_CauseDamage` subtracts points.** The handler at `0x699d40` calls `0x695b10`, which ends by passing
current health minus the argument to `0x695fb0`. That setter clamps to `0..maximum`.

**VERIFIED: `C_AddHealth` heals in two parts.** `0x699c10` calls `0x695a80`. It takes `min(points, 0x6763e0 x
maximum)` at once, if the character is alive, and adds the remainder to a pool at `[esi+0x72c]`. `0x68f510` drains
the pool each update by `0x6763c0 x seconds x maximum`, capped at what is left, while the character is alive.
`0x6763e0` reads slot `+0x18` of the difficulty table, `HealthGainedImmediately`. `0x6763c0` is taken to be slot
`+0x14`, `HealthRegenerateMultiplier`, from the table order at `0x677150`; its body was not read.

**STRONGLY SUPPORTED: the three protection flags.** `0x695fb0` returns early when `[esi+0x456]` is set, and raises a
result under `0.1` to `0.1` when `[esi+0x454]` or `[esi+0x455]` is set. `0x69a800` writes `0x454`, `0x697340` writes
`0x455` and the handler beside the first writes `0x456`. Their message order gives `C_SetImmortal`,
`C_SetDodgeImmortal` and `C_SetInvulnerable`; the dispatch tables were not read.

**HYPOTHESIS:** the engine feeds regeneration world time, so bullet time slows it. `0x5e53d0` supplies the seconds
and has not been read. The floor applies only to the living: `0x6d6eb0` guards it and is taken to be a dead flag.

**UNKNOWN:** the flash, sound and death side effects in `0x695b10` and `0x695fb0`.
