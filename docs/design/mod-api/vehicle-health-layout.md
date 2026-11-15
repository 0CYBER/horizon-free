# Design: vehicle health layout

Provisional implementation contract for arismoko/horizon#2741; records the provisional answers A and B, with vanilla default sizing preserved.
Approval admits this note only; public counterpart verification remains a shipping gate.
Mods supply effective maximum **hearts**, not rows. The engine prepares one immutable
result per visible HUD frame, before reserving space or drawing any status layer.
Reservation, vehicle drawing and the player's status layout consume that same result.
Cancelling a layer changes drawing only.

## Current owners

References are against main at `d57e01504e`.

- `crates/horizon-client/src/game/mobs/mounts.rs:951`: `draw_mount_hud` resolves
  `mount_index`, computes capacity, constructs vehicle draws, calls `keep_status_rows`
  before `gui_layer`, then returns capacity even when drawing was cancelled.
- `crates/horizon-client/src/game.rs:8770`: initializes the frame's HUD room;
  `game.rs:8867` calls mount drawing except for spectators, then passes its returned
  `vehicle_hearts` into player layout at `game.rs:8903`. This shared client presentation
  route serves single player, LAN and remote servers; dedicated servers draw no HUD.
- `crates/horizon-client/src/game/interface/hud_room.rs:59`: reserves status keep-outs
  from draw geometry, independently of whether the layer later draws.
- `crates/horizon-client-render/src/hud_layout.rs:473`: default capacity;
  `hud_layout.rs:478`: ten containers per row, starting at `h - 39`, stepping up
  ten pixels; `hud_layout.rs:674` selects food using zero vehicle capacity;
  `hud_layout.rs:704` offsets the player's air row using vehicle row count.
  Player health hearts themselves need no new vertical displacement.
- `crates/horizon-mod-vocab/src/client/hud_layout.rs:143`: `HealthInputs.vehicle_hearts`
  is the vocabulary crossing to player layout. The exports live in
  `crates/horizon-mod-vocab/src/exports/hud_layout.rs:1`.
- `crates/horizon-client/src/game/native_hooks/shared.rs:2202`: `gui_layer` runs
  preceding mod layers, Pre, conditional drawing, then Post. Today Post runs even
  when Pre cancels. Preserve that existing behavior pending verified counterpart evidence.

PR arismoko/horizon#2697 moves vocabulary `hud_layout.rs`; PR arismoko/horizon#2586 touches `mounts.rs`. Re-resolve
these owners after they land; do not create a parallel vocabulary or mount path.

## Vanilla and public counterpart evidence

Read vanilla 26.3 with these reproducible queries from the repository containing
prepared vanilla resources:

```sh
python3 tools/rea_vanilla.py method net.minecraft.client.gui.Hud getVehicleMaxHearts
python3 tools/rea_vanilla.py method net.minecraft.client.gui.Hud getVisibleVehicleHeartRows
python3 tools/rea_vanilla.py callers net.minecraft.client.gui.Hud getVehicleMaxHearts
python3 tools/rea_vanilla.py callers net.minecraft.client.gui.Hud getVisibleVehicleHeartRows
```

The private `Hud.getVehicleMaxHearts(LivingEntity)` returns zero for null or a
vehicle not showing health; otherwise `min(30, ((int)(maxHealth + 0.5f)) / 2)`.
The private `getVisibleVehicleHeartRows(int)` returns `ceil(hearts / 10.0)`.
These are sizing helpers, not public mod hooks. Thirty health therefore means
15 hearts and two rows; 8 current health fills four containers.

Vanilla 26.3 registers `minecraft:max_health` with default **20.0**, minimum
**1.0** and maximum **1024.0** health. The prepared vanilla `attributes.json`
records these as raw f64 bits `4034000000000000`, `3ff0000000000000` and
`4090000000000000`, respectively. These registered bounds are readable through
Horizon's attribute table: `crates/horizon-world/src/attribute_health.rs:79`
resolves `minecraft:max_health`, and `attribute_health.rs:114` reads its value.
At two health per heart, the allowed effective capacity is **0.5–512 hearts**.
The default HUD's 30-heart display cap is not the attribute maximum and must not
restrict mod output.

Public-only checks of the [NeoForge GUI layer documentation](https://docs.neoforged.net/docs/gui/layers/)
and the 26.3 `RenderGuiLayerEvent.Pre` javadoc were inaccessible. A named sizing counterpart, layer name and cancellation/Post
contract are **unverified**. No NeoForge source or patches were consulted.
If public 26.3 documentation establishes a counterpart at this sizing moment,
adopt its name, side and full contract before shipping; do not add an alias.

## Proposed preparation contract

Provisional name: `vehicle_health_layout`, client side, with `&dyn ClientHost`.
This is a sizing callback, without DrawContext or cancellation. Inputs are the
existing lifecycle-owned vehicle handle, current health, actual maximum health,
vanilla default heart capacity and frame dimensions. No second identity generation.
Output is a mutable f64 `effective_max_hearts`, measured in hearts and initialized
to the exact vanilla default, including zero at maximum health 1.0. Dependency-ordered listeners see prior accepted output; the last
accepted value wins. They cannot independently change rows or real entity health.

Resolve eligibility once after HUD visibility is known and before the first status
reservation or mod-layer drawing. No mount, spectator or hidden HUD prepares zero
without firing. Recheck the selected handle before applying; if retired, refuse
that preparation and use zero for all consumers for that frame, without recomputing.
Cache only for that frame, never across riding changes or HUD visibility changes.
Move preparation into a sibling module; keep oversized `game.rs` call-site edits
net zero lines.

Validate explicitly supplied callback output at the boundary; an unchanged output
retains the previous answer, including a zero vanilla default. Refuse NaN and either infinity,
retaining the previous validated capacity; clamp every finite answer to exactly
**[0.5, 512.0] hearts** (equivalently [1.0, 1024.0] health). Zero and negative
answers therefore become 0.5; hiding drawing remains the layer Pre hook's job.
Apply the same policy after native callbacks and wasm Wire decoding.

Keep fractional capacity separate from whole sprite containers: a half-heart
capacity occupies one container. Compute occupied containers as `ceil(capacity)`
and rows with vanilla's `ceil(capacity / 10.0)` arithmetic, once. Thus 0.5 hearts
means one container and one row, while 512 means 512 containers and 52 rows.
Vanilla's current-fill arithmetic uses `ceil(health)` health points and full/half
sprite comparisons; 1.0 current health paints a half heart in that one container.
This fractional-capacity rounding deliberately differs from the private default
`getVehicleMaxHearts` integer division quoted above, which returns zero for 1.0
maximum health. Preserve that helper's exact default sizing whenever no listener
changes output. Fractional capacity is reserved for explicitly supplied listener
output; attribute bounds do not impose a minimum on vanilla HUD containers.
Retain vanilla's 30-heart default display cap while allowing mod output to 512.
Clamp displayed current fill to capacity without altering entity health. A
panic/trap discards that listener's output and retains the previous validated
value, using the runtime's existing fault isolation and diagnostics.

The prepared record contains effective hearts, derived rows, selected vehicle and
health snapshot. Generate vehicle geometry and the status keep-out from it; pass its
occupied-container count to `HealthInputs`, whose integer field yields the
same row count. Retain fractional capacity in the prepared record for fill clipping. No consumer rereads max health or calls sizing
again. Pre and Post may inspect the prepared sizing but cannot mutate it. A Pre
cancellation leaves the keep-out and player placement unchanged. Custom redraws can
use the prepared geometry. Post follows the existing layer dispatcher contract.

Native and wasm use the same catalog declaration and API-owned Wire input/output
records, generated adapters and validation. Unsupported capability registration
must refuse with a named `UnsupportedCapability(vehicle_health_layout)` error;
never silently register an inert callback. The no-subscriber path performs one
O(1) subscriber-presence check, uses exact vanilla sizing without an unconditional half-heart minimum and creates no event,
serialization, listener scan or extra allocation. Existing draw allocation is
unchanged. With subscribers, dispatch only to the indexed subscriber list.

## Acceptance matrix

For air-row examples let GUI height be 240. With a vehicle the food row is skipped,
so the air baseline is `h - 49` = 191, and the air row is drawn at `191 - (rows - 1) * 10`.

| Case | Prepared hearts | Rows | Observable expectation |
| --- | ---: | ---: | --- |
| No listener or unchanged listener output, maximum health 1.0 | 0 | 0 | No vehicle containers; vanilla food and air layout |
| No listener, maximum health 30, current 8 | 15 | 2 | 15 containers, 4 full; vehicle rows at 201 and 191; air at 181 |
| Same mount, mod supplies 4 | 4 | 1 | 4 containers, all full; air at 191; one-row reservation |
| Mod supplies 10 | 10 | 1 | 10 containers at 201; air at 191 |
| Mod supplies 11 | 11 | 2 | 10 containers at 201, 1 at 191; air at 181 |
| 11 hearts, Pre cancels drawing | 11 | 2 | No vehicle sprites; reservation unchanged; air remains at 181 |
| Listener explicitly supplies 0.5 at 1.0 max/current health | 0.5 | 1 | 1 container with half-heart fill; air at 191 |
| Maximum: 1024.0 health, mod capacity 512 | 512 | 52 | 512 containers: 51 rows of 10 and final row of 2; air at -319 |
| Mod supplies 0 | 0.5 | 1 | Clamp to minimum; 1 container; air at 191 |
| Mod supplies -1 | 0.5 | 1 | Clamp to minimum, for native and wasm |
| Mod supplies 2000 | 512 | 52 | Clamp to maximum; same reservation and player air placement as 512 |
| Mod supplies 512.5 | 512 | 52 | Clamp to maximum |
| Mod supplies NaN after valid 4 | 4 | 1 | Refuse answer; retain 4; air at 191 |
| Mod supplies positive or negative infinity after valid 4 | 4 | 1 | Refuse answer; retain 4; air at 191 |
| Two listeners supply 4 then 11 | 11 | 2 | All three consumers use 11, not the intermediate 4 |
| Listener supplies 4 then another panics/traps | 4 | 1 | Prior validated capacity retained |
| No mount, spectator, or hidden HUD | 0 | 0 | No sizing dispatch or vehicle reservation |

Test malformed wasm output followed by the built-in layout,
frame-to-frame riding changes, one invocation per frame, and no-listener equivalence.

## Stages

1. Verify the public NeoForge counterpart and settle the provisional name. Use
   the registered attribute bounds above for implementation.
2. Add the shared preparation record, catalog/Wire contract and both adapters;
   update the three consumers together, after reconciling arismoko/horizon#2697 and arismoko/horizon#2586.
3. Add focused layout and native/wasm dispatch tests for the matrix, then regenerate
   hook and boundary documentation. This proposal itself changes documentation only.

## Open questions

- Does public NeoForge 26.3 name this sizing moment? Recommendation: retain the
  provisional name only if no counterpart exists; public evidence decides naming
  and any required layer dispatch correction.
