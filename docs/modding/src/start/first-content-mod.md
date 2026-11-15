# Your first content mod

A content mod adds blocks, items, recipes, loot, models and textures with data files alone. In this page you
make one block, see it in the game, and pack it as a `.hmod`.

You need Rust 1.95 or newer (`cargo`) and the Horizon launcher with an instance to play in.

## 1. Install the mod tool

```sh
cargo install --locked --git https://github.com/horizon-rust/horizon cargo-horizon-mod
```

This adds the `cargo horizon-mod` command.

## 2. Make the project

```sh
cargo horizon-mod new glowing-blocks
cd glowing-blocks
```

The tool turns the folder name into the mod id (`glowing_blocks`) and writes a complete mod with one block:

```text
glowing-blocks/
├── horizon-mod.toml                                   # the manifest: id, name, version, api, capabilities
├── assets/glowing_blocks/
│   ├── blockstates/glowing_amethyst.json
│   ├── models/block/glowing_amethyst.json
│   ├── items/glowing_amethyst.json                    # the item model
│   └── lang/en_us.json                                # the block's name
└── data/
    ├── glowing_blocks/horizon/block/glowing_amethyst.json   # the block itself
    ├── glowing_blocks/loot_table/blocks/glowing_amethyst.json
    ├── glowing_blocks/recipe/glowing_amethyst.json
    └── minecraft/tags/block/mineable/pickaxe.json
```

The block declaration starts from a vanilla block and changes what differs:

```json
{
  "template": "minecraft:amethyst_block",
  "properties": { "hardness": 2.0, "light": 7 }
}
```

The manifest declares the capability `registries`: a mod needs it to add blocks, items, mob effects or entity
types to a world.

## 3. Build it

```sh
cargo horizon-mod build
```

This validates the content with the same checks the game uses and writes `target/horizon-mod/glowing_blocks.hmod`,
the file you share.

## 4. Play it

1. In the launcher, open your instance's folder: **Open folder** in the instance's ⋮ menu in the sidebar. This is the
   game folder, the one that holds `options.txt`.
2. Copy `glowing_blocks.hmod` into its `horizon-mods/` folder (create the folder if it is missing).
3. Turn the mod on: add the line `mod.glowing_blocks:true` to `horizon-mods.txt` in the same game folder.
4. Start the instance, open a world and craft the block from an amethyst block and glowstone dust.

The game reads content once, at start-up: after you edit a file, build again, copy the new `.hmod` and restart.

If you run a Horizon game you built yourself, `cargo horizon-mod run --game <path to horizon-client>` builds the
mod and starts that game with it always on.

## 5. Check it before you publish

```sh
cargo horizon-mod check target/horizon-mod/glowing_blocks.hmod
```

`check` runs what the mod registry runs on a submission ([Publishing your mod](../how-to/publish.md)).

## Next

- [Content mods](../how-to/content-mods.md): every kind of content file, from blocks to world generation.
- [The manifest and the package](../reference/manifest.md): every manifest field and package rule.
- [Publishing your mod](../how-to/publish.md): signing and the registry.
