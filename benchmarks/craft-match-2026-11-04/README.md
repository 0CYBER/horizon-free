# A crafting grid matched by a mod (`LevelRef::craft`)

`game::native_recipes::tests::craft_cost`: 10,000 calls of `level.craft` for each grid, release build:

```sh
HORIZON_TEST_RESOURCES=.local/horizon-client/26.3 cargo test --release -p horizon-client --lib --bins --tests \
  craft_cost -- --ignored --nocapture
```

AMD Ryzen 7 5700X, Linux, 2026-11-04, vanilla's 1,266 crafting recipes:

| Grid | A call |
|---|---|
| cake (3x3, matches early) | 5.0 µs |
| sticks (1x2, matches late, a tag ingredient) | 20.3 µs |
| firework rocket (2x1, a special recipe) | 13.2 µs |
| dirt (1x1, no recipe: the whole list) | 13.2 µs |

The matcher is the crafting table's: it tries the recipes in order until one matches, so a call costs up to one pass
over the crafting recipes, with the grid and its result allocated. A crafter that asks every tick spends at most
about 20 µs of a 50 ms tick, 0.04%; a hundred of them, 4%. A mod that keeps the result until its grid changes, as
vanilla's crafting menu does, pays it once per change. Nothing runs when no mod calls it.
