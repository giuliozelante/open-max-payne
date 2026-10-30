# Play on Linux — REA + FixPack (not guesswork)

## Evidence (REA on installed `MaxPayne.exe`)

Target: `~/Games/MaxPayne1/MaxPayne.exe`  
SHA-256: `73173d6dc5abc1f782837931a1ad1556eed38caf0a704af0c6e595afdf767ca5`  
PE32 i386. Language pack on disk: `x_italian.ras` (Italian retail CD layout).

| Observation | Detail |
|-------------|--------|
| PE sections | `.text`, `.rdata`, `.data`, `.rsrc`, **`stxt774`**, **`stxt371`** |
| Entry point | RVA `0x4ce05e` → VA **`0x8ce05e`** inside **`stxt371`**, not `.text` |
| Ghidra | Single procedure named `entry` at that VA (protector stub) |
| Resources | RT_DIALOG ids 101/102/133/137 present (startup/config UI lives in the image once the stub runs) |
| Imports | `dinput`, `rlmfc`, `e2mfc`, `grphmfc`, `sndmfc`, … — real game code is behind the stub |

**Conclusion:** this CD executable is **SafeDisc-wrapped**. On modern Linux/Wine, `secdrv`-style drivers are gone. Symptom matches what we saw: process may appear briefly, **no usable window**, Wine flag thrashing does not unwrap DRM.

Official **1.05 patch** and/or **community fixed executables** (FixPack “CD exe”) replace or bypass that stub. That is the correct next step — not more Proton/EGL experiments.

## Community path (required reading)

[Max Payne 1, Complete FixPack - 2025 Edition](https://steamcommunity.com/sharedfiles/filedetails/?id=1633394421) (darkje)

For **this Italian CD install**, the guide itself says:

- High-res textures path: select **CD executable**, not Steam EXE  
- Steam EXE + Italian → store-page redirect; keep CD exe  
- Defaults that matter on modern machines:
  1. **Startup fix** (UCyborg) — fast CPUs never finish init / look like a no-op launch  
  2. **d3d8to9** (or dgVoodoo) — DX8 vsync/FPS on Win10+/Wine  
  3. Optional: DSOAL, Widescreen (ThirteenAG), JPEG/CPU fix on `rlmfc.dll`

Prefer the **notex** FixPack (~13 MB) first if you do not want the huge texture pack.

## Open Max Payne policy

1. **Install from CD** (unshield) → files only.  
2. Apply **1.05** and/or **FixPack (CD exe + startup fix + d3d8to9)**.  
3. Then our `launch-openmaxfx.sh` (Wine + virtual desktop + DXVK/d3d8to9 overlays).  
4. Further REA: decompile post-unwrap / patched EXE; document entry after FixPack for regression.

Do **not** treat random Wine env vars as substitutes for removing SafeDisc / applying the startup fix.
