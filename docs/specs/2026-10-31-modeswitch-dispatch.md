# Mode-switch command layout

Goal: record the binary layout of the three script commands that the startup block uses, so a later interpreter can name them from the vtable instead of from a string window.

In: the message vtable, the receiver deliver slot, and the mode-name offset consumed by `MaxPayne_ModeSwitch::lookup`.
Out: the FSM stepper that runs the next statement after `DO_Animate` returns. That function is not located. The camera keys are not in the decoded scene (the parser stops before the camera object).

The engine plays a handoff level by evaluating `altitude::camera.DO` tracks 1..5 (parent rig composed with the distance curve), then the comic page, then `gm_init`'s level.

## Command struct

Each script command is an `X_Message` subclass. The command name is not a map entry. Vtable slot +8 returns it.

| Offset | Field |
|---|---|
| +0x00 | vtable |
| +0x08 (slot) | `mov eax, <command string>; ret` |
| +0x0c (slot) | returns `0x868d70` (`X_Message`) |
| +0x14 (slot) | copies the parenthesized argument into the `std::string` at object+0x12 |
| +0x12 | `std::string` the lookup compares. MSVC: char* at +0x16, length at +0x1a |

Resolved instances:

| Command | Name thunk | Argument copy | Receiver deliver |
|---|---|---|---|
| `S_ModeSwitch` | `0x60e7b0` | `0x60e7c0` | `0x60b1b0` |
| `GM_Init` | `0x435160` | `0x435170` | `0x47e380` |
| `MPGNM_PickUpNote` | `0x499370` | `0x499930` | `0x4a7e10` |

`S_ModeSwitch` is the only one of the three whose receiver ends at `lookup` (`0x417790`), and it does so through the registered handler `0x6020c0` (`call [vtable+0x20]`), not through a direct call. `GM_Init` and `MPGNM_PickUpNote` go to their own mode objects.

## Behavior the bytes support

`startup_level` reaches the comic because one script block lists, in order, `DO_Animate(5)`, `mpgnm_pickupnote(P1L0a_001)`, `s_modeswitch(graphicnovel)`, `gm_init(part0_level1)`. The live lookup hits follow that order: `game` during the flyover, `graphicnovel` only after it.

Done when: this file and `docs/rea/2026-10-31-modeswitch-dispatch.md` name the four proven `call [reg+0x20]` sites, the three name thunks, and the script order. They must not claim a located animation-completion function.
