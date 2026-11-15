# Design: decide, fire, apply: route inventory

**Stage 1 of [arismoko/horizon#784](https://github.com/arismoko/horizon/issues/784): evidence and inventory only.**
This stage adds no runtime code, tokens or admissions. References below identify the existing owners in this worktree
(`feat/decide-fire-apply`, from `horizon-rust/horizon` main `f84b4be037`); the older numbers in
[decide, fire, apply](decide-fire-apply.md) are corrected here. Vanilla 26.3 was queried with
`python tools/rea_vanilla.py method <class> <method>`; its decompiled text is not reproduced, only
described. NeoForge could not be checked (see [NeoForge](#neoforge)), so no counterpart contract is
assigned to any row.

## How the list was found

Two mechanisms reach the mod API from client-lib code, and both were enumerated:

1. Catalog targets. `rg -n 'horizon_client_lib::' crates/horizon-mod-vocab/src/hooks` returns 22
   `target = "horizon_client_lib::…"` attributes in 19 declaration files. The generator classifies
   them in `crates/horizon-mod-api-gen/src/lib.rs:344` (`free_function_source`, prefix
   `horizon_client_lib::`) and resolves each target's `native_<hook>` in `dispatch.rs:46` (`emit`).
2. The callback table. 19 of the 22 are installed through `NativeClientHooks`
   (`crates/horizon-client/src/native_client_hooks.rs:9`, installed at `entry.rs:225`); the other 3
   call `mod_api::dispatch_` themselves (`crates/horizon-client/src/crafting/native_hooks.rs:14,40,56`).

Two consequences a reviewer should keep in mind:

- Every client-lib target currently names its function explicitly. None is implied by the
  `Game::native_<name>` default, so the design's "targets without an explicit annotation" set is
  empty today. This was checked against `catalog.rs:363,508` (the default is a Game
  method) and `hook_targets.rs:41` (resolution only rewrites Game native-hook modules).
  Re-run `rg -n 'default_target|entry\("target"\)' crates/horizon-mod-api-gen/src/catalog.rs`
  and read `hook_targets.rs` as well as the explicit-target and callback-table searches;
  searching explicit annotations alone does not establish the implicit set.
- None of the 22 receives a `Host`. 20 take `&LoopContext` and the two data-load collections
  (`ingredient`, `recipe_property_sets`) take `&LoadContext`. `LoopContext` is documented as holding
  "no game, world handle or mutable registry access"
  (`crates/horizon-world/src/native_load_context.rs:189`). The mutating ones instead write `&mut`
  out-parameters. That is the Host-less shape the clean break replaces.

## Client-lib target inventory

`Shape` is the current dispatch shape; `Treatment` is this inventory's recommendation.

| Hook | Declaration | Client-lib entry | Side / contract | Shape | Treatment |
| --- | --- | --- | --- | --- | --- |
| `living_jump` | `hooks/living_jump.rs:14` | `motion.rs:52` | both, listen | plain notice | motion exclusion |
| `living_jump_power` | `hooks/living_motion_values.rs:17` | `motion.rs:59` | both, listen | value transform (`&mut f32`) | motion exclusion |
| `living_speed` | `hooks/living_motion_values.rs:28` | `motion.rs:67` | both, listen | value transform (`&mut f32`) | motion exclusion |
| `living_flying_speed` | `hooks/living_motion_values.rs:39` | `motion.rs:75` | both, listen | value transform (`&mut f32`) | motion exclusion |
| `block_jump_factor` | `hooks/block_factors.rs:18` | `motion/surface.rs:48` | both, listen, block Part | value transform (`&mut f32`) | motion exclusion |
| `block_speed_factor` | `hooks/block_factors.rs:30` | `motion/surface.rs:56` | both, listen, block Part | value transform (`&mut f32`) | motion exclusion |
| `fluid_flow` | `hooks/fluid_flow.rs:17` | `motion.rs:95` | both, listen | value transform (`&mut [f64; 3]`) | motion exclusion |
| `player_stays_on_ground_surface` | `hooks/player_stays_on_ground_surface.rs:17` | `motion.rs:83` | both, listen | boolean query, written back | motion exclusion |
| `is_ladder` | `hooks/is_ladder.rs:15` | `motion/surface.rs:65` | both, listen, block Part | boolean query, written back | read-only fire |
| `item_stacked_on_other` | `hooks/item_stacked_on_other.rs:15` | `menu.rs:388` | both, cancel | cancel + slot/cursor write-back | typed decision |
| `menu_slot_rules` | `hooks/menu_slot_rules.rs:23` | `menu.rs:615` | both, listen, menu Part | query + write-back (`may_place`, `max_stack`) | typed decision |
| `living_heal` | `hooks/living_heal.rs:16` | `hunger.rs:23` | both, cancel | value transform + cancel | typed decision |
| `living_breathe` | `hooks/living_breathe.rs:27` | `breathing.rs:34` | server, listen | query + write-back (3 out-params) | typed decision |
| `living_drown` | `hooks/living_drown.rs:25` | `breathing.rs:71` | server, cancel | query + write-back + cancel | typed decision |
| `living_fall` | `hooks/living_fall.rs:16` | `landing.rs:45` | both, cancel | value transform + cancel | typed decision |
| `farmland_trample` | `hooks/farmland_trample.rs:15` | `landing.rs:31` | server, cancel | cancel notice | typed decision |
| `ender_entity_teleport` | `hooks/ender_entity_teleport.rs:16` | `mobs/enderman.rs:79` | server, cancel | target write-back + cancel | typed decision |
| `ingredient` | `hooks/ingredient.rs:32` | `crafting/native_hooks.rs:14` | both, collect, data load | collect (`Vec<String>`) | read-only fire |
| `ingredient_test` | `hooks/ingredient_test.rs:18` | `crafting/native_hooks.rs:56` | both, listen | pure boolean query at craft time | read-only fire |
| `recipe_property_sets` | `hooks/recipe_property_sets.rs:27` | `crafting/native_hooks.rs:40` | both, collect, data load | collect (additions) | read-only fire |
| `player_name_format` | `hooks/player_name_format.rs:16` | `lan/hub.rs:462` | both, collect | collect (`Text`) | read-only fire |
| `player_tab_list_name_format` | `hooks/player_tab_list_name_format.rs:17` | `lan/hub.rs:518` | server, collect | collect (`Option<Text>`) | read-only fire |

Counts: 22 targets, 19 files. 8 are the design's excluded motion values, `is_ladder` is a ninth
motion-module target the design does not list (see [Open items](#open-items)), 8 are mutating
gameplay moments that need a typed decision, and 6 (including `is_ladder`) are pure queries/collections that need only a
read-only fire boundary.

## Menu routes

The read sets and bold outcome names below are migration proposals, not existing token validation.
Existing callbacks mutate state directly; cancellation does not currently imply transaction rollback.

Paths are relative to the repository root. `hooks/` is `crates/horizon-mod-vocab/src/hooks/` and
`client/` is `crates/horizon-client/src/`. The design's line numbers moved; these are current.

| Route | Current owner | Vanilla method mirrored | Read set | Existing public result per outcome |
| --- | --- | --- | --- | --- |
| `item_stacked_on_other` (local pickup) | `client/menu.rs:3396` `Menu::pickup`, fires at `:3418`; event record `client/menu/models.rs:28` | `AbstractContainerMenu.doClick`, PICKUP branch | player uuid and entity id, slot index, backing container and its index, primary button, cursor stack, clicked slot stack, the slot's `may_place`/`may_pickup`/`allow_modification`, live writes through `SlotView` | cancel skips the bundle override, and a cancelled `handled` (the event's cancellation result) also skips the plain pickup; uncancelled, the bundle answer decides and an unhandled click takes the ordinary pickup; `click` returns `Ok(())` either way (`menu.rs:3422`) = **Canceled**/Applied; nothing reaches `InvalidInput` |
| `item_stacked_on_other` (authoritative) | `client/lan/menu.rs:909` `pickup`, fires at `:955`; slot view `lan/menu.rs:557` | same, server side | same, plus the guest's carried stack and the menu's registry id | same cancel/`handled` split (`lan/menu.rs:959`), then the guest resync (`lan_crafting.rs:380`) |
| `menu_slot_rules` (local) | `client/menu.rs:1874` `slot_rules` (fires `:1885`); top-up loop `menu.rs:2565` `move_to` (fires `:2592`) | `Slot.mayPlace`, `Slot.getMaxStackSize` (not a pickup-permission hook) | menu registry id, backing slot index, stack id and components and count, the slot's own vanilla answer, the item maximum, container limits (`menu.rs:1897`) | refusal → `may_place` false: the carried stack is not placed, but the click itself can still apply, because a slot stack that matches the carried stack is taken onto the cursor anyway (`pickup_inner`, `menu.rs:3487`); so a placement refusal is **Canceled** only for the placement, not for the whole click, and `click` returns `Ok(())` either way; clamp to 1..=99 (`menu.rs:637`, `CONTAINER_MAX` at `:428`) is **Applied** |
| `menu_slot_rules` (LAN) | `client/lan/menu.rs:470` `slot_rules` (fires `:493`) | same | same, resolved against the backing container or player inventory (`lan/menu.rs:486`) | placement refusal and bounded capacity answer as above; override applied at `lan/menu.rs:506` |
| local click, cursor, throw, swap | `client/menu.rs:3204` `click`; `:3396` `pickup`, `:3433` `pickup_inner`, `:3510` `swap`, `:3683` `pickup_all` | `AbstractContainerMenu.clicked` → `doClick` | slot count, cursor, every touched slot stack, input mode and button, quick-craft state | `Err(OUT_OF_BOUNDS)` = `"IndexOutOfBoundsException"` (`menu.rs:659`) → **InvalidInput**; `Ok(())` → **Applied** or **Canceled** |
| local quick move | `client/menu.rs:2645` `quick_move`; landing set `:1837` `quick_move_landing` | `AbstractContainerMenu.quickMoveStack` (abstract; menus override) and `moveItemStackTo` | source stack, destination ranges and roles, per-destination capacity via `slot_max`, registry id | no error path: no movement returns `None`; successful movement normally returns the original clicked stack (`menu.rs:3180`), with remainder stored back in the source; individual refused destinations are skipped |
| local drag / quick craft | `client/menu.rs:3626` `quick_craft_accepts`, `:3642` `quick_craft_preview`, drag branch of `click` at `:3213` | `doClick`, QUICK_CRAFT stages 0/1/2 | drag status, drag type, dragged slot list, cursor, per-slot `may_place` and maximum | preview is read-only and cannot commit; the commit is the ordinary click branch (**InvalidInput**/Applied as above) |
| local pick-all | `client/menu.rs:3589` `quick_move_all`, `:3600` `quick_move_all_each`, `:3579` `can_take_for_pick_all` | `AbstractContainerMenu.canTakeItemForPickAll` | target slot rules and the cursor stack | refused slots are skipped silently (**Canceled**), no error |
| LAN authoritative clicks and backing storage | `client/lan/menu.rs:813` `click`; `:909` pickup, `:729` `quick_move`, `:1125` `swap`, `:1225` `pickup_all`, `:1283` `quick_craft`; storage `Slots::read` `:280` / `write` `:306`, `still_valid` `:236` | `AbstractContainerMenu.clicked`/`doClick` on the server, reached through `handleContainerClick` | guest uuid and carried stack, authoritative menu generation and window id, menu registry id, backing container identity (`still_valid`), per-slot read/write through `Slots` | `lan_inventory_click` (`lan_crafting.rs:327`) answers through `inventory_clicked` (`lan/containers.rs:746`) and `answer_click` (`:782`): a stale state id sends the whole menu; otherwise mismatched or unclaimed changed slots and the cursor are corrected. This reconciles prediction after the click, rather than refusing a frozen action; no error packet, and **Busy** has no surface today |
| recipe-book placement | `client/game/network/lan_crafting.rs:637` `lan_place_recipe`, `:866` `lan_check_recipe`, `:919` `lan_apply_recipe`; fires `hooks/player_recipe_book_click.rs:13` at `:671` | `ServerPlaceRecipe.placeRecipe` (the menu overload, which returns `RecipeBookMenu.PostPlaceAction`) | recipe id resolved from the display index, its placement, guest craft grid, every source slot, `use_max`, creative, known recipes | unknown or impossible recipe and unknown placement drop the packet (`:662`); a ghost plan is answered explicitly (`:677`), the existing counterpart of `PostPlaceAction.GHOST` → refused/no-change |
| mod crafting placement | `client/game/network/lan_crafting.rs:1104` `lan_mod_craft_apply`, `:989` `lan_crafting_click` | `ServerPlaceRecipe.placeRecipe` over a mod menu | mod menu registry id, linked container slots, recipe placement, input slots | `None` when the menu is gone → refused; the click itself still returns handled |
| recipe transfer | `client/game/network/lan_crafting.rs:759` `guest_recipe_transfer`, `:802` `guest_recipe_check` | `ServerPlaceRecipe.placeRecipe` (client-initiated placement) | recipe id, all source slots, destinations, `all` | `TransferError::Unavailable` and the other transfer results are encoded in the reply (`:785`) → refused/no-change |
| anvil prepare and take | `client/menu.rs:1389` `anvil_created`, `:1395` `anvil_update`, `:1407` `anvil_updated`, `:2066` `anvil_undo`, `:2080` `anvil_cancel`, `:2091` `anvil_take`; ordered sequence `client/game/network/lan_stations.rs:172`–`:254`; hooks `hooks/anvil_craft_pre.rs:13`, `hooks/anvil_craft_post.rs:11`, `hooks/anvil_update.rs:21` | `AnvilMenu.createResult`, `AnvilMenu.onTake` | slot 0, slot 1, rename text, cost data slot, `only_renaming`, `repair_count`, result slot, player levels and inventory, cursor, the anvil block state | Pre cancel → `anvil_cancel(undo)` restores result, inputs, cost, inventory and cursor (`lan_stations.rs:190`) while `click` returns `Ok(())` = **Canceled**; Post is a notice; the update answer replaces output and cost, or cancels to no result |
| grindstone prepare and take | `client/menu.rs:1418` `grindstone_place`, `:1431` `grindstone_placed`, `:1446` `grindstone_taken`; ordered sequence `client/game/network/lan_stations.rs:205`–`:254`; hooks `hooks/grindstone_place.rs:18`, `hooks/grindstone_take.rs:19` | `GrindstoneMenu.createResult`; the take hook mirrors the result slot's take handling in that class | slot 0, slot 1, result slot, enchantments (experience), cursor | place cancel → result slot emptied (**Canceled**); take cancel → result taken with no experience, sound or input reset (`menu.rs:1452`) |
| menu close and merchant settlement | `client/menu.rs:3865` `merchant_close` (`MerchantMenu.removed`); ordinary container notice `client/game/modding/native_menus.rs:257,269`, `hooks/player_container_close.rs:21`; separate mod-menu notice `native_menus.rs:307`, `hooks/menu_closed.rs:12` | `AbstractContainerMenu.removed`, `MerchantMenu.removed` | cursor stack, payment slots 0 and 1, player inventory | no result shape: `player_container_close` follows ordinary container close; `menu_closed` is specifically for a mod-opened menu. `merchant_close` settles cursor and payments through `place_back`; see vanilla evidence below for the living/connected versus dead/disconnected distinction |
| merchant state | `client/menu.rs:3723` `merchant_update`, `:3766` `merchant_take`, `:3794` `merchant_select`, `:3831` `merchant_fill`; fires `client/game/items/trading.rs:543`, `client/game/network/lan_trading.rs:458`; hooks `hooks/trade_select.rs:11`, `hooks/trade_with_villager.rs:28` | `MerchantContainer.updateSellItem`, `MerchantResultSlot.onTake`, `MerchantMenu.tryMoveItems` | offers, selection hint, payment slots, player inventory 3..39, result slot, future experience | listen and collect only, so no cancel surface; the collect answer is written back and stands |

Two structural notes for stage 2:

- The local click path mutates in place (`menu.rs:3204`): there is no point where a decision could be
  validated after listeners and before the write.
- The LAN coordinators take the menu out of storage before dispatching, with different results for a
  recursive click. The crafting table (`lan_crafting.rs:996`–`:1002`) finds no menu and stops. The
  inventory menu (`lan_crafting.rs:341`–`:345`) is rebuilt empty with `unwrap_or_else`, so a
  recursive inventory click continues on a fresh menu. Neither answers an admission **Busy**.

The Game-only mod-menu click hooks are not additional client-lib targets: `menu_clicked` (`native_hooks/base.rs:3368`, fired at
`game/modding/native_menus.rs:306`) and `menu_quick_move`
(`native_hooks/base.rs:1615`, fired at `game/modding/api_values.rs:2020`) are `Game` hooks, not
client-lib targets, and are migrated with the rest of the server menu path.

## Baseline caller fixtures

`crates/horizon-client/src/menu/dfa_tests.rs` pins three stage-2 acceptance rows through today's
public `Menu::click` / `Menu::place_recipe` callers, without installing listeners or adding runtime code:

- Cursor stone 3 into an empty chest slot: cursor empty, destination stone 3.
- Quick move stone 10 into matching stacks of 60 and 58: each reaches 64 (adds 4 and 6),
  source empty, other chest slots and cursor unchanged.
- Place the vanilla stone pressure plate recipe from two stone: inventory source empty, exactly
  two grid stacks of one stone, output one plate. Taking that output consumes the grid once;
  a second empty-result click produces no additional plate. Placement stages ingredients; the
  result take consumes them. These are distinct operations in vanilla.

These are Rust caller regressions with independent vanilla expectations, not a new Java/Rust parity
harness or proof of dispatch counts, tokens, cancellation, stale refusal or rollback. The recipe fixture
is ignored by default and asserts that resources were actually loaded when explicitly requested.
Run on the build server with prepared resources at `.local/horizon-client/26.3`:

```sh
cargo test -p horizon-client-lib --lib dfa_baseline_ -- --include-ignored
```

The sources live under horizon-client, but `client_lib.rs` owns the `menu` module and its unit tests.
The recipe test reads `HORIZON_TEST_RESOURCES`, or `.local/horizon-client/26.3` in the checkout when
that is unset, and fails rather than passing silently when neither has the prepared data.

## Vanilla evidence

Each row is a method this page cites, queried with
`python tools/rea_vanilla.py method <class> <method>` from the 26.3 client jar.
For overloaded methods select each `--overload-index`. Inventory classes use
`net.minecraft.world.inventory`; `ServerPlaceRecipe` uses `net.minecraft.recipebook`, the packet
listener uses `net.minecraft.server.network`, and `LivingEntity` uses `net.minecraft.world.entity`.
The supplied data was read from the sibling horizon-pp `.local` via a process-local `lab.LOCAL`
override, with scratch `HORIZON_HOME` and JDK 25. No game files are included here.
REA's Java output for `moveItemStackTo` contains contradictory branches despite its `complete`
status; the matching/empty-slot branches were cross-checked with JDK `javap -c -p` on the same jar.

| Method | What it does (in words) | Result |
| --- | --- | --- |
| `AbstractContainerMenu.clicked` | Runs `doClick` synchronously inside one `try`; an exception becomes a crash report carrying menu type, menu class, slot count, slot, button and input type. | complete |
| `AbstractContainerMenu.doClick` | The click itself: drag staging for QUICK_CRAFT, then PICKUP (placement and take rules, safe insert/take, `onTake`), QUICK_MOVE through `quickMoveStack`, SWAP, CLONE and THROW. Reads the cursor and the slots it touches. | complete |
| `AbstractContainerMenu.quickMoveStack` | Abstract: the base class declares it and each menu supplies its own routing, so there is no single vanilla algorithm to mirror. | no body (abstract) |
| `AbstractContainerMenu.canTakeItemForPickAll` | Default answer is true; menus that restrict pick-all override it. | complete |
| `AbstractContainerMenu.moveItemStackTo` | Moves a stack into a slot range: first fills stacks with the same item and components up to each slot's maximum, then empty slots that accept the item, walking forwards or backwards as asked; the empty-slot pass stops after the first insertion; reports whether anything moved. | REA + bytecode cross-check |
| `AbstractContainerMenu.removed` | Server players only: a non-empty carried stack is dropped or placed back in the inventory and the carried slot is then emptied. | complete |
| `ServerGamePacketListenerImpl.handleContainerClick` | Runs on the server thread, resets the action time, drops a click for another container id, resyncs spectators and dead players, drops clicks on an invalid menu, logs and drops an out-of-range slot, then calls `clicked`. | complete |
| `Slot.mayPlace` | Default answer is true; subclasses restrict placement. | complete |
| `Slot.mayPickup` | Default answer is true; subclasses restrict taking. | complete |
| `Slot.getMaxStackSize` | The no-argument default is the container maximum; the stack overload also caps it at the item's maximum. | bytecode cross-check |
| `Slot.safeInsert` | The single-argument overload delegates with the whole count; the bounded overload refuses placement when disallowed, otherwise splits into an empty slot or merges matching stacks within capacity, returning the remainder. | both overloads complete |
| `ChestMenu.quickMoveStack` | Player inventory sources route to the chest range forwards; chest sources route to player slots backwards. The source is emptied or updated, and success returns the original stack copy. | complete |
| `ServerPlaceRecipe.moveItemToGrid` | Removes the requested matching ingredient count from inventory into a grid slot (or grows its stack), returning the still-missing count; no matching source returns -1. | complete |
| `ResultSlot.onTake` | Removes one item from each occupied recipe cell and handles recipe remainders. Two stone cells therefore consume two stone on one result take. | complete |
| `AnvilMenu.createResult` | Builds the result from the two input slots and the rename text, computing repair, enchantment and naming cost, `only_renaming` and the material count; writes the result slot, sets the cost data slot and broadcasts; empties the result when the cost is 40 or more without infinite materials. | complete |
| `AnvilMenu.onTake` | Charges the levels, uses up the material or input, clears the cost, sends the rename text through the server text filter, clears the left input, then with a 12% chance damages or breaks the anvil, always playing the use sound. | complete |
| `GrindstoneMenu.createResult` | Writes the result computed from the two inputs into the result slot and broadcasts. | complete |
| `MerchantContainer.updateSellItem` | Uses the second payment as the first matching candidate when the first is empty (without swapping stored slots); with nothing to pay it clears the result and future experience, otherwise matches an offer against the payments (retrying with them swapped), writes the result and its experience unless the offer is out of stock, and tells the client whether a result exists. | complete |
| `MerchantResultSlot.onTake` | Check-take achievements first, then for the active offer takes the payment in either order, tells the merchant, awards the trade statistic, writes the shrunken payments back and adds the offer's experience to the villager. | complete |
| `MerchantMenu.tryMoveItems` | Ignores an out-of-range index, sends each payment stack back to inventory slots 3..39 and gives up if one does not fit, then fills the payment slots from the selected offer's costs once both are empty. | complete |
| `MerchantMenu.removed` | Calls the base `removed` first, clears the trading player, returns on the client. On the server, for a dead or disconnected player it drops the two payment stacks; for a living connected player it returns them to the inventory (`placeItemBackInInventory`). | smali (Java body unavailable) |
| `ServerPlaceRecipe.placeRecipe` | Two overloads. The menu overload first answers `PostPlaceAction.NOTHING` when the grid cannot be cleared and dropping is disallowed; otherwise it gathers inventory/grid contents and calls `tryPlaceRecipe`, returning its `RecipeBookMenu.PostPlaceAction`. The private one takes the recipe and the stacked contents and places after checking the grid stacks can still grow. | both complete |
| `LivingEntity.onClimbable` | Spectators and gliding entities inside glide-through blocks are refused; a block in the climbable tag, or a usable trapdoor over a ladder, is accepted and its position remembered; otherwise false. | complete |

The three baseline expected values follow the pickup/insert rules, chest routing plus stack top-up,
and recipe placement plus result-taking above. The pressure plate's two-stone shape and single-item
output were checked in the supplied 26.3 `data/minecraft/recipe/stone_pressure_plate.json`.

## NeoForge

The issue names no NeoForge counterpart. The web reader could not access the
[26.3 menu Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/minecraft/world/inventory/AbstractContainerMenu.html)
or [stacking event Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/event/ItemStackedOnOtherEvent.html)
during this review. Other listed counterparts were not independently verified. No assertion that
all paths or versions return a particular HTTP status is made. No NeoForge source or patches
were read. Every counterpart contract below remains **unverified**; no equivalence is claimed.

| Hook or route | Public counterpart named in the catalog | Public page | Status |
| --- | --- | --- | --- |
| `item_stacked_on_other` | `ItemStackedOnOtherEvent` | 26.3 Javadoc path | unverified |
| `anvil_craft_pre`, `anvil_craft_post` | `AnvilCraftEvent.Pre`, `.Post` | 26.3 Javadoc path | unverified |
| `grindstone_place`, `grindstone_take` | `GrindstoneEvent.OnPlaceItem`, `.OnTakeItem` | 26.3 Javadoc path | unverified |
| `menu_closed`, `player_container_close` | `PlayerContainerEvent.Close` | 26.3 Javadoc path | unverified |
| `trade_with_villager` | `TradeWithVillagerEvent` | 26.3 Javadoc path | unverified |
| `is_ladder` | `IBlockExtension.isLadder` | 26.3 Javadoc path | unverified |
| `menu_clicked`, `menu_quick_move`, `menu_slot_rules`, `trade_select` | none (vanilla method overrides) | 26.3 Javadoc path | unverified |
| Menu click, cancellation and transaction contract | `AbstractContainerMenu` | 26.3 Javadoc path | unverified |

## Review coverage

The explicit search still finds 22 targets in 19 files; callback fields and direct crafting dispatches
account for all 22. Default-target construction and resolution introduce no implicit client-lib target.
Spot-checks included all nine motion entry lines, both menu hook entries, both breathing entries,
all three crafting entries, both player-name entries, the LAN read/write/validity owners and the close
notification owners. Every vanilla evidence method was re-queried (overloads separately); the added
`Slot.getMaxStackSize` evidence was checked directly in bytecode. Book links from SUMMARY and the
design note resolve relative to their containing Markdown files.

## Open items

1. **Read sets and versions.** No action decision epoch was identified in these routes. Network menu state ids already
   exist (`client/lan/containers.rs:797`), but serve prediction reconciliation, not complete ABA
   validation. `set_container_limits` (`client/menu.rs:1897`) is a setter, not a validation check;
   LAN `Slots::still_valid` (`client/lan/menu.rs:236`) checks backing storage and reach. Stage 2
   must establish complete action mutation epochs; the read-set column above is the proposed starting set per route.
2. **Reentrancy and close.** `menu_closed` follows a mod-opened menu close (`native_menus.rs:307`); ordinary container close uses `player_container_close` (`native_menus.rs:269`) and no route
   has an admission `Busy`. The LAN paths take the menu out of storage, but only the crafting table
   (`lan_crafting.rs:996`) then stops a recursive click; the inventory path (`:341`) rebuilds an empty
   menu and continues, and the local path clicks in place. Each route's public refusal for
   `Busy` must be chosen before code: local click has only `Ok(())`/`IndexOutOfBoundsException`,
   LAN has the resync, recipe placement has the transfer result byte.
3. **Budgets.** There are none today; a quick move walks every destination slot (`menu.rs:2565`) and
   a recipe plan is unbounded. Limits must be measured from real plans, as the design says.
4. **Clamp or reject.** The 1..=99 slot clamp (`menu.rs:637`, `CONTAINER_MAX` `menu.rs:428`) and the
   anvil update's material-cost clamp (negative to zero, `hooks/anvil_update.rs`) are existing
   documented clamps to preserve; structural errors already surface as `IndexOutOfBoundsException`.
5. **Complete route evidence.** For the two menu dispatch targets, `item_stacked_on_other` has explicit cancellation, and its
   cancel has the bundle/`handled` split above. `menu_slot_rules` is a query. The ordered
   anvil/grindstone sequence is in `game/network/lan_stations.rs`, not in `menu.rs`, so the design's
   owner list understates it. The server entry into `clicked` is now checked
   (`handleContainerClick`); the individual station algorithms' downstream callers are not.
6. **Host.** No target takes a `Host`: 20 take `&LoopContext` (`native_load_context.rs:189`) and the
   two data-load collections take `&LoadContext`; `is_ladder`'s declaration says so explicitly.
   Adding `Host` once, in the decide/fire step, avoids a second signature break later.
7. **`is_ladder`.** It is a ninth motion-module client-lib target but not one of the design's eight
   excluded value transforms. It is a per-block boolean query, so this inventory recommends a
   read-only fire, not a token; the design must say whether it joins the motion exclusion.
8. **NeoForge.** Until a public 26.3 page is reachable, no counterpart contract (cancel semantics,
   transaction boundary, thread) can be assigned to the menu or station hooks.
