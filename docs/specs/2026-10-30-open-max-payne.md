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
  - [ ] Blender KF2 path documented/stubbed (deferred)
  - [ ] Inc 3: patcher HTTPS/hash + Wine32 fail-closed (deferred)
