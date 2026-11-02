# KF2 vertex animation (MaxPayne.exe)

Artifact: `/home/gzelante/Games/MaxPayne1/MaxPayne.exe`  
Evidence-only. Separate from DO_Animate mesh-curve path (`docs/rea/2026-11-01-do-animate-playback.md`).

## Class

`.?AVKF_VertexAnimation@KF2@@` — vtable `0x7d11b8` (secondary vt at `+0x10` = `0x7d11a8`).  
Name thunk `0x57ba70` returns `0x884e7c` = `KF_VertexAnimation`.

Ctor installs those vts at `0x57aa43` / `0x57aa49`. Slot0 `0x57a9d0` = deleting dtor.  
Factory call into ctor `0x57a9f0` from `0x56169b` (inside `KF_VertexAnimationLODs` build at eh-prologue `0x561543`).

## Related KF2 vtables

| Class | Vt | Notes |
|-------|-----|--------|
| `KF_VirtualObject` | `0x7d0d90` | Base |
| `KF_ObjectAnimation` | `0x7d0e94` | Driven object at VA `+0x115` |
| `KF_VertexAnimationLODs` | `0x7d1074` | Owns VA children; `+0x48 = 0x422180` (returns `1`) |
| `KF_VertexAnimation` | `0x7d11b8` | Apply at `+0x48` |
| `KF_ObjectAnimationLODs` | `0x7d1334` | Parallel LOD; `+0x48` also stub `0x422180` |

## Apply / morph

| Addr | Vt slot | Role (bytes) |
|------|---------|--------------|
| `0x57cb70` | VA `+0x48` | Gate `0x57c850` = `[this+0x119]`; drives `+0x115` via vcalls `+0x64`/`+0x10`/`+0x30`; on success `E8` → `0x57c860` |
| `0x57c860` | (callee) | Morph/skin: walks `+0x131`…`+0x135` (stride 24 via `0x2aaaaaab`/`sar 3`); **3×vec3 × 3×3** at `0x57c954`–`0x57c9a4`; matrix at `edi+0x5c` after `+0xe0` flag |
| `0x57d4d0` | `+0x70` | → `0x57baa0` then vcall `+0x70` |
| `0x57de60` | `+0x24` | If `[obj+0x17d]`, vcall `+0x24` on `0x57ba90()` |

`0x57cb70`: **no** `E8` callers; sole imm32 ref = vt slot `0x7d1200`.  
`0x57c860`: sole `E8` from `0x57cc38` (inside apply).

## Object at `+0x115` / `+0x111` / `+0x119`

| Addr | Role (bytes) |
|------|----------------|
| `0x57c3a0` | Alloc `0x19b`, ctor `0x54c1c0` → store at `+0x115` (callers `0x57b0ab`, `0x57b2d1`) |
| `0x54c1c0` | vt `0x7d0e94` = `KF_ObjectAnimation` (after base `0x54b720`) |
| `0x57c850` | return `[this+0x119]` (apply gate) |
| `0x57baa0` | return `*([this+0x111])` |
| `0x57ba90` | return `[this+0x115]` |
| `0x57ba80` | vcall `+0x5c` on `+0x115` |

`KF_ObjectAnimation` slots used here: `+0x5c = 0x54fbc0` → `[this+0x111]` → `0x56cbe0` (ptr-vector walk → `0x5584e0`); `+0x64 = 0x54dc20` = `([this+0x119])[index]`.

## LOD container path

`KF_VertexAnimationLODs` getter `0x562440`:

- `[this+0x111]` as list base
- `__RTDynamicCast` args: from `.?AVP_BaseObject@@` (`0x884700`) → `.?AVKF_VertexAnimation@KF2@@` (`0x884c00`)

LOD forwards after get: **`+0x5c` / `+0x64` / `+0x70` / … — never `+0x48`**.  
LOD `+0x5c` (`0x561d90`) loops children and vcalls child `+0x5c` (≠ apply).

## Caller found (2026-11-02) — in `e2mfc.dll`, not `MaxPayne.exe`

**VERIFIED (dynamic + /proc maps + DLL exports):** after a save loads, `0x57cb70` is entered with
`[esp]` = `e2mfc.dll` + `0x1ce8f`. `e2mfc.dll` is relocated at runtime (`0x8d0000` in one run,
`0x9d0000` in the next; preferred base `0x10000000`). That is why the exe-only static search found no
caller, and why the address looked like it was "outside every module".

| e2mfc VA | Export (by ordinal) | Role |
|----------|---------------------|------|
| `0x1001cba0` | `P_BaseObject::executeHierarchy()` (protected) | Recursive scene walk; `0x1001ce8c: call [eax+0x48]` on each object, then children via `getFirstChild` `0x10040f60` and siblings via `[obj+0xec]` |
| `0x10018ec0` | `P_Scene::render(P_Camera*)` | Calls `executeHierarchy` at `0x1001906f` (return `+0x19074`) |

Chain: `P_Scene::render` → `P_BaseObject::executeHierarchy` (recursive, `0x1001d10f`) →
`call [vt+0x48]` → `KF_VertexAnimation` apply `0x57cb70` → morph `0x57c860`. A `bool` is returned and
tested at `0x1001ce8f` (`test al,al`).

**STRONGLY SUPPORTED:** `P_BaseObject` slot `+0x48` = `executeAlways()`, `+0x4c` = `executeVisible()`.
Both exports fold to the same default body `0x10035710`. `executeHierarchy` calls `+0x48`
before the culling helper `0x1001c900` and calls `+0x4c` only after it (`0x1001d04b`). So KF vertex
animation is applied in the scene graph's per-render "execute always" pass, not by a KF2-specific driver.

Tooling: the earlier crashes were caused by the debugger script (`pass` forwarded `0x80000003`), not by the game.
Use `cont`; `bt` is broken in this winedbg (`Unknown thread id`), so dump `x/32x $esp` instead.

## Dynamic attempt (2026-11-02) — INCONCLUSIVE (superseded above)

- The VA vtable `0x7d11b8` is a true primary (COL `0x7eb7c0`, offset 0), so the slot `+0x48` is correct
  (unlike the DO note, whose "primary" was a secondary vtable).
- Not hit at all during boot, the menu or the new-game cutscene (runs 1–2).
- Attaching `winedbg` to the running game in the house: `0x57cb70` was hit with `this = 0x68016b8`,
  `[esp] = 0x8ece8f`. That address is outside every PE section (image ends at `0x8ca56e`) and outside
  every DLL in the load log. The same stop then repeated with a page fault at `0xffbb11b0`, and the game
  crashed. Treat the return address as **UNKNOWN / unreliable**. Possibly the attach broke
  the stack, or the caller is generated code.
- Run from launch (2026-11-02): first hit right after a save loaded ("Riprendi Partita"), with
  `[esp] = 0x9ece8f`, exactly `0x100000` away from the earlier `0x8ece8f`. No game-folder DLL has
  a call returning to `…ce8f`. HYPOTHESIS: `[esp]` at entry is a heap value, not a return address,
  so `0x57cb70` is reached by push+jmp or a tail jump, not a `call [reg+0x48]`. The process then died
  because the script used `pass`, which forwards the debugger's own `0x80000003` breakpoint trap to the
  game (a tooling error, not a game bug). Use `cont` on breakpoint stops.
- Next try: dump `x/32x $esp` at each stop and pick the stack slots in `.text` (`0x401000`–`0x7c903c`)
  that follow a call instruction.
- Earlier plan: launch under `winedbg` from the start (that method was stable for the DO runs) in a virtual
  desktop, and break on `0x57c860` (morph) as well as `0x57cb70`.

## Who vcalls VA `+0x48`?

Static rung:

- 44× `call [reg+0x48]` in `.text`. None in `0x57xxxx`.
- Several that also test `[obj+0x125]` (e.g. `0x64d899`, `0x69570d`, `0x507be9`, `0x70a6b4`) then **cmp return to a char/id** from `0x512080` — different class family than KF apply (which returns `bool` in `al`).
- LOD child API never selects slot `0x48`.

**Gap:** production caller of `0x57cb70` not identified on static. Next rung = dynamic (break on `0x57cb70` under Wine) or REA `procedure_callers` once Ghidra answers (previously timed out).

## Related strings

- SKD mismatch `0x885019`… — **zero** imm32 code refs.
- `BoneMatrices@P_VirtualObject` at `0x862678`.
- Debug format strings `VertexAnimation %s…` — no imm32 code refs.

## `KF_ObjectAnimation` methods under load (2026-11-02, winedbg)

`KF_ObjectAnimation` (vtable `0x7d0e94`) derives from e2mfc `P_BaseObject`; slots 2–8, 12–15, 17, 19–22 are import
thunks (`addTo`, `setPosition`, `isInsideViewVolume`, `executeVisible`, …).

**VERIFIED, static:** its `executeAlways` (slot 18, `0x551020`) does no sampling. If `+0x17e` or `+0x17d` is set,
it calls `isInsideViewVolume` (slot `+0x30`) and sets or clears bit `0x10` of `+0xe0`, then returns false.

**Capture 1 (load and opening cutscene only; the player had no control yet).** House save, breakpoints on slot
16 and slots 23–39 (18 methods), 4000 stops logging `$eip` and the return address:

| Method | Slot | Hits | What it is (static) | Callers |
|--------|-----:|-----:|---------------------|---------|
| `0x54fdf0` | 28 | 1700 | `hasAnimation(id)`: `std::map` lookup at `+0x169`, returns found ≠ end | slots 29/31, `0x550de9`, LODs `0x57fdf3` |
| `0x54e270` | 31 | 821 | getter after the lookup | `0x54e2b1`, `0x54fe61`, LODs `0x57f4bf` |
| `0x54e240` | 29 | 626 | getter | `0x54e2e0`, LODs `0x57f43f` |
| `0x54e2a0` | 30 | 432 | getter | LODs `0x57f47f` |
| `0x54dc00` | 24 | 422 | child count of the `+0x119` vector | `0x550e2e`, `0x550ea1` |

The `0x550cxx`–`0x550exx` caller is a load routine (RTTI cast from `0x884680`, stream reads through
`0x7caa9c`/`0x7ca9a4`), entered 211 times, once per animation object. The rest are lookups driven by
`KF_ObjectAnimationLODs` (`0x57f4xx`).

**Capture 2 (player walking; 484 stops).** winedbg attached to the running game (`attach 0x138`) with
breakpoints on every non-thunk slot (0, 1, 9–11, 16, 23–39). The attach thread faulted in Wine's wow64 layer
(`EIP ffbb11b0`) and the game crashed after 484 stops, so this is a short sample:

| Method | Slot | Hits | What it is (static) | Callers |
|--------|-----:|-----:|---------------------|---------|
| `0x54dc20` | 25 | 198 | `getChild(i)`: `[+0x119][i]` | VA apply `0x57c91b`, `0x57ca3a`; `0x57245f` in `0x5723d0` |
| `0x54bb80` | 1 | ~180 | returns `+0xe8` | e2mfc (`0x9ec6cb`, `0x9ec6ee`, `0x9ec783`, `0x9ecea3`, `0x9ddfd9`), D3D driver (`0x1d75f3c`, `0x1d83620`) |
| `0x54fdf0` | 28 | 31 | `hasAnimation(id)` | getters |
| `0x551070` | 9 | 8 | `+0x17f` float when `+0x17d` is set | e2mfc `0x9ec29f` |

STRONGLY SUPPORTED: while Max walks, the object animation's own methods are only accessors. The per-frame work
reaches its children through the vertex-animation apply (`0x57cb70` → `0x57c860`, which calls `getChild`) and
through `0x5723d0`. The only static caller of `0x5723d0` is `0x54e7ca`, inside slot 35 (`0x54e790`). Slot 35's
entry was not hit in this sample, so it may be reached another way.

**`0x5723d0` is a pose snapshot (VERIFIED, static).** Arguments: an object animation and an optional filter.
`this+0x5`/`+0x9` is a vector of 48-byte entries (count = byte size / 48, `0x2aaaaaab` multiply). For each index
*i* it fetches child *i* of the object animation (vslot `+0x64` = slot 25, `getChild`). If the filter
(`0x570e90`) rejects that child, it clears bit *i* of the bitmask at `this+0x19`. Otherwise it copies the child's
48 bytes at `+0x2c` (a 3×4 `M_Matrix4x3Template`, 12 floats) into entry *i* and sets bit *i*.

STRONGLY SUPPORTED: KF2 bones are e2mfc scene nodes (`P_BaseObject` children of the object animation) whose local
transform sits at `+0x2c`. The exe does not sample keyframes itself. It snapshots node matrices into a per-bone
48-byte array (for skinning or blending). The keyframe interpolation that moves the nodes lives in e2mfc's
code that writes the node matrices. **Correction:** e2mfc exports no keyframe or controller symbols (1200
exports checked), and the exe imports no rotation or matrix setter for `P_BaseObject`. So the keyframe sampler is
in the exe and writes `+0x2c` directly. Next: a watchpoint on one bone's `+0x2c` under a launch-time capture.

## Skin matrices in the VA apply (VERIFIED, static, `0x57c860`)

The exe's call sites of `P_BaseObject::calculateObjectToWorldMatrix` (IAT `0x7ca4ac`) include `0x57c92d` and
`0x57ca4b`, the two per-frame `getChild` callers in capture 2. The loop, per bone *i*:

1. Copy 48 bytes from `this+0x121`[*i*] into the output entry `this+0x131`[*i*] (both vectors of 48-byte
   `M_Matrix4x3`).
2. Bone node = child *i* of the object animation (`0x57baa0`, then vslot `+0x64`). If bit 5 of `node+0xe0` is
   clear, call `calculateObjectToWorldMatrix` on it.
3. `out = out × node.objectToWorld`: 3×3 rows times `node+0x5c`, then translation `out.t × R + node+0x5c+0x24`.
4. The same with the root: child 0 of a second animation (`0x57ba90`). After its own world update, the matrix at
   `root+0x8c` (world-to-object) is applied.

STRONGLY SUPPORTED: skin matrix *i* = `bind⁻¹ᵢ × boneWorldᵢ × rootWorld⁻¹` (row-vector order). Vertices therefore
end up in the root's object space.

**Engine comparison:** `kf2::skin_vertices` uses `poseᵢ × inv(bindᵢ)` with the bind taken from `Pose.kf2` at t = 0,
composed in model space. That matches retail when the root sits at identity. Retail reads `bind⁻¹` from a stored
array (`+0x121`), not from `Pose.kf2`. UNKNOWN: where `+0x121` is filled. It is reset in `0x57b890`, and the init
chain `0x57ab60` → `0x57ad20` (→ `0x57b320`, `0x57b530`) is the next read. **Capture tooling:** attaching with winedbg crashes the game here; launch under winedbg and gate
the breakpoints on a later event instead.

## Open

- ~~Dynamic: who enters `0x57cb70`.~~ Answered above (e2mfc `executeHierarchy`).
- Exact type behind `+0x111` list (cast source is `P_BaseObject`).
- What `+0x119` gate means when non-null (LOD side vs flag).
- `KF_ObjectKeyframeAnimationCallback` bone sample path: bones are e2mfc nodes (`+0x2c` matrix); the sampler is in e2mfc. Next: a watchpoint on one bone's `+0x2c`.
- Character / `SKD` load → this VA tree.
