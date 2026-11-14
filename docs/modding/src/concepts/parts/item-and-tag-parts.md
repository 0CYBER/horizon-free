# Item and tag parts

For an item-specific Part, request its canonical registry key once in `register`:
`let stick = r.item_key("minecraft:stick"); r.item_part(stick, Sharp);`. Bare names use the
`minecraft:` namespace. Requests may refer to content declared later; the final item registry refuses
unknown IDs with the requesting mod. `ItemKey` equality uses its load-owned slot, and `key.id(host)`
reads its canonical diagnostic name. Item capability registration and item helpers use strings.

`r.block_part_tag("minecraft:logs", Rotates)` and `r.item_part_tag("minecraft:axes", Sharp)` attach a part to
every member of a tag (its name without `#`). Each game resolves them against its own tags when its world
opens, again on `/reload`, and when a client joins a server, so the window's game and its server thread each
answer from the tags they play with; a block that leaves the tag is no longer asked. A mod's parts on one
thing run in the order it registered them, by key or by tag. A part a tag gives a thing
that already has that kind of part warns, naming both mods, and the earlier mod's part stays. A tag that
doesn't exist warns. Opening a world fails, naming the mod, when a part is attached to a block or item the game
doesn't have. Scoped hooks run on a game's thread; one dispatched from any other thread (which no game hook
does) can't see tag parts, and warns once.

## Right click on a block

`Hooks::right_click_block` corresponds to NeoForge 26.3's
`PlayerInteractEvent.RightClickBlock`, on the clicking client and server, with
identical switches for native and wasm mods:

- `use_block = TriState::True` allows block use even while sneaking with an item
  in either hand. `Default` skips while sneaking with either hand full; `False` skips block use.
- `use_item = TriState::True` enters the item's `useOn` phase even with an empty
  hand or an item on cooldown. `Default` requires a held stack off cooldown;
  `False` skips this phase. Adventure placement restrictions apply.

A consuming block result ends the click before the item phase. For forced empty-hand
use, `use_item_on_block` fires with `phase = UsePhase::ItemAfterBlock` and
`click.stack = None`; a listener can consume that use. Without a listener answer,
there is no item Part or vanilla item behaviour to run and the empty use passes.
Canceling `right_click_block` skips both uses and substitutes `cancellation_result`.

## Instant item use

`ItemPart::use_item` receives `UseItemEvent { stack, hand, player, player_entity_id }`.
`hand: Hand` is the interaction hand of this click (`Hand::Main` or `Hand::Off`),
including air clicks with identical stacks in both hands. Read it directly when
choosing which held stack to shrink. `player` and `player_entity_id` name who clicked,
as `UseOnEvent` does: on a server the acting player's UUID and entity id (a guest's
own, never the host's), on a client this client's own player. They come with the
event, so a Part does not need `players().acting_player()` to be set while it runs. `living_used_item_hand` instead answers only during active item use and
cannot identify an instantaneous click. The Part runs on both sides with `HostSide`,
after the global right-click event and before the item's own use, on native and wasm.
Return `Some(InteractionResult::Success)` to consume and swing on the client,
`Some(SuccessServer)` to consume and swing on the server, or `Some(Consume)`
to consume without a swing. `Some(Fail)` skips this item's own use without
swinging and allows the off-hand sequence to proceed; unlike a block-use Fail,
it does not end the entire click.
`None` (the default) lets later Parts answer; if none answers, the item's own use
runs. `Some(Pass)` stops dispatch to later Parts and continues directly to the
item's own use.
`TryWithEmptyHand` belongs to block use and is treated as Pass here.
A thrown item can return Success on the client and Consume on the server.
The hand field corresponds to NeoForge's
`PlayerInteractEvent.RightClickItem.getHand()`.

`Hooks::right_click_item` runs before `Item.use`, including after the same hand's
unhandled block or entity interaction. A block success or FAIL, or an entity success, stops fallback.
The server fires it for the separate use-item packet, once for that packet's hand.
Canceling skips the item Part and vanilla action on native and wasm alike.
`Success`, `SuccessServer` and `Consume` end the client's hand loop.
`Success` swings on the client, `SuccessServer` on the server, and `Consume` has no swing.

## Item use on a block

`Hooks::use_item_on_block` (`UseItemOnBlockEvent`) runs on native and wasm with
`HostSide` before each selected phase: `ItemBeforeBlock`, `Block`, then
`ItemAfterBlock`. `event.click` describes the hit, hand, held stack and player.
Return `Flow::Cancel` to skip the selected phase's Part and vanilla behaviour,
using `*event.cancellation_result` (initially `TryWithEmptyHand`) as its result.

Canceling `ItemBeforeBlock` skips the item's `on_item_use_first` Part. `Pass`
continues to the block phase; any non-`Pass` result ends the block interaction. Cancellation alone does not suppress the later phases when its answer is `Pass`. A block-phase `TryWithEmptyHand`
can run the block's empty-hand behaviour on the main hand.

`ItemPart::use_on`, `Hooks::entity_interact` and `ItemPart::item_interact_mob`
use version 2, run on both logical sides, and expose native `HostSide`.
Their mutable `result` is `Option<InteractionResult>`: `None` defers to later
listeners and the ordinary behaviour; `Some(result)` supplies the replacement
answer. `item_interact_mob` also carries the interaction `hand`.
`ItemPart::use_item` keeps its existing both-side, version-2 contract.

Client execution predicts hands only. Client hosts allow reads and local effects,
not authoritative world, inventory or entity writes; perform those on the server.
Answer on the client as well when prediction must stop the hand loop: an answer
made only on the server cannot suppress an offhand attempt already predicted by
the client.

`Success`, `SuccessServer` and `Consume` consume the interaction and end the
client's hand loop. Only `Success` swings on the client; `SuccessServer` swings
on the server and `Consume` never swings. `Fail` ends block interaction only,
not an entity or air-use hand loop. `Pass` and `TryWithEmptyHand` fall through
(the latter can select the block's empty-hand behaviour at the block phase).

## Item wear

`ItemPart::damage_item` is `IItemExtension.damageItem`: the wear a stack of the part's item is about to take
(`ItemStack.hurtAndBreak`), after Unbreaking, as NeoForge passes it. The part reads `event.amount`, the points
about to be lost, and changes it (0 or less leaves the item whole) or returns `Flow::Cancel` for no wear at all.
`event.entity` is who wears or uses the item (`None` for a dispenser's tool), and setting `event.break_now` breaks
the item through the vanilla break whatever durability it has left, as NeoForge's `onBroken` does. It
runs on every wear path: a player's tools, armour, shield and mace, a tool a dispenser uses, and a mob's armour. It is
not asked when Unbreaking already saved the item or for a creative player's tools, and an item with no part costs
one table lookup. With no part, the vanilla amount applies.

To change wear without attaching a Part to an item, use an enchantment's
`minecraft:item_damage` effect (the component Unbreaking uses), or an injection.
A part does not get the wearing entity;
`host.players().acting_player()` names the player on the paths a player takes.

## Mending repair

`ItemPart::xp_repair_ratio` is `IItemExtension.getXpRepairRatio`: the durability points Mending restores to a
stack of the part's item per point of experience (`ExperienceOrb.repairPlayerItems`), asked on the server once for
each item it repairs, from an orb or from `PlayerRef::give_experience_points`. `event.ratio` arrives as vanilla's:
the item's `minecraft:repair_with_xp` repair divided by `event.experience`, 2.0 for vanilla Mending. The repair
becomes `floor(experience × ratio)`, capped at the item's damage, and the experience it doesn't use goes on to the
next damaged Mending item, then to the player. A ratio left as it arrived keeps vanilla's repair exactly. One that
isn't finite is refused (vanilla's repair is used, with one warning per item), and a negative one counts as 0; a
ratio that takes a positive repair to 0 leaves the experience for the player. The `Host` is read-only.

To change Mending for every item at once,
change the enchantment's `minecraft:repair_with_xp` effect in a data pack; the ratio a part gets arrives with it. To
change it for particular items, vanilla's or another mod's included, attach a part to their item keys.

## Hand, break and name on the client

Three client-side methods change how the local player's client shows and handles a held stack. Each `event` field
arrives holding vanilla's answer, which the part may set either way, and an item with no such part costs one table
lookup.

- `ItemPart::should_cause_reequip_animation` is `IItemExtension.shouldCauseReequipAnimation`: whether the
  first-person hand lowers and rises again to show `event.stack`, the stack the hand holds now, in place of
  `event.old`, the stack it shows. `event.slot_changed` says the stack came from another slot (always false for the
  off hand). `event.reequip` starts `false` for the same stack changed in place or an equal one but for its damage.
  It is asked each client tick, for each hand, while the shown and held stacks differ; an empty hand isn't asked.
- `ItemPart::should_cause_block_break_reset` is `IItemExtension.shouldCauseBlockBreakReset`: whether a break
  restarts because the main hand changed. `event.stack` is the stack the break began with and `event.held` the main
  hand now. `event.reset` starts `true` when they aren't the same item with the same components (count and damage
  left out). `false` keeps the break and its progress; `true` sends the abort and a new start, from 0. It is asked on
  each tick of a break of the same block, in single player and on a server's client. A `true` for an unchanged stack
  restarts the break every tick, so it never finishes.
- `ItemPart::highlight_tip` is `IItemExtension.getHighlightTip`: the name drawn above the hotbar for the selected
  stack. `event.name` arrives as vanilla draws it (the hover name in its rarity colour, italic when renamed), and the
  text the part leaves is drawn at the same place and fade. It is asked once a client tick while the name shows. The
  name's timer still restarts only when the item or its hover name changes, so a tip that changes every tick doesn't
  keep it up.
