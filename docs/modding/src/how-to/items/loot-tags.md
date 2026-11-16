# Reading tags in custom loot evaluation

`LootFunctionEvent` and `LootConditionEvent` carry `tags: &dyn Tags` alongside
their owned context and candidate snapshots. This is a read-only view of the
merged tags used by the current loot evaluator, not a full `Host`.

Resolve an `Identifier` with `event.tags.key(&id)`, retain the resulting `TagKey`,
and call `is_in_tag(item, tag)` for item membership. `items(tag)`, `blocks(tag)`
and `fluids(tag)` return ordered members by key. There is no operation to
enumerate every tag. Unknown tags return `Some(false)` or `Some(Vec::new())`;
unavailable key mappings return `None`. A retained key keeps its identity across
reloads, but its members are read from the current evaluation's tables.

The same Tags resource crosses Wire for native and wasm callbacks. Neither
callback gains a mutable game host or an opportunity to change tags. No tag
snapshot is copied when a loot hook has no subscribers.
