# Injections

![Three ways into the game: hooks and Parts first, then an [[inject]] listener at a point the game publishes,
then the native route, which compiles against the exact game build.](../images/reaching-the-game.svg)

An `[[inject]]` attaches a typed listener to an injection point without compiling game code. Prefer a named hook or Part method when one exists. API points work on native and wasm, on engine functions.

To change a call or value inside a function, replace its body, or run code inside a hot loop, a native mod
takes [the native route](native-route.md): [`[[native.mixin]]`](native-route/2-mixins.md), [`[[native.loop]]`](native-route/3-loops.md),
[`[[native.bridge]]`](native-route/4-bridges.md) and [`[[native.module]]`](native-route/5-support-modules.md).

## Declaration

```toml
[[inject]]
id = "count"
at = "horizon_client::game::Game<'_>::tick"
kind = "head"
with = "count"
priority = 0
```

`id`, `at`, `kind` and `with` identify the declaration, function, point kind and listener. `priority` defaults to zero. An injection is always through the API; there is no `access` key. An optional signature `hash` pins the target.

| Kind | Contract |
| --- | --- |
| `head` | Before the body; mutable argument record and cancellation/result |
| `return` | After the body; mutable returned value |
| `replace` | Owns the function and returns its result without running the body |

Listeners receive API records. Mutable parameters are copied back. A `Game` method's API point also supplies the side's Host. A parameter without an API/Wire form refuses registration, naming its type, on both targets.

A staged SDK build supplies these sites. A plain, unstaged game has no point sites and
refuses API listeners at load by name. The launcher selects the SDK build when any
enabled mod, native or wasm, declares an `[[inject]]` row. Each point is a replaceable
engine function with an API record: its head loads one replacement cell, and the
dispatcher for a listened point is installed through that cell.

## Ordering

Head cancellation skips the body and return point, but every head listener runs and may clear cancellation. Return listeners run only after the body. Replacement listeners run after other API listeners; otherwise priority descends, then dependency registration order, with inject ID breaking ties within a mod. Only one replacement may own a point; disabling it restores the original body. Calling the replaced function from its replacement re-enters the point, not the original body.

Native code that needs game internals uses [`[[native.mixin]]`](native-route/2-mixins.md). The `access` key is refused by name: `access = "internals"` points to `[[native.mixin]]`, and `access = "api"` says to remove the line.

A library mod publishes its own hooks and functions for other mods to use.

## Refusals

| Code or message | When |
| --- | --- |
| `E_INJECT_INVALID` | A field is malformed or unsupported; the field is named. `access = "internals"` points to `[[native.mixin]]`; `access = "api"` says to remove the line. |
| `E_INJECT_NO_SLOT` | A source-level kind (`before_call`, `after_call`, `modify_value`, `wrap_call`, `wrap_if` and the other source-level kinds): the message names the `[[native.mixin]]` form to use. Also a function whose record the API cannot carry. |
| `E_INJECT_MISSING` | `at` names no function the pinned game lists. |
| `E_INJECT_CHANGED` | The pinned `hash` does not match the function's signature; run `cargo horizon-mod inject update`. |
| "no API listener at …" | The function is listed but takes no API listener. The reason follows: a parameter with no API type, named; or "the function has no point site: it takes no replacement (…)" for a generic function, a function in a generic impl, or one taking `impl Trait`. |
| "no replace listener at …" | A `replace` at a function whose return value has no API form. |
| "this build has no point sites; start the game through the launcher's mod build" | The game was not built as the SDK. |
