# For agents

Rules for a coding agent making a Horizon mod. They are short on purpose; each links to its page.

1. **Choose the shape.** A content mod has no `Cargo.toml`. A code mod has `Cargo.toml` and `src/` at the top and
   implements `Hooks` with `#[mod_api::horizon_mod]`. Never write raw injections first.
2. **Use a hook or a Part method** when one exists (search the rustdoc of `horizon-mod-api`). A per-block,
   per-item or per-entity moment is a Part method ([Parts](../modding/src/concepts/parts.md)).
3. **Otherwise follow the [injection ladder](../modding/src/concepts/injections.md#declaration):** an API `[[inject]]` declaration
   naming a crate-root function and its `mod_points` record; use a native `[[native.mixin]]` and declared internals only when the API cannot express it. Every injection comes
   with a [hook request](../modding/src/concepts/injections.md#ask-for-the-hook). Injects never compile game code; unavailable API points are refused by name on both targets. Use a [mixin](../modding/src/concepts/native-route/2-mixins.md) for native function replacement against the prebuilt SDK.
4. **Names:** NeoForge 26.3 names for hooks that have a counterpart; no `on_` prefix; the event is `&mut`;
   handles come from the event; writes take `host`.
5. **Layout:** `horizon-mod.toml`, `assets/`, `data/`, `Cargo.toml` and `src/` at the top. The id and version come
   only from `horizon-mod.toml`; ids are `[a-z0-9_]`. No `native/` folder, no `horizon-native.json`.
6. **Manifest fields:** `id`, `name`, `version`, `description`, `authors`, `api = "0.2"`, `side`,
   `capabilities = ["registries"]`, `[dependencies]`, `[incompatible]`, `[discouraged]`, `load_before`, top-level `internals`, `[[inject]]`, `[native]`
   (`game`, `horizon`, `mixins`, optional `mixin`, `loop`, `bridge`, `module`; `state` is refused; see [the native route](../modding/src/concepts/native-route.md)) or `[wasm]`, `[[jar]]`. One of `[native]` or `[wasm]`, never both. Unknown fields are refused
   ([manifest](../modding/src/reference/manifest.md)).
7. **Freshness errors:** a changed function hash refuses the injection; use `inject update` or reselect it with `inject add`. Never substitute a different function.
8. **Data and assets** are read through a reload hook, once per reload, never on the tick.
9. **Missing hook?** File a [hook request](../modding/src/concepts/injections.md#ask-for-the-hook) on arismoko/horizon.
