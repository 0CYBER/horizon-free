# Camera and collision reads

`ClientHost::is_paused()` reads the applied client pause state, not whether a pause
screen is open. A multiplayer game continues running while its pause screen is open.
The pause-change pre hook sees the old state; the post hook sees the new state.

`ClientHost::playing()` is true while the player is in the world with nothing over it: no screen, title, sign
editor or book, so keys and zoom apply. `ClientHost::mapping(name)` reads a vanilla key mapping (`key.forward`,
`key.left`, `key.back`, `key.right`, `key.jump`, `key.sneak`, `key.sprint`, `key.attack`, `key.use`) as a
`Result<Option<KeyMapping>, TextFormError>`. `Ok(Some(mapping))` contains the held state in
`mapping.down` and the bound key's Controls label in `mapping.label`. Other names return
`Ok(None)`; an invalid label returns `TextFormError`, including components deeper than 512 nodes. Both read without `&mut`, so the HUD and screen hooks can ask them.

`ClientHost::clipboard_text()` reads the system clipboard (`KeyboardHandler.getClipboard`, empty text when it
holds none) and `ClientHost::set_clipboard_text(text)` writes it (`setClipboard`; empty text copies nothing). Both
work only while the game handles a key, mouse or typed-text event, from the window or from the control socket
(`horizon-ctl key`, `mouse`, `type` and its clicks): a key, mouse or screen hook that fires inside one, such as
Ctrl+V into a mod's own field. At any other moment (a tick, a frame, a packet) the read is `None` and the write
is `false` with nothing copied, so no mod reaches the clipboard without the player doing something. Both are
`None` and `false` on a server, and take `&self`, so a screen hook can call them. Vanilla's `KeyboardHandler`
has no such rule: Horizon adds it.

## Camera snapshots and perspective

The `ClientHost::camera()` helper returns the most recently extracted world frame's camera, or none before one exists. Its position, interpolated eye height, partial tick, angles, final FOV, near/far planes and dimension accompany column-major view and projection matrices. The view is camera-relative and includes bob and spin; projection uses reversed Z. Earlier callbacks in a frame can see the previous frame. `block_collision(pos)` returns block-local `[min_x, min_y, min_z, max_x, max_y, max_z]` boxes with an empty collision context; unloaded blocks and collision errors give no boxes.

`ClientHost::project(at)` places a world point on this frame's screen, as vanilla's
`GameRenderer.projectPointToScreen`: normalised device x and y (-1 to 1, x right, y up) and a
reversed depth z. A HUD or screen hook turns it into GUI pixels with the `DrawContext`'s size
`[w, h]`: `((x + 1) / 2 * w, (1 - y) / 2 * h)`. It is `None` for a point behind the camera or a
non-finite one (vanilla returns a mirrored point there), outside a world, before extraction and between render frames. It is supported in GUI, screen
and world drawing callbacks after camera extraction, not setup or tick callbacks.

`ClientHost::set_local_rotation(yaw, pitch)` turns the local player's view, as vanilla's `setYRot`
and `setXRot` store it: yaw as given (not wrapped; the server wraps it), pitch `% 360` then clamped to
-90..90. The previous-frame angles move by the same amount, so the view doesn't jump. The call sends no
packet. The game's own player sync at the end of the client tick sends the look in its one move packet,
of the kind vanilla picks, and only while the camera is the player (a rider sends the rotation-only
packet). Several calls in one tick send the last values once, and a call after the sync goes out the
next tick. NaN or infinity returns `NotFinite` and changes nothing; outside a world the call returns
`NoWorld`. The server sees an ordinary client look, at tick granularity, as from a mouse.
`calculate_player_turn` changes how far the mouse turns the view; `set_local_rotation` sets it directly.

`ClientHost::set_camera_type(type)` is a queued client-only effect and takes `&self`, so
shared callbacks such as `camera_pose` can request a perspective. `camera_type()` reports
the latest request immediately; several requests coalesce to the last one. Requests before
or during `camera_pose` apply before that frame's final camera, FOV and spectator post effects.
The pose hook runs only once: its explicit position or angle overrides survive a refreshed
vanilla camera. Requests from later render callbacks apply at the next frame's extraction.
Setting a type never emits `camera_type_cycle`; F5 alone cycles, and its final event choice
wins over requests made inside the cycle callback.

## Viewed levels

A server can open other levels for a client to view besides the one the player is in (`horizon:levels`,
as Immersive Portals' `ClientWorldLoader` keeps a `ClientLevel` per dimension). On a client,
`host.level("minecraft:the_nether")` also returns a handle for each such level while the server keeps it
open. Its block reads read the viewed level's own world, never the player's: `block_state`, `block`,
`block_property`, `clip`, `collision_boxes`, `has_collisions`, and the fluid reads built on
`block_state` (`fluid_shape`, `fluid_height`, `fluid_height_for_camera`, `fluid_can_be_replaced_with`).
So a mod can aim a ray or collide a box through a portal. They never write and never load a column: a
column the client doesn't hold is `None`, for a collision box as soon as its walk reaches one. `clip`
keeps its traversal limit and `collision_boxes` its cell cap.

The handle is `None` while the server has the level closed. If the server closes the level and opens it
again within the same client level, the old handle reads again. A login, a reconfiguration or a respawn
into another level replaces the client level: the old handle stays stale for good, and the mod calls
`host.level` again once the server reopens the level. A respawn in the same level drops the viewed
levels too, but keeps the client level, so the old handle reads again after the reopen. Writes,
server-only reads, the other level reads and the entity reads (`entities_in`) refuse a viewed level.
Native and wasm mods use the same handle methods.

## Crosshair reads

`ClientHost::crosshair_target()` returns the ordinary interaction-range target as
`CrosshairTarget::Block { pos, state, hit }` or
`CrosshairTarget::Entity { entity, point }`. The block hit contains its world-space
location, face and inside flag; the entity point is also in world coordinates.
It uses the game's current targeting ray and block/entity reach, respects blocks
occluding entities, and returns `None` on a miss or without a client world.
It can be read with a screen open. `entity` is an `EntityRef` carrying the current
client level's epoch and network entity ID, including dragon parts. It becomes stale
when the client level is replaced or the client disconnects; a same-level respawn
keeps its epoch. This supplies identity, not client entity reads: the server-only
entity data methods reject client handles. Unlike `view_hit` and `raycast`, this does not find blocks
in distant terrain or return a block behind a targeted entity.

## Local player reads

These read the local player as the client sees it. They take no mod-controlled numbers and send nothing.

- `ClientHost::client_session().client_latency()` is the player's latency in milliseconds as the
  tab list draws it (`PlayerInfo.getLatency`): the server's smoothed keep-alive average, refreshed by
  the server's own broadcast. It reads 0 before the server's first measurement, as vanilla does, and
  `None` outside a world.
- `ClientHost::local_item_use()` is the item in use: `ItemUse { hand, ticks_used, ticks_remaining }`
  (`isUsingItem`, `getUsedItemHand`, `getTicksUsingItem`, `getUseItemRemainingTicks`).
  `ticks_used + ticks_remaining` is the use duration. It is `None` when no item is in use or no world
  is open.
- `ClientHost::hover_name(&stack)` is `ItemStack.getHoverName()` as plain text: the custom name, else
  a written book's non-blank title, else the item's name in the client's language, without rarity
  colour or italics. An invalid stack reads an empty string.
- Reach is `targeting()` (below), which comes from the interaction-range attributes with the server's
  and equipment's modifiers. Menu slot numbers are `Slot.index`, the position in the menu's slot list
  that `menu_slot` and `menu_slot_count` take; `menu_slot_backing` gives the index in the backing container.

## Combat targeting

`ClientHost::targeting()` and `set_targeting(ClientTargeting)` control liquid picking,
entity occlusion and an optional crosshair entity override. Defaults preserve ordinary
vanilla targeting. Set `pick_liquids` to include source and flowing fluid shapes, or
`hit_through_walls` to ignore block occlusion for entity picks and attacks. An `entity`
override must name a live entity in this client level; stale or self handles make the
setter return `false` without changing the controls. The setter requires a logged-in
client world. An override is rechecked for reach, held-item range and occlusion when
used and disappears from reads after its level is replaced.

Reach remains attribute-backed: use the server player's living handle and
`set_attribute_base(host, "minecraft:block_interaction_range", value)` or
`"minecraft:entity_interaction_range"`. The client's cached reach follows the server's
attribute updates. Client targeting controls do not grant extra server reach.

`client_attack_target` receives a mutable `targets` list before an ordinary melee click
(spear jabs use their existing path). It starts with the crosshair entity. Replace it
for redirection, append for multiple hits, or clear it to suppress an entity attack.
`Flow::Cancel` cancels the batch. There is no batch cap; duplicates are sent once
in first-occurrence order. Each target is checked again against live handles,
attackability, reach, held-item range and the occlusion control. Clearing an entity hit
does not mine the block behind it; an empty list on a block or miss keeps the ordinary
path. Each accepted hit uses normal attack packets and prediction. Server validation
and cooldowns apply. Native and wasm mods use the same typed API.
