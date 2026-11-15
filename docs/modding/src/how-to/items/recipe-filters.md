# Recipe filters

You can remove recipes or replace inputs and outputs from a content mod or data
pack without listing recipe IDs. Create `data/<ns>/recipe_filter/<name>.json`
with a `{"filters": [...]}` object. Each filter requires all its match keys to
hold and exactly one action. A filter with no match keys matches every recipe:

| Key | Meaning |
|---|---|
| `id` | the recipe id; `*` matches any run of characters (`minecraft:*_planks`) |
| `type` | the recipe's `type`, with the same `*` (`mymod:*`) |
| `input` | an item id or `#tag` among the recipe's inputs |
| `output` | an item id among its outputs (anything under `result`, `results`, `output` or `outputs`) |
| `remove` | `true`: the recipe is not loaded |
| `replace_input` | `{"from": ..., "to": ...}`: every input that is `from` becomes `to` |
| `replace_output` | the same, for the outputs |

```json
{"filters": [
  {"id": "minecraft:*_planks", "remove": true},
  {"type": "mymod:*", "replace_input": {"from": "minecraft:iron_ingot", "to": "#c:ingots/iron"}}
]}
```

## Application rules

- Filters run after pack overrides are merged and recipe load conditions pass.
  They operate on JSON, including recipes of mod-defined types.
- Files apply in recipe-filter ID order; filters within a file apply in array
  order. Later filters see earlier replacements. Removal stops processing that recipe.
- A higher-priority pack's file replaces a file with the same ID.
- Input and output matching walks JSON strings, excluding `type`, `group`,
  `category` and `neoforge:conditions`. Strings under `result`, `results`,
  `output` or `outputs` count as outputs; other strings count as inputs.
  A `{"tag": "c:ingots/iron"}` value matches `#c:ingots/iron`.
- Replacement changes matching strings, not JSON object keys or structure.
  When replacing a `tag` value, the stored string omits the leading `#`.
- Crafting, station and brewing loaders, the recipe book and recipe data sent to
  players use the filtered recipes.

An unknown filter key, invalid value type, or anything other than exactly one
action skips that filter with a warning. `remove: false` does not count as an
action. A replacement requires string `from` and `to` values. Invalid JSON skips
the file with a warning.
