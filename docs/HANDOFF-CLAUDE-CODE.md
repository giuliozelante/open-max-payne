# Handoff — Open Max Payne → Claude Code

**Date:** 2026-11-02 (~07:30 CET)  
**From:** Cursor agent session (Composer)  
**Repo:** `/home/gzelante/Lavoro/personale/open-max-payne`  
**Owner intent:** Finish ASAP; evidence-only RE; no fabricated engine claims; BYOD / OpenLara-style north star.

Read first: `CLAUDE.md`, `docs/specs/2026-10-30-open-max-payne.md`, then the REA notes under `docs/rea/`.

---

## 1. What this project is

Standalone **Max Payne 1** project (not a multi-game host):

| Pillar | Role today |
|--------|------------|
| **Play** | Wine + DXVK / d3d8to9 bridge; Linux CD install scripts |
| **Content** | Rust `openras` (RAS/MPM), `openpfx` (PSE dump/export) |
| **Author** | RASMaker-oriented parity; Blender KF2 path later |
| **Studio** | Tauri GUI; front door `./openmaxfx` |
| **Engine** | Long arc: native Rust `maxfx-engine` walking skeleton → play without `MaxPayne.exe` |

**North star:** native engine + native MAX-FX tools; user supplies retail install. Wine is the bridge, not the destination.

**Legal / hygiene:** never commit `vendor/`, game installs, `*.exe`/`*.dll`/`*.ras`/`*.mpm`, FixPack blobs, or proprietary SDKs. Do not ship Remedy binaries or third-party mod asset trees.

---

## 2. Environment & artifacts

| Item | Path / note |
|------|-------------|
| Retail install (BYOD) | `~/Games/MaxPayne1/` |
| Primary RE artifact | `/home/gzelante/Games/MaxPayne1/MaxPayne.exe` (PE i386, ImageBase `0x400000`) |
| REA MCP | `user-rea` / rea-agents; prefer Ghidra provider when healthy |
| RE method | `~/.cursor/rules/rea-method.mdc` — Strings → Static → Dynamic → Symbolic → Evidence-then-code |
| Cargo | Prefer `--offline` when deps already fetched; Studio build: `cd studio && ./build.sh` (Cursor may pin RUSTC 1.83) |

---

## 3. Git / working tree (as of handoff)

- **Current branch:** `feature/startup-intro` @ `69e66bd`  
  (`feat: house comic sequence, third-person player spawn, and door interaction`)
- **`develop`:** `bb96ca8` (ahead of remote by 1) — skin KFS + KF2 idle/walk
- **Dirty (do not casually revert / do not mix into unrelated commits):**
  - `crates/maxfx-engine/src/{lib,menu_ui,render3d,scene}.rs`
  - `openmaxfx`, `studio/src-tauri/src/lib.rs`, `studio/src/main.ts`
  - **Untracked REA notes (important):**
    - `docs/rea/2026-11-01-do-animate-playback.md`
    - `docs/rea/2026-11-01-kf2-vertex-animation.md`
- Standing rule from owner: leave dirty `studio/` / `openmaxfx` alone unless the task is Studio.
- **Do not commit/push unless explicitly asked.**

---

## 4. What is already done (baseline)

Shipped / green enough to treat as baseline (see approved spec):

- [x] Repo + pillars + CI (OS matrix, CodeQL, Dependabot, release)
- [x] Linux CD → `~/Games/MaxPayne1` + bootstrap / localized safe stack (`docs/PLAY_LINUX.md`)
- [x] Tauri Studio + `./openmaxfx`
- [x] `openras` + RASMaker flag parity direction
- [x] `openpfx` PSE dump + PSE→RPS export path
- [x] `maxfx-patcher` / `maxfx-play` Wine play bridge
- [x] `maxfx-engine` walking skeleton: BYOD archive load, window path, early 3D / KFS skinning / KF2 idle-walk experiments
- [x] Startup / comic / house sequence work on `feature/startup-intro` (see recent commits)
- [x] ModeSwitch REA + specs (`docs/rea/2026-10-31-modeswitch*.md`, matching specs)
- [x] LevelDB / RAS / RPS / KF2 format specs under `docs/specs/`

Engine is **not** “retail-complete.” Do not claim a playable native replacement of MaxPayne.exe without REA-backed specs + a real walking skeleton for the claimed feature.

---

## 5. What the last Cursor session was doing (REA focus)

Owner asked to keep climbing RE (“yolo”), then wanted overnight progress. Cursor wake-loops were a bad fit (process died; IDE doesn’t guarantee agent wakes). **Prefer a normal Claude Code session**, not shell wake loops.

### 5.1 DO_Animate (dynamic mesh transform curves) — not KF2 bones

**SoT note:** `docs/rea/2026-11-01-do-animate-playback.md` (untracked)

| Topic | Status | Key evidence |
|-------|--------|----------------|
| Message apply | VERIFIED | `0x6f4a20` registers find/apply; apply `0x6f3d20`; name thunk `0x751f60` → `DO_Animate` |
| Playback mechanism | VERIFIED | Transform **curve sampling**, not KF2. Tick `0x6f34c0` (DO vt `+0xb0`). Sampler `0x5ff100` |
| Playhead fields | VERIFIED | `+0x49b` time; `+0x4a3` hold; `+0x4a7` reverse; `+0x4c8` pause save; pose copy → `+0x259` |
| Pause/resume slots | VERIFIED | vt `+0xb8`=`0x6f3b70` snapshot; `+0xbc`=`0x6f3b90` restore; invert helper `0x6f40e0` |
| Curve object | VERIFIED | `+0x483` → `X_LevelRuntimeDynamicObjectAnimation` → `X_LevelDBDynamicMeshAnimation` |
| Exit binder vs DMA | VERIFIED | Exit `+0x84` is pointer; DMA duration elsewhere — do not conflate |
| Who calls tick `0x6f34c0` | **BLOCKED** | Only **2** `call [reg+0xb0]` in entire PE: Stats (debug-gated) + LocalCharacterInput. `0x5ff100` only called from DO tick/apply. Production path **UNKNOWN** without dynamic. |
| LCI `+0x1b5` | STRONGLY SUPPORTED | Often `&GameMode+0x258` = `MaxPayne_PlayerInput`; that class’s `+0xb0` is `ret 4` (no-op). Map path `0x4afd7a` may store a different receiver — unproven. |

**Next DO climb (priority):**

1. **Dynamic:** break on `0x6f34c0` under Wine while a level DO is animating; capture caller.
2. Finish typing `LocalCharacterInput+0x1b5` (map node vs PlayerInput vs DO*).
3. Wire Pause/Resume/Stop/Invert **messages** to the vt slots (RTTI blobs at `0x6f7xxx` look like type-name registration, not the vcall sites).
4. Curve header tag `0x71`; channel counts; matrix IO ↔ `M_Matrix4x3Template`.

### 5.2 KF2 vertex animation (character/mesh morph path)

**SoT note:** `docs/rea/2026-11-01-kf2-vertex-animation.md` (untracked)

| Topic | Status | Key evidence |
|-------|--------|----------------|
| Apply | VERIFIED | VA vt `+0x48` = `0x57cb70` → morph `0x57c860` (matrix×verts) |
| Driven object | VERIFIED | `+0x115` = `KF_ObjectAnimation` (ctor `0x54c1c0`, vt `0x7d0e94`) |
| LOD factory | VERIFIED | `KF_VertexAnimationLODs` builds VAs; getter `0x562440` RTDynamicCast `P_BaseObject` → `KF_VertexAnimation` |
| LOD tick API | VERIFIED | Forwards `+0x5c`/`+0x64`/`+0x70`… — **never** `+0x48` |
| Who calls `0x57cb70` | **BLOCKED** | No `E8` callers; only vt imm. Static exhausted → **dynamic break** |

**Next KF2 climb:** break on `0x57cb70`; type `+0x111`/`+0x119`; bone path via `KF_ObjectKeyframeAnimationCallback`; SKD → VA tree.

### 5.3 Engine implementation vs REA

- Native engine already experiments with KFS skin + KF2 idle/walk (`develop` / recent merges).
- DO_Animate retail path is **separate** from KF2 vertex apply — do not merge the two in code or docs without evidence.
- Dirty engine/Studio files on `feature/startup-intro` may be mid-feature; don’t wipe them for RE docs.

---

## 6. Tooling pitfalls (this session)

- **REA/Ghidra** often timed out on `analyze_function` / `procedure_callers` / `xrefs`. Session may need `open_binary` again on `/home/gzelante/Games/MaxPayne1/MaxPayne.exe`. On timeout: static PE (`objdump` + careful RVA) is valid ladder rung 2 — still evidence-only.
- **Cursor overnight `/loop`:** failed (process died, empty log). Do not rely on it. Use Claude Code continuous work or owner-driven continues.
- **Gemini:** was rate-limited earlier; owner said climb solo. Only adapt Gemini claims when bytes match.
- Confidence labels in notes: **VERIFIED / STRONGLY SUPPORTED / HYPOTHESIS / UNKNOWN** — keep using them.

---

## 7. Suggested next work (ordered for ASAP)

1. **Dynamic RE (highest leverage)**  
   Wine + retail `MaxPayne.exe`, break `0x6f34c0` (DO tick) and `0x57cb70` (KF VA apply). Write callers into the two REA notes.

2. **Persist REA notes**  
   Add/commit (when asked) the two untracked `docs/rea/2026-11-01-*.md` files so they aren’t lost.

3. **Engine follow-through (only with REA)**  
   - Character skinning: align with KF2 note once production caller of `0x57cb70` is known.  
   - Level dynamic meshes: implement DO curve sampling from the DO note (`0x5ff100` semantics), not guesswork.

4. **Studio / startup-intro branch**  
   Finish or land `feature/startup-intro` separately from RE; don’t mix RE commits with Studio dirty tree unless asked.

5. **Near-term product (from CLAUDE.md)**  
   RASMaker parity polish; PSE→RPS already largely done; keep Wine Play as bridge.

---

## 8. Commands cheat sheet

```bash
cd /home/gzelante/Lavoro/personale/open-max-payne

# crate tests
cargo test -p openras --offline
cargo test -p openpfx --offline
cargo test -p maxfx-engine --offline

# Studio
cd studio && ./build.sh

# Play docs
# docs/PLAY_LINUX.md
# runtime/bootstrap-linux.sh , install-from-cd.sh

# RE artifact
# /home/gzelante/Games/MaxPayne1/MaxPayne.exe
```

---

## 9. Do / Don’t for the next agent

**Do**

- Spec + REA before claiming format/engine behaviour.
- Update `docs/rea/*.md` with addresses, callees, ruled-out paths.
- Prefer Rust crates; Studio must call crates (no Windows-only `.sh` as the real path).
- One issue / one branch when implementing features.

**Don’t**

- Invent verbs/types (“this starts KF2 play”) without matching bytes.
- Commit secrets, game binaries, or `vendor/`.
- Treat Stats-mode or PlayerInput no-op `+0xb0` as the production DO anim driver.
- Claim native engine parity with retail without evidence.
- Depend on Cursor timer loops for progress.

---

## 9b. Corrections (Claude Code, 2026-11-02)

- **Dirty tree is undocumented feature work, and CI-red.** Uncommitted `scene.rs` (+1.3k lines)
  adds FSM message dispatch (`DO_Animate`, `DO_Hide`, `DO_SetHitpoints`, `T_Enable`,
  `A_Play3DSound`), startup hides/enables, breakable bathroom glass, sound catalogs. Tests pass
  (all crates green), but `cargo fmt --check` fails (`scene.rs`, `studio/src-tauri/src/lib.rs`)
  and clippy `-D warnings` fails (`items_after_test_module`, `render3d.rs:1581` — `draw_line`
  after `mod tests`). Fix before landing.
- **Engine DO_Animate playback is heuristic, not REA-backed** — yet the sampler is already
  VERIFIED (see DO note). `anim_weight`/`bone_from_anim` guess which curve is translation by
  comparing its last sample to the distance, normalize by last sample, and snap rotation at end.
  Retail: curve 0 × `1/dist` = translation weight; curve 1 = slerp `t` between start/end quats.
  Doors that rotate were wrong. **Fixed 2026-11-02:** `track_weights` / `sample_curve` in
  `scene.rs` follow `0x5ff100` / `0x783450`; rotation slerps in the absolute-pose branch.
  Probe over retail test levels: 6271 curves, all 1 channel, header `0x71`/`3`.
  Still open: offset-pose branch (`world_pos != local_start_pos`) applies translation only.
- **Priority reorder.** The unknown production caller of `0x6f34c0` does **not** block engine
  playback: sampler semantics are fully specified by bytes. Do the sampler fix first; dynamic
  RE is for tick ordering/timing, not correctness of the pose.
- Static re-check: exactly 2 `call [reg+0xb0]`, zero `jmp [reg+0xb0]`, zero `mov r,[r+0xb0]; call r`,
  zero refs/thunks to `0x6f34c0`. Handoff claim holds; dynamic is the only remaining rung.

## 10. Open questions (copy for tracker)

- [x] Production caller of DO tick `0x6f34c0` — `0x6e1a00` (primary vt `+0x18`) via `call [vt+0x1c]`;
      the old "primary" vt `0x7d7ac0` was a secondary vtable (see DO note, dynamic result)
- Dynamic RE recipe: mount the CD and link `.wine/dosdevices/d:` to the mount point, or the game stops
  at the "minimum installation" dialog. Use `winedbg "Z:\…\MaxPayne.exe" -nodialog < script`
  (native winedbg; the gdb proxy loses sync on thread creation). Don't `wineserver -k` while the user is playing.
- [ ] Production caller of KF VA apply `0x57cb70`
- [ ] Exact type in `LocalCharacterInput+0x1b5` on animating-DO frames
- [ ] Pause/Resume/Stop/Invert message → vt wiring
- [ ] Curve tag `0x71` / channel layout
- [ ] KF `+0x111` / `+0x119` semantics; SKD → VA LOD load
- [ ] Whether `feature/startup-intro` dirty Studio/engine work should land before more RE

---

*End of handoff. Primary SoT for the unfinished RE climb is the two 2026-11-01 notes under `docs/rea/`.*
