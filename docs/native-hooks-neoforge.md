# Native hook points, modelled on NeoForge 26.3 (proposal)

Status: proposal, for the authors of the hook PRs in flight to comment on. Nothing here is built yet.

Native mods hook empty functions in `crates/horizon-client/src/game/native_hooks.rs`, listed by name in
`native-hooks.json` ([native-mods.md](native-mods.md)). So far each hook was designed on its own, and the PRs
in flight number their API versions over each other and change signatures between PRs. This proposes one
reference for every new hook point: the NeoForge 26.3 event that does the same thing.

Why NeoForge:

- Its events already answer the questions each of our hooks asks again: where exactly it fires, on which side,
  what it can cancel, what it is handed.
- Mod authors know them. A Java mod's `@SubscribeEvent` methods map to hook names one to one, which makes
  porting by hand, or by [horizon-port-pack](modpacks.md), straightforward.
- It targets the same Minecraft version: NeoForge has a `26.3.x` branch and 26.3.0.x builds.

What we take is event names and their documented contracts, from NeoForge's public user documentation
(<https://docs.neoforged.net/>) only, as [CONTRIBUTING.md](../CONTRIBUTING.md#only-original-code) allows. Nobody
reads, copies, translates or paraphrases NeoForge's source, its patches included: where a hook fires is decided
from vanilla 26.3's own methods, and the hooks stay Horizon's own Rust.

## Rules

1. **Name.** A hook with a NeoForge counterpart takes the event's name in snake case, its nested class
   included: `ServerTickEvent.Pre` → `server_tick_pre`, function `native_server_tick_pre`;
   `PlayerInteractEvent.RightClickBlock` → `right_click_block`. A hook with no counterpart (the minimap, the
   mods list, the item panel) keeps a Horizon name.
2. **Firing point.** It fires in Horizon's port of the vanilla method where the documented event happens
   (the tick starts, a block is broken), at the point vanilla's own steps make that true. The catalog names the vanilla method, so a
   reviewer can check it with `tools/rea_vanilla.py`.
3. **Side.** NeoForge fires an event on whichever side runs the method. Horizon has one binary with several
   `Game`s (the window, the server thread, a dedicated server), so each hook says its side:
   - `server`: the server's `Game` (single player's server thread, LAN host, dedicated) and the in-frame path.
   - `client`: the window's `Game`.
   - `both`: both, with the `Game`'s role available (`native_window`). Interactions are `both`, as in
     NeoForge: the client fires them for its prediction, the server for the real action. A mod that claims a
     use on one side only gets the two out of step.
4. **Cancel and results.** Hooks never return; several mods hook the same point. A cancellable event takes
   `canceled: &mut bool`; one with a result (an `InteractionResult`) takes `result: &mut Option<...>`. Every
   mod is called; a mod checks `*canceled` itself when it should stand back (NeoForge's `receiveCanceled`).
5. **Payload.** The event's getters become parameters, by value or `&`. A hook gets `&mut self` only when the
   NeoForge listener may change the world at that point (server-side events); otherwise `&self`, and the
   doc does not call a `&mut` hook read-only.
6. **Frozen once merged.** Name, parameters and call site never change after a merge (as today). A changed
   contract is a new hook (`..._v2`) or a new catalog `version`. A stack of PRs that refines one hook lands
   in one release batch, under one `NATIVE_HOOKS_API` bump.
7. **API numbers are given at merge**, not when a PR is opened: a PR says "API next" and takes `main + 1`
   when it merges, so open PRs stop colliding.

## Catalog fields

Each `native-hooks.json` entry gains three optional fields:

```json
{
  "name": "server_tick_pre",
  "version": 1,
  "target": "horizon_client::game::native_hooks::Game<'_>::native_server_tick_pre",
  "neoforge": "net.neoforged.neoforge.event.tick.ServerTickEvent.Pre",
  "side": "server",
  "cancellable": false
}
```

`neoforge` is absent for Horizon-only hooks. horizon-mixins ignores fields it doesn't know, so this needs no
tool change; the launcher and the docs can list them.

## The hooks we have

Kept as they are (rule 6); the catalog gains the counterpart for reference.

| Hook | Side | NeoForge counterpart | Note |
|---|---|---|---|
| `frame` | client | `RenderFrameEvent.Post` (client) | |
| `tick` | both | `ClientTickEvent.Post` / `ServerTickEvent.Post` | one hook for both sides; new code should use the split hooks below |
| `world_lifecycle` | server | `LevelEvent.Load` / `.Unload`, `ServerStartedEvent` / `ServerStoppingEvent` | phases, not one event each |
| `hud` | client | `RenderGuiEvent.Post` | layers would be `RenderGuiLayerEvent` |
| `key` | client | `InputEvent.Key`, `InputEvent.MouseButton.Post` | |
| `wheel` | client | `InputEvent.MouseScrollingEvent` (cancellable) | `claimed` is its cancel |
| `command` | server | `CommandEvent` | |
| `zoom` | client | `ComputeFovModifierEvent` | |
| `minimap`, `mods`, `settings`, `settings_click`, `panel_event` | client | none | Horizon's own |

## The PRs in flight

| PR | Its hook | NeoForge counterpart | To change before merge |
|---|---|---|---|
| #3878 (arismoko) | `server_tick` (Start/End) | `ServerTickEvent.Pre` / `.Post` | fire Start at the top of `tick_server`, before the first level's streaming; don't call a `&mut` hook read-only |
| #3923 (arismoko) | `native_server_block` (a query) | `Level.getBlockState` | a helper, not an event; fine as is |
| #3941 (arismoko) | `neighbor_changed` | none exact: `BlockEvent.NeighborNotifyEvent` fires on the *notifying* block, cancellable, with the directions | keep it as a Horizon hook (it observes the receiving side); a `neighbor_notify` hook can be added later |
| #3981 (arismoko) | `block_changed` | none exact (`IBlockExtension.onBlockStateChange` is per block) | say "written", not "committed" |
| #4284 (ludovic) | `tooltip` | `ItemTooltipEvent` | name it `item_tooltip` |
| #4302 (ludovic) | `use_item` | `PlayerInteractEvent.RightClickItem` and `.RightClickBlock` | split into `right_click_item` and `right_click_block`, side `both`: this is what removes the double call when a mod item is used on a block |
| #3996 → #4289 (Sylviromi) | block entity tick, slots, menus, screens | see below | reshape before any of it merges |
| #3964 (levzzz5154) | data files (helpers, no hook) | none | takes the next API number |

## Next hooks, by priority

Server, gameplay mods first:

| Hook | NeoForge | Side | Cancellable | Vanilla method it sits in |
|---|---|---|---|---|
| `block_break` | `level.block.BreakBlockEvent` | server | yes | `ServerPlayerGameMode.destroyBlock` |
| `block_place` | `BlockEvent.EntityPlaceEvent` | server | yes | `BlockItem.place` |
| `left_click_block` | `PlayerInteractEvent.LeftClickBlock` | both | yes | `ServerPlayerGameMode.handleBlockBreakAction`, `MultiPlayerGameMode` |
| `right_click_block` | `PlayerInteractEvent.RightClickBlock` | both | yes, with a result | `ServerPlayerGameMode.useItemOn`, `MultiPlayerGameMode.useItemOn` |
| `right_click_item` | `PlayerInteractEvent.RightClickItem` | both | yes, with a result | `ServerPlayerGameMode.useItem` |
| `entity_interact` | `PlayerInteractEvent.EntityInteract` | both | yes | `Player.interactOn` |
| `player_tick_pre` / `_post` | `PlayerTickEvent.Pre` / `.Post` | both | no | `Player.tick` |
| `level_tick_pre` / `_post` | `LevelTickEvent.Pre` / `.Post` | server | no | `ServerLevel.tick` |
| `entity_join_level` | `EntityJoinLevelEvent` | server | yes | `ServerLevel.addEntity` |
| `living_incoming_damage` | `LivingIncomingDamageEvent` | server | yes | `LivingEntity.hurtServer` |
| `living_death` | `LivingDeathEvent` | server | yes | `LivingEntity.die` |
| `living_drops` | `LivingDropsEvent` | server | yes | `LivingEntity.dropAllDeathLoot` |
| `chunk_load` / `chunk_unload` | `ChunkEvent.Load` / `.Unload` | server | no | chunk map load/unload |
| `container_open` / `_close` | `PlayerContainerEvent.Open` / `.Close` | server | no | `ServerPlayer.openMenu`, `doCloseContainer` |
| `server_chat` | `ServerChatEvent` | server | yes | chat handling |

Client:

| Hook | NeoForge | Note |
|---|---|---|
| `client_tick_pre` / `_post` | `ClientTickEvent.Pre` / `.Post` | the window's half of `tick` |
| `render_gui_layer` | `RenderGuiLayerEvent.Pre` / `.Post` | per vanilla HUD layer, cancellable before |
| `screen_init`, `screen_render`, `screen_key`, `screen_mouse` | `ScreenEvent.Init.Post`, `.Render.Post`, `.KeyPressed.Pre`, `.MouseButtonPressed.Pre` | for mods that add to vanilla screens |
| `render_level_stage` | `RenderLevelStageEvent` | world-space drawing; needs a small drawing API first |

## Block entities, menus and screens

NeoForge 26.3 splits what Sylviromi's stack does into parts that each stay stable:

- **The block entity and its ticking.** A mod registers a block entity type for its blocks and a ticker;
  vanilla calls the ticker with the block entity *still in the world*. A Horizon `block_entity_tick` hook
  should do the same, so a machine that breaks its own block still drops its contents (the item loss found
  in #4055).
- **What others can reach: capabilities.** Hoppers, comparators, pipes and other mods don't read a fixed
  `slots` field; they ask the block for a capability: `Capabilities.Item.BLOCK` gives a
  `ResourceHandler<ItemResource>` (the `transfer` package), `Capabilities.Energy.BLOCK` an `EnergyHandler`,
  each per side. A Horizon `block_capability(pos, kind, side, out)` hook fills an out parameter with a
  handler object; hoppers and comparators use it. New kinds (fluids, energy) are new kinds, not new fields
  on a struct, so mods built against an older API keep compiling.
- **Menus.** A `MenuType` with extra data sent on open (`IMenuTypeExtension`), server-side slots, and
  `quickMoveStack` on the server: a slot the screen hides must not exist in the server menu either (the
  shift-click issue in #4106).
- **Screens.** Registered per menu type on the client (`RegisterMenuScreensEvent`); public structs that mods
  fill get constructors or builders, so adding a field doesn't break them (the `NativeScreen` defaults in
  #4289).

Proposed order: `block_entity_tick`, then `block_capability` for items, then menus, then screens, landing as
one batch under one API number.

## Rollout

1. This document, and the three catalog fields filled in for the hooks we have.
2. Arismoko's #3878–#3981 merge in one batch (API 4–7, or one shared bump), with the #3878 firing point fix
   as a follow-up.
3. #4284 as `item_tooltip`; #4302 split into `right_click_item` / `right_click_block`, both sides.
4. Sylviromi's stack reshaped as above.
5. Then the priority list, one small PR per hook or per family, each with a test that it fires where
   NeoForge's does.

## Open questions

- Is the snake-case NeoForge name the one we want, or a `neoforge_` prefix to mark the family?
- Do `both`-side hooks pass the role as a parameter, or do mods keep calling `native_window`?
- Should horizon-port-pack translate `@SubscribeEvent` methods of ported Java mods to these hooks?
