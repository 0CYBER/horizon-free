# Server services and player handles

Use `ServerHost` services to run commands, send messages and find connected
players. Native and wasm mods use the same calls.

## Services

| Accessor | Operations |
|---|---|
| `host.commands()` | `run_command(line)` runs through the console command path with permission level 4. `feedback(text)` answers the command source. |
| `host.messages()` | `broadcast(text)` fires the broadcast hook and writes to the server log. `text_allowed(text)` checks the server chat filter. |
| `host.players()` | `server_players()` returns `Vec<PlayerRef>` in connection order. `acting_player()` returns the optional player whose state is active in the hook. `player_by_name(name)` finds a connected real player by case-insensitive account name, not nickname. |
| `host.profiles()` | `resolve_name(name)` and `resolve_id(uuid)` ask the game's Minecraft profile resolver, the one behind dynamic player heads and `/fetchprofile`, for an account's profile. The first call answers `ProfileResolution::Pending`; poll it from `server_tick` for `Done(Some(profile))`, or `Done(None)` when there is no such account or the lookup failed (for example, a rate-limited lookup). `Done(None)` does not prove the account is missing. The resolver caches answers, paces a mod's new lookups at 60 a minute, and reaches Mojang's services whatever the server's online mode. |
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

`player.connect_address(host)` gives the host and port the client typed to connect (`play.example.com`, 25570): the
name a server with several was reached by, which is not the IP in `address`. It is the handshake's own, cut at its first
NUL (where a proxy appends its data), and a transfer's is the one the transferring server gave. It is `None` for a
fake player, the single-player owner and a client that typed nothing. A mod can announce a resource pack URL under it
that this client can reach.

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

## Health and network entity ID

A `PlayerRef` gives you the player's UUID. To read health, first call
`player.entity()` for its `EntityRef`, then `host.living(entity)` for its
`LivingRef`. Call `health(host)` and `max_health(host)` on that handle. Both
return health points, where two points make one heart. Maximum health includes
attribute modifiers. Use `entity.id()` for the network entity ID that entity
events carry, and `player.uuid()` for persistent player identity.

```rust
use mod_api::{PlayerRef, ServerHost};

fn player_health(host: &dyn ServerHost, player: PlayerRef) -> Option<(u32, f32, f32)> {
    let entity = player.entity();
    let living = host.living(entity)?;
    Some((entity.id(), living.health(host)?, living.max_health(host)?))
}
```

Handle `None` when the player has no body available to this hook or the handle
has gone stale. A network ID comes from the entity handle itself, so reading
`entity.id()` does not check that the entity still exists.

## Profiles and skins

Two server-side lookups answer different questions.

`host.profile_by_uuid(uuid)` and `host.profile_by_name(name)` (any case) return the `GameProfile`
the server already knows: an online player's, with its properties, else the name from the user
cache (`usercache.json`, on a dedicated server), without properties. Neither asks Mojang, and
`None` means neither the server nor its cache knows the player. `player.profile(host)` returns a
connected or acting player's login profile, including the signed `textures` property.

`host.profiles().resolve_name("Notch")` (or `resolve_id(uuid)`) asks the game's own Minecraft
profile resolver, the one behind dynamic player heads and `/fetchprofile`, for an account's current
session-server profile. This is the asynchronous Mojang lookup, and it works whatever the server's
online mode. The first call answers `ProfileResolution::Pending` while the background lookup runs;
poll it from a command or `server_tick` until `ProfileResolution::Done(Some(profile))` with the
account's UUID, current name and properties, or `ProfileResolution::Done(None)` when there is no
such account or the lookup failed (for example, Mojang rate-limited it). `Done(None)` does not
prove the account is missing; retry later when the distinction matters. Calls through
`host.profiles()` are paced at 60 new lookups a minute; `/fetchprofile` and the game's own skin
lookups take the game path and are not paced. `Pending` can also mean that budget is refilling;
poll again later. The resolver caches answers and uses the TLS stack the game already links, so a
mod needs no HTTP client of its own. Keep the result in your own state; do not block a login hook
on it.

To change a skin, `player.set_profile_property(host, "textures", value, signature)` sets the
`textures` property (Mojang's base64 value and its signature). Other players see it once
`player.refresh(host)` sends the player to them again; the player's own client keeps its current
skin until they change level or join again. A property set this way is published and returned by
`profile_by_uuid` and `profile_by_name`; `player.profile(host)` keeps returning the properties the
player logged in with. The `player_login_profile` hook below does this before the login success
instead, so the player's own client gets the skin at once and no refresh is needed.

## A player's profile at login

`player_login_profile` runs once for each accepted login, after the name,
whitelist, ban and server-full checks and before the login success is sent, with
the UUID and name the login authenticated and the profile's `properties`. A skin
or identity mod replaces the `textures` property there; the login success and the
player list then carry the changed profile. It has no `Host` (it runs on the
connection thread), so answer from the arguments and your mod's own state alone,
and keep the list inside the profile's limits: at most 16 properties, names of 64
UTF-16 units, values of 32767 and signatures of 1024. A list past a limit is
refused and the login's own properties are sent. `player_login` follows later and
can still refuse the player.

`entity.set_swimming(host, swimming)` (`entity_set_swimming`) sets the swimming flag of a server player or a guest, the one the next water update reads, as Paper's `Entity#setSwimming` does. It returns `false` for a mob, which keeps its own swim, and for a stale handle.

`entity.detonate(host)` (`entity_detonate`) explodes an entity now: a primed TNT explodes where it stands with its power, as `PrimedTnt.explode` does, and a creeper is lit and explodes at once, as `Creeper.explodeCreeper` does. The entity is gone after its explosion. It returns `false`, changing nothing, for a stale handle, a kind that does not explode, or when the explosion itself fails.

`entity.set_fall_distance(host, distance)` (`entity_set_fall_distance`) sets the blocks a server player, a guest or a mob has fallen, as Paper's `Entity#setFallDistance` does, so the next landing hurts by that much. It returns `false` for another kind, a client, a stale handle or a value that is not finite; `entity_reset_fall_distance` is the zero.

`entity.set_fuse(host, ticks)` (`entity_set_fuse`) sets the ticks left before a primed TNT explodes, as vanilla's `PrimedTnt.setFuse` and Paper's `TNTPrimed#setFuseTicks` do. A fuse of `0` or less explodes it on its next tick. It returns `false`, changing nothing, for a negative count, a stale handle or an entity that is not primed TNT (a creeper's fuse is its swell, not this).

## Messages and visibility

Send a message with `player.tell(host, text)`, `player.action_bar(host, text)` or
`player.tab_list(host, header, footer)`. Messaging reaches acting and connected
players, including displaced online records, and refuses stale or disconnected
identities.

| Method | Behavior |
|---|---|
| `set_nickname(host, nickname)` | Changes the name shown in chat and the Tab list; `None` restores the account name. Commands, selectors, logs and events use the real name. Validate the nickname's length, filtering and conflicts yourself. |
| `set_sleep_ignored(host, ignored)` | Controls exclusion from the sleep count. |
| `disconnect(host, reason)` | Disconnects the player with a `Text` reason, as `/kick` does, also inside their own hook or a payload handler (the connection closes after it). Answers `false` for a stale or unavailable handle, the single-player owner and a player already being disconnected (the first reason stands). A fake player's kick keeps the first reason and completes at the next hub poll through the shared leave path. A reason the `Text` form checks refuse returns `TextFormError` before anything changes. |
| `hide(host, permission)` | Hides the player from players below the supplied permission level, starting next tick; `None` restores visibility. Hidden players disappear from the world and Tab list, and their join and leave messages are suppressed for those viewers. |

Nickname and sleep settings require an online player. These settings survive
reconnections until the server stops or you change them. Visibility is stored by
UUID: you can use a retained handle after disconnection until its world epoch
closes. You cannot target arbitrary offline UUIDs. Retain the handle value, not a
borrowed service.

`player.set_sidebar(host, Some(Sidebar::new(title, vec![SidebarLine::new(text, Some(value))])))` shows a
sidebar on that one player's client, in place of the one shown before; `None` takes it away. Up to 15 lines
are shown, top first, each with its text and an optional right-hand value. The sidebar is not stored in the
world's scoreboard, so `/scoreboard` and other players do not see it, and it is gone when the player leaves.
It returns `Ok(false)` for a player who is not online; a text the boundary refuses changes nothing.

To change only the name in the Tab list, use `player.set_list_name(host, Some(text))`.
Everyone sees the new name at once, colours included; `None` shows the nickname or real
name again. The name is kept by UUID until a mod sets it again or the server stops, also
across a leave and rejoin. The `player_tab_list_name_format` hook still has the last word,
and chat, name tags and commands keep the real name. The call returns `false` for a stale
handle, an unavailable server list or a player who is not online.

To hide one entity, a player included, from one player's client only, call
`entity.set_visible_to(host, viewer, false)` (Paper's `Player.hideEntity`): the entity tracker
stops pairing it with that client and removes it from it on the next pass, while everyone else
still sees it. `true` shows it again and `entity.visible_to(host, viewer)` reads the
state. A mod uses it for a per-player hologram, a vanish that only some players honour, or a
helper entity only its owner should see. It is kept by the entity's UUID until shown again or
the viewer leaves; a rejoin starts with nothing hidden. A hidden player stays in the viewer's
Tab list (`hide` lists them out too), and chat is not filtered. The calls return whether
anything changed, and refuse with `EntityVisibilityRefusal`: `Client`, `Stale`, `NoConnection` for a
fake viewer and `StaleEntity` for an entity handle that names nothing or the viewer's own player.

To show an entity to one player only, spawn it with `player.spawn_viewer_entity(host, kind, pos)`.
Its `AddEntity` goes to that client alone; the level does not hold it, and the other players do not
see it. Move it with `player.move_viewer_entity(host, id, pos)` and remove it with
`player.remove_viewer_entity(host, id)`. It has the type's default metadata only, and it is gone
when the viewer leaves, so a mod that wants it kept re-spawns it on the next join.

The reads `player.latency(host)`, `player.hidden(host)` and `player.nickname(host)`
return what the calls above set. Latency is the Tab list ping in milliseconds
(`Option<i32>`, `0` until the first keep-alive answer). `hidden` is the level
`hide` set, `None` when shown (`Option<Option<u8>>`). `nickname` is the nickname
in place of the player's name, `None` when there is none (`Option<Option<String>>`).
An outer `None` means a stale or unavailable player for all three, a client or an
unjoinable world for `hidden`, and no server list for `nickname`.

## The server list

Changing what the server list shows (a MOTD with colours or a random line, an icon, fake counts, a hidden player list):
the `server_list_ping` hook is heard once for each status request, on the game thread, before the answer goes out.
`protocol` and `virtual_host` (the host the client typed, without its port) are read-only; `motd` (`Text`),
`favicon` (`Option<Vec<u8>>`, PNG bytes), `online`, `max_players`, `sample` (`Vec<(String, u128)>`, a name and the UUID
the list shows) and `hide_players` start as vanilla's answer. `hide_players` leaves out the whole players part;
`cancel` closes the connection without an answer. At most 64 sample entries are kept, and an icon that is not a PNG
is dropped. An answer longer than a client reads (32767 characters) is replaced by the plain one, and so is any answer
the mods do not finish within half a second. With no listener the request is not put to the game thread at all.
The legacy ping of pre-1.7 clients is not asked.

## Resource packs

`player.push_resource_pack(host, ResourcePackOffer)` offers one player's client a pack while they play, as
`ClientboundResourcePackPushPacket` does; `player.pop_resource_pack(host, Some(id))` takes it off again, and `None`
takes off every pack the server pushed, the `resource-pack` of `server.properties` included. Build the offer with
`ResourcePackOffer::new(id, url)` and set `sha1` (40 hex digits, or empty for no check), `required` and `prompt`.
A pack pushed with the id of one the client holds replaces it; other ids stack. The client downloads the zip itself,
so the URL must be reachable from that player.
Like vanilla, popping does not retract an offer still waiting in an unanswered consent prompt;
accepting that prompt can load the pack later. It is not a pending-offer cancellation API.

`resource_pack_status_v2(player, name, pack, status)` hears the answers with the pack's id, including while the
player is dead; `resource_pack_status(player, name, status)` keeps its three arguments and its firing point.
`required` changes only
what the client's screen offers: the server does not disconnect a player who declines a pack a mod pushed (it does
so only for the `server.properties` pack), so a mod that needs the pack kicks on status 1 or 2 itself. Answers to the
`server.properties` pack are given before the player has joined and are not heard. Both calls return
`Result<(), PlayerPackRefusal>`: `Client`, `Stale`, `NoConnection`, and for a push `UrlTooLong` (over 32767
UTF-16 code units), `HashTooLong` (over 40 UTF-16 code units) and `BadPrompt` (text the boundary refuses); a refused
call sends nothing. Characters outside the BMP count as two code units.

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
own screen) and `Alive` (health above 0 and the game not won: the case the button ignores; a player who has won the game and is watching the End poem is respawned even alive, on the next server tick, as the button does).

`player.set_velocity(host, [x, y, z])` sets a server player's velocity in blocks a tick and sends it to their client, as
Paper's `Player.setVelocity` does (`player_set_velocity`, `WorldOnly` access). The value replaces the player's velocity,
where `entity_push` adds to it, and the client moves by it from its next tick. It returns `false`, with the velocity
unchanged, for a stale or unavailable player, a displaced outer player or a component that is not finite.
`entity_set_velocity` refuses players.

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

Reading every movement packet (an anticheat): `player_move` hears only a report whose position or rotation differs from the last reported one, so a stand-still or ground-flag packet never reaches it. The `player_move_packet(player, to, to_rotation, on_ground, has_position, has_rotation)` hook is heard once for every validated movement packet of a guest, also one that changes nothing, with the packet's own on-ground flag as the client sent it and whether the packet carried a position and a rotation (a status-only packet carries neither, and `to` and `to_rotation` then repeat the player's own). It is heard with the player's state swapped in, before `player_move` hears the same packet, and cannot cancel; a veto stays `player_move`'s. A guest is heard at most 8 packets a tick (`MOVE_PACKET_BUDGET`); the ones over it wait, in order, for the next ticks, at most 256 of them (the oldest are dropped past that), so one client cannot make the listeners run for its whole backlog at once. Not heard: the first tick in a level, a teleport's confirmation and a passage crossing. With no listener nothing is queued.

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

Mob and packet diagnostics are separate switches that are off by default.
`host.mob_cost_sampling(true)` times each mob's own tick, riders included, summed
by level and mob type; `host.take_mob_costs()` returns what was sampled since the
last call, costliest first, and clears it. `host.count_packets(true)` counts every
player's play packets by id and direction, for the whole server;
`player.take_packet_counts(host)` returns one player's counts since the last call
and clears them. Both switches return `false` on a client.

## World mutation: chunks, blocks shown and placed as a player

For mods that write blocks by other means (an editor's own section writes, a schematic) and then have to make the game and the client agree. Server only. Each method returns `Result` and names every refusal; a refusal changes nothing.

| Method | Behavior |
|---|---|
| `level.relight_chunks(host, &[[cx, cz]])` | Queues the given loaded chunks of the level in place, and the loaded chunks around them, to be lit again whole at the level's next light update, as `edit_blocks` and `level.check_light` queue theirs; nothing is lit during the call. A column whose eight neighbours are not all loaded waits until they are. The light that changed goes to the players who see it. There is no completion event: read the light on a later tick (`level.brightness`), or `chunk.resend` once it is there. Returns how many columns were queued, the ring included (columns not loaded or already waiting are not counted). Refuses with `ChunkRefusal::{Stale, Client, NotInPlace, TooManyChunks { asked, max }}` when the call has more than 256 chunks or would take the level's queue past 4096 columns; `max` is what a call may take now. |
| `chunk.resend(host)` | Queues the loaded chunk to go whole (light, block entities, biomes) to its watchers at the end of the tick, as an `edit_blocks` chunk does. `Ok` with nothing sent when nobody watches. Refuses with `ChunkRefusal::{Stale, Client, NotInPlace, NotLoaded}`. |
| `player.sees(host, target)` | Whether the player's client is shown `target` by the tracker's rule (`ServerPlayer.broadcastToPlayer`): a spectator only to spectators watching through their own camera, a player hidden with `hide` only to players of at least that level. Distance is not part of it. Refuses with `PlayerWorldRefusal::{Client, Stale, NoConnection}` (a fake player is no viewer or target). |
| `player.ack_block_changes(host, sequence)` | Acknowledges the player's block actions up to `sequence` for an action a mod handled on its own channel; the ack goes after the next tick's block changes, taking the maximum pending sequence. A lower value after a higher one has no effect. Any nonnegative value is accepted; a value too high can end pending client predictions early. Refuses with `Client`, `Stale`, `NoConnection` and `NegativeSequence { sequence }`. |
| `player.show_block(host, pos, state)` | One block update to that player alone; the world is unchanged and the real block returns when it or its chunk is next sent, so a mod sends it again while it wants it shown. The override is not remembered and does not itself acknowledge or clear pending client predictions. Refuses with `Client`, `Stale`, `NoConnection`, `UnknownState` and `PositionNotSendable { pos }` (y outside -2048..=2047, x or z outside -33554432..33554432: the packet's position wraps there). |
| `player.place_block(host, level, pos, state, update_neighbour_shapes)` | `level.set_block` with a placement's flags (neighbours told and, unless `update_neighbour_shapes` is false, their shapes updated; the block entity follows that write), then `Block.setPlacedBy` of the block standing there, in the player's turn (a bed gets its head, a door its upper half). It is not vanilla's use-on and not "as player" in the full sense: no survival, obstruction, build-permission or spawn-protection check, no `placed_block` advancement or statistic, no sound, no game event, no item used, and `setPlacedBy` gets no placer. Refuses with `Client`, `Stale`, `Dead`, `StaleLevel`, `OtherLevel` (not the level in place, where the player is), `UnknownState`, `OutOfWorld { pos }`, `NotLoaded { pos }` and `Unchanged` (that state already stands there). |

Where Horizon differs from vanilla: vanilla sends and relights per edit and has no call that lights a set of chunks at once, so `relight_chunks` only queues the work (256 chunks a call, 4096 columns waiting) and never stalls the server thread; `show_block` and `place_block` check the position and the state before anything runs, where vanilla's packet and `setBlock` would wrap or ignore them.

A command usage may also name `<pos:vec3>` (`Vec3Argument.vec3()`): the point typed, delivered as `CommandArgumentValue::Vec3([f64; 3])`. A whole x or z is the block's centre (`1` is `1.5`) and y is as typed; relative and local coordinates resolve against the command's source. Fewer than three numbers, or text, is a parse error and the command does not run.
## Console and player limit

For a mod that mirrors or drives the server's console (a Discord console channel, a remote assistant) or
sets how many players may be on. Server only.

| Call | Behavior |
|---|---|
| `console_line` hook | Every line the server logs (what its console prints and `logs/latest.log` keeps): `level` (`INFO`, `WARN` or `ERROR`), the `thread` it names and the `message` without time or thread. Lines are collected from every thread while a mod listens and heard on the server thread at the start of the next server tick, in the order logged, so a listener may use the host. A line a `console_line` listener logs itself is not collected, so a listener that logs what it hears does not feed itself. At most 4096 lines wait between two ticks, each kept to its first 8192 bytes of `message` and 128 of `thread` (cut at a character boundary); the oldest lines are dropped with no notice, and the lines logged before the first tick a mod listens are not heard. Paper has no event for this (plugins add a Log4j appender), and neither has NeoForge. |
| `host.commands().run_command_with_output(line)` | `run_command` with its feedback kept, on the same run path and source (the console's: one leading slash dropped, permission level 4, from the overworld's spawn): the same `CommandResult` as `entity.run_command_with_output` (see [server commands](server-commands.md)), with `success` (what `run_command` returns) and `feedback`, what the console would print as plain text, at most 256 lines of 4096 bytes each (further lines dropped, a longer one cut at a character boundary). Only the command's own feedback is kept, never lines logged meanwhile. A command that ran and failed is `success: false` with its message in `feedback`. A line run where no server is open to players, or empty once whitespace and the slash are dropped, runs nothing and is `success: false` with no feedback, as `run_command` answers `false` for it; there is no refusal type, as nothing here tells those apart from a failure that `run_command` cannot. The dedicated console's own commands (`stop`, `op`, `whitelist`, `ban`) are not reached and answer as a command that failed. A call made while another one runs captures its own feedback; the outer capture keeps what it had. |
| `host.server_max_players()` / `host.server_set_max_players(n)` | Paper's `Server.getMaxPlayers` and `setMaxPlayers`: the limit logins are refused beyond and the server list shows, live. The read is `None` where no server is open to players, its one reason. The write takes effect from the next login and ping, changes players already on nothing, and does not write `server.properties` (a restart goes back to it, as Paper's does; the management protocol's `max_players` setting does write it). Zero is taken. Refuses, changing nothing, with `PlayerLimitRefusal::NotOpen` or `TooHigh { maximum }` for a limit above `i32::MAX`, vanilla's `int`. |
## Inventory, ender chest and saved data

For mods that copy or edit a player whole (a player-data sync, `/invsee`, `/endersee`). Server only, online players only: a handle for an offline player, a stale one and a client answer `None` on a read and a named `PlayerDataRefusal` on a write.

| Method | Behavior |
|---|---|
| `inventory_slots(host)` | The non-empty slots by `Inventory`'s numbers, lowest first: 0 to 35 the main inventory (0 to 8 the hotbar), 36 feet, 37 legs, 38 chest, 39 head, 40 off hand, 41 body, 42 saddle. `None` when no online player is behind the handle. |
| `set_inventory_slot(host, slot, stack)` | `Inventory.setItem`: `None` empties the slot; there is no item check, so a worn slot takes any item. The client is sent the slot with the tick's inventory sync, and an open inventory menu shows the written stack. Returns `Result<(), PlayerDataRefusal>`; each refusal writes nothing: `Stale` (no online player), `Client`, `Dead`, `SlotOutOfRange { slot, last: 42 }` (never a panic) and `Stack(StackWriteError)` (a count or damage the item does not allow, an item the world lacks). |
| `ender_chest(host)` / `set_ender_chest_slot(host, slot, stack)` | The same pair for the 27 ender chest slots, 0 to 26, with `SlotOutOfRange { last: 26 }`; an open ender chest menu shows the written stack. |
| `give_stack(host, stack)` | Gives one stack as `/give` does: matching stacks, then empty slots, and what does not fit is thrown from the player's eyes for that player alone, with no pickup delay. Refuses with `Stale`, `Client`, `Dead` or `Stack`, giving nothing. |
| `persistent_data(host)` / `set_persistent_data(host, data)` | The compound mods keep on a player (NeoForge's `getPersistentData`), saved with the player under `NeoForgeData`; an empty compound removes it. It is never sent to a client and a change raises no notification. On a death respawn and on the first exit from the End only its `PlayerPersisted` sub-compound is carried over, whatever its type, and every other key is lost; dimension changes, later End exits and teleports keep all of it. Refuses with `Stale`, `Client`, `Dead` and `TooDeep` (data 512 levels deep or more in the player's file). For ports; new mods use attachments. |
| `host.item_from_snbt(snbt)` | The stack an SNBT item compound stands for, read as a saved item is: `{id:"minecraft:stone",count:3,components:{...}}` (`count` 1 when missing). Returns `None` for text that is not one compound (anything but whitespace after it included), a `count` outside 1 to 99 (refused, not clamped), air and an item the world lacks. It also refuses text over 2 MiB or nested past 512 levels: that is this API's own safety bound, the one place it differs from vanilla, whose SNBT parser has no size or depth limit (deep text overflows its stack); the figures are binary NBT's limits, taken so mod-supplied text cannot overflow the parser. |

A stack in an inventory slot, the ender chest or `give_stack` keeps all its components. Mods that move inventories between servers can write back a stack read from a slot unchanged. `item_from_snbt` does not apply the item's stack or damage limits, so its result can still be refused on write with `Err(Stack(StackWriteError))`.

Player inventory writes use vanilla numbers: 0 to 35 main (0 to 8 hotbar),
36 to 39 armour feet to head, 40 offhand, 41 body, 42 saddle.
`ItemPlace::PlayerSlot` uses 100 to 103 for armour and -106 for offhand.
The inventory menu has no slot for body or saddle. All four player data writes
(`set_inventory_slot`, `set_ender_chest_slot`, `give_stack`, `set_persistent_data`)
are undone by an open transaction abort; equipment effects and overflow drops
wait for the outermost commit.

## Advancement progress

For mods that carry a player's advancements between servers. Server only. `player.advancements(host)` returns
the progress as the JSON text `PlayerAdvancements.save` writes (each started advancement's `criteria`, with grant
dates, and `done`, plus `DataVersion`), or `None` for a stale or unavailable player and on a client.
`player.set_advancements(host, json)` replaces the whole progress from a JSON object, as a loaded file does:
advancements and criteria this server lacks are dropped, nothing is awarded (no toast, chat line or reward), and
the client receives the full tree with its next update. A hook that replaces the progress while a gameplay grant
runs does not make that grant complete an advancement: what it imported is not rewarded. It returns `false`,
changing nothing, for input that is not a JSON object, for a grant date that is not a date this server can send
(a number, other text, a year out of range), and for a player who cannot take it: a stale or unavailable handle, a
dead or not connected player, a client, or a world with no server list.

## Player statistics

Statistics are read and replaced whole, not one at a time. `player.statistics(host)` returns all of a player's
statistics as the JSON text of their `stats/<uuid>.json` save file, or `None` for a stale or unavailable player and on
a client. `player.set_statistics(host, json)` replaces all of them with a JSON object of that form and returns `false`,
changing nothing, for text that is not an object or for a stale, unavailable or dead player. A statistic the text lacks
goes back to 0, scoreboard criteria follow the changed values, and the client sees them on its next request. Server only;
`player_stat_award` is not heard.

Measuring a client's transaction round trip (a lag-compensating anticheat): send the player a `ping` with `player_send_packet` and hear the answer in `player_pong(player, name, id)`, which carries the number the ping had; the time between the two is finer than the keep-alive average `player_latency` reads. The engine's own pings (a recording's end-of-tick ping, whose id is the server tick) are answered here too, so pick ids that tell apart from small tick numbers. The hook is heard only for a pong that answers a ping the server sent on that connection, once per ping (an unsolicited or repeated pong, or an answer to a ping older than 30 seconds, is dropped), in order with the guest's other packets, with their state swapped in, and cannot cancel; with no listener nothing is queued.

## Entity broadcast range

`ConfigServerHost::entity_range_percent()` (server side) returns the entity broadcast range the server hosts with,
in percent of vanilla's (`entity-broadcast-range-percentage` of server.properties). It is `100` when no server is
hosting a world, which is vanilla's default. Read only.
