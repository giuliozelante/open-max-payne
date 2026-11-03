# DO_Animate playback path (MaxPayne.exe)

Artifact: `/home/gzelante/Games/MaxPayne1/MaxPayne.exe`  
PE ImageBase `0x400000`. `.text` VMA `0x401000` / file `0x1000` (file_off ≈ VA − `0x400000`).

Evidence-only. No invented verbs.

## Message → apply

| Addr | Role |
|------|------|
| `0x6f4a20` | registers find `0x6f3bd0` + apply `0x6f3d20` via `0x6f60b0` |
| `0x6f3bd0` | find: `0x751380` → name at message+`0x31`; lookup `0x6f5d50` in `this+0x4b8`; miss throws `0x89c9d8` |
| `0x6f3d20` | apply (below) |
| `0x751f60` | name thunk: returns `0x8a30b4` = `DO_Animate` |

RTTI: object is `.?AVX_LevelRuntimeDynamicObject@@` (vtable `0x7d7ac0`, typeinfo `0x8683c0`).

## Apply `0x6f3d20` (bytes)

1. Map hit → node in `ebp-0x18`.
2. `[node+0x1c]` via `0x751da0` (`mov eax,[ecx+0x5]`) then `0x5ff100` — pose sample used for continuous-path check; warn string `0x89ca0c` = `Dynamic object %s: Animation "%s" to "%s" is not continuous path!\n` via `0x584490`.
3. Store `[node+0x1c]` → `this+0x483`.
4. Copy anim name into `this+0x48b` (CString helpers `0x661800` / `0x54a4e0` / …).
5. Zero `this+0x49b`, `+0x4a3`, `+0x4c8`.
6. `0x6e2c00` — **not** KF2 start (see next section).
7. Vcall `[vtable+0x1c]` = `0x6f2b10` with `msg+0x11` from `0x753ca0` — field archive IO pattern.

## `0x6e2c00` (ruled out as play-start)

- Iterates a pointer range; `__RTDynamicCast` (`0x76f96a`) each entry `X_Message` → `X_LaggedMessage` (typeinfos `0x867e70` / `0x86ae30`).
- Builds stack object with vtable `0x7d142c` = `.?AVX_TimeUpdate@@` via ctor `0x582df0` → set `0x582e60` (`+0x1c=1`, floats at `+0xc/+0x10/+0x14`).
- `0x5848c0` copies that TimeUpdate into `lagged+0x11` (`0x582e30`).
- Then `0x51b6a0` with string `X_MessageQueue::send`.

`0x749760` only returns byte `+0x1c`. `0x583100` only zeros `+0x18`.

## Per-tick reader of `this+0x483`

| Addr | Vt slot | Role |
|------|---------|------|
| `0x6f34c0` | `+0xb0` (`0x7d7b70`) | If `+0x483 == 0` → exit. Advance with `0x5e53d0`: add into `+0x49b` or `fsubr` when `+0x4a7≠0`; optional hold float `+0x4a3`. Then `0x751da0` → `0x5ff100(time)` → copy 0x30 into `+0x259`. |
| `0x6f3b70` | `+0xb8` | `[+0x49b] → [+0x4c8]` then `jmp 0x6e2520` (pause snapshot). |
| `0x6f3b90` | `+0xbc` | `[+0x4c8] → [+0x49b]` then `jmp 0x6e2680` (resume). |
| `0x6f40e0` | (imm at `0x6f4a8a` reg) | toggles `+0x4a7` when `+0x483` set (invert direction). |
| `0x6f4100` | (imm at `0x6f4aea`) | `fld` via `0x74d340` → `+0x4a3`. |

Fields: `+0x49b` = playhead; `+0x4a3` = hold/remaining; `+0x4a7` = reverse; `+0x4c8` = pause save.

So mesh/DO_Animate playback is **transform curve sampling** into the DO local matrix via `0x6f34c0`, not KF2 bones.
## What `this+0x483` points at

RTTI `.?AVX_LevelRuntimeDynamicObjectAnimation@@`, vtable `0x7d9800` (installed in ctor `0x751aa0`, size `0x39`).

- Map insert `0x6f2ef5`: `node+0x1c =` that animation object.
- Apply copies `node+0x1c` → DO `+0x483`.
- Getter `0x751da0` = `mov eax,[ecx+0x5]` → pointer passed as `this` to `0x5ff100`.
- Setter `0x751c40` writes `anim+0x5` and sets `anim+0x4 = 1`.
- Ctor `0x751aa0` ends by `0x751c40(arg)` where arg is the insert-time `push edi` from `0x6f2df0` (result of `vcall [obj+0x24]` after `0x40e470` = `[mesh+0x98]`).

`0x5ff100` is **not** a vtable slot (no `.rdata` absolute ref). Sibling lea-helpers: `0x5ff400` → `+0x8c`, `0x5ff410` → `+0xac`, `0x73af50` → `+0x9c`.

## Ruled out (false Gemini claims)

| Claim | Bytes |
|-------|-------|
| `0x6e2c00` starts KF2 play | No — LaggedMessage / TimeUpdate only |
| Tick is `0x6f32e0` | No — misaligned mid-function; no `+0x483` |
| Callee `0x53e4c0` is KF2 bone apply | No — list/node cleanup; `0x6f34c0` calls `0x5ff100` at `0x6f35af` |

## `anim+0x5` / `0x5ff100` this (bytes)

Chain from DO load (`0x6f2c00` / insert `0x6f2df0`):

1. DO `+0x46f` → mesh; RTTI `.?AVX_LevelDBDynamicMesh@@`, vt `0x7d2d14`.
2. `0x40e470` = `[mesh+0x98]`.
3. `mesh+0x98` filled by mesh vcall `+0x18` (`0x5ef530` → `0x5f0380`): alloc `0x28`, ctor `0x5ff970`, final vt `0x7d33bc` = `.?AV?$X_LevelDBObjectContainerBase@…VX_LevelDBDynamicMeshAnimation@@…`.
4. Insert `vcall [container+0x24]` = `0x5d89c0` → returns list node `+0x10` element pointer.
5. That element is `.?AVX_LevelDBDynamicMeshAnimation@@` (container element type; name at `0x888328`).
6. Animation ctor `0x751aa0` / `0x751c40` stores that pointer at `anim+0x5`.
7. Tick `0x751da0` → `0x5ff100(this = anim+0x5)`.

So DO_Animate samples an **`X_LevelDBDynamicMeshAnimation`** transform curve, not KF2 bones.

Vtable `0x7d3360` (COL `0x7ef650` → td `0x888320`). Installed at `0x5fef8e`. Slot0 `0x5ff420` sits in the same cluster as sample helpers `0x5ff100` / `0x5ff400` / `0x5ff410`.

## DMA `+0x84` duration (float) — not the Exit binder

`X_LevelDBDynamicMeshAnimation` and `X_LevelDBExit` both have a dword at `+0x84`. They are **different fields**.

### Mesh animation duration

- Ctor `0x5fef82` / `0x5fef88`: zeros `+0x84` and `+0x88`.
- Archive **read** slot `vt+0x4` = `0x5ff7f0` (first op): `lea edx,[edi+0x84]` → `call [0x7cab20]` (float IO). Then pose blocks `+0x4` / `+0x34`, then key arrays, then curves `+0xbc` / `+0xd8` via `0x7833c0`.
- Archive **write** / finalize slot0 `0x5ff420`: writes `+0x84` via `[0x7cab24]`, later stores derived scale into `+0x88` (`fst`/`fstp` at `0x5ff71a` / `0x5ff735`; clamp vs `0.0` @ `0x7cb8b0` and `1.0` @ `0x7cbdd0`).
- Getter `0x5ff0f0`: `fld dword [ecx+0x84]; ret`. Called from DO tick at `0x6f361f` / `0x6f3642`.
- Sampler `0x5ff100`: if `time <= 0` → return pose at `+0x4`; if `time >= [+0x84]` → return pose at `+0x34`; else `time / [+0x84]` then sample curves; uses `[+0x88]` as a multiply on an intermediate.

No level-script field **name** string is bound at these sites (generic float IO thunks only).

### Sampler layout (bytes in `0x5ff100`)

| Offset | Role (from use sites) |
|--------|------------------------|
| `+0x4` | start **4×3 matrix** (0x30 = 12 floats); returned when `time <= 0`; archive via `[0x7caaf0]`/`[0x7caaec]` family |
| `+0x34` | end **4×3 matrix** (0x30) |
| `+0x28` | = `+0x4+0x24` — start translation row (last 3 floats of start matrix); not a separate allocation |
| `+0x58` | = `+0x34+0x24` — end translation row |
| `+0x64` / `+0x74` | quats from `0x5ffcb0(matrix)` (trace/`fsqrt` path); slerp via `0x5ffe70` |
| `+0x84` | duration float |
| `+0x88` | scale applied to first curve sample |
| `+0xbc` | curve → float blend for **translation** lerp (then `× +0x88`) |
| `+0xd8` | curve → float `t` for **quat slerp** `0x5ffe70` (`qa=+0x64`, `qb=+0x74`, dest `0x8b13b0`) |

Curve archive `0x7833c0` (edx = curve): header immediates `3` and `0x71`, then `[curve]` / `[curve+0x4]` counts, then floats via `[0x7cab20]` into pointers at `curve+0x8`…. Sample `0x783450` writes `[curve]` floats to the out pointer (call sites pass a single dword — one channel in practice). Floor via `call [0x7ca364]`. Scratch `0x8b13b0` / `0x8b1380` (init `0x448a20`).

**VERIFIED (2026-11-02, objdump of `0x5ff100` / `0x783450` / `0x5ff6a6`–`0x5ff735`):**

- Curve sample `0x783450(out, u)`: `k = floor(u·[c+0x14])`, `f = (u − k·[c+0x18])·[c+0x14]`, per channel `out = d[k]·(1−f) + d[k+1]·f`. Uniform piecewise-linear; `+0x14` = segment count, `+0x18` = its reciprocal. No clamp.
- `+0x88` = `1/|end_t − start_t|` (fast inverse-sqrt of squared length at `0x5ff6b9`…); `0.0` when length is zero. So curve `+0xbc` is in **distance units**; `a = sample(+0xbc, u)·[+0x88]` is the translation lerp weight.
- `b = sample(+0xd8, u)` is passed **raw** as slerp `t` (no normalize).
- Engine `.ldb` parser reads the two curves in the same order as archive read `0x5ff7f0` (`+0xbc` first, `+0xd8` second); header ints match immediates `3` / `0x71`.

Mid-range path: normalize `time/+0x84` → sample both curves → slerp quats → lerp translation between `+0x28` and `+0x58` → rebuild rotation (quat path uses `fdivr` of `2.0` @ `0x7d3378`). Endpoints still return raw matrices at `+0x4` / `+0x34`.

Pose copy into DO is the 0x30-byte block into `DO+0x259` from tick.

### Exit binder (ruled out for DMA duration)

- Archive IO `0x5e9210` is `vt+0x4` of `.?AVX_LevelDBExit@@` (`0x7d29d0`) and `.?AVX_LevelDBImportExit@@` (`0x7d1fa4`) — **not** DMA.
- Exit ctor (`0x5e8f57`…): `+0x64`/`+0x68` = `-1`; zeros `+0x7c`/`+0x80`/`+0x84`. `+0x6c` is a string-like member (dtor path `lea esi,[ebx+0x6c]`).
- Binder `0x5e9390` (only direct caller `0x5ebd7d` inside `0x5ebd50`):
  - key `+0x64` → map via `0x65a2e0` → store ptr at `+0x7c`
  - key `+0x68` → same map family → store ptr at `+0x80`
  - key `+0x6c` → map via `0x7269e0` → store `[entry+0x10]` ptr at `+0x84` (pointer, not float)
- Loop `__RTDynamicCast`s each element to Exit typeinfo `0x875ed8`.

## Class hierarchy (RTTI)

`.?AVX_LevelRuntimeDynamicObject@@` COL `0x7fd808`, primary vt `0x7d7ac0`. Bases (pmd):

| Base | pmd |
|------|-----|
| `X_LevelRuntimeDynamicObject` | 0 |
| `X_LevelRuntimeFSM` | 0 |
| `X_Receiver3DSound` | 0 |
| `X_LevelRuntimeSphereObjectBase` | 30 |
| `X_LevelRuntimeObjectBase` | 30 |
| `X_ParticleSystemReceiver` | 336 |
| `X_ProjectileReceiver` | 367 |
| `X_ObjectChild<FSM,…>` | 372 |
| `X_CameraReceiver` | 1121 |

Standalone FSM vt `0x7d75c0` has `+0xb0 = 0x658c80` (not anim). DO **overrides** `+0xb0` → `0x6f34c0`.

Related message RTTI: `…Message_Animate`, `_InvertAnimation`, `_PauseAnimation`, `_StopAnimation`, `_ResumeAnimation`.

## Dynamic result (2026-11-02, winedbg, retail CD boot → New Game)

**VERIFIED (dynamic):** 90/90 hits on `0x6f34c0` return to `0x6e1a93`, i.e. the call is
`0x6e1a90: call [eax+0x1c]` inside `0x6e1a00`, with `[esp+4] = 0x26faac` and many distinct
`this` values (one per animating DO).

**VERIFIED (static, RTTI COL):** the vtable assumed "primary" above is wrong.

| vtable | COL | COL offset | Role |
|--------|-----|-----------|------|
| `0x7d7b54` | `0x7fd8a0` | **0** | **primary** DO vtable |
| `0x7d7ac0` | `0x7fd808` | 336 | `X_ParticleSystemReceiver` subobject vtable |

All "vt `+0xNN`" offsets in this note that were measured from `0x7d7ac0` are therefore off by
`0x94`. Primary slots: `+0x10 = 0x6f3420`, `+0x18 = 0x6e1a00` (per-frame update),
**`+0x1c = 0x6f34c0` (DO tick)**, `+0x24 = 0x6f3b70` (pause snapshot), `+0x28 = 0x6f3b90`
(resume). The `+0xb0` caller hunt below (Stats / LocalCharacterInput) chased the wrong slot.
It is kept only as history.

Run 2 (VERIFIED dynamic, return addresses):

- `0x6e1a00` is called from `0x494db2: call [eax+0x18]` inside `0x493f80`. That loop walks a
  DO list: `esi = [esi+0x178]` (next link), with the arg `&local` at `ebp-0x84`.
  `0x493f80` (pushes `"MaxPayne_Level"` / `"LevelDB"`) has a single direct caller,
  `0x45008e` in `0x44fda0`. Its role as the level update is a HYPOTHESIS based on those strings.
- `DO_Animate` apply `0x6f3d20` returns to `0x6fd043`, i.e. `0x6fd040: call [ebp+0x10]`
  (function-pointer message dispatch). It fired 60× during the new-game cutscene.

Production path: per-frame `0x6e1a00` (primary `+0x18`) → `call [vt+0x1c]` → tick
`0x6f34c0` → sampler `0x5ff100` → pose copy into `+0x259`, which `0x6e1a93` then reads
(`add esi,0x259`).

## Pose → world composition in `0x6e1a00` (2026-11-02)

Row-vector convention (`v' = v·R + t`; "A·B" applies A first). **VERIFIED (static):**

1. `L` = sampled pose `+0x259` (the tick output). `+0x289 = L`. `+0x325 = L·(+0x319)` (a point carried with the object).
2. If parent `P = [+0x189]` (set by `0x6e23b0`, found by name in the map at `0x8b5ba0`):
   `+0x386 = P.+0x289 · (+0x191)`, `+0x289 = L · +0x386`, `M = L · +0x386`.
   `+0x191` comes from `0x63bfb0(this.[+0x1c1], P.[+0x1c1])`; it is identity when there is no parent.
3. `M = M · (+0x2b9)`, then `0x6e46d0(M)` and the `+0x1e` sphere subobject updates (`0x64aee0`, `0x642c00`).

So `X(node) = L · X(parent) · R191`, with `world = X · B` (`B = +0x2b9`). Parents must update before children.

Load: `0x6e1267`… reads two `M_Matrix4x3Template` via `operator>>(R_MemoryFile&, M_Matrix4x3Template&)`
(import `0x7caaf0`) into `+0x259` and `+0x2b9` (offsets as seen from the `+0x1e` subobject: `+0x23b`/`+0x29b`).
The constructor sets `+0x259`/`+0x289`/`+0x2b9`/`+0x2e9` to identity. FSM pause/resume (`0x6e2520`/`0x6e2680`)
snapshot `+0x2b9` ↔ `+0x1f5`.

**VERIFIED (retail data, house `Part0_Level1.ldb`):** in the `.ldb` dynamic-mesh record, after the
parent name, xform1 = composed world `W0`, and xform2 = local `L0` relative to the parent. xform2
equals the first track's start pose and the engine's `local_start`. For roots, xform1 == xform2. Child check:
`painting_rotate_parent` `L0.p=[0,-0.25,0.03125]` · parent `painting_parent06` W0 = `[3.21875,-0.25,-1]`
= its xform1. So for these records `R191 = B = I`, and **`W(t) = L(t) · W_parent(t)`**.
HYPOTHESIS: `R191`/`B` are identity for all retail DOs (not yet checked level-wide).

## (Superseded) Who calls DO vslot `+0xb0` (`0x6f34c0`)

Absolute vt slot only at `0x7d7b70`. **No** direct `E8`/`E9` to `0x6f34c0`.  
`call [reg+0xb0]` — **VERIFIED exhaustive** for 8-bit and disp32 encodings in `.text` (**exactly 2 sites**):

| Site | Parent | Notes |
|------|--------|-------|
| `0x6207bd` | `0x6202d0` on `.?AVX_StatisticsMode@@` vt `0x7d4130+0x3c` | Behind debug gate: `[ptr+0xe0] & 1` then `Updates: BV %d…` (`0x88c474`). Object via `0x60dbe0` = `[this+0x48]`. |
| `0x50361c` | `0x503600` on `.?AVMaxPayne_LocalCharacterInput@@` vt `0x7cff9c+0x84` | `[this+0x1b5]` (`0x503100`) then vcall `+0xb0`. `+0x1b5` filled from map/lookup (`0x4afd7a`) or direct store (`0x5030f1`); **receiver RTTI not proven** as DO (HYPOTHESIS: shares `+0xb0` layout with DO/FSM). |

Same `0x503100` receiver also gets `+0xb8` at `0x503667`. Stats also `+0xb8`/`+0xbc` at `0x620646`/`0x62066d`.

**VERIFIED:** sole `.rdata` slot holding `0x6f34c0` is DO vt `+0xb0`.  
**VERIFIED:** `0x5ff100` (curve sample) has only four `E8` sites — all inside DO tick/apply (`0x6f35af`, `0x6f366f`, `0x6f3743`, `0x6f3d6c`). Ongoing playback **must** enter `0x6f34c0`.  
**VERIFIED:** Stats `+0xb0` is behind `test [ptr+0xe0],1` (debug/stats flag) — not a normal gameplay path.  
**STRONGLY SUPPORTED:** GameMode embeds `MaxPayne_PlayerInput` at `+0x258` (ctor/setup `0x44fcbe`→`0x4ce880`; vt `0x7cf974` installs at `0x4ce552`). LCI often stores `&GameMode+0x258` into `+0x1b5` (`0x4566dd`). `PlayerInput+0xb0 = 0x723bd0` is `ret 4` (no-op). So the LCI `+0xb0` site does **not** advance DO curves when bound to PlayerInput.

**UNKNOWN (ASAP blocker):** which production path enters `0x6f34c0` for level DOs. Candidates: LCI `+0x1b5` sometimes holds a real DO* (map store `0x4afd7a`); or an undiscovered indirect call. Needs dynamic break on `0x6f34c0`.

**RTTI:** DO typeinfo `0x8683c0` (name `.?AVX_LevelRuntimeDynamicObject@@`). Most imm loads of `0x8683c0` are `type_info::name` / hash (`0x7ca34c`→`0x512080`), **not** casts. Sole `.text` `push 0x8683c0` + `__RTDynamicCast` (`0x76f96a`): `0x69769e` from `.?AVX_LevelRuntimeObjectBase@@` (`0x86d380`) → then vcall `+0x2c` on `obj+0x1e` — **not** the anim tick.
Sibling: `+0xac` = `0x6e1a00` (`FSM/DO/Trigger`, pose `+0x259`, no tick). `+0xa4` = `0x6f3420` → `0x6e3d40` + list `+0x4bc`.

Note: KF `VertexAnimation` / LODs also have slots at `+0xb8` (different vts) — do not conflate with DO.
## KF2

See `docs/rea/2026-11-01-kf2-vertex-animation.md` — `KF_VertexAnimation` apply `0x57c860` (matrix×verts).  
`KF_ObjectKeyframeAnimationCallback` still list/dtor-heavy.

## Animation control messages (2026-11-02, static)

**Handler table (VERIFIED).** `0x6f4a20`… registers one `(handler, registrar)` pair per message. Each registrar
(`0x6f60b0` + n·`0x820`) references the message's RTTI type descriptor, which names it:

| Handler | Message class (`X_LevelRuntimeDynamicObjectMessage_…`) |
|---------|--------------------------------------------------------|
| `0x6f3d20` | `Animate` |
| `0x6f40e0` | `InvertAnimation` |
| `0x6f4100` | `PauseAnimation` |
| `0x6f4120` | `StopAnimation` |
| `0x6f4150` | `ResumeAnimation` |
| `0x6f4180` | `Hide` |
| `0x6f4300` | `EnableCollisions` |
| `0x6f4320` | `EnableContinuousUpdate` |
| `0x6f4340` | `SetHitpoints` |
| `0x6f4360` / `0x6f43d0` | `X_CameraMessage_AttachToDO` / `X_CameraMessage_AnimateInPlace` |

The vt `+0xb8`/`+0xbc` slots (`0x6f3b70`/`0x6f3b90`) are **not** these messages. They snapshot the playhead
into `+0x4c8` around FSM pause/resume (`0x6e2520`/`0x6e2680`).

**Handler bodies (VERIFIED).** All act only while a curve is active (`+0x483 != 0`), except Resume:

- **Invert** `0x6f40e0`: `+0x4a7 = !+0x4a7`.
- **Pause(t)** `0x6f4100`: hold `+0x4a3` = float arg 0 (`0x74d340`).
- **Stop** `0x6f4120`: `+0x487 = +0x483`, `+0x49f = +0x49b`, `+0x483 = 0`. The pose at `+0x259` is not touched.
- **Resume** `0x6f4150`: if `+0x487` is set, `+0x483 = +0x487`, `+0x49b = +0x49f`, `+0x4a3 = 0`, `+0x487 = 0`.

**Tick `0x6f34c0` (VERIFIED).** `0x7cb8b0` is `0.0f`; duration is DMA `+0x84` (`0x5ff0f0`).

1. `hold == 0`: playhead `+0x49b` ± dt (− when reversed).
2. `hold > 0`: `hold -= dt`. If it goes negative, the overshoot moves the playhead (same direction rule) and hold = 0.
3. `hold < 0`: no advance (`PauseAnimation(10000)` and negative values freeze).
4. Sample at the playhead → `+0x259`.
5. `duration <= playhead`: sample at `duration`, clear hold/playhead/curve/reverse, send runtime-anim list `+0x29`
   (`0x751d90`) lagged by the overshoot.
6. Else `playhead < 0`: sample at 0, clear the same fields, send list `+0x19` (`0x751d80`) lagged by `-playhead`.

Animate apply zeroes `+0x49b`/`+0x4a3`/`+0x4c8` and sends list `+0x9` (`0x751d70`, at `0x6f40aa`). It does **not**
clear reverse `+0x4a7`.

**Message lists (VERIFIED).** The runtime anim (`0x751aa0`, vt `0x7d9800`) holds three vectors. Its compile
method `0x751d10` fills them from the DMA: `+0x9` ← DMA `+0x8c` (`0x5ff400`), `+0x19` ← `+0x9c` (`0x73af50`),
`+0x29` ← `+0xac` (`0x5ff410`). The DMA reader `0x5ff420` reads duration `+0x84`, two transforms, then three tag-`0x1c`
string lists into `+0x8c`, `+0x9c`, `+0xac`. So in the `.ldb`, list 1 = **start**, list 2 = **reverse end**, list 3 =
**end**.

Data check (STRONGLY SUPPORTED, 21 retail levels, 5092 tracks): 1782 have a start list, 545 a reverse-end list,
3792 an end list. Bunker door `open`: start plays `door_slide` and enables the exits; end chains `DO_Animate(delay)`;
reverse end re-enables the trigger and disables the exits, which is what an inverted open (the door closing again)
needs.

Message use in the 25 retail `.ldb`: `DO_StopAnimation` 847, `DO_InvertAnimation` 368, `DO_EnableCollisions` 99,
`DO_PauseAnimation` 39 (args `1`, `0.2`, `10000`), `DO_ResumeAnimation` 37 (case-insensitive counts).

**Engine:** `scene.rs` `DoAnimState` + `SceneDynamicMesh::step_animation` follow the tick above. Start lists are
queued and dispatched on the next `step_dynamic_anims`, which stands in for the lagged send.

## Open (next climb)

- ~~Prove RTTI of `LocalCharacterInput+0x1b5` receiver~~ — moot: the DO tick caller is `0x6e1a00` via vt `+0x1c` (above).
- ~~Wire Pause/Resume/Stop/Invert messages~~ — done above.
- `DO_EnableCollisions(b)`: the bytes are read, the meaning is not (see below). `EnableContinuousUpdate` (`0x6f4320`) is unread.

  VERIFIED, static: `0x6f4300` takes the bool (`0x74e5b0` = message `+0x30`) and `0x6f43f0` stores it at DO `+0x4ae`.
  `0x6f4410` then rewrites three bytes from `+0x143` (base object), `+0x4ae` and `+0x4ad` (set at DO load, `0x6f2c9b`):

  | `+0x143` | `+0x4ae` | `+0x4ad` | `+0x181` | `+0x182` | `+0x183` |
  |:-:|:-:|:-:|:-:|:-:|:-:|
  | 0 | – | – | 0 | ff | 0 |
  | 1 | 1 | 1 | ff | 0 | ff |
  | 1 | 0 | 1 | 0 | 0 | ff |
  | 1 | 1 | 0 | ff | 0 | 0 |
  | 1 | 0 | 0 | 0 | ff | 0 |

  UNKNOWN: what `+0x181`/`+0x182`/`+0x183` gate. Writers at `0x6d8834`/`0x6d8850`/`0x6d896d` and a reader of `+0x183` at
  `0x6d97bd` are the next leads. HYPOTHESIS, not used in code: one byte per collision class (characters, none,
  projectiles). The engine leaves `DO_EnableCollisions` unhandled until this is read.
- Lag semantics: the end/start lists go through `X_LaggedMessage` with the overshoot as delay (`0x6e2c00`). Exact
  delivery frame is UNKNOWN.
- Curve tag `0x71`; runtime channel counts.
- Matrix IO thunk ↔ `M_Matrix4x3Template` at runtime.
- KF2: who vcalls VA `+0x48` — see KF2 note (static exhausted → dynamic).
