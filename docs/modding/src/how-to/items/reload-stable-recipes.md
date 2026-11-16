# Cache reload-stable recipe definitions

Cache recipe **definitions** instead of fetching and parsing files on every
block-entity tick. Match against live inputs on each tick. You can use the same
cache pattern for native and wasm mods.

## Publish after the final server data

Listen to `tags_updated` on the server side. It runs after the recipes and tags have
been published, both on first open and after a successful reload. Build the whole next
table before taking the write lock, then replace the old table in one operation.
An empty table must replace the old one too: pack overrides and removals matter.
Tag-only reloads also rebuild the ingredients. A failed engine reload does not fire
the publication hook, so the previous definitions remain in force.

A reload listener's prepare/apply phase lets you read your own resources,
but is too early to cache the final recipe table. Do not move this work into the
first tick as a lazy initialization path.

This example caches a custom recipe type, whose fields live in `Recipe::json`. Use
the host's existing ingredient and result parsers rather than implementing tag,
component or custom-ingredient semantics yourself. Invalid definitions are skipped
in file order; the first matching valid definition wins.

```rust,ignore
{{#include ../../../examples/src/reload_recipes.rs:cache}}
```

Clear the table at server startup and after teardown. Share the listener's cache
with the machine Part (for example using `Arc<Processing>`); do not create a new
cache in the tick. Until publication, the empty table means no recipe can run.

```rust,ignore
{{#include ../../../examples/src/reload_recipes.rs:publication}}
```

Use one cache for the authoritative server in your mod group. The integrated,
LAN and dedicated server entry points each own a server table; the connected
client does not own it. Startup and teardown clearing prevents definitions from
being reused across sequential world opens.

## Keep live answers live

Every tick, call `result` with the actual input. `Ingredient::test` consults
the host and custom matchers each time. Do not remember a match by item ID: the
stack's components or the matcher's world-dependent answer can change without a
reload. This example takes one read guard per selection, not one per recipe, and
clones only the selected result stack, not the recipe array or its JSON.

After selection, check the current output's compatibility, count and stack limit.
Keep capability, pipe, inventory and world reads live too. Caching definitions
does not authorize a write or reserve output capacity. A blocked machine resets
progress according to its existing policy.

An example mod showing the full processing and transfer path lives at
<https://github.com/arismoko/horizon-machine-sample>.
