# Paper and Bukkit porting ledger

For executable event comparisons, see [Paper event parity](paper-parity.md).

This is a map for porting plugin logic to Horizon native mods, **not** a Bukkit or
Paper compatibility layer. Java plugins do not run through these APIs. Event-by-event
mappings belong in the native hook catalog; this page covers other plugin services.
The Paper/Bukkit references below are public Javadoc, not implementation sources.

| Plugin API | Horizon native-mod route and limits |
| --- | --- |
| [Permissions](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/permissions/Permissible.html) | **Partial:** `Command::op` uses vanilla operator levels 0–4, not named permission nodes or Bukkit attachments. `native_player_command` can reject a player command; implement any finer policy in the mod. The command tree and dispatch are in `crates/horizon-client/src/game/modding/native_server.rs`. |
| [Scheduler](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/scheduler/BukkitScheduler.html) | **Gap:** no general delayed, repeating or asynchronous plugin task service. `native_server_tick` provides start and end callbacks for non-paused server ticks: a mod can manage its own tick deadlines there, rather than assume Bukkit task ownership or cancellation. Block/fluid tick scheduling in `crates/horizon-world/src/level/` is for world ticks, not arbitrary tasks. |
| [PersistentDataContainer](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/persistence/PersistentDataContainer.html) | **Partial:** typed `AttachmentType<T>` with a codec persists mod-namespaced values on `Holder::Player` and `Holder::Entity`, among other holders. It is not a Bukkit data container or wire format; holders must be present, and player death copying is opt-in. See `crates/horizon-mod-runtime/src/attachments.rs` and [server attachments](modding/src/concepts/attachments.md). |
| [ServicesManager](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/plugin/ServicesManager.html) | **Gap:** no equivalent provider registry, priority selection or economy service contract. Native mods must arrange their own integration; the host and attachment APIs are not a service registry. |
| [CommandExecutor](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/command/CommandExecutor.html) and Brigadier | **Partial:** register `Command` usages through `native_register_commands`, then execute commands from any source in `native_run_mod_command` with typed source identity and arguments. Horizon builds a command tree from its supported usage syntax; this is not a general Bukkit executor or arbitrary Brigadier registration. See `crates/horizon-client/src/game/modding/native_server.rs` and [server commands](https://github.com/horizon-rust/horizon/tree/main/crates/horizon-mod-vocab/src/hooks). |
| [Player.Spigot#respawn](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/entity/Player.Spigot.html), [Inventory#setContents](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/inventory/Inventory.html) on an open inventory | **Partial:** `player.respawn(host)` presses the Respawn button for a dead player (the Javadoc: respawns the player if dead) and returns `Result<(), RespawnRefusal>`, each refusal by name and changing nothing: `Client`, `InPlayerHook` (not from inside a player's hook; call it from `server_tick`), `Stale`, `NoConnection` (a fake player, or the world's own player in a single-player world) and `Alive` (health above 0 and the game not won, the case the button ignores). `host.menu_set_items(uuid, window, items)` replaces the whole contents of a menu a mod opened with `open_menu`, in one packet and without reopening it, padding to the menu's slots and cutting at them; it returns `Result<(), MenuItemsRefusal>` (`Client`, `NoPlayer`, `NotOpen`, `Stack(StackWriteError)`). It is for `open_menu` menus only, not a block's container or a menu the game opened; writing single slots through `host.menu` is the other route. |
| [World](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/World.html) clock and game rules, [Server](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/Server.html) tick times and distances, [ChatColor](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/ChatColor.html) strings | **Partial:** `level.set_clock_time` and `level.set_game_rule` return `false` for a stale handle, a client, a negative time, an unknown rule or a value the rule refuses; a `time_skip` listener can keep the old time and a rule change fires `game_rule_changed` with no actor. `host.tick_times()` lists the last 100 tick durations by tick number modulo 100, not oldest first. `host.server_set_view_distance` and `host.server_set_simulation_distance` take 2 to 32 on a dedicated server only and do not write `server.properties`. `host.text_from_json` returns `Ok(None)` for anything that is not one text component (an object with no content key, an empty list, a wrong-kind content value) and `Err` only for a text over the size or nesting limits; `host.text_from_legacy` reads `§` codes and not `&`. |
| [Player#sendBlockChange](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/entity/Player.html), [Player#canSee](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/entity/Player.html), [Chunk](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/Chunk.html), [World#refreshChunk](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/World.html), [BlockData](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/block/data/BlockData.html) | **Partial:** `player.show_block` sends one block to one player and leaves the world alone (`Err(PositionNotSendable)` past the packet's range, `Err(UnknownState)`); `player.sees` is `canSee` by the tracker's rule, ignoring distance; `chunk.resend` queues a whole chunk to its watchers at the end of the tick and `level.relight_chunks` queues up to 256 chunks and their ring to be lit at the level's next light update (no completion event); `player.place_block` is `level.set_block` with a placement's flags plus `setPlacedBy` in the player's turn (not a use-on: no placement checks, advancement, sound, game event or item use). Each returns `Err(ChunkRefusal)` or `Err(PlayerWorldRefusal)` by name and changes nothing. **Gap:** no per-player block-change batches, no tile-entity or sign sends, no light-only resend. |
| [Server](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/Server.html) `dispatchCommand`, `getMaxPlayers` and `setMaxPlayers`, [ConsoleCommandSender](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/command/ConsoleCommandSender.html) | **Partial:** `host.commands().run_command_with_output` runs one line as `run_command` does (the console, permission level 4) and returns a `CommandResult` with `success` and `feedback` as plain text (at most 256 lines of 4096 bytes); a blank line or no open server runs nothing and is `success: false` with no feedback, as `run_command` answers `false`; it does not reach the dedicated console's own commands (`stop`, `op`, `whitelist`, `ban`), which answer as a failed command. It takes no sender: Paper's `dispatchCommand(CommandSender, String)` runs as the sender given. `host.server_max_players` reads the live limit (`None` when no server is open to players) and `host.server_set_max_players` sets it for the next login and ping, as `setMaxPlayers` does, without writing `server.properties`; it refuses with `Err(PlayerLimitRefusal)` (`NotOpen`, or `TooHigh` above `i32::MAX`) and keeps the players already on. |
| [PlayerInventory](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/inventory/PlayerInventory.html), [Player#getEnderChest](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/entity/Player.html#getEnderChest()), [PersistentDataContainer](https://hub.spigotmc.org/javadocs/bukkit/org/bukkit/persistence/PersistentDataContainer.html) on a player | **Partial:** `player.inventory_slots` and `player.set_inventory_slot` read and write the 43 slots by vanilla's numbers (0 to 35 main, 36 to 39 feet to head, 40 off hand, 41 body, 42 saddle), `player.ender_chest` and `player.set_ender_chest_slot` the 27 ender chest slots, and `player.give_stack` hands one stack over as `/give` does, the rest thrown for that player alone. A write returns `Err(PlayerDataRefusal)` and changes nothing, each refusal by name: `Stale`, `Client`, `Dead`, `SlotOutOfRange` and `Stack(StackWriteError)` for a stack the item does not allow. Online players only; there is no offline-player file access. `player.persistent_data` and `player.set_persistent_data` read and replace one compound saved with the player (empty removes it; `Err(Dead)`, `Err(Stale)` or `Err(TooDeep)`, nothing written; on a death respawn and the first End exit only the `PlayerPersisted` part is kept); it is a whole compound, not typed keys per namespace (use an attachment for that). `host.item_from_snbt` reads one SNBT item compound and returns `None` for anything else (text after the compound, a count outside 1 to 99, air, an item the world lacks) and for text over 2 MiB or nested past 512 levels, which is this API's own safety bound, not vanilla's. |

For callback signatures, side and current limitations, use [native mods](modding/src/how-to/native-mods.md)
and the hook declarations under `crates/horizon-mod-vocab/src/hooks/` at the revision being targeted.

## Event counterparts: miscellaneous additions (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc only; Horizon hooks are cited by function name.
Scope: the twenty `misc-add` hooks below.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `add_pack_finders` | **Gap:** none (nearest supplied candidate [`PlayerResourcePackStatusEvent`][paper-PlayerResourcePackStatusEvent]) | The candidate reports a player's response to a resource-pack request, not the pack-discovery and registration moment used by `discover_mod_packs`. No Paper event exposes pack-finder registration. No key. |
| `add_server_reload_listeners` | **Gap:** none | `prepare_server_reload` registers data-load listeners before a reload; Paper has no event for registering server reload listeners at that point. No key. |
| `affect_neighbors_after_removal` | **Gap:** none | The hook runs inside level writes to choose ordered neighbour updates before a removed block's updates run; no Paper event exposes those update kinds or their order. No key. |
| `anvil_update` | **Partial:** [`PrepareAnvilEvent`][paper-PrepareAnvilEvent] | The event fires at anvil result preparation and exposes the inventory and result; its [`AnvilView`][paper-AnvilView] also exposes XP and material cost setters. It is not cancellable, unlike Horizon's cancel contract (although a null result can suppress output). No key. |
| `armor_hurt` | **Partial:** [`EntityDamageItemEvent`][paper-EntityDamageItemEvent] | The event covers durability damage to an item from a hit, with a damage setter and cancel, but it is generic to any damaged item and does not expose the hit's damage type/source or Horizon's armor slot grouping. Per-item cancel cannot express the hook's single cancel for all pieces. No key. |
| `arrow_nock` | **Partial:** [`PlayerReadyArrowEvent`][paper-PlayerReadyArrowEvent] | The event documents choosing an arrow while firing a bow and exposes the bow, arrow and per-arrow cancel; Horizon asks whether bow use may start, passes the hand and `has_ammo`, and still fires when no ammunition was found. Its single use-cancel is not the arrow-selection result. No key. |
| `baby_entity_spawn` | **Partial:** [`EntityBreedEvent`][paper-EntityBreedEvent] | The server breeding event exposes both parents, the breeder and the child and is cancellable. Unlike Horizon's pre-creation baby-kind hook, it exposes an existing child, the breeding item and experience, including an experience setter that Horizon cannot express. No key. |
| `block_drops` | **Partial:** [`BlockDropItemEvent`][paper-BlockDropItemEvent] | `BlockDropItemEvent` fires after a player breaks a block and exposes the block state, player and item entities with cancel. Horizon also serves non-player breakers and passes the tool and experience, which the event does not. No key. |
| `block_jump_factor` | **Gap:** none | Paper's supplied jump events describe an entity jumping, not reads of a block's jump factor (`Block.getJumpFactor`). No event has the hook's per-block-state moment. No key. |
| `block_speed_factor` | **Gap:** none | No Paper event covers reads of a block's speed factor; the supplied movement events concern entity outcomes, not per-block factor lookups. No key. |
| `block_grow_feature` | **Partial:** [`StructureGrowEvent`][paper-StructureGrowEvent] | `StructureGrowEvent` matches a sapling or mushroom attempting to grow and is cancellable, but it exposes a `TreeType` species and block list rather than Horizon's configurable feature key, and does not document replacing the feature. No key. |
| `bounce_on` | **Gap:** none | No Paper event covers a block's `bounceOn` response with the post-bounce movement vector; the hook's moment is inside entity movement. No key. |
| `fall_on` | **Gap:** none | No Paper event exposes the landed block state, the calculated fall damage and the requested landing writes together at the hook's moment. No key. |
| `block_tool_modification` | **Gap:** none | No Paper event covers a tool transforming a block with an old/new block state and a cancelable final-state replacement; the hook also runs for simulated tool uses. No key. |
| `use_item_on` | **Partial:** [`PlayerInteractEvent`][paper-PlayerInteractEvent] | Filtering `RIGHT_CLICK_BLOCK` identifies the server right-click-block path and exposes the block, item, hand, face and clicked position, but the event precedes the block behaviour rather than replacing `useItemOn` itself. Its interaction allow/deny controls cannot express Horizon's replacement answer (`TryWithEmptyHand`, `Pass`, and the other results). No key. |
| `use_without_item` | **Partial:** [`PlayerInteractEvent`][paper-PlayerInteractEvent] | The generic right-click-block event precedes block behaviour; Horizon's `useWithoutItem` stage can also follow an item-bearing call that returns `TryWithEmptyHand`, rather than only an empty-hand click. The event's interaction allow/deny controls cannot replace that block behaviour's answer. No key. |
| `bonemeal` | **Gap:** none (nearest [`BlockFertilizeEvent`][paper-BlockFertilizeEvent]) | The event reports resulting fertilization block changes after any `StructureGrowEvent`, rather than Horizon's pre-growth validity and success/cancel decision with the bone meal stack and player-or-dispenser-or-villager cause. No key. |
| `can_continue_sleeping` | **Gap:** none | No Paper event covers the per-tick check that a sleeping player, villager or wandering trader may continue sleeping; the supplied block-build event is unrelated. No key. |
| `can_player_sleep` | **Partial:** [`PlayerBedEnterEvent`][paper-PlayerBedEnterEvent] | `PlayerBedEnterEvent` fires as a player is about to enter a bed and exposes the bed and a vanilla result, but its `BedEnterResult` is read-only and only allow/deny can be set. Horizon lets mods replace the specific problem, including `other_problem`. No key. |
| `chunk_sent` | **Partial:** [`PlayerChunkLoadEvent`][paper-PlayerChunkLoadEvent] | The non-cancellable server packet notification identifies the player and chunk when the player receives that chunk, matching `run_chunk_watches` after the chunk packet was sent and its listen-only result; the Javadoc recommends this moment for sending client-related data such as fake entities. Key added. Not told for the far ring's columns (a Horizon server option, off by default), which a server may still send to players, so a port that sends data per received chunk misses those. No key. |

[paper-PlayerResourcePackStatusEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerResourcePackStatusEvent.html
[paper-PrepareAnvilEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/PrepareAnvilEvent.html
[paper-EntityDamageItemEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/entity/EntityDamageItemEvent.html
[paper-PlayerReadyArrowEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerReadyArrowEvent.html
[paper-BlockDropItemEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockDropItemEvent.html
[paper-StructureGrowEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/StructureGrowEvent.html
[paper-PlayerInteractEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerInteractEvent.html
[paper-PlayerBedEnterEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerBedEnterEvent.html
[paper-EntityBreedEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityBreedEvent.html
[paper-PlayerChunkLoadEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/packet/PlayerChunkLoadEvent.html
[paper-BlockFertilizeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockFertilizeEvent.html
[paper-AnvilView]: https://jd.papermc.io/paper/26.3/org/bukkit/inventory/view/AnvilView.html

## Event counterparts: base-analog hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc only; Horizon hooks are cited by function name.
Scope: the twenty `base-analog` hooks below.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `analog_output` | **Gap:** none | The hook answers vanilla's `BlockState.getAnalogOutputSignal` for a comparator read, a pure query whose `output` a mod may replace; no Paper event reports or replaces a block's comparator signal answer. No key. |
| `anvil_craft_post` | **Gap:** none (nearest [`PrepareAnvilEvent`][paper-PrepareAnvilEvent]) | The preparation event fires when an item enters a slot and can replace the shown result, not when the player takes the result after the levels, inputs and anvil wear are settled. No Paper event reports the completed take. No key. |
| `anvil_craft_pre` | **Gap:** none (nearest [`PrepareAnvilEvent`][paper-PrepareAnvilEvent]) | The event covers result preparation before the take, is not cancellable and does not report the take itself; Horizon's `cancel` refuses the click and leaves the result, inputs and cursor as before. No key. |
| `anvil_damaged` | **Partial:** [`AnvilDamagedEvent`][paper-AnvilDamagedEvent] | The event covers an anvil damaged from use, but fires before the change with a settable damage state and breaking flag and a cancel; Horizon fires after the block already changed from `from` to `to` and is listen-only, so it can express neither. No key. |
| `apply_effect_tick` | **Partial:** [`EntityEffectTickEvent`][paper-EntityEffectTickEvent] | Both are server-side and fire on a tick when an effect acts on an entity; the event's cancel skips that tick's application while Horizon's cleared `keep` ends the effect, and the event lacks Horizon's holder health and food. The hook is told only for a content mod's own effects, not vanilla ones such as poison, so a port that relies on Paper's event for vanilla effects loses them. No key. |
| `armor_stand_manipulate` | **Partial:** [`PlayerArmorStandManipulateEvent`][paper-PlayerArmorStandManipulateEvent] | Same pre-move swap, retrieve or place moment and cancel, with the player, stand and equipment slot; the event also exposes the interacting hand and both item stacks, which Horizon's hook does not pass. No key. |
| `arrow_loose` | **Partial:** [`PlayerReadyArrowEvent`][paper-PlayerReadyArrowEvent] | The event fires while a bow is fired and the server picks an arrow and can cancel that arrow; Horizon fires at the release with the charge, lets mods change the charge, and also fires when no ammunition was found, none of which the event expresses. No key. |
| `attack_entity` | **Partial:** [`PrePlayerAttackEntityEvent`][paper-PrePlayerAttackEntityEvent] | The event fires when a player tries to attack an entity before damage logic and cancelling prevents the blow, matching Horizon's moment and `canceled`; it additionally reports `willAttack` and starts cancelled for entities vanilla will not attack. No key. |
| `beacon_state_change` | **Partial:** [`BeaconActivatedEvent`][paper-BeaconActivatedEvent], [`BeaconDeactivatedEvent`][paper-BeaconDeactivatedEvent] | Both are server-side, non-cancellable and carry the beacon block, but activation is documented when the beam becomes visible and deactivation includes destruction of the beacon itself. Horizon's `tick_beacon_at` reports pyramid-level zero/nonzero transitions after checking the beam; it does not report all those lifecycle moments, and no single event covers both transitions. No key. |
| `bell_ring` | **Partial:** [`BellRingEvent`][paper-BellRingEvent] | The cancellable event fires as the bell is being rung, with the block and the rung entity, matching Horizon's moment, side and cancel; it also reports the rung `BlockFace` direction, which the hook does not pass. No key. |
| `block_break_progress_update` | **Partial:** [`BlockBreakProgressUpdateEvent`][paper-BlockBreakProgressUpdateEvent] | Both are non-cancellable server notifications of block-breaking progress, with the block and breaker; Paper exposes float progress from 0.0 to 1.0 and an entity, while `guest_dig_feedback` passes integer animation stages for a server guest. The public Javadoc does not establish an equivalent stage conversion or limit the breaker to players. No key. |
| `block_burn` | **Partial:** [`BlockBurnEvent`][paper-BlockBurnEvent] | The event fires as fire destroys a block and its cancel keeps it, matching Horizon's moment and `cancel`; it also names the igniting block, which the hook does not pass. No key. |
| `block_changed` | **Gap:** none | The hook observes every committed `Level.setBlock` change before side effects and neighbour updates; Paper has no general block-state-changed event, and NeoForge's `onBlockStateChange` also runs after the change. No key. |
| `block_cook` | **Partial:** [`BlockCookEvent`][paper-BlockCookEvent] | The event reports a block successfully cooking a source into a result, with a settable result, a recipe and a cancel; Horizon's `block_cook` is a listen-only notice after the cook finishes and passes no recipe. No key. |
| `block_damage_abort` | **Partial:** [`BlockDamageAbortEvent`][paper-BlockDamageAbortEvent] | The non-cancellable event reports a player stopping damage to a block, matching Horizon's notification after the abort; it also exposes the item in hand, which the hook does not pass. No key. |
| `block_dispense_armor` | **Partial:** [`BlockDispenseArmorEvent`][paper-BlockDispenseArmorEvent] | The event covers a dispenser equipping an item on a nearby entity and can cancel it before the equip; Horizon fires after the equip, consumption, sound and game event completed as a listen-only notice. No key. |
| `block_entity` | **Gap:** none | The hook supplies a declared block entity's initial saved data inside the write that creates it; Paper has no event for block-entity creation or its initial data. No key. |
| `Registrar::block_container` | **Gap:** none | Startup registration binds a `ModContainer` descriptor to a `BlockKey`; the game's container/menu tables and registration-time `ITEMS` type coverage share that declaration. Paper's inventory model has no event for this declaration. No key. |
| `Registrar::block_energy` | **Gap:** none | Startup registration binds a `ModEnergyBuffer` to a `BlockKey`; the game's energy tables and registration-time `ENERGY` type coverage share that declaration. Paper has no block-entity energy capability or event. No key. |
| `Registrar::block_fluids` | **Gap:** none (nearest [`FluidLevelChangeEvent`][paper-FluidLevelChangeEvent]) | The nearest event reports a world fluid block's level changing from adjacent blocks. A registrar declaration binds `ModFluidTanks` to a `BlockKey` for the game's tank tables and type coverage, a different moment and subject. No key. |

[paper-PrepareAnvilEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/PrepareAnvilEvent.html
[paper-AnvilDamagedEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/block/AnvilDamagedEvent.html
[paper-EntityEffectTickEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/entity/EntityEffectTickEvent.html
[paper-PlayerArmorStandManipulateEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerArmorStandManipulateEvent.html
[paper-PlayerReadyArrowEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerReadyArrowEvent.html
[paper-PrePlayerAttackEntityEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PrePlayerAttackEntityEvent.html
[paper-BeaconActivatedEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/block/BeaconActivatedEvent.html
[paper-BeaconDeactivatedEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/block/BeaconDeactivatedEvent.html
[paper-BellRingEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BellRingEvent.html
[paper-BlockBreakProgressUpdateEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/block/BlockBreakProgressUpdateEvent.html
[paper-BlockBurnEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockBurnEvent.html
[paper-BlockCookEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockCookEvent.html
[paper-BlockDamageAbortEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockDamageAbortEvent.html
[paper-BlockDispenseArmorEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockDispenseArmorEvent.html
[paper-FluidLevelChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/FluidLevelChangeEvent.html

## Event counterparts: base block hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc; Horizon cited by function name; scope = the hooks below.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `block_entity_tick` | **Gap:** none | No Paper/Bukkit event covers a block entity's server tick (`Level.tickBlockEntities`); NeoForge has no counterpart either. No key. |
| `block_entity_ticks` | **Gap:** none | A world-open query (`EntityBlock.getTicker`), asked for each block state, for whether a block entity ticks; no Paper event or API hook exists for it. No key. |
| `block_entity_update_tag` | **Gap:** none | Server-side serialisation of a block entity's client update tag (`BlockEntity.getUpdateTag`) has no Paper event. No key. |
| `block_fade` | **Partial:** [`BlockFadeEvent`][paper-BlockFadeEvent] | Both fire before a block fades, for melting and coral death, and both can be cancelled so the old state stays; the event gives the block (position and current state) and `getNewState()`, which is `to`. Paper also fires it for fire burn-out and turtle eggs, which this hook does not reach, so a plugin relying on those cases loses them. No key. |
| `block_failed_dispense` | **Partial:** [`BlockFailedDispenseEvent`][paper-BlockFailedDispenseEvent] | Paper fires only when the dispenser's inventory is empty, while the hook fires for any failed dispense and passes the `item` that was not dispensed; the event exposes no item. Paper can also change whether the failure effect plays, which the listen-only hook cannot express; the narrower moment and missing item also prevent coverage. No key. |
| `block_form` | **Partial:** [`BlockFormEvent`][paper-BlockFormEvent] | Both fire before a block forms from world conditions and can cancel it, with the position and the new state (`getNewState()`); snow and ice forming are the event's documented examples. Paper also covers water/lava and concrete forms, which this hook does not reach, so a plugin relying on those cases loses them. No key. |
| `block_ignite` | **Partial:** [`BlockIgniteEvent`][paper-BlockIgniteEvent] | Both fire server-side before ignition and can cancel it, with the target block, nullable player and cause. Paper also exposes the igniting entity and source block, which the hook does not pass; its Javadoc directs player fire placement to BlockPlaceEvent. No key. |
| `block_lock_check` | **Partial:** [`BlockLockCheckEvent`][paper-BlockLockCheckEvent] | Both check a player's container lock and can change the answer (`setResult` / `locked`). Paper additionally exposes and can replace the key item and customize denial feedback; the hook passes only the player, block and boolean result, for server guests. No key. |
| `block_receive_game` | **Partial:** [`BlockReceiveGameEvent`][paper-BlockReceiveGameEvent] | Both can suppress a sculk sensor's response to a received game event, with the receiving block and event. Paper additionally exposes the source entity, which the hook lacks, and its Javadoc documents sensors only while the hook also reaches shriekers. No key. |
| `block_shear_entity` | **Partial:** [`BlockShearEntityEvent`][paper-BlockShearEntityEvent] | Paper fires when a dispenser shears a nearby entity and is cancellable, exposing the tool and a mutable drop list; the hook fires only after the shearing effects, is listen-only, and passes neither tool nor drops. No key. |
| `block_signal` | **Gap:** none | Redstone signal computation (`getSignal`, `getDirectSignal`) has no Paper/Bukkit event; this is a query, not a moment. No key. |
| `block_spread` | **Partial:** [`BlockSpreadEvent`][paper-BlockSpreadEvent] | Paper's cancellable event covers fire spreading from a source block, matching the hook's `fire` half, with `getSource()` as `from`. It has no fluid-flow case and no `kind`, so the hook's `fluid` half is not covered by this event; Paper's cancellable [`BlockFromToEvent`][paper-BlockFromToEvent] covers water and lava flow with source and destination blocks, but has no `kind` either. No key. |
| `block_step_on` | **Gap:** none | `Block.stepOn` has no Paper/Bukkit event; general movement events do not describe a block's own step callback. No key. |
| `blocks` | **Gap:** none | Paper has no event for declaring or registering new block types at startup; this is a registration collect hook, not a runtime moment. No key. |
| `break_block` | **Partial:** [`BlockBreakEvent`][paper-BlockBreakEvent] | Both fire server-side before a player breaks a block and can cancel it or override dropped experience. Paper exposes the sampled experience and an independent `setDropItems` control; the hook starts with an optional experience override and has no independent item-drop control. No key. |
| `brewing_complete` | **Partial:** [`BrewEvent`][paper-BrewEvent] | Paper documents brewing completion, but `getContents()` still holds the pre-finalization items and the event can cancel or change the result list; the hook fires after brewing, is listen-only and passes only the stand position. Paper also exposes the remaining fuel level. No key. |
| `brewing_fuel` | **Partial:** [`BrewingStandFuelEvent`][paper-BrewingStandFuelEvent] | Paper fires before a fuel item raises the fuel level and can cancel or change the fuel power; the hook fires after the fuel was consumed and gained, is listen-only, and cannot change the amount. Same fuel item, different moment and result. No key. |
| `brewing_start` | **Partial:** [`BrewingStartEvent`][paper-BrewingStartEvent] | Both concern a stand starting to brew, but the hook fires after fuel and the timer are set and is listen-only, while the event exposes setters for the brew time. No key. |
| `broadcast_message` | **Partial:** [`BroadcastMessageEvent`][paper-BroadcastMessageEvent] | Both fire before a server broadcast, let listeners change the message and cancel it. Paper additionally exposes the recipient set, which the hook lacks; Paper notes that this event is async when fired from an async thread. No key. |
| `bucket_empty` | **Partial:** [`PlayerBucketEmptyEvent`][paper-PlayerBucketEmptyEvent] | Both concern a player emptying a bucket server-side and can cancel placement, with the player and target block. Paper additionally exposes the clicked block, face, hand and exact bucket material and can replace the resulting held item; the hook passes only the fluid, collapsing aquatic-entity buckets to water, and cannot replace the result item. The hook is told only for guests of a server, not for the local player in single player or on a LAN host. No key. |

[paper-BlockFadeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockFadeEvent.html
[paper-BlockFailedDispenseEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/block/BlockFailedDispenseEvent.html
[paper-BlockFormEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockFormEvent.html
[paper-BlockIgniteEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockIgniteEvent.html
[paper-BlockLockCheckEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/block/BlockLockCheckEvent.html
[paper-BlockReceiveGameEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockReceiveGameEvent.html
[paper-BlockShearEntityEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockShearEntityEvent.html
[paper-BlockSpreadEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockSpreadEvent.html
[paper-BlockBreakEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockBreakEvent.html
[paper-BrewingStandFuelEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/BrewingStandFuelEvent.html
[paper-BrewingStartEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BrewingStartEvent.html
[paper-BroadcastMessageEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/server/BroadcastMessageEvent.html
[paper-PlayerBucketEmptyEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerBucketEmptyEvent.html

[paper-BrewEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/BrewEvent.html
[paper-BlockFromToEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockFromToEvent.html

## Event counterparts: base campfire hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc; Horizon cited by function name; scope is the 20 base-campfire hooks below.
A key is added only where moment, side, information and result all agree; otherwise the row records the mismatch.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `campfire_start` | **Partial:** [`CampfireStartEvent`][paper-CampfireStartEvent] | Both fire server-side when an item on a campfire begins cooking, with its block and source item exposed; Paper also exposes the recipe and lets plugins set total cook time, while the hook only listens with position and item ID. No key. |
| `can_eat` | **Gap:** none (nearest [`BlockCanBuildEvent`][paper-BlockCanBuildEvent]) | The supplied candidate decides block placement, not whether a player can eat a food stack; no Paper event exposes the vanilla `canEat` answer for the hook's server half. No key. |
| `can_survive` | **Partial:** [`BlockCanBuildEvent`][paper-BlockCanBuildEvent] | The placement check and buildable setter match the hook's placement case, but Paper does not fire this for the shape-update path and the event is not the general `canSurvive` question. No key. |
| `cauldron_level_changed` | **Partial:** [`CauldronLevelChangeEvent`][paper-CauldronLevelChangeEvent] | Both fire server-side before a cauldron level change and allow cancellation, but Paper exposes an acting entity rather than only an optional player and also permits changing the new level, which the hook cannot express. Read old/new block states rather than the deprecated level accessors. No key. |
| `chunk_load` | **Covered:** [`ChunkLoadEvent`][paper-ChunkLoadEvent] | Both fire server-side when a chunk is loaded; `isNewChunk()` matches the hook's `new` flag and the chunk/world identify dimension and x/z. Non-cancellable, matching the listen-only hook. Key added. |
| `chunk_unload` | **Partial:** [`ChunkUnloadEvent`][paper-ChunkUnloadEvent] | Both notify server-side chunk unloading with chunk/world coordinates, but Paper exposes and lets plugins change whether the chunk is saved; the hook only listens after its save copy has been made. No key. |
| `component_types` | **Gap:** none | Registering namespaced NBT component types is loader/registration work, not a Paper event moment; no supplied Javadoc event covers it. No key. |
| `compost_item` | **Partial:** [`EntityCompostItemEvent`][paper-EntityCompostItemEvent] | The entity/player compost event exposes the item, but it fires before composting while the hook fires after `composter_add`; it is also cancellable, which the listen-only hook cannot express. No key. |
| `consume_effect` | **Gap:** none | No Paper event covers a mod-defined `ConsumeEffect.apply` carrying an arbitrary kind and JSON; the supplied potion events only cover potion effects. No key. |
| `container_opened` | **Partial:** [`InventoryOpenEvent`][paper-InventoryOpenEvent] | Both concern server-side inventory opening and allow cancellation, but Paper also permits a title override that the hook cannot express, and its Javadoc does not establish the hook's timing before the previous menu closes or its restriction to game-owned menus. No key. |
| `crafter_craft` | **Partial:** [`CrafterCraftEvent`][paper-CrafterCraftEvent] | Same crafter/recipe subject on the server, but Paper fires before the craft and is cancellable/settable, whereas the hook fires after a completed craft and only listens. No key. |
| `creative_slot` | **Partial:** [`InventoryCreativeEvent`][paper-InventoryCreativeEvent] | Both cover the server-side creative inventory slot action with player, slot, item and count and allow cancellation, but Paper also lets plugins replace the cursor stack; the hook cannot change the submitted item or count. No key. |
| `creeper_ignite` | **Partial:** [`CreeperIgniteEvent`][paper-CreeperIgniteEvent] | The ignition moment and creeper agree, but the event carries no igniting player or item and is cancellable, while the hook passes both and only listens. No key. |
| `creeper_power` | **Partial:** [`CreeperPowerEvent`][paper-CreeperPowerEvent] | The lightning-charges-creeper moment, creeper and strike location agree, but the event is cancellable and the listen-only hook has no cancel out. No key. |
| `critical_hit` | **Gap:** none | No supplied event covers the melee `Player.attack` critical-hit decision with vanilla/critical/multiplier facts; the candidates are projectile or target-block hits. No key. |
| `data_map_types` | **Gap:** none | Registering data-map types while server data tables are assembled has no Paper event in the dossier; the supplied map events concern map items. No key. |
| `data_pack_files` | **Gap:** none | No Paper event supplies arbitrary data-pack files during server resource assembly; the resource-pack status candidate is client-facing and unrelated. No key. |
| `datapack_sync` | **Gap:** none | No Paper event asks a mod which recipe types to sync to clients; the dossier has no datapack-sync counterpart. No key. |
| `difficulty_changed` | **Partial:** [`WorldDifficultyChangeEvent`][paper-WorldDifficultyChangeEvent] | Both are server-side, non-cancellable notifications after a difficulty change, but Paper exposes only the new difficulty and is per world, while the hook passes `from` and `to` server-wide. No key. |
| `dispense_item` | **Partial:** [`BlockDispenseEvent`][paper-BlockDispenseEvent] | Both concern a dispenser using an item, but Paper fires at dispense time with cancel/item/velocity controls, not at `getDispenseMethod` where the hook replaces the behaviour, remainder and level event. No key. |

[paper-CampfireStartEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/CampfireStartEvent.html
[paper-BlockCanBuildEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockCanBuildEvent.html
[paper-CauldronLevelChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/CauldronLevelChangeEvent.html
[paper-ChunkLoadEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/ChunkLoadEvent.html
[paper-ChunkUnloadEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/ChunkUnloadEvent.html
[paper-EntityCompostItemEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/entity/EntityCompostItemEvent.html
[paper-CrafterCraftEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/CrafterCraftEvent.html
[paper-InventoryCreativeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/InventoryCreativeEvent.html
[paper-CreeperIgniteEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/CreeperIgniteEvent.html
[paper-CreeperPowerEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/CreeperPowerEvent.html
[paper-WorldDifficultyChangeEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/world/WorldDifficultyChangeEvent.html
[paper-BlockDispenseEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockDispenseEvent.html

[paper-InventoryOpenEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/InventoryOpenEvent.html

## Event counterparts: base world, entity and dragon server hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc only; Horizon hooks are cited by function
name and scope is the base/dragon server hooks below. Rows without a key differ in moment, side, information or result.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `dragon_egg_form` | **Covered:** [`DragonEggFormEvent`][paper-DragonEggFormEvent] | The same before-placement moment: the dragon is defeated and its first egg is about to appear, and `isCancelled()` decides whether the block is placed, exactly like the hook's `cancel`. Server-side, and the `DragonBattle` handle identifies the fight whose egg is forming. Key added. |
| `enchant_effect` | **Gap:** none (nearest [`EntityPotionEffectEvent`][paper-EntityPotionEffectEvent]) | The hook reports a mod-declared `post_attack` enchantment entry on a hit mob (enchantment id, kind, effect JSON, level, victim), which no Paper event describes. The potion-effect events cover effects being applied or ticking, not enchantment entries. No key. |
| `enchant_item` | **Partial:** [`EnchantItemEvent`][paper-EnchantItemEvent] | Same enchant-table action, with the enchanter and block, but `getItem()` is the item to be enchanted and `getExpLevelCost()` is the displayed minimum level, not the hook's finished item and levels charged. Cancellation and outcome setters also have no equivalent in the listen-only hook. No key. |
| `ender_pearl_teleport` | **Partial:** [`PlayerTeleportEvent`][paper-PlayerTeleportEvent] | The pearl teleport is a `PlayerTeleportEvent` (told apart by `getCause()`), before the teleport like the hook, and the player and from/to come through `PlayerMoveEvent`. The pearl entity and the fall-damage amount are missing, so `pearl` and `damage` can neither be read nor changed. No key. |
| `entities_load` | **Partial:** [`EntitiesLoadEvent`][paper-EntitiesLoadEvent] | Both report server chunk entity loading without cancellation, and the chunk supplies dimension and coordinates. The hook counts saved records read in `spawn_loaded_entities`, including records skipped or retained without a live entity; `getEntities()` lists loaded entities, not saved records. No key. |
| `entities_unload` | **Partial:** [`EntitiesUnloadEvent`][paper-EntitiesUnloadEvent] | Both report server chunk entity unloading without cancellation. The hook counts serialized records queued by `unload_entities` (passengers can be nested in a root record), whereas `getEntities()` lists unloaded entities; its size is not proof of the same record count. No key. |
| `entity_break_door` | **Partial:** [`EntityBreakDoorEvent`][paper-EntityBreakDoorEvent] | Same entity and door, but the event is cancellable (cancelling delays the break) and is raised as the door goes, while the hook only hears after the block was removed and cannot express that cancel. No key. |
| `entity_breed` | **Partial:** [`EntityBreedEvent`][paper-EntityBreedEvent] | Both parents and the child are exposed for breeding that creates a baby, but the hook also reports egg-laying with no child. The event is cancellable and can change experience, while the hook listens after `breed` returned and has no equivalent outputs. No key. |
| `entity_change_block` | **Partial:** [`EntityChangeBlockEvent`][paper-EntityChangeBlockEvent] | The event describes an entity changing a block, exposing the entity, block and proposed block data. It is cancellable while the hook listens after the state was applied; its generic coverage also excludes changes with a more specific event. No key. |
| `entity_drop_item` | **Partial:** [`EntityDropItemEvent`][paper-EntityDropItemEvent] | Fires when an entity creates an item drop, exposing the entity and dropped `Item` (stack, amount, location), corresponding to `item`, `count` and `pos`. The Javadoc does not establish that the item entity does not yet exist; cancellation has no equivalent in the listen-only hook. No key. |
| `entity_enter_block` | **Partial:** [`EntityEnterBlockEvent`][paper-EntityEnterBlockEvent] | The same bee-into-hive moment with the entity and `getBlock()`. The event is cancellable before storage, while the hook only hears after `bee_enter_hive` stored and removed the mob, so its cancel has no `out`. No key. |
| `entity_enter_love_mode` | **Partial:** [`EntityEnterLoveModeEvent`][paper-EntityEnterLoveModeEvent] | Both identify the animal and feeding human for entering love mode. Paper additionally exposes and can change ticks in love and permits cancellation, while the hook passes no duration and listens after love mode was set. No key. |
| `entity_interact` | **Partial:** [`PlayerInteractEntityEvent`][paper-PlayerInteractEntityEvent] | Same server entity-use moment and hand facts, but Horizon's version-2 hook also predicts on the client with native `HostSide` and mutable `result: Option<InteractionResult>`. Paper cancellation cannot express the full replacement result, including side-specific swing and non-consuming fallthrough. No key. |
| `entity_join_level` | **Partial:** [`EntityAddToWorldEvent`][paper-EntityAddToWorldEvent] | Called when an entity is added to a world for any reason, including chunk loading, with entity and world handles. It is non-cancellable and exposes no fresh-versus-disk flag, while the hook permits rejecting the addition and passes `loaded_from_disk`. No key. |
| `entity_leave_level` | **Partial:** [`EntityRemoveFromWorldEvent`][paper-EntityRemoveFromWorldEvent] | Both are non-cancellable server-side notifications that an entity left a world for any reason, including chunk unloading; the entity supplies ID and type, and Paper's `getWorld()` supplies the world being left. The hook is told only for the kinds `entity_join_level` is told (not the displays, markers and interactions Horizon keeps without simulating), and when the game next notices the entity gone rather than at the removal itself, so a port that relies on Paper's event for those entities or that exact moment loses them. No key. |
| `entity_mount` | **Partial:** [`EntityMountEvent`][paper-EntityMountEvent] | The cancellable attempt to mount exposes rider and vehicle, matching the mounting half of the hook. It does not describe dismounts or expose the hook's `mounting` direction; a separate `EntityDismountEvent` is required for getting off. No single-event key. No key. |
| `entity_portal` | **Partial:** [`EntityPortalEvent`][paper-EntityPortalEvent] | The event is called before a non-player entity teleports through a portal and exposes from/to locations with cancellation and destination setters. The hook fires in `send_through_portal` after the entity was removed and an arrival queued, passing only the source dimension, entity ID and portal block position; it cannot express those outcomes. No key. |
| `entity_pose_change` | **Partial:** [`EntityPoseChangeEvent`][paper-EntityPoseChangeEvent] | Called when the pose changes and non-cancellable, matching side and result. `getPose()` returns only the new pose, so the hook's `from` is not exposed; the hook also covers guest players only. No key. |
| `entity_shoot_bow` | **Partial:** [`EntityShootBowEvent`][paper-EntityShootBowEvent] | Both identify the shooter and projectile, but Paper documents `getForce()` as up to 1.0 whereas `release_bow` passes power multiplied by 3 and `shoot_crossbow` passes projectile speed. Paper also exposes weapon and ammunition and permits cancellation and projectile replacement; the hook only listens after the projectile entered the level. No key. |
| `entity_tame` | **Partial:** [`EntityTameEvent`][paper-EntityTameEvent] | The same taming, with the tamed `LivingEntity` and `getOwner()` matching the hook's entity and `owner`. The event is cancellable before the tame is applied, while the hook only hears after the owner and persistence were set, so the cancel has no `out`. No key. |

[paper-DragonEggFormEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/block/DragonEggFormEvent.html
[paper-EntityPotionEffectEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityPotionEffectEvent.html
[paper-EnchantItemEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/enchantment/EnchantItemEvent.html
[paper-PlayerTeleportEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerTeleportEvent.html
[paper-EntitiesLoadEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/EntitiesLoadEvent.html
[paper-EntitiesUnloadEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/EntitiesUnloadEvent.html
[paper-EntityBreakDoorEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityBreakDoorEvent.html
[paper-EntityBreedEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityBreedEvent.html
[paper-EntityDropItemEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityDropItemEvent.html
[paper-EntityEnterBlockEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityEnterBlockEvent.html
[paper-EntityEnterLoveModeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityEnterLoveModeEvent.html
[paper-PlayerInteractEntityEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerInteractEntityEvent.html
[paper-PlayerJoinEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerJoinEvent.html
[paper-PlayerBedLeaveEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerBedLeaveEvent.html
[paper-EntityMountEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityMountEvent.html
[paper-EntityPortalEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityPortalEvent.html
[paper-EntityPoseChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityPoseChangeEvent.html
[paper-EntityShootBowEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityShootBowEvent.html
[paper-EntityTameEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityTameEvent.html
[paper-EntityChangeBlockEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityChangeBlockEvent.html
[paper-EntityAddToWorldEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/EntityAddToWorldEvent.html
[paper-EntityRemoveFromWorldEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/EntityRemoveFromWorldEvent.html

## Event counterparts: base entity hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc only, cited by event name; Horizon hooks are cited by function name.
Scope: the 20 base-entity hooks below, comparing each Javadoc moment, side, exposed facts and cancel/setter behaviour against the hook's contract.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `entity_teleport` | **Partial:** [`PlayerTeleportEvent`][paper-PlayerTeleportEvent] | A player command teleport carries the same from/to coordinates and a `getCause()` field, but the event is players-only, fires before the move and is cancellable, while the hook reports every server entity after its same-level command teleport and is listen-only. No key. |
| `entity_tick_listened` | **Gap:** none | No Paper/Bukkit event describes whether a mod listens to entity ticks; the hook is Horizon's opt-in gate asked before the tick round and has no game moment to map. No key. |
| `entity_tick_post` | **Gap:** none | The Paper API has no per-entity tick event and none of the candidates mark the after-tick moment for mobs, items, minecarts or boats. No key. |
| `entity_tick_pre` | **Gap:** none | For the same reason no event marks an entity about to tick, so the hook's pre-tick cancel (no AI, physics, ageing or merging) has no counterpart. No key. |
| `entity_toggle_glide` | **Partial:** [`EntityToggleGlideEvent`][paper-EntityToggleGlideEvent] | Same server-side gliding toggle for a player and the `isGliding()` fact, but the event fires before the toggle is applied and is cancellable, while the hook reports a guest's change after it was applied and is listen-only. No key. |
| `entity_unleash` | **Partial:** [`EntityUnleashEvent`][paper-EntityUnleashEvent] | Same entity and server-side unleash, but the event is "immediately prior to" it and cancellable with `isDropLeash()`/`setDropLeash()`, while the hook fires after the leash is removed (before the lead drop is handled) and cannot cancel or choose the drop. No key. |
| `exp_bottle` | **Partial:** [`ExpBottleEvent`][paper-ExpBottleEvent] | Same thrown bottle and the experience it releases, but the event fires when the bottle hits and is cancellable with `setExperience()`/`setShowEffect()`, while the hook reports the break after the experience was awarded and cannot change it. No key. |
| `experience_orb_can_merge` | **Partial:** [`ExperienceOrbMergeEvent`][paper-ExperienceOrbMergeEvent] | The event marks the server about to merge two orbs, after vanilla's answer, while the hook is asked earlier for every pair whose boxes meet and lets a mod answer either way, so it can force a merge the event never fires for; the spawning orb (`from` is 0) is not an `ExperienceOrb` the event could pass. No key. |
| `experience_orb_merge` | **Partial:** [`ExperienceOrbMergeEvent`][paper-ExperienceOrbMergeEvent] | For two existing orbs the moment (about to merge), the target/source orbs and the cancel match the hook's `into`, `from` and `canceled`, but the hook also merges a spawning orb that is not an entity yet (`from` is 0, `from_count` 1), which an event exposing two orbs cannot describe. No key. |
| `explosion_detonate` | **Partial:** [`EntityExplodeEvent`][paper-EntityExplodeEvent] (block blasts: [`BlockExplodeEvent`][paper-BlockExplodeEvent]) | The server-side block-interaction moment and mutable `blockList()` are related to the hook's positions to spare, but Paper splits entity and block sources and documents no event when mob griefing disables block interaction, while the hook hears every server explosion. Paper also exposes cancellation and a drop-yield setter that the hook cannot express; its public Javadoc does not establish that cancellation prevents entity damage. No key. |
| `explosion_prime` | **Partial:** [`ExplosionPrimeEvent`][paper-ExplosionPrimeEvent] | Same moment (an explosion about to begin) and `getRadius()` gives the radius, but the event is documented for an entity's decision to explode and is cancellable with radius and fire setters, while the hook hears every server explosion and is listen-only. No key. |
| `fill_bucket` | **Partial:** [`PlayerBucketFillEvent`][paper-PlayerBucketFillEvent] / [`PlayerBucketEmptyEvent`][paper-PlayerBucketEmptyEvent] | The hook fires at one point in `BucketItem.use` for both filling and emptying, and Paper matches each half (same moment, player's bucket use, cancellable); but no single event covers the hook, because filling and emptying are separate events. No key. |
| `fire_odds` | **Gap:** none (nearest [`BlockSpreadEvent`][paper-BlockSpreadEvent]) | The hook replaces a level's flammability table for a block's position and the face toward the fire, a lookup rather than a game moment, and `BlockSpreadEvent` only fires when a block actually spreads and can only cancel that spread; it exposes no ignite or burn odds. No key. |
| `firework_explode` | **Partial:** [`FireworkExplodeEvent`][paper-FireworkExplodeEvent] | Same rocket and position via `getEntity()`, but the event fires when the firework explodes and is cancellable, while the hook reports the explosion after its effects and damage have run and cannot cancel. No key. |
| `fluid_level_change` | **Partial:** [`FluidLevelChangeEvent`][paper-FluidLevelChangeEvent] | Same server-side fluid-level change, with the block's current data and `getNewData()` carrying the fluid type and levels, and cancellation keeping the old level. However, `setNewData()` can change the resulting block data, while the hook can only cancel and cannot change `to`. No key. |
| `food_level_changed` | **Partial:** [`FoodLevelChangeEvent`][paper-FoodLevelChangeEvent] | Same server-side player food change, with a mutable resultant food level and cancellation, but Paper's `getItem()` identifies the triggering consumed item, which the hook does not pass. The hook also combines a player turn's changes into one notification, rather than identifying an individual triggering change. No key. |
| `furnace_burn` | **Partial:** [`FurnaceBurnEvent`][paper-FurnaceBurnEvent] | Same block and fuel item, but the event fires before the fuel is burned and is cancellable with burn-time and burning setters, while the hook reports after the furnace consumed the fuel and began or resumed burning and is listen-only. No key. |
| `furnace_extract` | **Partial:** [`FurnaceExtractEvent`][paper-FurnaceExtractEvent] | Same server-side moment when a player takes output, and `getPlayer()`, the block, `getItemStack()` and `getItemAmount()` carry the hook's player, pos, item and count. However, the inherited `setExpToDrop()` changes the experience awarded after the event, while the hook is listen-only and exposes no experience amount or output. No key. |
| `furnace_smelt` | **Partial:** [`FurnaceSmeltEvent`][paper-FurnaceSmeltEvent] | Same block, input and result, but the event fires before the result is written and can be cancelled, while the hook reports after the output grew and cannot change it. No key. |
| `furnace_start_smelt` | **Partial:** [`FurnaceStartSmeltEvent`][paper-FurnaceStartSmeltEvent] | Same server-side start of smelting, with the block and inherited `getSource()` carrying the position and input item. However, Paper exposes the recipe and total cook time and lets a plugin change that time with `setTotalCookTime()`, while the hook reports that the cooking timer advanced from zero and is listen-only. No key. |

[paper-EntityToggleGlideEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityToggleGlideEvent.html
[paper-EntityUnleashEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityUnleashEvent.html
[paper-ExpBottleEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/ExpBottleEvent.html
[paper-ExperienceOrbMergeEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/ExperienceOrbMergeEvent.html
[paper-EntityExplodeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityExplodeEvent.html
[paper-BlockExplodeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockExplodeEvent.html
[paper-ExplosionPrimeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/ExplosionPrimeEvent.html
[paper-PlayerBucketFillEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerBucketFillEvent.html
[paper-PlayerBucketEmptyEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerBucketEmptyEvent.html
[paper-BlockSpreadEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockSpreadEvent.html
[paper-FireworkExplodeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/FireworkExplodeEvent.html
[paper-FluidLevelChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/FluidLevelChangeEvent.html
[paper-FoodLevelChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/FoodLevelChangeEvent.html
[paper-FurnaceBurnEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/FurnaceBurnEvent.html
[paper-FurnaceExtractEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/FurnaceExtractEvent.html
[paper-FurnaceSmeltEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/FurnaceSmeltEvent.html
[paper-FurnaceStartSmeltEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/FurnaceStartSmeltEvent.html

## Event counterparts: base hooks (hanging, hopper, inventory, item, block state) (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc only; Horizon hooks are cited by function name. Scope: the 19 `base.rs` hooks below.
A key exists only where the moment, side, facts and outcome all agree; otherwise the row says which part differs.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `hanging_break` | **Partial:** [`HangingBreakEvent`][paper-HangingBreakEvent] | The hanging entity exposes its kind and position, and cancellation prevents removal, but Paper also reports `getCause()` (and the remover in `HangingBreakByEntityEvent`); `hanging_break` supplies only an optional player, so blast and lost support cannot be distinguished. No key. |
| `hanging_place` | **Partial:** [`HangingPlaceEvent`][paper-HangingPlaceEvent] | Same server placement moment and cancellation, with the player and hanging entity supplying kind and position; Paper additionally exposes the clicked block/face, hand and originating item stack, which `hanging_place` does not pass. No key. |
| `has_analog_output` | **Gap:** none | No Paper/Bukkit event is consulted when a comparator reads a block's analog output; the hook is a per-state virtual call with no event counterpart. No key. |
| `hears_projectile_impacts` | **Gap:** none (nearest [`ProjectileHitEvent`][paper-ProjectileHitEvent]) | The hook is a Horizon-only opt-in gate asked once per tick (whether any mod wants impact events), not an in-world moment; `ProjectileHitEvent` is the impact itself and fires regardless of listeners. No key. |
| `hopper_inventory_search` | **Partial:** [`HopperInventorySearchEvent`][paper-HopperInventorySearchEvent] | Same server search with the hopper and selected container, but Paper fires on each search attempt (including no inventory) and `setInventory()` changes the inventory the hopper will use; `hopper_inventory_search` only reports a selected target and cannot replace it. No key. |
| `hopper_pickup_item` | **Partial:** [`InventoryPickupItemEvent`][paper-InventoryPickupItemEvent] | Fired when a hopper or hopper minecart takes a dropped item, cancellable; the `Item` entity gives the stack's item and count plus its position, the inventory is the hopper (filter minecarts out if needed), and cancelling leaves the stack in the world as the hook's `cancel` does. The hook is documented only as "a hopper at `pos`"; Paper's event also fires for hopper minecarts, which the hook's declaration does not say it covers, so a port that relies on the event for minecarts must not assume it. No key. |
| `hurt_enemy` | **Gap:** none (nearest [`EntityDamageEvent`][paper-EntityDamageEvent]) | The generic damage event describes damage before it is applied, not the landed-hit `Item.hurtEnemy` callback before post-attack enchantments and weapon wear. Its mutable damage and cancellation also have no counterpart in this listen-only hook. No key. |
| `ingredient` | **Gap:** none | No Paper/Bukkit event resolves a custom recipe ingredient kind to item ids while a data load parses recipes; recipes are registered through the API after load, with no pre-parse hook. No key. |
| `inventory_block_start` | **Partial:** [`InventoryOpenEvent`][paper-InventoryOpenEvent] | Same server inventory opening and cancellation for block-held inventories, with player and block location available; Paper additionally lets a listener change the title sent in the open packet through `titleOverride(Component)`, which this cancel-only hook cannot express. No key. |
| `inventory_move_item` | **Partial:** [`InventoryMoveItemEvent`][paper-InventoryMoveItemEvent] | Same server transfer attempt with source, destination and item, and cancellation returns or leaves it at the source; Paper exposes the transferred stack and `setItem()` can replace it, whereas `inventory_move_item` reports only the item id for a one-item transfer and cannot change it. No key. |
| `inventory_tick` | **Gap:** none | No Paper/Bukkit event ticks each carried stack: the hook is `Item.inventoryTick` per slot per tick, with no event exposing a slot tick or letting a listener change the stack in place. No key. |
| `is_bonemeal_success` | **Gap:** none | No event asks a block whether this bone meal use grows it (the random roll inside `isBonemealSuccess`); Paper has no counterpart for the per-block success decision. No key. |
| `is_signal_source` | **Gap:** none | No event asks whether a block state is a redstone signal source; it is a per-state virtual the level consults when wire connects, with no Paper/Bukkit equivalent. No key. |
| `is_valid_bonemeal_target` | **Gap:** none | No event asks, before bone meal is used, whether a block accepts it; the `TargetHitEvent`, `EntityTargetEvent` and `SlimeTargetLivingEntityEvent` candidates match on the word "target" only and describe unrelated moments. No key. |
| `item_consumption_teleport` | **Partial:** [`PlayerTeleportEvent`][paper-PlayerTeleportEvent] | The chorus-fruit teleport is the same moment (`TeleportCause.CHORUS_FRUIT`), the from/to pair is the parent `PlayerMoveEvent`'s and can be changed, and cancel refuses it; but the hook fires per candidate draw before the ground check (up to 16) and its cancel ends the whole effect, while Paper fires for the teleport itself and its cancel only refuses that teleport. No key. |
| `item_crafted` | **Partial:** [`ItemCraftedEvent`][paper-ItemCraftedEvent] | Same moment (a player takes the result from a crafting grid, not a stonecutter, smithing table or furnace), server side and listen-only, but the event gives only the player and the crafted stack: the recipe id and the grid's inputs the hook passes are missing. No key. |
| `item_despawn` | **Partial:** [`ItemDespawnEvent`][paper-ItemDespawnEvent] | Same 6000-tick moment with the item entity, its stack and location, but the hook only hears while cancelling the event grants 5 more minutes: a listen-only hook cannot express that cancel (the separate `item_expire` hook extends life by ticks). No key. |
| `item_expire` | **Partial:** [`ItemDespawnEvent`][paper-ItemDespawnEvent] | Same lifetime moment, but the hook lets each mod set or add an arbitrary `extra_life` in ticks (capped at 32766) while the event offers only a cancel that extends by a fixed 5 minutes, so the hook's out cannot be expressed. No key. |
| `item_picked_up` | **Partial:** [`EntityPickupItemEvent`][paper-EntityPickupItemEvent] | Fired when a living entity picks an item off the ground, cancellable; the `Item` entity gives the stack's item and amount, `getRemaining()` the part left, and the entity is the player (filter out non-player pickups the hook does not hear), with cancel leaving the stack on the ground. The hook is told only for a player touching an item (`ItemEntity.playerTouch`), before its pickup delay and target are checked; `False` leaves the stack and sets the vanilla eligibility delay to at least 10 ticks (the hook still runs on each touch, and a later `True` overrides the delay), and `True` forces a pickup Paper's event cannot; Paper's event also covers every other living entity that picks items up, so a port that relies on mob pickups loses them. No key. |

[paper-HangingBreakEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/hanging/HangingBreakEvent.html
[paper-HangingPlaceEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/hanging/HangingPlaceEvent.html
[paper-ProjectileHitEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/ProjectileHitEvent.html
[paper-HopperInventorySearchEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/HopperInventorySearchEvent.html
[paper-InventoryPickupItemEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/InventoryPickupItemEvent.html
[paper-EntityDamageEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityDamageEvent.html
[paper-InventoryOpenEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/InventoryOpenEvent.html
[paper-InventoryMoveItemEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/InventoryMoveItemEvent.html
[paper-PlayerTeleportEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerTeleportEvent.html
[paper-ItemCraftedEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/inventory/ItemCraftedEvent.html
[paper-ItemDespawnEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/ItemDespawnEvent.html
[paper-EntityPickupItemEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityPickupItemEvent.html

## Event counterparts: base item hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc only: no Paper server source is used and no Bukkit compatibility layer is proposed. Horizon hooks are cited by function name; scope is the 20 `base-item` hooks below.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `item_spawn` | **Gap:** none (nearest [`ItemSpawnEvent`][paper-ItemSpawnEvent]) | Paper describes a cancellable spawn whose cancellation prevents the item from spawning, whereas `item_spawn` listens only after the item was successfully added. Its item entity supplies the stack and position, but the before/after boundary differs. No key. |
| `items` | **Gap:** none cited | Item declarations are collected once at registration; no cited Paper event fires when content is registered or offers the declared items as an out-value. No key. |
| `left_click_block` | **Partial:** [`PlayerInteractEvent`][paper-PlayerInteractEvent] | Action.LEFT_CLICK_BLOCK narrows the server interaction to a left click on a block, with the clicked block and face. Paper additionally exposes the item and hand and separate block-use and item-use results, whereas left_click_block passes block state, position and face with one cancel flag; its Javadoc also does not establish the hook's precise post-guard dig-start ordering. No key. |
| `level_load` | **Partial:** [`WorldLoadEvent`][paper-WorldLoadEvent] | Paper announces a loaded world on the server and exposes that world, but its Javadoc does not establish Horizon's ordering before the first chunk or its lazy dimension-open lifecycle. No key. |
| `level_tick_post` | **Gap:** none cited | No cited Paper event marks the end of one dimension's tick pass, and the cited set has no per-level tick event at all, so Horizon's per-dimension ordering cannot be expressed. No key. |
| `level_tick_pre` | **Gap:** none cited | As `level_tick_post`: the cited set has no per-level tick event, so the start of a dimension's tick pass is not observable through the cited events. No key. |
| `level_unload` | **Gap:** none (nearest [`WorldUnloadEvent`][paper-WorldUnloadEvent]) | Paper exposes the unloading world but is cancellable; it does not document the hook's already-closed, after-last-save notification. No key. |
| `lightning_strike` | **Partial:** [`LightningStrikeEvent`][paper-LightningStrikeEvent] | Both describe a server lightning strike with cancellation, position and cause; the lightning entity also exposes its causing entity. Paper distinguishes SPAWNER, TRAP and ENCHANTMENT as well as COMMAND, CUSTOM, TRIDENT and WEATHER, so its cause information cannot all be expressed by Horizon's weather/trident/command/mod values. No key. |
| `living_breathe` | **Gap:** none (nearest [`EntityAirChangeEvent`][paper-EntityAirChangeEvent]) | Paper fires when remaining air changes, with a settable amount and cancellation; it does not describe the per-tick breathing decision with separate mutable can_breathe, consume and refill inputs. No key. |
| `living_drown` | **Gap:** none (nearest [`EntityDamageEvent`][paper-EntityDamageEvent]) | Paper's drowning damage event concerns the damage application, rather than the air tick deciding whether to drown; it lacks the hook's is_drowning flag and bubble count. No key. |
| `living_entity_use_item_finish` | **Gap:** none (nearest [`PlayerItemConsumeEvent`][paper-PlayerItemConsumeEvent]) | Paper fires while a player is finishing consumption, before effects and item removal, and can cancel them or set a replacement. Horizon fires after finishUsingItem and exposes the actual resulting stack, also for non-consumable timed uses; the moments differ. No key. |
| `living_entity_use_item_start` | **Gap:** none (nearest [`PlayerInteractEvent`][paper-PlayerInteractEvent]) | A right-click interaction does not establish that LivingEntity.startUsingItem has been reached: interactions can perform other actions or never start a timed use. It also lacks the mutable duration and mob uses. No key. |
| `living_entity_use_item_stop` | **Partial:** [`PlayerStopUsingItemEvent`][paper-PlayerStopUsingItemEvent] | Paper detects a player releasing an item and exposes the item and elapsed ticks. It is non-cancellable, lacks the hand and remaining duration, and cannot express Horizon's cancellation of the release outcome or its mob uses. No key. |
| `living_entity_use_item_tick` | **Gap:** none cited | No cited Paper event fires on every tick of an item use or lets a listener move the remaining duration; the cited click events are a different moment. No key. |
| `living_equipment_change` | **Partial:** [`EntityEquipmentChangedEvent`][paper-EntityEquipmentChangedEvent] | Paper fires after an equipment change is detected, server-side, for a living entity, and its changed-slot map carries each slot's old and new stack, matching the hook's `from`/`to`; it is listen-only (non-cancellable) like the hook. Paper batches the slots where Horizon reports one per slot. The hook hears mobs and armor stands only (players are heard by `native_on_equip`), so a port that relies on Paper's event for players loses them. No key. |
| `living_equipment_listened` | **Gap:** none cited | It is a Horizon-only opt-in gate asked once a tick, not a game moment; no cited event expresses it, and the equipment events it gates are unrelated to the gate itself. No key. |
| `living_fall` | **Gap:** none (nearest [`EntityDamageEvent`][paper-EntityDamageEvent]) | Paper's damage event fires when fall damage lands, after fall handling, and exposes neither the fall distance nor the damage multiplier the hook lets mods change; the both-side pre-damage moment has no cited event. No key. |
| `living_heal` | **Partial:** [`EntityRegainHealthEvent`][paper-EntityRegainHealthEvent] | Paper exposes the healing entity, mutable amount and cancellation on the server, matching the server half of living_heal. It also supplies a regain reason and fast-regeneration flag which Horizon does not pass; its Javadoc does not specify the hook's precise pre-application ordering. No key. |
| `living_incoming_damage` | **Partial:** [`EntityDamageEvent`][paper-EntityDamageEvent] | The hit moment, server side, cancellation and a settable damage amount match, but Horizon also passes and lets mods change the invulnerability ticks a fresh hit leaves and passes the attacker's weapon, neither of which the event exposes. No key. |
| `living_jump` | **Partial:** [`EntityJumpEvent`][paper-EntityJumpEvent] | `EntityJumpEvent` detects a living entity's jump server-side and identifies the entity, but it is cancellable (a listen-only hook cannot express its cancel) and fires at the jump decision rather than after the velocity write; `HorseJumpEvent` and `PlayerJumpEvent` are narrower. No key. |

[paper-LightningStrikeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/weather/LightningStrikeEvent.html
[paper-EntityEquipmentChangedEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/entity/EntityEquipmentChangedEvent.html
[paper-EntityJumpEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/EntityJumpEvent.html
[paper-ItemSpawnEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/ItemSpawnEvent.html
[paper-WorldLoadEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/WorldLoadEvent.html
[paper-WorldUnloadEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/WorldUnloadEvent.html
[paper-EntityAirChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityAirChangeEvent.html
[paper-PlayerItemConsumeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerItemConsumeEvent.html
[paper-PlayerStopUsingItemEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerStopUsingItemEvent.html
[paper-EntityRegainHealthEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityRegainHealthEvent.html

## Event counterparts: base/living hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc; Horizon cited by function name; scope = the hooks below.
Decisions follow the documented moment, side, information and result; only Covered rows get a `paper` key.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `living_knockback` | **Partial:** [`EntityKnockbackEvent`][paper-EntityKnockbackEvent] | The server-side knockback moment and cancellable vector cover the push, but the event exposes no damage source, attacker id or held main/off stacks. No key. |
| `loot_condition` | **Gap:** none (nearest [`LootGenerateEvent`][paper-LootGenerateEvent]) | That event fires when a loot table generates and exposes rolled loot, not when a mod-defined condition type is evaluated; there is no condition kind, JSON or `holds` result. No key. |
| `loot_drops` | **Partial:** [`BlockDropItemEvent`][paper-BlockDropItemEvent] | It matches only player block breaks and the item list, while the hook also covers mob deaths, explosion/mob breaks, player-use and leash drops and passes table, position, entity, player, tool and explosion context. No key. |
| `loot_function` | **Gap:** none (nearest [`LootGenerateEvent`][paper-LootGenerateEvent]) | The nearest event fires after table generation; it has no per-function type, JSON, context or mutable stack at the function's evaluation moment. No key. |
| `loot_table_load` | **Gap:** none (nearest [`LootGenerateEvent`][paper-LootGenerateEvent]) | The hook runs as table JSON loads from data packs, before anything rolls it; `LootGenerateEvent` runs when a table generates in the world and offers generated loot, not the raw table document. No key. |
| `may_place_on` | **Gap:** none (nearest [`BlockCanBuildEvent`][paper-BlockCanBuildEvent]) | `BlockCanBuildEvent` asks whether a player can place a block at a location; the hook is the server-side `mayPlaceOn` survival answer for a plant state on a ground state, with no player, hand or placement. No key. |
| `xp_repair_ratio` | **Gap:** none (formerly Partial on [`PlayerItemMendEvent`][paper-PlayerItemMendEvent]) | `PlayerItemMendEvent` is a global pre-repair event with a mutable repair; the item-scoped, read-only `xp_repair_ratio` can't stand in for it. No key. |
| `menu_button` | **Gap:** none | No cited event covers a mod container's `clickMenuButton`, including the out-of-world block entity and the mutable carried stack. No key. |
| `menu_clicked` | **Partial:** [`InventoryClickEvent`][paper-InventoryClickEvent] | The server inventory-click event exposes the player, view, slot and click type, but runs within inventory modification and can cancel or change its outcome; `menu_clicked` only reports clicks that moved nothing after they are queued. No key. |
| `menu_closed` | **Partial:** [`InventoryCloseEvent`][paper-InventoryCloseEvent] | The non-cancellable server close notification exposes the player and inventory view, with documented PLAYER, OPEN_NEW, DISCONNECT and PLUGIN reasons matching the close causes. It does not expose the protocol window id passed by `menu_closed`, and its reason is absent from the hook. No key. |
| `menu_quick_move` | **Partial:** [`InventoryClickEvent`][paper-InventoryClickEvent] | `isShiftClick()` identifies server shift-clicks, and cancellation plus manual inventory changes can replace their outcome, but this is the inventory-click transaction rather than the menu's `quickMoveStack` routing call with a mutable out-of-world mod block entity and remaining stack. No key. |
| `menu_slot_rules` | **Gap:** none | No cited event covers `Slot.mayPlace` or per-slot maximum rules for registered vanilla and mod menus; the both-side hook has no server event counterpart here. No key. |
| `mob_effect_added` | **Gap:** none (nearest [`EntityPotionEffectEvent`][paper-EntityPotionEffectEvent]) | The Paper event precedes a potion-effect modification and can cancel it or change override behavior; `mob_effect_added` reports every addEffect call for a mod effect, changed or unchanged, after the start callback. Its ADDED/CHANGED actions do not establish that post-call, unchanged-inclusive moment. No key. |
| `mob_killed` | **Partial:** [`EntityDeathEvent`][paper-EntityDeathEvent] | Death of the living entity is the same after-the-blow moment, and the damage source can identify its causing player, but the event covers all deaths, exposes additional damage/drop/experience information, and is cancellable while the hook cannot cancel. No key. |
| `mob_spawned` | **Partial:** [`EntityAddToWorldEvent`][paper-EntityAddToWorldEvent] | The non-cancellable server world-add notification exposes the entity, world and position, but includes chunk loading and other additions excluded by the hook and has no spawn reason or `natural` flag. [`CreatureSpawnEvent`][paper-CreatureSpawnEvent] offers a spawn reason but is cancellable before spawning, whereas the hook reports a mob already added. No key. |
| `mod_container_viewers` | **Gap:** none | No cited event carries the aggregate viewer count and `previous`/`viewers` transition for a mod container; per-player open/close events would not supply it. No key. |
| `modify_default_components` | **Gap:** none | No cited Paper event modifies per-item default component maps at data-load/registry-freeze time. No key. |
| `moisture_change` | **Covered:** [`MoistureChangeEvent`][paper-MoistureChangeEvent] | The cancellable soil-moisture change is the same server-side before-change moment; the block's current state and `getNewState()` give from/to, and cancelling keeps the old moisture. Key added. |
| `neighbor_changed` | **Gap:** none | The hook is the per-block `neighborChanged` delivery; the dossier's player/world/equipment candidates are unrelated, and its own note says `NeighborNotifyEvent` is a different broadcast. No key. |

[paper-EntityKnockbackEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/entity/EntityKnockbackEvent.html
[paper-LootGenerateEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/LootGenerateEvent.html
[paper-BlockCanBuildEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockCanBuildEvent.html
[paper-EntityPotionEffectEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityPotionEffectEvent.html
[paper-EntityDeathEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityDeathEvent.html
[paper-CreatureSpawnEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/CreatureSpawnEvent.html
[paper-MoistureChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/MoistureChangeEvent.html

[paper-PlayerItemMendEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerItemMendEvent.html
[paper-InventoryClickEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/InventoryClickEvent.html
[paper-InventoryCloseEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/InventoryCloseEvent.html
[paper-EntityAddToWorldEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/EntityAddToWorldEvent.html

## Event counterparts: base-on hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc; Horizon cited by function name; scope = the 20 `base-on` hooks below.
Status is judged on moment, side, information and result; “No key” means no `paper` key is added.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `on_effect_added` | **Partial:** [`EntityPotionEffectEvent`][paper-EntityPotionEffectEvent] | The event is cancellable before the modification, while the hook is listen-only after `addEffect`; it does expose the entity and effect/amplifier facts. No key. |
| `on_equip` | **Partial:** [`EntityEquipmentChangedEvent`][paper-EntityEquipmentChangedEvent] | Non-cancellable and exposes each slot's old/new items through EquipmentChange, but detects changes after attribute/enchantment updates, including durability and login, rather than specifically before the equip sound. No key. |
| `on_use_tick` | **Gap:** none | No Paper event was provided for each tick of an in-progress item use with remaining ticks; interaction events fire on the initial click, not the use tick. No key. |
| `perform_bonemeal` | **Gap:** none | No Paper event for `BonemealableBlock.performBonemeal` was provided or cited; the vanilla growth moment has no counterpart in the dossier. No key. |
| `pickup_xp` | **Partial:** [`PlayerPickupExperienceEvent`][paper-PlayerPickupExperienceEvent] | Same attempt-to-pick-up moment and cancellation, and it exposes the orb, but it has no setters for the hook's `value` and `count` outputs. No key. |
| `piston_post` | **Gap:** none (nearest [`BlockPistonExtendEvent`][paper-BlockPistonExtendEvent] / [`BlockPistonRetractEvent`][paper-BlockPistonRetractEvent]) | Paper's piston events are the cancellable pre-move events; the hook fires after movement/destruction, so the post moment is uncovered. No key. |
| `piston_pre` | **Partial:** [`BlockPistonExtendEvent`][paper-BlockPistonExtendEvent] / [`BlockPistonRetractEvent`][paper-BlockPistonRetractEvent] | Cancellable pre-move events match the moment and side, but their `getBlocks()` describes moved blocks and does not expose the hook's separate `destroy` list. No key. |
| `player_advancement_done` | **Partial:** [`PlayerAdvancementDoneEvent`][paper-PlayerAdvancementDoneEvent] | Matches server-side completion and exposes the player and advancement, but message(Component) changes the announcement; the listen-only hook has no corresponding output. No key. |
| `player_advancement_progress` | **Partial:** [`PlayerAdvancementCriterionGrantEvent`][paper-PlayerAdvancementCriterionGrantEvent] | Covers the post-progress grant branch and exposes progress/criterion, but not revocation, and it is cancellable while the hook is listen-only. No key. |
| `player_arm_swing` | **Partial:** [`PlayerArmSwingEvent`][paper-PlayerArmSwingEvent] | Exposes the hand and is server-side, but it is cancellable (the hook only listens) and is the swing event rather than the hook's after-handling notification. No key. |
| `player_attack_cooldown_reset` | **Partial:** [`PlayerAttackEntityCooldownResetEvent`][paper-PlayerAttackEntityCooldownResetEvent] | Only fires for an attack on an entity, while the hook fires for every main-hand swing reset (including mining and clicking nothing); it is also deprecated and cancellable. No key. |
| `player_bed_enter` | **Partial:** [`PlayerBedEnterEvent`][paper-PlayerBedEnterEvent] | Matches the server-side pre-entry attempt and cancellation, but enterAction() exposes the default outcome and setUseBed(ALLOW) can force sleep despite it; the hook fires only after checks pass and can only cancel. No key. |
| `player_bed_leave` | **Partial:** [`PlayerBedLeaveEvent`][paper-PlayerBedLeaveEvent] | Same leaving-bed moment and bed information, but the event is cancellable (and can change spawn-setting behaviour) while the hook is listen-only. No key. |
| `player_bucket_entity` | **Partial:** [`PlayerBucketEntityEvent`][paper-PlayerBucketEntityEvent] | Exposes the captured entity and buckets server-side, but it is cancellable before the capture, while the hook is listen-only after the mob is removed and the bucket given. No key. |
| `player_bundle_item_select` | **Partial:** [`PlayerBundleItemSelectEvent`][paper-PlayerBundleItemSelectEvent] | Matches server-side non-cancellable selection and exposes player, slot and selected index, but also carries the previous index and previous/selected items, which the hook does not pass. No key. |
| `player_change_beacon_effect` | **Partial:** [`PlayerChangeBeaconEffectEvent`][paper-PlayerChangeBeaconEffectEvent] | Carries the beacon and primary/secondary effects, but it is cancellable with setters and fires when the player sets the effect, while the hook only listens after the effects are stored. No key. |
| `player_changed_dimension` | **Partial:** [`PlayerChangedWorldEvent`][paper-PlayerChangedWorldEvent] | Non-cancellable server-side notification when the player switches worlds; getFrom() and the player's current world supply the source and destination corresponding to the hook's dimension IDs. Not heard by a remote client window; listeners run on the server that simulates the world. The hook is heard on dedicated, integrated and LAN-hosted servers, but not for a move that ends where it started, and not for a player who leaves before the move ends, so a port that relies on Paper's event in those cases loses them. No key. |
| `player_client_loaded_world` | **Partial:** [`PlayerClientLoadedWorldEvent`][paper-PlayerClientLoadedWorldEvent] | The non-cancellable client-notification branch matches, but Paper also fires after a 60-tick timeout and exposes isTimeout(); the hook only handles the client report and passes no timeout fact. No key. |
| `player_clone` | **Gap:** none | No Paper event for copying player data to the new player after death or End return was provided; no cited event covers the moment. No key. |
| `player_command` | **Partial:** [`PlayerCommandPreprocessEvent`][paper-PlayerCommandPreprocessEvent] | Matches the early server-side player-command moment and cancellation, with the player and command text, but setMessage(String) can rewrite the command while the hook has no command output. No key. |

[paper-EntityEquipmentChangedEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/entity/EntityEquipmentChangedEvent.html
[paper-PlayerPickupExperienceEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerPickupExperienceEvent.html
[paper-BlockPistonExtendEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockPistonExtendEvent.html
[paper-BlockPistonRetractEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockPistonRetractEvent.html
[paper-PlayerAdvancementDoneEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerAdvancementDoneEvent.html
[paper-PlayerAdvancementCriterionGrantEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerAdvancementCriterionGrantEvent.html
[paper-PlayerArmSwingEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerArmSwingEvent.html
[paper-PlayerAttackEntityCooldownResetEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerAttackEntityCooldownResetEvent.html
[paper-PlayerBedLeaveEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerBedLeaveEvent.html
[paper-PlayerBucketEntityEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerBucketEntityEvent.html
[paper-PlayerBundleItemSelectEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/inventory/PlayerBundleItemSelectEvent.html
[paper-PlayerChangeBeaconEffectEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerChangeBeaconEffectEvent.html
[paper-PlayerPortalEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerPortalEvent.html
[paper-PlayerClientLoadedWorldEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerClientLoadedWorldEvent.html
[paper-ServerCommandEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/server/ServerCommandEvent.html
[paper-UnknownCommandEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/command/UnknownCommandEvent.html
[paper-PlayerCommandSendEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerCommandSendEvent.html

[paper-PlayerChangedWorldEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerChangedWorldEvent.html

[paper-PlayerCommandPreprocessEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerCommandPreprocessEvent.html
## Event counterparts: base player hooks (arismoko/horizon#1015)

Paper API 26.3 (`26.3.build.244-stable`), public Javadoc only; Horizon cited by hook function name, not line.
Scope: the 20 `base.rs` player hooks in this batch, each mapped by moment, side, information and result.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `player_commands_sent` | **Partial:** [`PlayerCommandSendEvent`][paper-PlayerCommandSendEvent] | Called when the command list is sent to a player (join and level change), the hook's moment, on the server; names are removed from the sent tree through the event's command collection, matching `out = "names"`, and it is non-cancellable like the collect contract. Not a full match: the resent tree keeps the last answer for a player, so the listener sees later sends differently from Paper's per-send collection. |
| `player_container_close` | **Partial:** [`InventoryCloseEvent`][paper-InventoryCloseEvent] | Called when a player closes an inventory, server-side and non-cancellable; its documented reasons include replacement, disconnect and death. However, it exposes a close reason absent from the hook, and its Javadoc does not establish the hook's after-cleanup moment. No key. |
| `player_container_open` | **Partial:** [`InventoryOpenEvent`][paper-InventoryOpenEvent] | Covers a player opening an inventory on the server, but cancellation prevents the screen showing, whereas the hook listens after the menu opened. The listen-only hook cannot cancel the open or change the event's title override. No key. |
| `player_deep_sleep` | **Partial:** [`PlayerDeepSleepEvent`][paper-PlayerDeepSleepEvent] | Same moment (slept long enough to count for the night skip), same server side, same player information; but the event is cancellable and the listen-only hook cannot stop the player counting as deeply asleep, so the result differs. No key. |
| `player_destroy_item` | **Partial:** [`PlayerItemBreakEvent`][paper-PlayerItemBreakEvent] | Same break moment and server side, and the broken stack is exposed; the event gives no equipment slot, which the hook passes (`mainhand` … `saddle`, armour included), so the information differs even though both are listen/non-cancellable. No key. |
| `player_died` | **Partial:** [`PlayerDeathEvent`][paper-PlayerDeathEvent] | Same server-side player death, but Paper exposes a death message component and damage source rather than guaranteeing Horizon's optional death-message translation key. It is cancellable and permits changing death outcomes, whereas the hook only listens where ServerPlayer.die starts; the class Javadoc does not establish an identical firing point. No key. |
| `player_edit_book` | **Partial:** [`PlayerEditBookEvent`][paper-PlayerEditBookEvent] | Same server-side edit/signing attempt and cancellation, but the hook passes only page count and signing, not the slot or old/new BookMeta contents. Paper can also replace the BookMeta and signing result; the hook only cancels. No key. |
| `player_egg_throw` | **Gap:** none (nearest [`PlayerEggThrowEvent`][paper-PlayerEggThrowEvent]) | The hook listens after egg launch and use; the candidate concerns a thrown egg possibly hatching and exposes mutable hatch data, not a documented post-launch notification. No evidenced event covers this exact moment. No key. |
| `player_elytra_boost` | **Partial:** [`PlayerElytraBoostEvent`][paper-PlayerElytraBoostEvent] | Same firework boost, but the event fires before the rocket is consumed (cancellable, with `shouldConsume()`), while the hook reports after launch and consumption; the listen-only hook cannot express that cancel. No key. |
| `player_exp_cooldown_change` | **Partial:** [`PlayerExpCooldownChangeEvent`][paper-PlayerExpCooldownChangeEvent] | Covers a server player's experience cooldown change and exposes the new ticks, but also exposes a change reason absent from the hook. Its setNewCooldown() changes the result, whereas the hook only listens after the value changed. No key. |
| `player_fish` | **Partial:** [`PlayerFishEvent`][paper-PlayerFishEvent] | The documented FISHING, BITE, CAUGHT_FISH, REEL_IN and CAUGHT_ENTITY states cover cast, bite and reel-in cases; BITE is explicitly the ready-to-reel-in bite, so FishHookStateChangeEvent is unnecessary. However, Paper exposes the hook entity, hand, caught entity and experience and can setExpToDrop(), while Horizon passes only the state and cancellation before reel-in processing. No key. |
| `player_flower_pot_manipulate` | **Partial:** [`PlayerFlowerPotManipulateEvent`][paper-PlayerFlowerPotManipulateEvent] | Same pot, item and player, but the event fires as the player is about to place the item (cancellable, `isPlacing()`), while the hook reports after the pot state was stored and the item consumed; the listen-only hook cannot express the cancel. No key. |
| `player_game_mode_changed` | **Partial:** [`PlayerGameModeChangeEvent`][paper-PlayerGameModeChangeEvent] | Same server-side mode-change attempt and cancellation, with the target mode and the previous mode readable from the player. However, Paper exposes getCause() and a mutable cancelMessage(), neither of which the hook passes or lets mods change; filtering DEFAULT_GAMEMODE does not supply those missing facts/results. No key. |
| `player_input` | **Partial:** [`PlayerInputEvent`][paper-PlayerInputEvent] | Called when a player sends updated input to the server, server side, with the `Input` object carrying the same flags the hook passes; non-cancellable, matching the listen contract. Not a full match: the hook fires only for server guests, not for the host's own player, which Paper reports. |
| `player_insert_lectern_book` | **Partial:** [`PlayerInsertLecternBookEvent`][paper-PlayerInsertLecternBookEvent] | Same player, lectern block and book, but the event fires on the click before insertion (cancellable, `setBook()`), while the hook reports after the block entity and inventory changed; the listen-only hook cannot express the cancel. No key. |
| `player_item_cooldown` | **Partial:** [`PlayerItemCooldownEvent`][paper-PlayerItemCooldownEvent] | Same server-side cooldown start with item type, ticks and cancellation, but inherited getCooldownGroup() exposes a group absent from the hook. Inherited setCooldown() can change the duration, whereas Horizon only permits cancellation. No key. |
| `player_item_frame_change` | **Partial:** [`PlayerItemFrameChangeEvent`][paper-PlayerItemFrameChangeEvent] | Covers putting, rotating and removing an item with `getAction()` and the stack, but not the hook's `break` action, and it fires before the change (cancellable), so the resulting rotation the hook passes is not exposed and the cancel cannot be expressed. No key. |
| `player_item_mend` | **Partial:** [`PlayerItemMendEvent`][paper-PlayerItemMendEvent] | Same server-side mending repair, with the item and repair amount; Horizon's experience is the original orb value, not Paper's getConsumedExperience(), with the orb separately available through getExperienceOrb(). Paper also exposes the equipment slot and can cancel or change the repair amount and durability-to-XP operation, which the listen-only hook cannot express. No key. |
| `player_jump` | **Partial:** [`PlayerJumpEvent`][paper-PlayerJumpEvent] | Same detected jump, server side, with the player; the event is cancellable (and can move `from`, with `getTo()`), while the hook only listens after the jump was applied, so the cancel cannot be expressed. No key. |
| `player_kicked` | **Partial:** [`PlayerKickEvent`][paper-PlayerKickEvent] | Covers a server kick with the player and reason(), but the event is cancellable and permits changing the reason and leave message before the kick. Horizon only listens after the hub recorded the rendered reason and cannot express those results or the event's cause. No key. |

[paper-PlayerDeepSleepEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerDeepSleepEvent.html
[paper-PlayerItemBreakEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerItemBreakEvent.html
[paper-PlayerDeathEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/PlayerDeathEvent.html
[paper-PlayerEditBookEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerEditBookEvent.html
[paper-PlayerEggThrowEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerEggThrowEvent.html
[paper-PlayerElytraBoostEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerElytraBoostEvent.html
[paper-PlayerExpCooldownChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerExpCooldownChangeEvent.html
[paper-PlayerFishEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerFishEvent.html
[paper-FishHookStateChangeEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/entity/FishHookStateChangeEvent.html
[paper-PlayerFlowerPotManipulateEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerFlowerPotManipulateEvent.html
[paper-PlayerGameModeChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerGameModeChangeEvent.html
[paper-PlayerInputEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerInputEvent.html
[paper-PlayerInsertLecternBookEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerInsertLecternBookEvent.html
[paper-PlayerItemCooldownEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerItemCooldownEvent.html
[paper-PlayerItemFrameChangeEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerItemFrameChangeEvent.html
[paper-PlayerItemMendEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerItemMendEvent.html
[paper-PlayerJumpEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerJumpEvent.html

[paper-InventoryCloseEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/InventoryCloseEvent.html

[paper-PlayerKickEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerKickEvent.html
## Event counterparts: base player hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc only; Horizon hooks are cited by function name, never by line, and Paper by event class only.
Scope = the 20 `base-player` hooks below (join/quit, login, respawn, portal, lectern/loom/sign/recipe stations, map, name tag, pick, arrows, projectile, saved player data); Covered needs the same moment, side, facts and result.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `player_launch_projectile` | **Partial:** [`PlayerLaunchProjectileEvent`][paper-PlayerLaunchProjectileEvent] | Same server moment — a player shoots a projectile — and `getProjectile()`/`getItemStack()` carry the projectile and its item, which the hook reports as `item`. But the event is cancellable and `setShouldConsume` can change the shot, while the listen-only hook fires after the projectile was created and cannot refuse it. No key. |
| `player_lectern_page_change` | **Partial:** [`PlayerLecternPageChangeEvent`][paper-PlayerLecternPageChangeEvent] | `getLectern()`, `getOldPage()`, `getNewPage()` and `setNewPage()` describe the same server action the hook reports through `pos` and `page`, though the class carries no Javadoc, so the moment is inferred from the methods alone. The event is cancellable and can rewrite the new page, while the listen-only hook fires after the page and redstone pulse changed. No key. |
| `player_level_change` | **Covered:** [`PlayerLevelChangeEvent`][paper-PlayerLevelChangeEvent] | Same server moment: the player's experience level has changed, and `getOldLevel()`/`getNewLevel()` expose the hook's `from`/`to` (the latter documented as current). Non-cancellable with no outcome setter, matching the listen-only hook. Key added. |
| `player_load_from_file` | **Gap:** none (nearest [`EntityRemoveFromWorldEvent`][paper-EntityRemoveFromWorldEvent]) | No Paper event covers a player's saved data being read and applied before they are placed, and none exposes the world's player directory. The nearest candidate fires when an entity is removed from a world, an unrelated moment. No key. |
| `player_logged_in` | **Partial:** [`PlayerJoinEvent`][paper-PlayerJoinEvent] | Same server moment: the player joins after placement in loaded chunks, and `Player` carries UUID and name. Although non-cancellable, `joinMessage(...)` changes or suppresses the broadcast; the listen-only hook has no corresponding output. No key. |
| `player_logged_out` | **Partial:** [`PlayerQuitEvent`][paper-PlayerQuitEvent] | Called when a player leaves a server, with `Player` carrying UUID and name. Although non-cancellable, `quitMessage(...)` changes the broadcast; the listen-only hook has no corresponding output. No key. |
| `player_login` | **Partial:** [`AsyncPlayerPreLoginEvent`][paper-AsyncPlayerPreLoginEvent] | `getName()`/`getUniqueId()` provide the identity, and `allow()`/`disallow(...)` with a kick message resemble the refusal result. But the event runs asynchronously at pre-login, rather than after the checks and reservation required by the hook; deprecated `PlayerLoginEvent` directs pre-login logic to `PlayerConnectionValidateLoginEvent`, whose Javadoc likewise does not establish that accepted stage. No key. |
| `player_loom_pattern_select` | **Partial:** [`PlayerLoomPatternSelectEvent`][paper-PlayerLoomPatternSelectEvent] | Same server action — a player selects a banner pattern in a loom — and `getPatternType()`/`getLoomInventory()` carry the choice the hook reports as the button index. The event is cancellable with a `setPatternType` setter, while the listen-only hook fires after the loom accepted the button. No key. |
| `player_map_filled` | **Partial:** [`PlayerMapFilledEvent`][paper-PlayerMapFilledEvent] | Same server action: creating a filled map from an empty map, with `getCreatedMap()` exposing its id. However, `setCreatedMap(...)` replaces the map that will be created, while the hook only listens after the map and saved data were created. No key. |
| `player_move` | **Covered:** [`PlayerMoveEvent`][paper-PlayerMoveEvent] | The same moment: Paper fires it from `handleMovePlayer` for a report that changed the position or look, before it is applied, and a cancel puts the player back. The hook is heard once for each validated report that differs from the player's last reported location, with the exact `from` and `to` and both rotations, and its `cancel` vetoes the report and corrects the client to the accepted state. Not heard: a player's first tick in a level (after a join or a level change), a teleport's confirmation, the veto's own correction, a report that arrives while a teleport is unconfirmed, a passage crossing, and a rider (`vehicle_moved` hears the vehicle). Paper's thresholds (1/256 square block, ten degrees) are not applied by the engine: the mod filters. Paper's `setTo` is not carried (cancel only). Key added. |
| `player_name_entity` | **Partial:** [`PlayerNameEntityEvent`][paper-PlayerNameEntityEvent] | Same server action — renaming a mob — and `getEntity()`, `getName()` and `isPersistent()` carry what the hook passes as `entity_id` and `entity_name`. The event is cancellable and its setters can rename or replace the entity, while the hook listens after the name and persistence changed. No key. |
| `player_open_sign` | **Partial:** [`PlayerOpenSignEvent`][paper-PlayerOpenSignEvent] | `getSign()` and `getSide()` give the player, sign position and front/back side the hook passes, at the moment a player begins editing a sign. But the event is cancellable (cancelling stops the editor) and fires as editing begins, whereas the hook fires after the editor state exists and only listens. No key. |
| `player_passage_cross` | **Gap:** none (nearest `PlayerTeleportEvent`) | Paper has no passage within a level, and no client-first crossing: `PlayerTeleportEvent` fires for a teleport the server starts, with a mutable destination. `player_passage_cross` fires on the server for a crossing the client already made, before the server works out the destination, with the player, the passage and the feet before, and can only cancel. No key. |
| `player_pick_item` | **Partial:** [`PlayerPickBlockEvent`][paper-PlayerPickBlockEvent] / [`PlayerPickEntityEvent`][paper-PlayerPickEntityEvent] | Both events fire before the slot swap, expose the block or entity, and support cancellation. Their common `PlayerPickItemEvent` base covers both forms, but its source/target slot setters change the inventory outcome, which the hook cannot express. No key. |
| `player_pickup_arrow` | **Partial:** [`PlayerPickupArrowEvent`][paper-PlayerPickupArrowEvent] | Same server pickup action: `getArrow()` exposes the arrow and inherited `getItem()` exposes its pickup item. The event is cancellable at pickup, while the hook only listens after the arrow was removed and the pickup applied. No key. |
| `entity_travel_to_dimension` | **Partial:** [`PlayerPortalEvent`][paper-PlayerPortalEvent] / [`EntityPortalEvent`][paper-EntityPortalEvent] | Horizon asks before travel, including passengers recursively, and cancellation leaves that entity behind; callback discard also prevents travel. The entity handle, target dimension and cause cover part of these portal events, but Paper's destination locations and portal-search setters have no hook outputs. Horizon also asks for cross-level commands and End gateways, beyond `PlayerPortalEvent`'s documented exit-portal scope. No key. |
| `player_ready_arrow` | **Partial:** [`PlayerReadyArrowEvent`][paper-PlayerReadyArrowEvent] | Documented as fired when a player is firing a bow and the server chooses the arrow; `getArrow()`/`getBow()` carry the item the hook passes, at the same server moment. The event is cancellable, while the listen-only hook cannot refuse the shot. No key. |
| `player_recipe_book_click` | **Partial:** [`PlayerRecipeBookClickEvent`][paper-PlayerRecipeBookClickEvent] | `getRecipe()` (a `NamespacedKey`) and `isMakeAll()` match the hook's `recipe` and shift-click `all` at the same server click. The event is cancellable and `setRecipe`/`setMakeAll` can change the request, while the hook only listens; the same-named `org.bukkit` event in the dossier is the deprecated alternative. No key. |
| `player_recipe_book_settings_change` | **Covered:** [`PlayerRecipeBookSettingsChangeEvent`][paper-PlayerRecipeBookSettingsChangeEvent] | Documented as called when a player changes recipe book settings, the hook's server moment; `getRecipeBookType()`, `isOpen()` and `isFiltering()` carry exactly the book, open and filtering facts (the enum stands in for the protocol id). Non-cancellable, matching the listen-only hook. Key added. |
| `player_recipe_discovered` | **Partial:** [`PlayerRecipeDiscoverEvent`][paper-PlayerRecipeDiscoverEvent] | Same server recipe-unlock moment, with `getRecipe()` carrying the recipe id and cancellation keeping it locked. However, `shouldShowNotification(boolean)` also controls the discovery toast, which the hook cannot change. No key. |
| `player_respawn_position` | **Partial:** [`PlayerRespawnEvent`][paper-PlayerRespawnEvent] | `setRespawnLocation(...)` changes the level, position and orientation, and inherited `isMissingRespawnBlock()` supplies the missing-block fact. However, there is no corresponding control for `copy_original_spawn_position`, and Paper fires during respawn before the reset whereas Horizon may resolve the point after placement and chunk loading. No key. |
| `player_respawned` | **Partial:** [`PlayerPostRespawnEvent`][paper-PlayerPostRespawnEvent] | Documented as fired after a player has respawned, the hook's server moment at the end of the respawn with the player placed, and the event's player carries the UUID and name. Non-cancellable, matching the listen-only hook; preferred over `PlayerRespawnEvent`, which fires during the respawn. Not a full match: the hook is not told for the End-exit respawn, which Paper's event also reports. |

[paper-PlayerLaunchProjectileEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerLaunchProjectileEvent.html
[paper-PlayerLecternPageChangeEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerLecternPageChangeEvent.html
[paper-PlayerQuitEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerQuitEvent.html
[paper-AsyncPlayerPreLoginEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/AsyncPlayerPreLoginEvent.html
[paper-PlayerLoomPatternSelectEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerLoomPatternSelectEvent.html
[paper-PlayerMapFilledEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerMapFilledEvent.html
[paper-PlayerNameEntityEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerNameEntityEvent.html
[paper-PlayerOpenSignEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerOpenSignEvent.html
[paper-PlayerPickBlockEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerPickBlockEvent.html
[paper-PlayerPickEntityEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerPickEntityEvent.html
[paper-PlayerPickupArrowEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerPickupArrowEvent.html
[paper-PlayerPortalEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerPortalEvent.html
[paper-PlayerRecipeBookClickEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerRecipeBookClickEvent.html
[paper-PlayerRecipeBookSettingsChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerRecipeBookSettingsChangeEvent.html
[paper-PlayerRecipeDiscoverEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerRecipeDiscoverEvent.html
[paper-PlayerRespawnEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerRespawnEvent.html
[paper-PlayerPostRespawnEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerPostRespawnEvent.html

[paper-PlayerLevelChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerLevelChangeEvent.html
## Event counterparts: base player hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc; Horizon cited by function name; scope = the 20 `base.rs` player hooks below, and `player_start_spectating_entity` beside its mirror.
Server side is assumed except for the two `both` tick hooks; a server-half counterpart could qualify, but Covered still requires moment, information and result to agree, and only Covered rows get a `paper` key.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `player_riptide` | **Partial:** [`PlayerRiptideEvent`][paper-PlayerRiptideEvent] | The server's riptide launch and the player match, but the event is cancellable and also carries the trident item and velocity, while the hook is a listen-only notification fired after the launch is applied and passes neither. No key. |
| `player_save_to_file` | **Gap:** none (nearest [`WorldSaveEvent`][paper-WorldSaveEvent]) | A world save is not the hook's successful per-player file-write completion, which can arrive on a later tick or when a player leaves. The event supplies neither the player nor the player-data directory. No key. |
| `player_shear_entity` | **Partial:** [`PlayerShearEntityEvent`][paper-PlayerShearEntityEvent] | The server-side pre-shear moment, player, entity, hand and cancellation match, but Paper also exposes the shears and drops and permits replacing the drops with `setDrops()`. The hook passes neither item nor drops and only returns cancellation. No key. |
| `player_shield_disable` | **Partial:** [`PlayerShieldDisableEvent`][paper-PlayerShieldDisableEvent] | Same server moment, just before the shield cooldown, and `getCooldown()`/`setCooldown()` expose the hook's `ticks`. The event also exposes the damager, is cancellable and lets listeners change the cooldown; the listen-only hook supplies no damager and cannot express those results. No key. |
| `player_sprint` | **Partial:** [`PlayerToggleSprintEvent`][paper-PlayerToggleSprintEvent] | The server-side sprint toggle and the new `isSprinting()` state match. The event is cancellable, so it is the about-to-change call, while the hook hears the change after the player is already sprinting and cannot cancel. No key. |
| `player_start_spectating_entity` | **Partial:** [`PlayerStartSpectatingEntityEvent`][paper-PlayerStartSpectatingEntityEvent] | Same start-spectating moment, player and spectated entity on the server. Diverges: the event is cancellable and precedes the camera change, while the listen-only hook is heard after the camera moved and the client was told, and cannot cancel. No key. |
| `player_start_tracking` | **Partial:** [`PlayerTrackEntityEvent`][paper-PlayerTrackEntityEvent] | Paper exposes the player and entity about to be tracked, including its ID, UUID and type, but cancellation prevents showing it; the listen-only hook runs after pairing has started. No key. |
| `player_stat_award` | **Partial:** [`PlayerStatisticIncrementEvent`][paper-PlayerStatisticIncrementEvent] | Paper reports cancellable statistic increments, exposing the statistic, material or entity type and previous/new counts from which the amount can be derived. Its Javadoc explicitly excludes some high-frequency statistics, while the hook covers awards including those statistics. No key. |
| `player_stonecutter_recipe_select` | **Partial:** [`PlayerStonecutterRecipeSelectEvent`][paper-PlayerStonecutterRecipeSelectEvent] | Name and the stonecutter selection match, but the class has no Javadoc to confirm the moment, it reports a `StonecuttingRecipe` where the hook reports a button index, and it is cancellable and rewritable while the hook is listen-only. No key. |
| `player_stop_spectating_entity` | **Partial:** [`PlayerStopSpectatingEntityEvent`][paper-PlayerStopSpectatingEntityEvent] | Same stop-spectating moment and spectated entity on the server. The event is cancellable (it precedes the camera return the hook reports after it), and the listen-only hook cannot cancel. No key. |
| `player_stop_tracking` | **Partial:** [`PlayerUntrackEntityEvent`][paper-PlayerUntrackEntityEvent] | Paper exposes the player and entity, including its ID, UUID and type, and is non-cancellable. However, `getEntity()` describes the entity that will be untracked; the public Javadoc does not establish the hook's after-removal notification or its delivery after logout. No key. |
| `player_stop_using_item` | **Partial:** [`PlayerStopUsingItemEvent`][paper-PlayerStopUsingItemEvent] | The server-side stop-using notification and non-cancellable result match, but Paper exposes `getItem()` and `getTicksHeldFor()` while the hook passes only the player identity. No key. |
| `player_swap_hands` | **Partial:** [`PlayerSwapHandItemsEvent`][paper-PlayerSwapHandItemsEvent] | The server-side swap-hands hotkey moment and cancellation match, but Paper exposes both resulting hand items and lets listeners replace them with setters; the hook passes no items and only returns cancellation. No key. |
| `player_swap_with_equipment_slot` | **Partial:** [`PlayerSwapWithEquipmentSlotEvent`][paper-PlayerSwapWithEquipmentSlotEvent] | Covers an equipment-slot swap on the server with the slot exposed, but it is a cancellable pre-swap event with item information the hook does not pass, whereas the hook is a non-cancellable notification after both slots changed, and the event is any equipment slot rather than the hands. No key. |
| `player_switch_hotbar_slot_post` | **Gap:** none (nearest [`PlayerItemHeldEvent`][paper-PlayerItemHeldEvent]) | Paper exposes the old and new selected hotbar slots, but its cancellable selection event does not establish a notification after the selection and main-hand item use have changed. No key. |
| `player_switch_hotbar_slot_pre` | **Covered:** [`PlayerItemHeldEvent`][paper-PlayerItemHeldEvent] | Fired on the server for a held hotbar slot change, with `getPreviousSlot()` and `getNewSlot()` matching the hook's old/new indices and `getPlayer()` identifying the player and entity; cancellation matches keeping the old selection before it changes. Key added. |
| `player_take_lectern_book` | **Partial:** [`PlayerTakeLecternBookEvent`][paper-PlayerTakeLecternBookEvent] | The player, lectern and book match on the server. The event is cancellable before the book leaves the lectern, while the hook is a listen-only notification after the menu and block entity changed. No key. |
| `player_teleport_end_gateway` | **Partial:** [`PlayerTeleportEndGatewayEvent`][paper-PlayerTeleportEndGatewayEvent] | The server-side pre-gateway travel, player, gateway and cancellation match, but Paper inherits teleport origin and destination information and destination rewriting via `setTo()`, which the hook does not expose. No key. |
| `player_teleported` | **Gap:** none (nearest [`PlayerTeleportEvent`][paper-PlayerTeleportEvent]) | Paper's nearest event is the cancellable about-to-teleport check; the hook is a listen-only notification after the command teleport has reached the player and their client, which no cited event covers. No key. |
| `player_tick_post` | **Gap:** none | No Paper event is identified for the completed player tick; a server-only event could cover the server half of this `both` hook, but no candidate is established. No key. |
| `player_tick_pre` | **Gap:** none | No Paper event is identified before a player tick; a server-only event could cover the server half of this `both` hook, but no candidate is established. No key. |

[paper-PlayerRiptideEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerRiptideEvent.html
[paper-WorldSaveEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/WorldSaveEvent.html
[paper-PlayerShearEntityEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerShearEntityEvent.html
[paper-PlayerShieldDisableEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerShieldDisableEvent.html
[paper-PlayerToggleSprintEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerToggleSprintEvent.html
[paper-PlayerStonecutterRecipeSelectEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerStonecutterRecipeSelectEvent.html
[paper-PlayerStartSpectatingEntityEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerStartSpectatingEntityEvent.html
[paper-PlayerStopSpectatingEntityEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerStopSpectatingEntityEvent.html
[paper-PlayerSwapHandItemsEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerSwapHandItemsEvent.html
[paper-PlayerSwapWithEquipmentSlotEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerSwapWithEquipmentSlotEvent.html
[paper-PlayerInventorySlotChangeEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerInventorySlotChangeEvent.html
[paper-PlayerTakeLecternBookEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerTakeLecternBookEvent.html
[paper-PlayerTeleportEndGatewayEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerTeleportEndGatewayEvent.html

[paper-PlayerTrackEntityEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerTrackEntityEvent.html

[paper-PlayerUntrackEntityEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerUntrackEntityEvent.html

[paper-PlayerStatisticIncrementEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerStatisticIncrementEvent.html

[paper-PlayerItemHeldEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerItemHeldEvent.html
## Event counterparts: base player hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc; Horizon cited by function
name; scope = the hooks below.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `player_toggle_entity_age_lock` | **Partial:** [`PlayerToggleEntityAgeLockEvent`][paper-PlayerToggleEntityAgeLockEvent] | The age-lock toggle and new `isAgeLocked()` value agree, but the event additionally exposes item and hand and is cancellable, while the hook only reports player, entity and lock state after the lock and item use changed. No key. |
| `player_toggle_flight` | **Partial:** [`PlayerToggleFlightEvent`][paper-PlayerToggleFlightEvent] | The client-requested change of flying state reaches the player (`isFlying()`), the event is cancellable, and the hook's `cancel` restores the previous state and resyncs abilities exactly as the event's cancel does. Not a full match: the hook is not told for a LAN host's own player nor for flight a game-mode change starts or ends, which Paper's event also reports for the host. |
| `player_toggle_sneak` | **Partial:** [`PlayerToggleSneakEvent`][paper-PlayerToggleSneakEvent] | The same sneaking change with `isSneaking()`, but the event is cancellable and the listen-only hook cannot express that cancel, and it runs before the state change where the hook runs after the hub applied it. No key. |
| `player_totem` | **Partial:** [`EntityResurrectEvent`][paper-EntityResurrectEvent] | The totem-resurrection opportunity overlaps, but the event is cancellable and can run without a totem, initially cancelled; `getHand()` identifies the totem hand. The hook only reports a server player whose save is already decided, just before consumption and effects, and cannot veto it or supply the hand. No key. |
| `player_trade` | **Partial:** [`PlayerTradeEvent`][paper-PlayerTradeEvent] | The completed trade with a villager or wandering trader and the merchant handle (`getMerchant()`) match, but the event is cancellable and raised as the trade is made, while the hook only listens once the trade completed. No key. |
| `player_unleash_entity` | **Partial:** [`PlayerUnleashEntityEvent`][paper-PlayerUnleashEntityEvent] | Same player action and entity, but the event is raised prior to the unleash and is cancellable, while the hook listens after the leash was cleared. No key. |
| `player_velocity` | **Partial:** [`PlayerVelocityEvent`][paper-PlayerVelocityEvent] | The applied velocity is the same fact (`getVelocity()`), but the event is cancellable and exposes `setVelocity`, while the hook only listens to the motion the server syncs to the client. No key. |
| `portal_create` | **Partial:** [`PortalCreateEvent`][paper-PortalCreateEvent] | Nether portal and End platform creation match (`getBlocks()` gives the placed blocks, `getReason()` says which), but the event is cancellable and runs before the blocks are placed, while the hook fires with the blocks already in the world. No key. |
| `potion_splash` | **Partial:** [`PotionSplashEvent`][paper-PotionSplashEvent], [`LingeringPotionSplashEvent`][paper-LingeringPotionSplashEvent] | The item and position of the burst match, but splash and lingering arrivals are separate cancellable events: splash exposes affected entities and intensity setters, while lingering exposes the area-effect cloud and its creation policy. The hook only listens after the hit effects applied. No key. |
| `prepare_hopper_transfer` | **Partial:** [`PrepareHopperTransferEvent`][paper-PrepareHopperTransferEvent] | Both concern preparing a hopper transfer, but the event already exposes source and destination inventories and can exclude source slots with `setSlotAllowed()`. The hook only reports the hopper position once before pushing or pulling, with no slot-selection result. No key. |
| `prepare_item_craft` | **Covered:** [`PrepareItemCraftEvent`][paper-PrepareItemCraftEvent] | The server crafting-result preparation matches: `getView()` identifies the player and `getInventory()` exposes the result through [`CraftingInventory.setResult()`][paper-CraftingInventory], matching writable `result`. The inventory carries the table location via [`Inventory.getLocation()`][paper-Inventory] (nullable for virtual containers, including the inventory grid), and neither is cancellable. Key added. |
| `prepare_item_enchant` | **Partial:** [`PrepareItemEnchantEvent`][paper-PrepareItemEnchantEvent] | Same offer-calculation moment with enchanter, table block and offered level costs; [`getOffers()`][paper-PrepareItemEnchantEvent] allows modifying offers, including their costs via [`EnchantmentOffer.setCost()`][paper-EnchantmentOffer], while `getExpLevelCostsOffered()` is deprecated. The event is cancellable and also exposes the item, enchantments and bookshelf bonus, while the hook only supplies player, position and writable costs and cannot cancel. No key. |
| `prepare_smithing` | **Covered:** [`PrepareSmithingEvent`][paper-PrepareSmithingEvent] | Fired when a smithing-table result is prepared; it is non-cancellable and offers `setResult` inherited from [`PrepareResultEvent`][paper-PrepareResultEvent], matching the hook's writable `result`, with the inventory carrying the player and table. Key added. |
| `projectile_impact` | **Partial:** [`ProjectileHitEvent`][paper-ProjectileHitEvent] | The projectile and hit entity/block/face match, but the event exposes no exact impact point for the hook's `at`. Its cancellation Javadoc explicitly leaves block collisions and explosions to occur unless their respective events are cancelled, while the hook suppresses the whole impact and lets the projectile fly on. No key. |
| `projectile_launch` | **Partial:** [`ProjectileLaunchEvent`][paper-ProjectileLaunchEvent] | The same projectile added for a player or another source, but the event is cancellable and raised before the launch completes, while the hook listens after the projectile joined the level. No key. |
| `raid_spawn_wave` | **Partial:** [`RaidSpawnWaveEvent`][paper-RaidSpawnWaveEvent] | A spawned wave with its raiders and patrol leader (non-cancellable, like the hook) matches, and the inherited raid handle exposes [`Raid.getSpawnedGroups()`][paper-Raid]. However, `Raid.getLocation()` is the raid center, not the wave spawn position passed as `pos`, which the event does not document. No key. |
| `raid_state_change` | **Partial:** [`RaidStopEvent`][paper-RaidStopEvent], [`RaidFinishEvent`][paper-RaidFinishEvent], [`RaidTriggerEvent`][paper-RaidTriggerEvent] | The hook's four states are split across separate events: trigger is cancellable (the hook cannot veto it), `RaidStopEvent` covers many stop reasons beyond the hook's `stopped`, and no single event carries the `triggered`/`victory`/`loss`/`stopped` value. No key. |
| `random_tick` | **Gap:** none | No Paper/Bukkit event marks a block's random tick, and the hook's `state`, `pos` and `handled` (skip the vanilla tick) have no Javadoc counterpart, so a port has nothing to listen to. No key. |
| `register_commands` | **Partial:** [`LifecycleEvents.COMMANDS`][paper-LifecycleEvents] | Paper does have a command-registration lifecycle event, supplying a [`Commands` registrar][paper-Commands] rather than filtering the client command list with `PlayerCommandSendEvent`. Its registration/reload lifecycle and ability to override vanilla command literals differ from the hook's world-opening or explicit refresh collection, which omits vanilla names; the generic `ReloadableRegistrarEvent` alone does not identify this command-specific event. No key. |
| `release_using` | **Partial:** [`PlayerStopUsingItemEvent`][paper-PlayerStopUsingItemEvent] | The item and `getTicksHeldFor()` match on the server half, but the event is player-only (the hook also fires for mobs) and has no way to set the hook's `used` result. No key. |

[paper-PlayerToggleEntityAgeLockEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerToggleEntityAgeLockEvent.html
[paper-PlayerToggleFlightEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerToggleFlightEvent.html
[paper-PlayerToggleSneakEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerToggleSneakEvent.html
[paper-PlayerTradeEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerTradeEvent.html
[paper-PlayerUnleashEntityEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerUnleashEntityEvent.html
[paper-PlayerVelocityEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerVelocityEvent.html
[paper-PortalCreateEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/PortalCreateEvent.html
[paper-PotionSplashEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/PotionSplashEvent.html
[paper-PrepareHopperTransferEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/inventory/PrepareHopperTransferEvent.html
[paper-PrepareItemCraftEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/PrepareItemCraftEvent.html
[paper-PrepareItemEnchantEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/enchantment/PrepareItemEnchantEvent.html
[paper-PrepareSmithingEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/PrepareSmithingEvent.html
[paper-PrepareResultEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/inventory/PrepareResultEvent.html
[paper-ProjectileLaunchEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/ProjectileLaunchEvent.html
[paper-RaidSpawnWaveEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/raid/RaidSpawnWaveEvent.html
[paper-RaidStopEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/raid/RaidStopEvent.html
[paper-RaidFinishEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/raid/RaidFinishEvent.html
[paper-RaidTriggerEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/raid/RaidTriggerEvent.html
[paper-PlayerCommandSendEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerCommandSendEvent.html
[paper-EntityResurrectEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityResurrectEvent.html
[paper-CraftingInventory]: https://jd.papermc.io/paper/26.3/org/bukkit/inventory/CraftingInventory.html
[paper-Inventory]: https://jd.papermc.io/paper/26.3/org/bukkit/inventory/Inventory.html
[paper-EnchantmentOffer]: https://jd.papermc.io/paper/26.3/org/bukkit/enchantments/EnchantmentOffer.html
[paper-LingeringPotionSplashEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/LingeringPotionSplashEvent.html
[paper-LifecycleEvents]: https://jd.papermc.io/paper/26.3/io/papermc/paper/plugin/lifecycle/event/types/LifecycleEvents.html
[paper-Commands]: https://jd.papermc.io/paper/26.3/io/papermc/paper/command/brigadier/Commands.html
[paper-Raid]: https://jd.papermc.io/paper/26.3/org/bukkit/Raid.html

## Event counterparts: base and resource hooks (arismoko/horizon#1015)

Paper API 26.3 (`26.3.build.244-stable`), public Javadoc; Horizon cited by function name; scope = the 20 `base.rs` hooks below.
Paper is server-only, so only the server side of `both` hooks is comparable; no Paper server code or invented event is used.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `resource_pack_status` | **Partial:** [`PlayerResourcePackStatusEvent`][paper-PlayerResourcePackStatusEvent] | Both listen for a client resource-pack answer on the server, but Paper exposes the pack UUID through getID(), which the hook omits. No key. |
| `right_click_block` | **Partial:** [`PlayerInteractEvent`][paper-PlayerInteractEvent] | Action.RIGHT_CLICK_BLOCK identifies the server interaction, with player, block, hit position/face, hand, item and separate block/item use controls. Paper cannot express cancellation_result and its swing/consumption choices. No key. |
| `run_mod_command` | **Gap:** none (nearest [`ServerCommandEvent`][paper-ServerCommandEvent]) | The cited event fires early for non-player commands, before permission and argument resolution, while the hook runs after both and exposes the mod invocation and `ran`. No key. |
| `screen` | **Gap:** none | No cited Paper event opens a block-entity screen or supplies the hidden-slot layout the hook collects; the dossier's candidates are unrelated inventory/recipe events. No key. |
| `sculk_bloom` | **Gap:** none (nearest [`SculkBloomEvent`][paper-SculkBloomEvent]) | Paper documents cursor creation at the cursor spawn block, including plugin-triggered creation; sculk_bloom observes the catalyst after its block state has begun blooming. The Javadoc does not establish that post-bloom moment or catalyst position; cursor charge and cancellation also differ. No key. |
| `server_about_to_start` | **Gap:** none | No cited Paper event marks the server's pre-world-load start; no dossier candidate covers the `dedicated` flag at that moment. No key. |
| `server_started` | **Partial:** [`ServerLoadEvent`][paper-ServerLoadEvent] | LoadType.STARTUP identifies completed server startup, but the Javadoc does not establish all worlds loaded before the first tick and first player, and the event lacks the dedicated flag. No key. |
| `server_starting` | **Gap:** none | No cited Paper event marks the post-`server_about_to_start`/pre-play start with the `dedicated` flag; no dossier candidate covers this moment. No key. |
| `server_stopped` | **Gap:** none | No cited Paper event marks the end of teardown after the final save and storage release, including failed startups; no dossier candidate covers it. No key. |
| `server_stopping` | **Gap:** none | No cited Paper event marks the start of teardown with worlds loaded and players still connected; no dossier candidate covers it. No key. |
| `server_tick` | **Partial:** [`ServerTickEndEvent`][paper-ServerTickEndEvent] | Paper documents the main-loop tick end; ServerTickStartEvent exposes the upcoming tick number. These separate events do not supply the combined start/end phases and lifecycle context, including world_running; the start event Javadoc does not establish precise pre-level ordering. No key. |
| `settings` | **Gap:** none | No cited Paper event collects per-mod settings-page buttons; the nearest dossier candidate is a player recipe-book setting change, not a mod page. No key. |
| `should_apply_effect_tick_this_tick` | **Gap:** none (nearest [`EntityEffectTickEvent`][paper-EntityEffectTickEvent]) | Paper documents effect application, including instant effects; the hook asks a content effect scheduling predicate every tick before deciding whether to apply it. The predicate supplies remaining tick_count and no entity, whereas Paper exposes an entity and cancels application. No key. |
| `sign_changed` | **Partial:** [`SignChangeEvent`][paper-SignChangeEvent] | Both fire for a player's checked sign-text edit on the server, with player, side, position and editable lines; the event is cancellable and lets the lines be rewritten. Not a full match: the hook is not told for the host's own sign edits on a LAN or single-player world, nor for the sign changes that are no text edit, while Paper reports the host's edit too. |
| `sleep_finished_time` | **Partial:** [`TimeSkipEvent`][paper-TimeSkipEvent] | Inherited getSkipReason() with NIGHT_SKIP identifies sleep skips, and getSkipAmount()/setSkipAmount() plus the world clock allow the target time to be read and changed; cancellation prevents the clock change. The Javadoc does not promise refusal of targets before current or the distinct gate before time_skip, so result and precise ordering are not established. No key. |
| `smoke_source` | **Gap:** none | No cited Paper event replaces or observes `CampfireBlock.isSmokeSource` for the block under a campfire. No key. |
| `spawn_change` | **Partial:** [`SpawnChangeEvent`][paper-SpawnChangeEvent] | Both listen on the server when the world spawn changes, and the world exposes the new spawn location. Paper additionally supplies getPreviousLocation(), which spawn_change does not pass; the information differs. No key. |
| `sponge_absorb` | **Partial:** [`SpongeAbsorbEvent`][paper-SpongeAbsorbEvent] | The event describes the same sponge absorption, but it fires with the world still unchanged and can cancel, while the hook fires after the sponge is wet and listens only. No key. |
| `state_for_placement` | **Gap:** none | No cited Paper event runs at `Block.getStateForPlacement` with the placement context, fluid state and replaceable answer; no dossier candidate covers it. No key. |
| `tab_complete` | **Partial:** [`TabCompleteEvent`][paper-TabCompleteEvent] | The event matches a server-side completion request and lets completions be changed, but the supplied Javadoc exposes no replaced byte range and the hook's `range` cannot be recovered; cancel semantics also differ. No key. |

[paper-ServerCommandEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/server/ServerCommandEvent.html
[paper-SculkBloomEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/SculkBloomEvent.html
[paper-SignChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/SignChangeEvent.html
[paper-TimeSkipEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/TimeSkipEvent.html
[paper-SpawnChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/SpawnChangeEvent.html
[paper-SpongeAbsorbEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/SpongeAbsorbEvent.html
[paper-TabCompleteEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/server/TabCompleteEvent.html

[paper-ServerLoadEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/server/ServerLoadEvent.html

[paper-ServerTickEndEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/server/ServerTickEndEvent.html

## Event counterparts: base-target hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc; Horizon cited by function name; scope = the base-target hooks below.
Server-only events are compared with the hook's server half; moments with no Paper/Bukkit event are recorded as gaps.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `target_hit` | **Partial:** [`TargetHitEvent`][paper-TargetHitEvent] | Paper describes the projectile hitting the target before its redstone signal and advancement can be cancelled; `target_hit` notifies after the output power is applied. Paper also exposes the projectile and allows signal-strength changes and cancellation, which the listen-only hook cannot express. No key. |
| `thunder_change` | **Partial:** [`ThunderChangeEvent`][paper-ThunderChangeEvent] | Same server-side thunder-state change, with `toThunderState` and cancellation matching the hook's `cancel`, but Paper exposes a cause that the command-only hook does not pass. No key. |
| `tick_block` | **Gap:** none | No Paper/Bukkit event covers a due scheduled block tick before `BlockState.tick`; no candidate is listed and Paper has no block-tick event. No key. |
| `time_skip` | **Partial:** [`TimeSkipEvent`][paper-TimeSkipEvent] | The event covers a server-side clock jump and can be cancelled, but the hook fires before the skip and exposes `from`, `to`, and cause, while Paper's event (through [`ClockTimeSkipEvent`][paper-ClockTimeSkipEvent]) also lets plugins change the skip amount, which the hook cannot express. No key. |
| `tnt_primed` | **Partial:** [`TNTPrimeEvent`][paper-TNTPrimeEvent] | Bukkit covers server-side TNT-block priming with cancellation and the priming entity, but also exposes the prime cause and priming block, which the hook does not pass. The hook additionally covers a dispenser's TNT item and excludes explosion priming, so its scope differs. No key. |
| `trade_select` | **Partial:** [`TradeSelectEvent`][paper-TradeSelectEvent] | Same server-side trade-sidebar selection with the villager, player, and index. The hook only listens, whereas Paper's event is cancellable, so the cancel result cannot be expressed. No key. |
| `trade_with_villager` | **Partial:** [`PlayerTradeEvent`][paper-PlayerTradeEvent] | Paper covers player trading with a villager or wandering trader and exposes a mutable trade. The collect hook also mutates the live offer, but only after the trade completes with uses counted; it cannot express Paper's cancellation. No key. |
| `update_shape` | **Gap:** none | No Paper/Bukkit event covers `BlockBehaviour.updateShape` neighbour recomputation with the old state, direction, neighbour, and replacement state. No key. |
| `use_duration` | **Gap:** none (nearest [`PlayerInteractEvent`][paper-PlayerInteractEvent]) | Paper's interact event is a server-side click event, not the client-and-server item `getUseDuration` query with no user; it has no duration value to read or change. No key. |
| `use_item` | **Partial:** [`PlayerInteractEvent`][paper-PlayerInteractEvent] | Paper exposes item, hand and air/block action before block handling. Horizon preserves its both-side, version-2 `Option<InteractionResult>` answer at `Item.use`, with native `HostSide`; cancellation cannot express its full replacement result or client prediction. No key. |
| `use_on` | **Partial:** [`PlayerInteractEvent`][paper-PlayerInteractEvent] | Paper exposes block, face and item before block handling, not at the item's `useOn` stage. Horizon's version-2 both-side hook exposes native `HostSide` and mutable `result: Option<InteractionResult>`; Paper cancellation cannot express its full replacement answer. No key. |
| `vault_change_state` | **Partial:** [`VaultChangeStateEvent`][paper-VaultChangeStateEvent] | Both concern a server-side vault state change, but the hook notifies after the block state is updated and passes only the new state and position. Paper permits cancellation and exposes the current state and associated player as well as the new state; the listen-only hook cannot express those facts or the cancel result. No key. |
| `vault_loot` | **Partial:** [`BlockDispenseLootEvent`][paper-BlockDispenseLootEvent] | Paper documents vault loot dispensing and exposes the block, loot list, loot table and unlocking player, with loot replacement and cancellation. The hook only hears each item immediately before ejection, passing its item ID and block position; it lacks the full loot batch, table, player and mutable result. No key. |
| `vehicle_create` | **Partial:** [`VehicleCreateEvent`][paper-VehicleCreateEvent] | Paper raises the event when a vehicle is created, with the vehicle type and location. The hook only listens, while Paper's event is cancellable, so the result differs. No key. |
| `vehicle_damage` | **Partial:** [`VehicleDamageEvent`][paper-VehicleDamageEvent] | Both cover server-side vehicle damage with a mutable damage amount and cancellation, but the hook passes only the vehicle kind and amount. Paper also identifies the vehicle, attacker and damage source, which the hook cannot express. No key. |
| `vehicle_destroy` | **Partial:** [`VehicleDestroyEvent`][paper-VehicleDestroyEvent] | Both concern vehicle destruction, but the hook only listens before removal, passing kind and position, and excludes TNT minecarts that prime. Paper exposes the vehicle, attacker and damage source and permits cancellation; these facts and the cancel result are absent from the hook. No key. |
| `vehicle_moved` | **Partial:** [`VehicleMoveEvent`][paper-VehicleMoveEvent] | Both provide server-side movement notifications with from/to positions and no cancellation. The hook passes kind and an optional rider UUID, without vehicle identity, and watches only vehicles carrying a server player or recently left by one; Paper exposes the vehicle and from/to locations without that rider restriction. No key. |
| `weather_change` | **Partial:** [`WeatherChangeEvent`][paper-WeatherChangeEvent] | The event covers a server-side weather change and cancellation, but it exposes only the rain state (`toWeatherState`) and cause, not the hook's requested thundering state. No key. |
| `whitelist_state_update` | **Covered:** [`WhitelistToggleEvent`][paper-WhitelistToggleEvent] | Same dedicated-server whitelist toggle, with `isEnabled` matching the hook's `enabled`; both are listen-only/non-cancellable. Key added. |
| `world_border_bounds_change_finish` | **Partial:** [`WorldBorderBoundsChangeFinishEvent`][paper-WorldBorderBoundsChangeFinishEvent] | Both notify after a timed server-side world-border move finishes, without cancellation. Paper exposes the border, old/new sizes and duration, while the hook passes no arguments, so the information does not match. No key. |

[paper-TargetHitEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/block/TargetHitEvent.html
[paper-ThunderChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/weather/ThunderChangeEvent.html
[paper-TimeSkipEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/TimeSkipEvent.html
[paper-ClockTimeSkipEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/ClockTimeSkipEvent.html
[paper-TradeSelectEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/TradeSelectEvent.html
[paper-PlayerTradeEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerTradeEvent.html
[paper-VaultChangeStateEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/block/VaultChangeStateEvent.html
[paper-VaultDisplayItemEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/VaultDisplayItemEvent.html
[paper-VehicleCreateEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/vehicle/VehicleCreateEvent.html
[paper-VehicleDamageEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/vehicle/VehicleDamageEvent.html
[paper-VehicleDestroyEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/vehicle/VehicleDestroyEvent.html
[paper-VehicleMoveEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/vehicle/VehicleMoveEvent.html
[paper-WeatherChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/weather/WeatherChangeEvent.html
[paper-WhitelistToggleEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/server/WhitelistToggleEvent.html
[paper-WorldBorderBoundsChangeFinishEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/world/border/WorldBorderBoundsChangeFinishEvent.html

[paper-TNTPrimeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/TNTPrimeEvent.html

[paper-BlockDispenseLootEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockDispenseLootEvent.html

## Event counterparts: base world border, lifecycle and save hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), cited from its public Javadoc only; Horizon hooks are cited by function name, never by line.
Scope = the four `base.rs` hooks below: two world-border notifications, the server world lifecycle and the completed-save cycle.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `world_border_bounds_changed` | **Partial:** [`WorldBorderBoundsChangeEvent`][paper-WorldBorderBoundsChangeEvent] | Paper describes instant changes and timed moves on the server, but its Javadoc does not establish the hook's notification after `cmd_worldborder` calls `set_size`/`lerp_to`. The event is cancellable and exposes old/new sizes and mutable size/duration, while the listen-only hook passes no size or duration and cannot cancel or alter the change. No key. |
| `world_border_center_changed` | **Partial:** [`WorldBorderCenterChangeEvent`][paper-WorldBorderCenterChangeEvent] | Paper describes a server world-border center change, but its Javadoc does not establish the hook's notification after `cmd_worldborder` calls `set_center`. The event exposes old/new centers and permits cancellation and `setNewCenter`, whereas the listen-only hook passes only the fact of the change and cannot prevent or retarget it. No key. |
| `world_lifecycle` | **Gap:** none (nearest [`WorldLoadEvent`][paper-WorldLoadEvent] / [`WorldUnloadEvent`][paper-WorldUnloadEvent]) | Paper describes individual world load/unload events, not the hook's server-world open-through-close phase sequence. Neither event exposes `WorldOpenFailed`/`WorldCloseFailed` with the engine's failure text, and the cancellable unload event does not establish a completed-close notification. No key. |
| `world_saved` | **Gap:** none (nearest [`WorldSaveEvent`][paper-WorldSaveEvent]) | Paper documents a world being saved, not completion of the whole save cycle in `save_game_with` after levels, parked levels, metadata and player autosaves. Its public API has no flush/still-pending flag corresponding to `wait`, so it does not establish the hook's moment or information. No key. |

[paper-WorldBorderBoundsChangeEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/world/border/WorldBorderBoundsChangeEvent.html
[paper-WorldBorderCenterChangeEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/world/border/WorldBorderCenterChangeEvent.html

[paper-WorldSaveEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/WorldSaveEvent.html

## Event counterparts: chunk watching, spawners and misc entity hooks (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc only; Horizon cited by function name, never by line number.
Scope: the twenty misc-chunk hooks below, from `chunk_unwatch` to `entity_change_block_pre`.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `chunk_unwatch` | **Partial:** [`PlayerChunkUnloadEvent`][paper-PlayerChunkUnloadEvent] | The event reports a player receiving a chunk unload packet and exposes player and chunk without cancellation. `chunk_unwatch` also reports leaving a level or disconnecting, after the network step; the packet event does not document all those watch removals. No key. |
| `chunk_watch` | **Gap:** none (nearest [`PlayerChunkLoadEvent`][paper-PlayerChunkLoadEvent]) | The event reports a player receiving a chunk, with player and chunk, rather than starting to watch or queuing it. `chunk_watch` precedes `chunk_sent`; receiving the chunk is a different moment. No key. |
| `client_information_updated` | **Partial:** [`PlayerClientOptionsChangeEvent`][paper-PlayerClientOptionsChangeEvent] | Both report server-side client settings and the event exposes locale, view distance, chat, skin parts, main hand, filtering, listing and particles. Its Javadoc promises a settings change, not one callback for every play-phase packet including equal values with explicit old and new snapshots as `client_information_updated` does. No key. |
| `clone_item_stack` | **Partial:** [`PlayerPickBlockEvent`][paper-PlayerPickBlockEvent] | Both concern a server-side block pick, and the event exposes the block and inherited item and include-data flag. Paper also carries player and source/target inventory slots and permits cancellation and slot changes, but returns a copy of the picked item with no replacement setter; `clone_item_stack` replaces the computed stack and carries no player or slots. No key. |
| `connects_to` | **Gap:** none (nearest [`BlockCanBuildEvent`][paper-BlockCanBuildEvent]) | `BlockCanBuildEvent` asks whether a block may be built at a position and offers only a buildable flag; it never asks whether a fence, wall or bars connects toward a neighbour and has no kind, direction or `face_solid`. No key. |
| `contents_changed` | **Gap:** none | No Paper event reports a container's backing block being marked changed after a write; the listed candidates (world change, main hand, equipment) carry none of the level, position and state the hook passes. No key. |
| `create_fluid_source` | **Gap:** none (nearest [`FluidLevelChangeEvent`][paper-FluidLevelChangeEvent]) | The event reports a block's fluid level changing due to adjacent blocks and permits cancelling or changing its new data. `create_fluid_source` instead decides whether each neighbouring source counts during fluid computation, including spread checks with no change; these are different moments. No key. |
| `create_spawn_position` | **Gap:** none (nearest [`SpawnChangeEvent`][paper-SpawnChangeEvent]) | `SpawnChangeEvent` fires after a world's spawn changes and offers only the previous location, so there is no pre-selection veto, no rotation change and no position out; the other candidates are per-player spawns. No key. |
| `crop_grow_post` | **Gap:** none (nearest [`BlockGrowEvent`][paper-BlockGrowEvent]) | `BlockGrowEvent` fires before a block grows (cancelling prevents it) and carries only the new state; no Paper event reports the crop after it grew or the original and current states the hook passes. No key. |
| `crop_grow_pre` | **Partial:** [`BlockGrowEvent`][paper-BlockGrowEvent] | `BlockGrowEvent` matches the about-to-grow moment and its cancel maps to `result = False`, but it fires only once growth is determined, cannot force growth (`result = True`) and reports the new state rather than the growing state. No key. |
| `custom_click_action` | **Partial:** [`PlayerCustomClickEvent`][paper-PlayerCustomClickEvent] | Both report custom clicks with an identifier and optional tag payload on the server. Paper exposes a common connection (including configuration connections) and is non-cancellable; `custom_click_action` carries a play-phase player and a cancel result. No key. |
| `custom_spawner_tick` | **Gap:** none (nearest [`PreSpawnerSpawnEvent`][paper-PreSpawnerSpawnEvent]) | `PreSpawnerSpawnEvent` fires only when a block spawner is about to spawn an entity, at the spawner's location; the hook is a per-tick callback for a mod-registered spawner key that runs the spawner's own logic and may spawn nothing. No key. |
| `modify_custom_spawners` | **Gap:** none | Paper has no event through which a mod adds custom spawner entries to a level's spawner list; the listed spawn events are per spawn and cannot register a spawner. No key. |
| `effect_particle_modification` | **Gap:** none (nearest [`EntityPotionEffectEvent`][paper-EntityPotionEffectEvent]) | The potion events fire when an effect is added, removed or ticks, not when its particles are chosen for a viewer, and neither [`EntityEffectTickEvent`][paper-EntityEffectTickEvent] nor the other candidates exposes the particle options or the visible flag. No key. |
| `enchanted_block_loot` | **Gap:** none (nearest [`BlockDispenseLootEvent`][paper-BlockDispenseLootEvent]) | The event reports a block dispensing loot from its designated loot table, such as a vault, and permits replacing the dispensed loot or cancelling. `enchanted_block_loot` instead modifies each enchantment level read during a block-drop roll with tool and state; there is no event here for that mid-roll query. No key. |
| `enchanted_entity_loot` | **Gap:** none (nearest [`LootGenerateEvent`][paper-LootGenerateEvent]) | `LootGenerateEvent`'s Javadoc says it is not called for an entity's loot table, and no other supplied event exposes the enchantment level read mid-roll with the entity, source and attacker. No key. |
| `enchantment_level_set` | **Gap:** none | The dossier supplies no Paper event for the enchanting table drawing each row's level; no candidate carries the offer (player, row, power, item, drawn level) or lets a mod change it. No key. |
| `ender_entity_teleport` | **Partial:** [`EntityTeleportEvent`][paper-EntityTeleportEvent] | The event explicitly includes natural Enderman and Shulker teleports and exposes entity, from/to locations, destination changes and cancellation. It also covers pathfinding and commands without a documented cause field to isolate self-teleports, and permits changing the origin, which `ender_entity_teleport` cannot express. No key. |
| `enderman_anger` | **Partial:** [`EndermanAttackPlayerEvent`][paper-EndermanAttackPlayerEvent] | Both concern an enderman deciding whether a player's stare counts, with enderman, player and cancellation on the server. Paper starts cancelled for a pumpkin or missed look and allows uncancelling; `enderman_anger` runs only after a valid visible stare without a pumpkin and can only suppress it, also serving freeze and close-range escape checks. No key. |
| `entity_change_block_pre` | **Partial:** [`EntityChangeBlockEvent`][paper-EntityChangeBlockEvent] | The event exposes the entity, block and proposed block data and permits vetoing a block change, matching the pre-change result. It is generic for entity block changes without a documented field limiting it to landing trampling, which is the scope of `entity_change_block_pre`. No key. |

[paper-BlockDispenseLootEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockDispenseLootEvent.html
[paper-BlockGrowEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockGrowEvent.html
[paper-EndermanAttackPlayerEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/EndermanAttackPlayerEvent.html
[paper-PlayerChangedMainHandEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerChangedMainHandEvent.html
[paper-PreSpawnerSpawnEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/PreSpawnerSpawnEvent.html
[paper-SpawnChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/SpawnChangeEvent.html
[paper-PlayerChunkUnloadEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/packet/PlayerChunkUnloadEvent.html
[paper-PlayerClientOptionsChangeEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerClientOptionsChangeEvent.html
[paper-PlayerPickBlockEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerPickBlockEvent.html
[paper-PlayerCustomClickEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/PlayerCustomClickEvent.html
[paper-EntityTeleportEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityTeleportEvent.html

## Event counterparts: entity lifecycle and interaction (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc only; Horizon hooks are cited by function name, never by line.
Scope: the 20 entity hooks below. A key is added only where moment, side, information and result all agree; everything else is Partial or Gap.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `entity_constructing` | **Gap:** none | No dossier candidate covers entity construction while its holder can still be given attachments; nothing on the server-only Paper side exposes the kind and holder at that point. No key. |
| `entity_inside` | **Partial:** [`EntityInsideBlockEvent`][paper-EntityInsideBlockEvent] | Same server-side subject (an entity inside a reacting block), with the block and entity available; but the event is documented for entering the hitbox while the hook fires once per entity and block per move, it is cancellable where the hook is listen-only, and it lacks the `precise` flag. No key. |
| `entity_interact_specific` | **Partial:** [`PlayerInteractAtEntityEvent`][paper-PlayerInteractAtEntityEvent] | Same server-side right-click-on-entity moment, with the entity, player, hand and clicked position available; the held stack is accessible through the player. Paper cancellation prevents the interaction, but does not document the hook's successful claim that swings the hand and ends the click. No key. |
| `entity_mob_griefing` | **Gap:** none | No dossier candidate answers the vanilla mob-griefing permission question for the different actions the hook gates; per-action events cannot replace the shared yes/no answer the hook returns. No key. |
| `entity_place` | **Partial:** [`BlockPlaceEvent`][paper-BlockPlaceEvent] | Matches a server-side player placing a block, with block against, placed block, item and cancellation; its [`BlockMultiPlaceEvent`][paper-BlockMultiPlaceEvent] subclass also describes multi-block player placements. Paper is player-only here, while the hook also covers mob placers (enderman, snow golem), and Paper's Javadoc does not promise the hook's before-write moment. No key. |
| `entity_size` | **Gap:** none | No dossier candidate reports a recalculated size, and no Paper event exposes width, height and eye height as values a mod can edit. No key. |
| `entity_struck_by_lightning` | **Partial:** [`EntityZapEvent`][paper-EntityZapEvent] | Server-side lightning striking an entity, exposing both the struck entity and bolt and allowing cancellation; however, this is an entity-transformation event with a replacement entity, not the hook's general pre-hit permission for every struck entity. No key. |
| `explosion_knockback` | **Partial:** [`EntityKnockbackEvent`][paper-EntityKnockbackEvent] | Server-side knockback about to be applied, with the entity and a settable vector; its documented `Cause.EXPLOSION` selects explosion knockback. It lacks the hook's explosion center and affected block list, and adds cancellation to the hook's vector override. No key. |
| `explosion_start` | **Partial:** [`ExplosionPrimeEvent`][paper-ExplosionPrimeEvent] | Fires server-side when an entity decides to explode and is cancellable, overlapping the hook's pre-blast moment; but it is entity-only, so it does not cover all explosions. Both expose radius and fire, but Paper makes them settable while the hook only permits cancellation, and Paper lacks the hook's block interaction and damage type. No key. |
| `farmland_trample` | **Partial:** [`PlayerInteractEvent`][paper-PlayerInteractEvent] | `Action.PHYSICAL` is the documented trampling path, server-side and cancellable; but the event is generic across physical interactions and player-only, and it lacks the fall `distance`, while the hook fires for mobs and players on landing. No key. |
| `finalize_spawn` | **Gap:** none (nearest [`PreCreatureSpawnEvent`][paper-PreCreatureSpawnEvent]) | This cancellable server event exposes type, location and reason for limited natural and spawner attempts, even ones that may fail later; its Javadoc does not place it at mob finalization. It supplies neither the hook's unspawned mob nor difficulty and group overrides. No key. |
| `fluid_flow` | **Gap:** none (nearest [`FluidLevelChangeEvent`][paper-FluidLevelChangeEvent]) | That event reports a block's fluid level changing and lets you set the new fluid data; it exposes no flow vector for an entity to edit, which is what `FlowingFluid.getFlow` computes. No key. |
| `fluid_place_block` | **Partial:** [`BlockFormEvent`][paper-BlockFormEvent] | Its documented cases include obsidian/cobblestone forming on contact with water, overlapping server-side fluid-caused block placement; cancellation prevents formation and the new block state is available through the inherited API. It also covers unrelated formation and exposes no source `liquid_pos`, so it cannot represent the hook's full information and scope. No key. |
| `game_rule_changed` | **Partial:** [`WorldGameRuleChangeEvent`][paper-WorldGameRuleChangeEvent] | Same subject and side, exposing the rule, the new value and a command sender; but the event is per-world and cancellable with a settable value, so it runs before the change is applied, while the hook is a listen-only server-wide notification that the rule was set. No key. |
| `game_shutting_down` | **Gap:** none | No dossier candidate covers the end of the process; the hook fires after the client or dedicated server has stopped, a lifecycle moment no listed event describes. No key. |
| `get_enchantment_level` | **Gap:** none | No dossier candidate fires on an enchantment-level read; nothing exposes a mutable level map for gameplay effects while leaving the stack's own component untouched. No key. |
| `grindstone_place` | **Partial:** [`PrepareGrindstoneEvent`][paper-PrepareGrindstoneEvent] | Fires server-side when inputs are put in for grinding, exposing the inventory and an inherited `getResult`/`setResult` API, so the output can be replaced or cleared. It is non-cancellable and has no experience override, unlike the hook's `cancel` and `xp`. No key. |
| `grindstone_take` | **Gap:** none (nearest [`PrepareGrindstoneEvent`][paper-PrepareGrindstoneEvent]) | It fires when an item is put in, not when the result is taken; no listed event covers the take or exposes the experience and input-remainder values. No key. |
| `ingredient_test` | **Gap:** none | No dossier candidate runs inside recipe matching for a custom ingredient's predicate; the hook answers per stack test, which no listed event exposes. No key. |
| `is_pathfindable` | **Gap:** none | No dossier candidate answers whether a block state is pathfindable per `PathComputationType`; no listed event exposes that per-state boolean. No key. |

[paper-EntityInsideBlockEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/entity/EntityInsideBlockEvent.html
[paper-BlockPlaceEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockPlaceEvent.html
[paper-RaidSpawnWaveEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/raid/RaidSpawnWaveEvent.html
[paper-WorldGameRuleChangeEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/world/WorldGameRuleChangeEvent.html
[paper-PrepareGrindstoneEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/PrepareGrindstoneEvent.html
[paper-PlayerInteractAtEntityEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerInteractAtEntityEvent.html
[paper-BlockMultiPlaceEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockMultiPlaceEvent.html
[paper-EntityZapEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/EntityZapEvent.html
[paper-PreCreatureSpawnEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/PreCreatureSpawnEvent.html

## Event counterparts: note blocks, spawning, permissions and player events (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc only; Horizon cited by function name.
Scope = the 20 hooks below; each status applies the moment, side, information and result rule.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `note_block_change` | **Gap:** none (nearest [`NotePlayEvent`][paper-NotePlayEvent]) | No Paper event fires while a player tunes a note block: `NotePlayEvent` describes the block playing, after the note changed, and exposes no old note, so the pre-change moment and its `old_note` fact are missing. No key. |
| `note_block_play` | **Partial:** [`NotePlayEvent`][paper-NotePlayEvent] | Fires at the same "about to play" moment server-side and exposes instrument and note with setters and a cancel, matching `instrument`, `note` and `cancel`; it exposes no acting player (or redstone-versus-player source), the fact `player` and `player_entity_id` carry. No key. |
| `potential_spawns` | **Gap:** none (nearest [`SpawnChangeEvent`][paper-SpawnChangeEvent]) | No Paper event offers the candidate spawn list at a server position for mods to add or remove entries, or to empty by cancelling; the name-matched event is about a world's spawn point. No key. |
| `village_siege` | **Gap:** none | No Paper event covers a village siege finding a spawn position, and nothing in the dossier expresses the `canceled` out value for that moment. No key. |
| `neighbor_notify` | **Gap:** none | No Paper event fires before a block tells its six neighbours it changed, and none can suppress that notification; the dossier gives no candidate. No key. |
| `on_item_use_first` | **Partial:** [`PlayerInteractEvent`][paper-PlayerInteractEvent] | With `Action.RIGHT_CLICK_BLOCK`, the server event exposes the player, item, block, hand and interaction point. Its separate block/item use results and cancellation do not express the hook's arbitrary `answer`, and its Javadoc does not establish the phase after `use_item_on_block` and before the block's own use. No key. |
| `permissions_changed` | **Gap:** none | No Paper event fires before the operator list gives a player a new permission level in place of the old one, and the dossier's candidates (world, main-hand and equipment changes) are other moments. No key. |
| `player_break_speed` | **Gap:** none (nearest [`PlayerItemBreakEvent`][paper-PlayerItemBreakEvent]) | No Paper event exposes or lets mods change a player's destroy speed for a block, or cancel it; the nearest candidate is about a tool breaking, a different moment. No key. |
| `player_brewed_potion` | **Gap:** none | No Paper event covers a player taking the bottle from a brewing stand's potion slot; the dossier's witch-potion candidates are unrelated. No key. |
| `player_enchant_item` | **Gap:** none (nearest [`EnchantItemEvent`][paper-EnchantItemEvent]) | The hook reports the enchanted stack and enchantments already given, after levels are paid; Paper documents an item still to be enchanted and mutable enchantments to add, with cancellation. It does not document the hook's completed-enchantment notification moment. No key. |
| `player_flyable_fall` | **Gap:** none | No Paper event reports a creative or spectator player's landing after a fall where `causeFallDamage` returned early; nothing in the dossier matches. No key. |
| `player_harvest_check` | **Gap:** none (nearest [`PlayerHarvestBlockEvent`][paper-PlayerHarvestBlockEvent]) | No Paper event exposes the tool-and-state check that decides whether a block can be harvested; `PlayerHarvestBlockEvent` covers a crop harvest that drops items and changes state, a different moment. No key. |
| `player_item_smithed` | **Gap:** none (nearest [`SmithItemEvent`][paper-SmithItemEvent]) | `SmithItemEvent` describes recipe completion in a smithing table but inherits a cancellable inventory click; its Javadoc does not establish the hook's notification after the take, with consumed inputs preserved as a snapshot. No key. |
| `player_name_format` | **Gap:** none (nearest [`PlayerNameEntityEvent`][paper-PlayerNameEntityEvent]) | No Paper event formats a player's chat or command display name at this moment; the nearest candidate renames a mob. Paper being server-only would permit a match for the server half of this both-sided hook, but no matching event is documented. No key. |
| `player_set_spawn` | **Partial:** [`PlayerSetSpawnEvent`][paper-PlayerSetSpawnEvent] | Fires server-side before a player's respawn point is set, exposes the player, nullable location and forced flag, and supports cancellation. Paper additionally exposes a cause and spawn angle and allows changing the location, forced flag and notification; the hook has no matching facts or outputs for these. No key. |
| `player_spawn_phantoms` | **Gap:** none (nearest [`PhantomPreSpawnEvent`][paper-PhantomPreSpawnEvent]) | No Paper event covers the spawner's attempt decision before its difficulty and rest rolls, and none can set the group size; the nearest candidate fires once a phantom is already spawning. No key. |
| `player_stays_on_ground_surface` | **Gap:** none | No Paper event documents the vanilla movement check in `staying_on_ground_surface` or replacement of its `staying` value. A server event could cover the server half of the both-sided hook, but no event covers this moment. No key. |
| `player_tab_list_name_format` | **Gap:** none (nearest [`TabCompleteEvent`][paper-TabCompleteEvent]) | No Paper event supplies or replaces the name shown in the player/Tab list; the name-matched candidates are command tab completion. No key. |
| `player_xp_change` | **Partial:** [`PlayerExpChangeEvent`][paper-PlayerExpChangeEvent] | Paper exposes the player and mutable amount of experience they will receive naturally, overlapping the hook's pre-grant moment. It is not cancellable, exposes a source entity absent from the hook, and does not document the hook's signed command and advancement grants; setting the amount to zero is not a matching cancel contract. No key. |
| `player_xp_level_change` | **Gap:** none (nearest [`PlayerLevelChangeEvent`][paper-PlayerLevelChangeEvent]) | Paper reports old and new (current) levels, without a setter or cancellation. It does not document the pre-application `add_experience_levels` moment where the hook can replace the signed level delta or prevent the change. No key. |

[paper-NotePlayEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/NotePlayEvent.html
[paper-PlayerHarvestBlockEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerHarvestBlockEvent.html
[paper-PlayerSetSpawnEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/player/PlayerSetSpawnEvent.html
[paper-PhantomPreSpawnEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/PhantomPreSpawnEvent.html

[paper-SmithItemEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/inventory/SmithItemEvent.html

[paper-PlayerExpChangeEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerExpChangeEvent.html
## Event counterparts: miscellaneous portal, brewing, recipe and server hooks (arismoko/horizon#1015)

Paper API 26.3 (`26.3.build.244-stable`), public Javadoc; Horizon cited by function name; scope = the 21 hooks below.
A key is added only where moment, server side, information and result all agree; otherwise the row records the boundary.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `console_line` | **Gap:** none (Paper has no console event; plugins attach a Log4j appender, which is not Javadoc) | Every line the server logs is kept while a mod listens, from any thread, and heard at the start of the next server tick with its `level` (`INFO`, `WARN`, `ERROR`), `thread` and `message`; the lines a listener logs itself are not heard. At most 4096 lines wait between two ticks, each kept to 8192 bytes of message and 128 of thread, and the oldest are dropped. A mod mirroring the console reads this where a plugin would add an appender. No key. |
| `portal_spawn` | **Partial:** [`PortalCreateEvent`][paper-PortalCreateEvent] | `CreateReason.FIRE` identifies portal blocks created when a frame is set on fire; the server event is cancellable and exposes associated block states. Its Javadoc does not identify the triggering fire position/state or establish the exact pre-fill point and fire-survival cancellation behaviour of `portal_spawn`. No key. |
| `potion_brew_post` | **Gap:** none (nearest [`BrewEvent`][paper-BrewEvent]) | `BrewEvent` exposes a pre-finalization inventory and cancellable pending results, not the final post-sound/remainder point or notification after a cancelled pre-brew that `potion_brew_post` provides. No key. |
| `potion_brew_pre` | **Partial:** [`BrewEvent`][paper-BrewEvent] | The server event is cancellable before finalization and exposes the stand and its pre-finalization inventory. It also exposes remaining fuel and mutable pending results, while the hook supplies five slots and retains slot edits only on cancellation; the Javadoc does not establish the same pre-brew cancellation and post notification semantics. No key. |
| `pre_remove_side_effects` | **Gap:** none | No Paper event covers block-entity side effects on removal or collecting extra drops; the potion-effect candidates are unrelated. No key. |
| `recipe_book_categories` | **Gap:** none | Paper's recipe-book events are player clicks, settings or discovery, not registration of categories for custom recipe types; there is no data-load declaration event. No key. |
| `recipe_property_sets` | **Gap:** none | No Paper event adds station-slot property sets during data load; recipe discovery and click events do not expose the set keys or accepted items. No key. |
| `right_click_item` | **Partial:** [`PlayerInteractEvent`][paper-PlayerInteractEvent] | The event is cancellable and carries player, hand, item, block/face and clicked position, matching the server half of `right_click_item`, but it is the generic air/block interaction event rather than a field-limited "immediately before `Item.use`" moment, and it cannot express the hook's interaction result. No key. |
| `server_chat` | **Partial:** [`AsyncChatEvent`][paper-AsyncChatEvent] | Fires for player chat before broadcast, server-side (asynchronously for incoming packets), with player, original/current message, cancellation and message replacement. Its inherited API additionally exposes signed-message information, mutable viewers and a renderer absent from `server_chat`, while the hook supplies a reply collection with no corresponding event output. No key. |
| `chat_recipient` | **Partial:** [`AsyncChatEvent`][paper-AsyncChatEvent] | The event's inherited `viewers()` is the set of audiences that receive the message, and removing one keeps the message from it. The hook keeps a line from one receiver the same way, but it is asked once for each placed receiver, the sender included, with the text players see, and also covers `/say` and `/me`; it has no renderer and no per-viewer text, and whispers are not asked. The others still receive the line signed as it was sent. No key. |
| `server_tick_rate_changed` | **Gap:** none | No dossier event covers a server tick-rate change; the player world, main-hand and equipment candidates are unrelated. No key. |
| `should_check_weak_power` | **Gap:** none | No Paper event asks whether indirect power is read through a block; the block-lock, creeper-power and server-full candidates are unrelated. No key. |
| `should_redstone_wire_connect_to` | **Gap:** none (nearest [`BlockRedstoneEvent`][paper-BlockRedstoneEvent]) | `BlockRedstoneEvent` reports a current change and lets listeners set the new current, not whether wire connects to a neighbouring state; it cannot answer the hook's shape query. No key. |
| `spread_players_teleport` | **Partial:** [`PlayerTeleportEvent`][paper-PlayerTeleportEvent] | For players this cancellable event offers from/to, a mutable destination and a cause, matching the hook's cancel and target move, but it is generic to all player teleports and cannot express non-player entities or limit itself to `/spreadplayers`. No key. |
| `stateful_block_entity` | **Gap:** none | No Paper event declares static block-entity factory ownership per block; this is a creation-time registry decision, not a Bukkit event. No key. |
| `sweep_attack` | **Gap:** none (nearest [`EntityDamageEvent`][paper-EntityDamageEvent]) | Damage events fire later for damage and expose no sweep decision or `sweeping` flag, so they cannot force or stop the sweep as `sweep_attack` does. No key. |
| `tags_updated` | **Gap:** none | No dossier event reports rebuilt tags or their data-load cause; Paper has no candidate for rebinding tag-derived caches. No key. |
| `teleport_command` | **Partial:** [`PlayerTeleportEvent`][paper-PlayerTeleportEvent] | The event matches player teleports with a mutable destination and cancel, and `TeleportCause.COMMAND` identifies a player executing a command. That cause does not distinguish `/tp` from other commands, and the event cannot cover the non-player entities of `teleport_command`. No key. |
| `use_item_on_block` | **Partial:** [`PlayerInteractEvent`][paper-PlayerInteractEvent] | `Action.RIGHT_CLICK_BLOCK` gives the block, face, hand, item and player and is cancellable, but the hook's chosen phase and cancellation result are not exposed, and the generic event also fires outside that moment. No key. |
| `validate_tickets` | **Gap:** none | No Paper event exposes a level's saved chunk tickets for a controller to validate; login and item-transport target validation are different systems. No key. |
| `vanilla_game_event` | **Partial:** [`GenericGameEvent`][paper-GenericGameEvent] | It is the generic cancellable game-event event with event id, location and causing entity, but it exposes no `affected_state` (the block state the event touched), which `vanilla_game_event` passes to mods. No key. |
| `wind_burst` | **Partial:** [`EntityExplodeEvent`][paper-EntityExplodeEvent] | The server event describes explosion block interaction, exposes its location and block list, and omits interaction when mob griefing disables it. Its Javadoc does not establish the wind-burst-specific shuffled candidate moment; it also exposes source entity, explosion result, cancellation and drop yield absent from `wind_burst`. No key. |

[paper-AsyncChatEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/player/AsyncChatEvent.html
[paper-BlockRedstoneEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/BlockRedstoneEvent.html
[paper-GenericGameEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/world/GenericGameEvent.html
## Event counterparts: experience orb targeting (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc; Horizon cited by function name; scope = the hook below.
The comparison is made against the Paper event's documented moment, side, exposed information and result behaviour.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `xp_orb_targeting` | **Gap:** none (nearest [`ExperienceOrbMergeEvent`][paper-ExperienceOrbMergeEvent]) | Paper's nearest event fires when two experience orbs are about to merge, not when an orb chooses which player to follow on each tick. It exposes the two orbs rather than the orb, scan distance and selected player, so it cannot carry this hook's decision. No key. |

[paper-ExperienceOrbMergeEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/ExperienceOrbMergeEvent.html
## Event counterparts: item, level and living hooks (arismoko/horizon#1015)

Paper API 26.3 (`26.3.build.244-stable`), public Javadoc only; Horizon cited by function name, never by position. Scope: the 21 item, level/sound and living hooks below; Paper is server-only, so the client half of a `both` hook has no counterpart by construction.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `damage_item` | **Partial:** [`EntityDamageItemEvent`][paper-EntityDamageItemEvent] | Same server durability-wear operation with the entity, the item, mutable damage and cancellation. The part form asks only the `ItemPart`s of the worn item (`IItemExtension.damageItem`) and passes the stack and the amount after Unbreaking, which Paper's public Javadoc does not establish for the generic event, with the wearing entity, and a part cannot change the wear of an item it is not attached to. No key. |
| `item_after_vertical_collision` | **Gap:** none (nearest [`EntityLandEvent`][paper-EntityLandEvent]) | Paper documents landing after falling, before fall damage and block interactions, rather than every dropped-item vertical collision response reported by `native_item_after_vertical_collision`. Canceling the landing prevents fall damage and block interaction; it does not discard the item or replace its stack as this hook can. No key. |
| `item_attribute_modifier` | **Gap:** none | No Paper event asks for or replaces an `ItemStack`'s attribute modifiers; Bukkit reads the modifiers from the stack directly. The `both` hook's client half (tooltips) has no server event at all. No key. |
| `Registrar::item_container` | **Gap:** none | Paper has no event for registering an item's container capability. Horizon binds an `ItemContainerProvider` to an `ItemKey` at startup; its dynamic `slots(&dyn Host)` callback and component-backed insert/extract helpers have no Bukkit event. Type coverage reads the declaration without invoking that callback. No key. |
| `item_fished` | **Partial:** [`PlayerFishEvent`][paper-PlayerFishEvent] | `CAUGHT_FISH` describes a successful catch being reeled in and exposes the player, bobber and caught item. It has no whole rolled drop list or rod-damage setter, and its Javadoc does not promise the hook's cancel outcome (bobber removed, rod still damaged, no catch rewards). No key. Diverges: Horizon lets a listener change the drop list itself; the reference lets a listener change the stacks but not the list. |
| `item_interact_mob` | **Partial:** [`PlayerInteractAtEntityEvent`][paper-PlayerInteractAtEntityEvent] | Paper exposes entity, hand and click position, but not the hook's later moment after ordinary entity interactions pass. Horizon's version-2 both-side hook includes hand, native `HostSide` and mutable `result: Option<InteractionResult>`; it excludes armor stands and other players. Consuming results end the client hand loop: `Success` swings client-side, `SuccessServer` server-side, `Consume` never. `Fail` ends block interaction only; `Pass`/`TryWithEmptyHand` fall through. Client execution permits reads/effects, never authoritative writes; server-only answers preserve client offhand prediction. Paper cancellation cannot express this contract. No key. |
| `item_smelted` | **Partial:** [`FurnaceExtractEvent`][paper-FurnaceExtractEvent] | The event describes a player taking items from a furnace, smoker or blast furnace and exposes player, stack and amount. Its Javadoc does not establish the per-`FurnaceResultSlot.checkTakeAchievements` calls (including the shift-click remainder with 0 and hotbar swap with 0) that `native_item_smelted` reports before `furnace_extract`; Paper also lets listeners change experience. No key. |
| `item_stacked_on_other` | **Gap:** none | No Paper event covers a pickup click's stack-onto-other step; the hook runs on both sides (client menus and the LAN host) and lets a mod take the click over via its cancellation result. No candidate matches. No key. |
| `item_toss` | **Partial:** [`PlayerDropItemEvent`][paper-PlayerDropItemEvent] | Both concern a server player dropping an inventory item and allow cancellation. Both expose the created item entity and its full stack: `native_item_toss` runs after the entity is made and before it joins the level; Paper's public Javadoc does not establish the hook's cancel outcome of losing the already-removed items or its overflow scope. No key. |
| `level_save` | **Covered:** [`WorldSaveEvent`][paper-WorldSaveEvent] | Same moment (the level has just been saved: level data, chunks and entities), same side (server), and the same facts, with the `World` standing in for the dimension id. It is non-cancellable, matching the listen-only hook. Key added. |
| `play_level_sound_at_entity` | **Gap:** none (nearest [`NotePlayEvent`][paper-NotePlayEvent]) | No Paper event sees an arbitrary entity-bound sound before playback: `NotePlayEvent` is limited to note blocks, with instrument and note fields, so it is not this moment. The hook also covers both sides and mod-supplied sounds. No key. |
| `play_level_sound_at_position` | **Gap:** none (nearest [`NotePlayEvent`][paper-NotePlayEvent]) | The only supplied sound event is the note-block one, which fixes instrument and note instead of exposing sound id, source, volume, pitch and a cancel for arbitrary positional playback. No key. |
| `viewed_level_sound` | **Gap:** none | A client-only hook: Paper runs no client, and has no other level a client keeps. No key. |
| `living_conversion_post` | **Gap:** none (nearest [`EntityTransformEvent`][paper-EntityTransformEvent]) | `EntityTransformEvent` documents an entity about to be replaced and permits cancellation. It does not document the post-conversion moment of `native_living_conversion_post`, after the new mob joined the level and before the old one is removed. No key. |
| `living_conversion_pre` | **Partial:** [`EntityTransformEvent`][paper-EntityTransformEvent] | The server event describes an entity about to be replaced, exposes the original and transformed entities (and hence their types), and is cancellable. It includes transformations outside the hook's scope and exposes already-created transformed entities and a reason that `native_living_conversion_pre` does not pass; its Javadoc does not establish the hook's earlier pre-creation decision or timer-reset outcome. No key. |
| `living_damage_post` | **Gap:** none (nearest [`EntityDamageEvent`][paper-EntityDamageEvent]) | Paper's damage event is fired before the hit is applied and lets mods change or cancel it, so it cannot report the post-hit final values (reductions, post-attack invulnerability ticks) this listen-only hook receives. The after-the-health-loss moment has no supplied event. No key. |
| `living_damage_pre` | **Partial:** [`EntityDamageEvent`][paper-EntityDamageEvent] | Both expose damage before health loss and permit changing damage; `native_living_damage_pre` receives a mutable container after armor and magic reductions, before absorption. Paper is cancellable and does not expose the hook's post-attack invulnerability-tick setter or document that exact reduction-stage moment. No key. |
| `living_death` | **Partial:** [`EntityDeathEvent`][paper-EntityDeathEvent] | Both expose the dying living entity and damage source, and cancellation can keep it alive. Paper already exposes the collected drops and experience and allows a configurable revive health (max health by default), whereas version 2 of `native_living_death` precedes loot and experience processing, passes the full accepted `HurtSource` and optional killing weapon, and preserves listener-written health on cancellation. An unhealed body is removed after 20 ticks without redispatch or loot. No key. |
| `living_destroy_block` | **Partial:** [`EntityChangeBlockEvent`][paper-EntityChangeBlockEvent] | This cancellable server event exposes the changing entity, block and intended new block data, making it closer than `BlockDestroyEvent`, which lacks the entity. Its generic block-change moment explicitly excludes cases with more specific events and does not establish coverage of every per-block destruction path listed by `native_living_destroy_block`. No key. |
| `get_equipment_drop_chance` | **Gap:** none (nearest [`EntityDropItemEvent`][paper-EntityDropItemEvent]) | No supplied event exposes the per-slot equipment drop chance before it is rolled: `EntityDropItemEvent` fires when the item entity is created, after the roll, and carries no slot, chance or preserved flag. No key. |
| `living_drops` | **Partial:** [`EntityDeathEvent`][paper-EntityDeathEvent] | Same moment (the collected death drop list before anything spawns) and the list is editable on both sides, so drops can be removed, changed or added. It differs in information: the event's `ItemStack` list carries no per-drop position, motion or pickup delay and no `recently_hit`, and its cancel revives the entity rather than only suppressing drops, so a port clears `getDrops()` instead. No key. |
| `living_experience_drop` | **Partial:** [`EntityDeathEvent`][paper-EntityDeathEvent] | The event exposes death experience before it spawns and `setDroppedExp(0)` suppresses it. Its killing-blow `DamageSource` does not establish the hook's last attacking player (which may differ from the killing blow), and its cancellation revives the entity rather than suppressing only experience; the Javadoc also does not establish the hook's post-eligibility, non-sculk firing conditions. No key. |

[paper-VehicleBlockCollisionEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/vehicle/VehicleBlockCollisionEvent.html
[paper-PlayerDropItemEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerDropItemEvent.html
[paper-NotePlayEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/block/NotePlayEvent.html
[paper-BlockDestroyEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/block/BlockDestroyEvent.html

[paper-EntityTransformEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityTransformEvent.html

[paper-PlayerInteractAtEntityEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerInteractAtEntityEvent.html
[paper-PlayerMoveEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/player/PlayerMoveEvent.html

[paper-EntityLandEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/entity/EntityLandEvent.html
## Event counterparts: living entities and mob lifecycle (arismoko/horizon#1015)

Paper API 26.3 (paper-api `26.3.build.244-stable`), public Javadoc only; Horizon hooks are cited by function name.
Scope: the 20 hooks below — living motion and projectile values, shield/totem/visibility, menus, minecarts, mob despawn/effects/spawning/splitting and mod configs.

| Horizon hook | Paper event / status | Evidence and porting boundary |
|---|---|---|
| `living_get_projectile` | **Partial:** [`EntityShootBowEvent`][paper-EntityShootBowEvent] | The bow-shot event exposes the living shooter, weapon and consumable ammunition, but runs after ammunition selection and replaces a projectile entity rather than the ammunition stack returned by `living_get_projectile`. For crossbows, [`EntityLoadCrossbowEvent`][paper-EntityLoadCrossbowEvent] fires when a living entity loads one and exposes the crossbow and hand, with cancel and a consume-item flag, but it has no ammunition getter or setter, so it cannot choose the projectile either. No key. |
| `living_flying_speed` | **Gap:** none | No Paper/Bukkit event is fired before a living entity's airborne acceleration (`getFlyingSpeed`) is used; there is no per-tick value hook, and this one is both-side and replayed. No key. |
| `living_jump_power` | **Partial:** [`HorseJumpEvent`][paper-HorseJumpEvent] | The jump moment overlaps, but only for an `AbstractHorse`, and its power is the charged jump multiplier rather than the final living jump power; `setPower` and cancellation are deprecated because horse jumping moved client side. [`EntityJumpEvent`][paper-EntityJumpEvent] and [`PlayerJumpEvent`][paper-PlayerJumpEvent] expose no power value. No key. |
| `living_speed` | **Gap:** none | No Paper event precedes a living entity's ground or fluid movement speed (`getSpeed`); attribute values are readable but there is no pre-use value hook, and this hook is per-tick on both sides. No key. |
| `living_shield_block` | **Gap:** none (nearest [`PlayerShieldDisableEvent`][paper-PlayerShieldDisableEvent]) | The nearest event fires later, when an axe disables a player's shield and a cooldown starts; no Paper event resolves what a raised `blocks_attacks` item stops before it is applied. Two events cover parts of the result: the `BLOCKING` modifier of [`EntityDamageEvent`][paper-EntityDamageEvent] reports how much damage blocking removed (players only), and [`EntityDamageItemEvent`][paper-EntityDamageItemEvent] reports the shield's durability damage with a setter and cancel. Neither decides whether the item blocks, and non-player blockers have no blocked amount. No key. |
| `living_use_totem` | **Partial:** [`EntityResurrectEvent`][paper-EntityResurrectEvent] | The server-side resurrection opportunity exposes the living entity and totem hand, and cancellation can prevent protection, but the event has no damage source. It also fires initially cancelled without a totem, whereas `living_use_totem` requires a held death-protection item. No key. |
| `living_visibility` | **Gap:** none | No Paper/Bukkit event is asked for a mob's visibility percentage while targeting, so neither the entity, the looking entity nor the scaled visibility can be observed. No key. |
| `menu_extra_data` | **Gap:** none | No Paper event hands a block's opening player the extra screen bytes for its mod menu; the menu-open path gives no per-block data channel, and no dossier candidate carries menu, block, position and data together. No key. |
| `minecart_pass` | **Partial:** [`EntityInsideBlockEvent`][paper-EntityInsideBlockEvent] | For detector rails only, this cancellable event fires when an entity is inside the rail's hitbox and exposes the entity and block (so its state and position). It does not fire for other rails or blocks in the minecart's column, and it follows hitbox contact rather than the hook's per-track-step timing. No key. |
| `mob_despawn` | **Gap:** none (nearest [`EntityRemoveEvent`][paper-EntityRemoveEvent]) | `EntityRemoveEvent` with `Cause.DESPAWN` reports a mob that has already despawned, but it is monitoring-only and fires after the fact. [`ItemDespawnEvent`][paper-ItemDespawnEvent] covers only dropped items. No Paper event precedes a mob's `checkDespawn` decision, so the hook's despawn result cannot be set. No key. |
| `mob_effect_applicable` | **Partial:** [`EntityPotionEffectEvent`][paper-EntityPotionEffectEvent] | `Action.ADDED` and `Action.CHANGED` isolate effect additions or changes, and cancellation can deny a modification, but the event does not document an applicability check or forcing an add rejected by vanilla tags. `setOverride` only controls replacement of an existing effect for `Action.CHANGED`, so it cannot express the hook's `Allow`. No key. |
| `mob_effect_expired` | **Covered:** [`EntityPotionEffectEvent`][paper-EntityPotionEffectEvent] | Filter by `Cause.EXPIRATION` to isolate expiry: on the server, the living entity, old effect type and amplifier identify the expiring effect, and cancellation prevents its removal, matching `mob_effect_expired`. Key added. |
| `mob_effect_remove` | **Covered:** [`EntityPotionEffectEvent`][paper-EntityPotionEffectEvent] | Filter by `Action.REMOVED` or `Action.CLEARED`, excluding `Cause.EXPIRATION` and `Cause.DEATH`, to isolate deliberate removal of one effect or each effect in a clear: the server-side living entity, old effect type/amplifier and cancellation keeping it match `mob_effect_remove`. Key added. |
| `mob_spawn_placement_check` | **Gap:** none (nearest [`PreCreatureSpawnEvent`][paper-PreCreatureSpawnEvent]) | The pre-spawn event exposes location, type and reason for limited natural/spawner cases, but its Javadoc does not identify the placement-rule decision or expose its vanilla result. Cancellation can veto a spawn, but cannot force placement acceptance. No key. |
| `mob_spawn_position_check` | **Gap:** none (nearest [`PreCreatureSpawnEvent`][paper-PreCreatureSpawnEvent]) | The pre-spawn event can veto limited natural/spawner attempts even when space or lighting will fail later, but its Javadoc does not place it at `checkSpawnRules`/`checkSpawnObstruction`. It cannot force success or promise to skip those checks and their random draws. No key. |
| `mob_split` | **Covered:** [`SlimeSplitEvent`][paper-SlimeSplitEvent] | Fired when an `AbstractCubeMob` splits on death: the parent (its size via `getSize()`), a settable child count and cancellation stopping the split match the hook's moment, server side, kind/size/count facts and cancel/out count, for slimes, magma cubes and sulfur cubes. Key added. |
| `config_loading` | **Gap:** none | No Paper event is fired when a mod's own config file is read; the hook is a mod-loader notification carrying the config's owner, file and text. No key. |
| `config_reloading` | **Gap:** none | No Paper event reports a mod config file changing on disk or carries its new text; the hook is Horizon's own poll of its config files. No key. |
| `config_unloading` | **Gap:** none | No Paper event accompanies a server-scoped mod config being dropped as a server stops. No key. |
| `register_configs` | **Gap:** none | No Paper event collects mod config declarations before a game or world starts; there is no mod-config registration phase, so the collected configs have no counterpart. No key. |

[paper-HorseJumpEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/HorseJumpEvent.html
[paper-SlimeSplitEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/SlimeSplitEvent.html
[paper-PreCreatureSpawnEvent]: https://jd.papermc.io/paper/26.3/com/destroystokyo/paper/event/entity/PreCreatureSpawnEvent.html
[paper-EntityLoadCrossbowEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/entity/EntityLoadCrossbowEvent.html
[paper-EntityInsideBlockEvent]: https://jd.papermc.io/paper/26.3/io/papermc/paper/event/entity/EntityInsideBlockEvent.html
[paper-EntityRemoveEvent]: https://jd.papermc.io/paper/26.3/org/bukkit/event/entity/EntityRemoveEvent.html

## Event counterparts: client-only hooks (arismoko/horizon#1015)

Paper and Bukkit plugins run on the server; the Paper API has no client events. Each hook below is
declared `side = "client"`, so it has no Paper counterpart and gets no key. A plugin port that needs
one of these moments has to move that logic into a client native mod.

| Hook file | Client-only hooks (no Paper counterpart) |
|---|---|
| `add_attribute_tooltips.rs` | `add_attribute_tooltips` |
| `add_client_reload_listeners.rs` | `add_client_reload_listeners` |
| `base.rs` | `biome_colors`, `block_entity_render_bounding_box`, `block_entity_submit`, `block_highlight`, `camera_distance`, `camera_shake`, `client_level_load`, `client_level_unload`, `client_resources_reloaded`, `command`, `compute_fog_color`, `compute_fov`, `emissive`, `framerate_limit`, `gui_layer_post`, `gui_layer_pre`, `health_hud`, `interaction_key_mapping_triggered`, `item_bar`, `item_decorations`, `map_decorations`, `minimap`, `mouse_button_post`, `mouse_button_pre`, `mouse_scrolling`, `music`, `music_layers`, `panel_event`, `recipes_received`, `register_client_commands`, `render_fog`, `render_needs`, `screen_char_typed_pre`, `screen_click`, `screen_draw`, `screen_key_pressed_pre`, `screen_key_released_pre`, `screen_mouse_button_pressed_pre`, `screen_mouse_button_released_pre`, `screen_mouse_dragged_pre`, `screen_mouse_scrolled_pre`, `screen_render_post`, `screen_tooltip`, `setting_changed`, `shader_params`, `standalone_models`, `tooltip`, `tooltip_component`, `tooltip_component_draw`, `use_animation`, `visuals`, `world_drawn`, `zoom` |
| `block_entity_view_distance.rs` | `block_entity_view_distance` |
| `block_render_visibility.rs` | `block_render_visibility` |
| `block_screen_effect_draw.rs` | `block_screen_effect_draw` |
| `calculate_player_turn.rs` | `calculate_player_turn` |
| `camera_pose.rs` | `camera_pose` |
| `camera_type.rs` | `camera_type_cycle` |
| `chat_suggestion_applied.rs` | `chat_suggestion_applied` |
| `client_chat.rs` | `client_chat` |
| `client_chat_received.rs` | `client_chat_received` |
| `client_mining_predict.rs` | `client_mining_predict` |
| `client_packet.rs` | `client_packet` |
| `client_packet_send.rs` | `client_packet_send` |
| `client_pause_change.rs` | `client_pause_change_post`, `client_pause_change_pre` |
| `client_player_change_game_type.rs` | `client_player_change_game_type` |
| `client_player_clone.rs` | `client_player_clone` |
| `client_player_logging_in.rs` | `client_player_logging_in` |
| `client_player_logging_out.rs` | `client_player_logging_out` |
| `client_services.rs` | `client_disconnecting`, `menu_background` |
| `client_setup.rs` | `client_setup` |
| `client_stopped.rs` | `client_stopped` |
| `client_stopping.rs` | `client_stopping` |
| `client_tick_post.rs` | `client_tick_post` |
| `client_tick_pre.rs` | `client_tick_pre` |
| `compute_fov_modifier.rs` | `compute_fov_modifier` |
| `customize_boss_event_progress.rs` | `customize_boss_event_progress` |
| `customize_chat_overlay.rs` | `customize_chat_overlay` |
| `debug_entry_lines.rs` | `debug_entry_lines` |
| `debug_renderer_draw.rs` | `debug_renderer_draw` |
| `entity_outline.rs` | `entity_outline` |
| `flip_frame.rs` | `flip_frame` |
| `gather_effect_screen_tooltips.rs` | `gather_effect_screen_tooltips` |
| `gather_skipped_attribute_tooltips.rs` | `gather_skipped_attribute_tooltips` |
| `get_arm_pose.rs` | `get_arm_pose` |
| `gui_layer_draw.rs` | `gui_layer_draw` |
| `gui_render_post.rs` | `gui_render_post` |
| `hand_drawn.rs` | `hand_drawn` |
| `input_key.rs` | `input_key` |
| `item_model_property.rs` | `item_model_property` |
| `item_model_range_property.rs` | `item_model_range_property` |
| `item_model_select_property.rs` | `item_model_select_property` |
| `item_tint_source.rs` | `item_tint_source` |
| `item_tooltip.rs` | `item_tooltip` |
| `key_mapping_pre.rs` | `key_mapping_pre` |
| `left_click_empty.rs` | `left_click_empty` |
| `movement_input.rs` | `movement_input` |
| `player_heart_type.rs` | `player_heart_type` |
| `register_block_tint_sources.rs` | `register_block_tint_sources` |
| `register_color_resolvers.rs` | `register_color_resolvers` |
| `register_debug_entries.rs` | `register_debug_entries` |
| `register_debug_renderers.rs` | `register_debug_renderers` |
| `register_gui_layers.rs` | `register_gui_layers` |
| `register_item_tint_sources.rs` | `register_item_tint_sources` |
| `register_layer_definitions.rs` | `register_layer_definitions` |
| `register_range_select_item_model_property.rs` | `register_range_select_item_model_property` |
| `register_runtime_sprites.rs` | `register_runtime_sprites` |
| `register_texture_atlases.rs` | `register_texture_atlases` |
| `render_arm.rs` | `render_arm` |
| `render_frame_post.rs` | `render_frame_post` |
| `render_frame_pre.rs` | `render_frame_pre` |
| `render_gui_post.rs` | `render_gui_post` |
| `render_gui_pre.rs` | `render_gui_pre` |
| `render_hand.rs` | `render_hand` |
| `render_item_in_frame.rs` | `render_item_in_frame` |
| `render_level_stage.rs` | `render_level_stage` |
| `render_living_post.rs` | `render_living_post` |
| `render_living_pre.rs` | `render_living_pre` |
| `render_name_tag.rs` | `render_name_tag` |
| `render_player_post.rs` | `render_player_post` |
| `render_player_pre.rs` | `render_player_pre` |
| `render_tooltip_gather_components.rs` | `render_tooltip_gather_components` |
| `render_tooltip_pre.rs` | `render_tooltip_pre` |
| `render_tooltip_texture.rs` | `render_tooltip_texture` |
| `right_click_empty.rs` | `right_click_empty` |
| `screen_closing.rs` | `screen_closing` |
| `screen_exclusion_areas.rs` | `screen_exclusion_areas` |
| `screen_init_post.rs` | `screen_init_post` |
| `screen_input_post.rs` | `screen_char_typed_post`, `screen_key_pressed_post`, `screen_key_released_post`, `screen_mouse_button_pressed_post`, `screen_mouse_button_released_post`, `screen_mouse_dragged_post`, `screen_mouse_scrolled_post` |
| `screen_opening.rs` | `screen_opening` |
| `screen_preedit_effects.rs` | `screen_preedit_post`, `screen_preedit_pre`, `screen_render_inventory_mob_effects` |
| `screen_render_stages.rs` | `screen_render_background`, `screen_render_foreground`, `screen_render_pre` |
| `screen_transparent_background.rs` | `screen_transparent_background` |
| `screenshot.rs` | `screenshot` |
| `sound_events.rs` | `play_sound_source`, `play_streaming_source`, `sound_engine_load` |
| `submit_custom_geometry.rs` | `submit_custom_geometry` |
| `texture_atlas_stitched.rs` | `texture_atlas_stitched` |
| `tick_target_millis.rs` | `tick_target_millis` |
| `toast_add.rs` | `toast_add` |
| `window_resize.rs` | `window_resize` |
