# Where the mod API is going

The mod API is built toward one shape, and every change to it is judged against that shape. This page
explains the shape and why it is the one. The [trait mods page](../../modding/src/concepts/parts.md) says how to write against it
today; the [project board](https://github.com/users/arismoko/projects/1) and the pinned roadmap issue
[arismoko/horizon#771](https://github.com/arismoko/horizon/issues/771) hold the work that is still open.

## One clean break

A mod is written against one API, `mod_api` (the crate `horizon-mod-api`), and the API keeps one hook per
moment. A hook whose NeoForge 26.3 counterpart exists takes its name, side and full contract. Two hooks at the
same vanilla call site, `_v1` and `_v2` pairs included, are one hook, and the old one is deleted, not kept
beside it. A veto before an action and a notice after it stay separate when vanilla runs code between them. A
hook whose vanilla call site is gone leaves the catalog.

There are no shims: no aliases, deprecated names, fallback paths, version gates or "kept for old mods"
branches. An old mod rebuilds, and the compiler says what moved. Removing a path updates every caller, the
catalog, these docs, the tests and the sample mod in the same change. A hook with no NeoForge counterpart is
judged on its merits like any other.

Every hook is declared once, in the catalog (`crates/horizon-mod-vocab/src/hooks/`): its name, side, contract,
scope and `doc`. The generator writes everything repeated from it: the `Hooks` trait, the part traits, the
`Host` traits, the dispatch and the [reference](../../modding/src/reference/api.md). A hook is added by declaring it, not by
editing match arms.

## The `Host` is split by side

A hook receives the game through a host, and the host is a small set of traits, so that a call on the wrong
side does not compile:

- `Host` is the shared base: handle constructors, registries and settings.
- `ServerHost: Host` and `ClientHost: Host` are generated from each catalog entry's `side`. Only the server
  writes the world, so `transaction` is on `ServerHost` alone.
- A hook that fires on both sides receives a typed `HostSide` to match on. A hook fired inside a level write,
  where there is no game to borrow, receives the read-only `LevelHost` (a `LevelSide`) instead.
- Drawing goes through a `DrawContext`, a trait object given only to render and screen hooks, beside a
  `&dyn ClientHost`.

A method with a natural receiver belongs to a handle, not to the host: `EntityRef`, `LivingRef`, `PlayerRef`,
`LevelRef`, `BlockEntityRef` and the other handles carry the per-object methods. A block entity's lock is the vanilla item-predicate compound: `lock` reads it and `set_lock` sets or clears it through the transaction journal. `replace_data` replaces saved data while retaining the entity's identity and position; it does not change its type. Changing type requires the owning block's lifecycle path, as described in [transactional block-entity removal](block-entity-removal.md). Reads take `&dyn Host`; writes
take `&mut dyn ServerHost`. Once a handle method exists, the id-based helper it covers is removed, so there is
one way to do each thing. Attachments live on handles too. Commands, players, messages and scheduling are small
services reached from the host, not methods on it.

A hook fires with the right host for its moment. A client-side decision that vanilla makes inside a client
library is split into decide, fire and apply: the library decides, the game fires the hook with a `ClientHost`,
and the apply re-checks the decision's preconditions and refuses, never recomputes, if they changed.

## Load access belongs to the owning thread

Registration has only a `RegistrationContext`. Each side then enters Setup, Config and World;
config loading can read settings and use its existing scoped `ConfigFiles` service, but cannot
acquire world handles, open transactions or call other game operations. Client setup and initial
Client/Common configs finish before title, connection or ordinary runtime callbacks begin, so the
first client pre-tick is already in World even without an open world. Data and asset reloads
temporarily enter Reloading and restore Setup or World when they finish or fail. The catalog marks
published asset/data queries as `resource_read`: completion hooks can refresh sprite/recipe caches
from the newly published generation without acquiring a world handle or transaction. Server reload
Host reads are restricted to those published facts, registry facts, settings and the mod list;
existing data-context callbacks keep `DataLoadContext`. A forbidden operation panics naming
the phase. The game alone advances the thread-local boundary, so the window and integrated server
cannot change one another's permissions. [Load phases and access](../../modding/src/concepts/load-phases.md)
lists the once-per-process, per-world and per-reload moments. No additional FML or intermod bus is added.

## Registering behaviour

Choose the entry point by who owns the result. For behaviour attached to an existing block, item, entity
or menu, implement `Hooks::register` and call the corresponding method on `&mut dyn Registrar`. For a
subsystem that rebuilds its own table, implement its collection hook. The game owns both the lifetime
of the table and the point at which it becomes visible.

`Hooks::register` runs once at startup, in mod dispatch order, with a `RegistrationContext`.
The registrar records the owning mod, checks for conflicting declarations and reports refusals before
installing the table. It accepts registry keys and tags before the final per-game tables are available.
When a game resolves its registries and tags, it binds the Parts to the objects they target. Runtime
hooks then dispatch to the attached Parts. See [registry keys](../../modding/src/concepts/registry-keys.md)
and [Parts](../../modding/src/concepts/parts.md).

| Behaviour | Entry point | Side and collection time | When usable |
| --- | --- | --- | --- |
| Block, block entity, item, entity and menu Parts | `Registrar::attach_block_part`, `attach_block_entity_part`, `attach_item_part`, `attach_entity_part`, `attach_menu_part` | Startup on the sides that load the mod | After resolution against the game's registries. Each Part method runs on the side declared for that hook. |
| Capability providers | `provide_block_entity_capability`, `provide_block_capability`, `provide_item_capability`, `provide_entity_capability` on the registrar | Startup registration, server provider calls | After the game resolves the registrations. A capability query supplies the holder and a server host. |
| Block entity types, default entity attributes and entity spawn placements | `declare_block_entity_type`, `declare_entity_attributes`, `modify_entity_attributes`, `declare_spawn_placement` | Startup declarations on the sides that load the mod | When the registered types, defaults and spawn rules are resolved against the content tables. |
| Biome modifiers | `declare_biome_modifier` | Startup declarations on the sides that load the mod | When the world's biome tables build, with the data-file biome modifiers. Not applied by the game yet. |
| Biome regions | `declare_biome_region` | Startup declarations on the sides that load the mod | Sorted by mod id, then region id. A new world records them; from then on its saved layout, not the mods, decides the Overworld's and Nether's regions ([the design](mod-biome-regions.md)). |
| New attribute types | `declare_attribute` | Startup declarations on the sides that load the mod | When the attribute catalog grows, before entity attribute rows resolve. The mod attributes append after the vanilla ones, sorted by mod id then attribute id — the network id order on every side running the same mods. |
| Game rules and categories | `game_rule_bool`, `game_rule_category` | Startup, before the game validates and installs the rule schema | When the game uses the installed schema to load rule values and build the rule UI. |
| Dimension, chunk generation, biome source and density function code, structure types, ticket controllers | `register_dimension`, `register_chunk_generator`, `register_biome_source`, `register_density_function_type`, `register_structure_type`, `register_ticket_controller` | Startup declarations, consumed by world setup and generation | When the world resolves its dimension, structure or ticket registrations; a density function type when density JSON naming it loads. |
| Controls bindings | `Registrar::register_key_mapping` | Startup registration, client consumer | When the client installs the bindings in Controls and restores saved bindings. |
| Component tooltips | `Registrar::register_tooltip_appender` | Startup registration, client consumer | The item tooltip builder uses the callback for the registered component and placement. |
| Server commands | `Hooks::register_commands` collects `Command` usages | Server, when opening a world and on explicit command refresh | After the server rebuilds the command tree. The server owns permission checks and sends the tree to players. |
| Command argument types | `Registrar::register_command_argument` declares parsing and suggestions | Startup, on the sides that load the mod | Before command trees that use the type are built at world open or refresh; clients that do not know the type complete it from the server. |
| Client commands | `Hooks::register_client_commands` collects `Command` usages | Client, at connection start and when the server sends its tree | After the client incorporates them into completion and local dispatch. |
| HUD layers and F3 entries | `register_gui_layers` and `register_debug_entries` | Client startup | When the installed HUD order or debug entry list is used for drawing. |
| World-space debug renderers | `register_debug_renderers` | Client, each refresh of the renderer list | The next debug draw uses the refreshed list. |
| Model layers, colour resolvers, block and item tints, numeric and categorical item model properties | `register_layer_definitions`, `register_color_resolvers`, `register_block_tint_sources`, `register_item_tint_sources`, `register_range_select_item_model_property`, `register_select_item_model_property` | Client startup before resource loading | The client can resolve the declarations when it builds its models and rendering tables. |
| Entity renderers | `register_entity_renderers` registers an `EntityRenderer` per entity type | Client startup before resource loading; resolved again on every resource load | When a frame draws an entity of the type through a route that reads the table: the content mods' thrown types and the living types drawn as mobs so far ([entity renderers](../../modding/src/how-to/client/entity-renderers.md)). |
| Entity layers | `register_entity_layers` adds an `EntityRenderLayer` to a living entity type or a player model | Client, on every resource load after the entity renderers resolve | When a frame draws a mob or player of the target, after its model and vanilla layers ([entity layers](../../modding/src/how-to/client/entity-layers.md)). |
| Special item model renderers | `register_special_model_renderer` registers a `SpecialModelRenderer` per special model type, the last registration of a type winning | Client startup before resource loading; a failed renderer is asked again, and kept answers dropped, on every resource load | When an item whose definition holds a `minecraft:special` model of the type is drawn with a stack that model node has no kept answer for; every display context draws that answer ([special item renderers](../../modding/src/how-to/client/special-item-renderers.md)). |
| Data map types | `data_map_types` | Server, at world data load and each reload | After the server publishes the loaded data maps. |
| Recipe property additions | `recipe_property_sets` | Server data load and client preload, after recipes resolve, and again on reload | When station slot checks use the published sets. |
| Mod configs | `register_configs` | Config phase on the owning side, once at startup for Client/Common and per world for Server | After the file is read and the mod receives its config loading callback. |

[Content mods](../../modding/src/how-to/content-mods.md) declare identities and data such as blocks, items, entity types
and menus in `data/<id>/horizon/`. These declarations supply the game's registry entries, models or
templates and other data. The startup `blocks` and `items` hooks can submit the same declaration types
from code and go through the same content checks. Attaching a Part or providing a callback adds behaviour
to an identity. For example, a menu declaration describes its slots, while a `MenuPart` supplies code for
that menu's lifecycle. Likewise, the item-scoped `item_decorations` hook draws after the game's default
item icon decorations in slots, the hotbar and other views that use the shared item decoration path.

### Tracked gaps in registration

The issues below specify registrations still missing at this revision. Use the existing registration
surface for startup declarations and the owning subsystem's collection hook for tables rebuilt during
reload. Each builder should collect typed declarations and validate keys before publishing them.

| Builder | Side and contract | Issue |
| --- | --- | --- |
| Data pack registries | Declare the entry codec before server data loading and rebuild the registry on reload. | [arismoko/horizon#811](https://github.com/arismoko/horizon/issues/811) |
| Attribute types | Declare a namespaced attribute and its value rules before entity defaults resolve. | [arismoko/horizon#812](https://github.com/arismoko/horizon/issues/812) |
| World generation features | Bind named feature and placement implementations before server worldgen data resolves them. | [arismoko/horizon#815](https://github.com/arismoko/horizon/issues/815) |
| Particle providers | Bind client provider factories to particle types before the particle engine spawns instances of those types. | [arismoko/horizon#822](https://github.com/arismoko/horizon/issues/822) |
| Entity spawn placement | Bind the server placement rules and predicates to entity types before natural spawning. | [arismoko/horizon#847](https://github.com/arismoko/horizon/issues/847) |
| Cauldron interactions | Bind item handlers to the relevant cauldron interaction table before gameplay. | [arismoko/horizon#854](https://github.com/arismoko/horizon/issues/854) |
| Entity renderers | The other entity families' draw routes reading the renderer table (boats, displays, TNT, the armour stand, the ender dragon), and the armour stand's and the ender dragon's draws running the layers. | [arismoko/horizon#860](https://github.com/arismoko/horizon/issues/860) |
| Biome colour resolvers | Register named client colour resolvers for block rendering and use them when meshing. | [arismoko/horizon#873](https://github.com/arismoko/horizon/issues/873) |
| Special item models | Register client special model codecs and bake their renderers when loading item models. | [arismoko/horizon#876](https://github.com/arismoko/horizon/issues/876) |
| Custom item models | Resolve named client item model codecs during resource decoding. | [arismoko/horizon#877](https://github.com/arismoko/horizon/issues/877) |
| Custom block models | Resolve named client block model codecs during model loading and baking. | [arismoko/horizon#878](https://github.com/arismoko/horizon/issues/878) |
| Block-state models | Resolve registered client block-state model types during resource loading. | [arismoko/horizon#879](https://github.com/arismoko/horizon/issues/879) |
| Animation types | Resolve client animation type declarations when decoding animation JSON. | [arismoko/horizon#880](https://github.com/arismoko/horizon/issues/880) |
| Skull models | Associate client model factories and textures with skull types before rendering. | [arismoko/horizon#882](https://github.com/arismoko/horizon/issues/882) |
| Particle groups | Register client render groups before custom particles enter the normal grouping and drawing path. | [arismoko/horizon#883](https://github.com/arismoko/horizon/issues/883) |
| Render pipelines | Register typed client pipeline resources before a draw can request them. | [arismoko/horizon#884](https://github.com/arismoko/horizon/issues/884) |
| Sprite source types | Resolve custom client sprite source codecs before collecting atlas sprites. | [arismoko/horizon#885](https://github.com/arismoko/horizon/issues/885) |
| Texture atlases | Declare client atlas ids and sources so resource loads can stitch and publish them. | [arismoko/horizon#886](https://github.com/arismoko/horizon/issues/886) |
| Picture-in-picture renderers | Bind client render-state types to renderer factories before GUI extraction uses them. | [arismoko/horizon#887](https://github.com/arismoko/horizon/issues/887) |
| World preset editors | Bind client editor factories to preset ids before the creation screen offers them. | [arismoko/horizon#888](https://github.com/arismoko/horizon/issues/888) |
| Entity spectator shaders | Bind client post effects to the types that select them in spectator view. | [arismoko/horizon#889](https://github.com/arismoko/horizon/issues/889) |
| Client extensions | Associate typed client extensions with item, block, fluid and effect identities. | [arismoko/horizon#895](https://github.com/arismoko/horizon/issues/895) |
| Fluid models | Resolve client fluid model declarations before rendering the fluid in the world. | [arismoko/horizon#896](https://github.com/arismoko/horizon/issues/896) |
| Entity render-state modifiers | Register client callbacks keyed to the relevant renderer and apply them during extraction. | [arismoko/horizon#897](https://github.com/arismoko/horizon/issues/897) |
| Map decoration types | Register server types whose decorations are saved with maps and synced to clients. Reuse the existing client icon draw path. | [arismoko/horizon#898](https://github.com/arismoko/horizon/issues/898) |

## Parts

A per-block, per-item or per-entity moment is a method on a part, as it is a method on `Block` or `Item` in
vanilla and NeoForge. A mod attaches parts once, in `Hooks::register`, and the game asks only the parts
attached to the thing in question. [Trait mods, Parts and attachments](../../modding/src/concepts/parts.md) covers writing
them; the design is this:

- **One trait per scope.** `BlockPart`, `BlockEntityPart`, `ItemPart` and `MenuPart` are generated from the
  catalog entries whose `scope` names them, so a part on a plain block has no tick method to write.
- **A global hook only where NeoForge has a bus event.** Where a NeoForge event is the counterpart for
  the override moment, that event is a hook on `Hooks`, asked first for everything, and its cancel skips the parts.
  Where it has none, the moment is a part method alone and has no global form. A mod reacting to its own blocks
  uses a part; one reacting to every block uses the event.
- **A part is configuration.** It takes `&self` and is `Send + Sync`, like a vanilla `Block`'s properties. State
  belongs to the game, in the block entity's slots, saved compound or attachments, so it saves, syncs and
  copies with the holder and is never kept in a mod's statics.
- **The same kind twice on one thing is refused at load**, naming the mods, and a part's kind is its type.
  Parts attach by key or by tag, and the tags resolve against each game's own tags.
  Block dispatch uses a load-prepared API key lookup, refreshed on tag reload. If no part subscribes
  for that block, the game skips converting the event's state values and write collections. This
  per-block filter does not restrict ordinary global hook listeners.

Items, fluids and energy are Parts as well, as capabilities in NeoForge's transfer shape. A capability is a
`CapabilityKey` (`ITEMS`, `FLUIDS`, `ENERGY`) for a block entity, and an `ItemCapabilityKey` (`ITEM_ITEMS`,
`ITEM_FLUIDS`, `ITEM_ENERGY`) or `EntityCapabilityKey` (`ENTITY_ITEMS`) for a stack or an entity. The game
answers a key for what it owns, a mod's provider answers it for what the mod owns, and the first to answer
wins. Handlers are NeoForge's `ResourceHandler<R>`, and every write runs in a transaction opened with
`host.transaction`:

- **One journal, in the game.** A handler holds no state and no mod writes a journal. A write applies in place;
  the first write to a storage at each depth snapshots it; a nested commit hands its snapshots to its parent;
  an abort restores them. Notifications to the world run once per changed storage at the outermost commit.
- **One move.** `move_resources` moves items and fluids between two handlers, each step all or nothing, and
  energy shares its step. A handler that answers more than it was asked or offered is a mod bug: the move is
  undone and the game warns once, naming the mod.
- **Exact units.** Fluid amounts are millibuckets. A Fabric amount that is not a whole number of millibuckets
  (81 droplets) is refused, never rounded.
- **Holders have stable identities.** An `ItemPlace` names where a stack lives, so a handler inside an item
  inside a player's inventory changes the stack where it is, journaled.

Systems that run over many things at once can be added beside the part methods later without changing them,
because the game owns the data, the data saves and syncs through the attachment codecs, holders have stable
identities, and a per-hook part method stays when a system joins it.

## One dependency model, native or wasm

A mod can be built two ways, from the same source: native, compiled into the game, or to `mod.wasm`, which the
game's one sandboxed runtime runs with no rebuild. The only difference between them is the build target.

- `[dependencies]` in `horizon-mod.toml` means the same for both: required and optional dependencies, semver
  against the manifest's versions, dispatch in dependency order, and a mod publishing functions and statics its
  dependents call. Native links through Cargo; wasm links through the runtime's linker, with one copy of each
  library shared by its dependents. Both targets use one resolver, keep bundle-order ties, and refuse cycles.
- There is no wasm-only packaging unit and no native-only dependency rule. A change to the model lands for both
  targets together.
- The wasm interface is generated from the same catalog as the `Host`, at build time, and never committed: a
  committed generated file would make every two catalog changes conflict. CI reports the added, changed and
  removed imports of a change in its job summary, and a coverage test fails on an entry with no binding.
- There is no second API. Every `mod_api` type is plain data or a resource, both generated, and nothing in the
  API is written twice for wasm. An API shape that cannot cross is fixed in the API.

[Building a mod to `mod.wasm`](../../modding/src/how-to/wasm-mods.md) is the how-to. The two targets differ in how a mod is
checked against the game, because they differ in what they link to:

| | Native | Wasm |
| --- | --- | --- |
| Pinned to | the exact game build, in `[native]` | the interface: the hooks it imports |
| Refused when | the build differs | an import it needs is missing or changed |

A wasm mod keeps running on a newer game while the hooks it imports are unchanged. `horizon-mod-api` therefore
always builds for `wasm32-unknown-unknown`: a hook that needs an engine type takes a plain-data type of the
API's own, converted on the game side.

## Injections stay, and the API comes first

`[[inject]]` is the shared native/wasm escape hatch for what the API does not yet reach. The rule is
API first: a mod needing something the API lacks asks for the hook that replaces the injection, and the
injection is the stopgap until it exists. The [injections chapter](../../modding/src/concepts/injections.md) holds the current forms,
the decision ladder and the hook-request format.

## The order of the build route

Compiling a native mod against a game already built is the route that makes native mods cheap, and each step
rests on the one before:

1. **The prebuilt game.** The game as a dylib with its metadata, built daily and published as prerelease
   builds, so a native mod compiles against it instead of rebuilding the game.
2. **Slots.** `[[inject]]` listeners fill compiled points without compiling game code.
   A missing slot is refused, including on native.
3. **The internals tier.** A per-mod declared list of game internals the prebuilt game exposes. It is native
   only and logged, and each internal a mod needs is a request for an API.
4. **Wasm points.** API `[[inject]]` listeners use the same records on native and wasm.
   Engine internals and listed call sites are native only.

A native mod that needs a supported source edit uses [mixins](../../modding/src/concepts/native-route/2-mixins.md),
which compile game code. An inject never falls back to one.

## No telemetry

The launcher and the game send nothing about the player or their machine to any server: no performance, crash,
bug or benchmark uploads, and no "I'm playing" ping. Reports stay local files the player can read and share by
hand. A mod API that reported on mods would break this, so none does; the repository's `docs/telemetry.md`
says what stays local.

## What is next

The open work is not listed here, because a list in a page goes stale. It is on the
[project board](https://github.com/users/arismoko/projects/1), grouped by phase, and the pinned roadmap issue
[arismoko/horizon#771](https://github.com/arismoko/horizon/issues/771) gives the direction. A change to the mod API updates its
catalog `doc` and the pages it affects in the same pull request, so this page and the reference follow the code.
