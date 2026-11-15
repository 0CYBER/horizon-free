# Level block notifications and cracks

## Notify clients

`level.send_block_updated(host, pos)` tells clients to read the current block again.
It does not change that block or mark the chunk unsaved. Repeated notifications of
one position are coalesced until the update log is consumed. The request is recorded immediately,
including inside a transaction; aborting storage does not retract it. A stale,
parked, unloaded or out-of-height write returns `false` and queues nothing.

## Show breaking cracks

`level.set_break_progress(host, breaker, pos, stage)` broadcasts a server crack.
Use a stable negative breaker ID for a machine. Stages `0..=9` draw a crack;
any other stage clears it. Inside a transaction the packet waits for the outermost
commit and aborting sends nothing. This is distinct from
`level.set_client_break_progress(client_host, breaker, pos, stage)`, which draws
only in the local client's view and requires `&mut dyn ClientHost`, not a server
host. Local cracks expire after 400 ticks without an update and clear when the
client changes levels or disconnects.

## Read tool speed

`level.block_break_speed(host, pos, tool)` reads the tool's destroy speed on that
block, including its `minecraft:tool` component rules and default speed. It does
not include Efficiency, player attributes, effects, fluid/airborne penalties or
block hardness. Bare hands return `1.0`; an unavailable position returns `None`.
This read takes an immutable host and does not modify blocks or tools.

## Biome writes in transactions

`LevelRef::set_biome` changes a 4x4x4 cell. Reads inside the transaction see the tentative
cells. An abort restores them without publishing changes. Saving, remeshing and resending
wait for the outermost commit, once per column. An inner commit does not publish; its cells
still roll back if the outer transaction aborts. Outside a transaction these effects run
immediately.
