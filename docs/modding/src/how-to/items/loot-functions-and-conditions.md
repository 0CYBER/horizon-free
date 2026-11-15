# Custom loot functions and conditions

A loot table's function or condition that names a `type` in a namespace outside `minecraft:` reaches `loot_function(kind, data, ctx, stack, tags)` or
`loot_condition(kind, data, ctx, holds, tags)` when the table rolls. `data` is a borrowed `serde_json::Value`; `tags` is the authoritative
read-only `&dyn Tags` for that roll. See [Reading tags](loot-tags.md). The function changes the stack; the
condition says whether it holds. The context brings the entity, the damage, the player who killed it, the
tool, the block, the luck, the biome and a seed from the roll's random. An unclaimed function leaves the
stack, an unclaimed condition fails, as unknown kinds do; a bare type name is minecraft's own.

The tables the items engine rolls (block drops, entity drops, `loot_table`, `interact_loot` and the
built-in tables) answer both hooks, with the tool and the block in the context. The function hook there
gets an owned API `LootStack`: its resolved `ItemKey`, signed intermediate count, damage, enchantments
(a book's stored ones), potion, ominous bottle amplifier, stew effects and ordered NBT component patches.
Each patch is `(String, Option<Tag>)`; `None` removes a component, and repeated IDs apply in order.
Unknown NBT values retain their exact type, including byte arrays and bytes. Names are API `Text` trees.
The context is an owned `LootContext`, including entity equipment and deferred exploration-map facts.
Conversion runs only after the subscription guard. Writeback validates the entire candidate before replacing
the engine roll; an invalid candidate leaves the roll unchanged. Intermediate counts can be non-positive:
these values are not occupied inventory stacks. A function that asks for a map's structure search (`map`/`exploration`) changes nothing there: the
items engine has no level to search. In the items engine's context the `entity`, `damage`, `player` and
`biome` fields are `None` and `luck` is 0; in the main loot engine's context the `tool` and `block` fields
are `None`.

Both engines' contexts carry `place`, where the roll happens: `origin` (a broken block's centre, a dead mob's
position), `block_entity` (the broken block's entity as it was before the break, saved with its id, position and
`Items`) and `level` (the dimension, the game time, the time of day, the rain and the thunder, for conditions like
vanilla's `time_check` and `weather_check`). The game sets it for a block a player or an explosion breaks and for a
mob's death loot; on other rolls `place` is `None`.

`queried_table_id` reports the caller's first supplied id, or
`neoforge:unknown_loot_table` until the root and every nested table finish.
Only the root then sets its id (an unnamed root sets the fallback), before the
future global modifier pass (arismoko/horizon#1270). Nested tables share all parameters and
never write the id. Consequently `neoforge:loot_table_id` inside a table normally
matches the fallback, rather than the root's name.

`queried_stack` is the stack whose data is being queried, distinct from the tool.
Existing engine callers leave it absent; nested tables preserve it unchanged.
Both fields cross the native and wasm loot callback boundary.

`neoforge:can_item_perform_ability` requires one string field, `ability`.
Names are plain and case-sensitive: `shears_dig` is valid; `minecraft:shears_dig`
never holds. Unknown names load without error and have no default ability.
The condition reads only `tool`; without one it is false. Tables without a tool
parameter continue to load as before. A part's answer replaces the item-layer
default, and a part that leaves the answer unchanged keeps that default.

Defaults are `sword_sweep` from `minecraft:swords`, `shovel_douse` from
`minecraft:douses_campfires`, the six shears abilities (`shears_dig`,
`shears_harvest`, `shears_remove_armor`, `shears_carve`, `shears_disarm`,
`shears_trim`), `fishing_rod_cast`, `trident_throw`, `brush_brush`,
`firestarter_light` for flint and steel or fire charge, and `spyglass_scope`.
All other defaults are false. Existing action sites retain their own vanilla
answers until arismoko/horizon#3330 adopts the shared table.

Engine callers can scope a shared `horizon_world::loot::query::LootQuery` with
`with_query`. Call `set_id` before rolling to supply the first id, and read
`table_id()` afterwards to inspect the completed root. Set `queried_stack` before
sharing the query; normal loot callers do not query an item's own data.
