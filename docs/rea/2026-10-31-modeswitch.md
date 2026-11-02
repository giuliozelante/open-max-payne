# MaxPayne_ModeSwitch — graphicnovel (2026-10-31)

Question: which function in `MaxPayne.exe` uses the mode name `graphicnovel`?

Target: `/home/gzelante/Games/MaxPayne1/MaxPayne.exe`, PE x86, image base `0x400000`, sha256 `342d296cc8a8e430609ad09bb9f1d454655fb45c081df022d7993bd1fc7da46e`.

Ghidra had the file open. `binary_overview`, `search_procedures`, `xrefs`, and `procedure_pseudo_code` timed out. The addresses below are immediate operands in `.text`, checked with `objdump`. No decompiler text.

## String

`graphicnovel` is one C string at file `0x468c2c`, VA `0x868c2c`, in `.data`, next to the other mode names:

| VA | Text | std::string global filled at startup |
|---|---|---|
| `0x868c08` | `game` | `0x8a6708` by `0x415150` |
| `0x868c10` | `menu` | `0x8a66f0` by `0x415230` |
| `0x868c18` | `console` | `0x8a66e0` by `0x415310` |
| `0x868c20` | `statistics` | `0x8a66d0` by `0x4153f0` |
| `0x868c2c` | `graphicnovel` | `0x8a66c0` by `0x4154d0` |
| `0x868c3c` | `HUD` | `0x8a66b0` by `0x4155d0` |

`gm_init`, `s_modeswitch`, and `mpgnm_pickupnote` are not in this exe. Those spellings live in the level script. This exe stores the mode name the script selects.

## Class

RTTI `.?AVMaxPayne_ModeSwitch@@` at `0x868c88`. Type descriptor `0x868c80`. Complete object locator `0x7dba18` points at that descriptor. Vtable `0x7cbf0c` (locator at `[-1]`). Constructors store that vtable at `0x4158ac` and `0x4158fd`.

## The lookup

`0x417790` (vtable slot +32). thiscall. Compares the argument, through the function pointer at `0x7cac44`, against three strings and returns the mode object:

| Match | Return |
|---|---|
| `menu` (`0x8a66f0`) | `this+0x7c` |
| `game` (`0x8a6708`) | `this+0x80` |
| `graphicnovel` (`0x8a66c0`) | `this+0x84` |
| anything else | 0 |

`0x417800` (vtable slot +44) is the same compare with all six names: HUD `this+0x78`, console `this+0x70`, menu `this+0x7c`, game `this+0x80`, statistics `this+0x74`, graphicnovel `this+0x84`.

`0x417a80` (vtable slot +40) maps an index to the name string: 0 game, 1 menu, 2 graphicnovel.

No direct `call 0x417790` in `.text`. Callers go through the vtable.

## Ruled out

The copies at `0x4154ee` and `0x44c640` are `strlen`/`memcpy` of the literal into a string. They are not the mode switch. `0x4171d0` only pushes the log text `Graphic novel mode`.

The exact strings `GM_Init` (`0x86b89c`), `MPGNM_PickUpNote` (`0x876294`), and `S_ModeSwitch` (`0x88a614`) each have one `.text` reference. Those are `mov eax, imm; ret` at `0x435160`, `0x499370`, and `0x60e7b0`. They return the RTTI type name. They are not the script dispatcher.

Uses of the mode-name `std::string` globals at `0x7851a8` (`game`), `0x7851c8` (`menu`), and `0x785228` (`graphicnovel`) are `mov ecx, imm; jmp 0x41c3a0`, packed between `int3`s. That is exception-unwind cleanup, not a call into `0x417790`.

`0x7cbf0c` is stored only by the two constructors, at `0x4158ae` and `0x4158ff`. A search for `call dword ptr [reg+0x20]` is not a caller list: `.text` has dozens, and the two `call [ecx+0x20]` sites (`0x517e0c`, `0x5194aa`) release the object at `this+5`.

## Dynamic

One run under winedbg (`--gdb`, port 23456). Breaks on the three match instructions, then `continue`. At a match the return address is `[esp+0xc]` (the prologue is `push ebx; push esi; push edi`, and the compare cleans its own two arguments). gdb's own backtrace is wrong here: the exe has no symbols, and it reported `0x51b86b` for a hit whose return address was `0x6020d8`.

| When the window showed | Match | Return address | Call |
|---|---|---|---|
| Main menu, after the intro | `menu` | `0x6020d8` | `0x6020d5` `call [ebx+0x20]` |
| Main menu | `menu` | `0x601e07` | `0x601e04` `call [edi+0x20]` |
| Manhattan flyover (water, Statue of Liberty, then the towers) | `game` | `0x6020d8`, `0x601d96`, `0x601db0` | same sites as below |
| Later, no key pressed; window then showed the house interior | `graphicnovel`, then `game`, then `graphicnovel`, then `game` | the same three | `0x6020d5`, `0x601d93`, `0x601dad` |

`graphicnovel` did not match during the flyover. The first matches after New Game → Fuggitivo were `game`, and the skyline was on screen. `graphicnovel` matched through the same call instructions, afterwards.

`0x6020c0` takes the request object in `ecx`'s caller argument. `0x6326e0` is `lea eax, [ecx+0x12]; ret`, so the mode name is the `std::string` at offset `0x12` of that object. The lookup result is a mode object. `0x749790` is `mov [ecx+0x1d], eax`. It stores that pointer. It does not enter the mode.

`0x601b40` is the request. `0x724b90` reads the current mode from `this+0x19`. If that is already the looked-up object, `0x601b40` returns at `0x601e49`. Otherwise it stores the new object at `this+0x1d` and calls `0x601af0`, which sets `this+0xd = 3`, `this+0x11 = 0`, and `this+0x15` to the float returned by the current mode's vtable slot +36. The frame function is `0x601250`. While `this+0xd == 3` it advances `this+0x11` through `0x601b10` and, once that ratio reaches the `1.0` at `0x7cbdd0`, copies the pending mode to `this+0x51` and sets `this+0xd = 2`. `0x6014d0` sees a non-null `this+0x51`, clears it, calls the current mode's vtable slot +12, passes the new mode to this object's vtable slot +72, clears `this+0x1d`, and calls the new mode's vtable slot +80. The store into `this+0x19` is `0x601710`. Its callers are `0x577fb3`, `0x57842b`, `0x578583`, `0x578851`, `0x578ab3`, and `0x5790ea`.

New Game is a request for `game`, and that object stays at `this+0x19` while the flyover is on screen. `graphicnovel` is a later request through the same function. The comic runs when that transition finishes, not when the menu entry is chosen.

## Chain

The frame that advances this is `0x40c2a2`. It passes a delta to `0x601250`. On the following check it calls `0x6014d0`. The log string immediately after that call is `Modeswitch sound update` (`0x867a78`, pushed at `0x40c30d`). `0x867a70` is the string `Windows`.

`0x6014d0` installs the parked mode by calling this object's vtable slot +72. On `MaxPayne_ModeSwitch` that slot is `0x601710` (`mov [ecx+0x19], eax`). The six direct calls at `0x577fb3`, `0x57842b`, `0x578583`, `0x578851`, `0x578ab3`, and `0x5790ea` are other stores into the same field. The install the breakpoint run hit is the vtable slot.

Complete-object vtables (RTTI offset 0):

| Class | Vtable | Enter, slot +80 |
|---|---|---|
| `MaxPayne_GameMode` | `0x7ccec4` | `0x44be20` → `0x451960` |
| `MaxPayne_GraphicNovelMode` | `0x7ce920` | `0x49af10` |
| `MaxPayne_MenuMode` | `0x7cc500` | `0x723bd0` (`ret 4`) |

`0x451960` reads a small integer from `0x40e4e0` and jumps through `0x451e3c`. The second arm (`0x451a8b`) calls `0x40ff40` with the id at `this+0xb8`, or `0x40fff0` with the string at `this+0xbc`, on the object at `this+0xb0`. That is the level load. The flyover is `startup_level` loaded here, while `this+0x19` is still the game mode. `0x44bd20` (game slot +36) is the duration the request reads: the float `0.05` at `0x7cc248` when `this+0xef` is set, otherwise the float from the object at `this+0x1234` via `0x5e53d0`.

`0x49af10` is the comic. It walks the pointer array at `this+0x104`, keeps the index at `this+0xf4`, and calls `0x4a8b50` on that page. Slot +32 (`0x49b3c0`) and slot +36 (`0x49b3d0`) are `fld` of `this+0xbf` and `this+0xc3`. Slot +76 is `0x723bd0` on all three mode classes (`ret 4`).

`0x602560` registers `0x6020c0` during startup (`push 0x601e60; push 0x6020c0; mov ecx, 0x8b1401; call 0x602b90`). Nothing in `.text` calls `0x6020c0` directly. The breakpoint is what shows it runs.

The camera line in the script is the command name `DO_Animate` at `0x8a30b4`, in the name list after `.?AVX_LevelRuntimeDynamicObjectMessage@@` (`0x8a3088`). The only code reference is `mov eax, 0x8a30b4; ret` at `0x751f62`, the same shape as the `GM_Init` RTTI thunk. It is not the function that moves the camera. That function is still unnamed. The class vtable for the message base is `0x7da40c`.
