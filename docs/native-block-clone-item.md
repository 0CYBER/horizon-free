# Block pick results

The fork uses `BlockPart::clone_item_stack`, with the owning `ServerHost`, typed level,
position, state, and `include_data`. Return `None` to keep the ordinary game result.
The active block Part is queried once; the creative pick path then adds block-entity
components when requested. Local and guest packet paths use that same query.

Diverges from upstream: upstream's `block_clone_item` query is folded into this
existing hook, not exposed as a second interception point. See the modding book's
[Block Parts](modding/src/concepts/parts/block-parts.md).
