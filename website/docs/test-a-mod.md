# Test a mod

Three kinds of checks catch most mistakes before players see them: unit tests for your own logic, the game's dev mode for how the mod looks and feels, and `cargo horizon-mod check`, which runs the same checks as the registry.

## Unit tests for your logic

Code that doesn't call the game (formatting, maths, parsing a command's arguments) is ordinary Rust. Put it in plain functions and test it with `#[test]`, as the [Playtime example](/docs/mod-code#a-worked-example-playtime) does with its `format` function:

```sh
cargo test
```

The tests run on your computer, with no game and no WebAssembly. Keeping the hooks thin and the logic in functions makes most of a mod testable this way.

## In the game, as a dev mod

Start the game with your mod as a dev mod ([how](/docs/make-a-mod#see-it-in-the-game)) and keep it running. After each edit:

```sh
cargo horizon-mod build
```

The game sees the new file within a second, says "Mods reloaded" in chat and starts the mod over. What it saved with `storage` stays. Things to try each time:

- Every command, with no arguments, with good ones and with wrong ones. An `Err` should say what to type instead.
- Leaving the world and opening another: `game::get_player()` is `None` on the title screen.
- Quitting and starting again, for a mod that saves.
- **Options, Mods...**: the mod's row lists what it may do. If it says the mod stopped, the reason is there.

The game log (`logs/latest.log` in the game folder) has your mod's `info` and `warn` lines as `[mod <id>]`, and the reason when the game refused or stopped the mod.

**Why a mod stops.** A mod that panics, runs past its time budget for one call (an endless loop, for example) or uses more than 64 MiB of memory is switched off. The rest of the game carries on. Fix the cause and build again: a dev mod starts over with the new file.

## Check it as the registry does

Before you publish, run what the registry runs on every submission:

```sh
cargo horizon-mod check target/horizon-mod/playtime.hmod
```

```
playtime 0.1.0 (mod, API 0.1, client)
  12424 bytes, SHA-256 6561…ac13, unsigned
  may draw text on your screen
  may show you chat messages
  may save its own settings to disk
  trial run: started, 40 ticks, 120 frames, a chat line, /playtime: ok
```

It reads the manifest, refuses code that uses a capability the manifest doesn't list, then plays the mod in the sandbox, as the game does: 40 ticks with a moving player, 120 frames, a chat line and each of its commands once with no arguments. The mod must start and must never crash or overrun its budget. It fails with the reason when one of those goes wrong.

- `--json` prints the report as JSON, for scripts.
- `--key <public key> --signature <file>.minisig` also checks your signature ([Publish](/docs/publish-a-mod#sign-it)).
- It checks modpacks (`.hpack`) and resource packs (`.zip`) too; `--kind mod|modpack|resourcepack` when the file's extension doesn't say which.

A mod that passes `check` passes the registry's automatic checks. A person still reviews it (see [Publish](/docs/publish-a-mod)).

Next: [publish it](/docs/publish-a-mod).
