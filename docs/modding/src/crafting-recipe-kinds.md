# Recipe serializers and recipe types

A recipe file's `type` is a **serializer ID**, not its recipe type. These are two
independent registries. All crafting serializers produce `minecraft:crafting`,
which the engine registers. Each mod registers its own serializer IDs and any new
recipe type IDs in namespaces it owns. Duplicate serializer IDs and duplicate
recipe type IDs refuse startup, naming the ID.

Register crafting code in `Hooks::register`, which receives `RegistrationContext`:

```rust,ignore
r.register_crafting_recipe_serializer("my_mod:diagonal", Diagonal);
```

Implement `CraftingRecipeKind`. Its mod-owned `Value` must supply
`Some(CraftingRecipeCodec::wire())` from `codec` and implement `Wire`; a missing
codec refuses startup on native and wasm alike. `decode` reads the recipe JSON
object with `type` removed. `write` writes an equivalent JSON object for Horizon
client synchronization. Known serializers that refuse malformed data skip that
recipe alone, with the file and serializer in the diagnostic.

`matches`, `assemble` and `remaining_items` receive the decoded value and an owned
`CraftingGrid`: width, height and row-major optional stacks, including empty
cells. Callbacks must be pure and deterministic. The engine retains the owning
handler with its value. Counts must be 1 through 99; encoded values are limited
to 1 MiB. Invalid result keys/counts refuse the result. Invalid remainder keys,
counts or vector lengths use default item remainders.

The server rechecks the current grid before a take. It asks remainders once per
take, including each shift-click iteration, before consuming inputs. A remainder
returns to an empty cell, merges with the same item and components, otherwise
enters inventory, then drops through `item_toss` if it cannot fit. A crafter ejects
remainders through its result path. Clients predict default remainders for mod
kinds and accept the server's correction. Mod crafting has no recipe-book display;
`/recipe give` and `take` track hidden known IDs. Query crafting through
`LevelRef::recipe_for` with `minecraft:crafting`, regardless of serializer ID.

For a machine whose recipes the mod reads itself, declare a recipe type and an
independent serializer:

```rust,ignore
r.register_recipe_type("my_mod:grinding");
r.register_raw_recipe_serializer("my_mod:grinder_json", "my_mod:grinding");
```

`{"type":"my_mod:grinder_json","duration":42}` is then published through
`recipes_of_type("my_mod:grinding")`, with its source JSON intact. It never joins
the crafting grid or a built-in station. The machine owns its matching and menu.
For custom decode/write logic, implement `RecipeSerializer` and use
`register_read_only_recipe_serializer(serializer_id, recipe_type_id, serializer)`.
Its value follows the same explicit Wire codec rule. Type declarations may be
forward references; startup validates them after all mods register.

Every engine serializer is registered too, including serializers in the
`minecraft:` namespace. An **unregistered serializer refuses the entire recipe
load**, naming the file and ID. Dedicated startup stops; a failed `/reload`
retains the old recipe table. There is no raw fallback for unknown IDs.
Content mods (`.hmod`) shipping mod recipe JSON need the corresponding registered
serializer. Optional-mod recipes must use load conditions: a false condition
skips a file quietly **before** serializer lookup. Unknown condition types retain
the existing fatal load behavior. Malformed recipes of a known serializer retain
the existing skip-one-file behavior; changing that is a separate task.

Reload re-reads values and preserves startup registration. Joining Horizon clients
receive serializer-written content; vanilla clients receive no custom kind.
`datapack_sync.recipe_types` selects actual recipe type IDs; the engine resolves
each file's serializer to its reported type for filtering and replacement, including
all serializers producing the same type. An empty registered crafting serializer still participates in sync
so removing its last recipe clears stale client data.
