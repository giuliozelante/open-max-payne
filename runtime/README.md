# Max Payne 1 — modern runtime (Linux / Proton / Wine)

Phase 5 of OpenMaxFX: make the retail game playable and stable on modern OSes without rewriting the engine.

## Target stack

| Layer | Component | Role |
|-------|-----------|------|
| Compatibility | Proton / Wine | Run Win32 `MaxPayne.exe` on Linux/macOS |
| Graphics | `d3d8to9` + DXVK | D3D8 → D3D9 → Vulkan |
| Audio | DSOAL + OpenAL Soft | Restore DirectSound3D / EAX spatial audio |
| Timing | FPS limiter (~60) | Avoid physics / ShootDodge breakage at uncapped FPS |

Localized language packs (`x_french.ras`, `x_german.ras`, `x_italian.ras`, `x_spanish.ras`, … — same pattern as [darkje’s language guide](https://steamcommunity.com/sharedfiles/filedetails/?id=945245625)): use `localized-safe-stack.sh` (park FixPack hi-res + ASI; DXVK over ThirteenAG). Widescreen ASI is optional and often breaks Wine.

## Layout

```
runtime/
  env.sh                   # GHIDRA + local tool paths
  bootstrap-linux.sh       # download/wire wrappers into a game dir
  detect-localized-lang.sh # x_<lang>.ras (skip english/data/music/level*)
  localized-safe-stack.sh  # Wine-safe graphics for localized retail
  install-from-cd.sh
  overlays/                # d3d8to9, dxvk, dsoal (gitignored binaries)
```

## Prerequisites

1. A legal install of Max Payne 1 (from the retail CD CABs or Steam).
2. `wine` or Steam Proton already on the host.
3. Game data extracted (`x_data.ras`, `x_english.ras`, levels, etc.).

Game InstallShield CABs live on the Max Payne CD under `Disk1/data1.cab` + `data2.cab` (same unshield workflow used for MAX-FX tools).

## Relation to OpenRAS / Blender tools

- **OpenRAS** builds/extracts mods (`.mpm` / `.ras`).
- **This runtime** runs the game + those mods on modern Linux.
- They are complementary: tooling first (Phase 2–4), then automated bootstrap here.
