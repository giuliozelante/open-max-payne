# Level FSM: states, commands, `FSM_Switch`, `FSM_Send` (MaxPayne.exe)

Retail use across the 25 main levels: `FSM_Switch` 2435, `FSM_Send` 2092. Before this note the engine ignored
both and ran only the stateless calls of an event script, so every per-state section went unused. The house
toilet lid and shower `T_Activate` scripts, for example, live entirely in `[open]`/`[closed]` and `[on]`/`[off]`
sections.

## Record layout (VERIFIED, static, `X_LevelDBFSM` serializer `0x5fad40`)

The serializer is shared by `X_LevelDBFSM` and `X_LevelDBImportFSM` (vtable slot 1). After the base object
(`0x5e8280`):

| Field | Record | Runtime FSM | Engine (`scene.rs`) |
|-------|--------|-------------|---------------------|
| states (`1c` list) | `+0x98` | | `states` |
| default state | `+0xa4` | | `default_state` |
| startup script | `+0xb4` | `+0x401` (`0x6e3fb0`) | `startup_actions` + first map + `_extra` |
| state scripts (`1f` map) | `+0xe4` | `+0x441` (`0x5faf70`) | `state_scripts` |
| commands (`1f` map) | `+0xf4` | `+0x431` (`0x5faf60`) | `commands` |
| events (`1f` map) | `+0x104` | `+0x451` (`0x5faf80`) | last map, in `hooks` |

A script (`0x5fb6e0`) is a call list, a per-state map of call lists, and a second call list. That is why the
startup list is followed by a map and another list in the engine parser: the three are one script. Map
setup is `0x6e3d40` → `0x6e3bb0`/`0x6e42d0`.

## Messages (VERIFIED, static)

Handlers come from the receiver allocators: `X_MessageReceiverAllocator<X_LevelRuntimeFSM, …_Send>`
slot 1 = `0x6ed390`, built by `0x6e76a0` in registrar `0x6e7210` (handler `0x6e2f10`);
`…_Switch` slot 1 = `0x6edf80`, built by `0x6e7ec0` in registrar `0x6e7a30` (handler `0x6e30a0`).

- **`FSM_Send(command)`** `0x6e2f10`: current state (`0x6e1670` = `&this+0x365`), command name (`0x751380`),
  lookup in `+0x431` (`0x6e37c0`; miss prints `FSM "%s"' does not have command "%s"`), run (`0x6e2cd0`).
- **`FSM_Switch(state)`** `0x6e30a0`: increments `+0x382` and warns above 32
  (`FSM "%s"' is recursively switching to state "%s" (this causes an infinite loop)`); reads the **old**
  state (`0x6e31a0`) before copying the new name into `+0x365`; looks the new state up in `+0x441`
  (`0x6e3910`; miss prints `… does not have state "%s"`) and runs it with the old state; decrements `+0x382`.
- Both pass a delay from the message (`0x753ca0` → `0x5e53d0`) and send through `0x6e2c00` (lagged).

STRONGLY SUPPORTED: the per-state section a script runs is the state passed alongside it. So a state script's
section names the state the FSM came **from** (`t_toilet`: `closed: [open] …`), while commands and events use
the current state (`d_fan01` `rotate`: `[1st] DO_Animate(1st); FSM_Switch(2nd)`).

## `FSM_Randomize` (VERIFIED, static, `0x6e3310`)

Registrar `0x6e8250` (allocator built at `0x6e86e0`). The handler takes the state count (`0x6e3770`: size of
record `+0x98`), picks `rand() % count` (MSVCRT `rand`, IAT `0x7ca368`), fetches that name (`0x6e3790`),
lowercases it (`tolower`, IAT `0x7ca328`) and continues exactly like `FSM_Switch` (same `+0x382` guard and warning
`0x89b294`). 23 retail uses.

The engine uses the MSVCRT generator (`seed·214013 + 2531011`, bits 16..30) from the unseeded start value 1.
UNKNOWN: the retail seed. `MaxPayne.exe` does not import `srand`, but `rlmfc.dll` does, and both share the
MSVCRT generator state. The engine's sequence is deterministic, retail's may not be.

## Engine

`SceneFsm::state` starts at `default_state`. `FSM_Switch` and `FSM_Send` follow the handlers above. Event
scripts (`dispatch_hook`) now run the current state's section after the stateless calls. Nested FSM scripts
stop at depth 32.

HYPOTHESIS: the order inside a script is first list, state section, second list. The engine merges the two
lists and runs the state section last. Delivery is immediate; retail lags it by the message delay.

## Open

- Startup scripts still run only the `DO_SetHitpoints` / `DO_Hide` / `T_Enable` calls; `FSM_Send` at startup
  (ceiling fans: `FSM_Send(rotate)`) is not dispatched yet.
- Script executor `0x6e2cd0` / `0x6dfb30` internals (call order, how the delay applies).
