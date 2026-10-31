# Spec: retail startup — intro, then startup_level

Goal:
  `maxfx-engine --window <game-dir>` follows retail boot after the options dialog:
  play `StartupVideoName`, then the main menu (`data\menu\menu.txt`). No SafeDisc.
  `startup_level` is what New Game starts, not the screen after the intro.

In:
  - Read `[startup] StartupVideoName` from `globalconfig.txt` (loose `data/` or `data.disabled-hires/`, else `x_data.ras`, else other `*.ras`).
  - Read `levels.txt` the same way. Use the block whose id is `startup_level` (`front_end/startup_level.ldb`).
  - Play `movies/intro.mpg` with `ffplay -autoexit` (Esc/q skips). Then show the main menu, not the level.
  - Observed on `MaxPayne.exe -nodialog` (2026-10-31): `movies\intro.mpg` opens first (37s, via `msvfw32`), then `data\menu\bitmaps\Menu_bg_Mainmenu.tga`. `startup_level.ldb` is only probed; New Game calls `gm_init(startup_level)`. Start Tutorial calls `gm_init(tutorial)`.
  - `./openmaxfx play3d` with no path uses the game directory so this sequence runs.
  - An explicit `.ldb` path still opens that file and does not play the intro.

Out:
  - SafeDisc, `cd.ini`, the startup options dialog.
  - An MPEG crate or an in-engine decoder.
  - Bitmap-perfect menu, graphic novel, load/save, options, credits.
  - Rebuilding the Studio bundle in this change (source default may move; the baked GUI still needs `studio/build.sh`).

Behavior:
  - Video file or `ffplay` missing: print the reason and still open the menu.
  - Start Tutorial loads `tutorial`. New Game → Normal loads `startup_level`. Quit does not load a level.
  - Closing the video continues into the menu.

Constraints:
  - No new Cargo dependencies.
  - No retail assets in git. Tests use a temp directory.

Done when:
  - `cargo test -p maxfx-engine resolve_startup --offline` passes (video path + `startup_level` bytes).
  - `cargo clippy -p maxfx-engine --offline -- -D warnings` is clean.
  - `cargo test -p maxfx-engine --offline` is green.
