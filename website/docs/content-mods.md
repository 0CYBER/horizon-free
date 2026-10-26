# Content mods

A content mod adds blocks and items to the game with data files only: no Rust, no compiler. Each new block or item copies a vanilla one (its template) and changes what differs, and comes with its recipes, loot table, tags, model, textures and name, in the same formats as vanilla's data packs and resource packs.

## What a content mod is

A folder (while you work on it) or a `.hmod` file (to share it) holding a manifest, `assets/` and `data/`. This is the SDK's [`ruby-content`](https://github.com/ludovic111/horizon/tree/main/crates/horizon-mod-sdk/examples/ruby-content) example, a ruby gem and a block of ruby:

```
ruby/
├── horizon-mod.toml
├── data/ruby/horizon/block/ruby_block.json        the block: a template and what differs
├── data/ruby/horizon/item/ruby.json               the item
├── data/ruby/recipe/ruby_block.json               recipes, as in a data pack
├── data/ruby/recipe/ruby_from_ruby_block.json
├── data/ruby/loot_table/blocks/ruby_block.json    what the block drops
├── data/minecraft/tags/block/mineable/pickaxe.json   added to vanilla's tags
├── data/minecraft/tags/block/needs_iron_tool.json
├── assets/ruby/blockstates/ruby_block.json        models and textures, as in a resource pack
├── assets/ruby/models/block/ruby_block.json
├── assets/ruby/models/item/ruby.json
├── assets/ruby/items/ruby.json
├── assets/ruby/items/ruby_block.json
├── assets/ruby/textures/block/ruby_block.png
├── assets/ruby/textures/item/ruby.png
└── assets/ruby/lang/en_us.json                    the names players see
```

Its manifest asks for API `0.2` and the `registries` capability. It has no code, so it needs no `Cargo.toml` and no `mod.wasm`:

```toml
id = "ruby"
name = "Ruby"
version = "0.1.0"
description = "A ruby gem and a block of ruby"
authors = ["You"]
api = "0.2"
side = "both"
capabilities = ["registries"]
```

## Blocks

A block is a file `data/<id>/horizon/block/<name>.json`. It becomes the block `<id>:<name>`, with the template's behaviour, states and shapes:

```json
{
  "template": "minecraft:iron_block",
  "properties": { "hardness": 4.0, "light": 3, "sounds": "minecraft:amethyst_block", "map_color": 28 }
}
```

| Key | Meaning |
| --- | --- |
| `template` | The vanilla block it copies, such as `minecraft:iron_block`, `minecraft:oak_door` or `minecraft:diamond_ore`. A door copies a door's states and shape, an ore an ore's. |
| `properties.hardness` | How long it takes to mine (vanilla's destroy time; `-1` cannot be broken). |
| `properties.explosion_resistance` | How well it stands up to explosions. |
| `properties.light` | The light it gives, 0 to 15. |
| `properties.requires_tool` | `true`: it drops nothing unless mined with the right tool (the tags say which). |
| `properties.friction` | How slippery it is (ice is `0.98`). |
| `properties.map_color` | Its colour on maps, a vanilla map colour id from 0 to 61. |
| `properties.sounds` | A vanilla block whose sounds it uses. |
| `item` | `false` for a block with no item. By default the block gets an item of the same id. |

Every property you leave out is the template's.

## Items

An item is a file `data/<id>/horizon/item/<name>.json`, copying a vanilla item:

```json
{ "template": "minecraft:diamond", "rarity": "uncommon" }
```

It may change `max_stack_size` (1 to 99) and `rarity` (`common`, `uncommon`, `rare` or `epic`, the colour of its name).

## Recipes, loot, tags, models and names

Everything else uses vanilla 26.3's own formats, so the [Minecraft Wiki's](https://minecraft.wiki/w/Data_pack) pages on data packs and resource packs apply:

- **Recipes** in `data/<id>/recipe/`, **loot tables** in `data/<id>/loot_table/`.
- **Tags**: your own in `data/<id>/tags/`. To add to vanilla's (which pickaxe mines the block, for example), put a tag file in `data/minecraft/tags/`; to add to the convention tags NeoForge and Fabric mods share, in `data/c/tags/`. They add to the tag as a data pack does; `"replace": true` drops what came before it, and the log says so.
- **Block states, models, item models and textures** in `assets/<id>/`, as in a resource pack. Models can use vanilla's parents, such as `minecraft:block/cube_all`.
- **Names** in `assets/<id>/lang/en_us.json`: `"block.ruby.ruby_block": "Block of Ruby"`, `"item.ruby.ruby": "Ruby"`.

**Structures.** A content mod can also carry structures, template pools and structure sets in its namespace, and add its own pieces to vanilla's villages or to another mod's pools. The [mod design page](https://github.com/ludovic111/horizon/blob/main/docs/mods.md#content-mods-api-02) has the details.

## Namespaces

Your files go under your mod's id (`assets/ruby/`, `data/ruby/`), plus vanilla's and the convention tags (`data/minecraft/tags/`, `data/c/tags/`). Tags are the only files a mod may put under `minecraft`.

A mod made from a Java mod whose files use other names lists them in its manifest: `namespaces = ["kaisyn", "towns_and_towers"]`. A listed namespace is the mod's own, as its id is, but blocks and items are still declared under `data/<id>/horizon/`. `minecraft`, `c` and `horizon` can't be listed, and two mods that are on can't own the same namespace: the game keeps the first and logs the clash.

## Limits

- A mod's files use vanilla's resource-name characters: at most 65,536 files, 16 MiB each and 512 MiB in all.
- A file the game can't read (a block model that doesn't parse, a recipe with a typo) is logged and skipped, as vanilla does: the rest of the mod still loads. Read the log while you work.
- Unknown keys in a block or item file are refused, and the mod stays off with the reason on its row.
- Content mods work in single player today. Playing them together on a server is planned.

## Start one

```sh
cargo horizon-mod new my-blocks --template content
```

makes a content mod with one block, Glowing Amethyst: an amethyst block that gives light, with its block state, model (using vanilla's amethyst texture), item model, name, loot table, a recipe (an amethyst block and glowstone dust) and the pickaxe tag. Rename and copy its files to add your own.

## Try it and share it

While you work, give the game the folder as a dev mod ([how](/docs/make-a-mod#see-it-in-the-game)): `--dev-mod "/path/to/ruby"`. The first start with a new set of mods takes a little longer while the game adds their content to its tables; after a change to the files, start the game again.

To share it, build the `.hmod` in the mod's folder:

```sh
cargo horizon-mod build
cargo horizon-mod check target/horizon-mod/ruby.hmod
```

A folder without a `Cargo.toml` is a content mod: `build` checks its files as the game will and packs the manifest, `assets/` and `data/` into `target/horizon-mod/<id>.hmod`, with no compiler involved. `check` reads it as the registry does and says "no code: content only, nothing to run". Players put that file in their `horizon-mods` folder and turn it on in **Options, Mods...**, and you can [publish it](/docs/publish-a-mod) like any mod.

A mod can have both: a `Cargo.toml` with code, and `assets/` and `data/` with content. `build` packs both.

Next: [test it](/docs/test-a-mod).
