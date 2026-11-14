# Registry keys

Request existing entries inside `Hooks::register` with `r.block_key(id)`,
`r.item_key(id)`, `r.effect_key(id)` or `r.fluid_key(id)`, where `r` is a
`&mut dyn Registrar`. Retain the returned keys in your mod or Part and reuse them.

- A bare registration ID uses `minecraft:`. Requests are canonicalized and deduplicated.
- Requests resolve after every mod's content is known, so another mod can supply an entry.
- An unknown request refuses the load with the requesting mod and canonical ID, even if
  you never attach the key to a Part.
- Keys are opaque `u32` slots on native and wasm, not engine numeric IDs.
- `key.id(host)` returns an owned canonical `String` for diagnostics. It needs a game-backed
  host; an invalid slot is a boundary error, not an empty name.

Keys identify entries for the running mod load, not persistent data. Persist canonical identifiers
and property strings rather than registration slots or state ordinals. Reload preserves key identity;
world numeric mappings and tag membership are separate reloadable data.

## Entity-type reads

`EntityRef::kind(host)` and the handles that delegate to it return
`Option<EntityTypeKey>`, using the existing entity-type registry. The key is a
copyable scalar on native and wasm; compare it with a retained key instead of
allocating and comparing a type-name string on every read. A stale or missing
entity still returns `None`. There is no separate equipment-only type read.

After the registry is available, resolve the type keys your Part needs once and
retain them. `host.entity_type_keys()` enumerates that registry; `key.id(host)`
explicitly requests its canonical namespaced name. Name resolution returns an
owned diagnostic string and can allocate. Persist the name, never the scalar slot.
For example, retain the zombie key at setup, then compare
`entity.kind(host) == Some(zombie_key)` in a predicate.

The key derives `Wire` through its scalar slot: host decoding checks the registered
slot, while a wasm guest carries the host-issued key without a separate name table.

This is a clean break from `Option<String>`: callers that need textual names must
request `.id(host)` explicitly. `UnspawnedMobRef::kind` keeps its separate scoped
`Identifier`/error contract, and block-entity type reads keep `BlockEntityTypeKey`.

## Block states

`BlockState` is a small owned value with private fields, not a tint-specific snapshot or an
engine numeric ID. `state.block()` reads its `BlockKey`; `state.ordinal()` reads its local
ordinal. Property operations take explicit metadata: retain `let metadata = host.block_states()`
and call `state.property(&metadata, "facing")` or `state.with(&metadata, "facing", "east")`.
`BlockState::of(&metadata, key, constraints)` starts from the published default and applies the
supplied `StateConstraint`s. Changes return a new value and never mutate the original.
Unknown keys, ordinals and property combinations return typed `BlockStateError`s.

`SceneBlock::new(pos, state, entity)` and `StructureTemplateBlock::new(pos, state, nbt)`
use this owned `BlockState`, including all properties. Structure reads preserve vanilla palette
selection, mirror-then-rotation and placement order; an unavailable state mapping refuses the
whole snapshot with `None`. `DrawContext::draw_scene` checks the entire scene before drawing:
an unknown key, invalid ordinal or unavailable mapping draws nothing, never a partial scene or
a replacement air block. Native and wasm use the same values and refusal rules.

Tint callbacks receive immutable block metadata explicitly, without a `Host`.
Local state ordinals retain their meaning when world numeric state IDs are reordered.

Use `host.block_state_default(key)` for a block's default state and
`host.block_in_tag(key, tag)` for membership in a string tag name. On a restricted
level host, `schedule_block_tick(pos, key, delay)` also takes a `BlockKey`.

`host.block_states().states_of(key)` returns `Option<BlockStateRange>` in immutable
local ordinal order. It returns `None` before metadata is published, for an unknown block,
or without a game-backed host. Its iterator yields owned `BlockState`s; it is not a span of world
numeric IDs. `BlockStateRange::new(first, count)` returns a typed error if the span would
exceed the `u16` ordinal domain, and decoded transport spans obey the same check. Membership
checks both the block key and the ordinal. A zero-count span is empty.

A restricted `LevelHost::block_state(pos)` read returns `Option<BlockState>`. A successful
engine read retains its vanilla result: unloaded or out-of-height reads that produce air return
`Some(air)`, using the owning registry's real air state. An unavailable owning state index or a
refused engine-to-API conversion returns `None`; the boundary never fabricates a key or air state
and never panics to satisfy a mandatory value. `LevelRef` reads follow their loaded-position policy.

`LevelRef::set_block`, `update_neighbors` and `neighbor_changed` take owned block states
and return `Result<bool, LevelWriteError>`. `Stale` identifies a closed or reopened level;
`Unavailable` identifies the wrong side, phase or nonactive write target; `InvalidState`
retains the block-state refusal. Errors do not mutate the level. `Ok(false)` is
vanilla refusal, such as an unloaded or out-of-height position or an unchanged block,
not a lost handle or a guessed replacement state. Update flags retain their vanilla effect.

## Registry lookup and enumeration

Walking a whole registry uses the typed keys, never names: `host.block_keys()` and
`host.item_keys()` return every entry in the registry's registration order (vanilla's
entries first, then mods' in the order they registered); `host.entity_type_keys()`,
`host.menu_keys()` do the same for the built-in entity-type and menu registries.
`host.enchantment_keys()` uses global sorted loaded-registry order, with vanilla and mod
data-pack entries sorted together. Block enumeration lists each block once, not once per state. Build lookup tables from these lists at
load time instead of resolving names per tick. A table is empty before its registry loads; registration-time
`RegistrationContext` does not see any of them.

A name that only arrives at run time (data, config, a command, a network message) turns into a key
with the host: `host.block_key("minecraft:stone")`, `host.effect_key("mymod:rage")` and
`host.fluid_key("minecraft:water")` return `Option`, and `key.id(host)` gives the name back for
blocks, effects, fluids and items. Names are full registry names; the lookup reads the completed
registry and does not allocate, so a hook can call it. It returns `None` for a name the registry does
not hold, and before the registry is complete, so a mod uses `r.block_key(id)` in `register`. These
work on the client and the server, for native and wasm mods alike.

```rust
fn resolve(host: &dyn mod_api::Host) {
    if let Some(block) = host.block_key("minecraft:stone") {
        assert_eq!(block.id(host), "minecraft:stone");
    }
    if let Some(effect) = host.effect_key("minecraft:speed") {
        assert_eq!(effect.id(host), "minecraft:speed");
    }
    assert!(host.block_key("test:missing_block").is_none());
    assert!(host.effect_key("test:missing_effect").is_none());
}
```

These calls are also available on `&dyn ServerHost` and `&dyn ClientHost`.
Both lookups return `None` before their registry completes, including for vanilla names.

## Fluids and effects

Declare a tank-only fluid in your own namespace with `r.declare_fluid("chocolate")`; the returned `FluidKey` identifies it for the mod load. Request vanilla or another mod's declared fluid with `r.fluid_key("minecraft:water")` or `r.fluid_key("other:chocolate")`. Requests resolve after all registrations, so declaration order does not matter; an unknown request stops the load and names the requesting mod. Tank-only fluids have no world simulation or world state. Fluid stacks and recipe ingredients use these keys.
Declare a fluid that exists in the world with `let mut fluid = WorldFluid::default(); fluid.drop_off = 2; r.declare_world_fluid("oil", fluid);`,
using `mod_api::WorldFluid`, the code form of a fluid file's `world` object ([content mods](../how-to/content-mods.md)): it adds the block
`mymod:oil`, the flowing fluid `mymod:flowing_oil` and the bucket `mymod:oil_bucket`, and its fluid file, if it has one,
describes its appearance. `r.fluid_part("mymod:oil", Part)` attaches a `FluidPart`, whose `fluid_rules` can change the
declared values by position on the server. It is asked for every cell a spread looks at, so answer fast.

A saved tank whose fluid's mod is absent carries an opaque unresolved identity. Its stack's
`key()` is `None`; `UnresolvedFluid::id(host)` reads its saved name. It can move between
name-backed mod tanks, but not into vanilla buckets or cauldrons.

`ActiveEffect::id` and effect hook payloads carry `EffectKey`; living effect queries,
application and removal use the same resolved key. `ActiveEffect::new(speed, …)`
uses a key requested through `r.effect_key("minecraft:speed")`. Beacon-selection hooks
carry `Option<EffectKey>` for each power; `None` is no selection. Enchantment effect-component
names (for example `minecraft:damage` in `has_enchantment_effect`) are a different registry
and use strings, as do labels and tags.

## Hot item and block identities

Resolve item and block keys during load and keep them for tick and menu calls.
`item_burn_time`, `item_compost_chance`, `give`, `enchantment_supports`'s item,
and GUI `item` / `item_model` take `ItemKey`. `block_item`, `fluid_bucket` and
`screen_hovered_item` return item keys. `level.block`, `lit_block_name` and
`Hit.block` return block keys; `BlockEntityDrawContext::block` takes one.
`HeldItemSlot.id` (including use-start/stop events) and `Stack.item`
carry item keys, not names. Unknown incoming slots are refused at the boundary.
`block_sprites` takes API-owned `BlockState` values, preserving properties without
parsing state strings or exposing numeric engine state IDs. The pre-change block event carries resolved `from` / `to`
states and does not allocate their names. Property names and values in
`set_block_property` are not block identifiers and use strings.

Retain keys to avoid formatting identifiers on hot paths.

## Matched recipe lookup

Use `level.recipe_for(host, &recipe_type, &inputs)` to find a matching recipe.
Use `RecipeInput::Crafting { width, height, slots }` for `minecraft:crafting`
(row-major slots, dimensions 1 through 3), `Single(stack)` for smelting, blasting,
smoking, campfire cooking and stonecutting, or `Smithing { template, base, addition }`
for `minecraft:smithing`. Recipe **types** are not serializers: shaped and shapeless
files both belong to `minecraft:crafting`, and transform/trim both belong to smithing.

The server uses the same engine matchers and assembled outputs as its menus,
including data-pack overrides of vanilla recipe types. A `RecipeMatch` contains the
matched recipe identifier, typed output, ordered remaining items, experience and
processing time. Lookup consumes nothing; the caller applies inventory changes
transactionally. Keep the result until inputs or the recipe load change.

`Ok(None)` means valid input without a match. Invalid shapes/items and unavailable
levels are typed errors. A type without an engine matcher returns
`RecipeLookupError::NotMatchable`, not an empty match. For mod-owned types, retain
`recipes_of_type` once per load/reload and match that cached data **inside the mod**,
with zero crossings per tick. Dynamic custom outputs and contextual predicates
remain mod-owned; the host does not call guest code to match recipes.

At publication, call `ingredient.item_keys(host)` once per ingredient to retain
`Arc<[ItemKey]>` for local matching. This load-time read returns exactly the plain
item membership for lists, expanded tags and standard shapes, using the engine
matcher. It returns `IngredientKeysError::NotEnumerable` if any custom or component
predicate occurs anywhere in the tree; a displayed item list is not a safe substitute.
`UnknownItem` refuses a missing registry identity. Do not call this method per tick.
Stack placement predicates and drawing consumers (HUD items, GUI item decorations,
block-entity items and tooltip images) encode API stacks without enforcing resident
count or damage maxima. Engine-legal over-limit stacks can be inspected and drawn.
Unknown item keys and invalid/unrepresentable effect payloads refuse encoding;
this is not a total conversion. Insertion and other world writes retain strict validation.

## Owned values

Use API-owned values rather than importing engine or package-loader types.

| Value | Contract |
| --- | --- |
| `TooltipDisplay::for_stack` | Takes an API `ItemStack`; its effective tooltip component includes vanilla's implicit defaults. |
| `LootContext`, `LootStack` | Own their facts. Candidate items use `ItemKey`; counts are signed. Ordered `Option<Tag>` patches preserve removals and NBT types; names are `Text` trees. Writeback is atomic. |
| `ItemBar`, `ItemRarity` | Describe item resource properties. |
| `Side` | Package metadata: `Client` or `Both`, not the executing logical `HostSide`. |
| `Suggestion` | Construct with `Suggestion::new((start, end), text)`. Offsets are bytes; `tooltip` and `int_value` are writable. |
| `Block`, `Item` | Registration declarations. Component patches are `(String, Option<Tag>)` entries; `None` removes a component. Content-file admission checks apply. |
| `BlockEntityTypeDecl` | Declaration snapshot produced by `r.block_entity_type(...).build()`. |
| `GuiDraw` | GUI commands; `DrawContext::gui_paint` returns a typed text-conversion result. Handle refusal before drawing. HUD hooks reject invalid writeback as a whole. |

`ServerHost::game_rule(id)` reads the one server-wide rule store without a
`LevelRef`, including in `server_about_to_start` and `server_starting`.
It returns `Result<GameRuleValue, GameRuleReadError>`; an unknown identifier gives
`GameRuleReadError::UnknownRule`. `ServerHost::game_rules()` lists every available
rule and its current value by namespaced id. Both native and wasm mods can use
these reads. Saved values and a new world's create-screen choices are installed
before the first server hook, without firing `game_rule_changed` for loading.
Level reads use the same store once the requested level exists.

`mod_api::GameRuleValue` owns the `Bool(bool)` and `Int(i32)` cases, in that order.
`level.game_rule(host, id)` returns `Result<Option<GameRuleValue>, LevelReadError>`:
`Ok(None)` means unknown rule, `Stale` means a gone/reopened level, and `Unavailable`
means the side or phase cannot read it. `level.game_rules(host)` returns the typed
result containing every available rule, not an optional list. Use the boolean-key getter for a retained `BoolGameRuleKey`.

`GameRuleDecl` records a mod's boolean rule name, category and default. Category references
resolve after all declarations; invalid declarations refuse installation without a partial table.

## Opaque NBT metadata

The shared JSON/NBT conversion is iterative and follows vanilla's 512 nested
list/compound limit; scalars and NBT typed arrays add no recursive container.
Conversion to JSON intentionally loses numeric tag widths, boolean-as-byte
identity and typed-array identity. `NonFinite` refuses NaN and infinities:
`serde_json::Value` cannot hold them, and Gson's strict JSON writer also refuses
them. `InvalidString` refuses an unpaired UTF-16 surrogate in a Tag string or
compound key. Neither case is replaced with null, a string or U+FFFD.
JSON input already uses UTF-8, so valid non-BMP pairs survive; a lone surrogate
cannot reach this converter through that input representation.

## Owned component trees

The `text` module defines owned component data: literal and translatable contents,
style overrides, ordered children, selector and NBT sources, inline objects, and click
and hover actions. Translation keys and argument components stay unresolved until the
game displays them. `None` style fields inherit; `Some(false)` overrides an inherited
decoration. Metadata, dialog/custom click payloads and hover-item component patches are
owned `Tag` data, never JSON strings or engine state. `TextProfile` carries an optional
name and UUID, signed property records and opaque unknown profile/property fields.
Signed record order and unsigned values within each property name are preserved.
Player names use U+0021..U+007E and at most 16 characters. Vanilla's unsigned
property-map form allows at most 16 distinct names; signed or metadata-bearing
records use a 16-entry list, with UTF-16 limits of 64, 32767 and 1024 units for
property name, value and signature respectively.
Inline player and atlas objects use `Option<Box<Text>>` for their fallback: `None`
keeps the engine-computed default; `Some` preserves a styled or translatable component
rather than flattening it to a string.

`Text::validate()` checks tree/payload bounds and profile fields
without cloning; `validate_depth()` is the component-only preflight. NaN/infinity and
unpaired UTF-16 are legal Tag data but cause typed `NonFinite` or `InvalidString`
refusals when engine emission requires JSON. Nested JSON null is refused as `NestedNull`
when capturing opaque NBT fields; it is never silently replaced.
Host methods accepting or returning Text return `Result<_, TextFormError>`; mixed
item/Text writes preserve the refusal in `StackWriteError::Text`, and unspawned-mob
reads use `UnspawnedMobError::Text`. `TooDeep` means a component path or opaque NBT
container path exceeds 512. Unavailable backends are distinct from invalid text or
unrepresentable payloads.
A refused hook capture does not dispatch; a refused writeback leaves every field unchanged.
No subscriber means no conversion or snapshot allocation.
Mutable stack arguments are compared using value equality with the snapshot passed to
the listener. An unchanged argument retains the original engine value without strict
write validation (including engine-legal stacks above effective count or damage maxima).
A changed argument is validated in full; a refusal commits no outputs. The same
rule applies to anvil/grindstone records and menu cursor/quick-move stacks.

Recursive `Clone`, `Eq` and `Drop` of a tree a mod itself built are ordinary Rust ownership,
not part of the boundary guarantee.

`Text::new(content)` constructs any content kind without exposing an engine value.
The content, NBT-source, object, click and hover enums are exhaustive protocol enums;
match every case explicitly. Component records such as `Text` and `TextStyle`
are `#[non_exhaustive]`: use their constructors and accessors rather than external
struct literals, and leave room for additional fields when destructuring.
Typed content and style fields are authoritative; metadata retains unrecognized
properties, not overrides of those typed fields. Game-side encoding is fallible
and reuses the engine's translation-argument and fallback construction rules.
Capture keeps dynamic contents and metadata rather than flattening the rendered spans.
It shares the engine's SNBT flag, shadow-colour, click-policy and inline-object parsers.
File-opening metadata is discarded before a listener receives the owned value; a
no-op listener cannot promote an untrusted file action into a local action.
A translatable's fallback is a literal format template, not another language key.
A missing key whose fallback equals an existing key therefore displays the
fallback text itself; parsing and native constructors follow the same rule.

## Particle identities

Request a `ParticleKey` with `Registrar::particle_key` and retain it. Every parameter-family
`ParticleOptions` constructor takes its key; use `simple(key)` for a simple particle.
Unknown forward references refuse publication of the whole particle registry. Retained keys
resolve to vanilla or custom named sprite providers without parsing identifiers per read.

## Placement and loot snapshots

`mod_api::PlacementContext` owns the placement inputs, including world-space click
coordinates and the placer view/flags. Its `item` is `Option<ItemKey>`: `None` is a
real empty hand, including falling-block placement contexts, not an invented air
item. Nonempty items resolve through the load-owned item table. The game applies the returned block state.

The `snapshots` module owns placement inputs and loot-roll facts. Placement and tool
items are resolved `ItemKey`s. Loot context entities, damage sources, tools, biome,
block and place facts are owned values; retaining one never retains a borrowed engine
context or a live level. A loot candidate is not an inventory `ItemStack`: its signed
count can be non-positive while loot functions run, in which case it produces no item.
Exploration searches remain deferred data, and name components remain unresolved text.

## Item values and tag names

Every item has an `ItemKey`, including vanilla items no mod requested during registration.
`r.item_key(id)` retains a forward reference in the same load-owned table; after the item
registry is complete, `host.item_key(&identifier)` resolves any registered item. A key is
a private `u32` index, not a copy of an item definition. Resolve once and retain it.
`host.max_stack_size(key)` and `host.max_damage(key)` read item defaults without exposing
the engine's item table.

`ItemStack` is owned API data with private fields. `ItemStack::new(key, count)` refuses
zero, while `ItemStack::of(key)` makes one item. Count mutation refuses zero and arithmetic
overflow. Registry-dependent maximum count and damage, including component overrides,
are checked by the game when accepting a write; an oversized value is refused with a typed
error, never silently clamped. This is the same boundary on native and wasm. Consuming
`split(n)` returns `StackSplit { taken, remainder }`, both optional; consuming `shrink(n)`
returns the remainder or `None`. There is no empty-stack sentinel. Component descriptors
read and write an optional owned NBT patch; a plain stack has no patch allocation.

`host.tags()` and a restricted level host's `tags()` expose the same `Tags` service. Resolve
a validated identifier once with `tags.key(&identifier)` and retain its `TagKey`. The runtime
owns tag-name slots for the process; reload replaces membership, not names or key identity.
`tags.is_in_tag(item_key, tag_key)` tests merged item membership. `tags.items(tag_key)`
`tags.blocks(tag_key)` and `tags.fluids(tag_key)` return registered keys in vanilla's member order. Unknown tags
return false or empty lists. Clients read the tags sent by their server. Membership reads return `None` before
authoritative tags are applied or if the relevant registry mapping is unavailable. Key resolution itself can be
refused if the runtime cannot allocate a name slot. No lookup stringifies a retained key.
Block enumeration returns `BlockKey` values, including blocks no mod requested. An unresolved
member returns `None` for the entire list, not a partial result.
The engine `TagRegistry`, `StackData`, `StackView` and `DisplayContext` are not API types.

## Scoped Parts

Hooks scoped to a block, block entity, item or menu belong to `BlockPart`,
`BlockEntityPart`, `ItemPart` or `MenuPart`, respectively. `BlockEntityPart` is also a
`BlockPart`. Attach a Part to react to your own content; use a global `Hooks` event
when you need to react to every matching object.

- Scoped methods have no `Hooks` method. Objects without Parts are not called.
- Parts run in dispatch order; cancellation or replacement skips later Parts.
- A corresponding global event runs before Parts; its cancellation skips them.
- Attaching the same Part type twice to one object, an invalid key declaration, or an
  unnamespaced ID in other Part registrations refuses startup with diagnostics naming the mods.
- Parts take `&self` and require `Send + Sync`. A Part panic adds `, part <type>` to
  the mod and hook attribution.

## Damage and item metadata snapshots

The three damage hooks convert only after their subscription guard. `DamageValues`
retains the entire sequence, including reductions and the pre-hook input, so changing
`new_damage` reaches the engine's original NaN and negative normalization point.
`DamageValues::set_reduction` edits snapshot data; it does not run a damage calculation.
The fixed reduction storage is private; use its accessors rather than an array index.
Use `DamageContainer` setters to edit damage and cooldown; sequencing is game-owned.

`ItemResource::damageable`, `foil` and `repairable` delegate to the game's
property readers through metadata access. Defaults, explicit overrides, component
removals, compass glint and repair tags follow gameplay's single property path.

## Recipe-file fields and small values

`Host::recipe_ingredient` and `Host::recipe_result` take the field's JSON **text**,
not a `serde_json::Value`. Pass a quoted item id (`r#""iron_ingot""#`), a quoted
`#tag`, or an ingredient array. Parse failures return `RecipeJsonError::InvalidJson`;
unknown items and empty tags return `Ok(None)`. Result objects retain their
`components` patch. These helpers are reload-time reads, not per-tick JSON parsers.

`KeyConflictContext`, `FireworkShape` and `JarInputsError` are API-owned values.
`JarInputsError` carries owned diagnostics and the original-jar identity.
