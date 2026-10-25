# Make a mod

This guide takes you from nothing to a mod that other players can install from [playhorizon.dev/mods](/mods). You write the mod in Rust. It is compiled to WebAssembly and runs in a sandbox, so a player can see what it may do before turning it on. You need no Minecraft modding experience.

## The path

1. **Set up** (this page): install Rust and `cargo horizon-mod`, make your first mod and see it in the game.
2. **[The manifest and capabilities](/docs/mod-manifest)**: the file that names your mod and lists what it may do.
3. **[Write the code](/docs/mod-code)**: the hooks the game calls, what your code can call, and a worked example.
4. **[Content mods](/docs/content-mods)**: blocks, items, recipes and textures from data files, with no code at all.
5. **[Test it](/docs/test-a-mod)**: unit tests, the game's dev mode and the registry's own checks.
6. **[Publish it](/docs/publish-a-mod)**: package, sign and submit it to the registry.

The [Mod API](/docs/api/mods) lists everything a mod can do today, and its [reference](/docs/api/mods-reference) has every function and type.

## What a mod can do today

- **Client mods** run in the player's game. They read what the F3 screen shows (position, facing, health, food, dimension, the time), draw a text panel on the screen, show chat lines only that player sees, add chat commands, and keep small files of their own. They work in single player and on any server, and a server cannot see them.
- **Content mods** add blocks and items copied from vanilla templates, with their recipes, loot tables, tags, models, textures and names. They need no code.
- **Server mods** (rules, world events, server commands) are being built. The [Mod API](/docs/api/mods#not-there-yet) says what comes next.

A Horizon mod is not a Java mod. Fabric, NeoForge and Forge jars don't run in Horizon, and Horizon mods don't run in Java Minecraft.

## Install the tools

You need [Rust](https://rustup.rs) (the stable toolchain), its WebAssembly target and `cargo horizon-mod`, the tool that makes, builds and checks mods. On Windows, macOS or Linux:

```sh
rustup target add wasm32-unknown-unknown
cargo install --git https://github.com/ludovic111/horizon cargo-horizon-mod
```

The second command builds the tool from source, which takes a few minutes the first time. Run it again later to update the tool.

You also need Horizon itself, installed with the launcher ([Install](/docs/install)), and prepared at least once so the game has its files.

## Make your first mod

```sh
cargo horizon-mod new my-mod
cd my-mod
```

`new` makes a folder with a working mod in it: a `/hello` command that answers in chat. `--name "My Mod"` sets the name players see, and `--template content` makes a [content mod](/docs/content-mods) instead, with no code. The folder holds:

```
my-mod/
├── Cargo.toml          the Rust crate (a cdylib depending on horizon-mod-sdk)
├── horizon-mod.toml    the manifest: id, name, version, what the mod may do
├── .gitignore
└── src/lib.rs          the code
```

Build it:

```sh
cargo horizon-mod build
```

This compiles the crate, checks that the code uses nothing its manifest doesn't declare, and writes `target/horizon-mod/my_mod.hmod`. A `.hmod` is a zip file holding the manifest and the compiled code (`mod.wasm`). It is the one file you give players: the same file runs on Windows, macOS and Linux.

## See it in the game

A **dev mod** is a mod the game loads from a path you give it. It is always on, and the game looks at its file once a second: build again and the game reloads it, saying "Mods reloaded" in chat. Its memory starts over; what it saved to its storage stays.

**From the launcher.** Open the launcher's **Settings** and put this in **Game arguments**, with the full path to your file:

```
--dev-mod "/path/to/my-mod/target/horizon-mod/my_mod.hmod"
```

Then press **Play**, open a world and type `/hello world` in chat. Leave the setting there while you work on the mod, and clear it when you are done. The launcher's command line does the same: `horizon-launcher play --arg --dev-mod --arg /path/to/my_mod.hmod`.

**With `cargo horizon-mod run`.** If you have a `horizon-client` you built from source, `cargo horizon-mod run` builds the mod and starts that game with it. It looks for `horizon-client` on your `PATH`, or takes `--game <path>` or the `HORIZON_CLIENT` variable; anything after `--` goes to the game:

```sh
cargo horizon-mod run --game ../horizon/target/fast/horizon-client -- --resources ../horizon/.local/horizon-client/26.3
```

Either way, edit `src/lib.rs`, run `cargo horizon-mod build`, and the game picks up the new file.

## Where things go

- **Your mod's messages** go to the game log as `[mod my_mod] …` (the `logs/latest.log` file in the game folder), and the game prints them in its terminal too.
- **Mods players install** go in the `horizon-mods` folder of the game folder (Linux `~/.local/share/horizon`, macOS `~/Library/Application Support/horizon`, Windows `%APPDATA%\horizon`). **Options, Mods...** in the game lists them, each with what it may do and a switch. They start off.
- **A mod that fails** to load, or that breaks while running, is switched off with the reason in red on its row of the Mods screen. It never takes the game down with it.

Next: [the manifest and capabilities](/docs/mod-manifest).
