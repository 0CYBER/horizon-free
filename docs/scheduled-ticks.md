# Horizon scheduled ticks

The Rust scheduler targets the repository-pinned **Minecraft Java Edition
26.3**. It implements scheduled block/fluid queue mechanics and saved tick
lists. Actual block/fluid updates, random ticks, block events, neighbor updates,
redstone behavior and a running game world remain unimplemented.

## Behavior

`horizon_core::ticks` supplies `TickPriority`, `ScheduledTick`, `SavedTick`,
`ChunkTicks` and `LevelTicks`. A numeric `TickTypeId` represents a registry-owned
block or fluid type. Block and fluid scheduling use separate `LevelTicks`
instances. Callers supply identities, game time, sub-tick counters, eligibility,
per-call tick limits and behavior callbacks.

The first request for a type and position wins, even if a duplicate has an
earlier trigger time or a higher priority. A chunk queue orders by signed
trigger time, then one of seven priorities, then signed sub-tick order. Once
heads are due, **different chunks compete by priority and sub-tick order**.
Consequently, globally sorting every tick by trigger time changes execution.
Within the selected chunk, draining stops when its next head loses to the
competing chunk head, exceeds the time bound, or exhausts the tick limit.

Equal comparator keys follow the pinned Java array heap behavior. Deadline
traversal follows the pinned fastutil long-map layout, including resizing,
collision shifts and wrapped iterator removals. These details matter when
loaded ticks in different chunks share priority and negative sub-tick order.
They are covered by differential tests rather than replaced with a new tie
breaker. The heap stores tick records inline and retains collection capacity
between frames. Area operations probe small chunk grids directly and visit
only loaded chunks for large sparse areas, preserving X/Z traversal order.
No game FPS or memory gain has been measured.

Collection happens before callbacks. A callback can schedule, query, clear,
copy, disable or unload chunks; newly scheduled ticks wait for the next
collection. `has_scheduled` covers container queues and pending saved ticks;
`will_tick_this_tick` covers the remaining collected queue. The current callback
has already moved to the already-run list. Vanilla's lazily built membership
set can remain stale after an area clear; that behavior is preserved until
cleanup. Area copying includes already-run, collected and queued ticks and
places copied sub-tick orders after the copied source range with wrapping
signed-long arithmetic.

Disabled chunks retain due entries. Removing a container returns its queue
for saving or later reinsertion; collected callbacks still run. Scheduling into
an absent chunk returns `MissingChunk` and does not enqueue. The Rust API adds
invariant checks for duplicate chunk insertion, wrong-chunk contents and invalid
collection phase transitions. These checks differ from Java's unchecked API
and logging, while preserving the tested valid lifecycle.

## Persistence and use

Saved ticks hold a signed 32-bit relative delay. Packing narrows the wrapped
time difference as Java does and stably sorts active ticks by sub-tick order.
Pending saved entries precede active entries. Unpacking assigns orders from
`-saved_count` through `-1` and adds delays with signed-long wrapping. Vanilla
does not deduplicate a saved list during unpack; duplicate entries and their
membership behavior are retained.

`horizon_world::tick_data` bridges the vanilla `i`, `x`, `y`, `z`, `t`, `p`
fields to type IDs through caller-supplied names. `ChunkData::saved_ticks`
loads a pending container and filters out positions belonging to other chunks;
`set_saved_ticks` writes packed entries while preserving unrelated root data.
The separate block/fluid name lookup is required because their IDs are not
the resident section's block-state IDs.

```rust
use horizon_core::{position::ChunkPos, ticks::{LevelTicks, ScheduledTick}};
use horizon_world::{chunk::ChunkData, tick_data::TickKind};

fn run_saved_blocks(
    chunk: &mut ChunkData,
    now: i64,
    resolve: impl FnMut(&str) -> Option<horizon_core::ticks::TickTypeId>,
    name: impl FnMut(horizon_core::ticks::TickTypeId) -> Option<String>,
    run: impl FnMut(&mut LevelTicks, ScheduledTick),
) -> Result<(), Box<dyn std::error::Error>> {
    let pos: ChunkPos = chunk.position();
    let mut pending = chunk.saved_ticks(TickKind::Block, resolve)?;
    pending.unpack(now);
    let mut level = LevelTicks::default();
    level.add_chunk(pos, pending)?;
    level.tick(now, 65_536, |_| true, run)?;
    let pending = level.remove_chunk(pos).unwrap();
    chunk.set_saved_ticks(TickKind::Block, &pending.pack(now), name)?;
    Ok(())
}
```

The caller supplies the behavior callback. A game integration must check the
current block/fluid type, invoke its actual behavior,
apply resulting state/neighbor/light changes, and persist remaining queues
through the chunk lifecycle. The scheduler alone does not provide gameplay.

## Validation

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
python3 tools/test_tick_parity.py --prepare
```

The runner authenticates both official reference JARs and dependencies against
the repository lock and bundled hashes. An original Java adapter invokes the
actual `LevelTicks`, `LevelChunkTicks`, `ScheduledTick`, `SavedTick` and
`TickPriority` classes; expected traces are not computed by another port.

The corpus has **25,815 commands**, including **2,096 tick invocations**, and
produces **97,776 matching output lines per reference JAR**. It covers:

- All priorities, overdue chunk competition, exact comparator ties, duplicate
  requests and signed time/order extremes.
- Map growth/shrink, collisions crossing the table boundary, limited draining,
  disabled chunks, unloading and restoration.
- Pending-list duplicates, save packing, negative load orders and repeated
  save/reload with changed game time.
- Membership queries and rescheduling, clearing, copying, unloading or
  disabling chunks during callbacks.
- Eight deterministic randomized scenarios with queue snapshots, saves,
  eligibility changes and area operations.

A separate adapter uses the actual block and fluid `SavedTick` registry codecs.
Rust loads, filters, unpacks, polls, reschedules and saves **251 block ticks and
251 fluid ticks**. Vanilla checks the resulting NBT and a Rust-written region
chunk. Fixtures include duplicate saved entries, out-of-chunk coordinates,
negative delays, overflowing times and numeric NBT coercions including NaNs,
infinities and narrowed longs.

Offline tests cover phase guards, eligibility, callback deferral, ordering,
area membership, copying, invalid data and atomic saved-list updates. CI runs
this suite alongside core and world-data comparisons. Evidence stays under
`.local/rust-tick-parity/`; success reports are cleared before each run and
source hashes must remain unchanged during validation. The checked-in
[scoped report](../benchmarks/rust-ticks-2026-10-05/scoped-parity.json) contains
metadata and hashes only. Line counts are corpus observations, not gameplay
coverage percentages or proof of complete vanilla parity.
