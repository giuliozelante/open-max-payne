# Mode-switch dispatch — 2026-10-31

Image base `0x400000`. Exe `/home/gzelante/Games/MaxPayne1/MaxPayne.exe` (1.05, SHA256 `342d296cc8a8e430609ad09bb9f1d454655fb45c081df022d7993bd1fc7da46e`). File offset in `.text` equals RVA. `.data` VA = `0x865000 + (file - 0x465000)`. `.rdata` VA = `0x7ca000 + (file - 0x3ca000)`.

Question: how do the script strings `S_ModeSwitch`, `GM_Init`, and `MPGNM_PickUpNote` reach `MaxPayne_ModeSwitch::lookup` (`0x417790`), and what posts `graphicnovel` after the `startup_level` flyover?

## Lookup

`0x417790` is thiscall. `ecx` is the mode-switch object (saved to `ebx`). The argument is the mode-name `std::string` at `[esp+4]` on entry, then `[esp+0xc]` after `push ebx; push esi`, then `esi`.

```
417790: push ebx
417791: push esi
417792: mov  esi, [esp+0xc]
417796: push edi
417797: mov  edi, [0x7cac44]     ; string compare
41779d: push 0x8a66f0        ; "menu"
4177a2: push esi
4177a3: mov  ebx, ecx
4177a5: call edi
4177a7: add  esp, 8
4177aa: test al, al
4177ac: je   0x4177b7
4177ae: mov  eax, [ebx+0x7c] ; menu object
4177b1: pop edi / pop esi / pop ebx / ret 4
4177b7: compare 0x8a6708     ; "game"     → [ebx+0x80] at 0x4177c6
4177d2: compare 0x8a66c0     ; "graphicnovel" → [ebx+0x84] at 0x4177e1
4177ed: xor eax, eax / pops / ret 4
```

There is no `call 0x417790`. The function is vtable slot `+0x20` of `MaxPayne_ModeSwitch`. Vtable `0x7cbf0c`, COL `0x7dba18`, type descriptor `0x868c80`, RTTI `.?AVMaxPayne_ModeSwitch@@` at `0x868c88`. The vtable pointer is stored only by the constructors at `0x4158ae` and `0x4158ff`.

`0x6326e0` is `lea eax, [ecx+0x12]; ret`. The mode name the lookup compares is the `std::string` at offset `0x12` of the object passed in. MSVC layout: char* at object+4, so the pointer sits at the caller's object+`0x16`.

## Who actually calls slot +0x20 of this vtable

`.text` contains 139 `call dword ptr [reg+0x20]` with an 8-bit displacement and no SIB: `eax` 48, `edx` 45, `edi` 42, `ebx` 2, `ecx` 2. Four of them entered `0x417790` under the debugger (return address read from `[esp+0xc]` at the match site; `bt` is wrong without symbols):

| Call | Return | Function | What it does |
|---|---|---|---|
| `0x6020d5` | `0x6020d8` | `0x6020c0` | `call 0x6326e0` on the argument, then `call [ebx+0x20]` |
| `0x601d93` | `0x601d96` | `0x601b40` | same lookup, current mode already set |
| `0x601dad` | `0x601db0` | `0x601b40` | second lookup in that function |
| `0x601e04` | `0x601e07` | `0x601b40` | lookup when `0x724b90` (read `[this+0x19]`) returns 0 |

`0x6020c0` prologue:

```
6020c0: mov  edi, [esp+0x10]
6020c4: mov  esi, ecx
6020c6: mov  ebx, [esi]
6020c8: mov  ecx, edi
6020ca: call 0x6326e0
6020cf: push eax
6020d0: mov  ecx, esi
6020d2: call [ebx+0x20]
```

It is not reached by an `E8`/`E9`. The only immediate reference is the static initializer:

```
602560: push 0x601e60
602565: push 0x6020c0
60256a: mov  ecx, 0x8b1401
60256f: call 0x602b90
```

`0x602b90` pushes the string `X_Message` (`0x868d74`) and calls imports. It is an initializer, not a `{name, function}` table. A sibling `0x6025c0` does `push 0; push 0x602100; mov ecx, 0x8b13ff; call 0x6033a0`.

The other 135 `call [reg+0x20]` sites are other vtables. Two that look similar and are not lookup:

- `0x430444` (`call [edi+0x20]`). `edi` was loaded from `[esi]` before a call to `0x76f96a` with type `.?AVMaxPayne_GameModeMessage_ChangeControlledGameSpeed@@` (`0x86ae50`) and base `.?AVX_Message@@` (`0x867e70`). The same shape, `push 0x867e70` then `call 0x76f96a`, occurs 563 times. That is the generated “allocate this message and call the owner’s slot +32” family. The live break on `0x4177ae` / `0x4177c6` / `0x4177e1` did not return into it.
- `0x454653` (`call [ebx+0x20]`). `ebx` comes from the vtable of whatever `0x421380` returned, after `0x582770` on `0x86cce0`. Not the mode-switch vtable.
- `0x601311` is inside the frame tick `0x601250`. It calls slot +32 of the pending mode (`0x749780` then `call [edx+0x20]`), which is game `0x44bd00` or novel `0x49b3c0`, not lookup.

## Command names are vtable slot +8

There is no lowercase `s_modeswitch`, `gm_init`, or `mpgnm_pickupnote` in the exe. Each exact string has one code reference, and it is `mov eax, imm; ret`.

| Script token | String VA | Name thunk | Message vtable | COL | RTTI |
|---|---|---|---|---|---|
| `S_ModeSwitch` | `0x88a614` | `0x60e7b0` | `0x7d39a8` | `0x7f0b80` | `.?AVX_SystemMessage_ModeSwitch@@` `0x889128` |
| `GM_Init` | `0x86b89c` | `0x435160` | `0x7ccbdc` | `0x7ddce0` | `.?AVMaxPayne_GameModeMessage_Init@@` `0x86b878` |
| `MPGNM_PickUpNote` | `0x876294` | `0x499370` | `0x7ce8a0` | `0x7e4cb0` | `.?AVMaxPayne_GraphicNovelModeMessage_PickUpNote@@` `0x876260` |

Shared layout of those three message vtables:

| Slot | ModeSwitch | GM_Init | PickUpNote | Role |
|---|---|---|---|---|
| +0x00 | `0x60e5f0` | `0x435010` | `0x499200` | scalar destructor |
| +0x04 | `0x60e720` | `0x4350d0` | `0x4992c0` | copy |
| +0x08 | `0x60e7b0` | `0x435160` | `0x499370` | `mov eax, <command name>; ret` |
| +0x0c | `0x436e80` | `0x436e80` | `0x436e80` | `mov eax, 0x868d70; ret` (four bytes before the string `X_Message` at `0x868d74`) |
| +0x10 | `0x6111d0` | `0x499900` | `0x499900` | |
| +0x14 | `0x60e7c0` | `0x435170` | `0x499930` | copy the argument string into the message |

`0x60e7c0` calls slot +0x0c, calls `0x51ff20`, then `rep movs` into the `std::string` at `this+0x12` (the pointer field the constructor zeros is `this+0x16`). `0x751f70`, the same slot on the `DO_Animate` message, does the same copy into `this+0x31`. Neither function calls lookup.

`X_SystemMessage_ModeSwitch` constructor `0x60e610` stores vtable `0x7d39a8` at `[esi]` and zeros `[esi+0x16]`, `[esi+0x1a]`, `[esi+0x1e]`.

## Receivers

Each message has an `X_MessageReceiver<Owner, Message>`. The deliver method is vtable slot +0x14 and has one shape: resolve the owner singleton, pack two pointers, call a per-type function.

| Message | Receiver RTTI | Receiver vtable | Deliver | Singleton lookup | Then |
|---|---|---|---|---|---|
| ModeSwitch | `0x8894d8` | `0x7d3588` | `0x60b1b0` | `0x60d490` | `0x608350` |
| GM_Init | `0x86dff0` | `0x7cd234` | `0x47e380` | `0x480660` | `0x473560` |
| PickUpNote | `0x8772f0` | `0x7ceb08` | `0x4a7e10` | `0x4a7df0` | `0x4a3dd0` |

`0x60d490` is the ModeSwitch singleton lookup:

```
60d490: mov  ecx, 0x868c60          ; X_ModeSwitch type descriptor
60d495: call dword ptr [0x7ca34c]
60d49b: push eax
60d49c: mov  ecx, 0x8b9260
60d4a1: call 0x512080
60d4a6: mov  eax, [0x8b9265]
60d4ab: ret
```

`0x60b1b0`:

```
60b1b0: mov  eax, [esp+4]          ; message
60b1b7: push eax
60b1b8: call 0x60d490             ; X_ModeSwitch instance
60b1bd: push eax
60b1be: lea  ecx, [esp+8]
60b1c2: call 0x511bf0             ; store the two pointers
60b1cd: call 0x511c10
60b1d2: call 0x608350
60b1da: ret  4
```

`0x511bf0` writes the two dwords. `0x511c10` copies them. `0x608350` reads a byte from the table at `0x8b926d` and returns. It does not call `0x417790`. The lookup runs later, when the registered handler `0x6020c0` is invoked on the mode-switch object with the message whose name string is at +`0x12`. No `E8` to `0x60b1b0`, `0x47e380`, or `0x4a7e10` exists; they are vtable slots.

`GM_Init` and `MPGNM_PickUpNote` use the same receiver shape and do not call `0x417790`. They are delivered to `MaxPayne_GameMode` and `MaxPayne_GraphicNovelMode`. The mode change itself is only `S_ModeSwitch`.

## startup_level → comic

`Startup_level.ldb` (extract of `x_data.ras`, 21,994,472 bytes) has one occurrence of each call. They are consecutive statements in one script block, file offset `0x145f611`:

```
maxpayne_hudmode->MPHM_FadeToColor(0x0, 0x000000ff, 2);
::startroom::cam_rotator::cam_rotator::altitude::camera.DO->DO_Animate(5);
maxpayne_graphicnovelmode->mpgnm_pickupnote(P1L0a_001);
x_modeswitch->s_modeswitch(graphicnovel);
maxpayne_gamemode->gm_init(part0_level1);
player->c_setwoundedanimations(false);
::startroom::cam_rotator::...::camera::final_snow->PS_StopAllEmissions("");
```

`DO_Animate` string VA `0x8a30b4`. Its name thunk is `0x751f60` (`mov eax, 0x8a30b4; ret`), stored at vtable dword `0x7d9814`. Neighbor slots: `0x7d980c` = `0x751e10`, `0x7d9810` = `0x751ed0`, `0x7d9814` = `0x751f60` (name), `0x7d9818` = `0x436e80`, `0x7d981c` = `0x751700`, `0x7d9820` = `0x751f70` (argument-string copy). `0x751700` calls slot +0x0c and the import at `[0x7ca33c]`. It does not post `S_ModeSwitch`.

The live break matches the statement order, not a hard-coded camera callback. Boot to the main menu: two `menu` hits (`0x6020d8`, `0x601e07`). New Game → Fuggitivo: three `game` hits while the window showed the Manhattan flyover. About 40 seconds later, with no key pressed: `graphicnovel`, then `game`, then `graphicnovel`, then `game`, and the window was the house interior. `graphicnovel` did not hit during the flyover.

So the transition is the next statement in that block, `x_modeswitch->s_modeswitch(graphicnovel)`, delivered by `0x60b1b0` and resolved by `0x6020c0` → slot +0x20 → `0x417790`. The function that advances the script when `DO_Animate(5)` reports done was not located. `0x751f60` and `0x751f70` are the name and the argument copy. They do not post the mode switch.

## What this does not show

A file-open of `bg_intro.kf2` is not the mode switch. The ASCII window in the level is the script the interpreter runs. Searching xrefs of `S_ModeSwitch` finds `0x60e7b0` and stops there. Slot +0x18 of `0x7cbf0c` is `0x4179a0`, not lookup.
