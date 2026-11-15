# Custom enchantment effect kinds

Implement `Hooks::enchant_effect` to handle custom `minecraft:post_attack` effects
when a player's weapon hits a mob. The hook runs on the server for each matching
entry, including each mob hit by a sweep.

An entry dispatches when:

- `enchanted` is `attacker` and `affected` is `victim`.
- `effect.type` has a namespace other than `minecraft:`. A bare type name uses
  the `minecraft` namespace.
- The entry's `requirements` pass.

The event provides the enchantment ID, effect kind, borrowed `Tag` payload,
enchantment level and victim mob ID. The mob may have died from the hit. The
acting player holds the enchanted weapon.

Other components and attacker/victim pairs, hits on players and entries nested
inside `minecraft:all_of` do not dispatch this hook. An unclaimed kind does
nothing. Payloads are converted to retained NBT when definitions load; invalid
payloads refuse the definition rather than failing during a hit.
