# Model converter and Blender add-on

**Status.** Proposed 2026-11-07; the owner agreed to the direction.

**Problem.** Modders need to move models and animation between the game and current 3D software. The original path
was 3ds Max 3 and 4 plugins. `tools/blender_io_kf2` is a stub, and the engine can read KF2, SKD and KFS but nothing can
write them.

**Goal.** A model goes from retail files into Blender, changes there, and comes back into the game.

## Design

The format work lives once, in Rust: a new crate `openkf2`, with a CLI, converting KF2/SKD/KFS to and from glTF 2.0.
glTF is the common format of Blender, Maya, 3ds Max, Cinema 4D and Houdini, so all of them are covered with no plugin.

The Blender add-on calls that CLI and adds what glTF cannot hold, as custom properties: clip messages, bone names the
game looks up, material categories. No plugin for paid software unless modders ask; 3ds Max would come first.

## Steps

1. Move the KF2, SKD and KFS readers from `maxfx-engine` into `openkf2`; the engine depends on it.
2. Export mesh, skeleton, skin weights and textures to glTF. Check with a retail character in Blender.
3. Export animation clips.
4. Add writers: KF2, SKD, KFS. A retail file read and written back must be equal or load the same.
5. Import from glTF, and pack with `openras` into a `.mpm` the engine loads.
6. Blender add-on, then a Studio tab.

## Verification

Round-trip tests on synthetic files in CI, retail-gated tests locally, and one character edited in Blender and seen in the
engine.

## Risks

- The writer needs fields the reader skips today. `docs/specs/kf2_format.md` must be finished first.
- Retail files may hold data with unknown meaning, which a writer has to preserve.
- KF2 vertex animation and `DO_Animate` curves are separate paths and stay separate.
