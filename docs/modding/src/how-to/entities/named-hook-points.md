# Named hook points

Implement catalog hooks as methods of `mod_api::Hooks`.

For function-slot listeners, declare `[[inject]]` with `id`, a target in `at`,
`kind` (`"head"`, `"return"` or `"replace"`), and a crate-root listener in `with`.
The optional `hash` pins the function fingerprint; when you omit it, the build
uses the pinned game's catalog. The schema rejects `hook` and `hook_version`.
See [Injections](../../concepts/injections.md).

Native rendering callbacks such as `horizon_mod_render::render::world_drawn`,
`hand_drawn` and `world_views` use a HEAD mixin. The launcher compiles the patched
function against the prebuilt game. Callbacks borrow the frame's GPU resources;
`world_views` supplies additional camera views. The renderer refuses invalid,
duplicate and oversized view requests by name, as `push` returns them.

Use the geometry types re-exported as `horizon_mod_render::render::{Vertex, EntitySpan}`.
Implement the `render_needs` callback to set `RenderNeeds` requests for world depth
and entity geometry; these requests feed the engine's frame requirements so it
retains the resources your drawing callbacks borrow.

## Leaving a bed

`player_bed_leave` is a server-side, listen-only notice. It runs before the
sleeping position and pose are cleared, so a listener can still read the player
as sleeping at `pos`; it cannot cancel the wake. For example, a listener recording
why a player left a bed should retain both flags rather than treating every wake
as a manual leave:

| Wake reason | `(wake_immediately, update_level)` |
| --- | --- |
| Morning after skipping the night | `(false, false)` |
| Explicit leave or bed-rule refusal | `(false, true)` |
| Generic stop-sleeping, missing bed or death | `(true, true)` |

`wake_immediately` resets the sleep counter to **0** when true, skipping the
fade-out; false sets it to **100** for the normal fade-out. `update_level` is false
for the morning wake because the sleeping list has already been handled, and
true for the individual wakes above. The same pairs apply to the acting player
and stored guests. Waking that same player again inside the callback does not
emit a second notice.

See the [generated API reference](../../reference/api.md) and the hook's
[caller notes](../../reference/hook-notes.md#player-bed-leave-callers).

## Physical movement decisions

`entity_move` runs on the server before one physical `Entity.move` invocation,
including its no-physics and stuck-multiplier handling. This hook receives the
loop context, not a `Host`: it decides only the movement delta. `MoveCause`
distinguishes self movement, player packets, pistons, shulker boxes and shulkers;
boats and minecarts use self movement.

A replacement must have finite components, each within ±64 blocks. Invalid
output is refused whole, retaining the requested delta rather than clamping it.
Cancellation skips the invocation: no position, velocity, collision-flag or fall
change, and no displacement notice. An overridden player or vehicle packet move
sends a correction even below the normal distance threshold, for creative players
and from obstructed starts too. A replacement equal to the request retains the
normal vanilla distance rules.

A native panic or trapped/refused wasm listener restores the originally requested
delta, not an earlier listener's replacement. Later listeners do not run on that
invocation, and the failed listener is disabled for this hook for the session;
its other hooks remain enabled. Native and wasm listeners share this contract.

## Teleport hooks

`ender_pearl_teleport` covers player-owned and mob-owned pearls. Its `owner` is an
`EntityRef`; listeners can cancel or redirect `target`, and read what the pearl hit
(`hit_entity`, or `hit_block` and `hit_face`, and the point `hit_at`). Player owners retain the
acting-player context and mutable fall `damage`. Mob owners take no pearl damage
and make no endermite roll. Pearl placement does not emit the command-only
`entity_teleport` notification. The pearl is consumed even when teleportation is canceled.

`item_consumption_teleport` covers players and foxes with the random-teleport
consume effect. Its `consumer` is a `LivingRef`, and `consumed` is the stack captured
before use, including count and components. Cancellation ends further teleport
attempts, not consumption. Fox callbacks run before that fox's living AI/movement
and before the next mob ticks, with the full server host. Listener changes to an
actor's item, position or eligibility survive rather than being overwritten by a
stale teleport decision.
