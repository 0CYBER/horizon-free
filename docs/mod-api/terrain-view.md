# Outbound terrain views

A server mod defines a `TerrainView` through `ServerHost::server_define_terrain_view` and selects its
ID with `ServerHost::player_set_terrain_view`. Define rules at `server_started` and choose the view for
each dimension at `player_login`, before the first send, using the event's UUID. This also covers
players held pending a transfer. The server uses the ordinary view until a mod selects a view
for that player and dimension. Refine the selection when permissions become available or change.
The [fixture](../../tests/terrain-view-mod) uses operator status for its bypass and a single chest
state for its target. A real mod can expand a block to its state range using the registry metadata.

The first mod to select a view owns that player's dimension slot until it releases the slot, or the
connection ends. Other mods receive `TerrainViewError::Owned`. Use `None` to release. Rule IDs are
local to the owning mod. Identical definitions share prepared rules. A changed definition or view
selection invalidates the old projection. The server forgets previously sent columns and queues
them through its ordinary near, far or viewed-level tracking, under the normal batch quota. The
client may already know any terrain it received under an earlier view.

Rules are an ordered list of Y bands, eligible states, replacement choices, and the states that
cover a face. The engine uses the first band matching the height and state and replaces a cell when
all six faces are covered. The mod decides whether an unavailable face counts as covered. Resident
neighbours supply known states, including air. Beyond the loaded columns or the height range,
the face is unavailable. Saved far columns use their own sections and the already resident faces
captured when the reader was queued. A lookup never generates or synchronously loads a neighbour.
An edit can change both the edited cell and any of its six neighbours. An arrival, unload or bulk
edit refreshes affected tracked columns, including face neighbours. Before publishing work, the
server checks the input snapshots and selects the body for the current view.

A band with one replacement expresses the fixed state policy. Weighted choices varying by block
or by chunk and Y layer express the other two Paper-style policies. The mod supplies the state
lists, height and dimension conditions, seed, treatment of missing faces, reveal radius and bypass.
Evaluating the terrain rules requires no worker call into the mod. [Paper's Anti-Xray modes](https://docs.papermc.io/paper/anti-xray/)

The server thread validates and prepares rule definitions and decides which view applies. Workers
receive immutable chunk snapshots and rules. Workers finish and cache the packet across
send passes. Players using the same rule snapshot share the bytes. Identical definitions
use the same snapshot. A packet with visible block entities gets its live tags when sent.
Nearby ready columns are prepared ahead of demand, with space reserved for regular sends.
Interior edits in a neighbour keep the cached packet when the faces the transform read and the
light occupancy mask still match. Bounded queues and caches can defer a send. A pending protected
column is never replaced by an ordinary body. Lighting, heightmaps and biomes describe the
authoritative terrain. The separate Horizon LOD service withholds replies for a protected dimension
because its top, under-surface and vertical span samples carry authoritative state IDs.

Replaced cells omit their original block-entity records. Block updates, prediction corrections
and block events use the selected projection. When a cell becomes visible, the server queues
its state before its tag. A normal block-entity update takes its transient fields once, after
finding entitled recipients, and fans the resulting tag out to those recipients. Initial and
replacement columns use ordinary snapshot tags. A successful authoritative player break can
reveal still-covered cells within the mod's configured Manhattan radius (up to four). The
server keeps up to 1,024 revealed positions for that view and reapplies their state and tags
after full resends. Terrain views apply to engine terrain packets. They do not infer hidden
content from application-defined payloads or other gameplay channels such as sounds.

The two additions are ordinary server Host calls generated for both native and WebAssembly mods.
They require a runtime that exports the calls. Existing Wasm modules continue to resolve their
own named imports. Native builds should use the SDK from their target runtime revision.

To check the example from the repository root, run
`cargo check --manifest-path tests/terrain-view-mod/Cargo.toml`. To build a packaged Wasm mod,
copy the fixture to a temporary directory inside this checkout's `target/` and run
`cargo horizon-mod build --wasm` there. The packager links the mod for Horizon's shared Wasm runtime.

## Existing paths and gaps

| Operation | Existing path | Why it needs the view |
| --- | --- | --- |
| Resident chest and movement or teleport | `Hub::update_chunks` takes ready columns for batches and calls the chunk encoder. The encoder writes sections before the chest's block-entity record. | `chunk_watch` and `chunk_sent` run after this network step and exclude the far ring. |
| Neighbour break, bulk edit and reveal | `Hub::broadcast_block_changes` groups the block log by section, sends state changes and builds tags. Bulk edits and `chunk_resend` go through full columns. | `block_changed` and block Parts observe authoritative writes. They lack per-recipient application and do not observe streaming admission, saved reads or every bulk storage write. `block_entity_update_tag` supplies the entity's data without any recipient, and a change tag consumes transient fields. |
| Direct correction | Use-on and aborted mining correct client prediction. The hub also corrects support blocks. | The existing `player_show_block` is an immediate mod-authored override. It cannot intercept engine sends or establish a consistent view. The interaction hooks describe gameplay decisions. |
| Saved and resident far ring | `lan/far.rs` reads a saved column on a reader. A resident far chunk uses the all-light body. | A protected reader needs a view-keyed result and immutable neighbouring faces. Nonloading reads and chunk load tickets do not control the bytes the reader returns. |
| Cross-dimension view | `viewed_levels/send.rs` encodes columns and wraps column and delta bodies in `horizon:levels` runs. | Its recipient's current dimension is independent of the dimension of the work. The same view key must be applied before wrapping. |
| Horizon LOD | `lod_net/server.rs` services a separate request queue. | Its coarse surface and span format contains block states even though it does not carry block-entity tags. |

The API injection catalog covers eligible `Game` methods, including gameplay actions. Those
points do not cover the lower-level `Hub` and saved-read worker and cannot establish stream-wide
ownership. The Host calls feed the hub directly, following the existing `player_show_block` and
`chunk_resend` forwarding pattern.
