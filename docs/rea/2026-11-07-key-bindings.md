# Key bindings and the keys tips name

Tips such as `TIP_08` ("Puoi sparare premendo il tasto #SHOOT#") name the keys the player has bound. The engine used to
replace each token with a fixed string. This note records where retail gets the bindings, their defaults and the key
names. PE i386, ImageBase `0x400000`; VA = file string offset + `0x400000`.

## Binding table

- **VERIFIED:** the bindings live in a global array at `0x8a9168`: 32 slots of `0x29` bytes, up to `0x8a9688`. Each slot
  holds four buttons of 9 bytes (`slot + button * 9`, setter `0x4cdc00(button, type, value)`) and an analog source at
  `slot + 0x24` (setter `0x4cdc60(type)`).
- **VERIFIED:** a button's type is 0 for none, 1 for a keyboard scan code (DirectInput, so `0xc8` is Up), 2 for a mouse
  button (value 0 is the first button). Registry values in a retail install agree.
- **STRONGLY SUPPORTED:** analog types are 1 mouse left/right, 2 mouse up/down, 3 mouse wheel. Slot 12 (aim left/right)
  gets 1, slot 11 (aim up/down) 2 and slot 26 (next/previous weapon) 3.
- **VERIFIED:** a slot is an axis. Buttons 0 and 2 drive one direction, buttons 1 and 3 the other. Slot 0 defaults to
  W, S, Up, Down: W and Up run, S and Down walk back.

## Loading and defaults

- **VERIFIED:** `0x4cb3d0` reads `HKCU\Software\Remedy Entertainment\Max Payne\Input Settings`. When its
  `Input Settings Version` is below the value at `0x87bf64` it calls `0x4cb090`, which clears every slot and writes the
  defaults. Otherwise it reads `" %d Slider type"` and `" %d Button%d type"` / `value` for each slot (format at
  `0x87bf68`).
- **VERIFIED:** defaults from `0x4cb090` (slot, button: type value):

| Slot | Action | Buttons | Analog |
|---|---|---|---|
| 0 | run / walk back | key 0x11 W, key 0x1f S, key 0xc8 Up, key 0xd0 Down | |
| 1 | strafe left / right | key 0x1e A, key 0x20 D, key 0xcb Left, key 0xcd Right | |
| 2 | shoot | mouse 0 | |
| 3 | reload | key 0x13 R, —, key 0x52 Num 0 | |
| 4 | jump | key 0x39 Space, —, key 0x36 Right Shift | |
| 5 | crouch | key 0x2e C, —, key 0x9d Right Ctrl | |
| 6-10 | dodge left/right/forward/backward, dodge modifier | none | |
| 11 | aim up / down | none | mouse up/down |
| 12 | aim left / right | none | mouse left/right |
| 13 | use | key 0x12 E, —, key 0x1c Enter | |
| 14-24 | select weapon slot 0-10 | slots 14-19: keys 1-6 (0x02-0x07); 20-24 none | |
| 25 | best weapon | key 0x22 G | |
| 26 | next / previous weapon | key 0x1a `[`, key 0x1b `]` | mouse wheel |
| 27 | pause | key 0x19 P, —, key 0xc5 Pause | |
| 28 | painkiller | key 0x0f Tab, —, key 0x0e Backspace | |
| 29 | sniper zoom | key 0x10 Q, —, key 0x4f Num 1 | |
| 30 | slow motion | none | |
| 31 | bullet time | mouse 1, —, key 0x2a Shift | |

## Tokens in tips

- **VERIFIED:** `0x4cce80` replaces each token with the names of one slot and direction, through a getter per action
  that calls `0x4cdcf0(direction)` on the slot. The order of the calls, with the strings at `0x87bf70`-`0x87c1a0`,
  gives: `#RUN#` 0/0, `#WALKBACK#` 0/1, `#STRAFELEFT#` 1/0, `#STAFERIGHT#` (retail spelling) 1/1, `#SHOOT#` 2,
  `#RELOAD#` 3, `#JUMP#` 4, `#CROUCH#` 5, `#DODGELEFT#` 6, `#DODGERIGHT#` 7, `#DODGEFORWARD#` 8, `#DODGEBACKWARD#` 9,
  `#DODGEMODIFIER#` 10, `#AIMUP#` 11/0, `#AIMDOWN#` 11/1, `#AIMLEFT#` 12/0, `#AIMRIGHT#` 12/1, `#USE#` 13,
  `#BESTWEAPON#` 25, `#NEXTWEAPON#` 26/0, `#PREVIOUSWEAPON#` 26/1, `#SELECTWEAPONSLOTn#` 14 + n (`0x4cc750`),
  `#PAUSE#` 27, `#PAINKILLER#` 28, `#SNIPERZOOM#` 29, `#SLOWMOTION#` 30, `#BULLETTIME#` 31.
- **VERIFIED:** `0x4cdcf0(d)` writes the name of button `d`, then, when button `d + 2` is bound, the separator and its
  name, then the same for the analog source. The separator (`0x8a96d8`) comes from `data/menu/menu.txt`
  `ControlSeparatorString = GCID_SEPARATOR`, defined in the language's `database/stringdefines.h` (Italian `" o "`).
  The retail HD recording shows "MB 2 o Shift" for `#BULLETTIME#`, which matches slot 31.
- **HYPOTHESIS:** a slot with nothing bound prints `UnassignedText` (`GCID_UNASSIGNED`, `"<non assegnato>"`). Not read
  in `0x4cdcf0`.

## Key names

- **VERIFIED:** `data/input.txt` names some scan codes (`[key] ScanCode = 200; KeyName = "UP";`) and, under `[names]`,
  `MouseButtonName = "MB "` (the code appends the button number counted from 1), `MouseWheelName`, `MouseLRName`,
  `MouseUDName`.
- **VERIFIED:** other keys are named by Windows: `GetKeyNameTextA` (import `0x7ca478`) is called from `0x519b80`. That
  is why the recording shows "Shift", which `input.txt` does not list.
- **Engine choice:** the engine cannot ask Windows on every OS, so for codes `input.txt` lacks it uses the names an
  English (US) Windows layout gives. Those are OS data, not retail data, and on a non-US layout retail would show the
  layout's own names. Calling the OS for names is left for later.
- **VERIFIED (recording):** retail under Wine prints "space o Shift" for `#JUMP#` and "Tab o BackSpace" for
  `#PAINKILLER#`. Those are Wine's names for 0x39, 0x36 and 0x0e; Windows names them "Space", "Right Shift" and
  "Backspace". The engine prints the Windows names, so it differs from the Wine recording on those keys.

## Not covered here

The engine's game controls read this table. Keyboard aim (slots 11 and 12, unbound by default) still turns at
2.2 rad/s, a value with no retail source. Saving bindings, reading them on Windows (the registry) rather than from a
Wine prefix's `user.reg`, and the analog sources (mouse look and wheel stay as they were) are later work.
