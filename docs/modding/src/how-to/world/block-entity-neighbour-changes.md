# Block entity neighbour changes

A block's `BlockPart::on_neighbor_change` receives the changed block entity's
position in `neighbor`. On authoritative servers, the comparator invalidation
walk visits the four horizontal neighbours, skipping unloaded adjacent chunks.
Vanilla comparator processing at each cell runs before its part is notified.
This is separate from `neighbor_changed`, the vanilla block neighbour callback.

A block one cell farther through a redstone conductor receives this notice only
when its `weak_changes` part sets `receive` to true (initially false). This query
belongs to the farther block. Reading that cell never loads a chunk.

These correspond to NeoForge's `IBlockExtension.onNeighborChange` and
`getWeakChanges`. With no parts, vanilla's comparator rules are unchanged.
