# MaxED / ParticleFX tutorial facts for the engine

Source: the official Remedy MaxED Tutorial v1.01HH-R1 and ParticleFX tutorial, plus saved 3D Realms forum
threads, converted to Markdown in the owner's local modding archive (not redistributed here). Remedy wrote these
for modders, so they state intent, not code. A claim taken only from them is **STRONGLY SUPPORTED** at best;
forum posts are **HYPOTHESIS**. Each entry names the engine or REA note it confirms or opens.

## Materials and blending

- **Alpha test** (MaxED "Alpha test"): texels below the category `AlphaReference` are cut; texels from the
  reference to 255 "are semi-transparent as usual", and the polygons still sort with the Z-buffer. Confirms
  the `BlendedAlphaTest` blend (`2026-11-02-material-texture-link.md`). Alpha materials without alpha test
  are depth-sorted, slow and prone to popping. The engine treats every masked material as alpha-tested.
- **Additive / multiplicative** ("Alternative blending methods") are per-polygon qualities (MaxED keys
  `A` / `M` / `S`), not material flags. Additive only lightens (black transparent); multiplicative only darkens
  (white transparent). They usually carry no lightmap. Open: decode the polygon flag in `.ldb` faces.
- **Dualsided** is a material flag that disables back-face culling; the lightmap still follows the polygon
  normal.
- **Light layer** is editor-only: MaxED bakes it into the lightmaps before export. Nothing to do at runtime.

## Lighting

- Point lights and spotlights light **characters and DOs only, never static geometry**. They are not blocked
  by geometry, only by room boundaries (a falloff that reaches an exit lights the next room). Matches the
  first golden capture (`n = 0` on lightmapped draws, `2026-11-02-lighting-layers.md`).
- Remedy shipped **no spotlights** in Max Payne. Emissive polygons export cone lights only with the DO flag
  "Export lights".
- `levels.txt` `AmbientColor` is the ambient character light. VERIFIED by golden data
  (`2026-11-02-lighting-layers.md`, "Level ambient"); implemented in `character_vertex_light`.
- **Particle lights** (`Light_*` effects in `particles.txt`, or any emitter with "Use Light") are Gouraud
  point lights that move with the emitter and light the environment too. Their radius comes from the
  emitter's Light Falloff Graph and their colour from the Light Color Graph, over the emission cycle;
  "Brightness" does nothing. Matches the second golden capture (a shot adds one light to level geometry).
  Open: read the light graphs from RPS with `openpfx` and spawn engine lights from `PS_StartEffect`.
- DOs without lightmaps are lit by dynamic lights. "Lightmapped" DOs ignore them unless "Pointlights affect"
  is set.

## Dynamic objects and entities

- DO flags: Export geometry, Export lights, Cast No Shadows, Insert into collision BSP (static only), Block
  explosions, Bullet collisions, Dynamic collisions, Lightmapped, Pointlights affect, **Cont. upd.**
  ("animate all the time even when it's not in the view cone"). Cont. upd. is the `+0x4af` flag that
  `CAM_AttachToDO` warns about (`2026-11-02-camera-attach-to-do.md`).
- Entities: Waypoint, Jumppoint (spawn; initial one named in `levels.txt`), Enemy, Level item, Player /
  Character / Projectile collide triggers (spheres with a radius), Action button trigger, Look-at trigger,
  Floating FSM (invisible relay, also a sound and particle emitter), Pointlight (static).
- Exits can be disabled by message ("Exit disabling"); FSM states can carry custom strings.

## Camera verbs (forum quote of the message reference, HYPOTHESIS)

- `CAM_AnimateAbsolute(block, room)`: absolute camera path from `data/database/camerapaths.txt` in a room.
- `CAM_AnimateInPlace(block)`: path relative to the current player camera (shakes and similar effects).
- `CAM_AnimateParented(block)`: path parented to the character that triggers it (death cams in the
  tutorial's `OnDeath` scripts).
- `CAM_AnimatePlayerParented(block)`: parented to the player. FadeIn / FadeOut only work here.

These are the open `CAM_AnimateInPlace` (184 uses) and `CAM_AnimateParented` (64) items in the camera
note. `camerapaths.txt` and `CAM_AnimateInPlace` are now in `2026-11-02-camera-attach-to-do.md`.

## Particles

- Particles are camera-facing billboards, additive or alpha blended (`BlendingMode = alpha | additive` in
  the bitmap set file). Colour graphs multiply additive bitmaps.
- Effects are exported to `.rps` and registered in `data/database/particles/particles.txt`; levels start
  them with `PS_StartEffect`.
