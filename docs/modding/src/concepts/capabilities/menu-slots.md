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
Named injections for `menu_slot_rules` request `hook_version = 3`.

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

A title or a row count changes only with a new menu, as in vanilla: `open_menu` again, with a new window id, the
title and all the items. The old menu closes on the server as a close does, its cursor stack going back to the
player's inventory, and packets for its window are ignored. `replace` is the choice a Java `MenuProvider` makes
(`shouldTriggerClientSideContainerClosingOnOpen`): `MenuReplace::CloseFirst`, vanilla's default, first sends the
client a close, which centres the mouse again; `MenuReplace::InPlace` sends none, so the new screen takes the old
one's place and the mouse stays where it is. Opening from the player's inventory sends no close either way.
`player.open_mod_menu` takes the same `replace` for a content mod's menu type
([machine menu data](../../how-to/world/machine-menu-data.md)).
