# One window for the whole session

**Problem.** `maxfx-engine` opens a new top-level window for every stage: an `ffplay` window for the intro
video, a minifb window for the main menu, another for each graphic-novel session, and a fresh winit or minifb
window per level. Every hand-off flashes the desktop and steals focus. Retail keeps one window from the
video to the credits.

**Goal.** One window per process. The intro video, the menu, the comic pages and every level draw into it.

## Design

`frontend::Screen` owns the window for the session:

- `Screen::Hard { front: WinitFrontend, gpu }`: the winit window plus, once created, the wgpu device and
  swapchain (`GpuRenderer`, `GpuSurface`). Chosen when `hardware_renderer()` is on.
- `Screen::Soft(minifb::Window)`: the software path, or the fallback when no adapter or surface exists.

`Screen` implements `Frontend` (same minifb key vocabulary) and adds:

- `pump()`: collects winit events. A no-op for minifb, which polls inside `update_with_buffer`.
- `show(&[u32])`: presents a 960 × 600 CPU frame. Hard mode uses the existing `GpuRenderer::present` with
  no 3D view, the frame as `background` and an all-clear HUD.
- `set_title(&str)`.
- `take_gpu(&scene)` / `put_gpu(..)`: the walkthrough borrows the device and swapchain. The renderer is
  rebuilt for the level's scene with the same device and queue (`GpuRenderer::for_scene`); the surface is
  kept. If the device cannot be created, `fall_back_to_software()` replaces the winit window with a minifb one
  once, as the walkthrough does today.

Callers take `&mut Screen`: `play_intro_video`, `menu_ui::run_main_menu`, `menu_ui::show_novel_session`,
`render3d::run_3d_walkthrough`. `main` opens the screen once per launch.

**Intro video.** `ffmpeg -re` decodes the file to raw BGRA at 960 × 600 on a pipe, and the frames go to
`Screen::show`. The sound plays through `ffplay -nodisp -autoexit`. Escape, Space or Enter stop both
processes. A missing `ffmpeg` is an error string, and the caller still opens the menu, as today.

## Not in scope

Resizing minifb, fullscreen, menu rendering on the GPU beyond the CPU-frame blit, and an in-process audio
mixer.

## Tests

- `Screen` is a thin adapter over windows; its logic is the frame conversion. `bgra_to_frame` is unit
  tested.
- `GpuRenderer::for_scene` keeps the device: a headless test builds a renderer, rebuilds it for a second
  scene, and renders with both.
- Manual: one launch shows the video, menu, comic and level in the same window (checked with a desktop
  screenshot of the window list).
