# Host helpers and remaining gaps

`Host` exposes the documented `Game` helpers declared by area in
`crates/horizon-mod-vocab/src/host/<area>.rs`, one trait per area; `Host`, `ServerHost` and `ClientHost` are the
unions of those traits, so a mod calls through them and never names an area trait. Each helper keeps its
receiver and documentation; a helper without `self` (`config_files`) takes
`&self` in `Host`. Level services stay on the level hosts above.
For the per-method comparison with NeoForge's block, item, entity, block-entity and fluid extension interfaces,
including which gaps are tracked, see [NeoForge extension coverage](neoforge-extensions.md).
For item registry facts, `host.item_crafting_remainder(&stack)` returns a count-one
plain remainder stack, or `None` when there is none or the item is unknown. Horizon
currently stores only the remainder item id, not a template's count or components.
`host.block_item(stone)` returns its item registry name; an unavailable block mapping
returns `None`, while a known block with no item returns `Some("minecraft:air")`.
Both reads are available on either side.

`level.entities_in(host, min, max)` also works on the client's displayed level: tracked
entities and the window's own non-spectator player are returned as readable handles.
Other players are omitted (arismoko/horizon#3033);
a level the client does not show, or a read before login, returns `None`.
A bound that is NaN or infinite returns `None` on either side, before any entity is walked;
a minimum above its maximum is the same box with the two swapped, as vanilla's `AABB`.
`host.experience_orbs(dimension, min, max)` takes its box the same way, with an empty list
for a bound that is not finite.
Every entity list is walked once per call, O(entities) regardless of the box;
each overlapping mob resolves through a mob-list scan, O(K*N) for K overlaps and N mobs.

For engine contributors, every area method declares its load access once with `#[access(...)]`.
`Metadata` admits registry facts during setup and reload; `WorldOnly`, `ClientSetup` and
`PublishedResources` retain their phase-specific contracts. The area's `host_area` attribute
builds the test-double defaults, and the API generator emits every game forwarder at build time.
Implement only the inherent `Game` function with the same name, not a hand-written forwarder.
Static implementations use `#[static_call]`; service accessors use `#[service]`.
A helper's `receiver` also exposes it on that handle. Receivers come from public
`*Ref` structs in `mod_api.rs`: `BlockEntityRef` gives `block_entity`, for example.
Adding a handle requires no separate generator registration.
The following documented `Game` helpers are not in the trait yet:

- `spawn_mob`, `mob`, `mobs`, `block_in_tag`, `item_in_tag`,
  `server_block_in_tag`, `server_item_in_tag`, `server_block`,
  `set_display`, `sidebar`, `experience_orbs`,
  `item_entities`, `player`, `panel_recipes`, `recipes`, `component`,
  `spawn_falling_block`, `explode`,
  `container_slots` uses gameplay-private types.
- `with_mod_block_entity` is generic, so its signature cannot be called through `dyn Host`.

Component writes live on `ItemPlace::edit(host, tx, |stack| { ... })`, not on
`Host::set_component` or `remove_component`. The editor can use `ItemStack::set`,
`remove`, `apply_components` and `copy_component_from`; `get` followed by `set`
covers `update`. A successful edit exchanges the whole stack, preserving the slot's
count rather than splitting off one item. Changing the item or count refuses the
edit without changing the place. The write is server-side and journaled in the
caller's transaction, so a parent abort restores it like other item-place writes.

Experience-orb values use vanilla's signed `i32` representation in
`ExperienceOrb`, `ServerHost::set_experience_orb`, and the pickup and merge hooks.
Reading or changing a value preserves zero and negative saved values rather than
clamping them through an unsigned representation. A negative value does not grant
positive experience when picked up.

Advancement triggers instead live on `PlayerRef::trigger_criterion`, with a mod-owned
`CriterionTest` and the game's scoped `CriterionContext` resource. The context keeps real
world-aware item predicates; it is not a conditions-only snapshot. See
[advancement triggers](https://github.com/horizon-rust/horizon/tree/main/crates/horizon-mod-vocab/src/hooks) for the contract and
the wasm runtime's current same-group callback limitation.

A mod that consumes a block use can run vanilla advancement predicates and rewards
through `PlayerRef::trigger_block_use(host, pos)` or
`PlayerRef::trigger_item_used_on_block(host, pos, original)`. These server
operations use the player's own level and never read inventory. The default
helper fires only `default_block_use`, without a tool. The item helper fires
only `item_used_on_block` against `original` (copied after a before-block PASS
and before block/item effects). Neither helper fires `any_block_use`: the engine
alone fires it once per consuming block-use packet, including a consuming
right-click or before-block cancellation. A mod may explicitly fire the inner
criterion when handling such a cancellation without duplicating the outer one.
A consuming item phase fires `item_used_on_block` first with the pre-block copy,
including a forced empty-hand consume, then the engine fires `any_block_use` once.
The outer criterion sees the stack the hand held when block use started, with
the action's own in-place changes applied: consuming one, damage, a component
edit, or the creative-mode count restore. If the action used the stack up, the
criterion sees it empty. It does not see a stack a listener swapped into the hand,
or a whole replacement by the action itself, such as a bucket becoming a water
bucket. This matches vanilla, which passes the same stack object it read before
the action. Single player uses the same integrated-server packet path as LAN. PASS and FAIL do not fire the outer criterion. Items such as
shears, honeycomb and block-transforming items can legitimately fire
`item_used_on_block` twice: once inside the item and once from the game mode. This follows
`ServerPlayerGameMode.useItemOn` and `ServerGamePacketListenerImpl.handleUseItemOn`;
the predicate evaluators are `DefaultBlockInteractionTrigger`,
`ItemUsedOnLocationTrigger`, and `AnyBlockInteractionTrigger`.

`Ok(())` means the player was reached, even if no criterion matched. Failures are
`NotServer` for a remote client, `PlayerUnavailable` for a stale, offline or
displaced handle, `UnloadedChunk` for a position outside loaded chunks, and
`Stack(StackWriteError)` when the item boundary rejects the snapshot before any
trigger fires. Native and wasm use the same generated operations. The
`minecraft:` guard on `trigger_criterion` remains: this grants no general access
to fire arbitrary vanilla triggers.

The following catalog hooks are not in the trait:

- `player_command`, `item_decorations`,
  `client_resources_reloaded`, `camera_shake`, `screen_key_pressed_pre`, `screen_char_typed_pre`,
  `screen_mouse_scrolled_pre`, `use_duration`, `use_animation`, `can_eat`,
  `living_entity_use_item_start`, `living_entity_use_item_finish`, `render_hand`, `shader_params`
  and `block_highlight`.
- `biome_colors`, `camera_distance`, `compute_fog_color`, `compute_fov`,
  `data_pack_files`, `render_living_pre`, `mouse_button_pre`, `mouse_button_post`, `player_respawned`
  and `tags_updated`.

The client `render_tooltip_texture` hook (NeoForge's `RenderTooltipEvent.Texture`) picks a tooltip box's look. Setting `texture` to a tooltip style `<namespace>:<name>` draws the GUI sprites `<namespace>:tooltip/<name>_background` and `_frame` from the resource packs, nine-sliced by their `gui.scaling` metadata or stretched without it, as vanilla draws an item's `tooltip_style`; the atlas a resource reload restitches is the one drawn. Left `None`, the box keeps Horizon's solid fills, whose background and top and bottom border gradient colours it changes as mutable ARGB `u32` values; it does not change text colours. It covers solid-fill item tooltips (including creative, recipe book, chat, trading and bundle-selected item names), screen text boxes, and native panel tooltips. The stack is `None` for text-only boxes, including recipe-book item lines where the stack is discarded before drawing. Sprite-based screen tooltips (including options and title screens) use a separate renderer and are not covered.

### Tooltip component geometry

`render_tooltip_gather_components` preserves the mixed text/image list, including
the built-in bundle image inserted immediately after the name (index 0 when
there is no text). `TooltipImage.source` selects `TooltipImageSource::Bundle`
or `ItemPart`, each carrying its stack. Gather listeners can move or remove
either source; pre sees the resulting list. Wrapping splits text in place.
The text pass visits the entire list before the image pass starts; both passes
advance through the same component heights, with 2 extra pixels after the first
component of an item tooltip. Each image callback receives the box's left edge
and its own running top, but the **whole content box size**, not `TooltipImage.size`.
That field is the component measurement used to reserve space. The box width is
the widest component; its height is the sum of heights, minus 2 normally, with no
subtraction for item tooltips containing multiple components. Text components
measure 10 pixels high. Factory registration and selectable fonts are separate
capabilities, not provided by this geometry contract.
