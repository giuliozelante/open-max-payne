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

## Open

- Dynamic: who enters `0x57cb70`.
- Exact type behind `+0x111` list (cast source is `P_BaseObject`).
- What `+0x119` gate means when non-null (LOD side vs flag).
- `KF_ObjectKeyframeAnimationCallback` bone sample path.
- Character / `SKD` load → this VA tree.
