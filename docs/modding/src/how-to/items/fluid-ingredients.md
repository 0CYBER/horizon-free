# Fluid ingredients

Use `FluidIngredient::from_json(json, context)` to parse fluid inputs for your recipe type. Pass the `DataLoadContext` supplied to your data-load hook.

| JSON input | Matches |
| --- | --- |
| `"minecraft:water"` | The named fluid. |
| `"#my_mod:coolants"` | Fluids in the load context's tag. |
| A list of ingredients | Any listed ingredient. |
| `neoforge:compound` | Any child. |
| `neoforge:intersection` | Every child. |
| `neoforge:difference` | The base, excluding the subtracted ingredient. |

Call `test(&FluidStack)` to match a tank's contents. Components are not checked. Use `fluids()` to list accepted fluid keys in source order, including tag order, for a recipe book.

Parsing returns `Result<FluidIngredient, FluidIngredientError>`. Invalid shapes and custom kinds return `FluidIngredientError::Invalid`; unknown fluid names return `FluidIngredientError::UnknownFluid`. Fluid ingredients do not dispatch a custom ingredient hook.

## Required amounts

`SizedFluidIngredient::from_json` parses an ingredient and a positive amount in millibuckets:

```json
{"ingredient": "minecraft:water", "amount": 500}
```

- Omit `amount` to require one bucket (1000 millibuckets).
- Invalid input, including zero or a negative amount, returns `None`.
- `test(&FluidStack)` requires both a matching fluid and at least the requested amount.
