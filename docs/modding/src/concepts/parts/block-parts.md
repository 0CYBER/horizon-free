# Block parts

`BlockPart::clone_item_stack` answers on the server when a player picks the block. It receives the
level, position, block state and `include_data`, true when a creative player picks with Ctrl. Return
`None` to keep the game's item.

## Light emission

`BlockPart::light_emission` supplies positional light for blocks that declare
`dynamic_light: true`. It runs on both sides with a read-only host. Return `Some(level)`
to replace the state's emission or `None` to keep it.

| Rule | Behavior |
| --- | --- |
| Eligibility | Every state of a block declaring `dynamic_light` is queried; other blocks are not. |
| Calls | Chunk arrival, block edits and `LevelRef::check_light(host, pos)` query emission. |
| Caching | Light workers read the stored answer; they do not call your Part. Call `check_light` when data your answer depends on changes. |
| Thread | The callback runs on the thread that owns the level. |
| Range | Values above 15 are clamped to 15 before entering the light queue. |
| Client updates | A server light update queries each affected section's cells at most once per cell per update. |
| Exclusions | The block's own rendered light, ambient occlusion and mob-spawn rules use the state's light instead. |

Keep the callback a pure function of the level, position and state, without side effects.

## Path classification

`BlockPart::block_path_type` answers for `event.state` at `event.pos`, with a read-only
host for the owning level. Return `Some(PathType)` to replace the whole static classification,
even for air, or `None` to use vanilla. The same state may answer differently by position.
This is separate from `is_pathfindable`, which remains state-only. The server caches the final
static classification in 4096 direct-mapped positional slots shared by its AI workers, including
`Open` for missing navigation-region chunks. A collision replaces a slot. A matching
`sendBlockUpdated` notification clears it: ordinary writes require an actual state change,
`UPDATE_CLIENTS` and full chunk status at least `BLOCK_TICKING`; direct block-entity notifications
clear even unchanged state. Time, `/reload`, chunk load and chunk unload do not clear it. Keep
the answer deterministic for that lifetime. Neighbour and mob-specific adjustments are not
stored in this cache; existing per-search evaluator caches remain.

`BlockPart::adjacent_block_path_type` answers for a neighbour at its own `event.pos`;
`event.path_type` is that neighbour's classified type, not the node's type. Return `Some`
to end the scan immediately, even if the answer equals the node's current type. Return
`None` to retain that neighbour's fluid and vanilla contribution. Neighbours are visited
in x, y, z order, each from -1 to 1, excluding the node's own column. No mob is supplied
to either query. A panicking callback is contained and falls back to the default.

## Connections

`BlockPart::can_be_connected_to` lets the block beside a fence, a wall, or iron bars or a pane
answer whether that connector connects to it. The event names your block's `pos` and `state`, the
`connector` state, its vanilla rule (`kind`), the `direction` from your block toward the connector
and `face_solid`, whether your block's face toward it is sturdy. Return `Some(true)` to connect,
`Some(false)` to refuse, or `None` to keep vanilla's rule.

| Rule | Behavior |
| --- | --- |
| Order | Vanilla's rule, then your block's answer, then the connector's own `connects_to`, which starts from that answer and has the last word. |
| Calls | When the connector is placed and each time it settles its shape toward your block, on authoritative servers. |
| Walls | A connected wall side is still low or tall from the block above it. |
| Panics | A panicking part is contained with a warning, and vanilla's answer is used. |

Answer from the event without touching the world: the level host is read-only.

## State changes

`BlockPart::on_block_state_change` hears a block change from `event.old` to `event.new` at
`event.pos`, at the end of a `Level.setBlock` that held. The parts of the old state's block hear
it, then, when the block changed, the parts of the new state's block; `event.state` names the
state whose parts are asked. The host is a read-only `LevelSide`.

| Rule | Behavior |
| --- | --- |
| Moment | After the client update, the neighbour updates and the neighbours' shape updates: a fence beside new stone already shows its connection. A write made inside those updates is heard after the updates it queued itself, so its neighbours are settled too; several such writes are heard in the order they happened. |
| Replaced writes | A write whose `on_place` replaced the block is not heard; the write that replaced it is. |
| Sides | Servers hear every changed write with `event.predicted: false`. Clients hear server updates, applied local predictions (including breaks and placements), settled corrections and piston replays. |
| Predictions | Applied local writes are heard immediately with `event.predicted: true`. Server updates at predicted positions stay held and unheard until acknowledgement. A confirmation of the same state has no second event; a denial is heard once from the predicted state to the server's with `predicted: false`. |
| Not heard | Equal-state writes, unapplied guesses made while deciding an action, and world generation's writes. |
| Cost | Without a listener, one flag check; no event or level adapter is built. |
| `block_changed` | The global server hook hears a change earlier, right after the state is written, and also for replaced writes. |
| Panics | A panicking part is contained with a warning. |

## Rotation

`BlockPart::rotate` answers how a structure template turns your block as it places it: world
generation's structures, `/place template`, the structure block, jigsaw blocks and the host's
template snapshots. `event.state` is already mirrored, `event.pos` is where it goes and
`event.rotation` the template's turn. `event.rotated` starts at the game's own turned state; set
it to another state of the same block, for example
`context.state_with_property(event.state, "facing", "north")`.

| Rule | Behavior |
| --- | --- |
| Context | A worldgen Part method: a `WorldgenHost` on the worldgen workers or the server thread, no `Host`, no writes. Keep it a pure function of the event. |
| Refusals | A state of another block, or one the game does not have, is refused with a warning; the game's state is placed. |
| Mirroring | Stays the game's: vanilla's table, or the template's for a mod block. |
| Panics | A panicking part is contained with a warning, and the game's state is placed. |

## Landing writes

`BlockPart::fall_on` runs at vanilla's landing moment on a read-only level random host.
Request a deferred write with `event.request(LandingWrite::SetBlock(state))`, where `state`
is an API `BlockState`, or `LandingWrite::DestroyBlock { drop }`. The game checks the
complete write batch before queuing it. An invalid block key or ordinal refuses that
callback's writes and other outputs, including damage; it never applies a valid prefix.
After the entity batch ticks, writes apply in request order if the block still matches
its original landed state. That guard runs once, so a first write can change the state
without suppressing the next write. SetBlock notifies neighbors and clients; DestroyBlock
preserves the requested resource-drop flag.

## Storage changes

`BlockPart::contents_changed(host, event)` observes a world block's storage at `setChanged`,
with a `ServerHost`, typed `event.level`, `event.pos: BlockPos` and `event.state`. Attach it to the specific
blocks you observe, including vanilla containers; it has no global `Hooks` counterpart. The backing
contents are already written, before dirty/comparator updates. Native transactions notify once per
backing block at root commit, never on abort or simulation. An explicit `BlockEntityRef::set_changed`
notifies even if contents are unchanged. A double container notifies each component, not once again
for the other half's comparator refresh. A listener's writes still apply, but a nested notice for that
same backing block does not re-enter the listener; writes to a different backing block notify normally.
Entity/menu placeholder coordinates are not block notices.
Read the committed items through the typed level:

```rust,ignore
{{#include ../../../../../crates/horizon-client/src/game/blocks/machines/contents_changed_tests.rs:contents_changed_listener}}
```

## Crop fertility and farmland hydration

Two `BlockPart` methods let a block change what vanilla's crops and farmland ask of the blocks around them.
Both run on an authoritative server with a read-only `LevelHost`, and a block with no part answers as vanilla does.

`BlockPart::is_fertile` is asked for the soil of a growing crop. On its random tick a crop, stem or pitcher
crop asks each of the 3 × 3 blocks under it, but only a block in `#minecraft:grows_crops`: a block outside the tag is
never asked. `fertile` starts as vanilla's answer, `moisture > 0`, and the contract is `collect`: set it to decide. A
fertile block adds 3 to the growth speed and any other tagged block 1 (a quarter of either off-centre).

`BlockPart::can_be_hydrated` belongs to a farmland block. On the farmland's random tick the game scans the 9 × 2 × 9 box
from its position (x and z from −4 to 4, y 0 and 1) in x, then y, then z order, and asks only at a position whose fluid
is not empty, passing the fluid (`FluidState`) and its position. The contract is `replace`:

| Answer | Meaning |
|---|---|
| `Some(true)` | This position waters the farmland; the first one ends the scan and moisture goes to 7. |
| `Some(false)` | This position does not water it, whatever the fluid. |
| `None` | Vanilla's rule for this position: a fluid in `#minecraft:water`. |

Rain on the block above, the watered-area ticket, moisture changes and drying out are unchanged. With no part on the
farmland the scan is vanilla's.
