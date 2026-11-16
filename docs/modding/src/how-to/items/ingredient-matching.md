# Match custom ingredients

Implement `Hooks::ingredient_test(context, event)` to check a concrete stack's components for your [custom ingredient kind](ingredient-kinds.md). Calling `Ingredient::test(host, stack)` reaches this hook after any nonempty load-time candidate list accepts the stack's item.

Set `event.accept` to `false` to reject the stack. It starts as `true`.

## Callback contract

- `context` is the loop context of the side performing the match. The hook itself takes no `Host`.
- The event supplies the ingredient kind, its retained NBT compound and the stack being matched.
- The callback can run many times during crafting. Give the same answer for the same stack and ingredient.
- An item absent from a nonempty load-time list fails without this callback. An empty custom list still invokes the predicate: manual crafting can match an accepted stack, although recipe-book placement, property sets and stonecutter selection are unavailable.

Read `event.data` as an API `Compound` with typed `Tag` variants (`Int`, `Long`, `List`, `Compound`, and others). The loader converts recipe metadata once using vanilla JsonOps-to-NbtOps rules; matching does not reconvert it. Numeric JSON arrays remain lists. Nested null values refuse the ingredient and fail the recipe during loading.
