# Tutorial enemies

**VERIFIED (`Tutorial.ldb`, retail video 2026-11-06):** all six characters start in the room `::Teleport`.
`::Teleport::Exit_Disable` closes both sides of its only exit at startup with `GM_EnableExit`. That room overlaps
`::Street` in world space, so it stays out of the world only because no open exit leads to it.

No enemy stands in the alley behind the padlock. Scripts bring them in:

1. `::Street::Use_04`, the ringing phone, enables `::Street::T_Dispenser`.
2. The dispenser button sends `touch` to `::Street::Dispenser_1` and `Dispenser_2`, which teleport characters to
   `Tele_*` start points. In the video the first enemy shows within a second of the press.
3. `::Subway::t_health` switches `::Street::T_01` to state 2. Its `T_Activate` then opens the van and teleports E04
   to E06 to `::Street::W03` to `W05`. The tip announces three enemies from the side alley.

The engine used to apply every `C_Teleport` found in any script at load, so all six stood in the street from the
start. Now only `startup_actions` place a character, and the rest arrive as orders.

**HYPOTHESIS:** a character is out of the world while its room has no open exit and Max did not start there. Retail
gets this from portal traversal, which the engine does not do for characters.
