# Menu slots

## Mod-created recipe input sets

`Hooks::recipe_property_sets` runs once after recipes resolve for each load and again after
`/reload`. Add `RecipePropertyAddition::new("mymod:cooking_pot_input", items)` to its
`event.additions`: a key in the listener's manifest-owned namespaces creates a set. Ownership
uses the package ID plus the manifest's `namespaces`, so a package named
`ported_farmersdelight` with `namespaces = ["farmersdelight"]` can create
`farmersdelight:cooking_pot_input`. A later listener may extend an existing set but cannot
create a new foreign key. Invalid keys, unknown vanilla keys and air are skipped with a
warning; unregistered items do not participate in acceptance or network sync.

Attach a `MenuPart` to the registered menu type and filter its input slot with the same set:

```rust,ignore
struct PotInput;
#[mod_api::horizon_part]
impl mod_api::MenuPart for PotInput {
    fn menu_slot_rules(
        &self,
        host: &dyn mod_api::Host,
        event: &mut mod_api::MenuSlotRulesEvent<'_>,
    ) {
        if event.slot == 0 {
            event.may_place &= host.mod_recipe_property_contains(
                "farmersdelight:cooking_pot_input", event.stack.item(),
            );
        }
    }
}
// In Hooks::register:
// registrar.menu_part("farmersdelight:cooking_pot", PotInput);
```

The `Host` exposes the `mod_recipe_property_contains(set, ItemKey)` query, and `menu_slot_rules` receives it (the read-only `Host`). This query covers mod-created sets only: unknown sets, unknown
items and vanilla set keys return false. Existing vanilla station acceptance is unchanged.
The server reads its owning load; the client reads local preload sets before its first
`UpdateRecipes`, then the authoritative server generation. A packet replaces the entire
generation, including an empty one. Closing a screen or changing dimension retains it;
disconnecting discards it. Reads allocate nothing and do not rescan recipes or registries.

Parts that read `context.setting(..)` or `side()` have no counterpart on the Host.
Named injections for `menu_slot_rules` request `hook_version = 4`.

### Who and where: the slot's context

A rule that depends on more than the stack reads `event.context`, a `MenuSlotContext` the game
fills the same way on the server and on the client's prediction. `context.player` is the UUID of
the player whose menu it is, `context.pos` the block entity the menu belongs to
(`ContainerLevelAccess`), and `context.slots` a read-only copy of the menu's own containers'
slots as they stand while the click plans, without the player's inventory. The containers are
keyed by `MenuContainer`: `Main` for the menu's own slots, and for a mod menu type with a crafting
grid `Grid` (the grid's slots, row by row) and `Result` (its crafted result, slot 0).
`event.container` names the container the asked slot is in, and `event.slot` indexes inside it;
`context.slot(container, index)` reads any of them. A machine whose upgrade slot takes one card
per kind, a cable bus that asks its block entity what a part slot holds, a grid slot that looks at
the result, or a slot only the block's owner fills answers from these:

```rust,ignore
use mod_api::MenuContainer;

fn menu_slot_rules(&self, host: &dyn mod_api::Host, event: &mut mod_api::MenuSlotRulesEvent<'_>) {
    let context = event.context;
    // Upgrade slots 4..8 of the main container take one card of each kind.
    if event.container == MenuContainer::Main && (4..8).contains(&event.slot) {
        let item = event.stack.item();
        event.may_place &= !(4..8).any(|i| {
            i != event.slot
                && context.slot(MenuContainer::Main, i).is_some_and(|s| s.item() == item)
        });
    }
    // A crafting grid takes no more while a result waits to be taken.
    if event.container == MenuContainer::Grid {
        event.may_place &= context.slot(MenuContainer::Result, 0).is_none();
    }
    // `context.pos` names the block entity to read through `host`.
    let _ = (host, context.pos, context.player);
}
```

`player` is None where no player is known and `pos` None for a menu that is not a block's (one
opened with no block by `player_open_mod_menu`, a vehicle's); a slot that cannot cross to the API
shows empty, and `slot` reads nothing for `MenuContainer::Player` or past a container's end. The
context is built once per click plan, and only for a menu type a part answers: vanilla menus and
types with no listening part never pay for it, and the asks of one click share one copy.

## Keeping a block menu open

The server closes a player's block menu when `AbstractContainerMenu.stillValid` says no: by default when the
block entity is gone or the player's eyes are farther than their block interaction range plus 4 from it. A
large machine whose menu belongs to a core block deep inside it answers `menu_still_valid` on its menu type
instead, as a Java menu overrides `stillValid`:

```rust,ignore
struct ShellReach;
#[mod_api::horizon_part]
impl mod_api::MenuPart for ShellReach {
    fn menu_still_valid(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::MenuStillValidEvent<'_>,
    ) {
        // `event.valid` arrives as the game's answer; keep the menu open while
        // the player stands within 12 blocks of the core.
        let d: f64 = (0..3)
            .map(|k| (event.eye[k] - (f64::from(event.pos[k]) + 0.5)).powi(2))
            .sum();
        event.valid = d < 12.0 * 12.0;
    }
}
// In Hooks::register:
// registrar.menu_part("mymod:torus", ShellReach);
```

It is asked each tick the menu is open and before each click on it, on the server only, and only for a mod
menu type: vanilla menu types keep their own check. A part that leaves `valid` alone keeps the game's answer,
and a menu whose block entity is gone closes without asking. Answer from the event and `LoopContext` alone; this callback has no `Host`.


## Reading and clicking

Client hooks can read the local player's menu with `host.local_player()?.container_menu(host)`,
including the inventory when no container is open. `MenuRef::slot_count`, `slot` and `carried`
read on either side; `slot` and `carried` return an outer `None` when the handle or slot is
unavailable and `Some(None)` for an empty slot. Each read checks the world epoch and menu generation,
so a handle cannot read a replacement, reopened or respawned menu. These methods do not click or
change slots.

Mutable client hooks can submit a vanilla click with
`menu.click(host, &[mod_api::MenuClick::QuickMove { slot }])`. The slot is the same menu-order
index used by `slot`, not a hotbar index or a GUI slot id. For example:

```rust,ignore
{{#include ../../../examples/src/lib.rs:menu_quick_move}}
```

The typed actions are `Pickup` (with `MouseButton::Left` or `Right`), `QuickMove`,
`Swap` (with `SwapWith::Hotbar(0..=8)` or `Offhand`), `Clone`, `Throw`,
`Drag` and `PickupAll`. `Pickup` accepts `None` for outside, dropping the carried stack.
A `Drag { kind, slots }` owns its menu-order slot list and always submits a complete gesture;
`DragKind::Split` divides evenly, `One` places one per slot and `Clone` fills slots in creative mode.
Every action in a batch is validated before any prediction or sending; out-of-range slots/hotbar
indices, creative-only actions in survival and empty batches refuse without effects.
Stale/replaced/respawned menus, spectators, non-client hosts and the creative inventory screen refuse with `false`.
`true` means submitted, not server acceptance: prediction uses the normal menu engine and sends
`ContainerClick` with its state id, changed slots and carried item. Server corrections take precedence.
Inventory reads and direct server-side item transactions keep their existing contracts; a click does
not expose direct client inventory writes or enforce a mod's locked-slot policy. Call from an explicit
input action rather than submitting every render/tick.

`menu.close(host)` closes the menu's screen as Escape does and tells the server (`ContainerClose`);
it returns `false` while that screen is not open, so the inventory menu closes only while the inventory
screen shows. `menu.click_button(host, button)` presses one of the open menu's buttons
(`ContainerButtonClick`, as a loom's pattern list does), for input the screen's own
buttons don't take; the server's menu decides what it does, and the inventory menu has none.

## Writing slot storage

`ItemPlace::MenuSlot { menu, slot }` is slot `slot` of a player's open menu, a `MenuRef` (`player.container_menu(host)`). NeoForge 26.3's `ItemAccess` reaches a player's inventory slot (`forPlayerSlot`) and cursor
(`forPlayerCursor`), not a menu's slot. It is the storage the slot shows, named by `menu.slot_backing(host, slot)`
(`MenuSlotBacking`), and a write goes to that storage through its own handler, so it is kept once by the transaction
and the storage's rules hold: the player's inventory, armour and off hand by their
[`ENTITY_ITEMS`](entity-items.md) index, a chest's or a furnace's slot through its `ITEMS` handler, a chest minecart's
or a chested mount's through its `ENTITY_ITEMS`, and the player's ender chest, inventory crafting grid and a mod's chest
menu (`open_menu`, reached with `host.menu(uuid, window)`), kept by the game. The slot's own rules come first, as a click has them:
`menu.slot_holds(host, slot, &stack)` is how many of a stack it holds (`Slot.mayPlace`, then `Slot.getMaxStackSize`: a
furnace's fuel slot one bucket, an armour slot one piece that fits it, a shulker box's slot no shulker box, a result
slot none), and `menu.slot_gives(host, slot)` whether it gives up what it holds (`Slot.mayPickup`: bound armour stays
on outside creative). An insert fills that slot alone, and what doesn't fit stays out. A crafting grid a transaction
changed has its result made again at the outermost commit, once for each player however many writes it took
(`InventoryMenu.slotsChanged`, with `prepare_item_craft`); the result slot takes nothing and gives nothing,
since taking it is a craft. A menu closed, replaced or reopened since the `MenuRef` was made, or one whose player left,
reads and writes nothing. `menu.cursor()` is that menu's cursor (`ItemPlace::PlayerCursor { menu }`,
`forPlayerCursor`), which reaches nothing once the menu is gone, so a kept cursor never reaches the next menu's. The player's screen follows with the tick's container sync.

```rust,ignore
// Put four of the menu's offered emeralds into slot 0 of the open chest, or nothing.
let (slot, cursor) = (ItemPlace::MenuSlot { menu, slot: 0 }, menu.cursor());
host.transaction(|host, tx| {
    let took = cursor.extract(host, tx, &emerald, 4);
    if took == 4 && slot.insert(host, tx, &emerald, 4) == 4 { Ok(()) } else { Err(()) }
})
```

Inside `menu_clicked`, `ItemPlace::MenuSlot` can take from and give to the clicking
player's inventory and hotbar slots as well as the menu's own storage. Inserts fill
only the named slot and return the amount that fits. Abort the transaction if a
partial move should leave both places unchanged. Stale menu or player identities
refuse writes. The same storage path is used by native and wasm listeners.

## A mod's chest menu

`host.open_menu(uuid, rows, title, items, replace)` opens a chest menu (`generic_9x1` to `generic_9x6`) for a player
in place of the menu they have open and returns its window id; `menu_clicked` and `menu_closed` name it too. Its items
change in place: `host.menu(uuid, window)` is its `MenuRef` while the player still has that window open (None for
another window, a closed menu or a player who left), and a write to `ItemPlace::MenuSlot { menu, slot }` in a
transaction is sent with the next tick's container sync, one `ContainerSetSlot` per changed slot, as vanilla's
`broadcastChanges` sends a menu a mod changed. Nothing reopens and the cursor stays where it is.

```rust,ignore
// In menu_clicked: show the next page's item in slot 4 of the same menu.
if let Some(menu) = host.menu(uuid, window) {
    let slot = ItemPlace::MenuSlot { menu, slot: 4 };
    let _ = host.transaction(|host, tx| {
        slot.extract(host, tx, &shown, 64);
        if slot.insert(host, tx, &next, 1) == 1 { Ok(()) } else { Err(()) }
    });
}
```

To rebuild the whole menu at once (a page turned, a list refiltered) `host.menu_set_items(uuid, window, items)`
replaces everything it shows and sends it as one `ContainerSetContent` for that window, now rather than with the next
tick's sync, as vanilla's `sendAllDataToRemote` does: the screen, the title and the cursor stay. Like `open_menu` it pads
`items` with empty slots and drops the ones past the menu's last. It returns `Result<(), MenuItemsRefusal>` and each
refusal sends and changes nothing: `Client`, `NoPlayer` (no placed player has that UUID), `NotOpen` (`window` is not
the player's open `open_menu` menu: another window, a closed menu, or a menu the game opened) and
`Stack(StackWriteError)` (an item the world lacks, a count or damage the item does not allow). For one slot or a few,
write them through `host.menu` as above: they cost one packet each and no replacement of the others.

A title or a row count changes only with a new menu, as in vanilla: `open_menu` again, with a new window id, the
title and all the items. The old menu closes on the server as a close does, its cursor stack going back to the
player's inventory, and packets for its window are ignored. `replace` is the choice a Java `MenuProvider` makes
(`shouldTriggerClientSideContainerClosingOnOpen`): `MenuReplace::CloseFirst`, vanilla's default, first sends the
client a close, which centres the mouse again; `MenuReplace::InPlace` sends none, so the new screen takes the old
one's place and the mouse stays where it is. Opening from the player's inventory sends no close either way.
`player.open_mod_menu` takes the same `replace` for a content mod's menu type
([machine menu data](../../how-to/world/machine-menu-data.md)).

## Slots backed by a held item or an entity's handler

A menu opened with no block (`player.open_mod_menu`, or `open_menu`'s chest menu) can show storage that is not a block
entity, as NeoForge's `StackCopySlot` and `ResourceHandlerSlot` do. Right after the open, back its first container
slots with `player.back_mod_menu_slots(host, window, places)`, one `MenuSlotPlace` per slot:

- `MenuSlotPlace::HeldItem { hand, slot }`: slot `slot` of the container inside the stack in the player's `hand`, through
  its `ITEM_ITEMS` handler (a backpack's contents). While the menu is open, the hotbar slot holding a main-hand stack takes
  no click, so the backpack can't go into itself.
- `MenuSlotPlace::EntityItems { entity, slot }`: slot `slot` of `entity`'s `ENTITY_ITEMS` handler, as a mod's provider
  answers it.

From then on `MenuRef::slot_backing` names the slot `MenuSlotBacking::Place(place)`. A pickup or shift-click on the
menu, and an `ItemPlace::MenuSlot` write, go through that storage's own handler in one transaction, so an aborted
transaction leaves it as it was. The client sees ordinary synced slots; there is no new packet. Nothing runs per tick:
the slots are mirrored after each click on the menu and after each `ItemPlace` write to the player's places (their hand,
a container held there, the menu's slots), and only the slots that changed are written. These writes queue a change
notice, deduplicated per menu; there is no next-tick scan. A handler changed independently of an `ItemPlace` write
is checked and mirrored at the next click, not automatically on the next tick. Before each click and at each
change notice the menu checks its places: it closes once one stops resolving (the backpack left the hand, the entity is
gone), or once a `HeldItem` hand no longer holds the stack bound when the slots were backed (another stack put in its
place, another hotbar slot selected). Stacks are values, so the bound stack is told apart by its item and components
other than its contents. Slots past `places` stay the mod's own (`menu_clicked`). A drag, a number-key swap, a
double-click collect or a throw on a backed menu is still the mod's to answer.

Backing the same window again replaces its places. A slot no longer backed (an empty list, or a shorter one) is
emptied: it never keeps the copy it showed, which stays in the storage it came from. A backing that cannot be made
changes nothing and says why with a `MenuSlotBackRefusal`:

- `NoPlayerOrWindow`: the player is not online here, or `window` is not the menu they have open.
- `UnsupportedMenu`: the open menu is not a mod menu opened with no block (a block's or an entity's menu shows its own
  storage).
- `TooManyPlaces`: more places than the menu's container slots.
- `PlaceUnresolved { index }`: `places[index]` names no storage now.

```rust,ignore
let window = player.open_mod_menu(host, backpack_menu, title, Vec::new(), None, MenuReplace::CloseFirst)?;
if let Some(window) = window {
    let places = (0..27).map(|slot| MenuSlotPlace::HeldItem { hand: Hand::Main, slot }).collect();
    if let Err(refusal) = player.back_mod_menu_slots(host, window, places) {
        log::warn!("backpack menu not backed: {refusal:?}");
    }
}
```

A vanilla menu and a menu no mod backed never take this path: their slots keep the backing they had.

## A workstation with no block

An item that carries a crafting table or an anvil ("workbench on a stick") opens the vanilla workstation menu with
no block, as a Java item does with a `ContainerLevelAccess` at the player:
`player.open_station(host, menu, inputs, keep)`. `menu` is the key of `minecraft:crafting`, `minecraft:loom`,
`minecraft:grindstone`, `minecraft:cartography_table`, `minecraft:stonecutter`, `minecraft:smithing` or
`minecraft:anvil`. The station works where the player stands at each use: its sounds, the grindstone's experience
and the smithing table's level event happen there. Its input slots start with `inputs` (the crafting grid's nine row
by row, a station's in its menu's order), so a mod can put back what the item kept. It stays open while the player's
inventory (main slots, armour, off hand) holds one of the items in `keep`, which stands in for the Java menu's
`stillValid`, and closes as walking away from a block does once it holds none. A stock client shows it: the client
opens station screens from the menu type alone.

The menu replaces the one the player had open. The call returns the window id, `Ok(None)` when a mod cancelled the
open (`container_opened`), or an `OpenStationError`: `NotStation` for any other menu type,
`PlayerUnavailable`, `TooManyInputs`, or `Stack(StackWriteError)` for an input the game refuses.

Before its inputs go back to the inventory on closing (the player closed it, it was no longer valid, another menu
replaced it), `station_closing` hands a mod the input slots. Empty a slot to keep its stack, for instance in the
item's `minecraft:container` component through an `ItemPlace` write in the same hook; only emptied slots count, so a
listener can't make or grow a stack there. When the player leaves, the inputs go back to the inventory before it is
saved and the hook doesn't fire. An anvil opened this way wears no block and plays no anvil event: its mod wears its
own item after `anvil_craft_post` and plays level event 1029 or 1030 with `level.level_event`.

```rust,ignore
// An ItemPart on the stick; `self.stick` is its ItemKey, from `r.item_key` in `register`.
fn use_item(&self, host: HostSide<'_>, event: &mut UseItemEvent<'_>) -> Option<InteractionResult> {
    let HostSide::Server(host) = host else { return Some(InteractionResult::Consume) };
    let player = host.player(event.player)?;
    let table = host
        .menu_keys()
        .into_iter()
        .find(|key| host.menu_key_id(*key) == "minecraft:crafting")?;
    let grid = stored_grid(event.stack); // what this item kept from its last use
    player.open_station(host, table, grid, vec![self.stick]).ok()?;
    Some(InteractionResult::SuccessServer)
}
```

## Recipe transfer into a mod menu

A recipe viewer's "+" calls `RecipesClientHost::transfer_recipe_id(recipe, max)` (or `transfer_recipe(book_id)`
for a recipe-book recipe). The game's own transfer fills crafting grids only, so for a machine's menu the
server first asks the `MenuPart` of the open mod menu type:

```rust,ignore
fn menu_transfer(
    &self,
    host: &mut dyn mod_api::ServerHost,
    event: &mut mod_api::MenuTransferEvent<'_>,
) -> Option<mod_api::RecipeTransferResult> {
    // Look up event.recipe, check event.player's inventory, and unless event.dry_run
    // move one set (as many as fit with event.max) into the block entity at event.pos.
    let _ = (host, event);
    Some(mod_api::RecipeTransferResult::Moves)
}
```

Return `None` to hand the transfer on to the game's grid transfer (`NotIntoMenu` for a menu without a
crafting grid). A dry run, the viewer's check, must move nothing. Vanilla menu types are never asked. A solo
world and a LAN host's own player get the same contract: the game is the server there and asks the part of the
player's open block menu before its own grid transfer, on the same thread.

`menu.slot_kinds(host)` is a client read in menu index order, with exactly
`slot_count` entries. It describes the client's layout and content declaration,
not the server's backing or acceptance rules. `Player(index)` uses hotbar 0–8,
main 9–35, armour 36–39 (feet to head), off hand 40. Container equipment is
`Equipment`; linked storage is `Storage`. Workstations without a classified layout (anvil, enchanting table, loom,
and similar types) return `Other` for every slot, including player slots. A game menu with no known client slot count returns `None`, as do
stale or foreign handles, missing content types, and calls without an open menu
or outside a world. Creative returns the underlying inventory menu's 46 slots,
never creative tab wrappers; read the screen type to distinguish them.
The answer is stable for one open menu. Cache it: each read builds one Vec
from the layout table and content declaration, with one allocation.
