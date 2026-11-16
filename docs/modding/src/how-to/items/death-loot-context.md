# Death sources and weapons at the loot hook

`LootDrops::damage_source` carries the death's damage type and causing/direct entity IDs.
Non-death drops have no source. Environmental deaths retain their type, with 0 for
absent or unknown IDs. `player` is remembered kill credit, not necessarily the attacker.

`context.weapon_item(host)` is the death-loot read corresponding to vanilla's
`DamageSource.getWeaponItem`. Native and wasm listeners use the same read and retain
all stack components, including custom names, enchantments and durability.

| Death source | Weapon returned |
| --- | --- |
| Bow arrow | The bow recorded when the arrow was fired |
| Crossbow arrow | The crossbow recorded when the arrow was fired |
| Thrown trident | The projectile's own original trident stack |
| Player in a Riptide spin | The stack recorded when the spin started |
| Other living direct entity | Its live main hand when the listener asks |
| Absent or unavailable weapon, or non-death drops | `None` |

Projectile weapons are captured only when damage is accepted and remain bound to that
accepted source. A later refused hit cannot replace them. They survive the projectile's
removal before loot runs. A bow named
“Longbow” with Power II reads as that bow after its shooter switches to a stick.
Melee remains live: a diamond sword with Sharpness V reads as an axe if the attacker
switches to the axe before the read. Earlier loot listeners' hand edits are visible.

`tool` is the breaking or using tool (`LootContextParams.TOOL`), and is always `None`
for entity deaths. For the causing entity's current hand independently of the damage
source's weapon, resolve `damage_source.causing` with `host.entity(id)`, then
`host.living(entity)`, and call `living.hand_item(host, Hand::Main)`. Handle resolution
can fail for an absent attacker. A shooter's current hand and its projectile's
recorded weapon are separate values.

Projectile direct IDs are 0; melee direct IDs equal the causing ID.
The recorded projectile weapon is carried separately in the call-scoped death context.

The live fallback follows `damage_source.direct`, rather than the causing entity. A
harming potion has a nonliving direct entity and therefore never returns its thrower's
held sword. If that direct entity is unavailable, the read returns `None`.
