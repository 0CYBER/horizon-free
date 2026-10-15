# Agent control: driving the launcher and the game

AI agents and scripts control Horizon through four surfaces:

| Surface | Where | Transport |
| --- | --- | --- |
| Game control socket | `horizon-client --control` (`crates/horizon-client/src/control.rs`, commands in `src/game/control.rs`, `agent.rs` and friends) | text line in, one JSON line out; Unix socket, or localhost TCP + token where there is none |
| `horizon-ctl` | `crates/horizon-client/src/bin/horizon-ctl/` | CLI over the socket: one table of commands (`commands.rs`) gives help, argument checks and the MCP tools |
| `horizon-ctl mcp` | same | MCP over stdio, a tool per `horizon-ctl` command |
| Launcher CLI and MCP | `launcher/src/cli.rs`, `launcher/src/mcp.rs` | `horizon-launcher <cmd> [--json]`; `horizon-launcher mcp` (stdio) |

This page is the inventory of what each surface can do and the design every new command
follows. Quick start for players and scripts: [native-client.md](native-client.md#agent-control-horizon-ctl-and-mcp).

## What exists today

Legend: **yes** complete, **part** partial (see note), **no** missing, **—** not applicable.
"Socket" is the game's own command; `horizon-ctl` and its MCP offer the subset in its table
(`horizon-ctl help --game` lists everything the game takes, test hooks included).

### Launcher

| Capability | Window | CLI | MCP | Notes |
| --- | --- | --- | --- | --- |
| Releases, installed versions, select | yes | yes | yes | `versions`, `list_versions`; selecting happens on install only (no `select` command) |
| Install / remove a version | yes | yes | yes | |
| Prepare resources | yes | yes | yes | |
| Start the client | yes | yes (attached) | yes | MCP starts it detached and cannot reach it afterwards |
| Dedicated server: start, console, log, stop | yes | yes | yes | |
| Accounts (no secrets) | yes | yes | yes | names and UUIDs only; sign-in is a device code the player enters |
| Launcher settings (game folder, auto-install, opening animation, desktop entry) | yes | part | no | CLI has `desktop-entry`; the rest only in the window |
| Game menu style | yes | yes | yes | |
| Logs, crash report | yes | yes | yes | |
| Updates (Horizon and launcher) | yes | yes | yes | MCP only checks |
| Patch notes, links | yes | yes | yes | |
| Benchmark | — | yes | no | |
| Telemetry consent | PR #250 | PR #250 | PR #250 | opt-in, in review |
| Friends: join by invite code | yes | yes (`play --join`) | no | |
| Profiles, packs, mods | no | no | no | the launcher has none yet |
| The running game's controls | — | no | no | an agent needs a second MCP server (`horizon-ctl mcp`) |

### Game

| Capability | Socket | horizon-ctl / MCP | Notes |
| --- | --- | --- | --- |
| Session: launch, wait until ready, open/create a world, save, quit, logs | yes | yes | `launch`, `wait-ready`, `open`, `save`, `quit`, `logs` |
| Player and world state | yes | yes | `state`, `status` (renderer, streaming, memory) |
| Crosshair target | yes | yes | `looking-at` |
| Blocks: one, a box, nearest of a kind | yes | yes | `block`, `blocks`, `find-block` |
| Entities near the player | yes | yes | `entities [RADIUS] [TYPE]`; no single-entity detail |
| Inventory, armor, offhand, carried stack | yes | yes | `inventory` |
| Containers and stations: read, click slots | yes | yes | `screen`/`menu`, `click-slot ID`, `container X Y Z` |
| Crafting (grid, result, recipe book) | part | part | grid by `click-slot`; the recipe book only by `click-at` |
| Movement, look, path-finding | yes | yes | `move`, `goto`, `look`, `look-at`, `turn --over`, `mouse-move` |
| Act on the world | yes | yes | `break`, `place`, `interact`, `use-item`, `attack`, `select`, `drop` |
| Raw input: keys, mouse buttons | yes | yes | `key`, `mouse`, `release` |
| Controllers | yes | yes | `pad` button/stick/trigger/status (a virtual pad) |
| Chat and commands | yes | yes | `chat`, `command`, `type`, `messages`, `chat-log` |
| Screens: which is open | yes | yes | `screen` gives a label (`title`, `options`, `inventory`...) but not the options page or sub-screen |
| Screens: widgets | part | part | `screen` lists **labelled buttons** only (label, bounds, enabled); sliders, fields, list rows, tabs and switches are invisible |
| Screens: act on a widget | part | part | `click-button LABEL` (buttons only), `click-at X Y`; no slider set, no field focus, no row select by id |
| Options | part | no | `options PAGE` opens a page and `render-distance N` sets one value; no read, no set of the other ~120 options |
| Packs (resource packs) | no | no | only through the Packs screen by pixel |
| F3 debug data | part | part | `status` has the overlay flags and the renderer; not the F3 lines |
| Screenshots | yes | yes | PNG of the next frame (works `--offscreen`) |
| Recording video | no | no | |
| Benchmarks and frame stats | yes | yes | `bench`, `stats`, `fly`, `--benchmark` |
| Multiplayer: connect, disconnect, respawn | yes | part | `connect`/`disconnect` are socket-only |
| LAN and friends hosting | yes | no | `lan open/close/status`, `friends host/stop/status` socket-only |
| Cheats and test hooks | yes | part | `give`, `time`, `setblock`, `summon`, `effect`; `xp`, `enchant`, `potion`, `trade`, `fill`, `particle`, `sculk` socket-only |
| Waits | part | part | `wait TICKS`, `wait-ready`; no "until screen X", "until chunks loaded", "until chat says Y" |
| Events | no | no | everything is polled |
| Errors with reasons | part | part | `{"error": "text"}`; horizon-ctl adds `kind` (`usage`, `connection`) to its own errors only |

## Design

### One vocabulary, four surfaces

- **A command has one name**, kebab-case, the same on the socket, in `horizon-ctl` and in the
  launcher CLI. Its MCP tool is the name with `_` for `-` (`click-slot` → `click_slot`).
  The launcher's MCP offers the game's tools under a `game_` prefix (`game_click_slot`) so they
  never collide with its own (`logs` exists on both sides).
- **Arguments** are positional words then `--flags`; the `horizon-ctl` table (`Param`) is the
  source of truth and generates the MCP `inputSchema`. A new game command is not done until it
  is in that table.
- **Replies are one JSON object.** A query returns its data; an action returns
  `{"ok": true, ...}` plus what changed (`clicked`, `screen`, `drops`...).
- **Errors** are `{"error": "<sentence an agent can act on>", "kind": "<reason>"}`. Kinds:
  `usage` (bad or missing argument), `unknown` (no such command, widget, option, entity),
  `state` (the game is not in a state that allows it: no world, wrong screen, disabled
  widget), `busy` (another action or benchmark runs), `timeout`, `connection` (game not
  reachable), `denied` (needs consent, below). The message names what to do next
  ("no world open; use: open WORLD").
- **Shapes are shared**: a position is `[x, y, z]`; a block is
  `{"pos", "block", "state", "loaded"}`; an item stack is `{"item", "count", "damage"?,
  "components"?}`; a slot is a stack plus `"slot": ID` (the slot ids of
  `click-slot`); bounds are `[x, y, w, h]` in GUI pixels, the coordinates of `click-at`.

### Structured UI: the widget tree

`ui` returns every interactive element the last frame drew, on every screen (title menus,
pause, options pages, inventories and stations, chat, death, Horizon-style menus):

```json
{"screen": "options", "gui_size": [480, 270],
 "widgets": [
   {"id": "fov", "kind": "slider", "label": "FOV: 70", "bounds": [40, 50, 150, 20], "enabled": true, "value": 0.5},
   {"id": "video-settings", "kind": "button", "label": "Video Settings...", "bounds": [...], "enabled": true},
   {"id": "row-2", "kind": "row", "bounds": [...], "enabled": true},
   {"id": "done", "kind": "button", "label": "Done", "bounds": [...], "enabled": true}]}
```

- **Ids are stable and readable**: the label up to its `:` as a slug (`FOV: 70` → `fov`,
  `Video Settings...` → `video-settings`); a repeated slug gets `-2`, `-3` in draw order;
  unlabelled elements are `<kind>-<n>` in draw order. The id of a slider does not change
  when its value (and so its label) does.
- **Kinds**: `button`, `slider` (`value` 0..1 as drawn), `field` (text box), `row` (list
  entry: world, server, pack, language), `tab` (`selected`), `slot`.
- **Actions by id**: `click ID [right]` (move the pointer there and click, exactly as a
  player would), `set ID VALUE` (a slider to a 0..1 position, a click on its track, as vanilla
  `AbstractSliderButton.onClick` reads the pointer). `type TEXT` keeps typing into the focused
  field; `click` on a field focuses it.
- **No new code paths in screens**: the tree is built from what the draw functions already
  register for the controller's focus navigation (`widgets.rs`), so every screen that a
  controller can drive is in it, and clicks go through the same window events as the mouse.

### Options

- `options-get [KEY...]`: every option of `options.txt` by its vanilla key, with its value
  and what it accepts: `{"key": "fov", "value": 70, "type": "int", "min": 30, "max": 110}`,
  enums with `choices`, key bindings as `key.keyboard.w`. Horizon's own files
  (`horizon-graphics.txt`, `horizon-controller.txt`, extras) under their own keys.
- `options-set KEY VALUE`: validated like vanilla's `OptionInstance.set` (out of range is an
  error here, not a silent reset), applied at once as the options screen would apply it, and
  saved. Reply: the new value.
- `options [PAGE]` keeps opening a page for screenshots and for `ui`.

### Inventory, containers, world, entities

The existing commands stay and keep their shapes: `inventory`, `screen`, `click-slot`,
`block`, `blocks`, `find-block`, `entities`, `looking-at`. Planned additions: `entity ID`
(full detail: equipment, effects, passengers, AI state), `entities` filters by position and
radius around any point (`--at X Y Z`), and `blocks --of BLOCK` to keep a box query small.

### Waits

`wait-for CONDITION [--timeout S]` replies when the condition holds, or with a `timeout`
error saying what it last saw:

- `screen NAME` (the label `state` shows), `widget ID` (a widget is on screen),
- `loaded` (a world is open, the player exists and no chunk within the render distance is
  pending),
- `chat TEXT` (a new chat line containing TEXT),
- `ticks N` (as `wait`).

### Events

`subscribe [KIND...]` turns the connection into a stream: the reply is `{"ok": true,
"subscribed": [...]}`, then one JSON line per event until the client closes it:

```json
{"event": "screen", "tick": 1234, "from": "playing", "to": "inventory"}
{"event": "chat", "tick": 1240, "text": "<Steve> hi"}
{"event": "death", "tick": 1300, "message": "Steve fell from a high place"}
{"event": "pickup", "tick": 1310, "item": "minecraft:oak_log", "count": 1}
{"event": "health", "tick": 1311, "health": 18.0, "food": 20}
{"event": "dimension", "tick": 1400, "dimension": "minecraft:the_nether"}
{"event": "world", "tick": 0, "state": "loaded"}
```

Events are found by comparing state once per tick on the game thread (no hooks inside game
logic), so they cannot change what the game does. `horizon-ctl subscribe` prints the stream;
in MCP, where tools reply once, `events [--since TICK]` returns what was buffered (the last 256).

### The launcher's MCP proxies the game

An agent configures one MCP server, `horizon-launcher mcp`. `launch_client` starts the game
with `--control` on a socket in the launcher's data folder; while it runs, `tools/list` also
lists every `horizon-ctl` MCP tool as `game_<name>` and calls are forwarded over the socket
(the code is shared: the launcher depends on `horizon-client`'s control client). When the
game is not running, `game_*` calls fail with `kind: "state"` and say to call
`launch_client`.

### Safety

- **Local only.** The socket is a Unix socket in the user's runtime directory, or a TCP port
  on 127.0.0.1 behind a 128-bit token in a file only the user can read. Never `0.0.0.0`,
  never a network listener; the launcher's MCP is stdio.
- **No secrets.** No surface returns tokens, session files or keychain entries: accounts are
  names and UUIDs, `--session-file` is read once and deleted, and `options-get` leaves out
  nothing secret because `options.txt` holds none (server addresses are in `servers.dat`,
  which no command reads).
- **Consent for anything that sends data.** Commands that reach the network (`connect`,
  `chat`/`command` on a server, `lan open`, `friends host`, `install_version`,
  `check_updates`, telemetry) carry the MCP `openWorldHint` annotation and their description
  says "only when the player asks". Telemetry stays opt-in (PR #250); no agent command turns
  it on. Destructive ones (`remove_version`, `sign_out`, `fill`, `setblock`) carry
  `destructiveHint`.
- **Vanilla behaviour untouched.** Control code reads state and synthesizes the same input
  events a player produces; registering widgets for the tree draws nothing. Every PR that
  touches game files runs `tools/parity.py --changed origin/main`.

## Plan (most valuable first)

1. This document.
2. `ui`, `click ID`, `set ID VALUE`: the widget tree for every screen.
3. `options-get` / `options-set`.
4. `wait-for` and `subscribe` / `events`.
5. The launcher's MCP proxies the game (`game_*` tools), `launch_client` with `--control`.
6. Error `kind` on every game error; MCP annotations.
7. `entity ID`, `entities --at`, packs (`packs list|enable|disable`), F3 lines, launcher settings.

Each PR adds its tests (unit tests next to the code, and steps in
`tools/agent_control_e2e.py`, which drives the real game offscreen on macOS or on a virtual
screen on Linux) and updates the tables above.
