# Spec: Open Max Payne (approved)

Goal:
  Standalone project to play Max Payne 1 and use MAX-FX on Windows 10+, Linux, macOS.

In / out:
  In: Play (CD install + wrappers), Content (RAS/MPM/particles), Author (Blender KF2), Studio GUI
  Out: Redistributing the game; third-party mod asset trees; full MaxEd rewrite

Behavior:
  Own repo `open-max-payne` — Max Payne play + MAX-FX tools only.

Constraints:
  Own media required; vendor/ gitignored; format claims backed by specs/REA.

Done when:
  - [x] Separate repo + README pillars
  - [x] Linux CD install → ~/Games/MaxPayne1 (exe + .ras + launcher)
  - [x] Native Tauri Studio binary + `./openmaxfx` front door
  - [x] Rust `crates/openras` + synthetic CI tests (Inc 1–2)
  - [x] Blender KF2 path documented/stubbed (`tools/blender_io_kf2`, kf2_format.md)
  - [x] Inc 3: `maxfx-patcher` HTTPS+SHA256, refuse SFX; `maxfx-play` Wine32/fail-closed; Studio process manager
