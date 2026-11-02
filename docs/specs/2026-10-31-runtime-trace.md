# Spec: Retail runtime APIs + skeletal playback (2026-10-31)

Goal:
  Trace how the original `MaxPayne.exe` drives animation / audio / input at
  runtime, then play the first real KF2 clips on SKD-weighted KFS skins so the
  walkthrough stops showing a broken T-pose.

In / out:
  In: FixPack `~/Games/MaxPayne1/MaxPayne.exe` (sha256 `342d296c…da46e`),
      `e2mfc.dll` / `sndmfc.dll` / `rl.dll`, BYOD `x_data.ras` extracts under
      `/tmp/omp-xdata-ref` (not committed).
  Out: claiming a playable native game; shipping retail assets.

## What REA can and cannot do here

| Tool | Result on this target |
|------|------------------------|
| `observe_native_calls` | **No.** LLDB / Mach-O only. PE32 `MaxPayne.exe` is not attachable. |
| `open_binary` + Ghidra | **Yes.** Session already on the 1.05 exe. Searches time out while auto-analysis runs. |
| `capture_process_scenario` | **Yes**, but it is a PTY wrapper — useful for Wine debug-channel logs, not in-process D3D hooks. |
| Live Win32 API | Wine `WINEDEBUG=+loaddll,+d3d8,+d3d,+dsound,+dinput` on the FixPack exe. |

Internal gameplay APIs (`C_Jump`, `A_Play3DSound`, `WS_AnimateShooting`) never
appear as Win32 imports. They live in the exe / `e2mfc.dll` and are dispatched
from `default_skeleton.txt` frame messages.

## Runtime API map (exports + skeleton script)

From `e2mfc.dll` RTTI / mangled exports:

- `P_SkinMesh`, `P_ReferenceSkinMesh` — CPU/driver skinned mesh
- `P_DriverException_TooManyBonesPerVertex` — ≥3 influences rejected
- `P_VertexBuffer`, `P_VertexShader`, `P_DDVertexBuffer` — D3D8 path

From `sndmfc.dll`:

- `S_Sound`, `S_SoundOmni`, `S_SoundSpot` + `S_Listener`
- flags `S_Sound::MUSIC`, `S_Sound::STREAM`

From `database/skeletons/default_skeleton.txt` (retail, not in git):

| Hook | Meaning |
|------|---------|
| `this->A_Play3DSound(bank, cue, bone)` | 3D sound attached to a named bone |
| `this->C_Jump(n)` / `C_Use` / `C_Reload` / `C_AddHealth` | character verbs |
| `RightHandWeapon->WS_Animate(WEAPONANIM_*)` | weapon KF2 on `Gun` / `Hand-L` |
| `WS_AnimateShooting(shoot, empty)` | fire + empty-click pair |
| `PS_StartEffect(fx, bone)` | ParticleFX on a bone |
| `P_CreateProjectileToBone(proj, n, bone)` | hitscan / grenade / blood dummy |
| `C_AnimateTexture(slot, face_id, time)` | face/grin swap |
| `C_DisplayCrosshair(0/1)` | HUD |

`CHARANIM_*` indices (`characteranimid.h`) map 1:1 onto those KF2 files.
One-handed stand/aim for Max is `idle_aim.kf2`; locomotion is `Walk.kf2` /
`run.kf2`. Two-handed set is the `2_*` prefix.

Camera / capsule constants from the same file (metres):

- `DefaultAimingPointOffset = (0.12, 1.50, 0.00)`
- `DefaultCameraPointOffset = (0.12, 1.75, -0.12)`
- `ENEMY_WALK = 1.5`, `ENEMY_RUN = 3.0`

## On-disk formats used by this increment

### KF2 animation (`0x00010012` ver 5)

One top-level chunk per bone. Payload:

1. Nested `0x00010013`: bone name
2. Parent bone name (empty string = root)
3. `0x0e` bool
4. `0x11` duration in frames (idle_aim = 90)
5. `0x11` keyframe count
6. ×N: `0x11` frame index + `0x1a` mat3 (column-major, same as LDB) + pos
7. Trailer (`0x11 1`, `0x11 0`, `0x09 1.0`, …)

Matrices are **parent-relative**, already in Max-FX Y-up.

### SKD (`0x00010014`)

1. `n_rigid` + `n_rigid × (vert, bone, 1.0)` — single-influence verts
2. `n_blend` + two lists of `n_blend × (vert, bone, weight)` — dual influence
   (`P_DriverException_TooManyBonesPerVertex` matches this cap)
3. Mesh name, bone-name list (28 for Max), `n_verts` bind vec3s (874 = KFS)

`648 + 226 = 874` on `PAYNE_MAX_L0.SKD`.

## Wine live load order (2026-10-31, 22s `WINEDEBUG=+loaddll,+d3d8,+d3d,+dsound,+dinput`)

Log: `/tmp/omp-wine-api.log`. Thread `0148` is `MaxPayne.exe`:

1. `MaxPayne.exe` @ `00400000`
2. `rlmfc.dll` — archive / script (`R_File`, `#include`)
3. `grphmfc.dll` — JPEG/TGA/PCX/DDS decode
4. `e2mfc.dll` — `P_SkinMesh` / D3D driver
5. `DSOUND.dll` (native) + `sndmfc.dll` (`S_SoundOmni` / `S_SoundSpot`)
6. `DINPUT.dll` + `dsoal-aldrv.dll`
7. `d3d9.dll` + `D3D8.DLL` (d3d8to9 probe; unloaded when the 22s timeout killed the config dialog)

`observe_native_calls` still cannot attach. This `+loaddll` trace is the PE equivalent of “which APIs come up at boot.”

## Native behaviour (this increment)

- Drop the procedural T-pose arm hack (it tears the mesh at the shoulders).
- Skin KFS verts with SKD weights + first/interpolated KF2 pose.
- Idle = `idle_aim.kf2`; moving = `Walk.kf2` (loop on `duration` frames @ 30 fps).
- If SKD/KF2 is missing, render the connected bind-pose mesh (no arm hack).

Done when:
  - Synthetic `parse_kf2_anim` / `parse_skd` / `skin_vertices` tests green
  - BYOD `PAYNE_MAX_L0.SKD` reports 28 bones + 874 weighted verts when present
  - Walkthrough no longer shows disconnected floating hands
  - Wine `+loaddll` log names `e2mfc` / `d3d8` / `dsound` / `dinput` on the FixPack exe
