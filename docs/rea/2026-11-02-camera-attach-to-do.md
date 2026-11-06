# `CAM_AttachToDO` (MaxPayne.exe)

188 retail uses. In-level cutscenes attach the view to a camera dynamic object and later hand it back:
`::finito::camera2` starts with `CAM_AttachToDO(100, false)`, and its track 3 end list sends
`CAM_AttachToDO(0, false)` together with `GM_EnableWidescreen(false, 0)` and `GM_SetPlayerControls(true)`.

## Handler chain (VERIFIED, static)

- DO handler `0x6f4360` (receiver template on `X_LevelRuntimeDynamicObject`). If the DO's continuous-update flag
  `+0x4af` is off it prints `Warning: DO "%s" doesn't have continuous update on while attaching camera to it`
  (`0x89ca6c`) and goes on. It calls `0x743f40` on the camera pointer at DO `+0x461`, passing the message and the DO's
  spatial base (`this+0x1e`).
- `0x743f40` vcalls camera slot `+0x74`: `X_CameraImplementation` (vtable `0x7d56cc`) slot 29 = `0x672ce0`.
  `AnimateInPlace` (`0x6f43d0`) and the sibling at `0x6f43b0` reach slots `+0x70` / `+0x6c` (`0x672c10`,
  `0x672b00`) the same way.
- `0x672ce0`: arg 0 (float, `0x74cab0`) → camera `+0x14f`; arg 1 (bool, `0x73e340`) → `+0x153`; target spatial →
  `+0x143`; then it builds the camera frame from the target's matrix.

HYPOTHESIS: arg 0 is the camera's weight in percent. 100 attaches and 0 detaches in every pair checked
(street01, street02, finito). Arg 1 is UNKNOWN; all checked calls pass `false`.

## Engine

`LdbScene::attached_camera` is the receiver DO for a positive weight and `None` for 0. While attached, `render3d`
takes the eye from the DO's world pose. Forward is the DO's **+z** axis and up its y axis, as for the startup
flyover. Right is up × forward, matching the player camera. The player keeps moving underneath; retail disables
controls with a separate `GM_SetPlayerControls`.

STRONGLY SUPPORTED, +z (2026-11-03): offline frames of the four house cutscene cameras along +z show their
intended shots. `::entrance05::camera1` gives a high establishing view of the living room (its five tracks are
static; the intro fades in on it with `STRING_MAXHOUSE_YEARS_AGO`), `::startroom::camera2` the hallway,
`::entrance06::camera2` the mirror, and `::Bathroom::camera2` the bathroom with the blood on the floor. Along −z
each of them faces a wall from close up.

**Superseded:** an earlier version of this note chose −z because the 24-poly camera model widens toward local −z
and because single static frames of `::street02::camera1` and P3L6 `altitude::camera1` looked better along −z.
Those frames were taken before the cameras' tracks run, and the model's shape does not set the view axis. The
four house cameras outweigh them.

`GM_SetPlayerControls(bool)` (61 uses) clears or sets `LdbScene::player_controls`. While it is off,
`render3d` ignores movement, jump, interact and fire; look keys stay live. HYPOTHESIS from the name and the
cutscene pairing only; the `MaxPayne_GameMode` handler has not been read.

`MPHM_FadeToColor(from, to, seconds)` (HUD mode) ramps a full-screen colour linearly; the
colours are `0xRRGGBBAA` (`0x0` → `0x000000ff` = fade to black). The engine holds an opaque target and drops
the fade once a fade to alpha 0 ends. HYPOTHESIS from the argument patterns (all 189 occurrences in the retail `.ldb` files, FSM and track lists, fit); the
handler has not been read. `MPHM_FlashToColor` (4 args, 101 uses) is not handled.

## `CAM_AnimateInPlace` (camera paths)

`data\database\camerapaths\camerapaths.txt` names blocks. Each block has `[Attributes]` (FadeInTime,
FadeOutTime, Abortable), an `[Animationset]` of `[Animation]` entries (`Index`, `Filename`) and an `[Exit]` list.
`Filename` is relative to `data\database\camerapaths\`. The name does not always match the file: `shake_quick`
plays `effects\explosion.kf2` and `shake_normal` plays `effects\shake_quick.kf2` (VERIFIED, retail text).

Each file is an ordinary skeletal KF2 with one `0x00010012` v5 channel, `Camera01`, at 30 fps. The shakes are
small jitters (centimetres, a few degrees) whose first and last keys are equal and whose first rotation is
identity. `death_01` starts 1.9 m in front of the target and turns 180° to look back at it (VERIFIED, parsed
keys).

All 313 `CAM_AnimateInPlace` calls in the shipped levels go to `player`: `shake_quick` 269, `shake_normal` 26,
`shake_slow` 11, `rumble_medium` 4, `rumble_low` 2, `shake_subway` 1.

The handler `0x672c10` first checks three camera vcalls (`+0x8`, `+0xc` and `+0x10`) and returns if any of
them is true. It then looks the block up by name (`0x54a470`). How the path is composed each frame has not been
read.

**Engine (HYPOTHESIS, from the tutorial's "relative to the current player camera"):** the path is applied in
camera space on top of the player view. The offset is measured from the first key, with x right, y up and
z forward (the `CAM_AttachToDO` axes). A new block replaces a running one. The path runs in game time and ends
with its duration. An attached DO camera takes precedence.

## `CAM_AnimateParented` (death cams)

All 64 calls are enemy death cams (`Death_01..03`, `special_killed`). Most come from a character's own
`OnDeath` list (`::end_combat::boris->CAM_AnimateParented(death_03)`). The rest come from a level death
counter: each enemy's `OnDeath` sends `::storage::death_counter->fsm_Send(add1)`. The counter steps through
its states, and in its last state it sends `::storage::e1->CAM_AnimateParented(death_03)`, so the last kill
gets the camera.

The character record in the `.ldb` holds four message lists after the skin. They were identified by their
content across every shipped level (STRONGLY SUPPORTED; the X_LevelDBCharacter reader has not been read):

| List | Tab | Evidence |
|------|-----|----------|
| 1 | StartUp | `c_setstatemachine`, `c_removeallweapons`, `c_pickupweapon` |
| 2 | OnDeath | all 99 death counters and death cams; 141 `A_StopAll3DSounds`, which the tutorial says belongs on OnDeath |
| 3 | OnActivate | 149 alert shouts, combat state machines |
| 4 | UNKNOWN | — |

**Engine (HYPOTHESIS for the frame):** when an enemy's health reaches 0, its `OnDeath` list is sent. A death
cam plays its `Camera01` path in the target's frame, standing at its feet and facing its yaw (+z), with x right,
y up and z forward. `death_01` starts 1.9 m in front at 0.63 m, faces back and circles to the side. The death
cam ends with its path. An attached DO camera still takes precedence, and the death cam takes precedence over a
shake.

## Open

- The view pose has been checked only in offline engine frames, not against a retail frame.
- ~~Startup flyover~~: the special-case `camera_flyover` is gone. `Startup_level` now runs on its own scripts:
  the startup `Start_FSM.DO->DO_Animate(1)` sends `FSM_Send(start)` to each camera rig, which attaches its
  camera (`CAM_AttachToDO`) and animates the rig. Five shots (`C1_PAN::C3`, `cam_mover::camera1`,
  `car_closeup…cam4`, `chopper…cam_00`, `cam_rotator…altitude::camera`) run for 75.6 s and end with
  `x_modeswitch->s_modeswitch(graphicnovel)`, which closes the intro. The old flyover covered only the last shot.
- `::street02::camera1` and P3L6 `altitude::camera1` along +z once their tracks run.
- Blend between weights (does 50 mix the views?), arg 1, field of view (`GM_ChangeFOV`).
- Camera path fades, `Abortable`, the `[Exit]` lists and the slow motion during death cams.
