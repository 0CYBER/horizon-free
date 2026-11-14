# Server services and player handles

Use `ServerHost` services to run commands, send messages and find connected
players. Native and wasm mods use the same calls.

## Services

| Accessor | Operations |
|---|---|
| `host.commands()` | `run_command(line)` runs through the console command path with permission level 4. `feedback(text)` answers the command source. |
| `host.messages()` | `broadcast(text)` fires the broadcast hook and writes to the server log. `text_allowed(text)` checks the server chat filter. |
| `host.players()` | `server_players()` returns `Vec<PlayerRef>` in connection order. `acting_player()` returns the optional player whose state is active in the hook. `player_by_name(name)` finds a connected real player by case-insensitive account name, not nickname. |
| `host.levels()` | `list_levels()` returns loaded server levels in vanilla dimension order, or an empty list before a server world opens. |

Player and level lookup services accept `&dyn ServerHost` and do not mutate the
world. These services are unavailable on `Host`, `ClientHost` and restricted
level hosts. Block and fluid scheduling uses `LevelTicksHost`;
`tick_stats()` belongs to `ServerHost`.

On a client, `host.chat_line(Text::literal("Local notice"))?` adds a line only to
the local chat HUD and history. It sends no packet and does not fire
`client_chat_received`. Handle its `Result<(), mod_api::TextFormError>`: text
exceeding the boundary's size or nesting limits is refused before reaching the HUD.

## Running a command as an entity

`host.commands().run_command` runs as the console. To run a line as a player or another entity instead, at a level
you choose, call `entity.run_command(host, line, PermissionLevel::Gamemasters)`: `@s` is that entity, `~` its
position, and only a player is answered. It returns `false` for a line that fails, a stale or removed entity, and an
entity outside the level in place (a player in another level included, since that level is parked while the hook
runs). To run a command in another level, do it from a moment in that level or from a task scheduled there.

## Identity and location

Read identity through `player.uuid()`, `player.name(host)`,
`player.permission_level(host)` and `player.address(host)`. The address is the
connection IP without a port, including the forwarded address behind a Velocity
proxy. An acting player without a known connection has an empty address.

The online list includes connected players during nested player turns. Name,
permission and address reads work for connected players whose bodies are displaced
by another player's turn; stale handles are refused. Entity and living handles
provide world-state reads, which require a resolvable body.

`player.level(host)` returns the authoritative `LevelRef`;
`player.dimension(host)` returns its dimension `Identifier`, including registered
custom dimensions. Both accept `&dyn Host`. On a server they identify the player's
current dimension, even when another dimension is ticking. On a client they
identify the local player's level. Unknown, disconnected, replaced or stale handles
return `None`.

## Messages and visibility

Send a message with `player.tell(host, text)`, `player.action_bar(host, text)` or
`player.tab_list(host, header, footer)`. Messaging reaches acting and connected
players, including displaced online records, and refuses stale or disconnected
identities.

| Method | Behavior |
|---|---|
| `set_nickname(host, nickname)` | Changes the name shown in chat and the Tab list; `None` restores the account name. Commands, selectors, logs and events use the real name. Validate the nickname's length, filtering and conflicts yourself. |
| `set_sleep_ignored(host, ignored)` | Controls exclusion from the sleep count. |
| `disconnect(host, reason)` | Disconnects the player with a `Text` reason, as `/kick` does, also inside their own hook or a payload handler (the connection closes after it). Answers `false` for a stale or unavailable handle, a fake player, the single-player owner and a player already being disconnected (the first reason stands). A reason the `Text` form checks refuse returns `TextFormError` before anything changes. |
| `hide(host, permission)` | Hides the player from players below the supplied permission level, starting next tick; `None` restores visibility. Hidden players disappear from the world and Tab list, and their join and leave messages are suppressed for those viewers. |

Nickname and sleep settings require an online player. These settings survive
reconnections until the server stops or you change them. Visibility is stored by
UUID: you can use a retained handle after disconnection until its world epoch
closes. You cannot target arbitrary offline UUIDs. Retain the handle value, not a
borrowed service.

## Chat lines and respawn

Keeping a public chat line from some players (an ignore list, a chat channel, a party chat): the `chat_recipient`
hook is asked once for each placed receiver, the sender included, before any of them is sent the line, with the
`sender`, `sender_name`, the `message` players see and the `recipient`. Set `hidden` and that one receiver is sent
nothing, no chat index and no pending acknowledgement; the others still get the line signed as it was sent, so its
signature verifies for them. It covers a player's chat after `server_chat` let it through and `/say` and `/me`, signed
or not; whispers (`/msg`) are not public and are not asked. It is not a second `server_chat`: that hook is heard once
for a typed line and cancels it for everyone, while this one is per receiver and also covers `/say` and `/me`. With no
listener nothing is asked or allocated. A wasm listener's fault or timeout answers nothing, so the line is sent. The
server's log keeps every line, hidden or not.

`player.respawn(host)` presses the Respawn button for a dead player, as their client's does (`PERFORM_RESPAWN`): they
are placed at their respawn point or the world spawn and `player_respawned` is heard. Call it outside every player's
hook (from `server_tick`, say). It returns `Result<(), RespawnRefusal>`, and each refusal changes nothing, checked in
this order: `Client`, `InPlayerHook` (a player's state is swapped in), `Stale` (no player on this server behind the
handle), `NoConnection` (a fake player, or the world's own player in a single-player world, which respawns through its
own screen) and `Alive`.

## Player rules and sleep

Reads accept `Host`; writes accept `ServerHost`.

| Method | Behavior |
|---|---|
| `may_use_item_at(host, pos, face, stack)` | Applies `Player.mayUseItemAt`: builders may use the item; adventure and spectator players need matching `can_place_on`. |
| `can_harm(host, other)` | Applies `ServerPlayer.canHarmPlayer`, including the `pvp` rule and teams. |
| `respawn_config(host)` | Reads the respawn point; `Some(None)` means no point is set. |
| `set_respawn_config(host, config)` | Sets the point as `/spawnpoint` does, including the `player_set_spawn` veto. Refuses stale, unavailable or dead players, unknown dimensions, non-finite yaw and canceled changes. |
| `text_filtering(host)` | Reads the option announced by the player's client. |
| `sleep_ignored(host)` | Reads the sleep-count exclusion setting. |
| `counts_for_sleep(host)` | Reports whether the player counts toward the night skip. |
| `sleep_timer(host)` / `set_sleep_timer(host, ticks)` | Reads or writes `Player.sleepCounter`, from 0 to 109; 100 means slept long enough. The write refuses larger values, fake players, guests in another level and stale players. |

A second disconnect on a guest already leaving is a no-op and keeps the first reason.

Vetoing a player's movement: the `player_move` hook hears each validated movement report whose position or rotation differs from the player's last reported location, after the server's own checks and before the report is committed, with the exact `from`, `to` and both rotations. The last reported location is kept on the player's connection, apart from the accepted position: it is the position given to the listener before it runs, and the destination of every server-side teleport (a join, a teleport in the level, a level change, a respawn, the resend of an unconfirmed teleport and the correction after a veto); `from` is that location. Set `cancel` and the report is not applied; the client is sent back to the accepted position and rotation. The veto adds no anticheat violation of its own, but the report was already seen by the server's own checks and by the anticheat's record of the client's ticks before the hook, so what a violation of those would have done still happens. The engine applies no threshold, so a mod that wants to ignore small moves compares the two positions itself (`(to - from).length() < 1.0 / 16.0` skips anything under 1/16 of a block). A held report is ruled on in the same tick; that player's later packets (further reports, the end of the client tick, pongs) wait in order behind it, then go on in that tick, so each later report is heard from the one before. A report is out of time at its 21st tick, counting the tick it was held at as the first: it is dropped with the correction, never committed, even when a ruling arrives after that; a disconnect, teleport or level change drops it cleanly. Not heard: the first tick in a level (the tick after a join or a level change; the second is heard), a player riding something (`vehicle_moved` hears the vehicle), any teleport (its confirmation is validated and committed without the hook, at the teleport's target, and so is the veto's own correction; a report that arrives while a teleport is unconfirmed is not heard either) and a passage crossing (its caller reads the position at once, so the report is committed in place). The first report after a teleport that passed validation is heard, with the teleport's destination as its `from`. With no listener nothing is held or copied; with one, the cost is a held report for each moving player a tick. A fault or an epoch timeout in a wasm listener answers nothing: it is not a veto and the report is committed, like every cancellable hook; the fault is logged, counted against the mod and stops the mod's group for the session. A native listener that never returns is out of scope: it blocks the game thread like any native code, and the 20-tick deadline covers only a report held pending across ticks.
## Text conversions

`host.text_from_json(json)` reads one JSON or SNBT component the way `/tellraw`
does and returns `Result<Option<Text>, TextFormError>`: `Ok(None)` when it does
not parse as a single component, `Err` when the text is over the size or
nesting limits. An object needs one content key (`text`, `translate`, `keybind`,
`score`, `selector`, `nbt` with a source, or an inline object); a `type`
binds alone and the other content keys are ignored, a null `type` counts as
absent. `text` must be a string, `extra` a non-empty list (null counts as
absent), `selector` a selector and `nbt` an NBT path that parse, and a style
field that is present and wrong (`color`, `shadow_color`, the format flags,
`click_event`, `hover_event`, `insertion`, `font`) refuses the whole component.
A translation's `fallback` and an `nbt` component's `interpret`, `plain` and
`separator` of the wrong kind are dropped instead. An object whose only content
key is `index` is refused, as in vanilla; Horizon follows vanilla there. JSON
escapes such as `\u0041` are decoded.

`host.text_from_legacy("§cRed")` reads `§` formatting codes as
the font draws them; `&` codes are not read, so replace `&` with `§` first.
Both work on either side.

## Server clock, rules and distances

`level.set_clock_time(host, ticks)` does what `/time set` does in that level's
default clock; `time_skip` fires with cause `mod` and a mod can keep the old
time. `level.set_game_rule(host, "keep_inventory", GameRuleValue::Bool(true))`
does what `/gamerule` does and fires `game_rule_changed` with no actor. Both
return `false` for a stale handle, a client, or a value the rule refuses.

On a server, `host.tick_times()` returns the last 100 tick durations in
nanoseconds (the tick numbered `n` at index `n % 100`; `tick_stats()` has the
average). `host.server_view_distance()` and `host.server_simulation_distance()`
read the distances in chunks; `host.server_set_view_distance(d)` and
`host.server_set_simulation_distance(d)` change them live for 2 to 32 on a
dedicated server and return `false` otherwise. `server.properties` is not
written.
