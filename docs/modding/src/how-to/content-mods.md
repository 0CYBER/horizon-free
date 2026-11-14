# Content mods

Add blocks, items, fluids, entities and world-generation data with a content package.
Create a project, edit its JSON files, then build and run it.

## Create and package your mod

```sh
# A prebuilt binary for Linux x86_64/aarch64, macOS x86_64/arm64 or Windows x86_64:
cargo binstall --git https://github.com/arismoko/horizon cargo-horizon-mod
# Or build it from source, without cargo-binstall:
cargo install --git https://github.com/arismoko/horizon cargo-horizon-mod
cargo horizon-mod new my-mod        # a content mod: manifest, one block with its model, loot and recipe
cd my-mod
cargo horizon-mod run               # build my-mod.hmod, then start the game with it turned on
cargo horizon-mod build             # only build: target/horizon-mod/my_mod.hmod
cargo horizon-mod check target/horizon-mod/my_mod.hmod   # the registry's checks; --json for JSON
```

- `new <path> [--name "Name"]` writes the content template. `--template native` writes
  a flat native mod instead, and `--template wasm` a wasm one: `horizon-mod.toml`, root `Cargo.toml`
  and `src/lib.rs`, and empty `assets/` and `data/` directories. The mod is pinned to the build of the current
  Nightly (players have it), or to `--horizon`'s: `nightly-<YYYY-MM-DD>`, or a commit sha (7 hex digits are enough when
  they name one build). A native mod pins it in `[native] horizon`, a wasm mod in `Cargo.toml`'s
  `[package.metadata.horizon] sdk`. `Cargo.toml` names the API by path,
  `horizon-mod-api = { path = ".horizon/sdk/api/crates/horizon-mod-api" }`, the only form the tool and the launcher
  take. Every other form is refused under `E_SDK_SOURCE`, naming why: the old git form, a version alone (Cargo
  would take the crate from crates.io, where anyone can publish its name), another source, or another path. `new` fetches that build's SDK into `.horizon/sdk/`,
  and every command that compiles the mod fetches it again when it is missing or another build's, so a fresh clone
  works after one command; your editor needs that first command too. `.horizon/` is in the template's `.gitignore`
  and is never packed. The fetch checks Horizon's signature on the release and every file against its `sdk.json`;
  `HORIZON_SDK_BASE` (a URL or a folder) reads the releases from a mirror instead. When that Nightly has no SDK,
  `new` says so and asks for `--horizon`. `update` repins a native mod the same way
  ([the SDK in a mod project](../reference/sdk/03-in-a-project.md)). Its root `impl Hooks` uses
  `#[mod_api::horizon_mod]`. Compile it with `horizon-launcher native-build <path> --trust-native-code`
  ([writing hooks](../start/writing-hooks.md)). Any other `--template` is refused.
- `build` packs the checked project into `target/horizon-mod/<id>.hmod`: `horizon-mod.toml`, `assets/`,
  `data/`, optional `icon.png` and, for code, the root Cargo project. It validates content with the same
  checks as the game; without `--wasm`, it does not compile code. Native code requires both `Cargo.toml` and `[native]`; inconsistent
  declarations and a manifest without capability `registries` are refused.
- `run` builds, then starts `horizon-client` (on `PATH`, or `--game PATH`, or `HORIZON_CLIENT`) with
  `--dev-mod target/horizon-mod/<id>.hmod`, always on. Arguments after `--` go to the game. The game reads content once
  at start, so rebuild and run again after an edit.
- `check <file>` runs what the registry runs: the file's size and SHA-256, its manifest and content, and a signature
  with `--key <public key> --signature <file>.minisig`. It reads `.hmod`, `.hpack`, resource pack `.zip`
  and `.hshader` files.
- `registry build|review|check|list|release|page` is the registry repository's CI ([the registry index](../reference/registry-index.md)).
- `use-jar <path>` records an original Java jar as an external asset/data source, without copying it into the mod
  ([Java jars as external assets](#java-jars-as-external-assets)).

## Declare content in code

Code mods declare content through `BlocksEvent` and `ItemsEvent`, using API-owned
`mod_api::Block` and `mod_api::Item` records. Start with `Block::new(id, template)` or
`Item::new(id, template)`, passing identifier strings, then set the overrides. These records are unvalidated; admission reports malformed values with the content loader's refusal messages.
These are load-time declarations, not live registry entries. The game admits them
with the content-file checks, atomically for each namespace, before resources load.
A block's `item` defaults to true, in code as in a file: `Block::with_item(false)` gives it none. Each mod receives its own declaration
batch. New identifiers from code—including block entity types and other registrar
declarations—may use any namespace owned by that mod's manifest. Bare names use
the mod ID; an explicit owned namespace stays unchanged. An unowned namespace
refuses registration with a diagnostic naming both the mod and namespace. This
is identical for native and wasm mods. Attaching a Part to an existing vanilla or
foreign block or item is a reference, not a new declaration, and remains allowed.

Code declarations carry component patches as `Vec<(String, Option<Tag>)>`:
`Some(tag)` sets the named component and `None` removes it (without a `!` prefix).
Repeated component IDs, including a set and removal of the same ID, are refused; `lore` and `minecraft:lore`
count as the same ID, and an ID may not start with the removal prefix `!`.
The game converts this opaque NBT data at load; file declarations below use JSON.
No declaration conversion runs per tick.

### Choosing what remains after removal

Implement `BlockPart::remove_block` for a block that leaves a support or another
state behind when removed. This server-only replacement receives the dimension,
position, removed state and `moved_by_piston`. Answer `Some(state)` to choose the
replacement, or `None` to keep vanilla's fluid block (air when dry). Only the
removed block's own Part is called, not the replacement block's Part.

The chosen state goes through the normal `setBlock` effects with flags 3, plus
64 for piston movement. An unknown or unregistered state is refused with a
reason and the vanilla replacement is written. This covers `Level.removeBlock`,
not every direct `setBlock` or the separate `destroyBlock` path. The level view
is read-only while answering, on both native and wasm.

## The manifest

```toml
id = "my_mod"                # [a-z0-9_]{1,64}: settings key, log name
name = "My mod"
version = "0.1.0"
description = "..."
authors = ["..."]
license = "MIT"              # optional: the Mods screen shows it
license_url = "https://..."  # optional: makes the license a link
credits = "..."              # optional: thanks, shown under the authors
homepage = "https://..."     # optional: the Mods screen's Homepage button
issues = "https://..."       # optional: the Mods screen's Issues button
api = "0.2"                  # content mods
side = "both"
capabilities = ["registries"]
namespaces = []              # optional: more namespaces the mod's files use (below)

[properties]                 # optional: values other mods read (reference/manifest.md)
tier = 3
```

The optional fields fill the game's Mods screen, which lists every mod with its icon, name, version, id, license,
authors, credits and description, as NeoForge's mod list does. A link must be `http` or `https` and at most 2048
characters; the others are plain text. The screen has no switch: a mod runs or not by what the launcher and
`horizon-mods.txt` say.

Unknown keys are refused, so a typo is an error, not a silently ignored permission. The shared manifest
uses exactly `api = "0.2"` and `capabilities = ["registries"]`, with `side = "client"` (the default) or `"both"`.
Code mods add `[native]` for exact build pins and optional raw injections ([Injections](../concepts/injections.md)); their id and version come only
from this manifest, not Cargo metadata ([Native mods](native-mods/02-the-package.md)). Both content and code mods support `[dependencies]`: use `base = "^1.2"` for a required dependency or
`extra = { version = "^2", optional = true }` for an optional one. Missing required dependencies, incompatible installed versions and cycles refuse startup. Content layers load after their dependencies; an absent optional dependency is ignored. Both kinds may include a fixed-name `icon.png` at the top level, with no
manifest key: a valid square PNG, at most 512 × 512 pixels and 256 KiB encoded ([Mod files](../reference/manifest.md)).
Use `cargo horizon-mod build --wasm` to build a Rust mod as `mod.wasm`. A wasm package
requires a matching `[wasm]` declaration. Loading refuses an undeclared or missing module, a module
over 64 MiB, invalid WebAssembly or shared-group structure, or bytes that do not match the declared SHA-512.

### Java jars as external assets

From a content or native mod's project folder:

```sh
cargo horizon-mod use-jar /path/to/original-mod.jar
```

The command reads NeoForge or Fabric metadata and appends a `[[jar]]` table to `horizon-mod.toml` with the original
`id`, `version` and the exact file's `sha512` (128 lowercase hexadecimal characters). It does not infer identity
from the filename, rename the Java mod to the Horizon mod's namespace, or run Java code. An unknown jar, invalid
identity, unresolved version or file larger than the shared 512 MiB hash limit is refused without changing the manifest.

The original manifest text, comments and existing list order are preserved. Listing the same hash and identity again
leaves the file byte-for-byte unchanged; a hash already listed with another ID or version is an error to correct,
not another entry. Use `[[jar]]` tables rather than an inline `jar = [...]` array when adding further jars with the command.
The list is ordered: new references are appended, not sorted.

Jar bytes stay external. The launcher finds the exact hash locally or asks before downloading that exact file from
Modrinth. Its `assets/` and `data/` are passive pack layers below the Horizon mod's own files; a player-local runtime
cache is not part of the mod package. Do not copy a jar or its extracted files into the mod's source or `.hmod`.
A jar reference supplies assets and data, not the original Java behavior: port that behavior through the mod API.

### Content mods (API 0.2)

A content mod has capability `registries` and carries `assets/` and `data/` in the `.hmod`. It adds blocks and items
by declaring them in `data/<id>/horizon/block/*.json` and `data/<id>/horizon/item/*.json`, each copying a vanilla
template ([modpacks.md](https://github.com/arismoko/horizon/blob/main/docs/modpacks.md#registries-02-data-driven-blocks-items-and-mob-effects)), and creative tabs, mob
effects, entity types and menu types (each below) in `creative_tab/`, `mob_effect/`, `entity_type/` and `menu/` beside
them. `fluid/` gives a fluid a client look (`sprite`, an atlas sprite id, and `tint`, `0xAARRGGBB`, both optional), as
NeoForge's `IClientFluidTypeExtensions`; the fluid needs no block, and a recipe page draws its well from it. Its `world`
object makes it exist in the world: `{"world": {}}` adds the block `<id>:<path>`, a copy of `minecraft:water` that flows
with vanilla's fluid simulation, and the bucket item `<id>:<path>_bucket`, which needs its model as any item does. `world`
takes `tick_delay` (1 or more, default 5), `drop_off` (1 to 7, default 1), `slope_find_distance` (1 to 4, default 4),
`source_conversion` (default false) and `bucket` (default true). The fluid is in no vanilla fluid tag, so entities don't
swim in it or get pushed by it. In the world it draws its `sprite`, a block atlas texture such as `mymod:block/oil`, times
its `tint`; with neither it looks like water, and a `tint` alone colours water's textures. A client mod's
`register_fluid_models` hook collects `mod_api::FluidModel` values with separate flowing and overlay sprites.
It is collected again before each resource bake, so a resource reload can change model identifiers as well as textures. Far terrain (LOD) omits mod fluids. LAN and dedicated server players play with them when they run the same mods: a client whose mods add different
content is refused when it joins, with the reason.

Every Horizon server checks loaded mods in the configuration phase (`horizon_protocol::mod_list`, channel
`horizon:mod_list`), including when its own list is empty. Only the ids and versions of `side = "both"` mods cross
this channel. Missing mods, extra both-side mods and version differences refuse the join, naming every id and
version in a translated, wrapped disconnect. Client-only mods stay private and never block a join. A manifest
with no `side` line is `client`, so a mod meant for both halves must say `side = "both"`. A vanilla or other
non-Horizon client is refused when the server needs both-side mods, with their names and versions and an English
fallback for clients without Horizon translations. The content registry identity check remains separate; the
mod list does not hash package bytes. Native build inventory validation is a separate server-export concern.

A payload that breaks the format (an oversized list, a duplicate id, trailing bytes) is refused the same way a
mismatch is: the join ends in a translated disconnect naming the decode error, never in play, and never as a
silently empty list.

Horizon peers identify themselves before sharing mod lists. After receiving the server's `horizon` brand, the
client announces its own brand and list before answering known packs; that reply orders the compatibility
check without an extra ping. The server sends its list only to an identified Horizon client, including when
either list is empty. Vanilla clients receive no mod-list payload, and clients do not disclose their mod
identities to servers that have not identified themselves as Horizon. Nothing here repeats per tick.

A block names its template and what differs from it:

```json
{ "template": "minecraft:iron_block",
  "properties": { "hardness": 4.0, "light": 3, "sounds": "minecraft:amethyst_block" },
  "item": true }
```

A fence gate needs no kind of its own: a block whose template is a vanilla `minecraft:*_fence_gate` is a
`FenceGateBlock`, with the template wood's open and close sounds. `"open_sound"` and `"close_sound"` (sound event ids,
such as `"minecraft:block.chain.break"` or a sound of the mod's own `sounds.json`) replace them, as a gate built with its
own `WoodType` has, and they are what opening by hand, a redstone signal and a wind-charge or other explosion toggle all
play. Either key needs a `*_fence_gate` template and a `namespace:path` id, or the load fails naming the file; leaving
both out keeps the wood's sounds.

A block gets a block item of its own id by default. `"item": false` gives it none: pick-block takes nothing and
`Block.asItem` is air (what a block drops comes from its loot table). `"block_item": "<item id>"` (with `"item": false`) makes the block
that other item instead, an item of the mod or a vanilla one, such as `"minecraft:diamond"`: pick-block and
`Block.asItem` both read it, and the load fails naming the block when no such item exists. `"none"` is not an item id;
write `"item": false`. A sign's wall form takes its sign's item through `wall_block`, not `block_item`.

Pick-block takes the block's item by default. `"pick_item": "<item id>"` names a different one, with any `item` and
`block_item`: vanilla's `getCloneItemStack`, which lets a mature crop pick its seed while `Block.asItem` stays what it
is (air for `"item": false`). It is an item of the mod or a vanilla one, and the load fails naming the block when no
such item exists. A sign's wall form picks its sign's item through `wall_block`, so it declares no `pick_item`.

`push_reaction` sets vanilla's `BlockBehaviour.Properties.pushReaction` to `"PUSH_PULL"`, `"PUSH"`, `"POPPED"`, `"IMMOVEABLE"`, or `"IGNORE_ENTITY"`.

`random_ticks` makes a block tick randomly whatever its template does, as vanilla's
`isRandomlyTicking(state)` does for its own blocks, so a crop, sapling or soil on a template that never
ticks grows: `"random_ticks": true` for every state (or `false`, to stop a template that ticks), or
`{ "property": "grown", "values": ["false"] }` for the states whose own state property (declared in
`state_properties`) is one of the values. The block's native `random_tick` hook then fires as it does
for a vanilla block that ticks, and its sections count it.

`render_shape` says how a block is drawn whatever its template does, as vanilla's `getRenderShape`:
`"MODEL"` puts its block model in the chunk mesh, `"ENTITYBLOCK_ANIMATED"` leaves the model out for a block whose
moving parts its block entity draws every frame (a turning shaft, a machine's arm), and `"INVISIBLE"` draws nothing.
Its outline, collision, light and occlusion stay as declared: on a template that occludes (stone), the faces it
covers stay hidden unless it also sets `occludes`.

`occludes: false` makes a block occlude nothing whatever its template does, as vanilla's `noOcclusion()`: the faces
of the blocks beside it are drawn, so a thin panel or a block drawn by its block entity on a full-cube template
doesn't leave a hole in the floor under it. It is not a solid render either, so light passes it as it passes
leaves: dampened by 1, or not at all if skylight passes down; special template light rules remain inherited.
Unset or `true` preserves the template's ability to occlude, using the declared outline when one is supplied.

`destroy_overlay: false` hides the breaking crack (`destroy_stage_0` to `9`) on the block, for one broken by use
(a feast or a cake taken apart bite by bite). It is read once at load into the block's facts, and only the overlay
is skipped: the mining progress, the server's progress and the destroy-progress packets other players see stay
vanilla's. Unset or `true`, the crack is drawn.

`dynamic_light: true` makes a block's light depend on where it is, not only on its state
(`IBlockExtension.hasDynamicLightEmission`): a lamp that glows only on a powered rail, a crystal brighter deeper
down. Its parts answer `BlockPart::light_emission` with the light a cell emits, and `light` (or `state_light`) is
what a cell emits without an answer. The game asks on the thread that owns the level, the server's and each
client's, as a chunk holding the block comes in and as an edit sets it, and keeps the answer until the cell's state
changes; the light engine never asks on its own. A part whose answer reads more than the state (the block below
it) calls `LevelRef::check_light(host, pos)` when that changes (`LevelLightEngine.checkBlock`): the server asks
again and relights, and clients ask their own parts again as its light update reaches them. The flag is per block,
as it sits on the block's row: every state of the block is asked, so a block with only some states that vary pays
the ask for all of them. Light that depends only on the state needs no part: use `light` or `state_light`.

A block copies its template's states. It can also declare state properties of
its own, which its template does not have — how a pipe says which sides it
connects to, or a machine whether it is running. Its states then become its
template's states times the combinations of the ones it declares, laid out as
vanilla lays out a block's states (`StateDefinition`): every property, the
template's and the declared ones, sorted by name with the first name varying
slowest, and each declared property's default value first. The order is fixed,
so ids do not move between loads, and a declared name may sort before one of
the template's (`bites` on a carved pumpkin, whose `facing` comes after it):

```json
{ "template": "minecraft:stone",
  "state_properties": {
    "facing": { "type": "enum", "values": ["north", "south", "east", "west"] },
    "lit": { "type": "boolean" } },
  "default_state": { "lit": "true" } }
```

A `boolean` is `false` and `true`, as vanilla spells them; an `enum` is a list
of values, and a name or value is lower case letters, digits and underscores.
A declared name may not be one the template already has, and `default_state`
takes one value per declared property too, so a block's default is exactly one
state of its own. Every combination has to fit in the states a world holds
(2^20), so a declaration that would not is refused when the mods load.

A block with template `minecraft:farmland` may set `"base_block": "namespace:block"` to choose the block it becomes when it dries out, is trampled, or has a solid block placed above it. The ID must name a registered block; without the key, the template's dirt base is used.

A block whose template is one of vanilla's signs is a sign, whatever its name: it has vanilla's sign block
entity, so the text, the editor, dye, glow ink, wax and saving all work as they do on an oak sign, and the
client draws its text. A wooden sign is two blocks and a hanging sign two more, one form per template, and the
one with the item names its wall form:

```json
{ "template": "minecraft:oak_sign", "wood_type": "ruby:elm", "wall_block": "ruby:elm_wall_sign" }
```

```json
{ "template": "minecraft:oak_wall_sign", "wood_type": "ruby:elm", "item": false }
```

The same goes for `minecraft:oak_hanging_sign` and `minecraft:oak_wall_hanging_sign`. `wall_block` is what the
sign's item places on a wall, a block of the same mod, declared in its files when the sign is; without it the
item places the template's wall sign. The wall form has no item of its own, and its sign's item is what picking
it gives. `wood_type` names the editor's background:
`ruby:elm` is `assets/ruby/textures/gui/signs/elm.png` (24x26) and `assets/ruby/textures/gui/hanging_signs/elm.png`
(16x16), and a bare name is vanilla's, such as `cherry`; without it the template's wood is used. The board is
the block's model: `minecraft:block/template_sign_rot_0` to `_rot_3` and `template_wall_sign`, and for hanging
signs `template_hanging_sign_rot_0` to `_rot_3`, `template_attached_hanging_sign_rot_0` to `_rot_3` and
`template_wall_hanging_sign`, with `all` and `particle` textures. List your blocks in vanilla's tags as well:
`minecraft:standing_signs`, `minecraft:wall_signs`, `minecraft:ceiling_hanging_signs` and
`minecraft:wall_hanging_signs`, which vanilla's other sign tags and `minecraft:mineable/axe` include. Give the
wall form a loot table of its own, or name it `<x>_wall_sign` so that it drops what `<x>_sign`'s table does.

`sign_text_color` (a `DyeColor` name such as `"white"` or `"light_blue"`, black when unset) and `sign_text_glow`
(`true` or `false`) are the text a new sign starts with, as `SignBlockEntity.createDefaultSignText` gives it, for a
dark board whose black text would not read. They apply to the front and back text of a sign when it is placed, and
dye and glow ink work on it as usual; a saved sign loads as vanilla's does. A sign and its `wall_block` declare the
same values, so a sign does not change colour when it is hung on a wall.

A block can override its targeting outline with `shapes`. The declared outline also controls render
solidity, face occlusion and default lighting: an occluding full cube is solid and blocks light; a partial
shape uses its covered faces to stop light directionally. An empty or partial outline is not a solid
render. Fluid-bearing states, including intrinsic water in bubble-column and kelp templates, retain their
fluid attenuation and downward-skylight behavior when the declared geometry is unchanged. Special
skylight and light-dampening rules remain inherited from the template: fluid-dependent skylight rules
(such as vegetation) use the resolved state's fluid, including declared waterlogging, while glass keeps
its unconditional skylight rule and leaves keep their dampening. For example, a partial poppy-template
outline passes skylight without attenuation when dry, but attenuates it by 1 when waterlogged.
`occludes: false` disables face occlusion.

Movement collision is independently selectable with `collision_shapes`; it does not change the outline,
render solidity, occlusion or lighting. Both take the same table format. Boxes are
`[minX,minY,minZ,maxX,maxY,maxZ]` in block units (one block is 1), at most 16 per shape.
Coordinates must be finite and each minimum must not exceed its maximum. `[]` is an empty shape.

```json
{
  "template": "minecraft:stone",
  "state_properties": {
    "facing": { "type": "enum", "values": ["north", "south"] },
    "count": { "type": "enum", "values": ["1", "2"] }
  },
  "shapes": {
    "properties": ["facing", "count"],
    "values": { "north,2": [[0,0,0,1,0.5,1]] },
    "default": [[0,0,0,1,1,1]]
  },
  "collision_shapes": { "default": [] }
}
```

`properties` names an ordered tuple of template and/or declared state properties; each `values` key
lists their values in that order, separated by commas without spaces. A nonmatching tuple uses
`default`. Without `default`, every tuple must be covered. A default-only table gives every state the
same shape. A single-property table uses `"properties":["axis"]`. Unknown properties, unreachable
value tuples, malformed boxes and duplicate property names reject the mod with a load error rather
than silently ignoring a typo. Omitted `shapes` preserves the template outline (or the connecting
block's `thickness` outline). Omitted `collision_shapes` preserves the existing behavior: a declared
outline also becomes collision, otherwise the template collision is retained. An explicit collision
table can be used without an outline override, and overrides `thickness` collision too.

### Entity-context shape declarations

Native declarations use the same fields on `mod_api::Block`, with API-owned
`ShapeEntityKind::{Item, Player, Living, Other}` keys and `BlockShapes` values.
`interaction_shapes` is an optional `BlockShapes`. The game validates these drafts
at content admission, just like their JSON counterparts.

`outline_context_shapes` and `collision_context_shapes` independently override
`shapes` and `collision_shapes` for a captured entity class. Their keys are
`item`, `player`, `living` (a non-player living entity) and `other`; each value
is a shape table with the same properties, tuples, defaults and box validation
as `shapes`. There is no inheritance between classes or channels. An omitted
class keeps that channel's ordinary shape. Empty and position-only queries
always keep the ordinary shape; a selector is not inferred from a runtime entity ID.

```json
{
  "template": "minecraft:stone",
  "shapes": { "default": [[0, 0, 0, 1, 0.125, 1]] },
  "outline_context_shapes": {
    "player": { "default": [[0, 0.125, 0, 1, 1, 1]] }
  },
  "collision_context_shapes": {
    "item": { "default": [[0, 0, 0, 1, 0.3125, 1]] }
  },
  "interaction_shapes": { "default": [[0, 0, 0, 1, 1, 1]] }
}
```

`interaction_shapes` uses the same state-table format. It supplies
`BlockGetter.clipWithInteractionOverride`: an earlier interaction-shape entry
changes the face of an existing outline hit while keeping its point. It cannot
create a hit through an empty outline or extend the ray's reach.

Tables compile once per resource snapshot. Collision scan emptiness and
out-of-cell metadata account for every collision variant. Dropped-item movement
and overlap checks carry `item`, player movement and placement carry `player`,
and player selection uses its own outline channel. Callers without a captured
entity class retain ordinary shapes. These declarations select state/class-only
geometry, not arbitrary world-dependent block behavior or native callbacks.

An item copies its template's data components, with `max_stack_size` and
`rarity` changed, and any other 26.3 component set or removed in
`components`, written as a data component patch is: `minecraft:<type>` sets
one, `!minecraft:<type>` with `{}` removes it. That is how a modded sword
gets its own damage and durability, or a tool its mining speed:

```json
{ "template": "minecraft:iron_sword",
  "components": {
    "minecraft:max_damage": 900,
    "minecraft:attribute_modifiers": [{ "type": "minecraft:attack_damage",
      "id": "minecraft:base_attack_damage", "amount": 7.0,
      "operation": "add_value", "slot": "mainhand" }],
    "!minecraft:repairable": {} } }
```

A held item's third-person arm pose follows its use animation, as vanilla's `AvatarRenderer.getArmPose` reads it: the
`minecraft:consumable` component's `animation` (`eat`, `drink`, `block`, `bow`, `trident`, `crossbow`, `spyglass`,
`toot_horn`, `brush`, `bundle`, `spear` or `none`), else `block` for an item with `minecraft:blocks_attacks`. A content
item with `"minecraft:consumable": { "animation": "toot_horn", "consume_seconds": 7 }` raises its arm like a goat horn
while it is used, for players and in other players' views.

`arm_pose` names the pose a humanoid's arm takes holding the item, in place of what vanilla gives it, for as long as
the item is in that hand (a used item's animation included): one of vanilla's `HumanoidModel.ArmPose` names, `empty`,
`item`, `block`, `bow_and_arrow`, `crossbow_hold`, `crossbow_charge`, `throw_trident`, `toot_horn`, `spyglass`, `spear`
or `brush`. A name that is no pose fails the mod's load, and an item without the key poses as before. It applies to
player models, in the first person's own third-person view and in other players'; `spear` poses an arm only for an item
with a kinetic weapon. A code mod that must decide by stack, hand or entity answers the `get_arm_pose` hook instead,
which wins over the declared key.

Code mods can also register a namespaced custom pose once in `Hooks::register` through
`Registrar::register_arm_pose("mymod:flip", uses_both_arms, affects_offhand)`. Keep its
`CustomArmPose` token and return `ArmPose::Custom(token)` from `get_arm_pose`. The two
flags independently control the humanoid's two-arm ordering and whether the other
hand's pose gives way. The token carries these flags as Wire data, so the pose's
flag getters do not require a registration table inside a wasm guest. Duplicate, malformed and foreign-namespace ids refuse registration.

In `arm_pose_transform`, match `event.pose` to that token and set `event.angles.right`
or `event.angles.left` to absolute `[pitch, yaw, roll]` in radians. `event.right_arm`
identifies the physical arm; `event.humanoid` supplies both stacks, the use state,
and the poses selected after `get_arm_pose` answers. Transformers follow vanilla's
physical-arm application order and do not run for a suppressed pose.
Each non-finite angle leaves the base model component unchanged. The transform runs
only for selected registered poses, on local players, remote players and humanoid mobs;
swing and mob-specific animations run afterward and can change those angles.
Native and wasm mods use the same registration, token and Wire angle value.

```json
{ "template": "minecraft:stick", "arm_pose": "toot_horn" }
```

An item can place a block by naming it with `block`, whether it is a vanilla block or a declared mod block:

```json
{ "template": "minecraft:wheat_seeds", "block": "ruby:crop" }
```

This uses the block's placement state and survival rules and consumes the item outside creative mode. Add seeds to
`minecraft:villager_plantable_seeds` for farmers to plant them. The mapping does not change what the block drops or
what Pick Block gives.

An item whose template is `minecraft:fishing_rod` (and no `block`) is a fishing rod, as a NeoForge mod's
`new FishingRodItem(...)` is: it casts, floats, bites, reels in and wears as vanilla's rod, its item model's
`minecraft:fishing_rod/cast` is true while its bobber is out, and `player_fish` hears it. List it in
`#minecraft:enchantable/fishing` for Lure and Luck of the Sea, as vanilla's rod is. Other behaviour that vanilla ties
to an item's class, such as a bow's draw or a shield's block, belongs to vanilla's item alone; components
(`minecraft:tool`, `minecraft:consumable`, `minecraft:equippable` and the others) carry over for every template.

```json
{ "template": "minecraft:fishing_rod", "components": { "minecraft:max_damage": 128 } }
```

Native mods can register component types before content loads. A content item
can set a registered type with a JSON value, such as `"sample:charge": {"charge": 42}`.
JSON values become NBT values. Values must fit the network NBT limits and
round-trip through saved NBT; negative zero is not supported. A key such as
`"!sample:charge": {}` removes the template's default. Unknown types fail when Horizon builds the content root;
`cargo horizon-mod check` cannot check native registrations.

`crafting_remainder` names what the item leaves in the crafting grid when a
recipe uses it, as a milk bucket leaves a bucket (vanilla's
`Item.Properties.craftRemainder`, not a data component): a vanilla item or
one of the mod's own, refused at load otherwise. It holds on the crafting
table, the inventory grid, the crafter and as furnace fuel (the last of the stack leaves it in
the fuel slot, a stack with more left drops it), not in a brewing stand. What the item leaves once eaten
or drunk is the `minecraft:use_remainder` component instead:

```json
{ "template": "minecraft:mushroom_stew", "crafting_remainder": "minecraft:bowl" }
```

A block's item takes the same `components`, over the components of its
template block's item; a block with `"item": false` can't have them. Block files
have no `max_stack_size` or `rarity` keys, so those two go in `components` here
(an integer 1 to 99, and one of `common`, `uncommon`, `rare`, `epic`):

```json
{ "template": "minecraft:iron_block",
  "components": {
    "minecraft:max_stack_size": 16,
    "minecraft:damage_resistant": { "types": "#minecraft:is_fire" } } }
```

A block that joins its neighbours in six directions (a cable, a pipe) copies
`minecraft:chorus_plant`, vanilla's `PipeBlock`, and names the block tag it
connects to besides itself:

```json
{ "template": "minecraft:chorus_plant", "connects_to": "#mymod:cable_connects" }
```

It keeps the chorus plant's six sides but none of its rules: it needs no
support, and each side is joined when the block there is the same block or in
the tag, on placement and whenever that neighbour changes. Its shapes are the
chorus plant's (10 pixels across) unless it sets `"thickness"`, 2 to 14
pixels: its outline and collision are then built as `PipeBlock` builds its
own, a core and an arm to each joined side.

A mod's creative inventory tabs are `data/<id>/horizon/creative_tab/*.json`:
a title, the item on the tab's button, and its items in order (items the game
doesn't have are left out):

```json
{ "title": "Ruby", "icon": "ruby:ruby", "items": ["ruby:ruby", "ruby:ruby_block"] }
```

They come after vanilla's tabs, ten to a page; the
buttons above the inventory turn the pages. The title is a lang key or English
text: `"title": "itemGroup.ruby"` uses that key from the mod's lang files, and a
lang file's `itemGroup.<mod id>.<file name>` (`creative_tab/gems.json` →
`itemGroup.ruby.gems`) translates any title. A mod's items join the search tab
under their names.

A file only makes new tabs. To put items into a tab the mod doesn't own, such as
vanilla's `minecraft:ingredients` or another mod's, a native mod listens to the
client hook `build_creative_mode_tab_contents` (NeoForge's
`BuildCreativeModeTabContentsEvent`) and pushes its stacks for that `tab`.

Unknown component types fail content loading unless a code mod registers them.
Item-id-specific behaviour does not follow a template: a copy of the trident does not throw.

A mod effect is declared in `data/<id>/horizon/mob_effect/<name>.json` with what vanilla's `MobEffect` takes: its
category (`beneficial`, `harmful` or `neutral`) and its RGB colour (0 to `0xFFFFFF`), which its particles take:

```json
{ "category": "harmful", "color": 9154528 }
```

An optional `sort_order` (an integer) sets where the effect sits among the HUD's icons and the inventory's effect list,
as NeoForge's `IMobEffectExtension.getSortOrder` does: the lists compare it where vanilla compares the colour, and it
is the colour when left out. It is fixed per effect, where NeoForge may answer per instance.

It is the effect `<id>:<name>`: `/effect give @s ruby:glow 30` gives it, saves and mobs keep it, and it reaches LAN and
dedicated server players who run the same mods (its network id follows vanilla's). Its name is the translation
`effect.<id>.<name>` from the mod's `assets/<id>/lang/`, its icon `assets/<id>/textures/mob_effect/<name>.png` (18x18,
as vanilla's; without one it draws as the missing texture). The declaration supplies neither tick behaviour nor attribute modifiers. A world opened without the mod drops the effect from its players and mobs. Clients join a server
only with the same mods' effects, as with its blocks.

Code mods read a registered type's classification with `effect_key.category(host)`,
which returns `EffectCategory::Beneficial`, `Harmful` or `Neutral`. For an active
instance, use `active_effect.id.category(host)`. The read works on both sides and
returns `None` before the effect registry is complete; it never guesses a category.

A mod entity type is declared in `data/<id>/horizon/entity_type/<name>.json`. It copies a vanilla type:
`minecraft:snowball`, a thrown item projectile, or a living mob (below). A thrown type's `item` is one of the mod's own
items (one type per item), and `damage` (0 by default) is what a hit deals to anything it hits, as `thrown` damage from
the thrower (the snowball's 3 to blazes is not copied):

```json
{ "template": "minecraft:snowball", "item": "ruby:ember", "damage": 2.0 }
```

It is the entity type `<id>:<name>`, with its network id after vanilla's. Using its item throws it as a snowball is
thrown: same speed, spread, sound and cooldown, and one item used outside creative. It flies as a snowball, breaks on
what it hits with its item's crumbs, and is drawn as its item's model (`ThrownItemRenderer`), unless a client
mod registers a renderer for it ([entity renderers](client/entity-renderers.md)). Its name is the translation
`entity.<id>.<name>`. It works in single player, on LAN and on dedicated servers with the same mods. Clients join a server only with the same mods' entity types, as with its blocks. It is saved with its
chunk, `/summon <id>:<name>` makes one, and it has its template's row in `entity-types.json`, with `template` naming
it. A world opened without the
mod keeps the saved ones as they are, frozen in place. A dispenser does not throw it.

A code mod can launch a thrown type with
`host.spawn_projectile_from_rotation(owner, &entity_type, &stack, offset, power, inaccuracy)`.
This is the owner-based `Projectile.spawnProjectileFromRotation` operation, not a summon:
ownership is set before shooting and the existing launch hook runs after insertion.
It supports registered mod thrown types and vanilla snowballs, eggs, pearls, potions,
experience bottles and wind charges. Spread uses the projectile's own random in x/y/z
order, and known owner movement is inherited (vertical movement only when airborne).
It does not consume an item, award use statistics or apply cooldowns; invalid parameters
or unsupported types return `None` without spawning or drawing random values.
The owner must be in the current level; a guest in another dimension is refused.
The carried item count is one; the supplied stack is unchanged. Once the projectile is
in the level, the stack's enchantments run their `projectile_spawned` effects on it, as
vanilla's `Projectile.applyOnProjectileSpawned` does after a throw: in the stack's order,
each effect's requirements tested with the level random just before it applies, and its
own draws from the projectile's random. `ignite`, `apply_impulse`, `play_sound` and an
`all_of` of them apply; `apply_mob_effect` and `apply_exhaustion` do nothing to a
projectile, as in vanilla. A stack with any other `projectile_spawned` effect (blocks,
damage, explosions, particles, summons, functions, item damage) is refused before spawning, so this operation never silently discards one.
Owner launch also refuses unsupported requirement contexts (including entity flags),
and invalid clamped level-value ranges before insertion. Supported requirements
are entity type predicates, random chance, and their boolean combinations.

A living type may set optional `shadow_radius` and `shadow_strength` next to `template`:
`{ "template": "minecraft:zombie", "shadow_radius": 0.2, "shadow_strength": 0.6 }`.
Each omitted value inherits the template renderer's value. Values must be finite and nonnegative;
admission errors name the file. Radius 0 disables its shadow. Radius follows `minecraft:scale`,
baby age scaling and the template's extra baby factors; cube sizes, pufferfish and dragon retain
their renderer rules. The usual visibility, 32-block radius cap and distance fade still apply.

A living type copies a mob, the way NeoForge's `EntityType.Builder.of(Zombie::new, ...)` makes a type of a vanilla
class: `minecraft:zombie`, `minecraft:villager`, `minecraft:wandering_trader`, or one of the tameable mobs
(`TamableAnimal`), `minecraft:wolf`, `minecraft:cat` and `minecraft:parrot`. A tameable type tames with its template's
item, then sits, follows and teleports to its owner, and fights for it, as its template does. Its owner, sitting state
and its template's own fields (a wolf's collar, a cat's or a parrot's variant) save and load under the type's id. A
tamed wolf type's hits are its owner's, as a wolf's are. A parrot type rides a player's shoulder and comes back off
as itself; drawing it there is its mod's (an [entity layer](client/entity-layers.md)), as vanilla draws only its own
parrot there. Every living type is tracked to clients as far, and as often, as its template. It has no `item` or
`damage`. `attributes`
sets default attribute values that differ from its template's (`EntityAttributeCreationEvent`), by attribute id, and
only for attributes the template has; one it lacks stops the content root's build:

```json
{ "template": "minecraft:zombie", "attributes": { "minecraft:max_health": 40.0, "minecraft:movement_speed": 0.3 } }
```

A living type can set `sounds.ambient`, `sounds.hurt`, `sounds.death` and `sounds.step`:

```json
{
  "template": "minecraft:wandering_trader",
  "sounds": {
    "ambient": "bird:entity.vulture.ambient",
    "hurt": "bird:hurt",
    "death": "bird:death",
    "step": "none"
  }
}
```

Each omitted slot inherits the template's existing selection, including runtime choices
such as a villager's trading voice and the supporting block's footstep sound. An explicit
full `namespace:path` id selects that sound event from the resource packs; `"none"` mutes
only that slot. Omission and `"none"` are different: a villager with no step override
plays `minecraft:block.stone.step` on stone. Invalid ids stop content admission,
with the entity and sound slot in the error. These four slots cover ordinary ambient,
hurt, death and footstep playback; additional events such as trading interactions or
projectile attacks keep their existing sounds. Native and wasm packages use this same
content definition and resource table.

A code mod can also register an attribute type of its own with `Registrar::declare_attribute(path, default,
min, max, syncable, sentiment, tooltip)` (vanilla's `RangedAttribute` in the attribute registry). Only both-side
packages may declare attributes; client-only packages refuse with `WrongSide` so extra
client mods cannot shift packet IDs. `mod_api::Sentiment` is `Positive`, `Negative`, or `Neutral`.
The id lands in the
mod's namespace, the bounds and default follow `RangedAttribute`'s own checks, `syncable` decides whether the
value syncs to clients, and `sentiment` (POSITIVE by default) colors the attribute's tooltip lines. The declared
attributes append after the vanilla ones in the attribute catalog — sorted by declaring mod id, then attribute
id, so the network ids are the same on every side running the same mods — before entity attribute rows resolve,
so a mod's entity set may name its own new attribute. Values sanitize through the definition (NaN maps to the
minimum, everything else clamps), and saves, item modifiers and the sync set carry the attribute by its string
id.

`tooltip` (`mod_api::AttributeTooltip`, `Default::default()` for vanilla's look) sets how the attribute's
modifier lines read on item tooltips, as NeoForge's `IAttributeExtension` does:

- `display: AttributeDisplay::Percentage` (NeoForge's `PercentageAttribute`) shows added values as percentages
  too, so `+0.05` reads `+5% Swim Speed`. `AttributeDisplay::Boolean` (`BooleanAttribute`) shows an added
  value above zero as enabled and any other as disabled (`horizon.attribute.modifier.enabled` and
  `.disabled`). `Ranged`, the default, is vanilla's.
- `base_modifier: Some("mymod:base_mana")` (`getBaseId`) makes a modifier with that namespaced id read as the
  green total of the player's base value and its amount, as attack damage's base modifier does. The recipe
  search, which has no player, shows it as a plain line. A base modifier that is not a namespaced id refuses
  with `InvalidId`.
- With advanced tooltips (F3+H), a mod attribute's line is followed by its modifier id, as NeoForge's default
  `getDebugInfo` does. There is no per-attribute callback for it.

The display is fixed per attribute type: NeoForge's `toValueComponent` and `toComponent` callbacks, and its
merged attribute tooltips (`getMergedStyle`), have no counterpart.

A code mod can widen the bounds of an attribute that already exists, vanilla's or another mod's, with
`Registrar::widen_attribute_bounds(id, min, max)` (Java mods use a mixin or an access transformer on
`RangedAttribute.minValue` / `maxValue`): an RPG mod lets `minecraft:max_health` go past 1024 with
`widen_attribute_bounds("minecraft:max_health", 1.0, 1e6)`. Every mod's asks join the catalog once, at load, after
the declared attributes: the bounds become their union (the lowest `min`, the highest `max`, never narrower than the
attribute's own), whatever the load order, so an ask narrower than another mod's is a silent no-op. The default never
moves. Every clamp and check of the value reads the catalog, on both sides: the player's and mobs' attributes, the
LAN record check, the sync. Only both-side packages may widen bounds (`WrongSide` otherwise); a non-finite bound
(`NotFinite`) or `min > max` (`InvalidBounds`) refuses at registration, and an attribute the catalog does not have or
a range that leaves the default out refuses the load, naming the mod. A client needs nothing more than the mod-set
check it already passes at join: a client without the mod cannot join. A save keeps a base past vanilla's bound while
the mod is there; once it is gone, the load clamps the base into the bounds and logs it, once per attribute in each
world load.

A code mod can declare a datapack registry of its own with `Registrar::declare_datapack_registry(path, sync,
fields)` (NeoForge's `DataPackRegistryEvent.NewRegistry`).
`path` lands in the mod's namespace (`spell` of `example` is `example:spell`), and `fields` is a flat record
schema: `Bool`, `Int { min, max }`, `Float { min, max }`, `Text { max_len }`, `Identifier` or `Reference { registry }`,
each required or given a default. Entry `addon:fire` is `data/addon/example/spell/fire.json`
(`DatapackRegistryDeclaration::entry_file`), the registry's namespace being a folder as NeoForge's modded
registries have it. The game decodes entries against the schema without running mod code, so native and wasm
mods get the same bounds: an undeclared key, a missing required key, a wrong type or an out-of-range value fails
the entry, and a default never stands in for invalid data. The engine adds no entry-size or field-count limit
of its own. Only both-side packages may declare registries. The game loads entries from mod content and enabled data packs.

Use the host's `level_datapack_entries(level, registry)` to list sorted entry IDs,
`level_datapack_handle(level, registry, entry)` to obtain a `DatapackHandle`, and
`level_datapack_read(level, &handle)` to copy decoded fields in schema order as
`Vec<(String, DatapackValue)>`. Reads do not access disk. Handles fail with
`StaleGeneration` after a reload or `WrongSession` in another world session.
Unknown registries and stale or parked levels return `UnknownRegistry`; absent entries return
`MissingEntry`. Listing returns `None` for an unavailable level or registry. Clients see only
registries declared `Required` or `Optional` that their server sends at join and reload.

A code mod can also use `Registrar::declare_entity_attributes(path, template, attributes)` during registration.
That declaration starts from the vanilla template's attribute row, replacing the content row's overrides.
Declared bases remain available to base-value reads, but computed values and spawn fields clamp them to each
attribute's vanilla range (for example, `minecraft:max_health` is clamped to 1–1024).

To give an existing type, vanilla or a mod's, an attribute it lacks, a code mod calls
`Registrar::modify_entity_attributes(entity, attributes)` (NeoForge's `EntityAttributeModificationEvent`) during
registration. Each attribute takes its base, or with `None` the attribute's own default. It changes the type's
default after every declared set, so entities created after load have it; a later addition of the same attribute
wins. A mod's type needs its declared set first: one that only a content file defines has no attribute row to add
to. An attribute the game does not know, or a type without an attribute row, stops the resource load:

```rust,ignore
registrar.modify_entity_attributes(
    "minecraft:zombie",
    vec![("minecraft:luck".to_string(), Some(2.5))],
)?;
```

It is a mob of its template's class under its own id, `<id>:<name>`: the template's size, model, texture, sounds and
behaviour, with its own attributes. It saves what its template's class saves under its own id (a zombie's `IsBaby`,
`CanBreakDoors` and conversion times) and loads back as itself. Its entity data is its template's class's. Its loot
table is its own, `<id>:entities/<name>` (vanilla's `EntityType.getDefaultLootTable`). It is not in vanilla's entity type
tags (`#minecraft:undead`, `#minecraft:zombies`...) unless the mod adds it. Its row in `entity-types.json` is its
template's, with `template` added and its attributes in place of the template's. To draw a custom model or texture,
a client mod registers a renderer for it ([entity renderers](client/entity-renderers.md)); the content declaration
inherits the template's goals.

#### Menu types

A mod menu type (vanilla's `MenuType`) is declared in `data/<id>/horizon/menu/<name>.json`: its container's slot count
(1 to 256), the data slots its menu syncs (`ContainerData`, 0 to 256, 0 by default), and the slots players only take
from (`results`, as a furnace's result slot):

```json
{ "slots": 9, "data_slots": 2, "results": [7, 8] }
```

It is the menu type `<id>:<name>`, with its network id after vanilla's. A mod block's container opens it when its
`menu` names it with as many slots ([native mods](native-mods.md), `r.block_container(&block, container)`). The menu holds
its slots, then the player's 36. A shift-click from the inventory goes to the first slots with room, never a result
slot, and one on the container goes to the inventory, as a chest's does. `results` fence players only: hoppers follow the
container's faces and `inputs`, so leave result slots out of `inputs`. The data slots are the block entity's `data`
(16-bit on the network, as vanilla's), all sent when it opens, then each change. A Horizon client shows it through the
mod's own screen (`screen`); without one it closes again. Clients join a server only with the same mods' menu
types, as with its blocks, and a client that isn't Horizon's is never sent one: the block opens nothing for it. Its
clicks run on the server's vanilla menu engine (`menu::Kind::Mod`), shift-clicks routed as `ChestMenu`'s, and a native
mod can decide what a slot takes in code with `menu_slot_rules` ([hook notes](https://github.com/arismoko/horizon/blob/main/crates/horizon-mod-vocab/src/hooks/menu_slot_rules.rs))
and how far away the menu stays open with `menu_still_valid` ([hook notes](https://github.com/arismoko/horizon/blob/main/crates/horizon-mod-vocab/src/hooks/menu_still_valid.rs)).
The declaration specifies per-slot rules through `results`; clicks are handled by the server.

##### Lending ingredients and linking containers

For a mod crafting menu, a server `recipe_sources` listener can lend the chest
beside its machine to recipe-book placement and transfer checks:

```rust,ignore
#[mod_api::horizon_mod]
impl mod_api::Hooks for Machine {
    fn recipe_sources(
        &self,
        _host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::RecipeSourcesEvent<'_>,
    ) {
        if event.menu == "machine:crafting" {
            let [x, y, z] = event.pos;
            event.sources.push([x + 1, y, z]);
        }
    }
}
```

`player` is the requesting player's UUID; their state is not swapped in. The
engine uses the player's inventory first, then answered containers in mod/push
order. It silently skips the open machine itself, duplicate positions (first
wins), missing or non-lendable containers, and anything after 64 accepted
containers. Chest-like block entities (including barrels, shulkers, hoppers,
dispensers and droppers) and mod containers' declared slots can lend. No answer
keeps vanilla placement. The engine owns ingredient removal, container
write-back and `container_changed` notifications, not the listener.

`menu_links` has the same input fields and an append-only `links` output for menu
types declaring linked slots. It silently skips `pos`, duplicate positions, missing
or non-lendable containers, and positions after 64 accepted containers
(`MAX_MENU_LINKS`), preserving first-accepted answer order. It refreshes the named containers' displayed items
in the same server tick. `recipe_sources` is asked per player action, never per
tick; `menu_links` is asked per open linked mod menu each tick, only when a mod
subscribes. With no listener its tick path does no work or allocation. Both hooks
are public server collect hooks with identical native and wasm behavior.

#### Namespaces and limits

The fence: files go in the mod's own namespaces (`assets/<id>/`,
`data/<id>/`, and those its manifest lists), plus additions to vanilla tags
(`data/minecraft/tags/`), to the convention tags NeoForge and Fabric mods
share (`data/c/tags/`), and sources added to vanilla's atlases
(`assets/minecraft/atlases/`), which follow vanilla's own as a reload appends them.
Templates and `sounds` name vanilla blocks; unknown keys are refused.

Some Java mods ship their files under other names than their mod id (Towns and Towers, mod id `t_and_t`, under
`kaisyn` and `towns_and_towers`). A content mod lists those in its manifest:

```toml
id = "t_and_t"
api = "0.2"
capabilities = ["registries"]
namespaces = ["kaisyn", "towns_and_towers"]
```

A listed namespace is the mod's as its id is: its `assets/` and `data/` files load, and the game reads its tags,
recipes, names and loot. Blocks, items, creative tabs, mob effects, entity types and menu types may be declared in
`data/<owned_namespace>/horizon/`. The same owned set governs code declarations.
For example, a package with ID `ported_farmersdelight` and
`namespaces = ["farmersdelight"]` can declare `farmersdelight:cooking_pot`
and its `farmersdelight:` block entity type without changing saved-world IDs.
The namespace collision fence applies. `minecraft`, `c` and
`horizon` cannot be listed. Two mods that are on may not own the same namespace (one's id or listed namespace being
the other's): the game keeps the first in load order and leaves the other's content out, with "mods A and B both use
the namespace N; turn one of them off" in the log. Paths use vanilla's resource characters, at most 65,536 files, 16
MiB each and 512 MiB in all. A mod whose content is refused is left out, with the reason in the log. The game reads
this content into its block and item tables (`horizon_client::mod_content`, [modpacks.md](https://github.com/arismoko/horizon/blob/main/docs/modpacks.md)). A tag entry
naming an id nobody registers (a block a port left out, another mod's item) is dropped, so the rest of the tag
loads; vanilla would drop the whole tag.

Tags merge as a data pack's do. Every mod's file for one tag (vanilla's `data/minecraft/tags/`, the convention
`data/c/tags/`, or another mod's) adds its values after vanilla's and the mods' before it, in load order. A file with
`"replace": true` first drops what came before it, vanilla's entries and the earlier mods', and the mods after it
add theirs, as a higher data pack does. Since the result depends on load order, the game says so in the log:
"mod towns replaces data/minecraft/tags/worldgen/biome/has_structure/village_plains.json: vanilla's entries and those
of adds are left out, ...". Only tags and `assets/minecraft/atlases/` files may use the `minecraft` namespace; other content stays in owned namespaces.

**Connecting a block to fences, walls or panes**

To make a narrow post connect to oak and other wooden fences, add its id to both
`data/minecraft/tags/block/fences.json` and `data/minecraft/tags/block/wooden_fences.json`:

```json
{ "values": ["mymod:rope_post"] }
```

Use only `fences.json` to join nether brick fences. A fence checks that the neighbour is in
`minecraft:fences` and that both blocks agree on membership in `minecraft:wooden_fences`.
A sturdy face toward the fence can also connect, subject to vanilla's connection exceptions.
Fence gates use their own orientation rule.

Use `data/minecraft/tags/block/walls.json` for a narrow block that should join walls, iron bars
and glass panes. The tags tell vanilla neighbours to draw their arms. Give the custom block
its own model and shapes for any arms it needs to draw.

Enchantments use vanilla files at `data/<namespace>/enchantment/<name>.json` (subfolders keep their path in the id,
`<namespace>:<sub>/<name>`), with the mod's own tags in `data/<namespace>/tags/enchantment/` and its English names
(`enchantment.<namespace>.<name>`) in `assets/<namespace>/lang/en_us.json`. They join vanilla's in the enchanting
table, the anvil, loot, commands and the registry a server sends its players, whose data no client knows, so it is
always sent. An enchantment reaches the enchanting table through vanilla's tags (`data/minecraft/tags/enchantment/
in_enchanting_table.json` and the others), which a mod adds to as any tag. A world's data pack replaces a mod's
enchantment of the same id. What an enchantment does beyond vanilla's effect components is the mod's code: a native
hook reads the held stack's enchantments, or claims a `post_attack` effect of its own kind
([enchantment effect kinds](items/enchantment-effect-kinds.md)).

A mod adds to structure sets, template pools and processor lists that are not its own (vanilla's villages, another
mod's houses) with a set, pool or processor list file in its own namespace holding `horizon:add_to`, a key vanilla data
never has. Its entries are appended to each set, pool or list named, after every mod's own files, by
`horizon:priority` (lower first, 1000 when it has none), then in mod and file order; the file is no set, pool or list
itself. A name nobody defines is skipped.

```json
{ "horizon:add_to": "minecraft:villages",
  "structures": [ { "structure": "mymod:village_beach", "weight": 10 } ] }
```

A pool addition holds `"elements"` as a pool does, a processor list addition `"processors"`. This is what
Lithostitched's `add_structure_set_entries`, `add_template_pool_elements` and `add_processor_list_processors` modifiers
do on NeoForge and Fabric, and `horizon-port-pack carry` writes the last two from those.

A mod's own configured and placed features (`data/<ns>/worldgen/feature/`, `placed_feature/`) generate in
biomes it does not own (vanilla's) through NeoForge's biome modifiers, files in `data/<ns>/neoforge/biome_modifier/`
(older Forge mods: `data/<ns>/forge/biome_modifier/`, `forge:` types). `add_features` appends its `features` to one
generation `step` of every biome in `biomes`; `remove_features` takes them out of its `steps` (every step when it lists
none); `none` does nothing. `biomes` and `features` are an id, a `#tag` or a list of ids; `biomes` may also be
NeoForge's `any`, `or`, `and` or `not`. Every addition comes before every removal, each in mod and file order, and a
feature already in the step is not added twice. Configured features use 26.3's `worldgen/feature/` directory
(registry `minecraft:worldgen/feature`).

Spawn modifiers also apply to those biomes: `add_spawns` appends `spawners` (one object or a list with `type`,
`weight`, `minCount` and `maxCount`) to each entity type's category, converting the count range to 26.3's `count`;
`remove_spawns` removes `entity_types` from every category. `add_spawn_costs` sets `spawn_costs` from `spawn_cost`
(`energy_budget` and `charge`) for its `entity_types`, and `remove_spawn_costs` removes them. Entity type selectors
accept an id, `#tag`, list of ids, or NeoForge's `any`, `or`, `and` or `not`.

Carver modifiers edit the biome's `carvers`: `add_carvers` appends `carvers` (an id or a list of ids) that it lacks, and
`remove_carvers` takes them out. They read the root's `worldgen/carvers.json` for the ids that exist, so a carver a
data pack adds is not known to them, and carver tags are not read. NeoForge's docs call these legacy carvers: noise
caves are not carvers.

NeoForge calls a modifier once in each of five phases (`BEFORE_EVERYTHING`, `ADD`, `REMOVE`, `MODIFY`,
`AFTER_EVERYTHING`, in that order; its public biome modifier documentation). Horizon applies every addition, then every
removal, then every `MODIFY` modifier, in mod order within each. NeoForge's own modifier types only add and remove;
`MODIFY` is for the single values (climate, colours) that a mod's own modifier type changes, so Horizon's is its own
type, `horizon:modify_biome`, which sets any of `temperature`, `downfall`, `has_precipitation`, `temperature_modifier`
(`none` or `frozen`) and `effects` colours (`sky_color`, `fog_color`, `water_color`, `water_fog_color`,
`foliage_color`, `dry_foliage_color`, `grass_color`), with vanilla's biome JSON names, on its `biomes`:

```json
{
  "type": "horizon:modify_biome",
  "biomes": "#minecraft:is_forest",
  "temperature": 0.3,
  "has_precipitation": true,
  "effects": { "sky_color": 12632319, "grass_color": "#80a0ff" }
}
```

What the file leaves out stays. A colour is an integer 0 to 0xFFFFFF or `#rrggbb`; the biome keeps `#rrggbb`, with the
sky, fog and water fog colours as its `minecraft:visual/...` attributes, where 26.3 keeps them. A value of the wrong
type, a colour out of range, an unknown colour name or a `temperature_modifier` other than `none` or `frozen` skips that
modifier with a warning naming the mod and file. `BEFORE_EVERYTHING` and `AFTER_EVERYTHING` have no modifier type
here, so the API names no phase for them.

A `null` for `foliage_color`, `dry_foliage_color` or `grass_color` removes that override from the biome's `effects`, so the
colormap tints it again; removing one the biome lacks changes nothing. The other colours are required or are attributes,
so a `null` there skips the modifier with a warning.

In code, these three optional colour overrides are `Option<BiomeColourChange>`:
`Some(BiomeColourChange::Set { rgb })` sets one, `Some(BiomeColourChange::Remove)`
removes one, and `None` preserves it.

A modifier also sets the biome's ambient and music attributes, in an `attributes` object with the vanilla keys and
values of a biome file (`warped_forest.json` has all of them). Each key present replaces the biome's whole value;
the keys it leaves out stay:

```json
{
  "type": "horizon:modify_biome",
  "biomes": "minecraft:plains",
  "attributes": {
    "minecraft:visual/ambient_particles": [{ "particle": { "type": "minecraft:ash" }, "probability": 0.25 }],
    "minecraft:audio/ambient_sounds": {
      "loop": "minecraft:ambient.warped_forest.loop",
      "additions": { "sound": "minecraft:ambient.warped_forest.additions", "tick_chance": 0.0111 },
      "mood": { "sound": "minecraft:ambient.warped_forest.mood", "tick_delay": 6000, "block_search_extent": 8, "offset": 2.0 }
    },
    "minecraft:audio/background_music": { "default": { "sound": "zinc:music.dunes", "min_delay": 12000, "max_delay": 24000 } },
    "minecraft:audio/music_volume": 0.5
  }
}
```

The particle list also accepts `{ "argument": [...] }`.

| Value | Validation |
|---|---|
| Particle or sound | Namespaced ID |
| Particle `probability`, music volume | 0 to 1 |
| Ambient additions | One object or a list; any finite `tick_chance` |
| Ambient mood | Requires sound, `tick_delay`, `block_search_extent` and `offset` |
| Music `default`, `creative`, `underwater` | Requires sound and nonnegative `min_delay` and `max_delay`; delays need not be ordered |

 Any other attribute, or a value that is not valid, skips the modifier with a warning naming the mod,
file and attribute. A client plays these from the biome data the server sends. In code, `ModifyBiome` has
`ambient_particles`, `ambient_loop`, `ambient_additions`, `ambient_mood`, `background_music` and `music_volume`; the
three ambient sound fields together make the one `ambient_sounds` value, and a value out of range refuses the mods at
startup.

**When they apply.** Modifiers are applied while the world's worldgen data is loaded, before any chunk is generated: the content build writes the modified biomes into the content root, and when a world opens, `horizon_client::vanillagen::biome_modifiers_over_packs` applies them again over the world's data packs (`shared_data_with` for the generator, `packed_biomes` for the game's biome table), so a pack that replaces a vanilla biome keeps the mods' changes and a pack's own biome a tag selects gets them. Modifiers a mod declares in code (`Registrar::declare_biome_modifier`) register before any world opens and are read at the same moment. The generator's data is loaded once per bundle and set of packs and shared by the world's dimensions, so a modifier never applies halfway through a world. The dedicated server is the same game run headless (`horizon-client --dedicated`, [server.md](https://github.com/arismoko/horizon/blob/main/docs/server.md)), so it takes this path too; `horizon-server` only supervises it.

A biome holder set can also use Horizon's own `horizon:climate` type, alone or nested in those boolean sets; it is not
a NeoForge format. It filters each biome's base `temperature` and `downfall` (the names in
[Fabric's public biome weather API](https://maven.fabricmc.net/docs/yarn-1.21-rc1+build.1/net/minecraft/world/biome/Biome.Weather.html)),
not its positional temperature. Each range takes `min`, `max`, or both, inclusive; an omitted bound is unbounded. At
least one field and one bound per range are required. Unknown keys, non-numbers and inverted bounds stop the content
load. Climate sets apply only to `biomes`, not `features`.

```json
{ "type": "neoforge:and", "values": ["#minecraft:is_overworld",
  { "type": "horizon:climate", "temperature": { "min": 0.8 }, "downfall": { "max": 0.5 } }] }
```

```json
{ "type": "neoforge:add_features", "biomes": "#minecraft:is_overworld",
  "features": "zinc:ore_zinc", "step": "underground_ores" }
```

A code mod declares the same modifiers without a file: `Registrar::declare_biome_modifier(path, modifier)` takes a
`BiomeModifier` (its `biomes`, and an operation: `AddFeatures`, `RemoveFeatures`, `AddSpawns`, `RemoveSpawns`,
`AddSpawnCosts`, `RemoveSpawnCosts`, `AddCarvers`, `RemoveCarvers` or `ModifyBiome`, whose `Option` fields are the
values above). `BiomeModifier::json` renders it as the file above, so a declared modifier and
a shipped one are applied by the same code, in the same order (additions first, then removals, then modifications, mods in dispatch
order, declarations in registration order). An id outside the mod's namespace or that is not a file path, a repeated
id, an empty list, a name that is not namespaced, an unknown generation step, a spawner of weight 0 or with its
smallest group above its largest, a spawn cost that is not finite, a climate number that is not finite, a colour above
0xFFFFFF and a temperature modifier other than `none` or `frozen` refuse the mods at startup, naming the mod.
Declared modifiers apply when the world loads, through the same content loader as file modifiers, to both the
generator and the game's biome table, including when no data pack is enabled.

A modifier changes a biome that already generates. To add a mod's own biomes to the Overworld or the Nether, a code mod
declares a **biome region** instead: `Registrar::declare_biome_region(path, region)` takes a `BiomeRegion` (its
`dimension`, `minecraft:overworld` or `minecraft:nether`; a `weight` from 1 to 1000 against vanilla's 10; and its
`points`, the same `BiomePoint`s as a mod dimension's). An area belongs to one region, and the region's own points
choose the biome there. A malformed or repeated id, an id outside the mod's namespace, another dimension, a weight
outside 1 to 1000, no points or an invalid point refuse the mods at startup, naming the mod. A new world records the
declared regions in `data/horizon/biome_regions.dat`, and from then on that record decides: a changed or added region
reaches an existing world only when its owner opts in, and a world whose region's mod is missing asks before it opens
(a dedicated server reads `[biome_regions] missing = "refuse" | "vanilla"` in `server.toml`).

The modifiers rewrite the biomes when the content root builds, and the generator orders every biome's features as
vanilla's `FeatureSorter` does, so an added feature takes its place in that order (its random seed follows from it). A
biome no modifier names keeps its text, and a world without mods keeps vanilla's biomes byte for byte. A modifier
naming a biome, biome tag, feature, feature tag, entity type or entity type tag the game doesn't have (another mod's, a
`#c:` convention tag), or of an unsupported type (a mod's own serializer), is skipped with a warning
naming the mod and file; the mods load. A malformed modifier of a type
that is read (an unknown step, a missing field, `minCount > maxCount`) stops the build, and the game starts without
content mods ("content mods not loaded: ..." in the log). Feature selectors read vanilla feature tags, not a mod's `placed_feature` tags. Modifiers are reapplied after the world's data packs, so a pack
that replaces a biome keeps the mods' changes too.

**A biome of your own.** A mod's `data/<ns>/worldgen/biome/<path>.json` (any namespace but `minecraft`) adds a biome.
The game loads it into its biome table after vanilla's biomes, sorted by path and then namespace, so vanilla's biome ids
never move. A joining client receives it in the biome registry with its data even when it has the vanilla pack, builds
it from that data, and plays its colours, ambient particles, ambient sounds and music from it; a client with the same
mod already has it. Its network id is its place in the registry sorted by path, then namespace, as in vanilla.
Saved worlds name a column's biomes, not their ids: a world saved before the mod existed holds only vanilla names and
loads unchanged; a world saved with the mod and opened without it reads the mod's biomes as plains, and the next save
writes plains. A mod biome appears in the world only where a dimension's biome source or a worldgen file uses it.

A pool element may be one of Lithostitched's: `lithostitched:limited` (`delegate`, `limit`, `min_depth`) places its
`delegate` at most `limit` times in one structure start, and `lithostitched:guaranteed` (`delegate`, `count`,
`min_depth`) is tried before its pool's other elements at each jigsaw that reaches that pool (as target or fallback)
until `count` are placed. Neither goes below a piece shallower than `min_depth` (the start is at depth 0).

A structure (and any of its pool elements, in place of the structure's) may hold
`horizon:enhanced_terrain_adaptation`: MoogsStructureLib's terrain kernel, which carves or buries terrain around the
pieces on top of vanilla's `terrain_adaptation`. `kernel_size` and `kernel_distance` shape the kernel; `top` and
`bottom` (`carve`, `bury` or `none`) act above and below a piece's floor; `bottom_offset`, `padding` (`x`, `top`,
`bottom`, `z`) and MoogsStructureLib's `band` (`bottom`, `top`, `piece_heights`) move the box it measures from. On a
structure, `rigid_junctions` adds rigid pieces' junctions as well and `inflate` grows the start's box by the kernel
radius. `kernel_size` 0 is none.

```json
{ "horizon:enhanced_terrain_adaptation": { "kernel_size": 24, "kernel_distance": 24,
    "top": "carve", "bottom": "none", "rigid_junctions": true, "inflate": true } }
```

### Passenger attachment points

An entity type can set `passenger_attachments` to seat offsets relative to its feet.
A list of exactly three plain numbers is **one** `[x, y, z]` point, not three heights.
For multiple seats use nested points, for example `[[1, 2, 3], [-1, 4, 5]]`.
Seats follow passenger order; extra passengers use the last point. Offsets scale with
the entity and rotate with its yaw.

### Custom block-state definition types

Register a type id with `Hooks::register_block_state_models` before client resources
load. Set `neoforge:definition_type` to that id in the blockstate JSON, and answer
`Hooks::instantiate_block_state_definition` on every load and reload. Read the whole
file from `event.config` and enumerate `event.states` in registry order. Append a
`BlockStateModelPick::Model` for each state you choose, with its index, model id,
quarter turns and UV-lock setting; append `Refuse` with a reason to reject a file.
See the generated hook notes for a powered-block example.

A state not picked inherits the lower pack layer's selection when present;
otherwise it draws no model. Pack layers bind independently. An unknown type,
explicit refusal, invalid model, out-of-range state, duplicate state or answer
with no picks rejects the whole layer with a diagnostic, leaving lower layers
available. Vanilla `variants` and `multipart` remain available for ordinary files.

## Per-state light

Blocks in JSON and code can set `state_light`, which overrides `properties.light`.
Use `properties` for an ordered list of state property names, `values` for
comma-separated value tuples mapped to emission (0 to 15), and `default` for
unmatched states. Without a default, every state must match a tuple.
For example, `"state_light":{"properties":["lit"],"values":{"true":15},"default":0}`
makes a lamp dark when unlit and emit 15 when lit. Code uses `mod_api::StateLight`
with the same `properties`, `values` and `default` fields on `Block::state_light`.
