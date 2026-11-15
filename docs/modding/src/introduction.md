# Modding Horizon

<p class="lede">Horizon is Minecraft: Java Edition 26.3 written in Rust. This book shows you how to make mods
for it: new blocks and items from data files, and Rust code that hooks into the game.</p>

![A Horizon mod is one .hmod package. Its horizon-mod.toml manifest, assets/ and data/ are read by every
game. A content mod stops there; a code mod adds a Rust crate, built either natively by the launcher or to
mod.wasm for the game's sandbox.](images/mod-kinds.svg)

## Three kinds of mod, one package

Every Horizon mod ships as one `.hmod` file: a zip holding a `horizon-mod.toml` manifest, the mod's
`assets/` and `data/` folders, and, for a code mod, its Rust source or compiled `mod.wasm`.

| Kind | What it holds | How it runs |
| --- | --- | --- |
| **Content mod** | `assets/` and `data/`: blocks, items, recipes, loot, tags, models, textures, world generation | The game reads it at start-up. No code. |
| **Native mod** | A Rust crate using the mod API, beside its content | The launcher compiles it on the player's machine against the exact game build it runs with. Full speed, full trust. |
| **WebAssembly mod** | `mod.wasm`, built from the same Rust source | The game runs it in a sandbox with limits on time and memory. No compiler on the player's machine. |

Native and WebAssembly mods are written against the same API, `mod_api` (the `horizon-mod-api` crate),
with the same hooks, Parts, attachments and `[dependencies]`. The build target is the only difference.

## Where to start

<div class="cards">

[**Install and play with mods**](start/installing.md)
Where mods go, how to turn them on, and what the Mods screen shows.

[**Make a content mod**](start/first-content-mod.md)
A new block with its item, model, recipe and loot, in a few files.

[**Make a native mod**](start/first-mod.md)
A Rust mod that answers a hook, built and run by the launcher.

[**Make a WebAssembly mod**](start/first-wasm-mod.md)
The same Rust, built to `mod.wasm` and run in the game's sandbox.

</div>

## How this book is organised

- **Getting started** walks you from nothing to a running mod.
- **Concepts** explains how the pieces fit: hooks and the `Host`, Parts, capabilities, attachments,
  transactions, and the routes a mod has into the game's own code.
- **How-to** pages each solve one task, grouped by what you are changing.
- **Reference** lists the manifest, the package and registry formats, and links the
  [API reference](reference/api.md), which documents every hook, event and `Host` method.

An example mod lives at <https://github.com/arismoko/horizon-machine-sample>.
A coding agent can start from [`llms.txt`](llms.txt).
