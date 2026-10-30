# Play on Linux

## Why “Launch” did nothing

REA on retail `MaxPayne.exe`: entry point sits in PE section **`stxt371`** (SafeDisc stub). Wine cannot run that wrapper on current kernels. See `docs/specs/2026-10-30-play-linux-rea.md`.

## Do this instead (evidence-based)

1. Keep the install at `~/Games/MaxPayne1` (already extracted).
2. **Apply official 1.05** — Rockstar’s current `maxpayne1-05patch.exe` is often a **drop-in `MaxPayne.exe`** (no SafeDisc). Studio’s button copies it over `MaxPayne.exe` (backup → `.safedisc.bak`). Running that file in Wine *is* starting the game; don’t do that. (Wise/InstallShield SFX installers must not be copied — Inc 3 will refuse them.)
3. Install **[Max Payne 1 Complete FixPack 2025](https://steamcommunity.com/sharedfiles/filedetails/?id=1633394421)** — use the **notex** build if you want a small download.  
   For retail CD: pick **CD executable** (not Steam), keep **Startup fix** + **d3d8to9** (defaults).
4. Launch:

```bash
~/Games/MaxPayne1/launch-openmaxfx.sh
```

Log: `~/Games/MaxPayne1/openmaxfx-launch.log`

## FixPack under Wine

```bash
cd ~/Games/MaxPayne1
# download FixPack notex from the Steam guide links, then:
WINEPREFIX=$PWD/.wine wine explorer /desktop=FixPack,1280x720 ./MP1-Fixpack*.exe
```

Point the installer at your real game folder via the Wine `Z:\` mapping (e.g. `Z:\home\<user>\Games\MaxPayne1`), not a vague `Z:\Max Payne`.

### Localized language packs — one command (preferred)

Language RAS files follow `x_<language>.ras` (english, french, german, italian, spanish, polish, portuguese, russian, …). Non-language archives: `x_data.ras`, `x_music.ras`, `x_level*.ras`.

```bash
bash runtime/localized-safe-stack.sh ~/Games/MaxPayne1
# or Studio → Play → Apply localized-safe stack
```

For **any non-English** pack: parks FixPack hi-res `data/` and ASI `dinput.dll`, installs **d3d8to9 + DXVK x32**, regenerates `launch-openmaxfx.sh`. (Same crash class first seen on Italian; applied generally.)

Manual checklist (same result):

1. Rename `data/` → `data.disabled-hires` (FixPack hi-res often breaks localized CDs under Wine)
2. Keep Rockstar **1.05** as `MaxPayne.exe` (no SafeDisc)
3. Use **d3d8to9 + DXVK `d3d9.dll`** — not ThirteenAG’s small wrapper
4. Park ASI: `mv dinput.dll dinput.dll.asi-loader.bak`
