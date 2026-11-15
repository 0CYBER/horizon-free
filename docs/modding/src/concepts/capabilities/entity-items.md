# Entity items

Use `entity.capability(host, &ENTITY_ITEMS, ())` to access an entity's items as a
`ResourceHandler<ItemResource>`. This is NeoForge's `Capabilities.Item.ENTITY`.
A stale handle or an entity outside the active level returns `None`. A retained
game handler cannot read or change items after its entity is removed or its world closes.

## Game providers

| Entity | Inventory exposed |
| --- | --- |
| Player | Main inventory, armour and off hand through `ItemPlace::PlayerSlot`. |
| Chest or hopper minecart, chest boat, chested donkey, mule or llama | Container slots through `ItemPlace::EntitySlot`. |
| Living non-player entity without a container, including an armour stand | Equipment in `EquipmentSlot` ordinal order, through `ItemPlace::Equipment`. |

Player slot indices are half-open ranges:

| Indices | Slots |
| --- | --- |
| `0..36` | Main inventory; `0..9` is the hotbar. |
| `36..40` | Armour, feet to head. |
| `40` | Off hand. |

An armour slot accepts one item whose `equippable` component permits that slot
and the player's type, by ID or tag. Bound armour can be removed only in creative.
`insert_any` merges matching stacks in the selected slot, off hand and main
inventory, then uses empty main-inventory slots; it does not fill armour slots.

For non-player equipment, hands accept a stack. Armour and body slots accept
one item whose `equippable` permits the slot and entity type.

## Custom providers and transactions

Register with `r.entity_capability(&ENTITY_ITEMS, "mymod:robot", |host, robot, ()| …)`.
Providers registered with `r.entity_capability_before_game` run before the game's
provider; ordinary providers run after it. Within each group, registration order
applies and the first handler returned wins.

Give, take and count player items through this capability. For example, this
trade aborts if an offer changes or either recipient lacks room:

```rust,ignore
// A trade: each offer is (index, the exact stack it must still hold). A stale offer or no
// room for one aborts the transaction, and every move in it is put back.
let (a, b) = (seller.capability(host, &ENTITY_ITEMS, ())?, buyer.capability(host, &ENTITY_ITEMS, ())?);
host.transaction(|host, tx| {
    for (from, to, offer) in [(&a, &b, &seller_offer), (&b, &a, &buyer_offer)] {
        for (index, stack) in offer {
            let offered = ItemResource::of(stack);
            let still = from.resource(host, *index).as_ref() == Some(&offered)
                && from.amount(host, *index) == stack.count();
            if !still
                || from.extract(host, tx, *index, &offered, stack.count()) != stack.count()
                || to.insert_any(host, tx, &offered, stack.count()) != stack.count()
            {
                return Err(());
            }
        }
    }
    Ok(())
})
```

Cauldron interactions outside a transaction retain vanilla's effect order.
Inside a transaction, statistics (including `player_stat_award`), sounds and
overflow item tosses or spawns wait for the outermost successful commit. An abort
emits none of those effects.
