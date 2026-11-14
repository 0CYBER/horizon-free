# NeoForge extension methods and Horizon

This page compares the public methods of NeoForge 26.3's object-extension interfaces with what a Horizon mod can
answer today. It is the reference the Parts lane starts from: before proposing a new block or item hook, check
whether content data, a handle or an existing Part already answers the same question.

Read from NeoForge `26.3.0.101` (`neoforge-26.3.0.101-universal.jar`, `net.neoforged.neoforge.common.extensions`)
with `javap -p`, and from Horizon at commit `061a06c7b`. Method lists and parameter names are factual citations; no
NeoForge source or Javadoc is copied. Counts exclude each interface's private `self()` accessor.

## How to read the verdicts

| Verdict | Meaning |
| --- | --- |
| **Covered** | A mod can answer the same question for its own object, at the same moment, through content data, a handle or a Part method. |
| **Partial** | The game lets a mod answer something close, but one of these differs: the object (a global hook instead of a per-block/item method), the moment, the arguments, or the answer (a factor or notice where NeoForge returns a decision). |
| **Missing** | Neither data, a handle nor a hook exposes the question. |
| **n/a** | The method has no Horizon equivalent because the architecture removes its reason to exist. |

A **Part method** is a generated method on `BlockPart`, `ItemPart`, `BlockEntityPart`, `MenuPart` or `EntityPart`,
declared with `scope` in `crates/horizon-mod-vocab/src/hooks/<area>.rs`. A Part method runs only for the objects a
mod attached its part to. An **unscoped hook** in the same files is global: every mod hears it and filters by the
arguments itself. **Data** means a content declaration in `crates/horizon-mod-package/src/content.rs`, read into
the game through `crates/horizon-items/src/blocks.rs`. **Handle** means a value or reference type in
`crates/horizon-mod-vocab/src/` (`ItemStack`, `ItemResource`, `AttachmentStore`).

`EntityPart` exists today with goal and behavior registration, `spawn_cluster_size`, and `is_shearable` / `on_sheared` (`docs/modding/src/concepts/parts/entity-parts-and-mob-goals.md`).
It does not answer `IEntityExtension` methods; those are covered only where a global hook or a handle does.

## Counts

`python3 tools/neoforge_extension_counts.py` prints the totals table from the method verdicts below;
there is no snapshot to update.

Of the 60 block methods, 57 exist a second time with the state as receiver, so the state-extension counts are
the block counts minus three; see [`IBlockStateExtension`](#iblockstateextension). The block follow-up is
[#1038](https://github.com/arismoko/horizon/issues/1038) and the item follow-up is
[#1039](https://github.com/arismoko/horizon/issues/1039). Their groups were written from the earlier audit and
do not name every Missing row yet. Both predate hooks that have since landed, so some of their rows are narrower
or already answered; the last section lists what changed.

The earlier audit's counts included `self()` and folded the four `isInFluidType` overloads into one; this page
counts each declared public method, so the entity total is 33 rather than 30.

## `IBlockExtension`

Data fields are static values from a block's content declaration, inherited from its template when unset. They
answer the method for a mod block but cannot vary by level, position or entity. The engine's own copy of the
same facts is `StateFacts` in `crates/horizon-items/src/blocks.rs`.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `getFriction(state, level, pos, entity)` | Covered | `BlockPart::friction` (`hooks/friction.rs`) | Asked where an entity on the block reads its friction, with the entity's id and type and the position, from the block's own value; an answer that is not finite or is outside `0..=2` is dropped. Players, mobs, armor stands, items, experience orbs, boats and leads. The part answers from its inputs, without the level. |
| `hasDynamicLightEmission(state)` | Covered | `BlockProperties.dynamic_light` → `StateFacts.dynamic_light` | Content data, per block, not per state: it sits on the block's row, so every state of a declared block is asked and a block with mixed states pays for all of them. An undeclared block is never asked, where NeoForge's declaration only gates a chunk's first lighting. |
| `getLightEmission(state, level, pos)` | Covered | `BlockPart::light_emission` (`hooks/light_emission.rs`), for a block declaring `dynamic_light` | Asked with the position as a chunk comes in, as an edit sets the block and at `LevelRef::check_light` (`LevelLightEngine.checkBlock`), where vanilla's engine reads emission again, only on the thread that owns the level. A client also asks again for the sections a server's light update changed, at most once per cell per update, a Horizon addition. Not asked for the block's own rendered light, ambient occlusion or the spawn rule, which take the state's light, nor from a light worker. The answer is clamped to 15 at the engine, where NeoForge clamps only its light manager's stored value. Without `dynamic_light`, `BlockProperties.light` is static. |
| `ignitedByLava(state, level, pos, direction)` | Covered | `BlockPart::ignited_by_lava` (`hooks/ignited_by_lava.rs`) | Asked from the face toward the air lava would light, where `LavaFluid.isFlammable` reads it; with no answer the template's `IGNITED_BY_LAVA` state flag (`StateFacts`) decides. No content field sets it. |
| `isLadder(state, level, pos, entity)` | Covered | `BlockPart::is_ladder` (`hooks/is_ladder.rs`) | Asked of the block a living entity stands in, with the entity id, after the `#climbable` tag: the part's answer replaces the tag's, either way. Players and mobs; armor stands never ask. |
| `makesOpenTrapdoorAboveClimbable(state, level, pos, stateAbove)` | Covered | `BlockPart::makes_open_trapdoor_above_climbable` (`hooks/makes_open_trapdoor_above_climbable.rs`) | Asked of the block under an open trapdoor a living entity stands in, with its position and the trapdoor's state, once the trapdoor itself is not a ladder; starts with vanilla's answer, a ladder facing the trapdoor's way. Players and mobs; armor stands never ask. |
| `isBurning(state, level, pos)` | Missing | — | No per-block burning query; `block_burn` is fire removing a block. |
| `canHarvestBlock(state, level, pos, player)` | Covered | `BlockPart::can_harvest_block` (`hooks/can_harvest_block.rs`) | Asked where `player_harvest_check` fires, before it: the global hook still hears and may change the part's answer. |
| `onDestroyedByPlayer(state, level, pos, player, stack, willHarvest, fluid)` | Partial | `BlockPart::on_destroyed_by_player` (`hooks/on_destroyed_by_player.rs`) | Asked in the player's break after `break_block` allowed it, before anything of the break changes the world, with the tool and whether the break harvests. The part answers whether the block was destroyed (`Some(true)`: it removed the block itself, `Some(false)`: it stays) in place of all of `playerWillDestroy` (pot cracking, piglin anger, tripwire disarming, a creative shulker box's or beehive's drop) and the removal, and `None` keeps vanilla's. `willHarvest` is the tool's own answer; the mods' harvest listeners are asked once after the removal, of the adjusted state and the stack in the hand then, as vanilla does. The fluid is not an argument. |
| `onDestroyedByPushReaction(state, level, pos, direction, fluid)` | Partial | `BlockPart::on_destroyed_by_push_reaction` (`hooks/on_destroyed_by_push_reaction.rs`) | A notice in `PistonBaseBlock.moveBlocks` after the block dropped its resources and before the engine sets air, with the direction the blocks move, the cell's fluid state and a level host that writes and schedules ticks. Here the removal always happens and writes to the move's cells are overwritten, and the fluid state of the cell is an argument. NeoForge's default performs the removal itself and an override may replace it: replacement parity remains unverified. |
| `getRespawnPosition(state, entityType, level, pos, angle)` | Covered | `BlockPart::respawn_position` (`hooks/respawn_position.rs`) | Asked on the server as the respawn point is resolved (and as a player leaves the End), starting from the bed's, anchor's or forced point's spot; `None` sends the player to the world spawn. Asked for any block, where NeoForge asks only beds; an anchor loses a charge only when the answer is a spot. The global `player_respawn_position` still hears the result. |
| `getExplosionResistance(state, level, pos, explosion)` | Covered | `BlockPart::explosion_resistance` (`hooks/explosion_resistance.rs`), over `BlockProperties.explosion_resistance` → `StateFacts.explosion_resistance` | Asked with the position and the `ExplosionStart` as a ray enters the block; the answer replaces the static value before the fluid's 100, and the anchor's water cell, a TNT minecart's rails and a dangerous wither skull's cap keep winning. A negative or non-finite answer is refused. |
| `getCloneItemStack(level, pos, state, includeData, player)` | Covered | `BlockPart::clone_item_stack` (`hooks/clone_item_stack.rs`) | `include_data` and the player are passed; leaving the stack unset keeps the game's item. |
| `addLandingEffects(state, level, pos, stateBelow, entity, count)` | Missing | — | `fall_on` edits fall damage, not the landing particle/sound burst. |
| `addRunningEffects(state, level, pos, entity)` | Missing | — | No hook for the running dust burst. |
| `playFallSound(state, level, pos, entity)` | Partial | `BlockPart::sound_type` (`hooks/sound_type.rs`, `cause` `Fall`) | Picks the event, volume and pitch of the one fall sound the game plays; the part can't play or suppress its own. Asked for mobs; a player's block fall sound is not played yet ([#3178](https://github.com/arismoko/horizon/issues/3178)). |
| `playStepSound(state, level, pos, entity, volume, pitch)` | Partial | `BlockPart::sound_type` (`hooks/sound_type.rs`, `cause` `Step`) | Picks the event, volume and pitch of the one step sound the game plays; the part can't play or suppress its own. |
| `canSustainPlant(state, level, pos, direction, plantState)` | Covered | `BlockPart::can_sustain_plant` (`hooks/can_sustain_plant.rs`) | The soil's answer for the plant above it (allow, deny or none), asked before the plant's `may_place_on`, which still answers for your plant. The direction is always up. |
| `onTreeGrow(state, level, posConsumer, random, pos, feature)` | Covered | `BlockPart::on_tree_grow` (`hooks/on_tree_grow.rs`) | Asked of the soil under each trunk column a trunk placer starts, in worldgen and when a sapling grows, with the tree's random: answer a state, none, or leave it to the tree's provider. The first worldgen Part method: no `Host`, a `WorldgenHost`. `feature` is left out: an inline tree feature has no key. |
| `isFertile(state, level, pos)` | Covered | `BlockPart::is_fertile` (`hooks/is_fertile.rs`) | The soil's `fertile` (starting at `moisture > 0`) for each `#grows_crops` block under a crop, stem or pitcher crop in `CropBlock.getGrowthSpeed`; a block outside the tag is never asked. |
| `getExpDrop(state, level, pos, blockEntity, breaker, tool)` | Partial | `break_block.exp_to_drop` (`hooks/base.rs`) | Replaces the XP sample for a player break; global, not per-block, and the block entity/tool/breaker are not arguments. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `rotate(state, level, pos, rotation)` | Covered | `BlockPart::rotate` (`hooks/rotate.rs`), a worldgen Part method | Asked as a structure template places each block, through `StateTransforms::rotate_at` (template placement in worldgen, `/place template` and the structure block, the legacy pieces' `placeBlock` and template filters, jigsaw blocks, and the host's template snapshots), with the already mirrored state, the position and the rotation. `rotated` starts at the table's answer (vanilla's, or the template's for a mod block); a state of another block or an unknown state is refused. The worldgen `context` has no `Host`; `state_with_property` is on it. Mirroring stays the template's. |
| `getEnchantPowerBonus(state, level, pos)` | Covered | `BlockPart::enchant_power_bonus` (`hooks/enchant_power_bonus.rs`, contract v2, both sides) | Offers query each bookshelf offset with a clear transmitter; the sum is truncated and a negative one counts as 0. Client enchant particles use the same power source: any nonzero answer is a provider, subject to the same transmitter check. Particle answers are cached once per state at world entry, parts reload and received-server-tag publication, after tags, rebound parts and tag-update listeners are published. This also rebuilds vanilla provider/transmitter answers without mods. Both hook positions are at the origin, so particle answers must be state-only; a mod answer change needs that rebuild. No dispatch occurs per frame. The provider tag supplies the vanilla default only. |
| `onNeighborChange(state, level, pos, fromPos)` | Missing | — | `neighbor_changed` is `Block.neighborChanged`, a different callback. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `shouldCheckWeakPower(state, level, pos, direction)` | Covered | `BlockPart::should_check_weak_power` (`hooks/should_check_weak_power.rs`) | The argument direction is the input side, as NeoForge's. |
| `getWeakChanges(state, level, pos)` | Missing | — | No weak-change observer query. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `getSoundType(state, level, pos, entity)` | Covered | `BlockPart::sound_type` (`hooks/sound_type.rs`) | Asked with the position, the entity (nullable: the stepping, landing, placing or digging one, none for a break or a dispenser's placement) and the sound about to play, on NeoForge's paths only: the step and combination step, a horse's step (at the block below a snow layer), a mob's fall, a block item's placement, the break event and a sniffer's digging. Not for the mining hit sound (its own client hook, `BlockPart::play_hit_sound`, below) or sculk growth; an answer with a volume or pitch that is not finite and at least 0, or an event that is not a `namespace:path` id, is dropped. The part answers from its inputs, without the level; `host.block_states().sound(state)` still reads the static type. |
| `getBeaconColorMultiplier(state, level, pos, beaconPos)` | Covered | `BlockPart::beacon_color_multiplier` (`hooks/beacon_color_multiplier.rs`) | Asked on both sides as a beacon builds its beam, with the beacon's position, starting from the stained glass or beacon colour; the beam keeps the answer's RGB and ignores alpha. |
| `getStateAtViewpoint(state, level, pos, viewpoint)` | Covered | `BlockPart::state_at_viewpoint` (`hooks/state_at_viewpoint.rs`) | Asked on the client for each block `Camera.getFluidInCamera` reads (the eye, the block above it, the near plane's centre and corners); the fluid and powder snow tests read the answer, so the fog, the fluid field of view and water vision follow it. |
| `getBlockPathType(state, level, pos, mob)` | Covered | `BlockPart::block_path_type` (`hooks/block_path_type.rs`) | Positional readonly-level query before all vanilla rules, including air. `Some` replaces classification; `None` declines to vanilla. No mob, matching NeoForge's null engine callers. Final static classification uses one server-level 4096-slot positional cache, shared by workers; matching sendBlockUpdated notifications or collisions invalidate it, not reload, time or chunk lifetime. Ordinary writes require real change, UPDATE_CLIENTS and full status at least BLOCK_TICKING; direct block-entity notifications clear unchanged state too. |
| `getAdjacentBlockPathType(state, level, pos, mob, pathType)` | Covered | `BlockPart::adjacent_block_path_type` (`hooks/adjacent_block_path_type.rs`) | Positional readonly-level query with the neighbour's own classified type; no mob, matching null engine callers. First `Some` ends the x/y/z scan, including an answer equal to the node's type; `None` keeps fluid then vanilla contributions. The node's own column is excluded. |
| `isSlimeBlock(state)` | Covered | `BlockPart::is_slime_block` (`hooks/block_sticky.rs`) | Server only, asked by the piston structure resolver for its slime-beside-honey rule. A piston moving a slime block still bounces the entities it pushes off vanilla's slime block alone. |
| `isStickyBlock(state)` | Covered | `BlockPart::is_sticky_block` (`hooks/block_sticky.rs`) | Server only, asked by the piston structure resolver; starts with vanilla's slime and honey. |
| `canStickTo(state, otherState)` | Covered | `BlockPart::can_stick_to` (`hooks/block_sticky.rs`) | The part of `state` answers; the resolver asks both sides and needs both to say yes. |
| `isFlammable(state, level, pos, direction)` | Covered | `BlockPart::fire_odds` (`hooks/fire_odds.rs`) | Flammable is ignite odds above zero, as `FireBlock.canBurn` reads it, so the odds answer it for each position and face; no separate predicate. |
| `getFireSpreadSpeed(state, level, pos, direction)` | Covered | `BlockPart::fire_odds` (`hooks/fire_odds.rs`) | Ignite/spread odds (the first element of `fire_odds`'s `(ignite, burn)` tuple) replace vanilla's table, with the block's position and the face toward the fire. |
| `getFlammability(state, level, pos, direction)` | Covered | `BlockPart::fire_odds` (`hooks/fire_odds.rs`) | Burn/destruction odds (the second element of `fire_odds`'s `(ignite, burn)` tuple) replace vanilla's table, with the block's position and the face toward the fire. |
| `onCaughtFire(state, level, pos, direction, igniter, stack)` | Partial | `BlockPart::on_caught_fire` (`hooks/on_caught_fire.rs`) | A notice after the burn roll and spread/burn/ignite cancel gates, before replacing the block, with its old state and face toward the fire. Writes at that position are overwritten; TNT primes afterward. Fire has no igniter and an empty stack. Lava, lightning, spread into air, flint and steel, TNT use, burning projectiles, redstone and dispensers do not fire this spread-only hook. |
| `isFireSource(state, level, pos, direction)` | Covered | `BlockPart::is_fire_source` (`hooks/is_fire_source.rs`) | Asked of the block under a fire on each fire tick, before the dimension's `infiniburn` tag: yes makes the fire endless, no refuses a tagged block. The direction is always up. |
| `canEntityDestroy(state, level, pos, entity)` | Partial | `BlockPart::can_entity_destroy` (`hooks/can_entity_destroy.rs`) | Asked where vanilla asks its per-block rule: the wither's block break, a dangerous wither skull's resistance cap (the entity is the skull, readable during the blast), the dragon's `checkWalls` (where `#dragon_immune` still holds after a yes), a zombie's door breaking and the turtle-egg `RemoveBlockGoal`'s target choice; after the griefing gate and before `living_destroy_block` (none at the egg goal's target choice). Ravager: griefing event only. The zombie door still asks `living_destroy_block` at the break, not at goal start with the other two (#3883). |
| `canDropFromExplosion(state, level, pos, explosion)` | Covered | `BlockPart::can_drop_from_explosion` (`hooks/can_drop_from_explosion.rs`) | Asked where `onExplosionHit` asks `Block.dropFromExplosion`; the answer gates the loot and `spawnAfterBreak`, and a TNT still primes. |
| `onBlockExploded(state, level, pos, explosion)` | Covered | `BlockPart::on_block_exploded` (`hooks/on_block_exploded.rs`) | A server notice per destroyed block after its loot and before the engine sets air, with a writable host, the live block entity and the `ExplosionStart`; the engine sets air even if the part wrote a block. |
| `collisionExtendsVertically(state, level, pos, entity)` | Missing | — | Collision shapes are data; no per-entity extension query. |
| `shouldDisplayFluidOverlay(state, level, pos, fluid)` | Missing | — | Overlay drawing is engine-owned. |
| `getToolModifiedState(state, context, ability, simulate)` | Covered | `BlockPart::tool_modified_state` (`hooks/tool_modified_state.rs`) | Asked of the clicked block before the global `block_tool_modification`, with the vanilla result: the part allows, replaces or, cancelled, denies the action. `simulate` is `ToolUse::simulated`. Axe, shovel and hoe actions and a shovel dousing a campfire. |
| `isScaffolding(state, level, pos, entity)` | Missing | — | Scaffolding is identified by block class; no content field or Part. |
| `hidesNeighborFace(level, pos, state, neighborState, direction)` | Missing | — | `block_render_visibility` hides a whole state from the mesher, not one face. |
| `supportsExternalFaceHiding(state)` | Missing | — | No per-face culling query. |
| `canBeHydrated(state, level, pos, fluid)` | Covered | `BlockPart::can_be_hydrated` (`hooks/can_be_hydrated.rs`) | The farmland's answer for each non-empty fluid in its 9×2×9 scan (with the fluid's position), `None` for the `#water` rule. The fluid's own side (`IFluidExtension.canHydrate`) is a fluid row. |
| `onBlockStateChange(level, pos, oldState, newState)` | Covered | `BlockPart::on_block_state_change` (`hooks/on_block_state_change.rs`, version 2) | Heard on both sides with a read-only `LevelSide` where `Level.setBlock` runs `updatePOIOnBlockStateChange`: after the client update, the neighbour updates and the neighbours' shape updates, only when the write held. The parts of the old state's block hear it, then those of the new state's block when the block changed. Applied local client predictions (including breaks and placements) carry `predicted: true`; server updates, settled corrections, piston replays and every server-side write carry `false`. Held server updates remain unheard until acknowledgement; a confirmation has no duplicate, a denial hears predicted-to-server once. Equal-state writes, unapplied guesses and world generation's writes are not heard. The global `block_changed` stays a separate, earlier server moment. |
| `getMapColor(state, level, pos, defaultColor)` | Partial | `BlockProperties.map_color` → `StateFacts.map_color` | Static; no position. |
| `getAppearance(state, level, pos, direction, neighborState, neighborPos)` | Missing | — | No appearance query. |
| `getPistonPushReaction(state)` | Covered | `BlockPart::piston_push_reaction`, `BlockProperties.push_reaction` | State-only override cached on both sides at load and reload; `None` preserves properties. |
| `isEmpty(state)` | Missing | — | Air/replaceable facts exist (`block_states().replaceable`), but no `isEmpty` answer for a state. |
| `getBubbleColumnDirection(state)` | Missing | — | Bubble-column rules are engine-owned. |
| `shouldHideAdjacentFluidFace(state, direction, fluid)` | Missing | — | Fluid-face culling is engine-owned. |
| `getBounceRestitution(level, pos, state, entity)` | Covered | `BlockPart::bounce_restitution` (`hooks/bounce_restitution.rs`) | Asked for the player and the mobs on both sides, with the entity's level as a read-only `LevelSide` host, the entity, its type and the block's position. The answer must be pure because replayed ticks ask again: the same host and event inputs must give the same factor. Dropped items keep their own slime rule. `BlockPart::bounce_on` still edits the movement after the bounce. |
| `getRelocability(level, pos, state)` | Missing | — | No relocation query (pistons, structure blocks). |
| `canBeConnectedTo(state, direction, neighborBlock)` | Covered | `BlockPart::can_be_connected_to` (`hooks/can_be_connected_to.rs`) | The block being connected to answers, with its position, the connector's state, its rule (`ConnectionKind`), the direction toward it and its face's sturdiness, after the connector's vanilla rule (fences, walls, bars and panes ask). The connector's own `BlockPart::connects_to` (`hooks/connects_to.rs`) is asked after and has the last word; that order is Horizon's. |
| `getModelData(state, level, pos)` | Missing | — | No `ModelData`; block-entity drawing takes a `block_entity_submit` part with its own model path. |

## `IBlockStateExtension`

This interface repeats the block questions with the state as receiver instead of the first argument. All 57
methods are the same behaviours and take the same verdict as the `IBlockExtension` row of the same name, with
the parameters shifted by one. Three block methods have no state-receiver form:

| Block-only method | Verdict there |
| --- | --- |
| `getPistonPushReaction(state)` | Covered (`BlockPart::piston_push_reaction`, content `push_reaction`) |
| `getMapColor(state, level, pos, defaultColor)` | Partial (content `map_color`) |
| `makesOpenTrapdoorAboveClimbable(state, level, pos, stateAbove)` | Covered |

The handful of signature differences (`canStickTo` takes
the neighbour state; `getCloneItemStack` reorders its arguments; `getEnchantPowerBonus` takes a `LevelReader`)
do not change any verdict.

## `IItemExtension`

Item data is a 26.3 component patch over a vanilla template (`crates/horizon-mod-package/src/content.rs`),
which is the same place NeoForge's item components live. Reads are on `host` (`host/items.rs`) or on the owned
`ItemStack` / `ItemResource` values.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `MutableDataComponentHolder`: `set`, `remove`, `update`, `applyComponents`, `copyFrom` | Covered | `ItemStack::set`, `remove`, `get`, `apply_components`, `copy_component_from`; `ItemPlace::edit` | `update` is `get` then `set`. Patch merges retain unrelated overrides and normalize damage atomically; copying preserves the source's own value/removal or clears the target override when the source inherits. Server-side `edit` writes the whole slot without changing its item/count or splitting one off, and a transaction abort restores it. [#3339](https://github.com/arismoko/horizon/issues/3339). |
| `getDefaultAttributeModifiers(stack)` | Partial | `item_attribute_modifier` (`hooks/item_attribute_modifier.rs`) | Global `ItemAttributeModifierEvent` edits the stack's modifiers; the stack filters it, but it is not a Part method. |
| `onDroppedByPlayer(stack, player)` | Covered | `ItemPart::on_dropped_by_player` (`hooks/on_dropped_by_player.rs`) | Runs on the server where `ServerPlayer.drop(boolean)` starts, for the drop key, with the whole selected stack and whether all of it goes; a cancel keeps the stack in the slot (no item entity, no statistic, no `item_toss`) and sends a server's player the slot back. A menu's throw is not asked, as in NeoForge; the global `item_toss` still hears every toss the part allows. |
| `getHighlightTip(stack, name)` | Covered | `ItemPart::highlight_tip` (`hooks/highlight_tip.rs`) | The selected stack's name above the hotbar, starting from vanilla's styled hover name; asked once each client tick while the name shows. A tip that changes every tick doesn't restart the display timer. |
| `onItemUseFirst(stack, context)` | Covered | `ItemPart::on_item_use_first` (`hooks/on_item_use_first.rs`) | Runs after `use_item_on_block` let it through, before the block's own use. |
| `isPiglinCurrency(stack)` | Covered | `ItemPart::is_piglin_currency` (`hooks/is_piglin_currency.rs`) | Starts from vanilla's answer, which is the item `minecraft:gold_ingot` (`PiglinAi.BARTERING_ITEM`; 26.3 has no currency tag), and is asked where vanilla asks: a piglin wanting a stack on the ground, an adult taking one from the hand (and the client's prediction of that click), and the end of the admiring, where currency is bartered or lost to a hit. A ground stack is admired only when it is also `#piglin_loved`. Both sides. [#3226](https://github.com/arismoko/horizon/issues/3226). |
| `makesPiglinsNeutral(stack, entity)` | Covered | `ItemPart::makes_piglins_neutral` (`hooks/makes_piglins_neutral.rs`) | Starts from `#minecraft:piglin_safe_armor` (which content mods can already add to) for each stack in a player's armour slots, with the wearer, and may make a piece count or refuse a tagged one. Server-side. [#3226](https://github.com/arismoko/horizon/issues/3226). |
| `isCombineRepairable(stack)` | Partial | `minecraft:repairable` component via `ItemResource::repairable` | Data answers which material repairs it; the anvil combine predicate is not exposed. |
| `getXpRepairRatio(stack)` | Covered | `ItemPart::xp_repair_ratio` (`hooks/xp_repair_ratio.rs`) | Asked in Mending's repair (`ExperienceOrb.repairPlayerItems`) for the picked item, with the ratio arriving as its `minecraft:repair_with_xp` repair per point. Server-side, read-only `Host`. |
| `onStopUsing(stack, entity, timeLeft)` | Partial | `ItemPart::release_using` (`hooks/base.rs`) | Runs on an early release only, after the vanilla item's release; finishing the use or switching to another item stops it without asking. Both sides. |
| `onLeftClickEntity(stack, player, entity)` | Covered | `ItemPart::on_left_click_entity` (`hooks/item_attack.rs`) | Runs where `Player.attack` starts, after `attack_entity` let the blow through; a cancel leaves the target unhurt, with no `hurt_enemy` and no wear. |
| `getCraftingRemainder(instance)` | Covered | `ItemPart::crafting_remainder`, content `crafting_remainder` | The per-stack Part override replaces the data remainder in crafting, fuel and quick moves. |
| `getEntityLifespan(stack, level)` | Partial | `item_expire` (`hooks/base.rs`) | Adds extra ticks to a dropped item's life; no absolute lifespan and not per-item. |
| `hasCustomEntity(stack)` | Missing | — | No custom dropped-entity selection. |
| `createEntity(level, entity, stack)` | Missing | — | Same. |
| `onEntityItemUpdate(stack, itemEntity)` | Missing | — | No per-item dropped-entity tick. [#1039](https://github.com/arismoko/horizon/issues/1039). |
| `doesSneakBypassUse(stack, level, pos, player)` | Missing | — | No sneak-bypass query. |
| `canEquip(stack, slot, entity)` | Covered | `ItemPart::can_equip` (`hooks/can_equip.rs`) | Starts as the stack's own `equippable` component's answer (slot and `allowed_entities`); the part sets it either way, with a read-only `Host` and the entity as a `LivingRef`. Asked for the player's armour slots (both menus), a dispenser's equip and a mob's pickup. The wasm generator supports a retained stack per guest instance and token-based borrowed asks; missing prepared exports refuse at load/registration. One dispense, menu input scope, or one mob’s pickup attempt shares its preparation across candidates. Scoped source revisions and identity generations invalidate stale asks and apply preconditions; no-listener paths convert nothing and add no entity lookup. Target type, position, living health/status and physical equipment reads reuse prepared values; other Host reads can allocate. |
| `getEquipmentSlot(stack)` | Covered | `ItemResource::equippable_slot`, `host.item_equippable_slot` | Read from the effective `equippable` component. |
| `onEntitySwing(stack, entity, hand)` | Covered | `ItemPart::on_entity_swing` (`hooks/item_attack.rs`) | Runs where `LivingEntity.swing` starts, on both sides; a cancel starts no swing (no animation, no packet). |
| `getDamage(stack)` | Covered | `ItemStack::damage` | The stack's numeric damage override. |
| `getMaxDamage(stack)` | Partial | `minecraft:max_damage` effective component, `host.max_damage` | The stack's effective component includes a patch over the item default; `host.max_damage` reads that default only. |
| `isDamaged(stack)` | Covered | `ItemStack::damage`, `damage_component` | Distinguishes explicit zero from absent. |
| `setDamage(stack, damage)` | Partial | `ItemStack::set_damage` | Stores the requested value; the game checks it against the effective maximum when it accepts the write instead of clamping here. |
| `canPerformAction(instance, ability)` | Partial | `ItemPart::can_perform_action` (`hooks/can_perform_action.rs`), `block_tool_modification` (`hooks/block_tool_modification.rs`) | The part answers per stack and per plain, case-sensitive ability name, starting from vanilla's answer, where vanilla checks the item kind: `shield_block` (a raised item blocking attacks, its use time and animation), `shears_harvest` (sheep, mooshroom, snow golem, bogged, bee nests, a dispenser's shears) and `shears_carve` (a pumpkin). The block side of a tool action (strip, wax, path, ...) is `block_tool_modification`; no tool ability (`axe_strip`...) is asked of the item yet. [#1039](https://github.com/arismoko/horizon/issues/1039). |
| `getMaxStackSize(stack)` | Partial | `max_stack_size` component, `host.max_stack_size` | The stack's effective component includes a patch over the item default; `host.max_stack_size` reads that default only, and invalid values are refused at the write. |
| `isPrimaryItemFor(stack, enchantment)` | Partial | `host.enchantment_supports` | Answers supported items; the narrower "primary item" distinction is not exposed. |
| `supportsEnchantment(stack, enchantment)` | Partial | `host.enchantment_supports` | A client-side read of `Enchantment.isSupportedItem` for an item id; not per stack and without NeoForge's enchanted-book special case. |
| `getEnchantmentLevel(instance, enchantment)` | Covered | `get_enchantment_level` (`hooks/get_enchantment_level.rs`) | Edits the level gameplay sees. |
| `getAllEnchantments(stack, lookup)` | Covered | `get_enchantment_level` with no target | The hook's `target` is `None` for every-enchantment reads. |
| `shouldCauseReequipAnimation(stack, oldStack, equipped)` | Covered | `ItemPart::should_cause_reequip_animation` (`hooks/should_cause_reequip_animation.rs`) | Asked in `FirstPersonHandsAndItems.tick` for each hand whose shown stack differs from the held one by slot or by value, starting from `!shouldInstantlyReplaceVisibleItem`; `equipped` is read as `slot_changed` (the held stack came from another slot). An empty hand isn't asked. |
| `shouldCauseBlockBreakReset(oldStack, newStack)` | Covered | `ItemPart::should_cause_block_break_reset` (`hooks/should_cause_block_break_reset.rs`) | `MultiPlayerGameMode.sameDestroyTarget`'s item half, single player and a server's client alike: the break keeps its stack and the part of that stack's item may overturn `!isSameItemSameComponents`. |
| `canContinueUsing(stack, otherStack)` | Missing | — | No continue-use query. |
| `getCreatorModId(registries, stack)` | Covered | `ItemPart::creator_mod_id` (`hooks/creator_mod_id.rs`), read by `ItemResource::creator_mod_id` | Both sides, from the stack alone. The answer arrives as the item id's namespace; a part sets another mod or none. An item without a part costs one table lookup. |
| `onAnimalArmorTick(stack, level, mob)` | Missing | — | No per-item animal-armour tick. |
| `damageItem(stack, amount, entity, onBreak)` | Covered | `ItemPart::damage_item` (`hooks/damage_item.rs`) | Runs on the server for the parts of the worn item, after Unbreaking, on every wear path (a player's tools, armour and shield, a dispenser's tool, a mob's armour). The part gets the stack, the wearing or using entity (`None` for a dispenser's tool) and the amount about to be lost, and changes it (0 or less leaves the item whole), cancels the wear, or sets `break_now` to break the item whatever durability is left, the Horizon form of calling `onBroken`. |
| `onDestroyed(itemEntity, source)` | Missing | — | `item_despawn` is lifetime removal, not damage destruction. |
| `isGazeDisguise(stack, player, target)` | Missing | — | No gaze-disguise query. |
| `canWalkOnPowderedSnow(stack, entity)` | Missing | — | Powder-snow walking is engine-owned. |
| `isDamageable(stack)` | Covered | `ItemResource::damageable(host)` | Reads the effective components: `minecraft:max_damage` and `minecraft:damage` present, `minecraft:unbreakable` absent. |
| `getSweepHitBox(stack, player, target)` | Missing | — | `sweep_attack` decides whether a sweep happens, not its box. |
| `isNotReplaceableByPickAction(stack, player, slot)` | Missing | — | Pick-block replacement is engine-owned. |
| `canGrindstoneRepair(stack)` | Missing | — | `grindstone_place` observes or edits the result; no per-item predicate. |
| `canBeHurtBy(stack, source)` | Missing | — | No item damage-immunity query. |
| `applyEnchantments(stack, enchantments)` | Missing | — | No item-level enchantment application hook. |
| `canFitInsideContainerItems(stack)` | Partial | `MenuPart::menu_slot_rules` (`hooks/base.rs`) | Slot rules implement container-item restrictions (a shulker box in a slot); no per-stack override. |
| `onGlideDamage(stack, entity, slot)` | Missing | — | No per-item glide-damage callback. |

## `IEntityExtension`

`EntityPart` carries no `IEntityExtension` method (its methods are goals, spawn group size and shearing), so
entity coverage is data, handles and global hooks. On the server a mod can act on entities through `EntityRef` and `AttachmentStore`.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `captureDrops()` | Missing | — | `living_drops` edits a dying mob's drops; it does not capture an arbitrary entity's drops. |
| `captureDrops(collection)` | Missing | — | Same. |
| `getPersistentData()` | Covered | `AttachmentStore`, `AttachmentHolder::Entity` | Saved and synced attachments replace the writable compound; the data is typed, not raw NBT. |
| `shouldRiderSit()` | Missing | — | Rider pose is engine-owned. |
| `canRiderInteract()` | Missing | — | No rider-interaction query. |
| `canBeRiddenUnderFluidType(type, rider)` | Missing | — | `entity_mount` fires on mount/dismount, not on fluid riding. |
| `canTrample(level, state, pos, distance)` | Partial | `farmland_trample` (`hooks/farmland_trample.rs`) | Global `BlockEvent.FarmlandTrampleEvent`; per-entity override is not exposed. |
| `getClassification(includeVehicles)` | Partial | `host.entity_classification`, content entity type | A read of the template's category; a mod entity cannot set a different one per entity. |
| `isAddedToLevel()` | Partial | `entity_join_level` / `entity_leave_level`, `host.entity_added_to_level` | State is tracked by the game; the handles tell a mod after the fact. |
| `onAddedToLevel()` | Partial | `entity_join_level` (`hooks/base.rs`) | Fires after the entity is in the level's lists; cancellable, unlike the method. |
| `onRemovedFromLevel()` | Partial | `entity_leave_level` (`hooks/base.rs`) | Fires after removal and only listens. |
| `revive()` | Missing | — | No entity revive API. |
| `isMultipartEntity()` | Missing | — | The dragon's parts are internal to the engine. |
| `getParts()` | Missing | — | Same; `EntityRef` names dragon parts in some reads but does not expose the array. |
| `getFluidTypeHeight(type)` | Partial | `host.entity_fluid_heights` | Each fluid the entity touched with its height, from its last fluid update. Server only; the client half is still to come (#3341). |
| `getMaxHeightFluidType()` | Partial | `host.entity_fluid_heights` | The first entry: the list is highest first. Server only. |
| `isInFluidType(state)` | Partial | `host.entity_fluid_heights` | Whether the state's fluid is in the list. Entities track water and lava; mod fluids join once the entity update tracks them. Server only. |
| `isInFluidType(type)` | Partial | `host.entity_fluid_heights` | Same. |
| `isInFluidType(predicate)` | Partial | `host.entity_fluid_heights` | The mod applies its predicate to the list. Server only. |
| `isInFluidType()` | Partial | `host.entity_fluid_heights` | A non-empty list. Server only. |
| `getFirstEyeInFluidType()` | Partial | `host.entity_eye_fluid` | Water or lava at the eyes from the last fluid update; mobs now track lava there too. Server only; the client half is still to come (#3341). |
| `isEyeInFluidType(type)` | Partial | `host.entity_eye_fluid` | Compare its answer. Server only. |
| `canStartSwimming()` | Missing | — | Swimming start is engine-owned. |
| `getFluidMotionScale(type)` | Missing | — | No per-entity fluid scale. |
| `isPushedByFluid(type)` | Partial | `FluidTypeProperties.can_push_entity` | Per-fluid data, not a per-entity answer. |
| `canSwimInFluidType(type)` | Partial | `FluidTypeProperties.can_swim` | Per-fluid data. |
| `canFluidExtinguish(type)` | Partial | `FluidTypeProperties.can_extinguish` | Per-fluid data. |
| `getFluidFallDistanceModifier(type)` | Partial | `FluidTypeProperties.fall_distance_modifier` | Per-fluid data. |
| `canHydrateInFluidType(type)` | Partial | `FluidTypeProperties.can_hydrate` | Per-fluid data. |
| `getSoundFromFluidType(type, action)` | Missing | — | No fluid sound-action table. |
| `sendPairingData(player, consumer)` | Missing | — | No pairing-payload API. |
| `copyAttachmentsFrom(entity, wasDeath)` | Partial | attachments survive dimension change; player respawn keeps `copy_on_death` entries; mob conversion copies every entry with a codec; `player_clone` (`hooks/base.rs`) | Copy rules are the attachment store's; there is no per-entity hook at the copy. |
| `getBlockBounciness(pos, state)` | Partial | `BlockPart::bounce_on` (`hooks/block_landing.rs`) | Edits post-bounce movement instead of returning a bounciness factor. |

## `ILivingEntityExtension`

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `canSwimInFluidType(type)` | Partial | `FluidTypeProperties.can_swim` | Per-fluid data, not per living entity. |
| `jumpInFluid(type)` | Missing | — | Fluid jump behaviour is engine-owned. |
| `sinkInFluid(type)` | Missing | — | Same. |
| `canDrownInFluidType(type)` | Partial | `living_drown` (`hooks/base.rs`), `FluidTypeProperties.can_drown` | The global drown event can cancel or change damage; fluid data decides whether it applies. |
| `moveInFluid(type, movement, fallDistance)` | Missing | — | Movement in fluid is engine-owned; no per-entity method. |
| `onDamageTaken(container)` | Partial | `living_damage_post` (`hooks/living_damage_post.rs`), `living_incoming_damage` (`hooks/base.rs`) | Global server hooks: the incoming container before the hit, and a post-damage notice at the end of every hurt that passed the cooldown (health damage 0 when none was taken), not a per-entity callback. |

## `IBlockEntityExtension`

Block entity parts are attached with `r.block_entity_part`; `BlockEntityPart` extends `BlockPart` and carries the
block-entity hooks (`crates/horizon-mod-vocab/src/hooks/` with `scope = "block_entity"`). Saved data and attachments
are reachable through `BlockEntityRef`.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `onDataPacket(connection, input)` | Partial | `BlockEntityPart::block_entity_update_tag` (`hooks/base.rs`) | The server half (`getUpdateTag`) is settable; the client's merge of the received tag is engine-side. |
| `handleUpdateTag(input)` | Partial | `BlockEntityPart::block_entity_update_tag` (`hooks/base.rs`) | Same: the data is covered, the client callback is not. |
| `getPersistentData()` | Covered | `AttachmentStore`, `AttachmentHolder::BlockEntity` | Typed saved/synced attachments replace the writable compound. |
| `onChunkUnloaded()` | Covered | `BlockEntityPart::on_chunk_unloaded` (`hooks/on_chunk_unloaded.rs`) | Once per mod block entity of an unloading chunk, on each side holding it, before it is dropped; not when the world closes. |
| `onLoad()` | Partial | `BlockEntityPart::block_entity` (`hooks/base.rs`) | Initial saved data is set at creation, before the saved NBT is applied; no after-load notice. |
| `requestModelDataUpdate()` | Missing | — | No model-data invalidation API. |
| `getModelData()` | Missing | — | No `ModelData`; drawing uses `block_entity_submit` and the model path. |
| `invalidateCapabilities()` | Covered | `LevelRef::invalidate_capabilities` (`crates/horizon-mod-vocab/src/host/levels.rs`) | `level.invalidate_capabilities(host, pos)` with the block entity's position, as NeoForge's calls `Level.invalidateCapabilities(worldPosition)`: it moves the capability generation there on; every `BlockCapabilityCache` there compares it and looks up again at its next `get`. At the root commit inside a transaction; nothing when it aborts. |
| `applyStructureRotation(mirror, rotation)` | Missing | — | Structure rotation is engine-owned; no callback. |

## `IFluidExtension`

Fluid type facts are one record, `FluidTypeProperties` (`crates/horizon-mod-vocab/src/block/fluid_states.rs`), read with
`host.fluid_states().type_properties(key)`. They are static per fluid, not per position or block, and they are
derived today: the record describes water's and lava's families and gives every other fluid inert values
(`crates/horizon-client/src/game/modding/native_block_states.rs`). A mod's own fluid declares a sprite and tint,
not its type properties, so "partial" below means the vanilla answer is readable but a mod fluid cannot set it.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `getExplosionResistance(state, level, pos, explosion)` | Missing | — | No fluid explosion resistance. |
| `getFluidType()` | Partial | `host.fluid_states().type_properties` | Reads water's and lava's type record; a mod's own fluid cannot declare one and reads as inert. |
| `canConvertToSource(state, level, pos)` | Partial | `create_fluid_source` (`hooks/create_fluid_source.rs`) | Global `CreateFluidSourceEvent`; not per-fluid and not a Part. |
| `supportsBoating(state, boat)` | Partial | `FluidTypeProperties.supports_boating` | Static, and only a water-family fluid supports boating; a mod fluid cannot opt in. |
| `getBlockPathType(state, level, pos, mob, canFloat)` | Partial | `BlockPart::is_pathfindable` (`hooks/is_pathfindable.rs`) | Block-level path computation type only; no fluid `PathType`. |
| `getAdjacentBlockPathType(state, level, pos, mob, pathType)` | Missing | — | No adjacent fluid path type. |
| `canHydrate(state, level, pos, farmland)` | Partial | `FluidTypeProperties.can_hydrate` | Static, and only a water-family fluid hydrates; a mod fluid cannot opt in. |
| `canExtinguish(state, level, pos)` | Partial | `FluidTypeProperties.can_extinguish` | Static, and only a water-family fluid extinguishes; a mod fluid cannot opt in. |
| `appendHoverText(stack, context, display, lines, flag)` | Missing | — | `FluidStack` has no tooltip hook. |

## `BlockItemExtension`

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `onBlockPlacedBy(context, level, pos, state, placer, stack)` | Covered | `ItemPart::on_block_placed_by` (`hooks/on_block_placed_by.rs`, hook version 1) | Server notice with a `ServerHost`, the pre-shrink stack, optional player, primary position, final state and placement face/click. Fires once after stack block-entity data and `Block.setPlacedBy` companions exist, before level events and consumption, on player and dispenser block-item placement; a dispenser has no placer. Refused, failed and `entity_place`-canceled placements never notify. An item with no subscribed Part costs one lookup and no stack conversion. |

With `HORIZON_TEST_RESOURCES` pointing to prepared 26.3 resources, the isolated Game regression
places a custom block item, a two-part red bed and a dispenser's shulker box, and checks cancellation
and refusal without consuming the stack:

```sh
cargo test -p horizon-client --lib game::native_hooks::on_block_placed_by_tests::block_items_notify_after_setup_and_before_consumption -- --exact --ignored
```

## `IMenuTypeExtension`

Menu types are content data (`MenuType` in `crates/horizon-mod-package/src/content.rs`, files under
`data/<id>/horizon/menu/`), and the game's own menu engine opens them (`menu::Kind::Mod`). A `MenuPart` can answer
slot rules and whether a player's block menu stays open (`menu_still_valid`, vanilla's `stillValid`), not
construct a menu.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `create(IContainerFactory)` (static) | Partial | content `MenuType` declaration | Declares slot count, data slots and take-only slots, but cannot supply a factory that builds the client menu from server-sent data. |
| `create(windowId, inventory, buffer)` | Partial | menu engine + `MenuPart::menu_slot_rules` | The engine builds the menu from the declaration; a mod cannot run code at construction. |

## `IDispensibleContainerItemExtension`

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `emptyContents(entity, level, pos, hitResult, container)` | Covered | `ItemPart::empty_contents` (`hooks/empty_contents.rs`) | Asked on the server for a player's pour once `bucket_empty` let it go on, and for a dispenser before its own placement (`player` and `hit` are `None`). `None` keeps vanilla's emptying; `Some(true)` means the part placed the contents and the stack becomes an empty bucket; `Some(false)` fails. An item with the part reaches the dispenser's bucket behaviour even when it is no mod-fluid bucket, and a server's client does not predict its pour. |

## What the follow-up issues cover

[#1038](https://github.com/arismoko/horizon/issues/1038) (blocks) and
[#1039](https://github.com/arismoko/horizon/issues/1039) (items) were written from the earlier audit. Some of
their rows have landed or become partial since and should be narrowed when they are filed:

- **Changed on the block side:** `shouldCheckWeakPower` and `should_redstone_wire_connect_to` (the issue says
  reuse them) are Parts; `getFlammability` / `getFireSpreadSpeed` are now `fire_odds` (with position and face), with `is_fire_source`, `ignited_by_lava` and
  `on_caught_fire` for the other fire questions; `canSustainPlant` is `can_sustain_plant` (the soil) before `may_place_on` (the
  plant); `canHarvestBlock` is `can_harvest_block` before `player_harvest_check`; `isLadder` is `is_ladder`;
  `onTreeGrow` is `on_tree_grow`; `getToolModifiedState` is `tool_modified_state` before
  `block_tool_modification`; `onDestroyedByPlayer` is `on_destroyed_by_player`; `canBeConnectedTo` is
  `can_be_connected_to` (the target) before `connects_to` (the connector); `onBlockStateChange` is
  `on_block_state_change`, after `block_changed`; `rotate` is a worldgen Part method of the same name.
- **Changed on the item side:** `getEquipmentSlot` is `ItemResource::equippable_slot`; `getCraftingRemainder`
  has `ItemPart::crafting_remainder` plus content data and `host.item_crafting_remainder`; `canEquip` is `ItemPart::can_equip`; `getXpRepairRatio` is `ItemPart::xp_repair_ratio`.
- **Block group rows that still need their own answer:** `getExpDrop`,
  `getWeakChanges`, `onNeighborChange`, `getBounceRestitution`, `isSlimeBlock`, `isStickyBlock`,
  `canStickTo` (all Partial or Missing above).
- **Item group rows that still need their own answer:** `onEntityItemUpdate`, `canPerformAction` (both
  Partial or Missing above).
- **Missing but not in either group:** the remaining block, entity, block-entity and fluid rows marked Missing
  above; they have no follow-up issue yet. The largest are the entity fluid queries, block entity model data and
  fluid tooltips.

### Resource-backed block-part checks

With `HORIZON_TEST_RESOURCES` pointing to prepared resources, run the tool and player-removal
fixtures together (including the ice-denial regression):

```sh
for suite in block_part_tool_tests player_destroy_tests; do
    cargo test -p horizon-client --lib "$suite" -- --ignored
done
```

These checks require prepared resources; they are not ordinary resource-free unit tests.

## IClientBlockExtensions: mining hit sound

| Method | Verdict | Horizon route |
| --- | --- | --- |
| `IClientBlockExtensions.playHitSound` | **Covered** | Client `BlockPart::play_hit_sound`, with state, position, face and `ClientHost`. Returning `Flow::Cancel` handles the hit and suppresses the default sound. Both local mining ticks and `ClientLevel.addBreakingBlockEffects` use the same sound helper. Sculk growth is outside this hook. |

## Client chunk events

This section is not an extension interface. It records where NeoForge's client-side `ChunkEvent` posts land in
Horizon, as #3425 asks.

| NeoForge | Verdict | Horizon route |
| --- | --- | --- |
| `ChunkEvent.Load` on the client (`isNewChunk` always false) | **Covered** | `client_chunk_load(dimension, chunk)`, once per adopted chunk packet after the column and its block entities are in the client level; a re-sent column is told again. |
| `ChunkEvent.Unload` on the client | **Covered, wider** | `client_chunk_unload(dimension, chunk)`, before the column's data is freed, on every route a column leaves by: the forget packet (NeoForge's only route), the view centre moving, the view radius shrinking and the client level going. |

Both are client-only, so the server's `chunk_load` and `chunk_unload` listeners are not called on a client. An
integrated world's own chunks are told only through the server hooks.

## IShearable: shearing entities

| Method | Verdict | Horizon route |
| --- | --- | --- |
| `IShearable.isShearable(player, item, level, pos)` | **Covered** | `EntityPart::is_shearable(host, entity, player, shears)`, server-side, asked by a player's shears and a dispenser's before `player_shear_entity` / `block_shear_entity`; the first part to answer wins, `None` leaves it to the species. A `false` from a part ends a player's click with nothing done. |
| `IShearable.onSheared(player, item, level, pos)` | **Covered** | `EntityPart::on_sheared(host, entity, player, shears)`, called on the part that said yes once the cancel hook passed; it returns the drops. |
| `IShearable.spawnShearedDrop(level, pos, drop)` | **Covered, engine-side** | The game spawns each returned drop one block above the entity with the sheep's random nudge, posts the shear game event and damages the shears. Not overridable per part. |

Vanilla's shearable species (sheep, mooshroom, snow golem, bogged, copper golem, sulfur cube) keep their own
shearing when no part answers. Entity parts are native-only, so wasm mods do not have these methods yet.

## IFallableExtension: falling tick

| Method | Verdict | Horizon route |
| --- | --- | --- |
| `IFallableExtension.fallingTick(level, currentPosition, entity)` | **Partial** | Server `BlockPart::falling_tick` (`hooks/falling_tick.rs`) on the falling block's state, with its block position and entity handle and a `&mut dyn ServerHost`, for each tick that ends with it still in the air; not the landing, drop or removal tick. It is delivered after the ordered pass that ticked the falling block, so a write lands on the stored entity. NeoForge also calls it on the client; Horizon calls it only on the server. |

## IBaseRailBlockExtension: rail track

| Method | Verdict | Horizon route |
| --- | --- | --- |
| `getRailDirection(state, level, pos, cart)` | **Covered** | `BlockPart::rail_direction` (`hooks/rail_direction.rs`), on both sides with a `LevelSide`, starting from the `shape` property: the cart's movement asks with the cart wherever it reads the track under it, and `RailState` and a minecart item's placement ask with none. |
| `canMakeSlopes(state, level, pos)` | **Covered** | Server `BlockPart::rail_can_make_slopes` (`hooks/rail_can_make_slopes.rs`), asked as `RailState` would make the rail climb. |
| `isFlexibleRail(state, level, pos)` | **Covered** | Server `BlockPart::rail_flexible` (`hooks/rail_flexible.rs`), read into `RailState`'s straightness. |
| `isValidRailShape(shape)` | **Covered** | Server `BlockPart::rail_valid_shape` (`hooks/rail_valid_shape.rs`), with the rail's state, asked before `RailState` writes a shape; a refused shape keeps the current one. It starts false for a value the rail's `shape` property lacks, as NeoForge's default answers. |

A minecart now treats any block in `#minecraft:rails` built on `BaseRailBlock` as track (`BaseRailBlock.isRail`), where it used to accept only the four vanilla rail names.

## Container loot results

| Surface | Status | Contract |
|---|---|---|
| `container_loot_filled` | Implemented; Horizon addition | Server-side rolled list before shuffle and placement; blocks, chest minecarts and chest boats; each double-chest half separately. Only a player open that starts the fill names an opener. Oversized counts are trimmed and excess stacks discarded. |
| Global loot modifier data files (#1270) | Separate work | This hook exposes container results and does not load NeoForge modifier data files. |

## Standalone model baking

These are model-registration counterparts, not object-extension methods (#3430).

| NeoForge | Verdict | Horizon route |
| --- | --- | --- |
| `ModelEvent.RegisterStandalone.register` | **Covered for quad results** | Client `standalone_models` contract version 2 retains plain file ids in `models` and adds `forms: Vec<StandaloneModelForm>` on the same hook, with `RegistrationContext` on native and wasm. |
| `SimpleUnbakedStandaloneModel` with a `ModelState` | **Covered for quad results** | `Rotated { id, model, turns, uvlock }` uses the blockstate baker's centered negative XYZ quarter-turns and inverse face UV transform; no draw-pose substitution. |
| `UnbakedStandaloneModel.resolveDependencies` and quad composition | **Covered for declared file dependencies** | `Combined { id, parts }` loads all sources and their transitive dependencies, concatenates their quads and publishes one `ModelRef`. A missing dependency drops the entire form and logs its registering mod. |
| Mod-defined baked result type `T` | **Out of scope** | Horizon's standalone result is baked quads, not arbitrary mod-owned objects. |

The namespace rule, `RenderAssets::model` resolution and `model_baking_completed` publication
are unchanged. Forms rebake on every resource load; failed reloads leave old handles valid.
See [the model drawing guide](../../modding/src/how-to/client/model-drawing.md#bake-time-standalone-forms).

CPU proof selectors in `horizon-client-render` are
`models::catalog::standalone_tests::standalone_rotated_arrow_has_literal_geometry_and_world_locked_uvs`,
`models::catalog::standalone_tests::standalone_combined_keeps_both_cubes_and_rebakes_changed_dependencies`
and `models::catalog::standalone_tests::standalone_missing_dependency_drops_entire_form_and_logs_registering_mod`.
The resource-backed `horizon-client` test
`game::model_baking_completed_tests::standalone_forms_resolve_as_one_model_and_republish_after_resource_reload`
exercises real Game publication, model-registry lookup, distinct full-block/slab geometry,
missing-form refusal and stale/replacement handles. It requires `HORIZON_TEST_RESOURCES`
and `--ignored`. These are CPU/model-registry proofs, not GPU rendering or screenshot proof.

## Changes

- `enchant_power_bonus` contract v2 widens server to both sides: client enchant particles now consume the shared power answer through a per-game cache, rebuilt after parts reload or complete server-tag publication, including tag-update listeners (#3749).
