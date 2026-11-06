# World spheres are meshes (2026-11-04)

Goal: explain why the intro showed the Manhattan skyline behind the chopper, where retail has empty sky.

## Findings

- **VERIFIED (retail data):** `worldspheres.txt` names a `.kf2` mesh per sky. The format is in
  `docs/specs/kf2_format.md`. All 11 files parse with every index in range. 7 of them have a second mesh with
  an opacity bitmap, so one image wrapped around the view cannot be right.
- **VERIFIED (retail data):** the root matrix differs per file. `intro`'s turns the sky, as the catalog's comment
  says ("docks rotated 180deg"), and lowers it 9.45 units. Its roots have determinant +1.
- **STRONGLY SUPPORTED (measured):** the mesh is used as stored, with no mirror, about the eye. The sky pixels of
  retail frame `original/0042` correlate 0.88 with the baked sky and 0.51 with its mirror image, each searched
  over yaw, pitch and field of view. The best fit is yaw −3°, half field of view 35°. The engine's camera there
  has yaw −2.5°.
- **HYPOTHESIS:** the sphere follows the eye and takes no fog. The match above supports it for this shot only.
- **UNKNOWN:** how the runtime reads these files. Nothing here comes from the reader's code.

## Engine

`kf2::parse_world_sphere` reads the meshes and `render3d::bake_sky` paints them at load, in file order, onto a
2048 x 1024 latitude-longitude image, about 0.25 s per sky in a dev build. Both renderers keep their single sky pass and
sample that image through `sky_uv`. Directions the mesh does not cover stay black. In the intro the water hides them.

The old code took the first bitmap and stretched it around the horizon, which put `intro`'s skyline toward −x and
left +z, where retail has the glow behind the city, nearly empty.
