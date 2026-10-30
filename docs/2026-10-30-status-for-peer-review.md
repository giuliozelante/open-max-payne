# Open Max Payne — status for peer AI review

**Date:** 2026-10-30  
**Repo:** `/home/gzelante/Lavoro/personale/open-max-payne`  
**Audience:** another coding AI (fresh context)  
**Ask:** critique architecture, Play stack, Studio surface, gaps, and risk — not praise.

Do **not** assume Silent Hill / other mod trees belong here. This product is Max Payne 1 play + MAX-FX tooling only.

---

## Review request (what we want from you)

Please answer with concrete file paths and ranked findings (`blocker` / `should-fix` / `nit`):

1. **Scope hygiene** — Is the Play/Content/Author/Studio split clear? Any leftover coupling to third-party mods or dead UI paths?
2. **Play correctness (Italian CD + Wine)** — Is the SafeDisc → 1.05 → Italian-safe (park hi-res/ASI, d3d8to9+DXVK) story coherent? Any footguns in `runtime/*.sh` or Studio launch auto-heal?
3. **Studio quality** — Tauri 2 commands vs UI: missing states, unsafe Wine arg handling, over-trust of scripts?
4. **Content/Author maturity** — Are openras / openpfx / blender stub claims oversold in README vs reality?
5. **Legal / repo hygiene** — `.gitignore` vs what still lives under gitignored `vendor/`; anything that must never be committed?
6. **Next 3 moves** — What should land next before marketing this as usable? Prefer smallest valuable increments.

Evidence over vibes. If you cannot verify (no game CD / no Wine), say so and review static tree only.

---

## Product intent (approved)

Standalone: **play Max Payne 1** and use **MAX-FX** tools on Win10+ / Linux / macOS. Own retail media required; no game redistribution.

Pillars (keep separate):

| Pillar | Intent |
|--------|--------|
| Play | CD install, Wine/Proton wrappers, one-click launch |
| Content | `.ras` / `.mpm` / particles list-extract-pack |
| Author | Blender KF2/skins (3ds Max plugins as REA reference only) |
| Studio | “For dummies” GUI front door |

Canonical mini-spec: `docs/specs/2026-10-30-open-max-payne.md`

---

## What landed (this session / extraction arc)

### Repo bootstrap

- New git repo (as of handoff: **no commits yet** on `main`; whole tree untracked).
- Extracted from a Silent Hill–mod workspace as a **standalone** product; SH install helpers and docs references were **removed** after an intermediate mistake that wired them in.
- Front door: `./openmaxfx` → release Tauri binary if present, else `tauri dev`.
- Agent notes: `CLAUDE.md`. User guide: `START_HERE.md`, `README.md`.

### Play (Linux — primary validation host)

Evidence-backed path for **Italian CD** (SafeDisc stub `stxt*` on stock EXE):

1. Install CABs → `~/Games/MaxPayne1` via `runtime/install-from-cd.sh`
2. Replace EXE with Rockstar **1.05 drop-in** (Studio / scripts treat the patch file as EXE copy, not “run installer as game”)
3. FixPack allowed for CD exe + startup + d3d8to9; **hi-res `data/` and ASI `dinput.dll` crash Italian under Wine** → park them
4. Prefer **d3d8to9 + DXVK x32 `d3d9.dll`**; ThirteenAG’s small `d3d9` wrapper is hostile on Wine
5. Launch: `~/Games/MaxPayne1/launch-openmaxfx.sh` (Wine virtual desktop; NVIDIA ICD hint)

Key scripts:

- `runtime/bootstrap-linux.sh` — overlays + regenerates launcher; Italian auto-park if `x_italian.ras` and bad `data/` / large ASI reappear (`log` after `log()` definition)
- `runtime/italian-safe-stack.sh` — one-shot normalize stack
- Docs: `docs/PLAY_LINUX.md`, REA notes `docs/specs/2026-10-30-play-linux-rea.md`

Wine pitfall encoded in Studio: never pass Unix absolute paths to `wine explorer` (treated as switches); use basename with cwd = game dir or `Z:\...`.

### Studio (preferred UI)

- Native **Tauri 2** app under `studio/` (Bun + Vite + Rust).
- Build: `cd studio && ./build.sh` (sources `rust-env.sh`; Cursor pins old RUSTC — do not `source` that from fish).
- Release binary observed built at: `studio/src-tauri/target/release/open-max-payne-studio`
- Bundle targets limited to `deb` + `rpm` (AppImage dropped as bulky).
- Play tab: status (SafeDisc / Italian flags / Italian-safe OK), apply 1.05, Italian-safe stack, FixPack guide/run, launch, open folder.
- Archives tab: sample MPM via openras; extract under `easy-output/` (gitignored).
- **Removed:** legacy NiceGUI (`tools/openmaxfx_gui`), port `:8787` kill hooks, “Install SH mod data” button/`install_sh_mod` command/`runtime/install-sh-mod.sh`.

### Content / Author (partial)

- `tools/openras` — RAS/MPM CLI; **pytest 5 passed** last run (uses `vendor/fixtures/Basic Room Example Mod.mpm` only).
- `tools/openpfx` — particles draft.
- `tools/blender_io_kf2` — stub `__init__.py`.
- Format drafts: `docs/specs/ras_format.md`, `kf2_format.md`, `rps_format.md` (KF2/RPS still draft; RAS more REA-backed).

### Cleanup already done

- Deleted NiceGUI app + NiceGUI kill UX.
- Deleted stub `OPENMAXFX.md`.
- Deleted retail level RAS copies from `vendor/fixtures` (`x_level1/2.ras` ~186 MB); sample MPM kept.
- Emptied / stopped shipping widescreen overlay dir; bootstrap no longer mkdirs it.
- Stripped **all** Silent Hill naming/paths from tree (verify with ripgrep).
- Unlinked accidental `~/Games/MaxPayne1/data` → sibling SH `data_newest`.

### Explicit non-goals / bans

- Do not commit `vendor/`, `runtime/overlays/` binaries, game installs, retail assets (see `.gitignore`).
- Do not reintroduce third-party mod asset trees or SH install bridges into this repo.
- Do not ship Max Payne binaries.

---

## Current tree (source-facing)

```
openmaxfx                 # CLI / Studio launcher
runtime/                  # install-from-cd, bootstrap, italian-safe-stack
studio/                   # Tauri Studio
tools/openras|openpfx|blender_io_kf2
docs/PLAY_LINUX.md
docs/specs/…
vendor/                   # gitignored local: max-fx-tools, 3dsmax SDK, sample MPM
```

Git: **uninitialized history** (no first commit). Local Studio `target/` exists (gitignored).

---

## Known gaps / risks (author self-notes — challenge these)

1. Spec checkbox still open: Blender KF2 path “documented/stubbed”.
2. Win10 / macOS Play paths largely unproven in this arc (Linux Italian CD was the lab).
3. DSOAL download often skipped in bootstrap; audio stack incomplete.
4. FixPack still manual (Studio opens guide / runs installer); Italian-safe is the automated rescue.
5. `has_mod_data` / status copy still mentions loose `data/` — OK generically, but Play UX may over-imply mod workflow.
6. No CI yet; no tagged release; no signed packages.
7. openpfx / blender not production-ready despite pillar marketing.
8. Game dir may still hold FixPack leftovers (`data.disabled-italian`, ASI `.bak`, ThirteenAG `.bak`) — runtime concern, not repo.

---

## How to verify quickly

```bash
cd /home/gzelante/Lavoro/personale/open-max-payne
rg -n -i 'silent.?hill|silent-hill|openmaxfx_gui|install-sh-mod' || echo clean
cd tools/openras && PYTHONPATH=src python3 -m pytest -q
test -x ../../studio/src-tauri/target/release/open-max-payne-studio
# optional Play (needs install):
# bash runtime/italian-safe-stack.sh ~/Games/MaxPayne1
# ~/Games/MaxPayne1/launch-openmaxfx.sh
```

---

## Reviewer output format (please)

```text
## Verdict
(one paragraph)

## Findings
- path:line — severity — problem — suggested fix

## Agree / disagree with self-notes
…

## Recommended next 3 increments
1. …
```

Prior chat context (optional): Cursor transcript id `ab3e8837-cdc5-4e85-bf1b-d2ffc52ad2b6` under this workspace’s agent-transcripts.
