# Item pickup notices

Listen to two server hooks to track items a player receives from the ground. Each pickup fires at
most one notice:

- `item_picked_up_post` (NeoForge's `ItemEntityPickupEvent.Post`): the whole stack went into the
  inventory.
- `item_picked_up_partial` (Horizon only): the inventory took part of the stack and the rest stayed
  on the ground, as when 8 apples lie there and only 5 fit.

Neither fires when `item_picked_up` refused the pickup or the inventory took nothing. A creative
player always takes the whole stack, so only `item_picked_up_post` fires for them.

Both carry the player's UUID, `original`, a copy of the stack as it lay, and `remaining`, the item
entity's live remainder: `None` after a whole pickup, the part that did not fit after a partial one.
When the notice arrives, the amount the inventory took is `original`'s count less `remaining`'s.
Set `remaining` to change what stays on the ground, or `None` to empty the entity; that changes the
ground, not what went into the inventory, so read the amount before editing `remaining`. Removing the entity in the listener keeps it removed, for
the host's player and LAN guests alike: there is no take animation, the stat and criterion
run.

For a quest counter or custom pickup sound, listen to both hooks and count the amount as it stood when the notice arrived: after any listener edits
`remaining`, `original` less `remaining` need not equal what the inventory took.
