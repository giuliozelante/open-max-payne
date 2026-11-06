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

**UNKNOWN:** the rest of `0x695fb0`, and `0x695b10`, which `C_CauseDamage`'s neighbour at `0x699d40` calls with a
flag of 0. `C_AddHealth` and `C_CauseDamage` reject negative values (`X_CharacterException_NegativeHealthValue`,
checks at `0x699b00` and `0x699c30`), but the units they apply are not read yet.
