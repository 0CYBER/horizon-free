# Mod API

Mods are Rust crates compiled to sandboxed WebAssembly components. They run in the player's game and, once server
mods land, wherever a world is simulated. Design and the runtime: [mods.md](../mods.md).

## Overview

A mod today can read the local player and the world clock, show chat lines only that player sees, draw a text panel
on the HUD, keep small files of its own, register chat commands, and run on every tick, frame or chat line it asks
for. A content mod adds blocks and items from JSON in its `.hmod`, copying vanilla templates, plus tags, recipes,
loot, models, textures and structures in its own namespaces. Every capability is declared in the manifest, shown to
the player before the mod is turned on, and checked against the component's real imports.

| | Count |
| --- | --- |
| Interfaces the game offers | 5: `log`, `game`, `chat`, `hud`, `storage` |
| Functions in them | 9 |
| Hooks a mod exports | 5: `init`, `tick`, `frame`, `command`, `chat` |
| Capabilities | 5: `game-read`, `chat`, `hud`, `storage`, `registries` |
| Manifest `api` lines served | 2: `0.1` (code), `0.2` (content) |
| SDK | the `Mod` trait (5 methods), `Registration` (4 builders), `export_mod!`, log helpers |
| `cargo horizon-mod` | `new`, `build`, `run` |
| Example mods | `coords-hud` (code), `ruby-content` (content) |

## Guide

1. `rustup target add wasm32-unknown-unknown`, `cargo install --git https://github.com/ludovic111/horizon
   cargo-horizon-mod`, then `cargo horizon-mod new my-mod` ([Making a mod](../mods.md#making-a-mod)).
2. Implement `Mod` for your type, list the capabilities you import in `horizon-mod.toml`
   ([The manifest](../mods.md#the-manifest)).
3. `cargo horizon-mod run` builds the `.hmod` and starts the game with it as a dev mod; after an edit,
   `cargo horizon-mod build` and the running game reloads it.
4. Content mods: [Content mods (API 0.2)](../mods.md#content-mods-api-02).

## Reference

[mods-reference.md](mods-reference.md), generated from `crates/horizon-mod-sdk/wit/horizon.wit`: every capability,
interface, type, function and hook, with the limits the guide lists (16 HUD lines of 256 characters, 8 chat lines per
call, 4 MiB of storage, fuel per call).

## Examples

- `crates/horizon-mod-sdk/examples/coords-hud`: coordinates, facing and time on the HUD; `/where`, `/coords`; storage.
- `crates/horizon-mod-sdk/examples/ruby-content`: no code; a gem and a block of ruby from vanilla templates, with
  their recipes, loot, tags, models, textures and names.
- `crates/horizon-mods/tests/guest`: a mod that misbehaves on command (spins, panics, grows its memory), for the
  sandbox tests.

## Ways in

One API (the WIT world) and one sandbox (WebAssembly), whatever the mod is written in. Every way in produces a
component for the same world, checked against the same capabilities and given the same limits; there is no second
scripting runtime in the game.

| Way in | For | Status | Template and example |
| --- | --- | --- | --- |
| Rust | most mods; the fastest | first-class: the SDK, `cargo horizon-mod` | `cargo horizon-mod new`, `coords-hud` |
| Lua scripts | people who don't want a compiler | planned: a script mod whose code is a `.lua` file, run by Horizon's own interpreter mod | planned, with the same example as Rust |
| C, C++, Zig | authors of those languages | to evaluate: bindings generated from the WIT, built without WASI | one each if it passes |
| Go (TinyGo), AssemblyScript, JavaScript / TypeScript | authors of those languages | to evaluate: each needs its toolchain to emit a component without WASI imports, or an interpreter mod as for Lua | one each if it passes |

- **Scripts run inside the sandbox.** The Lua interpreter is itself a WebAssembly mod, built in Rust with the SDK and
  shipped by Horizon. A script mod's manifest names the interpreter and its own capabilities; the game links the
  interpreter with only those, so a script can do exactly what a compiled mod with the same manifest can.
- **Hot reload.** The game keeps the interpreter compiled and loads the script into a fresh instance when the file
  changes, without restarting the game; what it saved with `storage` survives, as for dev mods.
- **Measured, not guessed.** Each way in is listed with its measured cost on the same tick-heavy example (time and
  fuel per tick, memory, load time) against Rust, and a language is supported only with a template, an example and a
  test that builds and loads it in CI.

## Stability

Preview ([stability.md](stability.md)): API 0.1 and content 0.2 are served together; a mod built for 0.1.0 keeps
loading on every 0.1.x game. Server mods come as their own line, `api = "0.3"`, experimental first.

## Not there yet

What real mods need next, in the order modpacks and servers need it:

- **Server mods** (`api = "0.3"`): the same component running on the server thread and the dedicated server, with world events (block
  broken or placed, entity spawned or killed, player joined or left), game rules and server commands with typed
  arguments.
- **World and blocks**: read blocks, block states and biomes in loaded chunks; change them through the server's block
  log, so clients and the parity harnesses see vanilla's updates.
- **Items and inventories**: the player's inventory, held item and open container; item stacks with components.
- **Entities**: query and change entities near a position; spawn from a mod's own entity types (data first).
- **Block entities and menus**: ticking machines with slots and their screens.
- **Screens and UI**: widgets built from vanilla's, a screen the mod owns, a settings page under Options > Mods.
- **Networking**: a channel between a mod's two halves, client and server, typed by the mod's own WIT.
- **Config**: typed settings the game shows and stores for the mod, on the server too.
- **Keys and the crosshair target**: keybinds in Controls; what the player is looking at.
- **Script mods and other languages**: see [Ways in](#ways-in).
- **SDK**: a headless test runner (load a mod, feed it ticks and events, assert on its output, no window), templates
  per kind of mod, typed wrappers in the SDK for every interface. Dev mods already reload when rebuilt.

## Changelog

- Experimental `input` (`experimental = ["input"]`): `claim-items`, `register-key`, `held`, `poll` (use-start,
  use-stop, key-down, key-up).
- Experimental `render` (`experimental = ["render"]`): HUD shapes (fill, quad, sprite, item, text), `gui-size`, camera
  `shake`, drawn by the game's own renderer.
- API 0.2 (content): data-driven blocks and items, tags, `horizon:add_to`, terrain adaptation.
- API 0.1.0: `log`, `game`, `chat`, `hud`, `storage`; `init`, `tick`, `frame`, `command`, `chat`.
