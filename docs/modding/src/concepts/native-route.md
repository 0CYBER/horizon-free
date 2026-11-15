# The native route

The native route lets a native mod change the game's own code: a function's body, a value or call inside
it, or a hot loop. You declare it with `[[native.*]]` tables written after `[native]` in `horizon-mod.toml`,
and the launcher applies them to the prebuilt game without rebuilding the game.

## When to use it

Try these first. They work on native and wasm:

1. a **hook**, a method of `mod_api::Hooks` that the game calls at one moment, or a **Part** method
   ([Your first native mod](../start/first-mod.md));
2. an **`[[inject]]` listener**: your function, run at the start or end of a game function with `mod_api`
   values ([Injections](injections.md)).

Take the native route when neither can do it: you need the game's real types or private fields, a change in
the middle of a function, a whole function replaced, or a change inside a hot loop. It is not for a wasm mod,
which cannot declare `[native]` tables. Data kept on one entity or block is an
[attachment](attachments.md), not this route.

## The tables

| Table | Use it to | Page |
| --- | --- | --- |
| `[[native.mixin]]` | run code at the start or end of a game function, change a value or a call inside it, or replace its body | [Mixins](native-route/2-mixins.md) |
| `[[native.loop]]` | run code before and after each entry of a hot loop the game publishes, such as the block entity tick | [Published loops](native-route/3-loops.md) |
| `[[native.bridge]]` | read, write or call a private field or method of a game type from your mixin code | [Bridges](native-route/4-bridges.md) |
| `[[native.module]]` | add a module of your own code inside a game source file, for your mixin code to share | [Support modules](native-route/5-support-modules.md) |
| `[[native.state]]` | add a field to a game type: refused, use an attachment | [Native state](native-route/6-native-state.md) |

Every row of these tables has an `id`. No two rows of one mod share an `id`, counting `[[inject]]` rows too.

## It is tied to one game build

A mod on this route builds against one exact game build: the commit in `[native] horizon`. Your code gets
the game's own types, such as `Game` and its fields, and these change between builds without notice.

- To move to a new build, change `horizon` and rebuild. A target that was renamed or moved fails the build
  with an error naming your mod.
- A change elsewhere in a target's body is not reported. Reread the functions you target on each new build.
- Read the game's source at your `horizon` commit on [GitHub](https://github.com/horizon-rust/horizon). Every
  target path, field and type on these pages comes from it.
- This code runs with the player's rights. It is not sandboxed.

## The `[native]` table

| Field | Value |
| --- | --- |
| `game` | The Minecraft version, `"26.3"`. |
| `horizon` | The game build: the exact Horizon commit, 40 lowercase hex characters. |
| `mixins` | The exact commit of the [horizon-mixins](https://github.com/horizon-rust/horizon-mixins) tool, which applies `[[native.mixin]]` rows; 40 lowercase hex characters. |

`cargo horizon-mod new path/to/mod --template native` writes all three; to move an existing mod, edit them. Otherwise the mod is an ordinary native mod
([Your first native mod](../start/first-mod.md)).

## Building

Build as for any native mod: the launcher's **Build**, or
`horizon-launcher native-build path/to/mod --trust-native-code`. The flag says you trust the mod's code to
run with your rights.

A build with a `[[native.mixin]]` row obtains the horizon-mixins tool for your `mixins` commit.

The prebuilt tool is checked against its release's `SHA256SUMS` and against the commit, patch and platform the
launcher pins. Where no prebuilt exists for your platform or pin, the launcher builds the tool from source.

Read the launcher's [build log](../how-to/native-mods/05-build-log.md) (`native/build.log`,
also available through **Build log**) for the number of patched functions and mixin rows.

What the launcher does under the hood is on [How the native route works](native-route/7-how-it-works.md).
