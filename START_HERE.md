# Open Max Payne — start here

**Open Max Payne** = play Max Payne 1 (Linux lab) + MAX-FX archive tools.  
**OpenMaxFX Studio** is the graphical front door.

## Start the Studio

```bash
./openmaxfx
```

Release build: `cd studio && ./build.sh` (from fish — don’t `source rust-env.sh`).

---

## What to click

### Tab “1. Archives”

| Button | Does |
|--------|------|
| **List / Extract / Pack** | Uses native Rust `openras` |
| **Open output folder** | Opens `easy-output/` |

Sample `.mpm` is optional local-only under gitignored `vendor/fixtures/` (not shipped).

### Tab “2. Play on Linux”

1. **A) Install from CD**  
2. **B) Make it playable** — 1.05 EXE → **Apply localized-safe stack** (any non-English `x_<lang>.ras`)  

3. **C) Launch**

Details: [docs/PLAY_LINUX.md](docs/PLAY_LINUX.md)

### Tab “3. More”

- **Run safety tests** → `cargo test -p openras`  
- ParticleFX / Blender: **planned / draft** (not in this baseline)

---

## CLI

```bash
./openmaxfx help
./openmaxfx test
cargo run -p openras -- extract archive.mpm -d ./out
```
